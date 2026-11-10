//! AI sohbet istemcisi: Groq (metin, web aramalı) ve Gemini (görsel, yedek).
//!
//! Her iki sağlayıcı da OpenAI uyumlu `/chat/completions` sunar. Metin istekleri önce Groq'a
//! gider; model gerektiğinde `browser_search` ile güncel bilgiyi kendisi arar. Görselli
//! istekler önce Gemini'ye gider, çünkü Groq'taki modeller görsel okuyamaz. Ağ hatası, zaman
//! aşımı, hız sınırı veya boş yanıt gibi geçici durumlarda, süre bütçesi el verdiği sürece
//! diğer sağlayıcı denenir. Yalnızca anahtarı tanımlı sağlayıcılar kullanılır.
//!
//! Uyumluluk için istek en dar ortak biçimde kurulur: yalnızca bir `system` ve bir `user`
//! mesajı gönderilir. Yanıt zinciri ve yanıtlanan mesaj ayrı `assistant` turları yerine
//! kullanıcı mesajının içine bağlam olarak yazılır; bazı modellerin sohbet şablonları
//! `assistant` ile başlayan ya da sırası bozulan turları reddeder. Görsel varsa `user`
//! içeriği metin + `image_url` parçalarına dönüşür.
//!
//! Gizlilik (en az veri): istek kimlik taşımaz (`user` alanı yok) ve konuşmadaki kişiler
//! [`discord`] tarafında takma adlarla gösterilir. Groq istemi varsayılan olarak saklamaz;
//! Gemini'nin ücretsiz katmanı istemleri Google'ın ürün geliştirmesinde kullanabilir.

pub mod discord;

use serde::Deserialize;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tracing::{error, info, warn};

use crate::utils::truncate_chars;

/// Varsayılan Groq modeli (ayarlardaki `ai.model` ile değiştirilir).
pub const DEFAULT_MODEL: &str = "openai/gpt-oss-120b";
/// Görselli istekler ve Groq hata verdiğinde kullanılan Gemini modeli.
const GEMINI_MODEL: &str = "gemini-3.5-flash-lite";

const GROQ_ENDPOINT: &str = "https://api.groq.com/openai/v1/chat/completions";
const GEMINI_ENDPOINT: &str =
    "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions";

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
/// Her sağlayıcı çağrısının üst sınırı. Takılan ilk sağlayıcı yedeğe geçişi engellemez; bir
/// istek bir yuvayı en fazla ~2×20 sn tutar.
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
    /// HTTP 401: anahtar geçersiz veya devre dışı.
    Auth(String),
    /// HTTP 403: moderasyon, guardrail veya yetki engeli.
    Blocked(String),
    /// HTTP 400: geçersiz istek (ör. desteklenmeyen parametre).
    BadRequest(String),
    /// HTTP 404: model bulunamadı ya da hesap erişemiyor.
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
            403 => AiError::Blocked(message),
            404 => AiError::NoProvider(message),
            408 => AiError::Timeout,
            429 => AiError::RateLimited(message),
            _ => AiError::Unavailable(format!("{code}: {message}")),
        }
    }
}

impl fmt::Display for AiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AiError::Busy => write!(f, "AI istek sınırı dolu"),
            AiError::InvalidModel(detail) => write!(f, "geçersiz model: {detail}"),
            AiError::Network(detail) => write!(f, "AI bağlantı hatası: {detail}"),
            AiError::Timeout => write!(f, "AI isteği zaman aşımına uğradı"),
            AiError::RateLimited(detail) => write!(f, "AI hız sınırı (429): {detail}"),
            AiError::Auth(detail) => write!(f, "AI kimlik hatası (401): {detail}"),
            AiError::Blocked(detail) => write!(f, "AI isteği engelledi (403): {detail}"),
            AiError::BadRequest(detail) => write!(f, "AI geçersiz istek (400): {detail}"),
            AiError::NoProvider(detail) => write!(f, "AI modeli bulunamadı (404): {detail}"),
            AiError::Unavailable(detail) => write!(f, "AI modeli kullanılamıyor: {detail}"),
            AiError::Truncated => write!(
                f,
                "model token bütçesini düşünmeye harcadı (finish_reason: length, içerik boş)"
            ),
            AiError::Empty => write!(f, "AI'dan boş yanıt döndü"),
            AiError::Malformed(detail) => write!(f, "AI yanıtı okunamadı: {detail}"),
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

/// API anahtarları; en az biri tanımlıysa AI sohbeti açıktır.
#[derive(Clone, Copy, Default)]
pub struct Keys<'a> {
    pub groq: Option<&'a str>,
    pub gemini: Option<&'a str>,
}

impl Keys<'_> {
    pub fn any(&self) -> bool {
        self.groq.is_some() || self.gemini.is_some()
    }
}

// ─── Yanıt ────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ChatCompletionResponse {
    #[serde(default)]
    choices: Option<Vec<ChatChoice>>,
    #[serde(default)]
    error: Option<ApiError>,
}

#[derive(Deserialize)]
struct ChatChoice {
    #[serde(default)]
    message: Option<ChatResponseMessage>,
    #[serde(default)]
    finish_reason: Option<String>,
    #[serde(default)]
    error: Option<ApiError>,
}

#[derive(Deserialize)]
struct ChatResponseMessage {
    #[serde(default)]
    content: Option<serde_json::Value>,
    #[serde(default)]
    refusal: Option<String>,
}

#[derive(Deserialize, Debug)]
struct ApiError {
    /// Groq'ta metin (`"rate_limit_exceeded"`), Gemini'de sayı olabilir.
    #[serde(default)]
    code: Option<serde_json::Value>,
    #[serde(default)]
    message: String,
}

impl ApiError {
    fn code(&self) -> Option<u16> {
        match self.code.as_ref()? {
            serde_json::Value::Number(n) => n.as_u64().and_then(|n| u16::try_from(n).ok()),
            serde_json::Value::String(s) => s.parse().ok(),
            _ => None,
        }
    }

    fn into_error(self, fallback_code: u16) -> AiError {
        let rate_limited =
            self.code.as_ref().and_then(|c| c.as_str()) == Some("rate_limit_exceeded");
        let code = self.code().unwrap_or(fallback_code);
        let detail = preview(&self.message);
        if rate_limited {
            AiError::RateLimited(detail)
        } else {
            AiError::from_code(code, detail)
        }
    }
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
            "\n({} görsel eklenemedi (boyut, biçim ya da görsel okuyamayan model); görmediğin görseli anlatmaya çalışma.)",
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

/// Eski OpenRouter model adlarını (`sağlayıcı/model:free`, `openrouter/free`) ve boş değeri
/// varsayılan Groq modeline çevirir; kayıtlı eski ayarlar böylece 404 üretmez.
pub fn normalize_model(raw: &str) -> String {
    let model = raw.trim().trim_matches(|c| c == '"' || c == '\'');
    if model.is_empty() || model.contains(":free") || model.starts_with("openrouter") {
        DEFAULT_MODEL.to_string()
    } else {
        model.to_string()
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
    let cleaned = strip_citations(&text);
    let trimmed = cleaned.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Groq'un arama atıf işaretlerini (`【1†L4-L6】`) kaldırır; Discord'da anlamsız görünürler.
fn strip_citations(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('【') {
        out.push_str(&rest[..start]);
        match rest[start..].find('】') {
            Some(end) => rest = &rest[start + end + '】'.len_utf8()..],
            None => return out + &rest[start..],
        }
    }
    out + rest
}

fn extract_reply(body_text: &str) -> Result<String, AiError> {
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
        Some(text) => Ok(truncate_chars(&text, MAX_REPLY_CHARS)),
        None => {
            if let Some(refusal) = message.and_then(|m| m.refusal.as_deref())
                && !refusal.trim().is_empty()
            {
                return Ok(truncate_chars(refusal.trim(), MAX_REPLY_CHARS));
            }
            if choice.finish_reason.as_deref() == Some("length") {
                Err(AiError::Truncated)
            } else {
                Err(AiError::Empty)
            }
        }
    }
}

/// Çağrı yapılacak sağlayıcı ve anahtarı.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Provider<'a> {
    groq: bool,
    key: &'a str,
}

impl Provider<'_> {
    fn name(&self) -> &'static str {
        if self.groq { "Groq" } else { "Gemini" }
    }

    fn endpoint(&self) -> &'static str {
        if self.groq {
            GROQ_ENDPOINT
        } else {
            GEMINI_ENDPOINT
        }
    }
}

/// Deneme sırası: metin önce Groq'a, görselli istek önce Gemini'ye gider (Groq modelleri görsel
/// okuyamaz). Anahtarı olmayan sağlayıcı atlanır.
fn chain<'a>(keys: &Keys<'a>, has_images: bool) -> Vec<Provider<'a>> {
    let groq = keys.groq.map(|key| Provider { groq: true, key });
    let gemini = keys.gemini.map(|key| Provider { groq: false, key });
    let order = if has_images {
        [gemini, groq]
    } else {
        [groq, gemini]
    };
    order.into_iter().flatten().collect()
}

/// Bir HTTP çağrısı: durum kodunu ve gövdeyi türlü hataya çevirir.
async fn send(
    client: &reqwest::Client,
    provider: &Provider<'_>,
    payload: &serde_json::Value,
) -> Result<String, AiError> {
    let response = client
        .post(provider.endpoint())
        .bearer_auth(provider.key.trim())
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

    // Hata gövdesi `{"error": {...}}` (Groq) ya da `[{"error": {...}}]` (Gemini) olabilir; değilse
    // (ör. ara vekil sunucunun HTML sayfası) yalnızca durum kodundan sınıflandırılır.
    let trimmed = body.trim_start();
    let parsed = serde_json::from_str::<ChatCompletionResponse>(trimmed)
        .ok()
        .and_then(|parsed| parsed.error)
        .or_else(|| {
            serde_json::from_str::<Vec<ChatCompletionResponse>>(trimmed)
                .ok()?
                .into_iter()
                .find_map(|parsed| parsed.error)
        });
    Err(match parsed {
        Some(err) => err.into_error(status.as_u16()),
        None => AiError::from_code(status.as_u16(), preview(&body)),
    })
}

/// Sağlayıcıya özel istek gövdesi. Gövde yalnızca bir `system` ve bir `user` mesajı içerir;
/// kullanıcı kimliği (`user` alanı) hiçbir zaman eklenmez.
fn build_payload(
    groq: bool,
    model: &str,
    system_prompt: &str,
    user_prompt: &str,
    images: &[ImageInput],
) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "model": model,
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": user_content(user_prompt, images) },
        ],
        "max_tokens": MAX_COMPLETION_TOKENS,
        "temperature": TEMPERATURE,
        "stream": false,
    });
    if groq {
        // Groq modeli güncel bilgi gerektiğinde kendi başına arar. Düşünme kısa tutulur ve
        // yanıta konmaz: arama turları dakikalık token sınırının büyük kısmını harcar.
        payload["tools"] = serde_json::json!([{ "type": "browser_search" }]);
        payload["reasoning_effort"] = "low".into();
        payload["include_reasoning"] = false.into();
    }
    payload
}

/// Kullanıcı mesajına laf sokucu / esprili yanıt üretir.
///
/// Sıra için bkz. [`chain`]; herhangi bir hatada sıradaki sağlayıcı denenir.
pub async fn query(
    client: &reqwest::Client,
    keys: &Keys<'_>,
    prompt: &ChatPrompt<'_>,
) -> Result<String, AiError> {
    let _permit = AI_IN_FLIGHT
        .clone()
        .try_acquire_owned()
        .map_err(|_| AiError::Busy)?;

    let groq_model = normalize_model(prompt.model);
    if groq_model.chars().count() > MAX_MODEL_CHARS || groq_model.chars().any(char::is_whitespace) {
        return Err(AiError::InvalidModel(preview(&groq_model)));
    }

    let system_prompt = compose_system_prompt(prompt.system_prompt, prompt.conversation);
    let conversation = prompt.conversation;
    let chain = chain(keys, !conversation.images.is_empty());
    if chain.is_empty() {
        return Err(AiError::Auth("API anahtarı tanımlı değil".to_string()));
    }

    let mut last_error = None;
    for provider in chain {
        // Groq modelleri görsel okuyamaz: görseller çıkarılır ve istem bunu söyler; yoksa model
        // eklenmemiş görseli anlatmaya kalkar.
        let (model, conversation) = if provider.groq {
            (groq_model.as_str(), without_images(conversation))
        } else {
            (GEMINI_MODEL, conversation.clone())
        };
        let user_prompt = compose_user_prompt(prompt.user_prompt, &conversation);
        let payload = build_payload(
            provider.groq,
            model,
            &system_prompt,
            &user_prompt,
            &conversation.images,
        );
        info!(
            "{} isteği gönderiliyor (Model: {model})...",
            provider.name()
        );
        match send(client, &provider, &payload).await {
            Ok(reply) => return Ok(reply),
            Err(error) => {
                // Anahtar/model hataları sessizce yedeğe düşüp haftalarca fark edilmeden kalmasın.
                if matches!(error, AiError::Auth(_) | AiError::NoProvider(_)) {
                    error!("{} yapılandırma hatası: {error}", provider.name());
                } else {
                    warn!("{} isteği başarısız: {error}", provider.name());
                }
                last_error = Some(error);
            }
        }
    }
    Err(last_error.unwrap_or(AiError::Empty))
}

/// Görselleri çıkarılmış bağlam; çıkarılanlar "eklenemedi" olarak sayılır.
fn without_images(conversation: &Conversation) -> Conversation {
    Conversation {
        images: Vec::new(),
        skipped_images: conversation.skipped_images + conversation.images.len(),
        ..conversation.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(body: &str) -> String {
        extract_reply(body).unwrap()
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

        assert_eq!(text(json_str), "Aferin sana, yine başardın!");
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
        let rate = r#"{"error": {"code": "rate_limit_exceeded", "message": "Rate limit reached"}}"#;
        assert!(matches!(extract_reply(rate), Err(AiError::RateLimited(_))));
        let missing = r#"{"error": {"code": "model_not_found", "message": "no such model"}}"#;
        assert!(matches!(
            extract_reply(missing),
            Err(AiError::Unavailable(_))
        ));
        let moderated = r#"{"error": {"code": "403", "message": "flagged"}}"#;
        assert!(matches!(extract_reply(moderated), Err(AiError::Blocked(_))));
    }

    #[test]
    fn leading_whitespace_is_tolerated() {
        // Sağlayıcı uzun süren isteklerde bağlantıyı açık tutmak için boşluk gönderebilir.
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
    fn groq_request_enables_search_and_gemini_does_not() {
        let groq = build_payload(true, "m", "s", "u", &[]);
        assert_eq!(
            groq["tools"],
            serde_json::json!([{ "type": "browser_search" }])
        );
        assert_eq!(groq["reasoning_effort"], "low");
        assert_eq!(groq["include_reasoning"], false);
        let gemini = build_payload(false, "m", "s", "u", &[]);
        assert!(gemini.get("tools").is_none() && gemini.get("reasoning_effort").is_none());
        for body in [&groq, &gemini] {
            assert_eq!(body["stream"], false);
            assert_eq!(body["messages"][1]["role"], "user");
            // En az veri: istek kullanıcı kimliği taşımaz.
            assert!(body.get("user").is_none());
        }
    }

    #[test]
    fn gemini_error_body_is_an_array() {
        let body =
            r#"[{"error": {"code": 429, "message": "quota", "status": "RESOURCE_EXHAUSTED"}}]"#;
        let parsed: Vec<ChatCompletionResponse> = serde_json::from_str(body).unwrap();
        let err = parsed.into_iter().find_map(|p| p.error).unwrap();
        assert!(matches!(err.into_error(500), AiError::RateLimited(_)));
    }

    #[test]
    fn citations_are_stripped() {
        assert_eq!(
            strip_citations("Dolar 50 TL【1†L4-L6】 oldu."),
            "Dolar 50 TL oldu."
        );
        assert_eq!(strip_citations("a【1†L1】b【2†L2】"), "ab");
        assert_eq!(strip_citations("açık【1†L1"), "açık【1†L1");
        assert_eq!(
            text(r#"{"choices":[{"message":{"content":"Merhaba【3†L1-L2】"}}]}"#),
            "Merhaba"
        );
    }

    #[test]
    fn test_normalize_model() {
        assert_eq!(normalize_model("  "), DEFAULT_MODEL);
        assert_eq!(normalize_model("openai/gpt-oss-20b"), "openai/gpt-oss-20b");
        assert_eq!(normalize_model("\"qwen/qwen3.8-27b\""), "qwen/qwen3.8-27b");
        // Kayıtlı eski OpenRouter ayarları varsayılana döner.
        assert_eq!(normalize_model("google/gemma-4-31b-it:free"), DEFAULT_MODEL);
        assert_eq!(normalize_model("openrouter/free"), DEFAULT_MODEL);
    }

    fn keys(groq: bool, gemini: bool) -> Keys<'static> {
        Keys {
            groq: groq.then_some("g"),
            gemini: gemini.then_some("m"),
        }
    }

    fn order(keys: &Keys<'_>, has_images: bool) -> Vec<&'static str> {
        chain(keys, has_images).iter().map(Provider::name).collect()
    }

    #[test]
    fn providers_are_ordered_by_request_kind() {
        assert_eq!(order(&keys(true, true), false), ["Groq", "Gemini"]);
        assert_eq!(order(&keys(true, true), true), ["Gemini", "Groq"]);
        assert_eq!(order(&keys(true, false), true), ["Groq"]);
        assert_eq!(order(&keys(false, true), false), ["Gemini"]);
        assert!(order(&keys(false, false), false).is_empty());
    }

    #[test]
    fn dropped_images_are_not_claimed_as_attached() {
        let conversation = Conversation {
            images: vec![ImageInput {
                data_url: "data:image/webp;base64,AA==".to_string(),
                from_reply: false,
            }],
            ..Conversation::default()
        };
        let text_only = without_images(&conversation);
        assert!(text_only.images.is_empty());
        assert_eq!(text_only.skipped_images, 1);
        let prompt = compose_user_prompt("bu ne?", &text_only);
        assert!(!prompt.contains("isteğe ekli") && prompt.contains("1 görsel eklenemedi"));
    }
}
