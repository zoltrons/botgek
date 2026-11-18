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
/// Sayfa sunucuda çizilir ve ilk bayta kadar geçen süre oynaktır (ölçülen 0.4–1.5 sn). İlk istek
/// bu süre içinde bitmezse aynı sayfa bir kez daha istenir; önce başarıyla biten kullanılır.
const HEDGE_AFTER: std::time::Duration = std::time::Duration::from_millis(700);

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
    // Dil öneksiz adres IP'ye göre `/{dil}/...` adresine 307 ile yönlenir (+1 gidiş-dönüş,
    // ~100 ms); klip verisi dilden bağımsızdır.
    let page = format!("https://www.xbox.com/en-US/play/media/{id}");
    let html = hedged(HEDGE_AFTER, || super::get_text(client, &page)).await?;
    clip_post(&html, id, &parsed.original_url)
}

/// `make` ile bir istek başlatır; `after` içinde bitmezse ikincisini başlatır ve önce başarıyla
/// biteni döner. İkisi de başarısızsa son biten hatayı döner. `after` dolmadan gelen hata
/// beklenmeden döner (yavaşlık değil, gerçek hata).
async fn hedged<T, E, Fut>(after: std::time::Duration, make: impl Fn() -> Fut) -> Result<T, E>
where
    Fut: std::future::Future<Output = Result<T, E>>,
{
    let first = make();
    tokio::pin!(first);
    tokio::select! {
        result = &mut first => return result,
        () = tokio::time::sleep(after) => {}
    }
    let second = make();
    tokio::pin!(second);
    tokio::select! {
        result = &mut first => match result {
            Ok(value) => Ok(value),
            Err(_) => second.await,
        },
        result = &mut second => match result {
            Ok(value) => Ok(value),
            Err(_) => first.await,
        },
    }
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
    post.extra_label = clip
        .title
        .map(|game| game.name)
        .filter(|name| !name.is_empty());
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// Yavaş ilk istek ikinciyle yarıştırılır; hızlı istekte ikinci istek hiç atılmaz.
    #[tokio::test]
    async fn slow_request_is_hedged_and_fast_one_is_not() {
        let calls = AtomicUsize::new(0);
        let slow_then_fast = || {
            let n = calls.fetch_add(1, Ordering::SeqCst);
            async move {
                tokio::time::sleep(Duration::from_millis(if n == 0 { 2_000 } else { 10 })).await;
                Ok::<_, ()>(n)
            }
        };
        let started = std::time::Instant::now();
        assert_eq!(
            hedged(Duration::from_millis(30), slow_then_fast).await,
            Ok(1)
        );
        assert!(started.elapsed() < Duration::from_millis(1_000));

        let calls = AtomicUsize::new(0);
        let fast = || {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Ok::<_, ()>(7) }
        };
        assert_eq!(hedged(Duration::from_millis(30), fast).await, Ok(7));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// İkinci istek hata verirse ilk isteğin sonucu beklenir.
    #[tokio::test]
    async fn hedge_falls_back_to_the_first_request_when_the_second_fails() {
        let calls = AtomicUsize::new(0);
        let make = || {
            let n = calls.fetch_add(1, Ordering::SeqCst);
            async move {
                if n == 0 {
                    tokio::time::sleep(Duration::from_millis(80)).await;
                    Ok(0)
                } else {
                    Err("ikinci")
                }
            }
        };
        assert_eq!(hedged(Duration::from_millis(20), make).await, Ok(0));
    }

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
