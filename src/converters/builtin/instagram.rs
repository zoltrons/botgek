//! Instagram gönderi verisi (native motor).
//!
//! Veri, giriş/API anahtarı gerektirmeyen **iki herkese açık** kaynaktan **paralel** çekilir
//! (toplam gecikme tek istek kadardır):
//! - **Embed sayfası** (`/{p|reel|tv}/{kod}/embed/captioned/`): yazar kullanıcı adı, doğrulama
//!   işareti, avatar, açıklama (caption) ve beğeni/yorum sayısı. Instagram, tarayıcı olmayan
//!   User-Agent'lara bu sayfayı **sunucu tarafında** (SSR) verir; bot da böyle bir UA kullanır.
//! - **Gönderi sayfası** (`/{tür}/{kod}/`): `og:title` (görünen ad), `og:url` (kullanıcı adı),
//!   `og:description` (beğeni/yorum/tarih) ve `og:image`.
//!
//! İki kaynak da hata verirse (ör. giriş duvarı) akış hata döner ve ayar açıksa proxy motoruna
//! düşülür. Instagram, video/reel akışını (mp4) sayfa kaynağına koymadığı için video
//! gönderilerinde kart, videonun **kapak karesini** gösterir; video Discord içinde
//! oynatılamadığından gönderi bağlantısına `Reel`/`Video` etiketi eşlik eder.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;
use tracing::debug;

use super::opengraph::{self, OpenGraph};
use super::{Author, BuiltinPost, Media, MediaKind, Stats, parse_timestamp, unescape_html};
use crate::converters::parser::{ParsedPost, Platform};

/// Kalıcı gönderi bağlantısından (`/p/`, `/reel/`, `/reels/`, `/tv/`) tür + kodu çıkarır.
static PERMALINK_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"instagram\.com/(p|reel|reels|tv)/([A-Za-z0-9_-]+)")
        .expect("geçerli Instagram permalink regex")
});

static MEDIA_TYPE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)data-media-type="([A-Za-z]+)""#).expect("geçerli medya türü regex")
});
static USERNAME_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)<span class="UsernameText">([^<]*)</span>"#)
        .expect("geçerli kullanıcı adı regex")
});
/// `alt="Instagram post shared by @kullanici"` (kullanıcı adı için yedek kaynak).
static ALT_USERNAME_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)Instagram post shared by (?:&#0?64;|@)([A-Za-z0-9_.]+)"#)
        .expect("geçerli alt kullanıcı adı regex")
});
static VERIFIED_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)class="UsernameText">[^<]*</span>\s*<i class="Sprite[^"]*VerifiedSprite"#)
        .expect("geçerli doğrulama regex")
});
static AVATAR_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)class="AvatarContainer".*?<img[^>]*\ssrc="([^"]+)""#)
        .expect("geçerli avatar regex")
});
static IMAGE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)EmbeddedMediaImage"[^>]*\ssrc="([^"]+)""#)
        .expect("geçerli medya görseli regex")
});
static LIKES_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)class="SocialProof">.{0,600}?([0-9][0-9.,]*[KkMmBb]?)\s*likes"#)
        .expect("geçerli beğeni regex")
});
static COMMENTS_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)View all ([0-9][0-9.,]*[KkMmBb]?) comments"#).expect("geçerli yorum regex")
});
static CAPTION_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)<div class="Caption">(.*?)<div class="CaptionComments""#)
        .expect("geçerli açıklama regex")
});
static CAPTION_FALLBACK_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)<div class="Caption">(.*?)<div class="Footer""#)
        .expect("geçerli yedek açıklama regex")
});
static CAPTION_USERNAME_TAG_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)<a class="CaptionUsername".*?</a>"#)
        .expect("geçerli caption kullanıcı regex")
});
static BR_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)<br\s*/?>"#).expect("geçerli br regex"));
static ANCHOR_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)<a[^>]*>(.*?)</a>"#).expect("geçerli anchor regex"));
static TAG_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?is)<[^>]+>"#).expect("geçerli etiket regex"));

// OpenGraph açıklaması: `975K likes, 16K comments - kullanici on August 26, 2022: "..."`.
static OG_LIKES_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)([0-9][0-9.,]*[KkMmBb]?)\s+likes").expect("geçerli og beğeni regex")
});
static OG_COMMENTS_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)([0-9][0-9.,]*[KkMmBb]?)\s+comments").expect("geçerli og yorum regex")
});
static OG_DATE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)\bon ([A-Za-z]+) ([0-9]{1,2}), ([0-9]{4}):").expect("geçerli og tarih regex")
});
static OG_URL_USERNAME_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"instagram\.com/([A-Za-z0-9_.]+)/(?:p|reel|reels|tv)/")
        .expect("geçerli og url regex")
});

// =====================================================================================
// OTURUM (sessionid) DESTEĞİ
// =====================================================================================
//
// Instagram, reel/video akışını (mp4) ve carousel görsellerinin tamamını **girişsiz**
// istemcilere vermiyor: `/api/v1/media/{pk}/info/` çerezsiz çağrıldığında giriş sayfasına
// yönlendiriyor, embed sayfasının `contextJSON` alanı `null` dönüyor ve web GraphQL'i
// tarayıcı-olmayan istemcileri engelliyor. Bu yüzden mp4 **yalnızca** `INSTAGRAM_SESSIONID`
// tanımlıysa çekilir; değer yoksa mevcut davranış (kapak görseli + `Reel` etiketi) korunur.

/// Kısa kod (shortcode) alfabesi — Instagram medya kimliği (pk) bu taban-64 alfabesiyle kodlanır.
const SHORTCODE_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
/// Uygulama API'si (`/media/{pk}/info/`) için uç nokta tabanları.
///
/// Birincil yol iOS uygulama kimliği + `i.instagram.com` (yt-dlp de bunu kullanır); sessizce
/// başarısız olursa web kimliği + `www.instagram.com` ile bir kez daha denenir.
const SESSION_API_BASES: [(&str, &str); 2] = [
    ("https://i.instagram.com/api/v1/media", "124024574287414"),
    ("https://www.instagram.com/api/v1/media", "936619743392459"),
];
/// Uygulama API'sinin beklediği istemci kimliği (mobil uygulama User-Agent'ı).
const SESSION_USER_AGENT: &str = "Instagram 273.0.0.16.70 (iPhone15,2; iOS 17_5_1; en_US; en-US; scale=3.00; 1290x2796; 470085518)";
/// Oturum yanıtı için okunacak en büyük gövde (1 MB) — video listesi + açıklama fazlasıyla sığar.
const SESSION_MAX_BODY_BYTES: usize = 1024 * 1024;

/// Oturum API'sinden çözülen medya ve kartı zenginleştiren alanlar.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct SessionMedia {
    /// Gönderinin tüm medyaları: video varsa video, carousel'de her öğe sırayla.
    media: Vec<Media>,
    username: Option<String>,
    full_name: Option<String>,
    is_verified: bool,
    caption: Option<String>,
    likes: Option<u64>,
    comments: Option<u64>,
    views: Option<u64>,
    taken_at: Option<i64>,
    /// `product_type == "clips"` → gönderi reel.
    is_reel: bool,
}

impl SessionMedia {
    /// Çözülen medya bir video içeriyor mu?
    fn has_video(&self) -> bool {
        self.media.iter().any(Media::is_video)
    }
}

/// Instagram kısa kodunu sayısal medya kimliğine (pk) çevirir.
///
/// `BsOGulcndj-` → `1949525278281554174` (Instagram'ın embed sayfasındaki `data-media-id` ile
/// birebir doğrulandı).
fn shortcode_to_pk(shortcode: &str) -> Option<u64> {
    if shortcode.is_empty() {
        return None;
    }
    let mut value: u64 = 0;
    for byte in shortcode.bytes() {
        let digit = SHORTCODE_ALPHABET.iter().position(|c| *c == byte)? as u64;
        value = value.checked_mul(64)?.checked_add(digit)?;
    }
    Some(value)
}

/// Oturum API'sinden gönderi verisini çeker (yalnızca çerez tanımlıysa anlamlıdır).
///
/// Herhangi bir hata (çerez geçersiz, gönderi gizli, ağ hatası, giriş yönlendirmesi) `None`
/// döner: kart yine herkese açık kaynaklarla üretilir, akış hiçbir durumda bozulmaz.
async fn fetch_session_media(
    client: &reqwest::Client,
    code: &str,
    sessionid: Option<&str>,
) -> Option<SessionMedia> {
    let token = sessionid
        .map(str::trim)
        .filter(|t| !t.is_empty())?
        .to_string();
    let pk = shortcode_to_pk(code)?;

    // Tarayıcı çerezi `%3A` ile kodlanmış olabilir: ham değer ve çözülmüş biçim denenir.
    let decoded = token.replace("%3A", ":").replace("%3a", ":");
    let mut cookies = vec![format!("sessionid={token}")];
    if decoded != token {
        cookies.push(format!("sessionid={decoded}"));
    }

    // Denemeler **sahiplenilmiş** String'lerle önceden kurulur: döngü boyunca yerel bir
    // değişkeni ödünç tutan bir iterator `await` üzerinden taşınırsa future `Send` kalmaz
    // (serenity'nin olay döngüsü `Send` ister) — bu yüzden ödünç yerine sahiplik kullanılır.
    let mut attempts: Vec<(String, String, String)> = Vec::new();
    for (base, app_id) in SESSION_API_BASES {
        for cookie in &cookies {
            attempts.push((base.to_string(), app_id.to_string(), cookie.clone()));
        }
    }

    for (base, app_id, cookie) in attempts {
        let url = format!("{base}/{pk}/info/");
        let response = client
            .get(&url)
            .header("user-agent", SESSION_USER_AGENT)
            .header("x-ig-app-id", app_id.as_str())
            .header("x-ig-www-claim", "0")
            .header("x-asbd-id", "359341")
            .header("x-ig-capabilities", "3brTvw8=")
            .header("x-ig-connection-type", "WIFI")
            .header("accept", "*/*")
            .header("cookie", cookie.as_str())
            .timeout(super::REQUEST_TIMEOUT)
            .send()
            .await;

        let response = match response {
            Ok(response) => response,
            Err(err) => {
                debug!("Instagram oturum isteği başarısız ({url}): {err}");
                continue;
            }
        };
        if !response.status().is_success() {
            // Çerez geçersizse Instagram giriş sayfasına yönlendirir (3xx) ya da 401 döner.
            debug!(
                "Instagram oturum isteği reddedildi ({}): {url}",
                response.status()
            );
            continue;
        }

        let body = match read_body_limited(response, SESSION_MAX_BODY_BYTES).await {
            Some(body) => body,
            None => continue,
        };
        let json: Value = match serde_json::from_str(&body) {
            Ok(json) => json,
            Err(err) => {
                debug!("Instagram oturum yanıtı çözümlenemedi ({url}): {err}");
                continue;
            }
        };
        if let Some(session) = parse_session_media(&json)
            && !session.media.is_empty()
        {
            debug!(
                "Instagram oturum verisi alındı ({url}): {} medya, video: {}",
                session.media.len(),
                session.has_video()
            );
            return Some(session);
        }
    }

    None
}

/// Yanıt gövdesini sınırlı okuyup UTF-8 metne çevirir (`None` = boş/çok büyük/geçersiz gövde).
async fn read_body_limited(mut response: reqwest::Response, max: usize) -> Option<String> {
    if let Some(length) = response.content_length()
        && length > max as u64
    {
        return None;
    }
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if body.len() + chunk.len() > max {
            return None;
        }
        body.extend_from_slice(&chunk);
    }
    String::from_utf8(body).ok()
}

/// Uygulama API'si yanıtını (`{"items":[…]}`) çözümler; gönderi yoksa `None`.
fn parse_session_media(json: &Value) -> Option<SessionMedia> {
    let item = json.get("items")?.as_array()?.first()?;

    // Carousel: tüm öğeler galeriye girer. Tek gönderide yalnızca kendisi.
    let mut media: Vec<Media> = Vec::new();
    match item.get("carousel_media").and_then(Value::as_array) {
        Some(nodes) if !nodes.is_empty() => {
            for node in nodes {
                push_session_node(&mut media, node);
            }
        }
        _ => push_session_node(&mut media, item),
    }

    let user = item.get("user");
    Some(SessionMedia {
        media,
        username: string_field(user, "username"),
        full_name: string_field(user, "full_name"),
        is_verified: user
            .and_then(|u| u.get("is_verified"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        caption: item
            .get("caption")
            .and_then(|caption| caption.get("text"))
            .and_then(Value::as_str)
            .map(str::to_string),
        likes: item.get("like_count").and_then(Value::as_u64),
        comments: item.get("comment_count").and_then(Value::as_u64),
        // Görüntülenme alanı sürüme göre farklı adlarla gelir.
        views: [
            "play_count",
            "ig_play_count",
            "view_count",
            "video_view_count",
        ]
        .iter()
        .find_map(|key| item.get(*key).and_then(Value::as_u64)),
        taken_at: item.get("taken_at").and_then(Value::as_i64),
        is_reel: item
            .get("product_type")
            .and_then(Value::as_str)
            .is_some_and(|value| value == "clips"),
    })
}

/// JSON nesnesinden boş olmayan metin alanı okur.
fn string_field(parent: Option<&Value>, key: &str) -> Option<String> {
    parent
        .and_then(|value| value.get(key))
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.is_empty())
}

/// Tek bir medya düğümünü (gönderi ya da carousel öğesi) listeye ekler.
fn push_session_node(out: &mut Vec<Media>, node: &Value) {
    let poster = node.get("image_versions2").and_then(largest_candidate_url);
    let width = node
        .get("original_width")
        .and_then(Value::as_u64)
        .map(|value| value as u32);
    let height = node
        .get("original_height")
        .and_then(Value::as_u64)
        .map(|value| value as u32);

    if let Some(video) = session_video_url(node) {
        out.push(Media {
            kind: MediaKind::Video,
            url: video,
            poster,
            width,
            height,
            duration: node
                .get("video_duration")
                .and_then(Value::as_f64)
                .map(|seconds| seconds.round() as u64),
        });
    } else if let Some(image) = poster {
        out.push(Media {
            kind: MediaKind::Image,
            url: image,
            poster: None,
            width,
            height,
            duration: None,
        });
    }
}

/// `video_versions` listesinden en yüksek çözünürlüklü mp4'ü seçer.
fn session_video_url(node: &Value) -> Option<String> {
    node.get("video_versions")?
        .as_array()?
        .iter()
        .max_by_key(|version| version.get("width").and_then(Value::as_u64).unwrap_or(0))
        .and_then(|version| version.get("url"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|url| !url.is_empty())
}

/// `image_versions2.candidates` listesinden en yüksek çözünürlüklü görseli seçer.
fn largest_candidate_url(image_versions: &Value) -> Option<String> {
    image_versions
        .get("candidates")?
        .as_array()?
        .iter()
        .max_by_key(|candidate| candidate.get("width").and_then(Value::as_u64).unwrap_or(0))
        .and_then(|candidate| candidate.get("url"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|url| !url.is_empty())
}

/// Embed (SSR) sayfasından çıkarılan alanlar.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct EmbedData {
    media_type: Option<String>,
    username: Option<String>,
    verified: bool,
    avatar: Option<String>,
    caption: Option<String>,
    likes: Option<u64>,
    comments: Option<u64>,
    /// Gönderinin yüksek çözünürlüklü görseli (video için kapak karesi).
    image: Option<String>,
}

impl EmbedData {
    /// Sayfa gerçekten gönderi içeriyor mu? (Giriş duvarı/boş sayfa `false` döner.)
    fn is_useful(&self) -> bool {
        self.username.is_some() || self.image.is_some() || self.caption.is_some()
    }
}

/// Instagram gönderisini herkese açık iki kaynaktan (embed + OpenGraph) ve — tanımlıysa —
/// oturum API'sinden **paralel** çekip karta dönüştürür.
///
/// `instagram_sessionid` yalnızca reel/video mp4'ü ve carousel görsellerinin tamamı için
/// gerekir; tanımlı değilse oturum isteği hiç yapılmaz (mevcut davranış korunur).
pub async fn fetch(
    client: &reqwest::Client,
    parsed: &ParsedPost,
    instagram_sessionid: Option<&str>,
) -> Result<BuiltinPost, String> {
    let (kind, code) = parse_permalink(&parsed.original_url).ok_or_else(|| {
        format!(
            "Instagram kalıcı linki çözümlenemedi: {}",
            parsed.original_url
        )
    })?;

    let embed_url = format!("https://www.instagram.com/{kind}/{code}/embed/captioned/");
    let page_url = format!("https://www.instagram.com/{kind}/{code}/");

    // Üç uç **paralel** istenir: toplam gecikme tek isteğin gecikmesi kadardır.
    let (embed_html, page_html, session) = tokio::join!(
        super::get_text(client, &embed_url),
        super::get_text(client, &page_url),
        fetch_session_media(client, &code, instagram_sessionid),
    );

    let embed = embed_html
        .ok()
        .map(|html| parse_embed(&html))
        .filter(EmbedData::is_useful);
    let graph = page_html
        .ok()
        .map(|html| opengraph::parse_open_graph(&html))
        .filter(graph_is_useful);

    if embed.is_none() && graph.is_none() && session.is_none() {
        return Err(format!(
            "Instagram gönderi verisi alınamadı (giriş duvarı olabilir): {}",
            parsed.original_url
        ));
    }

    Ok(build_post(parsed, &kind, embed, graph, session))
}

/// OpenGraph verisi kart üretmeye yetiyor mu? (Giriş duvarı yalnızca site adını döndürür.)
fn graph_is_useful(graph: &OpenGraph) -> bool {
    graph.image.is_some() || graph.description.is_some()
}

/// Kalıcı bağlantıdan gönderi türü ve kodunu çıkarır.
fn parse_permalink(url: &str) -> Option<(String, String)> {
    let caps = PERMALINK_REGEX.captures(url)?;
    Some((
        caps.get(1)?.as_str().to_string(),
        caps.get(2)?.as_str().to_string(),
    ))
}

/// SSR embed sayfasını ayrıştırır.
fn parse_embed(html: &str) -> EmbedData {
    let username = USERNAME_REGEX
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| unescape_html(m.as_str()).trim().to_string())
        .filter(|name| !name.is_empty())
        .or_else(|| {
            ALT_USERNAME_REGEX
                .captures(html)
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().to_string())
        });

    let caption = CAPTION_REGEX
        .captures(html)
        .or_else(|| CAPTION_FALLBACK_REGEX.captures(html))
        .and_then(|c| c.get(1))
        .map(|m| clean_caption(m.as_str()))
        .filter(|text| !text.is_empty());

    EmbedData {
        media_type: MEDIA_TYPE_REGEX
            .captures(html)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_string()),
        username,
        verified: VERIFIED_REGEX.is_match(html),
        avatar: first_capture(&AVATAR_REGEX, html),
        caption,
        likes: number_from(&LIKES_REGEX, html),
        comments: number_from(&COMMENTS_REGEX, html),
        image: first_capture(&IMAGE_REGEX, html),
    }
}

/// Regex'in ilk yakalama grubunu (HTML varlıkları çözülmüş hâlde) döndürür.
fn first_capture(regex: &Regex, html: &str) -> Option<String> {
    regex
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| unescape_html(m.as_str()).trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Regex'in ilk yakalama grubunu kompakt sayıya çevirir (`975K` → 975000).
fn number_from(regex: &Regex, html: &str) -> Option<u64> {
    regex
        .captures(html)
        .and_then(|c| c.get(1))
        .and_then(|m| parse_compact_number(m.as_str()))
}

/// `975K`, `1.5M`, `60,279,326` gibi metinleri sayıya çevirir.
fn parse_compact_number(raw: &str) -> Option<u64> {
    let cleaned = raw.trim().replace(',', "");
    let last = cleaned.chars().last()?;
    let (digits, multiplier) = match last {
        'K' | 'k' => (&cleaned[..cleaned.len() - 1], 1_000.0),
        'M' | 'm' => (&cleaned[..cleaned.len() - 1], 1_000_000.0),
        'B' | 'b' => (&cleaned[..cleaned.len() - 1], 1_000_000_000.0),
        _ => (cleaned.as_str(), 1.0),
    };
    let value: f64 = digits.trim().parse().ok()?;
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    Some((value * multiplier) as u64)
}

/// Embed açıklamasını düz metne çevirir: kullanıcı adı bağlantısı atılır, `<br />` satır sonu
/// olur, etiket bağlantıları (`#etiket`) korunur, kalan HTML etiketleri ve varlıkları temizlenir.
fn clean_caption(raw: &str) -> String {
    let without_username = CAPTION_USERNAME_TAG_REGEX.replace_all(raw, "");
    let with_breaks = BR_REGEX.replace_all(&without_username, "\n");
    let with_tags = ANCHOR_REGEX.replace_all(&with_breaks, "$1");
    let text = TAG_REGEX.replace_all(&with_tags, "");
    let decoded = unescape_html(&text);

    // 3+ ardışık satır sonu 2'ye indirilir; satır başları/sonları kırpılır.
    let mut out = String::with_capacity(decoded.len());
    let mut blank_run = 0;
    for line in decoded.split('\n') {
        let line = line.trim();
        if line.is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
        } else {
            blank_run = 0;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
    }
    out.trim().to_string()
}

/// Embed, OpenGraph ve (varsa) oturum API'si verilerini gönderi modeline birleştirir.
/// Embed (SSR) alanları önceliklidir; eksik kalanlar OpenGraph'tan; medya ve kesin
/// metrik/ad bilgileri (varsa) oturum API'sinden tamamlanır.
fn build_post(
    parsed: &ParsedPost,
    kind: &str,
    embed: Option<EmbedData>,
    graph: Option<OpenGraph>,
    session: Option<SessionMedia>,
) -> BuiltinPost {
    let embed = embed.unwrap_or_default();
    let graph = graph.unwrap_or_default();

    // Kullanıcı adı: embed > og:url yolu.
    let username = embed
        .username
        .clone()
        .or_else(|| graph.url.as_deref().and_then(username_from_og_url))
        .unwrap_or_default();

    // Görünen ad: og:title'ın `<Ad> on Instagram: ...` kısmı; yoksa kullanıcı adı.
    let name = graph
        .title
        .as_deref()
        .and_then(display_name_from_title)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| username.clone());
    let name = if name.trim().is_empty() {
        "Instagram".to_string()
    } else {
        name
    };

    let mut author = Author::new(name, username.clone());
    author.verified = embed.verified;
    author.avatar = embed.avatar.clone();
    author.profile_url =
        (!username.is_empty()).then(|| format!("https://www.instagram.com/{username}/"));

    let mut post = BuiltinPost::new(Platform::Instagram, parsed.original_url.clone(), author);

    // Açıklama: embed caption öncelikli; yoksa og:description arındırılır.
    post.text = embed
        .caption
        .clone()
        .or_else(|| {
            graph
                .description
                .as_deref()
                .and_then(caption_from_og_description)
        })
        .unwrap_or_default();

    // Tarih: og:description içindeki `... on August 26, 2022: ...`.
    post.created_at = graph.description.as_deref().and_then(parse_ig_date);

    // Metrikler: embed (tam sayı) öncelikli; yoksa og (kısaltmalı: 975K).
    let likes = embed
        .likes
        .or_else(|| stat_from_description(&OG_LIKES_REGEX, graph.description.as_deref()));
    let comments = embed
        .comments
        .or_else(|| stat_from_description(&OG_COMMENTS_REGEX, graph.description.as_deref()));
    post.stats = Stats {
        replies: comments.unwrap_or(0),
        likes: likes.unwrap_or(0),
        ..Stats::default()
    };

    // Medya: embed görseli (video için kapak karesi) öncelikli; yoksa og görseli/videosu.
    if let Some(image) = embed.image.clone().or_else(|| graph.image.clone()) {
        post.media.push(Media {
            kind: MediaKind::Image,
            url: image,
            poster: None,
            width: None,
            height: None,
            duration: None,
        });
    } else if let Some(video) = graph.video.clone() {
        post.media.push(Media {
            kind: MediaKind::Video,
            url: video,
            poster: graph.image.clone(),
            width: None,
            height: None,
            duration: None,
        });
    }

    // Medya: oturum API'si (varsa) kesin kaynaktır — reel/video mp4'ü ve carousel'in tamamı.
    if let Some(session) = session {
        apply_session(&mut post, session, kind);
    }

    // Video/reel göstergesi: mp4 gömülemediğinde kart kapak + etiket gösterir.
    let has_video = post.media.iter().any(Media::is_video);
    if post.extra_label.is_none() && (is_video_post(&embed.media_type, kind) || has_video) {
        post.extra_label = Some(video_label(kind).to_string());
    }

    post
}

/// Oturum API'sinden gelen veriyi karta uygular.
///
/// Medya **kesin kaynak** olarak değiştirilir (reel/video mp4'ü kartın içinde oynar,
/// carousel'in tüm öğeleri galeriye girer). Metin/istatistik alanları yalnızca mevcut
/// değer yoksa doldurulur — herkese açık kaynaklardan gelen veri korunur.
fn apply_session(post: &mut BuiltinPost, session: SessionMedia, kind: &str) {
    if !session.media.is_empty() {
        post.media = session.media;
    }

    if let Some(username) = session.username.filter(|u| !u.is_empty()) {
        if post.author.handle.is_empty() {
            post.author.handle = username.clone();
        }
        if post.author.profile_url.is_none() {
            post.author.profile_url = Some(format!("https://www.instagram.com/{username}/"));
        }
    }

    // Görünen ad: og:title'dan bir ad gelmediyse (ad, kullanıcı adına eşitse) API'den alınır.
    if let Some(full_name) = session.full_name.filter(|n| !n.trim().is_empty())
        && (post.author.name.trim().is_empty() || post.author.name == post.author.handle)
    {
        post.author.name = full_name;
    }

    if session.is_verified {
        post.author.verified = true;
    }

    if post.text.trim().is_empty()
        && let Some(caption) = session.caption.filter(|c| !c.trim().is_empty())
    {
        post.text = caption;
    }

    if post.created_at.is_none() {
        post.created_at = session.taken_at;
    }

    if let Some(likes) = session.likes.filter(|v| *v > 0) {
        post.stats.likes = likes;
    }
    if let Some(comments) = session.comments.filter(|v| *v > 0) {
        post.stats.replies = comments;
    }
    if let Some(views) = session.views.filter(|v| *v > 0) {
        post.stats.views = Some(views);
    }

    if post.media.iter().any(Media::is_video) {
        let label = if session.is_reel || matches!(kind, "reel" | "reels") {
            "Reel"
        } else {
            "Video"
        };
        post.extra_label = Some(label.to_string());
    }
}

/// Gönderi video/reel mi? (Embed türü `GraphVideo` veya link türü `reel`/`reels`/`tv`.)
fn is_video_post(media_type: &Option<String>, kind: &str) -> bool {
    media_type.as_deref() == Some("GraphVideo") || matches!(kind, "reel" | "reels" | "tv")
}

/// Video gönderisi için kart etiketi.
fn video_label(kind: &str) -> &'static str {
    match kind {
        "reel" | "reels" => "Reel",
        _ => "Video",
    }
}

/// og:description içindeki `975K likes` / `16K comments` değerini çıkarır.
fn stat_from_description(regex: &Regex, description: Option<&str>) -> Option<u64> {
    regex
        .captures(description?)
        .and_then(|c| c.get(1))
        .and_then(|m| parse_compact_number(m.as_str()))
}

/// og:title'ın `<Ad> on Instagram: "..."` biçiminden görünen adı çıkarır.
fn display_name_from_title(title: &str) -> Option<String> {
    let index = title.find(" on Instagram:")?;
    let name = title[..index].trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// `og:url` yolundan kullanıcı adını çıkarır (`.../kullanici/reel/kod/` → `kullanici`).
fn username_from_og_url(url: &str) -> Option<String> {
    OG_URL_USERNAME_REGEX
        .captures(url)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

/// og:description'ın `... on <tarih>: "<açıklama>".` biçiminden açıklamayı çıkarır.
fn caption_from_og_description(description: &str) -> Option<String> {
    let start = description.find(": \"")? + 3;
    let mut text = description[start..].trim_end().to_string();
    if let Some(stripped) = text.strip_suffix('.') {
        text = stripped.trim_end().to_string();
    }
    if let Some(stripped) = text.strip_suffix('"') {
        text = stripped.trim_end().to_string();
    }
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// og:description içindeki `on August 26, 2022:` tarihini unix zaman damgasına çevirir.
fn parse_ig_date(description: &str) -> Option<i64> {
    let caps = OG_DATE_REGEX.captures(description)?;
    let month = month_number(caps.get(1)?.as_str())?;
    let day: i64 = caps.get(2)?.as_str().parse().ok()?;
    let year: i64 = caps.get(3)?.as_str().parse().ok()?;
    parse_timestamp(&format!("{year:04}-{month:02}-{day:02}T00:00:00Z"))
}

/// İngilizce ay adını (kısaltma veya tam ad) numaraya çevirir.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Gerçek SSR embed sayfasından alınan (kısaltılmış) örnek: fotoğraf gönderisi.
    const EMBED_PHOTO: &str = r#"<html class="_9dls"><body>
        <div class="EmbedFrame" data-media-type="GraphImage" data-media-id="1949525278281554174" data-owner-id="10013772027">
        <div><div class="Header">
        <div class="AvatarContainer"><a class="Avatar" href="https://www.instagram.com/world_record_egg/">
        <img src="https://scontent.cdninstagram.com/avatar.jpg?stp=dst-jpg_s100x100&amp;_nc_cat=110" alt="world_record_egg" /></a></div>
        <div class="HeaderText"><a class="Username"><span class="UsernameText">world_record_egg</span>
        <i class="Sprite VerifiedSprite img sp_qYtR2jXevE_ sx_eee86e"><u>Doğrulandı</u></i></a></div></div>
        <div class="Content EmbedFrame"><a class="EmbeddedMedia"><img class="EmbeddedMediaImage"
        alt="Instagram post shared by &#064;world_record_egg"
        src="https://instagram.fesb6-1.fna.fbcdn.net/v/photo.jpg?stp=dst-jpg_e35&amp;_nc_cat=104" srcset="/x 1x" /></a></div>
        <div class="SocialProof"><a href="https://www.instagram.com/p/BsOGulcndj-/">60,279,326 likes</a></div>
        <div class="Caption"><a class="CaptionUsername" href="/x">world_record_egg</a><br /><br />
        Let&#x2019;s set a world record together!<br /><br /><a href="/explore/tags/EggGang/">#EggGang</a>
        <div class="CaptionComments"><a class="CaptionCommentsExpand">View all 3,771,804 comments</a></div></div>
        <div class="Footer"><a class="CommentInput">Add a comment...</a></div></div></div></body></html>"#;

    /// Gerçek SSR embed sayfasından alınan (kısaltılmış) örnek: reel (video).
    const EMBED_REEL: &str = r#"<div class="EmbedFrame" data-media-type="GraphVideo" data-media-id="2913440072144448240" data-owner-id="25025320">
        <div class="Header"><div class="AvatarContainer">
        <a class="Avatar" href="https://www.instagram.com/instagram/"><img src="https://scontent.cdninstagram.com/ig.jpg" alt="instagram" /></a></div>
        <div class="HeaderText"><a class="Username"><span class="UsernameText">instagram</span>
        <i class="Sprite VerifiedSprite img sp_x"><u>Doğrulandı</u></i></a></div></div>
        <div class="Content EmbedFrame"><img class="EmbeddedMediaImage" alt="Instagram post shared by &#064;instagram"
        src="https://instagram.fesb6-1.fna.fbcdn.net/v/cover.jpg?stp=dst-jpg_e15" /></div>
        <div class="SocialProof"><a href="/x">975K likes</a></div>
        <div class="Caption"><a class="CaptionUsername" href="/x">instagram</a><br /><br />Gingerton<br /><br />
        Reel by &#064;august_thegingercat<div class="CaptionComments"><a>View all 16K comments</a></div></div>
        <div class="Footer">Add a comment...</div>"#;

    fn parsed(kind: &str, code: &str) -> ParsedPost {
        ParsedPost {
            platform: Platform::Instagram,
            is_spoiler: false,
            user_comment: None,
            original_url: format!("https://www.instagram.com/{kind}/{code}"),
            fixed_url: format!("https://ddinstagram.com/{kind}/{code}"),
        }
    }

    #[test]
    fn permalink_is_parsed() {
        assert_eq!(
            parse_permalink("https://www.instagram.com/reel/C3abcXYZ123/?igsh=abcdef"),
            Some(("reel".to_string(), "C3abcXYZ123".to_string()))
        );
        assert_eq!(
            parse_permalink("https://instagram.com/p/BsOGulcndj-"),
            Some(("p".to_string(), "BsOGulcndj-".to_string()))
        );
        assert_eq!(
            parse_permalink("https://www.instagram.com/tv/BkfuX9UB-eK/"),
            Some(("tv".to_string(), "BkfuX9UB-eK".to_string()))
        );
        assert_eq!(parse_permalink("https://x.com/kullanici/status/1"), None);
    }

    #[test]
    fn compact_numbers_are_parsed() {
        assert_eq!(parse_compact_number("60,279,326"), Some(60_279_326));
        assert_eq!(parse_compact_number("975K"), Some(975_000));
        assert_eq!(parse_compact_number("1.5M"), Some(1_500_000));
        assert_eq!(parse_compact_number("2b"), Some(2_000_000_000));
        assert_eq!(parse_compact_number("12"), Some(12));
        assert_eq!(parse_compact_number(""), None);
    }

    /// Oturum API'si (`/media/{pk}/info/`) yanıtı: reel (video).
    const SESSION_REEL: &str = r#"{
        "items": [{
            "taken_at": 1784937600,
            "product_type": "clips",
            "media_type": 2,
            "like_count": 2000000,
            "comment_count": 3000,
            "play_count": 12345678,
            "caption": {"text": "Have you ever experienced this before?"},
            "user": {"username": "efewissiex", "full_name": "Efe ince", "is_verified": true},
            "original_width": 1080,
            "original_height": 1920,
            "video_duration": 12.34,
            "image_versions2": {"candidates": [
                {"width": 640, "height": 1136, "url": "https://cdn/cover-small.jpg"},
                {"width": 1080, "height": 1920, "url": "https://cdn/cover.jpg"}
            ]},
            "video_versions": [
                {"width": 720, "height": 1280, "url": "https://cdn/video-720.mp4"},
                {"width": 1080, "height": 1920, "url": "https://cdn/video-1080.mp4"}
            ]
        }]
    }"#;

    /// Oturum API'si yanıtı: carousel (iki görsel + bir video).
    const SESSION_CAROUSEL: &str = r#"{
        "items": [{
            "media_type": 8,
            "carousel_media": [
                {"image_versions2": {"candidates": [
                    {"width": 1080, "height": 1080, "url": "https://cdn/a.jpg"}]}},
                {"image_versions2": {"candidates": [
                    {"width": 720, "height": 1280, "url": "https://cdn/b.jpg"}]}},
                {"video_versions": [{"width": 640, "height": 1136, "url": "https://cdn/c.mp4"}],
                 "image_versions2": {"candidates": [
                    {"width": 640, "height": 1136, "url": "https://cdn/c.jpg"}]}}
            ]
        }]
    }"#;

    #[test]
    fn shortcode_converts_to_media_pk() {
        // Instagram embed sayfasındaki `data-media-id` değerleriyle birebir eşleşir.
        assert_eq!(
            shortcode_to_pk("BsOGulcndj-"),
            Some(1_949_525_278_281_554_174)
        );
        assert_eq!(
            shortcode_to_pk("Chunk8-jurw"),
            Some(2_913_440_072_144_448_240)
        );
        assert_eq!(shortcode_to_pk(""), None);
        assert_eq!(
            shortcode_to_pk("geçersiz!"),
            None,
            "alfabe dışı karakter reddedilir"
        );
    }

    #[test]
    fn session_reel_is_parsed_with_mp4_poster_and_stats() {
        let json: Value = serde_json::from_str(SESSION_REEL).unwrap();
        let session = parse_session_media(&json).expect("reel çözülmeli");

        assert!(session.has_video());
        assert!(session.is_reel, "product_type=clips → reel");
        assert_eq!(session.media.len(), 1);
        assert_eq!(session.media[0].kind, MediaKind::Video);
        // En yüksek çözünürlüklü mp4 ve kapak seçilir.
        assert_eq!(session.media[0].url, "https://cdn/video-1080.mp4");
        assert_eq!(
            session.media[0].poster.as_deref(),
            Some("https://cdn/cover.jpg")
        );
        assert_eq!(session.media[0].duration, Some(12));
        assert_eq!(session.media[0].width, Some(1080));
        assert_eq!(session.media[0].height, Some(1920));

        assert_eq!(session.username.as_deref(), Some("efewissiex"));
        assert_eq!(session.full_name.as_deref(), Some("Efe ince"));
        assert!(session.is_verified);
        assert_eq!(
            session.caption.as_deref(),
            Some("Have you ever experienced this before?")
        );
        assert_eq!(session.likes, Some(2_000_000));
        assert_eq!(session.comments, Some(3_000));
        assert_eq!(session.views, Some(12_345_678));
        assert_eq!(session.taken_at, Some(1_784_937_600));
    }

    #[test]
    fn session_carousel_keeps_every_item() {
        let json: Value = serde_json::from_str(SESSION_CAROUSEL).unwrap();
        let session = parse_session_media(&json).expect("carousel çözülmeli");

        assert_eq!(session.media.len(), 3, "carousel'in tamamı galeriye girer");
        assert_eq!(session.media[0].kind, MediaKind::Image);
        assert_eq!(session.media[0].url, "https://cdn/a.jpg");
        assert_eq!(session.media[1].kind, MediaKind::Image);
        assert!(session.media[2].is_video());
        assert_eq!(session.media[2].url, "https://cdn/c.mp4");
        assert_eq!(
            session.media[2].poster.as_deref(),
            Some("https://cdn/c.jpg")
        );
        assert!(session.has_video());
        assert!(!session.is_reel, "carousel gönderi reel değil");
    }

    #[test]
    fn session_media_replaces_cover_with_playable_video() {
        let session = parse_session_media(&serde_json::from_str(SESSION_REEL).unwrap()).unwrap();
        let post = build_post(
            &parsed("reel", "DbOKxSpolaP"),
            "reel",
            Some(parse_embed(EMBED_REEL)),
            None,
            Some(session),
        );

        // Kapak görseli yerine oynayan mp4 kartın içine girer.
        assert_eq!(post.media.len(), 1);
        assert!(post.media[0].is_video(), "reel mp4 olarak taşınmalı");
        assert_eq!(post.media[0].url, "https://cdn/video-1080.mp4");
        assert_eq!(
            post.media[0].poster.as_deref(),
            Some("https://cdn/cover.jpg")
        );
        assert_eq!(post.extra_label.as_deref(), Some("Reel"));

        // API verisi kartı zenginleştirir (embed metrikleri kesin değerlerle değişir).
        assert_eq!(post.stats.likes, 2_000_000);
        assert_eq!(post.stats.replies, 3_000);
        assert_eq!(post.stats.views, Some(12_345_678));
        assert_eq!(post.created_at, Some(1_784_937_600));
        assert!(post.author.verified);
        // og:title/görünen ad yoksa API'nin `full_name`'i kullanılır.
        assert_eq!(post.author.name, "Efe ince");
    }

    #[test]
    fn embed_page_is_parsed() {
        let embed = parse_embed(EMBED_PHOTO);
        assert_eq!(embed.media_type.as_deref(), Some("GraphImage"));
        assert_eq!(embed.username.as_deref(), Some("world_record_egg"));
        assert!(embed.verified, "doğrulama işareti bulunmalı");
        let avatar = embed.avatar.expect("avatar bulunmalı");
        assert!(avatar.starts_with("https://scontent.cdninstagram.com/avatar.jpg"));
        assert!(
            avatar.contains("&_nc_cat=110"),
            "HTML varlıkları çözülmeli: {avatar}"
        );
        assert_eq!(embed.likes, Some(60_279_326));
        assert_eq!(embed.comments, Some(3_771_804));
        assert_eq!(
            embed.image.as_deref(),
            Some("https://instagram.fesb6-1.fna.fbcdn.net/v/photo.jpg?stp=dst-jpg_e35&_nc_cat=104")
        );

        let caption = embed.caption.expect("açıklama bulunmalı");
        assert!(
            caption.contains("Let\u{2019}s set a world record together!"),
            "caption: {caption}"
        );
        assert!(caption.contains("#EggGang"), "etiket korunmalı: {caption}");
        assert!(
            !caption.contains("world_record_egg"),
            "kullanıcı adı çıkarılmalı: {caption}"
        );
        assert!(
            !caption.contains('<'),
            "HTML etiketleri temizlenmeli: {caption}"
        );
    }

    #[test]
    fn open_graph_helpers_are_parsed() {
        let desc = "975K likes, 16K comments - instagram on August 26, 2022: \"Gingerton\". ";
        assert_eq!(
            stat_from_description(&OG_LIKES_REGEX, Some(desc)),
            Some(975_000)
        );
        assert_eq!(
            stat_from_description(&OG_COMMENTS_REGEX, Some(desc)),
            Some(16_000)
        );
        assert_eq!(parse_ig_date(desc), parse_timestamp("2022-08-26T00:00:00Z"));
        assert_eq!(
            caption_from_og_description(desc).as_deref(),
            Some("Gingerton")
        );
        assert_eq!(
            display_name_from_title("Instagram on Instagram: \"...\"").as_deref(),
            Some("Instagram")
        );
        assert_eq!(
            display_name_from_title("Instagram"),
            None,
            "sadece site adı ad sayılmaz"
        );
        assert_eq!(
            username_from_og_url("https://www.instagram.com/world_record_egg/p/BsOGulcndj-/")
                .as_deref(),
            Some("world_record_egg")
        );
    }

    #[test]
    fn build_post_merges_embed_and_open_graph() {
        let graph = opengraph::parse_open_graph(
            r#"<meta property="og:site_name" content="Instagram" />
               <meta property="og:title" content="Just An Egg &#x1f95a; on Instagram: &quot;Let&#x2019;s set a world record&quot;" />
               <meta property="og:url" content="https://www.instagram.com/world_record_egg/p/BsOGulcndj-/" />
               <meta property="og:image" content="https://scontent.cdninstagram.com/thumb.jpg" />
               <meta property="og:description" content="60M likes, 4M comments - world_record_egg on January 4, 2019: &quot;Let&#x2019;s set a world record&quot;. " />"#,
        );

        let post = build_post(
            &parsed("p", "BsOGulcndj-"),
            "p",
            Some(parse_embed(EMBED_PHOTO)),
            Some(graph),
            None,
        );

        assert_eq!(post.platform, Platform::Instagram);
        assert_eq!(
            post.author.name, "Just An Egg \u{1f95a}",
            "og:title'dan görünen ad"
        );
        assert_eq!(post.author.handle, "world_record_egg");
        assert!(post.author.verified);
        assert_eq!(
            post.author.profile_url.as_deref(),
            Some("https://www.instagram.com/world_record_egg/")
        );
        // Embed (tam) metrikleri og'nin kısaltmalı değerlerini geçersiz kılar.
        assert_eq!(post.stats.likes, 60_279_326);
        assert_eq!(post.stats.replies, 3_771_804);
        assert_eq!(post.created_at, parse_timestamp("2019-01-04T00:00:00Z"));
        assert!(post.text.contains("world record together"));
        assert_eq!(post.media.len(), 1);
        assert!(post.media[0].is_image());
        assert_eq!(post.extra_label, None, "fotoğraf gönderisi etiket almaz");
    }

    #[test]
    fn reel_post_is_labelled_as_video() {
        let post = build_post(
            &parsed("reel", "Chunk8-jurw"),
            "reel",
            Some(parse_embed(EMBED_REEL)),
            None,
            None,
        );

        assert_eq!(post.author.handle, "instagram");
        assert_eq!(post.extra_label.as_deref(), Some("Reel"));
        assert_eq!(post.stats.likes, 975_000);
        assert_eq!(post.stats.replies, 16_000);
        assert_eq!(post.media.len(), 1);
        assert!(
            post.media[0].is_image(),
            "video kapağı görsel olarak taşınır"
        );
    }

    #[test]
    fn open_graph_only_post_still_builds() {
        let graph = opengraph::parse_open_graph(
            r#"<meta property="og:title" content="Somebody on Instagram: &quot;Merhaba&quot;" />
               <meta property="og:url" content="https://www.instagram.com/somebody/reel/CDUMkliABpa/" />
               <meta property="og:image" content="https://cdn/photo.jpg" />
               <meta property="og:description" content="120 likes, 3 comments - somebody on March 2, 2023: &quot;Merhaba dünya&quot;. " />"#,
        );

        let post = build_post(
            &parsed("reel", "CDUMkliABpa"),
            "reel",
            None,
            Some(graph),
            None,
        );

        assert_eq!(post.author.name, "Somebody");
        assert_eq!(post.author.handle, "somebody");
        assert_eq!(post.text, "Merhaba dünya");
        assert_eq!(post.stats.likes, 120);
        assert_eq!(post.stats.replies, 3);
        assert_eq!(post.extra_label.as_deref(), Some("Reel"));
        assert_eq!(post.media.len(), 1);
        assert_eq!(post.created_at, parse_timestamp("2023-03-02T00:00:00Z"));
    }

    /// Canlı kaynak testi: Instagram'ın herkese açık uçları gerçek veri döndürmeli.
    #[tokio::test]
    #[ignore = "requires live Instagram access; run explicitly"]
    async fn live_fetch_photo_and_reel() {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap();

        let photo = fetch(&client, &parsed("p", "BsOGulcndj-"), None)
            .await
            .expect("Instagram fotoğraf gönderisi çekilmeli");
        assert_eq!(photo.author.handle, "world_record_egg");
        assert!(photo.author.verified, "doğrulanmış hesap işareti gelmeli");
        assert!(!photo.media.is_empty(), "görsel gelmeli");
        assert!(photo.stats.likes > 0, "beğeni sayısı gelmeli");

        let reel = fetch(&client, &parsed("reel", "Chunk8-jurw"), None)
            .await
            .expect("Instagram reel gönderisi çekilmeli");
        assert_eq!(reel.author.handle, "instagram");
        assert_eq!(reel.extra_label.as_deref(), Some("Reel"));
        assert!(!reel.media.is_empty(), "video kapak karesi gelmeli");
    }

    /// Canlı test: **geçersiz** oturum çerezi akışı bozmamalı.
    ///
    /// Instagram çerezi reddeder (giriş sayfasına yönlendirir); kart yine herkese açık
    /// kaynaklarla üretilmeli ve video iddia edilmemelidir (kapak karesi kalır).
    #[tokio::test]
    #[ignore = "requires live Instagram access; run explicitly"]
    async fn live_invalid_session_falls_back_to_cover() {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap();

        let post = fetch(
            &client,
            &parsed("reel", "DbOKxSpolaP"),
            Some("gecersiz-cerez-degeri"),
        )
        .await
        .expect("geçersiz çerezde bile kart üretilmeli");

        assert_eq!(post.author.handle, "efewissiex");
        assert_eq!(
            post.extra_label.as_deref(),
            Some("Reel"),
            "reel etiketi korunur"
        );
        assert!(!post.media.is_empty(), "kapak karesi gelmeli");
        assert!(
            post.media.iter().all(Media::is_image),
            "çerez geçersizken video iddia edilmez: {:?}",
            post.media
        );
    }

    /// Canlı test: **geçerli** oturum çerezi verildiğinde reel **mp4** olarak gelmeli.
    ///
    /// `INSTAGRAM_SESSIONID` ortam değişkeni yoksa test atlanır (CI'da sır bulunmaz).
    #[tokio::test]
    #[ignore = "requires a private Instagram cookie and live access; run explicitly"]
    async fn live_session_reel_is_playable_when_cookie_is_set() {
        let sessionid =
            std::env::var("INSTAGRAM_SESSIONID").expect("live test requires INSTAGRAM_SESSIONID");
        assert!(
            !sessionid.trim().is_empty(),
            "live test requires a nonempty cookie"
        );

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap();
        let post = fetch(
            &client,
            &parsed("reel", "DbOKxSpolaP"),
            Some(sessionid.trim()),
        )
        .await
        .expect("reel çekilmeli");

        let video = post
            .media
            .iter()
            .find(|media| media.is_video())
            .unwrap_or_else(|| panic!("reel mp4 olarak gelmeli: {:?}", post.media));
        assert!(
            video.url.contains(".mp4"),
            "mp4 bağlantısı beklenir: {}",
            video.url
        );
        assert!(video.poster.is_some(), "video kapağı olmalı");
        assert_eq!(post.author.handle, "efewissiex");
        assert_eq!(post.extra_label.as_deref(), Some("Reel"));
    }
}
