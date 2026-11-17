//! TikTok gönderisi: önce gönderi sayfasının sunucu tarafında gömdüğü
//! `__UNIVERSAL_DATA_FOR_REHYDRATION__` verisi okunur (yazar, açıklama, sayılar, tarih ve
//! oynatılabilir MP4 seçenekleri). Sayfa okunamazsa oEmbed'e düşülür; oEmbed yalnızca başlık,
//! yazar ve kapak verir.
//!
//! Video CDN'i sayfadaki adresleri yalnızca o isteğin `tt_chain_token` çereziyle verir; Discord
//! bu çerezi gönderemez. Bu yüzden kartta fxTikTok'un ([`FXTIKTOK_VIDEO`]) çerezsiz imzalı CDN
//! adresine yönlendiren video adresi uzaktan oynatılır: bot videoyu indirip yüklemez.

use serde::Deserialize;
use tracing::debug;

use super::{Author, BuiltinPost, Media, MediaKind, REQUEST_TIMEOUT, Stats, USER_AGENT};
use crate::converters::parser::{ParsedPost, Platform};

const OEMBED_URL: &str = "https://www.tiktok.com/oembed";
const TIKTOK: &str = "https://www.tiktok.com";
/// fxTikTok (github.com/okdargy/fxtiktok): `<kimlik>` için oynatılabilir MP4'e 302 verir.
const FXTIKTOK_VIDEO: &str = "https://tnktok.com/generate/video/";

/// TikTok gönderisini çeker ve kart verisine dönüştürür.
pub async fn fetch(client: &reqwest::Client, parsed: &ParsedPost) -> Result<BuiltinPost, String> {
    // Doğrudan `/photo/` bağlantısı boş sayfa döndürür: ilk istek `/video/` sayfasına atılır.
    let url = photo_as_video(&parsed.original_url);
    match fetch_page(client, url.as_deref().unwrap_or(&parsed.original_url)).await {
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
    let (final_url, html) = fetch_html(client, url).await?;
    let item = match item_from_html(&html) {
        Ok(item) => item,
        // `/photo/` sayfası gönderi verisini taşımaz (oEmbed de 400 verir); aynı gönderinin
        // `/video/` sayfası taşır.
        Err(reason) => {
            let video_url = photo_as_video(&final_url).ok_or(reason)?;
            item_from_html(&fetch_html(client, &video_url).await?.1)?
        }
    };
    item_post(&item)
}

/// `…/@kullanıcı/photo/<kimlik>` → `…/@kullanıcı/video/<kimlik>` (sorgu atılır).
fn photo_as_video(url: &str) -> Option<String> {
    let mut url = url::Url::parse(url).ok()?;
    let path = url.path().replacen("/photo/", "/video/", 1);
    if path == url.path() {
        return None;
    }
    url.set_path(&path);
    url.set_query(None);
    Some(url.into())
}

/// Sayfayı alır; yönlendirmelerden sonraki son adresi ve HTML'i döndürür.
async fn fetch_html(client: &reqwest::Client, url: &str) -> Result<(String, String), String> {
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
    let final_url = response.url().to_string();
    let html = super::read_limited_text(response, url).await?;
    Ok((final_url, html))
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
        urlencoding::encode(&parsed.original_url)
    );
    let value: serde_json::Value = super::get_json(client, &url).await?;
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
        alt: None,
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
    music: Option<ItemMusic>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct ItemMusic {
    title: Option<String>,
    author_name: Option<String>,
}

impl ItemMusic {
    /// Alt bilgi etiketi: `🎵 Ses adı · Sanatçı` (markdown kaçışlı, kısa).
    fn label(&self) -> Option<String> {
        let clean = |value: &Option<String>| {
            value
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        let title = clean(&self.title)?;
        let text = match clean(&self.author_name) {
            Some(author) if !title.contains(&author) => format!("{title} · {author}"),
            _ => title,
        };
        Some(format!(
            "🎵 {}",
            super::render::escape_markdown(&crate::utils::truncate_chars(&text, 60))
        ))
    }
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
    cover: Option<String>,
    duration: Option<u64>,
    width: Option<u32>,
    height: Option<u32>,
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
    author.profile_url = Some(format!("{TIKTOK}/@{}", urlencoding::encode(handle)));

    let is_photo = item.image_post.is_some();
    let kind = if is_photo { "photo" } else { "video" };
    let url = format!(
        "{TIKTOK}/@{}/{kind}/{}",
        urlencoding::encode(handle),
        item.id
    );
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
    // Videonun video olduğu kartta zaten görünür; etiket yerine kullanılan ses yazılır.
    let music = item.music.as_ref().and_then(ItemMusic::label);
    post.extra_label = match (is_photo, music) {
        (true, Some(music)) => Some(format!("Fotoğraf • {music}")),
        (true, None) => Some("Fotoğraf".into()),
        (false, music) => music,
    };

    if let Some(photos) = &item.image_post {
        post.media.extend(
            photos
                .images
                .iter()
                .filter_map(|photo| {
                    Some(Media {
                        alt: None,
                        width: photo.image_width,
                        height: photo.image_height,
                        ..image(photo.image_url.as_ref()?.first_url()?)
                    })
                })
                .take(10),
        );
    } else if let Some(video) = &item.video {
        let poster = video.cover.clone().filter(|url| allowed_tiktok_url(url));
        match video.duration.filter(|d| *d > 0) {
            Some(duration) => post.media.push(Media {
                alt: None,
                kind: MediaKind::Video,
                url: format!("{FXTIKTOK_VIDEO}{}", item.id),
                poster,
                width: video.width,
                height: video.height,
                duration: Some(duration),
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
                alt: None,
                kind: MediaKind::Video,
                url: video_url,
                poster: thumbnail,
                width: self.thumbnail_width,
                height: self.thumbnail_height,
                duration: self.duration.as_ref().and_then(Count::as_u64),
            });
        } else if let Some(thumbnail) = thumbnail {
            post.media.push(Media {
                alt: None,
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

    #[test]
    fn music_label_is_escaped_short_and_not_repetitive() {
        let music = |title: &str, author: &str| ItemMusic {
            title: Some(title.into()),
            author_name: Some(author.into()),
        };
        assert_eq!(
            music("Espresso", "Sabrina Carpenter").label().as_deref(),
            Some("🎵 Espresso · Sabrina Carpenter")
        );
        assert_eq!(
            music("original sound - scout2015", "scout2015")
                .label()
                .as_deref(),
            Some("🎵 original sound - scout2015")
        );
        assert_eq!(
            music("*yıldız*", "").label().as_deref(),
            Some("🎵 \\*yıldız\\*")
        );
        assert_eq!(ItemMusic::default().label(), None);
    }

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
            urlencoding::encode(&parsed().original_url)
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
    fn page_item_becomes_a_full_card_with_a_remote_fxtiktok_video() {
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
        assert_eq!(
            video.url,
            "https://tnktok.com/generate/video/6718335390845095173"
        );
        assert_eq!((video.width, video.height), (Some(576), Some(1024)));
        assert_eq!(
            video.poster.as_deref(),
            Some("https://p16-common-sign.tiktokcdn.com/cover.jpg")
        );
        assert_eq!(video.duration, Some(10));
    }

    #[test]
    fn photo_page_url_is_retried_as_video_page() {
        assert_eq!(
            photo_as_video("https://www.tiktok.com/@flora/photo/7708955805491531040?_r=1&_t=x")
                .as_deref(),
            Some("https://www.tiktok.com/@flora/video/7708955805491531040")
        );
        assert_eq!(
            photo_as_video("https://www.tiktok.com/@flora/video/7708955805491531040"),
            None
        );
    }

    #[test]
    fn missing_or_failed_detail_is_an_error() {
        assert!(item_from_html("<html></html>").is_err());
        let failed = PAGE.replace(r#""statusCode":0"#, r#""statusCode":10204"#);
        assert!(item_from_html(&failed).is_err());
    }

    #[tokio::test]
    #[ignore = "requires live TikTok access; run explicitly"]
    async fn live_tiktok_photo_short_link_becomes_a_card() {
        let parsed = ParsedPost {
            platform: Platform::TikTok,
            original_url: "https://vt.tiktok.com/ZSptsf7Wc".into(),
            is_spoiler: false,
            user_comment: None,
        };
        let post = fetch(&reqwest::Client::new(), &parsed)
            .await
            .expect("gönderi okunmalı");
        assert!(post.url.contains("/photo/"), "{}", post.url);
        assert!(!post.media.is_empty() && post.media.iter().all(|m| !m.is_video()));
    }

    #[tokio::test]
    #[ignore = "requires live TikTok access; run explicitly"]
    async fn live_tiktok_video_downloads_through_fxtiktok() {
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
        assert!(video.is_video(), "video bekleniyor");
        let file = super::super::media::download_video(
            &reqwest::Client::new(),
            &video.url,
            None,
            super::super::media::DISCORD_FILE_LIMIT,
        )
        .await
        .expect("video fxTikTok üzerinden indirilmeli");
        assert!(file.mime.starts_with("video/"), "{}", file.mime);
    }
}
