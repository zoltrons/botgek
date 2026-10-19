//! Bot içi (native) link dönüştürme motoru.
//!
//! Proxy motoru linki `fixupx.com` benzeri bir önizleme servisine çevirip Discord'un
//! kendi embed'ini üretmesini bekler. Bu motor ise gönderi verisini doğrudan kaynaktan
//! (X syndication/fxtwitter, Reddit JSON, Bluesky XRPC, TikTok oEmbed) çeker; başlık,
//! metin, metrikler, alıntı ve medyayı kendisi indirip `CreateEmbed` olarak gönderir.

pub mod bluesky;
pub mod cache;
mod coordination;
pub mod facebook;
pub mod instagram;
pub mod media;
pub mod opengraph;
pub mod pixiv;
mod preview;
pub mod raw;
pub mod reddit;
pub mod reddit_video;
pub mod render;
mod safe_http;
pub mod spotify;
pub mod tiktok;
pub mod translate;
mod transport;
pub mod tumblr;
pub mod twitch;
pub mod twitter;
pub mod v2;
pub mod youtube;

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serenity::all::{
    Channel, ChannelId, ChannelType, Context, Embed, Message, MessageId, UserId, WebhookId,
};
use std::sync::LazyLock;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use super::parser::{ParsedPost, Platform};
use crate::config::Config;
use crate::settings::{EmojiSettings, EngineSettings};
use crate::utils::mentions::build_selective_allowed_mentions;

/// Dış servis istekleri için zaman aşımı (ışık hızı: askıda kalan istek kartı bekletmez).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(6);
/// Kartın **kullanıcıya görünmesi** için hedeflenen üst sınır: dönüşüm başlangıcından
/// (veri çekimi + medya + gönderim) itibaren. Tek bir isteğin ağ gecikmesi bu bütçenin
/// dışında kalabilir; amaç, veri çekimini kaynağın en yavaş yanıtına **bağlı bırakmamaktır**.
pub const LATENCY_TARGET: Duration = Duration::from_millis(1_000);
/// Kart verisi hazır olduktan sonra **isteğe bağlı** zenginleştirme (çeviri, üçüncü seviye
/// alıntı, görüntülenme/topluluk notu, harici link kartı) için tanınan süre.
///
/// Bu süre dolar ve kart hâlâ gönderilmediyse bekleyen zenginleştirme bırakılır: yavaş
/// vxTwitter yanıtı yüzünden kart saniyelerce gecikmez (X syndication'ın verdiği metin,
/// yazar, medya ve alıntı zaten kartı çizmeye yeter).
pub const FETCH_ENRICH_BUDGET: Duration = Duration::from_millis(650);
/// Hiçbir kaynak ilk veriyi vermezse, kart büsbütün düşmesin diye beklenecek üst sınır.
///
/// Bu sınıra ancak **tüm** kaynaklar yavaş/hatalıysa gelinir (syndication genelde hızlıdır).
/// Tek istek zaten [`REQUEST_TIMEOUT`] ile sınırlıdır; bu bütçe, seri kalan zincirlerin
/// 6 saniyeye kadar uzamasını engeller. Süre dolarsa orijinal mesaj korunur.
pub const FETCH_FIRST_DATA_BUDGET: Duration = Duration::from_millis(3_000);
/// Taklit webhook'u için tanınan **en fazla** ek süre.
///
/// Gerçek bekleme [`webhook_wait`] ile kalan bütçeye göre kısaltılır: veri fazı bütçeyi
/// yediyse webhook hiç beklenmez (kart bot adına gider), veri hızlı bittiyse en fazla bu kadar.
const WEBHOOK_WARMUP_WAIT: Duration = Duration::from_millis(200);
/// Discord POST'u için bütçede ayrılan pay (sıcak HTTP/2 bağlantısında ~200-250 ms).
const SEND_RESERVE: Duration = Duration::from_millis(250);

/// Taklit webhook'u hazır değilse, **kalan bütçe kadar** beklenir.
///
/// Warmup veri çekimiyle paralel başladığı için normalde gönderim anında hazırdır; bu süre
/// yalnızca yavaş bir kanalda kartın 1 saniyelik hedefi aşmaması için bir üst sınırdır.
fn webhook_wait(prepare: Duration) -> Duration {
    LATENCY_TARGET
        .saturating_sub(prepare)
        .saturating_sub(SEND_RESERVE)
        .min(WEBHOOK_WARMUP_WAIT)
}

/// Video/görsel indirme için daha uzun zaman aşımı (tek isteğin üst sınırı).
///
/// **Etkin** sınır bu değil [`render::VIDEO_BUDGET`] gibi süre bütçeleridir: kart, medya
/// indirmesini beklemez; bütçe dolan indirme iptal edilir ve medya uzak bağlantıya düşer.
pub const MEDIA_TIMEOUT: Duration = Duration::from_secs(12);
/// Tek bir JSON/metin yanıtı için okunacak maksimum gövde boyutu (2 MB).
pub const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
/// Tanımlayıcı User-Agent: fxtwitter gibi servisler uygulamanın kendini tanıtmasını ister.
/// Sürüm bilgisi derleme zamanında gömülür (bkz. [`crate::version`]).
pub const USER_AGENT: &str = crate::version::USER_AGENT;
/// Discord REST API için resmi standartta User-Agent (Discord Developer Docs: `DiscordBot ($url, $versionNumber)`).
pub const DISCORD_USER_AGENT: &str = crate::version::DISCORD_USER_AGENT;

/// Platform anahtarı (`twitter`, `reddit` ...) — ayarlarda ve loglarda kullanılır.
pub fn platform_key(platform: Platform) -> &'static str {
    match platform {
        Platform::Twitter => "twitter",
        Platform::TikTok => "tiktok",
        Platform::Instagram => "instagram",
        Platform::Reddit => "reddit",
        Platform::Bluesky => "bluesky",
        Platform::Spotify => "spotify",
        Platform::Twitch => "twitch",
        Platform::Pixiv => "pixiv",
        Platform::Tumblr => "tumblr",
        Platform::Facebook => "facebook",
        Platform::YouTube => "youtube",
    }
}

mod model;
pub use model::{
    Author, BuiltinPost, CustomCard, LinkPreview, Media, MediaKind, Stats, Translation,
};

// =====================================================================================
// HTTP YARDIMCILARI
// =====================================================================================

/// JSON yanıtını çeker ve çözümler. Gövde boyutu `MAX_BODY_BYTES` ile sınırlıdır.
pub async fn get_json(client: &reqwest::Client, url: &str) -> Result<serde_json::Value, String> {
    let body = get_text(client, url).await?;
    serde_json::from_str(&body).map_err(|e| format!("JSON çözümlenemedi ({url}): {e}"))
}

/// Metin yanıtını çeker. `Content-Length` veya akış `MAX_BODY_BYTES`'ı aşarsa hata verir.
pub async fn get_text(client: &reqwest::Client, url: &str) -> Result<String, String> {
    let response = client
        .get(url)
        .header("user-agent", USER_AGENT)
        .header("accept", "application/json, text/html;q=0.8, */*;q=0.5")
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("İstek başarısız ({url}): {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "Beklenmeyen durum kodu ({url}): {}",
            response.status()
        ));
    }
    if let Some(len) = response.content_length()
        && len > MAX_BODY_BYTES as u64
    {
        return Err(format!("Yanıt çok büyük ({url}): {len} bayt"));
    }

    let mut response = response;
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("Yanıt gövdesi okunamadı ({url}): {e}"))?
    {
        if body.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(format!("Yanıt gövdesi sınırı aşıldı ({url})"));
        }
        body.extend_from_slice(&chunk);
    }

    String::from_utf8(body).map_err(|_| format!("Yanıt UTF-8 değil ({url})"))
}

// =====================================================================================
// VERİ ÇEKME
// =====================================================================================

fn post_cache_key(parsed: &ParsedPost, engine: &EngineSettings) -> String {
    // The content-aware Twitter path may finish from Syndication when neither notes
    // nor metrics are requested. Do not reuse that result for a richer settings scope.
    format!(
        "post:{}|{}|{}|{}|{}|{}|{}|{}|{}",
        parsed.original_url,
        engine.twitter_source.key(),
        engine.auto_translate,
        engine.translation_target,
        engine.show_quote,
        engine.max_video_mb,
        parsed.is_spoiler,
        engine.show_community_note,
        engine.show_metrics,
    )
}

/// Paylaşılan gönderinin verisini, platforma uygun kaynaktan çeker.
/// Platforma göre gönderi verisini çeker. **Önbellek önce** denenir: aynı link kısa süre
/// içinde tekrar paylaşıldığında ağ isteği yapılmaz (dönüşüm anında tamamlanır).
pub async fn fetch(
    client: &reqwest::Client,
    parsed: &ParsedPost,
    engine: &EngineSettings,
    config: &Config,
) -> Result<std::sync::Arc<BuiltinPost>, String> {
    let key = post_cache_key(parsed, engine);

    let _single_flight = coordination::lock(key.clone()).await;

    if let Some(cached) = cache::get(&key).await {
        crate::telemetry::CACHE_HITS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        debug!("Gönderi önbellekten alındı: {}", parsed.original_url);
        return Ok(cached);
    }
    if let Some(reason) = cache::recent_failure(&key).await {
        return Err(reason);
    }

    let post = match parsed.platform {
        Platform::Twitter => twitter::fetch(client, parsed, engine).await,
        Platform::Bluesky => bluesky::fetch(client, parsed).await,
        Platform::Reddit => reddit::fetch(client, parsed, &config.reddit_client_id).await,
        Platform::TikTok => tiktok::fetch(client, parsed).await,
        Platform::Instagram => {
            instagram::fetch(client, parsed, config.instagram_sessionid.as_deref()).await
        }
        Platform::Spotify => spotify::fetch(client, parsed).await,
        Platform::Twitch => twitch::fetch(client, parsed).await,
        Platform::Pixiv => pixiv::fetch(client, parsed).await,
        Platform::Tumblr => tumblr::fetch(client, parsed).await,
        Platform::Facebook => facebook::fetch(client, parsed).await,
        Platform::YouTube => youtube::fetch(client, parsed).await,
    };
    let post = match post {
        Ok(post) => post,
        Err(reason) => {
            cache::put_failure(key, reason.clone()).await;
            return Err(reason);
        }
    };

    let post = std::sync::Arc::new(post);
    cache::put(key, post.clone()).await;
    Ok(post)
}

// =====================================================================================
// ORTAK METİN / ZAMAN YARDIMCILARI
// =====================================================================================

/// Gönderi metninin sonundaki medya kısa linklerini (`t.co` / `pic.x.com`) temizler.
/// Twitter, metnin sonuna görsel/video için kısa link ekler; bunlar embed'de gereksizdir.
pub fn strip_trailing_media_links(text: &str) -> String {
    let mut cleaned = text.trim().to_string();
    while let Some(last) = cleaned.split_whitespace().last() {
        let last = last.to_string();
        let is_media_link = last.starts_with("https://t.co/")
            || last.starts_with("http://t.co/")
            || last.starts_with("https://pic.x.com/")
            || last.starts_with("https://twitter.com/i/")
            || last.starts_with("https://x.com/i/");
        if !is_media_link {
            break;
        }
        let trimmed = cleaned.trim_end();
        let cut = trimmed.len().saturating_sub(last.len());
        cleaned.truncate(cut);
        cleaned = cleaned.trim_end().to_string();
    }
    cleaned
}

/// ISO 8601 (`2024-05-06T12:34:56Z`) veya Twitter biçimini
/// (`Wed Oct 10 20:19:24 +0000 2018`) unix zaman damgasına çevirir.
pub fn parse_timestamp(raw: &str) -> Option<i64> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    if raw.len() >= 19
        && raw.as_bytes().get(4) == Some(&b'-')
        && raw.as_bytes().get(10) == Some(&b'T')
    {
        let year: i64 = raw.get(0..4)?.parse().ok()?;
        let month: i64 = raw.get(5..7)?.parse().ok()?;
        let day: i64 = raw.get(8..10)?.parse().ok()?;
        let hour: i64 = raw.get(11..13)?.parse().ok()?;
        let minute: i64 = raw.get(14..16)?.parse().ok()?;
        let second: i64 = raw.get(17..19)?.parse().ok()?;
        let offset = parse_offset(&raw[19..]).unwrap_or(0);
        return Some(
            days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second
                - offset,
        );
    }

    // "Wed Oct 10 20:19:24 +0000 2018"
    let parts: Vec<&str> = raw.split_whitespace().collect();
    if parts.len() >= 5 {
        let month = month_number(parts[1])?;
        let day: i64 = parts[2].parse().ok()?;
        let year: i64 = parts[parts.len() - 1].parse().ok()?;
        let mut time = parts[3].split(':');
        let hour: i64 = time.next()?.parse().ok()?;
        let minute: i64 = time.next()?.parse().ok()?;
        let second: i64 = time.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let offset = parse_offset(parts[4]).unwrap_or(0);
        return Some(
            days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second
                - offset,
        );
    }

    None
}

/// `+0300`, `+03:00` veya `Z` biçimindeki saat farkını saniyeye çevirir.
fn parse_offset(raw: &str) -> Option<i64> {
    let bytes = raw.as_bytes();
    if bytes.len() < 3 {
        return None;
    }
    let sign: i64 = match bytes[0] {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let digits: String = raw[1..].chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() < 4 {
        return None;
    }
    let hours: i64 = digits[0..2].parse().ok()?;
    let minutes: i64 = digits[2..4].parse().ok()?;
    Some(sign * (hours * 3_600 + minutes * 60))
}

fn month_number(name: &str) -> Option<i64> {
    match name.get(0..3)?.to_ascii_lowercase().as_str() {
        "jan" => Some(1),
        "feb" => Some(2),
        "mar" => Some(3),
        "apr" => Some(4),
        "may" => Some(5),
        "jun" => Some(6),
        "jul" => Some(7),
        "aug" => Some(8),
        "sep" => Some(9),
        "oct" => Some(10),
        "nov" => Some(11),
        "dec" => Some(12),
        _ => None,
    }
}

/// Sivil tarihi (1970-01-01 = 0) gün sayısına çevirir (Howard Hinnant algoritması).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = (month + 9) % 12;
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Sorgu parametrelerinde kullanılmak üzere percent-encoding uygular.
pub fn encode_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// HTML varlıklarını (`&amp;`, `&#39;`, `&quot;` ...) düz metne çevirir.
///
/// Tek geçişte soldan sağa çözülür: çözülen bir varlığın çıktısı yeniden taranmaz, böylece `&amp;lt;` kaynakta olduğu gibi `&lt;` kalır (çift çözme yok).
pub fn unescape_html(text: &str) -> String {
    // En uzun desteklenen varlık `&#x10FFFF;` / `&#1114111;`; daha uzak `;` aranmaz.
    const MAX_ENTITY_LEN: usize = 10;

    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 1..];
        let decoded = tail
            .char_indices()
            .take(MAX_ENTITY_LEN)
            .find(|&(_, ch)| ch == ';')
            .and_then(|(end, _)| decode_entity(&tail[..end]).map(|ch| (ch, end)));
        match decoded {
            Some((ch, end)) => {
                out.push(ch);
                rest = &tail[end + 1..];
            }
            None => {
                out.push('&');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `&` ile `;` arasındaki varlık adını çözer; tanınmayan veya geçersiz kod noktası `None` döner.
fn decode_entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "nbsp" => Some(' '),
        _ => {
            let digits = name.strip_prefix('#')?;
            let code = match digits.strip_prefix(['x', 'X']) {
                Some(hex) if !hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit()) => {
                    u32::from_str_radix(hex, 16).ok()
                }
                Some(_) => None,
                None if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) => {
                    digits.parse::<u32>().ok()
                }
                None => None,
            }?;
            char::from_u32(code)
        }
    }
}

#[cfg(test)]
mod encoding_tests {
    use super::*;

    #[test]
    fn query_components_are_encoded() {
        assert_eq!(encode_component("a b/c?d=e"), "a%20b%2Fc%3Fd%3De");
        assert_eq!(encode_component("abc-._~"), "abc-._~");
    }

    #[test]
    fn html_entities_are_decoded() {
        assert_eq!(unescape_html("a &amp; b"), "a & b");
        assert_eq!(unescape_html("O&#39;Brien"), "O'Brien");
        assert_eq!(unescape_html("B&#x27;lue"), "B'lue");
        assert_eq!(unescape_html("fiyat &#8378; 10"), "fiyat ₺ 10");
        assert_eq!(unescape_html("kapanmamış &#"), "kapanmamış &#");
    }

    #[test]
    fn html_entities_are_decoded_once_left_to_right() {
        assert_eq!(unescape_html("&amp;lt;b&amp;gt;"), "&lt;b&gt;");
        assert_eq!(unescape_html("&amp;#39;"), "&#39;");
        assert_eq!(
            unescape_html("&lt;a&gt; &quot;b&quot; &apos;c&#x27; d&nbsp;e"),
            "<a> \"b\" 'c' d e"
        );
        assert_eq!(unescape_html("&#X41;&#66;"), "AB");
        assert_eq!(
            unescape_html("AT&T &bilinmeyen; &#xZZ; &#55296; &#+65;"),
            "AT&T &bilinmeyen; &#xZZ; &#55296; &#+65;"
        );
        assert_eq!(unescape_html("çok &ğ; ünlü &amp"), "çok &ğ; ünlü &amp");
        assert_eq!(unescape_html("&&amp;;"), "&&;");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trailing_media_links_are_removed() {
        assert_eq!(
            strip_trailing_media_links("Selam dünya https://t.co/abc123"),
            "Selam dünya"
        );
        assert_eq!(
            strip_trailing_media_links("Bir bak https://pic.x.com/abc123  "),
            "Bir bak"
        );
        // Aynı satırda gerçek bir link varsa korunur
        assert_eq!(
            strip_trailing_media_links("Şu habere bak https://example.com/haber"),
            "Şu habere bak https://example.com/haber"
        );
        assert_eq!(strip_trailing_media_links("   "), "");
    }

    #[test]
    fn timestamps_parse_in_both_formats() {
        assert_eq!(parse_timestamp("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_timestamp("2024-05-06T12:34:56Z"), Some(1714998896));
        assert_eq!(
            parse_timestamp("2024-05-06T15:34:56+03:00"),
            Some(1714998896)
        );
        assert_eq!(
            parse_timestamp("Wed Oct 10 20:19:24 +0000 2018"),
            Some(1539202764)
        );
        assert_eq!(
            parse_timestamp("Thu Mar 21 20:50:14 +0000 2019"),
            Some(1553201414)
        );
    }

    #[test]
    fn invalid_timestamps_are_rejected() {
        assert_eq!(parse_timestamp(""), None);
        assert_eq!(parse_timestamp("bugün"), None);
        assert!(
            parse_timestamp("2024-13-99T99:99:99Z").is_some(),
            "bozuk tarih panik yapmadan çözülmeli"
        );
    }

    /// Verilen UNIX-milisaniye anına denk gelen bir snowflake `MessageId` üretir
    /// (testte "taze"/"eski" mesaj kurmak için).
    fn snowflake_at(ms: i64) -> MessageId {
        MessageId::new(((ms - DISCORD_EPOCH_MS) as u64) << 22)
    }

    #[test]
    fn delete_wait_only_delays_fresh_messages() {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        // Yeni gönderilmiş mesaj: hayalet penceresini doldurmak için pozitif, ama
        // DELETE_MIN_AGE'i aşmayan bir bekleme olmalı.
        let wait = delete_wait(snowflake_at(now_ms)).expect("taze mesaj ertelenmeli");
        assert!(
            wait > Duration::ZERO && wait <= DELETE_MIN_AGE,
            "bekleme {wait:?} beklenen aralık dışında"
        );

        // Yaşlı mesaj: ek gecikme olmamalı.
        assert!(delete_wait(snowflake_at(now_ms - 60_000)).is_none());
    }
}

// =====================================================================================
// MESAJ AKIŞI
// =====================================================================================

/// Taklit için kullanılan webhook adı (kanalda tek bir tane tutulup yeniden kullanılır).
const WEBHOOK_NAME: &str = "Botgek Native Embed";

/// Kanaldaki taklit webhook'u (kimlik + token).
#[derive(Debug, Clone, PartialEq, Eq)]
struct NamedWebhook {
    id: WebhookId,
    token: String,
}

/// Taklit webhook'unun hedefi.
///
/// Alt başlıklarda (`thread`, forum gönderileri) webhook **üst kanalda** tutulur: Discord bu
/// kanallarda webhook oluşturmayı desteklemez, bu yüzden webhook üst kanalda aranır/oluşturulur
/// ve mesaj `thread_id` ile alt başlığa gönderilir. Aksi hâlde taklit gönderimi reddedilir ve
/// kart bot adına gönderiliyordu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WebhookTarget {
    /// Webhook'un sahibi olan kanal (alt başlıklarda üst kanal: metin / duyuru / forum).
    channel_id: ChannelId,
    /// Mesajın gönderileceği alt başlık (forum gönderisi dâhil).
    thread_id: Option<ChannelId>,
}

/// Veri çekilirken başlatılan webhook çözümlemesinin sonucu.
///
/// Sonuç gönderim aşamasına taşınır; böylece soğuk thread/forum kanallarında hedef kanal
/// ikinci kez REST'ten çözülmez ve warmup ile gerçek gönderim arasında yarış oluşmaz.
type WebhookWarmup = tokio::task::JoinHandle<(WebhookTarget, Option<NamedWebhook>)>;

impl WebhookTarget {
    /// Kanal türü ve üst kanalına göre hedefi belirler.
    ///
    /// Üst kanalı bilinmeyen bir alt başlıkta eski davranışa düşülür (kanalın kendisi denenir).
    fn new(channel_id: ChannelId, kind: ChannelType, parent_id: Option<ChannelId>) -> Self {
        match (is_thread_kind(kind), parent_id) {
            (true, Some(parent)) => Self {
                channel_id: parent,
                thread_id: Some(channel_id),
            },
            _ => Self {
                channel_id,
                thread_id: None,
            },
        }
    }
}

/// Kanal türü bir alt başlık mı?
///
/// Forum gönderileri Discord'da `PublicThread` olarak gelir; duyuru alt başlıkları ve kapalı
/// alt başlıklar da aynı kuralla üst kanallarında webhook kullanır.
fn is_thread_kind(kind: ChannelType) -> bool {
    matches!(
        kind,
        ChannelType::NewsThread | ChannelType::PublicThread | ChannelType::PrivateThread
    )
}

/// Mesajın bulunduğu kanalın taklit hedefini çözer.
///
/// Önbellek çoğu durumda yeterli (kanallar ve üst kanal kimlikleri önbellekte durur); soğuk
/// önbellekte kanal REST'ten okunur.
async fn webhook_target(ctx: &Context, msg: &Message) -> WebhookTarget {
    let channel_id = msg.channel_id;
    let cached = msg
        .guild_id
        .and_then(|guild_id| ctx.cache.guild(guild_id))
        .and_then(|guild| {
            // Alt başlıklar Discord'dan `threads` listesinde, normal kanallar `channels`
            // haritasında gelir; ikisi de kontrol edilir.
            guild
                .channels
                .get(&channel_id)
                .or_else(|| guild.threads.iter().find(|thread| thread.id == channel_id))
                .map(|channel| (channel.kind, channel.parent_id))
        });
    if let Some((kind, parent_id)) = cached {
        return WebhookTarget::new(channel_id, kind, parent_id);
    }

    match ctx.http.get_channel(channel_id).await {
        Ok(Channel::Guild(channel)) => {
            WebhookTarget::new(channel_id, channel.kind, channel.parent_id)
        }
        Ok(_) => WebhookTarget {
            channel_id,
            thread_id: None,
        },
        Err(err) => {
            warn!(
                "Kanal bilgisi alınamadı ({}): {} — taklit webhook'u kanalın kendisinde denenecek",
                channel_id, err
            );
            WebhookTarget {
                channel_id,
                thread_id: None,
            }
        }
    }
}

/// Returns the effective NSFW flag for the message channel.
///
/// Thread channels inherit the flag from their parent. Cache data is preferred, with at most
/// one REST request for the channel and one for a missing parent. DM channels and channels that
/// cannot be verified as belonging to the message guild return `None` (fail closed).
async fn resolve_channel_nsfw(ctx: &Context, msg: &Message) -> Option<bool> {
    let guild_id = msg.guild_id?;
    let channel_id = msg.channel_id;

    let cached = ctx.cache.guild(guild_id).and_then(|guild| {
        guild
            .channels
            .get(&channel_id)
            .or_else(|| guild.threads.iter().find(|thread| thread.id == channel_id))
            .cloned()
            .filter(|channel| channel.guild_id == guild_id)
    });

    let channel = match cached {
        Some(channel) => channel,
        None => match ctx.http.get_channel(channel_id).await.ok()? {
            Channel::Guild(channel) if channel.guild_id == guild_id => channel,
            _ => return None,
        },
    };

    let Some(parent_id) = channel.parent_id.filter(|_| is_thread_kind(channel.kind)) else {
        return Some(channel.nsfw);
    };

    if let Some(parent) = ctx
        .cache
        .guild(guild_id)
        .and_then(|guild| guild.channels.get(&parent_id).cloned())
    {
        return Some(parent.nsfw);
    }

    match ctx.http.get_channel(parent_id).await.ok()? {
        Channel::Guild(parent) if parent.guild_id == guild_id => Some(parent.nsfw),
        _ => None,
    }
}

/// Kanal → webhook önbelleği. `None`: kanalda webhook kullanılamıyor (yetki yok veya DM);
/// her mesajda yeniden denenmez.
type WebhookCache = HashMap<ChannelId, (std::time::Instant, Option<NamedWebhook>)>;
static CHANNEL_WEBHOOKS: LazyLock<Mutex<WebhookCache>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Mesajı paylaşan kullanıcının görünen adı ve avatarı (webhook taklidi).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Impersonation {
    pub username: String,
    pub avatar_url: Option<String>,
}

impl Impersonation {
    /// Sunucu takma adı → global görünen ad → kullanıcı adı sırasıyla adı,
    /// kullanıcının avatarını seçer.
    pub fn from_message(msg: &Message) -> Self {
        let author = &msg.author;
        let raw_name = msg
            .member
            .as_ref()
            .and_then(|member| member.nick.clone())
            .filter(|nick| !nick.trim().is_empty())
            .or_else(|| {
                author
                    .global_name
                    .clone()
                    .filter(|name| !name.trim().is_empty())
            })
            .unwrap_or_else(|| author.name.clone());

        let avatar = author
            .avatar_url()
            // Discord webhook avatarlarında png/webp güvenilir: webp bağlantısını png'ye çevir.
            .map(|url| url.replace(".webp", ".png"));

        Self {
            username: sanitize_webhook_name(&raw_name),
            avatar_url: avatar,
        }
    }
}

/// Webhook adını Discord kurallarına uygun hâle getirir: `@` temizlenir, yasaklı
/// kelimeler nötrlenir, 80 karaktere kısaltılır.
pub fn sanitize_webhook_name(name: &str) -> String {
    let mut cleaned: String = name
        .chars()
        .filter(|&c| c != '@' && c != '#' && c != ':' && c != '`')
        .collect();
    let trimmed = cleaned.trim();

    let lower = trimmed.to_ascii_lowercase();
    if lower == "everyone" || lower == "here" || trimmed.is_empty() {
        return "Botgek".to_string();
    }

    cleaned = replace_case_insensitive(trimmed, "discord", "d!scord");
    cleaned = replace_case_insensitive(&cleaned, "clyde", "c!yde");

    let cleaned: String = cleaned.trim().chars().take(80).collect();
    if cleaned.trim().is_empty() {
        "Botgek".to_string()
    } else {
        cleaned
    }
}

fn replace_case_insensitive(haystack: &str, needle: &str, replacement: &str) -> String {
    let needle_lower = needle.to_ascii_lowercase();
    let mut result = String::with_capacity(haystack.len());
    let mut last_end = 0;
    let haystack_lower = haystack.to_ascii_lowercase();

    for (start, _) in haystack_lower.match_indices(&needle_lower) {
        result.push_str(&haystack[last_end..start]);
        let matched = &haystack[start..start + needle.len()];
        if matched.chars().next().is_some_and(|c| c.is_uppercase()) {
            let mut rep_chars = replacement.chars();
            if let Some(first) = rep_chars.next() {
                result.extend(first.to_uppercase());
                result.extend(rep_chars);
            } else {
                result.push_str(replacement);
            }
        } else {
            result.push_str(replacement);
        }
        last_end = start + needle.len();
    }
    result.push_str(&haystack[last_end..]);
    result
}

/// Taklit edilen mesajın orijinal yazarını **kalıcı** olarak kaydeder (SQLite).
///
/// Bu kayıt kartın **Sil** butonunun yetki kontrolü, "kartlara verilen yanıt AI'yı
/// tetiklemesin" kuralı ve bayrak emojisiyle çeviri ayrımı için kullanılır. Bot yeniden başlasa, çökse veya güncellense
/// de kayıt kaybolmaz (eskiden bellekteki bir `HashMap` içindeydi ve yeniden başlatmada
/// siliniyordu).
pub async fn remember_author(message_id: MessageId, channel_id: ChannelId, author: UserId) {
    crate::store::blocking(move || crate::store::remember_card(message_id, channel_id, author))
        .await;
}

/// Kart kaydını siler (kart Discord'dan silindiğinde).
pub async fn forget_author(message_id: MessageId) {
    crate::store::blocking(move || crate::store::forget_card(message_id)).await;
}

/// Taklit edilen mesajın orijinal yazarı (kartın **Sil** butonunun izni bu bilgiyle verilir).
pub async fn original_author(message_id: MessageId) -> Option<UserId> {
    crate::store::blocking(move || crate::store::card_author(message_id)).await
}

/// Bu mesaj Botgek'in dönüştürdüğü bir kart mı? (native embed → AI tetiklenmez)
pub async fn is_native_message(message_id: MessageId) -> bool {
    crate::store::blocking(move || crate::store::is_card(message_id)).await
}

/// Kayıttan bağımsız **yapısal kart imzası** (test edilebilir çekirdek).
///
/// Kartlar **yalnızca** Components V2 olarak gönderilir: `content` alanı boştur ve bileşenler
/// bu crate'te tip olarak yoktur. Bu düşük seviyeli yardımcı, çağıranın ayrıca Components V2
/// bayrağını doğrulamasına uygun olarak yalnızca Botgek kimliğiyle birleşen yapısal alanları
/// kontrol eder; webhook kimliği tek başına güvenilir bir marker değildir.
pub fn has_card_signature(
    author_id: UserId,
    from_webhook: bool,
    content: &str,
    embeds: &[Embed],
    bot_id: UserId,
) -> bool {
    // A webhook can be created by any integration, so an empty webhook message is not
    // evidence that Botgek sent a card. Webhook cards are recognized only through their
    // persistent message-id registry in `is_card_message` below.
    !from_webhook && author_id == bot_id && content.trim().is_empty() && embeds.is_empty()
}

/// Bu mesaj Botgek'in dönüştürme kartı mı?
///
/// Kartlara verilen yanıtlar AI sohbetini **tetiklememelidir**. Tanıma iki katmanlıdır;
/// ilki bellekteki kayıttan bağımsızdır, çünkü kart kimliği kaydı bot yeniden
/// başladığında boşalır ve o durumda eski kartlara gelen yanıtlar AI'yı tetikliyordu:
///
/// 1. Yapısal imza ([`has_card_signature`]) — içeriksiz, embedsiz kart mesajı.
/// 2. Bu oturumda gönderilen kartların kimlik kaydı (taklit webhook'unda yazar bot
///    olmadığı için kesin sonucu bu kayıt verir). Kayıt kalıcıdır (SQLite).
pub async fn is_card_message(msg: &Message, bot_id: UserId) -> bool {
    if msg.author.id != bot_id && msg.webhook_id.is_none() {
        return false;
    }
    if is_native_message(msg.id).await {
        return true;
    }

    // The Components V2 flag plus this bot's author identity is a reliable structural
    // marker. A webhook's identity is not reliable here (the message only exposes that it
    // came from *some* webhook), so unregistered webhook messages must stay unclassified.
    if let Some(flags) = msg.flags
        && (flags.bits() & v2::COMPONENTS_V2_FLAG != 0)
        && msg.author.id == bot_id
        && has_card_signature(msg.author.id, false, &msg.content, &msg.embeds, bot_id)
    {
        return true;
    }

    // No Components V2 flag and no persistent card record means there is no reliable card
    // marker. In particular, do not classify arbitrary empty bot/webhook messages.
    false
}

// -------------------------------------------------------------------------------------
// TAKLİT WEBHOOK'U
// -------------------------------------------------------------------------------------

/// Taklit webhook'unu döner: önbellekte yoksa botun kendi webhook'unu arar,
/// hiç yoksa oluşturur. `refresh` önbelleği zorla yeniler (eskimiş token için).
///
/// `channel_id` webhook'un **sahibi** olan kanaldır ([`WebhookTarget::channel_id`]): alt
/// başlıklarda webhook oluşturulamadığı için orada üst kanal verilir.
async fn channel_webhook(
    ctx: &Context,
    client: &reqwest::Client,
    config: &Config,
    channel_id: ChannelId,
    refresh: bool,
) -> Option<NamedWebhook> {
    let _single_flight = coordination::lock(format!("webhook:{channel_id}")).await;
    {
        let mut cache = CHANNEL_WEBHOOKS.lock().await;
        cache.retain(|_, (created, hook)| {
            created.elapsed() < Duration::from_secs(if hook.is_some() { 600 } else { 60 })
        });
        if !refresh && let Some(entry) = cache.get(&channel_id) {
            return entry.1.clone();
        }
    }
    CHANNEL_WEBHOOKS.lock().await.remove(&channel_id);

    match resolve_webhook(ctx, client, config, channel_id).await {
        Ok(Some(webhook)) => {
            cache_webhook(channel_id, Some(webhook.clone())).await;
            Some(webhook)
        }
        Ok(None) => {
            warn!(
                "Taklit webhook'u için token alınamadı ({}): mesaj bot adına gönderilecek",
                channel_id
            );
            cache_webhook(channel_id, None).await;
            None
        }
        Err(err) => {
            warn!(
                "Taklit webhook'u kullanılamıyor ({}): {} — mesaj bot adına gönderilecek",
                channel_id, err
            );
            // Kalıcı ret (yetki yok veya DM kanalı) önbelleğe yazılır; geçici ağ hatalarında
            // bir sonraki mesajda yeniden denenir.
            if matches!(err, raw::DiscordError::Rejected(_)) {
                cache_webhook(channel_id, None).await;
            }
            None
        }
    }
}

/// Kanaldaki bot webhook'unu bulur; yoksa `WEBHOOK_NAME` adıyla oluşturur.
async fn cache_webhook(channel: ChannelId, hook: Option<NamedWebhook>) {
    let mut cache = CHANNEL_WEBHOOKS.lock().await;
    if cache.len() >= 1024
        && let Some(oldest) = cache
            .iter()
            .min_by_key(|(_, value)| value.0)
            .map(|(id, _)| *id)
    {
        cache.remove(&oldest);
    }
    cache.insert(channel, (std::time::Instant::now(), hook));
}

async fn resolve_webhook(
    ctx: &Context,
    client: &reqwest::Client,
    config: &Config,
    channel_id: ChannelId,
) -> Result<Option<NamedWebhook>, raw::DiscordError> {
    let hooks = raw::list_channel_webhooks(client, &config.discord_token, channel_id).await?;
    let bot_id = ctx.cache.current_user().id.get();

    // Yalnızca bu botun oluşturduğu, token'ı olan gelen (incoming) webhook'lar yeniden kullanılır.
    let existing = hooks.iter().find(|hook| {
        hook.get("type").and_then(|kind| kind.as_u64()) == Some(1)
            && hook
                .get("user")
                .and_then(|user| user.get("id"))
                .and_then(|id| id.as_str())
                == Some(bot_id.to_string().as_str())
    });

    let created;
    let value = match existing {
        Some(hook) => hook,
        None => {
            created = raw::create_channel_webhook(
                client,
                &config.discord_token,
                channel_id,
                WEBHOOK_NAME,
            )
            .await?;
            &created
        }
    };

    match raw::webhook_credentials(value) {
        Some((id, token)) => Ok(Some(NamedWebhook { id, token })),
        None => Err(raw::DiscordError::Unavailable),
    }
}

/// Webhook üzerinden ham gövde gönderir; token eskimişse bir kez yenileyip tekrar dener.
///
/// Mesajın bulunduğu kanal bir alt başlık veya forum gönderisiyse webhook **üst kanalda**
/// çözümlenir ve gönderim `thread_id` ile o alt başlığa yapılır (bkz. [`webhook_target`]).
#[allow(clippy::too_many_arguments)]
async fn send_via_webhook(
    ctx: &Context,
    client: &reqwest::Client,
    config: &Config,
    msg: &Message,
    payload: &serde_json::Value,
    files: &[serenity::all::CreateAttachment],
    warmup: Option<WebhookWarmup>,
    wait: Duration,
) -> Result<MessageId, raw::DiscordError> {
    let mut last_error = raw::DiscordError::Unavailable;
    let (target, mut warmed) = match warmup {
        Some(warmup) => match tokio::time::timeout(wait, warmup).await {
            Ok(Ok(result)) => result,
            Ok(Err(err)) => {
                warn!(
                    "Webhook warmup görevi başarısız oldu: {err}; gönderim sırasında yeniden çözümlenecek"
                );
                let target = webhook_target(ctx, msg).await;
                let webhook = channel_webhook(ctx, client, config, target.channel_id, false).await;
                (target, webhook)
            }
            Err(_) => {
                // Veri hazır ama webhook hâlâ çözümleniyor: kartı bekletmek yerine bot adına
                // gönder. Warmup görevi arka planda sürer ve sonucu önbelleğe yazar, böylece
                // sonraki kart yeniden çözümleme maliyeti ödemez.
                info!(
                    "Taklit webhook'u {} ms içinde hazır olmadı; kart bot adına gönderilecek ({})",
                    wait.as_millis(),
                    msg.id
                );
                return Err(raw::DiscordError::Unavailable);
            }
        },
        None => {
            let target = webhook_target(ctx, msg).await;
            let webhook = channel_webhook(ctx, client, config, target.channel_id, false).await;
            (target, webhook)
        }
    };

    for attempt in 0..2 {
        let webhook = if attempt == 0 {
            // Warmup başarısızsa (ör. geçici ağ hatası) mevcut davranışı koru ve gönderim
            // anında bir kez daha çözümle; kalıcı negatif önbellek varsa bu yalnızca ucuz bir
            // cache hit olur.
            match warmed.take() {
                Some(webhook) => Some(webhook),
                None => channel_webhook(ctx, client, config, target.channel_id, false).await,
            }
        } else {
            channel_webhook(ctx, client, config, target.channel_id, true).await
        };
        let Some(webhook) = webhook else {
            return Err(last_error);
        };

        match raw::execute_webhook(
            client,
            webhook.id,
            &webhook.token,
            target.thread_id,
            payload,
            files,
        )
        .await
        {
            Ok(Some(message_id)) => return Ok(message_id),
            Ok(None) => return Err(raw::DiscordError::Ambiguous),
            Err(err) if err.expired_webhook() => last_error = err,
            Err(err) => return Err(err),
        }
    }

    // İki denemede de başarısız: önbelleği bırak, sonraki mesajda yeniden çözümlenir.
    CHANNEL_WEBHOOKS.lock().await.remove(&target.channel_id);
    Err(last_error)
}

// -------------------------------------------------------------------------------------
// GÖNDERİM
// -------------------------------------------------------------------------------------

/// Taklit (webhook) kullanılmalı mı?
///
/// Discord webhook mesajları **yanıt olamaz**; kullanıcı linki yanıt olarak attıysa ve
/// `impersonate_skip_replies` açıksa mesaj bot adına gönderilir ki yanıt bağlantısı korunsun.
fn should_impersonate(engine: &EngineSettings, is_reply: bool) -> bool {
    engine.impersonate_user && !(is_reply && engine.impersonate_skip_replies)
}

/// Yanıt bağlantısı: bot, kullanıcının yanıt verdiği **aynı mesaja** yanıt verir (proxy
/// motoruyla aynı davranış). `fail_if_not_exists: false` sayesinde referans mesaj
/// silinmişse de mesaj gönderilir.
fn reply_reference(msg: &Message) -> Option<serde_json::Value> {
    let reference = msg.message_reference.as_ref()?;
    let message_id = reference.message_id?;

    let mut value = serde_json::json!({
        "message_id": message_id.to_string(),
        "channel_id": reference.channel_id.to_string(),
        "fail_if_not_exists": false,
    });
    if let Some(guild_id) = reference.guild_id {
        value["guild_id"] = serde_json::json!(guild_id.to_string());
    }

    Some(value)
}

/// Gönderenin istemcisi bir mesajı gönderirken onu yerelde (`nonce` ile) hemen çizer ve
/// sunucudaki kopyayla eşleştirince "onaylar". Mesaj bu eşleştirme penceresi içinde
/// silinirse, silme olayı istemcide bir kopyayla eşleşmez ve mesaj yalnızca gönderenin
/// ekranında "hayalet" (ghost) olarak kalır (bilinen Discord istemci davranışı —
/// discord-api-docs #850). Silme bu yüzden mesaj en az bu yaşa gelene kadar ertelenir.
const DELETE_MIN_AGE: Duration = Duration::from_millis(1200);

/// Orijinal mesajı **arka planda** siler: REST silme isteği kart gönderimini/dönüşümü
/// bloklamaz. Silme [`delete_user_message`] üzerinden yapılır; o da mesajın
/// [`DELETE_MIN_AGE`] yaşını doldurmasını bekler (hayalet mesaj önlemi).
pub fn spawn_delete(ctx: &Context, msg: &Message) {
    let ctx = ctx.clone();
    let msg = msg.clone();
    tokio::spawn(async move { delete_user_message(&ctx, &msg).await });
}

/// Orijinal mesajı siler; zaten silinmişse sessizce geçer (kartın **Sil** butonu da aynı
/// mesajı silebilir, bu yüzden mükerrer silme denemesi hata üretmez).
///
/// Silmeden **önce** mesajın en az [`DELETE_MIN_AGE`] yaşında olması beklenir; bu,
/// gönderenin istemcisinin optimistic (nonce'lu) mesajını sunucu kopyasıyla
/// eşleştirmesine zaman tanır ve "hayalet mesaj" oluşumunu engeller. Dönüşüm bu süreyi
/// zaten aştıysa ek gecikme olmaz.
async fn delete_user_message(ctx: &Context, msg: &Message) {
    if let Some(wait) = delete_wait(msg.id) {
        tokio::time::sleep(wait).await;
    }
    if let Err(err) = msg.delete(&ctx.http).await {
        debug!("Orijinal mesaj silinemedi (zaten silinmiş olabilir): {err:?}");
    }
}

/// Discord snowflake epoch (2015-01-01T00:00:00Z), milisaniye cinsinden.
const DISCORD_EPOCH_MS: i64 = 1_420_070_400_000;

/// "Hayalet penceresi"ni güvenle aşmak için beklenecek kalan süre; mesaj zaten
/// yeterince yaşlıysa `None` (ek gecikme yok). Yaş, mesaj kimliğinin (snowflake)
/// içerdiği oluşturulma anından ölçülür; böylece `Timestamp`'in chrono/time arka
/// ucundan bağımsız ve milisaniye hassasiyetinde kalır.
fn delete_wait(id: MessageId) -> Option<Duration> {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let created_ms = (id.get() >> 22) as i64 + DISCORD_EPOCH_MS;
    let age_ms = now_ms - created_ms;
    if age_ms < DELETE_MIN_AGE.as_millis() as i64 {
        Some(Duration::from_millis(
            (DELETE_MIN_AGE.as_millis() as i64 - age_ms) as u64,
        ))
    } else {
        None
    }
}

/// Taklit webhook'unu **veri çekmeyle paralel** önceden çözer (önbelleğe yazar).
///
/// Soğuk kanalda webhook çözümlemesi 1-2 REST çağrısıdır (listele + gerekirse oluştur) ve
/// eskiden gönderim anında bekleniyordu; artık dönüşümün kritik yolundan çıkar: kart
/// gönderilirken webhook hazır olur. Başarısızlık önbelleğe yazılır ([`channel_webhook`]),
/// bu yüzden gönderim yine tek denemede bot mesajına düşer.
fn spawn_webhook_warmup(
    ctx: &Context,
    client: &reqwest::Client,
    config: &Config,
    msg: &Message,
) -> WebhookWarmup {
    let ctx = ctx.clone();
    let client = client.clone();
    let config = config.clone();
    let msg = msg.clone();
    tokio::spawn(async move {
        let target = webhook_target(&ctx, &msg).await;
        let webhook = channel_webhook(&ctx, &client, &config, target.channel_id, false).await;
        (target, webhook)
    })
}

/// Native dönüşümün sonucu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandleResult {
    /// Kart gönderildi.
    Sent,
    /// Kaynak çözümlenemedi veya Discord isteği kesin olarak reddedildi; proxy yedeği denenebilir.
    Failed,
    /// Native iş kapasitesi dolu; bu istek için kart üretilmedi.
    Busy,
    /// Gönderim sonucu belirsiz (ağ hatası / yanıtsız POST). Yeniden denemek mükerrer mesaj
    /// oluşturabileceği için proxy yedeği de denenmez.
    Ambiguous,
}

/// Native embed akışı. `Failed` dönerse çağıran taraf (ayar açıksa) proxy motoruna düşer.
pub async fn handle(
    ctx: &Context,
    msg: &Message,
    client: &reqwest::Client,
    parsed: &ParsedPost,
    config: &Config,
    engine: &EngineSettings,
) -> HandleResult {
    // Süre ölçümü: dönüşüm gecikmesi veri / medya / gönderim olarak log'lanır.
    static SLOTS: LazyLock<tokio::sync::Semaphore> =
        LazyLock::new(|| tokio::sync::Semaphore::new(16));
    let Ok(_slot) = SLOTS.try_acquire() else {
        crate::telemetry::OVERLOADS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        warn!("Native conversion capacity reached; original message preserved");
        return HandleResult::Busy;
    };
    let started = std::time::Instant::now();

    // Taklit (webhook) kimliğinin çözümlenmesi **veri çekmeyle paralel** yürür: ikisi de REST
    // çağrısıdır ve eskiden dönüşümün kritik yolunda sırayla bekleniyordu.
    let identity = (!msg.author.bot && should_impersonate(engine, msg.message_reference.is_some()))
        .then(|| Impersonation::from_message(msg));
    let webhook_warmup = identity
        .as_ref()
        .map(|_| spawn_webhook_warmup(ctx, client, config, msg));

    // Orijinal mesaj burada **silinmez**: silme, kart gönderildikten sonra yapılır (native
    // akışta `finish()`, hata durumunda proxy akışı). Erken silmek, gönderenin istemcisi
    // optimistic (nonce'lu) mesajını sunucu kopyasıyla eşleştirmeden "hayalet mesaj"
    // bıraktığı için artık kullanılmaz (bkz. [`delete_user_message`]).
    let post = match fetch(client, parsed, engine, config).await {
        Ok(post) => post,
        Err(err) => {
            warn!(
                "Native embed verisi alınamadı ({}): {}",
                parsed.original_url, err
            );
            crate::telemetry::record(platform_key(parsed.platform), false, started.elapsed());
            return HandleResult::Failed;
        }
    };
    let fetched = started.elapsed();

    // Source/channel visibility is resolved per send. Never modify the cached Arc: the same
    // source post may be rendered in both NSFW and non-NSFW channels.
    let channel_nsfw = resolve_channel_nsfw(ctx, msg).await;
    let post = render::apply_visibility(
        &post,
        parsed.is_spoiler,
        engine.sensitive_mode,
        channel_nsfw,
    );

    let platform = platform_key(parsed.platform);
    let options = render::RenderOptions::from_engine(engine, platform);
    debug!(
        "Kart: V2 (tek mod: doğrudan; video indirme: {}) — {}",
        options.video_requires_download, parsed.original_url
    );
    let plan = render::plan_media(&post, &options);
    let resolved = render::resolve_media(client, &post, &plan, &options).await;
    let media_done = started.elapsed();

    debug!(
        "Hazırlık: veri {} ms, medya {} ms ({})",
        fetched.as_millis(),
        (media_done - fetched).as_millis(),
        parsed.original_url
    );

    // Hedefe uymak için veri fazı, gönderime ayrılan payı yememeli. Yiyorsa nedeni
    // log'da görünsün: kaynak yavaş mı, yoksa medya mı bekledi?
    if media_done > LATENCY_TARGET - SEND_RESERVE {
        warn!(
            "Veri fazı {} ms sürdü (veri {} ms, medya {} ms; gönderime kalan pay {} ms): {}",
            media_done.as_millis(),
            fetched.as_millis(),
            (media_done - fetched).as_millis(),
            LATENCY_TARGET.saturating_sub(media_done).as_millis(),
            parsed.original_url
        );
    }

    // Webhook çözümlemesi veri çekimiyle paralel başladı; hâlâ sürüyorsa **kalan bütçe**
    // kadar beklenir, gerisi kartı geciktirmemek için bot adına gönderilir.
    let webhook_wait = webhook_wait(media_done);

    // Kart **yalnızca** Components V2 olarak gönderilir (klasik embed yedeği yoktur).
    match send_components_v2(
        ctx,
        msg,
        client,
        config,
        parsed,
        &post,
        &options,
        &resolved,
        &engine.emojis,
        identity.as_ref(),
        webhook_warmup,
        webhook_wait,
    )
    .await
    {
        Ok((message_id, impersonated)) => {
            let total = started.elapsed();
            crate::telemetry::record(platform_key(parsed.platform), true, total);
            info!(
                "Mesaj Components V2 kart olarak dönüştürüldü ({:?}, taklit: {}, veri {} ms, medya {} ms, gönderim {} ms, toplam {} ms{}): {}",
                parsed.platform,
                impersonated,
                fetched.as_millis(),
                (media_done - fetched).as_millis(),
                (total - media_done).as_millis(),
                total.as_millis(),
                if total > LATENCY_TARGET {
                    " — hedef 1 sn aşıldı"
                } else {
                    ""
                },
                parsed.original_url
            );
            finish(ctx, msg, config, message_id).await;
            HandleResult::Sent
        }
        Err(err) => {
            error!("Components V2 kartı gönderilemedi: {err}");
            crate::telemetry::record(platform_key(parsed.platform), false, started.elapsed());
            // A lost POST response may already have created a message. Do not proxy-resend.
            if err.safe_fallback() {
                HandleResult::Failed
            } else {
                HandleResult::Ambiguous
            }
        }
    }
}

/// V2 kartı için `allowed_mentions` gövdesi.
///
/// Kart bot adına gönderildiğinde (`tag_sender`) paylaşanın etiketi kartın üstünde `<@id>`
/// olarak durur; **ping atmaması** için izinli kullanıcı listesinden çıkarılır (Discord
/// böylece etiketi düz `@kullanıcı` metni olarak gösterir — proxy/klasik akıştaki davranış).
fn card_allowed_mentions(msg: &Message, tag_sender: bool) -> Option<serde_json::Value> {
    let mut value = serde_json::to_value(build_selective_allowed_mentions(msg)).ok()?;
    if !tag_sender {
        return Some(value);
    }

    let author = msg.author.id.to_string();
    if let Some(users) = value
        .get_mut("users")
        .and_then(serde_json::Value::as_array_mut)
    {
        users.retain(|entry| entry.as_str() != Some(author.as_str()));
    }

    Some(value)
}

/// V2 kart gövdesini üretir (yanıt bağlantısı dahil).
///
/// `sender` verilirse kartın üstünde paylaşanın etiketi durur ve [`card_allowed_mentions`]
/// sayesinde ping atmaz; `impersonation` verilirse mesaj kullanıcının adıyla gider (taklit).
#[allow(clippy::too_many_arguments)]
fn v2_payload(
    msg: &Message,
    parsed: &ParsedPost,
    post: &BuiltinPost,
    options: &render::RenderOptions,
    resolved: &render::ResolvedMedia,
    emojis: &EmojiSettings,
    impersonation: Option<&Impersonation>,
    sender: Option<UserId>,
) -> serde_json::Value {
    let mut payload = v2::build_payload(&v2::V2Card {
        post,
        options,
        resolved,
        emojis,
        spoiler: parsed.is_spoiler,
        sender,
        user_comment: parsed.user_comment.as_deref(),
        impersonation,
        allowed_mentions: card_allowed_mentions(msg, sender.is_some()),
    });

    // Yanıt bağlantısı: kullanıcı bir mesaja yanıt verdiyse bot da aynı mesaja yanıt verir.
    if let Some(reference) = reply_reference(msg) {
        payload["message_reference"] = reference;
    }

    payload
}

/// Components V2 kartını gönderir. Taklit istenmişse önce webhook denenir, olmazsa
/// mesaj bot adına gönderilir (kart yine tek parça kalır).
#[allow(clippy::too_many_arguments)]
async fn send_components_v2(
    ctx: &Context,
    msg: &Message,
    client: &reqwest::Client,
    config: &Config,
    parsed: &ParsedPost,
    post: &BuiltinPost,
    options: &render::RenderOptions,
    resolved: &render::ResolvedMedia,
    emojis: &EmojiSettings,
    identity: Option<&Impersonation>,
    warmup: Option<WebhookWarmup>,
    webhook_wait: Duration,
) -> Result<(MessageId, bool), raw::DiscordError> {
    if let Some(identity) = identity {
        // Webhook mesajları Discord'da yanıt olamaz (dokümante edilmemiş alan yok sayılır):
        // bu yüzden webhook denenir; başarısız olursa mesaj bot adına (yanıtla) gönderilir.
        let payload = v2_payload(
            msg,
            parsed,
            post,
            options,
            resolved,
            emojis,
            Some(identity),
            None,
        );
        match send_via_webhook(
            ctx,
            client,
            config,
            msg,
            &payload,
            &resolved.attachments,
            warmup,
            webhook_wait,
        )
        .await
        {
            Ok(message_id) => return Ok((message_id, true)),
            Err(err) if err.safe_fallback() => warn!(
                "Components V2 taklit gönderimi başarısız ({}): {}",
                msg.id, err
            ),
            Err(err) => return Err(err),
        }
    }

    // Kart bot adına gider (taklit kapalı ya da webhook yok): paylaşanın etiketi kartın üstüne
    // konur, böylece linki kimin attığı görünür.
    let payload = v2_payload(
        msg,
        parsed,
        post,
        options,
        resolved,
        emojis,
        None,
        Some(msg.author.id),
    );
    let message_id = raw::send_channel_message(
        client,
        &config.discord_token,
        msg.channel_id,
        &payload,
        &resolved.attachments,
    )
    .await?;
    Ok((message_id, false))
}
/// Kartın **Sil** butonunun izni ve "AI tetiklenmesin" kuralı mesaj metninden bağımsız çalışır.
///
/// Silme **arka planda** yapılır: kart kullanıcıya çoktan görünmüştür, REST çağrısı dönüşüm
/// süresine eklenmez (ve mesaj işleme döngüsü serbest kalır). Silme, kart gönderildikten
/// **sonra** ve [`DELETE_MIN_AGE`] kadar gecikmeyle yapılır (hayalet mesaj önlemi).
async fn finish(ctx: &Context, msg: &Message, config: &Config, message_id: MessageId) {
    remember_author(message_id, msg.channel_id, msg.author.id).await;

    let ctx = ctx.clone();
    let msg = msg.clone();
    let delete_original = config.delete_original;
    tokio::spawn(async move {
        // Kart gönderildi; şimdi orijinal mesajı sil (en az DELETE_MIN_AGE yaşına gelene
        // kadar bekleyerek: gönderenin istemcisi optimistic mesajını eşleştirebilsin).
        if delete_original {
            delete_user_message(&ctx, &msg).await;
        }
    });
}

/// Discord'un 4xx yanıtı kalıcı bir ret anlamına gelir (ağ hatasından farklı olarak
/// tekrar denemek anlamsızdır).
#[cfg(test)]
fn is_discord_rejection(error: &str) -> bool {
    if error.starts_with("Discord 429") || error.starts_with("Discord 408") {
        return false;
    }
    error.starts_with("Discord 4")
}

#[cfg(test)]
mod message_tests {
    use super::*;

    /// Bütçe paylaşımı 1 saniyelik hedefi **kendi içinde** tutmalı: isteğe bağlı
    /// zenginleştirme + Discord POST payı hedefi aşamaz, webhook beklemesi de kalan
    /// bütçeden fazlasını isteyemez.
    #[test]
    fn latency_budgets_fit_inside_the_one_second_target() {
        assert!(
            FETCH_ENRICH_BUDGET + SEND_RESERVE <= LATENCY_TARGET,
            "zenginleştirme {} ms + gönderim {} ms > hedef {} ms",
            FETCH_ENRICH_BUDGET.as_millis(),
            SEND_RESERVE.as_millis(),
            LATENCY_TARGET.as_millis()
        );
        assert!(FETCH_ENRICH_BUDGET < FETCH_FIRST_DATA_BUDGET);

        // Veri fazı bütçeyi tam yese bile webhook beklemesi hedefi aşmaz.
        let full = FETCH_ENRICH_BUDGET + webhook_wait(FETCH_ENRICH_BUDGET);
        assert!(
            full + SEND_RESERVE <= LATENCY_TARGET,
            "zenginleştirme + webhook + gönderim hedefi aşıyor: {} ms",
            full.as_millis() + SEND_RESERVE.as_millis()
        );
    }

    /// Webhook beklemesi kalan bütçeye göre kısalır; geç kalan veri fazı hiç beklemez.
    #[test]
    fn webhook_wait_shrinks_with_the_remaining_budget() {
        assert_eq!(webhook_wait(Duration::ZERO), WEBHOOK_WARMUP_WAIT);
        assert!(webhook_wait(FETCH_ENRICH_BUDGET) <= WEBHOOK_WARMUP_WAIT);
        assert!(
            webhook_wait(FETCH_ENRICH_BUDGET) < webhook_wait(FETCH_ENRICH_BUDGET / 2),
            "veri fazı uzadıkça bekleme kısalmalı"
        );
        assert_eq!(webhook_wait(LATENCY_TARGET), Duration::ZERO);
        assert_eq!(webhook_wait(LATENCY_TARGET * 2), Duration::ZERO);
    }

    #[test]
    fn content_requirements_are_isolated_in_the_whole_post_cache_key() {
        let parsed = ParsedPost {
            platform: Platform::Twitter,
            original_url: "https://x.com/test/status/1".to_string(),
            fixed_url: "https://x.com/test/status/1".to_string(),
            is_spoiler: false,
            user_comment: None,
        };
        let full = EngineSettings::default();
        let mut minimal = full.clone();
        minimal.show_community_note = false;
        minimal.show_metrics = false;
        assert_ne!(
            post_cache_key(&parsed, &full),
            post_cache_key(&parsed, &minimal)
        );
        let minimal_key = post_cache_key(&parsed, &minimal);
        minimal.show_community_note = true;
        assert_ne!(minimal_key, post_cache_key(&parsed, &minimal));
        minimal.show_community_note = false;
        minimal.show_metrics = true;
        assert_ne!(minimal_key, post_cache_key(&parsed, &minimal));
    }

    #[test]
    fn is_discord_rejection_treats_transient_rates_and_timeouts_as_non_rejections() {
        assert!(is_discord_rejection("Discord 400 döndü: Bad Request"));

        assert!(is_discord_rejection(
            "Discord 403 döndü: Missing Permissions"
        ));
        assert!(is_discord_rejection("Discord 404 döndü: Unknown Webhook"));
        assert!(!is_discord_rejection(
            "Discord 429 döndü: Too Many Requests"
        ));
        assert!(!is_discord_rejection("Discord 408 döndü: Request Timeout"));
        assert!(!is_discord_rejection(
            "Discord 500 döndü: Internal Server Error"
        ));
    }

    #[test]
    fn card_signature_recognises_modern_cards() {
        let bot_id = UserId::new(1);
        let user_id = UserId::new(2);

        // Modern kart (Components V2): bot kimliğiyle ve içerik/embed yokluğuyla tanınır.
        assert!(
            has_card_signature(bot_id, false, "", &[], bot_id),
            "botun gönderdiği içeriksiz modern kart tanınmalı"
        );
        assert!(
            !has_card_signature(user_id, true, "", &[], bot_id),
            "rastgele webhook'un içeriksiz mesajı kart sayılmamalı"
        );
        assert!(
            !has_card_signature(bot_id, true, "", &[], bot_id),
            "webhook işaretli mesaj bot kimliğiyle de kart sayılmamalı"
        );

        // Kart olmayanlar: bunlara verilen yanıtlar AI'yı tetikleyebilmeli.
        assert!(
            !has_card_signature(bot_id, false, "Sen ne dedin öyle?", &[], bot_id),
            "botun AI yanıtı kart değildir"
        );
        assert!(!has_card_signature(user_id, false, "merhaba", &[], bot_id));
        assert!(
            !has_card_signature(user_id, true, "duyuru", &[], bot_id),
            "başka bir botun içerikli webhook mesajı kart değildir"
        );
    }

    #[tokio::test]
    async fn card_messages_are_recognised_without_any_registry_entry() {
        let bot_id = UserId::new(1);
        let user_id = UserId::new(2);
        let webhook_id = WebhookId::new(77);

        // Components V2 bayrağı olan bot kartı, kart kaydı hiç olmasa bile tanınmalı
        // (AI tetiklenmemesi bu sayede garanti).
        let card_id = MessageId::new(9_001);
        let card = components_v2_message_fixture(card_id, bot_id, "", vec![], None);
        assert!(
            !is_native_message(card_id).await,
            "bu kimlik için kayıt yok"
        );
        assert!(
            is_card_message(&card, bot_id).await,
            "içeriksiz modern kart tanınmalı"
        );

        // İçeriksiz bot mesajı Components V2 bayrağı taşımıyorsa kart değildir.
        let empty_bot = message_fixture(MessageId::new(9_007), bot_id, "", vec![], None);
        assert!(!is_card_message(&empty_bot, bot_id).await);

        // Kayıtsız bir webhook'un içeriksiz Components V2 mesajı kart sayılamaz; başka
        // entegrasyonların boş mesajları AI yanıtlarını susturmamalıdır.
        let impersonated = components_v2_message_fixture(
            MessageId::new(9_002),
            user_id,
            "",
            vec![],
            Some(webhook_id),
        );
        assert!(!is_card_message(&impersonated, bot_id).await);

        // Bayraksız boş webhook mesajı da kart değildir.
        let empty_webhook =
            message_fixture(MessageId::new(9_003), user_id, "", vec![], Some(webhook_id));
        assert!(!is_card_message(&empty_webhook, bot_id).await);

        // Kalıcı kayıt da tanımayı sağlar (kart kaydı kalıcıdır, yeniden başlatmada kaybolmaz).
        let recorded_id = MessageId::new(9_004);
        let recorded =
            message_fixture(recorded_id, user_id, "eski biçim", vec![], Some(webhook_id));
        assert!(!is_card_message(&recorded, bot_id).await);
        remember_author(recorded_id, ChannelId::new(5), user_id).await;
        assert!(is_card_message(&recorded, bot_id).await);

        // Botun AI yanıtı ve kullanıcı mesajı kart değildir (AI tetiklenebilmeli).
        let ai_reply = message_fixture(
            MessageId::new(9_005),
            bot_id,
            "Evet, haklısın.",
            vec![],
            None,
        );
        assert!(!is_card_message(&ai_reply, bot_id).await);
        let user_message = message_fixture(MessageId::new(9_006), user_id, "merhaba", vec![], None);
        assert!(!is_card_message(&user_message, bot_id).await);
    }

    /// Alt başlıklarda (thread) ve forum gönderilerinde taklit webhook'u **üst kanalda**
    /// tutulur; mesaj `thread_id` ile alt başlığa gönderilir. Discord alt başlıklarda webhook
    /// oluşturmayı desteklemediği için (forum gönderileri de birer alt başlıktır) hedefin üst
    /// kanal olması gerekir; aksi hâlde taklit gönderimi reddedilip kart bot adına gidiyordu.
    #[test]
    fn webhook_target_moves_threads_to_the_parent_channel() {
        let parent = ChannelId::new(100);
        let thread = ChannelId::new(200);

        for kind in [
            ChannelType::NewsThread,
            ChannelType::PublicThread,
            ChannelType::PrivateThread,
        ] {
            assert_eq!(
                WebhookTarget::new(thread, kind, Some(parent)),
                WebhookTarget {
                    channel_id: parent,
                    thread_id: Some(thread)
                },
                "alt başlıkta webhook üst kanalda olmalı ({kind:?})"
            );
        }

        // Forum gönderileri `PublicThread` olarak gelir: webhook forum kanalında, mesaj gönderide.
        let forum = ChannelId::new(300);
        assert_eq!(
            WebhookTarget::new(thread, ChannelType::PublicThread, Some(forum)),
            WebhookTarget {
                channel_id: forum,
                thread_id: Some(thread)
            }
        );

        // Normal kanallar (metin, duyuru, kategori, forum, ses): hedef kanalın kendisi.
        for kind in [
            ChannelType::Text,
            ChannelType::News,
            ChannelType::Category,
            ChannelType::Forum,
            ChannelType::Voice,
        ] {
            assert_eq!(
                WebhookTarget::new(parent, kind, Some(ChannelId::new(1))),
                WebhookTarget {
                    channel_id: parent,
                    thread_id: None
                },
                "normal kanalda hedef kanalın kendisi olmalı ({kind:?})"
            );
        }

        // Üst kanalı bilinmeyen alt başlıkta eski davranış korunur (kanalın kendisi denenir).
        assert_eq!(
            WebhookTarget::new(thread, ChannelType::PublicThread, None),
            WebhookTarget {
                channel_id: thread,
                thread_id: None
            }
        );
    }

    /// Discord gateway gövdesini taklit eden test mesajı.
    fn message_fixture(
        id: MessageId,
        author_id: UserId,
        content: &str,
        embeds: Vec<Embed>,
        webhook_id: Option<WebhookId>,
    ) -> Message {
        serde_json::from_value(serde_json::json!({
            "id": id.get().to_string(),
            "channel_id": "5",
            "author": {
                "id": author_id.get().to_string(),
                "username": "Botgek",
                "discriminator": "0001",
                "avatar": null,
                "bot": true
            },
            "webhook_id": webhook_id.map(|id| id.get().to_string()),
            "content": content,
            "timestamp": "2026-09-27T12:00:00.000000+00:00",
            "edited_timestamp": null,
            "tts": false,
            "mention_everyone": false,
            "mentions": [],
            "mention_roles": [],
            "attachments": [],
            "embeds": embeds,
            "pinned": false,
            "type": 0
        }))
        .expect("mesaj çözümlenmeli")
    }

    /// Components V2 bayrağı taşıyan test mesajı.
    fn components_v2_message_fixture(
        id: MessageId,
        author_id: UserId,
        content: &str,
        embeds: Vec<Embed>,
        webhook_id: Option<WebhookId>,
    ) -> Message {
        let mut message = message_fixture(id, author_id, content, embeds, webhook_id);
        message.flags = Some(serenity::all::MessageFlags::from_bits_retain(
            v2::COMPONENTS_V2_FLAG,
        ));
        message
    }

    #[test]
    fn webhook_names_are_sanitized_for_discord_rules() {
        assert_eq!(sanitize_webhook_name("  Eray  "), "Eray");
        assert_eq!(sanitize_webhook_name("@eray"), "eray");
        assert_eq!(sanitize_webhook_name(""), "Botgek");
        assert_eq!(sanitize_webhook_name("   "), "Botgek");
        assert_eq!(sanitize_webhook_name("Discord Clyde"), "D!scord C!yde");
        assert_eq!(sanitize_webhook_name("DISCORD CLYDE"), "D!scord C!yde");
        assert_eq!(sanitize_webhook_name("everyone"), "Botgek");
        assert_eq!(sanitize_webhook_name("@here"), "Botgek");
        assert_eq!(sanitize_webhook_name("User#1234:dev`"), "User1234dev");
        assert_eq!(sanitize_webhook_name(&"a".repeat(120)).chars().count(), 80);
    }

    #[test]
    fn bot_cards_do_not_ping_the_sender() {
        let mut message = message_fixture(MessageId::new(1), UserId::new(42), "link", vec![], None);
        // Discord kullanıcının kendini etiketlediği mesajlarda yazar `mentions` içinde gelir.
        message.mentions = vec![
            serde_json::from_value(serde_json::json!({
                "id": "42",
                "username": "eray",
                "discriminator": "0001",
                "avatar": null
            }))
            .expect("kullanıcı çözümlenmeli"),
        ];

        // Kart bot adına gidiyorsa etiket ping atmaz (yalnızca düz `@kullanıcı` görünür).
        let tagged = card_allowed_mentions(&message, true).unwrap();
        assert_eq!(
            tagged["users"],
            serde_json::json!([]),
            "paylaşan pinglenmemeli"
        );

        // Taklit (webhook) akışında kartın üstünde etiket yok → liste değişmez.
        let plain = card_allowed_mentions(&message, false).unwrap();
        assert_eq!(plain["users"], serde_json::json!(["42"]));
    }

    #[tokio::test]
    async fn remembered_authors_can_be_queried() {
        let message_id = MessageId::new(777);
        assert_eq!(original_author(message_id).await, None);
        remember_author(message_id, ChannelId::new(5), UserId::new(42)).await;
        assert_eq!(original_author(message_id).await, Some(UserId::new(42)));
        assert!(is_native_message(message_id).await);

        // Kayıt silinince kart tanıma da düşer (Sil butonuyla kart silindiğinde kullanılır).
        forget_author(message_id).await;
        assert_eq!(original_author(message_id).await, None);
        assert!(!is_native_message(message_id).await);
    }

    #[test]
    fn replies_keep_the_reply_context_instead_of_impersonating() {
        let mut engine = EngineSettings::default();

        assert!(
            should_impersonate(&engine, false),
            "yanıt değilse taklit kullanılır"
        );
        assert!(
            !should_impersonate(&engine, true),
            "yanıt olarak atılan link bot adına gönderilir (webhook yanıt veremez)"
        );

        // Ayar kapatılırsa yanıtlarda da taklit kullanılır (yanıt bağlantısı kaybolur).
        engine.impersonate_skip_replies = false;
        assert!(should_impersonate(&engine, true));

        // Taklit tamamen kapalıysa her durumda bot mesajı gider.
        engine.impersonate_user = false;
        assert!(!should_impersonate(&engine, false));
        assert!(!should_impersonate(&engine, true));
    }
}
