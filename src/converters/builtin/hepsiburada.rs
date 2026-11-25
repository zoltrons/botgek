//! Hepsiburada ürün bağlantısı (`hepsiburada.com/{ad}-p-{SKU}` ve `-pm-{SKU}`).
//!
//! Sayfa datacat vekilinden (`read_markdown`) Markdown olarak okunur; fiyat, puan, teslimat, satıcı
//! ve künye bu metinden çıkarılır ve ortak mağaza kartı (`shop::build_card`) kullanılır.
//!
//! shortcut: satır düzeni datacat'in Markdown çıktısına ve Hepsiburada'nın sayfa düzenine bağlı;
//! düzen değişirse ilgili alan sessizce boş kalır (başlık bulunamazsa kart hiç çıkmaz).

use std::sync::LazyLock;

use regex::Regex;

use super::BuiltinPost;
use super::shop::{MAX_TITLE_CHARS, Product, RATING, REVIEWS, product_post, savings, value};
use crate::converters::parser::{ParsedPost, Platform};
use crate::utils::truncate_chars;

const HEPSIBURADA_ORANGE: u32 = 0xFF6000;
const MAX_SPECS: usize = 6;
const MAX_SPEC_KEY: usize = 24;
const MAX_SPEC_VALUE: usize = 30;

/// `# [Marka](/marka) Ürün adı`
static TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^# \[([^\]]+)\]\([^)]*\)\s*(.+)$").expect("geçerli başlık regex")
});
/// Galerideki ilk ürün görseli; boyut parçası (`424-600`) `1500` ile değişince büyük boy gelir.
static IMAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"https://productimages\.hepsiburada\.net/s/([0-9]+)/[0-9-]+/([0-9]+\.(?:jpg|jpeg|png|webp))",
    )
    .expect("geçerli görsel regex")
});
static SELLER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^Satıcı: \[([^\]]+)\]\(/magaza/").expect("geçerli satıcı regex"));
/// Yalnızca tutardan oluşan satır ("79.380,00 TL", "83.300 TL"); varyant ("1 TB72.999,00 TL")
/// ve taksit satırları uymaz.
static PRICE_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[0-9][0-9.]*(?:,[0-9]+)? TL$").expect("geçerli fiyat satırı regex")
});
/// Seçili varyant: "SSD Kapasitesi: 512 GB".
static VARIANT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([^:\[\]!*]+): (\S.*)$").expect("geçerli varyant regex"));

pub async fn fetch(client: &reqwest::Client, parsed: &ParsedPost) -> Result<BuiltinPost, String> {
    let url = &parsed.original_url;
    let product = super::read_markdown(client, url, |markdown| {
        parse_product(markdown, url).ok_or_else(|| format!("Hepsiburada ürünü okunamadı: {url}"))
    })
    .await?;
    Ok(product_post(Platform::Hepsiburada, "Hepsiburada", &product))
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
        .position(|l| l.starts_with("Ek hizmet") || *l == "Sepete ekle")
        .unwrap_or(after.len());
    let buy = &after[..buy_end];
    // Ödenecek fiyat ilk tutar satırı; hemen altındaki daha yüksek tutar eski fiyattır
    // (sonraki "3.920 TL" kazanç tutarıdır, eski fiyat değil).
    let first = buy.iter().position(|l| PRICE_LINE.is_match(l));
    let price_now = first.map(|i| buy[i].to_string());
    let price_was = first
        .and_then(|i| buy.get(i + 1))
        .filter(|l| PRICE_LINE.is_match(l))
        .filter(|l| value(l) > price_now.as_deref().map_or(f64::MAX, value))
        .map(|l| l.to_string());

    Some(Product {
        title: truncate_chars(caps[2].trim(), MAX_TITLE_CHARS),
        clean_url: source_url.to_string(),
        store: "Hepsiburada".to_string(),
        brand: Some(truncate_chars(caps[1].trim(), 30)),
        deal: None,
        savings: savings(price_now.as_deref(), price_was.as_deref()),
        price_now,
        price_was,
        rating: after
            .first()
            .filter(|l| RATING.is_match(l))
            .map(|l| l.replace('.', ",")),
        reviews: buy
            .iter()
            .find_map(|l| REVIEWS.captures(l))
            .map(|c| c[1].to_string()),
        delivery: delivery(after).map(|d| (String::new(), d)),
        availability: None,
        seller: buy
            .iter()
            .find_map(|l| SELLER.captures(l))
            .map(|c| c[1].trim().to_string()),
        return_days: None,
        image: before.iter().find_map(|l| IMAGE.captures(l)).map(|c| {
            format!(
                "https://productimages.hepsiburada.net/s/{}/1500/{}",
                &c[1], &c[2]
            )
        }),
        specs: specs(buy, after),
        accent: HEPSIBURADA_ORANGE,
        button_label: "Hepsiburada'da aç",
    })
}

/// "Bugün kargoda" / "Tahmini **26 Kasım Perşembe** günü kargoda" (geri sayım satırı atlanır).
fn delivery(after: &[&str]) -> Option<String> {
    let start = after.iter().position(|l| *l == "Teslimat Seçenekleri")?;
    after[start + 1..]
        .iter()
        .take_while(|l| !l.starts_with("## "))
        .find(|l| l.ends_with("kargoda") && !l.starts_with("**"))
        .map(|l| l.replace("**", ""))
}

/// Seçili varyantlar (satın alma bloğu) ve "Ürün Bilgileri" bölümünün anahtar/değer çiftleri.
fn specs(buy: &[&str], after: &[&str]) -> Vec<(String, String)> {
    let variants = buy
        .iter()
        .filter_map(|l| VARIANT.captures(l))
        .filter(|c| c[1] != *"Satıcı")
        // "Ram (Sistem Belleği)" → "Ram": alan adı embed sütununa sığsın.
        .map(|c| {
            let key = c[1].split(" (").next().unwrap_or(&c[1]).trim().to_string();
            (key, c[2].trim().to_string())
        });
    let info: Vec<&str> = after
        .iter()
        .skip_while(|l| **l != "## Ürün Bilgileri")
        .skip(1)
        .take_while(|l| **l != "Tüm Ürün Bilgileri" && !l.starts_with('#'))
        .copied()
        .collect();
    let pairs = info
        .as_chunks::<2>()
        .0
        .iter()
        .map(|kv| (kv[0].to_string(), kv[1].to_string()));
    variants
        .chain(pairs)
        .filter(|(k, v)| {
            k.chars().count() <= MAX_SPEC_KEY && v.chars().count() <= MAX_SPEC_VALUE && v != "Yok"
        })
        .take(MAX_SPECS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gerçek hepsiburada.com ürün sayfasının datacat çıktısından kısaltılmış örnek.
    const SAMPLE: &str = r#"Markdown Content:
---
title: Apple MacBook Air M5 16GB 512GB SSD macOS 13 Taşınabilir Fiyatı
---
* [Değerlendirmelerim](/yorumlarim/yorum-bekleyenler)
1. ![MacBook Air M5 16GB 512GB SSD macOS 13" Taşınabilir Bilgisayar Gece Yarısı MDHE4TU/A](https://productimages.hepsiburada.net/s/777/424-600/110001554460717.jpg)
* ![MacBook Air M5 1](https://productimages.hepsiburada.net/s/777/48-64/110001554460718.jpg)

# [Apple](/apple) MacBook Air M5 16GB 512GB SSD macOS 13" Taşınabilir Bilgisayar Gece Yarısı MDHE4TU/A

4,8

[**37** Değerlendirme](/apple-macbook-air-m5-p-HBCV0000D5X0ME-yorumlari "Yorumları")

Satıcı: [CEPCENTER](/magaza/cepcenter)

9,8Satıcı puanı

Takip et

79.380,00 TL

83.300 TL

Kazancımı gör3.920 TL

Fiyat83.300 TL

🥳Bu üründe toplam kazanç

3.920 TL

Peşin fiyatına 9 x8.820 TL

SSD Kapasitesi: 512 GB

512 GB

Ram (Sistem Belleği): 16 GB

16 GB

Ek hizmet ister misin?

4.999,00 TL

Sepete ekle

Teslimat Seçenekleri

**4 saat 3dk** içinde sipariş verirsen bugün kargoda

Bugün kargoda

## Ürün Bilgileri

USB 3.0

Yok

Pil

4 Hücreli

Tüm Ürün Bilgileri

Satıcı: cemil shop
"#;

    fn sample() -> Product {
        parse_product(
            SAMPLE,
            "https://www.hepsiburada.com/apple-macbook-air-m5-pm-HBC0000D5X0MD",
        )
        .unwrap()
    }

    #[test]
    fn parses_every_card_field() {
        let p = sample();
        assert_eq!(
            p.title,
            "MacBook Air M5 16GB 512GB SSD macOS 13\" Taşınabilir Bilgisayar Gece Yarısı MDHE4TU/A"
        );
        assert_eq!(p.brand.as_deref(), Some("Apple"));
        assert_eq!(p.store, "Hepsiburada");
        assert_eq!(p.price_now.as_deref(), Some("79.380,00 TL"));
        assert_eq!(p.price_was.as_deref(), Some("83.300 TL"));
        assert_eq!(p.savings.as_deref(), Some("−%5"));
        assert_eq!(p.rating.as_deref(), Some("4,8"));
        assert_eq!(p.reviews.as_deref(), Some("37"));
        assert_eq!(p.seller.as_deref(), Some("CEPCENTER"));
        assert_eq!(p.delivery, Some((String::new(), "Bugün kargoda".into())));
        assert_eq!(
            p.image.as_deref(),
            Some("https://productimages.hepsiburada.net/s/777/1500/110001554460717.jpg")
        );
        assert_eq!(
            p.specs,
            [
                ("SSD Kapasitesi", "512 GB"),
                ("Ram", "16 GB"),
                ("Pil", "4 Hücreli"),
            ]
            .map(|(k, v)| (k.to_string(), v.to_string()))
        );
        assert_eq!(p.button_label, "Hepsiburada'da aç");
    }

    #[test]
    fn single_price_has_no_old_price_and_estimated_date_is_read() {
        let md = SAMPLE.replace("83.300 TL\n", "Kredi\n").replace(
            "Bugün kargoda",
            "Tahmini **26 Kasım Perşembe** günü kargoda",
        );
        let p = parse_product(&md, "u").unwrap();
        assert_eq!(p.price_now.as_deref(), Some("79.380,00 TL"));
        assert_eq!((p.price_was, p.savings), (None, None));
        assert_eq!(
            p.delivery.map(|d| d.1).as_deref(),
            Some("Tahmini 26 Kasım Perşembe günü kargoda")
        );
    }

    #[test]
    fn missing_product_page_is_rejected() {
        assert!(parse_product("Markdown Content:\n# Sayfa bulunamadı\n", "u").is_none());
    }
}
