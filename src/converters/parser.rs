use std::sync::LazyLock;
use regex::Regex;

use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Twitter,
    TikTok,
    Instagram,
    Reddit,
    Bluesky,
    Spotify,
}

impl Platform {
    pub fn display_name(&self) -> &'static str {
        match self {
            Platform::Twitter => "Twitter",
            Platform::TikTok => "TikTok",
            Platform::Instagram => "Instagram",
            Platform::Reddit => "Reddit",
            Platform::Bluesky => "Bluesky",
            Platform::Spotify => "Spotify",
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
static TWITTER_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\|\|\s*https?://(?:www\.)?(?:x|twitter)\.com/([^/\s]+)/status/(\d+)(?:/([a-zA-Z0-9_/]+))?/?(?:\?[^\s\|]*)?\s*\|\|")
        .expect("Geçersiz Spoiler Twitter regex deseni")
});
static TWITTER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?:www\.)?(?:x|twitter)\.com/([^/\s]+)/status/(\d+)(?:/([a-zA-Z0-9_/]+))?/?(?:\?[^\s]*)?")
        .expect("Geçersiz Twitter regex deseni")
});

// ==================== TIKTOK ====================
// TikTok: @user/(video|photo)/id
static TIKTOK_VIDEO_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\|\|\s*https?://(?:www\.)?tiktok\.com/@([^/\s]+)/(video|photo)/(\d+)/?(?:\?[^\s\|]*)?\s*\|\|")
        .expect("Geçersiz Spoiler TikTok Video regex deseni")
});
static TIKTOK_VIDEO_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?:www\.)?tiktok\.com/@([^/\s]+)/(video|photo)/(\d+)/?(?:\?[^\s]*)?")
        .expect("Geçersiz TikTok Video regex deseni")
});

// TikTok: /t/id (Kısa link)
static TIKTOK_T_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\|\|\s*https?://(?:www\.)?tiktok\.com/t/([a-zA-Z0-9_-]+)/?(?:\?[^\s\|]*)?\s*\|\|")
        .expect("Geçersiz Spoiler TikTok /t/ regex deseni")
});
static TIKTOK_T_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?:www\.)?tiktok\.com/t/([a-zA-Z0-9_-]+)/?(?:\?[^\s]*)?")
        .expect("Geçersiz TikTok /t/ regex deseni")
});

// TikTok: vm.tiktok.com veya vt.tiktok.com
static TIKTOK_SHORT_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\|\|\s*https?://(vm|vt)\.tiktok\.com/([a-zA-Z0-9_-]+)/?(?:\?[^\s\|]*)?\s*\|\|")
        .expect("Geçersiz Spoiler TikTok VM/VT regex deseni")
});
static TIKTOK_SHORT_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(vm|vt)\.tiktok\.com/([a-zA-Z0-9_-]+)/?(?:\?[^\s]*)?")
        .expect("Geçersiz TikTok VM/VT regex deseni")
});

// ==================== INSTAGRAM ====================
static INSTAGRAM_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\|\|\s*https?://(?:www\.)?instagram\.com/(p|reel|reels|tv)/([a-zA-Z0-9_-]+)(?:/[^\s\|]*)?/?(?:\?[^\s\|]*)?\s*\|\|")
        .expect("Geçersiz Spoiler Instagram regex deseni")
});
static INSTAGRAM_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?:www\.)?instagram\.com/(p|reel|reels|tv)/([a-zA-Z0-9_-]+)(?:/[^\s]*)?/?(?:\?[^\s]*)?")
        .expect("Geçersiz Instagram regex deseni")
});

// ==================== REDDIT ====================
// 1. Standart Gönderi: /r/subreddit/comments/id veya /(u|user)/username/comments/id
static REDDIT_POST_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\|\|\s*https?://(?:(?:www|old|new|np)\.)?reddit\.com/(r|user|u)/([^/\s]+)/comments/([a-zA-Z0-9]+)(?:/[^\s\|]*)?/?(?:\?[^\s\|]*)?\s*\|\|")
        .expect("Geçersiz Spoiler Reddit Post regex deseni")
});
static REDDIT_POST_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?:(?:www|old|new|np)\.)?reddit\.com/(r|user|u)/([^/\s]+)/comments/([a-zA-Z0-9]+)(?:/[^\s]*)?/?(?:\?[^\s]*)?")
        .expect("Geçersiz Reddit Post regex deseni")
});

// 2. Mobil Paylaşım Linki: /r/subreddit/s/share_id
static REDDIT_SHARE_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\|\|\s*https?://(?:(?:www|old|new|np)\.)?reddit\.com/r/([^/\s]+)/s/([a-zA-Z0-9_-]+)/?(?:\?[^\s\|]*)?\s*\|\|")
        .expect("Geçersiz Spoiler Reddit Share regex deseni")
});
static REDDIT_SHARE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?:(?:www|old|new|np)\.)?reddit\.com/r/([^/\s]+)/s/([a-zA-Z0-9_-]+)/?(?:\?[^\s]*)?")
        .expect("Geçersiz Reddit Share regex deseni")
});

// 3. Kısa Linkler: redd.it/id veya /comments/id
static REDDIT_SHORT_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\|\|\s*(?:https?://redd\.it/([a-zA-Z0-9]+)|https?://(?:(?:www|old|new|np)\.)?reddit\.com/comments/([a-zA-Z0-9]+))(?:/[^\s\|]*)?/?(?:\?[^\s\|]*)?\s*\|\|")
        .expect("Geçersiz Spoiler Reddit Short regex deseni")
});
static REDDIT_SHORT_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:https?://redd\.it/([a-zA-Z0-9]+)|https?://(?:(?:www|old|new|np)\.)?reddit\.com/comments/([a-zA-Z0-9]+))(?:/[^\s]*)?/?(?:\?[^\s]*)?")
        .expect("Geçersiz Reddit Short regex deseni")
});

// ==================== BLUESKY ====================
static BLUESKY_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\|\|\s*https?://(?:www\.)?bsky\.app/profile/([^/\s]+)/post/([a-zA-Z0-9]+)/?(?:\?[^\s\|]*)?\s*\|\|")
        .expect("Geçersiz Spoiler Bluesky regex deseni")
});
static BLUESKY_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?:www\.)?bsky\.app/profile/([^/\s]+)/post/([a-zA-Z0-9]+)/?(?:\?[^\s]*)?")
        .expect("Geçersiz Bluesky regex deseni")
});

// ==================== SPOTIFY ====================
// open.spotify.com/(intl-xx/)?(track|album|playlist|artist|episode|show)/id
// Spotify kimlikleri base62 (22 karakter) ama geriye dönük uyum için 16-40 aralığı kabul edilir.
static SPOTIFY_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\|\|\s*https?://open\.spotify\.com/(?:intl-[a-zA-Z-]+/)?(track|album|playlist|artist|episode|show)/([a-zA-Z0-9]{16,40})[^\s|]*\s*\|\|")
        .expect("Geçersiz Spoiler Spotify regex deseni")
});
static SPOTIFY_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://open\.spotify\.com/(?:intl-[a-zA-Z-]+/)?(track|album|playlist|artist|episode|show)/([a-zA-Z0-9]{16,40})/?(?:\?[^\s]*)?")
        .expect("Geçersiz Spotify regex deseni")
});

// spotify.link/token (Branch kısa linki) — native motorda hedefe çözülür.
static SPOTIFY_SHORT_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\|\|\s*https?://spotify\.link/([A-Za-z0-9]+)[^\s|]*\s*\|\|")
        .expect("Geçersiz Spoiler Spotify kısa link regex deseni")
});
static SPOTIFY_SHORT_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://spotify\.link/([A-Za-z0-9]+)")
        .expect("Geçersiz Spotify kısa link regex deseni")
});

/// Characters that would prematurely close or corrupt the Markdown link the
/// converter wraps these URLs in (`[:](url)` / `[**Platform**](<url>)`).
/// Left unescaped they let a crafted post URL inject arbitrary link text and a
/// second, attacker-chosen link target into a bot-authored message.
const MARKDOWN_UNSAFE: &[u8] = b"()[]<>\"{}|\\^`";

/// Percent-encodes URL characters that are unsafe inside Markdown link syntax.
/// `%` is deliberately passed through so already-encoded links stay intact.
fn encode_markdown_unsafe(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_control() {
            continue;
        }
        let byte = ch as u32;
        if byte < 128 && MARKDOWN_UNSAFE.contains(&(byte as u8)) {
            out.push_str(&format!("%{byte:02X}"));
        } else {
            out.push(ch);
        }
    }
    out
}

/// Escapes both URLs of a parsed post so they can be embedded in Markdown safely.
fn sanitize_urls(post: ParsedPost) -> ParsedPost {
    ParsedPost {
        original_url: encode_markdown_unsafe(&post.original_url),
        fixed_url: encode_markdown_unsafe(&post.fixed_url),
        ..post
    }
}

/// Verilen mesaj içeriğini tüm desteklenen platformlar için analiz eder.
/// Dönüştürme sırasında kullanılacak platform alan adları (proxy hedefleri).
/// `.env` değerleri varsayılandır, WebUI üzerinden sunucu bazında değiştirilebilir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformDomains {
    pub twitter: String,
    pub tiktok: String,
    pub instagram: String,
    pub reddit: String,
    pub bluesky: String,
    pub spotify: String,
}

impl Default for PlatformDomains {
    fn default() -> Self {
        Self {
            twitter: "fixupx.com".to_string(),
            tiktok: "tnktok.com".to_string(),
            instagram: "ddinstagram.com".to_string(),
            reddit: "vxreddit.com".to_string(),
            bluesky: "fxbsky.app".to_string(),
            spotify: "open.fixspotify.com".to_string(),
        }
    }
}

impl From<&Config> for PlatformDomains {
    fn from(config: &Config) -> Self {
        Self {
            twitter: config.twitter_domain.clone(),
            tiktok: config.tiktok_domain.clone(),
            instagram: config.instagram_domain.clone(),
            reddit: config.reddit_domain.clone(),
            bluesky: config.bluesky_domain.clone(),
            spotify: config.spotify_domain.clone(),
        }
    }
}

/// `.env` değerleriyle dönüştürme yapar (geriye dönük uyumlu giriş noktası; testler kullanır).
#[allow(dead_code)]
pub fn parse_message(raw_content: &str, config: &Config) -> Option<ParsedPost> {
    parse_message_with_domains(raw_content, &PlatformDomains::from(config))
}

/// Ayarlardan çözümlenmiş alan adlarıyla dönüştürme yapar.
pub fn parse_message_with_domains(raw_content: &str, domains: &PlatformDomains) -> Option<ParsedPost> {
    // Hızlı ön kontrol: Link olma ihtimali olmayan mesajlarda tüm regex'leri atla
    if !raw_content.contains("http://")
        && !raw_content.contains("https://")
        && !raw_content.contains("redd.it/")
    {
        return None;
    }

    // 1. Twitter / X
    if let Some(post) = parse_twitter(raw_content, &domains.twitter) {
        return Some(sanitize_urls(post));
    }

    // 2. TikTok (okdargy/fxTikTok)
    if let Some(post) = parse_tiktok(raw_content, &domains.tiktok) {
        return Some(sanitize_urls(post));
    }

    // 3. Instagram
    if let Some(post) = parse_instagram(raw_content, &domains.instagram) {
        return Some(sanitize_urls(post));
    }

    // 4. Reddit
    if let Some(post) = parse_reddit(raw_content, &domains.reddit) {
        return Some(sanitize_urls(post));
    }

    // 5. Bluesky
    if let Some(post) = parse_bluesky(raw_content, &domains.bluesky) {
        return Some(sanitize_urls(post));
    }

    // 6. Spotify
    if let Some(post) = parse_spotify(raw_content, &domains.spotify) {
        return Some(sanitize_urls(post));
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
    // 1. /r/sub/comments/id veya /(u|user)/username/comments/id (Spoiler)
    if let Some(caps) = REDDIT_POST_SPOILER_REGEX.captures(raw_content) {
        let prefix = caps.get(1)?.as_str();
        let target = caps.get(2)?.as_str();
        let id = caps.get(3)?.as_str();
        let section = if prefix == "u" { "user" } else { prefix };
        return Some(ParsedPost {
            platform: Platform::Reddit,
            is_spoiler: true,
            user_comment: extract_user_comment(raw_content, &REDDIT_POST_SPOILER_REGEX),
            original_url: format!("https://www.reddit.com/{}/{}/comments/{}", section, target, id),
            fixed_url: format!("https://{}/{}/{}/comments/{}", domain, section, target, id),
        });
    }
    // 1. /r/sub/comments/id veya /(u|user)/username/comments/id (Normal)
    if let Some(caps) = REDDIT_POST_REGEX.captures(raw_content) {
        let prefix = caps.get(1)?.as_str();
        let target = caps.get(2)?.as_str();
        let id = caps.get(3)?.as_str();
        let section = if prefix == "u" { "user" } else { prefix };
        return Some(ParsedPost {
            platform: Platform::Reddit,
            is_spoiler: false,
            user_comment: extract_user_comment(raw_content, &REDDIT_POST_REGEX),
            original_url: format!("https://www.reddit.com/{}/{}/comments/{}", section, target, id),
            fixed_url: format!("https://{}/{}/{}/comments/{}", domain, section, target, id),
        });
    }

    // 2. /r/sub/s/share_id (Mobil Paylaşım Linki - Spoiler)
    if let Some(caps) = REDDIT_SHARE_SPOILER_REGEX.captures(raw_content) {
        let sub = caps.get(1)?.as_str();
        let share_id = caps.get(2)?.as_str();
        return Some(ParsedPost {
            platform: Platform::Reddit,
            is_spoiler: true,
            user_comment: extract_user_comment(raw_content, &REDDIT_SHARE_SPOILER_REGEX),
            original_url: format!("https://www.reddit.com/r/{}/s/{}", sub, share_id),
            fixed_url: format!("https://{}/r/{}/s/{}", domain, sub, share_id),
        });
    }
    // 2. /r/sub/s/share_id (Mobil Paylaşım Linki - Normal)
    if let Some(caps) = REDDIT_SHARE_REGEX.captures(raw_content) {
        let sub = caps.get(1)?.as_str();
        let share_id = caps.get(2)?.as_str();
        return Some(ParsedPost {
            platform: Platform::Reddit,
            is_spoiler: false,
            user_comment: extract_user_comment(raw_content, &REDDIT_SHARE_REGEX),
            original_url: format!("https://www.reddit.com/r/{}/s/{}", sub, share_id),
            fixed_url: format!("https://{}/r/{}/s/{}", domain, sub, share_id),
        });
    }

    // 3. redd.it/id veya /comments/id (Kısa Linkler - Spoiler)
    if let Some(caps) = REDDIT_SHORT_SPOILER_REGEX.captures(raw_content) {
        let id = caps.get(1).or_else(|| caps.get(2))?.as_str();
        return Some(ParsedPost {
            platform: Platform::Reddit,
            is_spoiler: true,
            user_comment: extract_user_comment(raw_content, &REDDIT_SHORT_SPOILER_REGEX),
            original_url: format!("https://redd.it/{}", id),
            fixed_url: format!("https://{}/comments/{}", domain, id),
        });
    }
    // 3. redd.it/id veya /comments/id (Kısa Linkler - Normal)
    if let Some(caps) = REDDIT_SHORT_REGEX.captures(raw_content) {
        let id = caps.get(1).or_else(|| caps.get(2))?.as_str();
        return Some(ParsedPost {
            platform: Platform::Reddit,
            is_spoiler: false,
            user_comment: extract_user_comment(raw_content, &REDDIT_SHORT_REGEX),
            original_url: format!("https://redd.it/{}", id),
            fixed_url: format!("https://{}/comments/{}", domain, id),
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

fn parse_spotify(raw_content: &str, domain: &str) -> Option<ParsedPost> {
    // 1. Track / album / playlist / artist (spoiler)
    if let Some(caps) = SPOTIFY_SPOILER_REGEX.captures(raw_content) {
        let kind = caps.get(1)?.as_str();
        let id = caps.get(2)?.as_str();
        return Some(ParsedPost {
            platform: Platform::Spotify,
            is_spoiler: true,
            user_comment: extract_user_comment(raw_content, &SPOTIFY_SPOILER_REGEX),
            original_url: format!("https://open.spotify.com/{kind}/{id}"),
            fixed_url: format!("https://{domain}/{kind}/{id}"),
        });
    }
    // 1. Track / album / playlist / artist (normal)
    if let Some(caps) = SPOTIFY_REGEX.captures(raw_content) {
        let kind = caps.get(1)?.as_str();
        let id = caps.get(2)?.as_str();
        return Some(ParsedPost {
            platform: Platform::Spotify,
            is_spoiler: false,
            user_comment: extract_user_comment(raw_content, &SPOTIFY_REGEX),
            original_url: format!("https://open.spotify.com/{kind}/{id}"),
            fixed_url: format!("https://{domain}/{kind}/{id}"),
        });
    }
    // 2. spotify.link kısa link (spoiler)
    if let Some(caps) = SPOTIFY_SHORT_SPOILER_REGEX.captures(raw_content) {
        let token = caps.get(1)?.as_str();
        return Some(ParsedPost {
            platform: Platform::Spotify,
            is_spoiler: true,
            user_comment: extract_user_comment(raw_content, &SPOTIFY_SHORT_SPOILER_REGEX),
            original_url: format!("https://spotify.link/{token}"),
            fixed_url: format!("https://{domain}/{token}"),
        });
    }
    // 2. spotify.link kısa link (normal)
    if let Some(caps) = SPOTIFY_SHORT_REGEX.captures(raw_content) {
        let token = caps.get(1)?.as_str();
        return Some(ParsedPost {
            platform: Platform::Spotify,
            is_spoiler: false,
            user_comment: extract_user_comment(raw_content, &SPOTIFY_SHORT_REGEX),
            original_url: format!("https://spotify.link/{token}"),
            fixed_url: format!("https://{domain}/{token}"),
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
            reddit_domain: "vxreddit.com".to_string(),
            bluesky_domain: "fxbsky.app".to_string(),
            spotify_domain: "open.fixspotify.com".to_string(),
            delete_original: true,
            openrouter_api_key: None,
            reddit_client_id: "test-id".to_string(),
            instagram_sessionid: None,
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
        assert!(!parsed.is_spoiler);
        assert_eq!(parsed.user_comment.as_deref(), Some("Harika video  mutlaka izleyin"));
    }

    #[test]
    fn test_twitter_spoiler() {
        let cfg = dummy_config();
        let content = "Gizli: ||https://twitter.com/jack/status/20||";
        let parsed = parse_message(content, &cfg).unwrap();

        assert_eq!(parsed.platform, Platform::Twitter);
        assert!(parsed.is_spoiler);
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
        assert!(!parsed.is_spoiler);
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
        assert_eq!(parsed.fixed_url, "https://vxreddit.com/r/rust/comments/123456");
        assert!(!parsed.is_spoiler);
    }

    #[test]
    fn test_reddit_mobile_share() {
        let cfg = dummy_config();
        let content = "Buna bak https://www.reddit.com/r/pcmasterrace/s/AbCdEf1234 harika";
        let parsed = parse_message(content, &cfg).unwrap();

        assert_eq!(parsed.platform, Platform::Reddit);
        assert_eq!(parsed.original_url, "https://www.reddit.com/r/pcmasterrace/s/AbCdEf1234");
        assert_eq!(parsed.fixed_url, "https://vxreddit.com/r/pcmasterrace/s/AbCdEf1234");
        assert!(!parsed.is_spoiler);
        assert_eq!(parsed.user_comment.as_deref(), Some("Buna bak  harika"));
    }

    #[test]
    fn test_reddit_shortlink() {
        let cfg = dummy_config();
        let content = "https://redd.it/123456";
        let parsed = parse_message(content, &cfg).unwrap();

        assert_eq!(parsed.platform, Platform::Reddit);
        assert_eq!(parsed.original_url, "https://redd.it/123456");
        assert_eq!(parsed.fixed_url, "https://vxreddit.com/comments/123456");
        assert!(!parsed.is_spoiler);
    }

    #[test]
    fn test_reddit_user_post() {
        let cfg = dummy_config();
        let content = "https://www.reddit.com/u/someone/comments/abcdef12/";
        let parsed = parse_message(content, &cfg).unwrap();

        assert_eq!(parsed.platform, Platform::Reddit);
        assert_eq!(parsed.original_url, "https://www.reddit.com/user/someone/comments/abcdef12");
        assert_eq!(parsed.fixed_url, "https://vxreddit.com/user/someone/comments/abcdef12");
    }

    #[test]
    fn test_reddit_spoiler() {
        let cfg = dummy_config();
        let content = "||https://old.reddit.com/r/news/comments/789xyz/breaking/||";
        let parsed = parse_message(content, &cfg).unwrap();

        assert_eq!(parsed.platform, Platform::Reddit);
        assert_eq!(parsed.original_url, "https://www.reddit.com/r/news/comments/789xyz");
        assert_eq!(parsed.fixed_url, "https://vxreddit.com/r/news/comments/789xyz");
        assert!(parsed.is_spoiler);
    }

    #[test]
    fn crafted_url_cannot_break_out_of_markdown_link() {
        let cfg = dummy_config();
        // Kapalı parantezle [:](...) bitişini kırıp ikinci bir link hedefi enjekte etme girişimi
        let content = "https://www.reddit.com/r/x)y[sahte]/comments/abc";
        let parsed = parse_message(content, &cfg).unwrap();

        for url in [&parsed.original_url, &parsed.fixed_url] {
            assert!(!url.contains(')'), "Kaçırılmamış parantez: {url}");
            assert!(!url.contains('('), "Kaçırılmamış parantez: {url}");
            assert!(!url.contains('['), "Kaçırılmamış köşeli parantez: {url}");
            assert!(!url.contains(']'), "Kaçırılmamış köşeli parantez: {url}");
        }
        assert_eq!(
            parsed.fixed_url,
            "https://vxreddit.com/r/x%29y%5Bsahte%5D/comments/abc"
        );
    }

    #[test]
    fn already_encoded_urls_are_left_intact() {
        let cfg = dummy_config();
        let content = "https://x.com/a%20b/status/123";
        let parsed = parse_message(content, &cfg).unwrap();
        assert_eq!(parsed.fixed_url, "https://fixupx.com/a%20b/status/123");
    }

    #[test]
    fn test_bluesky_post() {
        let cfg = dummy_config();
        let content = "||https://bsky.app/profile/alice.bsky.social/post/3k6abc123||";
        let parsed = parse_message(content, &cfg).unwrap();

        assert_eq!(parsed.platform, Platform::Bluesky);
        assert_eq!(parsed.original_url, "https://bsky.app/profile/alice.bsky.social/post/3k6abc123");
        assert_eq!(parsed.fixed_url, "https://fxbsky.app/profile/alice.bsky.social/post/3k6abc123");
        assert!(parsed.is_spoiler);
    }

    #[test]
    fn test_spotify_track() {
        let cfg = dummy_config();
        let content = "Şu şarkıya bak https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT?si=abc harika";
        let parsed = parse_message(content, &cfg).unwrap();

        assert_eq!(parsed.platform, Platform::Spotify);
        assert_eq!(
            parsed.original_url,
            "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT"
        );
        assert_eq!(
            parsed.fixed_url,
            "https://open.fixspotify.com/track/4cOdK2wGLETKBW3PvgPWqT"
        );
        assert!(!parsed.is_spoiler);
        assert_eq!(parsed.user_comment.as_deref(), Some("Şu şarkıya bak  harika"));
    }

    #[test]
    fn test_spotify_intl_and_spoiler() {
        let cfg = dummy_config();
        let content = "||https://open.spotify.com/intl-tr/album/1DFixLWuPkv3KT3TnV35m3||";
        let parsed = parse_message(content, &cfg).unwrap();

        assert_eq!(parsed.platform, Platform::Spotify);
        assert_eq!(
            parsed.original_url,
            "https://open.spotify.com/album/1DFixLWuPkv3KT3TnV35m3"
        );
        assert_eq!(
            parsed.fixed_url,
            "https://open.fixspotify.com/album/1DFixLWuPkv3KT3TnV35m3"
        );
        assert!(parsed.is_spoiler);
    }

    #[test]
    fn test_spotify_short_link() {
        let cfg = dummy_config();
        let content = "https://spotify.link/AbCdEf1234";
        let parsed = parse_message(content, &cfg).unwrap();

        assert_eq!(parsed.platform, Platform::Spotify);
        assert_eq!(parsed.original_url, "https://spotify.link/AbCdEf1234");
        assert_eq!(parsed.fixed_url, "https://open.fixspotify.com/AbCdEf1234");
    }
}

