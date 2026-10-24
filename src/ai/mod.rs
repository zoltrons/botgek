//! OpenRouter sohbet istemcisi.
//!
//! Tek bir `/chat/completions` çağrısı yapılır; birincil model hata verirse OpenRouter'ın
//! sunucu tarafı yedekleme zinciri (`models`) ücretsiz yönlendiriciye (`openrouter/free`)
//! geçer. Ağ hatası, zaman aşımı, hız sınırı veya boş yanıt gibi geçici durumlarda istemci,
//! süre bütçesi el verdiği sürece ücretsiz yönlendiriciyle bir kez daha dener.
//!
//! Uyumluluk için istek en dar ortak biçimde kurulur: yalnızca bir `system` ve bir `user`
//! mesajı gönderilir. Yanıt zinciri ve yanıtlanan mesaj ayrı `assistant` turları yerine
//! kullanıcı mesajının içine bağlam olarak yazılır; bazı modellerin sohbet şablonları
//! (ör. Gemma) `assistant` ile başlayan ya da sırası bozulan turları reddeder. Görsel varsa
//! `user` içeriği metin + `image_url` parçalarına dönüşür.
//!
//! Gizlilik (en az veri): istek kimlik taşımaz (`user` alanı yok), konuşmadaki kişiler
//! [`discord`] tarafında takma adlarla gösterilir ve `provider.data_collection: "deny"` ile
//! yalnızca veriyi saklamayan / eğitimde kullanmayan sağlayıcılara yönlendirilir.

pub mod discord;

use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tracing::{info, warn};

use crate::utils::truncate_chars;

pub const DEFAULT_MODEL: &str = "google/gemma-4-31b-it:free";
/// Yalnızca ücretsiz modellere yönlendiren OpenRouter yönlendiricisi; yedek zincirin sonu.
pub const AUTO_FALLBACK_MODEL: &str = "openrouter/free";

const ENDPOINT: &str = "https://openrouter.ai/api/v1/chat/completions";
/// OpenRouter uygulama künyesi: `HTTP-Referer` uygulamayı tanımlar, başlık adını verir.
const APP_URL: &str = "https://github.com/zoltrons/botgek";
const APP_TITLE: &str = "Botgek";

/// Hard limits for an individual provider call. These are deliberately kept
/// here, next to the request construction, so callers cannot accidentally
/// bypass them by using a different prompt path.
pub const MAX_PROMPT_CHARS: usize = 8_000;
pub const MAX_RESPONSE_BYTES: usize = 1_048_576;
pub const MAX_REPLY_CHARS: usize = 8_000;
/// Yanıtlanan mesajdan isteme taşınan en fazla karakter.
const MAX_CONTEXT_CHARS: usize = 1_500;
/// Yanıt zincirindeki her eski mesajdan taşınan en fazla karakter.
const MAX_HISTORY_CHARS: usize = 600;
const MAX_MODEL_CHARS: usize = 256;
const MAX_IN_FLIGHT_REQUESTS: usize = 2;
/// Görünür yanıt + akıl yürütme toplam bütçesi. Reasoning modellerinde `max_tokens`
/// düşünmeyi de kapsar; küçük bir bütçe `finish_reason: "length"` ve boş içerikle biter.
const MAX_COMPLETION_TOKENS: u32 = 2_048;
const TEMPERATURE: f32 = 0.9;
/// Tek çağrının üst sınırı. İkinci deneme yalnızca ilk çağrı bu sürenin altında
/// bittiyse yapılır; böylece bir istek bir yuvayı en fazla ~2×20 sn tutar.
const PROVIDER_TIMEOUT: Duration = Duration::from_secs(20);

const SAFETY_GUARDRAIL: &str = "\n\n[KRİTİK GÜVENLİK TALİMATI]: Yanıtlarında ASLA '@everyone' veya '@here' etiketlerini KULLANMA. Kullanıcıları veya rolleri etiketleme. Bu kural kesindir ve çiğnenemez.\n[BİÇİM]: Yanıtın bir Discord mesajı olarak gösterilecek; 2000 karakteri geçme. Düşünme sürecini değil, yalnızca son cevabı yaz.\n[GİZLİLİK]: Konuşmadaki kişiler gizlilik için \"Kişi A\" gibi takma adlarla, seninle konuşan kişi \"Kullanıcı\" olarak gösterilir. Gerçek isimlerini sorma veya uydurma.";

/// Do not queue an unbounded number of provider calls. The handler already has
/// a per-user cooldown; this guard protects the whole process from a burst of
/// users or from a slow provider holding tasks indefinitely.
static AI_IN_FLIGHT: std::sync::LazyLock<Arc<Semaphore>> =
    std::sync::LazyLock::new(|| Arc::new(Semaphore::new(MAX_IN_FLIGHT_REQUESTS)));

/// Boş sağlayıcı yuvası var mı? Çağıran taraf kullanıcı/sunucu bütçesini harcamadan önce sorar; dolu yuvaya giden istek bütçeyi boşa tüketip hata bildirimi gönderiyordu.
pub fn has_capacity() -> bool {
    AI_IN_FLIGHT.available_permits() > 0
}

// ─── Hata türleri ─────────────────────────────────────────────────────────────

/// Sağlayıcı çağrısının başarısızlık nedeni. Ayrıntılar yalnızca log içindir; Discord'a
/// gösterilecek metin çağıran tarafta türden seçilir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiError {
    /// Süreç genelindeki eşzamanlı istek sınırı dolu.
    Busy,
    /// Model adı geçersiz ya da çok uzun.
    InvalidModel(String),
    /// Bağlantı kurulamadı veya yanıt okunamadı.
    Network(String),
    /// İstek zaman aşımına uğradı (istemci ya da HTTP 408).
    Timeout,
    /// HTTP 429 / `rate_limit_exceeded`.
    RateLimited(String),
    /// HTTP 401: anahtar geçersiz veya devre dışı.
    Auth(String),
    /// HTTP 402: hesapta ya da anahtarda kredi yok.
    Credits(String),
    /// HTTP 403: moderasyon, guardrail veya yetki engeli.
    Blocked(String),
    /// HTTP 400: geçersiz istek (ör. bilinmeyen model, desteklenmeyen parametre).
    BadRequest(String),
    /// HTTP 404: istenen özellikleri (ör. görsel girdi) ve veri politikasını karşılayan
    /// sağlayıcı yok.
    NoProvider(String),
    /// HTTP 502/503 ve diğer 5xx: model kapalı ya da uygun sağlayıcı yok.
    Unavailable(String),
    /// Model bütçeyi düşünmeye harcadı: `finish_reason: "length"` ve boş içerik.
    Truncated,
    /// Başarılı yanıt geldi ama metin içermiyor.
    Empty,
    /// Yanıt ayrıştırılamadı ya da boyut sınırını aştı.
    Malformed(String),
}

impl AiError {
    /// Hata kodundan (HTTP durumu veya gövdedeki `error.code`) türe çevirir.
    fn from_code(code: u16, message: String) -> Self {
        match code {
            400 => AiError::BadRequest(message),
            401 => AiError::Auth(message),
            402 => AiError::Credits(message),
            403 => AiError::Blocked(message),
            404 => AiError::NoProvider(message),
            408 => AiError::Timeout,
            429 => AiError::RateLimited(message),
            _ => AiError::Unavailable(format!("{code}: {message}")),
        }
    }

    /// Ücretsiz yönlendiriciyle tekrar denemeye değer mi? İstek, kimlik ve kredi hataları
    /// yedek modelde de aynen tekrar eder.
    fn retryable(&self) -> bool {
        matches!(
            self,
            AiError::Network(_)
                | AiError::Timeout
                | AiError::RateLimited(_)
                | AiError::NoProvider(_)
                | AiError::Unavailable(_)
                | AiError::Truncated
                | AiError::Empty
                | AiError::Malformed(_)
        )
    }
}

impl fmt::Display for AiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AiError::Busy => write!(f, "AI istek sınırı dolu"),
            AiError::InvalidModel(detail) => write!(f, "geçersiz model: {detail}"),
            AiError::Network(detail) => write!(f, "OpenRouter bağlantı hatası: {detail}"),
            AiError::Timeout => write!(f, "OpenRouter isteği zaman aşımına uğradı"),
            AiError::RateLimited(detail) => write!(f, "OpenRouter hız sınırı (429): {detail}"),
            AiError::Auth(detail) => write!(f, "OpenRouter kimlik hatası (401): {detail}"),
            AiError::Credits(detail) => write!(f, "OpenRouter kredisi yetersiz (402): {detail}"),
            AiError::Blocked(detail) => write!(f, "OpenRouter isteği engelledi (403): {detail}"),
            AiError::BadRequest(detail) => write!(f, "OpenRouter geçersiz istek (400): {detail}"),
            AiError::NoProvider(detail) => write!(
                f,
                "OpenRouter'da isteği ve veri politikasını (data_collection: deny) karşılayan sağlayıcı yok (404): {detail}"
            ),
            AiError::Unavailable(detail) => write!(f, "OpenRouter modeli kullanılamıyor: {detail}"),
            AiError::Truncated => write!(
                f,
                "model token bütçesini düşünmeye harcadı (finish_reason: length, içerik boş)"
            ),
            AiError::Empty => write!(f, "OpenRouter'dan boş yanıt döndü"),
            AiError::Malformed(detail) => write!(f, "OpenRouter yanıtı okunamadı: {detail}"),
        }
    }
}

impl std::error::Error for AiError {}

// ─── İstek ────────────────────────────────────────────────────────────────────

/// Bağlamdaki bir mesaj; konuşanın adı zaten takma ada çevrilmiştir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextMessage {
    /// "Kullanıcı", "Kişi A" veya botun kendisi için "Sen".
    pub speaker: String,
    pub text: String,
}

/// İsteğe eklenen görsel (`data:` adresi; Discord CDN bağlantısı sağlayıcıya verilmez).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageInput {
    pub data_url: String,
    /// Görsel yanıtlanan mesajdan mı, kullanıcının kendi mesajından mı geldi?
    pub from_reply: bool,
}

/// Kullanıcının mesajının çevresi: yanıt zinciri, yanıtlanan mesaj, görseller, emojiler.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Conversation {
    /// Yanıtlanan mesajdan önceki zincir, eskiden yeniye.
    pub history: Vec<ContextMessage>,
    /// Kullanıcının doğrudan yanıtladığı mesaj.
    pub replied: Option<ContextMessage>,
    pub images: Vec<ImageInput>,
    /// Boyut/biçim nedeniyle eklenemeyen görsel sayısı.
    pub skipped_images: usize,
    /// Sunucunun bot tarafından kullanılabilen özel emoji adları.
    pub emojis: Vec<String>,
}

/// Tek bir AI sohbet isteği. Hiçbir alan kullanıcı kimliği taşımaz.
pub struct ChatPrompt<'a> {
    pub model: &'a str,
    pub system_prompt: &'a str,
    pub user_prompt: &'a str,
    pub conversation: &'a Conversation,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: serde_json::Value,
}

#[derive(Serialize)]
struct ReasoningOptions {
    /// Düşünme metni yanıtta istenmez (kanala sızmasın, gövde şişmesin).
    exclude: bool,
}

#[derive(Serialize)]
struct ProviderPreferences {
    /// `deny`: yalnızca istemi saklamayan ve eğitimde kullanmayan sağlayıcılar.
    data_collection: &'static str,
}

#[derive(Serialize)]
struct ChatCompletionRequest<'a> {
    model: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    models: Option<Vec<&'a str>>,
    messages: [ChatMessage<'a>; 2],
    max_tokens: u32,
    temperature: f32,
    reasoning: ReasoningOptions,
    provider: ProviderPreferences,
    stream: bool,
}

// ─── Yanıt ────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ChatCompletionResponse {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    choices: Option<Vec<ChatChoice>>,
    #[serde(default)]
    error: Option<OpenRouterError>,
}

#[derive(Deserialize)]
struct ChatChoice {
    #[serde(default)]
    message: Option<ChatResponseMessage>,
    #[serde(default)]
    finish_reason: Option<String>,
    #[serde(default)]
    error: Option<OpenRouterError>,
}

#[derive(Deserialize)]
struct ChatResponseMessage {
    #[serde(default)]
    content: Option<serde_json::Value>,
    #[serde(default)]
    refusal: Option<String>,
}

#[derive(Deserialize, Debug)]
struct OpenRouterError {
    #[serde(default)]
    code: Option<serde_json::Value>,
    #[serde(default)]
    message: String,
    #[serde(default)]
    metadata: Option<serde_json::Value>,
}

impl OpenRouterError {
    fn code(&self) -> Option<u16> {
        match self.code.as_ref()? {
            serde_json::Value::Number(n) => n.as_u64().and_then(|n| u16::try_from(n).ok()),
            serde_json::Value::String(s) => s.parse().ok(),
            _ => None,
        }
    }

    /// Loga yazılacak açıklama: mesaj + sağlayıcı adı / tür (varsa).
    fn describe(&self) -> String {
        let mut text = preview(&self.message);
        if let Some(metadata) = &self.metadata {
            for key in ["provider_name", "error_type"] {
                if let Some(value) = metadata.get(key).and_then(|v| v.as_str()) {
                    text.push_str(&format!(" [{key}={value}]"));
                }
            }
        }
        text
    }

    fn into_error(self, fallback_code: u16) -> AiError {
        let code = self.code().unwrap_or(fallback_code);
        let rate_limited = self
            .metadata
            .as_ref()
            .and_then(|m| m.get("error_type"))
            .and_then(|v| v.as_str())
            == Some("rate_limit_exceeded");
        let detail = self.describe();
        if rate_limited {
            AiError::RateLimited(detail)
        } else {
            AiError::from_code(code, detail)
        }
    }
}

/// Başarılı yanıttaki metin ve onu üreten model.
#[derive(Debug, PartialEq, Eq)]
struct Reply {
    text: String,
    model: Option<String>,
}

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
        warn!(
            "AI {label} istemi {} karakterden {} karaktere kırpıldı",
            trimmed.chars().count(),
            MAX_PROMPT_CHARS
        );
        truncate_chars(trimmed, MAX_PROMPT_CHARS)
    }
}

fn quote(text: &str, max_chars: usize) -> String {
    truncate_chars(text.trim(), max_chars)
        .lines()
        .map(|line| format!("> {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Kullanıcı mesajını yanıt zinciri ve yanıtlanan mesajla birlikte tek bir `user` turuna dizer.
fn compose_user_prompt(query: &str, conversation: &Conversation) -> String {
    let mut sections = Vec::new();

    let history: Vec<String> = conversation
        .history
        .iter()
        .filter(|message| !message.text.trim().is_empty())
        .map(|message| {
            format!(
                "{}:\n{}",
                message.speaker,
                quote(&message.text, MAX_HISTORY_CHARS)
            )
        })
        .collect();
    if !history.is_empty() {
        sections.push(format!(
            "[Önceki konuşma, eskiden yeniye]\n{}",
            history.join("\n")
        ));
    }

    let reply_images = conversation.images.iter().filter(|i| i.from_reply).count();
    if let Some(replied) = &conversation.replied {
        let mut section = format!(
            "[Kullanıcının yanıtladığı mesaj · yazan: {}]",
            replied.speaker
        );
        if !replied.text.trim().is_empty() {
            section.push_str(&format!("\n{}", quote(&replied.text, MAX_CONTEXT_CHARS)));
        }
        if reply_images > 0 {
            section.push_str(&format!(
                "\n(Bu mesajdaki {reply_images} görsel isteğe ekli.)"
            ));
        }
        sections.push(section);
    }

    let mut own = format!("[Kullanıcının mesajı]\n{query}");
    let own_images = conversation.images.len() - reply_images;
    if own_images > 0 {
        own.push_str(&format!(
            "\n(Kullanıcının mesajındaki {own_images} görsel isteğe ekli.)"
        ));
    }
    if conversation.skipped_images > 0 {
        own.push_str(&format!(
            "\n({} görsel boyut veya biçim nedeniyle eklenemedi; görmediğin görseli anlatmaya çalışma.)",
            conversation.skipped_images
        ));
    }

    let prompt = if sections.is_empty() && own_images == 0 && conversation.skipped_images == 0 {
        query.to_string()
    } else {
        sections.push(own);
        sections.join("\n\n")
    };
    bounded_prompt(&prompt, "kullanıcı")
}

/// Kişiliğe güvenlik kuralları ve sunucu emojileri eklenir.
fn compose_system_prompt(system_prompt: &str, conversation: &Conversation) -> String {
    let mut prompt = format!(
        "{}{SAFETY_GUARDRAIL}",
        bounded_prompt(system_prompt, "system")
    );
    if !conversation.emojis.is_empty() {
        let list = conversation
            .emojis
            .iter()
            .map(|name| format!(":{name}:"))
            .collect::<Vec<_>>()
            .join(" ");
        prompt.push_str(&format!(
            "\n[SUNUCU EMOJİLERİ]: Bu sunucunun özel emojileri: {list}\nEmoji kullanmak zorunda değilsin; yanıtların çoğu emojisiz olmalı. Yalnızca bir emoji söylediğini gerçekten güçlendiriyorsa ve anlamı bağlama uyuyorsa en fazla bir tane, tam olarak :ad: biçiminde kullan. Listede olmayan özel emoji uydurma; emin değilsen hiç kullanma."
        ));
    }
    prompt
}

/// Görsel yoksa düz metin (en geniş uyumluluk), varsa metin önce, görseller sonra.
fn user_content(text: &str, images: &[ImageInput]) -> serde_json::Value {
    if images.is_empty() {
        return serde_json::Value::String(text.to_string());
    }
    let mut parts = vec![serde_json::json!({ "type": "text", "text": text })];
    parts.extend(images.iter().map(
        |image| serde_json::json!({ "type": "image_url", "image_url": { "url": image.data_url } }),
    ));
    serde_json::Value::Array(parts)
}

async fn read_bounded_body(mut response: reqwest::Response) -> Result<String, AiError> {
    let too_large = || {
        AiError::Malformed(format!(
            "yanıt boyut sınırını aşıyor (en fazla {MAX_RESPONSE_BYTES} bayt)"
        ))
    };
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(too_large());
    }

    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(too_large());
        }
        body.extend_from_slice(&chunk);
    }

    String::from_utf8(body).map_err(|_| AiError::Malformed("gövde UTF-8 değil".to_string()))
}

fn transport_error(err: reqwest::Error) -> AiError {
    if err.is_timeout() {
        AiError::Timeout
    } else {
        AiError::Network(format!("{err:?}"))
    }
}

/// WebUI / Cursor tarzı slug'ları OpenRouter'ın beklediği `sağlayıcı/model` biçimine çevirir.
/// Model sayfasının adresi (`https://openrouter.ai/sağlayıcı/model`) de kabul edilir.
pub fn normalize_openrouter_model(raw: &str) -> String {
    let mut model = raw
        .trim()
        .trim_matches(|c| c == '"' || c == '\'')
        .to_string();

    for prefix in [
        "https://openrouter.ai/",
        "http://openrouter.ai/",
        "openrouter.ai/",
    ] {
        if let Some(rest) = model.strip_prefix(prefix) {
            let rest = rest.split(['?', '#']).next().unwrap_or_default();
            model = rest
                .trim_end_matches('/')
                .trim_start_matches("models/")
                .to_string();
            break;
        }
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
    let text = match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(parts) => parts
            .iter()
            .filter_map(|part| match part {
                serde_json::Value::String(s) => Some(s.as_str()),
                // Yalnızca metin parçaları: `reasoning`/`thinking` türündeki parçalar kanala yazılmaz.
                serde_json::Value::Object(obj)
                    if obj
                        .get("type")
                        .and_then(|t| t.as_str())
                        .is_none_or(|t| t == "text" || t == "output_text") =>
                {
                    obj.get("text").and_then(|t| t.as_str())
                }
                _ => None,
            })
            .collect(),
        serde_json::Value::Object(obj) => return obj.get("text").and_then(json_to_text),
        _ => return None,
    };
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn extract_reply(body_text: &str) -> Result<Reply, AiError> {
    let parsed: ChatCompletionResponse = serde_json::from_str(body_text.trim_start())
        .map_err(|e| AiError::Malformed(format!("{e} (gövde: {})", preview(body_text))))?;

    if let Some(err) = parsed.error {
        return Err(err.into_error(500));
    }

    let choice = parsed
        .choices
        .and_then(|choices| choices.into_iter().next())
        .ok_or(AiError::Empty)?;
    let message = choice.message.as_ref();
    let text = message
        .and_then(|m| m.content.as_ref())
        .and_then(json_to_text);

    // Sağlayıcı yanıtın ortasında koptuysa yarım metin kanala yazılmaz.
    if let Some(err) = choice.error {
        return Err(err.into_error(502));
    }
    if choice.finish_reason.as_deref() == Some("error") {
        return Err(AiError::Unavailable(
            "sağlayıcı yanıtı hata ile bitirdi".to_string(),
        ));
    }

    match text {
        Some(text) => Ok(Reply {
            text: truncate_chars(&text, MAX_REPLY_CHARS),
            model: parsed.model,
        }),
        None => {
            if let Some(refusal) = message.and_then(|m| m.refusal.as_deref())
                && !refusal.trim().is_empty()
            {
                return Ok(Reply {
                    text: truncate_chars(refusal.trim(), MAX_REPLY_CHARS),
                    model: parsed.model,
                });
            }
            if choice.finish_reason.as_deref() == Some("length") {
                Err(AiError::Truncated)
            } else {
                Err(AiError::Empty)
            }
        }
    }
}

/// Bir HTTP çağrısı: durum kodunu ve gövdeyi türlü hataya çevirir.
async fn send(
    client: &reqwest::Client,
    api_key: &str,
    payload: &serde_json::Value,
) -> Result<Reply, AiError> {
    let response = client
        .post(ENDPOINT)
        .bearer_auth(api_key.trim())
        .header("HTTP-Referer", APP_URL)
        .header("X-OpenRouter-Title", APP_TITLE)
        // Eski başlık adı; OpenRouter geriye dönük uyumluluk için hâlâ okuyor.
        .header("X-Title", APP_TITLE)
        .json(payload)
        .timeout(PROVIDER_TIMEOUT)
        .send()
        .await
        .map_err(transport_error)?;

    let status = response.status();
    let body = read_bounded_body(response).await?;
    if status.is_success() {
        return extract_reply(&body);
    }

    // Hata gövdesi OpenRouter biçimindeyse (`{"error": {...}}`) ondan, değilse (ör. ara
    // vekil sunucunun HTML sayfası) yalnızca durum kodundan sınıflandırılır.
    let parsed = serde_json::from_str::<ChatCompletionResponse>(body.trim_start())
        .ok()
        .and_then(|parsed| parsed.error);
    Err(match parsed {
        Some(err) => err.into_error(status.as_u16()),
        None => AiError::from_code(status.as_u16(), preview(&body)),
    })
}

/// OpenRouter API'si üzerinden kullanıcı mesajına laf sokucu / esprili yanıt üretir.
pub async fn query_openrouter(
    client: &reqwest::Client,
    api_key: &str,
    prompt: &ChatPrompt<'_>,
) -> Result<String, AiError> {
    let _permit = AI_IN_FLIGHT
        .clone()
        .try_acquire_owned()
        .map_err(|_| AiError::Busy)?;

    let model = normalize_openrouter_model(prompt.model);
    if model.chars().count() > MAX_MODEL_CHARS || model.chars().any(char::is_whitespace) {
        return Err(AiError::InvalidModel(preview(&model)));
    }

    let system_prompt = compose_system_prompt(prompt.system_prompt, prompt.conversation);
    let user_prompt = compose_user_prompt(prompt.user_prompt, prompt.conversation);
    let messages = || {
        [
            ChatMessage {
                role: "system",
                content: serde_json::Value::String(system_prompt.clone()),
            },
            ChatMessage {
                role: "user",
                content: user_content(&user_prompt, &prompt.conversation.images),
            },
        ]
    };
    let payload = |model: &str, models: Option<Vec<&str>>| {
        ChatCompletionRequest {
            model,
            models,
            messages: messages(),
            max_tokens: MAX_COMPLETION_TOKENS,
            temperature: TEMPERATURE,
            reasoning: ReasoningOptions { exclude: true },
            provider: ProviderPreferences {
                data_collection: "deny",
            },
            stream: false,
        }
        .serialize(serde_json::value::Serializer)
        .unwrap_or_default()
    };

    // Sunucu tarafı yedekleme: birincil model hata verirse OpenRouter aynı istekte ücretsiz
    // yönlendiriciye geçer. Yönlendirici yalnızca ücretsiz modellere düştüğü için ücretli bir
    // birincil modelin yedeği ek ücret doğurmaz; görsel varsa görsel okuyabilen bir model seçer.
    let primary = if model == AUTO_FALLBACK_MODEL {
        payload(&model, None)
    } else {
        payload(&model, Some(vec![model.as_str(), AUTO_FALLBACK_MODEL]))
    };

    info!("OpenRouter isteği gönderiliyor (Model: {model})...");
    let started = Instant::now();
    let first = send(client, api_key, &primary).await;
    let error = match first {
        Ok(reply) => return Ok(log_reply(reply, &model)),
        Err(error) => error,
    };

    // İstemci tarafı ikinci savunma hattı: geçici hatalarda ücretsiz yönlendiriciyle bir kez
    // daha denenir. İlk çağrı zaten süre bütçesini tükettiyse yuva daha fazla tutulmaz.
    if !error.retryable() || started.elapsed() >= PROVIDER_TIMEOUT {
        return Err(error);
    }
    warn!(
        "OpenRouter isteği başarısız ({error}), '{AUTO_FALLBACK_MODEL}' ile yeniden deneniyor..."
    );
    let fallback = payload(AUTO_FALLBACK_MODEL, None);
    send(client, api_key, &fallback)
        .await
        .map(|reply| log_reply(reply, AUTO_FALLBACK_MODEL))
}

fn log_reply(reply: Reply, requested: &str) -> String {
    let served_by = reply.model.as_deref().unwrap_or(requested);
    if served_by != requested {
        info!("OpenRouter yanıtı '{served_by}' modelinden geldi (istenen: '{requested}')");
    }
    reply.text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(body: &str) -> String {
        extract_reply(body).unwrap().text
    }

    #[test]
    fn test_chat_completion_response_deserialization() {
        let json_str = r#"{
            "model": "google/gemma-4-31b-it:free",
            "choices": [
                {
                    "message": {
                        "content": "Aferin sana, yine başardın!"
                    },
                    "finish_reason": "stop"
                }
            ]
        }"#;

        let reply = extract_reply(json_str).unwrap();
        assert_eq!(reply.text, "Aferin sana, yine başardın!");
        assert_eq!(reply.model.as_deref(), Some("google/gemma-4-31b-it:free"));
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

        assert_eq!(extract_reply(json_str), Err(AiError::Empty));
    }

    #[test]
    fn reasoning_that_eats_the_budget_is_reported_as_truncated() {
        let json_str = r#"{
            "choices": [{"message": {"content": ""}, "finish_reason": "length"}]
        }"#;
        assert_eq!(extract_reply(json_str), Err(AiError::Truncated));
        assert!(AiError::Truncated.retryable());
    }

    #[test]
    fn test_multipart_content() {
        let json_str = r#"{
            "choices": [
                {
                    "message": {
                        "content": [
                            {"type": "reasoning", "text": "gizli düşünce"},
                            {"type": "text", "text": "Parça 1 "},
                            {"type": "text", "text": "parça 2"}
                        ]
                    }
                }
            ]
        }"#;

        assert_eq!(text(json_str), "Parça 1 parça 2");
    }

    #[test]
    fn refusal_is_shown_when_content_is_missing() {
        let json_str = r#"{
            "choices": [{"message": {"content": null, "refusal": "Bunu yapamam."}}]
        }"#;
        assert_eq!(text(json_str), "Bunu yapamam.");
    }

    #[test]
    fn mid_response_provider_error_discards_partial_text() {
        let json_str = r#"{
            "choices": [{
                "message": {"role": "assistant", "content": "yarım cev"},
                "finish_reason": "error",
                "error": {"code": 502, "message": "Provider disconnected", "metadata": {"error_type": "provider_unavailable"}}
            }]
        }"#;
        let err = extract_reply(json_str).unwrap_err();
        assert!(matches!(err, AiError::Unavailable(_)), "{err:?}");
    }

    #[test]
    fn top_level_errors_are_typed() {
        let rate = r#"{"error": {"code": 429, "message": "Rate limit exceeded", "metadata": {"error_type": "rate_limit_exceeded"}}}"#;
        assert!(matches!(extract_reply(rate), Err(AiError::RateLimited(_))));
        let credits = r#"{"error": {"code": 402, "message": "Insufficient credits"}}"#;
        assert!(matches!(extract_reply(credits), Err(AiError::Credits(_))));
        let moderated = r#"{"error": {"code": "403", "message": "flagged", "metadata": {"provider_name": "X"}}}"#;
        let err = extract_reply(moderated).unwrap_err();
        assert!(matches!(&err, AiError::Blocked(detail) if detail.contains("provider_name=X")));
    }

    #[test]
    fn only_transient_errors_are_retried() {
        assert!(AiError::from_code(429, String::new()).retryable());
        assert!(AiError::from_code(503, String::new()).retryable());
        assert!(AiError::from_code(408, String::new()).retryable());
        for code in [400, 401, 402, 403] {
            assert!(
                !AiError::from_code(code, String::new()).retryable(),
                "{code}"
            );
        }
    }

    #[test]
    fn leading_whitespace_is_tolerated() {
        // OpenRouter uzun süren isteklerde bağlantıyı açık tutmak için boşluk gönderebilir.
        let body = "\n\n   {\"choices\": [{\"message\": {\"content\": \"selam\"}}]}";
        assert_eq!(text(body), "selam");
    }

    #[test]
    fn oversized_provider_reply_is_bounded_before_returning() {
        let content = "x".repeat(MAX_REPLY_CHARS + 500);
        let json = serde_json::json!({
            "choices": [{"message": {"content": content}}]
        })
        .to_string();

        assert_eq!(text(&json).chars().count(), MAX_REPLY_CHARS);
    }

    #[test]
    fn oversized_prompt_is_bounded() {
        let prompt = bounded_prompt(&"x".repeat(MAX_PROMPT_CHARS + 1), "test");
        assert_eq!(prompt.chars().count(), MAX_PROMPT_CHARS);
    }

    fn person(speaker: &str, text: &str) -> ContextMessage {
        ContextMessage {
            speaker: speaker.to_string(),
            text: text.to_string(),
        }
    }

    #[test]
    fn plain_question_is_sent_as_is() {
        assert_eq!(
            compose_user_prompt("selam", &Conversation::default()),
            "selam"
        );
    }

    #[test]
    fn reply_chain_is_folded_into_a_single_user_turn() {
        let conversation = Conversation {
            history: vec![person("Kullanıcı", "ilk soru"), person("Sen", "ilk cevap")],
            replied: Some(person("Kişi A", "Çünkü\nöyle.")),
            images: vec![ImageInput {
                data_url: "data:image/webp;base64,AA==".into(),
                from_reply: true,
            }],
            ..Conversation::default()
        };
        let prompt = compose_user_prompt("özetle", &conversation);
        let first = prompt.find("ilk soru").unwrap();
        let replied = prompt.find("> Çünkü\n> öyle.").unwrap();
        assert!(first < replied, "zincir eskiden yeniye sıralanmalı");
        assert!(prompt.contains("yazan: Kişi A"));
        assert!(prompt.contains("1 görsel isteğe ekli"));
        assert!(prompt.ends_with("özetle"));
    }

    #[test]
    fn long_context_is_bounded() {
        let conversation = Conversation {
            replied: Some(person("Kişi A", &"a".repeat(MAX_CONTEXT_CHARS * 2))),
            ..Conversation::default()
        };
        let prompt = compose_user_prompt("ne diyor?", &conversation);
        assert!(prompt.chars().count() < MAX_CONTEXT_CHARS + 200);
    }

    #[test]
    fn images_become_content_parts_after_the_text() {
        assert_eq!(user_content("metin", &[]), serde_json::json!("metin"));
        let image = ImageInput {
            data_url: "data:image/webp;base64,AA==".into(),
            from_reply: false,
        };
        let content = user_content("metin", &[image]);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image_url");
        assert_eq!(
            content[1]["image_url"]["url"],
            "data:image/webp;base64,AA=="
        );
    }

    #[test]
    fn emojis_are_offered_in_the_system_prompt() {
        let conversation = Conversation {
            emojis: vec!["kekw".into(), "pepe_sad".into()],
            ..Conversation::default()
        };
        let prompt = compose_system_prompt("kişilik", &conversation);
        assert!(prompt.starts_with("kişilik"));
        assert!(prompt.contains(":kekw: :pepe_sad:"));
        assert!(!compose_system_prompt("kişilik", &Conversation::default()).contains("EMOJİ"));
    }

    #[test]
    fn request_body_matches_openrouter_schema() {
        let body = serde_json::to_value(ChatCompletionRequest {
            model: "a/b",
            models: Some(vec!["a/b", AUTO_FALLBACK_MODEL]),
            messages: [
                ChatMessage {
                    role: "system",
                    content: serde_json::json!("s"),
                },
                ChatMessage {
                    role: "user",
                    content: serde_json::json!("u"),
                },
            ],
            max_tokens: MAX_COMPLETION_TOKENS,
            temperature: TEMPERATURE,
            reasoning: ReasoningOptions { exclude: true },
            provider: ProviderPreferences {
                data_collection: "deny",
            },
            stream: false,
        })
        .unwrap();
        assert_eq!(
            body["models"],
            serde_json::json!(["a/b", "openrouter/free"])
        );
        assert_eq!(body["reasoning"], serde_json::json!({ "exclude": true }));
        assert_eq!(
            body["provider"],
            serde_json::json!({ "data_collection": "deny" })
        );
        assert_eq!(body["stream"], false);
        assert_eq!(body["messages"][1]["role"], "user");
        // En az veri: istek kullanıcı kimliği taşımaz.
        assert!(body.get("user").is_none());
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
        assert_eq!(
            normalize_openrouter_model("https://openrouter.ai/google/gemma-4-31b-it:free/"),
            "google/gemma-4-31b-it:free"
        );
        assert_eq!(
            normalize_openrouter_model("https://openrouter.ai/models/openai/gpt-5-mini?tab=api"),
            "openai/gpt-5-mini"
        );
        assert_eq!(
            normalize_openrouter_model("https://openrouter.ai/"),
            DEFAULT_MODEL
        );
    }
}
