//! OpenGraph tabanlı genel çözümleyici.
//!
//! Native API'si olmayan platformlar (Instagram) için HTML sayfasındaki
//! `og:` / `twitter:` meta etiketleri okunur. Instagram giriş duvarı döndürürse akış
//! hata verir ve ayar açıksa proxy motoruna düşülür.

use std::sync::LazyLock;
use regex::Regex;

use super::{unescape_html, Author, BuiltinPost, Media, MediaKind};
use crate::converters::parser::ParsedPost;
#[cfg(test)]
use crate::converters::parser::Platform;

static META_TAG_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<meta\s+[^>]*>").expect("geçerli meta regex"));
static KEY_ATTR_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)(?:property|name)\s*=\s*["']([^"']+)["']"#).expect("geçerli attribute regex")
});
static CONTENT_ATTR_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)content\s*=\s*["']([^"']*)["']"#).expect("geçerli content regex")
});
static TITLE_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<title[^>]*>(.*?)</title>").expect("geçerli title regex"));

/// Sayfadan çıkarılan OpenGraph verisi.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct OpenGraph {
    pub title: Option<String>,
    pub description: Option<String>,
    pub image: Option<String>,
    pub video: Option<String>,
    pub site_name: Option<String>,
}

/// Sayfayı indirip OpenGraph verisini gönderi modeline çevirir.
pub async fn fetch(client: &reqwest::Client, parsed: &ParsedPost) -> Result<BuiltinPost, String> {
    let html = super::get_text(client, &parsed.original_url).await?;
    let graph = parse_open_graph(&html);

    if !is_embeddable(&graph) {
        return Err(format!(
            "Sayfada gösterilecek OpenGraph içeriği yok (giriş duvarı olabilir): {}",
            parsed.original_url
        ));
    }

    Ok(graph_to_post(graph, parsed))
}

/// Embed üretmek için yeterli veri var mı?
///
/// Instagram giriş duvarı yalnızca genel site başlığı (`Instagram`) döndürür; bu durumda
/// içeriksiz bir kart göndermek yerine proxy motoruna düşülür. Gerçek gönderiler her zaman
/// bir görsel veya açıklama içerir.
fn is_embeddable(graph: &OpenGraph) -> bool {
    graph.image.is_some() || graph.description.is_some()
}

fn graph_to_post(graph: OpenGraph, parsed: &ParsedPost) -> BuiltinPost {
    let site_name = graph
        .site_name
        .clone()
        .unwrap_or_else(|| parsed.platform.display_name().to_string());
    let mut author = Author::new(site_name, String::new());
    author.profile_url = None;

    let mut post = BuiltinPost::new(parsed.platform, parsed.original_url.clone(), author);
    post.title = graph
        .title
        .clone()
        .filter(|title| !title.trim().is_empty())
        .map(|title| title.trim().to_string());
    post.text = graph
        .description
        .clone()
        .filter(|description| !description.trim().is_empty())
        .map(|description| description.trim().to_string())
        .unwrap_or_default();

    if let Some(video) = graph.video.clone() {
        post.media.push(Media {
            kind: MediaKind::Video,
            url: video,
            poster: graph.image.clone(),
            width: None,
            height: None,
            duration: None,
        });
    } else if let Some(image) = graph.image.clone() {
        post.media.push(Media {
            kind: MediaKind::Image,
            url: image,
            poster: None,
            width: None,
            height: None,
            duration: None,
        });
    }

    post
}


/// HTML içindeki `og:*` ve `twitter:*` meta etiketlerini ayrıştırır.
pub fn parse_open_graph(html: &str) -> OpenGraph {
    let mut graph = OpenGraph::default();

    for tag in META_TAG_REGEX.find_iter(html) {
        let tag = tag.as_str();
        let key = match KEY_ATTR_REGEX.captures(tag) {
            Some(captures) => captures
                .get(1)
                .map(|m| m.as_str().to_ascii_lowercase())
                .unwrap_or_default(),
            None => continue,
        };
        let content = match CONTENT_ATTR_REGEX.captures(tag) {
            Some(captures) => unescape_html(captures.get(1).map(|m| m.as_str()).unwrap_or_default()),
            None => continue,
        };
        let content = content.trim().to_string();
        if content.is_empty() {
            continue;
        }

        match key.as_str() {
            "og:title" => graph.title = Some(content),
            "twitter:title" => {
                if graph.title.is_none() {
                    graph.title = Some(content);
                }
            }
            "og:description" => graph.description = Some(content),
            "twitter:description" => {
                if graph.description.is_none() {
                    graph.description = Some(content);
                }
            }
            "og:image" | "og:image:secure_url" | "og:image:url" => graph.image = Some(content),
            "twitter:image" => {
                if graph.image.is_none() {
                    graph.image = Some(content);
                }
            }
            "og:video" | "og:video:secure_url" | "og:video:url" | "twitter:player:stream" => {
                graph.video = Some(content);
            }
            "og:site_name" => graph.site_name = Some(content),
            _ => {}
        }
    }

    // og:title yoksa <title> etiketi kullanılır.
    if graph.title.is_none()
        && let Some(captures) = TITLE_REGEX.captures(html) {
            let title = unescape_html(captures.get(1).map(|m| m.as_str()).unwrap_or_default())
                .trim()
                .to_string();
            if !title.is_empty() {
                graph.title = Some(title);
            }
        }

    graph
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_graph_tags_are_parsed() {
        let html = r#"<!DOCTYPE html><html><head>
            <title>Yedek başlık</title>
            <meta property="og:site_name" content="Instagram">
            <meta property="og:title" content="Kullanıcı on Instagram: &quot;Selam dünya&quot;">
            <meta property="og:description" content="1.234 beğeni, 56 yorum - kullanici on 1 Ocak">
            <meta property="og:image" content="https://scontent.cdninstagram.com/foto.jpg">
            <meta name="twitter:card" content="summary_large_image">
        </head><body></body></html>"#;

        let graph = parse_open_graph(html);
        assert_eq!(graph.site_name.as_deref(), Some("Instagram"));
        assert_eq!(
            graph.title.as_deref(),
            Some("Kullanıcı on Instagram: \"Selam dünya\"")
        );
        assert_eq!(
            graph.description.as_deref(),
            Some("1.234 beğeni, 56 yorum - kullanici on 1 Ocak")
        );
        assert_eq!(
            graph.image.as_deref(),
            Some("https://scontent.cdninstagram.com/foto.jpg")
        );
        assert!(graph.video.is_none());
    }

    #[test]
    fn login_wall_pages_are_rejected() {
        // Giriş duvarı yalnızca genel site başlığı döndürür -> embed üretilmez, proxy'ye düşülür
        let wall = OpenGraph {
            title: Some("Instagram".to_string()),
            site_name: Some("Instagram".to_string()),
            ..OpenGraph::default()
        };
        assert!(!is_embeddable(&wall));

        let real = OpenGraph {
            title: Some("Kullanıcı on Instagram".to_string()),
            image: Some("https://scontent.cdninstagram.com/foto.jpg".to_string()),
            ..OpenGraph::default()
        };
        assert!(is_embeddable(&real));

        let description_only = OpenGraph {
            description: Some("1.234 beğeni - kullanici on 1 Ocak".to_string()),
            ..OpenGraph::default()
        };
        assert!(is_embeddable(&description_only));
    }

    #[test]
    fn title_is_used_when_open_graph_title_is_missing() {
        let html = "<html><head><title>  Sadece başlık  </title></head></html>";
        let graph = parse_open_graph(html);
        assert_eq!(graph.title.as_deref(), Some("Sadece başlık"));
        assert!(graph.image.is_none());
    }

    #[test]
    fn og_video_wins_over_image_as_media() {
        let graph = OpenGraph {
            title: Some("Video gönderisi".to_string()),
            image: Some("https://cdn/kapak.jpg".to_string()),
            video: Some("https://cdn/video.mp4".to_string()),
            site_name: Some("Instagram".to_string()),
            ..OpenGraph::default()
        };

        let parsed = ParsedPost {
            platform: Platform::Instagram,
            is_spoiler: false,
            user_comment: None,
            original_url: "https://www.instagram.com/reel/abc/".to_string(),
            fixed_url: "https://ddinstagram.com/reel/abc/".to_string(),
        };

        let post = graph_to_post(graph, &parsed);
        assert_eq!(post.author.label(), "Instagram");
        assert_eq!(post.title.as_deref(), Some("Video gönderisi"));
        assert_eq!(post.media.len(), 1);
        assert_eq!(post.media[0].kind, MediaKind::Video);
        assert_eq!(post.media[0].url, "https://cdn/video.mp4");
        assert_eq!(post.media[0].poster.as_deref(), Some("https://cdn/kapak.jpg"));
    }
}
