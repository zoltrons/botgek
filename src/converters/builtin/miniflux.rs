//! Miniflux herkese açık paylaşım sayfaları (`https://{sunucu}/share/{kod}`).
//!
//! Sayfada OpenGraph etiketi yoktur (Discord önizlemesi boş kalır); makale `entry-content`
//! bölümünden okunur. Kart kısa bir özet gösterir, "Devamını oku" butonu mesajı yerinde
//! tam metinle günceller (bkz. [`toggle`]).

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Value, json};
use url::Url;

use super::{Author, BuiltinPost, CustomCard, render::escape_markdown, unescape_html};
use crate::converters::parser::{ParsedPost, Platform};
use crate::utils::truncate_chars;

const TYPE_ACTION_ROW: u64 = 1;
const TYPE_SECTION: u64 = 9;
const TYPE_TEXT_DISPLAY: u64 = 10;
const TYPE_THUMBNAIL: u64 = 11;
const TYPE_SEPARATOR: u64 = 14;

/// Yerinde güncellenen bileşenlerin `id`'leri (bkz. `spotify::TRACK_LIST_COMPONENT_ID`).
const BODY_COMPONENT_ID: u64 = 7401;
const ROW_COMPONENT_ID: u64 = 7402;
/// `miniflux:{more|less}:{kod}:{sunucu}`; Discord `custom_id` sınırı 100 karakterdir.
pub const CUSTOM_ID_PREFIX: &str = "miniflux:";
const CUSTOM_ID_LIMIT: usize = 100;

/// Daraltılmış kartta gösterilen özet uzunluğu.
const PREVIEW_CHARS: usize = 500;
/// Genişletilmiş metin; başlık ve kullanıcı notuyla birlikte 4000 karakterlik V2 sınırında kalır.
const FULL_CHARS: usize = 3200;
/// Sayfadaki saat dilimsiz `datetime` değeri Türkiye saatindedir.
/// shortcut: UTC+3 sabit; Miniflux hesabı başka saat dilimindeyse zaman kayar.
const PAGE_UTC_OFFSET_SECS: i64 = 3 * 3600;

static TITLE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<h1 id="page-header-title"[^>]*>\s*<a href="([^"]*)"[^>]*>(.*?)</a>"#)
        .expect("geçerli Miniflux başlık regex")
});
static FEED_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<span class="entry-website">.*?<a href="[^"]*">(.*?)</a>"#)
        .expect("geçerli Miniflux kaynak regex")
});
static AUTHOR_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<span class="entry-author">\s*[–-]?\s*<em>(.*?)</em>"#)
        .expect("geçerli Miniflux yazar regex")
});
static CONTENT_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<article class="entry-content[^"]*"[^>]*>(.*)</article>"#)
        .expect("geçerli Miniflux içerik regex")
});
static DATE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<time datetime="(\d{4})-(\d{2})-(\d{2}) (\d{2}):(\d{2}):(\d{2})""#)
        .expect("geçerli Miniflux tarih regex")
});
static IMAGE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)<img\s[^>]*?src="(https://[^"]+)""#).expect("geçerli Miniflux görsel regex")
});
static DROP_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?is)<(?:script|style|figure|svg)\b.*?</(?:script|style|figure|svg)>|<(?:img|iframe)\b[^>]*>")
        .expect("geçerli Miniflux atılacak regex")
});
static BLOCK_END_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)</(?:p|div|h[1-6]|ul|ol|blockquote|pre|tr)>")
        .expect("geçerli Miniflux blok regex")
});
static BREAK_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<br\s*/?>|</li>").expect("geçerli Miniflux satır regex"));
static LIST_ITEM_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<li\b[^>]*>").expect("geçerli Miniflux madde regex"));
static TAG_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<[^>]*>").expect("geçerli Miniflux etiket regex"));
static BLANK_LINES_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\n{3,}").expect("geçerli Miniflux boş satır regex"));

struct Article {
    title: String,
    url: String,
    feed: String,
    author: Option<String>,
    image: Option<String>,
    created_at: Option<i64>,
    text: String,
}

/// `https://{sunucu}/share/{kod}` → (sunucu, kod).
fn split_share_url(raw: &str) -> Option<(String, String)> {
    let url = Url::parse(raw).ok()?;
    let code = url.path().strip_prefix("/share/")?;
    let valid =
        url.scheme() == "https" && code.len() == 40 && code.bytes().all(|b| b.is_ascii_hexdigit());
    let host = match url.port() {
        Some(port) => format!("{}:{port}", url.host_str()?),
        None => url.host_str()?.to_string(),
    };
    // Parser ile aynı kural: `miniflux.{alan}.{uzantı}`, kök alan adı değil.
    let hostname = url.host_str()?;
    let valid = valid && hostname.starts_with("miniflux.") && hostname.split('.').count() >= 3;
    valid.then(|| (host, code.to_string()))
}

pub async fn fetch(parsed: &ParsedPost) -> Result<BuiltinPost, String> {
    let (host, code) = split_share_url(&parsed.original_url).ok_or("geçersiz Miniflux adresi")?;
    let article = fetch_article(&parsed.original_url).await?;

    let mut post = BuiltinPost::new(
        Platform::Miniflux,
        parsed.original_url.clone(),
        Author::new(article.feed.clone(), ""),
    );
    post.title = Some(article.title.clone());
    post.created_at = article.created_at;
    post.text = truncate_chars(&article.text, PREVIEW_CHARS);
    post.extra_label = Some("Miniflux".to_string());
    post.custom_card = Some(build_card(&article, &host, &code));
    Ok(post)
}

async fn fetch_article(share_url: &str) -> Result<Article, String> {
    let (_, html) = super::safe_http::preview(share_url).await?;
    parse_article(&html).ok_or_else(|| format!("Miniflux makalesi okunamadı: {share_url}"))
}

/// `<time datetime="2026-10-30 11:37:32">` → Unix saniye.
fn page_timestamp(html: &str) -> Option<i64> {
    let caps = DATE_REGEX.captures(html)?;
    let n = |i: usize| caps[i].parse::<i64>().ok();
    let (y, m, d) = (n(1)?, n(2)?, n(3)?);
    // Takvim günü → 1970'ten gün sayısı (Howard Hinnant algoritması).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + n(4)? * 3600 + n(5)? * 60 + n(6)? - PAGE_UTC_OFFSET_SECS)
}

fn parse_article(html: &str) -> Option<Article> {
    let title = TITLE_REGEX.captures(html)?;
    let url = Url::parse(&unescape_html(&title[1]))
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https"))?;
    let content = CONTENT_REGEX.captures(html)?.get(1)?.as_str();
    let text = html_to_text(content);
    let one_line = |raw: &str| {
        unescape_html(&TAG_REGEX.replace_all(raw, ""))
            .trim()
            .to_string()
    };
    Some(Article {
        title: one_line(&title[2]),
        url: super::preview::link_url(&url),
        feed: FEED_REGEX
            .captures(html)
            .map(|caps| one_line(&caps[1]))
            .filter(|feed| !feed.is_empty())
            .unwrap_or_else(|| "Miniflux".to_string()),
        author: AUTHOR_REGEX
            .captures(html)
            .map(|caps| one_line(&caps[1]))
            .filter(|author| !author.is_empty()),
        created_at: page_timestamp(html),
        image: IMAGE_REGEX
            .captures(content)
            .map(|caps| unescape_html(&caps[1])),
        text,
    })
}

/// Makale HTML'ini Discord için güvenli düz metne çevirir: paragraflar korunur, işaretleme
/// ve mention söz dizimi etkisiz hale getirilir.
fn html_to_text(raw: &str) -> String {
    let kept = DROP_REGEX.replace_all(raw, "");
    let blocks = BLOCK_END_REGEX.replace_all(&kept, "\n\n");
    let items = LIST_ITEM_REGEX.replace_all(&blocks, "• ");
    let breaks = BREAK_REGEX.replace_all(&items, "\n");
    let stripped = TAG_REGEX.replace_all(&breaks, "");
    let decoded = unescape_html(&stripped).replace('<', "‹");
    let lines: Vec<String> = decoded
        .lines()
        .map(|line| {
            let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
            let line = escape_markdown(&line);
            // Satır başındaki `#`, `-` ve `1.` kartta başlık/liste oluşturmasın.
            if line.starts_with(['#', '-'])
                || line
                    .split_once(". ")
                    .is_some_and(|(n, _)| n.parse::<u8>().is_ok())
            {
                format!("\\{line}")
            } else {
                line
            }
        })
        .collect();
    BLANK_LINES_REGEX
        .replace_all(&lines.join("\n"), "\n\n")
        .trim()
        .to_string()
}

fn text_display(id: Option<u64>, content: String) -> Value {
    let mut value = json!({ "type": TYPE_TEXT_DISPLAY, "content": content });
    if let Some(id) = id {
        value["id"] = json!(id);
    }
    value
}

/// Gövde bileşeni ve (varsa) buton satırı. `expanded` tam metni gösterir.
fn body_components(
    article: &Article,
    host: &str,
    code: &str,
    expanded: bool,
) -> (Value, Option<Value>) {
    let limit = if expanded { FULL_CHARS } else { PREVIEW_CHARS };
    let body = truncate_chars(&article.text, limit);
    let can_expand = article.text.chars().count() > PREVIEW_CHARS;

    let mut buttons = Vec::new();
    let (action, label) = if expanded {
        ("less", "Daralt")
    } else {
        ("more", "Devamını oku")
    };
    let custom_id = format!("{CUSTOM_ID_PREFIX}{action}:{code}:{host}");
    if can_expand && custom_id.len() <= CUSTOM_ID_LIMIT {
        buttons.push(json!({ "type": 2, "style": 2, "label": label, "custom_id": custom_id }));
    }
    buttons.push(json!({ "type": 2, "style": 5, "label": "Makaleyi aç", "url": format!("https://{host}/share/{code}") }));

    (
        text_display(Some(BODY_COMPONENT_ID), body),
        Some(json!({ "type": TYPE_ACTION_ROW, "id": ROW_COMPONENT_ID, "components": buttons })),
    )
}

fn build_card(article: &Article, host: &str, code: &str) -> CustomCard {
    let mut header = format!(
        "### [{}]({})\n-# {}",
        escape_markdown(&article.title),
        article.url,
        escape_markdown(&article.feed)
    );
    if let Some(author) = &article.author {
        header.push_str(&format!(" · {}", escape_markdown(author)));
    }
    if let Some(ts) = article.created_at {
        header.push_str(&format!(" · <t:{ts}:R>"));
    }
    let head = match &article.image {
        Some(image) => json!({
            "type": TYPE_SECTION,
            "components": [text_display(None, header)],
            "accessory": { "type": TYPE_THUMBNAIL, "media": { "url": image }, "description": article.title },
        }),
        None => text_display(None, header),
    };

    let (body, row) = body_components(article, host, code, false);
    let mut components = vec![head, json!({ "type": TYPE_SEPARATOR, "spacing": 1 }), body];
    components.extend(row);
    CustomCard {
        components,
        accent_color: None,
        video_components: None,
    }
}

/// "Devamını oku" / "Daralt" tıklaması: `components` mesajın mevcut bileşenleridir; yalnızca
/// gövde metni ve buton satırı değiştirilir. Spoiler'lı kartta metin spoiler olarak kalır.
pub async fn toggle(custom_id: &str, components: &mut Value) -> Result<(), String> {
    let mut parts = custom_id
        .strip_prefix(CUSTOM_ID_PREFIX)
        .ok_or("geçersiz kimlik")?
        .splitn(3, ':');
    let (action, code, host) = (parts.next(), parts.next(), parts.next());
    let (Some(action @ ("more" | "less")), Some(code), Some(host)) = (action, code, host) else {
        return Err("geçersiz Miniflux kimliği".into());
    };
    let share_url = format!("https://{host}/share/{code}");
    // Kimlik doğrulaması: sunucu adı ve kod, adres ayrıştırıcısından aynen geri dönmeli.
    if split_share_url(&share_url) != Some((host.to_string(), code.to_string())) {
        return Err("geçersiz Miniflux adresi".into());
    }
    let article = fetch_article(&share_url).await?;
    let (body, row) = body_components(&article, host, code, action == "more");
    if replace_by_id(components, &body, row.as_ref()) {
        Ok(())
    } else {
        Err("makale gövdesi mesajda bulunamadı".into())
    }
}

fn replace_by_id(tree: &mut Value, body: &Value, row: Option<&Value>) -> bool {
    match tree {
        Value::Array(items) => items
            .iter_mut()
            .fold(false, |found, item| replace_by_id(item, body, row) | found),
        Value::Object(object) => match object.get("id").and_then(Value::as_u64) {
            Some(BODY_COMPONENT_ID) => {
                let spoiler = object
                    .get("content")
                    .and_then(Value::as_str)
                    .is_some_and(|content| content.starts_with("||"));
                let mut body = body.clone();
                if spoiler && let Some(content) = body["content"].as_str() {
                    body["content"] = json!(format!("||{}||", content.replace("||", "|\u{200b}|")));
                }
                *tree = body;
                true
            }
            Some(ROW_COMPONENT_ID) => {
                if let Some(row) = row {
                    *tree = row.clone();
                }
                false
            }
            _ => object.values_mut().fold(false, |found, child| {
                replace_by_id(child, body, row) | found
            }),
        },
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODE: &str = "5bf90f5776eaa9139d02f9a9a5d964d20a472f59";

    fn page(content: &str) -> String {
        format!(
            r#"<section class="entry"><header class="entry-header">
<h1 id="page-header-title" dir="auto" ><a href="https://t24.com.tr/x,1" rel="noopener">Komşuda &#39;alarm&#39; * var</a></h1>
<div class="entry-meta"><span class="entry-website"><img src="/feed-icon/a" alt="x"><a href="https://t24.com.tr/son-dakika">Son Dakika</a></span>
<span class="entry-author">
                – <em>RSS-Bridge</em>
            </span></div></header></section>
<main id="main"><article class="entry-content gesture-nav-" dir="auto" >{content}</article></main>"#
        )
    }

    #[test]
    fn article_is_parsed_from_share_page() {
        let article = parse_article(&page(
            r#"<img src="https://cdn.example/a.webp" alt="x"><p><strong>T24</strong></p><p>Bir &amp; iki</p><p># başlık &lt;@1&gt;</p><ul><li>bir</li><li>iki</li></ul>"#,
        ))
        .unwrap();
        assert_eq!(article.title, "Komşuda 'alarm' * var");
        assert_eq!(article.feed, "Son Dakika");
        assert_eq!(article.author.as_deref(), Some("RSS-Bridge"));
        assert_eq!(article.image.as_deref(), Some("https://cdn.example/a.webp"));
        assert_eq!(article.url, "https://t24.com.tr/x,1");
        assert_eq!(
            article.text,
            "T24\n\nBir & iki\n\n\\# başlık ‹@1\\>\n\n• bir\n• iki"
        );
        assert!(parse_article("<html>boş</html>").is_none());
    }

    #[test]
    fn card_collapses_long_text_and_toggle_swaps_in_place() {
        let long = "kelime ".repeat(300);
        let article = parse_article(&page(&format!("<p>{long}</p>"))).unwrap();
        let host = "miniflux.turta.us";
        let card = build_card(&article, host, CODE);
        let mut tree = json!(card.components);

        let row = &card.components[3];
        assert_eq!(row["components"][0]["label"], "Devamını oku");
        let id = row["components"][0]["custom_id"].as_str().unwrap();
        assert_eq!(id, format!("miniflux:more:{CODE}:{host}"));
        assert!(id.len() <= CUSTOM_ID_LIMIT);
        assert!(
            card.components[2]["content"]
                .as_str()
                .unwrap()
                .chars()
                .count()
                <= PREVIEW_CHARS
        );

        let (body, row) = body_components(&article, host, CODE, true);
        assert!(replace_by_id(&mut tree, &body, row.as_ref()));
        assert_eq!(tree[3]["components"][0]["label"], "Daralt");
        assert!(tree[2]["content"].as_str().unwrap().chars().count() > PREVIEW_CHARS);
    }

    #[test]
    fn short_text_has_no_expand_button_and_spoiler_is_kept() {
        let article = parse_article(&page("<p>kısa</p>")).unwrap();
        let (body, row) = body_components(&article, "m.example", CODE, false);
        assert_eq!(row.unwrap()["components"].as_array().unwrap().len(), 1);

        let mut tree = json!([{ "type": 10, "id": BODY_COMPONENT_ID, "content": "||eski||" }]);
        assert!(replace_by_id(&mut tree, &body, None));
        assert_eq!(tree[0]["content"], "||kısa||");
    }

    #[test]
    fn share_url_validation() {
        let ok = format!("https://miniflux.example.com:8443/share/{CODE}");
        assert_eq!(
            split_share_url(&ok),
            Some(("miniflux.example.com:8443".into(), CODE.into()))
        );
        assert_eq!(
            split_share_url(&format!("http://miniflux.example.com/share/{CODE}")),
            None
        );
        assert_eq!(
            split_share_url("https://miniflux.example.com/share/zz"),
            None
        );
        assert_eq!(
            split_share_url(&format!("https://miniflux.site/share/{CODE}")),
            None
        );
    }
}
