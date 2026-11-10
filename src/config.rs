use std::env;
pub const DEFAULT_REDDIT_CLIENT_ID: &str = "ohXpoqrZYub1kg";

#[derive(Clone)]
pub struct Config {
    pub discord_token: String,
    pub delete_original: bool,
    /// Groq anahtarı (`GROQ_API_KEY`): metin sohbeti ve web araması.
    pub groq_api_key: Option<String>,
    /// Gemini anahtarı (`GEMINI_API_KEY`): görsel okuma ve yedek.
    pub gemini_api_key: Option<String>,
    /// Reddit OAuth istemci kimliği (`REDDIT_CLIENT_ID`).
    ///
    /// Reddit, kimliksiz `.json` isteklerini 403 ile engelliyor. Bot, redlib'in kullandığı
    /// **`installed_client`** akışıyla (kullanıcı izni gerektirmeyen, public istemci kimliği)
    /// bearer token alıp `oauth.reddit.com` üzerinden okur. Varsayılan, projede gömülü olan
    /// public istemci kimliğidir (redlib ile aynı); kendi Reddit uygulamanızın kimliğini
    /// `.env`'de bu değere yazarak değiştirebilirsiniz. Sır değildir (public kimlik).
    pub reddit_client_id: String,
    /// Instagram oturum çerezi (`sessionid`), yalnızca `.env`'den okunur.
    ///
    /// Instagram, reel/video mp4'ünü (ve çoklu carousel görsellerini) girişsiz istemcilere
    /// vermiyor. Bu değer ayarlanırsa bot, gönderi verisini Instagram'ın uygulama API'sinden
    /// (`/api/v1/media/{pk}/info/`) çeker ve videoyu doğrudan karta koyar. Gizli bir değerdir:
    /// `settings.json`'a ve WebUI'a yazılmaz.
    pub instagram_sessionid: Option<String>,
    pub web_port: u16,
    /// Geliştirme sunucusu (`DEV_GUILD_ID`). Tanımlıysa slash komutları global yerine yalnızca bu
    /// sunucuya kaydedilir; yayılma beklenmeden anında görünür. Kullanıcı kurulumu (user install)
    /// ve DM bağlamları sunucu komutlarında yoktur, bu yüzden yalnızca geliştirme botunda kullanılır.
    pub dev_guild_id: Option<serenity::all::GuildId>,
    /// Bot sahibinin Discord kullanıcı kimliği (`OWNER_ID`). Sahip Apps → Yenile ile her kartı
    /// yenileyebilir; diğer kullanıcılar yalnızca kendi paylaştığı kartı.
    pub owner_id: Option<serenity::all::UserId>,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("discord_token", &"[REDACTED]")
            .field("delete_original", &self.delete_original)
            .field(
                "groq_api_key",
                &self.groq_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "gemini_api_key",
                &self.gemini_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("reddit_client_id", &self.reddit_client_id)
            .field(
                "instagram_sessionid",
                &self.instagram_sessionid.as_ref().map(|_| "[REDACTED]"),
            )
            .field("web_port", &self.web_port)
            .field("dev_guild_id", &self.dev_guild_id)
            .field("owner_id", &self.owner_id)
            .finish()
    }
}

/// Ortam değişkenindeki mantıksal değeri yorumlar; tanınmayan değer `None` döner.
///
/// `yes/no`, `on/off`, `1/0`, `true/false` büyük/küçük harf ve boşluk duyarsız kabul edilir.
pub fn parse_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// Mantıksal ortam değişkenini okur; yoksa veya geçersizse `default` (geçersizse uyarı ile).
pub fn env_bool(name: &str, default: bool) -> bool {
    match env::var(name) {
        Ok(raw) => parse_bool(&raw).unwrap_or_else(|| {
            tracing::warn!("{name}={raw:?} tanınmadı; varsayılan {default} kullanılıyor");
            default
        }),
        Err(_) => default,
    }
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let discord_token = env::var("DISCORD_TOKEN")
            .map_err(|_| "HATA: DISCORD_TOKEN ortam değişkeni bulunamadı. Lütfen .env dosyasını kontrol edin.".to_string())?;

        let delete_original = env_bool("DELETE_ORIGINAL", true);

        let api_key = |name| env::var(name).ok().filter(|k| !k.trim().is_empty());
        let groq_api_key = api_key("GROQ_API_KEY");
        let gemini_api_key = api_key("GEMINI_API_KEY");

        // Reddit, kimliksiz `.json` isteklerini engellediği için `installed_client` OAuth
        // akışıyla okunur; istemci kimliği public'tir ve `.env`'den değiştirilebilir.
        let reddit_client_id = env::var("REDDIT_CLIENT_ID")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_REDDIT_CLIENT_ID.to_string());

        // Instagram oturum çerezi: yalnızca ortamdan okunur (sır; dosyaya/WebUI'a yazılmaz).
        let instagram_sessionid = env::var("INSTAGRAM_SESSIONID")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());

        let web_port = match env::var("WEB_PORT") {
            Ok(raw) => raw.trim().parse::<u16>().unwrap_or_else(|_| {
                tracing::warn!("WEB_PORT={raw:?} geçersiz; 3000 kullanılıyor");
                3000
            }),
            Err(_) => 3000,
        };

        let dev_guild_id = match env::var("DEV_GUILD_ID") {
            Ok(raw) if !raw.trim().is_empty() => match raw.trim().parse::<u64>() {
                Ok(id) if id != 0 => Some(serenity::all::GuildId::new(id)),
                _ => {
                    tracing::warn!("DEV_GUILD_ID={raw:?} geçersiz; komutlar global kaydedilecek");
                    None
                }
            },
            _ => None,
        };

        let owner_id = match env::var("OWNER_ID") {
            Ok(raw) if !raw.trim().is_empty() => match raw.trim().parse::<u64>() {
                Ok(id) if id != 0 => Some(serenity::all::UserId::new(id)),
                _ => {
                    tracing::warn!("OWNER_ID={raw:?} geçersiz; sahip komutları kapalı");
                    None
                }
            },
            _ => None,
        };

        Ok(Self {
            discord_token,
            delete_original,
            groq_api_key,
            gemini_api_key,
            reddit_client_id,
            instagram_sessionid,
            web_port,
            dev_guild_id,
            owner_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn booleans_accept_common_spellings_and_reject_unknown() {
        for raw in ["1", "true", " TRUE ", "yes", "On"] {
            assert_eq!(parse_bool(raw), Some(true), "{raw}");
        }
        for raw in ["0", "false", "no", " off", "FALSE"] {
            assert_eq!(parse_bool(raw), Some(false), "{raw}");
        }
        assert_eq!(parse_bool("belki"), None);
    }
}
