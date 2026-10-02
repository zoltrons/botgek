//! Medya indirme ve Discord dosya eki yardımcıları.
//!
//! Görselleri/videoları uzak CDN'den indirip mesaja dosya olarak eklemek, embed'in
//! önizleme kartıyla sınırlı kalmamasını sağlar (tam çözünürlük, video oynatma).

use serenity::all::CreateAttachment;

use super::{MEDIA_TIMEOUT, USER_AGENT};

/// Discord'un boost'suz sunuculardaki dosya limiti (10 MB).
pub const DISCORD_FILE_LIMIT: u64 = 10 * 1024 * 1024;
/// Üretilen GIF için üst sınır (Discord limitinin altında güvenli pay).
pub const MAX_GIF_BYTES: u64 = 8 * 1024 * 1024;
/// GIF dönüşümü için harici araç (yoksa GIF mp4 olarak eklenir).
const FFMPEG_BIN: &str = "ffmpeg";
/// GIF dönüşümünün en fazla süresi (gerekirse birden fazla ayar denenir).
const GIF_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// Tercih edilen GIF boyutu: bu sınırın altına inen ilk ayar kullanılır.
const GIF_PREFERRED_BYTES: u64 = 4 * 1024 * 1024;
/// Kaliteden boyuta doğru GIF ayarları (paletli, `-loop 0` ile sonsuz döngü):
/// yüksek kaliteli ayar sığmazsa bir sonraki denenir, hiçbiri sığmazsa mp4 kalır.
const GIF_FILTERS: [&str; 3] = [
    // 12 fps, en fazla 540 px genişlik, 160 renk
    "fps=12,scale='min(540,iw)':-2:flags=lanczos,split[a][b];[a]palettegen=max_colors=160:stats_mode=diff[p];[b][p]paletteuse=dither=none",
    // 12 fps, en fazla 480 px genişlik, 128 renk
    "fps=12,scale='min(480,iw)':-2:flags=lanczos,split[a][b];[a]palettegen=max_colors=128:stats_mode=diff[p];[b][p]paletteuse=dither=none",
    // 10 fps, en fazla 360 px genişlik, 96 renk
    "fps=10,scale='min(360,iw)':-2:flags=lanczos,split[a][b];[a]palettegen=max_colors=96:stats_mode=diff[p];[b][p]paletteuse=dither=none",
];

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
/// Discord kartı görseli en fazla ~550 px genişlikte çizer; `large` (2048 px, ~180-500 KB)
/// gereksiz büyüktür. `medium` (1200 px) yüksek yoğunluklu ekranlarda da net görünür ve
/// ~%35 daha az veri çekildiği için görsel **daha erken** belirir. `small` (680 px) daha da
/// hafiftir ama büyük/kaliteli ekranlarda yumuşak kalır.
pub const CARD_IMAGE_SIZE: &str = "medium";

/// Görseli, Discord'un **anında** çekebileceği hafif varyanta çevirir (kart gösterimi için).
///
/// X görselleri `?name=orig`/`?name=large` ile birkaç yüz KB-MB gelebilir; kart görseli
/// CDN'den çekildiği için [`CARD_IMAGE_SIZE`] varyantı kullanılır. Zaten daha hafif bir
/// varyant seçilmişse (`small`, `thumb`, `medium`) dokunulmaz; X dışı CDN'ler olduğu gibi
/// kalır. **İndirme** yolu bu işlevden geçmez: dosya eki modunda tam çözünürlük istenir.
pub fn card_image_url(url: &str) -> String {
    if !url.contains("pbs.twimg.com") {
        return url.to_string();
    }
    for lighter in ["name=small", "name=thumb", "name=medium"] {
        if url.contains(lighter) {
            return url.to_string();
        }
    }
    for heavier in ["name=orig", "name=large"] {
        if url.contains(heavier) {
            return url.replace(heavier, &format!("name={CARD_IMAGE_SIZE}"));
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
        let base = url.split(|c| c == '?').next().unwrap_or(url);
        return format!("{base}?name=orig");
    }
    url.to_string()
}

/// URL'den güvenli bir dosya adı türetir (uzantı yoksa `fallback_ext` kullanılır).
pub fn filename_for(url: &str, fallback_ext: &str) -> String {
    let without_query = url.split(|c| c == '?' || c == '#').next().unwrap_or(url);
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
        let stem = if stem.is_empty() { "media".to_string() } else { stem };
        return format!("{stem}.{ext}");
    }

    let stem: String = cleaned.split('.').next().unwrap_or("").chars().take(40).collect();
    let stem = if stem.is_empty() { "media".to_string() } else { stem };
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
    Some(format!("https://gifconvert.vxtwitter.com/convert.avif?url={encoded}"))
}

/// İndirilen içerik animasyonlu görsel mi? GIF (`image/gif`) veya animasyonlu AVIF
/// (`image/avif`, vxTwitter gifconvert servisi) kabul edilir. Servis beklenmedik bir şey
/// döndürürse mp4'e düşülür.
pub fn is_animated_image(file: &MediaFile) -> bool {
    file.mime == "image/gif"
        || file.bytes.starts_with(b"GIF8")
        || file.mime == "image/avif"
}

/// Dosya adının uzantısını değiştirir (`x.mp4` → `x.gif`).
pub fn replace_extension(filename: &str, ext: &str) -> String {
    let stem = filename.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(filename);
    format!("{stem}.{ext}")
}

/// URL'deki medyayı indirir. Boyut `max_bytes`'ı aşarsa ya da içerik medya değilse
/// (HTML hata sayfası vb.) `None` döner; böylece mesaj gönderimi bozulmaz.
pub async fn download(client: &reqwest::Client, url: &str, max_bytes: u64) -> Option<MediaFile> {
    if !url.starts_with("https://") {
        return None;
    }

    let response = client
        .get(download_url(url))
        .header("user-agent", USER_AGENT)
        .timeout(MEDIA_TIMEOUT)
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        return None;
    }
    if let Some(len) = response.content_length() {
        if len > max_bytes {
            return None;
        }
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

/// X'in `animated_gif` medyası yalnızca **sessiz mp4** olarak sunulur; Discord mp4'leri
/// oynatıcı (play tuşu, zaman çubuğu) olarak gösterdiği için GIF'ler video gibi görünür.
/// Bu fonksiyon mp4'ü `ffmpeg` ile **gerçek `.gif` dosyasına** çevirir: kartın içinde
/// GIF gibi (döngülü, oynatıcısız) oynar.
///
/// `ffmpeg` kurulu değilse, dönüşüm başarısız olursa veya çıktı `MAX_GIF_BYTES`'ı aşarsa
/// `None` döner; çağıran taraf mp4'ü ekler (eski davranış korunur).
pub async fn to_gif(source: &[u8]) -> Option<Vec<u8>> {
    let source = source.to_vec();
    let converted = tokio::time::timeout(
        GIF_TIMEOUT,
        tokio::task::spawn_blocking(move || convert(source)),
    )
    .await;

    match converted {
        Ok(Ok(bytes)) => bytes,
        _ => None,
    }
}

/// Kalite ayarlarını sırayla dener: `GIF_PREFERRED_BYTES` altına inen ilk sonuç döner,
/// hiçbiri tercih edilen boyuta inmemişse (ama Discord limitine sığan) en iyisi kullanılır.
fn convert(source: Vec<u8>) -> Option<Vec<u8>> {
    let mut fitting: Option<Vec<u8>> = None;

    for filter in GIF_FILTERS {
        let Some(bytes) = run_ffmpeg(&source, filter) else {
            continue;
        };
        let size = bytes.len() as u64;
        if size <= GIF_PREFERRED_BYTES {
            return Some(bytes);
        }
        if size <= MAX_GIF_BYTES {
            fitting = Some(bytes);
        }
    }

    fitting
}

/// ffmpeg'i çalıştırır: mp4 (stdin) → gif (stdout). Bloklayan iş `spawn_blocking` içinde yapılır.
fn run_ffmpeg(source: &[u8], filter: &str) -> Option<Vec<u8>> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new(FFMPEG_BIN)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            "pipe:0",
            "-vf",
            filter,
            // Aşırı uzun kaynaklarda süre sınırlanır; GIF sonsuz döngüde oynar.
            "-t",
            "30",
            "-loop",
            "0",
            "-f",
            "gif",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let mut stdin = child.stdin.take()?;
    // Boru tamponu dolup kilitlenmesin diye yazma ayrı bir iş parçacığında yapılır.
    let owned = source.to_vec();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&owned);
        let _ = stdin.flush();
    });

    let output = child.wait_with_output().ok();
    let _ = writer.join();

    let output = output?;
    if !output.status.success() || output.stdout.is_empty() {
        return None;
    }

    Some(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_image_url_downshifts_to_the_light_variant() {
        // Kart görselleri CDN'den çekildiği için hafif varyant kullanılır (daha erken belirir).
        assert_eq!(
            card_image_url("https://pbs.twimg.com/media/ABC.jpg?name=orig"),
            "https://pbs.twimg.com/media/ABC.jpg?name=medium"
        );
        assert_eq!(
            card_image_url("https://pbs.twimg.com/media/ABC.jpg?name=large"),
            "https://pbs.twimg.com/media/ABC.jpg?name=medium"
        );
        assert_eq!(
            card_image_url("https://pbs.twimg.com/media/ABC.jpg"),
            "https://pbs.twimg.com/media/ABC.jpg?name=medium"
        );
        // Yeni X biçimi (`?format=jpg`) ve zaten hafif varyantlar korunur.
        assert_eq!(
            card_image_url("https://pbs.twimg.com/media/ABC?format=jpg&name=large"),
            "https://pbs.twimg.com/media/ABC?format=jpg&name=medium"
        );
        assert_eq!(
            card_image_url("https://pbs.twimg.com/media/ABC?format=jpg"),
            "https://pbs.twimg.com/media/ABC?format=jpg&name=medium"
        );
        assert_eq!(
            card_image_url("https://pbs.twimg.com/media/ABC.jpg?name=small"),
            "https://pbs.twimg.com/media/ABC.jpg?name=small"
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
            filename_for("https://video.twimg.com/amplify_video/1/vid/720x1280/xyz.mp4", "mp4"),
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
            Some("https://gifconvert.vxtwitter.com/convert.avif?url=https%3A%2F%2Fvideo.twimg.com%2Ftweet_video%2FHTEtNFcXEAAY4Dv.mp4")
        );
        // Sorgu parametreli varyantlar da desteklenir (parametreler temizlenir)
        assert_eq!(
            gif_convert_url("https://video.twimg.com/tweet_video/abc.mp4?tag=12").as_deref(),
            Some("https://gifconvert.vxtwitter.com/convert.avif?url=https%3A%2F%2Fvideo.twimg.com%2Ftweet_video%2Fabc.mp4")
        );
        // Doğrudan gifconvert.vxtwitter.com adresi idempotent olarak döner
        assert_eq!(
            gif_convert_url("https://gifconvert.vxtwitter.com/convert.avif?url=https%3A%2F%2Fvideo.twimg.com%2Ftweet_video%2Fx.mp4").as_deref(),
            Some("https://gifconvert.vxtwitter.com/convert.avif?url=https%3A%2F%2Fvideo.twimg.com%2Ftweet_video%2Fx.mp4")
        );
        // Gerçek videolar (amplify/ext_tw_video) GIF servisine gönderilmez
        assert!(gif_convert_url("https://video.twimg.com/amplify_video/123/vid/720/x.mp4").is_none());
        assert!(gif_convert_url("https://cdn.example/video.mp4").is_none());
    }

    #[test]
    fn animated_image_is_detected_from_mime_or_magic_bytes() {
        let gif = MediaFile {
            bytes: b"GIF89a....".to_vec(),
            filename: "x.gif".to_string(),
            mime: "application/octet-stream".to_string(),
        };
        assert!(is_animated_image(&gif));

        let avif = MediaFile {
            bytes: vec![0, 0, 0, 28, b'f', b't', b'y', b'p'],
            filename: "convert.avif".to_string(),
            mime: "image/avif".to_string(),
        };
        assert!(is_animated_image(&avif));

        let mp4 = MediaFile {
            bytes: vec![0, 0, 0, 24, b'f', b't', b'y', b'p'],
            filename: "x.mp4".to_string(),
            mime: "video/mp4".to_string(),
        };
        assert!(!is_animated_image(&mp4));

        assert_eq!(replace_extension("HTEtNFcXEAAY4Dv.mp4", "gif"), "HTEtNFcXEAAY4Dv.gif");
        assert_eq!(replace_extension("noext", "gif"), "noext.gif");
    }

    #[test]
    fn spoiler_files_are_prefixed() {
        let file = MediaFile {
            bytes: vec![1, 2, 3],
            filename: "photo.jpg".to_string(),
            mime: "image/jpeg".to_string(),
        };
        assert!(to_attachment(file, true).filename.starts_with("SPOILER_"));
    }

    #[tokio::test]
    async fn gif_conversion_returns_nothing_for_non_video_input() {
        // ffmpeg kurulu değilse de, girdi video değilse de dönüşüm yapılmaz:
        // çağıran taraf mp4'e düşer (mesaj gönderimi bozulmaz).
        assert!(to_gif(b"bu bir video degil").await.is_none());
    }
}
