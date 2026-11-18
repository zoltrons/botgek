//! Xbox oyun klibi (`xbox.com/play/media/{id}` paylaşım bağlantısı).
//!
//! Sayfa sunucuda çizilir ve `window.__PRELOADED_STATE__` içinde klibin bilgisini taşır:
//! `xcloud.platform.xboxSocialClientReduxState` bir JSON **dizesidir**, onun içindeki
//! `feed.linkPreviews[id].data.item` klibin kendisidir. `originalContentUri` imzalı (~45 dk
//! geçerli) MP4 adresidir; video indirilmez, bu adres kartta doğrudan oynar. Discord'un önizleme
//! botu sayfada `og:video` bulamadığı için bağlantı tek başına oynamaz.

use serde::Deserialize;
use serde_json::Value;

use super::{Author, BuiltinPost, Media, MediaKind};
use crate::converters::parser::{ParsedPost, Platform};

const STATE_MARKER: &str = "window.__PRELOADED_STATE__ = ";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Clip {
    #[serde(rename = "type")]
    kind: String,
    original_content_uri: Option<String>,
    preview_thumbnail_uri: Option<String>,
    view_count: Option<u64>,
    like_count: Option<u64>,
    comment_count: Option<u64>,
    posted_date_in_ms: Option<i64>,
    title: Option<Game>,
    author_info: Option<AuthorInfo>,
}

#[derive(Debug, Deserialize)]
struct Game {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthorInfo {
    gamertag: String,
    gamerpic_url: Option<String>,
}

pub async fn fetch(client: &reqwest::Client, parsed: &ParsedPost) -> Result<BuiltinPost, String> {
    let id = parsed
        .original_url
        .strip_prefix("https://www.xbox.com/play/media/")
        .filter(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric()))
        .ok_or_else(|| format!("Xbox klip kimliği okunamadı: {}", parsed.original_url))?;
    let html = super::get_text(client, &parsed.original_url).await?;
    clip_post(&html, id, &parsed.original_url)
}

/// Sayfadaki durum nesnesinden klibi bulur.
fn find_clip(html: &str, id: &str) -> Result<Clip, String> {
    let start = html
        .find(STATE_MARKER)
        .ok_or("Xbox sayfasında durum nesnesi yok")?
        + STATE_MARKER.len();
    // Nesneden sonra gelen betik metni okunmaz: akış ilk değeri bitince durur.
    let state: Value = serde_json::Deserializer::from_str(&html[start..])
        .into_iter()
        .next()
        .ok_or("Xbox durum nesnesi boş")?
        .map_err(|e| format!("Xbox durum nesnesi çözülemedi: {e}"))?;
    let social = state
        .pointer("/xcloud/platform/xboxSocialClientReduxState")
        .and_then(Value::as_str)
        .ok_or("Xbox sosyal durumu yok")?;
    let social: Value =
        serde_json::from_str(social).map_err(|e| format!("Xbox sosyal durumu çözülemedi: {e}"))?;
    let item = social
        .get("feed")
        .and_then(|feed| feed.get("linkPreviews"))
        .and_then(|previews| previews.get(id))
        .and_then(|preview| preview.pointer("/data/item"))
        .ok_or_else(|| format!("Xbox klibi bulunamadı: {id}"))?;
    serde_json::from_value(item.clone()).map_err(|e| format!("Xbox klibi çözülemedi: {e}"))
}

fn https(url: Option<String>) -> Option<String> {
    url.map(|url| match url.strip_prefix("http://") {
        Some(rest) => format!("https://{rest}"),
        None => url,
    })
    .filter(|url| url.starts_with("https://"))
}

fn clip_post(html: &str, id: &str, url: &str) -> Result<BuiltinPost, String> {
    let clip = find_clip(html, id)?;
    if clip.kind != "GameClip" {
        return Err(format!("Xbox bağlantısı klip değil: {}", clip.kind));
    }
    let video = https(clip.original_content_uri).ok_or("Xbox klibinin video adresi yok")?;
    let info = clip.author_info.ok_or("Xbox klibinin sahibi yok")?;
    let mut author = Author::new(info.gamertag.clone(), info.gamertag);
    author.avatar = https(info.gamerpic_url);

    let mut post = BuiltinPost::new(Platform::Xbox, url, author);
    let game = clip
        .title
        .map(|game| game.name)
        .filter(|name| !name.is_empty());
    post.text = game.clone().unwrap_or_default();
    post.extra_label = game;
    post.created_at = clip.posted_date_in_ms.map(|ms| ms / 1000);
    post.stats.views = clip.view_count;
    post.stats.likes = clip.like_count.unwrap_or(0);
    post.stats.replies = clip.comment_count.unwrap_or(0);
    post.media.push(Media {
        alt: None,
        kind: MediaKind::Video,
        url: video,
        poster: https(clip.preview_thumbnail_uri),
        width: None,
        height: None,
        duration: None,
    });
    Ok(post)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Canlı sayfadan kısaltılmış örnek: iç durum JSON dizesi olarak gömülüdür.
    fn page(item: &str) -> String {
        let social = format!(
            r#"{{"feed":{{"linkPreviews":{{"esVuDv4Lyv":{{"type":2,"data":{{"item":{item}}}}}}}}}}}"#
        );
        let state = serde_json::json!({ "xcloud": { "platform": { "xboxSocialClientReduxState": social } } });
        format!("<script>{STATE_MARKER}{state};\n window.env = {{}};</script>")
    }

    const ITEM: &str = r#"{"type":"GameClip",
        "originalContentUri":"https://gameclipscontent-d3022.media.xboxlive.com/x/93261001.MP4?sv=1&sig=a%2Bb",
        "previewThumbnailUri":"http://images-eds.xboxlive.com/image?url=abc",
        "viewCount":1,"likeCount":2,"commentCount":0,"postedDateInMs":1795033866659,
        "title":{"name":"Grand Theft Auto VI"},
        "authorInfo":{"gamertag":"zoltronux","gamerpicUrl":"https://images-eds-ssl.xboxlive.com/image?url=pic"}}"#;

    #[test]
    fn clip_becomes_a_post_with_the_direct_video() {
        let url = "https://www.xbox.com/play/media/esVuDv4Lyv";
        let post = clip_post(&page(ITEM), "esVuDv4Lyv", url).unwrap();
        assert_eq!(post.platform, Platform::Xbox);
        assert_eq!(post.url, url);
        assert_eq!(post.author.name, "zoltronux");
        assert_eq!(post.extra_label.as_deref(), Some("Grand Theft Auto VI"));
        assert_eq!(post.created_at, Some(1_795_033_866));
        assert_eq!(post.stats.views, Some(1));
        assert_eq!(post.media.len(), 1);
        assert!(post.media[0].is_video());
        assert!(post.media[0].url.ends_with(".MP4?sv=1&sig=a%2Bb"));
        assert_eq!(
            post.media[0].poster.as_deref(),
            Some("https://images-eds.xboxlive.com/image?url=abc")
        );
    }

    #[test]
    fn missing_or_foreign_clip_is_an_error() {
        let url = "https://www.xbox.com/play/media/esVuDv4Lyv";
        assert!(clip_post(&page(ITEM), "other", url).is_err());
        assert!(clip_post("<html></html>", "esVuDv4Lyv", url).is_err());
        let screenshot = ITEM.replace("GameClip", "Screenshot");
        assert!(clip_post(&page(&screenshot), "esVuDv4Lyv", url).is_err());
    }

    #[tokio::test]
    #[ignore = "requires live Xbox access; run explicitly"]
    async fn live_clip_has_direct_video() {
        let parsed = crate::converters::parser::parse_message(
            "https://www.xbox.com/tr-TR/play/media/esVuDv4Lyv",
        )
        .unwrap();
        let post = fetch(&reqwest::Client::new(), &parsed).await.expect("klip");
        assert!(
            post.media[0].url.contains("xboxlive.com"),
            "{}",
            post.media[0].url
        );
    }
}
