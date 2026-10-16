use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tracing::{error, info};

use crate::utils::truncate_chars;

pub const DEFAULT_MODEL: &str = "google/gemma-4-31b-it:free";

/// Hard limits for an individual provider call. These are deliberately kept
/// here, next to the request construction, so callers cannot accidentally
/// bypass them by using a different prompt path.
pub const MAX_PROMPT_CHARS: usize = 8_000;
pub const MAX_RESPONSE_BYTES: usize = 1_048_576;
pub const MAX_REPLY_CHARS: usize = 8_000;
const MAX_MODEL_CHARS: usize = 256;
const MAX_IN_FLIGHT_REQUESTS: usize = 2;
/// İstek başına üst sınır: birincil + yedek çağrı birlikte iki yuvadan birini en fazla ~40 sn tutar (genel istemci zaman aşımı 30 sn idi, iki çağrıda 60 sn).
const PROVIDER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// Do not queue an unbounded number of provider calls. The handler already has
/// a per-user cooldown; this guard protects the whole process from a burst of
/// users or from a slow provider holding tasks indefinitely.
static AI_IN_FLIGHT: std::sync::LazyLock<Arc<Semaphore>> =
    std::sync::LazyLock::new(|| Arc::new(Semaphore::new(MAX_IN_FLIGHT_REQUESTS)));

/// Gövdeyi log/ hata metni için sınırlı tutar: ham yanıt logları şişirebilir ve
/// istem içeriğini geri yansıtabilir.
fn preview(body: &str) -> String {
    const MAX_CHARS: usize = 400;
    if body.chars().count() <= MAX_CHARS {
        body.to_string()
    } else {
        format!(
            "{} ({} karaktere kırpıldı)",
            truncate_chars(body, MAX_CHARS),
            body.chars().count()
        )
    }
}

fn bounded_prompt(prompt: &str, label: &str) -> String {
    let trimmed = prompt.trim();
    if trimmed.chars().count() <= MAX_PROMPT_CHARS {
        trimmed.to_string()
    } else {
        tracing::warn!(
            "AI {label} istemi {} karakterden {} karaktere kırpıldı",
            trimmed.chars().count(),
            MAX_PROMPT_CHARS
        );
        truncate_chars(trimmed, MAX_PROMPT_CHARS)
    }
}

async fn read_bounded_body(mut response: reqwest::Response) -> Result<String, String> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(format!(
            "OpenRouter yanıtı boyut sınırını aşıyor (en fazla {} bayt)",
            MAX_RESPONSE_BYTES
        ));
    }

    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("Yanıt gövdesi okunamadı: {:?}", e))?
    {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(format!(
                "OpenRouter yanıtı boyut sınırını aşıyor (en fazla {} bayt)",
                MAX_RESPONSE_BYTES
            ));
        }
        body.extend_from_slice(&chunk);
    }

    String::from_utf8(body).map_err(|_| "Yanıt gövdesi UTF-8 değil".to_string())
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct ChatCompletionRequest<'a> {
    model: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    models: Option<Vec<&'a str>>,
    messages: Vec<ChatMessage<'a>>,
    max_tokens: u32,
    temperature: f32,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
    error: Option<OpenRouterError>,
}

#[derive(Deserialize)]
struct ChatResponseMessage {
    #[serde(default)]
    content: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct ChatCompletionResponse {
    choices: Option<Vec<ChatChoice>>,
    error: Option<OpenRouterError>,
}

#[derive(Deserialize, Debug)]
struct OpenRouterError {
    message: String,
}

/// WebUI / Cursor tarzı slug'ları OpenRouter'ın beklediği `sağlayıcı/model` biçimine çevirir.
pub fn normalize_openrouter_model(raw: &str) -> String {
    let mut model = raw
        .trim()
        .trim_matches(|c| c == '"' || c == '\'')
        .to_string();
    if model.is_empty() {
        return DEFAULT_MODEL.to_string();
    }

    if let Some(rest) = model.strip_prefix("openrouter:") {
        model = if rest.contains('/') {
            rest.to_string()
        } else {
            format!("openrouter/{rest}")
        };
    }

    if let Some(rest) = model.strip_prefix("openrouter/")
        && rest.contains('/')
    {
        model = rest.to_string();
    }

    if model.is_empty() {
        DEFAULT_MODEL.to_string()
    } else {
        model
    }
}

fn json_to_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
        serde_json::Value::Array(parts) => {
            let mut out = String::new();
            for part in parts {
                match part {
                    serde_json::Value::String(s) => out.push_str(s),
                    serde_json::Value::Object(obj) => {
                        if let Some(serde_json::Value::String(s)) = obj.get("text") {
                            out.push_str(s);
                        }
                    }
                    _ => {}
                }
            }
            let t = out.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
        serde_json::Value::Object(obj) => obj.get("text").and_then(json_to_text),
        _ => None,
    }
}

fn choice_text(choice: &ChatChoice) -> Option<String> {
    if let Some(ref err) = choice.error {
        tracing::warn!("OpenRouter choice-level hata: {}", err.message);
        return None;
    }
    choice.message.content.as_ref().and_then(json_to_text)
}

fn extract_reply(body_text: &str) -> Result<String, String> {
    let parsed: ChatCompletionResponse = serde_json::from_str(body_text).map_err(|e| {
        format!(
            "OpenRouter JSON ayrıştırma hatası: {:?} (Gövde: {})",
            e,
            preview(body_text)
        )
    })?;

    if let Some(err) = parsed.error {
        return Err(format!("OpenRouter hata mesajı: {}", err.message));
    }

    if let Some(choices) = parsed.choices
        && let Some(choice) = choices.into_iter().next()
        && let Some(content) = choice_text(&choice)
    {
        return Ok(truncate_chars(&content, MAX_REPLY_CHARS));
    }

    Err("OpenRouter'dan boş yanıt döndü.".to_string())
}

pub const AUTO_FALLBACK_MODEL: &str = "openrouter/free";

/// Boş sağlayıcı yuvası var mı? Çağıran taraf kullanıcı/sunucu bütçesini harcamadan önce sorar; dolu yuvaya giden istek bütçeyi boşa tüketip hata bildirimi gönderiyordu.
pub fn has_capacity() -> bool {
    AI_IN_FLIGHT.available_permits() > 0
}

/// OpenRouter API'si üzerinden kullanıcı mesajına laf sokucu / esprili yanıt üretir.
pub async fn query_openrouter(
    client: &reqwest::Client,
    api_key: &str,
    model: &str,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<String, String> {
    let _permit = AI_IN_FLIGHT
        .clone()
        .try_acquire_owned()
        .map_err(|_| "AI istek sınırı dolu; lütfen biraz sonra tekrar deneyin.".to_string())?;

    let url = "https://openrouter.ai/api/v1/chat/completions";
    let effective_model = normalize_openrouter_model(model);
    if effective_model.chars().count() > MAX_MODEL_CHARS {
        return Err(format!(
            "OpenRouter model adı çok uzun (en fazla {} karakter)",
            MAX_MODEL_CHARS
        ));
    }

    // OpenRouter otomatik geçiş (failover) zinciri:
    // Eğer seçilen model rate-limit veya 504 alırsa, OpenRouter sonraki modeli anında dener.
    let mut model_chain = vec![effective_model.as_str()];
    if effective_model != "google/gemma-4-26b-a4b-it:free" {
        model_chain.push("google/gemma-4-26b-a4b-it:free");
    }
    if effective_model != AUTO_FALLBACK_MODEL {
        model_chain.push(AUTO_FALLBACK_MODEL);
    }

    let safety_guardrail = "\n\n[KRİTİK GÜVENLİK TALİMATI]: Yanıtlarında ASLA '@everyone' veya '@here' etiketlerini KULLANMA. Kullanıcıları veya rolleri etiketleme. Bu kural kesindir ve çiğnenemez.";
    let full_system_prompt = format!(
        "{}{}",
        bounded_prompt(system_prompt, "system"),
        safety_guardrail
    );
    let bounded_user_prompt = bounded_prompt(user_prompt, "kullanıcı");

    let payload = ChatCompletionRequest {
        model: &effective_model,
        models: Some(model_chain),
        messages: vec![
            ChatMessage {
                role: "system",
                content: &full_system_prompt,
            },
            ChatMessage {
                role: "user",
                content: &bounded_user_prompt,
            },
        ],
        max_tokens: 500,
        temperature: 0.9,
    };

    info!(
        "OpenRouter isteği gönderiliyor (Model: {})...",
        effective_model
    );

    let res = client
        .post(url)
        .header("Authorization", format!("Bearer {}", api_key.trim()))
        .header("HTTP-Referer", "https://github.com/botgek")
        .header("X-Title", "Botgek Discord Bot")
        .json(&payload)
        .timeout(PROVIDER_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("OpenRouter HTTP bağlantı hatası: {:?}", e))?;

    let status = res.status();
    let body_text = read_bounded_body(res).await?;

    if status.is_success() {
        match extract_reply(&body_text) {
            Ok(content) => return Ok(content),
            Err(e) if effective_model == AUTO_FALLBACK_MODEL => return Err(e),
            Err(e) => {
                tracing::warn!(
                    "Model '{}' yanıtı kullanılamadı ({}), '{}' deneniyor...",
                    effective_model,
                    e,
                    AUTO_FALLBACK_MODEL
                );
            }
        }
    } else {
        error!(
            "OpenRouter API hata döndürdü (HTTP {}): {}",
            status,
            preview(&body_text)
        );
        // İstek/kimlik/kredi hataları (400/401/402/403) yedek modelde de aynen tekrar eder.
        let retryable =
            status.as_u16() == 408 || status.as_u16() == 429 || status.is_server_error();
        if effective_model == AUTO_FALLBACK_MODEL || !retryable {
            return Err(format!(
                "OpenRouter API hatası ({}): {}",
                status,
                preview(&body_text)
            ));
        }
        tracing::warn!(
            "Model '{}' başarısız oldu (HTTP {}), '{}' ile yeniden deneniyor...",
            effective_model,
            status,
            AUTO_FALLBACK_MODEL
        );
    }

    // İstemci tarafı ikinci savunma hattı: Doğrudan OpenRouter Free Router ile tekrar dene
    let fallback_payload = ChatCompletionRequest {
        model: AUTO_FALLBACK_MODEL,
        models: None,
        messages: vec![
            ChatMessage {
                role: "system",
                content: &full_system_prompt,
            },
            ChatMessage {
                role: "user",
                content: &bounded_user_prompt,
            },
        ],
        max_tokens: 500,
        temperature: 0.9,
    };

    let fb_res = client
        .post(url)
        .header("Authorization", format!("Bearer {}", api_key.trim()))
        .header("HTTP-Referer", "https://github.com/botgek")
        .header("X-Title", "Botgek Discord Bot")
        .json(&fallback_payload)
        .timeout(PROVIDER_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("OpenRouter yedek HTTP bağlantı hatası: {:?}", e))?;

    let fb_status = fb_res.status();
    let fb_body = read_bounded_body(fb_res).await?;

    if !fb_status.is_success() {
        return Err(format!(
            "OpenRouter API hatası ({}): {}",
            fb_status,
            preview(&fb_body)
        ));
    }

    extract_reply(&fb_body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chat_completion_response_deserialization() {
        let json_str = r#"{
            "choices": [
                {
                    "message": {
                        "content": "Aferin sana, yine başardın!"
                    }
                }
            ]
        }"#;

        assert_eq!(
            extract_reply(json_str).unwrap(),
            "Aferin sana, yine başardın!"
        );
    }

    /// Modelin iç düşüncesi (`reasoning`) kanala yazılmaz: içerik boşsa yanıt boş sayılır ve yedek modele geçilir.
    #[test]
    fn null_content_never_leaks_reasoning() {
        let json_str = r#"{
            "choices": [
                {
                    "message": {
                        "content": null,
                        "reasoning": "Kısa cevap: merhaba."
                    }
                }
            ]
        }"#;

        assert!(extract_reply(json_str).is_err());
    }

    #[test]
    fn test_multipart_content() {
        let json_str = r#"{
            "choices": [
                {
                    "message": {
                        "content": [{"type": "text", "text": "Parça 1 "}, {"type": "text", "text": "parça 2"}]
                    }
                }
            ]
        }"#;

        assert_eq!(extract_reply(json_str).unwrap(), "Parça 1 parça 2");
    }

    #[test]
    fn oversized_provider_reply_is_bounded_before_returning() {
        let content = "x".repeat(MAX_REPLY_CHARS + 500);
        let json = serde_json::json!({
            "choices": [{"message": {"content": content}}]
        })
        .to_string();

        let reply = extract_reply(&json).unwrap();
        assert_eq!(reply.chars().count(), MAX_REPLY_CHARS);
    }

    #[test]
    fn oversized_prompt_is_bounded() {
        let prompt = bounded_prompt(&"x".repeat(MAX_PROMPT_CHARS + 1), "test");
        assert_eq!(prompt.chars().count(), MAX_PROMPT_CHARS);
    }

    #[test]
    fn test_normalize_openrouter_model() {
        assert_eq!(
            normalize_openrouter_model("openrouter:free"),
            "openrouter/free"
        );
        assert_eq!(
            normalize_openrouter_model("openrouter/free"),
            "openrouter/free"
        );
        assert_eq!(normalize_openrouter_model("  "), DEFAULT_MODEL);
        assert_eq!(
            normalize_openrouter_model("google/gemma-4-31b-it:free"),
            "google/gemma-4-31b-it:free"
        );
        assert_eq!(
            normalize_openrouter_model("openrouter:google/gemma-4-31b-it:free"),
            "google/gemma-4-31b-it:free"
        );
        assert_eq!(
            normalize_openrouter_model("openrouter/google/gemma-4-31b-it:free"),
            "google/gemma-4-31b-it:free"
        );
    }
}
