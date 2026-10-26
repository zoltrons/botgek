//! Medya indirme ve Discord dosya eki yardımcıları.
//!
//! Görselleri/videoları uzak CDN'den indirip mesaja dosya olarak eklemek, embed'in
//! önizleme kartıyla sınırlı kalmamasını sağlar (tam çözünürlük, video oynatma).

use serenity::all::CreateAttachment;

use super::{MEDIA_TIMEOUT, USER_AGENT};

/// Discord'un boost'suz sunuculardaki dosya limiti (10 MB).
pub const DISCORD_FILE_LIMIT: u64 = 10 * 1024 * 1024;

/// İndirilmiş medya dosyası.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaFile {
    pub bytes: Vec<u8>,
    pub filename: String,
    pub mime: String,
}

/// İndirilen dosyayı mesaj eki hâline getirir. Hassas gönderilerde dosya adı
/// `SPOILER_` ön ekiyle sarılır (serenity bunu spoiler olarak yorumlar).
pub fn to_attachment(file: MediaFile, spoiler: bool) -> CreateAttachment {
    let filename = if spoiler {
        format!("SPOILER_{}", file.filename)
    } else {
        file.filename
    };
    CreateAttachment::bytes(file.bytes, filename)
}

/// Kart görsellerinde kullanılan X CDN varyantı.
///
/// vxTwitter ve fxTwitter ile uyumlu olarak orijinal yüksek çözünürlük (`orig`) kullanılır.
/// Böylece hem kart içinde keskin ve kaliteli görünür hem de kullanıcı Discord'da görsele
/// tıkladığında/büyüttüğünde tam çözünürlüklü asıl görsel açılır.
pub const CARD_IMAGE_SIZE: &str = "orig";

/// Görseli, Discord'da en yüksek kalitede görünmesi için orijinal çözünürlüğe (`name=orig`) çevirir.
///
/// X/Twitter görsellerinde parametre verilmediğinde veya `name=medium` kullanıldığında
/// Twitter CDN görseli sıkıştırıp 1200 px sınırına indirir; bu nedenle vx/fxTwitter gibi
/// `name=orig` varyantı sağlanır. X dışı CDN'ler olduğu gibi kalır.
pub fn card_image_url(url: &str) -> String {
    if !url.contains("pbs.twimg.com") {
        return url.to_string();
    }
    if url.contains("name=orig") {
        return url.to_string();
    }
    for variant in ["name=large", "name=medium", "name=small", "name=thumb"] {
        if url.contains(variant) {
            return url.replace(variant, &format!("name={CARD_IMAGE_SIZE}"));
        }
    }
    if url.contains('?') {
        return format!("{url}&name={CARD_IMAGE_SIZE}");
    }
    format!("{url}?name={CARD_IMAGE_SIZE}")
}

/// İndirme sırasında tam çözünürlüğü ister (Twitter `name=large` yerine `name=orig`).
pub fn download_url(url: &str) -> String {
    if url.contains("pbs.twimg.com") {
        let base = url.split('?').next().unwrap_or(url);
        return format!("{base}?name=orig");
    }
    url.to_string()
}

/// URL'den güvenli bir dosya adı türetir (uzantı yoksa `fallback_ext` kullanılır).
pub fn filename_for(url: &str, fallback_ext: &str) -> String {
    let without_query = url.split(['?', '#']).next().unwrap_or(url);
    let path = without_query
        .strip_prefix("https://")
        .or_else(|| without_query.strip_prefix("http://"))
        .unwrap_or(without_query);
    // Yol yoksa (ör. "https://example.com") dosya adı üretilemez: "media" kullanılır.
    let last = if path.contains('/') {
        path.rsplit('/').next().unwrap_or("")
    } else {
        ""
    };
    let cleaned: String = last
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .collect();

    if let Some(ext) = known_extension(&cleaned) {
        let stem: String = cleaned
            .trim_end_matches(&format!(".{ext}"))
            .chars()
            .take(40)
            .collect();
        let stem = if stem.is_empty() {
            "media".to_string()
        } else {
            stem
        };
        return format!("{stem}.{ext}");
    }

    let stem: String = cleaned
        .split('.')
        .next()
        .unwrap_or("")
        .chars()
        .take(40)
        .collect();
    let stem = if stem.is_empty() {
        "media".to_string()
    } else {
        stem
    };
    format!("{stem}.{fallback_ext}")
}

/// Dosya adındaki uzantıyı bilinen medya türleriyle eşler.
pub fn known_extension(name: &str) -> Option<&'static str> {
    let ext = name.rsplit('.').next()?.to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => Some("jpg"),
        "png" => Some("png"),
        "webp" => Some("webp"),
        "gif" => Some("gif"),
        "mp4" => Some("mp4"),
        "m4v" => Some("m4v"),
        "mov" => Some("mov"),
        "webm" => Some("webm"),
        _ => None,
    }
}

/// MIME türünden dosya uzantısı üretir.
pub fn extension_from_mime(mime: &str) -> Option<&'static str> {
    match mime {
        "image/jpeg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        "video/mp4" => Some("mp4"),
        "video/webm" => Some("webm"),
        "video/quicktime" => Some("mov"),
        _ => None,
    }
}

/// X'in animasyonlu GIF'leri için vxTwitter'ın GIF dönüştürme servisi.
///
/// X GIF'leri yalnızca sessiz mp4 olarak sunar; vxTwitter (BetterTwitFix) bunları
/// animasyonlu AVIF'e çevirip CDN'inde önbellekler (`image/avif`, `max-age=1 hafta`).
/// Servis yalnızca `video.twimg.com/tweet_video/` yolunu kabul eder:
/// `https://video.twimg.com/tweet_video/<ad>.mp4`
///   → `https://gifconvert.vxtwitter.com/convert.avif?url=https%3A%2F%2Fvideo.twimg.com%2Ftweet_video%2F<ad>.mp4`
pub fn gif_convert_url(url: &str) -> Option<String> {
    // Zaten dönüştürülmüş bir URL ise olduğu gibi döner.
    if url.starts_with("https://gifconvert.vxtwitter.com/") {
        return Some(url.to_string());
    }
    // Yalnızca tweet_video kaynaklı mp4'ler dönüştürülür.
    if !url.contains("video.twimg.com/tweet_video/") {
        return None;
    }
    // Sorgu parametrelerini temizle: servis ham mp4 URL'ini bekler.
    let clean = url.split(['?', '#']).next()?.trim();
    if clean.is_empty() {
        return None;
    }
    let encoded = urlencoding::encode(clean);
    Some(format!(
        "https://gifconvert.vxtwitter.com/convert.avif?url={encoded}"
    ))
}

/// FxEmbed'in GIF dönüştürme servisi (`gif.fxtwitter.com`): `/gif-yap` komutu için.
///
/// vxTwitter gifconvert yalnızca `tweet_video` (X GIF'i) kabul eder; FxEmbed'in servisi ise
/// gerçek videoları (`ext_tw_video`, `amplify_video`) da `image/gif`'e çevirir. Servis
/// `video.twimg.com` yolunu aynen alır, uzantı `.gif` olarak verilir:
/// `https://video.twimg.com/ext_tw_video/<id>/pu/vid/480x270/<ad>.mp4?tag=14`
///   → `https://gif.fxtwitter.com/ext_tw_video/<id>/pu/vid/480x270/<ad>.gif`
pub fn fx_gif_url(url: &str) -> Option<String> {
    let clean = url.split(['?', '#']).next()?.trim();
    let path = clean.strip_prefix("https://video.twimg.com/")?;
    let stem = path.strip_suffix(".mp4")?;
    (!stem.is_empty()).then(|| format!("https://gif.fxtwitter.com/{stem}.gif"))
}

/// Videoyu indirir. **Reddit (v.redd.it)** gönderilerinde önce video ve sesi **sesli tek MP4**'e
/// birleştirmeyi dener (`reddit_video::merge`; proje ffmpeg kullanmaz, birleştirme saf Rust
/// transmux ile yapılır). Birleştirme başarısız/doluysa None döner: renderer uzak videoyu
/// korur; sessiz MP4'ü yeniden indirip yüklemek gereksiz trafik ve bellek tüketirdi.
///
/// **Spotify** önizlemesi (`p.scdn.co`) MP3'tür: `poster` (kapak) ile sesli tek MP4'e çevrilir
/// (`spotify_video::build`); kapak yoksa ya da üretim başarısızsa None döner.
pub async fn download_video(
    client: &reqwest::Client,
    url: &str,
    poster: Option<&str>,
    max_bytes: u64,
) -> Option<MediaFile> {
    if url::Url::parse(url)
        .ok()
        .is_some_and(|url| url.host_str() == Some("v.redd.it"))
    {
        return super::reddit_video::merge(client, url, max_bytes).await;
    }
    if super::spotify_video::is_preview_url(url) {
        return super::spotify_video::build(url, poster?, max_bytes).await;
    }
    if let Some(headers) = super::tiktok::video_headers(url) {
        return download_with(url, max_bytes, &headers).await;
    }

    download(url, max_bytes).await
}

/// URL'deki medyayı indirir. Boyut `max_bytes`'ı aşarsa ya da içerik medya değilse
/// (HTML hata sayfası vb.) `None` döner; böylece mesaj gönderimi bozulmaz.
pub async fn download(url: &str, max_bytes: u64) -> Option<MediaFile> {
    download_with(url, max_bytes, &[]).await
}

/// [`download`] ile aynı; her istekte ek başlıklar gönderir (TikTok çerezi ve Referer).
pub(super) async fn download_with(
    url: &str,
    max_bytes: u64,
    headers: &[(&str, String)],
) -> Option<MediaFile> {
    use crate::telemetry::{self, HttpHost};
    let host = HttpHost::classify(url);
    telemetry::http_request(host);
    let started = std::time::Instant::now();
    let result = download_inner(url, max_bytes, headers).await;
    match &result {
        Some(file) => telemetry::http_success(host, started.elapsed(), file.bytes.len()),
        None => telemetry::http_error(host, started.elapsed()),
    }
    result
}

/// Medya adresleri kaynak API'lerinden (oEmbed, gönderi JSON'u) gelir ve indirilen dosya
/// Discord'a yüklenir: istek bu yüzden ortak istemciyle değil, yalnızca herkese açık IP'lere
/// sabitlenen ve her yönlendirmeyi doğrulayan [`super::safe_http::public_get`] ile atılır.
/// Aksi hâlde bir alan adı ya da yönlendirme sunucunun iç ağındaki bir adresi okutabilirdi.
async fn download_inner(
    url: &str,
    max_bytes: u64,
    headers: &[(&str, String)],
) -> Option<MediaFile> {
    if !url.starts_with("https://") {
        return None;
    }

    let (_, response) =
        super::safe_http::public_get_with(&download_url(url), MEDIA_TIMEOUT, USER_AGENT, headers)
            .await
            .ok()?;
    if let Some(len) = response.content_length()
        && len > max_bytes
    {
        return None;
    }

    let mime = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream")
        .split(';')
        .next()
        .unwrap_or("application/octet-stream")
        .trim()
        .to_ascii_lowercase();

    let acceptable = mime.starts_with("image/")
        || mime.starts_with("video/")
        || mime == "application/octet-stream";
    if !acceptable {
        return None;
    }

    let mut response = response;
    let mut bytes: Vec<u8> = Vec::with_capacity(64 * 1024);
    while let Some(chunk) = response.chunk().await.ok()? {
        if bytes.len() as u64 + chunk.len() as u64 > max_bytes {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        return None;
    }

    let fallback_ext = if mime.starts_with("video/") {
        extension_from_mime(&mime).unwrap_or("mp4")
    } else {
        extension_from_mime(&mime).unwrap_or("jpg")
    };

    Some(MediaFile {
        bytes,
        filename: filename_for(url, fallback_ext),
        mime,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_image_url_ensures_orig_variant() {
        // Kart görselleri vx/fxTwitter gibi orijinal yüksek çözünürlükle sunulur.
        assert_eq!(
            card_image_url("https://pbs.twimg.com/media/ABC.jpg?name=orig"),
            "https://pbs.twimg.com/media/ABC.jpg?name=orig"
        );
        assert_eq!(
            card_image_url("https://pbs.twimg.com/media/ABC.jpg?name=large"),
            "https://pbs.twimg.com/media/ABC.jpg?name=orig"
        );
        assert_eq!(
            card_image_url("https://pbs.twimg.com/media/ABC.jpg?name=medium"),
            "https://pbs.twimg.com/media/ABC.jpg?name=orig"
        );
        assert_eq!(
            card_image_url("https://pbs.twimg.com/media/ABC.jpg"),
            "https://pbs.twimg.com/media/ABC.jpg?name=orig"
        );
        // Yeni X biçimi (`?format=jpg`) desteklenir.
        assert_eq!(
            card_image_url("https://pbs.twimg.com/media/ABC?format=jpg&name=large"),
            "https://pbs.twimg.com/media/ABC?format=jpg&name=orig"
        );
        assert_eq!(
            card_image_url("https://pbs.twimg.com/media/ABC?format=jpg"),
            "https://pbs.twimg.com/media/ABC?format=jpg&name=orig"
        );
        assert_eq!(
            card_image_url("https://cdn.bsky.app/img/x.jpg"),
            "https://cdn.bsky.app/img/x.jpg"
        );
        assert_eq!(
            card_image_url("https://i.redd.it/x.jpg"),
            "https://i.redd.it/x.jpg"
        );
    }

    #[test]
    fn downloads_request_original_resolution() {
        assert_eq!(
            download_url("https://pbs.twimg.com/media/ABC.jpg?name=large"),
            "https://pbs.twimg.com/media/ABC.jpg?name=orig"
        );
        assert_eq!(
            download_url("https://video.twimg.com/x/y.mp4"),
            "https://video.twimg.com/x/y.mp4"
        );
    }

    #[test]
    fn filenames_are_derived_safely() {
        assert_eq!(
            filename_for("https://pbs.twimg.com/media/ABC123.jpg?name=orig", "jpg"),
            "ABC123.jpg"
        );
        assert_eq!(
            filename_for(
                "https://video.twimg.com/amplify_video/1/vid/720x1280/xyz.mp4",
                "mp4"
            ),
            "xyz.mp4"
        );
        // Uzantı bilinmiyorsa fallback kullanılır
        assert_eq!(
            filename_for("https://example.com/media/playlist.m3u8", "mp4"),
            "playlist.mp4"
        );
        // Yol karakterleri temizlenir
        assert_eq!(
            filename_for("https://example.com/../../etc/passwd", "jpg"),
            "passwd.jpg"
        );
        assert_eq!(filename_for("https://example.com", "jpg"), "media.jpg");
    }

    #[test]
    fn vxtwitter_gif_convert_urls_are_derived_from_tweet_videos() {
        assert_eq!(
            gif_convert_url("https://video.twimg.com/tweet_video/HTEtNFcXEAAY4Dv.mp4").as_deref(),
            Some(
                "https://gifconvert.vxtwitter.com/convert.avif?url=https%3A%2F%2Fvideo.twimg.com%2Ftweet_video%2FHTEtNFcXEAAY4Dv.mp4"
            )
        );
        // Sorgu parametreli varyantlar da desteklenir (parametreler temizlenir)
        assert_eq!(
            gif_convert_url("https://video.twimg.com/tweet_video/abc.mp4?tag=12").as_deref(),
            Some(
                "https://gifconvert.vxtwitter.com/convert.avif?url=https%3A%2F%2Fvideo.twimg.com%2Ftweet_video%2Fabc.mp4"
            )
        );
        // Doğrudan gifconvert.vxtwitter.com adresi idempotent olarak döner
        assert_eq!(
            gif_convert_url("https://gifconvert.vxtwitter.com/convert.avif?url=https%3A%2F%2Fvideo.twimg.com%2Ftweet_video%2Fx.mp4").as_deref(),
            Some("https://gifconvert.vxtwitter.com/convert.avif?url=https%3A%2F%2Fvideo.twimg.com%2Ftweet_video%2Fx.mp4")
        );
        // Gerçek videolar (amplify/ext_tw_video) GIF servisine gönderilmez
        assert!(
            gif_convert_url("https://video.twimg.com/amplify_video/123/vid/720/x.mp4").is_none()
        );
        assert!(gif_convert_url("https://cdn.example/video.mp4").is_none());
    }

    #[test]
    fn fx_gif_urls_accept_real_videos_and_gifs() {
        assert_eq!(
            fx_gif_url("https://video.twimg.com/ext_tw_video/1/pu/vid/480x270/a.mp4?tag=14")
                .as_deref(),
            Some("https://gif.fxtwitter.com/ext_tw_video/1/pu/vid/480x270/a.gif")
        );
        assert_eq!(
            fx_gif_url("https://video.twimg.com/tweet_video/HTEtNFcXEAAY4Dv.mp4").as_deref(),
            Some("https://gif.fxtwitter.com/tweet_video/HTEtNFcXEAAY4Dv.gif")
        );
        assert!(fx_gif_url("https://video.twimg.com/ext_tw_video/1/pu/pl/x.m3u8").is_none());
        assert!(fx_gif_url("https://cdn.example/video.mp4").is_none());
    }
}
