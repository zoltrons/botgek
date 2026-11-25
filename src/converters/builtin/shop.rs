//! Mağaza ürün kartı (Amazon, Trendyol, Hepsiburada ortak). Mağaza modülleri sayfayı
//! [`Product`]'a çevirir; kart burada klasik (v1) embed olarak kurulur. Embed'in alanları
//! gerçek sütundur (künye hizalı durur) ve zaman damgasını izleyicinin cihazı yerel saatle yazar.
//!
//! ```text
//! ▌Amazon.com.tr · MSI                                   [görsel]
//! ▌MSI CYBORG 15 A13VE-2269XTRNN Dizüstü Bilgisayar     (başlık; link değil, link butonda)
//! ▌# 49.911,94 TL                                       (kartın en büyük yazısı)
//! ▌-# ~~56.999,00 TL~~ −%12 · Fırsat rozeti
//! ▌★ 4,3 · 19 değerlendirme
//! ▌Ücretsiz teslimat · 24 Kasım Salı · **Son 1 adet**
//! ▌İşlemci        Ekran kartı      Bellek             (satır içi alanlar, en çok 6)
//! ▌Intel Core i5  RTX4050 6GB      16GB DDR5
//! ▌satıcı Amazon.com.tr · 14 gün iade • Bugün 09:34   (alt bilgi + fiyatın alındığı an)
//! [Amazon'da aç]                                      (kartın altında tek, temiz link)
//! ```

use serde_json::json;

use super::render::escape_markdown as escape;
use super::{Author, BuiltinPost, CustomCard};
use crate::converters::parser::Platform;

/// Mağazadan bağımsız ürün kartı verisi.
#[derive(Debug, Default)]
pub(super) struct Product {
    pub(super) title: String,
    pub(super) clean_url: String,
    /// Kartın üst satırındaki mağaza adı ("Amazon.com.tr", "Trendyol").
    pub(super) store: String,
    pub(super) brand: Option<String>,
    pub(super) deal: Option<String>,
    pub(super) price_now: Option<String>,
    pub(super) price_was: Option<String>,
    pub(super) savings: Option<String>,
    pub(super) rating: Option<String>,
    pub(super) reviews: Option<String>,
    /// Ana teslimat: (ücret, tarih); ücret "ÜCRETSİZ", tutar ya da boş (yalnız tarih).
    pub(super) delivery: Option<(String, String)>,
    pub(super) availability: Option<String>,
    pub(super) seller: Option<String>,
    pub(super) return_days: Option<String>,
    pub(super) image: Option<String>,
    /// Etiketli künye: (etiket, değer).
    pub(super) specs: Vec<(String, String)>,
    pub(super) accent: u32,
    /// Kartın altındaki tek buton ("Amazon'da aç").
    pub(super) button_label: &'static str,
}

/// "83.899" / "82.172,02" → sayı.
pub(super) fn value(amount: &str) -> f64 {
    amount
        .trim_end_matches(" TL")
        .replace('.', "")
        .replace(',', ".")
        .parse()
        .unwrap_or(0.0)
}

/// İki fiyat da sayfada göründüğünde indirim oranı ("−%2"); mağaza oranı ayrıca yazmayabilir.
pub(super) fn savings(now: Option<&str>, was: Option<&str>) -> Option<String> {
    let (now, was) = (value(now?), value(was?));
    let pct = ((was - now) / was * 100.0).round();
    (was > 0.0 && pct >= 1.0).then(|| format!("−%{pct}"))
}

/// Ürün kartlı gönderi.
pub(super) fn product_post(platform: Platform, label: &str, product: &Product) -> BuiltinPost {
    let mut post = BuiltinPost::new(platform, product.clean_url.clone(), Author::new(label, ""));
    post.title = Some(product.title.clone());
    post.extra_label = Some(label.to_string());
    post.custom_card = Some(build_card(product, crate::utils::unix_secs()));
    post
}

pub(super) fn build_card(p: &Product, fetched_at: u64) -> CustomCard {
    // Açıklama: fiyat (en büyük yazı); altında tek küçük satırda eski fiyat · indirim · rozet;
    // puan; teslimat + stok (kalın yalnız aciliyet: "Son N adet").
    let mut lines = Vec::new();
    if let Some(now) = &p.price_now {
        lines.push(format!("# {}", escape(now)));
    }
    let was = [
        p.price_was.as_ref().map(|w| format!("~~{}~~", escape(w))),
        p.savings.clone(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ");
    let note = [
        Some(was).filter(|w| !w.is_empty()),
        p.deal.as_deref().map(escape),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    if !note.is_empty() {
        lines.push(format!("-# {}", note.join(" · ")));
    }
    if let Some(rating) = &p.rating {
        // Düz yıldız (renkli emoji değil): puan satırı fiyatla yarışmasın.
        let mut line = format!("★ {rating}");
        if let Some(reviews) = &p.reviews {
            line.push_str(&format!(" · {reviews} değerlendirme"));
        }
        lines.push(line);
    }
    let mut ship = Vec::new();
    if let Some((fee, date)) = &p.delivery {
        if fee.is_empty() {
            ship.push(escape(date));
        } else if fee.eq_ignore_ascii_case("ÜCRETSİZ") || fee.eq_ignore_ascii_case("FREE") {
            ship.push(format!("Ücretsiz teslimat · {}", escape(date)));
        } else {
            ship.push(format!("Teslimat {} · {}", escape(fee), escape(date)));
        }
    }
    if let Some(av) = &p.availability {
        // Bölünmez boşluk: "Son 18 / adet" diye satır sonunda kırılmasın.
        ship.push(format!("**{}**", escape(av).replace(' ', "\u{a0}")));
    }
    if !ship.is_empty() {
        lines.push(ship.join(" · "));
    }

    let mut author = p.store.clone();
    if let Some(brand) = &p.brand {
        author.push_str(&format!(" · {brand}"));
    }
    let mut embed = json!({
        "color": p.accent,
        "author": { "name": author },
        "title": p.title,
        // Satır içi alanlar gerçek sütundur: künye hizalı durur. Kalın yok (alan adı Discord'da
        // zaten vurgulu); kalın beyaz yalnız başlık, fiyat ve aciliyette.
        "fields": p.specs.iter().map(|(label, value)| {
            json!({ "name": label, "value": escape(value), "inline": true })
        }).collect::<Vec<_>>(),
    });
    if !lines.is_empty() {
        embed["description"] = json!(lines.join("\n"));
    }
    if let Some(url) = &p.image {
        embed["thumbnail"] = json!({ "url": url });
    }
    // Fiyatın alındığı an: Discord zaman damgasını izleyicinin cihazında yerel saatle yazar
    // (bot ile cihaz saati farklı olsa da "10 saniye sonra" gibi kayma görünmez).
    if p.price_now.is_some()
        && let Ok(at) = serenity::model::Timestamp::from_unix_timestamp(fetched_at as i64)
    {
        embed["timestamp"] = json!(at.to_string());
    }
    let mut footer = Vec::new();
    if let Some(seller) = p.seller.as_ref().filter(|s| p.brand.as_ref() != Some(*s)) {
        footer.push(format!("satıcı {seller}"));
    }
    if let Some(days) = &p.return_days {
        footer.push(format!("{days} gün iade"));
    }
    if !footer.is_empty() {
        embed["footer"] = json!({ "text": footer.join(" · ") });
    }

    CustomCard {
        components: Vec::new(),
        accent_color: Some(p.accent),
        video_components: None,
        // Kartın altında tek bağlantı butonu (izleme parametresiz temiz link).
        buttons: vec![json!({
            "type": 1,
            "components": [{ "type": 2, "style": 5, "label": p.button_label, "url": p.clean_url }],
        })],
        embed: Some(embed),
    }
}

/// Embed'in açıklaması (testler ve mağaza modüllerinin testleri için).
#[cfg(test)]
pub(super) fn description(card: &CustomCard) -> String {
    card.embed
        .as_ref()
        .and_then(|e| e["description"].as_str())
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Product {
        let s = |v: &str| Some(v.to_string());
        Product {
            title: "MSI CYBORG 15 A13VE-2269XTRNN Dizüstü Bilgisayar".into(),
            clean_url: "https://www.amazon.com.tr/dp/B0GHR4L88D".into(),
            store: "Amazon.com.tr".into(),
            brand: s("MSI"),
            deal: s("Gülümseten Kasım fırsatı"),
            price_now: s("49.911,94 TL"),
            price_was: s("56.999,00 TL"),
            savings: s("−%12"),
            rating: s("4,3"),
            reviews: s("19"),
            delivery: Some(("ÜCRETSİZ".into(), "24 Kasım Salı".into())),
            availability: s("Son 1 adet"),
            seller: s("Amazon.com.tr"),
            return_days: s("14"),
            image: s("https://m.media-amazon.com/images/I/71l4M2y8zwL.jpg"),
            specs: vec![("İşlemci".into(), "Intel Core I5-13420H".into())],
            accent: 0xFF9900,
            button_label: "Amazon'da aç",
        }
    }

    #[test]
    fn card_is_an_embed_with_the_concept_layout() {
        let card = build_card(&sample(), 1_700_000_000);
        assert!(card.components.is_empty(), "V2 bileşeni yok");
        let e = card.embed.as_ref().unwrap();
        assert_eq!(e["author"]["name"], "Amazon.com.tr · MSI");
        assert_eq!(
            e["title"],
            "MSI CYBORG 15 A13VE-2269XTRNN Dizüstü Bilgisayar"
        );
        assert_eq!(e["color"], 0xFF9900);
        assert_eq!(
            description(&card),
            "# 49.911,94 TL\n\
             -# ~~56.999,00 TL~~ −%12 · Gülümseten Kasım fırsatı\n\
             ★ 4,3 · 19 değerlendirme\n\
             Ücretsiz teslimat · 24 Kasım Salı · **Son\u{a0}1\u{a0}adet**"
        );
        assert_eq!(
            e["fields"],
            json!([{ "name": "İşlemci", "value": "Intel Core I5-13420H", "inline": true }])
        );
        assert_eq!(
            e["thumbnail"]["url"],
            "https://m.media-amazon.com/images/I/71l4M2y8zwL.jpg"
        );
        assert_eq!(e["footer"]["text"], "satıcı Amazon.com.tr · 14 gün iade");
        assert_eq!(e["timestamp"], "2023-11-14T22:13:20Z");
        let row = card.buttons[0]["components"].as_array().unwrap();
        assert_eq!(row.len(), 1);
        assert_eq!(row[0]["label"], "Amazon'da aç");
        assert_eq!(row[0]["url"], "https://www.amazon.com.tr/dp/B0GHR4L88D");
    }

    #[test]
    fn bold_is_reserved_for_urgency() {
        let text = description(&build_card(&sample(), 1));
        assert_eq!(text.matches("**").count(), 2, "{text}");
    }

    #[test]
    fn missing_fields_drop_their_parts_only() {
        let p = Product {
            price_now: None,
            price_was: None,
            savings: None,
            deal: None,
            image: None,
            seller: None,
            return_days: None,
            availability: None,
            specs: Vec::new(),
            ..sample()
        };
        let card = build_card(&p, 1);
        let e = card.embed.as_ref().unwrap();
        assert_eq!(
            description(&card),
            "★ 4,3 · 19 değerlendirme\nÜcretsiz teslimat · 24 Kasım Salı"
        );
        for key in ["thumbnail", "footer", "timestamp"] {
            assert!(e.get(key).is_none(), "{key}");
        }
        assert_eq!(e["fields"], json!([]));
        assert_eq!(card.buttons.len(), 1);
    }

    #[test]
    fn seller_same_as_brand_is_not_repeated() {
        let p = Product {
            seller: Some("MSI".into()),
            return_days: None,
            ..sample()
        };
        let card = build_card(&p, 1);
        assert!(card.embed.as_ref().unwrap().get("footer").is_none());
    }
}
