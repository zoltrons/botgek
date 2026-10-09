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
pub mod instagram;
pub mod raw;
pub mod reddit;
pub mod reddit_video;
pub mod render;
pub mod spotify;
pub mod tiktok;
pub mod translate;
pub mod twitter;
pub mod v2;

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use std::sync::LazyLock;
use serenity::all::{
    Channel, ChannelId, ChannelType, Context, Embed, Message, MessageId, ReactionType, UserId,
    WebhookId,
};
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use super::parser::{ParsedPost, Platform};
use crate::config::Config;
use crate::settings::{EmojiSettings, EngineSettings};
use crate::utils::mentions::build_selective_allowed_mentions;


/// Dış servis istekleri için zaman aşımı (ışık hızı: askıda kalan istek kartı bekletmez).
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(6);
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

    /// `Ad (@kullanici)` biçiminde etiket üretir. Kart V2'ye geçtiği için üretim kodunda
    /// kullanılmaz; dönüştürücü testleri yazar bilgisini bu biçimde doğrular.
    #[cfg(test)]
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

    /// Görsel mi? Kart V2'ye geçtiği için üretim kodunda kullanılmaz; dönüştürücü testleri
    /// medya listesinin görsel içerdiğini bu yardımcıyla doğrular.
    #[cfg(test)]
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

/// Gönderinin içindeki **harici bağlantının** önizlemesi (X'in kendi arayüzünde gösterdiği
/// "link kartı").
///
/// Gönderi medyasız olduğunda hedef sayfanın HTML'i çekilip `og:`/`twitter:` etiketleri
/// okunur ([`opengraph`]); resim galeride büyük olarak, başlık/açıklama/site adı ise
/// kartın içinde metin bloğu olarak gösterilir. Hiçbir medya indirilmez.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkPreview {
    /// Bağlantının hedef adresi (sayfa `og:url` bildirdiyse kanonik adres).
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    /// Sayfanın önizleme görseli (`og:image` / `twitter:image`); kartta CDN bağlantısıyla
    /// gösterilir.
    pub image: Option<String>,
    /// Site adı (`og:site_name`) veya alan adı (`example.com`).
    pub site_name: Option<String>,
}

impl LinkPreview {
    /// Kartta gösterilecek veri yok mu? Başlık, açıklama ve görselin **tümü** boşsa
    /// önizleme karta hiç eklenmez (boş bir link satırı kartı kirletir).
    pub fn is_empty(&self) -> bool {
        self.title.is_none() && self.description.is_none() && self.image.is_none()
    }
}

/// Platforma özel hazır kart (Components V2).
///
/// Bazı platformlar (ör. Spotify) genel kart düzeninden farklı bir görünüm ister:
/// bölüm + küçük resim (thumbnail) + buton satırı. Platform modülü kartın çocuk
/// bileşenlerini hazır JSON olarak üretip `BuiltinPost::custom_card`'a koyar; V2
/// katmanı genel kart yerine bu bileşenleri **tek bir `Container`** içinde (verilen
/// `accent_color` ile) gönderir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomCard {
    /// `Container` içine yerleştirilecek Components V2 çocuk bileşenleri.
    pub components: Vec<serde_json::Value>,
    /// Sol kenar renk çubuğu (`accent_color`); `None` ise çubuk çizilmez.
    pub accent_color: Option<u32>,
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
    /// Platforma özel hazır kart (Components V2); doluysa genel kart düzeni yerine bu
    /// bileşenler gönderilir (ör. Spotify'ın bölüm + thumbnail + buton düzeni).
    pub custom_card: Option<CustomCard>,
    /// Gönderinin içindeki harici bağlantının önizlemesi (X'in "link kartı"); yalnızca
    /// medyasız gönderilerde doldurulur.
    pub link_preview: Option<LinkPreview>,
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
            custom_card: None,
            link_preview: None,
        }
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
    if let Some(len) = response.content_length()
        && len > MAX_BODY_BYTES as u64 {
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

/// Paylaşılan gönderinin verisini, platforma uygun kaynaktan çeker.
/// Platforma göre gönderi verisini çeker. **Önbellek önce** denenir: aynı link kısa süre
/// içinde tekrar paylaşıldığında ağ isteği yapılmaz (dönüşüm anında tamamlanır).
pub async fn fetch(
    client: &reqwest::Client,
    parsed: &ParsedPost,
    engine: &EngineSettings,
    config: &Config,
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
        Platform::Reddit => reddit::fetch(client, parsed, &config.reddit_client_id).await,
        Platform::TikTok => tiktok::fetch(client, parsed).await,
        Platform::Instagram => instagram::fetch(client, parsed, config.instagram_sessionid.as_deref()).await,
        Platform::Spotify => spotify::fetch(client, parsed).await,
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
        Ok(Channel::Guild(channel)) => WebhookTarget::new(channel_id, channel.kind, channel.parent_id),
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

/// Kanal → webhook önbelleği. `None`: kanalda webhook kullanılamıyor (yetki yok veya DM);
/// her mesajda yeniden denenmez.
static CHANNEL_WEBHOOKS: LazyLock<Mutex<HashMap<ChannelId, Option<NamedWebhook>>>> =
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
/// Bu kayıt 🗑️ silme izni, "kartlara verilen yanıt AI'yı tetiklemesin" kuralı ve bayrak
/// emojisiyle çeviri ayrımı için kullanılır. Bot yeniden başlasa, çökse veya güncellense
/// de kayıt kaybolmaz (eskiden bellekteki bir `HashMap` içindeydi ve yeniden başlatmada
/// siliniyordu).
pub fn remember_author(message_id: MessageId, channel_id: ChannelId, author: UserId) {
    crate::store::remember_card(message_id, channel_id, author);
}

/// Kart kaydını siler (kart Discord'dan silindiğinde).
pub fn forget_author(message_id: MessageId) {
    crate::store::forget_card(message_id);
}

/// Taklit edilen mesajın orijinal yazarı (🗑️ silme izni bu bilgiyle verilir).
pub fn original_author(message_id: MessageId) -> Option<UserId> {
    crate::store::card_author(message_id)
}

/// Bu mesaj Botgek'in dönüştürdüğü bir kart mı? (native embed → AI tetiklenmez)
pub fn is_native_message(message_id: MessageId) -> bool {
    crate::store::is_card(message_id)
}

/// Kayıttan bağımsız **yapısal kart imzası** (test edilebilir çekirdek).
///
/// Kartlar **yalnızca** Components V2 olarak gönderilir: `content` alanı boştur ve bileşenler
/// bu crate'te tip olarak yoktur; bu yüzden kart, **gönderen kimliği** (bot ya da bir webhook)
/// ile birlikte "içeriksiz + embedsiz mesaj" olarak tanınır.
pub fn has_card_signature(
    author_id: UserId,
    from_webhook: bool,
    content: &str,
    embeds: &[Embed],
    bot_id: UserId,
) -> bool {
    let from_bot_or_webhook = author_id == bot_id || from_webhook;
    from_bot_or_webhook && content.trim().is_empty() && embeds.is_empty()
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
pub fn is_card_message(msg: &Message, bot_id: UserId) -> bool {
    if is_native_message(msg.id) {
        return true;
    }

    if let Some(flags) = msg.flags
        && (flags.bits() & (1 << 15) != 0) && (msg.author.id == bot_id || msg.webhook_id.is_some()) {
            return true;
        }

    has_card_signature(
        msg.author.id,
        msg.webhook_id.is_some(),
        &msg.content,
        &msg.embeds,
        bot_id,
    )
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
    {
        let cache = CHANNEL_WEBHOOKS.lock().await;
        if !refresh
            && let Some(entry) = cache.get(&channel_id) {
                return entry.clone();
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
///
/// Mesajın bulunduğu kanal bir alt başlık veya forum gönderisiyse webhook **üst kanalda**
/// çözümlenir ve gönderim `thread_id` ile o alt başlığa yapılır (bkz. [`webhook_target`]).
async fn send_via_webhook(
    ctx: &Context,
    client: &reqwest::Client,
    config: &Config,
    msg: &Message,
    payload: &serde_json::Value,
    files: &[serenity::all::CreateAttachment],
) -> Result<MessageId, String> {
    let mut last_error = "Webhook kullanılamıyor".to_string();
    let target = webhook_target(ctx, msg).await;

    for attempt in 0..2 {
        let Some(webhook) = channel_webhook(ctx, client, config, target.channel_id, attempt > 0).await else {
            return Err(last_error);
        };

        match raw::execute_webhook(client, webhook.id, &webhook.token, target.thread_id, payload, files).await {
            Ok(Some(message_id)) => return Ok(message_id),
            Ok(None) => last_error = "Webhook yanıtında mesaj kimliği yok".to_string(),
            Err(err) => last_error = err,
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

/// Orijinal mesajı siler; zaten silinmişse sessizce geçer (🗑️ reaksiyonu
/// yeni mesaja eklenir, bu yüzden mükerrer silme denemesi hata üretmez).
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
fn spawn_webhook_warmup(ctx: &Context, client: &reqwest::Client, config: &Config, msg: &Message) {
    let ctx = ctx.clone();
    let client = client.clone();
    let config = config.clone();
    let msg = msg.clone();
    tokio::spawn(async move {
        let target = webhook_target(&ctx, &msg).await;
        let _ = channel_webhook(&ctx, &client, &config, target.channel_id, false).await;
    });
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

    // Taklit (webhook) kimliğinin çözümlenmesi **veri çekmeyle paralel** yürür: ikisi de REST
    // çağrısıdır ve eskiden dönüşümün kritik yolunda sırayla bekleniyordu.
    let identity = should_impersonate(engine, msg.message_reference.is_some())
        .then(|| Impersonation::from_message(msg));
    if identity.is_some() {
        spawn_webhook_warmup(ctx, client, config, msg);
    }

    // Orijinal mesaj burada **silinmez**: silme, kart gönderildikten sonra yapılır (native
    // akışta `finish()`, hata durumunda proxy akışı). Erken silmek, gönderenin istemcisi
    // optimistic (nonce'lu) mesajını sunucu kopyasıyla eşleştirmeden "hayalet mesaj"
    // bıraktığı için artık kullanılmaz (bkz. [`delete_user_message`]).
    let post = match fetch(client, parsed, engine, config).await {
        Ok(post) => post,
        Err(err) => {
            warn!("Native embed verisi alınamadı ({}): {}", parsed.original_url, err);
            return false;
        }
    };
    let fetched = started.elapsed();

    let platform = platform_key(parsed.platform);
    let options = render::RenderOptions::from_engine(engine, platform);
    debug!(
        "Kart: V2 (tek mod: doğrudan; video indirme: {}) — {}",
        options.video_requires_download,
        parsed.original_url
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
    )
    .await
    {
        Ok((message_id, impersonated)) => {
            info!(
                "Mesaj Components V2 kart olarak dönüştürüldü ({:?}, taklit: {}, toplam {} ms): {}",
                parsed.platform,
                impersonated,
                started.elapsed().as_millis(),
                parsed.original_url
            );
            finish(ctx, msg, config, message_id);
            true
        }
        Err(err) => {
            error!("Components V2 kartı gönderilemedi: {err}");
            false
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
    if let Some(users) = value.get_mut("users").and_then(serde_json::Value::as_array_mut) {
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
) -> Result<(MessageId, bool), String> {
    if let Some(identity) = identity {
        // Webhook mesajları Discord'da yanıt olamaz (dokümante edilmemiş alan yok sayılır):
        // bu yüzden webhook denenir; başarısız olursa mesaj bot adına (yanıtla) gönderilir.
        let payload = v2_payload(msg, parsed, post, options, resolved, emojis, Some(identity), None);
        match send_via_webhook(ctx, client, config, msg, &payload, &resolved.attachments).await {
            Ok(message_id) => return Ok((message_id, true)),
            Err(err) => warn!("Components V2 taklit gönderimi başarısız ({}): {}", msg.id, err),
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
    let message_id =
        raw::send_channel_message(client, &config.discord_token, msg.channel_id, &payload, &resolved.attachments)
            .await?;
    Ok((message_id, false))
}
/// 🗑️ silme izni ve "AI tetiklenmesin" kuralı mesaj metninden bağımsız çalışır.
///
/// Reaksiyon ve silme **arka planda** yapılır: kart kullanıcıya çoktan görünmüştür, bu iki
/// REST çağrısı dönüşüm süresine eklenmez (ve mesaj işleme döngüsü serbest kalır). Silme,
/// kart gönderildikten **sonra** ve [`DELETE_MIN_AGE`] kadar gecikmeyle yapılır (hayalet
/// mesaj önlemi).
fn finish(ctx: &Context, msg: &Message, config: &Config, message_id: MessageId) {
    remember_author(message_id, msg.channel_id, msg.author.id);

    let ctx = ctx.clone();
    let msg = msg.clone();
    let delete_original = config.delete_original;
    tokio::spawn(async move {
        if let Err(e) = msg
            .channel_id
            .create_reaction(&ctx.http, message_id, ReactionType::Unicode("🗑️".to_string()))
            .await
        {
            error!("Silme reaksiyonu eklenemedi: {:?}", e);
        }

        // Kart gönderildi; şimdi orijinal mesajı sil (en az DELETE_MIN_AGE yaşına gelene
        // kadar bekleyerek: gönderenin istemcisi optimistic mesajını eşleştirebilsin).
        if delete_original {
            delete_user_message(&ctx, &msg).await;
        }
    });
}

/// Discord'un 4xx yanıtı kalıcı bir ret anlamına gelir (ağ hatasından farklı olarak
/// tekrar denemek anlamsızdır).
fn is_discord_rejection(error: &str) -> bool {
    if error.starts_with("Discord 429") || error.starts_with("Discord 408") {
        return false;
    }
    error.starts_with("Discord 4")
}

#[cfg(test)]
mod message_tests {
    use super::*;

    #[test]
    fn is_discord_rejection_treats_transient_rates_and_timeouts_as_non_rejections() {
        assert!(is_discord_rejection("Discord 400 döndü: Bad Request"));

        assert!(is_discord_rejection("Discord 403 döndü: Missing Permissions"));
        assert!(is_discord_rejection("Discord 404 döndü: Unknown Webhook"));
        assert!(!is_discord_rejection("Discord 429 döndü: Too Many Requests"));
        assert!(!is_discord_rejection("Discord 408 döndü: Request Timeout"));
        assert!(!is_discord_rejection("Discord 500 döndü: Internal Server Error"));
    }

    #[test]
    fn card_signature_recognises_modern_cards() {
        let bot_id = UserId::new(1);
        let user_id = UserId::new(2);

        // Modern kart (Components V2): içerik ve embed yok → gönderen kimliğiyle tanınır.
        assert!(
            has_card_signature(bot_id, false, "", &[], bot_id),
            "botun gönderdiği içeriksiz modern kart tanınmalı"
        );
        assert!(
            has_card_signature(user_id, true, "", &[], bot_id),
            "taklit webhook'unun gönderdiği modern kart tanınmalı"
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

    #[test]
    fn card_messages_are_recognised_without_any_registry_entry() {
        let bot_id = UserId::new(1);
        let user_id = UserId::new(2);
        let webhook_id = WebhookId::new(77);

        // Modern kart (içerik/embed yok): kart kaydı hiç olmasa bile yapısal imza ile
        // tanınmalı (AI tetiklenmemesi bu sayede garanti).
        let card_id = MessageId::new(9_001);
        let card = message_fixture(card_id, bot_id, "", vec![], None);
        assert!(!is_native_message(card_id), "bu kimlik için kayıt yok");
        assert!(is_card_message(&card, bot_id), "içeriksiz modern kart tanınmalı");

        // Taklit webhook'unun modern kartı: yazar bot değil, webhook kimliği kart sayılır.
        let impersonated = message_fixture(MessageId::new(9_002), user_id, "", vec![], Some(webhook_id));
        assert!(is_card_message(&impersonated, bot_id));

        // Kalıcı kayıt da tanımayı sağlar (kart kaydı kalıcıdır, yeniden başlatmada kaybolmaz).
        let recorded_id = MessageId::new(9_004);
        let recorded = message_fixture(recorded_id, user_id, "eski biçim", vec![], Some(webhook_id));
        assert!(!is_card_message(&recorded, bot_id));
        remember_author(recorded_id, ChannelId::new(5), user_id);
        assert!(is_card_message(&recorded, bot_id));

        // Botun AI yanıtı ve kullanıcı mesajı kart değildir (AI tetiklenebilmeli).
        let ai_reply = message_fixture(MessageId::new(9_005), bot_id, "Evet, haklısın.", vec![], None);
        assert!(!is_card_message(&ai_reply, bot_id));
        let user_message = message_fixture(MessageId::new(9_006), user_id, "merhaba", vec![], None);
        assert!(!is_card_message(&user_message, bot_id));
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
        message.mentions = vec![serde_json::from_value(serde_json::json!({
            "id": "42",
            "username": "eray",
            "discriminator": "0001",
            "avatar": null
        }))
        .expect("kullanıcı çözümlenmeli")];

        // Kart bot adına gidiyorsa etiket ping atmaz (yalnızca düz `@kullanıcı` görünür).
        let tagged = card_allowed_mentions(&message, true).unwrap();
        assert_eq!(tagged["users"], serde_json::json!([]), "paylaşan pinglenmemeli");

        // Taklit (webhook) akışında kartın üstünde etiket yok → liste değişmez.
        let plain = card_allowed_mentions(&message, false).unwrap();
        assert_eq!(plain["users"], serde_json::json!(["42"]));
    }

    #[test]
    fn remembered_authors_can_be_queried() {
        let message_id = MessageId::new(777);
        assert_eq!(original_author(message_id), None);
        remember_author(message_id, ChannelId::new(5), UserId::new(42));
        assert_eq!(original_author(message_id), Some(UserId::new(42)));
        assert!(is_native_message(message_id));

        // Kayıt silinince kart tanıma da düşer (🗑️ ile kart silindiğinde kullanılır).
        forget_author(message_id);
        assert_eq!(original_author(message_id), None);
        assert!(!is_native_message(message_id));
    }

    #[test]
    fn replies_keep_the_reply_context_instead_of_impersonating() {
        let mut engine = EngineSettings::default();

        assert!(should_impersonate(&engine, false), "yanıt değilse taklit kullanılır");
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

