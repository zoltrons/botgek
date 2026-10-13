//! TikTok gönderi verisi (oEmbed).
//!
//! TikTok'un herkese açık oEmbed uç noktası yalnızca başlık, yazar ve kapak görseli verir;
//! doğrudan oynatılabilir video akışı yoktur. Bu yüzden native motor TikTok için bir
//! "kart" üretir. Video akışı isteniyorsa varsayılan olan proxy modu kullanılmalıdır.

use serde::Deserialize;

use super::{Author, BuiltinPost, Media, MediaKind, Stats, encode_component};
use crate::converters::parser::{ParsedPost, Platform};

const OEMBED_URL: &str = "https://www.tiktok.com/oembed";

/// TikTok gönderisini oEmbed üzerinden çeker ve kart verisine dönüştürür.
pub async fn fetch(client: &reqwest::Client, parsed: &ParsedPost) -> Result<BuiltinPost, String> {
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

    Ok(oembed.into_post(parsed))
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
    let Some((scheme, remainder)) = raw.split_once("://") else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("https") {
        return false;
    }
    let host = remainder
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .split(':')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    host == "tiktok.com"
        || host.ends_with(".tiktok.com")
        || host.ends_with(".tiktokcdn.com")
        || host.ends_with(".ibytedtos.com")
        || host.ends_with(".musical.ly")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed() -> ParsedPost {
        ParsedPost {
            platform: Platform::TikTok,
            original_url: "https://www.tiktok.com/@kullanici/video/7106594312292453675".to_string(),
            fixed_url: "https://tnktok.com/@kullanici/video/7106594312292453675".to_string(),
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
    }
}
