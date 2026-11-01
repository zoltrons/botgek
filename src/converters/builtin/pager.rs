//! Görsel galerisi sayfalama: galeri sınırından (10) fazla görseli olan gönderilerde ◀ ▶ butonlarıyla
//! galerinin içeriği yerinde değiştirilir. Durum mesajda tutulmaz: `custom_id` sayfa ile
//! gönderi bağlantısını taşır, tıklamada gönderi (önbellekten) yeniden alınır.

use serde_json::{Value, json};

use super::{BuiltinPost, MediaKind};
use crate::config::Config;
use crate::converters::parser::parse_message;
use crate::settings::EngineSettings;

const TYPE_ACTION_ROW: u64 = 1;
/// Yerinde güncellenen bileşenlerin `id`'leri (bkz. `miniflux::BODY_COMPONENT_ID`).
pub const GALLERY_COMPONENT_ID: u64 = 7411;
const PAGER_COMPONENT_ID: u64 = 7412;
/// `imgs:{sayfa}:{bağlantı (https:// olmadan)}`; Discord `custom_id` sınırı 100 karakterdir.
pub const CUSTOM_ID_PREFIX: &str = "imgs:";
const CUSTOM_ID_LIMIT: usize = 100;
/// Media Gallery en fazla 10 öğe alır: 10 görsele kadar tek galeri, fazlası ek sayfalarda.
const GALLERY_LIMIT: usize = 10;

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
    let button = |label: &str, id: String, disabled: bool| json!({ "type": 2, "style": 2, "label": label, "custom_id": id, "disabled": disabled });
    Some(json!({
        "type": TYPE_ACTION_ROW,
        "id": PAGER_COMPONENT_ID,
        "components": [
            button("◀", target(page.saturating_sub(1)), page == 0),
            button(&format!("{}/{pages}", page + 1), format!("{CUSTOM_ID_PREFIX}i"), true),
            button("▶", target((page + 1).min(pages - 1)), page + 1 >= pages),
        ],
    }))
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
