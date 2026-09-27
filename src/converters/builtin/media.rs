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

/// Twitter görsellerine boyut parametresi ekler (`?name=large`).
pub fn with_size(url: &str, size: &str) -> String {
    if !url.contains("pbs.twimg.com") || url.contains('?') {
        return url.to_string();
    }
    format!("{url}?name={size}")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twitter_sizes_are_applied_only_when_missing() {
        assert_eq!(
            with_size("https://pbs.twimg.com/media/ABC.jpg", "large"),
            "https://pbs.twimg.com/media/ABC.jpg?name=large"
        );
        assert_eq!(
            with_size("https://pbs.twimg.com/media/ABC.jpg?name=orig", "large"),
            "https://pbs.twimg.com/media/ABC.jpg?name=orig"
        );
        assert_eq!(
            with_size("https://cdn.bsky.app/img/x.jpg", "large"),
            "https://cdn.bsky.app/img/x.jpg"
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
    fn spoiler_files_are_prefixed() {
        let file = MediaFile {
            bytes: vec![1, 2, 3],
            filename: "photo.jpg".to_string(),
            mime: "image/jpeg".to_string(),
        };
        assert!(to_attachment(file, true).filename.starts_with("SPOILER_"));
    }
}
