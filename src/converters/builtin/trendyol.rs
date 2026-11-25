//! Trendyol ürün bağlantısı (`trendyol.com/{marka}/{ad}-p-{id}`).
//!
//! Trendyol botun isteğine 403 döner; sayfa datacat vekilinden (`get_markdown`) Markdown olarak okunur.
//! Fiyat, puan, stok, teslimat, satıcı ve künye bu metinden çıkarılır ve Amazon'un onaylı ürün
//! kartı (`amazon::build_card`, Components V2) aynen kullanılır; yalnız renk ve buton değişir.
//!
//! shortcut: satır düzeni datacat'in Markdown çıktısına ve Trendyol'un sayfa düzenine bağlı;
//! düzen değişirse ilgili alan sessizce boş kalır (başlık bulunamazsa kart hiç çıkmaz).

use std::sync::LazyLock;

use regex::Regex;

use super::BuiltinPost;
use super::amazon::{Product, product_post};
use crate::converters::parser::{ParsedPost, Platform};
use crate::utils::truncate_chars;

const TRENDYOL_ORANGE: u32 = 0xF27A1A;
const MAX_TITLE_CHARS: usize = 150;
const MAX_SPECS: usize = 6;
const MAX_SPEC_KEY: usize = 22;
const MAX_SPEC_VALUE: usize = 30;

/// `# [**Marka**](marka-linki) Ürün adı`
static TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^# \[\*\*(.+?)\*\*\]\([^)]*\)\s*(.+)$").expect("geçerli başlık regex")
});
/// Ürün görselinin küçültülmüş adresi; `mnresize/G/Y/` atılınca özgün boyut kalır.
static IMAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"https://cdn\.dsmcdn\.com/mnresize/[0-9]+/[0-9]+/(ty[0-9]+/(?:[a-z]+/)*prod/[^)\s]+)",
    )
    .expect("geçerli görsel regex")
});
static RATING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[0-5][.,][0-9]$").expect("geçerli puan regex"));
static REVIEWS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\*\*([0-9][0-9.]*)\*\* Değerlendirme").expect("geçerli yorum regex")
});
static LOW_STOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^Son ([0-9]+) Ürün!?$").expect("geçerli stok regex"));
/// Yalnızca tutardan oluşan fiyat satırı: "82.172,02 TL", "Sepette 116,42 TL", "122,55 TL129 TL".
/// Taksit ("9x 9.130,22 TL") ve kredi satırları bu kalıba uymaz.
static PRICE_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:Sepette\s+)?(?:[0-9][0-9.]*(?:,[0-9]+)? TL\s*)+$")
        .expect("geçerli fiyat satırı regex")
});
static AMOUNT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([0-9][0-9.]*(?:,[0-9]+)?) TL").expect("geçerli tutar regex"));
static SELLER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[([^\]\[]+)\]\(https://www\.trendyol\.com/magaza/").expect("geçerli satıcı regex")
});
/// Değerlendirme linkindeki asıl ürün yolu (eski id yeni ürüne yönlenebilir).
static CANONICAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https://www\.trendyol\.com/([a-z0-9-]+/[a-z0-9-]+-p-[0-9]+)/yorumlar")
        .expect("geçerli canonical regex")
});

pub async fn fetch(client: &reqwest::Client, parsed: &ParsedPost) -> Result<BuiltinPost, String> {
    let markdown = super::get_markdown(client, &parsed.original_url).await?;
    let product = parse_product(&markdown, &parsed.original_url)
        .ok_or_else(|| format!("Trendyol ürünü okunamadı: {}", parsed.original_url))?;
    Ok(product_post(Platform::Trendyol, "Trendyol", &product))
}

fn parse_product(markdown: &str, source_url: &str) -> Option<Product> {
    let lines: Vec<&str> = markdown
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let title_at = lines.iter().position(|l| TITLE.is_match(l))?;
    let caps = TITLE.captures(lines[title_at])?;
    let (before, after) = (&lines[..title_at], &lines[title_at + 1..]);

    // Satın alma bloğu: başlıktan ek hizmetlere / sepet butonuna kadar.
    let buy_end = after
        .iter()
        .position(|l| l.starts_with("Ek Hizmetler") || l.starts_with("Şimdi Al"))
        .unwrap_or(after.len());
    let (price_now, price_was) = prices(&after[..buy_end]);
    // Teslimat bilgisi sepet butonundan sonra, ilk "##" bölümünden önce.
    let info_end = after
        .iter()
        .position(|l| l.starts_with("## "))
        .unwrap_or(after.len());
    let info = &after[..info_end];
    let date = info
        .iter()
        .find(|l| l.contains("kapında"))
        .map(|l| l.replace("**", ""));
    let free = info.contains(&"Ücretsiz Kargo");

    Some(Product {
        // "… Kedi Damla 50 ml | Anti Hairball | Biotin Damla": "|" sonrası arama etiketidir.
        title: truncate_chars(
            caps[2].split(" | ").next().unwrap_or("").trim(),
            MAX_TITLE_CHARS,
        ),
        clean_url: CANONICAL
            .captures(markdown)
            .map(|c| format!("https://www.trendyol.com/{}", &c[1]))
            .unwrap_or_else(|| source_url.to_string()),
        store: "Trendyol".to_string(),
        brand: Some(truncate_chars(caps[1].trim(), 30)),
        deal: after[..buy_end]
            .contains(&"Kampanyalı Fiyat")
            .then(|| "Kampanyalı fiyat".to_string()),
        savings: savings(price_now.as_deref(), price_was.as_deref()),
        price_now,
        price_was,
        rating: after
            .first()
            .filter(|l| RATING.is_match(l))
            .map(|l| l.replace('.', ",")),
        reviews: after[..buy_end]
            .iter()
            .find_map(|l| REVIEWS.captures(l))
            .map(|c| c[1].to_string()),
        delivery: date.map(|d| {
            let fee = if free { "ÜCRETSİZ" } else { "" };
            (fee.to_string(), d)
        }),
        availability: before
            .iter()
            .find_map(|l| LOW_STOCK.captures(l))
            .map(|c| format!("Son {} adet", &c[1])),
        seller: SELLER.captures(markdown).map(|c| c[1].trim().to_string()),
        return_days: None,
        image: before
            .iter()
            .find_map(|l| IMAGE.captures(l))
            .map(|c| format!("https://cdn.dsmcdn.com/{}", &c[1])),
        specs: specs(after),
        accent: TRENDYOL_ORANGE,
        button_label: "Trendyol'da aç",
    })
}

/// Ödenecek fiyat ilk tutar, eski fiyat (daha yüksekse) son tutar. "Trendyol Plus'a Özel" fiyat
/// yalnız üyelere geçerli olduğu için atlanır.
fn prices(block: &[&str]) -> (Option<String>, Option<String>) {
    let mut amounts = Vec::new();
    let mut skip_next = false;
    for line in block {
        if line.contains("Plus") {
            skip_next = true;
        } else if PRICE_LINE.is_match(line) {
            if std::mem::take(&mut skip_next) {
                continue;
            }
            amounts.extend(AMOUNT.captures_iter(line).map(|c| c[1].to_string()));
        }
    }
    let now = amounts.first().cloned();
    let was = amounts
        .last()
        .filter(|w| value(w) > now.as_deref().map_or(f64::MAX, value))
        .cloned();
    (
        now.map(|n| format!("{n} TL")),
        was.map(|w| format!("{w} TL")),
    )
}

/// "83.899" / "82.172,02" → sayı.
fn value(amount: &str) -> f64 {
    amount
        .trim_end_matches(" TL")
        .replace('.', "")
        .replace(',', ".")
        .parse()
        .unwrap_or(0.0)
}

/// İki fiyat da sayfada göründüğünde indirim oranı ("−%2"); Trendyol oranı ayrıca yazmaz.
fn savings(now: Option<&str>, was: Option<&str>) -> Option<String> {
    let (now, was) = (value(now?), value(was?));
    let pct = ((was - now) / was * 100.0).round();
    (was > 0.0 && pct >= 1.0).then(|| format!("−%{pct}"))
}

/// İlk "… Özellikleri" bölümündeki `anahtar` / `değer` satır çiftleri, kısa olanlar.
fn specs(lines: &[&str]) -> Vec<(String, String)> {
    let Some(start) = lines
        .iter()
        .position(|l| l.starts_with('#') && l.ends_with(" Özellikleri"))
    else {
        return Vec::new();
    };
    let rows: Vec<&str> = lines[start + 1..]
        .iter()
        .take_while(|l| !l.starts_with('#'))
        .filter(|l| !l.starts_with("!["))
        .copied()
        .collect();
    rows.as_chunks::<2>()
        .0
        .iter()
        .filter(|kv| {
            kv[0].chars().count() <= MAX_SPEC_KEY
                && kv[1].chars().count() <= MAX_SPEC_VALUE
                && kv[0] != "Menşei"
        })
        .map(|kv| (kv[0].to_string(), kv[1].to_string()))
        .take(MAX_SPECS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::converters::builtin::amazon::build_card;

    /// Gerçek bir Trendyol ürün sayfasının datacat çıktısından kısaltılmış örnek.
    const SAMPLE: &str = r#"URL Source: https://www.trendyol.com/apple/iphone-16-pro-max-256gb-siyah-titanyum-p-857296077
Markdown Content:
[![marketing info](https://cdn.dsmcdn.com/ty1000506/1795422012547-eg_4.gun.gif)](https://www.trendyol.com/sr)

![Yenilenmiş iPhone 16 Pro Max](https://cdn.dsmcdn.com/mnresize/620/920/ty1804/prod/QC_PREP/20251225/16/8a98/1_org_zoom.jpg)

500 TL Kupon Fırsatı!

Son 1 Ürün!

# [**Apple**](https://www.trendyol.com/apple-x-b101470) Yenilenmiş iPhone 16 Pro Max 256 GB Siyah Titanyum Cep Telefonu (12 Ay Garantili) - B Kalite

4.6

[**158** Değerlendirme![fotoğraflı yorum iconu](https://cdn.dsmcdn.com/x.png)](https://www.trendyol.com/apple/yenilenmis-iphone-16-pro-max-p-1075819285/yorumlar?boutiqueId=61&merchantId=451060)

Kampanyalı Fiyat

Sepette

82.172,02 TL

83.899 TL

Peşin Fiyatına **9 Taksit**9x **9.130,22 TL**

Ek Hizmetler

3.918 TL

Şimdi Al Sepete Ekle Son 1 ÜrünSepete Eklendi

Ücretsiz Kargo

Tahmini **2 gün içerisinde** kapında

Alışveriş Kredisi3 ay 29.518 TL'den başlayan ödeme fırsatları

## Öne Çıkan Özellikler:

## Apple Yenilenmiş iPhone 16 Pro Max Özellikleri

Dahili Hafıza

256 GB

Ana Kamera Çözünürlük Aralığı

10 - 15 MP

Garanti Süresi

1 Yıl

Ekran Boyutu

6,7 inç

## Ürünün Kampanyaları

[Elpafix](https://www.trendyol.com/magaza/elpafix-m-451060)
"#;

    fn sample() -> Product {
        parse_product(SAMPLE, "https://www.trendyol.com/apple/x-p-857296077").unwrap()
    }

    #[test]
    fn parses_every_card_field() {
        let p = sample();
        assert_eq!(
            p.title,
            "Yenilenmiş iPhone 16 Pro Max 256 GB Siyah Titanyum Cep Telefonu (12 Ay Garantili) - B Kalite"
        );
        assert_eq!(p.brand.as_deref(), Some("Apple"));
        assert_eq!(
            p.clean_url,
            "https://www.trendyol.com/apple/yenilenmis-iphone-16-pro-max-p-1075819285"
        );
        assert_eq!(p.price_now.as_deref(), Some("82.172,02 TL"));
        assert_eq!(p.price_was.as_deref(), Some("83.899 TL"));
        assert_eq!(p.savings.as_deref(), Some("−%2"));
        assert_eq!(p.deal.as_deref(), Some("Kampanyalı fiyat"));
        assert_eq!(p.rating.as_deref(), Some("4,6"));
        assert_eq!(p.reviews.as_deref(), Some("158"));
        assert_eq!(p.availability.as_deref(), Some("Son 1 adet"));
        assert_eq!(
            p.delivery,
            Some(("ÜCRETSİZ".into(), "Tahmini 2 gün içerisinde kapında".into()))
        );
        assert_eq!(p.seller.as_deref(), Some("Elpafix"));
        assert_eq!(
            p.image.as_deref(),
            Some("https://cdn.dsmcdn.com/ty1804/prod/QC_PREP/20251225/16/8a98/1_org_zoom.jpg")
        );
        // Uzun anahtar ("Ana Kamera Çözünürlük Aralığı") atlanır.
        assert_eq!(
            p.specs,
            vec![
                ("Dahili Hafıza".into(), "256 GB".into()),
                ("Garanti Süresi".into(), "1 Yıl".into()),
                ("Ekran Boyutu".into(), "6,7 inç".into()),
            ]
        );
    }

    #[test]
    fn plus_only_price_is_skipped_and_merged_prices_split() {
        let md = SAMPLE.replace(
            "Kampanyalı Fiyat\n\nSepette\n\n82.172,02 TL\n\n83.899 TL",
            "Trendyol Plus'a Özel\n\nSepette 116,42 TL\n\n122,55 TL129 TL",
        );
        let p = parse_product(&md, "u").unwrap();
        assert_eq!(p.price_now.as_deref(), Some("122,55 TL"));
        assert_eq!(p.price_was.as_deref(), Some("129 TL"));
        assert_eq!(p.savings.as_deref(), Some("−%5"));
        assert!(p.deal.is_none());
    }

    #[test]
    fn conditional_shipping_shows_only_the_date_and_card_uses_trendyol_button() {
        let md = SAMPLE.replace("Ücretsiz Kargo", "350 TL ve Üzeri Kargo Bedava");
        let p = parse_product(&md, "u").unwrap();
        assert_eq!(p.delivery.as_ref().unwrap().0, "");
        let card = build_card(&p, 0);
        let all = serde_json::to_string(&card.components).unwrap();
        assert!(all.contains("Tahmini 2 gün içerisinde kapında · **Son\u{a0}1\u{a0}adet**"));
        assert!(!all.contains("Ücretsiz teslimat"));
        assert_eq!(card.accent_color, Some(TRENDYOL_ORANGE));
        assert_eq!(card.buttons[0]["components"][0]["label"], "Trendyol'da aç");
    }

    #[test]
    fn missing_product_page_is_rejected() {
        assert!(parse_product("Aradığınız Sayfa Bulunamadı", "u").is_none());
    }
}
