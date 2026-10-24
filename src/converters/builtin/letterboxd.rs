//! Letterboxd film ve kullanıcı incelemesi kartları (native, Components V2).
//!
//! Veri giriş/API anahtarı gerektirmeyen herkese açık sayfalardan okunur:
//! - **Film sayfası** (`/film/{slug}/`): JSON-LD `Movie` nesnesi başlık, poster, yönetmen,
//!   oyuncular, türler, ülke, süre, açıklama ve Letterboxd ortalamasını (`aggregateRating`)
//!   taşır; HTML'den slogan, orijinal ad, yıl, fragman ve IMDb/TMDB bağlantıları alınır.
//! - **İnceleme sayfası** (`/{kullanıcı}/film/{slug}/`): JSON-LD `Review` nesnesi yazarı,
//!   puanı, metni (paragraflarıyla) ve tarihi taşır; HTML'den beğeni sayısı, kalp, yeniden
//!   izleme, izleme tarihi ve spoiler uyarısı okunur.
//!
//! Numaralı yeniden izleme kayıtları (`/{kullanıcı}/film/{slug}/{n}/`) Cloudflare tarafından
//! botlara kapatılmıştır (her User-Agent'ta 403). Bu durumda kart filmin kendisini gösterir ve
//! incelemenin kime ait olduğunu, bağlantısıyla birlikte belirtir. `boxd.it` kısa linkleri
//! hedefe çözülür; yanıt engellenmiş olsa bile varılan adres hedefi belirler.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Value, json};
use tracing::debug;

use super::render::{escape_markdown, format_count};
use super::{Author, BuiltinPost, CustomCard, REQUEST_TIMEOUT, USER_AGENT, unescape_html};
use crate::converters::parser::{ParsedPost, Platform};
use crate::utils::truncate_chars;

const LETTERBOXD: &str = "https://letterboxd.com";
/// Letterboxd yeşili (logodaki üç noktadan): film kartı.
const LETTERBOXD_GREEN: u32 = 0x00E054;
/// Letterboxd turuncusu: kullanıcı incelemesi kartı.
const LETTERBOXD_ORANGE: u32 = 0xFF8000;

/// Kartta gösterilen açıklama ve inceleme metni sınırları.
const SYNOPSIS_LIMIT: usize = 600;
const REVIEW_LIMIT: usize = 1_500;
const MAX_CAST: usize = 5;
const MAX_GENRES: usize = 4;

// Components V2 bileşen türleri (yalnızca burada kullanılanlar).
const TYPE_ACTION_ROW: u64 = 1;
const TYPE_BUTTON: u64 = 2;
const TYPE_SECTION: u64 = 9;
const TYPE_TEXT_DISPLAY: u64 = 10;
const TYPE_THUMBNAIL: u64 = 11;
const TYPE_SEPARATOR: u64 = 14;
const STYLE_LINK: u64 = 5;

static TARGET_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https://letterboxd\.com/(?:film/([a-z0-9-]+)/|([A-Za-z0-9_]+)/film/([a-z0-9-]+)/(?:([0-9]{1,6})/)?)")
        .expect("geçerli Letterboxd hedef regex")
});
static JSON_LD_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<script type="application/ld\+json">(.*?)</script>"#)
        .expect("geçerli JSON-LD regex")
});
static CDATA_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)/\*.*?\*/").expect("geçerli yorum regex"));
static YEAR_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<span class="releasedate"><a href="/films/year/([0-9]{4})/">"#)
        .expect("geçerli yıl regex")
});
static TAGLINE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<h4 class="tagline">(.*?)</h4>"#).expect("geçerli slogan regex")
});
static ORIGINAL_NAME_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<h2 class="originalname"[^>]*>(.*?)</h2>"#)
        .expect("geçerli orijinal ad regex")
});
static RUNTIME_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"text-footer">\s*([0-9]{1,4})(?:&nbsp;|\s)mins"#).expect("geçerli süre regex")
});
static IMDB_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"href="https?://www\.imdb\.com/title/(tt[0-9]{5,10})/"#)
        .expect("geçerli IMDb regex")
});
static TMDB_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"href="https://www\.themoviedb\.org/(movie|tv)/([0-9]{1,10})/?""#)
        .expect("geçerli TMDB regex")
});
static TRAILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"data-track-category="Trailer" href="//www\.youtube(?:-nocookie)?\.com/embed/([A-Za-z0-9_-]{6,20})"#)
        .expect("geçerli fragman regex")
});
static TAG_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<[^>]+>").expect("geçerli etiket regex"));
static WATCHED_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)class="view-date">\s*Watched\s*<a href="/[^/"]+/diary/for/([0-9]{4})/([0-9]{2})/([0-9]{2})/""#)
        .expect("geçerli izleme tarihi regex")
});
static LIKES_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)review-like".{0,800}?data-count="([0-9]{1,12})""#)
        .expect("geçerli beğeni regex")
});
static AVATAR_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<a class="avatar[^"]*" href="/([A-Za-z0-9_]+)/"\s*>\s*<img src="(https://a\.ltrbxd\.com/[^"]+)""#)
        .expect("geçerli avatar regex")
});

/// Kart hedefi: film, kullanıcı incelemesi (yeniden izlemede kayıt numarasıyla).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    Film {
        slug: String,
    },
    Review {
        user: String,
        slug: String,
        entry: Option<u32>,
    },
}

impl Target {
    fn from_url(url: &str) -> Option<Self> {
        let caps = TARGET_REGEX.captures(url)?;
        if let Some(slug) = caps.get(1) {
            return Some(Self::Film {
                slug: slug.as_str().to_string(),
            });
        }
        Some(Self::Review {
            user: caps.get(2)?.as_str().to_string(),
            slug: caps.get(3)?.as_str().to_string(),
            entry: caps.get(4).and_then(|n| n.as_str().parse().ok()),
        })
    }

    fn slug(&self) -> &str {
        match self {
            Self::Film { slug } | Self::Review { slug, .. } => slug,
        }
    }

    fn film_url(&self) -> String {
        format!("{LETTERBOXD}/film/{}/", self.slug())
    }

    fn url(&self) -> String {
        match self {
            Self::Film { .. } => self.film_url(),
            Self::Review { user, slug, entry } => {
                let entry = entry.map(|n| format!("{n}/")).unwrap_or_default();
                format!("{LETTERBOXD}/{user}/film/{slug}/{entry}")
            }
        }
    }
}

/// Bir Letterboxd sayfası: varılan adres ve (yalnızca 2xx'te) gövde.
struct Page {
    url: String,
    body: Option<String>,
}

/// Letterboxd bağlantısını karta dönüştürür.
pub async fn fetch(client: &reqwest::Client, parsed: &ParsedPost) -> Result<BuiltinPost, String> {
    let (target, page) = resolve(client, &parsed.original_url).await?;
    match &target {
        Target::Film { .. } => {
            let html = page.body.as_deref().ok_or_else(|| {
                format!("Letterboxd film sayfası alınamadı: {}", target.film_url())
            })?;
            let film = parse_film(html, &target.film_url())?;
            Ok(film_post(&film, None))
        }
        Target::Review { user, .. } => {
            if let Some(html) = page.body.as_deref()
                && let Some(review) = parse_review(html, &target)
            {
                return Ok(review_post(&review));
            }
            // İnceleme sayfası engellendi ya da okunamadı: film kartı + inceleme notu.
            debug!(
                "Letterboxd inceleme sayfası okunamadı, film kartına düşülüyor: {}",
                target.url()
            );
            let film_page = get_page(client, &target.film_url()).await?;
            let html = film_page.body.ok_or_else(|| {
                format!("Letterboxd film sayfası alınamadı: {}", target.film_url())
            })?;
            let film = parse_film(&html, &target.film_url())?;
            Ok(film_post(
                &film,
                Some(ReviewLink {
                    user: user.clone(),
                    url: target.url(),
                }),
            ))
        }
    }
}

/// Bağlantıyı hedefe ve sayfasına çözer. `boxd.it` kısa linki tek istekte hem çözülür hem
/// indirilir; hedefi belirleyen, gövde değil varılan adrestir (engelli sayfa da hedef verir).
async fn resolve(client: &reqwest::Client, original_url: &str) -> Result<(Target, Page), String> {
    let page = get_page(client, original_url).await?;
    let target = Target::from_url(&page.url)
        .or_else(|| Target::from_url(original_url))
        .ok_or_else(|| format!("Letterboxd hedefi çözümlenemedi: {original_url}"))?;
    Ok((target, page))
}

/// Sayfayı indirir. Durum 2xx değilse gövde okunmaz ama varılan adres yine döner (kısa link
/// çözümü ve engelli inceleme yedeği bunu kullanır). Yalnızca `letterboxd.com`'a varılabilir.
async fn get_page(client: &reqwest::Client, url: &str) -> Result<Page, String> {
    let response = client
        .get(url)
        .header("user-agent", USER_AGENT)
        .header("accept", "text/html,application/xhtml+xml;q=0.9,*/*;q=0.8")
        .header("accept-language", "en-US,en;q=0.9")
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("Letterboxd isteği başarısız ({url}): {e}"))?;
    let final_url = response.url().clone();
    if final_url.scheme() != "https" || final_url.host_str() != Some("letterboxd.com") {
        return Err(format!(
            "Letterboxd bağlantısı beklenmeyen bir adrese gitti: {final_url}"
        ));
    }
    let status = response.status();
    let body = if status.is_success() {
        Some(super::read_limited_text(response, url).await?)
    } else {
        debug!("Letterboxd sayfası {status} döndü: {final_url}");
        None
    };
    Ok(Page {
        url: final_url.to_string(),
        body,
    })
}

// ─── Ayrıştırma ──────────────────────────────────────────────────────────────

/// Sayfadaki `@type` uyan ilk JSON-LD nesnesi (Letterboxd onu `/* <![CDATA[ */` ile sarar).
fn json_ld(html: &str, kind: &str) -> Option<Value> {
    JSON_LD_REGEX.captures_iter(html).find_map(|caps| {
        let raw = CDATA_REGEX.replace_all(caps.get(1)?.as_str(), "");
        let value: Value = serde_json::from_str(raw.trim()).ok()?;
        (value.get("@type").and_then(Value::as_str) == Some(kind)).then_some(value)
    })
}

fn text_of(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

/// `[{"name": …}]` dizisindeki adlar (tek nesne de kabul edilir).
fn names_of(value: &Value, key: &str) -> Vec<String> {
    match value.get(key) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| text_of(item, "name"))
            .collect(),
        Some(item @ Value::Object(_)) => text_of(item, "name").into_iter().collect(),
        _ => Vec::new(),
    }
}

/// Düz metin dizisi (`genre`); tek metin de kabul edilir.
fn strings_of(value: &Value, key: &str) -> Vec<String> {
    match value.get(key) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
            .collect(),
        Some(Value::String(text)) if !text.trim().is_empty() => vec![text.trim().to_string()],
        _ => Vec::new(),
    }
}

fn first_capture(regex: &Regex, html: &str) -> Option<String> {
    regex
        .captures(html)
        .and_then(|caps| caps.get(1))
        .map(|m| {
            unescape_html(&TAG_REGEX.replace_all(m.as_str(), ""))
                .trim()
                .to_string()
        })
        .filter(|text| !text.is_empty())
}

/// ISO 8601 süresi (`PT2H13M`) → dakika.
fn iso_minutes(duration: &str) -> Option<u32> {
    let rest = duration.strip_prefix("PT")?;
    let (hours, rest) = match rest.split_once('H') {
        Some((hours, rest)) => (hours.parse::<u32>().ok()?, rest),
        None => (0, rest),
    };
    let minutes = match rest.strip_suffix('M') {
        Some(minutes) if !minutes.is_empty() => minutes.parse::<u32>().ok()?,
        _ => 0,
    };
    Some(hours * 60 + minutes).filter(|total| *total > 0)
}

/// Küçük poster varyantını kartta keskin görünen 600×900 varyantına çevirir (aynı dosya).
fn large_poster(url: &str) -> String {
    url.replace("-0-230-0-345-crop", "-0-600-0-900-crop")
}

fn https_image(url: Option<String>) -> Option<String> {
    url.filter(|url| url.starts_with("https://"))
}

#[derive(Debug, Clone, Default, PartialEq)]
struct Film {
    url: String,
    title: String,
    year: Option<String>,
    original_title: Option<String>,
    tagline: Option<String>,
    synopsis: Option<String>,
    poster: Option<String>,
    directors: Vec<String>,
    cast: Vec<String>,
    genres: Vec<String>,
    countries: Vec<String>,
    runtime: Option<u32>,
    /// Letterboxd ortalaması (0,5–5).
    rating: Option<f64>,
    rating_count: Option<u64>,
    imdb: Option<String>,
    tmdb: Option<String>,
    trailer: Option<String>,
}

fn parse_film(html: &str, url: &str) -> Result<Film, String> {
    let movie = json_ld(html, "Movie")
        .ok_or_else(|| format!("Letterboxd film verisi bulunamadı: {url}"))?;
    let title = text_of(&movie, "name")
        .ok_or_else(|| format!("Letterboxd film başlığı okunamadı: {url}"))?;
    let rating = movie.get("aggregateRating");
    let original_title = first_capture(&ORIGINAL_NAME_REGEX, html)
        .filter(|original| !original.eq_ignore_ascii_case(&title));

    Ok(Film {
        url: url.to_string(),
        year: first_capture(&YEAR_REGEX, html),
        original_title,
        tagline: first_capture(&TAGLINE_REGEX, html),
        synopsis: text_of(&movie, "description"),
        poster: https_image(text_of(&movie, "image").map(|url| large_poster(&url))),
        directors: names_of(&movie, "director"),
        cast: names_of(&movie, "actor"),
        genres: strings_of(&movie, "genre"),
        countries: names_of(&movie, "countryOfOrigin"),
        runtime: text_of(&movie, "duration")
            .and_then(|duration| iso_minutes(&duration))
            .or_else(|| first_capture(&RUNTIME_REGEX, html)?.parse().ok()),
        rating: rating
            .and_then(|r| r.get("ratingValue"))
            .and_then(Value::as_f64)
            .filter(|value| (0.5..=5.0).contains(value)),
        rating_count: rating
            .and_then(|r| r.get("ratingCount"))
            .and_then(Value::as_u64)
            .filter(|count| *count > 0),
        imdb: first_capture(&IMDB_REGEX, html)
            .map(|id| format!("https://www.imdb.com/title/{id}/")),
        tmdb: TMDB_REGEX
            .captures(html)
            .map(|caps| format!("https://www.themoviedb.org/{}/{}/", &caps[1], &caps[2])),
        trailer: first_capture(&TRAILER_REGEX, html)
            .map(|id| format!("https://www.youtube.com/watch?v={id}")),
        title,
    })
}

#[derive(Debug, Clone, Default, PartialEq)]
struct Review {
    url: String,
    author: String,
    profile_url: String,
    avatar: Option<String>,
    /// Kullanıcının puanı (0,5–5); puansız kayıtta `None`.
    rating: Option<f64>,
    body: Option<String>,
    spoiler: bool,
    liked: bool,
    rewatch: bool,
    /// İzleme günü (yıl, ay, gün); günlükte yoksa yayın tarihi.
    watched: Option<(u32, u32, u32)>,
    published: Option<(u32, u32, u32)>,
    likes: Option<u64>,
    film_title: String,
    film_year: Option<String>,
    film_url: String,
    poster: Option<String>,
    director: Option<String>,
}

fn parse_date(raw: &str) -> Option<(u32, u32, u32)> {
    let mut parts = raw.get(..10)?.split('-');
    let date = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    (1..=12).contains(&date.1).then_some(date)
}

/// Yalnızca kaydın üst şeridindeki tepkiler okunur: sayfanın geri kalanında başka
/// kullanıcıların incelemeleri (ve onların kalpleri) de bulunur.
fn reaction_strip(html: &str) -> &str {
    let Some(start) = html.find("content-reactions-strip") else {
        return "";
    };
    let rest = &html[start..];
    let end = rest.find("view-date").unwrap_or(rest.len().min(8_000));
    &rest[..end]
}

fn parse_review(html: &str, target: &Target) -> Option<Review> {
    let Target::Review { user, .. } = target else {
        return None;
    };
    let review = json_ld(html, "Review")?;
    let item = review.get("itemReviewed")?;
    let author = review
        .get("author")
        .and_then(|authors| match authors {
            Value::Array(items) => items.first(),
            other => Some(other),
        })
        .and_then(|author| text_of(author, "name"))
        .unwrap_or_else(|| user.clone());
    let strip = reaction_strip(html);
    let watched = WATCHED_REGEX.captures(html).and_then(|caps| {
        Some((
            caps[1].parse().ok()?,
            caps[2].parse().ok()?,
            caps[3].parse().ok()?,
        ))
    });

    Some(Review {
        url: target.url(),
        author,
        profile_url: format!("{LETTERBOXD}/{user}/"),
        avatar: AVATAR_REGEX
            .captures_iter(html)
            .find(|caps| &caps[1] == user.as_str())
            .map(|caps| caps[2].to_string()),
        rating: review
            .get("reviewRating")
            .and_then(|rating| rating.get("ratingValue"))
            .and_then(Value::as_f64)
            .filter(|value| (0.5..=5.0).contains(value)),
        body: text_of(&review, "reviewBody"),
        // Yalnızca bu kaydın meta açıklaması: sayfadaki diğer incelemeler de aynı uyarıyı taşır.
        spoiler: html.contains(r#"og:description" content="This review may contain spoilers"#),
        liked: strip.contains(r#"aria-label="Liked""#),
        rewatch: strip.contains(r#"aria-label="Rewatch"#) || strip.contains("icon-rewatch"),
        watched,
        published: text_of(&review, "datePublished").and_then(|date| parse_date(&date)),
        likes: LIKES_REGEX
            .captures(html)
            .and_then(|caps| caps[1].parse().ok())
            .filter(|count| *count > 0),
        film_title: text_of(item, "name")?,
        film_year: first_capture(&YEAR_REGEX, html),
        film_url: target.film_url(),
        poster: https_image(text_of(item, "image").map(|url| large_poster(&url))),
        director: names_of(item, "director").into_iter().next(),
    })
}

// ─── Kartlar ─────────────────────────────────────────────────────────────────

/// Puanı Letterboxd'un yıldız diliyle yazar: en yakın yarım yıldıza yuvarlanır (`★★★★½`).
fn stars(rating: f64) -> String {
    let halves = (rating * 2.0).round().clamp(1.0, 10.0) as usize;
    format!(
        "{}{}",
        "★".repeat(halves / 2),
        if halves % 2 == 1 { "½" } else { "" }
    )
}

fn runtime_label(minutes: u32) -> String {
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m} dk"),
        (h, 0) => format!("{h} sa"),
        (h, m) => format!("{h} sa {m} dk"),
    }
}

fn date_label((year, month, day): (u32, u32, u32)) -> String {
    const MONTHS: [&str; 12] = [
        "Oca", "Şub", "Mar", "Nis", "May", "Haz", "Tem", "Ağu", "Eyl", "Eki", "Kas", "Ara",
    ];
    let month = MONTHS
        .get(month.saturating_sub(1) as usize)
        .copied()
        .unwrap_or("?");
    format!("{day} {month} {year}")
}

fn text(content: impl Into<String>) -> Value {
    json!({ "type": TYPE_TEXT_DISPLAY, "content": content.into() })
}

fn link_button(label: &str, url: &str) -> Value {
    json!({ "type": TYPE_BUTTON, "style": STYLE_LINK, "label": label, "url": url })
}

/// Posterli bölüm; poster yoksa erişilebilirlik bir bağlantı butonu olur.
fn section(children: Vec<Value>, poster: Option<&str>, fallback_url: &str, alt: &str) -> Value {
    let accessory = match poster {
        Some(url) => json!({ "type": TYPE_THUMBNAIL, "media": { "url": url }, "description": alt }),
        None => link_button("Letterboxd", fallback_url),
    };
    json!({ "type": TYPE_SECTION, "components": children, "accessory": accessory })
}

/// Paragraflar korunarak kısaltılır; kısaldıysa sonuna Letterboxd'a yönlendiren not eklenir.
fn bounded(body: &str, limit: usize) -> String {
    let normalized = body.replace("\r\n", "\n");
    let mut lines: Vec<&str> = Vec::new();
    let mut blank = false;
    for line in normalized.lines().map(str::trim_end) {
        if line.trim().is_empty() {
            if !blank && !lines.is_empty() {
                lines.push("");
            }
            blank = true;
        } else {
            lines.push(line);
            blank = false;
        }
    }
    let text = lines.join("\n").trim().to_string();
    if text.chars().count() > limit {
        format!("{} *(devamı Letterboxd'da)*", truncate_chars(&text, limit))
    } else {
        text
    }
}

/// Engellenen incelemenin film kartına eklenen notu.
struct ReviewLink {
    user: String,
    url: String,
}

fn film_card(film: &Film, review: Option<&ReviewLink>) -> CustomCard {
    let mut header = format!("### [{}]({})", escape_markdown(&film.title), film.url);
    if let Some(year) = &film.year {
        header.push_str(&format!(" ({year})"));
    }
    let mut byline: Vec<String> = Vec::new();
    if let Some(original) = &film.original_title {
        byline.push(format!("*{}*", escape_markdown(original)));
    }
    if !film.directors.is_empty() {
        byline.push(format!(
            "Yönetmen: {}",
            escape_markdown(&film.directors.join(", "))
        ));
    }
    if !byline.is_empty() {
        header.push_str(&format!("\n-# {}", byline.join(" · ")));
    }

    let mut section_children = vec![text(header)];
    if let Some(tagline) = &film.tagline {
        section_children.push(text(format!("*“{}”*", escape_markdown(tagline))));
    }
    if let Some(rating) = film.rating {
        let mut line = format!("{} **{rating:.2}**/5", stars(rating));
        if let Some(count) = film.rating_count {
            line.push_str(&format!(" · {} oy", format_count(count)));
        }
        section_children.push(text(line));
    }

    let mut components = Vec::new();
    if let Some(review) = review {
        components.push(text(format!(
            "-# 📝 [{}]({}) adlı kullanıcının incelemesi · Letterboxd bu sayfayı botlara kapattığı için film gösteriliyor",
            escape_markdown(&review.user),
            review.url
        )));
    }
    components.push(section(
        section_children,
        film.poster.as_deref(),
        &film.url,
        &format!("{} posteri", film.title),
    ));
    if let Some(synopsis) = &film.synopsis {
        components.push(text(escape_markdown(&bounded(synopsis, SYNOPSIS_LIMIT))));
    }

    let mut facts: Vec<String> = film
        .genres
        .iter()
        .take(MAX_GENRES)
        .map(|genre| escape_markdown(genre))
        .collect();
    if let Some(runtime) = film.runtime {
        facts.push(runtime_label(runtime));
    }
    if let Some(country) = film.countries.first() {
        facts.push(escape_markdown(country));
    }
    let mut details = Vec::new();
    if !facts.is_empty() {
        details.push(format!("-# {}", facts.join(" · ")));
    }
    if !film.cast.is_empty() {
        let mut cast = film
            .cast
            .iter()
            .take(MAX_CAST)
            .map(|name| escape_markdown(name))
            .collect::<Vec<_>>()
            .join(", ");
        if film.cast.len() > MAX_CAST {
            cast.push_str(" …");
        }
        details.push(format!("**Oyuncular:** {cast}"));
    }
    if !details.is_empty() {
        components.push(text(details.join("\n")));
    }

    let mut buttons = Vec::new();
    if let Some(review) = review {
        buttons.push(link_button("İncelemeyi aç", &review.url));
    }
    buttons.push(link_button("Letterboxd", &film.url));
    if let Some(trailer) = &film.trailer {
        buttons.push(link_button("Fragman", trailer));
    }
    if let Some(imdb) = &film.imdb {
        buttons.push(link_button("IMDb", imdb));
    }
    if let Some(tmdb) = &film.tmdb {
        buttons.push(link_button("TMDB", tmdb));
    }
    components.push(json!({ "type": TYPE_SEPARATOR, "spacing": 1 }));
    components.push(json!({ "type": TYPE_ACTION_ROW, "components": buttons }));

    CustomCard {
        components,
        accent_color: Some(LETTERBOXD_GREEN),
    }
}

fn review_card(review: &Review) -> CustomCard {
    let mut verdict = review.rating.map(stars).unwrap_or_default();
    if review.liked {
        verdict.push_str(if verdict.is_empty() { "♥" } else { " ♥" });
    }
    let mut film = format!(
        "[{}]({})",
        escape_markdown(&review.film_title),
        review.film_url
    );
    if let Some(year) = &review.film_year {
        film.push_str(&format!(" ({year})"));
    }
    let mut header = if verdict.is_empty() {
        format!("### {film}")
    } else {
        format!("### {verdict}\n{film}")
    };
    header.push_str(&format!(
        "\n-# [{}]({}) incelemesi",
        escape_markdown(&review.author),
        review.profile_url
    ));

    let mut meta: Vec<String> = Vec::new();
    match (review.watched, review.published) {
        (Some(date), _) => meta.push(format!("{} izledi", date_label(date))),
        (None, Some(date)) => meta.push(format!("{} yazdı", date_label(date))),
        (None, None) => {}
    }
    if review.rewatch {
        meta.push("🔁 Yeniden izleme".to_string());
    }
    if let Some(likes) = review.likes {
        meta.push(format!("{} beğeni", format_count(likes)));
    }
    if let Some(director) = &review.director {
        meta.push(format!("Yönetmen: {}", escape_markdown(director)));
    }

    let mut section_children = vec![text(header)];
    if !meta.is_empty() {
        section_children.push(text(format!("-# {}", meta.join(" · "))));
    }

    let mut components = vec![section(
        section_children,
        review.poster.as_deref(),
        &review.url,
        &format!("{} posteri", review.film_title),
    )];
    match review
        .body
        .as_deref()
        .map(|body| bounded(body, REVIEW_LIMIT))
    {
        Some(body) if !body.is_empty() => {
            let body = escape_markdown(&body);
            components.push(text(if review.spoiler {
                format!("⚠️ **Spoiler içerir**\n||{body}||")
            } else {
                body
            }));
        }
        _ => {}
    }
    components.push(json!({ "type": TYPE_SEPARATOR, "spacing": 1 }));
    components.push(json!({
        "type": TYPE_ACTION_ROW,
        "components": [
            link_button("İncelemeyi aç", &review.url),
            link_button("Filmi aç", &review.film_url),
        ],
    }));

    CustomCard {
        components,
        accent_color: Some(LETTERBOXD_ORANGE),
    }
}

fn film_post(film: &Film, review: Option<ReviewLink>) -> BuiltinPost {
    let url = review
        .as_ref()
        .map_or_else(|| film.url.clone(), |review| review.url.clone());
    let mut post = BuiltinPost::new(Platform::Letterboxd, url, Author::new("Letterboxd", ""));
    post.title = Some(match &film.year {
        Some(year) => format!("{} ({year})", film.title),
        None => film.title.clone(),
    });
    post.text = film.synopsis.clone().unwrap_or_default();
    post.extra_label = Some("Letterboxd".to_string());
    post.custom_card = Some(film_card(film, review.as_ref()));
    post
}

fn review_post(review: &Review) -> BuiltinPost {
    let mut author = Author::new(review.author.clone(), "");
    author.avatar = review.avatar.clone();
    author.profile_url = Some(review.profile_url.clone());
    let mut post = BuiltinPost::new(Platform::Letterboxd, review.url.clone(), author);
    post.title = Some(format!("{} incelemesi", review.film_title));
    post.text = review.body.clone().unwrap_or_default();
    post.extra_label = Some("Letterboxd".to_string());
    post.custom_card = Some(review_card(review));
    post
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Canlı film sayfasından kısaltılmış örnek (JSON-LD + başlık alanları).
    const FILM_HTML: &str = r#"<html><head>
        <meta property="og:title" content="Parasite (2019)">
        <script type="application/ld+json">
        /* <![CDATA[ */
        {"image":"https://a.ltrbxd.com/resized/film-poster/4/2/6/4/0/6/426406-parasite-0-600-0-900-crop.jpg?v=8f5653f710","@type":"Movie","director":[{"@type":"Person","name":"Bong Joon Ho"}],"description":"All unemployed, Ki-taek's family takes peculiar interest in the wealthy and glamorous Parks.","url":"https://letterboxd.com/film/parasite-2019/","duration":"PT2H13M","actor":[{"name":"Song Kang-ho"},{"name":"Lee Sun-kyun"},{"name":"Cho Yeo-jeong"},{"name":"Choi Woo-shik"},{"name":"Park So-dam"},{"name":"Lee Jung-eun"}],"name":"Parasite","genre":["Thriller","Comedy","Drama"],"countryOfOrigin":[{"@type":"Country","name":"South Korea"}],"aggregateRating":{"bestRating":5,"reviewCount":803421,"@type":"AggregateRating","ratingValue":4.51,"ratingCount":5965596,"worstRating":0.5}}
        /* ]]> */
        </script></head><body>
        <div class="topline"><h1 class="headline-1 primaryname">Parasite</h1><span class="releasedate"><a href="/films/year/2019/">2019</a></span></div>
        <h2 class="originalname" lang="ko"><em class="quoted-creative-work-title">기생충</em></h2>
        <h4 class="tagline">Act like you own the place.</h4>
        <p class="text-link text-footer"> 133&nbsp;mins &nbsp; More at
        <a href="http://www.imdb.com/title/tt6751668/maindetails" class="micro-button track-event" data-track-action="IMDb">IMDb</a>
        <a href="https://www.themoviedb.org/movie/496243/" class="micro-button track-event" data-track-action="TMDB">TMDB</a></p>
        <p class="trailer-link js-watch-panel-trailer"><a class="play track-event js-video-zoom" data-track-category="Trailer" href="//www.youtube-nocookie.com/embed/PhPROyE0OaM?rel=0&amp;wmode=transparent">Trailer</a></p>
        </body></html>"#;

    /// Canlı inceleme sayfasından kısaltılmış örnek (spoiler uyarılı, beğenili).
    const REVIEW_HTML: &str = r##"<html><head>
        <meta property="og:description" content="This review may contain spoilers. Visit the page to bypass this warning and read the review.">
        <script type="application/ld+json">
        /* <![CDATA[ */
        {"datePublished":"2019-08-10","@type":"Review","author":[{"@type":"Person","name":"hollie amanda","sameAs":"https://letterboxd.com/hollieamanda/"}],"reviewBody":"call me by your name’s peach walked\r\n\r\nso parasite’s peach could run","itemReviewed":{"image":"https://a.ltrbxd.com/resized/film-poster/4/2/6/4/0/6/426406-parasite-0-230-0-345-crop.jpg?v=8f5653f710","@type":"Movie","director":[{"@type":"Person","name":"Bong Joon Ho"}],"name":"Parasite","sameAs":"https://letterboxd.com/film/parasite-2019/"},"@context":"https://schema.org","reviewRating":{"bestRating":5,"@type":"Rating","ratingValue":4.5,"worstRating":0.5},"url":"https://letterboxd.com/hollieamanda/film/parasite-2019/"}
        /* ]]> */
        </script></head><body>
        <a class="avatar -a24" href="/hollieamanda/" > <img src="https://a.ltrbxd.com/resized/avatar/upload/1/8/3/shard/avtr-0-48-0-48-crop.jpg?v=6791db69be" alt="hollie amanda"/></a>
        <div class="topline"><h2 class="primaryname prettify"><a href="/film/parasite-2019/">Parasite</a></h2><span class="releasedate"><a href="/films/year/2019/">2019</a></span></div>
        <div class="content-reactions-strip -viewing -single"><span class="inline-rating"><svg aria-label="★★★★½"></svg></span>
        <span class="inline-symbol"><svg class="glyph inline-liked -like" aria-label="Liked"><title>Liked</title></svg></span></div>
        <div><p class="view-date"> Watched <a href="/hollieamanda/diary/for/2019/08/09/">09</a> <a href="/hollieamanda/diary/for/2019/08/">Aug</a></p></div>
        <div class="js-spoiler-container"><p>This review may contain spoilers. <a href="#">I can handle the truth.</a></p></div>
        <p class="like-link-target react-component review-like" data-component-class="LikeComponent" data-format="count" data-count="43627" data-owner="hollieamanda"></p>
        <a class="avatar -a40" href="/baskasi/" > <img src="https://a.ltrbxd.com/resized/avatar/upload/9/shard/avtr-0-80-0-80-crop.jpg?v=1"/></a>
        <svg aria-label="Liked"></svg>
        </body></html>"##;

    fn review_target() -> Target {
        Target::Review {
            user: "hollieamanda".into(),
            slug: "parasite-2019".into(),
            entry: None,
        }
    }

    fn contents(card: &CustomCard) -> String {
        fn walk(value: &Value, out: &mut String) {
            match value {
                Value::Object(map) => {
                    if let Some(content) = map.get("content").and_then(Value::as_str) {
                        out.push_str(content);
                        out.push('\n');
                    }
                    map.values().for_each(|child| walk(child, out));
                }
                Value::Array(items) => items.iter().for_each(|child| walk(child, out)),
                _ => {}
            }
        }
        let mut out = String::new();
        card.components
            .iter()
            .for_each(|component| walk(component, &mut out));
        out
    }

    fn buttons(card: &CustomCard) -> Vec<(String, String)> {
        card.components
            .iter()
            .filter(|component| component["type"] == TYPE_ACTION_ROW)
            .flat_map(|row| row["components"].as_array().cloned().unwrap_or_default())
            .map(|button| {
                (
                    button["label"].as_str().unwrap_or_default().to_string(),
                    button["url"].as_str().unwrap_or_default().to_string(),
                )
            })
            .collect()
    }

    #[test]
    fn targets_are_read_from_canonical_urls() {
        assert_eq!(
            Target::from_url("https://letterboxd.com/film/parasite-2019/"),
            Some(Target::Film {
                slug: "parasite-2019".into()
            })
        );
        let rewatch = Target::from_url("https://letterboxd.com/biri_1/film/heat-1995/2/").unwrap();
        assert_eq!(
            rewatch,
            Target::Review {
                user: "biri_1".into(),
                slug: "heat-1995".into(),
                entry: Some(2)
            }
        );
        assert_eq!(
            rewatch.url(),
            "https://letterboxd.com/biri_1/film/heat-1995/2/"
        );
        assert_eq!(rewatch.film_url(), "https://letterboxd.com/film/heat-1995/");
        assert_eq!(Target::from_url("https://boxd.it/2bg8"), None);
        assert_eq!(Target::from_url("https://evil.test/film/x/"), None);
    }

    #[test]
    fn film_page_is_parsed() {
        let film = parse_film(FILM_HTML, "https://letterboxd.com/film/parasite-2019/").unwrap();
        assert_eq!(film.title, "Parasite");
        assert_eq!(film.year.as_deref(), Some("2019"));
        assert_eq!(film.original_title.as_deref(), Some("기생충"));
        assert_eq!(film.tagline.as_deref(), Some("Act like you own the place."));
        assert_eq!(film.directors, vec!["Bong Joon Ho"]);
        assert_eq!(film.cast.len(), 6);
        assert_eq!(film.genres, vec!["Thriller", "Comedy", "Drama"]);
        assert_eq!(film.countries, vec!["South Korea"]);
        assert_eq!(film.runtime, Some(133));
        assert_eq!(film.rating, Some(4.51));
        assert_eq!(film.rating_count, Some(5_965_596));
        assert_eq!(
            film.imdb.as_deref(),
            Some("https://www.imdb.com/title/tt6751668/")
        );
        assert_eq!(
            film.tmdb.as_deref(),
            Some("https://www.themoviedb.org/movie/496243/")
        );
        assert_eq!(
            film.trailer.as_deref(),
            Some("https://www.youtube.com/watch?v=PhPROyE0OaM")
        );
        assert!(
            film.poster
                .as_deref()
                .unwrap()
                .contains("-0-600-0-900-crop")
        );
        assert!(parse_film("<html></html>", "https://letterboxd.com/film/x/").is_err());
    }

    #[test]
    fn film_card_shows_rating_poster_synopsis_and_links() {
        let film = parse_film(FILM_HTML, "https://letterboxd.com/film/parasite-2019/").unwrap();
        let card = film_card(&film, None);
        assert_eq!(card.accent_color, Some(LETTERBOXD_GREEN));
        assert_eq!(card.components[0]["type"], TYPE_SECTION);
        assert_eq!(card.components[0]["accessory"]["type"], TYPE_THUMBNAIL);
        let text = contents(&card);
        assert!(
            text.contains("### [Parasite](https://letterboxd.com/film/parasite-2019/) (2019)"),
            "{text}"
        );
        assert!(text.contains("*기생충* · Yönetmen: Bong Joon Ho"), "{text}");
        assert!(text.contains("★★★★½ **4.51**/5 · 6M oy"), "{text}");
        assert!(text.contains("Act like you own the place."), "{text}");
        assert!(
            text.contains("Thriller · Comedy · Drama · 2 sa 13 dk · South Korea"),
            "{text}"
        );
        assert!(
            text.contains(
                "Song Kang-ho, Lee Sun-kyun, Cho Yeo-jeong, Choi Woo-shik, Park So-dam …"
            ),
            "{text}"
        );
        let labels: Vec<String> = buttons(&card).into_iter().map(|(label, _)| label).collect();
        assert_eq!(labels, vec!["Letterboxd", "Fragman", "IMDb", "TMDB"]);
    }

    #[test]
    fn review_page_is_parsed_without_other_users_reactions() {
        let review = parse_review(REVIEW_HTML, &review_target()).unwrap();
        assert_eq!(review.author, "hollie amanda");
        assert_eq!(review.rating, Some(4.5));
        assert!(review.spoiler);
        assert!(review.liked);
        assert!(!review.rewatch);
        assert_eq!(review.watched, Some((2019, 8, 9)));
        assert_eq!(review.published, Some((2019, 8, 10)));
        assert_eq!(review.likes, Some(43_627));
        assert_eq!(review.film_title, "Parasite");
        assert_eq!(review.film_year.as_deref(), Some("2019"));
        assert_eq!(review.director.as_deref(), Some("Bong Joon Ho"));
        assert!(
            review
                .poster
                .as_deref()
                .unwrap()
                .contains("-0-600-0-900-crop"),
            "küçük poster büyütülür"
        );
        assert!(
            review.avatar.as_deref().unwrap().contains("avtr-0-48-0-48"),
            "başkasının avatarı alınmaz"
        );
        assert!(parse_review("<html></html>", &review_target()).is_none());
    }

    #[test]
    fn review_card_hides_spoilers_and_keeps_paragraphs() {
        let review = parse_review(REVIEW_HTML, &review_target()).unwrap();
        let card = review_card(&review);
        assert_eq!(card.accent_color, Some(LETTERBOXD_ORANGE));
        let text = contents(&card);
        assert!(text.contains("### ★★★★½ ♥"), "{text}");
        assert!(
            text.contains("[hollie amanda](https://letterboxd.com/hollieamanda/) incelemesi"),
            "{text}"
        );
        assert!(
            text.contains("9 Ağu 2019 izledi · 43.6K beğeni · Yönetmen: Bong Joon Ho"),
            "{text}"
        );
        assert!(
            text.contains("⚠️ **Spoiler içerir**\n||call me by your name’s peach walked\n\nso parasite’s peach could run||"),
            "{text}"
        );
        let urls: Vec<String> = buttons(&card).into_iter().map(|(_, url)| url).collect();
        assert_eq!(
            urls,
            vec![
                "https://letterboxd.com/hollieamanda/film/parasite-2019/",
                "https://letterboxd.com/film/parasite-2019/"
            ]
        );

        let mut plain = review.clone();
        plain.spoiler = false;
        assert!(!contents(&review_card(&plain)).contains("||"));
    }

    #[test]
    fn blocked_reviews_fall_back_to_the_film_with_an_attribution() {
        let film = parse_film(FILM_HTML, "https://letterboxd.com/film/parasite-2019/").unwrap();
        let link = ReviewLink {
            user: "biri".into(),
            url: "https://letterboxd.com/biri/film/parasite-2019/2/".into(),
        };
        let post = film_post(&film, Some(link));
        assert_eq!(
            post.url,
            "https://letterboxd.com/biri/film/parasite-2019/2/"
        );
        let card = post.custom_card.unwrap();
        assert!(contents(&card).contains(
            "[biri](https://letterboxd.com/biri/film/parasite-2019/2/) adlı kullanıcının incelemesi"
        ));
        assert_eq!(buttons(&card)[0].0, "İncelemeyi aç");
    }

    #[test]
    fn helpers_format_like_letterboxd() {
        assert_eq!(stars(4.51), "★★★★½");
        assert_eq!(stars(3.0), "★★★");
        assert_eq!(stars(0.5), "½");
        assert_eq!(stars(4.2), "★★★★");
        assert_eq!(iso_minutes("PT2H13M"), Some(133));
        assert_eq!(iso_minutes("PT95M"), Some(95));
        assert_eq!(iso_minutes("PT2H"), Some(120));
        assert_eq!(iso_minutes("2 hours"), None);
        assert_eq!(runtime_label(133), "2 sa 13 dk");
        assert_eq!(runtime_label(45), "45 dk");
        assert_eq!(runtime_label(120), "2 sa");
        assert_eq!(date_label((2019, 8, 9)), "9 Ağu 2019");
        assert_eq!(bounded("a\r\n\r\n\r\nb", 100), "a\n\nb");
        assert!(bounded(&"x".repeat(50), 10).ends_with("*(devamı Letterboxd'da)*"));
    }

    #[test]
    fn untrusted_text_cannot_break_out_of_the_card_markdown() {
        let mut review = parse_review(REVIEW_HTML, &review_target()).unwrap();
        review.spoiler = true;
        review.body = Some("son || açık [link](https://evil.test)".into());
        review.film_title = "Film](https://evil.test)".into();
        let text = contents(&review_card(&review));
        assert!(
            !text.contains("son ||"),
            "spoiler erken kapanmamalı: {text}"
        );
        assert!(!text.contains("[link](https://evil.test)"), "{text}");
        assert!(!text.contains("[Film](https://evil.test)"), "{text}");
    }

    /// Canlı kaynak testi: Letterboxd'un herkese açık sayfaları gerçek veri döndürmeli.
    #[tokio::test]
    #[ignore = "requires live Letterboxd access; run explicitly"]
    async fn live_film_review_and_short_link() {
        let client = reqwest::Client::builder()
            .tls_backend_rustls()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .unwrap();
        let parsed = |url: &str| ParsedPost {
            platform: Platform::Letterboxd,
            is_spoiler: false,
            user_comment: None,
            original_url: url.to_string(),
            fixed_url: url.to_string(),
        };
        let film = fetch(
            &client,
            &parsed("https://letterboxd.com/film/parasite-2019/"),
        )
        .await
        .expect("film kartı üretilmeli");
        assert_eq!(film.title.as_deref(), Some("Parasite (2019)"));

        let review = fetch(
            &client,
            &parsed("https://letterboxd.com/hollieamanda/film/parasite-2019/"),
        )
        .await
        .expect("inceleme kartı üretilmeli");
        assert_eq!(review.author.name, "hollie amanda");

        let short = fetch(&client, &parsed("https://boxd.it/2bg8"))
            .await
            .expect("kısa link çözülmeli");
        assert!(short.url.starts_with("https://letterboxd.com/film/"));

        let blocked = fetch(
            &client,
            &parsed("https://letterboxd.com/demiadejuyigbe/film/parasite-2019/2/"),
        )
        .await
        .expect("engelli inceleme film kartına düşmeli");
        assert!(
            contents(blocked.custom_card.as_ref().unwrap())
                .contains("adlı kullanıcının incelemesi")
        );
    }
}
