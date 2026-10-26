//! Facebook gönderisi/videosu/reel'i: oturumsuz iki herkese açık sayfadan okunur.
//!
//! - Gönderi sayfasının OpenGraph etiketleri: metin (`og:description`), izlenme/tepki sayıları
//!   (`og:title`'ın `2.7M views · 66K reactions | ...` öneki), kapak görseli ve sayfa adresi
//!   (`og:url`, ör. `/Marvel/videos/...`).
//! - Video gönderilerinde Facebook'un gömme oynatıcısı (`/plugins/video.php`): sayfanın görünen
//!   adı ve oynatıcının doğrudan MP4 adresi (`hd_src`/`sd_src`). Adres imzalıdır ve süresi
//!   dolar; video gönderimde dosya olarak eklenir (bkz. `render::resolve_media`).
//!
//! Gönderi sayfası büyüktür (~450 KB) ve yavaş gelebilir: genel önizlemenin 2 saniyelik sınırı
//! yerine paylaşılan istemcinin süresi kullanılır. Okunamazsa genel önizlemeye düşülür.

use std::sync::LazyLock;

use regex::Regex;
use tracing::debug;

use super::opengraph::{self, OpenGraph};
use super::{
    Author, BuiltinPost, Media, MediaKind, REQUEST_TIMEOUT, USER_AGENT, preview, unescape_html,
};
use crate::converters::parser::{ParsedPost, Platform};

const FACEBOOK: &str = "https://www.facebook.com";

/// Gömme oynatıcıdaki doğrudan MP4 adresi (JSON dizesi; `\/` kaçışlı).
static VIDEO_SRC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#""(hd|sd)_src":"((?:[^"\\]|\\.)*)""#).expect("geçerli Facebook video regex")
});
/// Gömme oynatıcının üst bilgisindeki sayfa bağlantısı: `<a title="Marvel" ... href="/watch/Marvel/`.
static OWNER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<a title="([^"]{1,200})" target="_blank" href="/(?:watch/)?([A-Za-z0-9.]{1,100})/"#,
    )
    .expect("geçerli Facebook sahip regex")
});
/// oEmbed bağlantısının başlığı sayfa adıyla biter: `title="... | Marvel"`.
static OEMBED_TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?s)<link rel="alternate" href="https://graph\.facebook\.com/[^"]*" title="([^"]*)""#,
    )
    .expect("geçerli Facebook oEmbed regex")
});

/// Facebook gönderisini çeker ve karta dönüştürür.
pub async fn fetch(client: &reqwest::Client, parsed: &ParsedPost) -> Result<BuiltinPost, String> {
    let url = parsed.original_url.as_str();
    let player_url = is_video(url).then(|| {
        format!(
            "{FACEBOOK}/plugins/video.php?href={}",
            super::encode_component(url)
        )
    });
    let (page, player) = tokio::join!(get_page(client, url), async {
        match &player_url {
            Some(player_url) => get_page(client, player_url).await.ok(),
            None => None,
        }
    });
    let result = page.and_then(|html| post_from_html(url, &html, player.as_deref()));
    match result {
        Ok(post) => Ok(post),
        Err(reason) => {
            debug!("Facebook gönderisi okunamadı, genel önizlemeye düşülüyor: {reason}");
            preview::fetch(parsed, Platform::Facebook).await
        }
    }
}

/// Facebook sayfası. Tarayıcı kimliğiyle sayfa 2 MB'ı aşan tam uygulamayı döndürür; botun
/// kendi kimliğiyle OpenGraph etiketli hafif sürüm gelir. Sayılar İngilizce istenir.
async fn get_page(client: &reqwest::Client, url: &str) -> Result<String, String> {
    let response = client
        .get(url)
        .header("user-agent", USER_AGENT)
        .header("accept-language", "en-US,en;q=0.9")
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("Facebook isteği başarısız: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("Facebook {} döndü", response.status()));
    }
    super::read_limited_text(response, url).await
}

/// Reel, video ve izleme adresleri gömme oynatıcıyla açılabilir.
fn is_video(url: &str) -> bool {
    let path = url.strip_prefix(FACEBOOK).unwrap_or_default();
    path.starts_with("/reel/") || path.starts_with("/watch") || path.contains("/videos/")
}

/// `og:title`'ın başındaki sayı öneki: `2.7M views · 66K reactions | Metin` →
/// (izlenme, tepki, yorum, paylaşım) ve kalan başlık.
fn split_title(title: &str) -> (Stats, &str) {
    let mut stats = Stats::default();
    let Some((prefix, rest)) = title.split_once(" | ") else {
        return (stats, title);
    };
    let mut known = false;
    for part in prefix.split('·') {
        let mut words = part.split_whitespace();
        let (Some(number), Some(unit)) = (words.next(), words.next()) else {
            continue;
        };
        let Some(value) = super::parse_compact_count(number) else {
            continue;
        };
        let slot = match unit.trim_end_matches('s') {
            "view" | "play" => &mut stats.views,
            "reaction" | "like" => &mut stats.reactions,
            "comment" => &mut stats.comments,
            "share" => &mut stats.shares,
            _ => continue,
        };
        *slot = Some(value);
        known = true;
    }
    if known { (stats, rest) } else { (stats, title) }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Stats {
    views: Option<u64>,
    reactions: Option<u64>,
    comments: Option<u64>,
    shares: Option<u64>,
}

/// `og:url` yolunun ilk parçası sayfanın kullanıcı adıdır (`/Marvel/videos/...`); sayısal
/// kimlikler ve genel yollar (`/reel/`, `/watch/`) ad değildir.
fn page_slug(og_url: Option<&str>) -> Option<String> {
    let url = url::Url::parse(og_url?).ok()?;
    if !matches!(
        url.host_str()?,
        "www.facebook.com" | "facebook.com" | "m.facebook.com"
    ) {
        return None;
    }
    let slug = url.path_segments()?.next()?;
    let generic = [
        "reel",
        "watch",
        "videos",
        "story.php",
        "permalink.php",
        "people",
        "profile.php",
        "groups",
        "share",
    ];
    let valid = (1..=100).contains(&slug.len())
        && !generic.contains(&slug)
        && !slug.bytes().all(|b| b.is_ascii_digit())
        && slug.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.');
    valid.then(|| slug.to_string())
}

/// Gömme oynatıcının doğrudan MP4 adresi: önce HD, yoksa SD. Yalnızca Facebook CDN'i kabul edilir.
fn player_video(player: &str) -> Option<String> {
    let mut sources: Vec<(bool, String)> = VIDEO_SRC
        .captures_iter(player)
        .filter_map(|caps| {
            let url: String = serde_json::from_str(&format!("\"{}\"", &caps[2])).ok()?;
            Some((&caps[1] == "hd", url))
        })
        .filter(|(_, url)| fbcdn(url))
        .collect();
    sources.sort_by_key(|(hd, _)| !hd);
    sources.into_iter().next().map(|(_, url)| url)
}

fn fbcdn(raw: &str) -> bool {
    url::Url::parse(raw).is_ok_and(|url| {
        url.scheme() == "https"
            && url
                .host_str()
                .is_some_and(|host| host.ends_with(".fbcdn.net"))
    })
}

fn owner_name(html: &str, player: Option<&str>, slug: Option<&str>) -> Option<String> {
    // Oynatıcıdaki bağlantı yalnızca og:url'deki sayfayla eşleşiyorsa kullanılır.
    let from_player = player.and_then(|player| {
        OWNER
            .captures_iter(player)
            .find(|caps| slug.is_none_or(|slug| slug.eq_ignore_ascii_case(&caps[2])))
            .map(|caps| unescape_html(&caps[1]))
    });
    let from_oembed = || {
        let title = unescape_html(&OEMBED_TITLE.captures(html)?[1]);
        let name = title.rsplit_once(" | ")?.1.trim().to_string();
        (!name.is_empty() && name.len() <= 200).then_some(name)
    };
    from_player
        .or_else(from_oembed)
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
}

fn post_from_html(url: &str, html: &str, player: Option<&str>) -> Result<BuiltinPost, String> {
    let graph: OpenGraph = opengraph::parse_open_graph(html);
    let raw_title = graph.title.as_deref().unwrap_or_default();
    let (stats, title_text) = split_title(raw_title);
    // Giriş/hata sayfası: başlık yalnızca "Facebook" ya da "Log in" olur ve sayı öneki yoktur.
    let generic_title = matches!(
        title_text.trim().to_ascii_lowercase().as_str(),
        "" | "facebook" | "log in" | "log into facebook" | "content not found"
    );
    let text = graph
        .description
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .or_else(|| (!generic_title).then_some(title_text.trim()))
        .unwrap_or_default();
    let video = player.and_then(player_video);
    let image = graph.image.filter(|image| fbcdn(image));
    if text.is_empty() && video.is_none() && (generic_title || image.is_none()) {
        return Err(format!(
            "Facebook herkese açık gönderi verisi yok (başlık: {:?})",
            crate::utils::truncate_chars(raw_title, 80)
        ));
    }

    let slug = page_slug(graph.url.as_deref());
    let name = owner_name(html, player, slug.as_deref())
        .or_else(|| slug.clone())
        .unwrap_or_else(|| "Facebook".to_string());
    let mut author = Author::new(name, slug.clone().unwrap_or_default());
    author.profile_url = slug.as_ref().map(|slug| format!("{FACEBOOK}/{slug}"));

    let mut post = BuiltinPost::new(Platform::Facebook, url.to_string(), author);
    // Facebook paragraf aralarına yalnızca boşluk içeren satırlar koyar (`\n \n`).
    let text: Vec<&str> = text.lines().map(str::trim_end).collect();
    post.text = preview::escape_preview_markdown(&text.join("\n"));
    post.stats.views = stats.views;
    post.stats.likes = stats.reactions.unwrap_or(0);
    post.stats.replies = stats.comments.unwrap_or(0);
    post.stats.reposts = stats.shares.unwrap_or(0);
    let path = url.strip_prefix(FACEBOOK).unwrap_or_default();
    post.extra_label = Some(
        if path.starts_with("/reel/") {
            "Reel"
        } else if is_video(url) {
            "Video"
        } else {
            "Post"
        }
        .into(),
    );
    match video {
        Some(video) => post.media.push(Media {
            kind: MediaKind::Video,
            url: video,
            poster: image,
            width: None,
            height: None,
            duration: None,
        }),
        None => post.media.extend(image.map(|url| Media {
            kind: MediaKind::Image,
            url,
            poster: None,
            width: None,
            height: None,
            duration: None,
        })),
    }
    Ok(post)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Canlı reel sayfasından kısaltılmış örnek (çok satırlı başlık ve açıklama).
    const PAGE: &str = r#"<link rel="alternate" href="https://graph.facebook.com/v26.0/oembed_video?url=x" title="Stand together. | Marvel" type="application/json+oembed" />
        <meta property="og:type" content="video.other" />
        <meta property="og:title" content="2.7M views &#xb7; 66K reactions | Stand together.

Avengers: Doomsday ..." />
        <meta property="og:description" content="Stand together.

Avengers: Doomsday arrives in theaters everywhere December 18. Get tickets now: fandango.com/avengersdoomsday" />
        <meta property="og:url" content="https://www.facebook.com/Marvel/videos/stand-together/1609143203501953/" />
        <meta property="og:image" content="https://scontent.fesb6-1.fna.fbcdn.net/v/t15/poster.jpg?_nc_cat=1&amp;oe=6AE3B735" />
        <title>Facebook</title>"#;
    const PLAYER: &str = r#"<a title="Marvel" target="_blank" href="/watch/Marvel/?ref=embed_video" id="u_0_c">
        "sd_src":"https:\/\/video.fesb6-1.fna.fbcdn.net\/o1\/v\/sd.mp4?a=1&b=2",
        "hd_src":"https:\/\/video.fesb6-1.fna.fbcdn.net\/o1\/v\/hd.mp4?a=1&b=2","#;
    const REEL: &str = "https://www.facebook.com/reel/1609143203501953";

    #[test]
    fn title_prefix_carries_the_counts() {
        let (stats, rest) = split_title("2.7M views · 66K reactions | Stand together.");
        assert_eq!(
            stats,
            Stats {
                views: Some(2_700_000),
                reactions: Some(66_000),
                ..Stats::default()
            }
        );
        assert_eq!(rest, "Stand together.");
        let (stats, rest) = split_title("1,234 reactions · 56 comments · 7 shares | Hi | there");
        assert_eq!(stats.comments, Some(56));
        assert_eq!(stats.shares, Some(7));
        assert_eq!(rest, "Hi | there");
        assert_eq!(split_title("Marvel | Facebook").1, "Marvel | Facebook");
    }

    #[test]
    fn reel_becomes_a_playable_card() {
        let post = post_from_html(REEL, PAGE, Some(PLAYER)).unwrap();
        assert_eq!(post.url, REEL);
        assert_eq!(post.author.name, "Marvel");
        assert_eq!(post.author.handle, "Marvel");
        assert_eq!(
            post.author.profile_url.as_deref(),
            Some("https://www.facebook.com/Marvel")
        );
        assert!(
            post.text
                .starts_with("Stand together.\n\nAvengers: Doomsday arrives")
        );
        assert_eq!(post.stats.views, Some(2_700_000));
        assert_eq!(post.stats.likes, 66_000);
        assert_eq!(post.extra_label.as_deref(), Some("Reel"));
        assert_eq!(post.media.len(), 1);
        assert!(post.media[0].is_video());
        assert_eq!(
            post.media[0].url,
            "https://video.fesb6-1.fna.fbcdn.net/o1/v/hd.mp4?a=1&b=2"
        );
        assert_eq!(
            post.media[0].poster.as_deref(),
            Some("https://scontent.fesb6-1.fna.fbcdn.net/v/t15/poster.jpg?_nc_cat=1&oe=6AE3B735")
        );
    }

    #[test]
    fn without_the_player_the_poster_image_and_oembed_name_are_used() {
        let post = post_from_html(REEL, PAGE, None).unwrap();
        assert_eq!(post.author.name, "Marvel");
        assert_eq!(post.media.len(), 1);
        assert!(!post.media[0].is_video());
    }

    #[test]
    fn login_pages_and_foreign_media_are_rejected() {
        let login = r#"<meta property="og:title" content="Log into Facebook" />"#;
        assert!(post_from_html(REEL, login, None).is_err());
        let foreign = r#""hd_src":"https:\/\/evil.example\/a.mp4""#;
        assert_eq!(player_video(foreign), None);
        assert_eq!(page_slug(Some("https://www.facebook.com/reel/123/")), None);
        assert_eq!(
            page_slug(Some("https://www.facebook.com/100044606041735/videos/1/")),
            None
        );
        assert_eq!(page_slug(Some("https://evil.example/Marvel/")), None);
    }

    #[tokio::test]
    #[ignore = "requires live Facebook access; run explicitly"]
    async fn live_facebook_reel() {
        let parsed = ParsedPost {
            platform: Platform::Facebook,
            is_spoiler: false,
            user_comment: None,
            original_url: REEL.into(),
        };
        let post = fetch(&reqwest::Client::new(), &parsed)
            .await
            .expect("reel okunmalı");
        println!("{post:#?}");
        assert_eq!(post.author.handle, "Marvel");
        assert!(post.media.first().is_some_and(Media::is_video));
    }
}
