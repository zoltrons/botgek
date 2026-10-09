use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{error, info};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PlatformModules {
    pub twitter: bool,
    pub tiktok: bool,
    pub instagram: bool,
    pub reddit: bool,
    pub bluesky: bool,
    pub spotify: bool,
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
pub const NATIVE_ONLY_PLATFORMS: [&str; 3] = ["twitter", "instagram", "reddit"];

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
        self.translation =
            normalize_emoji(&self.translation, DEFAULT_TRANSLATION_EMOJI, TRANSLATION_NAMES);
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
        resolve_emoji(&self.translation, DEFAULT_TRANSLATION_EMOJI, "🌐", TRANSLATION_NAMES)
    }

    /// X/Twitter göstergesi; kullanılabilir bir emoji yoksa `None` (yerine platform adı yazılır).
    pub fn twitter(&self) -> Option<&str> {
        let resolved = resolve_emoji(&self.twitter, DEFAULT_TWITTER_EMOJI, "", TWITTER_NAMES);
        (!resolved.is_empty()).then_some(resolved)
    }

    /// Instagram göstergesi; kullanılabilir bir emoji yoksa `None` (yerine platform adı yazılır).
    pub fn instagram(&self) -> Option<&str> {
        let resolved = resolve_emoji(&self.instagram, DEFAULT_INSTAGRAM_EMOJI, "", INSTAGRAM_NAMES);
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
        && names.iter().any(|candidate| candidate.eq_ignore_ascii_case(name)) {
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
        && names.iter().any(|candidate| candidate.eq_ignore_ascii_case(name)) {
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

    if let Some(body) = value.strip_prefix("<:").or_else(|| value.strip_prefix("<a:")) {
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

    value.chars().count() <= 8 && !value.chars().any(|c| "[\\]()`<>|*_~".contains(c) || c.is_control())
}

/// Discord'un (boost'suz sunucularda) yüklenen dosya limiti 10 MB'tır; üzerine çıkılırsa
/// video indirilmez, link olarak paylaşılır.
pub const MAX_VIDEO_MB_RANGE: std::ops::RangeInclusive<u8> = 1..=50;

/// Native (built-in) embed motorunun davranış ayarları.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct EngineSettings {
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
    /// Türkçe/İngilizce dışındaki X gönderilerini X'in kendi çevirisiyle Türkçeye çevir.
    pub auto_translate: bool,
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
            fallback_to_proxy: true,
            impersonate_user: true,
            impersonate_skip_replies: true,
            emojis: EmojiSettings::default(),
        }
    }
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
            "translation" => &self.role_filters.translation,
            "ai_chat" => &self.role_filters.ai_chat,
            _ => &DEFAULT_DISABLED_FILTER,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppSettings {
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
    /// Sunucuya özel ayarlar (Guild ID -> GuildSettings)
    #[serde(default)]
    pub guilds: HashMap<String, GuildSettings>,
}

impl AppSettings {
    pub fn default_guild_settings(&self) -> GuildSettings {
        GuildSettings {
            modules: self.modules.clone(),
            role_filters: self.role_filters.clone(),
            ai: self.ai.clone(),
            engine: self.engine.clone(),
        }
    }

    pub fn get_guild_settings(&self, guild_id: Option<serenity::all::GuildId>) -> GuildSettings {
        if let Some(gid) = guild_id
            && let Some(custom) = self.guilds.get(&gid.to_string()) {
                let mut resolved = custom.clone();
                if resolved.ai.system_prompt.trim().is_empty() {
                    resolved.ai.system_prompt = self.ai.system_prompt.clone();
                }
                if resolved.ai.model.trim().is_empty() {
                    resolved.ai.model = self.ai.model.clone();
                }
                return resolved;
            }
        self.default_guild_settings()
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
        self.role_filters.translation = legacy.clone();
        self.role_filters.ai_chat = legacy;
    }
}

fn resolve_settings_path() -> PathBuf {
    let p = Path::new("settings.json");
    if p.is_dir() {
        // Docker bind-mount hostta dosya olmadığı için otomatik klasör olarak açılmışsa
        p.join("settings.json")
    } else {
        p.to_path_buf()
    }
}

impl AppSettings {
    pub fn load() -> Self {
        let path = resolve_settings_path();
        if path.is_file() {
            let loaded = fs::read_to_string(&path)
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
                        guild.engine.sanitize();
                    }

                    // Eski tek `role_filter` alanı modül filtresine taşınır (bir kez).
                    settings.migrate_legacy_role_filter();

                    info!("Ayarlar '{}' dosyasından başarıyla yüklendi.", path.display());
                    return settings;
                }
                Err(reason) => {
                    // Dosyanın üzerine yazmak kullanıcı ayarlarını kalıcı olarak yok eder;
                    // bozuk dosya olduğu gibi bırakılır ve varsayılanlarla devam edilir.
                    error!(
                        "Ayarlar dosyası ('{}') {} — mevcut dosya korunuyor, varsayılan ayarlarla başlanıyor.",
                        path.display(),
                        reason
                    );
                    return AppSettings::default();
                }
            }
        }

        let defaults = AppSettings::default();
        let _ = defaults.save();
        defaults
    }

    pub fn save(&self) -> Result<(), std::io::Error> {
        let path = resolve_settings_path();
        let json = serde_json::to_string_pretty(self)
            .map_err(std::io::Error::other)?;

        // Docker bind-mount durumunda inode korunması gerektiği için doğrudan dosyaya yazılır
        let write_result = if path.exists() && path.is_file() {
            fs::write(&path, &json)
        } else {
            // Dosya henüz yoksa önce temp dosyaya yazıp rename dene, başarısız olursa doğrudan yaz
            let temp_file = format!("{}.tmp", path.display());
            if fs::write(&temp_file, &json).is_ok() {
                if fs::rename(&temp_file, &path).is_err() {
                    let res = fs::write(&path, &json);
                    let _ = fs::remove_file(&temp_file);
                    res
                } else {
                    Ok(())
                }
            } else {
                fs::write(&path, &json)
            }
        };

        match write_result {
            Ok(_) => {
                info!("Ayarlar '{}' dosyasına kaydedildi.", path.display());
                Ok(())
            }
            Err(e) => {
                if e.kind() == std::io::ErrorKind::PermissionDenied {
                    error!(
                        "Ayarlar dosyasına ('{}') yazma izni reddedildi (Permission Denied)! \
                        Host üzerinde 'sudo chown 1000:1000 settings.json' veya 'chmod 666 settings.json' çalıştırın.",
                        path.display()
                    );
                } else {
                    error!("Ayarlar '{}' dosyasına kaydedilirken hata: {:?}", path.display(), e);
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
        assert!(settings.modules.reddit, "eksik alan struct varsayılanıyla doldurulmalı");
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
        assert!(!saved.contains("\"role_filter\""), "eski çift alan kaydedilmemeli: {saved}");

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
        assert_eq!(serde_json::to_string(&FilterMode::Blacklist).unwrap(), "\"blacklist\"");
        assert_eq!(serde_json::to_string(&FilterMode::Allowlist).unwrap(), "\"allowlist\"");
        assert_eq!(serde_json::to_string(&FilterMode::Disabled).unwrap(), "\"disabled\"");
    }

    #[test]
    fn test_conversion_mode_serde_and_defaults() {
        assert_eq!(serde_json::to_string(&ConversionMode::Builtin).unwrap(), "\"builtin\"");
        assert_eq!(serde_json::to_string(&ConversionMode::Proxy).unwrap(), "\"proxy\"");

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
        settings.engine.domains.set("spotify", "fixspotify.com".to_string());

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
        assert_eq!(settings.engine.mode_for("instagram"), ConversionMode::Builtin);
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
        engine.domains.set("tiktok", "https://Tnktok.com/".to_string());
        engine.domains.set("bluesky", "kötü domain!!".to_string());

        engine.sanitize();

        assert!(MAX_IMAGES_RANGE.contains(&engine.max_images));
        assert!(MAX_VIDEO_MB_RANGE.contains(&engine.max_video_mb));
        assert_eq!(engine.domains.tiktok, "tnktok.com");
        assert!(engine.domains.bluesky.is_empty(), "geçersiz alan adı temizlenmeli");
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
        assert_eq!(settings.engine.max_video_mb, MAX_VIDEO_MB_RANGE.end().to_owned());
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
        assert_eq!(raw.twitter(), Some(DEFAULT_TWITTER_EMOJI), "kimliksiz ad bot emojisine düşer");

        let unknown = EmojiSettings {
            twitter: ":botgek_emoji:".to_string(),
            ..EmojiSettings::default()
        };
        assert_eq!(unknown.twitter(), None, "tanınmayan ad platform adına düşer");
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
        assert_eq!(emojis.twitter(), Some("<:twitter:1476642012072920355>"), "baştaki/sondaki boşluk temizlenir");
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

        // Genel varsayılan
        settings.engine.modes.set("bluesky", ConversionMode::Proxy);
        assert_eq!(
            settings.get_guild_settings(Some(guild_id)).engine.modes.bluesky,
            ConversionMode::Proxy
        );

        // Sunucuya özel override
        let mut custom = settings.default_guild_settings();
        custom.engine.modes.set("bluesky", ConversionMode::Builtin);
        settings.guilds.insert(guild_id.to_string(), custom);

        assert_eq!(
            settings.get_guild_settings(Some(guild_id)).engine.modes.bluesky,
            ConversionMode::Builtin
        );
        assert_eq!(
            settings.get_guild_settings(Some(serenity::all::GuildId::new(7))).engine.modes.bluesky,
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

        // guild_1 için özel ayar: twitter kapalı, model farklı
        let mut custom = settings.default_guild_settings();
        custom.modules.twitter = false;
        custom.ai.model = "custom/model".to_string();
        settings.guilds.insert(guild_1.to_string(), custom);

        // guild_1 özel ayarı almalı
        let g1_settings = settings.get_guild_settings(Some(guild_1));
        assert!(!g1_settings.modules.twitter);
        assert_eq!(g1_settings.ai.model, "custom/model");

        // guild_2 ve None (DM) varsayılan ayarları korumalı
        let g2_settings = settings.get_guild_settings(Some(guild_2));
        assert!(g2_settings.modules.twitter);
        assert_eq!(g2_settings.ai.model, crate::ai::DEFAULT_MODEL);

        let dm_settings = settings.get_guild_settings(None);
        assert!(dm_settings.modules.twitter);
    }
}

