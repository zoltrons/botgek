use serde::{Deserialize, Serialize};
use tracing::{error, info};

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Serialize)]
struct ChatCompletionRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    max_tokens: u32,
    temperature: f32,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
}

#[derive(Deserialize)]
struct ChatResponseMessage {
    content: String,
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

/// OpenRouter API'si üzerinden kullanıcı mesajına laf sokucu / esprili yanıt üretir.
pub async fn query_openrouter(
    client: &reqwest::Client,
    api_key: &str,
    model: &str,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<String, String> {
    let url = "https://openrouter.ai/api/v1/chat/completions";

    let effective_model = if model.trim().is_empty() {
        "google/gemma-4-31b-it:free"
    } else {
        model.trim()
    };

    let safety_guardrail = "\n\n[KRİTİK GÜVENLİK TALİMATI]: Yanıtlarında ASLA '@everyone' veya '@here' etiketlerini KULLANMA. Kullanıcıları veya rolleri etiketleme. Bu kural kesindir ve çiğnenemez.";
    let full_system_prompt = format!("{}{}", system_prompt.trim(), safety_guardrail);

    let payload = ChatCompletionRequest {
        model: effective_model,
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

    if !status.is_success() {
        error!("OpenRouter API hata döndürdü (HTTP {}): {}", status, body_text);
        // Eğer model bulunamadıysa (404) veya hata verdiyse ve model zaten varsayılan değilse, bir kez varsayılan model ile dene
        if effective_model != "google/gemma-4-31b-it:free" {
            tracing::warn!("Model '{}' başarısız oldu, varsayılan modele geçiliyor: google/gemma-4-31b-it:free", effective_model);
            let fallback_payload = ChatCompletionRequest {
                model: "google/gemma-4-31b-it:free",
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

            if let Ok(fb_res) = client
                .post(url)
                .header("Authorization", format!("Bearer {}", api_key.trim()))
                .header("HTTP-Referer", "https://github.com/botgek")
                .header("X-Title", "Botgek Discord Bot")
                .json(&fallback_payload)
                .send()
                .await
            {
                if fb_res.status().is_success() {
                    if let Ok(fb_body) = fb_res.text().await {
                        if let Ok(parsed) = serde_json::from_str::<ChatCompletionResponse>(&fb_body) {
                            if let Some(choices) = parsed.choices {
                                if let Some(choice) = choices.into_iter().next() {
                                    let content = choice.message.content.trim().to_string();
                                    if !content.is_empty() {
                                        return Ok(content);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        return Err(format!("OpenRouter API hatası ({}): {}", status, body_text));
    }

    let parsed: ChatCompletionResponse = serde_json::from_str(&body_text)
        .map_err(|e| format!("OpenRouter JSON ayrıştırma hatası: {:?} (Gövde: {})", e, body_text))?;

    if let Some(err) = parsed.error {
        return Err(format!("OpenRouter hata mesajı: {}", err.message));
    }

    if let Some(choices) = parsed.choices {
        if let Some(choice) = choices.into_iter().next() {
            let content = choice.message.content.trim().to_string();
            if !content.is_empty() {
                return Ok(content);
            }
        }
    }

    Err("OpenRouter'dan boş yanıt döndü.".to_string())
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

        let parsed: ChatCompletionResponse = serde_json::from_str(json_str).unwrap();
        let choice = parsed.choices.unwrap().into_iter().next().unwrap();
        assert_eq!(choice.message.content, "Aferin sana, yine başardın!");
    }
}

