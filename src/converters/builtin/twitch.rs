//! Twitch klibi: Twitch'in web oynatıcısının kullandığı GraphQL uç noktasından okunur.
//!
//! Resmi Helix API klip videosunun adresini vermez; kart içinde oynayan video için sitenin
//! kendi istemci kimliğiyle `gql.twitch.tv`'ye tek sorgu atılır. Sorgu klibin bilgisini ve
//! imzalı MP4 adresini (`sig` + `token`, ~24 saat geçerli) birlikte döner. Video gönderimde
//! dosya olarak eklenir ki imza süresi dolunca eski mesajlardaki oynatıcı bozulmasın; sığmazsa
//! imzalı adres kartta uzaktan oynar. Uç nokta belgelenmemiştir: sorgu başarısız olursa
//! sayfanın genel önizlemesine düşülür.

use serde::Deserialize;
use serde_json::{Value, json};
use tracing::debug;

use super::render::{escape_markdown, format_count};
use super::{
    Author, BuiltinPost, CustomCard, Media, MediaKind, REQUEST_TIMEOUT, USER_AGENT, preview,
};
use crate::converters::parser::{ParsedPost, Platform};

const GQL_URL: &str = "https://gql.twitch.tv/gql";
/// Twitch web sitesinin herkese açık istemci kimliği (oturum gerektirmez).
const WEB_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";
const TWITCH: &str = "https://www.twitch.tv";
/// Twitch moru (`0x9146FF`).
const TWITCH_PURPLE: u32 = 0x9146FF;
/// Kartta oynayacak en yüksek çözünürlük: 720p ~2 Mbit/s ile kısa klipler Discord'un 10 MB
/// dosya sınırına sığar; 1080p aynı klipte ~4 kat büyüktür.
const MAX_QUALITY: u32 = 720;

// Components V2 bileşen türleri (yalnızca burada kullanılanlar).
const TYPE_ACTION_ROW: u64 = 1;
const TYPE_BUTTON: u64 = 2;
const TYPE_SECTION: u64 = 9;
const TYPE_TEXT_DISPLAY: u64 = 10;
const TYPE_THUMBNAIL: u64 = 11;
const TYPE_SEPARATOR: u64 = 14;
const STYLE_LINK: u64 = 5;

/// Klip bilgisi ve oynatma izni tek sorguda istenir. Kalıcı (persisted) sorgu yerine satır
/// içi sorgu kullanılır: Twitch kalıcı sorguların özetlerini zaman zaman değiştirir.
const CLIP_QUERY: &str = r#"query($slug: ID!) {
  clip(slug: $slug) {
    slug title viewCount createdAt durationSeconds
    thumbnailURL(width: 480, height: 272)
    broadcaster { login displayName profileImageURL(width: 150) }
    curator { displayName }
    game { displayName }
    video { id }
    videoOffsetSeconds
    videoQualities { quality sourceURL }
    playbackAccessToken(params: {platform: "web", playerBackend: "mediaplayer", playerType: "site"}) {
      signature value
    }
  }
}"#;

#[derive(Debug, Deserialize)]
struct GqlResponse {
    data: Option<GqlData>,
}

#[derive(Debug, Deserialize)]
struct GqlData {
    clip: Option<Clip>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Clip {
    slug: String,
    title: String,
    view_count: Option<u64>,
    created_at: Option<String>,
    duration_seconds: Option<u64>,
    #[serde(rename = "thumbnailURL")]
    thumbnail_url: Option<String>,
    broadcaster: Option<Broadcaster>,
    curator: Option<Named>,
    game: Option<Named>,
    video: Option<Vod>,
    video_offset_seconds: Option<u64>,
    #[serde(default)]
    video_qualities: Vec<Quality>,
    playback_access_token: Option<AccessToken>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Broadcaster {
    login: String,
    display_name: String,
    #[serde(rename = "profileImageURL")]
    profile_image_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Named {
    display_name: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Vod {
    id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Quality {
    quality: String,
    #[serde(rename = "sourceURL")]
    source_url: String,
}

#[derive(Debug, Clone, Deserialize)]
struct AccessToken {
    signature: String,
    value: String,
}

/// Twitch klibini çeker ve karta dönüştürür.
pub async fn fetch(client: &reqwest::Client, parsed: &ParsedPost) -> Result<BuiltinPost, String> {
    let Some(slug) = clip_slug(&parsed.original_url) else {
        return preview::fetch(parsed, Platform::Twitch).await;
    };
    match fetch_clip(client, &slug).await {
        Ok(clip) => clip_post(&clip),
        Err(reason) => {
            debug!("Twitch GQL klibi okunamadı, genel önizlemeye düşülüyor: {reason}");
            preview::fetch(parsed, Platform::Twitch).await
        }
    }
}

/// `https://clips.twitch.tv/{slug}` (ayrıştırıcının kanonik biçimi) → slug.
fn clip_slug(url: &str) -> Option<String> {
    let slug = url
        .strip_prefix("https://clips.twitch.tv/")?
        .split(['/', '?', '#'])
        .next()?;
    let valid = (1..=100).contains(&slug.len())
        && slug
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    valid.then(|| slug.to_string())
}

async fn fetch_clip(client: &reqwest::Client, slug: &str) -> Result<Clip, String> {
    let response = client
        .post(GQL_URL)
        .header("client-id", WEB_CLIENT_ID)
        .header("user-agent", USER_AGENT)
        .json(&json!({ "query": CLIP_QUERY, "variables": { "slug": slug } }))
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("Twitch GQL isteği başarısız: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("Twitch GQL {} döndü", response.status()));
    }
    let body = super::read_limited_text(response, GQL_URL).await?;
    let parsed: GqlResponse =
        serde_json::from_str(&body).map_err(|e| format!("Twitch GQL yanıtı çözülemedi: {e}"))?;
    parsed
        .data
        .and_then(|data| data.clip)
        .ok_or_else(|| format!("Twitch klibi bulunamadı: {slug}"))
}

/// Kart içinde oynayacak imzalı MP4: 720p'yi aşmayan en yüksek kalite. İmza olmadan CDN
/// 401 döner, bu yüzden erişim tokenı yoksa video yoktur.
fn video_url(clip: &Clip) -> Option<String> {
    let token = clip.playback_access_token.as_ref()?;
    let source = clip
        .video_qualities
        .iter()
        .filter_map(|q| Some((q.quality.parse::<u32>().ok()?, q.source_url.as_str())))
        .filter(|(height, url)| *height <= MAX_QUALITY && url.starts_with("https://"))
        .max_by_key(|(height, _)| *height)
        .map(|(_, url)| url)?;
    Some(format!(
        "{source}?sig={}&token={}",
        super::encode_component(&token.signature),
        super::encode_component(&token.value)
    ))
}

fn clip_url(clip: &Clip) -> String {
    format!("https://clips.twitch.tv/{}", clip.slug)
}

/// Klibin alındığı yayının (VOD) klip anına giden bağlantısı.
fn vod_url(clip: &Clip) -> Option<String> {
    let id = clip.video.as_ref().map(|vod| vod.id.as_str())?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let offset = clip.video_offset_seconds.unwrap_or(0);
    Some(format!(
        "{TWITCH}/videos/{id}?t={}h{}m{}s",
        offset / 3600,
        offset / 60 % 60,
        offset % 60
    ))
}

fn channel_url(login: &str) -> Option<String> {
    let valid = !login.is_empty()
        && login
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_');
    valid.then(|| format!("{TWITCH}/{login}"))
}

fn duration_label(seconds: u64) -> String {
    match (seconds / 60, seconds % 60) {
        (0, s) => format!("{s} sn"),
        (m, 0) => format!("{m} dk"),
        (m, s) => format!("{m} dk {s} sn"),
    }
}

fn https(url: Option<&String>) -> Option<&str> {
    url.map(String::as_str)
        .filter(|url| url.starts_with("https://"))
}

fn text(content: impl Into<String>) -> Value {
    json!({ "type": TYPE_TEXT_DISPLAY, "content": content.into() })
}

fn link_button(label: &str, url: &str) -> Value {
    json!({ "type": TYPE_BUTTON, "style": STYLE_LINK, "label": label, "url": url })
}

fn clip_post(clip: &Clip) -> Result<BuiltinPost, String> {
    let title = clip.title.trim();
    if title.is_empty() {
        return Err(format!("Twitch klip başlığı okunamadı: {}", clip.slug));
    }
    let broadcaster = clip.broadcaster.as_ref();
    let mut author = Author::new(
        broadcaster.map_or("Twitch", |b| b.display_name.as_str()),
        broadcaster.map_or("", |b| b.login.as_str()),
    );
    author.avatar = broadcaster
        .and_then(|b| https(b.profile_image_url.as_ref()))
        .map(str::to_string);
    author.profile_url = broadcaster.and_then(|b| channel_url(&b.login));

    let mut post = BuiltinPost::new(Platform::Twitch, clip_url(clip), author);
    post.title = Some(title.to_string());
    post.text = title.to_string();
    post.created_at = clip.created_at.as_deref().and_then(super::parse_timestamp);
    post.stats.views = clip.view_count;
    post.extra_label = clip.game.as_ref().map(|game| game.display_name.clone());
    if let Some(url) = video_url(clip) {
        post.media.push(Media {
            kind: MediaKind::Video,
            url,
            poster: https(clip.thumbnail_url.as_ref()).map(str::to_string),
            width: None,
            height: None,
            duration: clip.duration_seconds,
        });
    }
    post.custom_card = Some(build_card(clip, &post));
    Ok(post)
}

/// Kart: başlık + yayıncı/oyun, ayrıntı satırı ve bağlantı butonları. Video (eklenebildiyse
/// dosya, değilse imzalı adres) gönderimde başlığın hemen altına konur (bkz. `v2`).
fn build_card(clip: &Clip, post: &BuiltinPost) -> CustomCard {
    let url = clip_url(clip);
    let mut header = format!("### [{}]({url})", escape_markdown(clip.title.trim()));
    let mut byline = Vec::new();
    match (&clip.broadcaster, &post.author.profile_url) {
        (Some(b), Some(channel)) => {
            byline.push(format!("[{}]({channel})", escape_markdown(&b.display_name)))
        }
        (Some(b), None) => byline.push(escape_markdown(&b.display_name)),
        _ => {}
    }
    if let Some(game) = &clip.game {
        byline.push(escape_markdown(&game.display_name));
    }
    if !byline.is_empty() {
        header.push_str(&format!("\n-# {}", byline.join(" · ")));
    }

    let mut details = Vec::new();
    if let Some(views) = clip.view_count {
        details.push(format!("{} izlenme", format_count(views)));
    }
    if let Some(seconds) = clip.duration_seconds.filter(|s| *s > 0) {
        details.push(duration_label(seconds));
    }
    if let Some(ts) = post.created_at {
        details.push(format!("<t:{ts}:D>"));
    }
    if let Some(curator) = &clip.curator {
        details.push(format!(
            "Klipleyen: {}",
            escape_markdown(&curator.display_name)
        ));
    }

    let mut children = vec![text(header)];
    if !details.is_empty() {
        children.push(text(format!("-# {}", details.join(" · "))));
    }
    let header_component = match post.author.avatar.as_deref() {
        Some(avatar) => json!({
            "type": TYPE_SECTION,
            "components": children,
            "accessory": {
                "type": TYPE_THUMBNAIL,
                "media": { "url": avatar },
                "description": "Yayıncı avatarı",
            },
        }),
        // Section aksesuarsız olamaz: avatar yoksa klip bağlantısı aksesuar olur.
        None => json!({
            "type": TYPE_SECTION,
            "components": children,
            "accessory": link_button("Twitch", &url),
        }),
    };

    let mut buttons = vec![link_button("Klibi aç", &url)];
    if let Some(channel) = &post.author.profile_url {
        buttons.push(link_button("Kanal", channel));
    }
    if let Some(vod) = vod_url(clip) {
        buttons.push(link_button("Yayında izle", &vod));
    }

    CustomCard {
        components: vec![
            header_component,
            json!({ "type": TYPE_SEPARATOR, "spacing": 1 }),
            json!({ "type": TYPE_ACTION_ROW, "components": buttons }),
        ],
        accent_color: Some(TWITCH_PURPLE),
        video_components: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Canlı GQL yanıtından kısaltılmış örnek.
    const CLIP_JSON: &str = r#"{"data":{"clip":{
        "slug":"TangentialBillowingJalapenoYee-ccA2oB1pEYh4CHWJ","title":"F Shroud",
        "viewCount":563389,"createdAt":"2023-06-02T11:41:50Z","durationSeconds":28,
        "thumbnailURL":"https://static-cdn.jtvnw.net/thumb-1920x1080.jpg",
        "broadcaster":{"login":"shroud","displayName":"shroud","profileImageURL":"https://static-cdn.jtvnw.net/shroud-150x150.png"},
        "curator":{"displayName":"Pato_Bolado"},"game":{"displayName":"Diablo IV"},
        "video":{"id":"1835200464"},"videoOffsetSeconds":48930,
        "videoQualities":[
            {"quality":"1080","sourceURL":"https://cdn.example/1080/index.mp4"},
            {"quality":"720","sourceURL":"https://cdn.example/720/index.mp4"},
            {"quality":"480","sourceURL":"https://cdn.example/480/index.mp4"}],
        "playbackAccessToken":{"signature":"abc123","value":"{\"expires\":1792998710}"}}}}"#;

    fn sample() -> Clip {
        let parsed: GqlResponse = serde_json::from_str(CLIP_JSON).unwrap();
        parsed.data.unwrap().clip.unwrap()
    }

    fn contents(card: &CustomCard) -> String {
        serde_json::to_string(&card.components).unwrap()
    }

    #[test]
    fn slug_comes_from_the_canonical_clip_url() {
        assert_eq!(
            clip_slug("https://clips.twitch.tv/Ab_C-123").as_deref(),
            Some("Ab_C-123")
        );
        assert_eq!(
            clip_slug("https://clips.twitch.tv/Ab_C-123?tt_content=x").as_deref(),
            Some("Ab_C-123")
        );
        assert_eq!(clip_slug("https://www.twitch.tv/shroud"), None);
        assert_eq!(clip_slug("https://clips.twitch.tv/"), None);
        assert_eq!(clip_slug("https://clips.twitch.tv/a%2F..%2Fb"), None);
    }

    #[test]
    fn video_is_the_signed_720p_mp4() {
        let url = video_url(&sample()).unwrap();
        assert_eq!(
            url,
            "https://cdn.example/720/index.mp4?sig=abc123&token=%7B%22expires%22%3A1792998710%7D"
        );

        let mut unsigned = sample();
        unsigned.playback_access_token = None;
        assert_eq!(video_url(&unsigned), None, "imzasız adres 401 döner");

        let mut only_1080 = sample();
        only_1080.video_qualities.retain(|q| q.quality == "1080");
        assert_eq!(video_url(&only_1080), None);
    }

    #[test]
    fn clip_becomes_a_playable_card() {
        let post = clip_post(&sample()).unwrap();
        assert_eq!(post.platform, Platform::Twitch);
        assert_eq!(
            post.url,
            "https://clips.twitch.tv/TangentialBillowingJalapenoYee-ccA2oB1pEYh4CHWJ"
        );
        assert_eq!(post.author.name, "shroud");
        assert_eq!(
            post.author.profile_url.as_deref(),
            Some("https://www.twitch.tv/shroud")
        );
        assert_eq!(post.stats.views, Some(563_389));
        assert_eq!(post.extra_label.as_deref(), Some("Diablo IV"));
        assert_eq!(post.created_at, Some(1_685_706_110));
        assert_eq!(post.media.len(), 1);
        assert!(post.media[0].is_video());
        assert_eq!(
            post.media[0].poster.as_deref(),
            Some("https://static-cdn.jtvnw.net/thumb-1920x1080.jpg")
        );

        let card = post.custom_card.unwrap();
        assert_eq!(card.accent_color, Some(TWITCH_PURPLE));
        let text = contents(&card);
        assert!(
            text.contains("### [F Shroud](https://clips.twitch.tv/"),
            "{text}"
        );
        assert!(
            text.contains("-# [shroud](https://www.twitch.tv/shroud) · Diablo IV"),
            "{text}"
        );
        assert!(
            text.contains("563.4K izlenme · 28 sn · <t:1685706110:D> · Klipleyen: Pato_Bolado"),
            "{text}"
        );
        assert!(
            text.contains("https://www.twitch.tv/videos/1835200464?t=13h35m30s"),
            "{text}"
        );
        assert_eq!(card.components[0]["accessory"]["type"], TYPE_THUMBNAIL);
    }

    #[test]
    fn empty_title_is_an_error() {
        let mut clip = sample();
        clip.title = "  ".into();
        assert!(clip_post(&clip).is_err());
    }

    #[tokio::test]
    #[ignore = "requires live Twitch access; run explicitly"]
    async fn live_clip_has_signed_video() {
        let client = reqwest::Client::new();
        let clip = fetch_clip(&client, "TangentialBillowingJalapenoYee-ccA2oB1pEYh4CHWJ")
            .await
            .expect("klip okunmalı");
        assert_eq!(clip.broadcaster.as_ref().unwrap().login, "shroud");
        let url = video_url(&clip).expect("imzalı video adresi");
        assert!(url.contains("?sig="), "{url}");
    }
}
