use std::env;

#[derive(Clone)]
pub struct Config {
    pub discord_token: String,
    pub twitter_domain: String,
    pub tiktok_domain: String,
    pub instagram_domain: String,
    pub reddit_domain: String,
    pub bluesky_domain: String,
    pub spotify_domain: String,
    pub delete_original: bool,
    pub openrouter_api_key: Option<String>,
    pub web_port: u16,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("discord_token", &"[REDACTED]")
            .field("twitter_domain", &self.twitter_domain)
            .field("tiktok_domain", &self.tiktok_domain)
            .field("instagram_domain", &self.instagram_domain)
            .field("reddit_domain", &self.reddit_domain)
            .field("bluesky_domain", &self.bluesky_domain)
            .field("spotify_domain", &self.spotify_domain)
            .field("delete_original", &self.delete_original)
            .field("openrouter_api_key", &self.openrouter_api_key.as_ref().map(|_| "[REDACTED]"))
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

        let twitter_domain = sanitize_domain(env::var("TWITTER_DOMAIN"), "fixupx.com");
        let tiktok_domain = sanitize_domain(env::var("TIKTOK_DOMAIN"), "tnktok.com");
        let instagram_domain = sanitize_domain(env::var("INSTAGRAM_DOMAIN"), "ddinstagram.com");
        let reddit_domain = sanitize_domain(env::var("REDDIT_DOMAIN"), "vxreddit.com");
        let bluesky_domain = sanitize_domain(env::var("BLUESKY_DOMAIN"), "fxbsky.app");
        let spotify_domain = sanitize_domain(env::var("SPOTIFY_DOMAIN"), "open.fixspotify.com");

        let delete_original = env::var("DELETE_ORIGINAL")
            .map(|v| v != "0" && v.to_lowercase() != "false")
            .unwrap_or(true);

        let openrouter_api_key = env::var("OPENROUTER_API_KEY")
            .ok()
            .filter(|k| !k.trim().is_empty());

        let web_port = env::var("WEB_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(3000);

        Ok(Self {
            discord_token,
            twitter_domain,
            tiktok_domain,
            instagram_domain,
            reddit_domain,
            bluesky_domain,
            spotify_domain,
            delete_original,
            openrouter_api_key,
            web_port,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_domain_normalizes_user_input() {
        assert_eq!(clean_domain("  FixupX.com/  ").as_deref(), Some("fixupx.com"));
        assert_eq!(clean_domain("https://vxtwitter.com").as_deref(), Some("vxtwitter.com"));
        assert_eq!(clean_domain("http://tnktok.com/").as_deref(), Some("tnktok.com"));
        assert_eq!(clean_domain("localhost:8080").as_deref(), Some("localhost:8080"));
    }

    #[test]
    fn clean_domain_rejects_markdown_breaking_input() {
        // `[:](...)` linkini kırıp ikinci bir link enjekte etme denemesi
        assert_eq!(clean_domain("evil.com)](https://phish"), None);
        assert_eq!(clean_domain("evil.com/status/1<td>"), None);
        assert_eq!(clean_domain(""), None);
        assert_eq!(clean_domain("   "), None);
        assert_eq!(clean_domain("alanadi"), None, "nokta içermeyen alan adı geçersiz");
        assert_eq!(clean_domain(&format!("{}.com", "a".repeat(200))), None);
    }
}

