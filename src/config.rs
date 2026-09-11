use std::env;

#[derive(Clone)]
pub struct Config {
    pub discord_token: String,
    pub twitter_domain: String,
    pub tiktok_domain: String,
    pub instagram_domain: String,
    pub reddit_domain: String,
    pub bluesky_domain: String,
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
            .field("delete_original", &self.delete_original)
            .field("openrouter_api_key", &self.openrouter_api_key.as_ref().map(|_| "[REDACTED]"))
            .field("web_port", &self.web_port)
            .finish()
    }
}

fn sanitize_domain(raw: Result<String, env::VarError>, default: &str) -> String {
    match raw {
        Ok(val) => {
            let trimmed = val.trim();
            if trimmed.is_empty() {
                return default.to_string();
            }
            let without_proto = trimmed
                .strip_prefix("https://")
                .or_else(|| trimmed.strip_prefix("http://"))
                .unwrap_or(trimmed);
            let cleaned = without_proto.trim_end_matches('/');
            if cleaned.is_empty() {
                default.to_string()
            } else {
                cleaned.to_string()
            }
        }
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
            delete_original,
            openrouter_api_key,
            web_port,
        })
    }
}

