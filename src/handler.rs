use serenity::all::{
    async_trait, Context, CreateMessage, EditMessage, EventHandler, Message, Permissions,
    Reaction, ReactionType, Ready, UserId,
};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

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
        let current_settings = {
            self.settings.read().await.clone()
        };

        // 1. Sosyal Medya Link Dönüştürme Kontrolü
        converters::handle_message(&ctx, &msg, &self.config, &current_settings).await;

        // 2. Bot Etiketleme (@botgekv2) veya Yanıt (Reply) ile AI Sohbet Kontrolü
        let is_mentioned = msg.mentions.iter().any(|u| u.id == current_user_id);
        let is_reply_to_bot = if let Some(ref ref_msg) = msg.referenced_message {
            ref_msg.author.id == current_user_id
        } else if let Some(ref msg_ref) = msg.message_reference {
            if let Some(msg_id) = msg_ref.message_id {
                if let Ok(m) = msg.channel_id.message(&ctx.http, msg_id).await {
                    m.author.id == current_user_id
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
            if let Some(ref api_key) = self.config.openrouter_api_key {
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
                            let builder = CreateMessage::new()
                                .content(reply)
                                .reference_message(&msg);
                            if let Err(e) = msg.channel_id.send_message(&ctx.http, builder).await {
                                error!("AI yanıtı Discord'a gönderilemedi: {:?}", e);
                            }
                        }
                        Err(e) => {
                            error!("AI yanıtı üretilirken hata oluştu: {:?}", e);
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
        let translation_enabled = {
            self.settings.read().await.modules.translation
        };

        if !translation_enabled {
            return;
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

/// Mesaj başındaki <@123456789> formatından kullanıcı ID'sini çıkarır
fn extract_author_id(content: &str) -> Option<UserId> {
    if let Some(rest) = content.strip_prefix("<@") {
        if let Some(end) = rest.find('>') {
            return rest[..end].parse::<u64>().ok().map(UserId::new);
        }
    }
    None
}


