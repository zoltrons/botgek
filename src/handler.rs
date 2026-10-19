use serenity::all::{
    Channel, ChannelId, ChannelType, Context, CreateAllowedMentions, CreateMessage, EditMessage,
    EventHandler, GuildId, Interaction, Message, MessageId, MessageReference, MessageType,
    Permissions, Reaction, ReactionType, Ready, UserId, async_trait,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};
use tracing::{error, info, warn};

static AI_COOLDOWNS: LazyLock<Mutex<HashMap<UserId, Instant>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static BOT_SOURCE_GUARD: LazyLock<Mutex<HashMap<MessageId, Instant>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
const BOT_SOURCE_TTL: Duration = Duration::from_secs(300);

const AI_BUDGET_WINDOW: Duration = Duration::from_secs(10);
const AI_GLOBAL_REQUESTS_PER_WINDOW: u32 = 20;
const AI_GUILD_REQUESTS_PER_WINDOW: u32 = 5;

struct RateWindow {
    started: Instant,
    used: u32,
}

impl RateWindow {
    fn new(now: Instant) -> Self {
        Self {
            started: now,
            used: 0,
        }
    }

    fn available(&self, now: Instant, limit: u32) -> bool {
        now.duration_since(self.started) >= AI_BUDGET_WINDOW || self.used < limit
    }

    fn consume(&mut self, now: Instant) {
        if now.duration_since(self.started) >= AI_BUDGET_WINDOW {
            self.started = now;
            self.used = 0;
        }
        self.used = self.used.saturating_add(1);
    }
}

struct AiBudgetState {
    global: RateWindow,
    guilds: HashMap<GuildId, RateWindow>,
}

static AI_BUDGETS: LazyLock<Mutex<AiBudgetState>> = LazyLock::new(|| {
    let now = Instant::now();
    Mutex::new(AiBudgetState {
        global: RateWindow::new(now),
        guilds: HashMap::new(),
    })
});

async fn consume_ai_budget(guild_id: Option<GuildId>) -> bool {
    let now = Instant::now();
    let mut budgets = AI_BUDGETS.lock().await;
    budgets
        .guilds
        .retain(|_, window| now.duration_since(window.started) < AI_BUDGET_WINDOW);

    let guild_available = guild_id
        .and_then(|id| budgets.guilds.get(&id))
        .is_none_or(|window| window.available(now, AI_GUILD_REQUESTS_PER_WINDOW));
    if !budgets.global.available(now, AI_GLOBAL_REQUESTS_PER_WINDOW) || !guild_available {
        return false;
    }

    budgets.global.consume(now);
    if let Some(id) = guild_id {
        budgets
            .guilds
            .entry(id)
            .or_insert_with(|| RateWindow::new(now))
            .consume(now);
    }
    true
}

use crate::config::Config;
use crate::converters;
use crate::converters::translation;
use crate::settings::AppSettings;
use crate::utils::{DISCORD_CONTENT_LIMIT, truncate_chars};

pub struct Handler {
    pub config: Config,
    pub settings: Arc<RwLock<AppSettings>>,
    pub http_client: reqwest::Client,
}

#[async_trait]
impl EventHandler for Handler {
    async fn shard_stage_update(&self, _ctx: Context, event: serenity::all::ShardStageUpdateEvent) {
        crate::telemetry::DISCORD_READY.store(
            event.new == serenity::gateway::ConnectionStage::Connected,
            std::sync::atomic::Ordering::Relaxed,
        );
    }
    async fn ready(&self, ctx: Context, ready: Ready) {
        crate::telemetry::DISCORD_READY.store(true, std::sync::atomic::Ordering::Relaxed);
        info!("===========================================");
        info!("Botgek başarıyla bağlandı!");
        info!("Giriş yapılan hesap: {}", ready.user.tag());
        info!("WebUI: http://localhost:{}", self.config.web_port);
        // Yedek (proxy) alan adları **yalnızca** proxy motoru kullanılabilen platformlar için
        // yazılır: Twitter/X, Instagram ve Reddit native-only'dir (proxy modu ve yedeği yok).
        info!("TikTok Alan Adı: {}", self.config.tiktok_domain);
        info!("Bluesky Alan Adı: {}", self.config.bluesky_domain);
        info!("Native-only Platformlar: Twitter/X, Instagram, Reddit (proxy yok)");
        info!(
            "Instagram Oturum Çerezi: {}",
            if self.config.instagram_sessionid.is_some() {
                "tanımlı (reel/video kartta oynar)"
            } else {
                "yok (reel kapak karesi + etiket olarak gösterilir)"
            }
        );
        info!(
            "OpenRouter Entegrasyonu: {}",
            if self.config.openrouter_api_key.is_some() {
                "Aktif"
            } else {
                "Devre Dışı (API Key yok)"
            }
        );
        info!("Orijinal Mesajı Silme: {}", self.config.delete_original);
        info!("===========================================");

        // Global slash + mesaj komutları: DM'ler dâhil her yerde görünür. Kayıt ağ isteği
        // olduğu için gateway olay döngüsü bekletilmez.
        let http = ctx.http.clone();
        tokio::spawn(async move { crate::commands::register(http).await });
    }

    /// Slash ve sağ tık (mesaj) komutları.
    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        // Dönüşüm veri çekme + kart gönderimi nedeniyle uzun sürebilir; etkileşim hemen
        // ertelenir (komut tarafında), bu yüzden olay işleyicisi ayrı görevde çalıştırılır.
        let ctx = ctx.clone();
        let client = self.http_client.clone();
        let config = self.config.clone();
        let settings = self.settings.clone();
        tokio::spawn(async move {
            crate::commands::handle(&ctx, &interaction, &client, &config, &settings).await;
        });
    }

    async fn message(&self, ctx: Context, msg: Message) {
        let current_user_id = ctx.cache.current_user().id;
        if msg.author.id == current_user_id || msg.webhook_id.is_some() {
            return;
        }
        if !matches!(msg.kind, MessageType::Regular | MessageType::InlineReply) {
            return;
        }

        // Kendi dönüştürme kartlarımız (bot adına veya taklit webhook'u ile gönderilenler)
        // gelen mesaj olarak işlenmez: modern kartın içeriği boştur ama kartın metni
        // (kullanıcının yazdığı not) bota etiket içerebilir; bu etiket AI sohbetini
        // tetiklememeli. Kartlara verilen yanıtlar ayrı mesajlardır.
        if crate::converters::builtin::is_card_message(&msg, current_user_id).await {
            return;
        }

        // Bağlantı içermeyen, botu etiketlemeyen ve yanıt olmayan sıradan sohbet mesajı hiçbir
        // akışı tetikleyemez: ayar çözümleme (tam kopya) ve kanal sorgusu atlanır.
        let could_convert = msg.content.contains("://");
        let could_chat =
            msg.message_reference.is_some() || msg.mentions.iter().any(|u| u.id == current_user_id);
        if !could_convert && !could_chat {
            return;
        }

        let parent_id = discord_thread_parent(&ctx, msg.guild_id, msg.channel_id).await;
        let current_settings = self.settings.read().await.get_message_settings(
            msg.guild_id,
            msg.channel_id,
            parent_id,
        );

        // Bot kaynakları yalnızca açıkça etkinleştirilmiş dönüşüme girebilir. AI akışı aşağıda
        // bot girdileri için hiç çalıştırılmaz; taklit de botlar için zorla kapatılır.
        if msg.author.bot {
            if !current_settings.engine.respond_to_bots
                || !has_supported_bot_input(&msg.content)
                || !claim_bot_source(msg.id).await
            {
                return;
            }
            let mut bot_settings = current_settings.clone();
            bot_settings.engine.impersonate_user = false;
            let _ = converters::enqueue_message(
                &ctx,
                &msg,
                &self.http_client,
                &self.config,
                &bot_settings,
            )
            .await;
            return;
        }

        // 1. Sosyal Medya Link Dönüştürme Kontrolü
        let _conversion_accepted = converters::enqueue_message(
            &ctx,
            &msg,
            &self.http_client,
            &self.config,
            &current_settings,
        )
        .await;

        // 2. Bot Etiketleme (@botgekv2) veya Yanıt (Reply) ile AI Sohbet Kontrolü.
        // Modül kapalıysa yanıtlanan mesaj için REST/kayıt sorgusu hiç yapılmaz.
        if !current_settings.modules.ai_chat {
            return;
        }
        let referenced_info = if let Some(ref ref_msg) = msg.referenced_message {
            Some((
                ref_msg.author.id,
                ref_msg.content.clone(),
                is_converted_message(ref_msg, current_user_id).await,
            ))
        } else if let Some(ref msg_ref) = msg.message_reference {
            if let Some(msg_id) = msg_ref.message_id {
                match msg.channel_id.message(&ctx.http, msg_id).await {
                    Ok(referenced) => Some((
                        referenced.author.id,
                        referenced.content.clone(),
                        is_converted_message(&referenced, current_user_id).await,
                    )),
                    Err(_) => None,
                }
            } else {
                None
            }
        } else {
            None
        };

        let is_mentioned = msg.mentions.iter().any(|u| u.id == current_user_id);
        let should_trigger = should_trigger_ai_chat(
            is_mentioned,
            referenced_info.as_ref().map(|(id, _, _)| *id),
            referenced_info
                .as_ref()
                .map(|(_, content, _)| content.as_str()),
            referenced_info
                .as_ref()
                .map(|(_, _, converted)| *converted)
                .unwrap_or(false),
            current_user_id,
        );

        if should_trigger && current_settings.modules.ai_chat {
            // Modüle özel rol kontrolü (ai_chat)
            let role_filter = current_settings.get_role_filter("ai_chat");
            if !crate::policy::allows(
                &ctx,
                msg.guild_id,
                msg.author.id,
                msg.member.as_ref().map(|m| m.roles.as_slice()),
                role_filter,
            )
            .await
            {
                return;
            }
            if let Some(ref api_key) = self.config.openrouter_api_key {
                if !crate::ai::has_capacity() {
                    warn!(
                        "AI sağlayıcı yuvaları dolu, istek atlandı: {}",
                        msg.author.id
                    );
                    return;
                }
                // Hız sınırı (Rate limiting): Kullanıcı başına 3 saniye bekleme süresi
                {
                    let mut cooldowns = AI_COOLDOWNS.lock().await;
                    cooldowns.retain(|_, instant| instant.elapsed() < Duration::from_secs(10));
                    if let Some(last_time) = cooldowns.get(&msg.author.id)
                        && last_time.elapsed() < Duration::from_secs(3)
                    {
                        warn!("AI hız sınırı aşıldı, istek atlandı: {}", msg.author.id);
                        return;
                    }
                    cooldowns.insert(msg.author.id, Instant::now());
                }

                let clean_query = msg
                    .content
                    .replace(&format!("<@{}>", current_user_id), "")
                    .replace(&format!("<@!{}>", current_user_id), "")
                    .trim()
                    .to_string();

                if !clean_query.is_empty() {
                    if !consume_ai_budget(msg.guild_id).await {
                        warn!(
                            guild_id = ?msg.guild_id,
                            "AI guild/global bütçesi dolu, istek atlandı"
                        );
                        return;
                    }

                    // Discord "Yazıyor..." göstergesi
                    let _ = msg.channel_id.broadcast_typing(&ctx.http).await;

                    match crate::ai::query_openrouter(
                        &self.http_client,
                        api_key,
                        &current_settings.ai.model,
                        &current_settings.ai.system_prompt,
                        &clean_query,
                    )
                    .await
                    {
                        Ok(reply) => {
                            let safe_reply = sanitize_mentions(&reply);

                            // Güvenlik: LLM çıktısındaki @everyone veya rol etiketlerinin bildirim tetiklemesini engelle
                            let allowed_mentions = CreateAllowedMentions::new()
                                .empty_roles()
                                .empty_users()
                                .everyone(false)
                                .replied_user(true);

                            let builder = CreateMessage::new()
                                .content(truncate_chars(&safe_reply, DISCORD_CONTENT_LIMIT))
                                .reference_message(soft_reply(&msg))
                                .allowed_mentions(allowed_mentions);

                            if let Err(e) = msg.channel_id.send_message(&ctx.http, builder).await {
                                error!("AI yanıtı Discord'a gönderilemedi: {:?}", e);
                            }
                        }
                        Err(e) => {
                            // Ayrıntı yalnızca log'a yazılır: sunucu yapılandırmasına
                            // (.env, API anahtarı) dair bilgi Discord kullanıcılarına sızdırılmaz
                            error!("AI yanıtı üretilirken hata oluştu: {:?}", e);
                            let notice = if e.contains("429")
                                || e.contains("504")
                                || e.contains("rate-limited")
                            {
                                "Model sağlayıcısı şu an yoğunluk veya hız sınırı nedeniyle yanıt veremedi. Birkaç saniye sonra tekrar dene."
                            } else if e.contains("401") || e.contains("API key") {
                                "AI servisi yapılandırmasında bir sorun var, bot sahibi bilgilendirildi."
                            } else {
                                "AI servisi şu an geçici olarak yanıt veremedi. Lütfen biraz sonra tekrar dene."
                            };
                            let builder = CreateMessage::new()
                                .content(notice)
                                .reference_message(soft_reply(&msg));
                            if let Err(send_err) =
                                msg.channel_id.send_message(&ctx.http, builder).await
                            {
                                error!("AI hata bildirimi gönderilemedi: {:?}", send_err);
                            }
                        }
                    }
                }
            } else {
                info!(
                    "Bot etiketlendi veya yanıtlandı fakat OPENROUTER_API_KEY .env dosyasında tanımlı değil."
                );
            }
        }
    }

    /// Reaksiyon ekleme dinleyicisi:
    /// - 🗑️: Mesaj silme (orijinal yazar veya yönetici/moderatör)
    /// - Bayrak emojileri (🇹🇷, 🇬🇧, vb.): FxEmbed / FixupX / FxBluesky üzerinden anlık çeviri (/tr, /en)
    async fn reaction_add(&self, ctx: Context, reaction: Reaction) {
        let current_user_id = ctx.cache.current_user().id;

        // Botun kendi eklediği reaksiyonları yok say
        let user_id = match reaction.user_id {
            Some(id) if id != current_user_id => id,
            _ => return,
        };

        // 1. Silme İşlemi (🗑️)
        if reaction.emoji == ReactionType::Unicode("🗑️".to_string()) {
            // Kartı paylaşan kullanıcı kalıcı kayıttan okunur (taklit webhook mesajlarında
            // mesajın yazarı bot/kullanıcı olmadığı için); proxy mesajlarında ise mesaj
            // gövdesindeki `<@id>` etiketi kullanılır.
            let recorded_author =
                crate::converters::builtin::original_author(reaction.message_id).await;
            let original_author_id = match recorded_author {
                Some(author) => Some(author),
                None => {
                    let target_msg = match reaction.message(&ctx.http).await {
                        Ok(m) => m,
                        Err(e) => {
                            error!("Silme reaksiyon mesajı alınamadı: {:?}", e);
                            return;
                        }
                    };

                    // Mesaj bu bot tarafından atılmamışsa işlem yapma
                    if target_msg.author.id != current_user_id {
                        return;
                    }

                    extract_author_id(&target_msg.content)
                }
            };
            let is_author = original_author_id == Some(user_id);

            // Yazar zaten yetkili: üye/izin sorgusu yapılmaz.
            let is_admin_or_mod = match reaction.guild_id {
                Some(guild_id) if !is_author => {
                    let member = match reaction.member.clone() {
                        Some(member) => Ok(member),
                        None => guild_id.member(&ctx.http, user_id).await,
                    };
                    member.is_ok_and(|member| {
                        can_manage_messages(&ctx, guild_id, reaction.channel_id, &member)
                    })
                }
                _ => false,
            };

            if is_author || is_admin_or_mod {
                info!(
                    "Reaksiyon ile mesaj silme onaylandı. Silen: {} (Orijinal Yazar: {:?})",
                    user_id, original_author_id
                );
                match reaction
                    .channel_id
                    .delete_message(&ctx.http, reaction.message_id)
                    .await
                {
                    Err(e) => {
                        error!("Mesaj silinirken hata oluştu: {:?}", e);
                    }
                    _ => {
                        // Kart artık yok: kaydı da bırakma (veritabanı temiz kalır).
                        crate::converters::builtin::forget_author(reaction.message_id).await;
                    }
                }
            } else {
                warn!("Yetkisiz silme reaksiyonu! Tıklayan: {}", user_id);
                let _ = reaction.delete(&ctx.http).await;
            }
            return;
        }

        // 2. Bayrak Emojisi ile Çeviri (🇹🇷, 🇬🇧, 🇩🇪 vb.)
        // Bayrak olmayan reaksiyonlar ayar/rol/kanal sorgusuna hiç girmez (her reaksiyon
        // olayında REST çağrısı yapılmasın).
        let Some(lang_code) = translation::emoji_to_lang_code(&reaction.emoji) else {
            return;
        };
        let parent_id = discord_thread_parent(&ctx, reaction.guild_id, reaction.channel_id).await;
        let guild_settings = self.settings.read().await.get_message_settings(
            reaction.guild_id,
            reaction.channel_id,
            parent_id,
        );
        if !guild_settings.modules.translation {
            return;
        }

        // Modüle özel rol kontrolü (translation)
        let role_filter = guild_settings.get_role_filter("translation");
        // Gateway, sunucu reaksiyonlarında üyeyi de taşır: rol bilgisi REST'e gitmeden okunur.
        let known_roles = reaction.member.as_ref().map(|m| m.roles.as_slice());
        if !crate::policy::allows(&ctx, reaction.guild_id, user_id, known_roles, role_filter).await
        {
            return;
        }

        {
            // Native embed mesajları çeviri durumunu kendi kartında taşır ve proxy linki
            // içermez; bayrak reaksiyonu yalnızca proxy mesajlarını günceller.
            if crate::converters::builtin::is_native_message(reaction.message_id).await {
                return;
            }

            let mut target_msg = match reaction.message(&ctx.http).await {
                Ok(m) => m,
                Err(e) => {
                    error!("Çeviri reaksiyon mesajı alınamadı: {:?}", e);
                    return;
                }
            };

            if target_msg.author.id != current_user_id {
                return;
            }

            if let Some(new_content) =
                translation::update_link_translation(&target_msg.content, Some(lang_code))
            {
                let edit = EditMessage::new().content(new_content);
                match target_msg.edit(&ctx.http, edit).await {
                    Err(e) => {
                        error!("Mesaj çevirisi güncellenemedi: {:?}", e);
                    }
                    _ => {
                        info!(
                            "Mesaj çevirisi başarıyla güncellendi: {} (Dil: {})",
                            target_msg.id, lang_code
                        );
                    }
                }
            }
        }
    }

    /// Reaksiyon kaldırma dinleyicisi:
    /// Bayrak emojisi kaldırıldığında, mesajda başka bayrak yoksa orijinal linke döner;
    /// başka bayrak varsa o dilin çevirisine geçer.
    async fn reaction_remove(&self, ctx: Context, reaction: Reaction) {
        let current_user_id = ctx.cache.current_user().id;

        // Botun kendi işlemlerini yok say
        if reaction.user_id == Some(current_user_id) {
            return;
        }

        // Yalnızca bayrak emojisi kaldırıldığında kontrol et
        if translation::emoji_to_lang_code(&reaction.emoji).is_none() {
            return;
        }

        let parent_id = discord_thread_parent(&ctx, reaction.guild_id, reaction.channel_id).await;
        let settings = self.settings.read().await.get_message_settings(
            reaction.guild_id,
            reaction.channel_id,
            parent_id,
        );
        let Some(user_id) = reaction.user_id else {
            return;
        };
        if !settings.modules.translation
            || !crate::policy::allows(
                &ctx,
                reaction.guild_id,
                user_id,
                None,
                settings.get_role_filter("translation"),
            )
            .await
        {
            return;
        }
        if crate::converters::builtin::is_native_message(reaction.message_id).await {
            return;
        }

        let mut target_msg = match reaction.message(&ctx.http).await {
            Ok(m) => m,
            Err(e) => {
                error!("Reaksiyon kaldırma mesajı alınamadı: {:?}", e);
                return;
            }
        };

        if target_msg.author.id != current_user_id {
            return;
        }

        // Mesajda hala kalan başka bir bayrak emojisi var mı kontrol et
        let mut remaining_lang: Option<&'static str> = None;
        for r in &target_msg.reactions {
            if r.count > 0
                && let Some(code) = translation::emoji_to_lang_code(&r.reaction_type)
            {
                remaining_lang = Some(code);
                break;
            }
        }

        if let Some(new_content) =
            translation::update_link_translation(&target_msg.content, remaining_lang)
        {
            let edit = EditMessage::new().content(new_content);
            match target_msg.edit(&ctx.http, edit).await {
                Err(e) => {
                    error!("Mesaj çevirisi geri alınamadı: {:?}", e);
                }
                _ => {
                    info!(
                        "Mesaj çevirisi güncellendi (kaldırma sonrası): {} (Aktif dil: {:?})",
                        target_msg.id, remaining_lang
                    );
                }
            }
        }
    }
}

pub(crate) async fn discord_thread_parent(
    ctx: &Context,
    guild_id: Option<GuildId>,
    channel_id: ChannelId,
) -> Option<ChannelId> {
    // DM kanallarında alt başlık (thread) yoktur: REST'e gitmeden `None`.
    let guild_id = guild_id?;
    let cached = ctx.cache.guild(guild_id).and_then(|guild| {
        guild
            .channels
            .get(&channel_id)
            .or_else(|| guild.threads.iter().find(|thread| thread.id == channel_id))
            .map(|channel| (channel.kind, channel.parent_id))
    });
    if let Some((kind, parent)) = cached {
        return if matches!(
            kind,
            ChannelType::NewsThread | ChannelType::PublicThread | ChannelType::PrivateThread
        ) {
            parent
        } else {
            None
        };
    }
    let fetched = tokio::time::timeout(Duration::from_secs(3), ctx.http.get_channel(channel_id))
        .await
        .ok()?
        .ok()?;
    match fetched {
        Channel::Guild(channel)
            if matches!(
                channel.kind,
                ChannelType::NewsThread | ChannelType::PublicThread | ChannelType::PrivateThread
            ) =>
        {
            channel.parent_id
        }
        _ => None,
    }
}

/// Üyenin bu kanalda mesaj yönetme (veya yönetici) izni var mı?
///
/// Thread'ler önbellekte `guild.threads` içindedir ve izinlerini üst kanaldan alır; yalnızca `guild.channels`'a bakmak thread'lerde kanal ezmelerini yok sayıp sunucu geneli izne düşüyordu.
fn can_manage_messages(
    ctx: &Context,
    guild_id: GuildId,
    channel_id: ChannelId,
    member: &serenity::all::Member,
) -> bool {
    let allowed = |perms: Permissions| {
        perms.contains(Permissions::MANAGE_MESSAGES) || perms.contains(Permissions::ADMINISTRATOR)
    };
    if let Some(guild) = ctx.cache.guild(guild_id) {
        let channel = guild.channels.get(&channel_id).or_else(|| {
            guild
                .threads
                .iter()
                .find(|thread| thread.id == channel_id)
                .and_then(|thread| thread.parent_id)
                .and_then(|parent| guild.channels.get(&parent))
        });
        if let Some(channel) = channel {
            return allowed(guild.user_permissions_in(channel, member));
        }
    }
    #[allow(deprecated)]
    member.permissions(&ctx.cache).is_ok_and(allowed)
}

/// Yanıt referansı: hedef mesaj silinmişse (ör. `DELETE_ORIGINAL` ile dönüştürülen bağlantı)
/// Discord isteği "Unknown message" ile reddetmez, yanıtı normal mesaj olarak gönderir.
fn soft_reply(msg: &Message) -> MessageReference {
    MessageReference::from(msg).fail_if_not_exists(false)
}

fn has_supported_bot_input(content: &str) -> bool {
    let lower = content.to_ascii_lowercase();
    is_link_conversion_message(content)
        || [
            "twitter.com/",
            "x.com/",
            "t.co/",
            "tiktok.com/",
            "instagram.com/",
            "reddit.com/",
            "redd.it/",
            "bsky.app/",
            "spotify.com/",
            "twitch.tv/",
            "pixiv.net/",
            "tumblr.com/",
            "facebook.com/",
            "youtube.com/",
        ]
        .iter()
        .any(|host| lower.contains(host))
}

async fn claim_bot_source(message_id: MessageId) -> bool {
    let now = Instant::now();
    let mut guard = BOT_SOURCE_GUARD.lock().await;
    guard.retain(|_, seen| now.duration_since(*seen) < BOT_SOURCE_TTL);
    if guard.contains_key(&message_id) {
        return false;
    }
    guard.insert(message_id, now);
    true
}

/// Mesaj başındaki <@123456789> veya <@!123456789> formatından kullanıcı ID'sini çıkarır
fn extract_author_id(content: &str) -> Option<UserId> {
    if let Some(rest) = content.strip_prefix("<@") {
        let rest = rest.strip_prefix('!').unwrap_or(rest);
        if let Some(end) = rest.find('>') {
            return rest[..end].parse::<u64>().ok().map(UserId::new);
        }
    }
    None
}

/// Bu mesaj Botgek'in dönüştürme (proxy veya native kart) mesajı mı?
///
/// Kartlara verilen yanıtlarda AI sohbeti tetiklenmez. Proxy mesajları gövdedeki
/// `<@id>` etiketiyle, native kartlar ise [`crate::converters::builtin::is_card_message`]
/// ile (embed alt bilgisi imzası, içeriksiz modern kart veya kalıcı kayıt) tanınır.
async fn is_converted_message(msg: &Message, bot_id: UserId) -> bool {
    is_link_conversion_message(&msg.content)
        || crate::converters::builtin::is_card_message(msg, bot_id).await
}

/// Link dönüştürme mesajlarını tespit eder.
/// Format: `<@userid> [**Platform**](<url>)` veya taklit modunda
/// `<@userid> [Platform](url)` (eski kayıtlarda `[:](url)` biçimi de tanınır).
/// Bu mesajlara gelen yanıtlarda AI sohbet tetiklenmemeli.
pub fn is_link_conversion_message(content: &str) -> bool {
    if !content.starts_with("<@") {
        return false;
    }
    const LINK_MARKERS: [&str; 5] = [
        "[Twitter](",
        "[TikTok](",
        "[Instagram](",
        "[Reddit](",
        "[Bluesky](",
    ];
    content.contains("[:](")
        || content.contains("[**")
        || LINK_MARKERS.iter().any(|marker| content.contains(marker))
}

/// Mesajın AI sohbeti tetikleyip tetiklemeyeceğini belirler.
/// Kurallar:
/// 1. Mesaj botun bir link dönüştürme (native/proxy) mesajına yanıtsa -> ASLA tetikleme
///    (kullanıcının yanıtında mention olsa bile).
/// 2. Mesaj botun AI sohbet veya diğer mesajlarına yanıtsa -> Tetikle.
/// 3. Mesaj bota yanıt değilse ama bot etiketlenmişse -> Tetikle.
/// 4. Diğer tüm durumlarda -> Tetikleme.
pub fn should_trigger_ai_chat(
    is_mentioned: bool,
    referenced_author_id: Option<UserId>,
    referenced_content: Option<&str>,
    referenced_is_conversion: bool,
    bot_id: UserId,
) -> bool {
    // Taklit edilen mesajlarda yazar bot olmadığı için ayrıca kontrol edilir.
    if referenced_is_conversion {
        return false;
    }

    if let Some(author_id) = referenced_author_id
        && author_id == bot_id
    {
        if let Some(content) = referenced_content
            && is_link_conversion_message(content)
        {
            return false;
        }
        return true;
    }
    is_mentioned
}

/// LLM veya kullanıcı girdilerinden kaynaklanan @everyone, @here veya rol pingleme risklerini
/// sıfır genişlikli boşluk (zero-width space, U+200B) enjekte ederek tamamen etkisiz hale getirir.
/// Bu sayede Discord mesajı metin olarak gösterse bile asla mention olarak algılamaz ve bildirim tetiklemez.
pub fn sanitize_mentions(text: &str) -> String {
    static RE_EVERYONE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?i)@\s*everyone").unwrap());
    static RE_HERE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?i)@\s*here").unwrap());
    static RE_ROLES: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"<@&(\d+)>").unwrap());

    let step1 = RE_EVERYONE.replace_all(text, "@\u{200b}everyone");
    let step2 = RE_HERE.replace_all(&step1, "@\u{200b}here");
    let step3 = RE_ROLES.replace_all(&step2, "<@&\u{200b}$1>");

    step3.into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_mentions() {
        let input = "Selam @everyone ve @HERE, ayrıca @ everyone toplanın! Rol: <@&123456789>";
        let cleaned = sanitize_mentions(input);

        assert!(!cleaned.contains("@everyone"));
        assert!(!cleaned.contains("@HERE"));
        assert!(!cleaned.contains("@ everyone"));
        assert!(!cleaned.contains("<@&123456789>"));

        assert!(cleaned.contains("@\u{200b}everyone"));
        assert!(cleaned.contains("@\u{200b}here"));
        assert!(cleaned.contains("<@&\u{200b}123456789>"));
    }

    #[test]
    fn test_is_link_conversion_message() {
        // Link dönüştürme mesajları (AI tetiklenmemeli)
        assert!(is_link_conversion_message(
            "<@123456789>[:](https://vxtwitter.com/user/status/123) [**Twitter**](<https://twitter.com/user/status/123>)"
        ));
        assert!(is_link_conversion_message(
            "<@999>[:](https://vxtiktok.com/t/abc) [**TikTok**](<https://tiktok.com/t/abc>)"
        ));
        // Yeni biçim: link hilesi yok, okunabilir masked link
        assert!(is_link_conversion_message(
            "<@123456789> [Twitter](https://x.com/user/status/123)"
        ));
        assert!(is_link_conversion_message(
            "<@123456789> [Reddit](https://www.reddit.com/r/rust/comments/abc/x/)"
        ));

        // AI sohbet yanıtları veya normal mesajlar (AI tetiklenmeli)
        assert!(!is_link_conversion_message("Evet, haklısın bu konuda."));
        assert!(!is_link_conversion_message("Selam! Ben Botgek."));
        assert!(!is_link_conversion_message(""));
        // Kullanıcının kendi mesajı link içerse bile dönüştürme mesajı sayılmaz
        assert!(!is_link_conversion_message(
            "[Twitter](https://x.com/user/status/123)"
        ));
    }

    #[test]
    fn test_extract_author_id() {
        assert_eq!(
            extract_author_id("<@123456789>[:](url)"),
            Some(UserId::new(123456789))
        );
        assert_eq!(
            extract_author_id("<@!987654321>[:](url)"),
            Some(UserId::new(987654321))
        );
        assert_eq!(extract_author_id("Normal mesaj"), None);
    }

    #[test]
    fn test_should_trigger_ai_chat() {
        let bot_id = UserId::new(100);
        let user_id = UserId::new(200);

        let link_conversion_content =
            "<@200>[:](https://vxtwitter.com/u/123) [**Twitter**](<https://x.com/u/123>)";
        let new_link_conversion_content = "<@200> [Twitter](https://x.com/u/123)";
        let ai_response_content = "Sen Botgek'e ne dedin öyle?";

        // 1. Link dönüştürme mesajına verilen yanıt (Discord mention'ı açık olsa bile ASLA tetiklenmemeli)
        assert!(!should_trigger_ai_chat(
            true, // Discord reply otomatik bot mention eklese bile
            Some(bot_id),
            Some(link_conversion_content),
            false,
            bot_id
        ));
        assert!(!should_trigger_ai_chat(
            true,
            Some(bot_id),
            Some(new_link_conversion_content),
            false,
            bot_id
        ));

        // 1b. Taklit (webhook) dönüştürme mesajına yanıt: yazar bot değil, kayıt bilgisi ile engellenir
        assert!(!should_trigger_ai_chat(
            true,
            Some(user_id),
            Some(new_link_conversion_content),
            true,
            bot_id
        ));

        // 2. Botun AI yanıtına verilen yanıt (tetiklenmeli)
        assert!(should_trigger_ai_chat(
            true,
            Some(bot_id),
            Some(ai_response_content),
            false,
            bot_id
        ));
        assert!(should_trigger_ai_chat(
            false, // Mention toggle kapalı yanıt verilse bile tetiklenmeli
            Some(bot_id),
            Some(ai_response_content),
            false,
            bot_id
        ));

        // 3. Direkt bot etiketleme (yanıt yok, mention var -> tetiklenmeli)
        assert!(should_trigger_ai_chat(true, None, None, false, bot_id));

        // 4. Başka bir kullanıcının mesajına yanıt verip botu etiketleme (tetiklenmeli)
        assert!(should_trigger_ai_chat(
            true,
            Some(user_id),
            Some("Buna baksana"),
            false,
            bot_id
        ));

        // 5. Normal mesaj (etiket yok, yanıt yok -> tetiklenmemeli)
        assert!(!should_trigger_ai_chat(false, None, None, false, bot_id));
    }

    #[test]
    fn bot_conversion_policy_requires_opt_in_supported_plain_input() {
        assert!(has_supported_bot_input("https://x.com/user/status/1"));
        assert!(has_supported_bot_input("https://clips.twitch.tv/Ab_C-123"));
        assert!(has_supported_bot_input(
            "https://www.pixiv.net/en/artworks/12345"
        ));
        assert!(has_supported_bot_input(
            "https://artist.tumblr.com/post/987654321"
        ));
        assert!(has_supported_bot_input(
            "https://www.facebook.com/example/posts/123456"
        ));
        assert!(has_supported_bot_input(
            "https://www.youtube.com/post/UCabc_123"
        ));
        assert!(has_supported_bot_input(
            "<@1> [Twitter](https://x.com/user/status/1)"
        ));
        assert!(!has_supported_bot_input("hello from an integration"));
    }

    #[tokio::test]
    async fn bot_duplicate_guard_is_bounded_and_idempotent() {
        let id = MessageId::new(987654321);
        assert!(claim_bot_source(id).await);
        assert!(!claim_bot_source(id).await);
    }

    /// Kart imzası → AI kararı entegrasyonu: modern kartta mesaj içeriği boştur,
    /// bu yüzden yalnızca metin kontrolü yeterli değildir.
    #[test]
    fn replies_to_native_cards_do_not_trigger_the_ai() {
        let bot_id = UserId::new(100);

        // Modern kart (Components V2): içerik ve embed yok → yapısal imza ile tanınır
        let modern = crate::converters::builtin::has_card_signature(bot_id, false, "", &[], bot_id);
        assert!(modern, "içeriksiz modern kart tanınmalı");
        assert!(
            !should_trigger_ai_chat(true, Some(bot_id), Some(""), modern, bot_id),
            "modern karta verilen yanıt AI'yı tetiklememeli"
        );

        // Botun AI yanıtı (içerikli) kart değildir: yanıtlanınca AI tetiklenmeli
        let ai_reply = crate::converters::builtin::has_card_signature(
            bot_id,
            false,
            "Evet, haklısın.",
            &[],
            bot_id,
        );
        assert!(!ai_reply);
        assert!(
            should_trigger_ai_chat(
                true,
                Some(bot_id),
                Some("Evet, haklısın."),
                ai_reply,
                bot_id
            ),
            "AI yanıtına verilen yanıt AI'yı tetiklemeli"
        );
    }
}
