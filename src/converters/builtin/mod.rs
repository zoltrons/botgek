//! Bot içi (native) link dönüştürme motoru.
//!
//! Proxy motoru linki `fixupx.com` benzeri bir önizleme servisine çevirip Discord'un
//! kendi embed'ini üretmesini bekler. Bu motor ise gönderi verisini doğrudan kaynaktan
//! (X syndication/fxtwitter, Reddit JSON, Bluesky XRPC, TikTok oEmbed) çeker; başlık,
//! metin, metrikler, alıntı ve medyayı kendisi indirip `CreateEmbed` olarak gönderir.

pub mod bluesky;
pub mod media;
pub mod opengraph;
pub mod reddit;
pub mod render;
pub mod tiktok;
pub mod translate;
pub mod twitter;

use std::time::Duration;

use serenity::all::{Context, CreateMessage, Message, ReactionType};
use tracing::{error, info, warn};

use super::parser::{ParsedPost, Platform};
use crate::config::Config;
use crate::settings::EngineSettings;
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
pub async fn fetch(
    client: &reqwest::Client,
    parsed: &ParsedPost,
    engine: &EngineSettings,
) -> Result<BuiltinPost, String> {
    match parsed.platform {
        Platform::Twitter => twitter::fetch(client, parsed, engine).await,
        Platform::Bluesky => bluesky::fetch(client, parsed).await,
        Platform::Reddit => reddit::fetch(client, parsed).await,
        Platform::TikTok => tiktok::fetch(client, parsed).await,
        Platform::Instagram => opengraph::fetch(client, parsed).await,
    }
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

/// Native embed akışı. Veri çekilemez veya embed üretilemezse `false` döner; bu durumda
/// çağıran taraf (ayar açıksa) proxy motoruna düşer.
pub async fn handle(
    ctx: &Context,
    msg: &Message,
    client: &reqwest::Client,
    parsed: &ParsedPost,
    config: &Config,
    engine: &EngineSettings,
) -> bool {
    let post = match fetch(client, parsed, engine).await {
        Ok(post) => post,
        Err(err) => {
            warn!("Native embed verisi alınamadı ({}): {}", parsed.original_url, err);
            return false;
        }
    };

    let options = render::RenderOptions::from_engine(engine);
    let plan = render::plan_media(&post, &options);
    let resolved = render::resolve_media(client, &post, &plan, &options).await;
    let embeds = render::build_embeds(&post, &options, &resolved);

    if embeds.is_empty() {
        warn!("Native embed üretilemedi ({}): gösterilecek içerik yok", parsed.original_url);
        return false;
    }

    let content = build_content(msg, parsed, &post, &resolved);

    let mut builder = CreateMessage::new()
        .content(content)
        .allowed_mentions(build_selective_allowed_mentions(msg))
        .embeds(embeds);

    if !resolved.attachments.is_empty() {
        builder = builder.add_files(resolved.attachments.clone());
    }

    // Eğer orijinal mesaj bir yanıtsa, bot da aynı mesaja yanıt versin
    if let Some(ref message_ref) = msg.message_reference {
        builder = builder.reference_message(message_ref.clone());
    }

    let sent_msg = match msg.channel_id.send_message(&ctx.http, builder).await {
        Ok(m) => m,
        Err(e) => {
            error!("Native embed mesajı gönderilemedi: {:?}", e);
            return false;
        }
    };

    info!(
        "Mesaj native embed ile dönüştürüldü ({:?}): {}",
        parsed.platform, parsed.original_url
    );

    if let Err(e) = sent_msg
        .react(&ctx.http, ReactionType::Unicode("🗑️".to_string()))
        .await
    {
        error!("Silme reaksiyonu eklenemedi: {:?}", e);
    }

    if config.delete_original {
        if let Err(e) = msg.delete(&ctx.http).await {
            error!("Orijinal mesaj silinirken hata oluştu: {:?}", e);
        }
    }

    true
}

/// Proxy akışıyla aynı gövdeyi üretir: `<@kullanıcı>[:](link) [**Platform**](<orijinal>)`.
/// Native linkler `<...>` ile sarıldığı için Discord ikinci bir embed üretmez; metindeki
/// `[:](` işareti `handler::is_link_conversion_message` tarafından da tanınır.
fn build_content(
    msg: &Message,
    parsed: &ParsedPost,
    post: &BuiltinPost,
    resolved: &render::ResolvedMedia,
) -> String {
    let link_part = if parsed.is_spoiler {
        format!("||[:](<{}>)||", post.url)
    } else {
        format!("[:](<{}>)", post.url)
    };

    let base_content = format!(
        "<@{}>{} [**{}**](<{}>)",
        msg.author.id,
        link_part,
        parsed.platform.display_name(),
        parsed.original_url
    );

    let quoted_comment = parsed.user_comment.as_ref().map(|comment| {
        let quoted: Vec<String> = comment.lines().map(|line| format!("> {line}")).collect();
        format!("\n{}", quoted.join("\n"))
    });

    let video_line = resolved
        .video_link
        .as_ref()
        .map(|link| format!("\n[🎬 Videoyu aç](<{link}>)"));

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

