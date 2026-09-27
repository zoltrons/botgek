//! Twitter / X gönderi verisi.
//!
//! İki kaynak desteklenir:
//! - **Syndication** (`cdn.syndication.twimg.com/tweet-result`): X'in kendi uç noktası,
//!   üçüncü taraf yok, API anahtarı yok. Metin/görsel/video/alıntı/metrik verir.
//! - **FxTwitter** (`api.fxtwitter.com`): görüntülenme, repost, topluluk notu gibi ek alanlar.

use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;
use tracing::{debug, info, warn};

use super::media::with_size;
use super::translate::{needs_turkish_translation, TARGET_LANGUAGE_CODE};
use super::{parse_timestamp, strip_trailing_media_links, Author, BuiltinPost, Media, MediaKind, Stats, Translation};
use crate::converters::parser::{ParsedPost, Platform};
use crate::settings::{EngineSettings, TwitterSource};

const SYNDICATION_URL: &str = "https://cdn.syndication.twimg.com/tweet-result";
const FXTWITTER_URL: &str = "https://api.fxtwitter.com";

static TWEET_URL_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?:twitter\.com|x\.com)/([A-Za-z0-9_]{1,20})/status/(\d{1,25})").expect("geçerli tweet regex")
});

/// Gönderi verisini seçilen kaynaktan çeker.
///
/// `engine.auto_translate` açıkken X'in **kendi çeviri servisi** kullanılır (Türkçe/İngilizce
/// dışındaki gönderiler için):
/// - FxTwitter/hibrit kaynağında istek dil ekli uçtan yapılır; yanıt orijinal metni ve
///   X çevirisini birlikte içerir (tek istek, karar yerelde verilir).
/// - Syndication kaynağında gönderi yabancı dildeyse X çevirisi için FxTwitter'a bir kez
///   ek istek yapılır (syndication uç noktası çeviri içermez).
pub async fn fetch(
    client: &reqwest::Client,
    parsed: &ParsedPost,
    engine: &EngineSettings,
) -> Result<BuiltinPost, String> {
    let captures = TWEET_URL_REGEX
        .captures(&parsed.original_url)
        .ok_or_else(|| format!("Twitter kimliği çözümlenemedi: {}", parsed.original_url))?;
    let id = captures
        .get(2)
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| format!("Twitter gönderi kimliği yok: {}", parsed.original_url))?;

    let source = engine.twitter_source;
    debug!("X verisi çekiliyor (kaynak: {}, id: {id})", source.key());

    // Çeviri açıkken ve kaynak FxTwitter ise: orijinal + X çevirisi tek istekte gelir.
    if engine.auto_translate && source != TwitterSource::Syndication {
        match from_fxtwitter(client, &parsed.original_url, &id, Some(TARGET_LANGUAGE_CODE)).await {
            Ok(post) => return Ok(post),
            Err(err) => warn!("X çeviri isteği başarısız, normal akışa düşülüyor ({id}): {err}"),
        }
    }

    let mut post = match source {
        TwitterSource::Syndication => from_syndication(client, &parsed.original_url, &id).await?,
        TwitterSource::Fxtwitter => {
            from_fxtwitter(client, &parsed.original_url, &id, None).await?
        }
        TwitterSource::Hybrid => match from_syndication(client, &parsed.original_url, &id).await {
            Ok(post) => post,
            Err(err) => {
                warn!("X syndication başarısız, fxtwitter deneniyor ({id}): {err}");
                from_fxtwitter(client, &parsed.original_url, &id, None).await?
            }
        },
    };

    if engine.auto_translate && source != TwitterSource::Fxtwitter {
        // FxTwitter kaynağında çeviri isteği yukarıda denendi; burada tekrar denenmez.
        apply_x_translation(client, &parsed.original_url, &id, &mut post).await;
    }

    Ok(post)
}

/// Syndication ile çekilen gönderi Türkçe/İngilizce değilse X'in çevirisini uygular.
/// Çeviri, FxTwitter'ın dil ekli ucundan (X'in çeviri servisi) alınır.
async fn apply_x_translation(
    client: &reqwest::Client,
    original_url: &str,
    id: &str,
    post: &mut BuiltinPost,
) {
    if !needs_turkish_translation(post.language.as_deref()) {
        return;
    }

    match from_fxtwitter(client, original_url, id, Some(TARGET_LANGUAGE_CODE)).await {
        Ok(translated) => {
            let Some(translation) = translated.translation else {
                warn!("X çevirisi yanıtta bulunamadı ({id})");
                return;
            };
            post.text = translated.text;
            post.translation = Some(translation);
            info!(
                "X çevirisi uygulandı (kaynak: {}): {}",
                post.translation
                    .as_ref()
                    .and_then(|t| t.source.as_deref())
                    .unwrap_or("bilinmiyor"),
                original_url
            );
        }
        Err(err) => warn!("X çevirisi alınamadı ({id}): {err}"),
    }
}

/// X'in syndication uç noktası token'ı doğrulamıyor; istemci tarafında üretilen
/// deterministik token yeterli oluyor (referans JS: `((id / 1e15) * Math.PI).toString(36)`).
fn syndication_token(id: &str) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";

    let mut value = (id.parse::<f64>().unwrap_or(0.0) / 1e15) * std::f64::consts::PI;
    let integral = value.trunc();
    let mut whole = integral as u64;
    let mut stack: Vec<u8> = Vec::new();
    if whole == 0 {
        stack.push(b'0');
    }
    while whole > 0 {
        stack.push(DIGITS[(whole % 36) as usize]);
        whole /= 36;
    }

    let mut out = String::new();
    for digit in stack.iter().rev() {
        out.push(*digit as char);
    }

    value -= integral;
    if value > 0.0 {
        out.push('.');
        for _ in 0..18 {
            value *= 36.0;
            let digit = value.trunc() as usize;
            out.push(DIGITS[digit % 36] as char);
            value -= value.trunc();
            if value <= 0.0 {
                break;
            }
        }
    }

    // JS referansındaki `.replace(/(0+|\.)/g, '')` ile aynı davranış
    out.replace('.', "").replace('0', "")
}

// =====================================================================================
// SYNDICATION
// =====================================================================================

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SyndicationTweet {
    id_str: Option<String>,
    text: Option<String>,
    created_at: Option<String>,
    favorite_count: Option<u64>,
    conversation_count: Option<u64>,
    possibly_sensitive: Option<bool>,
    display_text_range: Option<Vec<usize>>,
    lang: Option<String>,
    user: Option<SyndicationUser>,
    entities: Option<SyndicationEntities>,
    photos: Option<Vec<SyndicationPhoto>>,
    video: Option<SyndicationVideo>,
    #[serde(rename = "mediaDetails")]
    media_details: Option<Vec<SyndicationMediaDetail>>,
    quoted_tweet: Option<Box<SyndicationTweet>>,
    note_tweet: Option<SyndicationNoteTweet>,
}

impl SyndicationTweet {
    /// Silinmiş/korunmuş gönderilerde tüm alanlar boş gelir.
    fn is_empty(&self) -> bool {
        self.text.is_none()
            && self.user.is_none()
            && self.media_details.is_none()
            && self.photos.is_none()
            && self.video.is_none()
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SyndicationUser {
    name: Option<String>,
    screen_name: Option<String>,
    profile_image_url_https: Option<String>,
    is_blue_verified: Option<bool>,
    verified: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SyndicationEntities {
    urls: Option<Vec<SyndicationUrlEntity>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SyndicationUrlEntity {
    url: Option<String>,
    expanded_url: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SyndicationPhoto {
    url: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SyndicationVideo {
    poster: Option<String>,
    variants: Option<Vec<SyndicationVariant>>,
    #[serde(rename = "durationMs")]
    duration_ms: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SyndicationVariant {
    src: Option<String>,
    url: Option<String>,
    content_type: Option<String>,
    bitrate: Option<u64>,
}

impl SyndicationVariant {
    fn location(&self) -> Option<&str> {
        self.src.as_deref().or(self.url.as_deref())
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SyndicationMediaDetail {
    #[serde(rename = "type")]
    kind: Option<String>,
    media_url_https: Option<String>,
    original_info: Option<SyndicationOriginalInfo>,
    video_info: Option<SyndicationVideoInfo>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SyndicationOriginalInfo {
    width: Option<u32>,
    height: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SyndicationVideoInfo {
    variants: Option<Vec<SyndicationVariant>>,
    duration_millis: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SyndicationNoteTweet {
    text: Option<String>,
}

/// En yüksek bit hızlı mp4 varyantını seçer (HLS/m3u8 oynatılamaz).
fn best_mp4(variants: Option<&[SyndicationVariant]>) -> Option<String> {
    variants?
        .iter()
        .filter(|v| {
            let content_type = v.content_type.as_deref().unwrap_or("video/mp4");
            content_type.contains("mp4")
        })
        .filter_map(|v| v.location().map(|url| (v.bitrate.unwrap_or(0), url.to_string())))
        .max_by_key(|(bitrate, _)| *bitrate)
        .map(|(_, url)| url)
}

async fn from_syndication(client: &reqwest::Client, original_url: &str, id: &str) -> Result<BuiltinPost, String> {
    let url = format!("{SYNDICATION_URL}?id={id}&token={}&lang=en", syndication_token(id));
    let value = super::get_json(client, &url).await?;
    let tweet: SyndicationTweet = serde_json::from_value(value)
        .map_err(|e| format!("Syndication yanıtı çözümlenemedi ({id}): {e}"))?;

    if tweet.is_empty() {
        return Err(format!("Gönderi bulunamadı (silinmiş veya korumalı olabilir): {id}"));
    }

    syndication_to_post(&tweet, original_url)
}

fn syndication_to_post(tweet: &SyndicationTweet, url: &str) -> Result<BuiltinPost, String> {
    let user = tweet.user.as_ref();
    let handle = user.and_then(|u| u.screen_name.clone()).unwrap_or_default();
    let mut author = Author::new(
        user.and_then(|u| u.name.clone()).unwrap_or_else(|| handle.clone()),
        handle.clone(),
    );
    author.avatar = user.and_then(|u| u.profile_image_url_https.clone());
    author.verified = user
        .and_then(|u| u.is_blue_verified.or(u.verified))
        .unwrap_or(false);
    author.profile_url = if handle.is_empty() {
        None
    } else {
        Some(format!("https://x.com/{handle}"))
    };

    let mut post = BuiltinPost::new(Platform::Twitter, url, author);

    let raw_text = tweet
        .note_tweet
        .as_ref()
        .and_then(|n| n.text.clone())
        .or_else(|| tweet.text.clone())
        .unwrap_or_default();
    let raw_text = if tweet.note_tweet.is_some() {
        raw_text
    } else {
        cut_to_display_range(&raw_text, tweet.display_text_range.as_deref())
    };
    post.text = strip_trailing_media_links(&expand_url_entities(&raw_text, tweet.entities.as_ref()));
    post.language = tweet.lang.clone().filter(|lang| !lang.trim().is_empty());

    post.created_at = tweet.created_at.as_deref().and_then(parse_timestamp);
    post.stats = Stats {
        replies: tweet.conversation_count.unwrap_or(0),
        reposts: 0,
        likes: tweet.favorite_count.unwrap_or(0),
        quotes: 0,
        views: None,
    };
    post.sensitive = tweet.possibly_sensitive.unwrap_or(false);
    post.media = syndication_media(tweet);

    if let Some(quoted) = tweet.quoted_tweet.as_deref() {
        let quoted_url = quoted.id_str.as_deref().map(|quoted_id| {
            let quoted_handle = quoted
                .user
                .as_ref()
                .and_then(|u| u.screen_name.clone())
                .unwrap_or_default();
            format!("https://x.com/{quoted_handle}/status/{quoted_id}")
        });
        if let Some(quoted_url) = quoted_url {
            if let Ok(quoted_post) = syndication_to_post(quoted, &quoted_url) {
                post.quote = Some(Box::new(quoted_post));
            }
        }
    }

    Ok(post)
}

/// Gönderi metnini `display_text_range` sınırına kadar keser (metnin sonundaki medya
/// kısa linki bu aralığın dışında kalır).
fn cut_to_display_range(text: &str, range: Option<&[usize]>) -> String {
    match range {
        Some(range) if range.len() >= 2 => text
            .chars()
            .skip(range[0])
            .take(range[1].saturating_sub(range[0]))
            .collect(),
        _ => text.to_string(),
    }
}

/// `entities.urls` içindeki `t.co` kısa linklerini gerçek adresleriyle değiştirir.
fn expand_url_entities(text: &str, entities: Option<&SyndicationEntities>) -> String {
    let mut out = text.to_string();
    if let Some(urls) = entities.and_then(|e| e.urls.as_deref()) {
        for entity in urls {
            if let (Some(short), Some(expanded)) = (entity.url.as_deref(), entity.expanded_url.as_deref()) {
                if short == expanded {
                    continue;
                }
                out = out.replace(short, expanded);
                out = out.replace(&short.replace("http://", "https://"), expanded);
            }
        }
    }
    out
}

/// `mediaDetails` (varsa) veya `photos` + `video` alanlarından medya listesi üretir.
fn syndication_media(tweet: &SyndicationTweet) -> Vec<Media> {
    if let Some(details) = tweet.media_details.as_deref() {
        if !details.is_empty() {
            return details.iter().filter_map(media_from_detail).collect();
        }
    }

    let mut media = Vec::new();
    if let Some(photos) = tweet.photos.as_deref() {
        for photo in photos {
            if let Some(url) = photo.url.as_deref() {
                media.push(Media {
                    kind: MediaKind::Image,
                    url: with_size(url, "large"),
                    poster: None,
                    width: photo.width,
                    height: photo.height,
                    duration: None,
                });
            }
        }
    }
    if let Some(video) = tweet.video.as_ref() {
        if let Some(url) = best_mp4(video.variants.as_deref()) {
            media.push(Media {
                kind: MediaKind::Video,
                url,
                poster: video.poster.as_deref().map(|p| with_size(p, "large")),
                width: None,
                height: None,
                duration: video.duration_ms.map(|ms| ms / 1000),
            });
        }
    }
    media
}

fn media_from_detail(detail: &SyndicationMediaDetail) -> Option<Media> {
    let kind = detail.kind.as_deref()?;
    let info = detail.original_info.as_ref();
    let width = info.and_then(|i| i.width);
    let height = info.and_then(|i| i.height);

    match kind {
        "photo" => Some(Media {
            kind: MediaKind::Image,
            url: with_size(detail.media_url_https.as_deref()?, "large"),
            poster: None,
            width,
            height,
            duration: None,
        }),
        "video" | "animated_gif" => {
            let video_info = detail.video_info.as_ref()?;
            let url = best_mp4(video_info.variants.as_deref())?;
            Some(Media {
                kind: if kind == "animated_gif" {
                    MediaKind::Gif
                } else {
                    MediaKind::Video
                },
                url,
                poster: detail
                    .media_url_https
                    .as_deref()
                    .map(|poster| with_size(poster, "large")),
                width,
                height,
                duration: video_info.duration_millis.map(|ms| ms / 1000),
            })
        }
        _ => None,
    }
}

// =====================================================================================
// FXTWITTER (api.fxtwitter.com)
// =====================================================================================

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FxResponse {
    code: Option<u16>,
    message: Option<String>,
    tweet: Option<FxTweet>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FxTweet {
    url: Option<String>,
    text: Option<String>,
    lang: Option<String>,
    created_at: Option<String>,
    created_timestamp: Option<i64>,
    author: Option<FxAuthor>,
    likes: Option<u64>,
    retweets: Option<u64>,
    replies: Option<u64>,
    quotes: Option<u64>,
    views: Option<u64>,
    possibly_sensitive: Option<bool>,
    community_note: Option<FxCommunityNote>,
    media: Option<FxMedia>,
    quote: Option<Box<FxTweet>>,
    translation: Option<FxTranslation>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FxAuthor {
    name: Option<String>,
    screen_name: Option<String>,
    avatar_url: Option<String>,
    verification: Option<FxVerification>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FxVerification {
    verified: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FxCommunityNote {
    text: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FxMedia {
    all: Option<Vec<FxMediaItem>>,
    photos: Option<Vec<FxMediaItem>>,
    videos: Option<Vec<FxMediaItem>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FxMediaItem {
    #[serde(rename = "type")]
    kind: Option<String>,
    url: Option<String>,
    thumbnail_url: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    duration: Option<f64>,
}

/// FxTwitter'ın dil ekli ucundan gelen X çevirisi (`provider` X'in çeviri servisidir).
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FxTranslation {
    text: Option<String>,
    source_lang: Option<String>,
    target_lang: Option<String>,
}

async fn from_fxtwitter(
    client: &reqwest::Client,
    original_url: &str,
    id: &str,
    language: Option<&str>,
) -> Result<BuiltinPost, String> {
    // Dil eki (`/tr`) verildiğinde yanıt orijinal metinle birlikte X'in çevirisini de içerir.
    let language_suffix = language.map(|lang| format!("/{lang}")).unwrap_or_default();
    let url = format!("{FXTWITTER_URL}/i/status/{id}{language_suffix}");
    let value = super::get_json(client, &url).await?;
    let response: FxResponse = serde_json::from_value(value)
        .map_err(|e| format!("fxtwitter yanıtı çözümlenemedi ({id}): {e}"))?;

    if let Some(code) = response.code {
        if code != 200 {
            return Err(format!(
                "fxtwitter gönderiyi döndürmedi ({id}, kod {code}): {}",
                response.message.unwrap_or_default()
            ));
        }
    }

    let tweet = response
        .tweet
        .ok_or_else(|| format!("fxtwitter yanıtında gönderi yok: {id}"))?;

    fx_to_post(&tweet, original_url)
}

fn fx_to_post(tweet: &FxTweet, fallback_url: &str) -> Result<BuiltinPost, String> {
    let user = tweet.author.as_ref();
    let handle = user.and_then(|a| a.screen_name.clone()).unwrap_or_default();
    let mut author = Author::new(
        user.and_then(|a| a.name.clone()).unwrap_or_else(|| handle.clone()),
        handle.clone(),
    );
    author.avatar = user.and_then(|a| a.avatar_url.clone());
    author.verified = user
        .and_then(|a| a.verification.as_ref())
        .and_then(|v| v.verified)
        .unwrap_or(false);
    author.profile_url = if handle.is_empty() {
        None
    } else {
        Some(format!("https://x.com/{handle}"))
    };

    let url = tweet
        .url
        .clone()
        .unwrap_or_else(|| fallback_url.to_string())
        .replace("://twitter.com/", "://x.com/");

    let mut post = BuiltinPost::new(Platform::Twitter, url, author);
    post.text = strip_trailing_media_links(&tweet.text.clone().unwrap_or_default());
    post.language = tweet.lang.clone().filter(|lang| !lang.trim().is_empty());
    post.created_at = tweet
        .created_timestamp
        .or_else(|| tweet.created_at.as_deref().and_then(parse_timestamp));
    post.stats = Stats {
        replies: tweet.replies.unwrap_or(0),
        reposts: tweet.retweets.unwrap_or(0),
        likes: tweet.likes.unwrap_or(0),
        quotes: tweet.quotes.unwrap_or(0),
        views: tweet.views,
    };
    post.sensitive = tweet.possibly_sensitive.unwrap_or(false);
    post.community_note = tweet
        .community_note
        .as_ref()
        .and_then(|note| note.text.clone())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty());
    post.media = fx_media(tweet.media.as_ref());

    if let Some(quoted) = tweet.quote.as_deref() {
        let quoted_url = quoted
            .url
            .as_deref()
            .unwrap_or_default()
            .replace("://twitter.com/", "://x.com/");
        if !quoted_url.is_empty() {
            if let Ok(quoted_post) = fx_to_post(quoted, &quoted_url) {
                post.quote = Some(Box::new(quoted_post));
            }
        }
    }

    // Alıntılanan gönderi de kendi `fx_to_post` çağrısında çevrilir.
    apply_fx_translation(&mut post, tweet);

    Ok(post)
}

/// Dil ekli FxTwitter yanıtındaki X çevirisini gönderiye uygular.
///
/// Yalnızca gönderi Türkçe/İngilizce değilse uygulanır; çeviri metni X'in kendi çeviri
/// servisinden geldiği için orijinal metnin yerine kullanılır ve alt bilgiye not eklenir.
fn apply_fx_translation(post: &mut BuiltinPost, tweet: &FxTweet) {
    if !needs_turkish_translation(post.language.as_deref()) {
        return;
    }

    let Some(translation) = tweet.translation.as_ref() else {
        return;
    };
    let Some(text) = translation
        .text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    else {
        return;
    };

    post.text = strip_trailing_media_links(text);
    post.translation = Some(Translation {
        source: translation
            .source_lang
            .clone()
            .or_else(|| post.language.clone()),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FxTwitter'ın dil ekli (`/tr`) ucundan alınan yanıtın alan yapısı (canlı yanıttan).
    fn translated_tweet(lang: &str) -> serde_json::Value {
        serde_json::json!({
            "url": "https://x.com/kullanici/status/1",
            "text": "Guten Morgen zusammen",
            "lang": lang,
            "created_timestamp": 1_714_998_896,
            "author": { "name": "Kullanıcı", "screen_name": "kullanici" },
            "translation": {
                "text": "Herkese günaydın",
                "source_lang": "de",
                "target_lang": "tr",
                "source_lang_en": "German",
                "provider": "grok"
            }
        })
    }

    fn post_from(json: &serde_json::Value) -> BuiltinPost {
        let tweet: FxTweet =
            serde_json::from_value(json.clone()).expect("örnek JSON geçerli olmalı");
        fx_to_post(&tweet, "https://x.com/kullanici/status/1").expect("gönderi üretilmeli")
    }

    #[test]
    fn foreign_post_uses_x_translation() {
        let post = post_from(&translated_tweet("de"));

        assert_eq!(post.language.as_deref(), Some("de"));
        assert_eq!(post.text, "Herkese günaydın", "X çevirisi metne yazılmalı");
        assert_eq!(
            post.translation.as_ref().and_then(|t| t.source.as_deref()),
            Some("de")
        );
    }

    #[test]
    fn turkish_and_english_posts_keep_original_text() {
        for lang in ["en", "tr", "en-GB"] {
            let post = post_from(&translated_tweet(lang));
            assert_eq!(post.text, "Guten Morgen zusammen", "orijinal metin korunmalı ({lang})");
            assert!(post.translation.is_none(), "tr/en gönderiler çevrilmemeli ({lang})");
        }
    }

    #[test]
    fn quoted_post_is_translated_separately() {
        let mut json = translated_tweet("ja");
        json["text"] = serde_json::json!("asıl gönderi");
        json["translation"] = serde_json::json!({
            "text": "asıl gönderinin çevirisi",
            "source_lang": "ja",
            "target_lang": "tr"
        });
        json["quote"] = serde_json::json!({
            "url": "https://x.com/diger/status/2",
            "text": "引用テキスト",
            "lang": "ja",
            "author": { "name": "Diğer", "screen_name": "diger" },
            "translation": {
                "text": "alıntı metni",
                "source_lang": "ja",
                "target_lang": "tr"
            }
        });

        let post = post_from(&json);
        assert_eq!(post.text, "asıl gönderinin çevirisi");

        let quote = post.quote.expect("alıntı eklenmeli");
        assert_eq!(quote.text, "alıntı metni");
        assert_eq!(
            quote.translation.as_ref().and_then(|t| t.source.as_deref()),
            Some("ja")
        );
    }

    #[test]
    fn trailing_media_link_is_removed_from_translation() {
        let mut json = translated_tweet("de");
        json["translation"]["text"] = serde_json::json!("Herkese günaydın https://t.co/abc123");

        let post = post_from(&json);
        assert_eq!(post.text, "Herkese günaydın");
    }

    #[test]
    fn missing_or_empty_translation_keeps_original() {
        let mut without_translation = translated_tweet("de");
        without_translation
            .as_object_mut()
            .expect("obje")
            .remove("translation");
        let post = post_from(&without_translation);
        assert_eq!(post.text, "Guten Morgen zusammen");
        assert!(post.translation.is_none());

        let mut empty_translation = translated_tweet("de");
        empty_translation["translation"]["text"] = serde_json::json!("   ");
        let post = post_from(&empty_translation);
        assert_eq!(post.text, "Guten Morgen zusammen");
        assert!(post.translation.is_none());
    }
}

/// `media.all` (sıralı) varsa onu, yoksa `photos` + `videos` listelerini kullanır.
fn fx_media(media: Option<&FxMedia>) -> Vec<Media> {
    let Some(media) = media else {
        return Vec::new();
    };

    let mut items: Vec<&FxMediaItem> = media.all.as_deref().unwrap_or_default().iter().collect();
    if items.is_empty() {
        items.extend(media.photos.as_deref().unwrap_or_default());
        items.extend(media.videos.as_deref().unwrap_or_default());
    }

    items.iter().filter_map(|item| fx_media_item(item)).collect()
}

fn fx_media_item(item: &FxMediaItem) -> Option<Media> {
    let url = item.url.as_deref()?;
    let kind = match item.kind.as_deref()? {
        "photo" => MediaKind::Image,
        "gif" => MediaKind::Gif,
        "video" => MediaKind::Video,
        _ => return None,
    };

    Some(Media {
        kind,
        url: url.to_string(),
        poster: item.thumbnail_url.clone(),
        width: item.width,
        height: item.height,
        duration: item.duration.map(|seconds| seconds.round() as u64),
    })
}

