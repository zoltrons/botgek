use once_cell::sync::Lazy;
use regex::Regex;

use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Twitter,
    TikTok,
    Instagram,
    Reddit,
    Bluesky,
}

impl Platform {
    pub fn display_name(&self) -> &'static str {
        match self {
            Platform::Twitter => "Twitter",
            Platform::TikTok => "TikTok",
            Platform::Instagram => "Instagram",
            Platform::Reddit => "Reddit",
            Platform::Bluesky => "Bluesky",
        }
    }

    #[allow(dead_code)]
    pub fn supports_translation(&self) -> bool {
        matches!(self, Platform::Twitter | Platform::Bluesky)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedPost {
    pub platform: Platform,
    pub is_spoiler: bool,
    /// Kullanıcının link haricinde mesaja eklediği not / yorum (varsa)
    pub user_comment: Option<String>,
    /// Orijinal link (örn: https://x.com/username/status/123)
    pub original_url: String,
    /// Dönüştürülmüş link (örn: https://fixupx.com/username/status/123)
    pub fixed_url: String,
}

// ==================== TWITTER / X ====================
static TWITTER_SPOILER_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\|\|\s*https?://(?:www\.)?(?:x|twitter)\.com/([^/\s]+)/status/(\d+)(?:/([a-zA-Z0-9_/]+))?/?(?:\?[^\s\|]*)?\s*\|\|")
        .expect("Geçersiz Spoiler Twitter regex deseni")
});
static TWITTER_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"https?://(?:www\.)?(?:x|twitter)\.com/([^/\s]+)/status/(\d+)(?:/([a-zA-Z0-9_/]+))?/?(?:\?[^\s]*)?")
        .expect("Geçersiz Twitter regex deseni")
});

// ==================== TIKTOK ====================
// TikTok: @user/(video|photo)/id
static TIKTOK_VIDEO_SPOILER_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\|\|\s*https?://(?:www\.)?tiktok\.com/@([^/\s]+)/(video|photo)/(\d+)/?(?:\?[^\s\|]*)?\s*\|\|")
        .expect("Geçersiz Spoiler TikTok Video regex deseni")
});
static TIKTOK_VIDEO_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"https?://(?:www\.)?tiktok\.com/@([^/\s]+)/(video|photo)/(\d+)/?(?:\?[^\s]*)?")
        .expect("Geçersiz TikTok Video regex deseni")
});

// TikTok: /t/id (Kısa link)
static TIKTOK_T_SPOILER_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\|\|\s*https?://(?:www\.)?tiktok\.com/t/([a-zA-Z0-9_-]+)/?(?:\?[^\s\|]*)?\s*\|\|")
        .expect("Geçersiz Spoiler TikTok /t/ regex deseni")
});
static TIKTOK_T_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"https?://(?:www\.)?tiktok\.com/t/([a-zA-Z0-9_-]+)/?(?:\?[^\s]*)?")
        .expect("Geçersiz TikTok /t/ regex deseni")
});

// TikTok: vm.tiktok.com veya vt.tiktok.com
static TIKTOK_SHORT_SPOILER_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\|\|\s*https?://(vm|vt)\.tiktok\.com/([a-zA-Z0-9_-]+)/?(?:\?[^\s\|]*)?\s*\|\|")
        .expect("Geçersiz Spoiler TikTok VM/VT regex deseni")
});
static TIKTOK_SHORT_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"https?://(vm|vt)\.tiktok\.com/([a-zA-Z0-9_-]+)/?(?:\?[^\s]*)?")
        .expect("Geçersiz TikTok VM/VT regex deseni")
});

// ==================== INSTAGRAM ====================
static INSTAGRAM_SPOILER_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\|\|\s*https?://(?:www\.)?instagram\.com/(p|reel|reels|tv)/([a-zA-Z0-9_-]+)(?:/[^\s\|]*)?/?(?:\?[^\s\|]*)?\s*\|\|")
        .expect("Geçersiz Spoiler Instagram regex deseni")
});
static INSTAGRAM_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"https?://(?:www\.)?instagram\.com/(p|reel|reels|tv)/([a-zA-Z0-9_-]+)(?:/[^\s]*)?/?(?:\?[^\s]*)?")
        .expect("Geçersiz Instagram regex deseni")
});

// ==================== REDDIT ====================
static REDDIT_SPOILER_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\|\|\s*https?://(?:www\.|old\.)?reddit\.com/r/([^/\s]+)/comments/([a-zA-Z0-9]+)(?:/[^\s\|]*)?/?(?:\?[^\s\|]*)?\s*\|\|")
        .expect("Geçersiz Spoiler Reddit regex deseni")
});
static REDDIT_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"https?://(?:www\.|old\.)?reddit\.com/r/([^/\s]+)/comments/([a-zA-Z0-9]+)(?:/[^\s]*)?/?(?:\?[^\s]*)?")
        .expect("Geçersiz Reddit regex deseni")
});

// ==================== BLUESKY ====================
static BLUESKY_SPOILER_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\|\|\s*https?://(?:www\.)?bsky\.app/profile/([^/\s]+)/post/([a-zA-Z0-9]+)/?(?:\?[^\s\|]*)?\s*\|\|")
        .expect("Geçersiz Spoiler Bluesky regex deseni")
});
static BLUESKY_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"https?://(?:www\.)?bsky\.app/profile/([^/\s]+)/post/([a-zA-Z0-9]+)/?(?:\?[^\s]*)?")
        .expect("Geçersiz Bluesky regex deseni")
});

/// Verilen mesaj içeriğini tüm desteklenen platformlar için analiz eder.
pub fn parse_message(raw_content: &str, config: &Config) -> Option<ParsedPost> {
    // 1. Twitter / X
    if let Some(post) = parse_twitter(raw_content, &config.twitter_domain) {
        return Some(post);
    }

    // 2. TikTok (okdargy/fxTikTok)
    if let Some(post) = parse_tiktok(raw_content, &config.tiktok_domain) {
        return Some(post);
    }

    // 3. Instagram
    if let Some(post) = parse_instagram(raw_content, &config.instagram_domain) {
        return Some(post);
    }

    // 4. Reddit
    if let Some(post) = parse_reddit(raw_content, &config.reddit_domain) {
        return Some(post);
    }

    // 5. Bluesky
    if let Some(post) = parse_bluesky(raw_content, &config.bluesky_domain) {
        return Some(post);
    }

    None
}

fn extract_user_comment(raw_content: &str, regex: &Regex) -> Option<String> {
    let cleaned = regex.replace(raw_content, "").trim().to_string();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn parse_twitter(raw_content: &str, domain: &str) -> Option<ParsedPost> {
    if let Some(caps) = TWITTER_SPOILER_REGEX.captures(raw_content) {
        let username = caps.get(1)?.as_str();
        let tweet_id = caps.get(2)?.as_str();
        let extra = caps.get(3).map(|m| format!("/{}", m.as_str())).unwrap_or_default();

        let original_url = format!("https://x.com/{}/status/{}{}", username, tweet_id, extra);
        let fixed_url = format!("https://{}/{}/status/{}{}", domain, username, tweet_id, extra);

        return Some(ParsedPost {
            platform: Platform::Twitter,
            is_spoiler: true,
            user_comment: extract_user_comment(raw_content, &TWITTER_SPOILER_REGEX),
            original_url,
            fixed_url,
        });
    }

    if let Some(caps) = TWITTER_REGEX.captures(raw_content) {
        let username = caps.get(1)?.as_str();
        let tweet_id = caps.get(2)?.as_str();
        let extra = caps.get(3).map(|m| format!("/{}", m.as_str())).unwrap_or_default();

        let original_url = format!("https://x.com/{}/status/{}{}", username, tweet_id, extra);
        let fixed_url = format!("https://{}/{}/status/{}{}", domain, username, tweet_id, extra);

        return Some(ParsedPost {
            platform: Platform::Twitter,
            is_spoiler: false,
            user_comment: extract_user_comment(raw_content, &TWITTER_REGEX),
            original_url,
            fixed_url,
        });
    }

    None
}

fn parse_tiktok(raw_content: &str, domain: &str) -> Option<ParsedPost> {
    // 1. @user/(video|photo)/id
    if let Some(caps) = TIKTOK_VIDEO_SPOILER_REGEX.captures(raw_content) {
        let user = caps.get(1)?.as_str();
        let kind = caps.get(2)?.as_str();
        let id = caps.get(3)?.as_str();
        return Some(ParsedPost {
            platform: Platform::TikTok,
            is_spoiler: true,
            user_comment: extract_user_comment(raw_content, &TIKTOK_VIDEO_SPOILER_REGEX),
            original_url: format!("https://www.tiktok.com/@{}/{}/{}", user, kind, id),
            fixed_url: format!("https://{}/@{}/{}/{}", domain, user, kind, id),
        });
    }
    if let Some(caps) = TIKTOK_VIDEO_REGEX.captures(raw_content) {
        let user = caps.get(1)?.as_str();
        let kind = caps.get(2)?.as_str();
        let id = caps.get(3)?.as_str();
        return Some(ParsedPost {
            platform: Platform::TikTok,
            is_spoiler: false,
            user_comment: extract_user_comment(raw_content, &TIKTOK_VIDEO_REGEX),
            original_url: format!("https://www.tiktok.com/@{}/{}/{}", user, kind, id),
            fixed_url: format!("https://{}/@{}/{}/{}", domain, user, kind, id),
        });
    }

    // 2. /t/id
    if let Some(caps) = TIKTOK_T_SPOILER_REGEX.captures(raw_content) {
        let id = caps.get(1)?.as_str();
        return Some(ParsedPost {
            platform: Platform::TikTok,
            is_spoiler: true,
            user_comment: extract_user_comment(raw_content, &TIKTOK_T_SPOILER_REGEX),
            original_url: format!("https://www.tiktok.com/t/{}", id),
            fixed_url: format!("https://{}/t/{}", domain, id),
        });
    }
    if let Some(caps) = TIKTOK_T_REGEX.captures(raw_content) {
        let id = caps.get(1)?.as_str();
        return Some(ParsedPost {
            platform: Platform::TikTok,
            is_spoiler: false,
            user_comment: extract_user_comment(raw_content, &TIKTOK_T_REGEX),
            original_url: format!("https://www.tiktok.com/t/{}", id),
            fixed_url: format!("https://{}/t/{}", domain, id),
        });
    }

    // 3. vm.tiktok.com veya vt.tiktok.com
    if let Some(caps) = TIKTOK_SHORT_SPOILER_REGEX.captures(raw_content) {
        let sub = caps.get(1)?.as_str();
        let id = caps.get(2)?.as_str();
        return Some(ParsedPost {
            platform: Platform::TikTok,
            is_spoiler: true,
            user_comment: extract_user_comment(raw_content, &TIKTOK_SHORT_SPOILER_REGEX),
            original_url: format!("https://{}.tiktok.com/{}", sub, id),
            fixed_url: format!("https://{}.{}/{}", sub, domain, id),
        });
    }
    if let Some(caps) = TIKTOK_SHORT_REGEX.captures(raw_content) {
        let sub = caps.get(1)?.as_str();
        let id = caps.get(2)?.as_str();
        return Some(ParsedPost {
            platform: Platform::TikTok,
            is_spoiler: false,
            user_comment: extract_user_comment(raw_content, &TIKTOK_SHORT_REGEX),
            original_url: format!("https://{}.tiktok.com/{}", sub, id),
            fixed_url: format!("https://{}.{}/{}", sub, domain, id),
        });
    }

    None
}

fn parse_instagram(raw_content: &str, domain: &str) -> Option<ParsedPost> {
    if let Some(caps) = INSTAGRAM_SPOILER_REGEX.captures(raw_content) {
        let kind = caps.get(1)?.as_str();
        let id = caps.get(2)?.as_str();
        return Some(ParsedPost {
            platform: Platform::Instagram,
            is_spoiler: true,
            user_comment: extract_user_comment(raw_content, &INSTAGRAM_SPOILER_REGEX),
            original_url: format!("https://www.instagram.com/{}/{}", kind, id),
            fixed_url: format!("https://{}/{}/{}", domain, kind, id),
        });
    }
    if let Some(caps) = INSTAGRAM_REGEX.captures(raw_content) {
        let kind = caps.get(1)?.as_str();
        let id = caps.get(2)?.as_str();
        return Some(ParsedPost {
            platform: Platform::Instagram,
            is_spoiler: false,
            user_comment: extract_user_comment(raw_content, &INSTAGRAM_REGEX),
            original_url: format!("https://www.instagram.com/{}/{}", kind, id),
            fixed_url: format!("https://{}/{}/{}", domain, kind, id),
        });
    }
    None
}

fn parse_reddit(raw_content: &str, domain: &str) -> Option<ParsedPost> {
    if let Some(caps) = REDDIT_SPOILER_REGEX.captures(raw_content) {
        let sub = caps.get(1)?.as_str();
        let id = caps.get(2)?.as_str();
        return Some(ParsedPost {
            platform: Platform::Reddit,
            is_spoiler: true,
            user_comment: extract_user_comment(raw_content, &REDDIT_SPOILER_REGEX),
            original_url: format!("https://www.reddit.com/r/{}/comments/{}", sub, id),
            fixed_url: format!("https://{}/r/{}/comments/{}", domain, sub, id),
        });
    }
    if let Some(caps) = REDDIT_REGEX.captures(raw_content) {
        let sub = caps.get(1)?.as_str();
        let id = caps.get(2)?.as_str();
        return Some(ParsedPost {
            platform: Platform::Reddit,
            is_spoiler: false,
            user_comment: extract_user_comment(raw_content, &REDDIT_REGEX),
            original_url: format!("https://www.reddit.com/r/{}/comments/{}", sub, id),
            fixed_url: format!("https://{}/r/{}/comments/{}", domain, sub, id),
        });
    }
    None
}

fn parse_bluesky(raw_content: &str, domain: &str) -> Option<ParsedPost> {
    if let Some(caps) = BLUESKY_SPOILER_REGEX.captures(raw_content) {
        let handle = caps.get(1)?.as_str();
        let rkey = caps.get(2)?.as_str();
        return Some(ParsedPost {
            platform: Platform::Bluesky,
            is_spoiler: true,
            user_comment: extract_user_comment(raw_content, &BLUESKY_SPOILER_REGEX),
            original_url: format!("https://bsky.app/profile/{}/post/{}", handle, rkey),
            fixed_url: format!("https://{}/profile/{}/post/{}", domain, handle, rkey),
        });
    }
    if let Some(caps) = BLUESKY_REGEX.captures(raw_content) {
        let handle = caps.get(1)?.as_str();
        let rkey = caps.get(2)?.as_str();
        return Some(ParsedPost {
            platform: Platform::Bluesky,
            is_spoiler: false,
            user_comment: extract_user_comment(raw_content, &BLUESKY_REGEX),
            original_url: format!("https://bsky.app/profile/{}/post/{}", handle, rkey),
            fixed_url: format!("https://{}/profile/{}/post/{}", domain, handle, rkey),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_config() -> Config {
        Config {
            discord_token: "dummy".to_string(),
            twitter_domain: "fixupx.com".to_string(),
            tiktok_domain: "tnktok.com".to_string(),
            instagram_domain: "ddinstagram.com".to_string(),
            reddit_domain: "rxddit.com".to_string(),
            bluesky_domain: "fxbsky.app".to_string(),
            delete_original: true,
            openrouter_api_key: None,
            web_port: 3000,
        }
    }

    #[test]
    fn test_twitter_parsing() {
        let cfg = dummy_config();
        let content = "Harika video https://x.com/elonmusk/status/123456789?s=20 mutlaka izleyin";
        let parsed = parse_message(content, &cfg).unwrap();

        assert_eq!(parsed.platform, Platform::Twitter);
        assert_eq!(parsed.original_url, "https://x.com/elonmusk/status/123456789");
        assert_eq!(parsed.fixed_url, "https://fixupx.com/elonmusk/status/123456789");
        assert_eq!(parsed.is_spoiler, false);
        assert_eq!(parsed.user_comment.as_deref(), Some("Harika video  mutlaka izleyin"));
    }

    #[test]
    fn test_twitter_spoiler() {
        let cfg = dummy_config();
        let content = "Gizli: ||https://twitter.com/jack/status/20||";
        let parsed = parse_message(content, &cfg).unwrap();

        assert_eq!(parsed.platform, Platform::Twitter);
        assert_eq!(parsed.is_spoiler, true);
        assert_eq!(parsed.user_comment.as_deref(), Some("Gizli:"));
    }

    #[test]
    fn test_tiktok_video() {
        let cfg = dummy_config();
        let content = "https://www.tiktok.com/@scout2015/video/6718335390845095173";
        let parsed = parse_message(content, &cfg).unwrap();

        assert_eq!(parsed.platform, Platform::TikTok);
        assert_eq!(parsed.original_url, "https://www.tiktok.com/@scout2015/video/6718335390845095173");
        assert_eq!(parsed.fixed_url, "https://tnktok.com/@scout2015/video/6718335390845095173");
        assert_eq!(parsed.is_spoiler, false);
        assert_eq!(parsed.user_comment, None);
    }

    #[test]
    fn test_tiktok_short() {
        let cfg = dummy_config();
        let content = "Buna bak https://vm.tiktok.com/ZM8abc123/ çok komik";
        let parsed = parse_message(content, &cfg).unwrap();

        assert_eq!(parsed.platform, Platform::TikTok);
        assert_eq!(parsed.original_url, "https://vm.tiktok.com/ZM8abc123");
        assert_eq!(parsed.fixed_url, "https://vm.tnktok.com/ZM8abc123");
        assert_eq!(parsed.user_comment.as_deref(), Some("Buna bak  çok komik"));
    }

    #[test]
    fn test_instagram_reel() {
        let cfg = dummy_config();
        let content = "https://www.instagram.com/reel/C3abcXYZ123/?igsh=abcdef";
        let parsed = parse_message(content, &cfg).unwrap();

        assert_eq!(parsed.platform, Platform::Instagram);
        assert_eq!(parsed.original_url, "https://www.instagram.com/reel/C3abcXYZ123");
        assert_eq!(parsed.fixed_url, "https://ddinstagram.com/reel/C3abcXYZ123");
    }

    #[test]
    fn test_reddit_post() {
        let cfg = dummy_config();
        let content = "https://reddit.com/r/rust/comments/123456/why_rust_is_great/";
        let parsed = parse_message(content, &cfg).unwrap();

        assert_eq!(parsed.platform, Platform::Reddit);
        assert_eq!(parsed.original_url, "https://www.reddit.com/r/rust/comments/123456");
        assert_eq!(parsed.fixed_url, "https://rxddit.com/r/rust/comments/123456");
    }

    #[test]
    fn test_bluesky_post() {
        let cfg = dummy_config();
        let content = "||https://bsky.app/profile/alice.bsky.social/post/3k6abc123||";
        let parsed = parse_message(content, &cfg).unwrap();

        assert_eq!(parsed.platform, Platform::Bluesky);
        assert_eq!(parsed.original_url, "https://bsky.app/profile/alice.bsky.social/post/3k6abc123");
        assert_eq!(parsed.fixed_url, "https://fxbsky.app/profile/alice.bsky.social/post/3k6abc123");
        assert_eq!(parsed.is_spoiler, true);
    }
}

