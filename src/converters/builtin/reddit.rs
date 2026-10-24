//! Reddit gönderi verisi.
//!
//! Veri, Reddit'in **OAuth API'sinden** (`oauth.reddit.com`) çekilir: gönderi başlığı, selftext,
//! skor, yorum sayısı, video/galeri medyası ve crosspost verisi içerir.
//!
//! Reddit, kimliksiz `.json` isteklerini `403` (JS meydan okuması) ile engellediği için
//! **`installed_client`** OAuth akışı kullanılır: kullanıcı izni/parolası gerekmez, yalnızca
//! herkese açık istemci kimliği yeterlidir (redlib'in kullandığı yöntem). Token bellekte
//! önbelleklenir ve süresi dolmadan yenilenir. Kimlik doğrulama başarısız olursa (engellenmemiş
//! IP'lerde çalışan) kimliksiz `www.reddit.com` yolu denenir; o da olmazsa proxy yedeğine düşülür.
//!
//! Video: Reddit'in verdiği mp4 (`fallback_url`, genelde `CMAF_*.mp4`) **video-only**'dir —
//! Reddit sesi ayrı akışta tutar. Proje **ffmpeg kullanmaz**; bu yüzden video kartta sessiz
//! oynar, sesli izlemek için gönderi bağlantısı (`[Link]`) Reddit'e gider. `secure_media`
//! alanı da (bazı gönderilerde medya yalnızca orada) desteklenir.

use std::sync::LazyLock;
use std::time::{Duration, Instant};

use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use tokio::sync::Mutex;
use tracing::debug;

use super::{
    Author, BuiltinPost, LinkPreview, MAX_BODY_BYTES, Media, MediaKind, REQUEST_TIMEOUT, Stats,
};
use crate::converters::parser::{ParsedPost, Platform};
use crate::utils::truncate_chars;
use url::Url;

/// Reddit, tarayıcı dışı istekleri 403 ile engeller. fxreddit servisinin de kullandığı,
/// gerçek bir Firefox istemcisi kılığındaki User-Agent gönderilir.
const REDDIT_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:109.0) Gecko/20100101 Firefox/116.0";

/// Kalıcı adresteki gönderi kimliğini (`.../comments/{id}`) yakalar.
static COMMENTS_ID_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/comments/([A-Za-z0-9]+)").expect("geçerli reddit regex"));

/// `redd.it/{id}` biçimindeki kısa linkteki gönderi kimliğini yakalar.
static SHORT_ID_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"redd\.it/([A-Za-z0-9]+)").expect("geçerli reddit regex"));

static COMMENT_ID_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"/comments/[A-Za-z0-9]+/[^/?#]+/([A-Za-z0-9]+)")
        .expect("geçerli reddit yorum regex")
});

const MAX_COMMENT_CONTEXT_CHARS: usize = 1_200;

/// Harici bağlantı önizlemesi için toplam süre: kart **asla** bağlantı için bekletilmez.
const LINK_PREVIEW_TIMEOUT: Duration = Duration::from_millis(2_500);
/// Harici bağlantı satırındaki başlık sınırı (Discord'un yerleşik önizlemesi gibi tek satır).
const LINK_PREVIEW_TITLE_LIMIT: usize = 180;
/// Alan adı etiketi sınırı.
const LINK_PREVIEW_SITE_LIMIT: usize = 80;

// =====================================================================================
// OAUTH (installed_client)
// =====================================================================================

/// Reddit'in **herkese açık** OAuth istemci kimliği (redlib'in de kullandığı Android uygulama
/// kimliği). `installed_client` akışı kullanıcı izni/parolası istemez. `.env`'deki
/// `REDDIT_CLIENT_ID` ile kendi uygulamanızın kimliğiyle değiştirilebilir.
#[cfg(test)]
pub const DEFAULT_CLIENT_ID: &str = crate::config::DEFAULT_REDDIT_CLIENT_ID;

/// Token uç noktası ve OAuth API tabanı.
const OAUTH_TOKEN_URL: &str = "https://www.reddit.com/api/v1/access_token";
const OAUTH_API_BASE: &str = "https://oauth.reddit.com";
/// `installed_client` akışında `device_id` zorunludur; "izleme yok" değeri kabul edilir.
const OAUTH_DEVICE_ID: &str = "DO_NOT_TRACK_THIS_DEVICE";
/// Token süresi dolmadan bu kadar önce yenilenir.
const TOKEN_REFRESH_MARGIN: Duration = Duration::from_secs(120);

/// Bellekte tutulan OAuth token'ı (süre bitince yenilenir).
#[derive(Debug, Clone)]
struct TokenEntry {
    token: String,
    expires_at: Instant,
}

/// Süreç genelinde paylaşılan token önbelleği: her gönderi için yeni token alınmaz.
static TOKEN: LazyLock<Mutex<Option<TokenEntry>>> = LazyLock::new(|| Mutex::new(None));
/// Başarısız token isteğinden sonra yeniden denemenin en erken anı.
static TOKEN_RETRY_AFTER: LazyLock<Mutex<Option<Instant>>> = LazyLock::new(|| Mutex::new(None));
/// Token ucu çöktüğünde her Reddit dönüşümü aynı kilitte sırayla zaman aşımını beklemesin:
/// başarısızlık bu süre boyunca hatırlanır ve doğrudan kimliksiz yola düşülür.
const TOKEN_FAILURE_BACKOFF: Duration = Duration::from_secs(60);

/// `Basic` kimlik doğrulaması için minimal base64 kodlayıcı (ek bağımlılık yok).
fn base64_ascii(input: &str) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0] as u32;
        let second = chunk.get(1).copied().unwrap_or(0) as u32;
        let third = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (first << 16) | (second << 8) | third;
        out.push(ALPHABET[((triple >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((triple >> 12) & 0x3F) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[((triple >> 6) & 0x3F) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(triple & 0x3F) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// `installed_client` akışıyla bearer token döner (önbellekten tazeyse ağ isteği yapılmaz).
///
/// Herhangi bir hata `None` döner; çağıran taraf kimliksiz yola düşer, akış bozulmaz.
pub(super) async fn oauth_token(client: &reqwest::Client, client_id: &str) -> Option<String> {
    // Taze token varken yenileme kilidi hiç beklenmez.
    if let Some(token) = cached_token().await {
        return Some(token);
    }
    let _refresh = super::coordination::lock("reddit:oauth".into()).await;
    // Kilidi bekleyen başka bir çağrı token'ı bu arada almış ya da az önce başarısız olmuş olabilir.
    if let Some(token) = cached_token().await {
        return Some(token);
    }
    if TOKEN_RETRY_AFTER
        .lock()
        .await
        .is_some_and(|retry_after| Instant::now() < retry_after)
    {
        return None;
    }
    let token = request_token(client, client_id).await;
    *TOKEN_RETRY_AFTER.lock().await = token
        .is_none()
        .then(|| Instant::now() + TOKEN_FAILURE_BACKOFF);
    token
}

async fn cached_token() -> Option<String> {
    TOKEN
        .lock()
        .await
        .as_ref()
        .filter(|entry| Instant::now() < entry.expires_at)
        .map(|entry| entry.token.clone())
}

/// Token ucundan yeni token alır ve önbelleğe yazar; herhangi bir hata `None` döner.
async fn request_token(client: &reqwest::Client, client_id: &str) -> Option<String> {
    // Reddit `installed_client` akışında istemci kimliği Basic kullanıcı adı, parola boştur.
    let credentials = base64_ascii(&format!("{client_id}:"));
    let body = format!(
        "grant_type=https%3A%2F%2Foauth.reddit.com%2Fgrants%2Finstalled_client&device_id={OAUTH_DEVICE_ID}"
    );

    let response = client
        .post(OAUTH_TOKEN_URL)
        .header("authorization", format!("Basic {credentials}"))
        .header("user-agent", REDDIT_USER_AGENT)
        .header("content-type", "application/x-www-form-urlencoded")
        .header("accept", "*/*")
        .timeout(REQUEST_TIMEOUT)
        .body(body)
        .send()
        .await;

    let response = match response {
        Ok(response) if response.status().is_success() => response,
        Ok(response) => {
            debug!(
                "Reddit OAuth token isteği reddedildi: {}",
                response.status()
            );
            return None;
        }
        Err(err) => {
            debug!("Reddit OAuth token isteği başarısız: {err}");
            return None;
        }
    };

    let json = read_json_response(response, OAUTH_TOKEN_URL).await.ok()?;
    let token = json.get("access_token")?.as_str()?.to_string();
    let expires_in = json
        .get("expires_in")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(3_600);
    let lifetime = Duration::from_secs(expires_in).saturating_sub(TOKEN_REFRESH_MARGIN);

    let mut guard = TOKEN.lock().await;
    *guard = Some(TokenEntry {
        token: token.clone(),
        expires_at: Instant::now() + lifetime,
    });
    debug!(
        "Reddit OAuth token alındı ({} sn geçerli)",
        lifetime.as_secs()
    );

    Some(token)
}

/// Gönderi verisini Reddit'ten çeker: önce **OAuth** (`oauth.reddit.com`, bearer token),
/// olmazsa kimliksiz `www.reddit.com` yolu. İkisi de başarısız olursa hata döner ve (ayar
/// açıksa) proxy yedeğine düşülür.
pub async fn fetch(
    client: &reqwest::Client,
    parsed: &ParsedPost,
    client_id: &str,
) -> Result<BuiltinPost, String> {
    let id = extract_post_id(client, &parsed.original_url).await?;
    let comment_id = comment_id_from_url(&parsed.original_url);
    let query = comments_query(comment_id);

    // 1) OAuth: Reddit'in kimliksiz istekleri engellediği (403) durumlarda da çalışır.
    if let Some(token) = oauth_token(client, client_id).await {
        let url = format!("{OAUTH_API_BASE}/comments/{id}?{query}");
        match get_json_with_bearer(client, &url, &token).await {
            Ok(value) => match listings_to_post(value, comment_id) {
                Ok(mut post) => {
                    fill_link_preview(&mut post).await;
                    return Ok(post);
                }
                Err(err) => debug!("Reddit OAuth yanıtı kullanılamadı ({id}): {err}"),
            },
            Err(err) => debug!("Reddit OAuth isteği başarısız ({id}): {err}"),
        }
    }

    // 2) Kimliksiz yol: engellenmemiş IP'lerde (ör. ev bağlantısı) çalışır.
    let url = format!("https://www.reddit.com/comments/{id}.json?{query}");
    let value = get_reddit_json(client, &url).await?;
    let mut post = listings_to_post(value, comment_id)?;
    fill_link_preview(&mut post).await;
    Ok(post)
}

/// Yorum listesi sorgusu; `raw_json=1` her zaman bulunur çünkü metin alanları bir daha HTML çözülmez.
///
/// Yorum bağlantısında `comment` + `context=0` istenir: varsayılan ilk 20 yorumda bulunmayan yorum da `listings[1]`'in kökü olarak gelir.
fn comments_query(comment_id: Option<&str>) -> String {
    match comment_id {
        Some(comment_id) => {
            format!("raw_json=1&depth=2&limit=20&comment={comment_id}&context=0")
        }
        None => "raw_json=1&depth=2&limit=20".to_string(),
    }
}

/// JSON gönderi listesini karta çevirir (crosspost ise asıl içerik ebeveyn gönderdedir).
fn listings_to_post(
    value: serde_json::Value,
    comment_id: Option<&str>,
) -> Result<BuiltinPost, String> {
    let listings: Vec<RedditListing> =
        serde_json::from_value(value).map_err(|e| format!("Reddit yanıtı çözümlenemedi: {e}"))?;

    let post = listings
        .first()
        .and_then(|listing| listing.data.children.first())
        .map(|child| &child.data)
        .ok_or_else(|| "Reddit gönderisi bulunamadı".to_string())?;

    let content = post
        .crosspost_parent_list
        .as_deref()
        .and_then(|parents| parents.first())
        .unwrap_or(post);

    let mut built = reddit_to_post(post, content);
    if let Some(comment_id) = comment_id.and_then(|id| find_comment(&listings, id)) {
        append_comment_context(&mut built, comment_id);
    }
    Ok(built)
}

/// Gönderi kimliğini linkten çıkarır; kimlik adres üzerinde yoksa (mobil paylaşım linki)
/// linki yönlendirme izleyerek kalıcı adrese çözer.
async fn extract_post_id(client: &reqwest::Client, original_url: &str) -> Result<String, String> {
    // 1) Adres üzerinde gömülü kimlik (normal linkler ve kısa linkler).
    if let Some(id) = post_id_from_url(original_url) {
        return Ok(id);
    }

    // 2) Mobil paylaşım linki (`/r/{sub}/s/{kod}`): Reddit içeriği yalnızca hedef adrese
    //    yönlendirerek verir. Link tarayıcı User-Agent'ı ile istenir ve sonunda varılan
    //    kalıcı adresten kimlik okunur (fxreddit ve redlib de bunu yapar).
    let response = client
        .get(original_url)
        .header("user-agent", REDDIT_USER_AGENT)
        .header(
            "accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("Reddit paylaşım linki çözülmedi ({original_url}): {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "Beklenmeyen durum kodu ({original_url}): {}",
            response.status()
        ));
    }

    let final_url = response.url().to_string();
    post_id_from_url(&final_url)
        .ok_or_else(|| format!("Reddit gönderi kimliği çözümlenemedi: {original_url}"))
}

/// Gönderi kimliği adres üzerinde taşınıyorsa döner: normal `/r/.../comments/{id}` linkleri
/// (`COMMENTS_ID_REGEX`) ve `redd.it/{id}` kısa linkleri (`SHORT_ID_REGEX`).
fn post_id_from_url(original_url: &str) -> Option<String> {
    if let Some(id) = COMMENTS_ID_REGEX
        .captures(original_url)
        .and_then(|c| c.get(1))
    {
        return Some(id.as_str().to_string());
    }
    SHORT_ID_REGEX
        .captures(original_url)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
}

fn comment_id_from_url(original_url: &str) -> Option<&str> {
    COMMENT_ID_REGEX
        .captures(original_url)
        .and_then(|captures| captures.get(1))
        .map(|match_| match_.as_str())
}

/// Finds only the requested direct comment in the bounded comments listing. Reddit replies are
/// represented as nested `replies.data.children`; no extra request or unbounded traversal occurs.
fn find_comment<'a>(listings: &'a [RedditListing], comment_id: &str) -> Option<&'a RedditPost> {
    listings
        .iter()
        .flat_map(|listing| listing.data.children.iter())
        .find_map(|child| find_comment_in_post(&child.data, comment_id, 0))
}

fn find_comment_in_post<'a>(
    post: &'a RedditPost,
    comment_id: &str,
    depth: usize,
) -> Option<&'a RedditPost> {
    if depth > 4 {
        return None;
    }
    if post.id.as_deref() == Some(comment_id) && post.body.is_some() {
        return Some(post);
    }
    post.replies
        .as_ref()
        .and_then(RedditReplies::listing)
        .and_then(|data| {
            data.children
                .iter()
                .find_map(|child| find_comment_in_post(&child.data, comment_id, depth + 1))
        })
}

fn append_comment_context(post: &mut BuiltinPost, comment: &RedditPost) {
    let Some(body) = comment
        .body
        .as_deref()
        .map(str::trim)
        .filter(|body| !body.is_empty() && *body != "[removed]" && *body != "[deleted]")
    else {
        return;
    };
    let body = quote_lines(&truncate_chars(body, MAX_COMMENT_CONTEXT_CHARS));
    if body.is_empty() {
        return;
    }
    let author = comment
        .author
        .as_deref()
        .filter(|author| !author.trim().is_empty())
        .map(|author| format!("u/{author}"))
        .unwrap_or_else(|| "silinmiş kullanıcı".to_string());
    if !post.text.is_empty() {
        post.text.push_str("\n\n");
    }
    post.text
        .push_str(&format!("**Yorum bağlamı — {author}:**\n{body}"));
}

/// Her dolu satırı `> ` ile alıntılar; boş satırlar atılır çünkü Discord çıplak `>` satırını ayrı, boş bir şerit gibi çizer.
fn quote_lines(text: &str) -> String {
    text.lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .map(|line| format!("> {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Kimliksiz JSON isteği. Tarayıcı kılığındaki User-Agent, Reddit'in bot isteklerine
/// uyguladığı 403 engelinin önüne geçmeye çalışır (engellenen IP'lerde OAuth devreye girer).
async fn get_reddit_json(client: &reqwest::Client, url: &str) -> Result<serde_json::Value, String> {
    let response = client
        .get(url)
        .header("user-agent", REDDIT_USER_AGENT)
        .header("accept", "application/json")
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

    read_json_response(response, url).await
}

/// OAuth API isteği (`Authorization: bearer`).
pub(super) async fn get_json_with_bearer(
    client: &reqwest::Client,
    url: &str,
    token: &str,
) -> Result<serde_json::Value, String> {
    let response = client
        .get(url)
        .header("authorization", format!("bearer {token}"))
        .header("user-agent", REDDIT_USER_AGENT)
        .header("accept", "application/json")
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("OAuth isteği başarısız ({url}): {e}"))?;

    let status = response.status();
    if matches!(
        status,
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
    ) {
        invalidate_token(token).await;
    }
    if !status.is_success() {
        return Err(format!("Beklenmeyen durum kodu ({url}): {status}"));
    }

    read_json_response(response, url).await
}

/// Reddit'in reddettiği token'ı önbellekten atar; aksi hâlde süresi dolana dek (≈1 saat) her istek aynı reddi alırdı.
///
/// Yalnızca reddedilen token silinir: bu arada başka bir istek yenisini almışsa o korunur.
async fn invalidate_token(token: &str) {
    let mut guard = TOKEN.lock().await;
    if guard.as_ref().is_some_and(|entry| entry.token == token) {
        *guard = None;
        debug!("Reddit OAuth token'ı reddedildi; sonraki istekte yenilenecek");
    }
}

/// Yanıt gövdesini `MAX_BODY_BYTES` sınırıyla okuyup JSON'a çevirir.
async fn read_json_response(
    mut response: reqwest::Response,
    url: &str,
) -> Result<serde_json::Value, String> {
    if let Some(len) = response.content_length()
        && len > MAX_BODY_BYTES as u64
    {
        return Err(format!("Yanıt çok büyük ({url}): {len} bayt"));
    }

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

    let text = String::from_utf8(body).map_err(|_| format!("Yanıt UTF-8 değil ({url}"))?;
    serde_json::from_str(&text).map_err(|e| format!("JSON çözümlenemedi ({url}): {e}"))
}

fn reddit_to_post(post: &RedditPost, content: &RedditPost) -> BuiltinPost {
    let author_name = content.author.clone().unwrap_or_default();
    let mut author = Author::new(format!("u/{author_name}"), String::new());
    author.profile_url = Some(format!("https://www.reddit.com/user/{author_name}"));

    let permalink = content
        .permalink
        .clone()
        .or_else(|| post.permalink.clone())
        .unwrap_or_default();
    let url = if permalink.is_empty() {
        format!(
            "https://www.reddit.com/comments/{}",
            post.id.clone().unwrap_or_default()
        )
    } else {
        format!("https://www.reddit.com{permalink}")
    };

    let mut built = BuiltinPost::new(Platform::Reddit, url, author);
    built.title = content
        .title
        .clone()
        .or_else(|| post.title.clone())
        .map(|title| title.trim().to_string());
    built.text = content
        .selftext
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty() && *text != "[removed]" && *text != "[deleted]")
        .map(str::to_string)
        .unwrap_or_default();
    // Galeri görsellerinin kendi açıklamaları (görsel üstüne yazılan metin) karta girer:
    // galeri bileşeni yalnızca görsel taşıdığı için açıklamalar metne numaralı eklenir.
    let captions = gallery_captions(content);
    if captions.iter().any(|caption| !caption.is_empty()) {
        if !built.text.is_empty() {
            built.text.push_str("\n\n");
        }
        built.text.push_str("**Görsel açıklamaları**");
        for (index, caption) in captions.iter().enumerate() {
            if caption.is_empty() {
                continue;
            }
            built.text.push_str(&format!("\n{}. {caption}", index + 1));
        }
    }
    built.created_at = content
        .created_utc
        .and_then(|secs| (secs > 0.0).then_some(secs as i64));
    built.sensitive = content.over_18.unwrap_or(false);
    built.source_spoiler = content.spoiler.unwrap_or(false);
    built.stats = Stats {
        replies: content.num_comments.unwrap_or(0),
        reposts: 0,
        likes: content
            .score
            .or(content.ups)
            .map_or(0, |score| score.max(0) as u64),
        quotes: 0,
        views: None,
    };
    built.media = reddit_media(content);
    if built.media.is_empty()
        && let Some(redgifs_url) = redgifs_source_url(content).and_then(|url| redgifs_link(&url))
    {
        built.link_preview = Some(LinkPreview {
            url: redgifs_url,
            title: built.title.clone(),
            description: (!built.text.is_empty()).then(|| built.text.clone()),
            image: preview_image(content),
            site_name: Some("Redgifs".to_string()),
        });
    }
    // Harici bağlantı satırı: Discord'un yerleşik Reddit önizlemesi gönderinin dış bağlantısını
    // `Site | Başlık` olarak tek satırda gösterir. Başlık/site **async** katmanda (`fetch`)
    // gönderinin kendi sayfasından (OG) doldurulur; burada yalnızca adres işaretlenir.
    // `image` **bilerek boş bırakılır**: kart sağ üstünde/galerisinde ek görsel istemiyoruz.
    if built.media.is_empty()
        && built.link_preview.is_none()
        && let Some(url) = external_link_url(content)
    {
        built.link_preview = Some(LinkPreview {
            url,
            ..Default::default()
        });
    }
    built.extra_label = reddit_extra_label(content);

    built
}

/// `r/rust • Tartışma` biçiminde ek bilgi satırı.
fn reddit_extra_label(post: &RedditPost) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(subreddit) = post.subreddit_name_prefixed.as_deref() {
        parts.push(subreddit.to_string());
    }
    if let Some(flair) = post.link_flair_text.as_deref()
        && !flair.trim().is_empty()
    {
        parts.push(flair.trim().to_string());
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" • "))
    }
}

/// Gönderi medyasını çıkarır: galeri > video > doğrudan görsel > önizleme görseli.
fn reddit_media(post: &RedditPost) -> Vec<Media> {
    if post.is_gallery.unwrap_or(false)
        && let Some(gallery) = gallery_media(post)
        && !gallery.is_empty()
    {
        return gallery;
    }

    if let Some(media) = redgifs_media(post) {
        return vec![media];
    }

    // Gönderinin videosu `media` **veya** `secure_media` altında olabilir (fxreddit de ikisini
    // okur; bazı gönderilerde medya yalnızca `secure_media` içinde bulunur).
    let reddit_video = post
        .media
        .as_ref()
        .or(post.secure_media.as_ref())
        .and_then(|media| media.reddit_video.as_ref());
    if let Some(video) = reddit_video
        && let Some(url) = video.fallback_url.as_deref()
    {
        return vec![Media {
            kind: MediaKind::Video,
            url: unescape_url(url),
            poster: preview_image(post),
            width: video.width,
            height: video.height,
            duration: video.duration,
        }];
    }

    if let Some(url) = direct_image_url(post) {
        let (width, height) = preview_dimensions(post);
        return vec![Media {
            kind: MediaKind::Image,
            url,
            poster: None,
            width,
            height,
            duration: None,
        }];
    }

    if let Some(url) = preview_image(post) {
        return vec![Media {
            kind: MediaKind::Image,
            url,
            poster: None,
            width: None,
            height: None,
            duration: None,
        }];
    }

    Vec::new()
}

/// Redgifs embeds are accepted only from the service's HTTPS host family. Page links become a
/// canonical link preview; direct mp4/HLS URLs already present in the payload can be rendered.
fn redgifs_media(post: &RedditPost) -> Option<Media> {
    let candidate = post
        .media
        .as_ref()
        .and_then(|media| media.redgifs.as_ref().or(media.oembed.as_ref()))
        .and_then(|media| {
            media
                .video_url
                .as_deref()
                .or(media.url.as_deref())
                .or(media.hls_url.as_deref())
        })
        .or_else(|| {
            post.secure_media
                .as_ref()
                .and_then(|media| media.redgifs.as_ref().or(media.oembed.as_ref()))
                .and_then(|media| {
                    media
                        .video_url
                        .as_deref()
                        .or(media.url.as_deref())
                        .or(media.hls_url.as_deref())
                })
        })?;
    allowed_redgifs_asset(candidate).map(|url| Media {
        kind: MediaKind::Video,
        url,
        poster: post
            .media
            .as_ref()
            .and_then(|media| media.redgifs.as_ref().or(media.oembed.as_ref()))
            .and_then(|media| media.thumbnail_url.clone()),
        width: None,
        height: None,
        duration: None,
    })
}

fn redgifs_source_url(post: &RedditPost) -> Option<String> {
    post.url_overridden_by_dest.clone().or_else(|| {
        post.media
            .as_ref()
            .and_then(|media| media.oembed.as_ref())
            .and_then(|media| media.url.clone())
    })
}

fn redgifs_link(raw: &str) -> Option<String> {
    let (scheme, remainder) = raw.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    let (host, path) = remainder.split_once('/').unwrap_or((remainder, ""));
    let host = host.split(':').next()?.to_ascii_lowercase();
    if host != "redgifs.com" && host != "www.redgifs.com" {
        return None;
    }
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let mut segments = path.split('/').filter(|segment| !segment.is_empty());
    let kind = segments.next()?;
    if !kind.eq_ignore_ascii_case("watch") && !kind.eq_ignore_ascii_case("ifr") {
        return None;
    }
    let id = segments.next()?;
    if id.is_empty()
        || !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        return None;
    }
    Some(format!("https://www.redgifs.com/watch/{id}"))
}

fn allowed_redgifs_asset(raw: &str) -> Option<String> {
    let (scheme, remainder) = raw.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("https") {
        return None;
    }
    let (host, path) = remainder.split_once('/').unwrap_or((remainder, ""));
    let host = host.split(':').next()?.to_ascii_lowercase();
    if host != "redgifs.com" && !host.ends_with(".redgifs.com") {
        return None;
    }
    let path = path
        .split(['?', '#'])
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase();
    [".mp4", ".m3u8", ".webm"]
        .iter()
        .any(|ext| path.ends_with(ext))
        .then(|| raw.to_string())
}

/// Gönderinin **harici** hedef adresi: Reddit dışı ve `http(s)`.
///
/// Reddit'in kendi sayfaları (yorum/galeri bağlantıları) dış bağlantı sayılmaz; medya olarak
/// işlenen adresler zaten galeriye girer.
fn external_link_url(post: &RedditPost) -> Option<String> {
    let raw = post.url_overridden_by_dest.as_deref()?.trim();
    let url = Url::parse(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let is_reddit = host == "reddit.com"
        || host.ends_with(".reddit.com")
        || host == "redd.it"
        || host.ends_with(".redd.it");
    // Gönderen kişinin verdiği adres kartta `[etiket](url)` içine basılır: `)`/`[`/`]` kaçırılmazsa
    // bağlantı kırılır ya da sahte bir maskeli bağlantı enjekte edilebilir.
    (!is_reddit).then(|| super::preview::link_url(&url))
}

/// Adresin alan adı (`www.` atılmış); ayrıştırılamazsa `None`.
fn host_label(raw: &str) -> Option<String> {
    let host = Url::parse(raw)
        .ok()?
        .host_str()?
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

/// OG değerini tek satıra indirip sınıra kısaltır.
fn meta_value(value: Option<String>, limit: usize) -> Option<String> {
    value
        .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|text| !text.is_empty())
        .map(|text| truncate_chars(&text, limit))
}

/// Harici bağlantı satırının başlık/site alanlarını hedef sayfanın OG verisinden doldurur.
///
/// Kart **bekletilmez** ([`LINK_PREVIEW_TIMEOUT`]) ve önizleme alınamazsa hata verilmez:
/// satır alan adına düşer (`v2::link_preview_component`), böylece bağlantı kartta görünür kalır.
async fn fill_link_preview(post: &mut BuiltinPost) {
    let Some(url) = post
        .link_preview
        .as_ref()
        .map(|preview| preview.url.clone())
        .filter(|url| !url.is_empty())
    else {
        return;
    };

    let host = host_label(&url);
    let graph =
        match tokio::time::timeout(LINK_PREVIEW_TIMEOUT, super::safe_http::preview(&url)).await {
            Ok(Ok((_final_url, html))) => Some(super::opengraph::parse_open_graph(&html)),
            Ok(Err(err)) => {
                debug!("Harici bağlantı önizlemesi alınamadı ({url}): {err}");
                None
            }
            Err(_elapsed) => {
                debug!(
                    "Harici bağlantı önizlemesi {} ms içinde gelmedi: {url}",
                    LINK_PREVIEW_TIMEOUT.as_millis()
                );
                None
            }
        };

    let Some(preview) = post.link_preview.as_mut() else {
        return;
    };
    preview.site_name = graph
        .as_ref()
        .and_then(|graph| meta_value(graph.site_name.clone(), LINK_PREVIEW_SITE_LIMIT))
        .or_else(|| host.clone());
    // Başlık yoksa alan adına düşülür: satır yine de gösterilir (bağlantı kaybolmaz).
    preview.title = graph
        .and_then(|graph| meta_value(graph.title, LINK_PREVIEW_TITLE_LIMIT))
        .or(host);
}

/// `url_overridden_by_dest` doğrudan bir görsel dosyasıysa onu döndürür.
fn direct_image_url(post: &RedditPost) -> Option<String> {
    let url = post.url_overridden_by_dest.as_deref()?;
    let path = url.split('?').next().unwrap_or(url).to_ascii_lowercase();
    let is_image = [".jpg", ".jpeg", ".png", ".webp", ".gif"]
        .iter()
        .any(|ext| path.ends_with(ext));
    is_image.then(|| unescape_url(url))
}

/// `preview.images[0].source.url` (HTML kaçışlı `&amp;` temizlenir).
fn preview_image(post: &RedditPost) -> Option<String> {
    let raw = post
        .preview
        .as_ref()
        .and_then(|preview| preview.images.as_ref())
        .and_then(|images| images.first())
        .and_then(|image| image.source.as_ref())
        .and_then(|source| source.url.clone())?;
    Some(unescape_url(&raw))
}

fn preview_dimensions(post: &RedditPost) -> (Option<u32>, Option<u32>) {
    let source = post
        .preview
        .as_ref()
        .and_then(|preview| preview.images.as_ref())
        .and_then(|images| images.first())
        .and_then(|image| image.source.as_ref());
    match source {
        Some(source) => (source.width, source.height),
        None => (None, None),
    }
}

/// Galeri görsellerinin kendi açıklamalarını **görsel sırasıyla** döner (boşlar `""`).
///
/// Reddit'te her galeri görselinin üstüne açıklama yazılabilir (`gallery_data.items[*].caption`
/// veya `media_metadata[*].caption`). V2 kartın galeri bileşeni yalnızca görsel taşıdığı için
/// bu açıklamalar gönderi metnine numaralı olarak eklenir; aksi hâlde kartta hiç görünmez.
fn gallery_captions(post: &RedditPost) -> Vec<String> {
    let meta = post.media_metadata.as_ref();
    let items = post
        .gallery_data
        .as_ref()
        .and_then(|data| data.items.as_ref());

    // Sıra: `gallery_data.items` (yoksa medya meta anahtarları) — medya listesiyle aynı kural.
    let order: Vec<String> = items
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.media_id.clone())
                .collect()
        })
        .or_else(|| meta.map(|meta| meta.keys().cloned().collect()))
        .unwrap_or_default();

    order
        .iter()
        .enumerate()
        .map(|(index, media_id)| {
            // Önce `gallery_data.items[i].caption`, yoksa `media_metadata[id].caption`.
            let from_item = items
                .and_then(|items| items.get(index))
                .and_then(|item| item.caption.as_deref());
            let from_meta = meta
                .and_then(|meta| meta.get(media_id))
                .and_then(|entry| entry.caption.as_deref());
            clean_caption(from_item.or(from_meta))
        })
        .collect()
}

/// Açıklamayı karta uygun hâle getirir: boş/boşluk girdi `""` olur.
///
/// HTML varlıkları çözülmez: `raw_json=1` metni zaten çözülmüş verir, ikinci çözüm metindeki gerçek `&lt;` yazısını `<` yapardı.
fn clean_caption(caption: Option<&str>) -> String {
    caption
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_default()
}

/// Galeri gönderilerinde `media_metadata` + `gallery_data` sıralamasını kullanır.
fn gallery_media(post: &RedditPost) -> Option<Vec<Media>> {
    let metadata = post.media_metadata.as_ref()?;
    let order: Vec<String> = post
        .gallery_data
        .as_ref()
        .and_then(|data| data.items.as_ref())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.media_id.clone())
                .collect()
        })
        .unwrap_or_else(|| metadata.keys().cloned().collect());

    let media: Vec<Media> = order
        .iter()
        .filter_map(|media_id| metadata.get(media_id))
        .filter_map(gallery_item_to_media)
        .collect();

    (!media.is_empty()).then_some(media)
}

fn gallery_item_to_media(item: &MediaMeta) -> Option<Media> {
    let source = item.s.as_ref()?;

    // Hareketli görseller (gif) mp4 olarak da sunulur.
    if item.e.as_deref() == Some("AnimatedImage")
        && let Some(mp4) = source.mp4.as_deref()
    {
        return Some(Media {
            kind: MediaKind::Gif,
            url: unescape_url(mp4),
            poster: source.u.as_deref().map(unescape_url),
            width: source.x,
            height: source.y,
            duration: None,
        });
    }

    Some(Media {
        kind: MediaKind::Image,
        url: unescape_url(source.u.as_deref()?),
        poster: None,
        width: source.x,
        height: source.y,
        duration: None,
    })
}

// =====================================================================================
// JSON MODELİ
// =====================================================================================

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RedditListing {
    data: RedditListingData,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RedditListingData {
    children: Vec<RedditChild>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RedditChild {
    data: RedditPost,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RedditPost {
    id: Option<String>,
    title: Option<String>,
    selftext: Option<String>,
    /// Comment bodies share the listing item model with posts.
    body: Option<String>,
    author: Option<String>,
    subreddit_name_prefixed: Option<String>,
    link_flair_text: Option<String>,
    permalink: Option<String>,
    url_overridden_by_dest: Option<String>,
    /// Yorumlarda skor sıklıkla negatiftir; `u64` olsaydı tek bir eksi yorum tüm yanıtın çözümünü bozardı.
    score: Option<i64>,
    ups: Option<i64>,
    num_comments: Option<u64>,
    created_utc: Option<f64>,
    over_18: Option<bool>,
    spoiler: Option<bool>,
    is_gallery: Option<bool>,
    gallery_data: Option<GalleryData>,
    media_metadata: Option<HashMap<String, MediaMeta>>,
    media: Option<RedditMedia>,
    /// Bazı gönderilerde video yalnızca burada bulunur (fxreddit de bunu okur).
    secure_media: Option<RedditMedia>,
    preview: Option<RedditPreview>,
    crosspost_parent_list: Option<Vec<RedditPost>>,
    replies: Option<RedditReplies>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RedditReplies {
    Listing { data: Option<RedditListingData> },
    Empty(serde_json::Value),
}

impl RedditReplies {
    fn listing(&self) -> Option<&RedditListingData> {
        match self {
            Self::Listing { data } => data.as_ref(),
            Self::Empty(value) => {
                let _ = value;
                None
            }
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RedditMedia {
    reddit_video: Option<RedditVideo>,
    redgifs: Option<RedgifsMedia>,
    oembed: Option<RedgifsMedia>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RedgifsMedia {
    #[serde(alias = "content_url", alias = "download_url")]
    video_url: Option<String>,
    url: Option<String>,
    hls_url: Option<String>,
    thumbnail_url: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RedditVideo {
    fallback_url: Option<String>,
    /// Reddit sesi ayrı akışta tutar; `false` ise video zaten sessizdir (GIF benzeri).
    has_audio: Option<bool>,
    is_gif: Option<bool>,
    width: Option<u32>,
    height: Option<u32>,
    duration: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RedditPreview {
    images: Option<Vec<RedditPreviewImage>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RedditPreviewImage {
    source: Option<RedditImageSource>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RedditImageSource {
    url: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct GalleryData {
    items: Option<Vec<GalleryItem>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct GalleryItem {
    media_id: Option<String>,
    /// Açıklamanın `gallery_data` tarafında tutulduğu yanıtlar için.
    caption: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct MediaMeta {
    #[serde(rename = "e")]
    e: Option<String>,
    s: Option<MediaMetaSource>,
    /// Görselin üstüne yazılan açıklama (Reddit galerilerinde bulunur).
    caption: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct MediaMetaSource {
    u: Option<String>,
    mp4: Option<String>,
    x: Option<u32>,
    y: Option<u32>,
}

/// Reddit URL'lerindeki `&amp;` kaçışını düzeltir.
pub fn unescape_url(url: &str) -> String {
    url.replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> BuiltinPost {
        let listings: Vec<RedditListing> =
            serde_json::from_str(json).expect("örnek JSON geçerli olmalı");
        let post = &listings[0].data.children[0].data;
        // `fetch` ile aynı davranış: crosspost ise içerik ebeveyn gönderiden alınır.
        let content = post
            .crosspost_parent_list
            .as_deref()
            .and_then(|parents| parents.first())
            .unwrap_or(post);
        reddit_to_post(post, content)
    }

    #[test]
    fn external_link_posts_are_marked_for_the_link_line() {
        let post = parse(
            r#"[{ "data": { "children": [{ "data": {
                "id": "link1",
                "title": "Release Note for 3/8/2024",
                "author": "walk3",
                "subreddit_name_prefixed": "r/GlobalOffensive",
                "permalink": "/r/GlobalOffensive/comments/link1/release_note/",
                "url_overridden_by_dest": "https://blog.counter-strike.net/index.php/2024/03/steam-event/"
            }}] }}]"#,
        );
        let preview = post
            .link_preview
            .expect("harici bağlantı, link satırı için işaretlenmeli");
        assert_eq!(
            preview.url,
            "https://blog.counter-strike.net/index.php/2024/03/steam-event/"
        );
        // Başlık/site **async** katmanda (OG) doldurulur: burada yalnızca adres işaretlenir.
        assert!(preview.title.is_none());
        // Kartta ek görsel istenmez: link satırı görsel taşımaz.
        assert!(preview.image.is_none());
    }

    #[test]
    fn external_link_url_cannot_break_out_of_the_markdown_link() {
        let post = parse(
            r#"[{ "data": { "children": [{ "data": {
                "id": "link2",
                "title": "t",
                "permalink": "/r/test/comments/link2/t/",
                "url_overridden_by_dest": "https://example.com/a)[Bedava](https://phish.example/x"
            }}] }}]"#,
        );
        let url = post.link_preview.expect("link satırı").url;
        assert!(
            !url.contains(')') && !url.contains('[') && !url.contains(']'),
            "{url}"
        );
    }

    #[test]
    fn self_posts_and_media_posts_have_no_link_line() {
        // Reddit'in kendi adresine işaret eden gönderi (self/metin) dış bağlantı değildir.
        let self_post = parse(
            r#"[{ "data": { "children": [{ "data": {
                "id": "self1",
                "title": "Sohbet",
                "selftext": "merhaba",
                "author": "biri",
                "permalink": "/r/rust/comments/self1/sohbet/",
                "url_overridden_by_dest": "https://www.reddit.com/r/rust/comments/self1/sohbet/"
            }}] }}]"#,
        );
        assert!(
            self_post.link_preview.is_none(),
            "Reddit'in kendi adresi dış bağlantı sayılmaz"
        );

        // Medyası olan gönderide harici adres link satırına dönüşmez (medya galeride kalır).
        let media_post = parse(
            r#"[{ "data": { "children": [{ "data": {
                "id": "img1",
                "title": "Manzara",
                "author": "gezgin",
                "permalink": "/r/EarthPorn/comments/img1/manzara/",
                "url_overridden_by_dest": "https://blog.counter-strike.net/x",
                "preview": { "images": [{ "source": { "url": "https://preview.redd.it/x.jpg" } }] }
            }}] }}]"#,
        );
        assert_eq!(
            media_post.media.len(),
            1,
            "önizleme görseli galeriye girmeli"
        );
        assert!(
            media_post.link_preview.is_none(),
            "medyalı gönderide link satırı olmaz"
        );
    }

    #[test]
    fn image_post_is_converted_with_metrics() {
        let json = r#"[{
            "kind": "Listing",
            "data": { "children": [{ "kind": "t3", "data": {
                "id": "abc123",
                "title": "Güzel bir manzara",
                "selftext": "",
                "author": "gezgin",
                "subreddit_name_prefixed": "r/EarthPorn",
                "link_flair_text": "Manzara",
                "permalink": "/r/EarthPorn/comments/abc123/guzel_bir_manzara/",
                "url_overridden_by_dest": "https://i.redd.it/ornek.jpg",
                "score": 15234,
                "num_comments": 421,
                "created_utc": 1539202764,
                "over_18": false,
                "preview": { "images": [{ "source": {
                    "url": "https://preview.redd.it/ornek.jpg?width=1080&amp;format=pjpg",
                    "width": 1080, "height": 720
                }}] }
            }}] }
        }]"#;

        let post = parse(json);
        assert_eq!(post.platform, Platform::Reddit);
        assert_eq!(post.title.as_deref(), Some("Güzel bir manzara"));
        assert_eq!(
            post.url,
            "https://www.reddit.com/r/EarthPorn/comments/abc123/guzel_bir_manzara/"
        );
        assert_eq!(post.author.label(), "u/gezgin");
        assert_eq!(post.stats.likes, 15234);
        assert_eq!(post.stats.replies, 421);
        assert_eq!(post.created_at, Some(1539202764));
        assert_eq!(post.extra_label.as_deref(), Some("r/EarthPorn • Manzara"));
        assert!(!post.sensitive);
        assert_eq!(post.media.len(), 1);
        assert_eq!(post.media[0].kind, MediaKind::Image);
        assert_eq!(post.media[0].width, Some(1080));
    }

    #[test]
    fn image_post_title_reaches_the_v2_card_above_the_image() {
        let post = parse(
            r#"[{ "data": { "children": [{ "data": {
            "id": "1x1quvc",
            "title": "Samsung S26 ve pc için alınır mı? Ldac desteği olmaması kararsız bırakıyor.",
            "selftext": "",
            "author": "AlexanderMirzayev",
            "permalink": "/r/teknoloji/comments/1x1quvc/samsung_s26_ve_pc_için_alınır_mı_ldac_desteği/",
            "url_overridden_by_dest": "https://i.redd.it/ornek.jpg"
        }}] }}]"#,
        );
        assert!(post.text.is_empty());
        let options = super::super::render::RenderOptions::from_engine(
            &crate::settings::EngineSettings::default(),
            "reddit",
        );
        let resolved = super::super::render::ResolvedMedia {
            images: vec![post.media[0].url.clone()],
            ..Default::default()
        };
        let emojis = crate::settings::EmojiSettings::default();
        let payload = super::super::v2::build_payload(&super::super::v2::V2Card {
            post: &post,
            options: &options,
            resolved: &resolved,
            emojis: &emojis,
            spoiler: false,
            sender: None,
            user_comment: None,
            impersonation: None,
            allowed_mentions: None,
        });
        let children = payload["components"][0]["components"].as_array().unwrap();
        assert_eq!(children[1]["type"], 10);
        assert_eq!(
            children[1]["content"],
            "**Samsung S26 ve pc için alınır mı? Ldac desteği olmaması kararsız bırakıyor.**"
        );
        assert_eq!(children[2]["type"], 12);
        assert_eq!(children[2]["items"][0]["media"]["url"], post.media[0].url);
    }

    #[test]
    fn hosted_video_uses_fallback_url_and_preview_poster() {
        let json = r#"[{ "data": { "children": [{ "data": {
            "id": "vid1",
            "title": "Video",
            "author": "video_cu",
            "subreddit_name_prefixed": "r/videos",
            "permalink": "/r/videos/comments/vid1/video/",
            "score": 10,
            "num_comments": 2,
            "created_utc": 1539202764.0,
            "over_18": true,
            "media": { "reddit_video": {
                "fallback_url": "https://v.redd.it/vid1/DASH_720.mp4",
                "width": 1280, "height": 720, "duration": 42
            } },
            "preview": { "images": [{ "source": { "url": "https://preview.redd.it/vid1.jpg" } }] }
        }}]}}]"#;

        let post = parse(json);
        assert!(post.sensitive, "NSFW gönderiler spoiler olmalı");
        assert_eq!(post.media.len(), 1);
        assert_eq!(post.media[0].kind, MediaKind::Video);
        assert_eq!(post.media[0].url, "https://v.redd.it/vid1/DASH_720.mp4");
        assert_eq!(
            post.media[0].poster.as_deref(),
            Some("https://preview.redd.it/vid1.jpg")
        );
        assert_eq!(post.media[0].duration, Some(42));
    }

    #[test]
    fn reddit_source_spoiler_is_distinct_from_nsfw_metadata() {
        let post = parse(
            r#"[{ "data": { "children": [{ "data": {
                "id": "spoiler1",
                "title": "Spoiler",
                "author": "writer",
                "permalink": "/r/test/comments/spoiler1/spoiler/",
                "over_18": false,
                "spoiler": true
            }}] }}]"#,
        );
        assert!(!post.sensitive, "spoiler alone is not NSFW metadata");
        assert!(post.source_spoiler, "Reddit spoiler metadata is retained");
    }

    #[test]
    fn gallery_order_follows_gallery_data() {
        let json = r#"[{ "data": { "children": [{ "data": {
            "id": "gal1",
            "title": "Galeri",
            "author": "foto",
            "permalink": "/r/pics/comments/gal1/galeri/",
            "is_gallery": true,
            "gallery_data": { "items": [ { "media_id": "b" }, { "media_id": "a" } ] },
            "media_metadata": {
                "a": { "e": "Image", "s": { "u": "https://i.redd.it/a.jpg", "x": 10, "y": 20 } },
                "b": { "e": "Image", "s": { "u": "https://i.redd.it/b.jpg", "x": 30, "y": 40 } }
            }
        }}]}}]"#;

        let post = parse(json);
        let urls: Vec<&str> = post.media.iter().map(|m| m.url.as_str()).collect();
        assert_eq!(
            urls,
            vec!["https://i.redd.it/b.jpg", "https://i.redd.it/a.jpg"]
        );
        assert_eq!(post.media[0].width, Some(30));
    }

    #[test]
    fn gallery_captions_are_appended_to_the_card_text() {
        let json = r#"[{ "data": { "children": [{ "data": {
            "id": "cap1",
            "title": "Galeri",
            "author": "foto",
            "permalink": "/r/pics/comments/cap1/galeri/",
            "is_gallery": true,
            "selftext": "Açıklamalı galeri",
            "gallery_data": { "items": [
                { "media_id": "b", "caption": "İlk görsel açıklaması" },
                { "media_id": "a" }
            ] },
            "media_metadata": {
                "a": { "e": "Image", "caption": "İkinci görselin açıklaması", "s": { "u": "https://i.redd.it/a.jpg" } },
                "b": { "e": "Image", "caption": "Yedek: gallery_data yoksa bu kullanılır", "s": { "u": "https://i.redd.it/b.jpg" } }
            }
        }}]}}]"#;

        let post = parse(json);

        // `gallery_data.items[*].caption` önceliklidir; sıra medya sırasıyla aynıdır.
        assert!(
            post.text.contains("**Görsel açıklamaları**"),
            "açıklama başlığı eklenmeli: {}",
            post.text
        );
        assert!(
            post.text.contains("1. İlk görsel açıklaması"),
            "1. görselin açıklaması: {}",
            post.text
        );
        assert!(
            post.text.contains("2. İkinci görselin açıklaması"),
            "gallery_data'da yoksa media_metadata'dan alınmalı: {}",
            post.text
        );
        assert!(
            post.text.starts_with("Açıklamalı galeri"),
            "selftext korunur"
        );
    }

    #[test]
    fn gallery_without_captions_keeps_text_clean() {
        let json = r#"[{ "data": { "children": [{ "data": {
            "id": "nocap",
            "title": "Galeri",
            "author": "foto",
            "permalink": "/r/pics/comments/nocap/galeri/",
            "is_gallery": true,
            "gallery_data": { "items": [ { "media_id": "a" } ] },
            "media_metadata": {
                "a": { "e": "Image", "s": { "u": "https://i.redd.it/a.jpg" } }
            }
        }}]}}]"#;

        let post = parse(json);
        assert!(
            !post.text.contains("Görsel açıklamaları"),
            "açıklama yoksa ek blok eklenmez: {}",
            post.text
        );
        assert!(post.text.is_empty());
    }

    #[test]
    fn crosspost_uses_parent_content() {
        let json = r#"[{ "data": { "children": [{ "data": {
            "id": "cp1",
            "title": "Crosspost başlığı",
            "author": "paylasan",
            "subreddit_name_prefixed": "r/xpost",
            "permalink": "/r/xpost/comments/cp1/crosspost/",
            "crosspost_parent_list": [{
                "id": "parent1",
                "title": "Asıl başlık",
                "author": "ozgun",
                "subreddit_name_prefixed": "r/original",
                "permalink": "/r/original/comments/parent1/asil/",
                "url_overridden_by_dest": "https://i.redd.it/asil.jpg"
            }]
        }}]}}]"#;

        let post = parse(json);
        assert_eq!(post.title.as_deref(), Some("Asıl başlık"));
        assert_eq!(post.author.label(), "u/ozgun");
        assert_eq!(
            post.url,
            "https://www.reddit.com/r/original/comments/parent1/asil/"
        );
        assert_eq!(post.media[0].url, "https://i.redd.it/asil.jpg");
    }

    #[test]
    fn removed_selftext_is_dropped_and_escaped_urls_cleaned() {
        let json = r#"[{ "data": { "children": [{ "data": {
            "id": "del1",
            "title": "Silinmiş",
            "selftext": "[removed]",
            "author": "biri",
            "permalink": "/r/test/comments/del1/silinmis/",
            "url_overridden_by_dest": "https://i.redd.it/x.jpg?a=1&amp;b=2"
        }}]}}]"#;

        let post = parse(json);
        assert!(post.text.is_empty(), "silinmiş selftext gösterilmemeli");
        assert_eq!(post.media[0].url, "https://i.redd.it/x.jpg?a=1&b=2");
    }

    #[test]
    fn post_id_is_extracted_from_url_forms() {
        // Normal kalıcı link (subreddit + başlık + sondaki slash).
        assert_eq!(
            post_id_from_url("https://www.reddit.com/r/rust/comments/1abc2x/bir_baslik/"),
            Some("1abc2x".into())
        );
        // Kullanıcı profil linki (old.reddit.com + /user/).
        assert_eq!(
            post_id_from_url("https://old.reddit.com/user/biri/comments/abc123/x/"),
            Some("abc123".into())
        );
        // Kısa link (redd.it).
        assert_eq!(
            post_id_from_url("https://redd.it/1w9fa2z"),
            Some("1w9fa2z".into())
        );
        // Mobil paylaşım linki adres üzerinde kimlik taşımaz → yönlendirme adımı gerekir.
        assert_eq!(
            post_id_from_url("https://www.reddit.com/r/MutfakDertlileri/s/9spRJSE8Ar"),
            None
        );
    }

    #[test]
    fn base64_matches_known_vectors() {
        // Bu değer gerçek OAuth isteğinde kullanıldı ve curl ile doğrulandı.
        assert_eq!(base64_ascii("ohXpoqrZYub1kg:"), "b2hYcG9xclpZdWIxa2c6");
        assert_eq!(base64_ascii("a"), "YQ==");
        assert_eq!(base64_ascii("ab"), "YWI=");
        assert_eq!(base64_ascii("abc"), "YWJj");
        assert_eq!(base64_ascii(""), "");
    }

    #[test]
    fn secure_media_video_is_used_when_media_is_missing() {
        // Bazı gönderilerde `media` boş gelir; video yalnızca `secure_media` altındadır
        // (fxreddit de bu yüzden iki alanı birlikte okur).
        let json = r#"[{
            "kind": "Listing",
            "data": { "children": [{ "kind": "t3", "data": {
                "id": "sec123",
                "title": "Sadece secure_media iceren gonderi",
                "author": "biri",
                "subreddit_name_prefixed": "r/Deneme",
                "permalink": "/r/Deneme/comments/sec123/baslik/",
                "score": 12,
                "num_comments": 3,
                "created_utc": 1700000000.0,
                "post_hint": "hosted:video",
                "is_video": true,
                "secure_media": { "reddit_video": {
                    "fallback_url": "https://v.redd.it/secvid/CMAF_720.mp4?source=fallback",
                    "width": 720,
                    "height": 1280,
                    "duration": 30,
                    "has_audio": true,
                    "is_gif": false
                } },
                "preview": { "images": [{ "source": { "url": "https://preview.redd.it/sec.jpg" } }] }
            }}]}}
        ]"#;

        let post = parse(json);
        assert_eq!(post.media.len(), 1);
        assert_eq!(post.media[0].kind, MediaKind::Video);
        assert_eq!(
            post.media[0].url,
            "https://v.redd.it/secvid/CMAF_720.mp4?source=fallback"
        );
        assert_eq!(
            post.media[0].poster.as_deref(),
            Some("https://preview.redd.it/sec.jpg")
        );
        assert_eq!(post.media[0].duration, Some(30));
        assert_eq!(post.media[0].width, Some(720));
        assert_eq!(post.media[0].height, Some(1280));
    }

    #[test]
    fn direct_comment_context_is_bounded_and_deleted_comments_are_safe() {
        let body = "x".repeat(MAX_COMMENT_CONTEXT_CHARS + 100);
        let json = format!(
            r#"[
                {{"data":{{"children":[{{"data":{{"id":"post1","title":"Gönderi","author":"op","permalink":"/r/test/comments/post1/baslik/"}}}}]}}}},
                {{"data":{{"children":[{{"data":{{"id":"comment1","author":"yorumcu","body":"{body}"}}}},{{"data":{{"id":"deleted","author":null,"body":"[deleted]"}}}}]}}}}
            ]"#
        );
        let post = listings_to_post(
            serde_json::from_str(&json).expect("listing geçerli olmalı"),
            Some("comment1"),
        )
        .expect("gönderi dönüşmeli");
        assert!(post.text.contains("Yorum bağlamı — u/yorumcu:"));
        assert!(post.text.chars().count() < MAX_COMMENT_CONTEXT_CHARS + 100);

        let deleted = listings_to_post(
            serde_json::from_str(&json).expect("listing geçerli olmalı"),
            Some("deleted"),
        )
        .expect("silinmiş yorum kartı bozmaz");
        assert!(!deleted.text.contains("Yorum bağlamı"));
    }

    #[test]
    fn negative_comment_scores_do_not_break_the_listing() {
        let json = r#"[
            {"data":{"children":[{"data":{"id":"post1","title":"Gönderi","author":"op","score":-3,"permalink":"/r/test/comments/post1/baslik/"}}]}},
            {"data":{"children":[{"data":{"id":"c1","author":"yorumcu","body":"eksi","score":-42,"ups":-42}}]}}
        ]"#;
        let post = listings_to_post(
            serde_json::from_str(json).expect("listing geçerli olmalı"),
            Some("c1"),
        )
        .expect("negatif skor yanıtı bozmamalı");
        assert_eq!(post.stats.likes, 0, "negatif skor sıfıra kırpılır");
        assert!(post.text.contains("> eksi"));
    }

    #[test]
    fn comment_links_request_the_comment_as_listing_root() {
        assert_eq!(comments_query(None), "raw_json=1&depth=2&limit=20");
        let cid =
            comment_id_from_url("https://www.reddit.com/r/rust/comments/1abc2x/bir_baslik/kx9z1q/");
        assert_eq!(cid, Some("kx9z1q"));
        assert_eq!(
            comments_query(cid),
            "raw_json=1&depth=2&limit=20&comment=kx9z1q&context=0"
        );
    }

    #[test]
    fn multiline_comment_context_quotes_every_line_without_bare_markers() {
        let json = r#"[
            {"data":{"children":[{"data":{"id":"post1","title":"Gönderi","author":"op","permalink":"/r/test/comments/post1/baslik/"}}]}},
            {"data":{"children":[{"data":{"id":"c1","author":"yorumcu","body":"ilk satır\n\nikinci satır\nüçüncü"}}]}}
        ]"#;
        let post = listings_to_post(
            serde_json::from_str(json).expect("listing geçerli olmalı"),
            Some("c1"),
        )
        .expect("gönderi dönüşmeli");
        let quoted: Vec<&str> = post.text.lines().skip(1).collect();
        assert_eq!(quoted, vec!["> ilk satır", "> ikinci satır", "> üçüncü"]);
        assert!(!post.text.lines().any(|line| line.trim() == ">"));
    }

    #[test]
    fn raw_json_text_is_not_html_decoded_twice() {
        // `raw_json=1` yanıtında `&amp;lt;` metindeki gerçek `&lt;` yazısıdır.
        let post = parse(
            r#"[{"data":{"children":[{"data":{
                "id":"esc1", "title":"a &amp;lt; b", "selftext":"x &lt;tag&gt;", "author":"biri",
                "permalink":"/r/test/comments/esc1/esc/"
            }}]}}]"#,
        );
        assert_eq!(post.title.as_deref(), Some("a &amp;lt; b"));
        assert_eq!(post.text, "x &lt;tag&gt;");
    }

    #[tokio::test]
    async fn rejected_token_is_dropped_but_a_fresh_one_is_kept() {
        let entry = |token: &str| TokenEntry {
            token: token.to_string(),
            expires_at: Instant::now() + Duration::from_secs(60),
        };
        *TOKEN.lock().await = Some(entry("yeni"));
        invalidate_token("eski").await;
        assert_eq!(
            TOKEN.lock().await.as_ref().map(|e| e.token.clone()),
            Some("yeni".to_string())
        );
        invalidate_token("yeni").await;
        assert!(TOKEN.lock().await.is_none());
    }

    #[test]
    fn redgifs_uses_canonical_link_and_rejects_untrusted_redirects() {
        assert_eq!(
            redgifs_link("https://redgifs.com/ifr/AbC_12?param=ignored"),
            Some("https://www.redgifs.com/watch/AbC_12".to_string())
        );
        assert_eq!(redgifs_link("https://evil.example/watch/abc"), None);
        assert_eq!(redgifs_link("http://redgifs.com/watch/abc"), None);

        let post = parse(
            r#"[{"data":{"children":[{"data":{
                "id":"rg1", "title":"Redgifs bağlantısı", "author":"biri",
                "url_overridden_by_dest":"https://redgifs.com/watch/AbC123",
                "permalink":"/r/test/comments/rg1/redgifs/"
            }}]}}]"#,
        );
        assert_eq!(
            post.link_preview
                .as_ref()
                .map(|preview| preview.url.as_str()),
            Some("https://www.redgifs.com/watch/AbC123")
        );
    }

    /// Canlı test: `installed_client` OAuth akışı Reddit API'sinden veri getirmeli.
    ///
    /// Reddit kimliksiz `.json` isteklerini `403` ile engellediği (JS meydan okuması) için
    /// bu akış native motorun çalışması adına zorunludur.
    #[tokio::test]
    #[ignore = "requires live Reddit access; run explicitly"]
    async fn live_oauth_fetch_returns_post() {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap();

        let token = oauth_token(&client, DEFAULT_CLIENT_ID)
            .await
            .expect("installed_client token alınmalı");
        assert!(!token.is_empty());

        // Gönderi kimliği canlı listeden alınır: test silinmiş gönderilere bağlı kalmaz.
        let listing = get_json_with_bearer(
            &client,
            "https://oauth.reddit.com/r/videos/hot?limit=5&raw_json=1",
            &token,
        )
        .await
        .expect("liste isteği başarılı olmalı");
        let id = listing["data"]["children"]
            .as_array()
            .and_then(|children| children.first())
            .and_then(|child| child["data"]["id"].as_str())
            .expect("liste gönderi kimliği içermeli")
            .to_string();

        let parsed = ParsedPost {
            platform: Platform::Reddit,
            is_spoiler: false,
            user_comment: None,
            original_url: format!("https://www.reddit.com/comments/{id}"),
            fixed_url: format!("https://vxreddit.com/comments/{id}"),
        };
        let post = fetch(&client, &parsed, DEFAULT_CLIENT_ID)
            .await
            .expect("Reddit gönderisi çekilmeli");

        assert!(
            post.title
                .as_deref()
                .is_some_and(|title| !title.trim().is_empty()),
            "başlık gelmeli: {:?}",
            post.title
        );
    }
}
