use serenity::all::{
    Channel, ChannelId, ChannelType, Context, CreateAllowedMentions, CreateMessage, EventHandler,
    GuildId, Interaction, Message, MessageId, MessageReference, MessageType, MessageUpdateEvent,
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

/// Aynı kullanıcının iki AI isteği arasındaki en kısa süre.
const AI_USER_COOLDOWN: Duration = Duration::from_secs(3);
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
use crate::settings::{AppSettings, GuildSettings};
use crate::utils::{DISCORD_CONTENT_LIMIT, truncate_chars};

pub struct Handler {
    pub config: Config,
    pub settings: Arc<RwLock<AppSettings>>,
    pub http_client: reqwest::Client,
}

#[async_trait]
impl EventHandler for Handler {
    async fn shard_stage_update(&self, _ctx: Context, event: serenity::all::ShardStageUpdateEvent) {
        crate::telemetry::gateway_event(crate::telemetry::GatewayEvent::ShardStage);
        crate::telemetry::DISCORD_READY.store(
            event.new == serenity::gateway::ConnectionStage::Connected,
            std::sync::atomic::Ordering::Relaxed,
        );
    }
    async fn ready(&self, ctx: Context, ready: Ready) {
        crate::telemetry::gateway_event(crate::telemetry::GatewayEvent::Ready);
        crate::telemetry::DISCORD_READY.store(true, std::sync::atomic::Ordering::Relaxed);
        info!("===========================================");
        info!("Botgek başarıyla bağlandı!");
        info!("Giriş yapılan hesap: {}", ready.user.tag());
        info!("WebUI: http://localhost:{}", self.config.web_port);
        info!("Dönüştürme: tüm platformlar native kart (proxy yok)");
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
        let dev_guild = self.config.dev_guild_id;
        tokio::spawn(async move { crate::commands::register(http, dev_guild).await });
    }

    /// Slash ve sağ tık (mesaj) komutları.
    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        crate::telemetry::gateway_event(crate::telemetry::GatewayEvent::Interaction);
        // Dönüşüm veri çekme + kart gönderimi nedeniyle uzun sürebilir; etkileşim hemen
        // ertelenir (komut tarafında), bu yüzden olay işleyicisi ayrı görevde çalıştırılır.
        // `commands::handle` işleyiciyi kendi görevinde koşturur ve paniği yakalar.
        tokio::spawn(crate::commands::handle(
            ctx,
            interaction,
            self.http_client.clone(),
            self.config.clone(),
            self.settings.clone(),
        ));
    }

    async fn message(&self, ctx: Context, msg: Message) {
        crate::telemetry::gateway_event(crate::telemetry::GatewayEvent::Message);
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
        if current_settings.modules.ai_chat {
            self.reply_with_ai(&ctx, &msg, current_user_id, &current_settings)
                .await;
        }
    }

    /// Kaynak mesaj silinirse ondan üretilen kart da silinir.
    async fn message_delete(
        &self,
        ctx: Context,
        _channel_id: ChannelId,
        deleted_message_id: MessageId,
        _guild_id: Option<GuildId>,
    ) {
        crate::telemetry::gateway_event(crate::telemetry::GatewayEvent::MessageDelete);
        converters::tracking::source_deleted(&ctx, deleted_message_id).await;
    }

    async fn message_delete_bulk(
        &self,
        ctx: Context,
        _channel_id: ChannelId,
        multiple_deleted_messages_ids: Vec<MessageId>,
        _guild_id: Option<GuildId>,
    ) {
        crate::telemetry::gateway_event(crate::telemetry::GatewayEvent::MessageDelete);
        converters::tracking::sources_deleted(&ctx, &multiple_deleted_messages_ids).await;
    }

    /// Kaynak mesaj düzenlenirse kart bağlantıya göre güncellenir (bkz. `converters::tracking`).
    async fn message_update(
        &self,
        ctx: Context,
        _old_if_available: Option<Message>,
        _new: Option<Message>,
        event: MessageUpdateEvent,
    ) {
        crate::telemetry::gateway_event(crate::telemetry::GatewayEvent::MessageUpdate);
        converters::tracking::source_edited(
            &ctx,
            &event,
            &self.http_client,
            &self.config,
            &self.settings,
        )
        .await;
    }

    /// Reaksiyon ekleme dinleyicisi: 🗑️ ile kartı silme (orijinal yazar veya yönetici/moderatör).
    async fn reaction_add(&self, ctx: Context, reaction: Reaction) {
        crate::telemetry::gateway_event(crate::telemetry::GatewayEvent::Reaction);
        let current_user_id = ctx.cache.current_user().id;

        // Botun kendi eklediği reaksiyonları yok say
        let user_id = match reaction.user_id {
            Some(id) if id != current_user_id => id,
            _ => return,
        };

        // Silme İşlemi (🗑️)
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

                    // Baştaki `<@id>` yalnızca proxy dönüşüm mesajında paylaşanı gösterir. AI
                    // yanıtları gibi diğer bot mesajlarının içeriğini model/kullanıcı belirler;
                    // oradaki bir etiket kimseye silme yetkisi vermemeli.
                    if is_link_conversion_message(&target_msg.content) {
                        extract_author_id(&target_msg.content)
                    } else {
                        None
                    }
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
                let delete_started = std::time::Instant::now();
                let deleted = reaction
                    .channel_id
                    .delete_message(&ctx.http, reaction.message_id)
                    .await;
                crate::telemetry::record_phase(
                    crate::telemetry::Phase::Delete,
                    delete_started.elapsed(),
                );
                match deleted {
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
        }
    }
}

impl Handler {
    /// Bot etiketlendiğinde veya botun dönüşüm kartı olmayan bir mesajına yanıt verildiğinde
    /// AI sohbet yanıtı üretir. Ucuz kontroller (rol, boş istem, yuva) kullanıcı bekleme
    /// süresini ve guild/global bütçeyi tüketmeden önce yapılır.
    async fn reply_with_ai(
        &self,
        ctx: &Context,
        msg: &Message,
        bot_id: UserId,
        settings: &GuildSettings,
    ) {
        let referenced = referenced_message_info(ctx, msg, bot_id).await;
        // Eğlence komutlarının yanıtlarına (uyum kartı, düello, 8top...) verilen yanıtlar
        // sohbet değildir; Discord yanıtı botu etiketlese bile AI çalışmaz.
        if referenced.as_ref().is_some_and(|r| r.command_response) {
            return;
        }
        let should_trigger = should_trigger_ai_chat(
            msg.mentions.iter().any(|u| u.id == bot_id),
            referenced.as_ref().map(|r| r.message.author.id),
            referenced.as_ref().map(|r| r.message.content.as_str()),
            referenced.as_ref().is_some_and(|r| r.converted),
            bot_id,
        );
        if !should_trigger {
            return;
        }
        if !crate::policy::allows(
            ctx,
            msg.guild_id,
            msg.author.id,
            msg.member.as_ref().map(|m| m.roles.as_slice()),
            settings.get_role_filter("ai_chat"),
        )
        .await
        {
            return;
        }
        let Some(api_key) = self.config.openrouter_api_key.as_deref() else {
            info!(
                "Bot etiketlendi veya yanıtlandı fakat OPENROUTER_API_KEY .env dosyasında tanımlı değil."
            );
            return;
        };

        // Boş etiket ancak yanıtlanan bir mesaj ya da görsel varsa anlamlıdır ("@bot" + yanıt
        // = "buna ne dersin?"). Bağlamın kendisi bütçe kontrollerinden sonra toplanır.
        let bare_mention = msg
            .content
            .replace(&format!("<@{bot_id}>"), "")
            .replace(&format!("<@!{bot_id}>"), "")
            .trim()
            .is_empty();
        if bare_mention && referenced.is_none() && !crate::ai::discord::has_images(msg) {
            return;
        }
        if !crate::ai::has_capacity() {
            warn!(
                "AI sağlayıcı yuvaları dolu, istek atlandı: {}",
                msg.author.id
            );
            return;
        }
        if !claim_ai_cooldown(msg.author.id).await {
            warn!("AI hız sınırı aşıldı, istek atlandı: {}", msg.author.id);
            return;
        }
        if !consume_ai_budget(msg.guild_id).await {
            warn!(
                guild_id = ?msg.guild_id,
                "AI guild/global bütçesi dolu, istek atlandı"
            );
            return;
        }

        // Discord "Yazıyor..." göstergesi
        let _ = msg.channel_id.broadcast_typing(&ctx.http).await;

        // Yanıt zinciri, yanıtlanan mesaj, görseller ve sunucu emojileri; kimlikler takma ada
        // çevrilir (bkz. `ai::discord`).
        let emojis = crate::ai::discord::GuildEmojis::from_cache(ctx, msg.guild_id);
        let Some(gathered) = crate::ai::discord::gather(
            ctx,
            &self.http_client,
            msg,
            referenced.as_ref().map(|r| &r.message),
            bot_id,
            &emojis,
        )
        .await
        else {
            return;
        };
        let prompt = crate::ai::ChatPrompt {
            model: &settings.ai.model,
            system_prompt: &settings.ai.system_prompt,
            user_prompt: &gathered.query,
            conversation: &gathered.conversation,
        };
        let content = match crate::ai::query_openrouter(&self.http_client, api_key, &prompt).await {
            Ok(reply) => truncate_chars(
                &sanitize_mentions(&emojis.render(&reply)),
                DISCORD_CONTENT_LIMIT,
            ),
            Err(e) => {
                // Ayrıntı yalnızca log'a yazılır: sunucu yapılandırmasına (.env, API anahtarı)
                // dair bilgi Discord kullanıcılarına sızdırılmaz.
                error!("AI yanıtı üretilirken hata oluştu: {e}");
                ai_error_notice(&e).to_string()
            }
        };
        // Güvenlik: LLM çıktısındaki @everyone, rol veya kullanıcı etiketleri bildirim tetiklemez.
        let allowed_mentions = CreateAllowedMentions::new()
            .empty_roles()
            .empty_users()
            .everyone(false)
            .replied_user(true);
        let builder = CreateMessage::new()
            .content(content)
            .reference_message(soft_reply(msg))
            .allowed_mentions(allowed_mentions);
        if let Err(e) = msg.channel_id.send_message(&ctx.http, builder).await {
            error!("AI yanıtı Discord'a gönderilemedi: {:?}", e);
        }
    }
}

/// Yanıtlanan mesaj ve AI tetikleme kararı için ondan çıkarılan bilgiler.
struct ReferencedMessage {
    message: Message,
    converted: bool,
    command_response: bool,
}

/// Yanıtlanan mesajı gateway yükünden, yoksa REST'ten okur.
async fn referenced_message_info(
    ctx: &Context,
    msg: &Message,
    bot_id: UserId,
) -> Option<ReferencedMessage> {
    let fetched;
    let referenced = match msg.referenced_message.as_deref() {
        Some(referenced) => referenced,
        None => {
            let id = msg.message_reference.as_ref()?.message_id?;
            fetched = msg.channel_id.message(&ctx.http, id).await.ok()?;
            &fetched
        }
    };
    Some(ReferencedMessage {
        converted: is_converted_message(referenced, bot_id).await,
        command_response: referenced.author.id == bot_id
            && referenced.interaction_metadata.is_some(),
        message: referenced.clone(),
    })
}

/// Kullanıcı başına AI bekleme süresini talep eder; süre dolmadıysa `false`.
async fn claim_ai_cooldown(user: UserId) -> bool {
    let mut cooldowns = AI_COOLDOWNS.lock().await;
    cooldowns.retain(|_, at| at.elapsed() < AI_USER_COOLDOWN);
    if cooldowns.contains_key(&user) {
        return false;
    }
    cooldowns.insert(user, Instant::now());
    true
}

/// Sağlayıcı hatasını kullanıcıya gösterilecek, sır içermeyen bildirime çevirir.
fn ai_error_notice(error: &crate::ai::AiError) -> &'static str {
    use crate::ai::AiError;
    match error {
        AiError::Busy | AiError::RateLimited(_) => {
            "Model sağlayıcısı şu an yoğunluk veya hız sınırı nedeniyle yanıt veremedi. Birkaç saniye sonra tekrar dene."
        }
        AiError::Auth(_)
        | AiError::Credits(_)
        | AiError::BadRequest(_)
        | AiError::InvalidModel(_) => {
            "AI servisinin yapılandırmasında bir sorun var; bot sahibinin bakması gerekiyor."
        }
        AiError::Blocked(_) => {
            "Bu mesaja yanıt veremiyorum; model sağlayıcısının içerik filtresine takıldı."
        }
        AiError::Truncated => {
            "Model uzun uzun düşündü ama bir cevaba varamadı 🤔 Soruyu biraz daha kısa sorar mısın?"
        }
        AiError::NoProvider(_) => {
            "Gizlilik kuralıma (veri saklamayan sağlayıcı) uyan bir model şu an bulunamadı. Biraz sonra tekrar dene."
        }
        AiError::Network(_)
        | AiError::Timeout
        | AiError::Unavailable(_)
        | AiError::Empty
        | AiError::Malformed(_) => {
            "AI servisi şu an geçici olarak yanıt veremedi. Biraz sonra tekrar dene."
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
            "letterboxd.com/",
            "boxd.it/",
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

    #[tokio::test]
    async fn ai_cooldown_blocks_only_the_same_user() {
        let user = UserId::new(9_000_000_000_000_001);
        assert!(claim_ai_cooldown(user).await);
        assert!(!claim_ai_cooldown(user).await);
        assert!(claim_ai_cooldown(UserId::new(9_000_000_000_000_002)).await);
    }

    #[test]
    fn ai_error_notice_never_leaks_provider_details() {
        use crate::ai::AiError;
        let notice = ai_error_notice(&AiError::Auth("invalid API key sk-or-123".into()));
        assert!(!notice.contains("sk-or") && !notice.contains("401"));
        let notice = ai_error_notice(&AiError::Credits("Add credits at openrouter.ai".into()));
        assert!(!notice.contains("openrouter") && !notice.contains("402"));
        assert!(ai_error_notice(&AiError::RateLimited(String::new())).contains("hız sınırı"));
    }

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
