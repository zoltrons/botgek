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
use super::{Author, BuiltinPost, CustomCard, MAX_BODY_BYTES};
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
/// Dış isteklere zaman aşımı.
const TIMEOUT: Duration = Duration::from_secs(8);

// Components V2 bileşen türleri (yalnızca burada kullanılanlar).
const TYPE_ACTION_ROW: u64 = 1;
const TYPE_SECTION: u64 = 9;
const TYPE_TEXT_DISPLAY: u64 = 10;
const TYPE_THUMBNAIL: u64 = 11;
const TYPE_SEPARATOR: u64 = 14;

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
    let embed_url = format!(
        "{SPOTIFY_EMBED}/{}/{}?utm_source=botgek",
        target.kind, target.id
    );
    let html = get_html(client, &embed_url).await?;

    let next = extract_next_data(&html)
        .ok_or_else(|| format!("Spotify verisi çözümlenemedi: {}", parsed.original_url))?;

    let page_props = next.get("props").and_then(|value| value.get("pageProps"));
    if let Some(status) = page_props
        .and_then(|props| props.get("status"))
        .and_then(Value::as_i64)
        && (status == 404 || status == 500)
    {
        return Err(format!("Spotify içeriği bulunamadı: {}", target.open_url));
    }

    let entity = page_props
        .and_then(|props| props.get("state"))
        .and_then(|state| state.get("data"))
        .and_then(|data| data.get("entity"))
        .ok_or_else(|| format!("Spotify varlık verisi yok: {}", target.open_url))?;

    let view = EntityView::from(entity, &target.kind);
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
    Ok(post)
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
}

impl EntityView {
    fn from(entity: &Value, kind: &str) -> Self {
        let title = json_str(entity, "name").or_else(|| json_str(entity, "title"));
        let image = best_image(entity);
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
            .map(|iso| format!("Çıkış {}", &iso[..iso.len().min(10)]));

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
        }
    }
}

/// Components V2 kart gövdesini (bölüm + ayraç + buton satırı) üretir.
///
/// Kart **kaynak verisi** olarak üretilir ve önbelleğe alınır: spoiler görünümü gönderim başına
/// `render::apply_visibility` tarafından uygulanır (aynı gönderi hem spoiler'lı hem spoiler'sız
/// kanalda gösterilebilir).
///
/// Buton satırı: Spotify (asıl) + Apple Music ve YouTube Music aramaları. Discord V2
/// mesajlarında 30 sn önizleme satır içi oynatılamadığı için harici müzik servisleri sunulur.
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
        None => link_button("Spotify", open_url),
    };

    let buttons = vec![
        link_button("Spotify", open_url),
        link_button("Apple Music", &apple_music_search_url(view)),
        link_button("YouTube Music", &youtube_music_search_url(view)),
    ];

    let components = vec![
        json!({
            "type": TYPE_SECTION,
            "components": section_children,
            "accessory": accessory,
        }),
        json!({ "type": TYPE_SEPARATOR, "spacing": 1 }),
        json!({ "type": TYPE_ACTION_ROW, "components": buttons }),
    ];

    CustomCard {
        components,
        accent_color: Some(SPOTIFY_GREEN),
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

/// `{"type": 2, "style": 5, ...}` — bağlantı (link) butonu.
fn link_button(label: &str, url: &str) -> Value {
    json!({ "type": 2, "style": 5, "label": label, "url": url })
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
    }

    #[test]
    fn entity_without_artists_or_preview_still_builds() {
        let entity = json!({ "name": "Keşfet", "visualIdentity": { "image": [] } });
        let view = EntityView::from(&entity, "playlist");
        assert_eq!(view.title.as_deref(), Some("Keşfet"));
        assert!(view.subtitle.is_none());

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
}
