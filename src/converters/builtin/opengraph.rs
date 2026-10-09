//! OpenGraph / twitter meta etiket çözümleyicisi.
//!
//! Native API'si olmayan veya ek veri (görünen ad, tarih, kanonik bağlantı) gerektiren
//! platformlar için HTML sayfasındaki `og:` / `twitter:` meta etiketleri okunur. Instagram
//! native motoru ([`super::instagram`]) hem SSR embed sayfasını hem de bu çözümleyiciyi
//! kullanır: embed'de olmayan görünen ad, tarih ve medya bağlantısı buradan tamamlanır.

use std::sync::LazyLock;
use regex::Regex;

use super::unescape_html;

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
    /// `og:url` — gönderinin kanonik bağlantısı (Instagram'da kullanıcı adını taşır).
    pub url: Option<String>,
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
            "og:url" => graph.url = Some(content),
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
            <meta property="og:url" content="https://www.instagram.com/kullanici/p/abc/">
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
        assert_eq!(
            graph.url.as_deref(),
            Some("https://www.instagram.com/kullanici/p/abc/")
        );
        assert!(graph.video.is_none());
    }

    #[test]
    fn title_is_used_when_open_graph_title_is_missing() {
        let html = "<html><head><title>  Sadece başlık  </title></head></html>";
        let graph = parse_open_graph(html);
        assert_eq!(graph.title.as_deref(), Some("Sadece başlık"));
        assert!(graph.image.is_none());
    }
}
