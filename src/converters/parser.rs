use regex::Regex;
use std::sync::LazyLock;

use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Twitter,
    TikTok,
    Instagram,
    Reddit,
    Bluesky,
    Spotify,
    Twitch,
    Pixiv,
    Tumblr,
    Facebook,
    YouTube,
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
            Platform::Twitch => "Twitch",
            Platform::Pixiv => "Pixiv",
            Platform::Tumblr => "Tumblr",
            Platform::Facebook => "Facebook",
            Platform::YouTube => "YouTube",
        }
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
    /// Dönüştürülmüş link (örn: https://tnktok.com/@user/video/123). Proxy motoru
    /// kullanılmayan (native-only) platformlarda **orijinal linke eşittir**.
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

// ==================== YENİ KAMUYA AÇIK PLATFORM LİNKLERİ ====================
// Bu desenler yalnızca kanonik gönderi biçimlerini kabul eder; profil/arama sayfaları
// bilerek dönüştürülmez. URL'nin geri kalanındaki sorgu ve parça, normalleştirme sırasında
// atılır; böylece Markdown hedefinde kullanıcı tarafından eklenmiş karakterler taşınmaz.
static TWITCH_CLIP_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\|\|\s*(https?://(?:www\.)?(?:clips\.twitch\.tv/[A-Za-z0-9_-]+|twitch\.tv/[^/\s]+/clip/[A-Za-z0-9_-]+))[^\s|]*\s*\|\|")
        .expect("geçerli Twitch clip spoiler regex")
});
static TWITCH_CLIP_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)https?://(?:www\.)?(?:clips\.twitch\.tv/[A-Za-z0-9_-]+|twitch\.tv/[^/\s]+/clip/[A-Za-z0-9_-]+)[^\s<>()]*")
        .expect("geçerli Twitch clip regex")
});
static PIXIV_ARTWORK_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\|\|\s*(https?://(?:www\.)?pixiv\.net/(?:[a-z]{2}/)?artworks/[0-9]+)[^\s|]*\s*\|\|",
    )
    .expect("geçerli Pixiv spoiler regex")
});
static PIXIV_ARTWORK_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)https?://(?:www\.)?pixiv\.net/(?:[a-z]{2}/)?artworks/[0-9]+[^\s<>()]*")
        .expect("geçerli Pixiv regex")
});
static TUMBLR_POST_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\|\|\s*(https?://(?:www\.)?(?:[A-Za-z0-9-]+\.tumblr\.com/post/[0-9]+|tumblr\.com/(?:[^/\s]+/post/[0-9]+|[^/\s]+/[0-9]+)))[^\s|]*\s*\|\|")
        .expect("geçerli Tumblr spoiler regex")
});
static TUMBLR_POST_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)https?://(?:www\.)?(?:[A-Za-z0-9-]+\.tumblr\.com/post/[0-9]+|tumblr\.com/(?:[^/\s]+/post/[0-9]+|[^/\s]+/[0-9]+))[^\s<>()]*")
        .expect("geçerli Tumblr regex")
});
static FACEBOOK_POST_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\|\|\s*(https?://(?:www\.|m\.)?facebook\.com/(?:[^/\s]+/posts/(?:[0-9]+|pfbid[A-Za-z0-9_-]+)|watch/\?v=[0-9]+|videos/(?:[0-9]+|pfbid[A-Za-z0-9_-]+)|reel/[0-9]+))[^\s|]*\s*\|\|")
        .expect("geçerli Facebook spoiler regex")
});
static FACEBOOK_POST_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)https?://(?:www\.|m\.)?facebook\.com/(?:[^/\s]+/posts/(?:[0-9]+|pfbid[A-Za-z0-9_-]+)|watch/\?v=[0-9]+|videos/(?:[0-9]+|pfbid[A-Za-z0-9_-]+)|reel/[0-9]+)[^\s<>()]*")
        .expect("geçerli Facebook regex")
});
static YOUTUBE_COMMUNITY_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\|\|\s*(https?://(?:www\.|m\.)?youtube\.com/post/[A-Za-z0-9_-]+)[^\s|]*\s*\|\|",
    )
    .expect("geçerli YouTube spoiler regex")
});
static YOUTUBE_COMMUNITY_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)https?://(?:www\.|m\.)?youtube\.com/post/[A-Za-z0-9_-]+[^\s<>()]*")
        .expect("geçerli YouTube community regex")
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
/// Proxy motoru kullanılabilen platformların yedek alan adları.
///
/// Twitter/X, Instagram ve Reddit **native-only**'dir (proxy motoru ve proxy yedeği
/// kaldırıldı), bu yüzden burada yer almazlar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformDomains {
    pub tiktok: String,
    pub bluesky: String,
    pub spotify: String,
}

impl Default for PlatformDomains {
    fn default() -> Self {
        Self {
            tiktok: "tnktok.com".to_string(),
            bluesky: "fxbsky.app".to_string(),
            spotify: "open.fixspotify.com".to_string(),
        }
    }
}

impl From<&Config> for PlatformDomains {
    fn from(config: &Config) -> Self {
        Self {
            tiktok: config.tiktok_domain.clone(),
            bluesky: config.bluesky_domain.clone(),
            spotify: config.spotify_domain.clone(),
        }
    }
}

/// `.env` değerleriyle dönüştürme yapar (yalnızca testlerin kullandığı giriş noktası).
#[cfg(test)]
pub fn parse_message(raw_content: &str, config: &Config) -> Option<ParsedPost> {
    parse_message_with_domains(raw_content, &PlatformDomains::from(config))
}

/// Ayarlardan çözümlenmiş alan adlarıyla dönüştürme yapar.
pub fn parse_message_with_domains(
    raw_content: &str,
    domains: &PlatformDomains,
) -> Option<ParsedPost> {
    // Hızlı ön kontrol: Link olma ihtimali olmayan mesajlarda tüm regex'leri atla
    if !raw_content.contains("http://")
        && !raw_content.contains("https://")
        && !raw_content.contains("redd.it/")
    {
        return None;
    }

    // 1. Twitter / X — native-only: proxy motoru kaldırıldı, dönüştürme daima native.
    if let Some(post) = parse_twitter(raw_content) {
        return Some(sanitize_urls(post));
    }

    // 2. TikTok (okdargy/fxTikTok)
    if let Some(post) = parse_tiktok(raw_content, &domains.tiktok) {
        return Some(sanitize_urls(post));
    }

    // 3. Instagram — native-only.
    if let Some(post) = parse_instagram(raw_content) {
        return Some(sanitize_urls(post));
    }

    // 4. Reddit — native-only.
    if let Some(post) = parse_reddit(raw_content) {
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

    if let Some(post) = parse_twitch(raw_content) {
        return Some(sanitize_urls(post));
    }
    if let Some(post) = parse_pixiv(raw_content) {
        return Some(sanitize_urls(post));
    }
    if let Some(post) = parse_tumblr(raw_content) {
        return Some(sanitize_urls(post));
    }
    if let Some(post) = parse_facebook(raw_content) {
        return Some(sanitize_urls(post));
    }
    if let Some(post) = parse_youtube(raw_content) {
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

/// Twitter/X: **native-only** platform (proxy motoru kaldırıldı), bu yüzden "dönüştürülmüş"
/// link orijinal linkin kendisidir.
fn parse_twitter(raw_content: &str) -> Option<ParsedPost> {
    if let Some(caps) = TWITTER_SPOILER_REGEX.captures(raw_content) {
        let username = caps.get(1)?.as_str();
        let tweet_id = caps.get(2)?.as_str();
        let extra = caps
            .get(3)
            .map(|m| format!("/{}", m.as_str()))
            .unwrap_or_default();

        let original_url = format!("https://x.com/{}/status/{}{}", username, tweet_id, extra);
        let fixed_url = original_url.clone();

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
        let extra = caps
            .get(3)
            .map(|m| format!("/{}", m.as_str()))
            .unwrap_or_default();

        let original_url = format!("https://x.com/{}/status/{}{}", username, tweet_id, extra);
        let fixed_url = original_url.clone();

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

/// Instagram: **native-only** platform (proxy motoru kaldırıldı).
fn parse_instagram(raw_content: &str) -> Option<ParsedPost> {
    if let Some(caps) = INSTAGRAM_SPOILER_REGEX.captures(raw_content) {
        let kind = caps.get(1)?.as_str();
        let id = caps.get(2)?.as_str();
        let original_url = format!("https://www.instagram.com/{}/{}", kind, id);
        return Some(ParsedPost {
            platform: Platform::Instagram,
            is_spoiler: true,
            user_comment: extract_user_comment(raw_content, &INSTAGRAM_SPOILER_REGEX),
            fixed_url: original_url.clone(),
            original_url,
        });
    }
    if let Some(caps) = INSTAGRAM_REGEX.captures(raw_content) {
        let kind = caps.get(1)?.as_str();
        let id = caps.get(2)?.as_str();
        let original_url = format!("https://www.instagram.com/{}/{}", kind, id);
        return Some(ParsedPost {
            platform: Platform::Instagram,
            is_spoiler: false,
            user_comment: extract_user_comment(raw_content, &INSTAGRAM_REGEX),
            fixed_url: original_url.clone(),
            original_url,
        });
    }
    None
}

/// Reddit: **native-only** platform (proxy motoru kaldırıldı).
fn parse_reddit(raw_content: &str) -> Option<ParsedPost> {
    // 1. /r/sub/comments/id veya /(u|user)/username/comments/id (Spoiler)
    if let Some(caps) = REDDIT_POST_SPOILER_REGEX.captures(raw_content) {
        let prefix = caps.get(1)?.as_str();
        let target = caps.get(2)?.as_str();
        let id = caps.get(3)?.as_str();
        let section = if prefix == "u" { "user" } else { prefix };
        let original_url = format!(
            "https://www.reddit.com/{}/{}/comments/{}",
            section, target, id
        );
        return Some(ParsedPost {
            platform: Platform::Reddit,
            is_spoiler: true,
            user_comment: extract_user_comment(raw_content, &REDDIT_POST_SPOILER_REGEX),
            fixed_url: original_url.clone(),
            original_url,
        });
    }
    // 1. /r/sub/comments/id veya /(u|user)/username/comments/id (Normal)
    if let Some(caps) = REDDIT_POST_REGEX.captures(raw_content) {
        let prefix = caps.get(1)?.as_str();
        let target = caps.get(2)?.as_str();
        let id = caps.get(3)?.as_str();
        let section = if prefix == "u" { "user" } else { prefix };
        let original_url = format!(
            "https://www.reddit.com/{}/{}/comments/{}",
            section, target, id
        );
        return Some(ParsedPost {
            platform: Platform::Reddit,
            is_spoiler: false,
            user_comment: extract_user_comment(raw_content, &REDDIT_POST_REGEX),
            fixed_url: original_url.clone(),
            original_url,
        });
    }

    // 2. /r/sub/s/share_id (Mobil Paylaşım Linki - Spoiler)
    if let Some(caps) = REDDIT_SHARE_SPOILER_REGEX.captures(raw_content) {
        let sub = caps.get(1)?.as_str();
        let share_id = caps.get(2)?.as_str();
        let original_url = format!("https://www.reddit.com/r/{}/s/{}", sub, share_id);
        return Some(ParsedPost {
            platform: Platform::Reddit,
            is_spoiler: true,
            user_comment: extract_user_comment(raw_content, &REDDIT_SHARE_SPOILER_REGEX),
            fixed_url: original_url.clone(),
            original_url,
        });
    }
    // 2. /r/sub/s/share_id (Mobil Paylaşım Linki - Normal)
    if let Some(caps) = REDDIT_SHARE_REGEX.captures(raw_content) {
        let sub = caps.get(1)?.as_str();
        let share_id = caps.get(2)?.as_str();
        let original_url = format!("https://www.reddit.com/r/{}/s/{}", sub, share_id);
        return Some(ParsedPost {
            platform: Platform::Reddit,
            is_spoiler: false,
            user_comment: extract_user_comment(raw_content, &REDDIT_SHARE_REGEX),
            fixed_url: original_url.clone(),
            original_url,
        });
    }

    // 3. redd.it/id veya /comments/id (Kısa Linkler - Spoiler)
    if let Some(caps) = REDDIT_SHORT_SPOILER_REGEX.captures(raw_content) {
        let id = caps.get(1).or_else(|| caps.get(2))?.as_str();
        let original_url = format!("https://redd.it/{}", id);
        return Some(ParsedPost {
            platform: Platform::Reddit,
            is_spoiler: true,
            user_comment: extract_user_comment(raw_content, &REDDIT_SHORT_SPOILER_REGEX),
            fixed_url: original_url.clone(),
            original_url,
        });
    }
    // 3. redd.it/id veya /comments/id (Kısa Linkler - Normal)
    if let Some(caps) = REDDIT_SHORT_REGEX.captures(raw_content) {
        let id = caps.get(1).or_else(|| caps.get(2))?.as_str();
        let original_url = format!("https://redd.it/{}", id);
        return Some(ParsedPost {
            platform: Platform::Reddit,
            is_spoiler: false,
            user_comment: extract_user_comment(raw_content, &REDDIT_SHORT_REGEX),
            fixed_url: original_url.clone(),
            original_url,
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

fn canonical_clip_url(raw: &str) -> Option<String> {
    let raw = raw.trim_end_matches(['.', ',', '!', ';']);
    let path = raw.split(['?', '#']).next()?.trim_end_matches('/');
    let slug = path.rsplit('/').next()?;
    (!slug.is_empty()).then(|| format!("https://clips.twitch.tv/{slug}"))
}

fn parse_twitch(raw_content: &str) -> Option<ParsedPost> {
    for (regex, spoiler) in [
        (&*TWITCH_CLIP_SPOILER_REGEX, true),
        (&*TWITCH_CLIP_REGEX, false),
    ] {
        if let Some(caps) = regex.captures(raw_content) {
            let raw = caps.get(1).or_else(|| caps.get(0))?.as_str();
            let original_url = canonical_clip_url(raw)?;
            return Some(ParsedPost {
                platform: Platform::Twitch,
                is_spoiler: spoiler,
                user_comment: extract_user_comment(raw_content, regex),
                fixed_url: original_url.clone(),
                original_url,
            });
        }
    }
    None
}

fn canonical_path_url(raw: &str, host: &str) -> Option<String> {
    let raw = raw.trim_end_matches(['.', ',', '!', ';']);
    let parsed = url::Url::parse(raw).ok()?;
    let path = parsed.path().trim_end_matches('/');
    (!path.is_empty()).then(|| format!("https://{host}{path}"))
}

fn parse_pixiv(raw_content: &str) -> Option<ParsedPost> {
    for (regex, spoiler) in [
        (&*PIXIV_ARTWORK_SPOILER_REGEX, true),
        (&*PIXIV_ARTWORK_REGEX, false),
    ] {
        if let Some(caps) = regex.captures(raw_content) {
            let raw = caps.get(1).or_else(|| caps.get(0))?.as_str();
            let original_url = canonical_path_url(raw, "www.pixiv.net")?;
            return Some(ParsedPost {
                platform: Platform::Pixiv,
                is_spoiler: spoiler,
                user_comment: extract_user_comment(raw_content, regex),
                fixed_url: original_url.clone(),
                original_url,
            });
        }
    }
    None
}

fn tumblr_canonical(raw: &str) -> Option<String> {
    let raw = raw.trim_end_matches(['.', ',', '!', ';']);
    let parsed = url::Url::parse(raw).ok()?;
    let path = parsed.path().trim_end_matches('/');
    let segments: Vec<&str> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let host = parsed.host_str()?.to_ascii_lowercase();
    let blog = host
        .strip_suffix(".tumblr.com")
        .filter(|b| *b != "www")
        .or_else(|| segments.first().copied())?;
    let id_index = if host.ends_with(".tumblr.com") {
        1
    } else if segments.get(1) == Some(&"post") {
        2
    } else {
        1
    };
    let id = segments
        .get(id_index)
        .filter(|id| id.chars().all(|c| c.is_ascii_digit()))?;
    if host == "tumblr.com" || host == "www.tumblr.com" {
        return Some(format!("https://www.tumblr.com/{blog}/{id}"));
    }
    Some(format!("https://{blog}.tumblr.com/post/{id}"))
}

fn parse_tumblr(raw_content: &str) -> Option<ParsedPost> {
    for (regex, spoiler) in [
        (&*TUMBLR_POST_SPOILER_REGEX, true),
        (&*TUMBLR_POST_REGEX, false),
    ] {
        if let Some(caps) = regex.captures(raw_content) {
            let raw = caps.get(1).or_else(|| caps.get(0))?.as_str();
            let original_url = tumblr_canonical(raw)?;
            return Some(ParsedPost {
                platform: Platform::Tumblr,
                is_spoiler: spoiler,
                user_comment: extract_user_comment(raw_content, regex),
                fixed_url: original_url.clone(),
                original_url,
            });
        }
    }
    None
}

fn parse_facebook(raw_content: &str) -> Option<ParsedPost> {
    for (regex, spoiler) in [
        (&*FACEBOOK_POST_SPOILER_REGEX, true),
        (&*FACEBOOK_POST_REGEX, false),
    ] {
        if let Some(caps) = regex.captures(raw_content) {
            let raw = caps.get(1).or_else(|| caps.get(0))?.as_str();
            let raw = raw.trim_end_matches(['.', ',', '!', ';']);
            let parsed = url::Url::parse(raw).ok()?;
            let mut original_url = format!("https://www.facebook.com{}", parsed.path());
            if let Some(query) = parsed.query()
                && parsed.path() == "/watch/"
            {
                original_url.push('?');
                original_url.push_str(query);
            }
            return Some(ParsedPost {
                platform: Platform::Facebook,
                is_spoiler: spoiler,
                user_comment: extract_user_comment(raw_content, regex),
                fixed_url: original_url.clone(),
                original_url,
            });
        }
    }
    None
}

fn parse_youtube(raw_content: &str) -> Option<ParsedPost> {
    for (regex, spoiler) in [
        (&*YOUTUBE_COMMUNITY_SPOILER_REGEX, true),
        (&*YOUTUBE_COMMUNITY_REGEX, false),
    ] {
        if let Some(caps) = regex.captures(raw_content) {
            let raw = caps.get(1).or_else(|| caps.get(0))?.as_str();
            let original_url = canonical_path_url(raw, "www.youtube.com")?;
            return Some(ParsedPost {
                platform: Platform::YouTube,
                is_spoiler: spoiler,
                user_comment: extract_user_comment(raw_content, regex),
                fixed_url: original_url.clone(),
                original_url,
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_config() -> Config {
        Config {
            discord_token: "dummy".to_string(),
            tiktok_domain: "tnktok.com".to_string(),
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
        assert_eq!(
            parsed.original_url,
            "https://x.com/elonmusk/status/123456789"
        );
        assert_eq!(parsed.fixed_url, parsed.original_url);
        assert!(!parsed.is_spoiler);
        assert_eq!(
            parsed.user_comment.as_deref(),
            Some("Harika video  mutlaka izleyin")
        );
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
        assert_eq!(
            parsed.original_url,
            "https://www.tiktok.com/@scout2015/video/6718335390845095173"
        );
        assert_eq!(
            parsed.fixed_url,
            "https://tnktok.com/@scout2015/video/6718335390845095173"
        );
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
        assert_eq!(
            parsed.original_url,
            "https://www.instagram.com/reel/C3abcXYZ123"
        );
        assert_eq!(
            parsed.fixed_url, parsed.original_url,
            "native-only platform: proxy linki üretilmez"
        );
    }

    #[test]
    fn test_reddit_post() {
        let cfg = dummy_config();
        let content = "https://reddit.com/r/rust/comments/123456/why_rust_is_great/";
        let parsed = parse_message(content, &cfg).unwrap();

        assert_eq!(parsed.platform, Platform::Reddit);
        assert_eq!(
            parsed.original_url,
            "https://www.reddit.com/r/rust/comments/123456"
        );
        assert_eq!(parsed.fixed_url, parsed.original_url);
        assert!(!parsed.is_spoiler);
    }

    #[test]
    fn test_reddit_mobile_share() {
        let cfg = dummy_config();
        let content = "Buna bak https://www.reddit.com/r/pcmasterrace/s/AbCdEf1234 harika";
        let parsed = parse_message(content, &cfg).unwrap();

        assert_eq!(parsed.platform, Platform::Reddit);
        assert_eq!(
            parsed.original_url,
            "https://www.reddit.com/r/pcmasterrace/s/AbCdEf1234"
        );
        assert_eq!(parsed.fixed_url, parsed.original_url);
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
        assert_eq!(parsed.fixed_url, parsed.original_url);
        assert!(!parsed.is_spoiler);
    }

    #[test]
    fn test_reddit_user_post() {
        let cfg = dummy_config();
        let content = "https://www.reddit.com/u/someone/comments/abcdef12/";
        let parsed = parse_message(content, &cfg).unwrap();

        assert_eq!(parsed.platform, Platform::Reddit);
        assert_eq!(
            parsed.original_url,
            "https://www.reddit.com/user/someone/comments/abcdef12"
        );
        assert_eq!(parsed.fixed_url, parsed.original_url);
    }

    #[test]
    fn test_reddit_spoiler() {
        let cfg = dummy_config();
        let content = "||https://old.reddit.com/r/news/comments/789xyz/breaking/||";
        let parsed = parse_message(content, &cfg).unwrap();

        assert_eq!(parsed.platform, Platform::Reddit);
        assert_eq!(
            parsed.original_url,
            "https://www.reddit.com/r/news/comments/789xyz"
        );
        assert_eq!(parsed.fixed_url, parsed.original_url);
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
            parsed.fixed_url, parsed.original_url,
            "native-only platform: dönüştürülmüş link orijinal linktir"
        );
    }

    #[test]
    fn already_encoded_urls_are_left_intact() {
        let cfg = dummy_config();
        let content = "https://x.com/a%20b/status/123";
        let parsed = parse_message(content, &cfg).unwrap();
        assert_eq!(parsed.fixed_url, "https://x.com/a%20b/status/123");
    }

    #[test]
    fn test_bluesky_post() {
        let cfg = dummy_config();
        let content = "||https://bsky.app/profile/alice.bsky.social/post/3k6abc123||";
        let parsed = parse_message(content, &cfg).unwrap();

        assert_eq!(parsed.platform, Platform::Bluesky);
        assert_eq!(
            parsed.original_url,
            "https://bsky.app/profile/alice.bsky.social/post/3k6abc123"
        );
        assert_eq!(
            parsed.fixed_url,
            "https://fxbsky.app/profile/alice.bsky.social/post/3k6abc123"
        );
        assert!(parsed.is_spoiler);
    }

    #[test]
    fn test_spotify_track() {
        let cfg = dummy_config();
        let content =
            "Şu şarkıya bak https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT?si=abc harika";
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
        assert_eq!(
            parsed.user_comment.as_deref(),
            Some("Şu şarkıya bak  harika")
        );
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

    #[test]
    fn new_platform_urls_have_canonical_forms_and_comments() {
        let cfg = dummy_config();
        let cases = [
            (
                "bak https://www.twitch.tv/channel/clip/Ab_C-123 not",
                Platform::Twitch,
                "https://clips.twitch.tv/Ab_C-123",
            ),
            (
                "||https://www.pixiv.net/en/artworks/12345||",
                Platform::Pixiv,
                "https://www.pixiv.net/en/artworks/12345",
            ),
            (
                "not https://artist.tumblr.com/post/987654321?x=1",
                Platform::Tumblr,
                "https://artist.tumblr.com/post/987654321",
            ),
            (
                "https://www.tumblr.com/ballots/829790623215321088?source=share",
                Platform::Tumblr,
                "https://www.tumblr.com/ballots/829790623215321088",
            ),
            (
                "https://www.facebook.com/example/posts/123456?comment_id=2",
                Platform::Facebook,
                "https://www.facebook.com/example/posts/123456",
            ),
            (
                "https://www.facebook.com/NASA/posts/pfbid02yLSD4sQr4mcpEEKwHR3jVmJmRMk6ReLukg697i5a731BXyY8mZ4hwwTT8s49852bl",
                Platform::Facebook,
                "https://www.facebook.com/NASA/posts/pfbid02yLSD4sQr4mcpEEKwHR3jVmJmRMk6ReLukg697i5a731BXyY8mZ4hwwTT8s49852bl",
            ),
            (
                "hello https://m.youtube.com/post/UCabc_123",
                Platform::YouTube,
                "https://www.youtube.com/post/UCabc_123",
            ),
        ];
        for (input, platform, url) in cases {
            let parsed = parse_message(input, &cfg).expect("yeni platform linki bulunmalı");
            assert_eq!(parsed.platform, platform);
            assert_eq!(parsed.original_url, url);
            assert_eq!(parsed.fixed_url, url);
        }
        assert!(
            parse_message("||https://www.pixiv.net/en/artworks/12345||", &cfg)
                .expect("spoiler")
                .is_spoiler
        );
    }

    #[test]
    fn non_canonical_new_platform_pages_are_ignored() {
        let cfg = dummy_config();
        for input in [
            "https://www.twitch.tv/channel",
            "https://www.pixiv.net/users/123",
            "https://artist.tumblr.com/",
            "https://www.facebook.com/example",
            "https://www.youtube.com/channel/abc",
        ] {
            assert!(parse_message(input, &cfg).is_none(), "{input}");
        }
    }
}
