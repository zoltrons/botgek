use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{error, info};

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RoleFilterSettings {
    pub mode: FilterMode,
    /// Discord Rol ID'leri (örn: ["123456789012345678"])
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppSettings {
    pub modules: PlatformModules,
    pub role_filter: RoleFilterSettings,
    pub ai: AiSettings,
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
                    Ok(settings) => {
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
        if path.exists() && path.is_file() {
            fs::write(&path, &json)?;
            info!("Ayarlar '{}' dosyasına kaydedildi.", path.display());
            return Ok(());
        }

        // Dosya henüz yoksa önce temp dosyaya yazıp rename dene, başarısız olursa doğrudan yaz
        let temp_file = format!("{}.tmp", path.display());
        if fs::write(&temp_file, &json).is_ok() {
            if fs::rename(&temp_file, &path).is_err() {
                fs::write(&path, &json)?;
                let _ = fs::remove_file(&temp_file);
            }
        } else {
            fs::write(&path, &json)?;
        }

        info!("Ayarlar '{}' dosyasına kaydedildi.", path.display());
        Ok(())
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
        assert_eq!(deserialized.role_filter.mode, FilterMode::Disabled);
        assert_eq!(deserialized.ai.model, "google/gemma-4-31b-it:free");
    }

    #[test]
    fn test_filter_mode_serde() {
        assert_eq!(serde_json::to_string(&FilterMode::Blacklist).unwrap(), "\"blacklist\"");
        assert_eq!(serde_json::to_string(&FilterMode::Allowlist).unwrap(), "\"allowlist\"");
        assert_eq!(serde_json::to_string(&FilterMode::Disabled).unwrap(), "\"disabled\"");
    }
}

