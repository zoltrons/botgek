//! OpenGraph / twitter meta etiket çözümleyicisi.
//!
//! Native API'si olmayan veya ek veri (görünen ad, tarih, kanonik bağlantı) gerektiren
//! platformlar için HTML sayfasındaki `og:` / `twitter:` meta etiketleri okunur. Instagram
//! native motoru ([`super::instagram`]) hem SSR embed sayfasını hem de bu çözümleyiciyi
//! kullanır: embed'de olmayan görünen ad, tarih ve medya bağlantısı buradan tamamlanır.

use regex::Regex;
use std::sync::LazyLock;

use super::unescape_html;

// Tırnaklı değerler bir bütün olarak geçilir: değer içindeki `>` etiketi erken bitirmez.
// Tırnaklı parça `<` geçemez ve eşi olmayan tırnak tek karakter sayılır; bozuk bir etiket sonraki etiketleri yutmaz.
pub(super) static META_TAG_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)<meta\s(?:[^>"']|"[^"<]*"|'[^'<]*'|["'])*>"#).expect("geçerli meta regex")
});
// Öznitelikler soldan sağa sırayla tüketilir; böylece bir değerin içindeki `name=...` metni ya da `data-content=` gibi önekli adlar ayrı öznitelik sanılmaz.
// Çift tırnaklı değer kesme işareti (`Don't`), tek tırnaklı değer çift tırnak içerebilir.
static ATTR_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)([a-z0-9_:.-]+)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+))"#)
        .expect("geçerli attribute regex")
});
static TITLE_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<title[^>]*>(.*?)</title>").expect("geçerli title regex"));

/// Etiketin `property`/`name` anahtarını (küçük harfle) ve ham `content` değerini döndürür.
fn meta_key_and_content(tag: &str) -> (Option<String>, Option<&str>) {
    let mut key = None;
    let mut content = None;
    for captures in ATTR_REGEX.captures_iter(tag) {
        let value = captures
            .get(2)
            .or_else(|| captures.get(3))
            .or_else(|| captures.get(4))
            .map_or("", |m| m.as_str());
        match captures[1].to_ascii_lowercase().as_str() {
            "property" | "name" if key.is_none() && !value.is_empty() => {
                key = Some(value.to_ascii_lowercase());
            }
            "content" if content.is_none() => content = Some(value),
            _ => {}
        }
    }
    (key, content)
}

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
        let (Some(key), Some(content)) = meta_key_and_content(tag.as_str()) else {
            continue;
        };
        let content = unescape_html(content).trim().to_string();
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
        && let Some(captures) = TITLE_REGEX.captures(html)
    {
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

    #[test]
    fn quote_characters_inside_values_do_not_end_the_value() {
        let html = r#"<meta property="og:title" content="Don't stop">
            <meta property='og:description' content='Say "hi" > bye'>
            <meta name=og:site_name content=Örnek>"#;
        let graph = parse_open_graph(html);
        assert_eq!(graph.title.as_deref(), Some("Don't stop"));
        assert_eq!(graph.description.as_deref(), Some("Say \"hi\" > bye"));
        assert_eq!(graph.site_name.as_deref(), Some("Örnek"));
    }

    #[test]
    fn prefixed_or_embedded_attribute_names_are_not_matched() {
        let html = r#"<meta data-content="sahte" property="og:title" content="Gerçek">
            <meta data-property="og:image" content="https://evil.example/a.jpg">
            <meta content="x name='og:description'" property="og:site_name">"#;
        let graph = parse_open_graph(html);
        assert_eq!(graph.title.as_deref(), Some("Gerçek"));
        assert!(graph.image.is_none());
        assert!(graph.description.is_none());
        assert_eq!(graph.site_name.as_deref(), Some("x name='og:description'"));
    }

    #[test]
    fn a_stray_quote_does_not_swallow_following_tags() {
        let html = r#"<meta name=x content=Don't>
            <meta property="og:title" content="Sonraki">"#;
        assert_eq!(parse_open_graph(html).title.as_deref(), Some("Sonraki"));
    }
}
