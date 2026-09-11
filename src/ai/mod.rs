use serde::{Deserialize, Serialize};
use tracing::{error, info};

pub const DEFAULT_MODEL: &str = "google/gemma-4-31b-it:free";

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
    #[serde(default)]
    reasoning: Option<serde_json::Value>,
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
    let mut model = raw.trim().trim_matches(|c| c == '"' || c == '\'').to_string();
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

    if let Some(rest) = model.strip_prefix("openrouter/") {
        if rest.contains('/') {
            model = rest.to_string();
        }
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
    choice
        .message
        .content
        .as_ref()
        .and_then(json_to_text)
        .or_else(|| choice.message.reasoning.as_ref().and_then(json_to_text))
}

fn extract_reply(body_text: &str) -> Result<String, String> {
    let parsed: ChatCompletionResponse = serde_json::from_str(body_text)
        .map_err(|e| format!("OpenRouter JSON ayrıştırma hatası: {:?} (Gövde: {})", e, body_text))?;

    if let Some(err) = parsed.error {
        return Err(format!("OpenRouter hata mesajı: {}", err.message));
    }

    if let Some(choices) = parsed.choices {
        if let Some(choice) = choices.into_iter().next() {
            if let Some(content) = choice_text(&choice) {
                return Ok(content);
            }
        }
    }

    Err("OpenRouter'dan boş yanıt döndü.".to_string())
}

/// OpenRouter API'si üzerinden kullanıcı mesajına laf sokucu / esprili yanıt üretir.
pub async fn query_openrouter(
    client: &reqwest::Client,
    api_key: &str,
    model: &str,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<String, String> {
    let url = "https://openrouter.ai/api/v1/chat/completions";
    let effective_model = normalize_openrouter_model(model);
    let fallbacks = if effective_model != DEFAULT_MODEL {
        Some(vec![DEFAULT_MODEL])
    } else {
        None
    };

    let safety_guardrail = "\n\n[KRİTİK GÜVENLİK TALİMATI]: Yanıtlarında ASLA '@everyone' veya '@here' etiketlerini KULLANMA. Kullanıcıları veya rolleri etiketleme. Bu kural kesindir ve çiğnenemez.";
    let full_system_prompt = format!("{}{}", system_prompt.trim(), safety_guardrail);

    let payload = ChatCompletionRequest {
        model: &effective_model,
        models: fallbacks,
        messages: vec![
            ChatMessage {
                role: "system",
                content: &full_system_prompt,
            },
            ChatMessage {
                role: "user",
                content: user_prompt,
            },
        ],
        max_tokens: 500,
        temperature: 0.9,
    };

    info!("OpenRouter isteği gönderiliyor (Model: {})...", effective_model);

    let res = client
        .post(url)
        .header("Authorization", format!("Bearer {}", api_key.trim()))
        .header("HTTP-Referer", "https://github.com/botgek")
        .header("X-Title", "Botgek Discord Bot")
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("OpenRouter HTTP bağlantı hatası: {:?}", e))?;

    let status = res.status();
    let body_text = res
        .text()
        .await
        .map_err(|e| format!("Yanıt gövdesi okunamadı: {:?}", e))?;

    if status.is_success() {
        match extract_reply(&body_text) {
            Ok(content) => return Ok(content),
            Err(e) if effective_model == DEFAULT_MODEL => return Err(e),
            Err(e) => {
                tracing::warn!("Model '{}' yanıtı kullanılamadı ({}), varsayılana geçiliyor: {}", effective_model, e, DEFAULT_MODEL);
            }
        }
    } else {
        error!("OpenRouter API hata döndürdü (HTTP {}): {}", status, body_text);
        if effective_model == DEFAULT_MODEL {
            return Err(format!("OpenRouter API hatası ({}): {}", status, body_text));
        }
        tracing::warn!("Model '{}' başarısız oldu, varsayılan modele geçiliyor: {}", effective_model, DEFAULT_MODEL);
    }

    let fallback_payload = ChatCompletionRequest {
        model: DEFAULT_MODEL,
        models: None,
        messages: vec![
            ChatMessage {
                role: "system",
                content: &full_system_prompt,
            },
            ChatMessage {
                role: "user",
                content: user_prompt,
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
        .send()
        .await
        .map_err(|e| format!("OpenRouter yedek HTTP bağlantı hatası: {:?}", e))?;

    let fb_status = fb_res.status();
    let fb_body = fb_res
        .text()
        .await
        .map_err(|e| format!("Yedek yanıt gövdesi okunamadı: {:?}", e))?;

    if !fb_status.is_success() {
        return Err(format!("OpenRouter API hatası ({}): {}", fb_status, fb_body));
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

        assert_eq!(extract_reply(json_str).unwrap(), "Aferin sana, yine başardın!");
    }

    #[test]
    fn test_null_content_falls_back_to_reasoning() {
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

        assert_eq!(extract_reply(json_str).unwrap(), "Kısa cevap: merhaba.");
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
    fn test_normalize_openrouter_model() {
        assert_eq!(normalize_openrouter_model("openrouter:free"), "openrouter/free");
        assert_eq!(normalize_openrouter_model("openrouter/free"), "openrouter/free");
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
