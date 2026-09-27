//! Bot içi (native) link dönüştürme motoru.
//!
//! Proxy motoru linki `fixupx.com` benzeri bir önizleme servisine çevirip Discord'un
//! kendi embed'ini üretmesini bekler. Bu motor ise gönderi verisini doğrudan kaynaktan
//! (X syndication/fxtwitter, Reddit JSON, Bluesky XRPC, TikTok oEmbed) çeker; başlık,
//! metin, metrikler, alıntı ve medyayı kendisi indirip `CreateEmbed` olarak gönderir.

pub mod bluesky;
pub mod media;
pub mod opengraph;
pub mod cache;
pub mod raw;
pub mod reddit;
pub mod render;
pub mod tiktok;
pub mod translate;
pub mod twitter;
pub mod v2;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use once_cell::sync::Lazy;
use serenity::all::{
    ChannelId, Context, CreateMessage, Message, MessageFlags, MessageId, ReactionType, UserId, WebhookId,
};
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use super::parser::{ParsedPost, Platform};
use crate::config::Config;
use crate::settings::{CardStyle, EmojiSettings, EngineSettings};
use crate::utils::mentions::build_selective_allowed_mentions;
use crate::utils::{truncate_chars, DISCORD_CONTENT_LIMIT};


/// Dış servis istekleri için zaman aşımı (mesaj gönderimini uzun süre bloklamamalı).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(12);
/// Video/görsel indirme için daha uzun zaman aşımı.
pub const MEDIA_TIMEOUT: Duration = Duration::from_secs(45);
/// Tek bir JSON/metin yanıtı için okunacak maksimum gövde boyutu (2 MB).
pub const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
/// Tanımlayıcı User-Agent: fxtwitter gibi servisler uygulamanın kendini tanıtmasını ister.
pub const USER_AGENT: &str = concat!("Botgek/", env!("CARGO_PKG_VERSION"), " (Discord link converter)");

/// Platform anahtarı (`twitter`, `reddit` ...) — ayarlarda ve loglarda kullanılır.
pub fn platform_key(platform: Platform) -> &'static str {
    match platform {
        Platform::Twitter => "twitter",
        Platform::TikTok => "tiktok",
        Platform::Instagram => "instagram",
        Platform::Reddit => "reddit",
        Platform::Bluesky => "bluesky",
    }
}

/// Embed'in sol kenar rengi (platform marka rengi).
pub fn platform_color(platform: Platform) -> u32 {
    match platform {
        Platform::Twitter => 0x1D9BF0,
        Platform::TikTok => 0xFE2C55,
        Platform::Instagram => 0xE1306C,
        Platform::Reddit => 0xFF4500,
        Platform::Bluesky => 0x1185FE,
    }
}

/// Gönderi sahibi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Author {
    pub name: String,
    /// `@` işareti olmadan kullanıcı adı (Twitter: screen_name, Bluesky: handle).
    pub handle: String,
    pub avatar: Option<String>,
    pub verified: bool,
    pub profile_url: Option<String>,
}

impl Author {
    pub fn new(name: impl Into<String>, handle: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            handle: handle.into(),
            avatar: None,
            verified: false,
            profile_url: None,
        }
    }

    /// `Ad (@kullanici)` biçiminde etiket üretir.
    pub fn label(&self) -> String {
        if self.handle.is_empty() {
            self.name.clone()
        } else {
            format!("{} (@{})", self.name, self.handle)
        }
    }
}

/// Etkileşim metrikleri (0 / `None` olanlar gösterilmez).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub replies: u64,
    pub reposts: u64,
    pub likes: u64,
    pub quotes: u64,
    pub views: Option<u64>,
}

impl Stats {
    pub fn is_empty(&self) -> bool {
        self.replies == 0 && self.reposts == 0 && self.likes == 0 && self.quotes == 0 && self.views.is_none()
    }
}

/// Medya türü.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Video,
    Gif,
}

/// Gönderiye ait tek bir medya öğesi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Media {
    pub kind: MediaKind,
    /// Doğrudan medya URL'i (görsel veya mp4/HLS).
    pub url: String,
    /// Videonun kapak görseli.
    pub poster: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Video süresi (saniye).
    pub duration: Option<u64>,
}

impl Media {
    pub fn is_video(&self) -> bool {
        matches!(self.kind, MediaKind::Video | MediaKind::Gif)
    }

    pub fn is_image(&self) -> bool {
        matches!(self.kind, MediaKind::Image)
    }
}

/// Otomatik çeviri sonucu (embed alt bilgisinde gösterilir).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Translation {
    /// Kaynak dil kodu (platform bildirmediyse `None`).
    pub source: Option<String>,
}

/// Motorun ürettiği platformdan bağımsız gönderi modeli (render katmanının girdisi).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltinPost {
    pub platform: Platform,
    /// Kullanıcının paylaştığı (orijinal) gönderi bağlantısı.
    pub url: String,
    pub author: Author,
    /// Reddit başlığı gibi opsiyonel başlık.
    pub title: Option<String>,
    pub text: String,
    /// Unix zaman damgası (Discord göreli zaman gösterimi için).
    pub created_at: Option<i64>,
    pub stats: Stats,
    pub media: Vec<Media>,
    /// Alıntılanan/yanıtlanan gönderi (tek seviye).
    pub quote: Option<Box<BuiltinPost>>,
    pub community_note: Option<String>,
    /// Hassas içerik (spoiler ile gösterilir).
    pub sensitive: bool,
    /// `r/rust • 128 yorum` gibi ek bilgi satırı.
    pub extra_label: Option<String>,
    /// Platformun bildirdiği gönderi dili (ISO 639-1, ör. `de`). Bilinmiyorsa `None`.
    pub language: Option<String>,
    /// Otomatik çeviri yapıldıysa çeviri bilgisi.
    pub translation: Option<Translation>,
}

impl BuiltinPost {
    pub fn new(platform: Platform, url: impl Into<String>, author: Author) -> Self {
        Self {
            platform,
            url: url.into(),
            author,
            title: None,
            text: String::new(),
            created_at: None,
            stats: Stats::default(),
            media: Vec::new(),
            quote: None,
            community_note: None,
            sensitive: false,
            extra_label: None,
            language: None,
            translation: None,
        }
    }

    /// Kapak olarak kullanılabilecek ilk görsel (video poster'ı dahil).
    pub fn poster_url(&self) -> Option<String> {
        self.media
            .iter()
            .find_map(|m| m.poster.clone())
            .or_else(|| self.media.iter().find(|m| m.is_image()).map(|m| m.url.clone()))
    }
}

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
        return Err(format!("Beklenmeyen durum kodu ({url}): {}", response.status()));
    }
    if let Some(len) = response.content_length() {
        if len > MAX_BODY_BYTES as u64 {
            return Err(format!("Yanıt çok büyük ({url}): {len} bayt"));
        }
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

/// Paylaşılan gönderinin verisini, platforma uygun kaynaktan çeker.
/// Platforma göre gönderi verisini çeker. **Önbellek önce** denenir: aynı link kısa süre
/// içinde tekrar paylaşıldığında ağ isteği yapılmaz (dönüşüm anında tamamlanır).
pub async fn fetch(
    client: &reqwest::Client,
    parsed: &ParsedPost,
    engine: &EngineSettings,
) -> Result<BuiltinPost, String> {
    // Sonucu etkileyen ayarlar anahtara dahil edilir (kaynak + çeviri).
    let key = format!(
        "{}|{}|{}",
        parsed.original_url,
        engine.twitter_source.key(),
        engine.auto_translate
    );

    if let Some(cached) = cache::get(&key).await {
        debug!("Gönderi önbellekten alındı: {}", parsed.original_url);
        return Ok((*cached).clone());
    }

    let post = match parsed.platform {
        Platform::Twitter => twitter::fetch(client, parsed, engine).await,
        Platform::Bluesky => bluesky::fetch(client, parsed).await,
        Platform::Reddit => reddit::fetch(client, parsed).await,
        Platform::TikTok => tiktok::fetch(client, parsed).await,
        Platform::Instagram => opengraph::fetch(client, parsed).await,
    }?;

    cache::put(key, &post).await;
    Ok(post)
}

// =====================================================================================
// ORTAK METİN / ZAMAN YARDIMCILARI
// =====================================================================================

/// Gönderi metninin sonundaki medya kısa linklerini (`t.co` / `pic.x.com`) temizler.
/// Twitter, metnin sonuna görsel/video için kısa link ekler; bunlar embed'de gereksizdir.
pub fn strip_trailing_media_links(text: &str) -> String {
    let mut cleaned = text.trim().to_string();
    loop {
        let last = match cleaned.split_whitespace().last() {
            Some(last) => last.to_string(),
            None => break,
        };
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

    if raw.len() >= 19 && raw.as_bytes().get(4) == Some(&b'-') && raw.as_bytes().get(10) == Some(&b'T') {
        let year: i64 = raw.get(0..4)?.parse().ok()?;
        let month: i64 = raw.get(5..7)?.parse().ok()?;
        let day: i64 = raw.get(8..10)?.parse().ok()?;
        let hour: i64 = raw.get(11..13)?.parse().ok()?;
        let minute: i64 = raw.get(14..16)?.parse().ok()?;
        let second: i64 = raw.get(17..19)?.parse().ok()?;
        let offset = parse_offset(&raw[19..]).unwrap_or(0);
        return Some(days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second - offset);
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
        return Some(days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second - offset);
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
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// HTML varlıklarını (`&amp;`, `&#39;`, `&quot;` ...) düz metne çevirir.
pub fn unescape_html(text: &str) -> String {
    let mut out = text
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ");

    // Sayısal varlıklar (&#8217; gibi)
    if out.contains("&#") {
        let mut decoded = String::with_capacity(out.len());
        let mut rest = out.as_str();
        while let Some(start) = rest.find("&#") {
            decoded.push_str(&rest[..start]);
            let tail = &rest[start + 2..];
            let end = tail.find(';');
            match end {
                Some(end) => {
                    let digits = &tail[..end];
                    let code = if let Some(hex) = digits.strip_prefix(['x', 'X']) {
                        u32::from_str_radix(hex, 16).ok()
                    } else {
                        digits.parse::<u32>().ok()
                    };
                    match code.and_then(char::from_u32) {
                        Some(ch) => decoded.push(ch),
                        None => decoded.push_str(&rest[start..start + 2 + end + 1]),
                    }
                    rest = &tail[end + 1..];
                }
                None => {
                    decoded.push_str(&rest[start..]);
                    rest = "";
                }
            }
        }
        decoded.push_str(rest);
        out = decoded;
    }

    out
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
        assert_eq!(parse_timestamp("2024-05-06T15:34:56+03:00"), Some(1714998896));
        assert_eq!(parse_timestamp("Wed Oct 10 20:19:24 +0000 2018"), Some(1539202764));
        assert_eq!(parse_timestamp("Thu Mar 21 20:50:14 +0000 2019"), Some(1553201414));
    }

    #[test]
    fn invalid_timestamps_are_rejected() {
        assert_eq!(parse_timestamp(""), None);
        assert_eq!(parse_timestamp("bugün"), None);
        assert!(parse_timestamp("2024-13-99T99:99:99Z").is_some(), "bozuk tarih panik yapmadan çözülmeli");
    }
}


// =====================================================================================
// MESAJ AKIŞI
// =====================================================================================

/// Taklit için kullanılan webhook adı (kanalda tek bir tane tutulup yeniden kullanılır).
const WEBHOOK_NAME: &str = "Botgek Native Embed";
/// Taklit edilen mesajların orijinal yazarı bu sayıya kadar hatırlanır
/// (🗑️ silme izni ve AI koruması aynı oturum boyunca çalışır).
const MAX_REMEMBERED_AUTHORS: usize = 4096;

/// Kanaldaki taklit webhook'u (kimlik + token).
#[derive(Debug, Clone, PartialEq, Eq)]
struct NamedWebhook {
    id: WebhookId,
    token: String,
}

/// Kanal → webhook önbelleği. `None`: kanalda webhook kullanılamıyor (yetki yok veya DM);
/// her mesajda yeniden denenmez.
static CHANNEL_WEBHOOKS: Lazy<Mutex<HashMap<ChannelId, Option<NamedWebhook>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
/// Taklit edilen mesaj → orijinal yazar eşlemesi (🗑️ izni ve AI koruması için).
static MESSAGE_AUTHORS: Lazy<Mutex<HashMap<MessageId, UserId>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
/// Components V2 gönderimi Discord tarafından reddedilirse (ör. eski API) bir daha denenmez.
static V2_UNAVAILABLE: AtomicBool = AtomicBool::new(false);

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
            .or_else(|| author.global_name.clone().filter(|name| !name.trim().is_empty()))
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
    let mut cleaned = name.trim().trim_start_matches('@').to_string();
    for (from, to) in [("discord", "d!scord"), ("Discord", "D!scord"), ("clyde", "c!yde"), ("Clyde", "C!yde")]
    {
        cleaned = cleaned.replace(from, to);
    }
    let cleaned: String = cleaned.trim().chars().take(80).collect();
    if cleaned.trim().is_empty() {
        "Botgek".to_string()
    } else {
        cleaned
    }
}

/// Taklit edilen mesajın orijinal yazarını kaydeder.
pub async fn remember_author(message_id: MessageId, author: UserId) {
    let mut authors = MESSAGE_AUTHORS.lock().await;
    if authors.len() >= MAX_REMEMBERED_AUTHORS {
        authors.clear();
    }
    authors.insert(message_id, author);
}

/// Taklit edilen mesajın orijinal yazarı (🗑️ silme izni bu bilgiyle verilir).
pub async fn original_author(message_id: MessageId) -> Option<UserId> {
    MESSAGE_AUTHORS.lock().await.get(&message_id).copied()
}

/// Bu mesaj Botgek'in taklit (webhook) mesajı mı? (native embed → AI tetiklenmez)
pub async fn is_native_message(message_id: MessageId) -> bool {
    MESSAGE_AUTHORS.lock().await.contains_key(&message_id)
}

// -------------------------------------------------------------------------------------
// TAKLİT WEBHOOK'U
// -------------------------------------------------------------------------------------

/// Kanalın taklit webhook'unu döner: önbellekte yoksa botun kendi webhook'unu arar,
/// hiç yoksa oluşturur. `refresh` önbelleği zorla yeniler (eskimiş token için).
async fn channel_webhook(
    ctx: &Context,
    client: &reqwest::Client,
    config: &Config,
    channel_id: ChannelId,
    refresh: bool,
) -> Option<NamedWebhook> {
    {
        let cache = CHANNEL_WEBHOOKS.lock().await;
        if !refresh {
            if let Some(entry) = cache.get(&channel_id) {
                return entry.clone();
            }
        }
    }
    CHANNEL_WEBHOOKS.lock().await.remove(&channel_id);

    match resolve_webhook(ctx, client, config, channel_id).await {
        Ok(Some(webhook)) => {
            CHANNEL_WEBHOOKS.lock().await.insert(channel_id, Some(webhook.clone()));
            Some(webhook)
        }
        Ok(None) => {
            warn!("Taklit webhook'u için token alınamadı ({}): mesaj bot adına gönderilecek", channel_id);
            CHANNEL_WEBHOOKS.lock().await.insert(channel_id, None);
            None
        }
        Err(err) => {
            warn!(
                "Taklit webhook'u kullanılamıyor ({}): {} — mesaj bot adına gönderilecek",
                channel_id, err
            );
            // Kalıcı ret (yetki yok veya DM kanalı) önbelleğe yazılır; geçici ağ hatalarında
            // bir sonraki mesajda yeniden denenir.
            if is_discord_rejection(&err) {
                CHANNEL_WEBHOOKS.lock().await.insert(channel_id, None);
            }
            None
        }
    }
}

/// Kanaldaki bot webhook'unu bulur; yoksa `WEBHOOK_NAME` adıyla oluşturur.
async fn resolve_webhook(
    ctx: &Context,
    client: &reqwest::Client,
    config: &Config,
    channel_id: ChannelId,
) -> Result<Option<NamedWebhook>, String> {
    let hooks = raw::list_channel_webhooks(client, &config.discord_token, channel_id).await?;
    let bot_id = ctx.cache.current_user().id.get();

    // Yalnızca bu botun oluşturduğu, token'ı olan gelen (incoming) webhook'lar yeniden kullanılır.
    let existing = hooks.iter().find(|hook| {
        hook.get("type").and_then(|kind| kind.as_u64()) == Some(1)
            && hook.get("user").and_then(|user| user.get("id")).and_then(|id| id.as_str())
                == Some(bot_id.to_string().as_str())
    });

    let created;
    let value = match existing {
        Some(hook) => hook,
        None => {
            created = raw::create_channel_webhook(client, &config.discord_token, channel_id, WEBHOOK_NAME).await?;
            &created
        }
    };

    match raw::webhook_credentials(value) {
        Some((id, token)) => Ok(Some(NamedWebhook { id, token })),
        None => Err("Webhook kimliği/token'ı okunamadı".to_string()),
    }
}

/// Webhook üzerinden ham gövde gönderir; token eskimişse bir kez yenileyip tekrar dener.
async fn send_via_webhook(
    ctx: &Context,
    client: &reqwest::Client,
    config: &Config,
    channel_id: ChannelId,
    payload: &serde_json::Value,
    files: &[serenity::all::CreateAttachment],
) -> Result<MessageId, String> {
    let mut last_error = "Webhook kullanılamıyor".to_string();

    for attempt in 0..2 {
        let Some(webhook) = channel_webhook(ctx, client, config, channel_id, attempt > 0).await else {
            return Err(last_error);
        };

        match raw::execute_webhook(client, webhook.id, &webhook.token, payload, files).await {
            Ok(Some(message_id)) => return Ok(message_id),
            Ok(None) => last_error = "Webhook yanıtında mesaj kimliği yok".to_string(),
            Err(err) => last_error = err,
        }
    }

    // İki denemede de başarısız: önbelleği bırak, sonraki mesajda yeniden çözümlenir.
    CHANNEL_WEBHOOKS.lock().await.remove(&channel_id);
    Err(last_error)
}

// -------------------------------------------------------------------------------------
// GÖNDERİM
// -------------------------------------------------------------------------------------

/// Kart stili modern (Components V2) kartı gerektiriyor mu?
///
/// `Auto` stilinde yalnızca **video içeren** gönderilerde modern kart kullanılır (video
/// kartın içinde oynar); diğer gönderiler fxembed tarzı klasik embed ile gönderilir ve
/// profil resmi kullanıcı adının solunda küçük durur.
fn use_modern_card(style: CardStyle, post: &BuiltinPost) -> bool {
    match style {
        CardStyle::Modern => true,
        CardStyle::FxEmbed => false,
        CardStyle::Auto => post.media.iter().any(|media| media.is_video()),
    }
}

/// Native embed akışı. Veri çekilemez veya mesaj gönderilemezse `false` döner; bu durumda
/// çağıran taraf (ayar açıksa) proxy motoruna düşer.
pub async fn handle(
    ctx: &Context,
    msg: &Message,
    client: &reqwest::Client,
    parsed: &ParsedPost,
    config: &Config,
    engine: &EngineSettings,
) -> bool {
    // Süre ölçümü: dönüşüm gecikmesi veri / medya / gönderim olarak log'lanır.
    let started = std::time::Instant::now();

    let post = match fetch(client, parsed, engine).await {
        Ok(post) => post,
        Err(err) => {
            warn!("Native embed verisi alınamadı ({}): {}", parsed.original_url, err);
            return false;
        }
    };
    let fetched = started.elapsed();

    let options = render::RenderOptions::from_engine(engine);
    let plan = render::plan_media(&post, &options);
    let resolved = render::resolve_media(client, &post, &plan, &options).await;
    let media_done = started.elapsed();
    let identity = engine.impersonate_user.then(|| Impersonation::from_message(msg));

    debug!(
        "Hazırlık: veri {} ms, medya {} ms ({})",
        fetched.as_millis(),
        (media_done - fetched).as_millis(),
        parsed.original_url
    );

    // 1) Modern bileşen düzeni: yalnızca seçilen kart stili izin veriyorsa.
    let modern_card = use_modern_card(engine.card_style, &post);
    debug!(
        "Kart stili: {} (modern kart: {}) — {}",
        engine.card_style.key(),
        modern_card,
        parsed.original_url
    );

    if modern_card && !V2_UNAVAILABLE.load(Ordering::Relaxed) {
        let attempt = send_components_v2(
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
        )
        .await;

        match attempt {
            Ok((message_id, impersonated)) => {
                info!(
                    "Mesaj Components V2 kart olarak dönüştürüldü ({:?}, taklit: {}): {}",
                    parsed.platform, impersonated, parsed.original_url
                );
                finish(ctx, msg, config, message_id).await;
                return true;
            }
            Err(err) => {
                if is_discord_rejection(&err) {
                    // Kalıcı ret (ör. API sürümü Components V2'yi desteklemiyor): tekrar denenmez.
                    V2_UNAVAILABLE.store(true, Ordering::Relaxed);
                }
                warn!(
                    "Components V2 gönderilemedi ({}): {} — klasik embed'e düşülüyor",
                    parsed.original_url, err
                );
            }
        }
    }

    // 2) Klasik embed (+ istenirse taklit webhook'u)
    let embeds = render::build_embeds(&post, &options, &resolved);
    if embeds.is_empty() {
        warn!("Native embed üretilemedi ({}): gösterilecek içerik yok", parsed.original_url);
        return false;
    }

    match send_classic(ctx, msg, client, config, parsed, &post, &resolved, &embeds, identity.as_ref()).await {
        Ok((message_id, impersonated)) => {
            info!(
                "Mesaj native embed ile dönüştürüldü ({:?}, taklit: {}, toplam {} ms): {}",
                parsed.platform,
                impersonated,
                started.elapsed().as_millis(),
                parsed.original_url
            );
            finish(ctx, msg, config, message_id).await;
            true
        }
        Err(err) => {
            error!("Native embed mesajı gönderilemedi: {}", err);
            false
        }
    }
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
) -> Result<(MessageId, bool), String> {
    let allowed_mentions = serde_json::to_value(build_selective_allowed_mentions(msg)).ok();
    let mut payload = v2::build_payload(&v2::V2Card {
        post,
        options,
        resolved,
        emojis,
        user_comment: parsed.user_comment.as_deref(),
        impersonation: identity,
        allowed_mentions,
    });

    // Bot mesajında orijinal mesaja yanıt bağlantısı korunur (webhook mesajları yanıtlayamaz).
    match msg.message_reference.as_ref().and_then(|reference| serde_json::to_value(reference).ok()) {
        Some(reference) => payload["message_reference"] = reference,
        None => {}
    }

    if identity.is_some() {
        let mut webhook_payload = payload.clone();
        if let Some(object) = webhook_payload.as_object_mut() {
            object.remove("message_reference");
        }

        match send_via_webhook(ctx, client, config, msg.channel_id, &webhook_payload, &resolved.attachments).await {
            Ok(message_id) => return Ok((message_id, true)),
            Err(err) => warn!("Components V2 taklit gönderimi başarısız ({}): {}", msg.id, err),
        }
    }

    let message_id =
        raw::send_channel_message(client, &config.discord_token, msg.channel_id, &payload, &resolved.attachments)
            .await?;
    Ok((message_id, false))
}

/// Klasik embed mesajını gönderir. Taklit istenmişse webhook üzerinden (kullanıcı adı ve
/// avatarı ile) gönderilir; webhook kullanılamıyorsa bot mesajına düşer.
#[allow(clippy::too_many_arguments)]
async fn send_classic(
    ctx: &Context,
    msg: &Message,
    client: &reqwest::Client,
    config: &Config,
    parsed: &ParsedPost,
    post: &BuiltinPost,
    resolved: &render::ResolvedMedia,
    embeds: &[serenity::all::CreateEmbed],
    identity: Option<&Impersonation>,
) -> Result<(MessageId, bool), String> {
    if let Some(identity) = identity {
        let allowed_mentions = serde_json::to_value(build_selective_allowed_mentions(msg)).unwrap_or_default();
        let payload = classic_payload(msg.author.id, parsed, post, resolved, embeds, identity, allowed_mentions)?;
        match send_via_webhook(ctx, client, config, msg.channel_id, &payload, &resolved.attachments).await {
            Ok(message_id) => return Ok((message_id, true)),
            Err(err) => warn!("Taklit mesaj gönderilemedi ({}): {} — bot adına gönderiliyor", msg.id, err),
        }
    }

    // Bot mesajı: kullanıcı etiketi korunur (🗑️ silme izni bu etiketten okunur).
    let content = build_content(msg.author.id, parsed, post, resolved, true);
    let mut builder = CreateMessage::new()
        .content(content)
        .allowed_mentions(build_selective_allowed_mentions(msg))
        .flags(MessageFlags::SUPPRESS_EMBEDS)
        .embeds(embeds.to_vec());

    if !resolved.attachments.is_empty() {
        builder = builder.add_files(resolved.attachments.clone());
    }
    if let Some(reference) = &msg.message_reference {
        builder = builder.reference_message(reference.clone());
    }

    let sent = msg
        .channel_id
        .send_message(&ctx.http, builder)
        .await
        .map_err(|e| format!("{e:?}"))?;
    Ok((sent.id, false))
}

/// Klasik embed gövdesini ham webhook isteği için JSON'a çevirir.
/// `SUPPRESS_EMBEDS` sayesinde metindeki link için Discord kendi önizlemesini üretmez;
/// bu yüzden link `< >` ile sarılmaz.
fn classic_payload(
    author_id: UserId,
    parsed: &ParsedPost,
    post: &BuiltinPost,
    resolved: &render::ResolvedMedia,
    embeds: &[serenity::all::CreateEmbed],
    identity: &Impersonation,
    allowed_mentions: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let embeds: Vec<serde_json::Value> = embeds
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Embed serileştirilemedi: {e}"))?;

    let attachments: Vec<serde_json::Value> = resolved
        .attachments
        .iter()
        .enumerate()
        .map(|(id, file)| serde_json::json!({"id": id, "filename": file.filename}))
        .collect();

    let mut payload = serde_json::json!({
        "content": build_content(author_id, parsed, post, resolved, false),
        "embeds": embeds,
        "attachments": attachments,
        "flags": MessageFlags::SUPPRESS_EMBEDS,
        "username": identity.username,
        "allowed_mentions": allowed_mentions,
    });

    if let Some(avatar) = &identity.avatar_url {
        payload["avatar_url"] = serde_json::json!(avatar);
    }

    Ok(payload)
}


/// Mesaj gönderildikten sonraki ortak adımlar: yazar kaydı, 🗑️ reaksiyonu, orijinali silme.
///
/// Yazar kaydı sayesinde taklit edilen (veya Components V2 ile gönderilen) mesajlarda
/// 🗑️ silme izni ve "AI tetiklenmesin" kuralı mesaj metninden bağımsız çalışır.
async fn finish(ctx: &Context, msg: &Message, config: &Config, message_id: MessageId) {
    remember_author(message_id, msg.author.id).await;

    if let Err(e) = msg
        .channel_id
        .create_reaction(&ctx.http, message_id, ReactionType::Unicode("🗑️".to_string()))
        .await
    {
        error!("Silme reaksiyonu eklenemedi: {:?}", e);
    }

    if config.delete_original {
        if let Err(e) = msg.delete(&ctx.http).await {
            error!("Orijinal mesaj silinirken hata oluştu: {:?}", e);
        }
    }
}

/// Discord'un 4xx yanıtı kalıcı bir ret anlamına gelir (ağ hatasından farklı olarak
/// tekrar denemek anlamsızdır).
fn is_discord_rejection(error: &str) -> bool {
    error.starts_with("Discord 4")
}

/// Klasik akışın mesaj gövdesi: `[Twitter](gönderi linki)`.
///
/// Link artık `< >` ile sarılmaz; Discord'un kendi önizlemesini üretmemesi için mesaja
/// `SUPPRESS_EMBEDS` bayrağı eklenir. Kullanıcı etiketi yalnızca mesaj bot adına
/// gönderilirken eklenir (taklit modunda mesaj zaten kullanıcının kendisi gibi görünür).
fn build_content(
    author_id: UserId,
    parsed: &ParsedPost,
    post: &BuiltinPost,
    resolved: &render::ResolvedMedia,
    mention_author: bool,
) -> String {
    let link = format!("[{}]({})", parsed.platform.display_name(), post.url);
    let link = if parsed.is_spoiler { format!("||{link}||") } else { link };

    let base_content = if mention_author {
        format!("<@{author_id}> {link}")
    } else {
        link
    };

    let quoted_comment = parsed.user_comment.as_ref().map(|comment| {
        let quoted: Vec<String> = comment.lines().map(|line| format!("> {line}")).collect();
        format!("\n{}", quoted.join("\n"))
    });

    let video_line = resolved
        .video_link
        .as_ref()
        .map(|link| format!("\n[🎬 Videoyu aç]({link})"));

    let mut content = match &quoted_comment {
        Some(comment) => format!("{base_content}{comment}"),
        None => base_content.clone(),
    };
    if let Some(video_line) = &video_line {
        content.push_str(video_line);
    }

    // Discord 2000 karakteri aşan içeriği reddeder: önce alıntıyı düşür, sonra kırp
    if content.chars().count() > DISCORD_CONTENT_LIMIT {
        content = base_content;
        if let Some(video_line) = &video_line {
            content.push_str(video_line);
        }
    }

    truncate_chars(&content, DISCORD_CONTENT_LIMIT)
}


#[cfg(test)]
mod message_tests {
    use super::*;
    use crate::converters::builtin::{Author, BuiltinPost};

    fn parsed() -> ParsedPost {
        ParsedPost {
            platform: Platform::Twitter,
            is_spoiler: false,
            user_comment: None,
            original_url: "https://x.com/kullanici/status/1".to_string(),
            fixed_url: "https://fixupx.com/kullanici/status/1".to_string(),
        }
    }

    fn post() -> BuiltinPost {
        BuiltinPost::new(
            Platform::Twitter,
            "https://x.com/kullanici/status/1",
            Author::new("Ad", "kullanici"),
        )
    }

    #[test]
    fn content_shows_the_link_as_a_masked_platform_link() {
        let content = build_content(
            UserId::new(123),
            &parsed(),
            &post(),
            &render::ResolvedMedia::default(),
            false,
        );
        assert_eq!(content, "[Twitter](https://x.com/kullanici/status/1)");
        assert!(!content.contains("<https://"), "link hilesi kalmamalı: {content}");
        assert!(!content.contains("[:]("), "eski işaret kalmamalı: {content}");
    }

    #[test]
    fn content_mentions_the_author_only_for_bot_messages() {
        let with_mention = build_content(
            UserId::new(123),
            &parsed(),
            &post(),
            &render::ResolvedMedia::default(),
            true,
        );
        assert!(with_mention.starts_with("<@123> [Twitter]("), "içerik: {with_mention}");
    }

    #[test]
    fn content_keeps_comment_spoiler_and_video_fallback() {
        let mut parsed = parsed();
        parsed.user_comment = Some("şuna bak".to_string());
        parsed.is_spoiler = true;

        let resolved = render::ResolvedMedia {
            video_link: Some("https://cdn/v1.mp4".to_string()),
            ..render::ResolvedMedia::default()
        };
        let content = build_content(UserId::new(9), &parsed, &post(), &resolved, false);

        assert!(content.starts_with("||[Twitter](https://x.com/kullanici/status/1)||"), "içerik: {content}");
        assert!(content.contains("\n> şuna bak"), "içerik: {content}");
        assert!(content.contains("[🎬 Videoyu aç](https://cdn/v1.mp4)"), "içerik: {content}");
    }

    #[test]
    fn webhook_names_are_sanitized_for_discord_rules() {
        assert_eq!(sanitize_webhook_name("  Eray  "), "Eray");
        assert_eq!(sanitize_webhook_name("@eray"), "eray");
        assert_eq!(sanitize_webhook_name(""), "Botgek");
        assert_eq!(sanitize_webhook_name("   "), "Botgek");
        assert_eq!(sanitize_webhook_name("Discord Clyde"), "D!scord C!yde");
        assert_eq!(sanitize_webhook_name(&"a".repeat(120)).chars().count(), 80);
    }

    #[test]
    fn classic_webhook_payload_suppresses_discord_previews() {
        let identity = Impersonation {
            username: "Eray".to_string(),
            avatar_url: Some("https://cdn.example/a.png".to_string()),
        };
        let embeds = vec![serenity::all::CreateEmbed::new().title("başlık")];
        let payload = classic_payload(
            UserId::new(5),
            &parsed(),
            &post(),
            &render::ResolvedMedia::default(),
            &embeds,
            &identity,
            serde_json::json!({"parse": []}),
        )
        .unwrap();

        assert_eq!(payload["username"], "Eray");
        assert_eq!(payload["avatar_url"], "https://cdn.example/a.png");
        assert_eq!(payload["flags"], MessageFlags::SUPPRESS_EMBEDS.bits());
        assert_eq!(payload["content"], "[Twitter](https://x.com/kullanici/status/1)");
        assert_eq!(payload["embeds"][0]["title"], "başlık");
        assert_eq!(payload["attachments"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn remembered_authors_can_be_queried() {
        let message_id = MessageId::new(777);
        assert_eq!(original_author(message_id).await, None);
        remember_author(message_id, UserId::new(42)).await;
        assert_eq!(original_author(message_id).await, Some(UserId::new(42)));
        assert!(is_native_message(message_id).await);
    }

    #[test]
    fn card_style_decides_whether_the_modern_card_is_used() {
        let mut video_post = post();
        video_post.media = vec![Media {
            kind: MediaKind::Video,
            url: "https://cdn/v1.mp4".to_string(),
            poster: None,
            width: None,
            height: None,
            duration: None,
        }];

        assert!(use_modern_card(CardStyle::Modern, &post()));
        assert!(
            !use_modern_card(CardStyle::FxEmbed, &video_post),
            "fxembed stilinde hep klasik embed kullanılır"
        );
        assert!(
            use_modern_card(CardStyle::Auto, &video_post),
            "otomatikte video kartın içinde oynar"
        );
        assert!(
            !use_modern_card(CardStyle::Auto, &post()),
            "otomatikte metin/görsel gönderiler fxembed düzeninde gönderilir"
        );
    }
}

