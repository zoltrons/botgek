//! TikTok gönderisi: önce gönderi sayfasının sunucu tarafında gömdüğü
//! `__UNIVERSAL_DATA_FOR_REHYDRATION__` verisi okunur (yazar, açıklama, sayılar, tarih ve
//! oynatılabilir MP4 seçenekleri). Sayfa okunamazsa oEmbed'e düşülür; oEmbed yalnızca başlık,
//! yazar ve kapak verir.
//!
//! Video CDN'i adresi yalnızca **o sayfayı veren isteğin** `tt_chain_token` çereziyle ve TikTok
//! Referer'ıyla verir; Discord bu başlıkları gönderemediği için video mesaja dosya olarak eklenir
//! ([`video_headers`] ile indirilir). İndirilemezse kartta kapak görseli gösterilir.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use serde::Deserialize;
use tracing::debug;

use super::{
    Author, BuiltinPost, Media, MediaKind, REQUEST_TIMEOUT, Stats, USER_AGENT, encode_component,
};
use crate::converters::parser::{ParsedPost, Platform};

const OEMBED_URL: &str = "https://www.tiktok.com/oembed";
const TIKTOK: &str = "https://www.tiktok.com";
/// Discord'un boost'suz dosya sınırı: daha büyük kalite seçenekleri ek olarak gönderilemez.
const MAX_VIDEO_BYTES: u64 = super::media::DISCORD_FILE_LIMIT;
/// Bellekte tutulan video adresi → çerez eşlemesi sınırı (dolunca temizlenir).
const MAX_TOKENS: usize = 512;

static VIDEO_TOKENS: LazyLock<Mutex<HashMap<String, String>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// TikTok video adresinin indirilmesi için gereken başlıklar. Adres bu süreçte okunmuş bir
/// TikTok sayfasından gelmiyorsa `None` döner (indirme denenmez).
pub fn video_headers(url: &str) -> Option<Vec<(&'static str, String)>> {
    let token = VIDEO_TOKENS.lock().ok()?.get(url).cloned()?;
    Some(vec![
        ("cookie", format!("tt_chain_token={token}")),
        ("referer", format!("{TIKTOK}/")),
    ])
}

fn remember_token(url: &str, token: &str) {
    if let Ok(mut tokens) = VIDEO_TOKENS.lock() {
        if tokens.len() >= MAX_TOKENS {
            tokens.clear();
        }
        tokens.insert(url.to_string(), token.to_string());
    }
}

/// TikTok gönderisini çeker ve kart verisine dönüştürür.
pub async fn fetch(client: &reqwest::Client, parsed: &ParsedPost) -> Result<BuiltinPost, String> {
    match fetch_page(client, &parsed.original_url).await {
        Ok(post) => Ok(post),
        Err(reason) => {
            debug!("TikTok sayfası okunamadı, oEmbed'e düşülüyor: {reason}");
            fetch_oembed(client, parsed).await
        }
    }
}

/// Gönderi sayfası botun kendi User-Agent'ıyla istenir: tarayıcı kimliğiyle TikTok WAF sınama
/// sayfası döndürür. Kısa bağlantılar (`vm.tiktok.com`, `/t/`) yönlendirmeyle gönderiye varır.
async fn fetch_page(client: &reqwest::Client, url: &str) -> Result<BuiltinPost, String> {
    let response = client
        .get(url)
        .header("user-agent", USER_AGENT)
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("TikTok isteği başarısız: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("TikTok {} döndü", response.status()));
    }
    if super::https_url_on(response.url().as_str(), &["tiktok.com"]).is_none() {
        return Err("TikTok yönlendirmesi platform dışına çıktı".into());
    }
    let token = chain_token(response.headers());
    let html = super::read_limited_text(response, url).await?;
    let mut post = item_post(&item_from_html(&html)?)?;
    match token {
        Some(token) => {
            if let Some(video) = post.media.first().filter(|media| media.is_video()) {
                remember_token(&video.url, &token);
            }
        }
        // Çerezsiz adres her istemcide 403 verir: kapak görseli gösterilir.
        None => video_to_poster(&mut post),
    }
    Ok(post)
}

/// Yanıtın `Set-Cookie` başlıklarındaki `tt_chain_token` değeri.
fn chain_token(headers: &reqwest::header::HeaderMap) -> Option<String> {
    headers
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(|cookie| cookie.strip_prefix("tt_chain_token="))
        .and_then(|rest| rest.split(';').next())
        .filter(|token| {
            !token.is_empty()
                && token
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"+/=_-%.".contains(&b))
        })
        .map(str::to_string)
}

/// İndirilemeyecek videoyu kapak görseline çevirir.
fn video_to_poster(post: &mut BuiltinPost) {
    if post.media.first().is_some_and(Media::is_video) {
        let poster = post.media.remove(0).poster;
        post.media.extend(poster.map(image));
    }
}

async fn fetch_oembed(
    client: &reqwest::Client,
    parsed: &ParsedPost,
) -> Result<BuiltinPost, String> {
    let url = format!(
        "{OEMBED_URL}?url={}",
        encode_component(&parsed.original_url)
    );
    let value = super::get_json(client, &url).await?;
    let oembed: TikTokOEmbed = serde_json::from_value(value)
        .map_err(|e| format!("TikTok oEmbed yanıtı çözümlenemedi: {e}"))?;

    if oembed.title.is_none()
        && oembed.description.is_none()
        && oembed.thumbnail_url.is_none()
        && oembed.video_url().is_none()
    {
        return Err(format!(
            "TikTok gönderisi bulunamadı: {}",
            parsed.original_url
        ));
    }

    // oEmbed çerez vermez: varsa video adresi indirilemez, kapağa çevrilir.
    let mut post = oembed.into_post(parsed);
    video_to_poster(&mut post);
    Ok(post)
}

fn image(url: String) -> Media {
    Media {
        kind: MediaKind::Image,
        url,
        poster: None,
        width: None,
        height: None,
        duration: None,
    }
}

// ----- Sayfa verisi (`webapp.video-detail`) -----

#[derive(Debug, Deserialize)]
struct Rehydration {
    #[serde(rename = "__DEFAULT_SCOPE__")]
    scope: Scope,
}

#[derive(Debug, Deserialize)]
struct Scope {
    #[serde(rename = "webapp.video-detail")]
    detail: Option<Detail>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Detail {
    #[serde(default)]
    status_code: i64,
    item_info: Option<ItemInfo>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ItemInfo {
    item_struct: Item,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    id: String,
    #[serde(default)]
    desc: String,
    create_time: Option<Count>,
    author: ItemAuthor,
    stats: Option<TikTokStats>,
    stats_v2: Option<TikTokStats>,
    video: Option<ItemVideo>,
    image_post: Option<ImagePost>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ItemAuthor {
    unique_id: String,
    #[serde(default)]
    nickname: String,
    #[serde(default)]
    verified: bool,
    avatar_medium: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ItemVideo {
    play_addr: Option<String>,
    cover: Option<String>,
    duration: Option<u64>,
    width: Option<u32>,
    height: Option<u32>,
    bitrate_info: Vec<Bitrate>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Bitrate {
    #[serde(default)]
    bitrate: u64,
    #[serde(default)]
    codec_type: String,
    play_addr: BitrateAddr,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct BitrateAddr {
    data_size: Option<Count>,
    width: Option<u32>,
    height: Option<u32>,
    #[serde(default)]
    url_list: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ImagePost {
    #[serde(default)]
    images: Vec<PostImage>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PostImage {
    #[serde(rename = "imageURL")]
    image_url: Option<TikTokAddress>,
    image_width: Option<u32>,
    image_height: Option<u32>,
}

fn item_from_html(html: &str) -> Result<Item, String> {
    const START: &str =
        r#"<script id="__UNIVERSAL_DATA_FOR_REHYDRATION__" type="application/json">"#;
    let start = html.find(START).ok_or("TikTok sayfa verisi yok")? + START.len();
    let end = html[start..]
        .find("</script>")
        .ok_or("TikTok sayfa verisi kapanmıyor")?;
    let data: Rehydration = serde_json::from_str(&html[start..start + end])
        .map_err(|e| format!("TikTok sayfa verisi çözülemedi: {e}"))?;
    let detail = data.scope.detail.ok_or("TikTok gönderi ayrıntısı yok")?;
    if detail.status_code != 0 {
        return Err(format!(
            "TikTok gönderisi açılamadı (durum {})",
            detail.status_code
        ));
    }
    detail
        .item_info
        .map(|info| info.item_struct)
        .ok_or_else(|| "TikTok gönderisi bulunamadı".into())
}

/// Ek olarak gönderilecek MP4: Discord'un her istemcide oynattığı h264 seçeneklerinden dosya
/// sınırına sığan en yüksek bit hızlısı. Boyutu bilinmeyen seçenek atlanır; hiçbiri uymazsa
/// sayfanın varsayılan oynatma adresi denenir.
fn best_video(video: &ItemVideo) -> Option<(String, Option<u32>, Option<u32>)> {
    video
        .bitrate_info
        .iter()
        .filter(|b| b.codec_type == "h264")
        .filter(|b| {
            b.play_addr
                .data_size
                .as_ref()
                .and_then(Count::as_u64)
                .is_some_and(|size| size <= MAX_VIDEO_BYTES)
        })
        .filter_map(|b| {
            let url = b
                .play_addr
                .url_list
                .iter()
                .find(|u| allowed_tiktok_url(u))?;
            Some((
                b.bitrate,
                (url.clone(), b.play_addr.width, b.play_addr.height),
            ))
        })
        .max_by_key(|(bitrate, _)| *bitrate)
        .map(|(_, choice)| choice)
        .or_else(|| {
            let url = video.play_addr.clone().filter(|u| allowed_tiktok_url(u))?;
            Some((url, video.width, video.height))
        })
}

fn item_post(item: &Item) -> Result<BuiltinPost, String> {
    let handle = item.author.unique_id.trim();
    if handle.is_empty() || item.id.is_empty() || !item.id.bytes().all(|b| b.is_ascii_digit()) {
        return Err("TikTok gönderisinde yazar/kimlik yok".into());
    }
    let name = item.author.nickname.trim();
    let mut author = Author::new(if name.is_empty() { handle } else { name }, handle);
    author.verified = item.author.verified;
    author.avatar = item
        .author
        .avatar_medium
        .clone()
        .filter(|url| allowed_tiktok_url(url));
    author.profile_url = Some(format!("{TIKTOK}/@{}", encode_component(handle)));

    let is_photo = item.image_post.is_some();
    let kind = if is_photo { "photo" } else { "video" };
    let url = format!("{TIKTOK}/@{}/{kind}/{}", encode_component(handle), item.id);
    let mut post = BuiltinPost::new(Platform::TikTok, url, author);
    post.text = super::preview::escape_preview_markdown(&item.desc);
    post.created_at = item.create_time.as_ref().and_then(Count::as_i64);

    // `statsV2` sayıları dize olarak taşır ve 32 bit sınırına takılmaz; yoksa `stats`.
    let count = |pick: fn(&TikTokStats) -> Option<&Count>| {
        [item.stats_v2.as_ref(), item.stats.as_ref()]
            .into_iter()
            .flatten()
            .find_map(|stats| pick(stats).and_then(Count::as_u64))
    };
    post.stats = Stats {
        replies: count(|s| s.comment_count.as_ref()).unwrap_or(0),
        reposts: count(|s| s.share_count.as_ref()).unwrap_or(0),
        likes: count(|s| s.digg_count.as_ref()).unwrap_or(0),
        quotes: 0,
        views: count(|s| s.play_count.as_ref()),
    };
    post.extra_label = Some(if is_photo { "Fotoğraf" } else { "Video" }.into());

    if let Some(photos) = &item.image_post {
        post.media.extend(
            photos
                .images
                .iter()
                .filter_map(|photo| {
                    Some(Media {
                        width: photo.image_width,
                        height: photo.image_height,
                        ..image(photo.image_url.as_ref()?.first_url()?)
                    })
                })
                .take(10),
        );
    } else if let Some(video) = &item.video {
        let poster = video.cover.clone().filter(|url| allowed_tiktok_url(url));
        match best_video(video) {
            Some((url, width, height)) => post.media.push(Media {
                kind: MediaKind::Video,
                url,
                poster,
                width,
                height,
                duration: video.duration.filter(|d| *d > 0),
            }),
            None => post.media.extend(poster.map(image)),
        }
    }
    if post.text.is_empty() && post.media.is_empty() {
        return Err("TikTok gönderisi boş".into());
    }
    Ok(post)
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TikTokOEmbed {
    title: Option<String>,
    author_name: Option<String>,
    #[serde(alias = "authorUniqueId")]
    author_unique_id: Option<String>,
    thumbnail_url: Option<String>,
    thumbnail_width: Option<u32>,
    thumbnail_height: Option<u32>,
    /// Rich item payloads use `desc`; oEmbed itself generally leaves this absent.
    desc: Option<String>,
    description: Option<String>,
    #[serde(alias = "challenges")]
    hashtags: Option<Vec<TikTokHashtag>>,
    #[serde(alias = "followerCount")]
    follower_count: Option<Count>,
    #[serde(rename = "author_stats", alias = "authorStats")]
    author_stats: Option<TikTokAuthorStats>,
    #[serde(alias = "diggCount")]
    digg_count: Option<Count>,
    #[serde(alias = "likeCount")]
    like_count: Option<Count>,
    #[serde(alias = "commentCount")]
    comment_count: Option<Count>,
    #[serde(alias = "shareCount")]
    share_count: Option<Count>,
    #[serde(alias = "playCount")]
    play_count: Option<Count>,
    likes: Option<Count>,
    comments: Option<Count>,
    shares: Option<Count>,
    views: Option<Count>,
    #[serde(rename = "stats")]
    stats: Option<TikTokStats>,
    #[serde(alias = "videoUrl")]
    video_url: Option<String>,
    #[serde(alias = "playAddr")]
    play_addr: Option<TikTokAddress>,
    #[serde(alias = "downloadAddr")]
    download_addr: Option<TikTokAddress>,
    video: Option<TikTokVideo>,
    #[serde(alias = "videoDuration")]
    duration: Option<Count>,
    #[serde(alias = "createTime")]
    create_time: Option<Count>,
    author: Option<TikTokAuthor>,
}

impl TikTokOEmbed {
    fn video_url(&self) -> Option<String> {
        [
            self.video_url.clone(),
            self.play_addr.as_ref().and_then(TikTokAddress::first_url),
            self.download_addr
                .as_ref()
                .and_then(TikTokAddress::first_url),
            self.video.as_ref().and_then(TikTokVideo::video_url),
        ]
        .into_iter()
        .flatten()
        .find(|url| allowed_tiktok_url(url))
    }

    fn into_post(self, parsed: &ParsedPost) -> BuiltinPost {
        let rich_author = self.author.as_ref();
        let handle = self
            .author_unique_id
            .clone()
            .or_else(|| rich_author.and_then(|author| author.unique_id.clone()))
            .unwrap_or_default();
        let mut author = Author::new(
            self.author_name
                .clone()
                .or_else(|| rich_author.and_then(|author| author.nickname.clone()))
                .unwrap_or_else(|| handle.clone()),
            handle.clone(),
        );
        author.profile_url = if handle.is_empty() {
            None
        } else {
            Some(format!("https://www.tiktok.com/@{handle}"))
        };

        let mut post = BuiltinPost::new(Platform::TikTok, parsed.original_url.clone(), author);
        post.title = self.title.clone().filter(|title| !title.trim().is_empty());
        post.text = self
            .description
            .clone()
            .or(self.desc.clone())
            .unwrap_or_default()
            .trim()
            .to_string();
        post.created_at = self.create_time.as_ref().and_then(Count::as_i64);
        let stats = self.stats.as_ref();
        post.stats = Stats {
            replies: self
                .comment_count
                .as_ref()
                .or(self.comments.as_ref())
                .or_else(|| stats.and_then(|stats| stats.comment_count.as_ref()))
                .and_then(Count::as_u64)
                .unwrap_or(0),
            reposts: self
                .share_count
                .as_ref()
                .or(self.shares.as_ref())
                .or_else(|| stats.and_then(|stats| stats.share_count.as_ref()))
                .and_then(Count::as_u64)
                .unwrap_or(0),
            likes: self
                .digg_count
                .as_ref()
                .or(self.like_count.as_ref())
                .or(self.likes.as_ref())
                .or_else(|| stats.and_then(|stats| stats.digg_count.as_ref()))
                .and_then(Count::as_u64)
                .unwrap_or(0),
            quotes: 0,
            views: self
                .play_count
                .as_ref()
                .or(self.views.as_ref())
                .or_else(|| stats.and_then(|stats| stats.play_count.as_ref()))
                .and_then(Count::as_u64),
        };

        let mut labels = vec!["TikTok".to_string()];
        if let Some(followers) = self
            .follower_count
            .as_ref()
            .or_else(|| {
                self.author_stats
                    .as_ref()
                    .and_then(|stats| stats.follower_count.as_ref())
            })
            .and_then(Count::as_u64)
        {
            labels.push(format!("{followers} takipçi"));
        }
        if let Some(tags) = self.hashtags.as_ref() {
            for tag in tags.iter().take(8).filter_map(TikTokHashtag::label) {
                labels.push(format!("#{tag}"));
            }
        }
        post.extra_label = Some(labels.join(" • "));

        let thumbnail = self
            .thumbnail_url
            .clone()
            .or_else(|| self.video.as_ref().and_then(|video| video.cover.clone()));
        if let Some(video_url) = self.video_url() {
            post.media.push(Media {
                kind: MediaKind::Video,
                url: video_url,
                poster: thumbnail,
                width: self.thumbnail_width,
                height: self.thumbnail_height,
                duration: self.duration.as_ref().and_then(Count::as_u64),
            });
        } else if let Some(thumbnail) = thumbnail {
            post.media.push(Media {
                kind: MediaKind::Image,
                url: thumbnail,
                poster: None,
                width: self.thumbnail_width,
                height: self.thumbnail_height,
                duration: None,
            });
        }

        post
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum Count {
    Number(u64),
    Text(String),
}

impl Count {
    fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Number(value) => Some(*value),
            Self::Text(value) => value.trim().parse().ok(),
        }
    }

    fn as_i64(&self) -> Option<i64> {
        self.as_u64().and_then(|value| i64::try_from(value).ok())
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct TikTokStats {
    #[serde(alias = "diggCount")]
    digg_count: Option<Count>,
    #[serde(alias = "commentCount")]
    comment_count: Option<Count>,
    #[serde(alias = "shareCount")]
    share_count: Option<Count>,
    #[serde(alias = "playCount")]
    play_count: Option<Count>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct TikTokAuthorStats {
    #[serde(alias = "followerCount")]
    follower_count: Option<Count>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct TikTokAuthor {
    #[serde(alias = "uniqueId")]
    unique_id: Option<String>,
    nickname: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum TikTokHashtag {
    Text(String),
    Object {
        title: Option<String>,
        name: Option<String>,
    },
}

impl TikTokHashtag {
    fn label(&self) -> Option<String> {
        let value = match self {
            Self::Text(value) => value.as_str(),
            Self::Object { title, name } => title.as_deref().or(name.as_deref())?,
        };
        let value = value.trim().trim_start_matches('#');
        (!value.is_empty()).then(|| value.chars().take(64).collect())
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct TikTokAddress {
    #[serde(alias = "urlList")]
    url_list: Vec<String>,
}

impl TikTokAddress {
    fn first_url(&self) -> Option<String> {
        self.url_list
            .iter()
            .find(|url| allowed_tiktok_url(url))
            .cloned()
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct TikTokVideo {
    #[serde(alias = "playAddr")]
    play_addr: Option<TikTokAddress>,
    #[serde(alias = "downloadAddr")]
    download_addr: Option<TikTokAddress>,
    cover: Option<String>,
}

impl TikTokVideo {
    fn video_url(&self) -> Option<String> {
        self.play_addr
            .as_ref()
            .and_then(TikTokAddress::first_url)
            .or_else(|| {
                self.download_addr
                    .as_ref()
                    .and_then(TikTokAddress::first_url)
            })
    }
}

fn allowed_tiktok_url(raw: &str) -> bool {
    super::https_url_on(
        raw,
        &["tiktok.com", "tiktokcdn.com", "ibytedtos.com", "musical.ly"],
    )
    .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed() -> ParsedPost {
        ParsedPost {
            platform: Platform::TikTok,
            original_url: "https://www.tiktok.com/@kullanici/video/7106594312292453675".to_string(),
            is_spoiler: false,
            user_comment: None,
        }
    }

    #[test]
    fn oembed_response_becomes_a_card_post() {
        let json = r#"{
            "title": "Harika bir dans",
            "author_name": "Kullanıcı",
            "author_unique_id": "kullanici",
            "thumbnail_url": "https://p16-sign.tiktokcdn.com/thumb.jpg",
            "thumbnail_width": 720,
            "thumbnail_height": 1280
        }"#;

        let oembed: TikTokOEmbed = serde_json::from_str(json).expect("örnek JSON geçerli olmalı");
        let post = oembed.into_post(&parsed());

        assert_eq!(post.author.label(), "Kullanıcı (@kullanici)");
        assert_eq!(
            post.author.profile_url.as_deref(),
            Some("https://www.tiktok.com/@kullanici")
        );
        assert_eq!(post.title.as_deref(), Some("Harika bir dans"));
        assert_eq!(post.url, parsed().original_url);
        assert_eq!(post.media.len(), 1);
        assert_eq!(post.media[0].kind, MediaKind::Image);
        assert_eq!(post.media[0].height, Some(1280));
    }

    #[test]
    fn oembed_request_url_is_encoded() {
        let url = format!(
            "{OEMBED_URL}?url={}",
            encode_component(&parsed().original_url)
        );
        assert!(
            url.contains("%3A%2F%2Fwww.tiktok.com%2F%40kullanici"),
            "url: {url}"
        );
    }

    #[test]
    fn rich_fixture_populates_optional_metadata_and_video_without_breaking_oembed_fields() {
        let json = r#"{
            "title": "Başlık",
            "desc": "Açıklama",
            "author_name": "Kullanıcı",
            "author_unique_id": "kullanici",
            "thumbnail_url": "https://p16-sign.tiktokcdn.com/thumb.jpg",
            "thumbnail_width": 720,
            "thumbnail_height": 1280,
            "hashtags": [{"title":"dans"}, "trend"],
            "author_stats": {"follower_count": 12345},
            "stats": {"digg_count": "42", "comment_count": 7, "share_count": 3, "play_count": "9001"},
            "video_url": "https://v16.tiktokcdn.com/video.mp4",
            "duration": 12,
            "create_time": "1714998896"
        }"#;
        let oembed: TikTokOEmbed = serde_json::from_str(json).expect("rich fixture geçerli olmalı");
        let post = oembed.into_post(&parsed());
        assert_eq!(post.text, "Açıklama");
        assert_eq!(post.stats.likes, 42);
        assert_eq!(post.stats.replies, 7);
        assert_eq!(post.stats.reposts, 3);
        assert_eq!(post.stats.views, Some(9001));
        assert_eq!(post.created_at, Some(1714998896));
        assert_eq!(post.media[0].kind, MediaKind::Video);
        assert_eq!(post.media[0].duration, Some(12));
        assert!(
            post.extra_label
                .as_deref()
                .is_some_and(|label| label.contains("12345 takipçi") && label.contains("#dans"))
        );
    }

    #[test]
    fn minimal_or_deleted_like_payload_does_not_accept_untrusted_video_urls() {
        let json = r#"{
            "title": "Silinmiş video",
            "video_url": "https://evil.example/video.mp4",
            "description": ""
        }"#;
        let oembed: TikTokOEmbed =
            serde_json::from_str(json).expect("minimal fixture geçerli olmalı");
        let post = oembed.into_post(&parsed());
        assert!(post.media.is_empty());
        assert_eq!(post.title.as_deref(), Some("Silinmiş video"));
        assert!(!allowed_tiktok_url("http://v16.tiktokcdn.com/video.mp4"));
        // Ayrıştırıcı `\` karakterini yol sayar: gerçek host 169.254.169.254'tür.
        assert!(!allowed_tiktok_url(
            "https://169.254.169.254\\.tiktokcdn.com/video.mp4"
        ));
        assert!(!allowed_tiktok_url(
            "https://user@v16.tiktokcdn.com/video.mp4"
        ));
        assert!(!allowed_tiktok_url("https://nottiktok.com/video.mp4"));
    }

    /// Canlı sayfadan kısaltılmış `__UNIVERSAL_DATA_FOR_REHYDRATION__`.
    const PAGE: &str = r#"<script id="__UNIVERSAL_DATA_FOR_REHYDRATION__" type="application/json">{"__DEFAULT_SCOPE__":{"webapp.video-detail":{"statusCode":0,"itemInfo":{"itemStruct":{
        "id":"6718335390845095173","desc":"Scramble up ur name *now* #foryoupage","createTime":"1564234358",
        "author":{"uniqueId":"scout2015","nickname":"Scout, Suki & Stella","verified":true,"avatarMedium":"https://p16-common-sign.tiktokcdn.com/avt.jpeg"},
        "stats":{"diggCount":35200,"shareCount":1517,"commentCount":5486,"playCount":159600},
        "statsV2":{"diggCount":"35201","shareCount":"1517","commentCount":"5486","playCount":"159600"},
        "video":{"playAddr":"https://v16-webapp-prime.tiktok.com/video/default/","cover":"https://p16-common-sign.tiktokcdn.com/cover.jpg","duration":10,"width":576,"height":1024,
            "bitrateInfo":[
                {"Bitrate":2240963,"CodecType":"h264","PlayAddr":{"DataSize":"2953029","Width":576,"Height":1024,"UrlList":["https://v16-webapp-prime.tiktok.com/video/540/"]}},
                {"Bitrate":9000000,"CodecType":"h264","PlayAddr":{"DataSize":"99000000","Width":1080,"Height":1920,"UrlList":["https://v16-webapp-prime.tiktok.com/video/huge/"]}},
                {"Bitrate":3000000,"CodecType":"h265_hvc1","PlayAddr":{"DataSize":"2004627","Width":720,"Height":1280,"UrlList":["https://v16-webapp-prime.tiktok.com/video/hevc/"]}},
                {"Bitrate":992286,"CodecType":"h264","PlayAddr":{"DataSize":"1307586","Width":576,"Height":1024,"UrlList":["https://evil.example/lowest/"]}}]}
    }}}}}</script>"#;

    #[test]
    fn page_item_becomes_a_full_card_with_the_best_fitting_h264_video() {
        let post = item_post(&item_from_html(PAGE).unwrap()).unwrap();
        assert_eq!(
            post.url,
            "https://www.tiktok.com/@scout2015/video/6718335390845095173"
        );
        assert_eq!(post.author.name, "Scout, Suki & Stella");
        assert_eq!(post.author.handle, "scout2015");
        assert!(post.author.verified);
        assert_eq!(post.text, "Scramble up ur name \\*now\\* #foryoupage");
        assert_eq!(post.created_at, Some(1_564_234_358));
        assert_eq!(post.stats.likes, 35_201, "statsV2 önceliklidir");
        assert_eq!(post.stats.replies, 5_486);
        assert_eq!(post.stats.reposts, 1_517);
        assert_eq!(post.stats.views, Some(159_600));
        assert_eq!(post.media.len(), 1);
        let video = &post.media[0];
        assert!(video.is_video());
        // h265 ve dosya sınırını aşan seçenek atlanır, güvenilmeyen ana bilgisayar kabul edilmez.
        assert_eq!(video.url, "https://v16-webapp-prime.tiktok.com/video/540/");
        assert_eq!(
            video.poster.as_deref(),
            Some("https://p16-common-sign.tiktokcdn.com/cover.jpg")
        );
        assert_eq!(video.duration, Some(10));
    }

    #[test]
    fn chain_token_is_read_from_set_cookie_and_used_for_download() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.append(
            reqwest::header::SET_COOKIE,
            "msToken=abc; Path=/".parse().unwrap(),
        );
        headers.append(
            reqwest::header::SET_COOKIE,
            "tt_chain_token=Ab+c/1==; Max-Age=15552000; HttpOnly"
                .parse()
                .unwrap(),
        );
        assert_eq!(chain_token(&headers).as_deref(), Some("Ab+c/1=="));

        let url = "https://v16-webapp-prime.tiktok.com/video/token-test/";
        assert!(video_headers(url).is_none(), "bilinmeyen adres indirilmez");
        remember_token(url, "Ab+c/1==");
        let headers = video_headers(url).unwrap();
        assert!(headers.contains(&("cookie", "tt_chain_token=Ab+c/1==".to_string())));
        assert!(headers.contains(&("referer", "https://www.tiktok.com/".to_string())));
    }

    #[test]
    fn missing_or_failed_detail_is_an_error() {
        assert!(item_from_html("<html></html>").is_err());
        let failed = PAGE.replace(r#""statusCode":0"#, r#""statusCode":10204"#);
        assert!(item_from_html(&failed).is_err());
    }

    #[tokio::test]
    #[ignore = "requires live TikTok access; run explicitly"]
    async fn live_tiktok_video_downloads_with_the_page_cookie() {
        let url = "https://www.tiktok.com/@scout2015/video/6718335390845095173".to_string();
        let parsed = ParsedPost {
            platform: Platform::TikTok,
            original_url: url.clone(),
            is_spoiler: false,
            user_comment: None,
        };
        let post = fetch(&reqwest::Client::new(), &parsed)
            .await
            .expect("gönderi okunmalı");
        println!("{post:#?}");
        let video = post.media.first().expect("medya olmalı");
        assert!(video.is_video(), "çerezle oynatılabilir video bekleniyor");
        let file = super::super::media::download_video(
            &reqwest::Client::new(),
            &video.url,
            None,
            MAX_VIDEO_BYTES,
        )
        .await
        .expect("video çerezle indirilmeli");
        assert!(file.mime.starts_with("video/"), "{}", file.mime);
    }
}
