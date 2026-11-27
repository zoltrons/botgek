//! Görsel galerisi sayfalama: galeri sınırından (10) fazla görseli olan gönderilerde ◀ ▶ butonlarıyla
//! galerinin içeriği yerinde değiştirilir. Durum mesajda tutulmaz: `custom_id` sayfa ile
//! gönderi bağlantısını taşır, tıklamada gönderi (önbellekten) yeniden alınır.
//!
//! Reddit metin sayfalama: kartta kısaltılan gövdenin tamamı "Devamını oku" ile yalnız tıklayana
//! görünen bir mesajda ◀ ▶ sayfalarıyla okunur (bkz. [`text_page`]); kart değişmez.

use serde_json::{Value, json};

use super::{BuiltinPost, MediaKind, reddit, render::escape_markdown};
use crate::config::Config;
use crate::converters::parser::{Platform, parse_message};
use crate::settings::EngineSettings;
use crate::utils::truncate_chars;

const TYPE_ACTION_ROW: u64 = 1;
/// Yerinde güncellenen bileşenlerin `id`'leri (bkz. `miniflux::BODY_COMPONENT_ID`).
pub const GALLERY_COMPONENT_ID: u64 = 7411;
const PAGER_COMPONENT_ID: u64 = 7412;
/// `imgs:{sayfa}:{bağlantı (https:// olmadan)}`; Discord `custom_id` sınırı 100 karakterdir.
pub const CUSTOM_ID_PREFIX: &str = "imgs:";
const CUSTOM_ID_LIMIT: usize = 100;
/// Media Gallery en fazla 10 öğe alır: 10 görsele kadar tek galeri, fazlası ek sayfalarda.
const GALLERY_LIMIT: usize = 10;
/// `rtext:{sayfa}:{s = gizli | a = açık}:{reddit::compact_path}`.
pub const TEXT_CUSTOM_ID_PREFIX: &str = "rtext:";
/// Embed açıklaması en çok 4096 karakter; gizli metinde başlık (≤ 300) da açıklamaya girer.
const TEXT_PAGE_CHARS: usize = 3500;
const EMBED_TITLE_LIMIT: usize = 256;

/// Gönderinin sayfalanabilir görselleri: yalnızca hepsi düz görselse (video/GIF yoksa).
pub fn all_images(post: &BuiltinPost) -> Vec<&str> {
    if post
        .media
        .iter()
        .any(|media| media.kind != MediaKind::Image)
    {
        return Vec::new();
    }
    post.media.iter().map(|media| media.url.as_str()).collect()
}

/// Sayfa butonu satırı; sayfalanacak bir şey yoksa ya da `custom_id` sığmıyorsa `None`.
pub fn pager_row(post: &BuiltinPost, page: usize) -> Option<Value> {
    let total = all_images(post).len();
    let pages = total.div_ceil(GALLERY_LIMIT);
    let rest = post.url.strip_prefix("https://")?;
    let target = |page: usize| format!("{CUSTOM_ID_PREFIX}{page}:{rest}");
    if pages < 2 || target(pages - 1).len() > CUSTOM_ID_LIMIT {
        return None;
    }
    let mut row = nav_row(page, pages, target, &format!("{CUSTOM_ID_PREFIX}i"));
    row["id"] = json!(PAGER_COMPONENT_ID);
    Some(row)
}

/// ◀ n/N ▶ satırı; `target` sayfa düğmesinin `custom_id`'sini, `indicator` ortadaki pasif düğmeninkini verir.
fn nav_row(page: usize, pages: usize, target: impl Fn(usize) -> String, indicator: &str) -> Value {
    let button = |label: &str, id: String, disabled: bool| json!({ "type": 2, "style": 2, "label": label, "custom_id": id, "disabled": disabled });
    json!({
        "type": TYPE_ACTION_ROW,
        "components": [
            button("◀", target(page.saturating_sub(1)), page == 0),
            button(&format!("{}/{pages}", page + 1), indicator.to_string(), true),
            button("▶", target((page + 1).min(pages - 1)), page + 1 >= pages),
        ],
    })
}

/// Buton tıklaması: gönderiyi yeniden alır, galeri öğelerini ve sayfa satırını değiştirir.
pub async fn turn_page(
    client: &reqwest::Client,
    config: &Config,
    engine: &EngineSettings,
    custom_id: &str,
    components: &mut Value,
) -> Result<(), String> {
    let (page, rest) = custom_id
        .strip_prefix(CUSTOM_ID_PREFIX)
        .and_then(|tail| tail.split_once(':'))
        .ok_or("geçersiz sayfa kimliği")?;
    let page: usize = page.parse().map_err(|_| "geçersiz sayfa")?;
    let parsed = parse_message(&format!("https://{rest}")).ok_or("bağlantı çözülemedi")?;
    let post = super::fetch(client, &parsed, engine, config).await?;
    let images = all_images(&post);
    let row = pager_row(&post, page).ok_or("sayfalanacak görsel yok")?;
    let slice = images
        .get(page * GALLERY_LIMIT..)
        .filter(|s| !s.is_empty())
        .ok_or("sayfa yok")?;
    let slice = &slice[..slice.len().min(GALLERY_LIMIT)];
    if swap(components, slice, &row) {
        Ok(())
    } else {
        Err("galeri mesajda bulunamadı".into())
    }
}

/// Metni en fazla `max` karakterlik sayfalara böler; kesim önce paragraf, sonra satır, sonra
/// boşluk sınırında yapılır.
pub fn paginate(text: &str, max: usize) -> Vec<String> {
    let mut pages = Vec::new();
    let mut rest = text.trim();
    while rest.chars().count() > max {
        let limit = rest.char_indices().nth(max).map_or(rest.len(), |(i, _)| i);
        let window = &rest[..limit];
        let half = window.len() / 2;
        let cut = ["\n\n", "\n", " "]
            .iter()
            .find_map(|sep| window.rfind(sep).filter(|at| *at >= half))
            .unwrap_or(limit);
        pages.push(rest[..cut].trim().to_string());
        rest = rest[cut..].trim_start();
    }
    pages.push(rest.to_string());
    pages
}

/// Kısaltılmış Reddit gövdesinin altındaki "Devamını oku" satırı; gönderi yolu çıkarılamazsa `None`.
pub fn read_more_row(post: &BuiltinPost, hidden: bool) -> Option<Value> {
    let path = reddit::compact_path(&post.url)?;
    let flag = if hidden { 's' } else { 'a' };
    Some(json!({
        "type": TYPE_ACTION_ROW,
        "components": [{
            "type": 2, "style": 2, "label": "Devamını oku",
            "custom_id": format!("{TEXT_CUSTOM_ID_PREFIX}0:{flag}:{path}"),
        }],
    }))
}

/// "Devamını oku" ya da sayfa tıklaması: gönderiyi yeniden alır, istenen sayfanın mesaj gövdesini döndürür.
pub async fn text_page(
    client: &reqwest::Client,
    config: &Config,
    engine: &EngineSettings,
    custom_id: &str,
) -> Result<Value, String> {
    let mut parts = custom_id
        .strip_prefix(TEXT_CUSTOM_ID_PREFIX)
        .ok_or("geçersiz metin kimliği")?
        .splitn(3, ':');
    let (Some(page), Some(flag), Some(path)) = (parts.next(), parts.next(), parts.next()) else {
        return Err("geçersiz metin kimliği".into());
    };
    let page: usize = page.parse().map_err(|_| "geçersiz sayfa")?;
    let url = format!("https://www.reddit.com/{path}");
    // Kimlik doğrulaması: yol, kartın ürettiği kısa biçimle birebir aynı olmalı.
    if reddit::compact_path(&url).as_deref() != Some(path) {
        return Err("geçersiz Reddit yolu".into());
    }
    let parsed = parse_message(&url)
        .filter(|parsed| parsed.platform == Platform::Reddit)
        .ok_or("bağlantı çözülemedi")?;
    let post = super::fetch(client, &parsed, engine, config).await?;
    let pages = paginate(&post.text, TEXT_PAGE_CHARS);
    Ok(text_payload(&post, &pages, page, flag == "s", path))
}

fn text_payload(
    post: &BuiltinPost,
    pages: &[String],
    page: usize,
    spoiler: bool,
    path: &str,
) -> Value {
    let page = page.min(pages.len() - 1);
    let title = post
        .title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or(&post.author.name);
    // Embed başlığı spoiler'a alınamaz: gizli gönderide başlık da metinle birlikte gizlenir.
    let embed = if spoiler {
        let body = format!("**{}**\n{}", escape_markdown(title), pages[page]);
        json!({ "description": format!("||{}||", body.replace("||", "")) })
    } else {
        json!({
            "title": truncate_chars(title, EMBED_TITLE_LIMIT),
            "url": post.url,
            "description": pages[page],
        })
    };
    let mut components = Vec::new();
    if pages.len() > 1 {
        let flag = if spoiler { 's' } else { 'a' };
        let target = |to: usize| format!("{TEXT_CUSTOM_ID_PREFIX}{to}:{flag}:{path}");
        components.push(nav_row(
            page,
            pages.len(),
            target,
            &format!("{TEXT_CUSTOM_ID_PREFIX}i"),
        ));
    }
    json!({ "embeds": [embed], "components": components, "allowed_mentions": { "parse": [] } })
}

/// Bileşen ağacında galeri öğelerini (spoiler durumu korunarak) ve sayfa satırını değiştirir.
fn swap(tree: &mut Value, urls: &[&str], row: &Value) -> bool {
    match tree {
        Value::Array(items) => items
            .iter_mut()
            .fold(false, |f, item| swap(item, urls, row) | f),
        Value::Object(object) => {
            match object.get("id").and_then(Value::as_u64) {
                Some(GALLERY_COMPONENT_ID) => {
                    let spoiler = object["items"][0]["spoiler"].as_bool().unwrap_or(false);
                    object.insert(
                        "items".into(),
                        urls.iter()
                            .map(|url| json!({ "media": { "url": url }, "spoiler": spoiler }))
                            .collect(),
                    );
                    return true;
                }
                Some(PAGER_COMPONENT_ID) => {
                    *tree = row.clone();
                    return false;
                }
                _ => {}
            }
            object
                .values_mut()
                .fold(false, |f, child| swap(child, urls, row) | f)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::converters::builtin::{Author, Media};
    use crate::converters::parser::Platform;

    fn post(count: usize) -> BuiltinPost {
        let mut post = BuiltinPost::new(
            Platform::Bluesky,
            "https://bsky.app/profile/a.bsky.social/post/3abc",
            Author::new("a", "a"),
        );
        post.media = (0..count)
            .map(|i| Media {
                alt: None,
                kind: MediaKind::Image,
                url: format!("https://cdn/{i}.jpg"),
                poster: None,
                width: None,
                height: None,
                duration: None,
            })
            .collect();
        post
    }

    #[test]
    fn pager_only_for_more_images_than_page_size() {
        assert!(pager_row(&post(10), 0).is_none());
        let row = pager_row(&post(25), 0).unwrap();
        assert_eq!(row["components"][1]["label"], "1/3");
        assert_eq!(row["components"][0]["disabled"], true);
        let last = pager_row(&post(25), 2).unwrap();
        assert_eq!(last["components"][2]["disabled"], true);
    }

    #[test]
    fn reddit_text_pages_and_spoiler() {
        assert_eq!(
            reddit::compact_path("https://www.reddit.com/r/a/comments/abc/baslik/def/").as_deref(),
            Some("comments/abc/_/def")
        );
        let url = "https://www.reddit.com/comments/abc/_/def";
        assert_eq!(
            reddit::compact_path(url).as_deref(),
            Some("comments/abc/_/def")
        );
        assert_eq!(
            parse_message(url).map(|p| p.platform),
            Some(Platform::Reddit)
        );

        let mut post = post(0);
        post.title = Some("Başlık".into());
        let pages = vec!["bir".to_string(), "iki".to_string()];
        let open = text_payload(&post, &pages, 0, false, "comments/abc");
        assert_eq!(open["embeds"][0]["title"], "Başlık");
        assert_eq!(open["embeds"][0]["description"], "bir");
        let row = &open["components"][0]["components"];
        assert_eq!(row[0]["disabled"], true);
        assert_eq!(row[2]["custom_id"], "rtext:1:a:comments/abc");

        let hidden = text_payload(&post, &pages, 9, true, "comments/abc");
        assert!(hidden["embeds"][0].get("title").is_none());
        assert_eq!(hidden["embeds"][0]["description"], "||**Başlık**\niki||");
        assert_eq!(hidden["components"][0]["components"][2]["disabled"], true);
    }

    #[test]
    fn swap_replaces_items_and_keeps_spoiler() {
        let mut tree = json!([{ "type": 12, "id": GALLERY_COMPONENT_ID,
            "items": [{ "media": { "url": "a" }, "spoiler": true }] },
            { "type": 1, "id": PAGER_COMPONENT_ID, "components": [] }]);
        assert!(swap(
            &mut tree,
            &["x", "y"],
            &json!({ "type": 1, "id": PAGER_COMPONENT_ID })
        ));
        assert_eq!(tree[0]["items"][1]["media"]["url"], "y");
        assert_eq!(tree[0]["items"][1]["spoiler"], true);
    }
}
