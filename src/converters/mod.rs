pub mod builtin;
pub mod parser;
pub mod translation;

use serenity::all::{Context, CreateMessage, Message, ReactionType};
use tracing::{error, info, warn};

use crate::config::Config;
use crate::settings::{ConversionMode, EngineSettings, FilterMode, GuildSettings};
use crate::utils::mentions::build_selective_allowed_mentions;
use crate::utils::{truncate_chars, DISCORD_CONTENT_LIMIT};

/// Ayarlardaki alan adı override'larını (WebUI) `.env` değerleriyle birleştirir.
fn resolve_domains(engine: &EngineSettings, config: &Config) -> parser::PlatformDomains {
    parser::PlatformDomains {
        twitter: engine.domain_for("twitter", config),
        tiktok: engine.domain_for("tiktok", config),
        instagram: engine.domain_for("instagram", config),
        reddit: engine.domain_for("reddit", config),
        bluesky: engine.domain_for("bluesky", config),
        spotify: engine.domain_for("spotify", config),
    }
}

/// Kullanıcının mesajındaki desteklenen sosyal medya linkini tespit eder ve dönüştürür.
///
/// Dönüştürme, platformun ayarına göre iki motordan biriyle yapılır:
/// - `builtin`: bot veriyi kendisi çeker ve native Discord embed'i üretir.
/// - `proxy`: link harici bir önizleme servisine (fixupx.com vb.) yönlendirilir.
///
/// Proxy formatı: `<@User>[:](fixedLink) [**Platform**](<origLink>)\n> Alıntı`
/// Silme işlemi için kompakt 🗑️ reaksiyonu eklenir.
pub async fn handle_message(
    ctx: &Context,
    msg: &Message,
    client: &reqwest::Client,
    config: &Config,
    settings: &GuildSettings,
) {
    let domains = resolve_domains(&settings.engine, config);
    let parsed = match parser::parse_message_with_domains(&msg.content, &domains) {
        Some(p) => p,
        None => return, // Desteklenen bir bağlantı bulunamadı
    };

    // 1. Modül Etkinlik Kontrolü
    let (module_enabled, module_key) = match parsed.platform {
        parser::Platform::Twitter => (settings.modules.twitter, "twitter"),
        parser::Platform::TikTok => (settings.modules.tiktok, "tiktok"),
        parser::Platform::Instagram => (settings.modules.instagram, "instagram"),
        parser::Platform::Reddit => (settings.modules.reddit, "reddit"),
        parser::Platform::Bluesky => (settings.modules.bluesky, "bluesky"),
        parser::Platform::Spotify => (settings.modules.spotify, "spotify"),
    };

    if !module_enabled {
        return;
    }

    // 2. Modüle Özel Rol Tabanlı Allowlist / Blacklist Filtresi
    let role_filter = settings.get_role_filter(module_key);
    if role_filter.mode != FilterMode::Disabled && !role_filter.roles.is_empty() {
        let user_roles: Vec<String> = if let Some(ref member) = msg.member {
            member.roles.iter().map(|r| r.get().to_string()).collect()
        } else if let Some(guild_id) = msg.guild_id {
            match guild_id.member(&ctx.http, msg.author.id).await { Ok(member) => {
                member.roles.iter().map(|r| r.get().to_string()).collect()
            } _ => {
                Vec::new()
            }}
        } else {
            Vec::new()
        };

        let has_matching_role = user_roles.iter().any(|r| role_filter.roles.contains(r));

        match role_filter.mode {
            FilterMode::Blacklist => {
                if has_matching_role {
                    info!("Kullanıcı modül kara listesinde ({}: Blacklist rolü var), dönüştürme atlandı: {}", module_key, msg.author.id);
                    return;
                }
            }
            FilterMode::Allowlist => {
                if !has_matching_role {
                    info!("Kullanıcı modül beyaz listesinde değil ({}: Allowlist rolü yok), dönüştürme atlandı: {}", module_key, msg.author.id);
                    return;
                }
            }
            FilterMode::Disabled => {}
        }
    }

    // 3. Motor Seçimi: native embed veya proxy yönlendirmesi
    let platform_key = builtin::platform_key(parsed.platform);
    if settings.engine.mode_for(platform_key) == ConversionMode::Builtin {
        if builtin::handle(ctx, msg, client, &parsed, config, &settings.engine).await {
            return;
        }

        if !settings.engine.fallback_to_proxy {
            warn!(
                "Native embed üretilemedi ve proxy yedeği kapalı, dönüştürme atlandı: {}",
                parsed.original_url
            );
            return;
        }

        warn!(
            "Native embed üretilemedi, proxy motoruna düşülüyor ({}): {}",
            platform_key, parsed.original_url
        );
    }

    // 4. Embed Formatı (Spoiler varsa gizle)
    let embed_part = if parsed.is_spoiler {
        format!("||[:]({})||", parsed.fixed_url)
    } else {
        format!("[:]({})", parsed.fixed_url)
    };

    // 5. Mesajın link kısmı (her koşulda korunur)
    let base_content = format!(
        "<@{}>{} [**{}**](<{}>)",
        msg.author.id,
        embed_part,
        parsed.platform.display_name(),
        parsed.original_url
    );

    // 6. Alıntı Formatı (Varsa) - her satır ayrı ayrı blok alıntı olarak işaretlenir
    let quoted_comment = parsed.user_comment.as_ref().map(|comment| {
        let quoted: Vec<String> = comment.lines().map(|line| format!("> {line}")).collect();
        format!("\n{}", quoted.join("\n"))
    });

    let mut content = match &quoted_comment {
        Some(comment) => format!("{base_content}{comment}"),
        None => base_content.clone(),
    };

    // Discord 2000 karakteri aşan içeriği reddeder: önce alıntıyı düşür, sonra kırp
    if content.chars().count() > DISCORD_CONTENT_LIMIT {
        content = base_content;
    }
    let content = truncate_chars(&content, DISCORD_CONTENT_LIMIT);

    let allowed_mentions = build_selective_allowed_mentions(msg);

    let mut builder = CreateMessage::new()
        .content(content)
        .allowed_mentions(allowed_mentions);

    // Eğer orijinal mesaj bir yanıtsa, bot da aynı mesaja yanıt versin
    if let Some(ref message_ref) = msg.message_reference {
        builder = builder.reference_message(message_ref.clone());
    }

    // 7. Mesajı Gönder. Orijinal mesaj burada silinmez: erken silmek, gönderenin istemcisi
    // optimistic (nonce'lu) mesajını sunucu kopyasıyla eşleştirmeden "hayalet mesaj"
    // bırakır (discord-api-docs #850). Silme, gönderimden **sonra** arka planda yapılır.
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

    // 8. Orijinal mesajı **arka planda** sil (kart gönderildikten sonra). Builtin yardımcısı
    // silmeyi mesaj en az DELETE_MIN_AGE yaşına gelene kadar erteler: gönderenin istemcisi
    // optimistic mesajını eşleştirir ve mesaj "hayalet" olarak kalmaz.
    if config.delete_original {
        builtin::spawn_delete(ctx, msg);
    }

    // 9. Kompakt 🗑️ reaksiyonu ekle
    if let Err(e) = sent_msg
        .react(&ctx.http, ReactionType::Unicode("🗑️".to_string()))
        .await
    {
        error!("Silme reaksiyonu eklenemedi: {:?}", e);
    }
}

