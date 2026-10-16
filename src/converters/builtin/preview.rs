//! Conservative public-preview boundary shared only by the five OG-only adapters.
//! The page transport validates/pins DNS at every redirect. Media is not downloaded here;
//! its syntax and DNS are checked before publication, not a promise of future playability.

use std::net::IpAddr;

use url::{Host, Url};

use super::{Author, BuiltinPost, LinkPreview, Media, MediaKind, opengraph};
use crate::converters::parser::{ParsedPost, Platform};

/// Wrap DNS, redirects, streaming and optional media DNS checks in one total budget.
pub(super) async fn fetch(parsed: &ParsedPost, platform: Platform) -> Result<BuiltinPost, String> {
    if parsed.platform != platform {
        return Err("preview platform mismatch".into());
    }
    validate_page(&parsed.original_url, platform)?;
    tokio::time::timeout(super::REQUEST_TIMEOUT, async {
        let (final_url, html) = super::safe_http::preview(&parsed.original_url).await?;
        let mut post = from_html(&parsed.original_url, &final_url, platform, &html)?;
        // A syntactically public name can still resolve to a private address. Check each
        // distinct URL once, and drop unavailable media without losing useful text.
        let mut checked = std::collections::HashMap::new();
        for item in &post.media {
            for raw in std::iter::once(&item.url).chain(item.poster.iter()) {
                if !checked.contains_key(raw) {
                    checked.insert(raw.clone(), resolves_public(raw).await);
                }
            }
        }
        post.media
            .retain(|item| checked.get(&item.url) == Some(&true));
        for item in &mut post.media {
            if item
                .poster
                .as_ref()
                .is_some_and(|url| checked.get(url) != Some(&true))
            {
                item.poster = None;
            }
        }
        Ok(post)
    })
    .await
    .map_err(|_| "public preview timed out".to_string())?
}

fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_multicast()
                || ip.is_unspecified()
                || ip.is_broadcast()
                || ip.is_documentation()
                || a == 0
                || a >= 240
                || (a == 100 && (64..=127).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 192 && b == 88 && c == 99)
                || (a == 198 && (b == 18 || b == 19)))
        }
        IpAddr::V6(ip) => {
            let s = ip.segments();
            s[0] & 0xe000 == 0x2000
                && s[0] != 0x2002
                && !(s[0] == 0x2001 && (s[1] < 0x0200 || s[1] == 0x0db8))
                && !(s[0] == 0x3fff && s[1] < 0x1000)
        }
    }
}

fn public_url(raw: &str) -> Option<Url> {
    if raw.len() > 8192 || raw.chars().any(|ch| ch.is_control() || ch == '\\') {
        return None;
    }
    let url = Url::parse(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(url.port_or_known_default(), Some(80 | 443))
    {
        return None;
    }
    let public = match url.host()? {
        Host::Ipv4(ip) => public_ip(ip.into()),
        Host::Ipv6(ip) => public_ip(ip.into()),
        Host::Domain(host) => {
            let host = host.trim_end_matches('.');
            host.contains('.')
                && ![
                    "localhost",
                    "local",
                    "internal",
                    "lan",
                    "home",
                    "test",
                    "invalid",
                    "onion",
                ]
                .iter()
                .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
        }
    };
    public.then_some(url)
}

async fn resolves_public(raw: &str) -> bool {
    let Some(url) = public_url(raw) else {
        return false;
    };
    let host = url.host_str().unwrap_or_default().trim_matches(['[', ']']);
    let lookup = tokio::time::timeout(
        std::time::Duration::from_millis(300),
        tokio::net::lookup_host((host, url.port_or_known_default().unwrap_or(443))),
    )
    .await;
    let Ok(Ok(addresses)) = lookup else {
        return false;
    };
    let addresses: Vec<_> = addresses.take(17).collect();
    !addresses.is_empty() && addresses.len() <= 16 && addresses.iter().all(|a| public_ip(a.ip()))
}

fn validate_page(raw: &str, platform: Platform) -> Result<Url, String> {
    let url = public_url(raw).ok_or("invalid public preview URL")?;
    let host = url.host_str().ok_or("missing preview host")?;
    let allowed = match platform {
        Platform::Twitch => matches!(
            host,
            "twitch.tv" | "www.twitch.tv" | "clips.twitch.tv" | "m.twitch.tv"
        ),
        Platform::Pixiv => matches!(host, "pixiv.net" | "www.pixiv.net"),
        Platform::Tumblr => host == "tumblr.com" || host.ends_with(".tumblr.com"),
        Platform::Facebook => {
            matches!(host, "facebook.com" | "www.facebook.com" | "m.facebook.com")
        }
        Platform::YouTube => matches!(host, "youtube.com" | "www.youtube.com" | "m.youtube.com"),
        _ => false,
    };
    if !allowed {
        return Err("preview left the requested platform".into());
    }
    let path = urlencoding::decode(url.path()).map_err(|_| "invalid preview path")?;
    let parts: Vec<_> = path.trim_matches('/').split('/').collect();
    let token = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    };
    let numeric = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let is_post = match platform {
        Platform::Twitch => {
            (host == "clips.twitch.tv"
                && parts.len() == 1
                && token(parts[0])
                && !["login", "consent", "signup", "embed"].contains(&parts[0]))
                || (parts.len() == 3 && token(parts[0]) && parts[1] == "clip" && token(parts[2]))
        }
        Platform::Pixiv => {
            (parts.len() == 2 && parts[0] == "artworks" && numeric(parts[1]))
                || (parts.len() == 3
                    && parts[0].len() == 2
                    && parts[1] == "artworks"
                    && numeric(parts[2]))
        }
        Platform::Tumblr => {
            (host != "www.tumblr.com"
                && host.ends_with(".tumblr.com")
                && parts.len() >= 2
                && parts[0] == "post"
                && numeric(parts[1]))
                || (matches!(host, "tumblr.com" | "www.tumblr.com")
                    && parts.len() >= 2
                    && token(parts[0])
                    && (numeric(parts[1])
                        || (parts.len() >= 3 && parts[1] == "post" && numeric(parts[2]))))
        }
        Platform::Facebook => {
            (parts.len() >= 3
                && token(parts[0])
                && matches!(parts[1], "posts" | "videos")
                && token(parts[2]))
                || (parts.len() == 2 && matches!(parts[0], "reel" | "videos") && numeric(parts[1]))
                || (matches!(path.as_ref(), "/watch/" | "/watch")
                    && url.query_pairs().any(|(k, v)| k == "v" && numeric(&v)))
                || (matches!(path.as_ref(), "/permalink.php" | "/story.php")
                    && url
                        .query_pairs()
                        .any(|(k, v)| k == "story_fbid" && token(&v)))
        }
        Platform::YouTube => parts.len() == 2 && parts[0] == "post" && token(parts[1]),
        _ => false,
    };
    if !is_post {
        return Err(
            "preview destination is not a public post (login/consent or unsupported path)".into(),
        );
    }
    Ok(url)
}

pub(super) fn link_url(url: &Url) -> String {
    // The renderer places URLs in Markdown without further escaping.
    let mut result = String::new();
    for ch in url.as_str().chars() {
        if "()[]<>\"{}|\\^`".contains(ch) {
            result.push_str(&format!("%{:02X}", ch as u32));
        } else {
            result.push(ch);
        }
    }
    result
}

fn media_url(raw: &str, video: bool) -> Option<String> {
    let url = public_url(raw)?;
    let path = url.path().to_ascii_lowercase();
    // HLS playlists and HTML players are not direct playable file previews.
    if video && ![".mp4", ".webm"].iter().any(|ext| path.ends_with(ext)) {
        return None;
    }
    Some(link_url(&url))
}

pub(super) fn plain_text(raw: &str) -> String {
    raw.chars()
        .filter(|ch| !ch.is_control() || *ch == '\n')
        .take(2000)
        .collect::<String>()
        .replace('@', "@\u{200b}")
        .replace('<', "‹")
        .replace('>', "›")
        .trim()
        .to_string()
}

/// Önizleme metni için kaçış: paylaşılan `escape_markdown` sözleşmesi + **yalnızca satır
/// başındaki** `#`.
///
/// `_` bilerek kaçırılmaz: Discord (CommonMark) kelime içi alt çizgiyi vurgu başlatıcı saymadığı
/// için kaçış gereksizdir ve kartta kaynakta olmayan bir `\` gösterir
/// (`world_record_egg` → `world\_record\_egg`, bkz. `render::escape_markdown` ve 2026-10-09
/// kararı). Aynı nedenle satır ortasındaki `#` de kaçırılmaz; ancak satır başındaki (en fazla
/// üç boşluk girintili) `#` kartta başlık oluşturabildiği için kaçırılır.
pub(super) fn escape_preview_markdown(raw: &str) -> String {
    let escaped = super::render::escape_markdown(&plain_text(raw));
    let mut out = String::with_capacity(escaped.len());
    for (index, line) in escaped.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let indent = line.len() - line.trim_start_matches(' ').len();
        let (leading, rest) = line.split_at(indent);
        out.push_str(leading);
        if indent <= 3 && rest.starts_with('#') {
            out.push('\\');
        }
        out.push_str(rest);
    }
    out
}

fn generic(raw: &str, platform: Platform) -> bool {
    let lower = raw.trim().to_lowercase();
    lower.is_empty()
        || lower == platform.display_name().to_lowercase()
        || [
            "log in",
            "login",
            "sign in",
            "sign up",
            "consent",
            "content unavailable",
            "content isn't available",
            "not found",
            "access denied",
            "just a moment",
            "before you continue",
            "verify you are human",
            "giriş yap",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
        || lower.starts_with("welcome to ")
        || lower.starts_with("enjoy the videos and music you love")
        || lower.starts_with("twitch is the world's leading")
}

pub(super) fn from_html(
    requested: &str,
    final_url: &str,
    platform: Platform,
    html: &str,
) -> Result<BuiltinPost, String> {
    validate_page(requested, platform)?;
    let final_url = validate_page(final_url, platform)?;
    if html.len() > super::MAX_BODY_BYTES {
        return Err("preview body too large".into());
    }
    // Parse meta tags only: <title> on an error/login page is never post metadata. Keep
    // gallery order and accept either attribute ordering using the existing OG parser.
    let mut title = None;
    let mut description = None;
    let mut images = Vec::new();
    let mut video = None;
    for tag in opengraph::META_TAG_REGEX.find_iter(html).take(512) {
        let graph = opengraph::parse_open_graph(tag.as_str());
        if let Some(value) = graph.title {
            title = Some(value);
        }
        if let Some(value) = graph.description {
            description = Some(value);
        }
        if let Some(image) = graph.image.and_then(|raw| media_url(&raw, false))
            && images.len() < 10
            && !images.contains(&image)
        {
            images.push(image);
        }
        if let Some(url) = graph.video.and_then(|raw| media_url(&raw, true)) {
            video = Some(url);
        }
    }
    // Generic site metadata plus a logo is not enough to replace a user's original message.
    if title.as_deref().is_some_and(|text| generic(text, platform))
        || (title.is_none()
            && description
                .as_deref()
                .is_none_or(|text| generic(text, platform)))
    {
        return Err("no meaningful public post metadata".into());
    }
    let title = title.map(|value| plain_text(&value));
    let description = description
        .filter(|value| !generic(value, platform))
        .map(|value| escape_preview_markdown(&value));
    let canonical = link_url(&final_url);
    let mut post = BuiltinPost::new(
        platform,
        canonical.clone(),
        Author::new(platform.display_name(), ""),
    );
    post.title = title.clone();
    post.extra_label = Some(
        match platform {
            Platform::Twitch => "Clip",
            Platform::Pixiv => "Artwork",
            Platform::YouTube => "Community",
            Platform::Facebook if final_url.path().starts_with("/reel/") => "Reel",
            _ => "Post",
        }
        .into(),
    );
    // Metadata cannot establish author identity, verification or sensitivity. Keep source
    // flags at their unknown/default values; never copy the sharing user's spoiler here.
    let poster = images.first().cloned();
    if platform != Platform::Pixiv
        && let Some(url) = video
    {
        post.media.push(Media {
            kind: MediaKind::Video,
            url,
            poster: poster.clone(),
            width: None,
            height: None,
            duration: None,
        });
    }
    let has_video = !post.media.is_empty();
    let count = if platform == Platform::Tumblr {
        10 - post.media.len()
    } else {
        1
    };
    let skip_poster = has_video && poster.as_ref() == images.first();
    for url in images
        .into_iter()
        .skip(usize::from(skip_poster))
        .take(count)
    {
        post.media.push(Media {
            kind: MediaKind::Image,
            url,
            poster: None,
            width: None,
            height: None,
            duration: None,
        });
    }
    // V2 renders non-Reddit titles via link_preview. Render the description exactly once;
    // post.text stays empty and media already contains the poster/gallery.
    post.link_preview = Some(LinkPreview {
        url: canonical,
        title,
        description,
        image: None,
        site_name: Some(platform.display_name().into()),
    });
    Ok(post)
}

/// Parse a trusted third-party renderer's HTML while keeping the original Tumblr URL as the
/// published source. The caller must fetch the renderer through `safe_http` and pin its final
/// host before calling this helper.
pub(super) fn from_external_html(
    source_url: &str,
    platform: Platform,
    html: &str,
) -> Result<BuiltinPost, String> {
    from_html(source_url, source_url, platform, html)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RICH: &str = r#"
        <meta property="og:title" content="A real public post">
        <meta property="og:description" content="Caption with @everyone and [markdown]">
        <meta property="og:image" content="https://cdn.example.com/poster.jpg">
        <meta property="og:video" content="https://cdn.example.com/clip.mp4?name=.m3u8">
    "#;

    #[test]
    fn login_only_and_generic_pages_are_rejected() {
        for html in [
            r#"<title>Log in</title><meta property="og:title" content="Log in">"#,
            r#"<meta property="og:title" content="YouTube"><meta property="og:image" content="https://cdn.example.test/logo.jpg">"#,
            r#"<title>Before you continue</title>"#,
        ] {
            assert!(
                from_html(
                    "https://www.youtube.com/post/abc_123",
                    "https://www.youtube.com/post/abc_123",
                    Platform::YouTube,
                    html
                )
                .is_err()
            );
        }
    }

    #[test]
    fn cross_host_redirect_and_credential_or_javascript_pages_are_rejected() {
        assert!(validate_page("https://clips.twitch.tv/Abc_123", Platform::Twitch).is_ok());
        assert!(validate_page("https://evil.example/post/Abc_123", Platform::Twitch).is_err());
        assert!(validate_page("https://clips.twitch.tv/login", Platform::Twitch).is_err());
        assert!(public_url("javascript:alert(1)").is_none());
        assert!(public_url("https://user:pass@cdn.example.com/a.jpg").is_none());
        assert!(public_url("http://127.0.0.1/a.jpg").is_none());
        assert!(public_url("https://cdn.example.com/a.jpg?name=.mp4").is_some());
    }

    #[test]
    fn preview_text_follows_the_shared_markdown_contract() {
        // Kelime içi/kenar alt çizgi kaçırılmaz: kaçış kartta kaynakta olmayan bir `\` gösterir.
        assert_eq!(
            escape_preview_markdown("world_record_egg"),
            "world_record_egg"
        );
        assert_eq!(escape_preview_markdown("_öne çıkan_"), "_öne çıkan_");
        // Biçimi gerçekten bozabilecek karakterler yine kaçırılır.
        assert_eq!(
            escape_preview_markdown("**kalın** [link]"),
            "\\*\\*kalın\\*\\* \\[link\\]"
        );
        // Yalnızca satır başındaki `#` başlık oluşturabilir (satır ortasındaki kaçırılmaz).
        assert_eq!(escape_preview_markdown("# Başlık"), "\\# Başlık");
        assert_eq!(escape_preview_markdown("C# dili"), "C# dili");
        assert_eq!(escape_preview_markdown("a\n# b"), "a\n\\# b");
        assert_eq!(
            escape_preview_markdown("a\n    # girinti"),
            "a\n    # girinti"
        );
        // Bahsetme enjeksiyonu ve açılı parantezler etkisizleştirilir.
        assert_eq!(escape_preview_markdown("@everyone"), "@\u{200b}everyone");
    }

    #[test]
    fn apostrophes_and_angle_brackets_inside_values_survive() {
        let html = r#"<meta property="og:title" content="Don't > stop">
            <meta property="og:description" content="It's here">"#;
        let post = from_html(
            "https://www.youtube.com/post/abc_123",
            "https://www.youtube.com/post/abc_123",
            Platform::YouTube,
            html,
        )
        .expect("meaningful metadata");
        let preview = post.link_preview.unwrap();
        assert_eq!(preview.title.as_deref(), Some("Don't › stop"));
        assert_eq!(preview.description.as_deref(), Some("It's here"));
    }

    #[test]
    fn invalid_media_is_dropped_and_author_stays_neutral() {
        let html = r#"<meta property="og:title" content="Sunset - artist123">
            <meta property="og:description" content="Real caption">
            <meta property="og:image" content="http://127.0.0.1/poster.jpg">
            <meta property="og:video" content="https://cdn.example.com/player.m3u8">
        "#;
        let post = from_html(
            "https://www.pixiv.net/artworks/12345",
            "https://www.pixiv.net/artworks/12345",
            Platform::Pixiv,
            html,
        )
        .expect("meaningful OG metadata should still build a degraded card");
        assert!(
            post.media.is_empty(),
            "private/playlist media must not be claimed"
        );
        assert_eq!(post.author.name, "Pixiv");
        assert!(post.author.handle.is_empty());
        assert!(post.author.profile_url.is_none());
        assert!(
            post.text.is_empty(),
            "description is rendered through LinkPreview once"
        );
        assert_eq!(
            post.link_preview.as_ref().and_then(|p| p.title.as_deref()),
            Some("Sunset - artist123")
        );
    }

    #[test]
    fn rich_fixture_uses_path_suffix_and_escapes_metadata() {
        let post = from_html(
            "https://www.youtube.com/post/abc_123",
            "https://www.youtube.com/post/abc_123",
            Platform::YouTube,
            RICH,
        )
        .expect("rich fixture");
        assert_eq!(
            post.media.len(),
            1,
            "poster is not duplicated beside the video"
        );
        assert_eq!(post.media[0].kind, MediaKind::Video);
        assert!(
            post.link_preview
                .as_ref()
                .unwrap()
                .description
                .as_deref()
                .unwrap()
                .contains("@\u{200b}")
        );
        assert!(
            post.link_preview
                .as_ref()
                .unwrap()
                .description
                .as_deref()
                .unwrap()
                .contains("\\[")
        );
    }
}
