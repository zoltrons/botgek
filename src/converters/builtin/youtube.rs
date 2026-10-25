//! YouTube topluluk gönderisi: gönderi sayfasının sunucu tarafında gömdüğü `ytInitialData`
//! içindeki `backstagePostRenderer` okunur (oturum gerektirmez). Kanal adı/kullanıcı adı,
//! bağlantıları korunmuş metin, beğeni sayısı, görsel/çoklu görsel, paylaşılan video ve anket
//! seçenekleri karta taşınır.
//!
//! Sayfa `hl=en` ile istenir: beğeni ve tarih metinleri İngilizce biçimde okunur. YouTube yalnızca
//! göreli tarih verir (`4 days ago`); kartın göreli zamanı bundan türetilir. Gönderi okunamazsa
//! (üyelere özel, kaldırılmış) sayfanın genel önizlemesine düşülür.

use serde_json::Value;
use tracing::debug;

use super::render::escape_markdown;
use super::{Author, BuiltinPost, Media, MediaKind, preview};
use crate::converters::parser::{ParsedPost, Platform};

const YOUTUBE: &str = "https://www.youtube.com";
/// Kartta en fazla gösterilecek görsel (Discord medya galerisi sınırı).
const MAX_IMAGES: usize = 10;
/// Kartta gösterilecek en geniş görsel sürümü (piksel).
const MAX_IMAGE_WIDTH: u64 = 2048;

/// Topluluk gönderisini çeker ve karta dönüştürür.
pub async fn fetch(client: &reqwest::Client, parsed: &ParsedPost) -> Result<BuiltinPost, String> {
    let Some(id) = post_id(&parsed.original_url) else {
        return preview::fetch(parsed, Platform::YouTube).await;
    };
    let page = format!("{YOUTUBE}/post/{id}?hl=en&gl=US");
    let result = super::get_text(client, &page)
        .await
        .and_then(|html| post_from_html(&id, &html, now()));
    match result {
        Ok(post) => Ok(post),
        Err(reason) => {
            debug!("YouTube topluluk gönderisi okunamadı, genel önizlemeye düşülüyor: {reason}");
            preview::fetch(parsed, Platform::YouTube).await
        }
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// `https://www.youtube.com/post/{id}` (ayrıştırıcının kanonik biçimi) → id.
fn post_id(url: &str) -> Option<String> {
    let id = url.strip_prefix(YOUTUBE)?.strip_prefix("/post/")?;
    let valid = (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    valid.then(|| id.to_string())
}

/// Sayfadaki `var ytInitialData = {...};` nesnesini okur. Nesnenin sonu JSON çözücüyle
/// bulunur: metin içinde geçen `;</script>` değeri erken kesmez.
fn initial_data(html: &str) -> Option<Value> {
    let start = html.find("var ytInitialData = ")? + "var ytInitialData = ".len();
    serde_json::Deserializer::from_str(&html[start..])
        .into_iter::<Value>()
        .next()?
        .ok()
}

/// Ağaçtaki ilk `key` değerini derinlik öncelikli arar.
fn find<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    match value {
        Value::Object(map) => map
            .get(key)
            .or_else(|| map.values().find_map(|child| find(child, key))),
        Value::Array(items) => items.iter().find_map(|child| find(child, key)),
        _ => None,
    }
}

fn str_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter()
        .try_fold(value, |node, key| node.get(key))?
        .as_str()
}

/// `{runs: [{text}]}` ya da `{simpleText}` biçimindeki düz metin.
fn plain(value: Option<&Value>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    if let Some(text) = value.get("simpleText").and_then(Value::as_str) {
        return text.to_string();
    }
    value
        .get("runs")
        .and_then(Value::as_array)
        .map(|runs| {
            runs.iter()
                .filter_map(|run| run.get("text").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default()
}

/// Yönlendirme/bahsetme işaretleri (`\u{202a}@kanal\u{202c}`) kartta görünmez karakter bırakır.
fn clean(text: &str) -> String {
    text.chars()
        .filter(|ch| !matches!(ch, '\u{202a}'..='\u{202e}' | '\u{200e}' | '\u{200f}'))
        .collect()
}

/// Metin parçasının hedef bağlantısı: dış bağlantılar YouTube'un `redirect?q=` sarmalayıcısından
/// çıkarılır, iç bağlantılar (kanal, hashtag, video) tam adrese çevrilir.
fn run_link(run: &Value) -> Option<String> {
    let endpoint = run.get("navigationEndpoint")?;
    if let Some(url) = str_at(endpoint, &["urlEndpoint", "url"]) {
        let target = url::Url::parse(url)
            .ok()
            .filter(|u| u.host_str() == Some("www.youtube.com") && u.path() == "/redirect")
            .and_then(|u| {
                u.query_pairs()
                    .find(|(key, _)| key == "q")
                    .map(|(_, v)| v.into_owned())
            })
            .unwrap_or_else(|| url.to_string());
        return (target.starts_with("https://") || target.starts_with("http://")).then_some(target);
    }
    let path = str_at(endpoint, &["commandMetadata", "webCommandMetadata", "url"])?;
    (path.starts_with('/') && !path.starts_with("//")).then(|| format!("{YOUTUBE}{path}"))
}

/// Gönderi metni: bağlantılı parçalar Markdown bağlantısı, kalanlar kaçırılmış düz metindir.
fn content_markdown(content: Option<&Value>) -> String {
    let Some(runs) = content
        .and_then(|c| c.get("runs"))
        .and_then(Value::as_array)
    else {
        return preview::escape_preview_markdown(&plain(content));
    };
    let mut out = String::new();
    for run in runs {
        let text = clean(run.get("text").and_then(Value::as_str).unwrap_or_default());
        // Adres Markdown'a kaçışsız girer; `link_url` parantez ve köşeli ayraçları kodlar.
        let link = run_link(run).and_then(|url| url::Url::parse(&url).ok());
        match link {
            // Adresin kendisi yazılıysa Discord onu zaten bağlantı yapar.
            Some(url) if text.trim().starts_with("http") => out.push_str(&preview::link_url(&url)),
            Some(url) if !text.trim().is_empty() => out.push_str(&format!(
                "[{}]({})",
                escape_markdown(text.trim()),
                preview::link_url(&url)
            )),
            _ => out.push_str(&escape_run(&text)),
        }
    }
    out.trim().to_string()
}

/// Metin parçasını kaçırır. `escape_preview_markdown` kenar boşluklarını kırptığı için
/// kullanılmaz: parçalar arasındaki boşluklar (`watch ` + `@kanal`) korunmalıdır.
fn escape_run(text: &str) -> String {
    let text: String = text
        .chars()
        .filter(|ch| !ch.is_control() || *ch == '\n')
        .collect();
    escape_markdown(
        &text
            .replace('@', "@\u{200b}")
            .replace('<', "‹")
            .replace('>', "›"),
    )
}

/// Küçük resim listesindeki (küçükten büyüğe sıralı) en büyük görsel; liste 4500 px'e kadar
/// çıktığı için 2048 px'i aşmayan sonuncu seçilir. Protokolsüz adresler `https:` ile tamamlanır.
fn best_thumbnail(value: Option<&Value>) -> Option<String> {
    let thumbs: Vec<(u64, &str)> = value?
        .get("thumbnails")?
        .as_array()?
        .iter()
        .filter_map(|thumb| {
            let url = thumb.get("url").and_then(Value::as_str)?;
            Some((thumb.get("width").and_then(Value::as_u64).unwrap_or(0), url))
        })
        .collect();
    let url = thumbs
        .iter()
        .rev()
        .find(|(width, _)| *width <= MAX_IMAGE_WIDTH)
        .or(thumbs.last())?
        .1;
    let url = if url.starts_with("//") {
        format!("https:{url}")
    } else {
        url.to_string()
    };
    let host = url::Url::parse(&url).ok()?.host_str()?.to_string();
    let trusted = url.starts_with("https://")
        && ["ggpht.com", "googleusercontent.com", "ytimg.com"]
            .iter()
            .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")));
    trusted.then_some(url)
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

/// `4 days ago`, `1 month ago (edited)` → yaklaşık unix zamanı.
fn relative_time(text: &str, now: i64) -> Option<i64> {
    let mut words = text.split_whitespace();
    let amount: i64 = words.next()?.parse().ok()?;
    let unit = words.next()?.trim_end_matches('s');
    let seconds = match unit {
        "second" => 1,
        "minute" => 60,
        "hour" => 3_600,
        "day" => 86_400,
        "week" => 7 * 86_400,
        "month" => 30 * 86_400,
        "year" => 365 * 86_400,
        _ => return None,
    };
    (words.next() == Some("ago")).then(|| now - amount * seconds)
}

/// Beğeni düğmesinin erişilebilirlik metni tam sayıyı taşır
/// (`Like this post along with 4,347 other people`); yoksa kısaltılmış `4.3K` okunur.
fn like_count(post: &Value) -> u64 {
    let label = str_at(
        post,
        &[
            "actionButtons",
            "commentActionButtonsRenderer",
            "likeButton",
            "toggleButtonRenderer",
            "accessibilityData",
            "accessibilityData",
            "label",
        ],
    )
    .unwrap_or_default();
    let digits: String = label.chars().filter(char::is_ascii_digit).collect();
    digits
        .parse()
        .ok()
        .or_else(|| preview::parse_compact_count(&plain(post.get("voteCount"))))
        .unwrap_or(0)
}

/// Ekler: görsel(ler) galeriye girer; paylaşılan video ve anket metne eklenir.
fn attachment(post: &Value, media: &mut Vec<Media>, blocks: &mut Vec<String>) {
    let Some(attachment) = post.get("backstageAttachment") else {
        return;
    };
    if let Some(images) = attachment
        .get("postMultiImageRenderer")
        .and_then(|r| r.get("images"))
        .and_then(Value::as_array)
    {
        media.extend(
            images
                .iter()
                .filter_map(|img| best_thumbnail(find(img, "image")))
                .take(MAX_IMAGES)
                .map(image),
        );
    } else if let Some(single) = attachment.get("backstageImageRenderer") {
        media.extend(best_thumbnail(single.get("image")).map(image));
    } else if let Some(video) = attachment.get("videoRenderer") {
        let id = video
            .get("videoId")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if !id.is_empty()
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            let title = clean(&plain(video.get("title")));
            let title = if title.trim().is_empty() {
                "Video".to_string()
            } else {
                escape_markdown(title.trim())
            };
            let mut line = format!("▶️ [{title}]({YOUTUBE}/watch?v={id})");
            let length = plain(video.get("lengthText"));
            if !length.is_empty() {
                line.push_str(&format!(" `{}`", length.replace('`', "")));
            }
            blocks.push(line);
            media.extend(best_thumbnail(video.get("thumbnail")).map(image));
        }
    } else if let Some(poll) = attachment.get("pollRenderer") {
        let choices: Vec<String> = poll
            .get("choices")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|choice| clean(&plain(choice.get("text"))))
            .filter(|text| !text.trim().is_empty())
            .map(|text| format!("▫️ {}", escape_markdown(text.trim())))
            .collect();
        if !choices.is_empty() {
            let mut block = choices.join("\n");
            let votes = plain(poll.get("totalVotes"));
            if !votes.is_empty() {
                block.push_str(&format!("\n-# 📊 {}", escape_markdown(&votes)));
            }
            blocks.push(block);
        }
    }
}

fn post_from_html(id: &str, html: &str, now: i64) -> Result<BuiltinPost, String> {
    let data = initial_data(html).ok_or("ytInitialData bulunamadı")?;
    let post = find(&data, "backstagePostRenderer").ok_or("topluluk gönderisi bulunamadı")?;

    let name = clean(&plain(post.get("authorText")));
    let channel_path = str_at(
        post,
        &["authorEndpoint", "browseEndpoint", "canonicalBaseUrl"],
    )
    .filter(|path| path.starts_with('/') && !path.starts_with("//"));
    let handle = channel_path
        .and_then(|path| path.strip_prefix("/@"))
        .unwrap_or_default();
    let mut author = Author::new(name.trim(), urlencoding::decode(handle).unwrap_or_default());
    author.avatar = best_thumbnail(post.get("authorThumbnail"));
    author.profile_url = channel_path.map(|path| format!("{YOUTUBE}{path}"));

    let mut built = BuiltinPost::new(Platform::YouTube, format!("{YOUTUBE}/post/{id}"), author);
    let mut blocks = Vec::new();
    let text = content_markdown(post.get("contentText"));
    if !text.is_empty() {
        blocks.push(text);
    }
    attachment(post, &mut built.media, &mut blocks);
    if blocks.is_empty() && built.media.is_empty() {
        return Err("topluluk gönderisi boş".into());
    }
    built.text = blocks.join("\n\n");
    built.created_at = relative_time(&plain(post.get("publishedTimeText")), now);
    built.stats.likes = like_count(post);
    built.extra_label = Some("Community".into());
    Ok(built)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_793_000_000;

    /// Canlı sayfadan kısaltılmış `backstagePostRenderer` (çoklu görsel + bahsetme + bağlantı).
    const PAGE: &str = r#"<script>var ytInitialData = {"contents":{"x":[{"backstagePostThreadRenderer":{"post":{"backstagePostRenderer":{
        "postId":"UgkxYyo1fnjWfHtrybG1kENO14IS9HH8eJvP",
        "authorText":{"runs":[{"text":"YouTube"}]},
        "authorThumbnail":{"thumbnails":[{"url":"//yt3.googleusercontent.com/a=s32"},{"url":"//yt3.googleusercontent.com/a=s76"}]},
        "authorEndpoint":{"browseEndpoint":{"browseId":"UCBR8-60-B28hp2BmDPdntcQ","canonicalBaseUrl":"/@YouTube"}},
        "contentText":{"runs":[
            {"text":"creators on *Jeopardy!* \n\nwatch "},
            {"text":"[LRE]@MichelleKhare[PDF]","navigationEndpoint":{"commandMetadata":{"webCommandMetadata":{"url":"/channel/UCGGZ_POGmIWG1pQXTDzQv-g"}}}},
            {"text":" at "},
            {"text":"https://jeopardy.com/yt","navigationEndpoint":{"urlEndpoint":{"url":"https://www.youtube.com/redirect?event=backstage&q=https%3A%2F%2Fjeopardy.com%2Fyt"}}},
            {"text":" ;</script> stays text"}]},
        "backstageAttachment":{"postMultiImageRenderer":{"images":[
            {"backstageImageRenderer":{"image":{"thumbnails":[{"url":"https://yt3.ggpht.com/one=s288"},{"url":"https://yt3.ggpht.com/one=s2048"}]}}},
            {"backstageImageRenderer":{"image":{"thumbnails":[{"url":"https://yt3.ggpht.com/two=s2048"}]}}},
            {"backstageImageRenderer":{"image":{"thumbnails":[{"url":"https://evil.example/three.jpg"}]}}}]}},
        "publishedTimeText":{"runs":[{"text":"9 days ago"}]},
        "voteCount":{"simpleText":"2.3K"},
        "actionButtons":{"commentActionButtonsRenderer":{"likeButton":{"toggleButtonRenderer":{
            "accessibilityData":{"accessibilityData":{"label":"Like this post along with 2,389 other people"}}}}}}
    }}}}]}};</script>"#;

    #[test]
    fn post_id_comes_from_the_canonical_url() {
        assert_eq!(
            post_id("https://www.youtube.com/post/Ugkx_a-1").as_deref(),
            Some("Ugkx_a-1")
        );
        assert_eq!(post_id("https://www.youtube.com/post/a%2Fb"), None);
        assert_eq!(post_id("https://www.youtube.com/watch?v=abc"), None);
    }

    #[test]
    fn community_post_becomes_a_full_card() {
        // Bahsetmeyi saran yön işaretleri kaynakta görünmez karakter olarak bırakılmaz.
        let page = PAGE
            .replace("[LRE]", "\u{202a}")
            .replace("[PDF]", "\u{202c}");
        let post = post_from_html("UgkxYyo1fnjWfHtrybG1kENO14IS9HH8eJvP", &page, NOW).unwrap();
        assert_eq!(
            post.url,
            "https://www.youtube.com/post/UgkxYyo1fnjWfHtrybG1kENO14IS9HH8eJvP"
        );
        assert_eq!(post.author.name, "YouTube");
        assert_eq!(post.author.handle, "YouTube");
        assert_eq!(
            post.author.profile_url.as_deref(),
            Some("https://www.youtube.com/@YouTube")
        );
        assert_eq!(
            post.author.avatar.as_deref(),
            Some("https://yt3.googleusercontent.com/a=s76")
        );
        assert_eq!(
            post.text,
            "creators on \\*Jeopardy!\\* \n\nwatch \
             [@MichelleKhare](https://www.youtube.com/channel/UCGGZ_POGmIWG1pQXTDzQv-g) at \
             https://jeopardy.com/yt ;‹/script› stays text"
        );
        assert_eq!(post.stats.likes, 2389);
        assert_eq!(post.created_at, Some(NOW - 9 * 86_400));
        let urls: Vec<_> = post.media.iter().map(|m| m.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://yt3.ggpht.com/one=s2048",
                "https://yt3.ggpht.com/two=s2048"
            ],
            "güvenilmeyen ana bilgisayar atlanır"
        );
    }

    #[test]
    fn video_and_poll_attachments_are_rendered_as_text() {
        let video = serde_json::json!({"backstageAttachment":{"videoRenderer":{
            "videoId":"dQw4w9WgXcQ","title":{"runs":[{"text":"Never [Gonna]"}]},
            "lengthText":{"simpleText":"3:33"},
            "thumbnail":{"thumbnails":[{"url":"https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg"}]}}}});
        let (mut media, mut blocks) = (Vec::new(), Vec::new());
        attachment(&video, &mut media, &mut blocks);
        assert_eq!(
            blocks,
            ["▶️ [Never \\[Gonna\\]](https://www.youtube.com/watch?v=dQw4w9WgXcQ) `3:33`"]
        );
        assert_eq!(media.len(), 1);

        let poll = serde_json::json!({"backstageAttachment":{"pollRenderer":{
            "choices":[{"text":{"runs":[{"text":"Yes"}]}},{"text":{"runs":[{"text":"No"}]}}],
            "totalVotes":{"simpleText":"12K votes"}}}});
        let (mut media, mut blocks) = (Vec::new(), Vec::new());
        attachment(&poll, &mut media, &mut blocks);
        assert_eq!(blocks, ["▫️ Yes\n▫️ No\n-# 📊 12K votes"]);
        assert!(media.is_empty());
    }

    #[test]
    fn relative_time_and_like_fallbacks() {
        assert_eq!(relative_time("1 hour ago", NOW), Some(NOW - 3_600));
        assert_eq!(
            relative_time("2 weeks ago (edited)", NOW),
            Some(NOW - 14 * 86_400)
        );
        assert_eq!(relative_time("4 gün önce", NOW), None);
        let post = serde_json::json!({"voteCount":{"simpleText":"4.3K"}});
        assert_eq!(like_count(&post), 4_300);
    }

    #[test]
    fn pages_without_a_post_are_errors() {
        assert!(post_from_html("x", "<html></html>", NOW).is_err());
        assert!(post_from_html("x", "var ytInitialData = {\"a\":1};", NOW).is_err());
    }

    #[tokio::test]
    #[ignore = "requires live YouTube access; run explicitly"]
    async fn live_community_post() {
        let url = "https://www.youtube.com/post/UgkxYyo1fnjWfHtrybG1kENO14IS9HH8eJvP".to_string();
        let parsed = ParsedPost {
            platform: Platform::YouTube,
            is_spoiler: false,
            user_comment: None,
            original_url: url,
        };
        let post = fetch(&reqwest::Client::new(), &parsed)
            .await
            .expect("gönderi okunmalı");
        println!("{post:#?}");
        assert_eq!(post.author.handle, "YouTube");
        assert!(post.stats.likes > 0);
        assert!(!post.media.is_empty());
    }
}
