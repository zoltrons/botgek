use std::env;
pub const DEFAULT_REDDIT_CLIENT_ID: &str = "ohXpoqrZYub1kg";

#[derive(Clone)]
pub struct Config {
    pub discord_token: String,
    pub tiktok_domain: String,
    pub bluesky_domain: String,
    pub spotify_domain: String,
    pub delete_original: bool,
    pub openrouter_api_key: Option<String>,
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
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("discord_token", &"[REDACTED]")
            .field("tiktok_domain", &self.tiktok_domain)
            .field("bluesky_domain", &self.bluesky_domain)
            .field("spotify_domain", &self.spotify_domain)
            .field("delete_original", &self.delete_original)
            .field(
                "openrouter_api_key",
                &self.openrouter_api_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("reddit_client_id", &self.reddit_client_id)
            .field(
                "instagram_sessionid",
                &self.instagram_sessionid.as_ref().map(|_| "[REDACTED]"),
            )
            .field("web_port", &self.web_port)
            .finish()
    }
}

/// Kullanıcı/WebUI girdisinden temiz bir alan adı üretir.
/// `https://FixupX.com/` -> `fixupx.com`; geçersiz girdi -> `None`.
///
/// Yalnızca harf, rakam, tire, nokta ve port ayıracı (`:`) kabul edilir; bu sayede
/// WebUI'dan yazılan domain değeri `[:](...)` Markdown linkini kıramaz.
pub fn clean_domain(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let without_proto = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    let cleaned = without_proto.trim_end_matches('/').trim();
    // En az bir nokta (alan adı) veya port ayıracı (`localhost:3000`) beklenir; bu sayede
    // "alanadi" gibi eksik girdiler reddedilirken yerel test adresleri kabul edilir.
    if cleaned.is_empty()
        || cleaned.len() > 128
        || !cleaned
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
        || !(cleaned.contains('.') || cleaned.contains(':'))
    {
        return None;
    }
    Some(cleaned.to_ascii_lowercase())
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

fn sanitize_domain(raw: Result<String, env::VarError>, default: &str) -> String {
    match raw {
        Ok(val) => clean_domain(&val).unwrap_or_else(|| default.to_string()),
        Err(_) => default.to_string(),
    }
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let discord_token = env::var("DISCORD_TOKEN")
            .map_err(|_| "HATA: DISCORD_TOKEN ortam değişkeni bulunamadı. Lütfen .env dosyasını kontrol edin.".to_string())?;

        // Yedek (proxy) alan adları **yalnızca** proxy motoru kullanılabilen platformlar için
        // okunur: TikTok, Bluesky ve Spotify. Twitter/X, Instagram ve Reddit native-only'dir
        // (proxy modu ve yedeği kaldırıldı), bu yüzden alan adı ayarı yoktur.
        let tiktok_domain = sanitize_domain(env::var("TIKTOK_DOMAIN"), "tnktok.com");
        let bluesky_domain = sanitize_domain(env::var("BLUESKY_DOMAIN"), "fxbsky.app");
        let spotify_domain = sanitize_domain(env::var("SPOTIFY_DOMAIN"), "open.fixspotify.com");

        let delete_original = env_bool("DELETE_ORIGINAL", true);

        let openrouter_api_key = env::var("OPENROUTER_API_KEY")
            .ok()
            .filter(|k| !k.trim().is_empty());

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

        Ok(Self {
            discord_token,
            tiktok_domain,
            bluesky_domain,
            spotify_domain,
            delete_original,
            openrouter_api_key,
            reddit_client_id,
            instagram_sessionid,
            web_port,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_domain_normalizes_user_input() {
        assert_eq!(
            clean_domain("  FixupX.com/  ").as_deref(),
            Some("fixupx.com")
        );
        assert_eq!(
            clean_domain("https://vxtwitter.com").as_deref(),
            Some("vxtwitter.com")
        );
        assert_eq!(
            clean_domain("http://tnktok.com/").as_deref(),
            Some("tnktok.com")
        );
        assert_eq!(
            clean_domain("localhost:8080").as_deref(),
            Some("localhost:8080")
        );
    }

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

    #[test]
    fn clean_domain_rejects_markdown_breaking_input() {
        // `[:](...)` linkini kırıp ikinci bir link enjekte etme denemesi
        assert_eq!(clean_domain("evil.com)](https://phish"), None);
        assert_eq!(clean_domain("evil.com/status/1<td>"), None);
        assert_eq!(clean_domain(""), None);
        assert_eq!(clean_domain("   "), None);
        assert_eq!(
            clean_domain("alanadi"),
            None,
            "nokta içermeyen alan adı geçersiz"
        );
        assert_eq!(clean_domain(&format!("{}.com", "a".repeat(200))), None);
    }
}
