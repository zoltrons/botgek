//! Reddit gönderi verisi.
//!
//! Genel JSON uç noktası (`reddit.com/comments/{id}.json`) kullanılır: gönderi başlığı,
//! selftext, skor, yorum sayısı, video/galeri medyası ve crosspost verisi içerir.

use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;

use super::{Author, BuiltinPost, Media, MediaKind, Stats};
use crate::converters::parser::{ParsedPost, Platform};

static REDDIT_ID_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?:reddit\.com/(?:r/[^/]+/)?comments/|redd\.it/)([A-Za-z0-9]{4,12})")
        .expect("geçerli reddit regex")
});

/// Gönderi verisini Reddit'in herkese açık JSON uç noktasından çeker.
pub async fn fetch(client: &reqwest::Client, parsed: &ParsedPost) -> Result<BuiltinPost, String> {
    let id = REDDIT_ID_REGEX
        .captures(&parsed.original_url)
        .and_then(|captures| captures.get(1))
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| format!("Reddit gönderi kimliği çözümlenemedi: {}", parsed.original_url))?;

    let url = format!("https://www.reddit.com/comments/{id}.json?raw_json=1&limit=1");
    let value = super::get_json(client, &url).await?;
    let listings: Vec<RedditListing> = serde_json::from_value(value)
        .map_err(|e| format!("Reddit yanıtı çözümlenemedi ({id}): {e}"))?;

    let post = listings
        .first()
        .and_then(|listing| listing.data.children.first())
        .map(|child| &child.data)
        .ok_or_else(|| format!("Reddit gönderisi bulunamadı: {id}"))?;

    // Crosspost ise asıl içerik ebeveyn gönderide bulunur.
    let content = post
        .crosspost_parent_list
        .as_deref()
        .and_then(|parents| parents.first())
        .unwrap_or(post);

    Ok(reddit_to_post(post, content))
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
        .map(|title| super::unescape_html(title.trim()));
    built.text = content
        .selftext
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty() && *text != "[removed]" && *text != "[deleted]")
        .map(super::unescape_html)
        .unwrap_or_default();
    built.created_at = content
        .created_utc
        .and_then(|secs| (secs > 0.0).then_some(secs as i64));
    built.sensitive = content.over_18.unwrap_or(false) || content.spoiler.unwrap_or(false);
    built.stats = Stats {
        replies: content.num_comments.unwrap_or(0),
        reposts: 0,
        likes: content
            .score
            .or_else(|| content.ups.map(|ups| ups.max(0) as u64))
            .unwrap_or(0),
        quotes: 0,
        views: None,
    };
    built.media = reddit_media(content);
    built.extra_label = reddit_extra_label(content);

    built
}

/// `r/rust • Tartışma` biçiminde ek bilgi satırı.
fn reddit_extra_label(post: &RedditPost) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(subreddit) = post.subreddit_name_prefixed.as_deref() {
        parts.push(subreddit.to_string());
    }
    if let Some(flair) = post.link_flair_text.as_deref() {
        if !flair.trim().is_empty() {
            parts.push(flair.trim().to_string());
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" • "))
    }
}


/// Gönderi medyasını çıkarır: galeri > video > doğrudan görsel > önizleme görseli.
fn reddit_media(post: &RedditPost) -> Vec<Media> {
    if post.is_gallery.unwrap_or(false) {
        if let Some(gallery) = gallery_media(post) {
            if !gallery.is_empty() {
                return gallery;
            }
        }
    }

    if let Some(video) = post.media.as_ref().and_then(|media| media.reddit_video.as_ref()) {
        if let Some(url) = video.fallback_url.as_deref() {
            return vec![Media {
                kind: MediaKind::Video,
                url: unescape_url(url),
                poster: preview_image(post),
                width: video.width,
                height: video.height,
                duration: video.duration,
            }];
        }
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

/// `url_overridden_by_dest` doğrudan bir görsel dosyasıysa onu döndürür.
fn direct_image_url(post: &RedditPost) -> Option<String> {
    let url = post.url_overridden_by_dest.as_deref()?;
    let path = url
        .split(|c| c == '?')
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
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

/// Galeri gönderilerinde `media_metadata` + `gallery_data` sıralamasını kullanır.
fn gallery_media(post: &RedditPost) -> Option<Vec<Media>> {
    let metadata = post.media_metadata.as_ref()?;
    let order: Vec<String> = post
        .gallery_data
        .as_ref()
        .and_then(|data| data.items.as_ref())
        .map(|items| items.iter().filter_map(|item| item.media_id.clone()).collect())
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
    if item.e.as_deref() == Some("AnimatedImage") {
        if let Some(mp4) = source.mp4.as_deref() {
            return Some(Media {
                kind: MediaKind::Gif,
                url: unescape_url(mp4),
                poster: source.u.as_deref().map(unescape_url),
                width: source.x,
                height: source.y,
                duration: None,
            });
        }
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
    author: Option<String>,
    subreddit_name_prefixed: Option<String>,
    link_flair_text: Option<String>,
    permalink: Option<String>,
    url_overridden_by_dest: Option<String>,
    score: Option<u64>,
    ups: Option<i64>,
    num_comments: Option<u64>,
    created_utc: Option<f64>,
    over_18: Option<bool>,
    spoiler: Option<bool>,
    is_gallery: Option<bool>,
    gallery_data: Option<GalleryData>,
    media_metadata: Option<HashMap<String, MediaMeta>>,
    media: Option<RedditMedia>,
    preview: Option<RedditPreview>,
    crosspost_parent_list: Option<Vec<RedditPost>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RedditMedia {
    reddit_video: Option<RedditVideo>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RedditVideo {
    fallback_url: Option<String>,
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
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct MediaMeta {
    #[serde(rename = "e")]
    e: Option<String>,
    s: Option<MediaMetaSource>,
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
        let listings: Vec<RedditListing> = serde_json::from_str(json).expect("örnek JSON geçerli olmalı");
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
        assert_eq!(post.url, "https://www.reddit.com/r/EarthPorn/comments/abc123/guzel_bir_manzara/");
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
        assert_eq!(post.media[0].poster.as_deref(), Some("https://preview.redd.it/vid1.jpg"));
        assert_eq!(post.media[0].duration, Some(42));
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
        assert_eq!(urls, vec!["https://i.redd.it/b.jpg", "https://i.redd.it/a.jpg"]);
        assert_eq!(post.media[0].width, Some(30));
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
        assert_eq!(post.url, "https://www.reddit.com/r/original/comments/parent1/asil/");
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
}
