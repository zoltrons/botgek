//! Spotify gönderi verisi (önizleme çekme) ve modern (Components V2) kartı.
//!
//! Bu modül iki referans servisin yaklaşımını birleştirir:
//! - **Önizleme çekme (fxspotify):** `open.spotify.com/embed/{kind}/{id}` HTML sayfasındaki
//!   `__NEXT_DATA__` JSON'undan parça adı, sanatçılar, 30 sn `audioPreview.url`, albüm
//!   görseli ve çıkış tarihi okunur (Spotify API anahtarı gerekmez).
//! - **Modern embed (fixspotify):** veri Discord'un bileşen düzeniyle (Components V2) tek
//!   kart olarak gönderilir — Spotify yeşili `accent_color`, `Section` + `Thumbnail`,
//!   `Separator` ve `Action Row` içinde Spotify / Apple Music / YouTube Music butonları.
//!
//! `track`, `album`, `playlist`, `artist`, `episode` ve `show` bağlantıları desteklenir;
//! `spotify.link` kısa linkleri native motorda hedefe çözülür.

use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use serde_json::{Value, json};

use super::render::{escape_markdown, format_count};
use super::{Author, BuiltinPost, CustomCard, MAX_BODY_BYTES, Media, MediaKind};
use crate::converters::parser::{ParsedPost, Platform};

const SPOTIFY_OPEN: &str = "https://open.spotify.com";
const SPOTIFY_EMBED: &str = "https://open.spotify.com/embed";
/// `spotify.link` kısa linklerinin çözüldüğü Branch uç noktası (fixspotify ile aynı).
const SPOTIFY_LINK: &str = "https://spotify.app.link";
/// Spotify embed sayfası tarayıcı benzeri bir User-Agent bekler.
const BROWSER_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:134.0) Gecko/20100101 Firefox/134.0";
/// Spotify yeşili (`0x1DB954`).
const SPOTIFY_GREEN: u32 = 0x1DB954;
/// Buton logoları: botun kendi (application) emojileri — her sunucuda görünür. Kart
/// gönderimden bağımsız önbelleğe alındığı için ayar değil sabittir (Bluesky logosu gibi).
const SPOTIFY_EMOJI: (&str, &str) = ("spotify", "1563861508482666616");
const APPLE_MUSIC_EMOJI: (&str, &str) = ("applemusic", "1563861231931236452");
const YOUTUBE_MUSIC_EMOJI: (&str, &str) = ("youtubemusic", "1563862603413983365");
/// Dış isteklere zaman aşımı.
const TIMEOUT: Duration = Duration::from_secs(8);

// Components V2 bileşen türleri (yalnızca burada kullanılanlar).
const TYPE_ACTION_ROW: u64 = 1;
const TYPE_SECTION: u64 = 9;
const TYPE_TEXT_DISPLAY: u64 = 10;
const TYPE_THUMBNAIL: u64 = 11;
const TYPE_SEPARATOR: u64 = 14;

/// Parça listesinde sayfa başına parça.
const TRACKS_PER_PAGE: usize = 10;
/// Parça adı bu uzunluktan sonra `…` ile kısaltılır (liste satırı tek satırda kalsın).
const MAX_TRACK_TITLE_CHARS: usize = 60;
/// Sayfa değişiminde yerinde güncellenen bileşenlerin Components V2 `id`'leri: işleyici
/// mesajın geri kalanına (kullanıcı notu, gönderen, spoiler) dokunmadan yalnızca bunları değiştirir.
const TRACK_LIST_COMPONENT_ID: u64 = 7301;
const PAGER_COMPONENT_ID: u64 = 7302;
/// Sayfa butonlarının `custom_id` öneki: `spotify:page:{kind}:{id}:{page}`.
pub const PAGE_CUSTOM_ID_PREFIX: &str = "spotify:page:";
/// Sayfa göstergesi (devre dışı buton) için sabit kimlik.
const PAGE_INDICATOR_CUSTOM_ID: &str = "spotify:noop";

/// `open.spotify.com/(intl-xx/)?(kind)/id` — adres içinde (redirect/HTML gövdesi) aranır.
static OPEN_URL_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)open\.spotify\.com/(?:intl-[a-z-]+/)?(track|album|playlist|artist|episode|show)/([a-zA-Z0-9]{16,40})")
        .expect("geçerli Spotify link regex")
});
/// Embed sayfasındaki Next.js veri bloğu.
static NEXT_DATA_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<script id="__NEXT_DATA__" type="application/json">(.*?)</script>"#)
        .expect("geçerli __NEXT_DATA__ regex")
});
/// Sayfanın kanonik adresi (`og:url`) — kısa link çözümlemesinde yedek yol.
static OG_URL_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)<meta\s+property="og:url"\s+content="([^"]+)""#)
        .expect("geçerli og:url regex")
});

/// Çözümlenmiş Spotify hedefi.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Target {
    kind: String,
    id: String,
    open_url: String,
}

/// Spotify gönderisini çeker ve modern karta dönüştürür.
pub async fn fetch(client: &reqwest::Client, parsed: &ParsedPost) -> Result<BuiltinPost, String> {
    let target = resolve_target(client, &parsed.original_url).await?;
    let entity = fetch_entity(client, &target).await?;

    let view = EntityView::from(&entity, &target.kind);
    if view.title.is_none() {
        return Err(format!("Spotify başlığı okunamadı: {}", target.open_url));
    }

    let mut post = BuiltinPost::new(
        Platform::Spotify,
        target.open_url.clone(),
        Author::new("Spotify", ""),
    );
    post.title = view.title.clone();
    post.text = view.subtitle.clone().unwrap_or_default();
    post.extra_label = Some("Spotify".to_string());
    // `||url||` is a per-message presentation choice, not source metadata.  Keeping the
    // fetched card plain is important because the post is cached and may be requested both
    // with and without a user spoiler.
    post.custom_card = Some(build_card(&view, &target.open_url));
    if let Some(media) = preview_media(&view) {
        post.media.push(media);
    }
    Ok(post)
}

/// Embed sayfasındaki `entity` ağacını indirir.
async fn fetch_entity(client: &reqwest::Client, target: &Target) -> Result<Value, String> {
    let embed_url = format!(
        "{SPOTIFY_EMBED}/{}/{}?utm_source=botgek",
        target.kind, target.id
    );
    let html = get_html(client, &embed_url).await?;

    let next = extract_next_data(&html)
        .ok_or_else(|| format!("Spotify verisi çözümlenemedi: {}", target.open_url))?;

    let page_props = next.get("props").and_then(|value| value.get("pageProps"));
    if let Some(status) = page_props
        .and_then(|props| props.get("status"))
        .and_then(Value::as_i64)
        && (status == 404 || status == 500)
    {
        return Err(format!("Spotify içeriği bulunamadı: {}", target.open_url));
    }

    page_props
        .and_then(|props| props.get("state"))
        .and_then(|state| state.get("data"))
        .and_then(|data| data.get("entity"))
        .cloned()
        .ok_or_else(|| format!("Spotify varlık verisi yok: {}", target.open_url))
}

/// Hedefi `original_url`'den okur; `spotify.link` kısa linki ise hedefe çözer.
async fn resolve_target(_client: &reqwest::Client, original_url: &str) -> Result<Target, String> {
    if let Some(target) = target_from_url(original_url) {
        return Ok(target);
    }

    // spotify.link/token → spotify.app.link/token üzerinden hedefe yönlendirilir.
    let token = original_url
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("")
        .trim();
    if token.is_empty() || !token.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(format!("Spotify link biçimi tanınamadı: {original_url}"));
    }

    let url = format!("{SPOTIFY_LINK}/{}", super::encode_component(token));
    // Kısa linkler güvenilmeyen bir redirect/HTML sınırıdır. Ortak güvenli HTTP yolu;
    // durum kodunu doğrular, yönlendirmeleri elle ve sınırlı sayıda takip eder, her hedefi
    // public HTTP endpoint olarak doğrular ve gövdeyi akış sırasında sınırlar.
    let (resolved_url, body) = super::safe_http::preview(&url)
        .await
        .map_err(|e| format!("Spotify kısa link çözümlenemedi ({url}): {e}"))?;

    // 1) Yönlendirme sonrası adres zaten open.spotify.com olabilir.
    if let Some(target) = target_from_url(&resolved_url) {
        return Ok(target);
    }
    // 2) Aksi halde gövdedeki kanonik adres (og:url) okunur.
    if let Some(og) = OG_URL_REGEX.captures(&body).and_then(|caps| caps.get(1))
        && let Some(target) = target_from_url(og.as_str())
    {
        return Ok(target);
    }
    // 3) Son çare: gövdedeki ilk open.spotify.com bağlantısı.
    if let Some(target) = target_from_url(&body) {
        return Ok(target);
    }
    Err(format!(
        "Spotify kısa link hedefi bulunamadı: {original_url}"
    ))
}

/// Adres metninden (tam URL, redirect adresi veya HTML gövdesi) Spotify hedefi çıkarır.
fn target_from_url(candidate: &str) -> Option<Target> {
    let caps = OPEN_URL_REGEX.captures(candidate)?;
    let kind = caps.get(1)?.as_str().to_string();
    let id = caps.get(2)?.as_str().to_string();
    Some(Target {
        open_url: format!("{SPOTIFY_OPEN}/{kind}/{id}"),
        kind,
        id,
    })
}

/// HTML sayfasını tarayıcı benzeri başlıklarla indirir; gövde `MAX_BODY_BYTES` ile sınırlıdır.
async fn get_html(client: &reqwest::Client, url: &str) -> Result<String, String> {
    let response = client
        .get(url)
        .header("user-agent", BROWSER_USER_AGENT)
        .header(
            "accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header("accept-language", "en-US,en;q=0.9")
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("Spotify sayfası alınamadı ({url}): {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "Spotify sayfası {} döndü ({url})",
            response.status()
        ));
    }
    if let Some(len) = response.content_length()
        && len > MAX_BODY_BYTES as u64
    {
        return Err(format!("Spotify yanıtı çok büyük ({url}): {len} bayt"));
    }

    let mut response = response;
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("Spotify gövdesi okunamadı ({url}): {e}"))?
    {
        if body.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(format!("Spotify gövdesi sınırı aşıldı ({url})"));
        }
        body.extend_from_slice(&chunk);
    }

    String::from_utf8(body).map_err(|_| format!("Spotify yanıtı UTF-8 değil ({url})"))
}

/// Embed sayfasındaki `__NEXT_DATA__` JSON bloğunu çözümler.
fn extract_next_data(html: &str) -> Option<Value> {
    let caps = NEXT_DATA_REGEX.captures(html)?;
    serde_json::from_str(caps.get(1)?.as_str()).ok()
}

/// `entity` ağacından çıkarılan, karta hazır Spotify görünüm verisi.
#[derive(Debug, Default, PartialEq, Eq)]
struct EntityView {
    title: Option<String>,
    subtitle: Option<String>,
    /// Alt detay parçaları (ör. `["3:21", "Çıkış 2025-03-14"]`).
    details: Vec<String>,
    image: Option<String>,
    /// 30 sn önizleme sesi (`audioPreview.url`, MP3).
    preview: Option<String>,
    /// Albüm/playlist/podcast parçaları (`trackList`; embed en fazla 100 parça verir).
    tracks: Vec<TrackLine>,
}

/// Parça listesinin tek satırı.
#[derive(Debug, Default, PartialEq, Eq)]
struct TrackLine {
    title: String,
    subtitle: Option<String>,
    url: Option<String>,
    duration: Option<String>,
}

impl TrackLine {
    fn from(track: &Value) -> Option<Self> {
        let title = json_str(track, "title").or_else(|| json_str(track, "name"))?;
        // `spotify:track:ID` → `https://open.spotify.com/track/ID`
        let url = json_str(track, "uri").and_then(|uri| {
            let mut parts = uri.split(':');
            match (parts.next(), parts.next(), parts.next()) {
                (Some("spotify"), Some(kind), Some(id))
                    if !kind.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric()) =>
                {
                    Some(format!("{SPOTIFY_OPEN}/{kind}/{id}"))
                }
                _ => None,
            }
        });
        Some(Self {
            title,
            subtitle: json_str(track, "subtitle"),
            url,
            duration: track
                .get("duration")
                .and_then(Value::as_u64)
                .map(format_duration_ms),
        })
    }
}

impl EntityView {
    fn from(entity: &Value, kind: &str) -> Self {
        let title = json_str(entity, "name").or_else(|| json_str(entity, "title"));
        let image = best_image(entity);
        let preview = entity
            .get("audioPreview")
            .and_then(|preview| json_str(preview, "url"));
        let duration = entity
            .get("duration")
            .and_then(Value::as_u64)
            .map(format_duration_ms);

        let artists = artists_joined(entity);
        let subtitle = match kind {
            "artist" => genres_joined(entity).or(artists),
            "playlist" | "show" => json_str(entity, "subtitle")
                .or(artists)
                .or_else(|| json_str(entity, "description")),
            _ => artists.or_else(|| json_str(entity, "subtitle")),
        };

        let release = entity
            .get("releaseDate")
            .and_then(|release| json_str(release, "isoString"))
            .map(|iso| format!("Çıkış {}", iso.get(..10).unwrap_or(&iso)));

        let tracks: Vec<TrackLine> = entity
            .get("trackList")
            .and_then(Value::as_array)
            .map(|tracks| tracks.iter().filter_map(TrackLine::from).collect())
            .unwrap_or_default();
        let total_tracks = entity
            .get("trackList")
            .and_then(Value::as_array)
            .map(|tracks| tracks.len() as u64)
            .or_else(|| entity.get("trackCount").and_then(Value::as_u64));

        let mut details: Vec<String> = Vec::new();
        match kind {
            "artist" => {
                if let Some(followers) = entity.get("followers").and_then(Value::as_u64) {
                    details.push(format!("{} takipçi", format_count(followers)));
                }
                if let Some(duration) = &duration {
                    details.push(duration.clone());
                }
            }
            "album" | "playlist" | "show" => {
                if let Some(total) = total_tracks {
                    details.push(format!("{total} parça"));
                }
                if let Some(release) = release {
                    details.push(release);
                }
            }
            _ => {
                if let Some(duration) = &duration {
                    details.push(duration.clone());
                }
                if let Some(release) = release {
                    details.push(release);
                }
            }
        }

        Self {
            title,
            subtitle,
            details,
            image,
            preview,
            // Tek parçalık türlerde (track/episode) liste gösterilmez.
            tracks: if matches!(kind, "track" | "episode") {
                Vec::new()
            } else {
                tracks
            },
        }
    }
}

/// Önizleme + kapak varsa karta video olarak eklenecek medya: gönderimde
/// `spotify_video` MP3'ü kapakla tek MP4'e çevirir ve kartın galerisinde oynatır.
fn preview_media(view: &EntityView) -> Option<Media> {
    let preview = view
        .preview
        .as_deref()
        .filter(|url| url.starts_with("https://"))?;
    let cover = view
        .image
        .as_deref()
        .filter(|url| url.starts_with("https://"))?;
    Some(Media {
        kind: MediaKind::Video,
        url: preview.to_string(),
        poster: Some(cover.to_string()),
        width: None,
        height: None,
        duration: None,
    })
}

/// Components V2 kart gövdesini (bölüm + ayraç + buton satırı) üretir.
///
/// Kart **kaynak verisi** olarak üretilir ve önbelleğe alınır: spoiler görünümü gönderim başına
/// `render::apply_visibility` tarafından uygulanır (aynı gönderi hem spoiler'lı hem spoiler'sız
/// kanalda gösterilebilir).
///
/// Buton satırı: Spotify (asıl) + Apple Music ve YouTube Music aramaları. 30 sn önizleme
/// kartta değil, gönderimde üretilen video olarak eklenir (bkz. [`preview_media`]); video
/// eklenebilirse kapak zaten videoda göründüğü için [`compact_video_components`] kullanılır.
fn build_card(view: &EntityView, open_url: &str) -> CustomCard {
    let title = escape_markdown(view.title.as_deref().unwrap_or("Spotify"));
    let subtitle_line = match view
        .subtitle
        .as_deref()
        .map(str::trim)
        .filter(|sub| !sub.is_empty())
    {
        Some(sub) => format!("\nby {}", escape_markdown(sub)),
        None => String::new(),
    };
    let header = format!("### [{title}]({open_url}){subtitle_line}");

    let mut section_children = vec![text_component(header)];
    if !view.details.is_empty() {
        section_children.push(text_component(format!("-# {}", view.details.join(" · "))));
    }

    let accessory = match view.image.as_deref().filter(|url| url.starts_with("http")) {
        Some(image) => json!({
            "type": TYPE_THUMBNAIL,
            "media": { "url": image },
            "description": "Spotify kapak görseli",
        }),
        None => link_button("Spotify", open_url, SPOTIFY_EMOJI),
    };

    let mut components = vec![json!({
        "type": TYPE_SECTION,
        "components": section_children,
        "accessory": accessory,
    })];
    if let Some((list, pager)) =
        target_from_url(open_url).and_then(|target| track_page(view, &target.kind, &target.id, 0))
    {
        components.push(json!({ "type": TYPE_SEPARATOR, "spacing": 1 }));
        components.push(list);
        components.extend(pager);
    }

    let buttons = vec![
        link_button("Spotify", open_url, SPOTIFY_EMOJI),
        link_button(
            "Apple Music",
            &apple_music_search_url(view),
            APPLE_MUSIC_EMOJI,
        ),
        link_button(
            "YouTube Music",
            &youtube_music_search_url(view),
            YOUTUBE_MUSIC_EMOJI,
        ),
    ];

    let button_row = json!({ "type": TYPE_ACTION_ROW, "components": buttons });
    let video_components =
        preview_media(view).map(|_| compact_video_components(view, open_url, button_row.clone()));
    components.push(json!({ "type": TYPE_SEPARATOR, "spacing": 1 }));
    components.push(button_row);

    CustomCard {
        components,
        accent_color: Some(SPOTIFY_GREEN),
        video_components,
    }
}

/// Video eklendiğinde kullanılan kompakt düzen: küçük görsel yok, başlık + sanatçı tek satırda,
/// detaylar küçük yazıyla altında; video (V2 tarafından) başlığın hemen altına, butonlar
/// ayraçsız en alta gelir.
fn compact_video_components(view: &EntityView, open_url: &str, button_row: Value) -> Vec<Value> {
    let title = escape_markdown(view.title.as_deref().unwrap_or("Spotify"));
    let mut header = format!("**[{title}]({open_url})**");
    if let Some(subtitle) = view
        .subtitle
        .as_deref()
        .map(str::trim)
        .filter(|sub| !sub.is_empty())
    {
        header.push_str(&format!(" · {}", escape_markdown(subtitle)));
    }
    if !view.details.is_empty() {
        header.push_str(&format!("\n-# {}", view.details.join(" · ")));
    }
    vec![text_component(header), button_row]
}

/// `page` sayfasının parça listesi (metin) ve birden fazla sayfa varsa sayfa butonları.
/// Parça yoksa ya da sayfa aralık dışındaysa `None`.
fn track_page(
    view: &EntityView,
    kind: &str,
    id: &str,
    page: usize,
) -> Option<(Value, Option<Value>)> {
    let pages = view.tracks.len().div_ceil(TRACKS_PER_PAGE);
    if page >= pages {
        return None;
    }
    let start = page * TRACKS_PER_PAGE;
    // Albümde her parçanın sanatçısı genelde albümünkiyle aynıdır: tekrar yazılmaz.
    let owner = view.subtitle.as_deref().map(str::trim);
    let lines: Vec<String> = view
        .tracks
        .iter()
        .enumerate()
        .skip(start)
        .take(TRACKS_PER_PAGE)
        .map(|(index, track)| {
            let title = escape_markdown(&shorten(&track.title, MAX_TRACK_TITLE_CHARS));
            let mut line = match &track.url {
                Some(url) => format!("`{:02}` [{title}]({url})", index + 1),
                None => format!("`{:02}` {title}", index + 1),
            };
            if let Some(artist) = track
                .subtitle
                .as_deref()
                .map(str::trim)
                .filter(|artist| !artist.is_empty() && Some(*artist) != owner)
            {
                line.push_str(&format!(" — {}", escape_markdown(artist)));
            }
            if let Some(duration) = &track.duration {
                line.push_str(&format!(" · {duration}"));
            }
            line
        })
        .collect();

    let list = json!({
        "type": TYPE_TEXT_DISPLAY,
        "id": TRACK_LIST_COMPONENT_ID,
        "content": lines.join("\n"),
    });
    let pager = (pages > 1).then(|| {
        let target = |page: usize| format!("{PAGE_CUSTOM_ID_PREFIX}{kind}:{id}:{page}");
        json!({
            "type": TYPE_ACTION_ROW,
            "id": PAGER_COMPONENT_ID,
            "components": [
                page_button("◀", &target(page.saturating_sub(1)), page == 0),
                page_button(&format!("{}/{pages}", page + 1), PAGE_INDICATOR_CUSTOM_ID, true),
                page_button("▶", &target(page + 1), page + 1 >= pages),
            ],
        })
    });
    Some((list, pager))
}

/// `{"type": 2, "style": 2, ...}` — gri (secondary) sayfa butonu.
fn page_button(label: &str, custom_id: &str, disabled: bool) -> Value {
    json!({ "type": 2, "style": 2, "label": label, "custom_id": custom_id, "disabled": disabled })
}

/// Metni `max` karakterde `…` ile keser.
fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Sayfa butonu tıklaması: `custom_id`'den hedefi ve sayfayı okur, parça listesini yeniden
/// çeker ve mesajın bileşen ağacında yalnızca liste + sayfa butonlarını değiştirir.
///
/// `components` mesajın mevcut (Discord'dan okunan) bileşenleridir. Spoiler'lı kartta liste
/// metni spoiler olarak kalır.
pub async fn turn_page(
    client: &reqwest::Client,
    custom_id: &str,
    components: &mut Value,
) -> Result<(), String> {
    let (kind, id, page) = parse_page_custom_id(custom_id).ok_or("geçersiz sayfa kimliği")?;
    let target = Target {
        open_url: format!("{SPOTIFY_OPEN}/{kind}/{id}"),
        kind: kind.to_string(),
        id: id.to_string(),
    };
    let entity = fetch_entity(client, &target).await?;
    let view = EntityView::from(&entity, kind);
    let (list, pager) = track_page(&view, kind, id, page).ok_or("sayfa yok")?;
    if replace_page_components(components, &list, pager.as_ref()) {
        Ok(())
    } else {
        Err("parça listesi mesajda bulunamadı".into())
    }
}

/// `spotify:page:{kind}:{id}:{page}` → (kind, id, page).
fn parse_page_custom_id(custom_id: &str) -> Option<(&str, &str, usize)> {
    let rest = custom_id.strip_prefix(PAGE_CUSTOM_ID_PREFIX)?;
    let mut parts = rest.split(':');
    let (kind, id, page) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some()
        || !matches!(kind, "album" | "playlist" | "show" | "artist")
        || id.is_empty()
        || id.len() > 40
        || !id.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return None;
    }
    let page: usize = page.parse().ok()?;
    (page < 100).then_some((kind, id, page))
}

/// Bileşen ağacında `id`'si eşleşen liste ve sayfa satırını değiştirir; liste bulunduysa `true`.
fn replace_page_components(tree: &mut Value, list: &Value, pager: Option<&Value>) -> bool {
    match tree {
        Value::Array(items) => {
            let mut found = false;
            for item in items {
                found |= replace_page_components(item, list, pager);
            }
            found
        }
        Value::Object(object) => {
            match object.get("id").and_then(Value::as_u64) {
                Some(TRACK_LIST_COMPONENT_ID) => {
                    let spoiler = object
                        .get("content")
                        .and_then(Value::as_str)
                        .is_some_and(|content| content.starts_with("||"));
                    let mut list = list.clone();
                    if spoiler && let Some(content) = list["content"].as_str() {
                        list["content"] =
                            json!(format!("||{}||", content.replace("||", "|\u{200b}|")));
                    }
                    *tree = list;
                    return true;
                }
                Some(PAGER_COMPONENT_ID) => {
                    if let Some(pager) = pager {
                        *tree = pager.clone();
                    }
                    return false;
                }
                _ => {}
            }
            object.values_mut().fold(false, |found, child| {
                replace_page_components(child, list, pager) | found
            })
        }
        _ => false,
    }
}

/// Kart butonları için arama sorgusu (`Başlık Sanatçı`).
fn search_query(view: &EntityView) -> String {
    let title = view.title.as_deref().unwrap_or("").trim();
    let artists = view.subtitle.as_deref().unwrap_or("").trim();
    format!("{title} {artists}").trim().to_string()
}

/// Apple Music arama bağlantısı.
fn apple_music_search_url(view: &EntityView) -> String {
    format!(
        "https://music.apple.com/search?term={}",
        super::encode_component(&search_query(view))
    )
}

/// YouTube Music arama bağlantısı.
fn youtube_music_search_url(view: &EntityView) -> String {
    format!(
        "https://music.youtube.com/search?q={}",
        super::encode_component(&search_query(view))
    )
}

/// `{"type": 10, "content": ...}` — markdown destekli metin bileşeni.
fn text_component(content: impl Into<String>) -> Value {
    json!({ "type": TYPE_TEXT_DISPLAY, "content": content.into() })
}

/// `{"type": 2, "style": 5, ...}` — platform logolu bağlantı (link) butonu.
fn link_button(label: &str, url: &str, (emoji_name, emoji_id): (&str, &str)) -> Value {
    json!({
        "type": 2,
        "style": 5,
        "label": label,
        "url": url,
        "emoji": { "name": emoji_name, "id": emoji_id },
    })
}

/// Nesne alanından boş olmayan metin okur.
fn json_str(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

/// `artists` dizisindeki sanatçı adlarını `, ` ile birleştirir.
fn artists_joined(entity: &Value) -> Option<String> {
    let artists = entity.get("artists").and_then(Value::as_array)?;
    let names: Vec<String> = artists
        .iter()
        .filter_map(|artist| artist.get("name").and_then(Value::as_str))
        .map(str::to_string)
        .collect();
    (!names.is_empty()).then(|| names.join(", "))
}

/// `genres` dizisini `, ` ile birleştirir (sanatçı kartında alt başlık).
fn genres_joined(entity: &Value) -> Option<String> {
    let genres = entity.get("genres").and_then(Value::as_array)?;
    let names: Vec<String> = genres
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    (!names.is_empty()).then(|| names.join(", "))
}

/// `visualIdentity.image` içinden en yüksek genişlikteki kapağı seçer.
fn best_image(entity: &Value) -> Option<String> {
    let images = entity
        .get("visualIdentity")
        .and_then(|identity| identity.get("image"))
        .and_then(Value::as_array)?;
    let mut best: Option<(u64, String)> = None;
    for image in images {
        if let Some(url) = image.get("url").and_then(Value::as_str) {
            let width = image.get("maxWidth").and_then(Value::as_u64).unwrap_or(0);
            let better = best.as_ref().map(|(w, _)| width > *w).unwrap_or(true);
            if better {
                best = Some((width, url.to_string()));
            }
        }
    }
    best.map(|(_, url)| url)
}

/// Milisaniyeyi `d:ss` biçimine çevirir.
fn format_duration_ms(ms: u64) -> String {
    let total_seconds = ms / 1000;
    format!("{}:{:02}", total_seconds / 60, total_seconds % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_url_variants_are_resolved() {
        for (input, kind, id) in [
            (
                "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT",
                "track",
                "4cOdK2wGLETKBW3PvgPWqT",
            ),
            (
                "https://open.spotify.com/intl-tr/album/1DFixLWuPkv3KT3TnV35m3?si=abc",
                "album",
                "1DFixLWuPkv3KT3TnV35m3",
            ),
            (
                "https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M",
                "playlist",
                "37i9dQZF1DXcBWIGoYBM5M",
            ),
        ] {
            let target = target_from_url(input).expect("hedef çözümlenmeli");
            assert_eq!(target.kind, kind);
            assert_eq!(target.id, id);
            assert_eq!(
                target.open_url,
                format!("https://open.spotify.com/{kind}/{id}")
            );
        }
    }

    #[tokio::test]
    async fn short_link_with_invalid_token_is_rejected_without_request() {
        let client = reqwest::Client::new();
        let error = resolve_target(&client, "https://spotify.link/not-a-valid-token!")
            .await
            .expect_err("geçersiz kısa link ağ isteği yapmamalı");
        assert!(error.contains("link biçimi tanınamadı"));
    }

    #[test]
    fn next_data_is_extracted_from_embed_html() {
        let html = r#"<html><body><script id="__NEXT_DATA__" type="application/json">{"props":{"pageProps":{"state":{"data":{"entity":{"name":"Şarkı"}}}}}}</script></body></html>"#;
        let value = extract_next_data(html).expect("JSON çözümlenmeli");
        assert_eq!(
            value["props"]["pageProps"]["state"]["data"]["entity"]["name"],
            "Şarkı"
        );
    }

    #[test]
    fn track_entity_builds_card_with_preview_button() {
        let entity = json!({
            "name": "Never Gonna Give You Up",
            "artists": [{ "name": "Rick Astley" }],
            "audioPreview": { "url": "https://p.scdn.co/mp3-preview/abc.mp3" },
            "duration": 213573,
            "releaseDate": { "isoString": "1987-11-12T00:00:00Z" },
            "visualIdentity": {
                "image": [
                    { "url": "https://i.scdn.co/image/small.jpg", "maxWidth": 64 },
                    { "url": "https://i.scdn.co/image/large.jpg", "maxWidth": 640 }
                ]
            }
        });

        let view = EntityView::from(&entity, "track");
        assert_eq!(view.title.as_deref(), Some("Never Gonna Give You Up"));
        assert_eq!(view.subtitle.as_deref(), Some("Rick Astley"));
        assert_eq!(
            view.image.as_deref(),
            Some("https://i.scdn.co/image/large.jpg")
        );
        assert_eq!(view.details, vec!["3:33", "Çıkış 1987-11-12"]);

        let compact = build_card(
            &view,
            "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT",
        )
        .video_components
        .expect("önizlemeli parçada kompakt video düzeni");
        assert_eq!(compact.len(), 2, "başlık + butonlar (video arasına girer)");
        assert_eq!(
            compact[0]["content"],
            "**[Never Gonna Give You Up](https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT)** \
             · Rick Astley\n-# 3:33 · Çıkış 1987-11-12"
        );
        assert!(
            !compact.iter().any(|c| c["type"] == TYPE_SECTION),
            "küçük görsel yok"
        );
        assert_eq!(compact[1]["type"], TYPE_ACTION_ROW);

        let media = preview_media(&view).expect("önizleme + kapak → video medyası");
        assert_eq!(media.kind, MediaKind::Video);
        assert_eq!(media.url, "https://p.scdn.co/mp3-preview/abc.mp3");
        assert_eq!(
            media.poster.as_deref(),
            Some("https://i.scdn.co/image/large.jpg")
        );

        let card = build_card(
            &view,
            "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT",
        );
        assert_eq!(card.accent_color, Some(0x1DB954));
        assert_eq!(card.components.len(), 3);

        let section = &card.components[0];
        assert_eq!(section["type"], TYPE_SECTION);
        assert_eq!(section["accessory"]["type"], TYPE_THUMBNAIL);
        assert!(
            section["components"][0]["content"]
                .as_str()
                .unwrap()
                .contains("[Never Gonna Give You Up](https://open.spotify.com/track/")
        );

        // Buton satırı: Spotify + Apple Music + YouTube Music (önizleme butonu yok).
        let buttons = card.components[2]["components"].as_array().unwrap();
        assert_eq!(buttons.len(), 3);
        assert_eq!(
            buttons[0]["url"],
            "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT"
        );
        let apple = buttons[1]["url"].as_str().unwrap();
        assert!(
            apple.starts_with("https://music.apple.com/search?term="),
            "apple: {apple}"
        );
        assert!(apple.contains("Never"));
        let youtube = buttons[2]["url"].as_str().unwrap();
        assert!(
            youtube.starts_with("https://music.youtube.com/search?q="),
            "youtube: {youtube}"
        );
        assert!(youtube.contains("Rick"));
        for button in buttons {
            assert_eq!(button["style"], 5);
        }
        // Her butonda platform logosu (botun application emojisi) etiketin yanında durur.
        assert_eq!(buttons[0]["emoji"]["id"], SPOTIFY_EMOJI.1);
        assert_eq!(buttons[1]["emoji"]["id"], APPLE_MUSIC_EMOJI.1);
        assert_eq!(buttons[2]["emoji"]["name"], "youtubemusic");
        assert_eq!(buttons[2]["label"], "YouTube Music");
    }

    #[test]
    fn entity_without_artists_or_preview_still_builds() {
        let entity = json!({ "name": "Keşfet", "visualIdentity": { "image": [] } });
        let view = EntityView::from(&entity, "playlist");
        assert_eq!(view.title.as_deref(), Some("Keşfet"));
        assert!(view.subtitle.is_none());
        assert!(
            preview_media(&view).is_none(),
            "önizleme yoksa video da yok"
        );
        assert!(
            build_card(&view, "https://open.spotify.com/playlist/x")
                .video_components
                .is_none()
        );

        let card = build_card(&view, "https://open.spotify.com/playlist/x");
        // Görsel yoksa bölüm erişilebilirliği bir bağlantı butonu olur.
        assert_eq!(card.components[0]["accessory"]["type"], 2);
        // Müzik butonları her zaman vardır (Spotify + Apple Music + YouTube Music).
        assert_eq!(
            card.components[2]["components"].as_array().unwrap().len(),
            3
        );
    }

    #[test]
    fn cached_spotify_card_stays_plain_for_per_send_spoilers() {
        let entity = json!({ "name": "Gizli parça", "artists": [{ "name": "Biri" }] });
        let view = EntityView::from(&entity, "track");
        let card = build_card(&view, "https://open.spotify.com/track/x");
        assert!(
            !card.components[0]["components"][0]["content"]
                .as_str()
                .unwrap()
                .contains("||")
        );
    }

    fn playlist_entity(count: usize) -> Value {
        let tracks: Vec<Value> = (0..count)
            .map(|i| {
                json!({
                    "uri": format!("spotify:track:id{i:02}"),
                    "title": format!("Parça [{i}]"),
                    "subtitle": if i % 2 == 0 { "Sanatçı A" } else { "Liste Sahibi" },
                    "duration": 149_772,
                })
            })
            .collect();
        json!({ "name": "Liste", "subtitle": "Liste Sahibi", "trackList": tracks })
    }

    fn component_with_id(tree: &Value, id: u64) -> Option<&Value> {
        match tree {
            Value::Array(items) => items.iter().find_map(|item| component_with_id(item, id)),
            Value::Object(object) => {
                if object.get("id").and_then(Value::as_u64) == Some(id) {
                    return Some(tree);
                }
                object
                    .values()
                    .find_map(|child| component_with_id(child, id))
            }
            _ => None,
        }
    }

    #[test]
    fn playlist_card_lists_first_page_with_pager() {
        let view = EntityView::from(&playlist_entity(23), "playlist");
        assert_eq!(view.tracks.len(), 23);
        let card = build_card(
            &view,
            "https://open.spotify.com/playlist/0ej4YNptsYtdPEYN5y4bvc",
        );
        let components = Value::Array(card.components.clone());

        let list = component_with_id(&components, TRACK_LIST_COMPONENT_ID).expect("liste");
        let content = list["content"].as_str().unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 10);
        assert_eq!(
            lines[0],
            "`01` [Parça \\[0\\]](https://open.spotify.com/track/id00) — Sanatçı A · 2:29"
        );
        // Liste sahibiyle aynı sanatçı tekrar yazılmaz.
        assert_eq!(
            lines[1],
            "`02` [Parça \\[1\\]](https://open.spotify.com/track/id01) · 2:29"
        );

        let pager = component_with_id(&components, PAGER_COMPONENT_ID).expect("sayfa butonları");
        let buttons = pager["components"].as_array().unwrap();
        assert_eq!(buttons[0]["disabled"], true, "ilk sayfada geri kapalı");
        assert_eq!(buttons[1]["label"], "1/3");
        assert_eq!(
            buttons[2]["custom_id"],
            "spotify:page:playlist:0ej4YNptsYtdPEYN5y4bvc:1"
        );
        assert_eq!(buttons[2]["disabled"], false);
        // Sıra: bölüm, ayraç, liste, sayfa, ayraç, bağlantı butonları.
        assert_eq!(card.components.len(), 6);
        assert_eq!(card.components[5]["components"][0]["label"], "Spotify");
    }

    #[test]
    fn short_lists_have_no_pager_and_tracks_have_no_list() {
        let view = EntityView::from(&playlist_entity(7), "album");
        let card = build_card(
            &view,
            "https://open.spotify.com/album/1DFixLWuPkv3KT3TnV35m3",
        );
        let components = Value::Array(card.components.clone());
        assert!(component_with_id(&components, TRACK_LIST_COMPONENT_ID).is_some());
        assert!(component_with_id(&components, PAGER_COMPONENT_ID).is_none());

        let view = EntityView::from(&playlist_entity(30), "track");
        assert!(view.tracks.is_empty());
    }

    #[test]
    fn last_page_disables_next_and_out_of_range_is_rejected() {
        let view = EntityView::from(&playlist_entity(23), "playlist");
        let (list, pager) = track_page(&view, "playlist", "abc", 2).expect("son sayfa");
        let content = list["content"].as_str().unwrap();
        assert_eq!(content.lines().count(), 3);
        assert!(content.starts_with("`21` "));
        let pager = pager.expect("sayfa butonları");
        assert_eq!(pager["components"][1]["label"], "3/3");
        assert_eq!(pager["components"][2]["disabled"], true);
        assert!(track_page(&view, "playlist", "abc", 3).is_none());
    }

    #[test]
    fn page_custom_id_is_validated() {
        assert_eq!(
            parse_page_custom_id("spotify:page:playlist:0ej4YNptsYtdPEYN5y4bvc:3"),
            Some(("playlist", "0ej4YNptsYtdPEYN5y4bvc", 3))
        );
        for bad in [
            "spotify:page:track:abc:1",
            "spotify:page:playlist:a/b:1",
            "spotify:page:playlist:abc:x",
            "spotify:page:playlist:abc:1:2",
            "spotify:page:playlist:abc:100",
            "spotify:noop",
        ] {
            assert!(parse_page_custom_id(bad).is_none(), "{bad}");
        }
        let longest = format!("{PAGE_CUSTOM_ID_PREFIX}playlist:{}:99", "a".repeat(40));
        assert!(longest.len() <= 100, "Discord custom_id sınırı");
    }

    #[test]
    fn page_swap_touches_only_list_and_pager_and_keeps_spoiler() {
        let view = EntityView::from(&playlist_entity(23), "playlist");
        let card = build_card(
            &view,
            "https://open.spotify.com/playlist/0ej4YNptsYtdPEYN5y4bvc",
        );
        let mut children = card.components.clone();
        // Gönderimde spoiler uygulanmış liste metni.
        children[2]["content"] = json!(format!("||{}||", children[2]["content"].as_str().unwrap()));
        let mut message = json!([
            { "type": 10, "id": 1, "content": "kullanıcı notu" },
            { "type": 17, "id": 2, "components": children },
        ]);

        let (list, pager) = track_page(&view, "playlist", "abc", 1).unwrap();
        assert!(replace_page_components(&mut message, &list, pager.as_ref()));

        assert_eq!(message[0]["content"], "kullanıcı notu");
        let list = component_with_id(&message, TRACK_LIST_COMPONENT_ID).unwrap();
        let content = list["content"].as_str().unwrap();
        assert!(content.starts_with("||`11` "), "{content}");
        assert!(content.ends_with("||"));
        let pager = component_with_id(&message, PAGER_COMPONENT_ID).unwrap();
        assert_eq!(pager["components"][1]["label"], "2/3");
        assert_eq!(pager["components"][0]["disabled"], false);

        let mut without_list = json!([{ "type": 10, "content": "x" }]);
        assert!(!replace_page_components(
            &mut without_list,
            &list.clone(),
            None
        ));
    }
}
