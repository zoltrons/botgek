use std::env;

#[derive(Debug, Clone)]
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

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let discord_token = env::var("DISCORD_TOKEN")
            .map_err(|_| "HATA: DISCORD_TOKEN ortam değişkeni bulunamadı. Lütfen .env dosyasını kontrol edin.".to_string())?;

        let twitter_domain = env::var("TWITTER_DOMAIN")
            .unwrap_or_else(|_| "fixupx.com".to_string());

        let tiktok_domain = env::var("TIKTOK_DOMAIN")
            .unwrap_or_else(|_| "tnktok.com".to_string());

        let instagram_domain = env::var("INSTAGRAM_DOMAIN")
            .unwrap_or_else(|_| "ddinstagram.com".to_string());

        let reddit_domain = env::var("REDDIT_DOMAIN")
            .unwrap_or_else(|_| "vxreddit.com".to_string());

        let bluesky_domain = env::var("BLUESKY_DOMAIN")
            .unwrap_or_else(|_| "fxbsky.app".to_string());

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

