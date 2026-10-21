pub mod builtin;
pub mod parser;
pub mod translation;

use serenity::all::{
    ChannelId, Context, CreateMessage, GuildId, Member, Message, MessageId, MessageReference,
    ReactionType, User,
};
use std::sync::LazyLock;
use std::sync::OnceLock;
use tokio::sync::Semaphore;
use tokio::sync::mpsc::{self, Sender, error::TrySendError};
use tracing::{error, info, warn};

use crate::config::Config;
use crate::settings::{ConversionMode, EngineSettings, GuildSettings};
use crate::utils::mentions::build_selective_allowed_mentions;
use crate::utils::{DISCORD_CONTENT_LIMIT, truncate_chars};

/// Tek tüketicili dönüşüm kuyruğu, Discord olayları başına sınırsız görev birikmesini önler.
/// Kuyruk dolduğunda yeni bağlantı sessizce bekletilmez; özgün mesaj korunarak açıkça reddedilir.
pub const CONVERSION_QUEUE_CAPACITY: usize = 32;

/// Aynı anda yürüyen dönüşüm sayısı. Tek bir yavaş iş (Reddit videosu, 30 sn zaman aşımı)
/// diğer sunucuların kartlarını bekletmesin diye işler paralel yürür; sınır, native akışın
/// kendi kapasitesinin (`builtin::handle`, 16) altında tutulur.
const CONVERSION_WORKERS: usize = 8;

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
        let (sender, mut receiver) = mpsc::channel::<ConversionJob>(CONVERSION_QUEUE_CAPACITY);
        let slots = std::sync::Arc::new(tokio::sync::Semaphore::new(CONVERSION_WORKERS));
        tokio::spawn(async move {
            loop {
                // Kuyruktan yalnızca boş yuva varken alınır: bekleyen işler kanal kapasitesini
                // tüketmeye devam eder, böylece backpressure (`try_send` → Full) korunur.
                let Ok(permit) = slots.clone().acquire_owned().await else {
                    break;
                };
                let Some(job) = receiver.recv().await else {
                    break;
                };
                crate::telemetry::CONVERSION_QUEUE_DEPTH
                    .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                // Her iş kendi görevinde yürür: bir işteki panik yalnızca o kartı düşürür,
                // kuyruk tüketicisini öldürmez.
                tokio::spawn(async move {
                    let _permit = permit;
                    process_message(
                        &job.ctx,
                        &job.msg,
                        &job.client,
                        &job.config,
                        &job.settings,
                        &job.parsed,
                        None,
                    )
                    .await;
                    crate::telemetry::CONVERSION_QUEUE_COMPLETED
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                });
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

fn module_enabled(settings: &GuildSettings, platform: parser::Platform) -> (bool, &'static str) {
    match platform {
        parser::Platform::Twitter => (settings.modules.twitter, "twitter"),
        parser::Platform::TikTok => (settings.modules.tiktok, "tiktok"),
        parser::Platform::Instagram => (settings.modules.instagram, "instagram"),
        parser::Platform::Reddit => (settings.modules.reddit, "reddit"),
        parser::Platform::Bluesky => (settings.modules.bluesky, "bluesky"),
        parser::Platform::Spotify => (settings.modules.spotify, "spotify"),
        parser::Platform::Twitch => (settings.modules.twitch, "twitch"),
        parser::Platform::Pixiv => (settings.modules.pixiv, "pixiv"),
        parser::Platform::Tumblr => (settings.modules.tumblr, "tumblr"),
        parser::Platform::Facebook => (settings.modules.facebook, "facebook"),
        parser::Platform::YouTube => (settings.modules.youtube, "youtube"),
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

/// `/dönüştür` gibi etkileşimlerin **eşzamanlı** dönüşüm sayısı.
///
/// Etkileşim yanıtı kullanıcıyı beklettiği için otomatik mesaj kuyruğundan ayrı, küçük bir
/// kapasiteyle sınırlanır: yoğunlukta komut anında "meşgul" yanıtı alır, kuyruktaki
/// konuşmaların kartları gecikmez.
const INTERACTION_WORKERS: usize = 4;
static INTERACTION_SLOTS: LazyLock<Semaphore> =
    LazyLock::new(|| Semaphore::new(INTERACTION_WORKERS));

/// Etkileşimden (slash/mesaj komutu) yapılan dönüşümün sonucu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConvertError {
    /// Mesajda/argümanda desteklenen bir bağlantı yok.
    Unsupported,
    /// Rol filtresi kullanıcıya bu dönüşümü izin vermiyor.
    RoleDenied,
    /// Eşzamanlı etkileşim dönüşüm kapasitesi dolu.
    Busy,
    /// Bağlantı çözümlenemedi veya kart gönderilemedi.
    Failed,
}

impl ConvertError {
    /// Discord'a gösterilecek kısa, sır içermeyen açıklama.
    pub fn message(self) -> &'static str {
        match self {
            Self::Unsupported => {
                "Desteklenen bir bağlantı bulunamadı. Örnek: `/dönüştür link:https://x.com/kullanici/status/123`"
            }
            Self::RoleDenied => "Rol filtresi bu dönüştürmeye izin vermiyor.",
            Self::Busy => "Dönüşüm kapasitesi şu an dolu; birkaç saniye sonra tekrar dene.",
            Self::Failed => {
                "Bağlantı dönüştürülemedi (gönderi silinmiş veya kaynağa erişilemiyor olabilir)."
            }
        }
    }
}

/// Etkileşim dönüşümünün ayar kopyası.
///
/// - Kaynak mesaj/taklit yok: `impersonate_user` kapatılır.
/// - X çevirisi komut için **varsayılan olarak açıktır**: WebUI'daki `auto_translate`
///   anahtarı kapalı olsa bile `/dönüştür` tweetleri hedef dile çevirir. Hedef dil ayarı
///   korunur (varsayılan `tr`).
fn interaction_settings(settings: &GuildSettings) -> GuildSettings {
    let mut settings = settings.clone();
    settings.engine.impersonate_user = false;
    settings.engine.auto_translate = true;
    settings
}

/// Etkileşimden gelen bağlantıyı **bekleyerek** dönüştürür.
///
/// Otomatik mesaj akışından farkları:
/// - **Platform modülü açık/kapalı ayarına bakmaz:** komut kullanıcı tarafından açıkça
///   çağrıldığı için, WebUI'da o platform kapalı olsa bile dönüştürür. Rol filtresi ve
///   kanal/thread motor ayarları yine uygulanır.
/// - **X çevirisi varsayılan olarak açıktır:** WebUI'daki `auto_translate` kapalı olsa bile
///   tweetler hedef dile çevrilir; hedef dil ayarı (`translation_target`) korunur.
/// - Kullanıcının yazdığı bir mesaj yoktur: silinecek kaynak ve taklit edilecek gönderen
///   bulunmaz, bu yüzden `delete_original` ve `impersonate_user` bu iş için kapatılır.
/// - Sonuç çağırana döner; slash komutu yanıtı bu sonuca göre güncellenir.
///
/// `author`, kartın "Gönderen" etiketinde görünecek kullanıcıdır (slash komutunda komutu
/// kullanan, mesaj komutunda hedef mesajın yazarı). `actor` ise rol filtresinden geçen
/// **komutu kullanan** kullanıcıdır; ikisi mesaj komutunda bilinçli olarak ayrıdır.
#[allow(clippy::too_many_arguments)]
pub async fn convert_now(
    ctx: &Context,
    content: &str,
    author: &User,
    actor: &User,
    actor_member: Option<&Member>,
    guild_id: Option<GuildId>,
    channel_id: ChannelId,
    // Mesaj komutunda kartın yanıt olarak bağlanacağı hedef mesaj.
    reply_to: Option<MessageId>,
    client: &reqwest::Client,
    config: &Config,
    settings: &GuildSettings,
    delivery: Option<&builtin::InteractionDelivery>,
) -> Result<parser::Platform, ConvertError> {
    let domains = resolve_domains(&settings.engine, config);
    let Some(parsed) = parser::parse_message_with_domains(content, &domains) else {
        return Err(ConvertError::Unsupported);
    };

    // Komut açıkça çağrıldığı için platform modülünün WebUI'da kapalı olması engel değildir.
    // Rol filtresi yine uygulanır: modül anahtarı kartın/rol filtresinin platformunu belirler.
    let module_key = builtin::platform_key(parsed.platform);

    let role_filter = settings.get_role_filter(module_key);
    if !crate::policy::allows(
        ctx,
        guild_id,
        actor.id,
        actor_member.map(|member| member.roles.as_slice()),
        role_filter,
    )
    .await
    {
        return Err(ConvertError::RoleDenied);
    }

    let Ok(_slot) = INTERACTION_SLOTS.try_acquire() else {
        return Err(ConvertError::Busy);
    };

    let mut msg = Message::default();
    msg.id = generated_message_id();
    msg.channel_id = channel_id;
    msg.guild_id = guild_id;
    msg.author = author.clone();
    msg.content = content.to_string();
    if let Some(target_id) = reply_to {
        let mut reference = MessageReference::new(Default::default(), channel_id)
            .message_id(target_id)
            .fail_if_not_exists(false);
        if let Some(guild_id) = guild_id {
            reference = reference.guild_id(guild_id);
        }
        msg.message_reference = Some(reference);
    }

    // Kaynak mesaj yok: ne silinecek bir kullanıcı mesajı ne de taklit edilecek bir gönderen var.
    // X çevirisi etkileşim dönüşümünde varsayılan olarak açıktır (bkz. `interaction_settings`).
    let mut job_config = config.clone();
    job_config.delete_original = false;
    let job_settings = interaction_settings(settings);

    let delivered = process_message(
        ctx,
        &msg,
        client,
        &job_config,
        &job_settings,
        &parsed,
        delivery,
    )
    .await;
    if delivered {
        Ok(parsed.platform)
    } else {
        Err(ConvertError::Failed)
    }
}

/// Etkileşim için üretilen yapay mesajın kimliği.
///
/// Gerçek bir Discord mesajına işaret etmez; yalnızca `Message` tipinin zorunlu alanını
/// doldurur. Bu işte silme ve taklit kapalıdır, dolayısıyla kimlik hiçbir REST çağrısında
/// kullanılmaz. Yine de sıfır olmayan, tarihçe olarak tutarlı bir snowflake üretilir.
fn generated_message_id() -> serenity::all::MessageId {
    const DISCORD_EPOCH_MS: u64 = 1_420_070_400_000;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    serenity::all::MessageId::new(now_ms.saturating_sub(DISCORD_EPOCH_MS) << 22)
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
    let (module_enabled, module_key) = module_enabled(settings, parsed.platform);

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

/// Dönüşümü uçtan uca yürütür. Kart/proxy mesajı gerçekten gönderildiyse `true` döner;
/// kaynak çözümlenemediğinde veya Discord gönderimi reddettiğinde `false`.
async fn process_message(
    ctx: &Context,
    msg: &Message,
    client: &reqwest::Client,
    config: &Config,
    settings: &GuildSettings,
    parsed: &parser::ParsedPost,
    delivery: Option<&builtin::InteractionDelivery>,
) -> bool {
    // 3. Motor Seçimi: native embed veya (yalnızca proxy destekleyen platformlarda) proxy
    //    yönlendirmesi. Twitter/X, Instagram ve Reddit **native-only**'dir: `mode_for` bu
    //    platformlarda daima `Builtin` döner ve proxy yedeği hiç kullanılmaz.
    let platform_key = builtin::platform_key(parsed.platform);
    if settings.engine.mode_for(platform_key) == ConversionMode::Builtin {
        match builtin::handle(ctx, msg, client, parsed, config, &settings.engine, delivery).await {
            builtin::HandleResult::Sent => return true,
            // Kapasite dolu veya sonuç belirsiz: proxy'ye düşmek mükerrer/yanlış kart üretebilir.
            builtin::HandleResult::Busy | builtin::HandleResult::Ambiguous => return false,
            builtin::HandleResult::Failed => {}
        }

        if !settings.engine.allows_proxy_fallback(platform_key) {
            warn!(
                "Native embed üretilemedi ve bu platformda proxy yedeği yok, dönüştürme atlandı: {}",
                parsed.original_url
            );
            return false;
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

    // 6. Alıntı Formatı (Varsa) - her satır ayrı ayrı blok alıntı olarak işaretlenir.
    // Boş satırlar **boş alıntı satırı** (`>`) üretmez: Discord bunu mesajda fazladan bir `>`
    // işareti/ayrı boş şerit olarak çiziyor (aynı kural `v2::community_note_content` ve
    // `quote_component` için de geçerlidir).
    let quoted_comment = parsed.user_comment.as_ref().map(|comment| {
        let quoted: Vec<String> = comment
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| format!("> {line}"))
            .collect();
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
        builder = builder.reference_message(message_ref.clone().fail_if_not_exists(false));
    }

    // 7. Mesajı Gönder. Orijinal mesaj burada silinmez: erken silmek, gönderenin istemcisi
    // optimistic (nonce'lu) mesajını sunucu kopyasıyla eşleştirmeden "hayalet mesaj"
    // bırakır (discord-api-docs #850). Silme, gönderimden **sonra** arka planda yapılır.
    let sent_msg = match msg.channel_id.send_message(&ctx.http, builder).await {
        Ok(m) => m,
        Err(e) => {
            error!("Mesaj gönderilirken hata oluştu: {:?}", e);
            return false;
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

    true
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

    #[test]
    fn interaction_settings_force_translation_and_disable_impersonation() {
        let mut settings = GuildSettings::default();
        settings.engine.auto_translate = false;
        settings.engine.impersonate_user = true;
        settings.engine.translation_target = "de".to_string();

        let resolved = interaction_settings(&settings);

        assert!(
            resolved.engine.auto_translate,
            "etkileşim dönüşümü X çevirisini varsayılan olarak açmalı"
        );
        assert!(
            !resolved.engine.impersonate_user,
            "etkileşim dönüşümünde taklit kapalı olmalı"
        );
        assert_eq!(
            resolved.engine.translation_target, "de",
            "hedef dil ayarı korunmalı"
        );
    }
}
