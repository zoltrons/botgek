use serenity::all::{
    async_trait, Context, CreateAllowedMentions, CreateMessage, EditMessage, EventHandler, Message, Permissions,
    Reaction, ReactionType, Ready, UserId,
};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};
use tracing::{error, info, warn};

static AI_COOLDOWNS: Lazy<Mutex<HashMap<UserId, Instant>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

use crate::config::Config;
use crate::converters;
use crate::converters::translation;
use crate::settings::AppSettings;

pub struct Handler {
    pub config: Config,
    pub settings: Arc<RwLock<AppSettings>>,
    pub http_client: reqwest::Client,
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, _ctx: Context, ready: Ready) {
        info!("===========================================");
        info!("Botgek başarıyla bağlandı!");
        info!("Giriş yapılan hesap: {}", ready.user.tag());
        info!("WebUI: http://localhost:{}", self.config.web_port);
        info!("Twitter Alan Adı: {}", self.config.twitter_domain);
        info!("TikTok Alan Adı: {}", self.config.tiktok_domain);
        info!("Instagram Alan Adı: {}", self.config.instagram_domain);
        info!("Reddit Alan Adı: {}", self.config.reddit_domain);
        info!("Bluesky Alan Adı: {}", self.config.bluesky_domain);
        info!("OpenRouter Entegrasyonu: {}", if self.config.openrouter_api_key.is_some() { "Aktif" } else { "Devre Dışı (API Key yok)" });
        info!("Orijinal Mesajı Silme: {}", self.config.delete_original);
        info!("===========================================");
    }

    async fn message(&self, ctx: Context, msg: Message) {
        // Botların kendi mesajlarını ve diğer botları yok say
        if msg.author.bot {
            return;
        }

        let current_user_id = ctx.cache.current_user().id;
        let current_settings = self.settings.read().await.get_guild_settings(msg.guild_id);

        // 1. Sosyal Medya Link Dönüştürme Kontrolü
        converters::handle_message(&ctx, &msg, &self.config, &current_settings).await;

        // 2. Bot Etiketleme (@botgekv2) veya Yanıt (Reply) ile AI Sohbet Kontrolü
        let is_mentioned = msg.mentions.iter().any(|u| u.id == current_user_id);
        let is_reply_to_bot = if let Some(ref ref_msg) = msg.referenced_message {
            if ref_msg.author.id == current_user_id {
                // Link dönüştürme mesajlarına verilen yanıtlarda AI tetikleme
                !is_link_conversion_message(&ref_msg.content)
            } else {
                false
            }
        } else if let Some(ref msg_ref) = msg.message_reference {
            if let Some(msg_id) = msg_ref.message_id {
                if let Ok(m) = msg.channel_id.message(&ctx.http, msg_id).await {
                    if m.author.id == current_user_id {
                        !is_link_conversion_message(&m.content)
                    } else {
                        false
                    }
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };

        if (is_mentioned || is_reply_to_bot) && current_settings.modules.ai_chat {
            // Modüle özel rol kontrolü (ai_chat)
            let role_filter = current_settings.get_role_filter("ai_chat");
            if role_filter.mode != crate::settings::FilterMode::Disabled && !role_filter.roles.is_empty() {
                let user_roles: Vec<String> = if let Some(ref member) = msg.member {
                    member.roles.iter().map(|r| r.get().to_string()).collect()
                } else if let Some(guild_id) = msg.guild_id {
                    if let Ok(member) = guild_id.member(&ctx.http, msg.author.id).await {
                        member.roles.iter().map(|r| r.get().to_string()).collect()
                    } else {
                        Vec::new()
                    }
                } else {
                    Vec::new()
                };

                let has_matching_role = user_roles.iter().any(|r| role_filter.roles.contains(r));
                match role_filter.mode {
                    crate::settings::FilterMode::Blacklist => {
                        if has_matching_role {
                            warn!("Kullanıcı AI chat modülü kara listesinde, atlandı: {}", msg.author.id);
                            return;
                        }
                    }
                    crate::settings::FilterMode::Allowlist => {
                        if !has_matching_role {
                            warn!("Kullanıcı AI chat modülü beyaz listesinde değil, atlandı: {}", msg.author.id);
                            return;
                        }
                    }
                    crate::settings::FilterMode::Disabled => {}
                }
            }
            if let Some(ref api_key) = self.config.openrouter_api_key {
                // Hız sınırı (Rate limiting): Kullanıcı başına 3 saniye bekleme süresi
                {
                    let mut cooldowns = AI_COOLDOWNS.lock().await;
                    cooldowns.retain(|_, instant| instant.elapsed() < Duration::from_secs(10));
                    if let Some(last_time) = cooldowns.get(&msg.author.id) {
                        if last_time.elapsed() < Duration::from_secs(3) {
                            warn!("AI hız sınırı aşıldı, istek atlandı: {}", msg.author.id);
                            return;
                        }
                    }
                    cooldowns.insert(msg.author.id, Instant::now());
                }

                let clean_query = msg.content
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
                                .content(safe_reply)
                                .reference_message(&msg)
                                .allowed_mentions(allowed_mentions);

                            if let Err(e) = msg.channel_id.send_message(&ctx.http, builder).await {
                                error!("AI yanıtı Discord'a gönderilemedi: {:?}", e);
                            }
                        }
                        Err(e) => {
                            error!("AI yanıtı üretilirken hata oluştu: {:?}", e);
                            let notice = "Bu model şu an yanıt veremedi. Model slug'ını kontrol et veya Auto Free Router'a dön.";
                            let builder = CreateMessage::new()
                                .content(notice)
                                .reference_message(&msg);
                            if let Err(send_err) = msg.channel_id.send_message(&ctx.http, builder).await {
                                error!("AI hata bildirimi gönderilemedi: {:?}", send_err);
                            }
                        }
                    }
                }
            } else {
                info!("Bot etiketlendi veya yanıtlandı fakat OPENROUTER_API_KEY .env dosyasında tanımlı değil.");
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

            let original_author_id = extract_author_id(&target_msg.content);
            let is_author = original_author_id == Some(user_id);

            let is_admin_or_mod = if let Some(guild_id) = reaction.guild_id {
                if let Ok(member) = guild_id.member(&ctx.http, user_id).await {
                    #[allow(deprecated)]
                    member
                        .permissions(&ctx.cache)
                        .map(|perms| {
                            perms.contains(Permissions::MANAGE_MESSAGES)
                                || perms.contains(Permissions::ADMINISTRATOR)
                        })
                        .unwrap_or(false)
                } else {
                    false
                }
            } else {
                false
            };

            if is_author || is_admin_or_mod {
                info!(
                    "Reaksiyon ile mesaj silme onaylandı. Silen: {} (Orijinal Yazar: {:?})",
                    user_id, original_author_id
                );
                if let Err(e) = target_msg.delete(&ctx.http).await {
                    error!("Mesaj silinirken hata oluştu: {:?}", e);
                }
            } else {
                warn!("Yetkisiz silme reaksiyonu! Tıklayan: {}", user_id);
                let _ = reaction.delete(&ctx.http).await;
            }
            return;
        }

        // 2. Bayrak Emojisi ile Çeviri (🇹🇷, 🇬🇧, 🇩🇪 vb.)
        let guild_settings = self.settings.read().await.get_guild_settings(reaction.guild_id);
        if !guild_settings.modules.translation {
            return;
        }

        // Modüle özel rol kontrolü (translation)
        let role_filter = guild_settings.get_role_filter("translation");
        if role_filter.mode != crate::settings::FilterMode::Disabled && !role_filter.roles.is_empty() {
            let user_roles: Vec<String> = if let Some(guild_id) = reaction.guild_id {
                if let Ok(member) = guild_id.member(&ctx.http, user_id).await {
                    member.roles.iter().map(|r| r.get().to_string()).collect()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };

            let has_matching_role = user_roles.iter().any(|r| role_filter.roles.contains(r));
            match role_filter.mode {
                crate::settings::FilterMode::Blacklist => {
                    if has_matching_role {
                        warn!("Kullanıcı çeviri modülü kara listesinde, reaksiyon atlandı: {}", user_id);
                        return;
                    }
                }
                crate::settings::FilterMode::Allowlist => {
                    if !has_matching_role {
                        warn!("Kullanıcı çeviri modülü beyaz listesinde değil, reaksiyon atlandı: {}", user_id);
                        return;
                    }
                }
                crate::settings::FilterMode::Disabled => {}
            }
        }

        if let Some(lang_code) = translation::emoji_to_lang_code(&reaction.emoji) {
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

            if let Some(new_content) = translation::update_link_translation(&target_msg.content, Some(lang_code)) {
                let edit = EditMessage::new().content(new_content);
                if let Err(e) = target_msg.edit(&ctx.http, edit).await {
                    error!("Mesaj çevirisi güncellenemedi: {:?}", e);
                } else {
                    info!(
                        "Mesaj çevirisi başarıyla güncellendi: {} (Dil: {})",
                        target_msg.id, lang_code
                    );
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
            if r.count > 0 {
                if let Some(code) = translation::emoji_to_lang_code(&r.reaction_type) {
                    remaining_lang = Some(code);
                    break;
                }
            }
        }

        if let Some(new_content) = translation::update_link_translation(&target_msg.content, remaining_lang) {
            let edit = EditMessage::new().content(new_content);
            if let Err(e) = target_msg.edit(&ctx.http, edit).await {
                error!("Mesaj çevirisi geri alınamadı: {:?}", e);
            } else {
                info!(
                    "Mesaj çevirisi güncellendi (kaldırma sonrası): {} (Aktif dil: {:?})",
                    target_msg.id, remaining_lang
                );
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

/// Link dönüştürme mesajlarını tespit eder.
/// Format: `<@userid>[:](url) [**Platform**](<url>)` şeklindeki mesajlar.
/// Bu mesajlara gelen yanıtlarda AI sohbet tetiklenmemeli.
fn is_link_conversion_message(content: &str) -> bool {
    content.starts_with("<@") && content.contains("[:](")
}

/// LLM veya kullanıcı girdilerinden kaynaklanan @everyone, @here veya rol pingleme risklerini
/// sıfır genişlikli boşluk (zero-width space, U+200B) enjekte ederek tamamen etkisiz hale getirir.
/// Bu sayede Discord mesajı metin olarak gösterse bile asla mention olarak algılamaz ve bildirim tetiklemez.
pub fn sanitize_mentions(text: &str) -> String {
    static RE_EVERYONE: Lazy<regex::Regex> = Lazy::new(|| regex::Regex::new(r"(?i)@\s*everyone").unwrap());
    static RE_HERE: Lazy<regex::Regex> = Lazy::new(|| regex::Regex::new(r"(?i)@\s*here").unwrap());
    static RE_ROLES: Lazy<regex::Regex> = Lazy::new(|| regex::Regex::new(r"<@&(\d+)>").unwrap());

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
        assert!(is_link_conversion_message("<@123456789>[:](https://vxtwitter.com/user/status/123) [**Twitter**](<https://twitter.com/user/status/123>)"));
        assert!(is_link_conversion_message("<@999>[:](https://vxtiktok.com/t/abc) [**TikTok**](<https://tiktok.com/t/abc>)"));

        // AI sohbet yanıtları veya normal mesajlar (AI tetiklenmeli)
        assert!(!is_link_conversion_message("Evet, haklısın bu konuda."));
        assert!(!is_link_conversion_message("Selam! Ben Botgek."));
        assert!(!is_link_conversion_message(""));
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
}


