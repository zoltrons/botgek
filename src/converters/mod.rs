pub mod builtin;
pub mod parser;
pub mod translation;

use serenity::all::{Context, CreateMessage, Message, ReactionType};
use std::sync::OnceLock;
use tokio::sync::mpsc::{self, Sender, TrySendError};
use tracing::{error, info, warn};

use crate::config::Config;
use crate::settings::{ConversionMode, EngineSettings, GuildSettings};
use crate::utils::mentions::build_selective_allowed_mentions;
use crate::utils::{DISCORD_CONTENT_LIMIT, truncate_chars};

/// Tek tüketicili dönüşüm kuyruğu, Discord olayları başına sınırsız görev birikmesini önler.
/// Kuyruk dolduğunda yeni bağlantı sessizce bekletilmez; özgün mesaj korunarak açıkça reddedilir.
pub const CONVERSION_QUEUE_CAPACITY: usize = 32;

struct ConversionJob {
    ctx: Context,
    msg: Message,
    client: reqwest::Client,
    config: Config,
    settings: GuildSettings,
    parsed: parser::ParsedPost,
}

static CONVERSION_QUEUE: OnceLock<Sender<ConversionJob>> = OnceLock::new();

fn conversion_sender() -> &'static Sender<ConversionJob> {
    CONVERSION_QUEUE.get_or_init(|| {
        let (sender, mut receiver) = mpsc::channel(CONVERSION_QUEUE_CAPACITY);
        tokio::spawn(async move {
            while let Some(job) = receiver.recv().await {
                crate::telemetry::CONVERSION_QUEUE_DEPTH
                    .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                process_message(
                    &job.ctx,
                    &job.msg,
                    &job.client,
                    &job.config,
                    &job.settings,
                    &job.parsed,
                )
                .await;
                crate::telemetry::CONVERSION_QUEUE_COMPLETED
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        });
        sender
    })
}

/// Ayarlardaki alan adı override'larını (WebUI) `.env` değerleriyle birleştirir.
fn resolve_domains(engine: &EngineSettings, config: &Config) -> parser::PlatformDomains {
    parser::PlatformDomains {
        tiktok: engine.domain_for("tiktok", config),
        bluesky: engine.domain_for("bluesky", config),
        spotify: engine.domain_for("spotify", config),
    }
}

/// Kullanıcının mesajındaki desteklenen sosyal medya linkini kuyruğa alır.
///
/// Kuyruk doluysa `false` döner. Bu, ölçülebilir backpressure davranışıdır: özgün mesaj
/// korunur ve çağıran taraf AI akışına devam edebilir. Link/module/rol kontrolleri kuyruğa
/// girmeden önce yapılır; böylece yetkisiz işler kapasite tüketmez.
pub async fn enqueue_message(
    ctx: &Context,
    msg: &Message,
    client: &reqwest::Client,
    config: &Config,
    settings: &GuildSettings,
) -> bool {
    let Some(job) = prepare_job(ctx, msg, client, config, settings).await else {
        return true;
    };

    crate::telemetry::CONVERSION_QUEUE_DEPTH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    match conversion_sender().try_send(job) {
        Ok(()) => {
            crate::telemetry::CONVERSION_QUEUE_ENQUEUED
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            true
        }
        Err(TrySendError::Full(_job)) => {
            crate::telemetry::CONVERSION_QUEUE_DEPTH
                .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            crate::telemetry::CONVERSION_QUEUE_SATURATED
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            crate::telemetry::OVERLOADS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            warn!(
                capacity = CONVERSION_QUEUE_CAPACITY,
                "Conversion queue capacity reached; original message preserved"
            );
            false
        }
        Err(TrySendError::Closed(_job)) => {
            crate::telemetry::CONVERSION_QUEUE_DEPTH
                .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            crate::telemetry::CONVERSION_QUEUE_SATURATED
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            crate::telemetry::OVERLOADS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            error!("Conversion queue worker stopped; original message preserved");
            false
        }
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
    let Some(job) = prepare_job(ctx, msg, client, config, settings).await else {
        return;
    };
    process_message(
        &job.ctx,
        &job.msg,
        &job.client,
        &job.config,
        &job.settings,
        &job.parsed,
    )
    .await;
}

async fn prepare_job(
    ctx: &Context,
    msg: &Message,
    client: &reqwest::Client,
    config: &Config,
    settings: &GuildSettings,
) -> Option<ConversionJob> {
    let domains = resolve_domains(&settings.engine, config);
    let parsed = match parser::parse_message_with_domains(&msg.content, &domains) {
        Some(p) => p,
        None => return None, // Desteklenen bir bağlantı bulunamadı
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
        return None;
    }

    // 2. Modüle Özel Rol Tabanlı Allowlist / Blacklist Filtresi
    let role_filter = settings.get_role_filter(module_key);
    if !crate::policy::allows(
        ctx,
        msg.guild_id,
        msg.author.id,
        msg.member.as_ref().map(|m| m.roles.as_slice()),
        role_filter,
    )
    .await
    {
        return None;
    }

    Some(ConversionJob {
        ctx: ctx.clone(),
        msg: msg.clone(),
        client: client.clone(),
        config: config.clone(),
        settings: settings.clone(),
        parsed,
    })
}

async fn process_message(
    ctx: &Context,
    msg: &Message,
    client: &reqwest::Client,
    config: &Config,
    settings: &GuildSettings,
    parsed: &parser::ParsedPost,
) {
    // 3. Motor Seçimi: native embed veya (yalnızca proxy destekleyen platformlarda) proxy
    //    yönlendirmesi. Twitter/X, Instagram ve Reddit **native-only**'dir: `mode_for` bu
    //    platformlarda daima `Builtin` döner ve proxy yedeği hiç kullanılmaz.
    let platform_key = builtin::platform_key(parsed.platform);
    if settings.engine.mode_for(platform_key) == ConversionMode::Builtin {
        if builtin::handle(ctx, msg, client, parsed, config, &settings.engine).await {
            return;
        }

        if !settings.engine.allows_proxy_fallback(platform_key) {
            warn!(
                "Native embed üretilemedi ve bu platformda proxy yedeği yok, dönüştürme atlandı: {}",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn conversion_queue_rejects_after_fixed_capacity_and_keeps_fifo_order() {
        let (sender, mut receiver) = mpsc::channel(CONVERSION_QUEUE_CAPACITY);

        for value in 0..CONVERSION_QUEUE_CAPACITY {
            sender
                .try_send(value)
                .expect("capacity should admit each slot");
        }
        assert!(matches!(
            sender.try_send(CONVERSION_QUEUE_CAPACITY),
            Err(TrySendError::Full(value)) if value == CONVERSION_QUEUE_CAPACITY
        ));

        for expected in 0..CONVERSION_QUEUE_CAPACITY {
            assert_eq!(receiver.recv().await, Some(expected));
        }
        assert!(receiver.try_recv().is_err());
    }
}
