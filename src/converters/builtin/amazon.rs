//! Amazon ürün bağlantısı (`amazon.com.tr/.../dp/{ASIN}` ve `amzn.eu/d/{token}` kısa linki).
//!
//! Ürün sayfası botun kendi kimliğiyle istenir ve sunucuda çizilir; fiyat, puan, stok, teslimat ve teknik künye doğrudan HTML'den
//! okunur (ayrı bir API yok). Discord'un kendi önizlemesi başlığı kırpar, fiyat göstermez ve küçük
//! görsel olarak Amazon'un reklam şeridini koyar; bunun yerine onaylı konseptteki kart üretilir:
//!
//! ```text
//! -# Amazon.com.tr · MSI
//! ## Ürün adı                                            [görsel]
//! 🏷️ Fırsat rozeti
//! # 49.911,94 TL
//! -# ~~56.999,00 TL~~ −%12            (yalnız Amazon eski fiyat gösterdiğinde)
//! ⭐ 4,3 · 19 değerlendirme
//! Ücretsiz teslimat · 24 Kasım Salı · **Son 1 adet**
//! ───
//! İşlemci: … · Depolama: …          (etiketli künye, normal yazı)
//! -# Fiyat 12 dakika önce alındı · satıcı Amazon.com.tr · 14 gün iade
//! [Amazon'da aç]                      (kartın altında tek, izleme parametresiz link)
//! ```
//!
//! shortcut: HTML sınıf/kimlik seçicileri Amazon'un düzenine bağlı; düzen değişirse ilgili alan
//! sessizce boş kalır (kart yine çıkar). Seçiciler kırılırsa buradaki regex'ler güncellenir.

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

/// `/dp/ASIN`, `/gp/product/ASIN`, `/gp/aw/d/ASIN` biçimlerinden 10 haneli ASIN.
static ASIN_IN_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)/(?:dp|gp/product|gp/aw/d|gp/offer-listing)/([A-Z0-9]{10})")
        .expect("geçerli ASIN URL regex")
});
static AMAZON_HOST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^https?://([^/]*amazon\.[^/]+)/").expect("geçerli host regex")
});
static ASIN_IN_HTML: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(?:"asin"\s*:\s*"|data-asin=")([A-Z0-9]{10})""#)
        .expect("geçerli ASIN HTML regex")
});
static CANONICAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)rel="canonical"\s+href="([^"]+)""#).expect("geçerli canonical regex")
});
static TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)id="productTitle"[^>]*>(.*?)</span>"#).expect("geçerli başlık regex")
});
/// Ödenecek fiyatın erişilebilirlik etiketi: görünen fiyat parçalı span'larda, düz metin burada.
static PRICE_TO_PAY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)id="apex-pricetopay-accessibility-label"[^>]*>\s*([^<]+?)\s*<"#)
        .expect("geçerli fiyat regex")
});
/// Üstü çizili ("Önceki Fiyat") fiyat: `a-text-price…` + `data-a-strike="true"`.
static STRIKE_PRICE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)class="a-price a-text-price[^"]*"[^>]*data-a-strike="true"[^>]*>\s*<span class="a-offscreen">\s*([^<]+?)\s*</span>"#)
        .expect("geçerli eski fiyat regex")
});
/// Fiyat metnindeki tutar: etiket indirimde "Yüzde 2 tasarruf ile 685,00 TL" olur.
static AMOUNT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"([0-9][0-9.]*(?:,[0-9]+)?)\s*(?:TL|₺)").expect("geçerli tutar regex")
});
static SAVINGS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)savingsPercentage[^>]*>\s*([^<]+?)\s*<"#).expect("geçerli indirim regex")
});
static DEAL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)id="dealBadgeSupportingText"[^>]*>(.*?)</span>"#)
        .expect("geçerli fırsat regex")
});
static RATING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)id="acrPopover"[^>]*title="[^"]*?([0-9]+[.,][0-9])\s*""#)
        .expect("geçerli puan regex")
});
static REVIEWS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)id="acrCustomerReviewText"[^>]*>\s*\(?\s*([0-9][0-9.]*)"#)
        .expect("geçerli yorum sayısı regex")
});
/// Ana teslimat seçeneği (`DEXUnifiedCXPDM`): ücret ve tarih.
static DELIVERY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)data-csa-c-content-id="DEXUnifiedCXPDM"[^>]*data-csa-c-delivery-price="([^"]*)"[^>]*data-csa-c-delivery-time="([^"]*)""#)
        .expect("geçerli teslimat regex")
});
static RETURNS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([0-9]+) gün içinde iade").expect("geçerli iade regex"));
/// "Satıcı" alanı (`desktop-merchant-info`); "Gönderici" ayrı bir alandır.
static SELLER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)offer-display-feature-name="desktop-merchant-info".{0,3000}?offer-display-feature-text-message">\s*([^<]+?)\s*<"#)
        .expect("geçerli satıcı regex")
});
/// Ürün görseli: ana görselin büyük sürümü, yoksa galerinin ilk `hiRes`'i, yoksa og:image.
static IMAGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)data-old-hires="(https://[^"]+)"|"hiRes":"(https://[^"]+)"|property="og:image"\s+content="(https://[^"]+)""#)
        .expect("geçerli görsel regex")
});
static BYLINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)id="bylineInfo"[^>]*>(.*?)</a>"#).expect("geçerli marka regex")
});
static DETAIL_ROW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)prodDetSectionEntry">(.*?)</th>\s*<td[^>]*>(.*?)</td>"#)
        .expect("geçerli künye satırı regex")
});
static AVAILABILITY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)id="availability"[^>]*>\s*<span[^>]*>(.*?)</span>"#)
        .expect("geçerli stok regex")
});
static LOW_STOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)stokta sadece ([0-9]+) adet").expect("geçerli az stok regex")
});
static TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<[^>]+>").expect("geçerli etiket regex"));

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
    ("Yüklü RAM Belleği", "Bellek"),
    ("Ekran Boyutu", "Ekran"),
    ("Sabit Disk Boyutu", "Depolama"),
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
];

#[derive(Debug, Default)]
struct Product {
    title: String,
    clean_url: String,
    /// Kartın üst satırındaki mağaza adı, bağlantının host'undan ("Amazon.com.tr").
    store: String,
    brand: Option<String>,
    deal: Option<String>,
    price_now: Option<String>,
    price_was: Option<String>,
    savings: Option<String>,
    rating: Option<String>,
    reviews: Option<String>,
    /// Ana teslimat: (ücret, tarih); ücret "ÜCRETSİZ" ya da tutar.
    delivery: Option<(String, String)>,
    availability: Option<String>,
    seller: Option<String>,
    return_days: Option<String>,
    image: Option<String>,
    /// Etiketli künye: (etiket, değer).
    specs: Vec<(String, String)>,
}

pub async fn fetch(client: &reqwest::Client, parsed: &ParsedPost) -> Result<BuiltinPost, String> {
    // Botun kendi kimliğiyle (`get_text`) istenir. Tarayıcı taklidi (Chrome UA + rustls parmak
    // izi) Amazon'da captcha'yı tetikliyordu; captcha atlatılmaya çalışılmaz, açık hata döner.
    let html = super::get_text(client, &parsed.original_url).await?;
    if is_captcha(&html) {
        return Err(format!(
            "Amazon robot doğrulaması istedi (captcha): {}",
            parsed.original_url
        ));
    }
    let product = parse_product(&html, &parsed.original_url)
        .ok_or_else(|| format!("Amazon ürünü okunamadı: {}", parsed.original_url))?;

    let mut post = BuiltinPost::new(
        Platform::Amazon,
        product.clean_url.clone(),
        Author::new("Amazon", ""),
    );
    post.title = Some(product.title.clone());
    post.extra_label = Some("Amazon".to_string());
    post.custom_card = Some(build_card(&product, crate::utils::unix_secs()));
    Ok(post)
}

fn parse_product(html: &str, source_url: &str) -> Option<Product> {
    let asin = ASIN_IN_URL
        .captures(source_url)
        .or_else(|| ASIN_IN_HTML.captures(html))
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())?;
    let host = AMAZON_HOST
        .captures(source_url)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .or_else(|| {
            CANONICAL
                .captures(html)
                .and_then(|c| c.get(1))
                .and_then(|m| AMAZON_HOST.captures(m.as_str()))
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().to_string())
        })
        .unwrap_or_else(|| "www.amazon.com.tr".to_string());
    let clean_url = format!("https://{host}/dp/{asin}");
    let store = host
        .trim_start_matches("www.")
        .replacen("amazon", "Amazon", 1);

    let full_title = first(&TITLE, html).filter(|t| !t.is_empty())?;
    // Satıcılar teknik özellikleri başlığa virgülle dizer: ilk virgüle kadarı ürün adı,
    // kalanı künye. Baş kısım çok kısaysa (ör. "Krem, 80 ml") başlık bölünmez.
    let (head, tail) = match full_title.split_once(", ") {
        Some((head, tail)) if head.chars().count() >= 20 => (head.to_string(), Some(tail)),
        _ => (full_title.clone(), None),
    };

    // Fiyatlar yalnızca ödenecek fiyat bloğundan okunur; taksit, "benzer ürün" vb. fiyatlar
    // kartı yanıltmasın.
    let price_region = price_region(html);
    let price_now = first(&PRICE_TO_PAY, price_region).and_then(|t| amount(&t));
    let price_was = first(&STRIKE_PRICE, price_region)
        .and_then(|t| amount(&t))
        .filter(|p| Some(p) != price_now.as_ref());

    let rows = detail_rows(html);

    Some(Product {
        title: truncate_chars(&head, MAX_TITLE_CHARS),
        clean_url,
        store,
        brand: brand(html),
        deal: first(&DEAL, html).filter(|d| !d.is_empty()),
        price_now,
        price_was,
        savings: first(&SAVINGS, price_region)
            .map(|s| normalize_savings(&s))
            .filter(|s| !s.is_empty()),
        rating: first(&RATING, html),
        reviews: first(&REVIEWS, html),
        delivery: DELIVERY
            .captures(html)
            .map(|c| (clean_text(&c[1]), clean_text(&c[2])))
            .filter(|(_, date)| !date.is_empty()),
        availability: availability(html),
        seller: first(&SELLER, html).filter(|s| !s.is_empty()),
        return_days: RETURNS.captures(html).map(|c| c[1].to_string()),
        image: IMAGE
            .captures(html)
            .and_then(|c| c.get(1).or_else(|| c.get(2)).or_else(|| c.get(3)))
            .map(|m| m.as_str().to_string()),
        specs: specs(tail, &rows),
    })
}

/// Amazon'un robot doğrulama sayfası (ürün yerine ~4 KB captcha formu).
fn is_captcha(html: &str) -> bool {
    html.contains("/errors/validateCaptcha")
}

/// Regex'in ilk grubunu düz metne çevirir.
fn first(re: &Regex, html: &str) -> Option<String> {
    re.captures(html)
        .and_then(|c| c.get(1))
        .map(|m| clean_text(m.as_str()))
}

/// `i`'yi aşmayan en yakın harf sınırı: Türkçe harfin ortasından dilimleme panikler.
fn floor_boundary(html: &str, i: usize) -> usize {
    let mut end = i.min(html.len());
    while !html.is_char_boundary(end) {
        end -= 1;
    }
    end
}

/// Çekirdek fiyat bloğundan başlayan dilim (yoksa tüm HTML).
fn price_region(html: &str) -> &str {
    for marker in [
        "apex-pricetopay-accessibility-label",
        "corePriceDisplay_desktop_feature_div",
    ] {
        if let Some(i) = html.find(marker) {
            let start = html[..i].rfind('<').unwrap_or(i);
            return &html[start..floor_boundary(html, i + 8000)];
        }
    }
    html
}

fn detail_rows(html: &str) -> Vec<(String, String)> {
    DETAIL_ROW
        .captures_iter(html)
        .map(|c| (clean_text(&c[1]), clean_text(&c[2])))
        .filter(|(k, v)| !k.is_empty() && !v.is_empty())
        .collect()
}

/// Etiketli künye: önce başlığın sınıflandırılabilen parçaları, sonra tablodan eşlenen ve
/// kısa olan satırlar; aynı etiket bir kez, konseptteki sırayla, en çok [`MAX_SPECS`].
fn specs(title_tail: Option<&str>, rows: &[(String, String)]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut push = |label: &str, value: String| {
        if !value.is_empty() && !out.iter().any(|(l, _)| l == label) {
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
        let lower = key.to_lowercase();
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
        _ => value.to_string(),
    }
}

fn availability(html: &str) -> Option<String> {
    // Bloğun ilk span'ı stok cümlesidir; içindeki <script> vb. gürültü sonraki span'lardadır.
    let value = first(&AVAILABILITY, html)?;
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

fn brand(html: &str) -> Option<String> {
    let raw = first(&BYLINE, html)?;
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

fn build_card(p: &Product, fetched_at: u64) -> CustomCard {
    // 1) Mağaza · marka (küçük) ve ürün adı (başlık; link butonda).
    let mut head = format!("-# {}", p.store);
    if let Some(brand) = &p.brand {
        head.push_str(&format!(" · {}", escape(brand)));
    }
    head.push_str(&format!("\n## {}", escape(&p.title)));

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
        if fee.eq_ignore_ascii_case("ÜCRETSİZ") || fee.eq_ignore_ascii_case("FREE") {
            ship.push(format!("Ücretsiz teslimat · {}", escape(date)));
        } else {
            ship.push(format!("Teslimat {} · {}", escape(fee), escape(date)));
        }
    }
    if let Some(av) = &p.availability {
        ship.push(format!("**{}**", escape(av)));
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
    if !p.specs.is_empty() {
        // İki sütun: sol sütun ilk yarı, sağ sütun ikinci yarı (konseptteki sıra). Kalın yok:
        // kalın beyaz yalnız başlık, fiyat ve aciliyette; künye düz okunur.
        let half = p.specs.len().div_ceil(2);
        let cell =
            |(label, value): &(String, String)| format!("{}: {}", escape(label), escape(value));
        for i in 0..half {
            let mut line = cell(&p.specs[i]);
            if let Some(right) = p.specs.get(i + half) {
                line.push_str(&format!(" · {}", cell(right)));
            }
            bottom.push(line);
        }
    }
    let mut footer = Vec::new();
    if p.price_now.is_some() {
        // Fiyat değişir; kart sohbette kaldıkça ne kadar eski olduğu görünsün.
        footer.push(format!("Fiyat <t:{fetched_at}:R> alındı"));
    }
    if let Some(seller) = &p.seller {
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
        accent_color: Some(AMAZON_ORANGE),
        video_components: None,
        // Kartın altında tek bağlantı butonu (izleme parametresiz temiz link).
        buttons: vec![json!({
            "type": 1,
            "components": [{ "type": 2, "style": 5, "label": "Amazon'da aç", "url": p.clean_url }],
        })],
    }
}

fn text(content: String) -> Value {
    json!({ "type": 10, "content": content })
}

/// HTML'den düz metin: etiketleri at, HTML varlıklarını çöz, boşlukları toparla.
fn clean_text(raw: &str) -> String {
    let no_tags = TAG.replace_all(raw, " ");
    unescape_html(&no_tags)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// "-%12", "%12 (₺6.000 tasarruf)" → "−%12".
fn normalize_savings(raw: &str) -> String {
    // Yalnız ilk sayı: tutar rakamları yüzdeye karışmasın.
    let digits: String = raw
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    if digits.is_empty() {
        return String::new();
    }
    format!("−%{digits}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gerçek amazon.com.tr ürün sayfasının işaretlemesinden kısaltılmış örnek.
    const SAMPLE: &str = r##"<html><head>
<link rel="canonical" href="https://www.amazon.com.tr/MSI-Cyborg/dp/B0GHR4L88D"></head><body>
<div data-asin="B0GHR4L88D"></div>
<a id="bylineInfo" class="a-link-normal" href="/stores/MSI">MSI Store&rsquo;u ziyaret edin</a>
<span id="productTitle" class="a-size-large">  MSI CYBORG 15 A13VE-2269XTRNN Dizüstü Bilgisayar, Intel Core I5-13420H, 15.6 FHD 144Hz, 16GB RAM DDR5, RTX4050 GDDR6 6GB, 512GB SSD, DOS  </span>
<span id="acrPopover" class="reviewCountTextLinkedHistogram noUnderline" title="5 yıldız üzerinden 4,3">
<span id="acrCustomerReviewText" aria-label="19 Yorumlar" class="a-size-small">(19)</span>
<img data-a-image-name="landingImage" id="landingImage" data-old-hires="https://m.media-amazon.com/images/I/71l4M2y8zwL._AC_SL1500_.jpg">
<span class="a-offscreen">1.234,00TL</span>
<span id="dealBadgeSupportingText" class="a-size-small dealBadgeTextColor a-text-bold"> Gülümseten Kasım fırsatı </span>
<div id="corePriceDisplay_desktop_feature_div">
  <span id="apex-pricetopay-accessibility-label" data-pricetopay-label="{priceToPay}" data-pricetopay-savings-label="Yüzde {savings} tasarruf ile {priceToPay}" class="aok-offscreen"> Yüzde 12 tasarruf ile 49.911,94&nbsp;TL </span>
  <span class="a-price priceToPay"><span class="a-offscreen"> </span></span>
  <span class="a-size-large a-color-price savingPriceOverride aok-align-center reinventPriceSavingsPercentageMargin savingsPercentage apex-savings-percentage">-%12</span>
  <span class="basisPrice"><span class="apex-basisprice-label">Önceki Fiyat:</span> <span class="a-price a-text-price apex-basisprice-value" data-a-size="s" data-a-strike="true" data-a-color="secondary"><span class="a-offscreen">56.999,00TL</span><span aria-hidden="true">56.999,00TL</span></span></span>
</div>
<span data-csa-c-type="element" data-csa-c-content-id="DEXUnifiedCXPDM" data-csa-c-delivery-price="ÜCRETSİZ" data-csa-c-value-proposition="" data-csa-c-delivery-type="teslimat" data-csa-c-delivery-time="24 Kasım Salı" data-csa-c-delivery-destination="">
<div id="availability" class="a-section"> <span class="a-size-medium a-color-price"> Stokta sadece 1 adet kaldı. </span><script>noise()</script></div>
<div offer-display-feature-name="desktop-merchant-info" class="x"><span class="a-size-small offer-display-feature-text-message">Amazon.com.tr</span></div>
<span>Teslimat tarihinden sonraki 14 gün içinde iade edilebilir</span>
<table>
<tr><th class="prodDetSectionEntry"> Sabit Disk Boyutu </th><td> 512 GB </td></tr>
<tr><th class="prodDetSectionEntry"> İşletim Sistemi </th><td> DOS </td></tr>
<tr><th class="prodDetSectionEntry"> Grafik Açıklaması </th><td> Özel </td></tr>
<tr><th class="prodDetSectionEntry"> Ürünün Ağırlığı </th><td> 1.98 Kilogram </td></tr>
<tr><th class="prodDetSectionEntry"> Klavye Açıklaması </th><td> Arkadan Aydınlatmalı </td></tr>
<tr><th class="prodDetSectionEntry"> Garanti Bilgileri </th><td> 2 </td></tr>
<tr><th class="prodDetSectionEntry"> Model Numarası </th><td> A13VE-2269XTRNN </td></tr>
<tr><th class="prodDetSectionEntry"> En Çok Satanlar Sıralaması </th><td> Bilgisayar Şu kategoride 264. sırada: (<a href="/x">Bilgisayar Şu kategorideki En Popüler 100 Ürünü göster</a>) <span>Geleneksel Dizüstü Bilgisayarlar Şu kategoride 3. sırada:</span> </td></tr>
<tr><th class="prodDetSectionEntry"> ASIN </th><td> B0GHR4L88D </td></tr>
</table>
</body></html>"##;

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
            Some("https://m.media-amazon.com/images/I/71l4M2y8zwL._AC_SL1500_.jpg")
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
             ## MSI CYBORG 15 A13VE-2269XTRNN Dizüstü Bilgisayar\n\
             🏷️ Gülümseten Kasım fırsatı\n\
             # 49.911,94 TL\n\
             -# ~~56.999,00 TL~~ −%12\n\
             ⭐ 4,3 · 19 değerlendirme\n\
             Ücretsiz teslimat · 24 Kasım Salı · **Son 1 adet**\n\
             İşlemci: Intel Core I5-13420H · Depolama: 512GB SSD\n\
             Ekran kartı: RTX4050 GDDR6 6GB · Ağırlık: 1,98 kg\n\
             Bellek: 16GB RAM DDR5 · Klavye: Arkadan Aydınlatmalı\n\
             Ekran: 15.6 FHD 144Hz · Garanti: 2 yıl\n\
             -# Fiyat <t:1700000000:R> alındı · satıcı Amazon.com.tr · 14 gün iade"
        );
        assert_eq!(card.components[0]["type"], 9, "görselli bölüm");
        assert_eq!(card.components[1]["type"], 14, "ayraç");
    }

    #[test]
    fn bold_is_reserved_for_title_price_and_urgency() {
        // Okunabilirlik: künye, rozet, teslimat düz yazı; kalın yalnız aciliyette.
        let text = card_text(&build_card(&sample(), 1));
        assert_eq!(text.matches("**").count(), 2, "{text}");
        assert!(text.contains("**Son 1 adet**"));
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
            .replace(r#"data-a-strike="true""#, "")
            .replace("-%12", "")
            .replace("Yüzde 12 tasarruf ile ", "")
            .replace("dealBadgeSupportingText", "x")
            .replace("Stokta sadece 1 adet kaldı.", "Stokta var");
        let p = parse_product(&html, "https://www.amazon.com.tr/dp/B0GHR4L88D").unwrap();
        let text = card_text(&build_card(&p, 1));
        assert!(text.contains("\n# 49.911,94 TL"));
        assert!(!text.contains("~~") && !text.contains("🏷️") && !text.contains("Son "));
        assert!(text.contains("Ücretsiz teslimat · 24 Kasım Salı"));
    }

    #[test]
    fn missing_price_drops_price_lines_and_fetch_time() {
        let html = SAMPLE.replace("apex-pricetopay-accessibility-label", "x");
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
        let html = SAMPLE.replace("data-old-hires", "data-x");
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

    #[test]
    fn captcha_page_is_recognized_and_product_page_is_not() {
        let captcha = r#"<html><head><title dir="ltr">Amazon.com.tr</title></head><body>
<form method="get" action="/errors/validateCaptcha" name=""><input type=hidden name="amzn" value="x"></form></body></html>"#;
        assert!(is_captcha(captcha));
        assert!(parse_product(captcha, "https://www.amazon.com.tr/dp/B0GHR4L88D").is_none());
        assert!(!is_captcha(SAMPLE));
    }

    #[test]
    fn price_region_cut_never_splits_a_turkish_letter() {
        // Kesim noktası (işaret + 8000 bayt) iki baytlık "ş"nin ortasına denk gelsin.
        let marker = "apex-pricetopay-accessibility-label";
        for pad in 0..4 {
            let html = format!(
                "<span id=\"{marker}\">{}{}",
                "a".repeat(8000 - marker.len() - 10 + pad),
                "ş".repeat(50)
            );
            let _ = price_region(&html);
        }
    }

    #[test]
    fn savings_takes_only_the_percentage_and_store_follows_host() {
        assert_eq!(normalize_savings("%12 (₺6.000 tasarruf)"), "−%12");
        assert_eq!(normalize_savings("-%12"), "−%12");
        assert_eq!(normalize_savings("tasarruf"), "");
        let p = parse_product(SAMPLE, "https://www.amazon.com/dp/B0GHR4L88D").unwrap();
        assert_eq!(p.store, "Amazon.com");
        assert_eq!(p.clean_url, "https://www.amazon.com/dp/B0GHR4L88D");
    }
}
