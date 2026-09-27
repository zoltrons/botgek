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

/// Platform bazlı motor seçimi.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PlatformEngineModes {
    pub twitter: ConversionMode,
    pub tiktok: ConversionMode,
    pub instagram: ConversionMode,
    pub reddit: ConversionMode,
    pub bluesky: ConversionMode,
}

impl Default for PlatformEngineModes {
    /// Twitter, Reddit ve Bluesky için native embed motoru açıktır (başarısız olursa
    /// `EngineSettings::fallback_to_proxy` sayesinde eski proxy davranışına düşer).
    /// TikTok (yalnızca kart) ve Instagram (native API yok) için varsayılan proxy'dir.
    fn default() -> Self {
        Self {
            twitter: ConversionMode::Builtin,
            tiktok: ConversionMode::Proxy,
            instagram: ConversionMode::Proxy,
            reddit: ConversionMode::Builtin,
            bluesky: ConversionMode::Builtin,
        }
    }
}

impl PlatformEngineModes {
    pub fn get(&self, platform_key: &str) -> ConversionMode {
        match platform_key {
            "twitter" => self.twitter,
            "tiktok" => self.tiktok,
            "instagram" => self.instagram,
            "reddit" => self.reddit,
            "bluesky" => self.bluesky,
            _ => ConversionMode::Proxy,
        }
    }

    /// Programatik kullanım (testler/varsayılanlar) içindir; WebUI değerleri JSON ile yazar.
    #[allow(dead_code)]
    pub fn set(&mut self, platform_key: &str, mode: ConversionMode) {
        match platform_key {
            "twitter" => self.twitter = mode,
            "tiktok" => self.tiktok = mode,
            "instagram" => self.instagram = mode,
            "reddit" => self.reddit = mode,
            "bluesky" => self.bluesky = mode,
            _ => {}
        }
    }
}

/// Proxy modunda kullanılacak alan adları. Boş bırakılan alan `.env` / varsayılan
/// değeri kullanır (WebUI'dan .env dosyasına dokunmadan alan adı değiştirilebilir).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ProxyDomains {
    pub twitter: String,
    pub tiktok: String,
    pub instagram: String,
    pub reddit: String,
    pub bluesky: String,
}

impl ProxyDomains {
    pub fn get(&self, platform_key: &str) -> &str {
        match platform_key {
            "twitter" => &self.twitter,
            "tiktok" => &self.tiktok,
            "instagram" => &self.instagram,
            "reddit" => &self.reddit,
            "bluesky" => &self.bluesky,
            _ => "",
        }
    }

    pub fn set(&mut self, platform_key: &str, value: String) {
        match platform_key {
            "twitter" => self.twitter = value,
            "tiktok" => self.tiktok = value,
            "instagram" => self.instagram = value,
            "reddit" => self.reddit = value,
            "bluesky" => self.bluesky = value,
            _ => {}
        }
    }
}

/// Tek mesajda ek olarak yüklenecek maksimum görsel sayısı sınırı.
pub const MAX_IMAGES_RANGE: std::ops::RangeInclusive<u8> = 1..=10;
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
    /// Görselleri indirip mesaja dosya olarak ekle (tam çözünürlük + CDN bağımsızlığı).
    pub attach_images: bool,
    /// Videoları indirip mesaja dosya olarak ekle (limit altındaysa).
    pub attach_videos: bool,
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
}

impl Default for EngineSettings {
    fn default() -> Self {
        Self {
            modes: PlatformEngineModes::default(),
            domains: ProxyDomains::default(),
            twitter_source: TwitterSource::default(),
            attach_images: true,
            attach_videos: true,
            max_images: 4,
            max_video_mb: 8,
            show_metrics: true,
            show_timestamp: true,
            show_community_note: true,
            show_quote: true,
            auto_translate: true,
            fallback_to_proxy: true,
        }
    }
}

impl EngineSettings {
    /// Platform anahtarı (`twitter`, `tiktok` ...) için etkin mod.
    pub fn mode_for(&self, platform_key: &str) -> ConversionMode {
        self.modes.get(platform_key)
    }

    /// Platform için geçerli proxy alan adı: WebUI ayarı doluysa o, değilse `.env` değeri.
    pub fn domain_for(&self, platform_key: &str, config: &crate::config::Config) -> String {
        let from_settings = self.domains.get(platform_key);
        if !from_settings.is_empty() {
            return from_settings.to_string();
        }
        match platform_key {
            "twitter" => config.twitter_domain.clone(),
            "tiktok" => config.tiktok_domain.clone(),
            "instagram" => config.instagram_domain.clone(),
            "reddit" => config.reddit_domain.clone(),
            "bluesky" => config.bluesky_domain.clone(),
            _ => String::new(),
        }
    }

    /// WebUI girdilerini güvenli sınırlara çeker ve alan adlarını temizler.
    /// Geçersiz alan adı boşaltılır (bu da `.env` değerinin kullanılması demektir).
    pub fn sanitize(&mut self) {
        for key in ["twitter", "tiktok", "instagram", "reddit", "bluesky"] {
            let cleaned = crate::config::clean_domain(self.domains.get(key)).unwrap_or_default();
            self.domains.set(key, cleaned);
        }

        if !MAX_IMAGES_RANGE.contains(&self.max_images) {
            self.max_images = self.max_images.clamp(1, 10);
        }
        if !MAX_VIDEO_MB_RANGE.contains(&self.max_video_mb) {
            self.max_video_mb = self.max_video_mb.clamp(1, 50);
        }
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

static DEFAULT_DISABLED_FILTER: once_cell::sync::Lazy<RoleFilterSettings> =
    once_cell::sync::Lazy::new(RoleFilterSettings::default);

impl GuildSettings {
    pub fn get_role_filter(&self, module_key: &str) -> &RoleFilterSettings {
        match module_key {
            "twitter" => &self.role_filters.twitter,
            "tiktok" => &self.role_filters.tiktok,
            "instagram" => &self.role_filters.instagram,
            "reddit" => &self.role_filters.reddit,
            "bluesky" => &self.role_filters.bluesky,
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
    /// Geriye dönük uyumluluk için eski genel role_filter (varsa)
    #[serde(default)]
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
        if let Some(gid) = guild_id {
            if let Some(custom) = self.guilds.get(&gid.to_string()) {
                let mut resolved = custom.clone();
                if resolved.ai.system_prompt.trim().is_empty() {
                    resolved.ai.system_prompt = self.ai.system_prompt.clone();
                }
                if resolved.ai.model.trim().is_empty() {
                    resolved.ai.model = self.ai.model.clone();
                }
                return resolved;
            }
        }
        self.default_guild_settings()
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
                    // Geriye dönük uyumluluk: Eski role_filter doluysa ve yeniler boşsa aktar
                    if let Some(ref legacy) = settings.role_filter {
                        if !legacy.roles.is_empty() && settings.role_filters.twitter.roles.is_empty() {
                            settings.role_filters.twitter = legacy.clone();
                            settings.role_filters.tiktok = legacy.clone();
                            settings.role_filters.instagram = legacy.clone();
                            settings.role_filters.reddit = legacy.clone();
                            settings.role_filters.bluesky = legacy.clone();
                            settings.role_filters.translation = legacy.clone();
                            settings.role_filters.ai_chat = legacy.clone();
                        }
                    }
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

        // Varsayılan: twitter/reddit/bluesky builtin, tiktok/instagram proxy
        let cfg = AppSettings::default();
        assert_eq!(cfg.engine.mode_for("twitter"), ConversionMode::Builtin);
        assert_eq!(cfg.engine.mode_for("reddit"), ConversionMode::Builtin);
        assert_eq!(cfg.engine.mode_for("bluesky"), ConversionMode::Builtin);
        assert_eq!(cfg.engine.mode_for("tiktok"), ConversionMode::Proxy);
        assert_eq!(cfg.engine.mode_for("instagram"), ConversionMode::Proxy);
        assert_eq!(cfg.engine.mode_for("bilinmeyen"), ConversionMode::Proxy);
    }

    #[test]
    fn engine_settings_round_trip_through_json() {
        let mut settings = AppSettings::default();
        settings.engine.modes.set("twitter", ConversionMode::Proxy);
        settings.engine.modes.set("instagram", ConversionMode::Builtin);
        settings.engine.twitter_source = TwitterSource::Fxtwitter;
        settings.engine.max_video_mb = 12;
        settings.engine.domains.set("twitter", "vxtwitter.com".to_string());

        let json = serde_json::to_string(&settings).unwrap();
        let decoded: AppSettings = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded.engine.modes.twitter, ConversionMode::Proxy);
        assert_eq!(decoded.engine.modes.instagram, ConversionMode::Builtin);
        assert_eq!(decoded.engine.twitter_source, TwitterSource::Fxtwitter);
        assert_eq!(decoded.engine.twitter_source.key(), "fxtwitter");
        assert_eq!(decoded.engine.max_video_mb, 12);
        assert_eq!(decoded.engine.domains.twitter, "vxtwitter.com");
    }

    #[test]
    fn engine_settings_missing_fields_fall_back_to_defaults() {
        let json = r#"{ "engine": { "modes": { "twitter": "proxy" }, "attach_videos": false } }"#;
        let settings: AppSettings = serde_json::from_str(json).unwrap();

        assert_eq!(settings.engine.modes.twitter, ConversionMode::Proxy);
        assert!(!settings.engine.attach_videos);
        // Eksik alanlar varsayılan kalmalı
        assert!(settings.engine.attach_images);
        assert_eq!(settings.engine.twitter_source, TwitterSource::Syndication);
        assert_eq!(settings.engine.max_images, 4);
        assert!(settings.engine.auto_translate);
        assert!(settings.engine.fallback_to_proxy);
    }

    #[test]
    fn engine_domain_prefers_webui_override_then_env() {
        let config = crate::config::Config {
            discord_token: "dummy".to_string(),
            twitter_domain: "fixupx.com".to_string(),
            tiktok_domain: "tnktok.com".to_string(),
            instagram_domain: "ddinstagram.com".to_string(),
            reddit_domain: "vxreddit.com".to_string(),
            bluesky_domain: "fxbsky.app".to_string(),
            delete_original: true,
            openrouter_api_key: None,
            web_port: 3000,
        };

        let mut engine = EngineSettings::default();
        assert_eq!(engine.domain_for("twitter", &config), "fixupx.com");

        engine.domains.set("twitter", "vxtwitter.com".to_string());
        assert_eq!(engine.domain_for("twitter", &config), "vxtwitter.com");
        assert_eq!(engine.domain_for("tiktok", &config), "tnktok.com");
    }

    #[test]
    fn engine_sanitize_clamps_values_and_cleans_domains() {
        let mut engine = EngineSettings::default();
        engine.max_images = 0;
        engine.max_video_mb = 200;
        engine.domains.set("twitter", "https://FixupX.com/".to_string());
        engine.domains.set("reddit", "kötü domain!!".to_string());

        engine.sanitize();

        assert!(MAX_IMAGES_RANGE.contains(&engine.max_images));
        assert!(MAX_VIDEO_MB_RANGE.contains(&engine.max_video_mb));
        assert_eq!(engine.domains.twitter, "fixupx.com");
        assert!(engine.domains.reddit.is_empty(), "geçersiz alan adı temizlenmeli");
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
                    "twitter": "builtin",
                    "tiktok": "builtin",
                    "instagram": "proxy",
                    "reddit": "builtin",
                    "bluesky": "builtin"
                },
                "domains": {
                    "twitter": "  VxTwitter.com/  ",
                    "tiktok": "",
                    "instagram": "",
                    "reddit": "",
                    "bluesky": ""
                },
                "twitter_source": "hybrid",
                "attach_images": true,
                "attach_videos": false,
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
        assert_eq!(settings.engine.modes.instagram, ConversionMode::Proxy);
        assert_eq!(settings.engine.twitter_source, TwitterSource::Hybrid);
        assert!(!settings.engine.attach_videos);
        assert!(!settings.engine.show_timestamp);
        assert!(!settings.engine.show_quote);
        assert!(!settings.engine.auto_translate);
        assert_eq!(settings.engine.max_images, 7);
        // WebUI aralık dışı değer gönderirse güvenli sınıra çekilir
        assert_eq!(settings.engine.max_video_mb, MAX_VIDEO_MB_RANGE.end().to_owned());
        assert_eq!(settings.engine.domains.twitter, "vxtwitter.com");
    }

    #[test]
    fn guild_engine_settings_are_inherited_and_overridable() {
        let mut settings = AppSettings::default();
        let guild_id = serenity::all::GuildId::new(4242);

        // Genel varsayılan
        settings.engine.modes.set("twitter", ConversionMode::Proxy);
        assert_eq!(
            settings.get_guild_settings(Some(guild_id)).engine.modes.twitter,
            ConversionMode::Proxy
        );

        // Sunucuya özel override
        let mut custom = settings.default_guild_settings();
        custom.engine.modes.set("twitter", ConversionMode::Builtin);
        settings.guilds.insert(guild_id.to_string(), custom);

        assert_eq!(
            settings.get_guild_settings(Some(guild_id)).engine.modes.twitter,
            ConversionMode::Builtin
        );
        assert_eq!(
            settings.get_guild_settings(Some(serenity::all::GuildId::new(7))).engine.modes.twitter,
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

