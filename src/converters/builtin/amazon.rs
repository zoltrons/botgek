//! Amazon ürün bağlantısı (`amazon.com.tr/.../dp/{ASIN}` ve `amzn.eu/d/{token}` kısa linki).
//!
//! Amazon botun isteğini engeller (503 / captcha); ürün sayfası datacat vekilinden Markdown olarak
//! alınır (`get_markdown`). Fiyat, puan, stok, teslimat ve teknik künye bu metinden okunur (ayrı API yok). Discord'un kendi önizlemesi başlığı kırpar, fiyat göstermez ve küçük
//! görsel olarak Amazon'un reklam şeridini koyar; bunun yerine onaylı konseptteki kart üretilir:
//!
//! ```text
//! -# Amazon.com.tr · MSI
//! ### Ürün adı                                           [görsel]
//! 🏷️ Fırsat rozeti
//! # 49.911,94 TL
//! -# ~~56.999,00 TL~~ −%12            (yalnız Amazon eski fiyat gösterdiğinde)
//! ⭐ 4,3 · 19 değerlendirme
//! Ücretsiz teslimat · 24 Kasım Salı · **Son 1 adet**
//! ───
//! İşlemci: … · Depolama: …          (etiketli künye, normal yazı)
//! -# Fiyat 09:34 itibarıyla · satıcı Amazon.com.tr · 14 gün iade
//! [Amazon'da aç]                      (kartın altında tek, izleme parametresiz link)
//! ```
//!
//! shortcut: satır kalıpları datacat'in Markdown çıktısına ve Amazon'un sayfa düzenine bağlı; düzen
//! değişirse ilgili alan sessizce boş kalır (kart yine çıkar). Kırılırsa buradaki regex'ler güncellenir.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Value, json};

use super::render::escape_markdown as escape;
use super::{Author, BuiltinPost, CustomCard, unescape_html};
use crate::converters::parser::{ParsedPost, Platform};
use crate::utils::truncate_chars;

const AMAZON_ORANGE: u32 = 0xFF9900;
const MAX_TITLE_CHARS: usize = 150;
/// Künye en çok 8 satır (iki sütunda 4 satır), konseptteki gibi.
const MAX_SPECS: usize = 8;
/// Tablodan etiketsiz eşlenen satır, kesilmeden sığmıyorsa alınmaz.
const MAX_SPEC_KEY: usize = 18;
const MAX_SPEC_VALUE: usize = 30;
/// İki künye hücresi aynı satıra yalnız bu genişliğe sığarsa konur (telefonda satır kırılmasın).
const MAX_SPEC_LINE: usize = 52;

/// `/dp/ASIN`, `/gp/product/ASIN`, `/gp/aw/d/ASIN` biçimlerinden 10 haneli ASIN.
static ASIN_IN_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)/(?:dp|gp/product|gp/aw/d|gp/offer-listing)/([A-Z0-9]{10})")
        .expect("geçerli ASIN URL regex")
});
static AMAZON_HOST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^https?://([^/]*amazon\.[^/]+)/").expect("geçerli host regex")
});
/// Kısa linkte ASIN sayfadan: künye tablosu ya da marka linkindeki `lp_asin`.
static ASIN_IN_PAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:\|\s*ASIN\s*\||lp(?:_|%5F)asin=)\s*([A-Z0-9]{10})")
        .expect("geçerli ASIN sayfa regex")
});
/// Ürün başlığı sayfadaki tek `# ` başlığıdır (alt bölümler `## `/`### `).
static TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^#\s+(\S.*)$").expect("geçerli başlık regex"));
/// Başlığın altındaki marka linki: "[MSI Store’u ziyaret edin](…)".
static BYLINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\[\s*([^\]]+?)\s*\]\(").expect("geçerli marka regex"));
/// Fiyat metnindeki tutar: "Yüzde 3 tasarruf ile 38.999,00 TL \-%3 38.999,00TL".
static AMOUNT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"([0-9][0-9.]*(?:,[0-9]+)?)\s*(?:TL|₺)").expect("geçerli tutar regex")
});
static SAVINGS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\\?-%\s*([0-9]+)").expect("geçerli indirim regex"));
/// Fırsat rozeti, geri sayım şablonundan önceki metin: "Gülümseten Kasım fırsatı NO\_OF\_HOURS…".
static DEAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(.+?)\s+NO\\_OF\\_").expect("geçerli fırsat regex"));
static RATING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"5 yıldız üzerinden ([0-9][.,][0-9])").expect("geçerli puan regex")
});
static REVIEWS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\(\s*([0-9][0-9.]*)\s*\)\s*\]\(#averageCustomerReviewsAnchor")
        .expect("geçerli yorum sayısı regex")
});
/// Satın alma kutusunun ana teslimat satırı: "ÜCRETSİZ teslimat 27 Kasım Cuma. 7 saat … verirseniz".
static DELIVERY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(ÜCRETSİZ|FREE|[0-9][0-9.,]*\s*TL) (?:teslimat|delivery) ([^.]+)")
        .expect("geçerli teslimat regex")
});
static RETURNS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([0-9]+) gün içinde iade").expect("geçerli iade regex"));
/// Ürün görseli (alt metni ürün adı olan büyük görsel); boyut eki (`._AC_SX679_`) atılınca özgün dosya kalır.
static IMAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"!\[([^\]]+)\]\((https://m\.media-amazon\.com/images/I/[^)\s]+)\)")
        .expect("geçerli görsel regex")
});
static IMAGE_SIZE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\._[^/]*_\.(jpg|png)$").expect("geçerli görsel boyutu regex"));
/// İki sütunlu künye tablosu satırı; taksit tabloları dört sütunludur, eşleşmez.
static DETAIL_ROW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\|\s*([^|]+?)\s*\|\s*([^|]+?)\s*\|$").expect("geçerli künye satırı regex")
});
static LOW_STOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)stokta sadece ([0-9]+) adet").expect("geçerli az stok regex")
});

/// Başlığın virgülle ayrılmış parçasını künye etiketine eşler (sıra önemli: "RTX… GDDR6"
/// ekran kartıdır, belleğe düşmemeli; "512GB SSD" depolamadır).
static SPEC_CLASSES: LazyLock<[(Regex, &'static str); 5]> = LazyLock::new(|| {
    let re = |p: &str| Regex::new(p).expect("geçerli künye sınıfı regex");
    [
        (re(r"(?i)\b(rtx|gtx|radeon|geforce|arc a)"), "Ekran kartı"),
        (re(r"(?i)\b(ssd|hdd|nvme|emmc)\b"), "Depolama"),
        (
            re(r"(?i)\b(i[3579]-|core|ryzen|celeron|pentium|snapdragon|apple m[1-4]|ultra [3579])"),
            "İşlemci",
        ),
        (re(r"(?i)\b(ram|ddr[3-5])\b"), "Bellek"),
        (
            re(r#"(?i)(\bhz\b|[0-9]hz|\b(fhd|qhd|uhd|oled|ips)\b|inç|")"#),
            "Ekran",
        ),
    ]
});

/// Teknik tablo anahtarı → künye etiketi (konseptteki adlar). Sıra, künyedeki sıradır.
const TABLE_LABELS: &[(&str, &str)] = &[
    ("CPU Modeli", "İşlemci"),
    ("Grafik Kartı Açıklaması", "Ekran kartı"),
    ("Yüklü RAM Belleği", "Bellek"),
    ("Kurulu RAM Belleği Boyutu", "Bellek"),
    ("Ekran Boyutu", "Ekran"),
    ("Sabit Disk Boyutu", "Depolama"),
    ("Hard Disk Boyutu", "Depolama"),
    ("Ürünün Ağırlığı", "Ağırlık"),
    ("Klavye Açıklaması", "Klavye"),
    ("Garanti Bilgileri", "Garanti"),
    ("Donanım Arabirimi", "Bağlantı"),
    ("Video Yakalama Çözünürlüğü", "Çözünürlük"),
    ("Video Çıkış Formatı", "Çıkış"),
    ("Renk", "Renk"),
    ("Hacim", "Hacim"),
    ("Malzeme", "Malzeme"),
];
/// Künye etiketlerinin konseptteki sırası; listede olmayanlar sonra gelir.
const LABEL_ORDER: &[&str] = &[
    "İşlemci",
    "Ekran kartı",
    "Bellek",
    "Ekran",
    "Depolama",
    "Ağırlık",
    "Klavye",
    "Garanti",
];
/// Etiketsiz tablo satırlarında atlanan anahtarlar (kimlik, sıralama, ayrıntı gürültüsü).
const SPEC_SKIP: &[&str] = &[
    "asin",
    "en çok satanlar",
    "müşteri yorumları",
    "marka",
    "model",
    "üretici",
    "parça numarası",
    "orijin",
    "öğelerin",
    "boyut",
    "dahili bileşen",
    "işletim sistemi",
    "pil",
    "lityum",
    "ürün ağırlığı",
    "birim sayısı",
];

/// Mağazadan bağımsız ürün kartı verisi; Trendyol da aynı kartı kullanır (`trendyol.rs`).
#[derive(Debug, Default)]
pub(super) struct Product {
    pub(super) title: String,
    pub(super) clean_url: String,
    /// Kartın üst satırındaki mağaza adı, bağlantının host'undan ("Amazon.com.tr").
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

pub async fn fetch(client: &reqwest::Client, parsed: &ParsedPost) -> Result<BuiltinPost, String> {
    // Captcha atlatılmaya çalışılmaz: vekil de captcha sayfası getirirse açık hata döner.
    let markdown = super::get_markdown(client, &parsed.original_url).await?;
    if is_captcha(&markdown) {
        return Err(format!(
            "Amazon robot doğrulaması istedi (captcha): {}",
            parsed.original_url
        ));
    }
    let product = parse_product(&markdown, &parsed.original_url)
        .ok_or_else(|| format!("Amazon ürünü okunamadı: {}", parsed.original_url))?;
    Ok(product_post(Platform::Amazon, "Amazon", &product))
}

/// Ürün kartlı gönderi (Amazon ve Trendyol ortak).
pub(super) fn product_post(platform: Platform, label: &str, product: &Product) -> BuiltinPost {
    let mut post = BuiltinPost::new(platform, product.clean_url.clone(), Author::new(label, ""));
    post.title = Some(product.title.clone());
    post.extra_label = Some(label.to_string());
    post.custom_card = Some(build_card(product, crate::utils::unix_secs()));
    post
}

fn parse_product(markdown: &str, source_url: &str) -> Option<Product> {
    let lines: Vec<&str> = markdown
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let title_at = title_line(markdown, &lines)?;
    let full_title = clean_text(&TITLE.captures(lines[title_at])?[1]);
    // Satın alma kutusu (teslimat, stok, satıcı) başlıktan önce, fiyat bloğu başlıktan sonra gelir.
    let (buy_box, after) = (&lines[..title_at], &lines[title_at + 1..]);
    let price_block = &after[..after
        .iter()
        .position(|l| l.contains("Taksitler") || l.contains("Bu ürün hakkında"))
        .unwrap_or(after.len())
        .min(40)];

    let asin = ASIN_IN_URL
        .captures(source_url)
        .or_else(|| ASIN_IN_PAGE.captures(markdown))
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())?;
    let host = AMAZON_HOST
        .captures(source_url)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| "www.amazon.com.tr".to_string());
    let clean_url = format!("https://{host}/dp/{asin}");
    let store = host
        .trim_start_matches("www.")
        .replacen("amazon", "Amazon", 1);

    // Satıcılar teknik özellikleri başlığa virgülle dizer: ilk virgüle kadarı ürün adı,
    // kalanı künye. Baş kısım çok kısaysa (ör. "Krem, 80 ml") başlık bölünmez.
    let (head, tail) = match full_title.split_once(", ") {
        Some((head, tail)) if head.chars().count() >= 20 => (head.to_string(), Some(tail)),
        _ => (full_title.clone(), None),
    };

    // Ödenecek fiyat, başlıktan sonraki ilk tutar satırı; taksit tablosu ve "benzer ürün"
    // fiyatları bloğun dışında kalır.
    let price_line = price_block
        .iter()
        .find(|l| !l.starts_with("Önceki") && AMOUNT.is_match(l));
    let price_now = price_line.and_then(|l| amount(l));
    let price_was = price_block
        .iter()
        .find(|l| l.starts_with("Önceki Fiyat"))
        .and_then(|l| amount(l))
        .filter(|p| Some(p) != price_now.as_ref());

    let rows: Vec<(String, String)> = lines
        .iter()
        .filter_map(|l| DETAIL_ROW.captures(l))
        .map(|c| (clean_text(&c[1]), clean_text(&c[2])))
        .filter(|(k, v)| !k.is_empty() && !v.is_empty() && !v.starts_with("---"))
        .collect();

    Some(Product {
        title: truncate_chars(&head, MAX_TITLE_CHARS),
        clean_url,
        store,
        brand: after
            .first()
            .and_then(|l| BYLINE.captures(l))
            .and_then(|c| brand(&c[1])),
        deal: price_block
            .iter()
            .find_map(|l| DEAL.captures(l))
            .map(|c| clean_text(&c[1])),
        savings: price_line
            .and_then(|l| SAVINGS.captures(l))
            .map(|c| format!("−%{}", &c[1])),
        price_now,
        price_was,
        rating: after
            .iter()
            .take(3)
            .find_map(|l| RATING.captures(l))
            .map(|c| c[1].replace('.', ",")),
        reviews: after
            .iter()
            .take(3)
            .find_map(|l| REVIEWS.captures(l))
            .map(|c| c[1].to_string()),
        delivery: buy_box
            .iter()
            .find_map(|l| DELIVERY.captures(l))
            .map(|c| (c[1].to_string(), c[2].trim().to_string())),
        availability: buy_box
            .iter()
            .find(|l| {
                l.starts_with("Stokta")
                    || l.starts_with("Şu anda mevcut değil")
                    || l.starts_with("Geçici olarak")
            })
            .and_then(|l| availability(l)),
        seller: buy_box
            .iter()
            .position(|l| *l == "Gönderici / Satıcı" || *l == "Satıcı")
            .and_then(|i| buy_box.get(i + 1))
            .map(|l| clean_text(l.trim_start_matches('[').split("](").next().unwrap_or(l)))
            .filter(|s| !s.is_empty()),
        return_days: RETURNS.captures(markdown).map(|c| c[1].to_string()),
        image: image(buy_box, &full_title),
        specs: specs(tail, &rows),
        accent: AMAZON_ORANGE,
        button_label: "Amazon'da aç",
    })
}

/// Ürün başlığı satırı: sayfanın meta başlığıyla ("Ürün adı : Amazon.com.tr: Moda") başlayan ilk `# `
/// satırı. Başlıktan önce başka `# ` satırı olabilir ("# Ürün özeti, temel ürün bilgilerini sunar").
fn title_line(markdown: &str, lines: &[&str]) -> Option<usize> {
    let meta = markdown
        .lines()
        .take(20)
        .find_map(|l| l.strip_prefix("title:"))
        .map(|t| clean_text(t.trim().trim_matches('"')));
    let heading = |l: &&str| TITLE.captures(l).map(|c| clean_text(&c[1]));
    let matches_meta = |l: &&str| {
        heading(l).is_some_and(|h| {
            let prefix: String = h.chars().take(20).collect();
            // Meta başlık markayla başlayabilir ("Fashcolle Dynamic…"), sayfa başlığı markasız.
            meta.as_deref().is_some_and(|m| m.contains(&prefix))
        })
    };
    lines
        .iter()
        .position(matches_meta)
        .or_else(|| lines.iter().position(|l| heading(l).is_some()))
}

/// Amazon'un robot doğrulama sayfası (ürün yerine captcha formu).
fn is_captcha(markdown: &str) -> bool {
    markdown.contains("/errors/validateCaptcha")
}

/// Alt metni ürün adıyla başlayan ilk görsel (galeri küçükleri alt metinsiz, bannerlar başka adlı).
fn image(buy_box: &[&str], title: &str) -> Option<String> {
    let prefix: String = title.chars().take(20).collect();
    buy_box
        .iter()
        .flat_map(|l| IMAGE.captures_iter(l))
        // Alt metin markayla başlayabilir ("Fashcolle Dynamic…"); sayfa başlığı markasız.
        .find(|c| clean_text(&c[1]).contains(&prefix))
        .map(|c| IMAGE_SIZE.replace(&c[2], ".$1").into_owned())
}

/// Etiketli künye: önce başlığın sınıflandırılabilen parçaları, sonra tablodan eşlenen ve
/// kısa olan satırlar; aynı etiket bir kez, konseptteki sırayla, en çok [`MAX_SPECS`].
fn specs(title_tail: Option<&str>, rows: &[(String, String)]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut push = |label: &str, value: String| {
        // Aynı değer iki etiketle gelebilir ("Önemli Özellik" / "Özellikleri"): bir kez.
        if !value.is_empty() && !out.iter().any(|(l, v)| l == label || *v == value) {
            out.push((label.to_string(), value));
        }
    };
    for piece in title_tail.into_iter().flat_map(|t| t.split(", ")) {
        if let Some((_, label)) = SPEC_CLASSES.iter().find(|(re, _)| re.is_match(piece)) {
            push(label, truncate_chars(piece.trim(), 30));
        }
    }
    for (key, label) in TABLE_LABELS {
        if let Some((_, value)) = rows.iter().find(|(k, _)| k == key)
            && value.chars().count() <= MAX_SPEC_VALUE
            && !value.eq_ignore_ascii_case("Özel")
        {
            push(label, pretty_value(label, value));
        }
    }
    for (key, value) in rows {
        // "İ".to_lowercase() "i̇" (noktalı) olur; atlama listesi düz "i" ile yazılı.
        let lower = key.replace('İ', "i").to_lowercase();
        let mapped = TABLE_LABELS.iter().any(|(k, _)| k == key);
        if !mapped
            && key.chars().count() <= MAX_SPEC_KEY
            && value.chars().count() <= MAX_SPEC_VALUE
            && !value.eq_ignore_ascii_case("Özel")
            && !SPEC_SKIP.iter().any(|s| lower.contains(s))
        {
            push(key, value.clone());
        }
    }
    let rank = |label: &str| {
        LABEL_ORDER
            .iter()
            .position(|l| *l == label)
            .unwrap_or(LABEL_ORDER.len())
    };
    out.sort_by_key(|(label, _)| rank(label));
    out.truncate(MAX_SPECS);
    out
}

/// "1.98 Kilogram" → "1,98 kg"; tek sayı garanti "2" → "2 yıl".
fn pretty_value(label: &str, value: &str) -> String {
    match label {
        "Ağırlık" => value
            .replace(" Kilogram", " kg")
            .replace(" Gram", " g")
            .replace('.', ","),
        "Garanti" if value.bytes().all(|b| b.is_ascii_digit()) => format!("{value} yıl"),
        // Tablo depolamayı birimsiz yazar ("512"); küçük sayı TB'dir.
        "Depolama" => match value.parse::<u32>() {
            Ok(n) if n <= 8 => format!("{n} TB"),
            Ok(n) => format!("{n} GB"),
            Err(_) => value.to_string(),
        },
        _ => value.to_string(),
    }
}

fn availability(line: &str) -> Option<String> {
    let value = clean_text(line);
    if let Some(n) = LOW_STOCK.captures(&value) {
        return Some(format!("Son {} adet", &n[1]));
    }
    // Sıradan "Stokta var" kart için bilgi taşımaz; yalnız az stok / tükendi gösterilir.
    let ordinary = value
        .trim_end_matches('.')
        .eq_ignore_ascii_case("Stokta var")
        || value.eq_ignore_ascii_case("In Stock");
    (!value.is_empty() && !ordinary).then(|| truncate_chars(value.trim_end_matches('.'), 40))
}

/// Metindeki son tutar, "685,00 TL" biçiminde (bitişik "699,00TL" de ayrılır).
fn amount(text: &str) -> Option<String> {
    AMOUNT
        .captures_iter(text)
        .last()
        .map(|c| format!("{} TL", &c[1]))
}

fn brand(raw: &str) -> Option<String> {
    let raw = clean_text(raw);
    let mut brand = raw.as_str();
    for prefix in ["Marka:", "Ziyaret edin:", "Mağaza:", "Brand:"] {
        brand = brand.trim_start_matches(prefix).trim();
    }
    // "MSI Store’u ziyaret edin", "X mağazasını ziyaret edin" → "MSI", "X".
    for cut in [" Store", " mağaza", " Mağaza", " markas"] {
        if let Some(i) = brand.find(cut) {
            brand = brand[..i].trim();
        }
    }
    (!brand.is_empty() && brand.chars().any(char::is_alphabetic)).then(|| truncate_chars(brand, 30))
}

pub(super) fn build_card(p: &Product, fetched_at: u64) -> CustomCard {
    // 1) Mağaza · marka (küçük) ve ürün adı (başlık; link butonda).
    let mut head = format!("-# {}", p.store);
    if let Some(brand) = &p.brand {
        head.push_str(&format!(" · {}", escape(brand)));
    }
    head.push_str(&format!("\n### {}", escape(&p.title)));

    // 2) Fırsat rozeti, fiyat (kartın en büyük yazısı), Amazon'un eski fiyatı + indirimi.
    let mut price = Vec::new();
    if let Some(deal) = &p.deal {
        price.push(format!("🏷️ {}", escape(deal)));
    }
    if let Some(now) = &p.price_now {
        price.push(format!("# {}", escape(now)));
        let was = [
            p.price_was.as_ref().map(|w| format!("~~{}~~", escape(w))),
            p.savings.clone(),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        if !was.is_empty() {
            price.push(format!("-# {}", was.join(" ")));
        }
    }

    // 3) Puan; teslimat + stok (kalın yalnız aciliyet: "Son N adet").
    let mut trust = Vec::new();
    if let Some(rating) = &p.rating {
        let mut line = format!("⭐ {rating}");
        if let Some(reviews) = &p.reviews {
            line.push_str(&format!(" · {reviews} değerlendirme"));
        }
        trust.push(line);
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
        trust.push(ship.join(" · "));
    }

    let section_children: Vec<Value> = [head, price.join("\n"), trust.join("\n")]
        .into_iter()
        .filter(|t| !t.is_empty())
        .map(text)
        .collect();

    // Discord, aksesuarsız Section'ı (tip 9) reddeder: görsel yoksa metin doğrudan konur.
    let mut components = match p.image.as_deref() {
        Some(url) => vec![json!({
            "type": 9,
            "components": section_children,
            "accessory": { "type": 11, "media": { "url": url }, "description": "Ürün görseli" },
        })],
        None => section_children,
    };

    let mut bottom = Vec::new();
    // İki sütun: sol sütun ilk yarı, sağ sütun ikinci yarı (konseptteki sıra); çift telefona
    // sığmıyorsa iki satıra bölünür. Kalın yok: kalın beyaz yalnız başlık, fiyat ve aciliyette.
    let half = p.specs.len().div_ceil(2);
    let cell = |(label, value): &(String, String)| format!("{}: {}", escape(label), escape(value));
    for i in 0..half {
        let left = cell(&p.specs[i]);
        match p.specs.get(i + half).map(cell) {
            Some(right) if left.chars().count() + 3 + right.chars().count() <= MAX_SPEC_LINE => {
                bottom.push(format!("{left} · {right}"));
            }
            right => bottom.extend(std::iter::once(left).chain(right)),
        }
    }
    let mut footer = Vec::new();
    if p.price_now.is_some() {
        // Fiyat değişir; alındığı saat görünsün. Göreli zaman (`:R`) bot ile izleyicinin saati
        // farklıysa yeni kartta "10 saniye sonra alındı" yazar; mutlak saat kaymadan etkilenmez.
        footer.push(format!("Fiyat <t:{fetched_at}:t> itibarıyla"));
    }
    if let Some(seller) = p.seller.as_ref().filter(|s| p.brand.as_ref() != Some(*s)) {
        footer.push(format!("satıcı {}", escape(seller)));
    }
    if let Some(days) = &p.return_days {
        footer.push(format!("{days} gün iade"));
    }
    if !footer.is_empty() {
        bottom.push(format!("-# {}", footer.join(" · ")));
    }
    if !bottom.is_empty() {
        components.push(json!({ "type": 14, "spacing": 1 }));
        components.push(text(bottom.join("\n")));
    }

    CustomCard {
        components,
        accent_color: Some(p.accent),
        video_components: None,
        // Kartın altında tek bağlantı butonu (izleme parametresiz temiz link).
        buttons: vec![json!({
            "type": 1,
            "components": [{ "type": 2, "style": 5, "label": p.button_label, "url": p.clean_url }],
        })],
    }
}

fn text(content: String) -> Value {
    json!({ "type": 10, "content": content })
}

/// Markdown hücresinden düz metin: HTML varlıklarını ve kaçışları çöz, boşlukları toparla.
fn clean_text(raw: &str) -> String {
    unescape_html(&raw.replace('\\', ""))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gerçek amazon.com.tr ürün sayfasının datacat çıktısından kısaltılmış örnek.
    const SAMPLE: &str = r#"URL Source: https://www.amazon.com.tr/dp/B0GHR4L88D
Markdown Content:
---
title: "MSI CYBORG 15 A13VE-2269XTRNN Dizüstü Bilgisayar, Intel Core I5-13420H : Amazon.com.tr: Bilgisayar"
---
#### Çerez ve tanıtım seçenekleri
# Ürün özeti, temel ürün bilgilerini sunar ---Klavye kısayolu  Shift + alt + opt + D
[![Yaşam ve Hobi](https://m.media-amazon.com/images/I/41YoKYnoxdL.jpg)](https://www.amazon.com.tr/b/?node=29761388031)

Prime ile ÜCRETSİZ teslimat, özel fırsatlar, ödüllü film ve dizilerin keyfini çıkar.

1.234,00TL

ÜCRETSİZ teslimat 24 Kasım Salı. 7 saat 54 dakika içinde sipariş verirseniz

veya en hızlı teslimat Yarın, 22 Kasım

 Stokta sadece 1 adet kaldı.  

Gönderici / Satıcı

Amazon.com.tr

[Amazon.com.tr](<javascript:void(0)>)

[Teslimat tarihinden sonraki 14 gün içinde iade edilebilir](<javascript:void(0)>)

- ![](https://m.media-amazon.com/images/I/41Ra744t9ML._AC_US40_.jpg)
- ![MSI CYBORG 15 A13VE-2269XTRNN Dizüstü Bilgisayar, Intel Core I5-13420H, 15.6 FHD 144Hz, 16GB RAM DDR5, RTX4050 GDDR6 6GB, 512GB SSD, DOS](https://m.media-amazon.com/images/I/71l4M2y8zwL._AC_SX679_.jpg)

# MSI CYBORG 15 A13VE-2269XTRNN Dizüstü Bilgisayar, Intel Core I5-13420H, 15.6 FHD 144Hz, 16GB RAM DDR5, RTX4050 GDDR6 6GB, 512GB SSD, DOS

[MSI Store’u ziyaret edin](https://www.amazon.com.tr/stores/MSI/page/D40D?lp_asin=B0GHR4L88D&ref_=ast_bln)

[4,3 *5 yıldız üzerinden 4,3*](<javascript:void(0)>) [(19)](#averageCustomerReviewsAnchor)

Geçen ay 200 adetten fazla satın alındı

Gülümseten Kasım fırsatı NO\_OF\_HOURS saat NO\_OF\_MINUTES dakika Gülümseten Kasım fırsatı

 Yüzde 12 tasarruf ile 49.911,94 TL \-%12 49.911,94TL 

Önceki Fiyat: 56.999,00 TLÖnceki Fiyat: 56.999,00TL56.999,00TL [ ![](https://m.media-amazon.com/images/S/sash//GN8m.svg) ](#) 

12 aya varan taksit seçenekleri [Taksitler](<javascript:void(0)>)

| Dönem | Aylık Tutar | Vade Farkı | Toplam Tutar |
| --- | --- | --- | --- |
| 2x ay | 24.955,97 TL | ücretsiz | 49.911,94 TL |

[Lenovo LOQ](https://www.amazon.com.tr/dp/B0X) 28.929,00TL Stokta sadece 13 adet kaldı.

| Sabit Disk Boyutu | 512 GB |
| --- | --- |
| İşletim Sistemi | DOS |
| Grafik Açıklaması | Özel |
| Ürünün Ağırlığı | 1.98 Kilogram |
| Klavye Açıklaması | Arkadan Aydınlatmalı |
| Garanti Bilgileri | 2 |
| Model Numarası | A13VE-2269XTRNN |
| En Çok Satanlar Sıralaması | Bilgisayar Şu kategoride 264. sırada: ([Bilgisayar Şu kategorideki En Popüler 100 Ürünü göster](/x)) |
| ASIN | B0GHR4L88D |
"#;

    fn sample() -> Product {
        parse_product(SAMPLE, "https://www.amazon.com.tr/dp/B0GHR4L88D?ref=spam").unwrap()
    }

    fn card_text(card: &CustomCard) -> String {
        let mut out = Vec::new();
        for c in &card.components {
            for t in std::iter::once(c).chain(c["components"].as_array().into_iter().flatten()) {
                if let Some(s) = t["content"].as_str() {
                    out.push(s.to_string());
                }
            }
        }
        out.join("\n")
    }

    #[test]
    fn parses_every_concept_field() {
        let p = sample();
        assert_eq!(p.title, "MSI CYBORG 15 A13VE-2269XTRNN Dizüstü Bilgisayar");
        assert_eq!(p.clean_url, "https://www.amazon.com.tr/dp/B0GHR4L88D");
        assert_eq!(p.store, "Amazon.com.tr");
        assert_eq!(p.brand.as_deref(), Some("MSI"));
        assert_eq!(p.deal.as_deref(), Some("Gülümseten Kasım fırsatı"));
        // Sayfanın başındaki ilgisiz "1.234,00TL" değil, ödenecek fiyat okunur.
        assert_eq!(p.price_now.as_deref(), Some("49.911,94 TL"));
        assert_eq!(p.price_was.as_deref(), Some("56.999,00 TL"));
        assert_eq!(p.savings.as_deref(), Some("−%12"));
        assert_eq!(p.rating.as_deref(), Some("4,3"));
        assert_eq!(p.reviews.as_deref(), Some("19"));
        assert_eq!(
            p.delivery,
            Some(("ÜCRETSİZ".into(), "24 Kasım Salı".into()))
        );
        assert_eq!(p.availability.as_deref(), Some("Son 1 adet"));
        assert_eq!(p.seller.as_deref(), Some("Amazon.com.tr"));
        assert_eq!(p.return_days.as_deref(), Some("14"));
        assert_eq!(
            p.image.as_deref(),
            Some("https://m.media-amazon.com/images/I/71l4M2y8zwL.jpg")
        );
    }

    #[test]
    fn specs_are_labeled_in_concept_order() {
        assert_eq!(
            sample().specs,
            [
                ("İşlemci", "Intel Core I5-13420H"),
                ("Ekran kartı", "RTX4050 GDDR6 6GB"),
                ("Bellek", "16GB RAM DDR5"),
                ("Ekran", "15.6 FHD 144Hz"),
                ("Depolama", "512GB SSD"),
                ("Ağırlık", "1,98 kg"),
                ("Klavye", "Arkadan Aydınlatmalı"),
                ("Garanti", "2 yıl"),
            ]
            .map(|(l, v)| (l.to_string(), v.to_string()))
        );
    }

    #[test]
    fn card_follows_the_concept_layout() {
        let card = build_card(&sample(), 1_700_000_000);
        assert_eq!(
            card_text(&card),
            "-# Amazon.com.tr · MSI\n\
             ### MSI CYBORG 15 A13VE-2269XTRNN Dizüstü Bilgisayar\n\
             🏷️ Gülümseten Kasım fırsatı\n\
             # 49.911,94 TL\n\
             -# ~~56.999,00 TL~~ −%12\n\
             ⭐ 4,3 · 19 değerlendirme\n\
             Ücretsiz teslimat · 24 Kasım Salı · **Son\u{a0}1\u{a0}adet**\n\
             İşlemci: Intel Core I5-13420H · Depolama: 512GB SSD\n\
             Ekran kartı: RTX4050 GDDR6 6GB · Ağırlık: 1,98 kg\n\
             Bellek: 16GB RAM DDR5 · Klavye: Arkadan Aydınlatmalı\n\
             Ekran: 15.6 FHD 144Hz · Garanti: 2 yıl\n\
             -# Fiyat <t:1700000000:t> itibarıyla · satıcı Amazon.com.tr · 14 gün iade"
        );
        assert_eq!(card.components[0]["type"], 9, "görselli bölüm");
        assert_eq!(card.components[1]["type"], 14, "ayraç");
    }

    #[test]
    fn bold_is_reserved_for_title_price_and_urgency() {
        // Okunabilirlik: künye, rozet, teslimat düz yazı; kalın yalnız aciliyette.
        let text = card_text(&build_card(&sample(), 1));
        assert_eq!(text.matches("**").count(), 2, "{text}");
        assert!(text.contains("**Son\u{a0}1\u{a0}adet**"));
    }

    #[test]
    fn single_clean_button_and_short_link_resolves_from_page() {
        let p = parse_product(SAMPLE, "https://amzn.eu/d/04JNI9Sx").unwrap();
        assert_eq!(p.clean_url, "https://www.amazon.com.tr/dp/B0GHR4L88D");
        let card = build_card(&p, 1_700_000_000);
        assert_eq!(card.buttons.len(), 1, "kart altında tek buton satırı");
        let row = card.buttons[0]["components"].as_array().unwrap();
        assert_eq!(row.len(), 1);
        assert_eq!(row[0]["label"], "Amazon'da aç");
        assert_eq!(row[0]["url"], "https://www.amazon.com.tr/dp/B0GHR4L88D");
    }

    #[test]
    fn missing_fields_drop_their_lines_only() {
        let html = SAMPLE
            .replace("Önceki Fiyat: ", "")
            .replace("\\-%12", "")
            .replace("Yüzde 12 tasarruf ile ", "")
            .replace("NO\\_OF\\_", "")
            .replace(" Stokta sadece 1 adet kaldı.", "Stokta var");
        let p = parse_product(&html, "https://www.amazon.com.tr/dp/B0GHR4L88D").unwrap();
        let text = card_text(&build_card(&p, 1));
        assert!(text.contains("\n# 49.911,94 TL"));
        assert!(!text.contains("~~") && !text.contains("🏷️") && !text.contains("Son "));
        assert!(text.contains("Ücretsiz teslimat · 24 Kasım Salı"));
    }

    #[test]
    fn missing_price_drops_price_lines_and_fetch_time() {
        let html = SAMPLE
            .replace(" Yüzde 12 tasarruf ile 49.911,94 TL \\-%12 49.911,94TL", "")
            .replace(
                "Önceki Fiyat: 56.999,00 TLÖnceki Fiyat: 56.999,00TL56.999,00TL",
                "",
            );
        let p = parse_product(&html, "https://www.amazon.com.tr/dp/B0GHR4L88D").unwrap();
        assert_eq!(p.price_now, None);
        let card = build_card(&p, 1);
        let text = card_text(&card);
        assert!(
            !text.lines().any(|l| l.starts_with("# "))
                && !text.contains("~~")
                && !text.contains("alındı")
        );
        assert_eq!(card.buttons.len(), 1);
    }

    #[test]
    fn card_without_image_has_no_accessoryless_section() {
        let html = SAMPLE.replace("- ![MSI CYBORG", "- ![Başka ürün");
        let p = parse_product(&html, "https://www.amazon.com.tr/dp/B0GHR4L88D").unwrap();
        assert_eq!(p.image, None);
        let card = build_card(&p, 1);
        assert!(card.components.iter().all(|c| c["type"] != 9));
        assert!(card_text(&card).contains("\n# 49.911,94 TL"));
    }

    #[test]
    fn non_laptop_uses_short_table_rows() {
        let rows: Vec<(String, String)> = [
            ("Donanım Arabirimi", "USB"),
            ("Video Yakalama Çözünürlüğü", "1080p"),
            ("İşletim Sistemi", "Windows / MacOS / Linux / Android"),
            ("Ürün İçin Önerilen Kullanım Alanları", "Video Kayıt"),
            ("Platform", "Android, Linux, Windows"),
            ("Video Çıkış Formatı", "HDMI"),
            ("Ürünün Ağırlığı", "0.5 Kilogram"),
            ("Marka Adı", "VegGieg"),
            ("Garanti Bilgileri", "2 yıl"),
            ("İşletim Sistemi", "Freedos"),
        ]
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .to_vec();
        let labels: Vec<String> = specs(None, &rows)
            .into_iter()
            .map(|(l, v)| format!("{l}: {v}"))
            .collect();
        assert_eq!(
            labels,
            [
                "Ağırlık: 0,5 kg",
                "Garanti: 2 yıl",
                "Bağlantı: USB",
                "Çözünürlük: 1080p",
                "Çıkış: HDMI",
                "Platform: Android, Linux, Windows",
            ]
        );
    }

    /// Gerçek Fashcolle sayfası: meta başlık ve görsel alt metni markayla başlar, sayfa başlığı
    /// markasız; önce gelen "Ürün özeti" başlığı link biçimindedir.
    #[test]
    fn brandless_page_title_matches_branded_meta_and_image() {
        let md = r#"Markdown Content:
title: Fashcolle Dynamic Black Masajlı Oyuncu Koltuğu : Amazon.com.tr: Ev ve Yaşam
# [ Ürün özeti, temel ürün bilgilerini sunar ](#)
![](https://m.media-amazon.com/images/I/31ygo11zySL._AC_US40_.jpg)
![Fashcolle Dynamic Black Masajlı Oyuncu Koltuğu](https://m.media-amazon.com/images/I/71CNvM0zR3L._AC_SY879_.jpg)
# Dynamic Black Masajlı Oyuncu Koltuğu
"#;
        let p = parse_product(md, "https://www.amazon.com.tr/dp/B0HHDWB1JY").unwrap();
        assert_eq!(p.title, "Dynamic Black Masajlı Oyuncu Koltuğu");
        assert_eq!(
            p.image.as_deref(),
            Some("https://m.media-amazon.com/images/I/71CNvM0zR3L.jpg")
        );
    }

    /// Datacat'in HTML artıklı ikinci biçimi (gerçek Lenovo sayfasından): satır başı boşluk,
    /// `_` italik, parantez içi boşluk, kaçışlı `\-%`.
    #[test]
    fn second_datacat_format_is_read_too() {
        let md = r#"Markdown Content:
title: Lenovo IdeaPad Slim 3 Intel Core 5 320 16GB 512GB SSD 15.3&quot; WUXGA FreeDOS 83RR00HUTR : Amazon.com.tr: Bilgisayar
 ÜCRETSİZ teslimat 28 Kasım Cumartesi 
 Stokta sadece 3 adet kaldı.   
#  Lenovo IdeaPad Slim 3 Intel Core 5 320 16GB 512GB SSD 15.3" WUXGA FreeDOS 83RR00HUTR 
[Lenovo Store’u ziyaret edin](/stores/Lenovo/page/6B75?lp%5Fasin=B0HCYVGLS3&ref%5F=ast%5Fbln) 
[  5,0 _5 yıldız üzerinden 5,0_ ](javascript:void%280%29) [ (2) ](#averageCustomerReviewsAnchor) div> 
365 günün en düşük fiyatı NO\_OF\_HOURS saat NO\_OF\_MINUTES dakika
 Yüzde 3 tasarruf ile 38.999,00 TL \-%3 38.999,00TL 
Önceki Fiyat: 39.999,00 TLÖnceki Fiyat: 39.999,00TL39.999,00TL [ ![](https://m.media-amazon.com/images/S/sash//GN8m.svg) ](#) 
12 aya varan taksit seçenekleri [ Taksitler ](javascript:void%280%29) 
"#;
        let p = parse_product(md, "https://amzn.eu/d/x").unwrap();
        assert_eq!(p.clean_url, "https://www.amazon.com.tr/dp/B0HCYVGLS3");
        assert_eq!(p.brand.as_deref(), Some("Lenovo"));
        assert_eq!(p.rating.as_deref(), Some("5,0"));
        assert_eq!(p.reviews.as_deref(), Some("2"));
        assert_eq!(p.deal.as_deref(), Some("365 günün en düşük fiyatı"));
        assert_eq!(p.price_now.as_deref(), Some("38.999,00 TL"));
        assert_eq!(p.price_was.as_deref(), Some("39.999,00 TL"));
        assert_eq!(p.savings.as_deref(), Some("−%3"));
        assert_eq!(p.availability.as_deref(), Some("Son 3 adet"));
        assert_eq!(
            p.delivery,
            Some(("ÜCRETSİZ".into(), "28 Kasım Cumartesi".into()))
        );
    }

    #[test]
    fn captcha_page_is_recognized_and_product_page_is_not() {
        let captcha = "Markdown Content:\nKarakterleri girin\n[Devam](https://www.amazon.com.tr/errors/validateCaptcha?amzn=x)";
        assert!(is_captcha(captcha));
        assert!(parse_product(captcha, "https://www.amazon.com.tr/dp/B0GHR4L88D").is_none());
        assert!(!is_captcha(SAMPLE));
    }

    #[test]
    fn store_follows_host() {
        let p = parse_product(SAMPLE, "https://www.amazon.com/dp/B0GHR4L88D").unwrap();
        assert_eq!(p.store, "Amazon.com");
        assert_eq!(p.clean_url, "https://www.amazon.com/dp/B0GHR4L88D");
    }
}
