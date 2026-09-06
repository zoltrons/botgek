pub mod parser;
pub mod translation;

use serenity::all::{Context, CreateMessage, Message, ReactionType};
use tracing::{error, info};

use crate::config::Config;
use crate::settings::{AppSettings, FilterMode};
use crate::utils::mentions::build_selective_allowed_mentions;

/// Kullanıcının mesajındaki desteklenen sosyal medya linkini tespit eder ve dönüştürür.
/// Format: <@User>[:](fixedLink) [**Platform**](<origLink>)\n> Alıntı
/// Silme işlemi için kompakt 🗑️ reaksiyonu eklenir.
pub async fn handle_message(ctx: &Context, msg: &Message, config: &Config, settings: &AppSettings) {
    let parsed = match parser::parse_message(&msg.content, config) {
        Some(p) => p,
        None => return, // Desteklenen bir bağlantı bulunamadı
    };

    // 1. Modül Etkinlik Kontrolü
    let module_enabled = match parsed.platform {
        parser::Platform::Twitter => settings.modules.twitter,
        parser::Platform::TikTok => settings.modules.tiktok,
        parser::Platform::Instagram => settings.modules.instagram,
        parser::Platform::Reddit => settings.modules.reddit,
        parser::Platform::Bluesky => settings.modules.bluesky,
    };

    if !module_enabled {
        return;
    }

    // 2. Rol Tabanlı Allowlist / Blacklist Filtresi
    if settings.role_filter.mode != FilterMode::Disabled && !settings.role_filter.roles.is_empty() {
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

        let has_matching_role = user_roles.iter().any(|r| settings.role_filter.roles.contains(r));

        match settings.role_filter.mode {
            FilterMode::Blacklist => {
                if has_matching_role {
                    info!("Kullanıcı kara listede (Blacklist rolü var), dönüştürme atlandı: {}", msg.author.id);
                    return;
                }
            }
            FilterMode::Allowlist => {
                if !has_matching_role {
                    info!("Kullanıcı beyaz listede değil (Allowlist rolü yok), dönüştürme atlandı: {}", msg.author.id);
                    return;
                }
            }
            FilterMode::Disabled => {}
        }
    }

    // 3. Embed Formatı (Spoiler varsa gizle)
    let embed_part = if parsed.is_spoiler {
        format!("||[:]({})||", parsed.fixed_url)
    } else {
        format!("[:]({})", parsed.fixed_url)
    };


    // 2. Alıntı Formatı (Varsa)
    let comment_part = match &parsed.user_comment {
        Some(comment) => format!("\n> {}", comment),
        None => String::new(),
    };

    // 3. Mesaj İçeriği
    let content = format!(
        "<@{}>{} [**{}**](<{}>){}",
        msg.author.id,
        embed_part,
        parsed.platform.display_name(),
        parsed.original_url,
        comment_part
    );

    let allowed_mentions = build_selective_allowed_mentions(msg);

    let mut builder = CreateMessage::new()
        .content(content)
        .allowed_mentions(allowed_mentions);

    // Eğer orijinal mesaj bir yanıtsa, bot da aynı mesaja yanıt versin
    if let Some(ref message_ref) = msg.message_reference {
        builder = builder.reference_message(message_ref.clone());
    }

    // 4. Mesajı Gönder
    let sent_msg = match msg.channel_id.send_message(&ctx.http, builder).await {
        Ok(m) => m,
        Err(e) => {
            error!("Mesaj gönderilirken hata oluştu: {:?}", e);
            return;
        }
    };
    info!(
        "Mesaj başarıyla dönüştürüldü ({:?}): {}",
        parsed.platform, parsed.original_url
    );

    // 5. Kompakt 🗑️ reaksiyonu ekle
    if let Err(e) = sent_msg
        .react(&ctx.http, ReactionType::Unicode("🗑️".to_string()))
        .await
    {
        error!("Silme reaksiyonu eklenemedi: {:?}", e);
    }

    // 6. Eski Mesajı Sil
    if config.delete_original {
        if let Err(e) = msg.delete(&ctx.http).await {
            error!("Orijinal mesaj silinirken hata oluştu: {:?}", e);
        }
    }
}

