use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{error, info};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FilterMode {
    Disabled,
    Blacklist,
    Allowlist,
}

impl Default for FilterMode {
    fn default() -> Self {
        FilterMode::Disabled
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct RoleFilterSettings {
    pub mode: FilterMode,
    /// Discord Rol ID'leri (örn: ["123456789012345678"])
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiSettings {
    pub model: String,
    pub system_prompt: String,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            model: "google/gemma-4-31b-it:free".to_string(),
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
}

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
            _ => &self.role_filters.twitter,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppSettings {
    pub modules: PlatformModules,
    #[serde(default)]
    pub role_filters: ModuleRoleFilters,
    /// Geriye dönük uyumluluk için eski genel role_filter (varsa)
    #[serde(default)]
    pub role_filter: Option<RoleFilterSettings>,
    pub ai: AiSettings,
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

    pub fn get_role_filter(&self, module_key: &str) -> &RoleFilterSettings {
        match module_key {
            "twitter" => &self.role_filters.twitter,
            "tiktok" => &self.role_filters.tiktok,
            "instagram" => &self.role_filters.instagram,
            "reddit" => &self.role_filters.reddit,
            "bluesky" => &self.role_filters.bluesky,
            "translation" => &self.role_filters.translation,
            "ai_chat" => &self.role_filters.ai_chat,
            _ => &self.role_filters.twitter,
        }
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
        if path.exists() && path.is_file() {
            match fs::read_to_string(&path) {
                Ok(content) => match serde_json::from_str::<AppSettings>(&content) {
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
                    Err(e) => {
                        error!("Ayarlar dosyası ayrıştırılamadı, varsayılanlar kullanılacak: {:?}", e);
                    }
                },
                Err(e) => {
                    error!("Ayarlar dosyası okunamadı: {:?}", e);
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
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

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

        assert_eq!(deserialized.modules.twitter, true);
        assert_eq!(deserialized.modules.tiktok, true);
        assert_eq!(deserialized.role_filters.twitter.mode, FilterMode::Disabled);
        assert_eq!(deserialized.role_filters.tiktok.mode, FilterMode::Disabled);
        assert_eq!(deserialized.ai.model, "google/gemma-4-31b-it:free");
    }

    #[test]
    fn test_per_module_role_filter() {
        let mut settings = AppSettings::default();
        settings.role_filters.twitter.mode = FilterMode::Blacklist;
        settings.role_filters.twitter.roles = vec!["111".to_string()];

        settings.role_filters.tiktok.mode = FilterMode::Allowlist;
        settings.role_filters.tiktok.roles = vec!["222".to_string()];

        assert_eq!(settings.get_role_filter("twitter").mode, FilterMode::Blacklist);
        assert_eq!(settings.get_role_filter("twitter").roles, vec!["111".to_string()]);

        assert_eq!(settings.get_role_filter("tiktok").mode, FilterMode::Allowlist);
        assert_eq!(settings.get_role_filter("tiktok").roles, vec!["222".to_string()]);

        assert_eq!(settings.get_role_filter("instagram").mode, FilterMode::Disabled);
    }

    #[test]
    fn test_filter_mode_serde() {
        assert_eq!(serde_json::to_string(&FilterMode::Blacklist).unwrap(), "\"blacklist\"");
        assert_eq!(serde_json::to_string(&FilterMode::Allowlist).unwrap(), "\"allowlist\"");
        assert_eq!(serde_json::to_string(&FilterMode::Disabled).unwrap(), "\"disabled\"");
    }

    #[test]
    fn test_per_guild_settings_fallback_and_override() {
        let mut settings = AppSettings::default();
        let guild_1 = serenity::all::GuildId::new(123456789);
        let guild_2 = serenity::all::GuildId::new(987654321);

        // Varsayılan ayarlarda twitter açık
        assert_eq!(settings.get_guild_settings(Some(guild_1)).modules.twitter, true);
        assert_eq!(settings.get_guild_settings(None).modules.twitter, true);

        // guild_1 için özel ayar: twitter kapalı, model farklı
        let mut custom = settings.default_guild_settings();
        custom.modules.twitter = false;
        custom.ai.model = "custom/model".to_string();
        settings.guilds.insert(guild_1.to_string(), custom);

        // guild_1 özel ayarı almalı
        let g1_settings = settings.get_guild_settings(Some(guild_1));
        assert_eq!(g1_settings.modules.twitter, false);
        assert_eq!(g1_settings.ai.model, "custom/model");

        // guild_2 ve None (DM) varsayılan ayarları korumalı
        let g2_settings = settings.get_guild_settings(Some(guild_2));
        assert_eq!(g2_settings.modules.twitter, true);
        assert_eq!(g2_settings.ai.model, "google/gemma-4-31b-it:free");

        let dm_settings = settings.get_guild_settings(None);
        assert_eq!(dm_settings.modules.twitter, true);
    }
}

