use serenity::all::{
    Context, CreateAllowedMentions, CreateMessage, EditMessage, EventHandler, Message, Permissions,
    Reaction, ReactionType, Ready, UserId, async_trait,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};
use tracing::{error, info, warn};

static AI_COOLDOWNS: LazyLock<Mutex<HashMap<UserId, Instant>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

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
    async fn ready(&self, _ctx: Context, ready: Ready) {
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
    }

    async fn message(&self, ctx: Context, msg: Message) {
        // Botların kendi mesajlarını ve diğer botları yok say
        if msg.author.bot {
            return;
        }

        let current_user_id = ctx.cache.current_user().id;

        // Kendi dönüştürme kartlarımız (bot adına veya taklit webhook'u ile gönderilenler)
        // gelen mesaj olarak işlenmez: modern kartın içeriği boştur ama kartın metni
        // (kullanıcının yazdığı not) bota etiket içerebilir; bu etiket AI sohbetini
        // tetiklememeli. Kartlara verilen yanıtlar ayrı mesajlardır.
        if crate::converters::builtin::is_card_message(&msg, current_user_id).await {
            return;
        }

        let current_settings = self.settings.read().await.get_guild_settings(msg.guild_id);

        // 1. Sosyal Medya Link Dönüştürme Kontrolü
        converters::handle_message(
            &ctx,
            &msg,
            &self.http_client,
            &self.config,
            &current_settings,
        )
        .await;

        // 2. Bot Etiketleme (@botgekv2) veya Yanıt (Reply) ile AI Sohbet Kontrolü
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
                                .reference_message(&msg)
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
                            let builder =
                                CreateMessage::new().content(notice).reference_message(&msg);
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

            let is_admin_or_mod = if let Some(guild_id) = reaction.guild_id {
                match guild_id.member(&ctx.http, user_id).await {
                    Ok(member) => {
                        match ctx.cache.guild(guild_id) {
                            Some(guild) => {
                                // Discord kanal izin ezmelerini (channel overwrites) hesaba katan kanonik izin kontrolü
                                if let Some(channel) = guild.channels.get(&reaction.channel_id) {
                                    let perms = guild.user_permissions_in(channel, &member);
                                    perms.contains(Permissions::MANAGE_MESSAGES)
                                        || perms.contains(Permissions::ADMINISTRATOR)
                                } else {
                                    #[allow(deprecated)]
                                    member
                                        .permissions(&ctx.cache)
                                        .map(|perms| {
                                            perms.contains(Permissions::MANAGE_MESSAGES)
                                                || perms.contains(Permissions::ADMINISTRATOR)
                                        })
                                        .unwrap_or(false)
                                }
                            }
                            _ =>
                            {
                                #[allow(deprecated)]
                                member
                                    .permissions(&ctx.cache)
                                    .map(|perms| {
                                        perms.contains(Permissions::MANAGE_MESSAGES)
                                            || perms.contains(Permissions::ADMINISTRATOR)
                                    })
                                    .unwrap_or(false)
                            }
                        }
                    }
                    _ => false,
                }
            } else {
                false
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
        let guild_settings = self
            .settings
            .read()
            .await
            .get_guild_settings(reaction.guild_id);
        if !guild_settings.modules.translation {
            return;
        }

        // Modüle özel rol kontrolü (translation)
        let role_filter = guild_settings.get_role_filter("translation");
        if !crate::policy::allows(&ctx, reaction.guild_id, user_id, None, role_filter).await {
            return;
        }

        if let Some(lang_code) = translation::emoji_to_lang_code(&reaction.emoji) {
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

        let settings = self
            .settings
            .read()
            .await
            .get_guild_settings(reaction.guild_id);
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
