//! Botun ürettiği videoların Zipline'da barındırılması.
//!
//! Discord'a ek yüklemek yavaştır: gövde Cloudflare'e hat hızında gider ama Discord'un dosyayı
//! işleyip yanıt vermesi saniyeler sürer (ölçülen: 7,67 MB'ta gönderim 3,3 sn). Zipline aynı
//! makinede çalışır; yükleme yerel ağda ~0,3 sn sürer ve kart videoyu Zipline'ın herkese açık
//! adresinden oynatır (ölçülen gönderim 0,6 sn). Discord'un ek sınırı (20/50/100 MiB) yerine
//! [`MAX_BYTES`] geçerlidir. Dosya [`RETENTION`] sonra Zipline tarafından silinir; eski
//! kartlardaki video o zaman oynamaz.

use std::sync::OnceLock;
use std::time::Duration;

use serde_json::Value;

use super::media::MediaFile;
use crate::config::ZiplineConfig;
use crate::telemetry::{HttpHost, InFlight};

/// Barındırılan videonun üst sınırı.
pub const MAX_BYTES: u64 = 50 * 1024 * 1024;
/// Zipline'ın dosyayı sileceği süre (`x-zipline-deletes-at`).
pub const RETENTION: &str = "30d";
/// Yükleme süre sınırı (bağlantı kurma dahil). Yerel ağda 50 MB bundan kısa sürer; Zipline'a
/// ulaşılamazsa kart bu kadar bekleyip ek yoluna düşer.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(5);

static ZIPLINE: OnceLock<ZiplineConfig> = OnceLock::new();

/// Başlangıçta bir kez çağrılır; yapılandırma yoksa barındırma kapalı kalır.
pub fn init(config: Option<ZiplineConfig>) {
    if let Some(config) = config {
        let _ = ZIPLINE.set(config);
    }
}

/// Barındırmanın açık olduğu alan adı.
pub fn domain() -> Option<&'static str> {
    ZIPLINE.get().map(|config| config.domain.as_str())
}

/// Videoyu yükler ve herkese açık adresini döner.
pub async fn upload(client: &reqwest::Client, file: &MediaFile) -> Result<String, String> {
    let config = ZIPLINE.get().ok_or("Zipline yapılandırılmamış")?;
    let request = InFlight::start(HttpHost::Other);
    let result = send(client, config, file).await;
    match &result {
        Ok(_) => request.success(file.bytes.len()),
        Err(_) => request.error(),
    }
    result
}

async fn send(
    client: &reqwest::Client,
    config: &ZiplineConfig,
    file: &MediaFile,
) -> Result<String, String> {
    let endpoint = config
        .url
        .join("/api/upload")
        .map_err(|err| format!("Zipline adresi geçersiz: {err}"))?;
    let (boundary, body) = multipart(file);
    let response = client
        .post(endpoint)
        .header("authorization", &config.token)
        .header("x-zipline-deletes-at", RETENTION)
        // Kısa rastgele adlar tahmin edilebilir (ör. `M.mp4`); UUID edilemez.
        .header("x-zipline-format", "uuid")
        .header("x-zipline-domain", &config.domain)
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .timeout(UPLOAD_TIMEOUT)
        .body(body)
        .send()
        .await
        .map_err(|err| format!("Zipline yüklemesi başarısız: {err}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("Zipline yüklemeyi reddetti: {status}"));
    }
    let json: Value = response
        .json()
        .await
        .map_err(|err| format!("Zipline yanıtı çözülemedi: {err}"))?;
    public_url(&json, &config.domain).ok_or_else(|| "Zipline yanıtında adres yok".to_string())
}

/// Yanıttaki dosya adresi; yalnızca yapılandırılan alan adındaki HTTPS adresi kabul edilir
/// (kartta Discord'a verilen adres yanıttan gelir).
fn public_url(json: &Value, domain: &str) -> Option<String> {
    let url = json.pointer("/files/0/url")?.as_str()?;
    let parsed = url::Url::parse(url).ok()?;
    (parsed.scheme() == "https" && parsed.host_str() == Some(domain)).then(|| url.to_string())
}

/// Tek dosyalık `multipart/form-data` gövdesi (`file` alanı).
fn multipart(file: &MediaFile) -> (String, Vec<u8>) {
    let boundary = super::raw::boundary();
    let filename = file.filename.replace(['"', '\r', '\n'], "_");
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n\
         Content-Type: {}\r\n\r\n",
        file.mime
    )
    .into_bytes();
    body.extend_from_slice(&file.bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (boundary, body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_https_urls_on_the_configured_domain_are_accepted() {
        let reply = |url: &str| json!({"files": [{"id": "x", "url": url}]});
        assert_eq!(
            public_url(&reply("https://z.ornek.com/u/a.mp4"), "z.ornek.com").as_deref(),
            Some("https://z.ornek.com/u/a.mp4")
        );
        assert!(public_url(&reply("http://z.ornek.com/u/a.mp4"), "z.ornek.com").is_none());
        assert!(public_url(&reply("https://evil.test/a.mp4"), "z.ornek.com").is_none());
        assert!(public_url(&json!({"files": []}), "z.ornek.com").is_none());
    }

    #[test]
    fn multipart_body_wraps_the_file() {
        let file = MediaFile {
            bytes: b"mp4".to_vec(),
            filename: "a\"b.mp4".to_string(),
            mime: "video/mp4".to_string(),
        };
        let (boundary, body) = multipart(&file);
        let body = String::from_utf8(body).unwrap();
        assert!(body.starts_with(&format!("--{boundary}\r\n")));
        assert!(body.contains("filename=\"a_b.mp4\""));
        assert!(body.contains("Content-Type: video/mp4\r\n\r\nmp4\r\n"));
        assert!(body.ends_with(&format!("--{boundary}--\r\n")));
    }
}
