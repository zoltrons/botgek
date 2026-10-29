//! Letterboxd film, kullanıcı incelemesi ve kullanıcı profili kartları (native, Components V2).
//!
//! Veri giriş/API anahtarı gerektirmeyen herkese açık sayfalardan okunur:
//! - **Film sayfası** (`/film/{slug}/`): JSON-LD `Movie` nesnesi başlık, poster, yönetmen,
//!   oyuncular, türler, ülke, süre, açıklama ve Letterboxd ortalamasını (`aggregateRating`)
//!   taşır; HTML'den slogan, orijinal ad, yıl, fragman ve IMDb/TMDB bağlantıları alınır.
//! - **İnceleme sayfası** (`/{kullanıcı}/film/{slug}/`): JSON-LD `Review` nesnesi yazarı,
//!   puanı, metni (paragraflarıyla) ve tarihi taşır; HTML'den beğeni sayısı, kalp, yeniden
//!   izleme, izleme tarihi ve spoiler uyarısı okunur.
//! - **Profil sayfası** (`/{kullanıcı}/`): JSON-LD yoktur. HTML'den görünen ad, avatar, rozet
//!   (Pro/Patron/HQ), bio, istatistikler (film, bu yıl, liste, takip, takipçi), konum,
//!   favori filmler ve son etkinlik okunur.
//!
//! Cloudflare, Letterboxd'un profil ve numaralı inceleme sayfalarını sıradan User-Agent'lara
//! 403 ile kapatır. Discord'un bağlantı botu tanıtıcısı bu kuraldan muaftır; bot da çıktısını
//! Discord kartı olarak ürettiği için aynı tanıtıcıyı kullanır ve sonuna kendi adını ekler
//! (bkz. [`LETTERBOXD_USER_AGENT`]). Sayfa yine okunamazsa inceleme kartı, incelemenin kime
//! ait olduğunu bağlantısıyla belirten film kartına düşer. `boxd.it` kısa linkleri hedefe
//! çözülür; yanıt engellenmiş olsa bile varılan adres hedefi belirler.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Value, json};
use tracing::debug;

use super::render::{escape_markdown, format_count};
use super::{Author, BuiltinPost, CustomCard, REQUEST_TIMEOUT, USER_AGENT, unescape_html};
use crate::converters::parser::{ParsedPost, Platform, letterboxd_profile_user};
use crate::utils::truncate_chars;

const LETTERBOXD: &str = "https://letterboxd.com";
/// Letterboxd yeşili (logodaki üç noktadan): film kartı.
const LETTERBOXD_GREEN: u32 = 0x00E054;
/// Letterboxd turuncusu: kullanıcı incelemesi kartı.
const LETTERBOXD_ORANGE: u32 = 0xFF8000;
/// Letterboxd mavisi (logodaki üç noktadan): kullanıcı profili kartı.
const LETTERBOXD_BLUE: u32 = 0x40BCF4;

/// Botun kendi (application) emojisi: her sunucuda görünür. Kart gönderimden bağımsız
/// önbelleğe alınan kaynak veri olduğu için ikonlar ayar değil sabittir (Spotify logoları gibi).
#[derive(Debug, Clone, Copy)]
struct Emoji {
    name: &'static str,
    id: &'static str,
}

impl Emoji {
    /// Metin içindeki biçim: `<:ad:id>`.
    fn tag(self) -> String {
        format!("<:{}:{}>", self.name, self.id)
    }

    /// Buton `emoji` alanı.
    fn json(self) -> Value {
        json!({ "name": self.name, "id": self.id })
    }
}

// Letterboxd'un kendi sprite'ından üretilen ikonlar.
const EMOJI_LOGO: Emoji = Emoji {
    name: "letterboxd",
    id: "1563865506841362505",
};
const EMOJI_STAR: Emoji = Emoji {
    name: "lb_star",
    id: "1563865502881808486",
};
const EMOJI_HALF: Emoji = Emoji {
    name: "lb_half",
    id: "1563865487144914995",
};
const EMOJI_LIKE: Emoji = Emoji {
    name: "lb_like",
    id: "1563865488831029358",
};
const EMOJI_REWATCH: Emoji = Emoji {
    name: "lb_rewatch",
    id: "1563865499153072229",
};
const EMOJI_REVIEW: Emoji = Emoji {
    name: "lb_review",
    id: "1563865497492131860",
};
const EMOJI_SPOILER: Emoji = Emoji {
    name: "lb_spoiler",
    id: "1563865500931330179",
};
const EMOJI_WATCHED: Emoji = Emoji {
    name: "lb_watched",
    id: "1563865505725685790",
};
const EMOJI_LIKES: Emoji = Emoji {
    name: "lb_likes",
    id: "1563865490441506826",
};
const EMOJI_PLAY: Emoji = Emoji {
    name: "lb_play",
    id: "1563865496019927101",
};
const EMOJI_FILMS: Emoji = Emoji {
    name: "lb_films",
    id: "1563865484900962304",
};
const EMOJI_THIS_YEAR: Emoji = Emoji {
    name: "lb_thisyear",
    id: "1563865504420986910",
};
const EMOJI_LISTS: Emoji = Emoji {
    name: "lb_lists",
    id: "1563865491725099060",
};
const EMOJI_PEOPLE: Emoji = Emoji {
    name: "lb_people",
    id: "1563865494522437662",
};
const EMOJI_PATRON: Emoji = Emoji {
    name: "lb_patron",
    id: "1563865493029257216",
};

/// Cloudflare, profil ve numaralı inceleme sayfalarını sıradan User-Agent'lara 403 ile kapatır.
/// Discord'un bağlantı botu tanıtıcısı bu kapıdan geçer; sonuna botun kendi adı eklenir ki
/// istekler tamamen anonim görünmesin ve sunucu tarafında ayırt edilebilsin.
static LETTERBOXD_USER_AGENT: LazyLock<String> = LazyLock::new(|| {
    format!("Mozilla/5.0 (compatible; Discordbot/2.0; +https://discordapp.com) {USER_AGENT}")
});

/// Kartta gösterilen açıklama ve inceleme metni sınırları.
const SYNOPSIS_LIMIT: usize = 600;
const REVIEW_LIMIT: usize = 1_500;
/// Profil bio'su ve listeleri için kart sınırları.
const BIO_LIMIT: usize = 800;
const MAX_CAST: usize = 5;
const MAX_GENRES: usize = 4;
const MAX_FAVORITES: usize = 4;
const MAX_RECENT: usize = 4;
const MAX_PROFILE_LINKS: usize = 2;

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
/// Poster bileşeninin çözülecek poster bilgisi (JSON, HTML varlıklarıyla kaçışlı). Sahibi
/// özel poster seçtiyse `preferredAlternativePosterId` taşır.
static RESOLVABLE_POSTER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"data-resolvable-poster-path='([^']{1,2000})'")
        .expect("geçerli çözülebilir poster regex")
});
static AVATAR_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<a class="avatar[^"]*" href="/([A-Za-z0-9_]+)/"\s*>\s*<img src="(https://a\.ltrbxd\.com/[^"]+)""#)
        .expect("geçerli avatar regex")
});

// ── Profil sayfası ─────────────────────────────────────────────────────────────
static PROFILE_HEADER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<(?:div|section) class="profile-header[^"]*"[^>]*>"#)
        .expect("geçerli profil başlığı regex")
});
static PROFILE_H1_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<h1 class="person-display-name[^"]*"[^>]*>(.*?)</h1>"#)
        .expect("geçerli profil adı regex")
});
static PROFILE_LABEL_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<span class="label">(.*?)</span>"#).expect("geçerli profil etiketi regex")
});
static PROFILE_BADGE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<span class="badge[^"]*">(.*?)</span>"#).expect("geçerli rozet regex")
});
static PROFILE_AVATAR_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<div class="profile-avatar">.*?<img src="([^"]+)""#)
        .expect("geçerli profil avatarı regex")
});
static PROFILE_BIO_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<div class="[^"]*js-bio-content[^"]*"[^>]*>(.*?)</div>"#)
        .expect("geçerli profil bio regex")
});
static PROFILE_STAT_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?s)<h4 class="profile-statistic[^"]*"[^>]*>.*?<span class="value">([^<]+)</span><span[^>]*>([^<]+)</span>"#,
    )
    .expect("geçerli profil istatistiği regex")
});
static PROFILE_METADATUM_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<(?:div|a) class="metadatum[^"]*"[^>]*>(.*?)</(?:div|a)>"#)
        .expect("geçerli profil meta regex")
});
static PROFILE_HREF_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<a [^>]*href="(https?://[^"]+)""#).expect("geçerli profil bağlantısı regex")
});
static PROFILE_ITEM_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)data-item-name="([^"]+)"[^>]*?data-item-link="([^"]+)""#)
        .expect("geçerli profil öğesi regex")
});
static PROFILE_TARGET_LINK_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"data-target-link="([^"]+)""#).expect("geçerli profil etkinlik bağlantısı regex")
});
static PROFILE_RATED_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"class="rating rated-([0-9]{1,2})""#).expect("geçerli profil puanı regex")
});
/// Bio HTML'ini paragraflara çevirirken blok ve satır kıran etiketler.
static PROFILE_PARAGRAPH_BREAK_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)</p\s*>|</div\s*>").expect("geçerli paragraf kırma regex"));
static PROFILE_BR_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<br\s*/?>").expect("geçerli satır kırma regex"));
static OG_IMAGE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)<meta\s+property="og:image"\s+content="([^"]+)""#)
        .expect("geçerli og:image regex")
});

/// Kart hedefi: film, kullanıcı incelemesi (yeniden izlemede kayıt numarasıyla), profil.
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
    Profile {
        user: String,
    },
}

impl Target {
    fn from_url(url: &str) -> Option<Self> {
        // Profil yolu tek parçadır; `letterboxd_profile_user` film/inceleme alt yollarını,
        // sabit sayfaları (`/films/`, `/about/` …) ve geçersiz adları eler.
        if let Some(user) = letterboxd_profile_user(url) {
            return Some(Self::Profile { user });
        }
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

    fn slug(&self) -> Option<&str> {
        match self {
            Self::Film { slug } | Self::Review { slug, .. } => Some(slug),
            Self::Profile { .. } => None,
        }
    }

    /// Filmin sayfası; profil hedefinde film yoktur.
    fn film_url(&self) -> Option<String> {
        self.slug().map(film_page_url)
    }

    fn url(&self) -> String {
        match self {
            Self::Film { slug } => film_page_url(slug),
            Self::Review { user, slug, entry } => {
                let entry = entry.map(|n| format!("{n}/")).unwrap_or_default();
                format!("{LETTERBOXD}/{user}/film/{slug}/{entry}")
            }
            Self::Profile { user } => format!("{LETTERBOXD}/{user}/"),
        }
    }
}

fn film_page_url(slug: &str) -> String {
    format!("{LETTERBOXD}/film/{slug}/")
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
        Target::Film { slug } => {
            let film_url = film_page_url(slug);
            let html = page
                .body
                .as_deref()
                .ok_or_else(|| format!("Letterboxd film sayfası alınamadı: {film_url}"))?;
            let film = parse_film(html, &film_url)?;
            Ok(film_post(&film, None))
        }
        Target::Review { user, slug, .. } => {
            if let Some(html) = page.body.as_deref()
                && let Some(mut review) = parse_review(html, &target)
            {
                // İnceleme sahibinin seçtiği özel poster (Patron/Pro): varsayılanın yerine geçer.
                if let Some(path) = custom_poster_path(html, slug)
                    && let Some(poster) = resolve_poster(client, &path).await
                {
                    review.poster = Some(poster);
                }
                return Ok(review_post(&review));
            }
            // İnceleme sayfası engellendi ya da okunamadı: film kartı + inceleme notu.
            debug!(
                "Letterboxd inceleme sayfası okunamadı, film kartına düşülüyor: {}",
                target.url()
            );
            let film_url = film_page_url(slug);
            let film_page = get_page(client, &film_url).await?;
            let html = film_page
                .body
                .ok_or_else(|| format!("Letterboxd film sayfası alınamadı: {film_url}"))?;
            let film = parse_film(&html, &film_url)?;
            Ok(film_post(
                &film,
                Some(ReviewLink {
                    user: user.clone(),
                    url: target.url(),
                }),
            ))
        }
        Target::Profile { .. } => {
            let html = page.body.as_deref().ok_or_else(|| {
                format!(
                    "Letterboxd profil sayfası alınamadı (Cloudflare engeli olabilir): {}",
                    target.url()
                )
            })?;
            let profile = parse_profile(html, &target)?;
            Ok(profile_post(&profile))
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
///
/// User-Agent bilinçli olarak Discord'un bağlantı botu tanıtıcısıdır: Cloudflare, profil ve
/// numaralı inceleme sayfalarında sıradan tanıtıcıları 403 ile durduruyor.
async fn get_page(client: &reqwest::Client, url: &str) -> Result<Page, String> {
    let response = client
        .get(url)
        .header("user-agent", LETTERBOXD_USER_AGENT.as_str())
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

/// Letterboxd'un poster çözücüsü (`…/poster/std/{id}/600/`) özel posterin CDN adresini döner;
/// sayfa bunu tarayıcıda çözer. Hata olursa `None`: kart varsayılan posterle kalır.
async fn resolve_poster(client: &reqwest::Client, path: &str) -> Option<String> {
    let url = format!("{LETTERBOXD}{path}");
    let body = match get_page(client, &url).await {
        Ok(page) => page.body?,
        Err(error) => {
            debug!("Letterboxd özel posteri çözülemedi: {error}");
            return None;
        }
    };
    let value: Value = serde_json::from_str(&body).ok()?;
    text_of(&value, "url").filter(|url| url.starts_with("https://a.ltrbxd.com/"))
}

// ─── Ayrıştırma ──────────────────────────────────────────────────────────────

/// İncelenen filmin özel posteri için çözücü yolu. Sayfadaki diğer posterler (kullanıcının
/// başka filmleri) atlanır: yalnızca `posteredBaseLink` bu filme çıkan kayıt sayılır.
fn custom_poster_path(html: &str, slug: &str) -> Option<String> {
    let base = format!("/film/{slug}/");
    RESOLVABLE_POSTER_REGEX
        .captures_iter(html)
        .find_map(|caps| {
            let value: Value = serde_json::from_str(&unescape_html(&caps[1])).ok()?;
            if value.get("posteredBaseLink").and_then(Value::as_str) != Some(base.as_str()) {
                return None;
            }
            let id = match value.get("preferredAlternativePosterId")? {
                Value::String(id) => id.clone(),
                Value::Number(id) => id.to_string(),
                _ => return None,
            };
            if id.is_empty() || id.len() > 12 || !id.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let mut path = format!("{base}poster/std/{id}/600/");
            if let Some(key) = value
                .get("cacheBustingKey")
                .and_then(Value::as_str)
                .filter(|key| key.len() <= 16 && key.bytes().all(|b| b.is_ascii_alphanumeric()))
            {
                path.push_str(&format!("?k={key}"));
            }
            Some(path)
        })
}

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
        film_url: target.film_url()?,
        poster: https_image(text_of(item, "image").map(|url| large_poster(&url))),
        director: names_of(item, "director").into_iter().next(),
    })
}

// ─── Profil ayrıştırma ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ProfileStats {
    films: Option<u64>,
    this_year: Option<u64>,
    lists: Option<u64>,
    following: Option<u64>,
    followers: Option<u64>,
}

impl ProfileStats {
    /// Düz metin özet (gönderi metni yedeği).
    fn line(&self) -> Option<String> {
        self.render(false)
    }

    /// Kart satırı: her sayının önünde Letterboxd'un profil sekmesi ikonu.
    fn card_line(&self) -> Option<String> {
        self.render(true)
    }

    fn render(&self, icons: bool) -> Option<String> {
        let mut parts = Vec::new();
        let mut push = |count: Option<u64>, icon: Emoji, label: &str| {
            if let Some(count) = count {
                let count = format_profile_count(count);
                parts.push(if icons {
                    format!("{} **{count}** {label}", icon.tag())
                } else {
                    format!("**{count}** {label}")
                });
            }
        };
        push(self.films, EMOJI_FILMS, "film");
        push(self.this_year, EMOJI_THIS_YEAR, "bu yıl");
        push(self.lists, EMOJI_LISTS, "liste");
        push(self.following, EMOJI_PEOPLE, "takip");
        push(self.followers, EMOJI_PEOPLE, "takipçi");
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProfileLink {
    label: String,
    url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProfileFilm {
    title: String,
    url: String,
}

#[derive(Debug, Clone, PartialEq)]
struct ProfileActivity {
    title: String,
    url: String,
    rating: Option<f64>,
    liked: bool,
    rewatch: bool,
    review: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct Profile {
    url: String,
    user: String,
    name: String,
    badge: Option<String>,
    avatar: Option<String>,
    bio: Option<String>,
    location: Option<String>,
    links: Vec<ProfileLink>,
    stats: ProfileStats,
    favorites: Vec<ProfileFilm>,
    recent: Vec<ProfileActivity>,
}

/// Profil sayfasını HTML'den okur. Sayfa gerçekten bir profil değilse (404/engel/başka
/// sayfa) `Err` döner; kart üretilmez.
fn parse_profile(html: &str, target: &Target) -> Result<Profile, String> {
    let Target::Profile { user } = target else {
        return Err(format!(
            "Letterboxd profil hedefi bekleniyordu: {}",
            target.url()
        ));
    };
    let url = target.url();
    if !PROFILE_HEADER_REGEX.is_match(html) {
        return Err(format!("Letterboxd profil verisi bulunamadı: {url}"));
    }

    let h1 = PROFILE_H1_REGEX
        .captures(html)
        .and_then(|caps| caps.get(1))
        .map(|inner| inner.as_str())
        .unwrap_or_default();
    let name = first_capture(&PROFILE_LABEL_REGEX, h1).unwrap_or_else(|| user.clone());
    let badge = first_capture(&PROFILE_BADGE_REGEX, h1);
    let avatar = first_capture(&PROFILE_AVATAR_REGEX, html)
        .or_else(|| first_capture(&OG_IMAGE_REGEX, html))
        .map(|url| large_avatar(&url))
        .filter(|url| url.starts_with("https://"));
    let bio = PROFILE_BIO_REGEX
        .captures(html)
        .and_then(|caps| caps.get(1))
        .map(|inner| html_to_text(inner.as_str()))
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty());

    let mut stats = ProfileStats::default();
    for caps in PROFILE_STAT_REGEX.captures_iter(html) {
        let Some(value) = parse_stat_count(&caps[1]) else {
            continue;
        };
        match caps[2].trim().to_ascii_lowercase().as_str() {
            "films" => stats.films = Some(value),
            "this year" => stats.this_year = Some(value),
            "lists" => stats.lists = Some(value),
            "following" => stats.following = Some(value),
            "followers" => stats.followers = Some(value),
            _ => {}
        }
    }

    let (location, links) = profile_metadata(html);
    let favorites = profile_favorites(html);
    let recent = profile_recent(html);

    Ok(Profile {
        url,
        user: user.clone(),
        name,
        badge,
        avatar,
        bio,
        location,
        links,
        stats,
        favorites,
        recent,
    })
}

/// Profil başlığındaki konum ve harici bağlantılar (`profile-metadata`).
fn profile_metadata(html: &str) -> (Option<String>, Vec<ProfileLink>) {
    let mut location = None;
    let mut links = Vec::new();
    for caps in PROFILE_METADATUM_REGEX.captures_iter(html) {
        let whole = caps.get(0).map(|m| m.as_str()).unwrap_or_default();
        // `-has-no-label` yalnızca ikon taşır; etiketi olmayan bağlantı kartta anlamsız görünür.
        if whole.contains("-has-no-label") {
            continue;
        }
        let inner = caps.get(1).map(|m| m.as_str()).unwrap_or_default();
        let Some(label) = first_capture(&PROFILE_LABEL_REGEX, inner) else {
            continue;
        };
        let href = PROFILE_HREF_REGEX
            .captures(whole)
            .and_then(|caps| caps.get(1))
            .and_then(|url| safe_external_link(url.as_str()));
        match href {
            Some(url) => {
                if links.len() < MAX_PROFILE_LINKS {
                    links.push(ProfileLink { label, url });
                }
            }
            None => {
                if location.is_none() {
                    location = Some(label);
                }
            }
        }
    }
    (location, links)
}

/// Yalnızca `https` harici bağlantı kabul edilir; Markdown bağlantı hedefini bozabilecek
/// karakterler (`(`, `)`, boşluk) elenir.
fn safe_external_link(raw: &str) -> Option<String> {
    let url = url::Url::parse(raw).ok()?;
    if url.scheme() != "https" || url.host_str().is_none() {
        return None;
    }
    let text = url.to_string();
    (!text
        .chars()
        .any(|ch| ch.is_whitespace() || matches!(ch, '(' | ')' | '<' | '>' | '|')))
    .then_some(text)
}

/// Profil sayfasının `id`'li bölümünü keser (favoriler, son etkinlik).
fn profile_section<'a>(html: &'a str, id: &str) -> Option<&'a str> {
    let marker = format!("<section id=\"{id}\"");
    let start = html.find(&marker)?;
    let end = html[start..].find("</section>")?;
    Some(&html[start..start + end])
}

/// Letterboxd gövde HTML'ini düz metne çevirir: paragraflar korunur, etiketler ve varlıklar
/// çözülür.
fn html_to_text(raw: &str) -> String {
    let blocks = PROFILE_PARAGRAPH_BREAK_REGEX.replace_all(raw, "\n\n");
    let breaks = PROFILE_BR_REGEX.replace_all(&blocks, "\n");
    let stripped = TAG_REGEX.replace_all(&breaks, "");
    unescape_html(&stripped)
}

/// Profil sayaçlarını binlik ayırıcıyla yazar (`2.590`, `16.700`); sayaçların tam değeri
/// Letterboxd'da göründüğü gibi korunur.
fn format_profile_count(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push('.');
        }
        out.push(digit);
    }
    out
}

/// `2,590`, `16.7K`, `1.2M` gibi profil sayaçlarını tam sayıya çevirir (HTML varlıkları çözülür).
fn parse_stat_count(raw: &str) -> Option<u64> {
    super::parse_compact_count(&unescape_html(raw))
}

/// Küçük avatar varyantını kartta keskin görünen 1000×1000 varyantına çevirir (aynı dosya).
fn large_avatar(url: &str) -> String {
    url.replace("-0-220-0-220-crop", "-0-1000-0-1000-crop")
        .replace("-0-110-0-110-crop", "-0-1000-0-1000-crop")
}

/// Letterboxd içi bir yolu (veya mutlak `letterboxd.com` adresini) güvenli bir adrese çevirir;
/// başka alan adlarına giden değerler kabul edilmez.
fn letterboxd_url(path: &str) -> Option<String> {
    let path = path.trim();
    let url = if path.starts_with("http://") || path.starts_with("https://") {
        url::Url::parse(path).ok()?
    } else {
        url::Url::parse(LETTERBOXD).ok()?.join(path).ok()?
    };
    (url.scheme() == "https" && url.host_str() == Some("letterboxd.com")).then(|| url.to_string())
}

/// Profildeki favori filmler (en fazla dört poster).
fn profile_favorites(html: &str) -> Vec<ProfileFilm> {
    let Some(section) = profile_section(html, "favourites") else {
        return Vec::new();
    };
    PROFILE_ITEM_REGEX
        .captures_iter(section)
        .filter_map(|caps| {
            let title = unescape_html(&caps[1]).trim().to_string();
            let url = letterboxd_url(&caps[2])?;
            (!title.is_empty()).then_some(ProfileFilm { title, url })
        })
        .take(MAX_FAVORITES)
        .collect()
}

/// Profildeki son etkinlik (film, puan, kalp, yeniden izleme, inceleme).
fn profile_recent(html: &str) -> Vec<ProfileActivity> {
    let Some(section) = profile_section(html, "recent-activity") else {
        return Vec::new();
    };
    section
        .split("<li class=\"griditem\">")
        .skip(1)
        .filter_map(|chunk| {
            let caps = PROFILE_ITEM_REGEX.captures(chunk)?;
            let title = unescape_html(&caps[1]).trim().to_string();
            let film_url = letterboxd_url(&caps[2])?;
            let url = PROFILE_TARGET_LINK_REGEX
                .captures(chunk)
                .and_then(|caps| letterboxd_url(&caps[1]))
                .unwrap_or_else(|| film_url.clone());
            let rating = PROFILE_RATED_REGEX
                .captures(chunk)
                .and_then(|caps| caps[1].parse::<u32>().ok())
                .filter(|score| (1..=10).contains(score))
                .map(|score| f64::from(score) / 2.0);
            (!title.is_empty()).then_some(ProfileActivity {
                title,
                url,
                rating,
                liked: chunk.contains(r#"class="rating liked""#),
                rewatch: chunk.contains("icon-rewatch"),
                review: chunk.contains("icon-review"),
            })
        })
        .take(MAX_RECENT)
        .collect()
}

// ─── Kartlar ─────────────────────────────────────────────────────────────────

/// Puanı Letterboxd'un yeşil yıldızlarıyla yazar: en yakın yarım yıldıza yuvarlanır
/// (`★★★★½` → dört yıldız ve `½` emojisi).
fn stars(rating: f64) -> String {
    let halves = (rating * 2.0).round().clamp(1.0, 10.0) as usize;
    let mut out = EMOJI_STAR.tag().repeat(halves / 2);
    if halves % 2 == 1 {
        out.push_str(&EMOJI_HALF.tag());
    }
    out
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

/// Bağlantı butonu; ikon verilirse etiketin solunda Letterboxd ikonu durur.
fn link_button(label: &str, url: &str, icon: Option<Emoji>) -> Value {
    let mut button =
        json!({ "type": TYPE_BUTTON, "style": STYLE_LINK, "label": label, "url": url });
    if let Some(icon) = icon {
        button["emoji"] = icon.json();
    }
    button
}

/// Posteri/avatarı taşıyan küçük görsel (Section aksesuarı).
fn thumbnail(url: &str, alt: &str) -> Value {
    json!({ "type": TYPE_THUMBNAIL, "media": { "url": url }, "description": alt })
}

/// Posterli bölüm; poster yoksa erişilebilirlik bir bağlantı butonu olur.
fn section(children: Vec<Value>, poster: Option<&str>, fallback_url: &str, alt: &str) -> Value {
    let accessory = match poster {
        Some(url) => thumbnail(url, alt),
        None => link_button("Letterboxd", fallback_url, Some(EMOJI_LOGO)),
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
            "-# {} [{}]({}) adlı kullanıcının incelemesi · Letterboxd bu sayfayı botlara kapattığı için yalnızca film kartı gösteriliyor",
            EMOJI_REVIEW.tag(),
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
        buttons.push(link_button(
            "İncelemeyi aç",
            &review.url,
            Some(EMOJI_REVIEW),
        ));
    }
    buttons.push(link_button("Letterboxd", &film.url, Some(EMOJI_LOGO)));
    if let Some(trailer) = &film.trailer {
        buttons.push(link_button("Fragman", trailer, Some(EMOJI_PLAY)));
    }
    if let Some(imdb) = &film.imdb {
        buttons.push(link_button("IMDb", imdb, None));
    }
    if let Some(tmdb) = &film.tmdb {
        buttons.push(link_button("TMDB", tmdb, None));
    }
    components.push(json!({ "type": TYPE_SEPARATOR, "spacing": 1 }));
    components.push(json!({ "type": TYPE_ACTION_ROW, "components": buttons }));

    CustomCard {
        components,
        accent_color: Some(LETTERBOXD_GREEN),
        video_components: None,
    }
}

fn review_card(review: &Review) -> CustomCard {
    let mut verdict = review.rating.map(stars).unwrap_or_default();
    if review.liked {
        if !verdict.is_empty() {
            verdict.push(' ');
        }
        verdict.push_str(&EMOJI_LIKE.tag());
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
        "\n-# İnceleyen: [{}]({})",
        escape_markdown(&review.author),
        review.profile_url
    ));

    let mut meta: Vec<String> = Vec::new();
    match (review.watched, review.published) {
        (Some(date), _) => meta.push(format!(
            "{} {} izledi",
            EMOJI_WATCHED.tag(),
            date_label(date)
        )),
        (None, Some(date)) => meta.push(format!("{} yazdı", date_label(date))),
        (None, None) => {}
    }
    if review.rewatch {
        meta.push(format!("{} Yeniden izleme", EMOJI_REWATCH.tag()));
    }
    if let Some(likes) = review.likes {
        meta.push(format!(
            "{} {} beğeni",
            EMOJI_LIKES.tag(),
            format_count(likes)
        ));
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
                format!("{} **Spoiler içerir**\n||{body}||", EMOJI_SPOILER.tag())
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
            link_button("İncelemeyi aç", &review.url, Some(EMOJI_REVIEW)),
            link_button("Filmi aç", &review.film_url, Some(EMOJI_LOGO)),
        ],
    }));

    CustomCard {
        components,
        accent_color: Some(LETTERBOXD_ORANGE),
        video_components: None,
    }
}

/// Son etkinlik satırı: yıldızlar, filmin bağlantısı ve varsa kalp/yeniden izleme işaretleri.
fn activity_line(activity: &ProfileActivity) -> String {
    let mut line = String::new();
    if let Some(rating) = activity.rating {
        line.push_str(&stars(rating));
        line.push(' ');
    }
    line.push_str(&format!(
        "[{}]({})",
        escape_markdown(&activity.title),
        activity.url
    ));
    for (flag, icon) in [
        (activity.liked, EMOJI_LIKE),
        (activity.rewatch, EMOJI_REWATCH),
        (activity.review, EMOJI_REVIEW),
    ] {
        if flag {
            line.push(' ');
            line.push_str(&icon.tag());
        }
    }
    line
}

fn profile_subpage(user: &str, page: &str) -> String {
    format!("{LETTERBOXD}/{user}/{page}/")
}

fn profile_card(profile: &Profile) -> CustomCard {
    let mut header = format!("### [{}]({})", escape_markdown(&profile.name), profile.url);
    let mut byline = format!("@{}", escape_markdown(&profile.user));
    if let Some(badge) = &profile.badge {
        // Patron rozetine Letterboxd'un tacı eşlik eder; diğer rozetler (Pro, HQ) yalnızca yazı.
        let crown = if badge.eq_ignore_ascii_case("patron") {
            format!("{} ", EMOJI_PATRON.tag())
        } else {
            String::new()
        };
        byline.push_str(&format!(" · {crown}{}", escape_markdown(badge)));
    }
    header.push_str(&format!("\n-# {byline}"));

    let mut section_children = vec![text(header)];
    if let Some(stats) = profile.stats.card_line() {
        section_children.push(text(stats));
    }
    let mut meta = Vec::new();
    if let Some(location) = &profile.location {
        meta.push(format!("📍 {}", escape_markdown(location)));
    }
    for link in &profile.links {
        meta.push(format!("[{}]({})", escape_markdown(&link.label), link.url));
    }
    if !meta.is_empty() {
        section_children.push(text(format!("-# {}", meta.join(" · "))));
    }

    // Discord'da Section aksesuarı zorunludur: avatar yoksa profil bağlantı butonu konur.
    let mut components = vec![section(
        section_children,
        profile.avatar.as_deref(),
        &profile.url,
        &format!("{} avatarı", profile.name),
    )];

    if let Some(bio) = &profile.bio {
        components.push(text(escape_markdown(&bounded(bio, BIO_LIMIT))));
    }
    if !profile.favorites.is_empty() {
        let favorites = profile
            .favorites
            .iter()
            .map(|film| format!("[{}]({})", escape_markdown(&film.title), film.url))
            .collect::<Vec<_>>()
            .join(", ");
        components.push(text(format!("**Favori filmler:** {favorites}")));
    }
    if !profile.recent.is_empty() {
        let mut lines = vec!["**Son etkinlik**".to_string()];
        lines.extend(profile.recent.iter().map(activity_line));
        components.push(text(lines.join("\n")));
    }

    let buttons = vec![
        link_button("Letterboxd", &profile.url, Some(EMOJI_LOGO)),
        link_button(
            "Filmler",
            &profile_subpage(&profile.user, "films"),
            Some(EMOJI_FILMS),
        ),
        link_button(
            "Günlük",
            &profile_subpage(&profile.user, "diary"),
            Some(EMOJI_THIS_YEAR),
        ),
        link_button(
            "Listeler",
            &profile_subpage(&profile.user, "lists"),
            Some(EMOJI_LISTS),
        ),
    ];
    components.push(json!({ "type": TYPE_SEPARATOR, "spacing": 1 }));
    components.push(json!({ "type": TYPE_ACTION_ROW, "components": buttons }));

    CustomCard {
        components,
        accent_color: Some(LETTERBOXD_BLUE),
        video_components: None,
    }
}

fn profile_post(profile: &Profile) -> BuiltinPost {
    let mut author = Author::new(profile.name.clone(), "");
    author.avatar = profile.avatar.clone();
    author.profile_url = Some(profile.url.clone());
    let mut post = BuiltinPost::new(Platform::Letterboxd, profile.url.clone(), author);
    post.title = Some(format!("{} (@{})", profile.name, profile.user));
    post.text = profile
        .bio
        .clone()
        .filter(|bio| !bio.is_empty())
        .or_else(|| profile.stats.line())
        .unwrap_or_default();
    post.extra_label = Some("Letterboxd".to_string());
    post.custom_card = Some(profile_card(profile));
    post
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

    /// Canlı profil sayfasından kısaltılmış örnek (rozet, bio, istatistik, favoriler, etkinlik).
    const PROFILE_HTML: &str = r##"<html><head>
        <meta property="og:image" content="https://a.ltrbxd.com/resized/avatar/upload/1/8/3/2/2/6/shard/avtr-0-1000-0-1000-crop.jpg?v=6791db69be">
        </head><body>
        <div class="profile-header js-profile-header -is-not-mini-nav" data-person="hollieamanda">
            <div class="profile-summary js-profile-summary -pending">
                <div class="profile-avatar">
                    <span class="avatar -a110 -large" > <img src="https://a.ltrbxd.com/resized/avatar/upload/1/8/3/2/2/6/shard/avtr-0-220-0-220-crop.jpg?v=6791db69be" alt="hollie amanda" width="110" height="110" /> </span>
                </div>
                <div class="profile-name-and-actions js-profile-name-and-actions">
                    <h1 class="person-display-name js-display-name">
                        <span class="displayname tooltip" title="hollieamanda"><span class="label">hollie amanda</span></span><span class="_inline-tail tail">&nbsp;<span class="badge -patron ">Patron</span></span>
                    </h1>
                </div>
            </div>
            <div class="profile-info js-profile-info">
                <div class="profile-stats js-profile-stats">
                    <h4 class="profile-statistic statistic"><a href="/hollieamanda/films/" class="thousands"><span class="value">2,590</span><span class="definition title-all-caps -small">Films</span></a></h4>
                    <h4 class="profile-statistic statistic"><a href="/hollieamanda/diary/for/2026/"><span class="value">64</span><span class="definition title-all-caps -small">This year</span></a></h4>
                    <h4 class="profile-statistic statistic"><a href="/hollieamanda/lists/"><span class="value">14</span><span class="definition title-all-caps -small">Lists</span></a></h4>
                    <h4 class="profile-statistic statistic"><a href="/hollieamanda/following/"><span class="value">396</span><span class="definition title-all-caps -small">Following</span></a></h4>
                    <h4 class="profile-statistic statistic"><a href="/hollieamanda/followers/" class="thousands"><span class="value">16,700</span><span class="definition title-all-caps -small">Followers</span></a></h4>
                </div>
                <div class="bio js-bio">
                    <div class="js-collapsible-text body-text -small -reset js-bio-content" data-full-text-url="/s/full-text/person:183226/">
                        <p>i watch movie, i have an epiphany, i forget about it the next day. that&#039;s just who i am</p>
                    </div>
                </div>
                <div class="profile-metadata js-profile-metadata">
                    <div class="metadatum -has-label"><svg role="presentation" width="8" height="16"><path d="M4 2"/></svg><span class="label">halifax, ns</span></div>
                    <a class="metadatum -has-label" href="https://instagram.com/hollie.amanda" rel="me nofollow noopener noreferrer" target="_blank"><svg role="presentation" width="14" height="16"><path d="M8 7"/></svg><span class="label">instagram.com</span></a>
                </div>
            </div>
        </div>
        <div class="cols-2"><div class="col-16">
        <section id="favourites" class="section"><h2 class="section-heading">Favorite films</h2><div class="poster-grid"><ul class="grid -p150">
            <li class="griditem"><div class="favourite-production-poster-container"><div class="react-component" data-item-name="Her (2013)" data-item-slug="her" data-item-link="/film/her/" data-item-full-display-name="Her (2013)"></div></div></li>
            <li class="griditem"><div class="favourite-production-poster-container"><div class="react-component" data-item-name="Harold and Maude (1971)" data-item-slug="harold-and-maude" data-item-link="/film/harold-and-maude/"></div></div></li>
            <li class="griditem"><div class="favourite-production-poster-container"><div class="react-component" data-item-name="Twin Peaks: Fire Walk with Me (1992)" data-item-slug="twin-peaks-fire-walk-with-me" data-item-link="/film/twin-peaks-fire-walk-with-me/"></div></div></li>
            <li class="griditem"><div class="favourite-production-poster-container"><div class="react-component" data-item-name="Suspiria (1977)" data-item-slug="suspiria" data-item-link="/film/suspiria/"></div></div></li>
        </ul></div></section>
        <section id="recent-activity" class="section -tinyicons"><h2 class="section-heading"><a href="/hollieamanda/activity/">Recent activity</a></h2><div class="poster-grid"><ul class="grid -p150">
            <li class="griditem"><div class="viewing-poster-container" data-owner="hollieamanda"><div class="react-component" data-item-name="Over the Garden Wall (2014)" data-item-slug="over-the-garden-wall-2014" data-item-link="/film/over-the-garden-wall-2014/" data-target-link="/hollieamanda/film/over-the-garden-wall-2014/"></div><p class="poster-viewingdata"><span class="rating rated-8"> ★★★★ </span><span class="rating liked"> </span></p></div></li>
            <li class="griditem"><div class="viewing-poster-container" data-owner="hollieamanda"><div class="react-component" data-item-name="The Devils (1971)" data-item-slug="the-devils" data-item-link="/film/the-devils/" data-target-link="/hollieamanda/film/the-devils/1/"></div><p class="poster-viewingdata"><span class="rating rated-8"> ★★★★ </span><span class="rating liked"> </span><span class="has-icon icon-rewatch rewatch-small icon-16"></span></p></div></li>
            <li class="griditem"><div class="viewing-poster-container" data-owner="hollieamanda"><div class="react-component" data-item-name="You Can See Everything (2026)" data-item-slug="you-can-see-everything" data-item-link="/film/you-can-see-everything/" data-target-link="/hollieamanda/film/you-can-see-everything/"></div><p class="poster-viewingdata"><span class="rating rated-9"> ★★★★½ </span><a href="/hollieamanda/film/you-can-see-everything/" class="has-icon icon-review review-small icon-16 tooltip" title="Review"></a></p></div></li>
        </ul></div></section>
        </div></div>
        </body></html>"##;

    /// Biyografisi, istatistiği, favorisi ve etkinliği olmayan asgari profil.
    const MINIMAL_PROFILE_HTML: &str = r##"<html><head></head><body>
        <div class="profile-header js-profile-header" data-person="yeniuye">
            <h1 class="person-display-name js-display-name"><span class="displayname tooltip" title="yeniuye"><span class="label">Yeni Üye</span></span></h1>
        </div>
        </body></html>"##;

    fn review_target() -> Target {
        Target::Review {
            user: "hollieamanda".into(),
            slug: "parasite-2019".into(),
            entry: None,
        }
    }

    fn profile_target() -> Target {
        Target::Profile {
            user: "hollieamanda".into(),
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
        readable(&out)
    }

    /// Emoji etiketlerini okunur işaretlere çevirir: testler yerleşimi `★★★★½ ♥` gibi
    /// okunur biçimde doğrular (etiketlerin kendisi `icons_are_bot_emojis` testinde).
    fn readable(text: &str) -> String {
        [
            (EMOJI_LOGO, "LB"),
            (EMOJI_STAR, "★"),
            (EMOJI_HALF, "½"),
            (EMOJI_LIKE, "♥"),
            (EMOJI_REWATCH, "🔁"),
            (EMOJI_REVIEW, "📝"),
            (EMOJI_SPOILER, "⚠️"),
            (EMOJI_WATCHED, "👁"),
            (EMOJI_LIKES, "♡"),
            (EMOJI_PLAY, "▶"),
            (EMOJI_FILMS, "🎞"),
            (EMOJI_THIS_YEAR, "📅"),
            (EMOJI_LISTS, "▦"),
            (EMOJI_PEOPLE, "👤"),
            (EMOJI_PATRON, "👑"),
        ]
        .iter()
        .fold(text.to_string(), |acc, (emoji, mark)| {
            acc.replace(&emoji.tag(), mark)
        })
    }

    /// Butonların ikon adları (ikonsuz buton boş ad döner).
    fn button_icons(card: &CustomCard) -> Vec<String> {
        card.components
            .iter()
            .filter(|component| component["type"] == TYPE_ACTION_ROW)
            .flat_map(|row| row["components"].as_array().cloned().unwrap_or_default())
            .map(|button| {
                button["emoji"]["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string()
            })
            .collect()
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
        assert_eq!(
            rewatch.film_url().as_deref(),
            Some("https://letterboxd.com/film/heat-1995/")
        );
        assert_eq!(
            Target::from_url("https://letterboxd.com/hollieamanda/"),
            Some(Target::Profile {
                user: "hollieamanda".into()
            })
        );
        assert_eq!(
            Target::from_url("https://letterboxd.com/hollieamanda/?utm_source=profil"),
            Some(Target::Profile {
                user: "hollieamanda".into()
            })
        );
        assert_eq!(
            Target::from_url("https://letterboxd.com/hollieamanda/")
                .unwrap()
                .url(),
            "https://letterboxd.com/hollieamanda/"
        );
        assert!(
            Target::from_url("https://letterboxd.com/hollieamanda/")
                .unwrap()
                .film_url()
                .is_none()
        );
        assert_eq!(
            Target::from_url("https://letterboxd.com/hollieamanda/films/"),
            None
        );
        assert_eq!(Target::from_url("https://letterboxd.com/films/"), None);
        assert_eq!(Target::from_url("https://letterboxd.com/about/"), None);
        assert_eq!(Target::from_url("https://boxd.it/2bg8"), None);
        assert_eq!(Target::from_url("https://evil.test/film/x/"), None);
    }

    #[test]
    fn profile_page_is_parsed() {
        let profile = parse_profile(PROFILE_HTML, &profile_target()).unwrap();
        assert_eq!(profile.user, "hollieamanda");
        assert_eq!(profile.name, "hollie amanda");
        assert_eq!(profile.badge.as_deref(), Some("Patron"));
        assert_eq!(profile.url, "https://letterboxd.com/hollieamanda/");
        assert!(
            profile
                .avatar
                .as_deref()
                .unwrap()
                .contains("-0-1000-0-1000-crop")
        );
        assert_eq!(
            profile.bio.as_deref(),
            Some(
                "i watch movie, i have an epiphany, i forget about it the next day. that's just who i am"
            )
        );
        assert_eq!(profile.location.as_deref(), Some("halifax, ns"));
        assert_eq!(
            profile.links,
            vec![ProfileLink {
                label: "instagram.com".into(),
                url: "https://instagram.com/hollie.amanda".into()
            }]
        );
        assert_eq!(
            profile.stats,
            ProfileStats {
                films: Some(2_590),
                this_year: Some(64),
                lists: Some(14),
                following: Some(396),
                followers: Some(16_700),
            }
        );
        assert_eq!(
            profile.stats.line().as_deref(),
            Some(
                "**2.590** film · **64** bu yıl · **14** liste · **396** takip · **16.700** takipçi"
            )
        );
        assert_eq!(profile.favorites.len(), 4);
        assert_eq!(profile.favorites[0].title, "Her (2013)");
        assert_eq!(profile.favorites[0].url, "https://letterboxd.com/film/her/");
        assert_eq!(profile.recent.len(), 3);
        assert_eq!(profile.recent[0].rating, Some(4.0));
        assert!(profile.recent[0].liked);
        assert_eq!(
            profile.recent[0].url,
            "https://letterboxd.com/hollieamanda/film/over-the-garden-wall-2014/"
        );
        assert!(profile.recent[1].rewatch);
        assert_eq!(
            profile.recent[1].url,
            "https://letterboxd.com/hollieamanda/film/the-devils/1/"
        );
        assert!(profile.recent[2].review);
        assert_eq!(profile.recent[2].rating, Some(4.5));
        assert!(parse_profile("<html></html>", &profile_target()).is_err());
    }

    #[test]
    fn profile_card_shows_stats_bio_favourites_and_activity() {
        let profile = parse_profile(PROFILE_HTML, &profile_target()).unwrap();
        let card = profile_card(&profile);
        assert_eq!(card.accent_color, Some(LETTERBOXD_BLUE));
        assert_eq!(card.components[0]["type"], TYPE_SECTION);
        assert_eq!(card.components[0]["accessory"]["type"], TYPE_THUMBNAIL);
        let text = contents(&card);
        assert!(
            text.contains("### [hollie amanda](https://letterboxd.com/hollieamanda/)"),
            "{text}"
        );
        assert!(text.contains("-# @hollieamanda · 👑 Patron"), "{text}");
        assert!(
            text.contains(
                "🎞 **2.590** film · 📅 **64** bu yıl · ▦ **14** liste · 👤 **396** takip · 👤 **16.700** takipçi"
            ),
            "{text}"
        );
        assert!(text.contains("📍 halifax, ns"), "{text}");
        assert!(
            text.contains("[instagram.com](https://instagram.com/hollie.amanda)"),
            "{text}"
        );
        assert!(
            text.contains("**Favori filmler:** [Her (2013)](https://letterboxd.com/film/her/)"),
            "{text}"
        );
        assert!(
            text.contains(
                "★★★★ [Over the Garden Wall (2014)](https://letterboxd.com/hollieamanda/film/over-the-garden-wall-2014/) ♥"
            ),
            "{text}"
        );
        assert!(
            text.contains(
                "★★★★ [The Devils (1971)](https://letterboxd.com/hollieamanda/film/the-devils/1/) ♥ 🔁"
            ),
            "{text}"
        );
        assert!(
            text.contains(
                "★★★★½ [You Can See Everything (2026)](https://letterboxd.com/hollieamanda/film/you-can-see-everything/) 📝"
            ),
            "{text}"
        );
        let labels: Vec<String> = buttons(&card).into_iter().map(|(label, _)| label).collect();
        assert_eq!(labels, vec!["Letterboxd", "Filmler", "Günlük", "Listeler"]);
        assert_eq!(
            button_icons(&card),
            vec!["letterboxd", "lb_films", "lb_thisyear", "lb_lists"]
        );
        let urls: Vec<String> = buttons(&card).into_iter().map(|(_, url)| url).collect();
        assert_eq!(
            urls,
            vec![
                "https://letterboxd.com/hollieamanda/",
                "https://letterboxd.com/hollieamanda/films/",
                "https://letterboxd.com/hollieamanda/diary/",
                "https://letterboxd.com/hollieamanda/lists/",
            ]
        );
    }

    #[test]
    fn minimal_profile_still_renders_a_base_card() {
        let target = Target::Profile {
            user: "yeniuye".into(),
        };
        let profile = parse_profile(MINIMAL_PROFILE_HTML, &target).unwrap();
        assert_eq!(profile.name, "Yeni Üye");
        assert!(profile.badge.is_none());
        assert!(profile.bio.is_none());
        assert!(profile.stats.line().is_none());
        assert!(profile.favorites.is_empty());
        assert!(profile.recent.is_empty());
        let card = profile_card(&profile);
        assert!(
            contents(&card).contains("### [Yeni Üye](https://letterboxd.com/yeniuye/)"),
            "{}",
            contents(&card)
        );
        assert!(!contents(&card).contains("Favori"));
        // Discord, aksesuarsız Section içeren mesajı reddeder: avatar yoksa bağlantı butonu.
        let accessory = &card.components[0]["accessory"];
        assert_eq!(
            accessory["type"], TYPE_BUTTON,
            "avatar yoksa buton aksesuar olmalı"
        );
        assert_eq!(accessory["url"], "https://letterboxd.com/yeniuye/");
    }

    #[test]
    fn profile_requests_carry_the_cloudflare_safe_user_agent() {
        assert!(
            LETTERBOXD_USER_AGENT.starts_with("Mozilla/5.0 (compatible; Discordbot/2.0;"),
            "{}",
            *LETTERBOXD_USER_AGENT
        );
        assert!(
            LETTERBOXD_USER_AGENT.ends_with(USER_AGENT),
            "bot kimliği tanıtıcının sonunda kalmalı: {}",
            *LETTERBOXD_USER_AGENT
        );
    }

    #[test]
    fn profile_counts_accept_letterboxd_suffixes() {
        assert_eq!(parse_stat_count("2,590"), Some(2_590));
        assert_eq!(parse_stat_count("16.7K"), Some(16_700));
        assert_eq!(parse_stat_count("1.2M"), Some(1_200_000));
        assert_eq!(parse_stat_count("0"), Some(0));
        assert_eq!(parse_stat_count("bilinmeyen"), None);
        assert_eq!(format_profile_count(2_590), "2.590");
        assert_eq!(format_profile_count(16_700), "16.700");
    }

    #[test]
    fn untrusted_profile_text_cannot_break_out_of_the_card_markdown() {
        let mut profile = parse_profile(PROFILE_HTML, &profile_target()).unwrap();
        profile.name = "Kötü](https://evil.test)".into();
        profile.bio = Some("bio ](https://evil.test) | sonrası".into());
        profile.favorites[0].title = "Film](https://evil.test)".into();
        let text = contents(&profile_card(&profile));
        assert!(!text.contains("[Kötü](https://evil.test)"), "{text}");
        assert!(!text.contains("[Film](https://evil.test)"), "{text}");
        assert!(!text.contains("bio ](https://evil.test)"), "{text}");
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
        // IMDb ve TMDB logoları Letterboxd sprite'ında yok: yalnızca yazı.
        assert_eq!(button_icons(&card), vec!["letterboxd", "lb_play", "", ""]);
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
            text.contains("İnceleyen: [hollie amanda](https://letterboxd.com/hollieamanda/)"),
            "{text}"
        );
        assert!(
            text.contains("👁 9 Ağu 2019 izledi · ♡ 43.6K beğeni · Yönetmen: Bong Joon Ho"),
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
    fn custom_poster_is_read_only_for_the_reviewed_film() {
        // Kullanıcının başka filmi önce gelir; varsayılan posterli kayıt özel poster sayılmaz.
        let html = r#"
            <div data-resolvable-poster-path='{&quot;preferredAlternativePosterId&quot;:&quot;665014&quot;,&quot;posteredBaseLink&quot;:&quot;/film/hoard/&quot;,&quot;hasDefaultPoster&quot;:true,&quot;cacheBustingKey&quot;:&quot;571a9ff2&quot;}'></div>
            <div data-resolvable-poster-path='{&quot;preferredAlternativePosterId&quot;:&quot;333479&quot;,&quot;postered&quot;:{&quot;uid&quot;:&quot;film:710352&quot;},&quot;posteredBaseLink&quot;:&quot;/film/poor-things-2023/&quot;,&quot;hasDefaultPoster&quot;:true,&quot;cacheBustingKey&quot;:&quot;70debc79&quot;}'></div>
        "#;
        assert_eq!(
            custom_poster_path(html, "poor-things-2023").as_deref(),
            Some("/film/poor-things-2023/poster/std/333479/600/?k=70debc79")
        );
        assert_eq!(custom_poster_path(html, "barbie"), None);

        let default_only = r#"<div data-resolvable-poster-path='{&quot;posteredBaseLink&quot;:&quot;/film/parasite-2019/&quot;,&quot;hasDefaultPoster&quot;:true}'></div>"#;
        assert_eq!(custom_poster_path(default_only, "parasite-2019"), None);
        assert_eq!(custom_poster_path(REVIEW_HTML, "parasite-2019"), None);

        // Kimlik yalnızca rakam olabilir: yol enjeksiyonu kabul edilmez.
        let hostile = r#"<div data-resolvable-poster-path='{&quot;preferredAlternativePosterId&quot;:&quot;1/../../x&quot;,&quot;posteredBaseLink&quot;:&quot;/film/barbie/&quot;}'></div>"#;
        assert_eq!(custom_poster_path(hostile, "barbie"), None);
    }

    #[test]
    fn icons_are_bot_emojis() {
        assert_eq!(EMOJI_STAR.tag(), "<:lb_star:1563865502881808486>");
        assert_eq!(
            stars(1.5),
            "<:lb_star:1563865502881808486><:lb_half:1563865487144914995>"
        );
        let button = link_button("Letterboxd", LETTERBOXD, Some(EMOJI_LOGO));
        assert_eq!(
            button["emoji"],
            json!({ "name": "letterboxd", "id": "1563865506841362505" })
        );
        assert!(link_button("IMDb", LETTERBOXD, None).get("emoji").is_none());
    }

    #[test]
    fn helpers_format_like_letterboxd() {
        assert_eq!(readable(&stars(4.51)), "★★★★½");
        assert_eq!(readable(&stars(3.0)), "★★★");
        assert_eq!(readable(&stars(0.5)), "½");
        assert_eq!(readable(&stars(4.2)), "★★★★");
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
    async fn live_film_review_profile_and_short_link() {
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

        let profile = fetch(&client, &parsed("https://letterboxd.com/hollieamanda/"))
            .await
            .expect("profil kartı üretilmeli");
        assert_eq!(profile.author.name, "hollie amanda");
        assert_eq!(
            profile.author.profile_url.as_deref(),
            Some("https://letterboxd.com/hollieamanda/")
        );
        let profile_text = contents(profile.custom_card.as_ref().unwrap());
        assert!(profile_text.contains("takipçi"), "{profile_text}");

        let short = fetch(&client, &parsed("https://boxd.it/2bg8"))
            .await
            .expect("kısa link çözülmeli");
        assert!(short.url.starts_with("https://letterboxd.com/film/"));

        // Kısa link bir profile de çözülebilir.
        let short_profile = fetch(&client, &parsed("https://boxd.it/7GEH"))
            .await
            .expect("profil kısa linki çözülmeli");
        assert_eq!(
            short_profile.author.profile_url.as_deref(),
            Some("https://letterboxd.com/hollieamanda/")
        );

        // Numaralı yeniden izleme kaydı, Cloudflare'e takılmayan tanıtıcıyla okunur.
        let rewatch = fetch(
            &client,
            &parsed("https://letterboxd.com/demiadejuyigbe/film/parasite-2019/2/"),
        )
        .await
        .expect("numaralı inceleme kartı üretilmeli");
        assert_eq!(rewatch.author.name, "demi adejuyigbe");

        // Özel poster seçen kullanıcının incelemesi kendi posteriyle, diğeri varsayılanla gelir.
        let poster = |post: &BuiltinPost| {
            post.custom_card.as_ref().unwrap().components[0]["accessory"]["media"]["url"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        };
        let custom = fetch(
            &client,
            &parsed("https://letterboxd.com/ashonfilm/film/poor-things-2023/"),
        )
        .await
        .expect("özel posterli inceleme kartı üretilmeli");
        assert!(
            poster(&custom).contains("/alternative-poster/"),
            "{}",
            poster(&custom)
        );
        assert!(
            poster(&review).contains("/film-poster/"),
            "{}",
            poster(&review)
        );
    }
}
