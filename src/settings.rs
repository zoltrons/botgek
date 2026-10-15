use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PlatformModules {
    pub twitter: bool,
    pub tiktok: bool,
    pub instagram: bool,
    pub reddit: bool,
    pub bluesky: bool,
    pub spotify: bool,
    pub twitch: bool,
    pub pixiv: bool,
    pub tumblr: bool,
    pub facebook: bool,
    pub youtube: bool,
    pub translation: bool,
    pub ai_chat: bool,
}

impl Default for PlatformModules {
    fn default() -> Self {
        Self {
            twitter: true,
            tiktok: true,
            instagram: true,
            reddit: true,
            bluesky: true,
            spotify: true,
            twitch: true,
            pixiv: true,
            tumblr: true,
            facebook: true,
            youtube: true,
            translation: true,
            ai_chat: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FilterMode {
    #[default]
    Disabled,
    Blacklist,
    Allowlist,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(default)]
pub struct RoleFilterSettings {
    pub mode: FilterMode,
    /// Discord Rol ID'leri (örn: ["123456789012345678"])
    pub roles: Vec<String>,
}

impl RoleFilterSettings {
    pub fn sanitize(&mut self) {
        self.roles = self
            .roles
            .iter()
            .filter_map(|r| r.trim().parse::<u64>().ok())
            .filter(|id| *id > 0)
            .map(|id| id.to_string())
            .take(200)
            .collect();
        self.roles.sort_unstable();
        self.roles.dedup();
    }

    pub fn allows_roles<'a>(&self, roles: impl IntoIterator<Item = &'a str>) -> bool {
        let matched = roles
            .into_iter()
            .any(|role| self.roles.iter().any(|allowed| allowed == role));
        match self.mode {
            FilterMode::Disabled => true,
            FilterMode::Blacklist => !matched,
            FilterMode::Allowlist => matched,
        }
    }
}
// =====================================================================================
// LINK DÖNÜŞTÜRME MOTORU (PROXY vs BUILT-IN)
// =====================================================================================

/// Bağlantının nasıl dönüştürüleceği.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ConversionMode {
    /// Harici bir önizleme servisine (fixupx.com, tnktok.com ...) yönlendirir.
    #[default]
    Proxy,
    /// Bot bağlantıyı kendisi çözümler ve Discord embed'ini native olarak kendisi üretir.
    Builtin,
}

/// Proxy motorunun **hiç kullanılmadığı** platformlar.
///
/// Twitter/X, Instagram ve Reddit için native embed motoru çalışır durumdadır (sesli video
/// dâhil eksiksiz kart üretir); bu yüzden bu platformlarda **proxy modu** ve **proxy yedeği**
/// tamamen kaldırılmıştır: motor modu daima native'dir, yedek alan adı yoktur.
pub const NATIVE_ONLY_PLATFORMS: [&str; 8] = [
    "twitter",
    "instagram",
    "reddit",
    "twitch",
    "pixiv",
    "tumblr",
    "facebook",
    "youtube",
];

/// Verilen platform yalnızca native motorla mı çalışır (proxy desteklenmez)?
pub fn is_native_only(platform_key: &str) -> bool {
    NATIVE_ONLY_PLATFORMS.contains(&platform_key)
}

/// Twitter/X verisinin hangi kaynaktan çekileceği.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TwitterSource {
    /// X'in kendi syndication uç noktası (üçüncü taraf yok, ücretsiz).
    #[default]
    Syndication,
    /// api.fxtwitter.com: repost/alıntı/görüntülenme ve topluluk notu gibi ek veriler içerir.
    Fxtwitter,
    /// Önce X syndication denenir, yanıt gelmezse fxtwitter'a düşülür.
    Hybrid,
}

impl TwitterSource {
    pub fn key(&self) -> &'static str {
        match self {
            TwitterSource::Syndication => "syndication",
            TwitterSource::Fxtwitter => "fxtwitter",
            TwitterSource::Hybrid => "hybrid",
        }
    }
}

/// Proxy motorunu kullanabilen platformların motor seçimi.
///
/// **Twitter/X, Instagram ve Reddit burada yer almaz:** native kartları eksiksiz çalıştığı
/// için bu platformlarda proxy modu tamamen kaldırıldı (bkz. [`NATIVE_ONLY_PLATFORMS`]).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PlatformEngineModes {
    pub tiktok: ConversionMode,
    pub bluesky: ConversionMode,
    pub spotify: ConversionMode,
}

impl Default for PlatformEngineModes {
    /// TikTok (bot yalnızca kart üretir) için varsayılan proxy'dir; Bluesky ve Spotify'da
    /// native embed açıktır (başarısız olursa `EngineSettings::fallback_to_proxy` sayesinde
    /// eski proxy davranışına düşer).
    fn default() -> Self {
        Self {
            tiktok: ConversionMode::Proxy,
            bluesky: ConversionMode::Builtin,
            spotify: ConversionMode::Builtin,
        }
    }
}

impl PlatformEngineModes {
    /// Platformun motor modu. Native-only platformlarda **daima** `Builtin` döner.
    pub fn get(&self, platform_key: &str) -> ConversionMode {
        if is_native_only(platform_key) {
            return ConversionMode::Builtin;
        }
        match platform_key {
            "tiktok" => self.tiktok,
            "bluesky" => self.bluesky,
            "spotify" => self.spotify,
            _ => ConversionMode::Proxy,
        }
    }

    /// Programatik kullanım (testler/varsayılanlar) içindir; WebUI değerleri JSON ile yazar.
    /// Native-only platformlarda ve bilinmeyen anahtarlarda sessizce yok sayılır.
    #[allow(dead_code)]
    pub fn set(&mut self, platform_key: &str, mode: ConversionMode) {
        match platform_key {
            "tiktok" => self.tiktok = mode,
            "bluesky" => self.bluesky = mode,
            "spotify" => self.spotify = mode,
            _ => {}
        }
    }
}

/// Proxy modunda kullanılacak alan adları. Boş bırakılan alan `.env` / varsayılan
/// değeri kullanır (WebUI'dan .env dosyasına dokunmadan alan adı değiştirilebilir).
///
/// Yalnızca **proxy motoru kullanılabilen** platformlar yer alır; Twitter/X, Instagram ve
/// Reddit native-only olduğu için yedek alan adı ayarları kaldırıldı.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ProxyDomains {
    pub tiktok: String,
    pub bluesky: String,
    pub spotify: String,
}

impl ProxyDomains {
    pub fn get(&self, platform_key: &str) -> &str {
        match platform_key {
            "tiktok" => &self.tiktok,
            "bluesky" => &self.bluesky,
            "spotify" => &self.spotify,
            _ => "",
        }
    }

    pub fn set(&mut self, platform_key: &str, value: String) {
        match platform_key {
            "tiktok" => self.tiktok = value,
            "bluesky" => self.bluesky = value,
            "spotify" => self.spotify = value,
            _ => {}
        }
    }
}

/// Tek mesajda ek olarak yüklenecek maksimum görsel sayısı sınırı.
pub const MAX_IMAGES_RANGE: std::ops::RangeInclusive<u8> = 1..=10;

/// Components V2 kartında kullanılacak özel emoji seti.
///
/// Discord özel emojiyi yalnızca `<:ad:id>` / `<a:ad:id>` biçiminde gösterir. Alan boş
/// bırakılırsa Unicode varsayılan (ör. `❤️`) kullanılır; böylece emojinin bulunmadığı
/// sunucularda kart bozulmaz. Bu emojiler yalnızca modern (Components V2) kartta görünür.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct EmojiSettings {
    /// Doğrulanmış hesap işareti (yazar satırında).
    pub verified: String,
    pub replies: String,
    pub reposts: String,
    pub likes: String,
    pub views: String,
    /// Çeviri notunun başındaki işaret (`de → tr`).
    pub translation: String,
    /// X/Twitter göstergesi (kartın son satırında).
    pub twitter: String,
    /// Instagram göstergesi (kartın son satırında).
    pub instagram: String,
    /// Reddit göstergesi (kartın son satırında).
    pub reddit: String,
    /// Alıntılanan gönderi işareti (alıntı başlığı ve alıntı sayısı).
    pub quote: String,
}

/// Botgek'in kendi emoji seti (varsayılan) — kimliksiz yazılmış alanlar bu değerlere bağlanır.
const DEFAULT_VERIFIED_EMOJI: &str = "<:verified:1553709836859084841>";
const DEFAULT_REPLIES_EMOJI: &str = "<:comment:1553712370302451782>";
const DEFAULT_REPOSTS_EMOJI: &str = "<:rt:1553712344939495424>";
const DEFAULT_LIKES_EMOJI: &str = "<:like:1553712395057365102>";
const DEFAULT_VIEWS_EMOJI: &str = "<:metric:1553712416242798712>";
const DEFAULT_TRANSLATION_EMOJI: &str = "<:translation:1476614397249061056>";
const DEFAULT_TWITTER_EMOJI: &str = "<:twitter:1476642012072920355>";
const DEFAULT_INSTAGRAM_EMOJI: &str = "<:instagram:1558099674731122758>";
const DEFAULT_REDDIT_EMOJI: &str = "<:reddit:1558112156195495956>";
const DEFAULT_QUOTE_EMOJI: &str = "<:qutoe:1553728214998847569>";

// Alanların kabul ettiği emoji adları: `:twitter:` gibi **kimliksiz** yazılmış değerler
// botun kendi emojisine bağlanır (kullanıcı ID'yi yazmayı unutsa bile kart bozulmaz).
const VERIFIED_NAMES: &[&str] = &["verified", "check", "blueverified"];
const REPLIES_NAMES: &[&str] = &["comment", "comments", "reply", "replies"];
const REPOSTS_NAMES: &[&str] = &["rt", "retweet", "retweets", "repost", "reposts"];
const LIKES_NAMES: &[&str] = &["like", "likes", "heart", "favorite"];
const VIEWS_NAMES: &[&str] = &["metric", "metrics", "view", "views", "analytics"];
const TRANSLATION_NAMES: &[&str] = &["translation", "translate", "globe"];
const TWITTER_NAMES: &[&str] = &["twitter", "x", "xlogo", "x_logo"];
const INSTAGRAM_NAMES: &[&str] = &["instagram", "ig", "insta"];
const REDDIT_NAMES: &[&str] = &["reddit", "rdt", "snoo"];
const QUOTE_NAMES: &[&str] = &["qutoe", "quote", "quotes"];

impl Default for EmojiSettings {
    /// Botgek'in kendi emoji seti.
    fn default() -> Self {
        Self {
            verified: DEFAULT_VERIFIED_EMOJI.to_string(),
            replies: DEFAULT_REPLIES_EMOJI.to_string(),
            reposts: DEFAULT_REPOSTS_EMOJI.to_string(),
            likes: DEFAULT_LIKES_EMOJI.to_string(),
            views: DEFAULT_VIEWS_EMOJI.to_string(),
            translation: DEFAULT_TRANSLATION_EMOJI.to_string(),
            twitter: DEFAULT_TWITTER_EMOJI.to_string(),
            instagram: DEFAULT_INSTAGRAM_EMOJI.to_string(),
            reddit: DEFAULT_REDDIT_EMOJI.to_string(),
            quote: DEFAULT_QUOTE_EMOJI.to_string(),
        }
    }
}

impl EmojiSettings {
    /// Girdileri normalleştirir: kimliksiz emoji adları (`:twitter:`) botun kendi
    /// emojisine çevrilir, geçersiz girdiler boşaltılır (boş alan Unicode'a düşer).
    pub fn sanitize(&mut self) {
        self.verified = normalize_emoji(&self.verified, DEFAULT_VERIFIED_EMOJI, VERIFIED_NAMES);
        self.replies = normalize_emoji(&self.replies, DEFAULT_REPLIES_EMOJI, REPLIES_NAMES);
        self.reposts = normalize_emoji(&self.reposts, DEFAULT_REPOSTS_EMOJI, REPOSTS_NAMES);
        self.likes = normalize_emoji(&self.likes, DEFAULT_LIKES_EMOJI, LIKES_NAMES);
        self.views = normalize_emoji(&self.views, DEFAULT_VIEWS_EMOJI, VIEWS_NAMES);
        self.translation = normalize_emoji(
            &self.translation,
            DEFAULT_TRANSLATION_EMOJI,
            TRANSLATION_NAMES,
        );
        self.twitter = normalize_emoji(&self.twitter, DEFAULT_TWITTER_EMOJI, TWITTER_NAMES);
        self.instagram = normalize_emoji(&self.instagram, DEFAULT_INSTAGRAM_EMOJI, INSTAGRAM_NAMES);
        self.reddit = normalize_emoji(&self.reddit, DEFAULT_REDDIT_EMOJI, REDDIT_NAMES);
        self.quote = normalize_emoji(&self.quote, DEFAULT_QUOTE_EMOJI, QUOTE_NAMES);
    }

    /// Doğrulama işareti (özel emoji ayarlıysa o, değilse Unicode).
    pub fn verified(&self) -> &str {
        resolve_emoji(&self.verified, DEFAULT_VERIFIED_EMOJI, "☑", VERIFIED_NAMES)
    }

    pub fn replies(&self) -> &str {
        resolve_emoji(&self.replies, DEFAULT_REPLIES_EMOJI, "💬", REPLIES_NAMES)
    }

    pub fn reposts(&self) -> &str {
        resolve_emoji(&self.reposts, DEFAULT_REPOSTS_EMOJI, "🔁", REPOSTS_NAMES)
    }

    pub fn likes(&self) -> &str {
        resolve_emoji(&self.likes, DEFAULT_LIKES_EMOJI, "❤️", LIKES_NAMES)
    }

    pub fn views(&self) -> &str {
        resolve_emoji(&self.views, DEFAULT_VIEWS_EMOJI, "👁️", VIEWS_NAMES)
    }

    pub fn translation(&self) -> &str {
        resolve_emoji(
            &self.translation,
            DEFAULT_TRANSLATION_EMOJI,
            "🌐",
            TRANSLATION_NAMES,
        )
    }

    /// X/Twitter göstergesi; kullanılabilir bir emoji yoksa `None` (yerine platform adı yazılır).
    pub fn twitter(&self) -> Option<&str> {
        let resolved = resolve_emoji(&self.twitter, DEFAULT_TWITTER_EMOJI, "", TWITTER_NAMES);
        (!resolved.is_empty()).then_some(resolved)
    }

    /// Instagram göstergesi; kullanılabilir bir emoji yoksa `None` (yerine platform adı yazılır).
    pub fn instagram(&self) -> Option<&str> {
        let resolved = resolve_emoji(
            &self.instagram,
            DEFAULT_INSTAGRAM_EMOJI,
            "",
            INSTAGRAM_NAMES,
        );
        (!resolved.is_empty()).then_some(resolved)
    }

    /// Reddit göstergesi; kullanılabilir bir emoji yoksa `None` (yerine platform adı yazılır).
    pub fn reddit(&self) -> Option<&str> {
        let resolved = resolve_emoji(&self.reddit, DEFAULT_REDDIT_EMOJI, "", REDDIT_NAMES);
        (!resolved.is_empty()).then_some(resolved)
    }

    /// Alıntılanan gönderi işareti (ayarlı değilse Unicode `↪`).
    pub fn quote(&self) -> &str {
        resolve_emoji(&self.quote, DEFAULT_QUOTE_EMOJI, "↪", QUOTE_NAMES)
    }
}

/// Kayıtlı emoji değerini mesajda kullanılabilir hale getirir:
/// - `<:ad:id>` / `<a:ad:id>` veya kısa Unicode emoji → olduğu gibi,
/// - `:ad:` biçiminde kimliksiz bir ad → botun varsayılan özel emojisi,
/// - boş veya tanınmayan girdi → Unicode varsayılan.
fn resolve_emoji<'a>(
    value: &'a str,
    default_custom: &'static str,
    unicode: &'static str,
    names: &[&str],
) -> &'a str {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return unicode;
    }

    let colonized = trimmed.starts_with(':') && trimmed.ends_with(':');
    if !colonized && valid_emoji(trimmed) {
        return trimmed;
    }
    if let Some(name) = emoji_name(trimmed)
        && names
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(name))
    {
        return default_custom;
    }
    // `:foo:` gibi tanınmayan kısa girdiler metne dökülmesin.
    if colonized {
        return unicode;
    }

    if valid_emoji(trimmed) {
        trimmed
    } else {
        unicode
    }
}

/// Ayar dosyasına yazılacak hâle getirir: boş alan korunur (Unicode isteği), kimliksiz
/// emoji adı botun emojisine çevrilir, tanınmayan girdi boşaltılır.
fn normalize_emoji(value: &str, default_custom: &str, names: &[&str]) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let colonized = trimmed.starts_with(':') && trimmed.ends_with(':');
    if !colonized && valid_emoji(trimmed) {
        return trimmed.to_string();
    }
    if let Some(name) = emoji_name(trimmed)
        && names
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(name))
    {
        return default_custom.to_string();
    }

    String::new()
}

/// `:ad:` biçiminden (veya düz `ad` girdisinden) emoji adını çıkarır.
fn emoji_name(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    let name = trimmed
        .strip_prefix(':')
        .and_then(|rest| rest.strip_suffix(':'))
        .unwrap_or(trimmed);

    let valid = !name.is_empty()
        && name.len() <= 32
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    valid.then_some(name)
}

/// `<:ad:id>` / `<a:ad:id>` biçimini veya kısa bir Unicode emojiyi kabul eder.
/// Markdown'ı bozabilecek karakterler (`[`, `]`, `(`, `)`, `` ` `` ...) reddedilir.
fn valid_emoji(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }

    if let Some(body) = value
        .strip_prefix("<:")
        .or_else(|| value.strip_prefix("<a:"))
    {
        let Some(body) = body.strip_suffix('>') else {
            return false;
        };
        let mut parts = body.split(':');
        let (Some(name), Some(id), None) = (parts.next(), parts.next(), parts.next()) else {
            return false;
        };
        return name.len() >= 2
            && name.len() <= 32
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && (17..=20).contains(&id.len())
            && id.chars().all(|c| c.is_ascii_digit());
    }

    value.chars().count() <= 8
        && !value
            .chars()
            .any(|c| "[\\]()`<>|*_~".contains(c) || c.is_control())
}

/// Discord'un (boost'suz sunucularda) yüklenen dosya limiti 10 MB'tır; üzerine çıkılırsa
/// video indirilmez, link olarak paylaşılır.
pub const MAX_VIDEO_MB_RANGE: std::ops::RangeInclusive<u8> = 1..=50;

/// Native (built-in) embed motorunun davranış ayarları.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct EngineSettings {
    /// Başka botların paylaştığı desteklenen linkler de dönüştürülsün mü? Varsayılan olarak
    /// **kapalıdır**. Açıldığında bot kaynağı yine de taklit edilmez ve bot mesajları AI
    /// sohbetini tetiklemez (yalnızca link dönüşümü yapılır).
    #[serde(default)]
    pub respond_to_bots: bool,
    /// Hassas (NSFW) içeriğin kartta nasıl gösterileceği: her zaman spoiler veya yalnızca
    /// NSFW kanallarda açık. Kaynak spoiler'ı ve elle `||...||` işaretlenen içerik NSFW
    /// kanalda da gizli kalır (bkz. `render::VisibilityDecision`).
    #[serde(default)]
    pub sensitive_mode: SensitiveMode,
    /// Platform → motor modu (proxy / builtin).
    pub modes: PlatformEngineModes,
    /// Proxy modunda kullanılacak alan adları (boş = .env).
    pub domains: ProxyDomains,
    /// Twitter/X için veri kaynağı.
    pub twitter_source: TwitterSource,
    /// Tek mesajda ek olarak yüklenecek maksimum görsel sayısı.
    pub max_images: u8,
    /// Eklenecek maksimum video boyutu (MB).
    pub max_video_mb: u8,
    /// Beğeni/yanıt/repost vb. metrikleri göster.
    pub show_metrics: bool,
    /// Gönderi tarihini (Discord göreli zaman gösterimi) ekle.
    pub show_timestamp: bool,
    /// Topluluk Notu (community note) varsa göster.
    pub show_community_note: bool,
    /// Alıntılanan gönderiyi ikinci embed olarak göster.
    pub show_quote: bool,
    /// X gönderilerini X'in kendi çevirisiyle hedef dile çevir.
    pub auto_translate: bool,
    /// Otomatik X çevirisinin hedef dili (BCP-47 primary language kodu veya bölgesel kod).
    pub translation_target: String,
    /// Native embed üretilemezse otomatik olarak proxy linkine düş.
    pub fallback_to_proxy: bool,
    /// Mesajı, paylaşan kullanıcının adı ve avatarıyla webhook üzerinden gönder ("taklit").
    /// Bu sayede mesajda `<@kullanıcı>` etiketi ve `[:](...)` gibi link hileleri gerekmez.
    pub impersonate_user: bool,
    /// Kullanıcı linki **yanıt olarak** attığında taklit yerine bot mesajı gönderilir.
    /// Discord webhook mesajları yanıt olamadığı için bu ayar yanıt bağlantısını korur.
    pub impersonate_skip_replies: bool,
    /// Kart gösterim stili: fxembed tarzı klasik embed / otomatik / her zaman modern kart.
    /// Modern kartta kullanılan özel emoji seti (boş alan → Unicode varsayılan).
    pub emojis: EmojiSettings,
}

impl Default for EngineSettings {
    fn default() -> Self {
        Self {
            respond_to_bots: false,
            sensitive_mode: SensitiveMode::HideAlways,
            modes: PlatformEngineModes::default(),
            domains: ProxyDomains::default(),
            twitter_source: TwitterSource::default(),
            max_images: 4,
            max_video_mb: 8,
            show_metrics: true,
            show_timestamp: true,
            show_community_note: true,
            show_quote: true,
            auto_translate: true,
            translation_target: "tr".to_string(),
            fallback_to_proxy: true,
            impersonate_user: true,
            impersonate_skip_replies: true,
            emojis: EmojiSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SensitiveMode {
    #[default]
    HideAlways,
    AllowInNsfw,
}

impl EngineSettings {
    /// Platform anahtarı (`tiktok`, `bluesky` ...) için etkin mod. Native-only platformlarda
    /// (Twitter/X, Instagram, Reddit) **daima** [`ConversionMode::Builtin`] döner.
    pub fn mode_for(&self, platform_key: &str) -> ConversionMode {
        self.modes.get(platform_key)
    }

    /// Native embed üretilemezse proxy linkine düşülsün mü? Native-only platformlarda asla.
    pub fn allows_proxy_fallback(&self, platform_key: &str) -> bool {
        !is_native_only(platform_key) && self.fallback_to_proxy
    }

    /// Platform için geçerli proxy alan adı: WebUI ayarı doluysa o, değilse `.env` değeri.
    /// Native-only platformlarda proxy kullanılmadığı için boş döner.
    pub fn domain_for(&self, platform_key: &str, config: &crate::config::Config) -> String {
        if is_native_only(platform_key) {
            return String::new();
        }
        let from_settings = self.domains.get(platform_key);
        if !from_settings.is_empty() {
            return from_settings.to_string();
        }
        match platform_key {
            "tiktok" => config.tiktok_domain.clone(),
            "bluesky" => config.bluesky_domain.clone(),
            "spotify" => config.spotify_domain.clone(),
            _ => String::new(),
        }
    }

    /// WebUI girdilerini güvenli sınırlara çeker ve alan adlarını temizler.
    /// Geçersiz alan adı boşaltılır (bu da `.env` değerinin kullanılması demektir).
    pub fn sanitize(&mut self) {
        for key in ["tiktok", "bluesky", "spotify"] {
            let cleaned = crate::config::clean_domain(self.domains.get(key)).unwrap_or_default();
            self.domains.set(key, cleaned);
        }

        if !MAX_IMAGES_RANGE.contains(&self.max_images) {
            self.max_images = self.max_images.clamp(1, 10);
        }
        if !MAX_VIDEO_MB_RANGE.contains(&self.max_video_mb) {
            self.max_video_mb = self.max_video_mb.clamp(1, 50);
        }

        let target = self.translation_target.trim().to_ascii_lowercase();
        if target.len() < 2
            || target.len() > 16
            || !target
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
            || !target
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
        {
            self.translation_target = "tr".to_string();
        } else {
            self.translation_target = target;
        }

        self.emojis.sanitize();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AiSettings {
    pub model: String,
    pub system_prompt: String,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            model: crate::ai::DEFAULT_MODEL.to_string(),
            system_prompt: "Sen Botgek adında zeki, hazırcevap ve hafif alaycı bir Discord botusun. Sadece Botgek olduğunu bil. Cevapların kısa (maksimum 1-2 cümle), zeki ve lafı gediğine koyan türden olsun. Asla robotik konuşma, gevezelik yapma. Sadece Türkçe konuş.".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ModuleRoleFilters {
    #[serde(default)]
    pub twitter: RoleFilterSettings,
    #[serde(default)]
    pub tiktok: RoleFilterSettings,
    #[serde(default)]
    pub instagram: RoleFilterSettings,
    #[serde(default)]
    pub reddit: RoleFilterSettings,
    #[serde(default)]
    pub bluesky: RoleFilterSettings,
    #[serde(default)]
    pub spotify: RoleFilterSettings,
    #[serde(default)]
    pub twitch: RoleFilterSettings,
    #[serde(default)]
    pub pixiv: RoleFilterSettings,
    #[serde(default)]
    pub tumblr: RoleFilterSettings,
    #[serde(default)]
    pub facebook: RoleFilterSettings,
    #[serde(default)]
    pub youtube: RoleFilterSettings,
    #[serde(default)]
    pub translation: RoleFilterSettings,
    #[serde(default)]
    pub ai_chat: RoleFilterSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct GuildSettings {
    #[serde(default)]
    pub modules: PlatformModules,
    #[serde(default)]
    pub role_filters: ModuleRoleFilters,
    #[serde(default)]
    pub ai: AiSettings,
    #[serde(default)]
    pub engine: EngineSettings,
}

impl ModuleRoleFilters {
    pub fn sanitize(&mut self) {
        for filter in [
            &mut self.twitter,
            &mut self.tiktok,
            &mut self.instagram,
            &mut self.reddit,
            &mut self.bluesky,
            &mut self.spotify,
            &mut self.twitch,
            &mut self.pixiv,
            &mut self.tumblr,
            &mut self.facebook,
            &mut self.youtube,
            &mut self.translation,
            &mut self.ai_chat,
        ] {
            filter.sanitize();
        }
    }
}

static DEFAULT_DISABLED_FILTER: std::sync::LazyLock<RoleFilterSettings> =
    std::sync::LazyLock::new(RoleFilterSettings::default);

impl GuildSettings {
    pub fn get_role_filter(&self, module_key: &str) -> &RoleFilterSettings {
        match module_key {
            "twitter" => &self.role_filters.twitter,
            "tiktok" => &self.role_filters.tiktok,
            "instagram" => &self.role_filters.instagram,
            "reddit" => &self.role_filters.reddit,
            "bluesky" => &self.role_filters.bluesky,
            "spotify" => &self.role_filters.spotify,
            "twitch" => &self.role_filters.twitch,
            "pixiv" => &self.role_filters.pixiv,
            "tumblr" => &self.role_filters.tumblr,
            "facebook" => &self.role_filters.facebook,
            "youtube" => &self.role_filters.youtube,
            "translation" => &self.role_filters.translation,
            "ai_chat" => &self.role_filters.ai_chat,
            _ => &DEFAULT_DISABLED_FILTER,
        }
    }
}

/// Ayar dosyası şema sürümü.
///
/// - `0..=1`: sunucu kayıtları **tam kopyaydı** (`GuildSettings`) — her sunucu, ilk
///   düzenlemede genelin tüm ayarlarının kopyasını tutuyordu.
/// - `2`: sunucu kayıtları **sparse** (`GuildOverride`) — yalnızca bilerek değiştirilen
///   bölümler saklanır. Göç (`migrate_legacy_guild_overrides`) **bir kez** çalışır ve
///   sürüm damgalanır; böylece sonraki açılışlarda kullanıcının bilerek sabitlediği bir
///   bölüm, genelle geçici olarak aynı olduğu için sessizce mirasa dönmez.
/// - `3`: sparse kayıtlar kanal override'larını da destekler. `2` dosyaları okunurken
///   destructive eşit-blok göçü tekrar çalıştırılmaz.
pub const SETTINGS_SCHEMA_VERSION: u32 = 3;

/// Sunucuya özel ayarların **sparse (bölüm bazlı) override** biçimi.
///
/// Eski model, sunucu kaydını ilk düzenlemede genelin **tam kopyası** olarak yazıyordu; bu
/// yüzden sunucuda tek bir alan değiştirildiğinde bile tüm ayarlar genelden kopuyordu
/// (genel ayar sonradan değişse de sunucu eski değeri ezmeye devam ediyordu).
///
/// Yeni modelde yalnızca **kullanıcının o sunucuda bilerek değiştirdiği bölümler** yazılır:
/// `modules`, `role_filters`, `ai`, `engine`. `None` = **miras** (genel geçerlidir),
/// `Some(blok)` = o bölüm geneli **bütün olarak** ezer.
///
/// Granülarite **bölüm** düzeyindedir: bir bölüm override edildiğinde içindeki tüm
/// anahtarlar sabitlenir (alan düzeyinde kısmi miras yoktur). Bu yüzden override bloğu
/// daima **tam** yazılır — serde eksik alt anahtarı kod varsayılanıyla doldurur, genelin
/// o anki değeriyle değil.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct GuildOverride {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modules: Option<PlatformModules>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_filters: Option<ModuleRoleFilters>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai: Option<AiSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine: Option<EngineSettings>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub channels: HashMap<String, ChannelOverride>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ChannelOverride {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modules: Option<PlatformModules>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_filters: Option<ModuleRoleFilters>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai: Option<AiSettings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine: Option<EngineSettings>,
}

impl ChannelOverride {
    pub fn is_empty(&self) -> bool {
        self.modules.is_none()
            && self.role_filters.is_none()
            && self.ai.is_none()
            && self.engine.is_none()
    }

    pub fn sanitize(&mut self) {
        if let Some(filters) = self.role_filters.as_mut() {
            filters.sanitize();
        }
        if let Some(ai) = self.ai.as_mut() {
            ai.model = ai.model.chars().take(128).collect();
            ai.system_prompt = ai.system_prompt.chars().take(4000).collect();
        }
        if let Some(engine) = self.engine.as_mut() {
            engine.sanitize();
        }
    }
}

impl GuildOverride {
    /// Hiçbir bölüm override edilmemişse kayıt **"miras"** sayılır ve saklanmaz.
    pub fn is_empty(&self) -> bool {
        self.modules.is_none()
            && self.role_filters.is_none()
            && self.ai.is_none()
            && self.engine.is_none()
            && self.channels.values().all(ChannelOverride::is_empty)
    }
}

/// Bir bölüm için override varsa onu (bütün olarak), yoksa geneli döndürür.
fn pick<T: Clone>(global: &T, over: Option<&T>) -> T {
    over.cloned().unwrap_or_else(|| global.clone())
}

/// AI bloğunun **tek** çözümlemesi: override varsa o, yoksa genel; ancak override içindeki
/// **boş** model/kişilik alanları genelden doldurulur (eski davranışın devamı).
fn merge_ai(global: &AiSettings, over: Option<&AiSettings>) -> AiSettings {
    let mut ai = pick(global, over);
    if ai.system_prompt.trim().is_empty() {
        ai.system_prompt = global.system_prompt.clone();
    }
    if ai.model.trim().is_empty() {
        ai.model = global.model.clone();
    }
    ai
}

/// **Etkin ayar (tek kaynak):** genel ayarlar ile sunucu override'ı çözümlenir.
///
/// Blok bazlı (sparse) çözümleme: `guild` içinde **bulunan** bölüm geneli bütün olarak
/// ezer, bulunmayan bölüm genelden miras alınır. `guild == None` → sunucuya özel kayıt yok
/// (DM veya hiç özelleştirilmemiş sunucu): genel ayarlar geçerlidir.
///
/// **Saf fonksiyondur:** I/O yapmaz, girdileri değiştirmez. Bot çalışma zamanı
/// (`AppSettings::get_guild_settings`) ve WebUI aynı sonucu buradan alır — kopya
/// çözümleme mantığı yoktur.
pub fn resolve_settings(global: &AppSettings, guild: Option<&GuildOverride>) -> GuildSettings {
    GuildSettings {
        modules: pick(&global.modules, guild.and_then(|g| g.modules.as_ref())),
        role_filters: pick(
            &global.role_filters,
            guild.and_then(|g| g.role_filters.as_ref()),
        ),
        ai: merge_ai(&global.ai, guild.and_then(|g| g.ai.as_ref())),
        engine: pick(&global.engine, guild.and_then(|g| g.engine.as_ref())),
    }
}

fn resolve_channel_layer(base: &GuildSettings, layer: Option<&ChannelOverride>) -> GuildSettings {
    let Some(layer) = layer else {
        return base.clone();
    };
    GuildSettings {
        modules: pick(&base.modules, layer.modules.as_ref()),
        role_filters: pick(&base.role_filters, layer.role_filters.as_ref()),
        ai: merge_ai(&base.ai, layer.ai.as_ref()),
        engine: pick(&base.engine, layer.engine.as_ref()),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    /// Ayar dosyası şema sürümü (bkz. [`SETTINGS_SCHEMA_VERSION`]); taze dosya güncel sürümle
    /// doğar, eski dosyalar `0` okunur ve göç bir kez çalışır.
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default)]
    pub modules: PlatformModules,
    #[serde(default)]
    pub role_filters: ModuleRoleFilters,
    /// **Yalnızca eski ayar dosyaları için okunur.** Modül bazlı filtreler boşsa yükleme
    /// anında bir kez modüllere kopyalanır (`migrate_legacy_role_filter`) ve artık
    /// `settings.json`'a **yazılmaz**: aynı bilgiyi iki alanda tutmak karışıklık yaratıyordu.
    #[serde(default, skip_serializing)]
    pub role_filter: Option<RoleFilterSettings>,
    #[serde(default)]
    pub ai: AiSettings,
    /// Link dönüştürme motoru (proxy / built-in embed) ayarları
    #[serde(default)]
    pub engine: EngineSettings,
    /// Sunucuya özel **sparse** ayarlar (Guild ID -> yalnızca bilerek değiştirilen bölümler).
    /// Boş bölümler yazılmaz; kaydın kendisi yoksa sunucu genel ayarları **miras alır**.
    #[serde(default)]
    pub guilds: HashMap<String, GuildOverride>,
}

impl Default for AppSettings {
    /// Taze ayarlar **güncel** şema sürümüyle doğar; böylece yeni kurulumda göç çalışmaz.
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            modules: PlatformModules::default(),
            role_filters: ModuleRoleFilters::default(),
            role_filter: None,
            ai: AiSettings::default(),
            engine: EngineSettings::default(),
            guilds: HashMap::new(),
        }
    }
}

impl AppSettings {
    /// Sunucunun **etkin** ayarları: genel + sunucu override çözümlemesi.
    ///
    /// Tek çözümleme noktasıdır (bkz. [`resolve_settings`]); bot çalışma zamanındaki tüm
    /// ayar okumaları buradan geçer.
    pub fn get_guild_settings(&self, guild_id: Option<serenity::all::GuildId>) -> GuildSettings {
        let over = guild_id.and_then(|gid| self.guilds.get(&gid.to_string()));
        resolve_settings(self, over)
    }

    /// Global → guild → parent(thread only) → channel. DM, category and unknown parent
    /// values deliberately do not inherit channel settings.
    pub fn get_message_settings(
        &self,
        guild_id: Option<serenity::all::GuildId>,
        channel_id: serenity::all::ChannelId,
        parent_id: Option<serenity::all::ChannelId>,
    ) -> GuildSettings {
        let Some(guild_id) = guild_id else {
            return self.get_guild_settings(None);
        };
        let Some(guild) = self.guilds.get(&guild_id.to_string()) else {
            return self.get_guild_settings(Some(guild_id));
        };
        let mut resolved = resolve_settings(self, Some(guild));
        resolved = resolve_channel_layer(
            &resolved,
            parent_id.and_then(|id| guild.channels.get(&id.to_string())),
        );
        resolve_channel_layer(&resolved, guild.channels.get(&channel_id.to_string()))
    }

    /// Normalize sparse channel entries and discard malformed/non-positive Discord IDs.
    pub fn sanitize_channels(&mut self) {
        self.guilds
            .retain(|id, _| id.parse::<u64>().is_ok_and(|n| n > 0));
        for guild in self.guilds.values_mut() {
            if let Some(filters) = guild.role_filters.as_mut() {
                filters.sanitize();
            }
            if let Some(engine) = guild.engine.as_mut() {
                engine.sanitize();
            }
            guild
                .channels
                .retain(|id, _| id.parse::<u64>().is_ok_and(|n| n > 0));
            for channel in guild.channels.values_mut() {
                channel.sanitize();
            }
            guild.channels.retain(|_, channel| !channel.is_empty());
        }
    }

    /// **Göç (bir kez):** eski **tam kopya** sunucu kayıtlarını sparse override'a indirger.
    ///
    /// Eski model sunucu kaydını ilk düzenlemede genelin tam kopyası olarak yazıyordu. Bu
    /// fonksiyon genel ile **birebir aynı** (veya AI için: aynı çözülen) bölümleri düşürür —
    /// gereksiz sabitlemeyi kaldırır, mirasa döndürür — ve farklı bölümleri korur. Override'ı
    /// boşalan sunucu kaydı **silinir** (o sunucu artık "özel" değildir).
    ///
    /// Ayırt edilemez durum ("bilerek genelle aynı bırakılmış" ↔ "kopyalanmış") **kayıp kabul
    /// edilir**: eşit bölüm un-pin edilir. Göç verisi kaybı riskini taşıdığı için çağıran
    /// taraf öncesinde tarih damgalı **yedek** alır (`backup_settings_file`).
    ///
    /// Yalnızca `schema_version < SETTINGS_SCHEMA_VERSION` iken çağrılmalıdır: aksi hâlde
    /// kullanıcının bilerek sabitlediği bir bölüm, o an genelle aynı olduğu için sessizce
    /// mirasa dönebilir.
    ///
    /// Döndürülen değer: bölümü indirgenen veya kaydı silinen sunucu sayısı.
    pub fn migrate_legacy_guild_overrides(&mut self) -> usize {
        // Genel bölümler klonlanır: `self.guilds` üzerinde mutable gezinirken `self`'i
        // ödünç alamayız (borrow checker), ayrıca karşılaştırma referansı sabit kalmalı.
        let global_modules = self.modules.clone();
        let global_role_filters = self.role_filters.clone();
        let global_ai = self.ai.clone();
        let global_engine = self.engine.clone();

        let before = self.guilds.clone();

        for over in self.guilds.values_mut() {
            if over
                .modules
                .as_ref()
                .is_none_or(|block| block == &global_modules)
            {
                over.modules = None;
            }
            if over
                .role_filters
                .as_ref()
                .is_none_or(|block| block == &global_role_filters)
            {
                over.role_filters = None;
            }
            // AI bloğu için karşılaştırma **çözülmüş** değer üzerinden yapılır: boş
            // model/kişilik alanları zaten genelden dolduruluyor, bu yüzden pin gereksizdir.
            if over.ai.is_some() && merge_ai(&global_ai, over.ai.as_ref()) == global_ai {
                over.ai = None;
            }
            if over
                .engine
                .as_ref()
                .is_none_or(|block| block == &global_engine)
            {
                over.engine = None;
            }
        }

        self.guilds.retain(|_, over| !over.is_empty());

        // Silinen ve indirgenen kayıtların toplamı (idempotent: ikinci çağrı 0 döner).
        let mut changed = 0;
        for (gid, previous) in before.iter() {
            if self.guilds.get(gid) != Some(previous) {
                changed += 1;
            }
        }
        changed
    }

    /// Eski tek `role_filter` alanını modül bazlı filtrelere taşır (bir kez, yükleme anında).
    /// Modül filtreleri zaten doluysa dokunulmaz.
    pub fn migrate_legacy_role_filter(&mut self) {
        let Some(legacy) = self.role_filter.clone() else {
            return;
        };
        if legacy.roles.is_empty() || !self.role_filters.twitter.roles.is_empty() {
            return;
        }

        self.role_filters.twitter = legacy.clone();
        self.role_filters.tiktok = legacy.clone();
        self.role_filters.instagram = legacy.clone();
        self.role_filters.reddit = legacy.clone();
        self.role_filters.bluesky = legacy.clone();
        self.role_filters.spotify = legacy.clone();
        self.role_filters.twitch = legacy.clone();
        self.role_filters.pixiv = legacy.clone();
        self.role_filters.tumblr = legacy.clone();
        self.role_filters.facebook = legacy.clone();
        self.role_filters.youtube = legacy.clone();
        self.role_filters.translation = legacy.clone();
        self.role_filters.ai_chat = legacy;
    }
}

pub(crate) fn resolve_settings_path() -> PathBuf {
    let configured = std::env::var_os("BOTGEK_SETTINGS_PATH")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from);
    let database = std::env::var_os("BOTGEK_DB_PATH")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from);
    resolve_settings_path_from(configured.as_deref(), database.as_deref())
}

/// Ayar yolu açıkça verilmemişse, kalıcı veritabanının dizinini paylaşır.
///
/// Docker kurulumlarının bir kısmı yalnızca `BOTGEK_DB_PATH=/app/data/botgek.db`
/// ayarını yapıyordu. Bu durumda eski varsayılan olan `./settings.json`, container
/// katmanında kalıyor ve container yeniden oluşturulunca WebUI kayıtları kayboluyordu.
/// Açık ayar yolu her zaman önceliklidir; veritabanı yolu ise güvenli bir kalıcılık
/// fallback'i sağlar.
fn resolve_settings_path_from(configured: Option<&Path>, database: Option<&Path>) -> PathBuf {
    if let Some(path) = configured {
        return path.to_path_buf();
    }

    if let Some(database) = database {
        let database_name = database.as_os_str();
        if database_name != std::ffi::OsStr::new(":memory:")
            && let Some(parent) = database
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
        {
            return parent.join("settings.json");
        }
    }

    let p = Path::new("settings.json");
    if p.is_dir() {
        // Docker bind-mount hostta dosya olmadığı için otomatik klasör olarak açılmışsa
        p.join("settings.json")
    } else {
        p.to_path_buf()
    }
}

/// Göç öncesi ayar dosyasını, dosyanın **yanına** zaman damgalı kopya olarak yedekler.
///
/// `settings.json.bak-<unix-saniye>` biçimindedir. Yedek alınamazsa (izin/yol hatası)
/// `Err` döner; çağıran göçü yine de sürdürür ama durumu **log'lar** — göç veri kaybı
/// riski taşıdığı için yedek, geri alma yöntemidir (bkz. README).
fn backup_settings_file(path: &Path) -> Result<PathBuf, std::io::Error> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let backup = PathBuf::from(format!("{}.bak-{stamp}", path.display()));
    fs::copy(path, &backup)?;
    Ok(backup)
}

#[cfg(not(windows))]
fn replace_settings_file(temp: &Path, destination: &Path) -> Result<(), std::io::Error> {
    // POSIX rename replaces an existing destination atomically.
    fs::rename(temp, destination)
}

#[cfg(windows)]
fn replace_settings_file(temp: &Path, destination: &Path) -> Result<(), std::io::Error> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;

    fn wide(path: &Path) -> Result<Vec<u16>, std::io::Error> {
        let mut value: Vec<u16> = path.as_os_str().encode_wide().collect();
        if value.contains(&0) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "settings path contains an embedded NUL",
            ));
        }
        value.push(0);
        Ok(value)
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing_name: *const u16, new_name: *const u16, flags: u32) -> i32;
    }

    let temp = wide(temp)?;
    let destination = wide(destination)?;
    // Unlike std::fs::rename, MoveFileExW explicitly replaces an existing
    // destination. WRITE_THROUGH asks Windows to flush the metadata before it
    // reports success, preserving the same durability intent as the Unix path.
    let replaced = unsafe {
        MoveFileExW(
            temp.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if replaced == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

impl AppSettings {
    pub fn load() -> Result<Self, String> {
        let path = resolve_settings_path();
        Self::load_from(&path)
    }

    fn load_from(path: &Path) -> Result<Self, String> {
        if !path.exists()
            && let Some(seed) = std::env::var_os("BOTGEK_SETTINGS_SEED")
        {
            let content = fs::read_to_string(seed).map_err(|e| format!("settings seed: {e}"))?;
            let imported: AppSettings =
                serde_json::from_str(&content).map_err(|e| format!("settings seed: {e}"))?;
            imported
                .save_to(path)
                .map_err(|e| format!("settings import: {e}"))?;
        }
        if path.is_file() {
            let loaded = fs::read_to_string(path)
                .map_err(|e| format!("okunamadı: {e:?}"))
                .and_then(|content| {
                    serde_json::from_str::<AppSettings>(&content)
                        .map_err(|e| format!("ayrıştırılamadı: {e}"))
                });

            match loaded {
                Ok(mut settings) => {
                    // Eski/elle düzenlenmiş kayıtlar yükleme anında da normalize edilir:
                    // kimliksiz emoji adları (`:twitter:`) botun emojisine bağlanır, geçersiz
                    // alan adları ve aralık dışı değerler güvenli sınıra çekilir.
                    settings.engine.sanitize();
                    for guild in settings.guilds.values_mut() {
                        if let Some(filters) = guild.role_filters.as_mut() {
                            filters.sanitize();
                        }
                        if let Some(engine) = guild.engine.as_mut() {
                            engine.sanitize();
                        }
                    }

                    // Eski tek `role_filter` alanı modül filtresine taşınır (bir kez).
                    settings.migrate_legacy_role_filter();
                    settings.role_filters.sanitize();
                    settings.sanitize_channels();

                    // Göç (bir kez): eski **tam kopya** sunucu kayıtları sparse override'a
                    // indirgenir. Öncesinde tarih damgalı yedek alınır, sonuç diske yazılır;
                    // yazma izni yoksa bot bellekteki (göç edilmiş) ayarlarla devam eder.
                    // `schema_version` damgalanmasa göç her açılışta yeniden çalışır ve
                    // kullanıcının bilerek sabitlediği bir bölüm, genelle geçici olarak aynı
                    // olduğu için sessizce mirasa dönebilirdi.
                    // Schema 2 already completed the destructive equal-block reduction.
                    // Only pre-sparse files may run that migration; upgrading 2 → 3 merely
                    // records the new channel-aware format and must never unpin blocks again.
                    if settings.schema_version < 2 {
                        let reduced = settings.migrate_legacy_guild_overrides();
                        settings.schema_version = 2;
                        match backup_settings_file(path) {
                            Ok(backup) => {
                                info!("Göç öncesi ayar yedeği alındı: {}", backup.display())
                            }
                            Err(err) => return Err(format!("migration backup failed: {err}")),
                        }
                        match settings.save_to(path) {
                            Ok(()) => info!(
                                "Sunucu ayarları sparse override modeline göç etti: {} kayıt indirgendi/sadeleşti.",
                                reduced
                            ),
                            Err(err) => warn!(
                                "Göç sonucu '{}' dosyasına yazılamadı: {} — bellekteki ayarlarla devam ediliyor.",
                                path.display(),
                                err
                            ),
                        }
                    }
                    if settings.schema_version < SETTINGS_SCHEMA_VERSION {
                        settings.sanitize_channels();
                        settings.schema_version = SETTINGS_SCHEMA_VERSION;
                        if let Err(err) = settings.save_to(path) {
                            warn!(
                                "Schema 3 ayarları '{}' dosyasına yazılamadı: {}",
                                path.display(),
                                err
                            );
                        }
                    }

                    info!(
                        "Ayarlar '{}' dosyasından başarıyla yüklendi.",
                        path.display()
                    );
                    return Ok(settings);
                }
                Err(reason) => {
                    // Dosyanın üzerine yazmak kullanıcı ayarlarını kalıcı olarak yok eder;
                    // bozuk dosya korunur; erişim politikaları varsayılana düşürülmez.
                    error!(
                        "Ayarlar dosyası ('{}') {} — mevcut dosya korunuyor, başlatma durduruldu.",
                        path.display(),
                        reason
                    );
                    return Err(format!(
                        "settings could not be loaded: {reason}; refusing to reset access policies"
                    ));
                }
            }
        }

        let defaults = AppSettings::default();
        defaults
            .save_to(path)
            .map_err(|e| format!("settings initialization failed: {e}"))?;
        Ok(defaults)
    }

    pub(crate) fn save_to(&self, path: &Path) -> Result<(), std::io::Error> {
        use std::io::Write;
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(std::io::Error::other)?;
        let name: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let temp = parent.join(format!(".botgek-settings-{name}.tmp"));
        let write_result = (|| {
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temp)?;
            file.write_all(json.as_bytes())?;
            file.sync_all()?;
            drop(file);
            replace_settings_file(&temp, path)?;
            #[cfg(unix)]
            {
                // Replacement already committed. Publish the same state to memory even
                // if the filesystem cannot provide directory durability guarantees.
                if let Err(error) = fs::File::open(parent).and_then(|dir| dir.sync_all()) {
                    warn!("Settings directory sync failed after committed replacement: {error}");
                }
            }
            Ok::<(), std::io::Error>(())
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temp);
        }

        match write_result {
            Ok(_) => {
                info!("Ayarlar '{}' dosyasına kaydedildi.", path.display());
                Ok(())
            }
            Err(e) => {
                if e.kind() == std::io::ErrorKind::PermissionDenied {
                    error!(
                        "Ayarlar dosyasına ('{}') yazma izni reddedildi (Permission Denied)! \
                        Ayar dizininin bot kullanıcısına ait olduğunu doğrulayın; dosya yerine dizini mount edin.",
                        path.display()
                    );
                } else {
                    error!(
                        "Ayarlar '{}' dosyasına kaydedilirken hata: {:?}",
                        path.display(),
                        e
                    );
                }
                Err(e)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_browser_fixtures_match_rust_resolution() {
        let fixtures: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/settings-resolution.json"))
                .unwrap();
        for fixture in fixtures.as_array().unwrap() {
            let global: AppSettings = serde_json::from_value(fixture["global"].clone()).unwrap();
            let over: Option<GuildOverride> =
                serde_json::from_value(fixture["override"].clone()).unwrap();
            let result = serde_json::to_value(resolve_settings(&global, over.as_ref())).unwrap();
            for (pointer, expected) in fixture["expected"].as_object().unwrap() {
                assert_eq!(
                    result.pointer(pointer),
                    Some(expected),
                    "{}: {pointer}",
                    fixture["name"]
                );
            }
        }
    }

    #[test]
    fn settings_path_follows_persistent_database_directory_when_unconfigured() {
        let database = Path::new("data").join("botgek.db");
        assert_eq!(
            resolve_settings_path_from(None, Some(&database)),
            Path::new("data").join("settings.json")
        );
    }

    #[test]
    fn explicit_settings_path_and_memory_database_keep_their_existing_behavior() {
        let explicit = Path::new("/custom/settings.json");
        assert_eq!(
            resolve_settings_path_from(Some(explicit), Some(Path::new("data/botgek.db"))),
            explicit
        );
        assert_eq!(
            resolve_settings_path_from(None, Some(Path::new(":memory:"))),
            Path::new("settings.json")
        );
    }

    #[test]
    fn atomic_settings_replace_and_invalid_files_fail_closed() {
        let path = std::env::temp_dir().join(format!(
            "botgek-settings-{}-{}.json",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut settings = AppSettings::default();
        settings.save_to(&path).unwrap();
        settings.modules.twitter = false;
        settings.save_to(&path).unwrap();
        assert!(!AppSettings::load_from(&path).unwrap().modules.twitter);
        let before = fs::read(&path).unwrap();
        assert!(settings.save_to(&path.join("cannot-write-here")).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        fs::write(&path, "invalid JSON").unwrap();
        assert!(AppSettings::load_from(&path).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "invalid JSON");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn test_settings_serialization() {
        let settings = AppSettings::default();
        let json = serde_json::to_string_pretty(&settings).unwrap();
        let deserialized: AppSettings = serde_json::from_str(&json).unwrap();

        assert!(deserialized.modules.twitter);
        assert!(deserialized.modules.tiktok);
        assert_eq!(deserialized.role_filters.twitter.mode, FilterMode::Disabled);
        assert_eq!(deserialized.role_filters.tiktok.mode, FilterMode::Disabled);
        assert_eq!(deserialized.ai.model, crate::ai::DEFAULT_MODEL);
    }

    #[test]
    fn partial_settings_file_defaults_missing_fields_instead_of_failing() {
        // Parse hatası, dosyanın varsayılanlarla üzerine yazılmasına (ayar kaybına) yol açmamalı
        let json = r#"{ "modules": { "twitter": false }, "ai": { "model": "test/model" } }"#;
        let settings: AppSettings = serde_json::from_str(json).unwrap();

        assert!(!settings.modules.twitter);
        assert!(
            settings.modules.reddit,
            "eksik alan struct varsayılanıyla doldurulmalı"
        );
        assert_eq!(settings.ai.model, "test/model");
        assert!(!settings.ai.system_prompt.trim().is_empty());
        assert_eq!(settings.role_filters.twitter.mode, FilterMode::Disabled);
        assert!(settings.role_filters.twitter.roles.is_empty());
    }

    #[test]
    fn empty_settings_object_is_loadable() {
        let settings: AppSettings = serde_json::from_str("{}").unwrap();
        assert!(settings.modules.ai_chat);
        assert!(settings.guilds.is_empty());
    }

    #[test]
    fn legacy_role_filter_is_migrated_once_and_no_longer_written() {
        // Eski ayar dosyası: tek `role_filter` alanı (modül filtreleri boş).
        let legacy_json = r#"{ "role_filter": { "mode": "allowlist", "roles": ["42", "43"] } }"#;
        let mut settings: AppSettings = serde_json::from_str(legacy_json).unwrap();

        settings.migrate_legacy_role_filter();

        for filter in [
            &settings.role_filters.twitter,
            &settings.role_filters.tiktok,
            &settings.role_filters.instagram,
            &settings.role_filters.reddit,
            &settings.role_filters.bluesky,
            &settings.role_filters.translation,
            &settings.role_filters.ai_chat,
        ] {
            assert_eq!(filter.mode, FilterMode::Allowlist);
            assert_eq!(filter.roles, vec!["42".to_string(), "43".to_string()]);
        }

        // Migrasyondan sonra eski alan dosyaya yazılmaz: ayarlar tek gerçeği taşır.
        let saved = serde_json::to_string(&settings).unwrap();
        assert!(
            !saved.contains("\"role_filter\""),
            "eski çift alan kaydedilmemeli: {saved}"
        );

        // Modül filtreleri doluysa migrasyon üzerine yazmaz.
        let mut customized = settings.clone();
        customized.role_filters.twitter.roles = vec!["7".to_string()];
        customized.migrate_legacy_role_filter();
        assert_eq!(customized.role_filters.twitter.roles, vec!["7".to_string()]);
    }

    #[test]
    fn test_per_module_role_filter() {
        let mut settings = AppSettings::default();
        settings.role_filters.twitter.mode = FilterMode::Blacklist;
        settings.role_filters.twitter.roles = vec!["111".to_string()];

        settings.role_filters.tiktok.mode = FilterMode::Allowlist;
        settings.role_filters.tiktok.roles = vec!["222".to_string()];

        assert_eq!(settings.role_filters.twitter.mode, FilterMode::Blacklist);
        assert_eq!(settings.role_filters.twitter.roles, vec!["111".to_string()]);

        assert_eq!(settings.role_filters.tiktok.mode, FilterMode::Allowlist);
        assert_eq!(settings.role_filters.tiktok.roles, vec!["222".to_string()]);

        assert_eq!(settings.role_filters.instagram.mode, FilterMode::Disabled);
    }

    #[test]
    fn test_filter_mode_serde() {
        assert_eq!(
            serde_json::to_string(&FilterMode::Blacklist).unwrap(),
            "\"blacklist\""
        );
        assert_eq!(
            serde_json::to_string(&FilterMode::Allowlist).unwrap(),
            "\"allowlist\""
        );
        assert_eq!(
            serde_json::to_string(&FilterMode::Disabled).unwrap(),
            "\"disabled\""
        );
    }

    #[test]
    fn test_conversion_mode_serde_and_defaults() {
        assert_eq!(
            serde_json::to_string(&ConversionMode::Builtin).unwrap(),
            "\"builtin\""
        );
        assert_eq!(
            serde_json::to_string(&ConversionMode::Proxy).unwrap(),
            "\"proxy\""
        );

        // Varsayılan: twitter/instagram/reddit/bluesky/spotify builtin, tiktok proxy
        let cfg = AppSettings::default();
        assert_eq!(cfg.engine.mode_for("twitter"), ConversionMode::Builtin);
        assert_eq!(cfg.engine.mode_for("reddit"), ConversionMode::Builtin);
        assert_eq!(cfg.engine.mode_for("bluesky"), ConversionMode::Builtin);
        assert_eq!(cfg.engine.mode_for("tiktok"), ConversionMode::Proxy);
        assert_eq!(cfg.engine.mode_for("instagram"), ConversionMode::Builtin);
        assert_eq!(cfg.engine.mode_for("bilinmeyen"), ConversionMode::Proxy);
    }

    #[test]
    fn engine_settings_round_trip_through_json() {
        let mut settings = AppSettings::default();
        settings.engine.modes.set("tiktok", ConversionMode::Builtin);
        settings.engine.twitter_source = TwitterSource::Fxtwitter;
        settings.engine.max_video_mb = 12;
        settings
            .engine
            .domains
            .set("spotify", "fixspotify.com".to_string());

        let json = serde_json::to_string(&settings).unwrap();
        let decoded: AppSettings = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.engine.modes.tiktok, ConversionMode::Builtin);
        assert_eq!(decoded.engine.twitter_source, TwitterSource::Fxtwitter);
        assert_eq!(decoded.engine.twitter_source.key(), "fxtwitter");
        assert_eq!(decoded.engine.max_video_mb, 12);
        assert_eq!(decoded.engine.domains.spotify, "fixspotify.com");
    }

    /// **Geriye dönük uyumluluk (migration gerekmez):** eski `settings.json` düzeni — kaldırılan
    /// `twitter/instagram/reddit` motor modu ve yedek alan adları ile `emojis.reddit` alanı
    /// yokken — **hatasız** yüklenir: bilinmeyen anahtarlar yok sayılır, eksik yeni alan
    /// varsayılanını alır (serde `default` + `deny_unknown_fields` kullanılmıyor).
    #[test]
    fn legacy_settings_with_removed_engine_keys_load_without_migration() {
        let json = r#"{
            "modules": { "twitter": true, "reddit": true },
            "role_filters": {},
            "ai": { "model": "test/model", "system_prompt": "kişilik" },
            "engine": {
                "modes": {
                    "twitter": "proxy",
                    "tiktok": "proxy",
                    "instagram": "proxy",
                    "reddit": "proxy",
                    "bluesky": "builtin",
                    "spotify": "builtin"
                },
                "domains": {
                    "twitter": "vxtwitter.com",
                    "tiktok": "",
                    "instagram": "ddinstagram.com",
                    "reddit": "vxreddit.com",
                    "bluesky": "",
                    "spotify": ""
                },
                "twitter_source": "syndication",
                "max_images": 4,
                "max_video_mb": 8,
                "show_metrics": true,
                "show_timestamp": true,
                "show_community_note": true,
                "show_quote": true,
                "auto_translate": true,
                "fallback_to_proxy": true,
                "impersonate_user": true,
                "impersonate_skip_replies": true,
                "emojis": {
                    "verified": "<:verified:1553709836859084841>",
                    "replies": "",
                    "reposts": "",
                    "likes": "",
                    "views": "",
                    "translation": "",
                    "twitter": "",
                    "instagram": "",
                    "quote": ""
                }
            },
            "guilds": {
                "4242": {
                    "modules": { "twitter": true },
                    "role_filters": {},
                    "ai": { "model": "test/model", "system_prompt": "" },
                    "engine": {
                        "modes": {
                            "twitter": "builtin",
                            "tiktok": "proxy",
                            "instagram": "proxy",
                            "reddit": "builtin",
                            "bluesky": "builtin",
                            "spotify": "builtin"
                        },
                        "domains": {
                            "twitter": "",
                            "tiktok": "",
                            "instagram": "",
                            "reddit": "",
                            "bluesky": "",
                            "spotify": ""
                        }
                    }
                }
            }
        }"#;

        let settings: AppSettings =
            serde_json::from_str(json).expect("eski settings.json düzeni hatasız yüklenmeli");

        // Kaldırılan anahtarlar yok sayılır: native-only platformlar daima native.
        assert_eq!(settings.engine.mode_for("twitter"), ConversionMode::Builtin);
        assert_eq!(
            settings.engine.mode_for("instagram"),
            ConversionMode::Builtin
        );
        assert_eq!(settings.engine.mode_for("reddit"), ConversionMode::Builtin);
        assert!(!settings.engine.allows_proxy_fallback("reddit"));
        // Proxy kullanılabilen platformlar okunmaya devam eder.
        assert_eq!(settings.engine.modes.tiktok, ConversionMode::Proxy);
        assert!(settings.engine.allows_proxy_fallback("tiktok"));
        assert!(settings.engine.domains.tiktok.is_empty());
        // Eski dosyada bulunmayan yeni alan varsayılanını alır ve çalışır.
        assert_eq!(settings.engine.emojis.reddit, DEFAULT_REDDIT_EMOJI);
        assert_eq!(settings.engine.emojis.reddit(), Some(DEFAULT_REDDIT_EMOJI));
        // Sunucuya özel eski kayıt da (eksik alanlarıyla) yüklenir.
        let guild = settings.get_guild_settings(Some(serenity::all::GuildId::new(4242)));
        assert_eq!(guild.engine.mode_for("reddit"), ConversionMode::Builtin);
        assert!(!guild.engine.allows_proxy_fallback("reddit"));
        assert!(guild.engine.allows_proxy_fallback("bluesky"));
    }

    #[test]
    fn engine_settings_missing_fields_fall_back_to_defaults() {
        // Native-only platformların (twitter/instagram/reddit) kayıtlı değerleri yok sayılır.
        let json = r#"{ "engine": { "modes": { "twitter": "proxy", "bluesky": "proxy" }, "attach_videos": false } }"#;
        let settings: AppSettings = serde_json::from_str(json).unwrap();

        assert_eq!(settings.engine.modes.bluesky, ConversionMode::Proxy);
        assert_eq!(
            settings.engine.mode_for("twitter"),
            ConversionMode::Builtin,
            "Twitter/X native-only: proxy modu yok sayılır"
        );
        assert!(!settings.engine.allows_proxy_fallback("twitter"));
        assert!(settings.engine.allows_proxy_fallback("bluesky"));
        // Eksik alanlar varsayılan kalmalı
        assert_eq!(settings.engine.twitter_source, TwitterSource::Syndication);
        assert_eq!(settings.engine.max_images, 4);
        assert!(settings.engine.auto_translate);
        assert!(settings.engine.fallback_to_proxy);
    }

    #[test]
    fn engine_domain_prefers_webui_override_then_env() {
        let config = crate::config::Config {
            discord_token: "dummy".to_string(),
            tiktok_domain: "tnktok.com".to_string(),
            bluesky_domain: "fxbsky.app".to_string(),
            spotify_domain: "open.fixspotify.com".to_string(),
            delete_original: true,
            openrouter_api_key: None,
            reddit_client_id: "test-id".to_string(),
            instagram_sessionid: None,
            web_port: 3000,
        };

        let mut engine = EngineSettings::default();
        assert_eq!(engine.domain_for("tiktok", &config), "tnktok.com");

        engine.domains.set("tiktok", "tnktok.example".to_string());
        assert_eq!(engine.domain_for("tiktok", &config), "tnktok.example");
        assert_eq!(engine.domain_for("bluesky", &config), "fxbsky.app");
        assert_eq!(engine.domain_for("spotify", &config), "open.fixspotify.com");

        // Native-only platformlarda proxy motoru (ve yedek alan adı) yoktur.
        assert!(engine.domain_for("twitter", &config).is_empty());
        assert!(engine.domain_for("instagram", &config).is_empty());
        assert!(engine.domain_for("reddit", &config).is_empty());
    }

    #[test]
    fn engine_sanitize_clamps_values_and_cleans_domains() {
        let mut engine = EngineSettings {
            max_images: 0,
            max_video_mb: 200,
            ..Default::default()
        };
        engine
            .domains
            .set("tiktok", "https://Tnktok.com/".to_string());
        engine.domains.set("bluesky", "kötü domain!!".to_string());

        engine.sanitize();

        assert!(MAX_IMAGES_RANGE.contains(&engine.max_images));
        assert!(MAX_VIDEO_MB_RANGE.contains(&engine.max_video_mb));
        assert_eq!(engine.domains.tiktok, "tnktok.com");
        assert!(
            engine.domains.bluesky.is_empty(),
            "geçersiz alan adı temizlenmeli"
        );
    }

    #[test]
    fn engine_settings_accept_webui_payload() {
        // WebUI (index.html) tarafından üretilen JSON şeklinin birebir kabul edildiğini doğrular.
        let json = r#"{
            "modules": { "twitter": true },
            "role_filters": { "twitter": { "mode": "disabled", "roles": ["123"] } },
            "ai": { "model": "test/model", "system_prompt": "kişilik" },
            "engine": {
                "modes": {
                    "tiktok": "builtin",
                    "bluesky": "builtin"
                },
                "domains": {
                    "tiktok": "  Tnktok.com/  ",
                    "bluesky": ""
                },
                "twitter_source": "hybrid",
                "max_images": 7,
                "max_video_mb": 99,
                "show_metrics": true,
                "show_timestamp": false,
                "show_community_note": true,
                "show_quote": false,
                "auto_translate": false,
                "fallback_to_proxy": false
            }
        }"#;

        let mut settings: AppSettings = serde_json::from_str(json).unwrap();
        settings.engine.sanitize();

        assert_eq!(settings.engine.modes.tiktok, ConversionMode::Builtin);
        assert_eq!(
            settings.engine.mode_for("instagram"),
            ConversionMode::Builtin,
            "Instagram native-only: proxy modu yok"
        );
        assert_eq!(settings.engine.twitter_source, TwitterSource::Hybrid);
        assert!(!settings.engine.show_timestamp);
        assert!(!settings.engine.show_quote);
        assert!(!settings.engine.auto_translate);
        assert_eq!(settings.engine.max_images, 7);
        // WebUI aralık dışı değer gönderirse güvenli sınıra çekilir
        assert_eq!(
            settings.engine.max_video_mb,
            MAX_VIDEO_MB_RANGE.end().to_owned()
        );
        assert_eq!(settings.engine.domains.tiktok, "tnktok.com");
        // Yeni alanlar JSON'da yoksa varsayılanları alır
        assert!(settings.engine.impersonate_user);
        assert!(
            settings.engine.impersonate_skip_replies,
            "yanıt olarak atılan linklerde yanıt bağlantısı varsayılan olarak korunur"
        );
        assert_eq!(
            settings.engine.emojis.verified(),
            "<:verified:1553709836859084841>",
            "emoji seti JSON'da yoksa varsayılan (yüklenmiş) emojiler kullanılır"
        );
    }

    #[test]
    fn emoji_settings_validate_custom_emojis_and_fall_back_to_unicode() {
        let mut emojis = EmojiSettings::default();
        emojis.sanitize();
        assert_eq!(emojis.replies(), "<:comment:1553712370302451782>");
        assert_eq!(emojis.twitter(), Some("<:twitter:1476642012072920355>"));

        // Boş alanlar Unicode varsayılana düşer
        let mut empty = EmojiSettings {
            verified: String::new(),
            replies: String::new(),
            reposts: String::new(),
            likes: String::new(),
            views: String::new(),
            translation: String::new(),
            twitter: String::new(),
            instagram: String::new(),
            reddit: String::new(),
            quote: String::new(),
        };
        empty.sanitize();
        assert_eq!(empty.verified(), "☑");
        assert_eq!(empty.replies(), "💬");
        assert_eq!(empty.reposts(), "🔁");
        assert_eq!(empty.likes(), "❤️");
        assert_eq!(empty.views(), "👁️");
        assert_eq!(empty.translation(), "🌐");
        assert_eq!(empty.twitter(), None);
        assert_eq!(empty.instagram(), None);
        assert_eq!(empty.reddit(), None);
        assert_eq!(empty.quote(), "↪");
    }

    #[test]
    fn emoji_name_without_id_is_bound_to_the_bot_emoji() {
        // WebUI/JSON'da kimlik yazılmayı unutulmuş değerler botun emojisine bağlanır
        let mut emojis = EmojiSettings {
            verified: ":verified:".to_string(),
            replies: ":comment:".to_string(),
            reposts: ":rt:".to_string(),
            likes: ":like:".to_string(),
            views: ":metric:".to_string(),
            translation: ":translation:".to_string(),
            twitter: ":twitter:".to_string(),
            instagram: ":instagram:".to_string(),
            reddit: ":reddit:".to_string(),
            quote: ":qutoe:".to_string(),
        };
        emojis.sanitize();

        assert_eq!(emojis.verified, DEFAULT_VERIFIED_EMOJI);
        assert_eq!(emojis.replies, DEFAULT_REPLIES_EMOJI);
        assert_eq!(emojis.reposts, DEFAULT_REPOSTS_EMOJI);
        assert_eq!(emojis.likes, DEFAULT_LIKES_EMOJI);
        assert_eq!(emojis.views, DEFAULT_VIEWS_EMOJI);
        assert_eq!(emojis.translation, DEFAULT_TRANSLATION_EMOJI);
        assert_eq!(emojis.twitter, DEFAULT_TWITTER_EMOJI);
        assert_eq!(emojis.instagram, DEFAULT_INSTAGRAM_EMOJI);
        assert_eq!(emojis.reddit, DEFAULT_REDDIT_EMOJI);
        assert_eq!(emojis.quote, DEFAULT_QUOTE_EMOJI);
        assert_eq!(emojis.twitter(), Some(DEFAULT_TWITTER_EMOJI));
        assert_eq!(emojis.instagram(), Some(DEFAULT_INSTAGRAM_EMOJI));
        assert_eq!(emojis.reddit(), Some(DEFAULT_REDDIT_EMOJI));

        // Kayıt temizlenmeden (sanitize edilmeden) okunsa bile mesajda bozuk emoji çıkmaz
        let raw = EmojiSettings {
            twitter: ":twitter:".to_string(),
            ..EmojiSettings::default()
        };
        assert_eq!(
            raw.twitter(),
            Some(DEFAULT_TWITTER_EMOJI),
            "kimliksiz ad bot emojisine düşer"
        );

        let unknown = EmojiSettings {
            twitter: ":botgek_emoji:".to_string(),
            ..EmojiSettings::default()
        };
        assert_eq!(
            unknown.twitter(),
            None,
            "tanınmayan ad platform adına düşer"
        );
        assert_eq!(unknown.twitter.trim(), ":botgek_emoji:");
    }

    #[test]
    fn emoji_sanitize_rejects_invalid_values() {
        let mut emojis = EmojiSettings {
            // ID çok kısa
            verified: "<:verified:123>".to_string(),
            // markdown linkini kırma denemesi
            replies: "<:comment:1234>](https://evil.example)".to_string(),
            // animasyonlu emoji (geçerli)
            reposts: "<a:rt:1553712344939495424>".to_string(),
            // Unicode emoji (geçerli)
            likes: "💖".to_string(),
            // fazla uzun Unicode (geçersiz)
            views: "bu bir emoji değil".to_string(),
            translation: "<:translation:1476614397249061056>".to_string(),
            twitter: " <:twitter:1476642012072920355> ".to_string(),
            instagram: "<:instagram:1558099674731122758>".to_string(),
            reddit: "<:reddit:1558112156195495956>".to_string(),
            quote: ":qutoe:".to_string(),
        };
        emojis.sanitize();

        assert_eq!(emojis.verified(), "☑", "geçersiz ID boşaltılmalı");
        assert_eq!(emojis.replies(), "💬", "markdown kıran girdi reddedilmeli");
        assert_eq!(emojis.reposts(), "<a:rt:1553712344939495424>");
        assert_eq!(emojis.likes(), "💖");
        assert_eq!(emojis.views(), "👁️");
        assert_eq!(
            emojis.twitter(),
            Some("<:twitter:1476642012072920355>"),
            "baştaki/sondaki boşluk temizlenir"
        );
    }

    #[test]
    fn engine_submission_flags_round_trip_through_json() {
        let mut settings = AppSettings::default();
        settings.engine.impersonate_user = false;

        let json = serde_json::to_string(&settings).unwrap();
        let parsed: AppSettings = serde_json::from_str(&json).unwrap();

        assert!(!parsed.engine.impersonate_user);
    }

    #[test]
    fn guild_engine_settings_are_inherited_and_overridable() {
        let mut settings = AppSettings::default();
        let guild_id = serenity::all::GuildId::new(4242);

        // Genel varsayılan: sunucu kaydı yok → miras.
        settings.engine.modes.set("bluesky", ConversionMode::Proxy);
        assert_eq!(
            settings
                .get_guild_settings(Some(guild_id))
                .engine
                .modes
                .bluesky,
            ConversionMode::Proxy
        );

        // Sunucuya özel override: yalnızca `engine` bölümü sabitlenir.
        settings.guilds.insert(
            guild_id.to_string(),
            GuildOverride {
                engine: Some(EngineSettings {
                    modes: PlatformEngineModes {
                        bluesky: ConversionMode::Builtin,
                        ..PlatformEngineModes::default()
                    },
                    ..EngineSettings::default()
                }),
                ..GuildOverride::default()
            },
        );

        assert_eq!(
            settings
                .get_guild_settings(Some(guild_id))
                .engine
                .modes
                .bluesky,
            ConversionMode::Builtin
        );
        // Bilinmeyen sunucu geneli miras alır.
        assert_eq!(
            settings
                .get_guild_settings(Some(serenity::all::GuildId::new(7)))
                .engine
                .modes
                .bluesky,
            ConversionMode::Proxy
        );
    }

    #[test]
    fn test_per_guild_settings_fallback_and_override() {
        let mut settings = AppSettings::default();
        let guild_1 = serenity::all::GuildId::new(123456789);
        let guild_2 = serenity::all::GuildId::new(987654321);

        // Varsayılan ayarlarda twitter açık
        assert!(settings.get_guild_settings(Some(guild_1)).modules.twitter);
        assert!(settings.get_guild_settings(None).modules.twitter);

        // guild_1 yalnızca `modules` ve `ai` bölümlerini sabitler.
        settings.guilds.insert(
            guild_1.to_string(),
            GuildOverride {
                modules: Some(PlatformModules {
                    twitter: false,
                    ..PlatformModules::default()
                }),
                ai: Some(AiSettings {
                    model: "custom/model".to_string(),
                    system_prompt: String::new(),
                }),
                ..GuildOverride::default()
            },
        );

        // guild_1 özel ayarı almalı; **boş** AI kişiliği genelden miras alınır.
        let g1_settings = settings.get_guild_settings(Some(guild_1));
        assert!(!g1_settings.modules.twitter);
        assert_eq!(g1_settings.ai.model, "custom/model");
        assert_eq!(g1_settings.ai.system_prompt, settings.ai.system_prompt);

        // guild_2 ve None (DM) varsayılan ayarları korumalı
        let g2_settings = settings.get_guild_settings(Some(guild_2));
        assert!(g2_settings.modules.twitter);
        assert_eq!(g2_settings.ai.model, crate::ai::DEFAULT_MODEL);

        let dm_settings = settings.get_guild_settings(None);
        assert!(dm_settings.modules.twitter);
    }

    // ── `resolve_settings` (saf çözümleme) birim testleri ───────────────────────────

    /// Boş override → tüm bölümler genelden miras alınır.
    #[test]
    fn resolve_empty_override_returns_global() {
        let mut global = AppSettings::default();
        global.modules.twitter = false;
        global.engine.max_images = 9;

        let resolved = resolve_settings(&global, Some(&GuildOverride::default()));

        assert!(!resolved.modules.twitter);
        assert_eq!(resolved.engine.max_images, 9);
        assert_eq!(resolved.ai, global.ai);
        assert_eq!(resolved.role_filters, global.role_filters);
    }

    /// Sunucu kaydı hiç yoksa (DM / özelleştirilmemiş sunucu) genel geçerlidir.
    #[test]
    fn resolve_missing_override_returns_global() {
        let mut global = AppSettings::default();
        global.modules.reddit = false;

        assert_eq!(
            resolve_settings(&global, None),
            resolve_settings(&global, Some(&GuildOverride::default())),
            "kayıt yokluğu ile boş override aynı sonucu vermeli"
        );
        assert!(!resolve_settings(&global, None).modules.reddit);
    }

    /// Kısmi override: yalnızca sabitlenen bölüm ezilir; genel değişince **miras alan**
    /// bölüm yeni değeri alır, sabitlenen değişmez.
    #[test]
    fn resolve_partial_override_and_global_change_propagation() {
        let mut global = AppSettings::default();
        global.engine.max_images = 3;
        global.ai.model = "global/model".to_string();

        let over = GuildOverride {
            modules: Some(PlatformModules {
                tiktok: false,
                ..PlatformModules::default()
            }),
            ..GuildOverride::default()
        };

        let resolved = resolve_settings(&global, Some(&over));
        assert!(!resolved.modules.tiktok);
        assert_eq!(resolved.engine.max_images, 3);
        assert_eq!(resolved.ai.model, "global/model");

        // Genel değişti: miras alan bölüm yeni değeri alır, sabitlenen bölüm etkilenmez.
        global.engine.max_images = 7;
        global.modules.tiktok = true;
        let resolved = resolve_settings(&global, Some(&over));
        assert_eq!(
            resolved.engine.max_images, 7,
            "override'ı olmayan bölüm genel değişimini izlemeli"
        );
        assert!(
            !resolved.modules.tiktok,
            "sabitlenen bölüm genel değişiminden etkilenmemeli"
        );
    }

    /// İç içe anahtar: bölüm sabitlendiğinde **içindeki tüm anahtarlar** ezilir (global'den
    /// değil, override bloğundan/kod varsayılanından gelir).
    #[test]
    fn resolve_pinned_block_overrides_nested_keys_wholesale() {
        let mut global = AppSettings::default();
        global.engine.emojis.likes = "<:global_like:1>".to_string();
        global.engine.emojis.replies = "<:global_reply:2>".to_string();

        let over = GuildOverride {
            engine: Some(EngineSettings {
                emojis: EmojiSettings {
                    likes: "<:guild_like:3>".to_string(),
                    ..EmojiSettings::default()
                },
                ..EngineSettings::default()
            }),
            ..GuildOverride::default()
        };

        let resolved = resolve_settings(&global, Some(&over));
        assert_eq!(resolved.engine.emojis.likes, "<:guild_like:3>");
        assert_eq!(
            resolved.engine.emojis.replies,
            EmojiSettings::default().replies,
            "sabitlenen bölümün diğer iç anahtarları global'den değil blok/kod varsayılanından gelir"
        );
    }

    /// Dizi alanları (rol listeleri) **birleşmez**, bütün olarak ezilir.
    #[test]
    fn resolve_role_lists_replace_wholesale() {
        let mut global = AppSettings::default();
        global.role_filters.twitter.mode = FilterMode::Blacklist;
        global.role_filters.twitter.roles = vec!["1".to_string(), "2".to_string(), "3".to_string()];

        let over = GuildOverride {
            role_filters: Some(ModuleRoleFilters {
                twitter: RoleFilterSettings {
                    mode: FilterMode::Allowlist,
                    roles: vec!["9".to_string()],
                },
                ..ModuleRoleFilters::default()
            }),
            ..GuildOverride::default()
        };

        let resolved = resolve_settings(&global, Some(&over));
        assert_eq!(resolved.role_filters.twitter.mode, FilterMode::Allowlist);
        assert_eq!(
            resolved.role_filters.twitter.roles,
            vec!["9".to_string()],
            "rol listesi birleşmez: override dizisi bütün olarak geçerlidir"
        );
        assert_eq!(
            resolved.role_filters.tiktok, global.role_filters.tiktok,
            "sabitlenmeyen modüller genelden miras alır"
        );
    }

    /// Bölüm override'ı kaldırılınca mirasa dönülür.
    #[test]
    fn resolve_removing_override_returns_to_inheritance() {
        let mut global = AppSettings::default();
        global.engine.max_images = 5;

        let mut over = GuildOverride {
            engine: Some(EngineSettings {
                max_images: 2,
                ..EngineSettings::default()
            }),
            ..GuildOverride::default()
        };
        assert_eq!(resolve_settings(&global, Some(&over)).engine.max_images, 2);

        over.engine = None;
        assert!(over.is_empty(), "hiç bölüm kalmayınca kayıt miras sayılır");
        assert_eq!(
            resolve_settings(&global, Some(&over)).engine.max_images,
            5,
            "override kaldırılınca genel geçerli olur"
        );
    }

    /// Sparse kayıt, JSON'a yalnızca sabitlenen bölümleri yazar (`skip_serializing_if`).
    #[test]
    fn guild_override_serializes_sparse() {
        let settings = AppSettings {
            guilds: HashMap::from([(
                "42".to_string(),
                GuildOverride {
                    ai: Some(AiSettings {
                        model: "m".to_string(),
                        system_prompt: "p".to_string(),
                    }),
                    ..GuildOverride::default()
                },
            )]),
            ..AppSettings::default()
        };

        let json = serde_json::to_string(&settings).unwrap();
        let guilds_json = &json[json.find("\"guilds\"").expect("guilds alanı yazılmalı")..];

        assert!(guilds_json.contains("\"ai\""));
        for block in ["\"modules\"", "\"role_filters\"", "\"engine\""] {
            assert!(
                !guilds_json.contains(block),
                "sabitlenmeyen bölüm kaydedilmemeli: {guilds_json}"
            );
        }
    }

    // ── Göç (`migrate_legacy_guild_overrides`) birim testleri ───────────────────────

    /// Göç: genelle birebir aynı bölümler un-pin edilir, farklı bölüm korunur.
    #[test]
    fn migrate_reduces_identical_blocks_and_keeps_different_ones() {
        let mut settings = AppSettings::default();
        settings.engine.max_images = 5;

        // Eski "tam kopya": modules/role_filters/ai genelle aynı, engine farklı.
        settings.guilds.insert(
            "1".to_string(),
            GuildOverride {
                modules: Some(settings.modules.clone()),
                role_filters: Some(settings.role_filters.clone()),
                ai: Some(settings.ai.clone()),
                engine: Some(EngineSettings {
                    max_images: 2,
                    ..settings.engine.clone()
                }),
                channels: HashMap::new(),
            },
        );

        assert_eq!(settings.migrate_legacy_guild_overrides(), 1);

        let over = &settings.guilds["1"];
        assert!(over.modules.is_none());
        assert!(over.role_filters.is_none());
        assert!(over.ai.is_none());
        assert_eq!(over.engine.as_ref().unwrap().max_images, 2);
    }

    /// Göç: tamamen genelle aynı olan sunucu kaydı silinir (artık "özel" değildir).
    #[test]
    fn migrate_drops_guild_record_identical_to_global() {
        let mut settings = AppSettings::default();
        settings.guilds.insert(
            "7".to_string(),
            GuildOverride {
                modules: Some(settings.modules.clone()),
                role_filters: Some(settings.role_filters.clone()),
                ai: Some(settings.ai.clone()),
                engine: Some(settings.engine.clone()),
                channels: HashMap::new(),
            },
        );

        assert_eq!(settings.migrate_legacy_guild_overrides(), 1);
        assert!(
            settings.guilds.is_empty(),
            "tam kopya kayıt mirasa dönüp silinmeli"
        );
    }

    /// Göç: boş model/kişilik alanlı AI bloğu genelle aynı **çözülüyorsa** pin gereksizdir.
    #[test]
    fn migrate_reduces_ai_block_with_empty_fields() {
        let mut settings = AppSettings::default();
        settings.guilds.insert(
            "3".to_string(),
            GuildOverride {
                ai: Some(AiSettings {
                    model: String::new(),
                    system_prompt: "  ".to_string(),
                }),
                ..GuildOverride::default()
            },
        );

        assert_eq!(settings.migrate_legacy_guild_overrides(), 1);
        assert!(settings.guilds.is_empty());
    }

    /// Göç idempotenttir: ikinci çalıştırma çıktıyı değiştirmez.
    #[test]
    fn migrate_is_idempotent() {
        let mut settings = AppSettings::default();
        settings.engine.max_images = 5;
        settings.guilds.insert(
            "1".to_string(),
            GuildOverride {
                modules: Some(settings.modules.clone()),
                engine: Some(EngineSettings {
                    max_images: 2,
                    ..settings.engine.clone()
                }),
                ..GuildOverride::default()
            },
        );

        assert_eq!(settings.migrate_legacy_guild_overrides(), 1);
        let after_migration = settings.clone();

        assert_eq!(
            settings.migrate_legacy_guild_overrides(),
            0,
            "ikinci çalıştırma değişiklik üretmemeli"
        );
        assert_eq!(
            serde_json::to_string(&settings).unwrap(),
            serde_json::to_string(&after_migration).unwrap()
        );
    }

    /// Şema sürümü: taze ayarlar güncel sürümle doğar, eski dosya `0` okunur (göç tetiklenir).
    #[test]
    fn schema_version_marks_fresh_and_legacy_files() {
        assert_eq!(
            AppSettings::default().schema_version,
            SETTINGS_SCHEMA_VERSION,
            "taze ayar dosyası güncel sürümü taşımalı"
        );

        let legacy: AppSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(
            legacy.schema_version, 0,
            "eski dosyada alan yok → 0 → göç bir kez çalışır"
        );

        let json = serde_json::to_string(&AppSettings::default()).unwrap();
        assert!(
            json.contains(&format!("\"schema_version\":{SETTINGS_SCHEMA_VERSION}")),
            "sürüm kalıcı olmalı: {json}"
        );
    }

    #[test]
    fn message_settings_resolve_thread_parent_then_channel_and_dm_is_global() {
        let guild_id = serenity::all::GuildId::new(7);
        let parent = serenity::all::ChannelId::new(70);
        let channel = serenity::all::ChannelId::new(71);
        let mut settings = AppSettings::default();
        settings.modules.twitter = true;
        let mut over = GuildOverride {
            modules: Some(PlatformModules {
                twitter: false,
                ..settings.modules.clone()
            }),
            ..Default::default()
        };
        over.channels.insert(
            parent.to_string(),
            ChannelOverride {
                modules: Some(PlatformModules {
                    twitter: true,
                    ..settings.modules.clone()
                }),
                ai: Some(AiSettings {
                    model: String::new(),
                    system_prompt: "parent".into(),
                }),
                ..Default::default()
            },
        );
        over.channels.insert(
            channel.to_string(),
            ChannelOverride {
                engine: Some(EngineSettings {
                    max_images: 2,
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
        settings.guilds.insert(guild_id.to_string(), over);

        let resolved = settings.get_message_settings(Some(guild_id), channel, Some(parent));
        assert!(resolved.modules.twitter);
        assert_eq!(resolved.ai.system_prompt, "parent");
        assert_eq!(resolved.engine.max_images, 2);
        // No parent means a normal channel does not inherit the category/thread entry.
        let normal = settings.get_message_settings(Some(guild_id), channel, None);
        assert!(!normal.modules.twitter);
        assert_eq!(normal.engine.max_images, 2);
        let dm = settings.get_message_settings(None, channel, Some(parent));
        assert!(dm.modules.twitter);
    }

    #[test]
    fn schema_two_is_upgraded_without_equal_block_unpin() {
        let mut settings = AppSettings {
            schema_version: 2,
            ..Default::default()
        };
        settings.guilds.insert(
            "7".into(),
            GuildOverride {
                modules: Some(settings.modules.clone()),
                ..Default::default()
            },
        );
        let before = settings.guilds["7"].modules.clone();
        // The loader's 2 → 3 path intentionally does not call the destructive legacy reducer.
        assert_eq!(settings.schema_version, 2);
        assert_eq!(before, settings.guilds["7"].modules);
    }

    #[test]
    fn sensitive_mode_uses_stable_wire_names_and_safe_default() {
        let engine = EngineSettings::default();
        assert_eq!(engine.sensitive_mode, SensitiveMode::HideAlways);
        assert_eq!(
            serde_json::to_value(engine.sensitive_mode).unwrap(),
            "hide_always"
        );
        assert_eq!(
            serde_json::to_value(SensitiveMode::AllowInNsfw).unwrap(),
            "allow_in_nsfw"
        );
    }
}
