//! Pixiv eseri: sitenin kendi web istemcisinin kullandığı `/ajax/illust/{id}` uç noktasından
//! okunur (oturum gerektirmez). Başlık, açıklama, etiketler, sanatçı, tarih, beğeni/yer imi/
//! izlenme/yorum sayıları ve çok sayfalı eserlerin tüm sayfaları (`/pages`) alınır.
//!
//! `i.pximg.net` yalnızca Pixiv'in `Referer`'ı ile görsel verir; Discord'un medya vekili bu
//! başlığı göndermediği için görseller aynı yolu birebir sunan `i.pixiv.re` aynasından
//! gösterilir. Uç nokta belgelenmemiştir: okunamazsa sayfanın genel önizlemesine düşülür.

use serde::Deserialize;
use tracing::debug;

use super::render::{escape_markdown, format_count};
use super::{Author, BuiltinPost, Media, MediaKind, preview, unescape_html};
use crate::converters::parser::{ParsedPost, Platform};

const PIXIV: &str = "https://www.pixiv.net";
const PXIMG: &str = "https://i.pximg.net/";
/// `Referer` istemeyen görsel aynası (pixiv.cat); yol `i.pximg.net` ile aynıdır.
const IMAGE_MIRROR: &str = "https://i.pixiv.re/";
/// Kartta en fazla gösterilecek sayfa (Discord medya galerisi sınırı).
const MAX_PAGES: usize = 10;
/// Açıklamanın altında gösterilecek en fazla etiket.
const MAX_TAGS: usize = 6;

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    error: bool,
    body: Option<T>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Illust {
    illust_id: String,
    illust_title: String,
    #[serde(default)]
    illust_comment: String,
    /// 0 illüstrasyon, 1 manga, 2 ugoira (hareketli).
    #[serde(default)]
    illust_type: u8,
    user_id: String,
    user_name: String,
    #[serde(default)]
    user_account: String,
    create_date: Option<String>,
    #[serde(default)]
    like_count: u64,
    #[serde(default)]
    bookmark_count: u64,
    #[serde(default)]
    view_count: u64,
    #[serde(default)]
    comment_count: u64,
    #[serde(default = "one")]
    page_count: u32,
    /// 0 herkese açık, 1 R-18, 2 R-18G.
    #[serde(default)]
    x_restrict: u8,
    /// 2: yapay zekâ ile üretilmiş.
    #[serde(default)]
    ai_type: u8,
    urls: Urls,
    width: Option<u32>,
    height: Option<u32>,
    tags: Option<Tags>,
}

fn one() -> u32 {
    1
}

#[derive(Debug, Clone, Default, Deserialize)]
struct Urls {
    regular: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Tags {
    #[serde(default)]
    tags: Vec<Tag>,
}

#[derive(Debug, Clone, Deserialize)]
struct Tag {
    tag: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Page {
    urls: Urls,
    width: Option<u32>,
    height: Option<u32>,
}

/// Pixiv eserini çeker ve karta dönüştürür.
pub async fn fetch(client: &reqwest::Client, parsed: &ParsedPost) -> Result<BuiltinPost, String> {
    let Some(id) = artwork_id(&parsed.original_url) else {
        return preview::fetch(parsed, Platform::Pixiv).await;
    };
    let illust_url = format!("{PIXIV}/ajax/illust/{id}");
    let pages_url = format!("{PIXIV}/ajax/illust/{id}/pages");
    let (illust, pages) = tokio::join!(
        super::get_json_as::<Envelope<Illust>>(client, &illust_url),
        super::get_json_as::<Envelope<Vec<Page>>>(client, &pages_url),
    );
    let illust = match illust.and_then(|e| unwrap(e, &id)) {
        Ok(illust) => illust,
        Err(reason) => {
            debug!("Pixiv eseri okunamadı, genel önizlemeye düşülüyor: {reason}");
            return preview::fetch(parsed, Platform::Pixiv).await;
        }
    };
    // Sayfa listesi yalnızca ek sayfalar içindir; okunamazsa ilk sayfa yeterlidir.
    let pages = pages.and_then(|e| unwrap(e, &id)).unwrap_or_default();
    illust_post(&illust, &pages)
}

fn unwrap<T>(envelope: Envelope<T>, id: &str) -> Result<T, String> {
    match envelope {
        Envelope {
            error: false,
            body: Some(body),
        } => Ok(body),
        _ => Err(format!("Pixiv eseri bulunamadı: {id}")),
    }
}

/// `https://www.pixiv.net/[xx/]artworks/{id}` (ayrıştırıcının kanonik biçimi) → id.
fn artwork_id(url: &str) -> Option<String> {
    let path = url.strip_prefix(PIXIV)?;
    let id = path.rsplit_once("/artworks/")?.1;
    let valid = (1..=12).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit());
    valid.then(|| id.to_string())
}

/// `i.pximg.net` adresini aynaya taşır; başka bir ana bilgisayar kabul edilmez.
fn mirror(url: &str) -> Option<String> {
    let path = url.strip_prefix(PXIMG)?;
    let safe = !path.is_empty()
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/_-.".contains(&b));
    safe.then(|| format!("{IMAGE_MIRROR}{path}"))
}

/// Açıklama HTML'idir: satır sonları korunur, `jump.php` yönlendirmeleri hedef adrese açılır,
/// kalan etiketler atılır.
fn comment_text(html: &str) -> String {
    static LINK: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r#"(?is)<a\s[^>]*href="([^"]*)"[^>]*>(.*?)</a>"#).expect("geçerli regex")
    });
    static TAG: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"(?s)<[^>]*>").expect("geçerli regex"));
    let text = html
        .replace("<br />", "\n")
        .replace("<br/>", "\n")
        .replace("<br>", "\n");
    let text = LINK.replace_all(&text, |caps: &regex::Captures| {
        let href = unescape_html(&caps[1]);
        let target = href
            .strip_prefix("/jump.php?")
            .and_then(|raw| urlencoding::decode(raw).ok())
            .map(|url| url.into_owned())
            .unwrap_or(href);
        if target.starts_with("https://") || target.starts_with("http://") {
            target
        } else {
            TAG.replace_all(&caps[2], "").into_owned()
        }
    });
    unescape_html(&TAG.replace_all(&text, ""))
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn tag_line(illust: &Illust) -> Option<String> {
    let tags: Vec<String> = illust
        .tags
        .iter()
        .flat_map(|tags| &tags.tags)
        .map(|tag| tag.tag.trim())
        .filter(|tag| !tag.is_empty())
        .take(MAX_TAGS)
        .map(|tag| {
            format!(
                "[#{}]({PIXIV}/tags/{}/artworks)",
                escape_markdown(tag),
                super::encode_component(tag)
            )
        })
        .collect();
    (!tags.is_empty()).then(|| format!("-# {}", tags.join(" ")))
}

fn image(url: &Urls, width: Option<u32>, height: Option<u32>) -> Option<Media> {
    Some(Media {
        kind: MediaKind::Image,
        url: mirror(url.regular.as_deref()?)?,
        poster: None,
        width,
        height,
        duration: None,
    })
}

fn illust_post(illust: &Illust, pages: &[Page]) -> Result<BuiltinPost, String> {
    let title = illust.illust_title.trim();
    if title.is_empty() && illust.user_name.trim().is_empty() {
        return Err(format!("Pixiv eseri boş: {}", illust.illust_id));
    }
    let mut author = Author::new(illust.user_name.trim(), illust.user_account.trim());
    if illust.user_id.bytes().all(|b| b.is_ascii_digit()) && !illust.user_id.is_empty() {
        author.profile_url = Some(format!("{PIXIV}/users/{}", illust.user_id));
    }

    let url = format!("{PIXIV}/artworks/{}", illust.illust_id);
    let mut post = BuiltinPost::new(Platform::Pixiv, url, author);
    post.title = (!title.is_empty()).then(|| title.to_string());

    let mut blocks = Vec::new();
    if !title.is_empty() {
        blocks.push(format!("**{}**", escape_markdown(title)));
    }
    let comment = comment_text(&illust.illust_comment);
    if !comment.is_empty() {
        blocks.push(preview::escape_preview_markdown(&comment));
    }
    if let Some(tags) = tag_line(illust) {
        blocks.push(tags);
    }
    post.text = blocks.join("\n");

    post.created_at = illust
        .create_date
        .as_deref()
        .and_then(super::parse_timestamp);
    post.stats.likes = illust.like_count;
    post.stats.replies = illust.comment_count;
    post.stats.views = (illust.view_count > 0).then_some(illust.view_count);
    post.sensitive = illust.x_restrict > 0;

    let mut label = vec![
        match illust.illust_type {
            1 => "Manga",
            2 => "Ugoira",
            _ => "İllüstrasyon",
        }
        .to_string(),
    ];
    if illust.page_count > 1 {
        label.push(format!("{} sayfa", illust.page_count));
    }
    if illust.bookmark_count > 0 {
        label.push(format!("🔖 {}", format_count(illust.bookmark_count)));
    }
    if illust.ai_type == 2 {
        label.push("Yapay zekâ".into());
    }
    match illust.x_restrict {
        1 => label.push("R-18".into()),
        2 => label.push("R-18G".into()),
        _ => {}
    }
    post.extra_label = Some(label.join(" · "));

    post.media = pages
        .iter()
        .filter_map(|page| image(&page.urls, page.width, page.height))
        .take(MAX_PAGES)
        .collect();
    if post.media.is_empty()
        && let Some(first) = image(&illust.urls, illust.width, illust.height)
    {
        post.media.push(first);
    }
    Ok(post)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Canlı `/ajax/illust` yanıtından kısaltılmış örnek.
    const ILLUST_JSON: &str = r#"{"error":false,"message":"","body":{
        "illustId":"151194171","illustTitle":"女の子になっちゃったダウナーくんの日々",
        "illustComment":"番外編15.5話です！<br />【Amazon】 <a href=\"/jump.php?https%3A%2F%2Flink.amazon%2FB00B5FUqK\" target=\"_blank\">https://link.amazon/B00B5FUqK</a> &amp; more",
        "illustType":1,"createDate":"2026-10-23T15:01:00+00:00",
        "userId":"2119505","userName":"浅月のりと","userAccount":"norito_a",
        "likeCount":1925,"bookmarkCount":2132,"viewCount":29108,"commentCount":15,
        "pageCount":2,"xRestrict":0,"aiType":1,"width":1299,"height":1831,
        "urls":{"regular":"https://i.pximg.net/img-master/img/2026/10/24/00/01/14/151194171_p0_master1200.jpg"},
        "tags":{"tags":[{"tag":"漫画"},{"tag":"オリジナル"}]}}}"#;
    const PAGES_JSON: &str = r#"{"error":false,"message":"","body":[
        {"urls":{"regular":"https://i.pximg.net/img-master/img/2026/10/24/00/01/14/151194171_p0_master1200.jpg"},"width":1299,"height":1831},
        {"urls":{"regular":"https://i.pximg.net/img-master/img/2026/10/24/00/01/14/151194171_p1_master1200.jpg"},"width":1299,"height":1831}]}"#;

    fn sample() -> (Illust, Vec<Page>) {
        let illust: Envelope<Illust> = serde_json::from_str(ILLUST_JSON).unwrap();
        let pages: Envelope<Vec<Page>> = serde_json::from_str(PAGES_JSON).unwrap();
        (illust.body.unwrap(), pages.body.unwrap())
    }

    #[test]
    fn artwork_id_comes_from_the_canonical_url() {
        assert_eq!(
            artwork_id("https://www.pixiv.net/artworks/151194171").as_deref(),
            Some("151194171")
        );
        assert_eq!(
            artwork_id("https://www.pixiv.net/en/artworks/42").as_deref(),
            Some("42")
        );
        assert_eq!(artwork_id("https://www.pixiv.net/artworks/12a"), None);
        assert_eq!(artwork_id("https://evil.example/artworks/1"), None);
    }

    #[test]
    fn images_use_the_referer_free_mirror_only_for_pximg() {
        assert_eq!(
            mirror("https://i.pximg.net/img-master/img/2026/1_p0_master1200.jpg").as_deref(),
            Some("https://i.pixiv.re/img-master/img/2026/1_p0_master1200.jpg")
        );
        assert_eq!(mirror("https://evil.example/a.jpg"), None);
        assert_eq!(mirror("https://i.pximg.net/a.jpg?x=1"), None);
    }

    #[test]
    fn comment_html_becomes_plain_text_with_real_links() {
        let (illust, _) = sample();
        assert_eq!(
            comment_text(&illust.illust_comment),
            "番外編15.5話です！\n【Amazon】 https://link.amazon/B00B5FUqK & more"
        );
    }

    #[test]
    fn artwork_becomes_a_full_card() {
        let (illust, pages) = sample();
        let post = illust_post(&illust, &pages).unwrap();
        assert_eq!(post.url, "https://www.pixiv.net/artworks/151194171");
        assert_eq!(post.author.name, "浅月のりと");
        assert_eq!(post.author.handle, "norito_a");
        assert_eq!(
            post.author.profile_url.as_deref(),
            Some("https://www.pixiv.net/users/2119505")
        );
        assert!(
            post.text
                .starts_with("**女の子になっちゃったダウナーくんの日々**\n番外編"),
            "{}",
            post.text
        );
        assert!(
            post.text.contains(
                "-# [#漫画](https://www.pixiv.net/tags/%E6%BC%AB%E7%94%BB/artworks) [#オリジナル]"
            ),
            "{}",
            post.text
        );
        assert_eq!(post.created_at, Some(1_792_767_660));
        assert_eq!(post.stats.likes, 1925);
        assert_eq!(post.stats.replies, 15);
        assert_eq!(post.stats.views, Some(29108));
        assert_eq!(
            post.extra_label.as_deref(),
            Some("Manga · 2 sayfa · 🔖 2.1K")
        );
        assert!(!post.sensitive);
        assert_eq!(post.media.len(), 2);
        assert!(post.media[1].url.starts_with("https://i.pixiv.re/"));
        assert!(post.media[1].url.ends_with("_p1_master1200.jpg"));
    }

    #[test]
    fn restricted_artwork_is_sensitive_and_single_page_falls_back_to_regular() {
        let (mut illust, _) = sample();
        illust.x_restrict = 1;
        illust.ai_type = 2;
        let post = illust_post(&illust, &[]).unwrap();
        assert!(post.sensitive);
        assert!(
            post.extra_label
                .as_deref()
                .unwrap()
                .ends_with("Yapay zekâ · R-18")
        );
        assert_eq!(post.media.len(), 1);
    }

    #[tokio::test]
    #[ignore = "requires live Pixiv access; run explicitly"]
    async fn live_pixiv_artwork() {
        let client = reqwest::Client::new();
        let url = "https://www.pixiv.net/artworks/151194171".to_string();
        let parsed = ParsedPost {
            platform: Platform::Pixiv,
            is_spoiler: false,
            user_comment: None,
            original_url: url,
        };
        let post = fetch(&client, &parsed).await.expect("eser okunmalı");
        println!("{post:#?}");
        assert_eq!(post.author.handle, "norito_a");
        assert!(post.media.len() > 1);
    }
}
