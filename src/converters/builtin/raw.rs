//! Ham Discord REST gönderimi.
//!
//! Components V2 bileşenleri (Container / Media Gallery) serenity 0.12'nin tip güvenli
//! builder'larıyla üretilemediği için gövde `serde_json` ile kurulur ve doğrudan
//! Discord REST uç noktasına gönderilir. Dosya yüklemeleri için `multipart/form-data`
//! gövdesi elle oluşturulur (ek crate bağımlılığı yok).

use std::borrow::Cow;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serenity::all::{ChannelId, CreateAttachment, MessageId, WebhookId};

use super::DISCORD_USER_AGENT;
pub use super::transport::Error as DiscordError;
#[cfg(test)]
use super::transport::retry_after as extract_retry_after;

/// Discord REST tabanı (API v10).
pub const API_BASE: &str = "https://discord.com/api/v10";
/// Medya yüklemesi içeren istekler için zaman aşımı.
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(90);
/// Hata gövdesinden log'a yazılacak en fazla karakter.
#[cfg(test)]
const MAX_ERROR_BYTES: usize = 600;
static BOUNDARY_COUNTER: AtomicU64 = AtomicU64::new(0);

fn message_request(
    client: &reqwest::Client,
    url: &str,
    payload: &serde_json::Value,
    files: &[CreateAttachment],
) -> Result<reqwest::RequestBuilder, DiscordError> {
    message_request_with(reqwest::Method::POST, client, url, payload, files)
}

fn message_request_with(
    method: reqwest::Method,
    client: &reqwest::Client,
    url: &str,
    payload: &serde_json::Value,
    files: &[CreateAttachment],
) -> Result<reqwest::RequestBuilder, DiscordError> {
    let json = serde_json::to_string(payload).map_err(|_| DiscordError::InvalidPayload)?;
    let request = client
        .request(method, url)
        .header("user-agent", DISCORD_USER_AGENT)
        .timeout(UPLOAD_TIMEOUT);
    Ok(if files.is_empty() {
        request
            .header("content-type", "application/json")
            .body(json)
    } else {
        // Build once; Request::try_clone shares reqwest's immutable byte body on 429 retries.
        let (boundary, body) = build_multipart(&json, files);
        request
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(body)
    })
}

/// Kanala mesaj gönderir ve oluşan mesajın kimliğini döner.
pub async fn send_channel_message(
    client: &reqwest::Client,
    token: &str,
    channel_id: ChannelId,
    payload: &serde_json::Value,
    files: &[CreateAttachment],
) -> Result<MessageId, DiscordError> {
    let url = format!("{API_BASE}/channels/{}/messages", channel_id.get());
    // `username`/`avatar_url` yalnızca webhook alanlarıdır: kanal mesajında gönderilmez.
    let payload = payload_without_fields(payload, &["username", "avatar_url"]);
    let request = message_request(client, &url, &payload, files)?
        .header("authorization", format!("Bot {token}"));
    let body = super::transport::send(client, request, format!("channel:{channel_id}")).await?;
    parse_message_id(&body).map_err(|_| DiscordError::Ambiguous)
}

/// Etkileşimin **özgün yanıtını** (defer edilmiş "düşünüyor" mesajını) kartla değiştirir.
///
/// DM/grup DM'de bot kanal üyesi olmadığı için normal kanal mesajı gönderemez; user-install
/// etkileşimleri yalnızca interaction token'ı üzerinden yanıtlayabilir. Uç nokta
/// `PATCH /webhooks/{application_id}/{token}/messages/@original` şeklindedir; kimlik
/// doğrulaması URL'deki token ile yapılır, ayrıca bot yetkilendirmesi gerekmez.
pub async fn edit_interaction_response(
    client: &reqwest::Client,
    application_id: u64,
    token: &str,
    payload: &serde_json::Value,
    files: &[CreateAttachment],
) -> Result<MessageId, DiscordError> {
    let url = format!("{API_BASE}/webhooks/{application_id}/{token}/messages/@original");
    // Etkileşim yanıtı webhook kimliği ve mesaj referansı taşımaz.
    let payload = payload_without_fields(payload, &["username", "avatar_url", "message_reference"]);
    let request = message_request_with(reqwest::Method::PATCH, client, &url, &payload, files)?;
    let body =
        super::transport::send(client, request, format!("interaction:{application_id}")).await?;
    // Düzenleme başarılıysa gövde mesaj nesnesidir; boş gövde de kabul edilir.
    match parse_message_id(&body) {
        Ok(id) => Ok(id),
        Err(_) if body.trim().is_empty() => Ok(MessageId::new(0)),
        Err(_) => Err(DiscordError::Ambiguous),
    }
}

/// Webhook çalıştırma adresi.
///
/// `wait=true`: Discord oluşan mesajı yanıt olarak döner (mesaj kimliği 🗑️ silme kaydı için
/// gerekir). `thread_id`: mesaj **alt başlığa** (veya forum gönderisine) gönderilir; Discord
/// alt başlıklarda webhook oluşturmayı desteklemediği için webhook üst kanalda tutulur ve
/// hedef bu parametreyle belirtilir.
pub fn execute_webhook_url(
    webhook_id: WebhookId,
    token: &str,
    thread_id: Option<ChannelId>,
) -> String {
    match thread_id {
        Some(thread) => format!(
            "{API_BASE}/webhooks/{}/{}?wait=true&thread_id={}",
            webhook_id.get(),
            token,
            thread.get()
        ),
        None => format!(
            "{API_BASE}/webhooks/{}/{}?wait=true",
            webhook_id.get(),
            token
        ),
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
) -> Result<Option<MessageId>, DiscordError> {
    let url = execute_webhook_url(webhook_id, token, thread_id);
    // Discord Execute Webhook uç noktası `message_reference` alanını desteklemez (400 Bad Request döner).
    let payload = payload_without_fields(payload, &["message_reference"]);
    let request = message_request(client, &url, &payload, files)?;
    let body = super::transport::send(client, request, format!("hook_exec:{webhook_id}")).await?;
    parse_message_id(&body)
        .map(Some)
        .map_err(|_| DiscordError::Ambiguous)
}

fn payload_without_fields<'a>(
    payload: &'a serde_json::Value,
    fields: &[&str],
) -> Cow<'a, serde_json::Value> {
    let Some(object) = payload.as_object() else {
        return Cow::Borrowed(payload);
    };
    if !fields.iter().any(|field| object.contains_key(*field)) {
        return Cow::Borrowed(payload);
    }

    let mut payload = payload.clone();
    if let Some(object) = payload.as_object_mut() {
        for field in fields {
            object.remove(*field);
        }
    }
    Cow::Owned(payload)
}

/// Kanalın webhook'larını listeler (Manage Webhooks izni gerekir).
pub async fn list_channel_webhooks(
    client: &reqwest::Client,
    bot_token: &str,
    channel_id: ChannelId,
) -> Result<Vec<serde_json::Value>, DiscordError> {
    let url = format!("{API_BASE}/channels/{}/webhooks", channel_id.get());

    let request = client
        .get(&url)
        .header("user-agent", DISCORD_USER_AGENT)
        .header("authorization", format!("Bot {bot_token}"))
        .timeout(UPLOAD_TIMEOUT);
    let body = super::transport::send(client, request, format!("hook_list:{channel_id}")).await?;
    serde_json::from_str(&body).map_err(|_| DiscordError::Unavailable)
}

/// Kanala yeni bir webhook ekler.
pub async fn create_channel_webhook(
    client: &reqwest::Client,
    bot_token: &str,
    channel_id: ChannelId,
    name: &str,
) -> Result<serde_json::Value, DiscordError> {
    let url = format!("{API_BASE}/channels/{}/webhooks", channel_id.get());

    let request = client
        .post(&url)
        .header("user-agent", DISCORD_USER_AGENT)
        .header("authorization", format!("Bot {bot_token}"))
        .json(&serde_json::json!({"name": name}))
        .timeout(UPLOAD_TIMEOUT);
    let body = super::transport::send(client, request, format!("hook_create:{channel_id}")).await?;
    serde_json::from_str(&body).map_err(|_| DiscordError::Ambiguous)
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
        .map(|c| {
            if matches!(c, '"' | '\\') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect()
}

/// Dosya adına göre medya türü (Discord video/görsel ayrımını uzantıdan yapar).
pub fn mime_for_filename(name: &str) -> &'static str {
    match name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
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
#[cfg(test)]
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
        assert!(
            plain.ends_with("/webhooks/42/tok?wait=true"),
            "adres: {plain}"
        );
        assert!(
            !plain.contains("thread_id"),
            "alt başlık yoksa parametre eklenmemeli: {plain}"
        );

        // Alt başlık/forum gönderisi: webhook üst kanalda, mesaj `thread_id` ile o gönderiye.
        let threaded = execute_webhook_url(webhook, "tok", Some(ChannelId::new(777)));
        assert!(
            threaded.contains("?wait=true&thread_id=777"),
            "adres: {threaded}"
        );
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
        assert!(
            text.contains("Content-Type: application/json"),
            "payload türü eksik"
        );
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
        let (boundary, body) =
            build_multipart("{}", &[CreateAttachment::bytes(vec![], "we\"ird.mp4")]);
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
        assert_eq!(
            extract_retry_after(&headers, "{}"),
            Duration::from_millis(1500)
        );

        let headers_empty = reqwest::header::HeaderMap::new();
        let body = r#"{"retry_after": 2.25}"#;
        assert_eq!(
            extract_retry_after(&headers_empty, body),
            Duration::from_millis(2250)
        );
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
    fn payload_field_filter_avoids_unneeded_clones() {
        let unchanged = serde_json::json!({"content": "hello"});
        let filtered = payload_without_fields(&unchanged, &["username", "avatar_url"]);
        assert!(matches!(&filtered, Cow::Borrowed(_)));
        assert_eq!(filtered.as_ref(), &unchanged);

        let webhook_payload = serde_json::json!({
            "content": "hello",
            "username": "webhook",
            "avatar_url": "https://example.test/avatar.png"
        });
        let filtered = payload_without_fields(&webhook_payload, &["username", "avatar_url"]);
        assert!(matches!(&filtered, Cow::Owned(_)));
        assert_eq!(filtered.as_ref(), &serde_json::json!({"content": "hello"}));
        assert_eq!(webhook_payload["username"], "webhook");
    }

    #[test]
    fn error_bodies_are_truncated_for_logs() {
        let long_body = "x".repeat(MAX_ERROR_BYTES * 2);
        let message = error_message(400, &long_body);
        assert!(message.starts_with("Discord 400 döndü: "));
        assert!(message.chars().count() < MAX_ERROR_BYTES + 40);
    }
}
