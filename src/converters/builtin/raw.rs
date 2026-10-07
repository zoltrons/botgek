//! Ham Discord REST gönderimi.
//!
//! Components V2 bileşenleri (Container / Media Gallery) serenity 0.12'nin tip güvenli
//! builder'larıyla üretilemediği için gövde `serde_json` ile kurulur ve doğrudan
//! Discord REST uç noktasına gönderilir. Dosya yüklemeleri için `multipart/form-data`
//! gövdesi elle oluşturulur (ek crate bağımlılığı yok).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serenity::all::{ChannelId, CreateAttachment, MessageId, WebhookId};

use super::DISCORD_USER_AGENT;

/// Discord REST tabanı (API v10).
pub const API_BASE: &str = "https://discord.com/api/v10";
/// Medya yüklemesi içeren istekler için zaman aşımı.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(90);
/// Hata gövdesinden log'a yazılacak en fazla karakter.
const MAX_ERROR_BYTES: usize = 600;
static BOUNDARY_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Discord 429 (Rate Limit) yanıtından bekleme süresini (Retry-After) okur.
pub fn extract_retry_after(headers: &reqwest::header::HeaderMap, body: &str) -> Duration {
    if let Some(header_val) = headers.get("retry-after")
        && let Ok(s) = header_val.to_str()
            && let Ok(secs) = s.parse::<f64>() {
                return Duration::from_secs_f64(secs.clamp(0.05, 5.0));
            }
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(body)
        && let Some(secs) = val.get("retry_after").and_then(|v| v.as_f64()) {
            return Duration::from_secs_f64(secs.clamp(0.05, 5.0));
        }
    Duration::from_millis(500)
}

/// Kanala mesaj gönderir ve oluşan mesajın kimliğini döner.
pub async fn send_channel_message(
    client: &reqwest::Client,
    token: &str,
    channel_id: ChannelId,
    payload: &serde_json::Value,
    files: &[CreateAttachment],
) -> Result<MessageId, String> {
    let url = format!("{API_BASE}/channels/{}/messages", channel_id.get());
    let mut payload = payload.clone();
    // `username`/`avatar_url` yalnızca webhook alanlarıdır: kanal mesajında gönderilmez.
    if let Some(object) = payload.as_object_mut() {
        object.remove("username");
        object.remove("avatar_url");
    }
    let json = serde_json::to_string(&payload).map_err(|e| format!("Gövde serileştirilemedi: {e}"))?;

    for attempt in 0..3 {
        let mut request = client.post(&url).header("user-agent", DISCORD_USER_AGENT).timeout(UPLOAD_TIMEOUT);
        request = if files.is_empty() {
            request.header("content-type", "application/json").body(json.clone())
        } else {
            let (boundary, body) = build_multipart(&json, files);
            request
                .header("content-type", format!("multipart/form-data; boundary={boundary}"))
                .body(body)
        };

        let response = request
            .header("authorization", format!("Bot {token}"))
            .send()
            .await
            .map_err(|e| format!("İstek gönderilemedi: {e}"))?;

        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let body = response.text().await.unwrap_or_default();

        if status == 429 && attempt < 2 {
            let wait = extract_retry_after(&headers, &body);
            tracing::warn!("Discord REST 429 (Rate Limit), {wait:?} bekleniyor (deneme {attempt})");
            tokio::time::sleep(wait).await;
            continue;
        }

        if !(200..300).contains(&status) {
            return Err(error_message(status, &body));
        }
        return parse_message_id(&body);
    }
    Err("Discord REST: 429 deneme sayısı aşıldı".to_string())
}

/// Webhook çalıştırma adresi.
///
/// `wait=true`: Discord oluşan mesajı yanıt olarak döner (mesaj kimliği 🗑️ silme kaydı için
/// gerekir). `thread_id`: mesaj **alt başlığa** (veya forum gönderisine) gönderilir; Discord
/// alt başlıklarda webhook oluşturmayı desteklemediği için webhook üst kanalda tutulur ve
/// hedef bu parametreyle belirtilir.
pub fn execute_webhook_url(webhook_id: WebhookId, token: &str, thread_id: Option<ChannelId>) -> String {
    match thread_id {
        Some(thread) => format!(
            "{API_BASE}/webhooks/{}/{}?wait=true&thread_id={}",
            webhook_id.get(),
            token,
            thread.get()
        ),
        None => format!("{API_BASE}/webhooks/{}/{}?wait=true", webhook_id.get(), token),
    }
}

/// Webhook'u çalıştırır (`wait=true`) ve oluşan mesajın kimliğini döner.
/// `thread_id` doluysa mesaj o alt başlığa/forum gönderisine gönderilir.
pub async fn execute_webhook(
    client: &reqwest::Client,
    webhook_id: WebhookId,
    token: &str,
    thread_id: Option<ChannelId>,
    payload: &serde_json::Value,
    files: &[CreateAttachment],
) -> Result<Option<MessageId>, String> {
    let url = execute_webhook_url(webhook_id, token, thread_id);
    let mut payload = payload.clone();
    // Discord Execute Webhook uç noktası `message_reference` alanını desteklemez (400 Bad Request döner).
    if let Some(object) = payload.as_object_mut() {
        object.remove("message_reference");
    }
    let json = serde_json::to_string(&payload).map_err(|e| format!("Gövde serileştirilemedi: {e}"))?;

    for attempt in 0..3 {
        let mut request = client.post(&url).header("user-agent", DISCORD_USER_AGENT).timeout(UPLOAD_TIMEOUT);
        request = if files.is_empty() {
            request.header("content-type", "application/json").body(json.clone())
        } else {
            let (boundary, body) = build_multipart(&json, files);
            request
                .header("content-type", format!("multipart/form-data; boundary={boundary}"))
                .body(body)
        };

        let response = request.send().await.map_err(|e| format!("İstek gönderilemedi: {e}"))?;

        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let body = response.text().await.unwrap_or_default();

        if status == 429 && attempt < 2 {
            let wait = extract_retry_after(&headers, &body);
            tracing::warn!("Discord REST 429 (Rate Limit), {wait:?} bekleniyor (deneme {attempt})");
            tokio::time::sleep(wait).await;
            continue;
        }

        if !(200..300).contains(&status) {
            return Err(error_message(status, &body));
        }
        return Ok(parse_message_id(&body).ok());
    }
    Err("Discord REST: 429 deneme sayısı aşıldı".to_string())
}

/// Kanalın webhook'larını listeler (Manage Webhooks izni gerekir).
pub async fn list_channel_webhooks(
    client: &reqwest::Client,
    bot_token: &str,
    channel_id: ChannelId,
) -> Result<Vec<serde_json::Value>, String> {
    let url = format!("{API_BASE}/channels/{}/webhooks", channel_id.get());

    for attempt in 0..3 {
        let response = client
            .get(&url)
            .header("user-agent", DISCORD_USER_AGENT)
            .header("authorization", format!("Bot {bot_token}"))
            .timeout(UPLOAD_TIMEOUT)
            .send()
            .await
            .map_err(|e| format!("Webhook listesi istenemedi: {e}"))?;

        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let body = response.text().await.unwrap_or_default();

        if status == 429 && attempt < 2 {
            let wait = extract_retry_after(&headers, &body);
            tracing::warn!("Discord REST 429 (Rate Limit), {wait:?} bekleniyor (deneme {attempt})");
            tokio::time::sleep(wait).await;
            continue;
        }

        if !(200..300).contains(&status) {
            return Err(error_message(status, &body));
        }

        let value: serde_json::Value =
            serde_json::from_str(&body).map_err(|e| format!("Webhook listesi çözümlenemedi: {e}"))?;
        return Ok(value.as_array().cloned().unwrap_or_default());
    }
    Err("Discord REST: 429 deneme sayısı aşıldı".to_string())
}

/// Kanala yeni bir webhook ekler.
pub async fn create_channel_webhook(
    client: &reqwest::Client,
    bot_token: &str,
    channel_id: ChannelId,
    name: &str,
) -> Result<serde_json::Value, String> {
    let url = format!("{API_BASE}/channels/{}/webhooks", channel_id.get());

    for attempt in 0..3 {
        let response = client
            .post(&url)
            .header("user-agent", DISCORD_USER_AGENT)
            .header("authorization", format!("Bot {bot_token}"))
            .json(&serde_json::json!({"name": name}))
            .timeout(UPLOAD_TIMEOUT)
            .send()
            .await
            .map_err(|e| format!("Webhook oluşturma isteği başarısız: {e}"))?;

        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let body = response.text().await.unwrap_or_default();

        if status == 429 && attempt < 2 {
            let wait = extract_retry_after(&headers, &body);
            tracing::warn!("Discord REST 429 (Rate Limit), {wait:?} bekleniyor (deneme {attempt})");
            tokio::time::sleep(wait).await;
            continue;
        }

        if !(200..300).contains(&status) {
            return Err(error_message(status, &body));
        }
        return serde_json::from_str(&body).map_err(|e| format!("Webhook yanıtı çözümlenemedi: {e}"));
    }
    Err("Discord REST: 429 deneme sayısı aşıldı".to_string())
}

/// Webhook JSON'undan kimlik ve token'ı çıkarır.
pub fn webhook_credentials(value: &serde_json::Value) -> Option<(WebhookId, String)> {
    let id = value.get("id")?.as_str()?.parse::<u64>().ok()?;
    let token = value.get("token")?.as_str()?.to_string();
    Some((WebhookId::new(id), token))
}

/// `multipart/form-data` gövdesini üretir: `payload_json` + `files[n]` parçaları.
pub fn build_multipart(payload_json: &str, files: &[CreateAttachment]) -> (String, Vec<u8>) {
    let boundary = boundary();
    let mut body: Vec<u8> = Vec::with_capacity(payload_json.len() + 512);

    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"payload_json\"\r\n\
             Content-Type: application/json\r\n\r\n{payload_json}\r\n"
        )
        .as_bytes(),
    );

    for (index, file) in files.iter().enumerate() {
        let filename = escape_filename(&file.filename);
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"files[{index}]\"; \
                 filename=\"{filename}\"\r\nContent-Type: {}\r\n\r\n",
                mime_for_filename(&file.filename)
            )
            .as_bytes(),
        );
        body.extend_from_slice(&file.data);
        body.extend_from_slice(b"\r\n");
    }

    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (boundary, body)
}

/// Her istek için benzersiz bir multipart sınırı üretir.
fn boundary() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let counter = BOUNDARY_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("botgek-{nanos:x}-{counter:x}")
}

/// Dosya adını `Content-Disposition` başlığına güvenle yazar.
fn escape_filename(name: &str) -> String {
    name.chars()
        .map(|c| if matches!(c, '"' | '\\') || c.is_control() { '_' } else { c })
        .collect()
}

/// Dosya adına göre medya türü (Discord video/görsel ayrımını uzantıdan yapar).
pub fn mime_for_filename(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "mp4" => "video/mp4",
        "m4v" => "video/x-m4v",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}

/// Yanıt gövdesinden mesaj kimliğini çıkarır.
pub fn parse_message_id(body: &str) -> Result<MessageId, String> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| format!("Yanıt çözümlenemedi: {e}"))?;
    value
        .get("id")
        .and_then(|id| id.as_str())
        .and_then(|id| id.parse::<u64>().ok())
        .map(MessageId::new)
        .ok_or_else(|| "Yanıtta mesaj kimliği yok".to_string())
}

/// Discord hata gövdesini log'lanabilir tek satıra çevirir.
fn error_message(status: u16, body: &str) -> String {
    let trimmed: String = body.chars().take(MAX_ERROR_BYTES).collect();
    format!("Discord {} döndü: {}", status, trimmed.replace('\n', " "))
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webhook_url_targets_threads_with_thread_id() {
        let webhook = WebhookId::new(42);

        let plain = execute_webhook_url(webhook, "tok", None);
        assert!(plain.ends_with("/webhooks/42/tok?wait=true"), "adres: {plain}");
        assert!(
            !plain.contains("thread_id"),
            "alt başlık yoksa parametre eklenmemeli: {plain}"
        );

        // Alt başlık/forum gönderisi: webhook üst kanalda, mesaj `thread_id` ile o gönderiye.
        let threaded = execute_webhook_url(webhook, "tok", Some(ChannelId::new(777)));
        assert!(threaded.contains("?wait=true&thread_id=777"), "adres: {threaded}");
    }

    #[test]
    fn multipart_body_carries_payload_and_files() {
        let payload = r#"{"flags":32768}"#;
        let files = vec![
            CreateAttachment::bytes(vec![1, 2, 3], "video.mp4"),
            CreateAttachment::bytes(vec![9], "photo.jpg"),
        ];

        let (boundary, body) = build_multipart(payload, &files);
        let text = String::from_utf8_lossy(&body);

        assert!(text.contains(&format!("--{boundary}\r\n")));
        assert!(text.contains("Content-Disposition: form-data; name=\"payload_json\""));
        assert!(text.contains("Content-Type: application/json"), "payload türü eksik");
        assert!(text.contains(payload));
        assert!(text.contains("name=\"files[0]\"; filename=\"video.mp4\""));
        assert!(text.contains("Content-Type: video/mp4"));
        assert!(text.contains("name=\"files[1]\"; filename=\"photo.jpg\""));
        assert!(text.contains("Content-Type: image/jpeg"));
        assert!(text.trim_end().ends_with(&format!("--{boundary}--")));
        // Dosya baytları gövdeye ham olarak eklenir
        assert!(body.windows(3).any(|window| window == [1, 2, 3]));
    }

    #[test]
    fn multipart_boundaries_are_unique() {
        let (first, _) = build_multipart("{}", &[]);
        let (second, _) = build_multipart("{}", &[]);
        assert_ne!(first, second);
    }

    #[test]
    fn attachment_filenames_are_escaped() {
        assert_eq!(escape_filename("a\"b\\c.mp4"), "a_b_c.mp4");
        let (boundary, body) = build_multipart("{}", &[CreateAttachment::bytes(vec![], "we\"ird.mp4")]);
        let text = String::from_utf8_lossy(&body);
        assert!(text.contains(&format!("--{boundary}")));
        assert!(text.contains("filename=\"we_ird.mp4\""));
    }

    #[test]
    fn media_types_follow_the_extension() {
        assert_eq!(mime_for_filename("SPOILER_video.mp4"), "video/mp4");
        assert_eq!(mime_for_filename("photo.JPG"), "image/jpeg");
        assert_eq!(mime_for_filename("clip.webm"), "video/webm");
        assert_eq!(mime_for_filename("bilinmeyen"), "application/octet-stream");
    }

    #[test]
    fn retry_after_is_extracted_from_header_and_body() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("retry-after", "1.5".parse().unwrap());
        assert_eq!(extract_retry_after(&headers, "{}"), Duration::from_millis(1500));

        let headers_empty = reqwest::header::HeaderMap::new();
        let body = r#"{"retry_after": 2.25}"#;
        assert_eq!(extract_retry_after(&headers_empty, body), Duration::from_millis(2250));
    }

    #[test]
    fn message_ids_are_parsed_from_the_response() {
        assert_eq!(
            parse_message_id(r#"{"id":"123456789012345678","content":""}"#).unwrap(),
            MessageId::new(123_456_789_012_345_678)
        );
        assert!(parse_message_id("{}").is_err());
        assert!(parse_message_id("gecersiz").is_err());
    }

    #[test]
    fn error_bodies_are_truncated_for_logs() {
        let long_body = "x".repeat(MAX_ERROR_BYTES * 2);
        let message = error_message(400, &long_body);
        assert!(message.starts_with("Discord 400 döndü: "));
        assert!(message.chars().count() < MAX_ERROR_BYTES + 40);
    }
}

