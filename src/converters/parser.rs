use regex::{Captures, Regex};
use std::sync::LazyLock;

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
    Letterboxd,
    Miniflux,
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
            Platform::Letterboxd => "Letterboxd",
            Platform::Miniflux => "Miniflux",
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
}

// ==================== TWITTER / X ====================
static TWITTER_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\|\|\s*https?://(?i:(?:(?:www|mobile)\.)?(?:x|twitter)\.com)/(?:i/web|([^/\s]+))/status/(\d+)(?:/([a-zA-Z0-9_/]+))?/?(?:\?[^\s\|]*)?\s*\|\|")
        .expect("Geçersiz Spoiler Twitter regex deseni")
});
static TWITTER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?i:(?:(?:www|mobile)\.)?(?:x|twitter)\.com)/(?:i/web|([^/\s]+))/status/(\d+)(?:/([a-zA-Z0-9_/]+))?/?(?:\?[^\s]*)?")
        .expect("Geçersiz Twitter regex deseni")
});

// ==================== TIKTOK ====================
// TikTok: @user/(video|photo)/id
static TIKTOK_VIDEO_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\|\|\s*https?://(?i:(?:www\.)?tiktok\.com)/@([^/\s]+)/(video|photo)/(\d+)/?(?:\?[^\s\|]*)?\s*\|\|")
        .expect("Geçersiz Spoiler TikTok Video regex deseni")
});
static TIKTOK_VIDEO_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?i:(?:www\.)?tiktok\.com)/@([^/\s]+)/(video|photo)/(\d+)/?(?:\?[^\s]*)?")
        .expect("Geçersiz TikTok Video regex deseni")
});

// TikTok: /t/id (Kısa link)
static TIKTOK_T_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\|\|\s*https?://(?i:(?:www\.)?tiktok\.com)/t/([a-zA-Z0-9_-]+)/?(?:\?[^\s\|]*)?\s*\|\|",
    )
    .expect("Geçersiz Spoiler TikTok /t/ regex deseni")
});
static TIKTOK_T_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?i:(?:www\.)?tiktok\.com)/t/([a-zA-Z0-9_-]+)/?(?:\?[^\s]*)?")
        .expect("Geçersiz TikTok /t/ regex deseni")
});

// TikTok: vm.tiktok.com veya vt.tiktok.com
static TIKTOK_SHORT_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\|\|\s*https?://(?i:(vm|vt)\.tiktok\.com)/([a-zA-Z0-9_-]+)/?(?:\?[^\s\|]*)?\s*\|\|",
    )
    .expect("Geçersiz Spoiler TikTok VM/VT regex deseni")
});
static TIKTOK_SHORT_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?i:(vm|vt)\.tiktok\.com)/([a-zA-Z0-9_-]+)/?(?:\?[^\s]*)?")
        .expect("Geçersiz TikTok VM/VT regex deseni")
});

// ==================== INSTAGRAM ====================
static INSTAGRAM_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\|\|\s*https?://(?i:(?:www\.)?instagram\.com)/(?:[A-Za-z0-9_.]{1,30}/)?(p|reel|reels|tv)/([a-zA-Z0-9_-]+)(?:/[^\s\|]*)?/?(?:\?[^\s\|]*)?\s*\|\|")
        .expect("Geçersiz Spoiler Instagram regex deseni")
});
static INSTAGRAM_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?i:(?:www\.)?instagram\.com)/(?:[A-Za-z0-9_.]{1,30}/)?(p|reel|reels|tv)/([a-zA-Z0-9_-]+)(?:/[^\s]*)?/?(?:\?[^\s]*)?")
        .expect("Geçersiz Instagram regex deseni")
});

// ==================== REDDIT ====================
// 1. Standart Gönderi: /r/subreddit/comments/id veya /(u|user)/username/comments/id
// (yorum bağlantısında `/başlık/yorum_id` dördüncü grupta yakalanır)
static REDDIT_POST_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\|\|\s*https?://(?i:(?:(?:www|old|new|np|m|sh)\.)?reddit\.com)/(r|user|u)/([^/\s]+)/comments/([a-zA-Z0-9]+)(?:/[^/\s|?#]+/([a-zA-Z0-9]+))?(?:/[^\s\|]*)?/?(?:\?[^\s\|]*)?\s*\|\|")
        .expect("Geçersiz Spoiler Reddit Post regex deseni")
});
static REDDIT_POST_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?i:(?:(?:www|old|new|np|m|sh)\.)?reddit\.com)/(r|user|u)/([^/\s]+)/comments/([a-zA-Z0-9]+)(?:/[^/\s?#]+/([a-zA-Z0-9]+))?(?:/[^\s]*)?/?(?:\?[^\s]*)?")
        .expect("Geçersiz Reddit Post regex deseni")
});

// 2. Mobil Paylaşım Linki: /r/subreddit/s/share_id
static REDDIT_SHARE_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\|\|\s*https?://(?i:(?:(?:www|old|new|np|m|sh)\.)?reddit\.com)/r/([^/\s]+)/s/([a-zA-Z0-9_-]+)/?(?:\?[^\s\|]*)?\s*\|\|")
        .expect("Geçersiz Spoiler Reddit Share regex deseni")
});
static REDDIT_SHARE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?i:(?:(?:www|old|new|np|m|sh)\.)?reddit\.com)/r/([^/\s]+)/s/([a-zA-Z0-9_-]+)/?(?:\?[^\s]*)?")
        .expect("Geçersiz Reddit Share regex deseni")
});

// 3. Kısa Linkler: redd.it/id veya /comments/id
static REDDIT_SHORT_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\|\|\s*(?:https?://(?i:redd\.it)/([a-zA-Z0-9]+)|https?://(?i:(?:(?:www|old|new|np|m|sh)\.)?reddit\.com)/comments/([a-zA-Z0-9]+))(?:/[^\s\|]*)?/?(?:\?[^\s\|]*)?\s*\|\|")
        .expect("Geçersiz Spoiler Reddit Short regex deseni")
});
static REDDIT_SHORT_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:https?://(?i:redd\.it)/([a-zA-Z0-9]+)|https?://(?i:(?:(?:www|old|new|np|m|sh)\.)?reddit\.com)/comments/([a-zA-Z0-9]+))(?:/[^\s]*)?/?(?:\?[^\s]*)?")
        .expect("Geçersiz Reddit Short regex deseni")
});

// ==================== BLUESKY ====================
static BLUESKY_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\|\|\s*https?://(?i:(?:www\.)?bsky\.app)/profile/([^/\s]+)/post/([a-zA-Z0-9]+)/?(?:\?[^\s\|]*)?\s*\|\|")
        .expect("Geçersiz Spoiler Bluesky regex deseni")
});
static BLUESKY_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"https?://(?i:(?:www\.)?bsky\.app)/profile/([^/\s]+)/post/([a-zA-Z0-9]+)/?(?:\?[^\s]*)?",
    )
    .expect("Geçersiz Bluesky regex deseni")
});

// ==================== SPOTIFY ====================
// open.spotify.com/(intl-xx/)?(track|album|playlist|artist|episode|show)/id
// Spotify kimlikleri base62 (22 karakter) ama geriye dönük uyum için 16-40 aralığı kabul edilir.
static SPOTIFY_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\|\|\s*https?://(?i:open\.spotify\.com)/(?:intl-[a-zA-Z-]+/)?(track|album|playlist|artist|episode|show)/([a-zA-Z0-9]{16,40})[^\s|]*\s*\|\|")
        .expect("Geçersiz Spoiler Spotify regex deseni")
});
static SPOTIFY_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?i:open\.spotify\.com)/(?:intl-[a-zA-Z-]+/)?(track|album|playlist|artist|episode|show)/([a-zA-Z0-9]{16,40})/?(?:\?[^\s]*)?")
        .expect("Geçersiz Spotify regex deseni")
});

// spotify.link/token (Branch kısa linki) — native motorda hedefe çözülür.
static SPOTIFY_SHORT_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\|\|\s*https?://(?i:spotify\.link)/([A-Za-z0-9]+)[^\s|]*\s*\|\|")
        .expect("Geçersiz Spoiler Spotify kısa link regex deseni")
});
static SPOTIFY_SHORT_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?i:spotify\.link)/([A-Za-z0-9]+)")
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

// ==================== MINIFLUX ====================
// Herkese açık paylaşım: https://miniflux.{alan}.{uzantı}/share/{40 onaltılık}. Yalnızca `miniflux`
// alt alan adı (miniflux.site.url); kök alan adı (miniflux.site) tanınmaz.
static MINIFLUX_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\|\|\s*https://((?i:miniflux)\.[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+(?::[0-9]{1,5})?)/share/([0-9a-fA-F]{40})[^\s|]*\s*\|\|",
    )
    .expect("geçerli Miniflux spoiler regex")
});
static MINIFLUX_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https://((?i:miniflux)\.[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+(?::[0-9]{1,5})?)/share/([0-9a-fA-F]{40})[^\s<>()]*")
        .expect("geçerli Miniflux regex")
});

// ==================== LETTERBOXD ====================
// Film sayfası: /film/{slug}/ (alt sayfalar — /reviews/, /crew/ … — film adresine iner).
static LETTERBOXD_FILM_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\|\|\s*https?://(?i:(?:www\.)?letterboxd\.com)/film/([a-z0-9-]+)[^\s|]*\s*\|\|")
        .expect("geçerli Letterboxd film spoiler regex")
});
static LETTERBOXD_FILM_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?i:(?:www\.)?letterboxd\.com)/film/([a-z0-9-]+)[^\s<>()]*")
        .expect("geçerli Letterboxd film regex")
});
// Kullanıcı kaydı / incelemesi: /{kullanıcı}/film/{slug}/ ya da yeniden izleme için /{n}/.
static LETTERBOXD_REVIEW_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\|\|\s*https?://(?i:(?:www\.)?letterboxd\.com)/([A-Za-z0-9_]+)/film/([a-z0-9-]+)(?:/([0-9]{1,6}))?[^\s|]*\s*\|\|")
        .expect("geçerli Letterboxd inceleme spoiler regex")
});
static LETTERBOXD_REVIEW_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?i:(?:www\.)?letterboxd\.com)/([A-Za-z0-9_]+)/film/([a-z0-9-]+)(?:/([0-9]{1,6}))?[^\s<>()]*")
        .expect("geçerli Letterboxd inceleme regex")
});
// boxd.it kısa linki: native motorda hedefe çözülür.
static LETTERBOXD_SHORT_SPOILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\|\|\s*https?://(?i:boxd\.it)/([A-Za-z0-9]+)[^\s|]*\s*\|\|")
        .expect("geçerli boxd.it spoiler regex")
});
static LETTERBOXD_SHORT_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https?://(?i:boxd\.it)/([A-Za-z0-9]+)").expect("geçerli boxd.it regex")
});
// Kullanıcı profili: /{kullanıcı}/ (tek yol parçası). Aday adresler burada toplanır; film,
// inceleme ve alt sayfalar ayrıştırıcıda yol doğrulamasıyla elenir.
static LETTERBOXD_PROFILE_CANDIDATE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)https?://(?:www\.)?letterboxd\.com/[^\s<>()|]*")
        .expect("geçerli Letterboxd profil aday regex")
});
/// Letterboxd'un üst düzey sabit yolları: bunlar bir kullanıcı adı değildir, profil sayfası
/// sayılmamalıdır (ör. `/films/`, `/about/`, `/settings/`).
const LETTERBOXD_RESERVED_PROFILE_SEGMENTS: &[&str] = &[
    "about",
    "account",
    "activity",
    "advertise",
    "api",
    "api-beta",
    "apps",
    "contact",
    "country",
    "create-account",
    "crew",
    "decade",
    "diary",
    "dmca",
    "export",
    "film",
    "films",
    "followers",
    "following",
    "genre",
    "gift-guide",
    "guidelines",
    "help",
    "import",
    "incoming",
    "invite",
    "journal",
    "language",
    "likes",
    "list",
    "lists",
    "logout",
    "members",
    "network",
    "news",
    "notifications",
    "person",
    "podcast",
    "privacy",
    "pro",
    "register",
    "review",
    "reviews",
    "robots.txt",
    "rss",
    "s",
    "search",
    "settings",
    "showdown",
    "sign-in",
    "signin",
    "stats",
    "studio",
    "support",
    "tags",
    "terms",
    "theme",
    "welcome",
    "watchlist",
    "year-in-review",
];

/// Adres sonuna yapışan noktalama (`...`, `,`, `!`): profil bağlantısının parçası değildir.
const URL_TRAILING_PUNCTUATION: [char; 8] = ['.', ',', ';', ':', '!', '?', '"', '\''];

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
        ..post
    }
}

/// Verilen mesaj içeriğini tüm desteklenen platformlar için analiz eder.
pub fn parse_message(raw_content: &str) -> Option<ParsedPost> {
    // Hızlı ön kontrol: Link olma ihtimali olmayan mesajlarda tüm regex'leri atla
    if !raw_content.contains("http://")
        && !raw_content.contains("https://")
        && !raw_content.contains("redd.it/")
    {
        return None;
    }

    // 1. Twitter / X
    if let Some(post) = parse_twitter(raw_content) {
        return Some(sanitize_urls(post));
    }

    // 2. TikTok (okdargy/fxTikTok)
    if let Some(post) = parse_tiktok(raw_content) {
        return Some(sanitize_urls(post));
    }

    // 3. Instagram
    if let Some(post) = parse_instagram(raw_content) {
        return Some(sanitize_urls(post));
    }

    // 4. Reddit
    if let Some(post) = parse_reddit(raw_content) {
        return Some(sanitize_urls(post));
    }

    // 5. Bluesky
    if let Some(post) = parse_bluesky(raw_content) {
        return Some(sanitize_urls(post));
    }

    // 6. Spotify
    if let Some(post) = parse_spotify(raw_content) {
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
    if let Some(post) = parse_letterboxd(raw_content) {
        return Some(sanitize_urls(post));
    }
    if let Some(post) = parse_miniflux(raw_content) {
        return Some(sanitize_urls(post));
    }

    None
}

fn extract_user_comment(raw_content: &str, regex: &Regex) -> Option<String> {
    let Some(found) = regex.find(raw_content) else {
        let cleaned = raw_content.trim();
        return (!cleaned.is_empty()).then(|| cleaned.to_string());
    };
    extract_user_comment_span(raw_content, found.start(), found.end())
}

/// `start..end` aralığındaki bağlantıyı metinden çıkarıp kalanı yorum olarak döndürür.
///
/// `<url>` Discord'un önizleme bastırma sözdizimidir; bağlantı yine dönüştürülür ama açılı
/// parantezler yorumda artık kalmamalı.
fn extract_user_comment_span(
    raw_content: &str,
    mut start: usize,
    mut end: usize,
) -> Option<String> {
    if raw_content[..start].ends_with('<') {
        start -= 1;
        if raw_content[end..].starts_with('>') {
            end += 1;
        }
    }
    let cleaned = format!("{}{}", &raw_content[..start], &raw_content[end..])
        .trim()
        .to_string();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

/// Bir platformun spoiler (`||link||`) ve düz desenini sırayla dener; ilk eşleşen desenin
/// yakalamalarından `build` ile (orijinal, dönüştürülmüş) adres çifti kurulur.
///
/// Spoiler deseni önce denenir, çünkü düz desen `||` içindeki bağlantıyla da eşleşir.
/// `build` `None` dönerse bu çift için düz desen denenmez (bağlantı biçimsizdir).
fn match_pair(
    raw_content: &str,
    platform: Platform,
    spoiler: &Regex,
    plain: &Regex,
    build: impl Fn(&Captures<'_>) -> Option<String>,
) -> Option<ParsedPost> {
    for (regex, is_spoiler) in [(spoiler, true), (plain, false)] {
        if let Some(caps) = regex.captures(raw_content) {
            let original_url = build(&caps)?;
            return Some(ParsedPost {
                platform,
                is_spoiler,
                user_comment: extract_user_comment(raw_content, regex),
                original_url,
            });
        }
    }
    None
}

fn native(original_url: String) -> Option<String> {
    Some(original_url)
}

/// Genel desenler bağlantının tamamını spoiler'da ilk grupta, düz biçimde eşleşmenin kendisinde taşır.
fn whole_link<'h>(caps: &Captures<'h>) -> Option<&'h str> {
    caps.get(1).or_else(|| caps.get(0)).map(|m| m.as_str())
}

/// Twitter/X.
fn parse_twitter(raw_content: &str) -> Option<ParsedPost> {
    match_pair(
        raw_content,
        Platform::Twitter,
        &TWITTER_SPOILER_REGEX,
        &TWITTER_REGEX,
        |caps| {
            let username = caps.get(1).map_or("i", |m| m.as_str());
            let tweet_id = caps.get(2)?.as_str();
            let extra = caps
                .get(3)
                .map(|m| format!("/{}", m.as_str()))
                .unwrap_or_default();
            native(format!("https://x.com/{username}/status/{tweet_id}{extra}"))
        },
    )
}

fn parse_tiktok(raw_content: &str) -> Option<ParsedPost> {
    // 1. @user/(video|photo)/id
    match_pair(
        raw_content,
        Platform::TikTok,
        &TIKTOK_VIDEO_SPOILER_REGEX,
        &TIKTOK_VIDEO_REGEX,
        |caps| {
            let (user, kind, id) = (
                caps.get(1)?.as_str(),
                caps.get(2)?.as_str(),
                caps.get(3)?.as_str(),
            );
            Some(format!("https://www.tiktok.com/@{user}/{kind}/{id}"))
        },
    )
    // 2. /t/id
    .or_else(|| {
        match_pair(
            raw_content,
            Platform::TikTok,
            &TIKTOK_T_SPOILER_REGEX,
            &TIKTOK_T_REGEX,
            |caps| {
                let id = caps.get(1)?.as_str();
                Some(format!("https://www.tiktok.com/t/{id}"))
            },
        )
    })
    // 3. vm.tiktok.com veya vt.tiktok.com
    .or_else(|| {
        match_pair(
            raw_content,
            Platform::TikTok,
            &TIKTOK_SHORT_SPOILER_REGEX,
            &TIKTOK_SHORT_REGEX,
            |caps| {
                let sub = caps.get(1)?.as_str().to_ascii_lowercase();
                let id = caps.get(2)?.as_str();
                Some(format!("https://{sub}.tiktok.com/{id}"))
            },
        )
    })
}

/// Instagram.
fn parse_instagram(raw_content: &str) -> Option<ParsedPost> {
    match_pair(
        raw_content,
        Platform::Instagram,
        &INSTAGRAM_SPOILER_REGEX,
        &INSTAGRAM_REGEX,
        |caps| {
            let kind = caps.get(1)?.as_str();
            let id = caps.get(2)?.as_str();
            native(format!("https://www.instagram.com/{kind}/{id}"))
        },
    )
}

/// Reddit.
fn parse_reddit(raw_content: &str) -> Option<ParsedPost> {
    // 1. /r/sub/comments/id veya /(u|user)/username/comments/id
    match_pair(
        raw_content,
        Platform::Reddit,
        &REDDIT_POST_SPOILER_REGEX,
        &REDDIT_POST_REGEX,
        |caps| {
            let prefix = caps.get(1)?.as_str();
            let target = caps.get(2)?.as_str();
            let id = caps.get(3)?.as_str();
            let section = if prefix == "u" { "user" } else { prefix };
            // Yorum bağlantısında yorum kimliği korunur: kart yorumu bağlamıyla gösterir
            // (`reddit::comment_id_from_url`). Kullanıcı metni taşımamak için başlık `_` olur.
            let comment = caps
                .get(4)
                .map(|comment| format!("/_/{}", comment.as_str()))
                .unwrap_or_default();
            native(format!(
                "https://www.reddit.com/{section}/{target}/comments/{id}{comment}"
            ))
        },
    )
    // 2. /r/sub/s/share_id (Mobil Paylaşım Linki)
    .or_else(|| {
        match_pair(
            raw_content,
            Platform::Reddit,
            &REDDIT_SHARE_SPOILER_REGEX,
            &REDDIT_SHARE_REGEX,
            |caps| {
                let sub = caps.get(1)?.as_str();
                let share_id = caps.get(2)?.as_str();
                native(format!("https://www.reddit.com/r/{sub}/s/{share_id}"))
            },
        )
    })
    // 3. redd.it/id veya /comments/id (Kısa Linkler)
    .or_else(|| {
        match_pair(
            raw_content,
            Platform::Reddit,
            &REDDIT_SHORT_SPOILER_REGEX,
            &REDDIT_SHORT_REGEX,
            |caps| {
                let id = caps.get(1).or_else(|| caps.get(2))?.as_str();
                native(format!("https://redd.it/{id}"))
            },
        )
    })
}

fn parse_bluesky(raw_content: &str) -> Option<ParsedPost> {
    match_pair(
        raw_content,
        Platform::Bluesky,
        &BLUESKY_SPOILER_REGEX,
        &BLUESKY_REGEX,
        |caps| {
            let handle = caps.get(1)?.as_str();
            let rkey = caps.get(2)?.as_str();
            Some(format!("https://bsky.app/profile/{handle}/post/{rkey}"))
        },
    )
}

fn parse_spotify(raw_content: &str) -> Option<ParsedPost> {
    // 1. Track / album / playlist / artist
    match_pair(
        raw_content,
        Platform::Spotify,
        &SPOTIFY_SPOILER_REGEX,
        &SPOTIFY_REGEX,
        |caps| {
            let kind = caps.get(1)?.as_str();
            let id = caps.get(2)?.as_str();
            Some(format!("https://open.spotify.com/{kind}/{id}"))
        },
    )
    // 2. spotify.link kısa link
    .or_else(|| {
        match_pair(
            raw_content,
            Platform::Spotify,
            &SPOTIFY_SHORT_SPOILER_REGEX,
            &SPOTIFY_SHORT_REGEX,
            |caps| {
                let token = caps.get(1)?.as_str();
                Some(format!("https://spotify.link/{token}"))
            },
        )
    })
}

fn canonical_clip_url(raw: &str) -> Option<String> {
    let raw = raw.trim_end_matches(['.', ',', '!', ';']);
    let path = raw.split(['?', '#']).next()?.trim_end_matches('/');
    let slug = path.rsplit('/').next()?;
    (!slug.is_empty()).then(|| format!("https://clips.twitch.tv/{slug}"))
}

fn parse_twitch(raw_content: &str) -> Option<ParsedPost> {
    match_pair(
        raw_content,
        Platform::Twitch,
        &TWITCH_CLIP_SPOILER_REGEX,
        &TWITCH_CLIP_REGEX,
        |caps| native(canonical_clip_url(whole_link(caps)?)?),
    )
}

fn canonical_path_url(raw: &str, host: &str) -> Option<String> {
    let raw = raw.trim_end_matches(['.', ',', '!', ';']);
    let parsed = url::Url::parse(raw).ok()?;
    let path = parsed.path().trim_end_matches('/');
    (!path.is_empty()).then(|| format!("https://{host}{path}"))
}

fn parse_pixiv(raw_content: &str) -> Option<ParsedPost> {
    match_pair(
        raw_content,
        Platform::Pixiv,
        &PIXIV_ARTWORK_SPOILER_REGEX,
        &PIXIV_ARTWORK_REGEX,
        |caps| native(canonical_path_url(whole_link(caps)?, "www.pixiv.net")?),
    )
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
    match_pair(
        raw_content,
        Platform::Tumblr,
        &TUMBLR_POST_SPOILER_REGEX,
        &TUMBLR_POST_REGEX,
        |caps| native(tumblr_canonical(whole_link(caps)?)?),
    )
}

fn parse_facebook(raw_content: &str) -> Option<ParsedPost> {
    match_pair(
        raw_content,
        Platform::Facebook,
        &FACEBOOK_POST_SPOILER_REGEX,
        &FACEBOOK_POST_REGEX,
        |caps| {
            let raw = whole_link(caps)?.trim_end_matches(['.', ',', '!', ';']);
            let parsed = url::Url::parse(raw).ok()?;
            let mut original_url = format!("https://www.facebook.com{}", parsed.path());
            if let Some(query) = parsed.query()
                && parsed.path() == "/watch/"
            {
                original_url.push('?');
                original_url.push_str(query);
            }
            native(original_url)
        },
    )
}

fn parse_youtube(raw_content: &str) -> Option<ParsedPost> {
    match_pair(
        raw_content,
        Platform::YouTube,
        &YOUTUBE_COMMUNITY_SPOILER_REGEX,
        &YOUTUBE_COMMUNITY_REGEX,
        |caps| native(canonical_path_url(whole_link(caps)?, "www.youtube.com")?),
    )
}

/// Miniflux paylaşım bağlantısı. Kanonik biçim `https://{sunucu}/share/{kod}` (küçük harf kod).
/// shortcut: alt dizinde (`/miniflux/share/..`) ve http'de çalışan sunucular tanınmaz.
fn parse_miniflux(raw_content: &str) -> Option<ParsedPost> {
    match_pair(
        raw_content,
        Platform::Miniflux,
        &MINIFLUX_SPOILER_REGEX,
        &MINIFLUX_REGEX,
        |caps| {
            let host = caps.get(1)?.as_str().to_ascii_lowercase();
            let code = caps.get(2)?.as_str().to_ascii_lowercase();
            native(format!("https://{host}/share/{code}"))
        },
    )
}

/// Letterboxd: kullanıcı profili, kullanıcı incelemesi ve film sayfası
/// desteklenir; kısa linkler (`boxd.it`) hedefe native motorda çözülür. Adresler kanonik
/// biçime indirilir (sorgu, parça ve alt sayfa atılır).
fn parse_letterboxd(raw_content: &str) -> Option<ParsedPost> {
    match_pair(
        raw_content,
        Platform::Letterboxd,
        &LETTERBOXD_REVIEW_SPOILER_REGEX,
        &LETTERBOXD_REVIEW_REGEX,
        |caps| {
            let user = caps.get(1)?.as_str();
            let slug = caps.get(2)?.as_str();
            let entry = caps
                .get(3)
                .map(|n| format!("{}/", n.as_str()))
                .unwrap_or_default();
            native(format!("https://letterboxd.com/{user}/film/{slug}/{entry}"))
        },
    )
    .or_else(|| {
        match_pair(
            raw_content,
            Platform::Letterboxd,
            &LETTERBOXD_FILM_SPOILER_REGEX,
            &LETTERBOXD_FILM_REGEX,
            |caps| {
                native(format!(
                    "https://letterboxd.com/film/{}/",
                    caps.get(1)?.as_str()
                ))
            },
        )
    })
    .or_else(|| {
        match_pair(
            raw_content,
            Platform::Letterboxd,
            &LETTERBOXD_SHORT_SPOILER_REGEX,
            &LETTERBOXD_SHORT_REGEX,
            |caps| native(format!("https://boxd.it/{}", caps.get(1)?.as_str())),
        )
    })
    .or_else(|| parse_letterboxd_profile(raw_content))
}

/// Bağlantı tek yol parçalı bir Letterboxd kullanıcı profili mi? (`/{kullanıcı}/`)
///
/// Film/inceleme/liste alt sayfaları, sabit üst düzey yollar ve kullanıcı adı olmayan
/// adresler `None` döner. Adres kırpılmış gelmelidir (sondaki noktalamayı çağıran atar).
pub(crate) fn letterboxd_profile_user(raw: &str) -> Option<String> {
    let url = url::Url::parse(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !matches!(
            url.host_str(),
            Some("letterboxd.com" | "www.letterboxd.com")
        )
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    let mut segments = url.path_segments()?.filter(|segment| !segment.is_empty());
    let user = segments.next()?;
    if segments.next().is_some()
        || user.len() > 30
        || !user
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return None;
    }
    let lower = user.to_ascii_lowercase();
    if LETTERBOXD_RESERVED_PROFILE_SEGMENTS.contains(&lower.as_str()) {
        return None;
    }
    Some(user.to_string())
}

/// Mesajdaki ilk geçerli Letterboxd profil bağlantısını bulur.
///
/// Aday adresler tek tek doğrulanır: ilk eşleşme bir alt sayfa olsa bile (`/u/films/`)
/// aramak vazgeçmez, sonraki adaya geçer. Spoiler biçimi (`||url||`) bağlantının hemen
/// çevresindeki ayırıcılardan anlaşılır.
fn parse_letterboxd_profile(raw_content: &str) -> Option<ParsedPost> {
    for candidate in LETTERBOXD_PROFILE_CANDIDATE_REGEX.find_iter(raw_content) {
        let token = candidate.as_str();
        let trimmed = token.trim_end_matches(URL_TRAILING_PUNCTUATION);
        let Some(user) = letterboxd_profile_user(trimmed) else {
            continue;
        };
        let (mut start, mut end) = (candidate.start(), candidate.start() + trimmed.len());
        let before = raw_content[..start].trim_end();
        let after = raw_content[end..].trim_start();
        let is_spoiler = before.ends_with("||") && after.starts_with("||");
        if is_spoiler {
            // `||` işaretleri de bağlantıyla birlikte yorumdan çıkarılır.
            start = before.len() - 2;
            end = raw_content.len() - after.len() + 2;
        }
        let url = format!("https://letterboxd.com/{user}/");
        return Some(ParsedPost {
            platform: Platform::Letterboxd,
            is_spoiler,
            user_comment: extract_user_comment_span(raw_content, start, end),
            original_url: url,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn miniflux_share_links_are_canonicalized() {
        let code = "5bf90f5776eaa9139d02f9a9a5d964d20a472f59";
        let parsed =
            parse_message(&format!("oku https://Miniflux.Turta.us/share/{code}?x=1")).unwrap();
        assert_eq!(parsed.platform, Platform::Miniflux);
        assert_eq!(
            parsed.original_url,
            format!("https://miniflux.turta.us/share/{code}")
        );
        assert!(!parsed.is_spoiler);
        assert_eq!(parsed.user_comment.as_deref(), Some("oku"));
        assert!(
            parse_message(&format!("||https://miniflux.example.com/share/{code}||"))
                .unwrap()
                .is_spoiler
        );
        assert!(parse_miniflux("https://miniflux.example.com/share/abc").is_none());
        assert!(parse_miniflux(&format!("https://miniflux.site/share/{code}")).is_none());
        assert!(parse_miniflux(&format!("https://wiki.example.com/share/{code}")).is_none());
        assert!(parse_miniflux(&format!("http://miniflux.example.com/share/{code}")).is_none());
    }

    #[test]
    fn letterboxd_links_are_canonicalized() {
        let parse =
            |text: &str| parse_letterboxd(text).map(|post| (post.original_url, post.is_spoiler));
        assert_eq!(
            parse("bak https://letterboxd.com/film/parasite-2019/reviews/by/activity/?x=1"),
            Some(("https://letterboxd.com/film/parasite-2019/".into(), false))
        );
        assert_eq!(
            parse("https://www.letterboxd.com/hollieamanda/film/parasite-2019/"),
            Some((
                "https://letterboxd.com/hollieamanda/film/parasite-2019/".into(),
                false
            ))
        );
        assert_eq!(
            parse("||https://letterboxd.com/biri_1/film/heat-1995/2/||"),
            Some((
                "https://letterboxd.com/biri_1/film/heat-1995/2/".into(),
                true
            ))
        );
        assert_eq!(
            parse("https://boxd.it/2bg8"),
            Some(("https://boxd.it/2bg8".into(), false))
        );
        // Profil tek yol parçasıysa dönüştürülür; film, liste ve arama alt sayfaları ve
        // Letterboxd'un sabit üst düzey yolları profil sayılmaz.
        assert_eq!(
            parse("https://letterboxd.com/hollieamanda/"),
            Some(("https://letterboxd.com/hollieamanda/".into(), false))
        );
        assert_eq!(
            parse("profil: https://letterboxd.com/hollieamanda"),
            Some(("https://letterboxd.com/hollieamanda/".into(), false))
        );
        assert_eq!(
            parse("||https://letterboxd.com/hollieamanda/?ref=spoiler||"),
            Some(("https://letterboxd.com/hollieamanda/".into(), true))
        );
        assert_eq!(parse("https://letterboxd.com/hollieamanda/films/"), None);
        assert_eq!(
            parse("https://letterboxd.com/hollieamanda/film/parasite-2019/"),
            Some((
                "https://letterboxd.com/hollieamanda/film/parasite-2019/".into(),
                false
            ))
        );
        assert_eq!(parse("https://letterboxd.com/films/"), None);
        assert_eq!(parse("https://letterboxd.com/about/"), None);
        assert_eq!(parse("https://letterboxd.com/biri/list/en-iyiler/"), None);
        // Alt sayfa ilk aday olsa da aynı mesajdaki geçerli profil bulunur.
        assert_eq!(
            parse("https://letterboxd.com/biri/films/ ve https://letterboxd.com/hollieamanda/"),
            Some(("https://letterboxd.com/hollieamanda/".into(), false))
        );
        // Spoiler işaretleri yorumda kalmaz.
        let spoiler = parse_letterboxd("||https://letterboxd.com/hollieamanda/||").unwrap();
        assert!(spoiler.is_spoiler);
        assert_eq!(spoiler.user_comment, None);
        let spoiler =
            parse_letterboxd("bak || https://letterboxd.com/hollieamanda/ || işte").unwrap();
        assert_eq!(spoiler.user_comment.as_deref(), Some("bak  işte"));
        // Profil bağlantısı yorumdan temizlenir, kalan metin yorum olur.
        let with_comment =
            parse_letterboxd("bak https://letterboxd.com/hollieamanda/ ne güzel").unwrap();
        assert_eq!(with_comment.user_comment.as_deref(), Some("bak  ne güzel"));
        let parsed = parse_message("https://letterboxd.com/film/heat-1995/").unwrap();
        assert_eq!(parsed.platform, Platform::Letterboxd);
    }

    #[test]
    fn test_twitter_parsing() {
        let content = "Harika video https://x.com/elonmusk/status/123456789?s=20 mutlaka izleyin";
        let parsed = parse_message(content).unwrap();

        assert_eq!(parsed.platform, Platform::Twitter);
        assert_eq!(
            parsed.original_url,
            "https://x.com/elonmusk/status/123456789"
        );
        assert!(!parsed.is_spoiler);
        assert_eq!(
            parsed.user_comment.as_deref(),
            Some("Harika video  mutlaka izleyin")
        );
    }

    #[test]
    fn test_twitter_spoiler() {
        let content = "Gizli: ||https://twitter.com/jack/status/20||";
        let parsed = parse_message(content).unwrap();

        assert_eq!(parsed.platform, Platform::Twitter);
        assert!(parsed.is_spoiler);
        assert_eq!(parsed.user_comment.as_deref(), Some("Gizli:"));
    }

    #[test]
    fn test_tiktok_video() {
        let content = "https://www.tiktok.com/@scout2015/video/6718335390845095173";
        let parsed = parse_message(content).unwrap();

        assert_eq!(parsed.platform, Platform::TikTok);
        assert_eq!(
            parsed.original_url,
            "https://www.tiktok.com/@scout2015/video/6718335390845095173"
        );
        assert!(!parsed.is_spoiler);
        assert_eq!(parsed.user_comment, None);
    }

    #[test]
    fn test_tiktok_short() {
        let content = "Buna bak https://vm.tiktok.com/ZM8abc123/ çok komik";
        let parsed = parse_message(content).unwrap();

        assert_eq!(parsed.platform, Platform::TikTok);
        assert_eq!(parsed.original_url, "https://vm.tiktok.com/ZM8abc123");
        assert_eq!(parsed.user_comment.as_deref(), Some("Buna bak  çok komik"));
    }

    #[test]
    fn test_instagram_reel() {
        let content = "https://www.instagram.com/reel/C3abcXYZ123/?igsh=abcdef";
        let parsed = parse_message(content).unwrap();

        assert_eq!(parsed.platform, Platform::Instagram);
        assert_eq!(
            parsed.original_url,
            "https://www.instagram.com/reel/C3abcXYZ123"
        );
    }

    #[test]
    fn test_reddit_post() {
        let content = "https://reddit.com/r/rust/comments/123456/why_rust_is_great/";
        let parsed = parse_message(content).unwrap();

        assert_eq!(parsed.platform, Platform::Reddit);
        assert_eq!(
            parsed.original_url,
            "https://www.reddit.com/r/rust/comments/123456"
        );
        assert!(!parsed.is_spoiler);
    }

    /// Yorum kimliği atılırsa kart yorumu değil yalnızca gönderiyi gösterirdi.
    #[test]
    fn reddit_comment_links_keep_the_comment_id() {
        for (content, spoiler) in [
            (
                "https://www.reddit.com/r/rust/comments/abc123/baslik/def456/?context=3",
                false,
            ),
            (
                "||https://old.reddit.com/r/rust/comments/abc123/baslik/def456/||",
                true,
            ),
        ] {
            let parsed = parse_message(content).unwrap();
            assert_eq!(parsed.is_spoiler, spoiler);
            assert_eq!(
                parsed.original_url,
                "https://www.reddit.com/r/rust/comments/abc123/_/def456"
            );
        }
        // Başlıkla biten gönderi bağlantısı yorum sanılmaz.
        let parsed =
            parse_message("https://www.reddit.com/r/rust/comments/abc123/baslik/").unwrap();
        assert_eq!(
            parsed.original_url,
            "https://www.reddit.com/r/rust/comments/abc123"
        );
    }

    #[test]
    fn test_reddit_mobile_share() {
        let content = "Buna bak https://www.reddit.com/r/pcmasterrace/s/AbCdEf1234 harika";
        let parsed = parse_message(content).unwrap();

        assert_eq!(parsed.platform, Platform::Reddit);
        assert_eq!(
            parsed.original_url,
            "https://www.reddit.com/r/pcmasterrace/s/AbCdEf1234"
        );
        assert!(!parsed.is_spoiler);
        assert_eq!(parsed.user_comment.as_deref(), Some("Buna bak  harika"));
    }

    #[test]
    fn test_reddit_shortlink() {
        let content = "https://redd.it/123456";
        let parsed = parse_message(content).unwrap();

        assert_eq!(parsed.platform, Platform::Reddit);
        assert_eq!(parsed.original_url, "https://redd.it/123456");
        assert!(!parsed.is_spoiler);
    }

    #[test]
    fn test_reddit_user_post() {
        let content = "https://www.reddit.com/u/someone/comments/abcdef12/";
        let parsed = parse_message(content).unwrap();

        assert_eq!(parsed.platform, Platform::Reddit);
        assert_eq!(
            parsed.original_url,
            "https://www.reddit.com/user/someone/comments/abcdef12"
        );
    }

    #[test]
    fn test_reddit_spoiler() {
        let content = "||https://old.reddit.com/r/news/comments/789xyz/breaking/||";
        let parsed = parse_message(content).unwrap();

        assert_eq!(parsed.platform, Platform::Reddit);
        assert_eq!(
            parsed.original_url,
            "https://www.reddit.com/r/news/comments/789xyz"
        );
        assert!(parsed.is_spoiler);
    }

    #[test]
    fn crafted_url_cannot_break_out_of_markdown_link() {
        // Kapalı parantezle [:](...) bitişini kırıp ikinci bir link hedefi enjekte etme girişimi
        let content = "https://www.reddit.com/r/x)y[sahte]/comments/abc";
        let parsed = parse_message(content).unwrap();

        let url = &parsed.original_url;
        assert!(!url.contains(')'), "Kaçırılmamış parantez: {url}");
        assert!(!url.contains('('), "Kaçırılmamış parantez: {url}");
        assert!(!url.contains('['), "Kaçırılmamış köşeli parantez: {url}");
        assert!(!url.contains(']'), "Kaçırılmamış köşeli parantez: {url}");
    }

    #[test]
    fn already_encoded_urls_are_left_intact() {
        let content = "https://x.com/a%20b/status/123";
        let parsed = parse_message(content).unwrap();
        assert_eq!(parsed.original_url, "https://x.com/a%20b/status/123");
    }

    #[test]
    fn test_bluesky_post() {
        let content = "||https://bsky.app/profile/alice.bsky.social/post/3k6abc123||";
        let parsed = parse_message(content).unwrap();

        assert_eq!(parsed.platform, Platform::Bluesky);
        assert_eq!(
            parsed.original_url,
            "https://bsky.app/profile/alice.bsky.social/post/3k6abc123"
        );
        assert!(parsed.is_spoiler);
    }

    #[test]
    fn test_spotify_track() {
        let content =
            "Şu şarkıya bak https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT?si=abc harika";
        let parsed = parse_message(content).unwrap();

        assert_eq!(parsed.platform, Platform::Spotify);
        assert_eq!(
            parsed.original_url,
            "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT"
        );
        assert!(!parsed.is_spoiler);
        assert_eq!(
            parsed.user_comment.as_deref(),
            Some("Şu şarkıya bak  harika")
        );
    }

    #[test]
    fn test_spotify_intl_and_spoiler() {
        let content = "||https://open.spotify.com/intl-tr/album/1DFixLWuPkv3KT3TnV35m3||";
        let parsed = parse_message(content).unwrap();

        assert_eq!(parsed.platform, Platform::Spotify);
        assert_eq!(
            parsed.original_url,
            "https://open.spotify.com/album/1DFixLWuPkv3KT3TnV35m3"
        );
        assert!(parsed.is_spoiler);
    }

    #[test]
    fn test_spotify_short_link() {
        let content = "https://spotify.link/AbCdEf1234";
        let parsed = parse_message(content).unwrap();

        assert_eq!(parsed.platform, Platform::Spotify);
        assert_eq!(parsed.original_url, "https://spotify.link/AbCdEf1234");
    }

    #[test]
    fn new_platform_urls_have_canonical_forms_and_comments() {
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
            let parsed = parse_message(input).expect("yeni platform linki bulunmalı");
            assert_eq!(parsed.platform, platform);
            assert_eq!(parsed.original_url, url);
        }
        assert!(
            parse_message("||https://www.pixiv.net/en/artworks/12345||")
                .expect("spoiler")
                .is_spoiler
        );
    }

    #[test]
    fn non_canonical_new_platform_pages_are_ignored() {
        for input in [
            "https://www.twitch.tv/channel",
            "https://www.pixiv.net/users/123",
            "https://artist.tumblr.com/",
            "https://www.facebook.com/example",
            "https://www.youtube.com/channel/abc",
        ] {
            assert!(parse_message(input).is_none(), "{input}");
        }
    }

    #[test]
    fn mobile_web_and_uppercase_host_forms_are_canonicalized() {
        let cases = [
            (
                "https://mobile.twitter.com/jack/status/20",
                Platform::Twitter,
                "https://x.com/jack/status/20",
            ),
            (
                "https://x.com/i/web/status/1234567890",
                Platform::Twitter,
                "https://x.com/i/status/1234567890",
            ),
            (
                "https://X.COM/Jack/status/20?s=1",
                Platform::Twitter,
                "https://x.com/Jack/status/20",
            ),
            (
                "https://www.instagram.com/kullanici.adi/p/C3abcXYZ123/?igsh=x",
                Platform::Instagram,
                "https://www.instagram.com/p/C3abcXYZ123",
            ),
            (
                "https://www.instagram.com/kullanici_1/reel/C3abcXYZ123/",
                Platform::Instagram,
                "https://www.instagram.com/reel/C3abcXYZ123",
            ),
            (
                "https://WWW.Instagram.com/p/C3abc",
                Platform::Instagram,
                "https://www.instagram.com/p/C3abc",
            ),
            (
                "https://m.reddit.com/r/rust/comments/abc123/baslik/",
                Platform::Reddit,
                "https://www.reddit.com/r/rust/comments/abc123",
            ),
            (
                "https://sh.reddit.com/r/rust/comments/abc123",
                Platform::Reddit,
                "https://www.reddit.com/r/rust/comments/abc123",
            ),
            (
                "https://Reddit.com/r/rust/s/AbC123",
                Platform::Reddit,
                "https://www.reddit.com/r/rust/s/AbC123",
            ),
        ];
        for (input, platform, url) in cases {
            let parsed = parse_message(input).unwrap_or_else(|| panic!("{input}"));
            assert_eq!(parsed.platform, platform, "{input}");
            assert_eq!(parsed.original_url, url, "{input}");
        }

        let tiktok = parse_message("https://VM.TikTok.com/ZM8abc123/").unwrap();
        assert_eq!(tiktok.original_url, "https://vm.tiktok.com/ZM8abc123");
    }

    #[test]
    fn instagram_profile_pages_are_still_ignored() {
        for input in [
            "https://www.instagram.com/kullanici/",
            "https://www.instagram.com/kullanici/reels/",
        ] {
            assert!(parse_message(input).is_none(), "{input}");
        }
    }

    #[test]
    fn embed_suppressed_links_leave_no_angle_bracket_residue() {
        for (input, comment) in [
            ("<https://x.com/jack/status/20>", None),
            (
                "bak <https://x.com/jack/status/20?s=20> işte",
                Some("bak  işte"),
            ),
            ("<https://www.instagram.com/p/C3abc/>", None),
            ("<https://www.reddit.com/r/rust/comments/abc123/>", None),
            ("not <https://www.pixiv.net/artworks/12345>", Some("not")),
            ("a < b https://x.com/jack/status/20", Some("a < b")),
        ] {
            let parsed = parse_message(input).unwrap_or_else(|| panic!("{input}"));
            assert_eq!(parsed.user_comment.as_deref(), comment, "{input}");
        }
    }
}
