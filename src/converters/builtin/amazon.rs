//! Amazon ürün bağlantısı (`amazon.com.tr/.../dp/{ASIN}` ve `amzn.eu/d/{token}` kısa linki).
//!
//! Ürün sayfası sunucuda çizilir; fiyat, puan, stok ve teknik künye doğrudan HTML'den okunur
//! (ayrı bir API yok). Discord'un kendi önizlemesi başlığı kırpar, fiyat göstermez ve küçük
//! görsel olarak Amazon'un reklam şeridini koyar; bunun yerine temiz bir Components V2 kartı
//! üretilir: mağaza · marka, ürün adı, fiyat (varsa üstü çizili eski fiyat + indirim), puan ·
//! değerlendirme · stok, kısa künye ve altta tek "Amazon'da aç" butonu (izleme parametresiz link).
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

/// Gerçek tarayıcı UA'sı: bot UA'sıyla Amazon sık sık içeriği boşaltılmış sayfa döndürür.
const BROWSER_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

const AMAZON_ORANGE: u32 = 0xFF9900;
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);
const MAX_SPECS: usize = 4;
const MAX_TITLE_CHARS: usize = 150;

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
/// Üstü çizili (liste / 30 günün en düşüğü) fiyat: `a-text-price` + `data-a-strike="true"`.
static STRIKE_PRICE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)class="a-price a-text-price"[^>]*data-a-strike="true"[^>]*>\s*<span class="a-offscreen">\s*([^<]+?)\s*</span>"#)
        .expect("geçerli eski fiyat regex")
});
static SAVINGS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)savingsPercentage[^>]*>\s*([^<]+?)\s*<"#).expect("geçerli indirim regex")
});
static RATING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)id="acrPopover"[^>]*title="[^"]*?([0-9]+[.,][0-9])\s*""#)
        .expect("geçerli puan regex")
});
static REVIEWS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)id="acrCustomerReviewText"[^>]*>\s*\(?\s*([0-9][0-9.]*)"#)
        .expect("geçerli yorum sayısı regex")
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
static TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<[^>]+>").expect("geçerli etiket regex"));

/// Künyeye alınmayan gürültü satırları (kart için anlamsız alanlar).
const SPEC_SKIP: &[&str] = &[
    "asin",
    "en çok satanlar",
    "müşteri yorumları",
    "garanti",
    "model numarası",
    "orijin ülke",
    "öğelerin sayısı",
    "pil",
    "lityum",
];

struct Product {
    title: String,
    clean_url: String,
    /// Kartın üst satırındaki mağaza adı, bağlantının host'undan ("Amazon.com.tr").
    store: String,
    brand: Option<String>,
    price_now: Option<String>,
    price_was: Option<String>,
    savings: Option<String>,
    rating: Option<String>,
    reviews: Option<String>,
    availability: Option<String>,
    image: Option<String>,
    /// Kısa künye parçaları (tek satırda `·` ile birleşir).
    specs: Vec<String>,
}

pub async fn fetch(client: &reqwest::Client, parsed: &ParsedPost) -> Result<BuiltinPost, String> {
    let html = get_html(client, &parsed.original_url).await?;
    let product = parse_product(&html, &parsed.original_url)
        .ok_or_else(|| format!("Amazon ürünü okunamadı: {}", parsed.original_url))?;

    let mut post = BuiltinPost::new(
        Platform::Amazon,
        product.clean_url.clone(),
        Author::new("Amazon", ""),
    );
    post.title = Some(product.title.clone());
    post.extra_label = Some("Amazon".to_string());
    post.custom_card = Some(build_card(&product));
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

    let full_title = TITLE
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| clean_text(m.as_str()))
        .filter(|t| !t.is_empty())?;
    // Satıcılar teknik özellikleri başlığa virgülle dizer: ilk virgüle kadarı ürün adı,
    // kalanı künye. Baş kısım çok kısaysa (ör. "Krem, 80 ml") başlık bölünmez.
    let (head, tail) = match full_title.split_once(", ") {
        Some((head, tail)) if head.chars().count() >= 20 => (head.to_string(), Some(tail)),
        _ => (full_title.clone(), None),
    };
    let title = truncate_chars(&head, MAX_TITLE_CHARS);

    // Fiyatlar yalnızca ödenecek fiyat bloğundan okunur; taksit, "benzer ürün" vb. fiyatlar
    // kartı yanıltmasın.
    let price_region = price_region(html);
    let price_now = PRICE_TO_PAY
        .captures(price_region)
        .and_then(|c| c.get(1))
        .map(|m| clean_text(m.as_str()))
        .filter(|p| !p.is_empty());
    let price_was = STRIKE_PRICE
        .captures(price_region)
        .and_then(|c| c.get(1))
        .map(|m| clean_text(m.as_str()))
        .filter(|p| !p.is_empty() && Some(p) != price_now.as_ref());
    let savings = SAVINGS
        .captures(price_region)
        .and_then(|c| c.get(1))
        .map(|m| normalize_savings(m.as_str()))
        .filter(|s| !s.is_empty());

    let rating = RATING
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string());
    let reviews = REVIEWS
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string());
    let availability = availability(html);
    let image = IMAGE
        .captures(html)
        .and_then(|c| c.get(1).or_else(|| c.get(2)).or_else(|| c.get(3)))
        .map(|m| m.as_str().to_string());
    let brand = brand(html);
    let specs = match tail {
        Some(tail) => tail
            .split(", ")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .take(MAX_SPECS)
            .map(|s| truncate_chars(s, 40))
            .collect(),
        None => table_specs(html),
    };

    Some(Product {
        title,
        clean_url,
        store,
        brand,
        price_now,
        price_was,
        savings,
        rating,
        reviews,
        availability,
        image,
        specs,
    })
}

/// Çekirdek fiyat bloğundan başlayan dilim (yoksa tüm HTML).
fn price_region(html: &str) -> &str {
    for marker in [
        "apex-pricetopay-accessibility-label",
        "corePriceDisplay_desktop_feature_div",
    ] {
        if let Some(i) = html.find(marker) {
            let start = html[..i].rfind('<').unwrap_or(i);
            // Bayt sınırı Türkçe harfin ortasına düşerse dilimleme panikler.
            let mut end = (i + 8000).min(html.len());
            while !html.is_char_boundary(end) {
                end -= 1;
            }
            return &html[start..end];
        }
    }
    html
}

fn availability(html: &str) -> Option<String> {
    // Bloğun ilk span'ı stok cümlesidir; içindeki <script> vb. gürültü sonraki span'lardadır.
    let value = AVAILABILITY
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| clean_text(m.as_str()))?;
    // Sıradan "Stokta var" kart için bilgi taşımaz; yalnız az stok / tükendi gösterilir.
    let ordinary = value
        .trim_end_matches('.')
        .eq_ignore_ascii_case("Stokta var")
        || value.eq_ignore_ascii_case("In Stock");
    (!value.is_empty() && !ordinary).then(|| truncate_chars(&value, 40))
}

fn brand(html: &str) -> Option<String> {
    let raw = BYLINE
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| clean_text(m.as_str()))?;
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

/// Başlıkta künye yoksa teknik tablodan ilk anlamlı satırlar ("Anahtar: değer").
fn table_specs(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    for caps in DETAIL_ROW.captures_iter(html) {
        if out.len() >= MAX_SPECS {
            break;
        }
        let (Some(k), Some(v)) = (caps.get(1), caps.get(2)) else {
            continue;
        };
        let key = clean_text(k.as_str());
        let value = clean_text(v.as_str());
        let lower = key.to_lowercase();
        if key.is_empty()
            || value.is_empty()
            || value.eq_ignore_ascii_case("Özel")
            || SPEC_SKIP.iter().any(|s| lower.contains(s))
        {
            continue;
        }
        out.push(format!(
            "{}: {}",
            truncate_chars(&key, 28),
            truncate_chars(&value, 40)
        ));
    }
    out
}

fn build_card(p: &Product) -> CustomCard {
    // Üst satır (küçük): mağaza · marka, altında kalın ürün adı (temiz linkle).
    let mut head = format!("-# {}", p.store);
    if let Some(brand) = &p.brand {
        head.push_str(&format!(" · {}", escape(brand)));
    }
    head.push_str(&format!("\n**[{}]({})**", escape(&p.title), p.clean_url));

    // Fiyat satırı: güncel kalın, varsa üstü çizili eski fiyat + indirim; altında puan · stok.
    let mut info = String::new();
    if let Some(now) = &p.price_now {
        info.push_str(&format!("**{}**", escape(now)));
        if let Some(was) = &p.price_was {
            info.push_str(&format!("  ~~{}~~", escape(was)));
        }
        if let Some(s) = &p.savings {
            info.push_str(&format!("  {}", escape(s)));
        }
    }
    let status = status_line(p);
    if !status.is_empty() {
        if !info.is_empty() {
            info.push('\n');
        }
        info.push_str(&format!("-# {status}"));
    }

    let mut section_children = vec![text(head)];
    if !info.is_empty() {
        section_children.push(text(info));
    }

    // Discord, aksesuarsız Section'ı (tip 9) reddeder: görsel yoksa metin doğrudan konur.
    let mut components = match p.image.as_deref() {
        Some(url) => vec![json!({
            "type": 9,
            "components": section_children,
            "accessory": { "type": 11, "media": { "url": url }, "description": "Ürün görseli" },
        })],
        None => section_children,
    };
    if !p.specs.is_empty() {
        let line: Vec<String> = p.specs.iter().map(|s| escape(s)).collect();
        components.push(json!({ "type": 14, "spacing": 1 }));
        components.push(text(format!("-# {}", line.join(" · "))));
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

fn status_line(p: &Product) -> String {
    let mut parts = Vec::new();
    if let Some(rating) = &p.rating {
        let mut s = format!("★ {rating}");
        if let Some(reviews) = &p.reviews {
            s.push_str(&format!(" · {reviews} değerlendirme"));
        }
        parts.push(s);
    }
    if let Some(av) = &p.availability {
        parts.push(escape(av));
    }
    parts.join(" · ")
}

fn text(content: impl Into<String>) -> Value {
    json!({ "type": 10, "content": content.into() })
}

async fn get_html(client: &reqwest::Client, url: &str) -> Result<String, String> {
    let response = client
        .get(url)
        .header("user-agent", BROWSER_USER_AGENT)
        .header(
            "accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header("accept-language", "tr-TR,tr;q=0.9,en;q=0.8")
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("Amazon sayfası alınamadı ({url}): {e}"))?;

    if !response.status().is_success() {
        return Err(format!(
            "Amazon sayfası {} döndü ({url})",
            response.status()
        ));
    }
    super::read_limited_text(response, url).await
}

/// HTML'den düz metin: etiketleri at, HTML varlıklarını çöz, boşlukları toparla.
fn clean_text(raw: &str) -> String {
    let no_tags = TAG.replace_all(raw, " ");
    unescape_html(&no_tags)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// "-%12", "%12 indirim" vb. → "−%12".
fn normalize_savings(raw: &str) -> String {
    // Yalnız ilk sayı: "%12 (₺6.000 tasarruf)" gibi metinde tutar rakamları karışmasın.
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
<div id="corePriceDisplay_desktop_feature_div">
  <span id="apex-pricetopay-accessibility-label" class="aok-offscreen"> 49.911,94&nbsp;TL </span>
  <span class="a-price priceToPay"><span class="a-offscreen"> </span></span>
  <span class="savingsPercentage">-%12</span>
  <span class="a-price a-text-price" data-a-size="s" data-a-strike="true" data-a-color="secondary"><span class="a-offscreen">56.999,00 TL</span></span>
</div>
<div id="availability" class="a-section"> <span class="a-size-medium a-color-price"> Stokta sadece 1 adet kaldı. </span><script>noise()</script></div>
</body></html>"##;

    #[test]
    fn parses_core_fields_and_clean_url() {
        let p = parse_product(SAMPLE, "https://www.amazon.com.tr/dp/B0GHR4L88D?ref=spam").unwrap();
        assert_eq!(p.title, "MSI CYBORG 15 A13VE-2269XTRNN Dizüstü Bilgisayar");
        assert_eq!(p.clean_url, "https://www.amazon.com.tr/dp/B0GHR4L88D");
        assert_eq!(p.store, "Amazon.com.tr");
        // Sayfanın başındaki ilgisiz "1.234,00TL" değil, ödenecek fiyat okunur.
        assert_eq!(p.price_now.as_deref(), Some("49.911,94 TL"));
        assert_eq!(p.price_was.as_deref(), Some("56.999,00 TL"));
        assert_eq!(p.savings.as_deref(), Some("−%12"));
        assert_eq!(p.rating.as_deref(), Some("4,3"));
        assert_eq!(p.reviews.as_deref(), Some("19"));
        assert_eq!(
            p.availability.as_deref(),
            Some("Stokta sadece 1 adet kaldı.")
        );
        assert_eq!(p.brand.as_deref(), Some("MSI"));
        assert_eq!(
            p.image.as_deref(),
            Some("https://m.media-amazon.com/images/I/71l4M2y8zwL._AC_SL1500_.jpg")
        );
        assert_eq!(
            p.specs,
            [
                "Intel Core I5-13420H",
                "15.6 FHD 144Hz",
                "16GB RAM DDR5",
                "RTX4050 GDDR6 6GB"
            ]
        );
    }

    #[test]
    fn short_link_resolves_asin_from_page_and_card_has_one_clean_button() {
        let p = parse_product(SAMPLE, "https://amzn.eu/d/04JNI9Sx").unwrap();
        assert_eq!(p.clean_url, "https://www.amazon.com.tr/dp/B0GHR4L88D");
        let card = build_card(&p);
        let blob = serde_json::to_string(&card.components).unwrap();
        assert!(blob.contains("**49.911,94 TL**  ~~56.999,00 TL~~  −%12"));
        assert!(blob.contains("★ 4,3 · 19 değerlendirme · Stokta sadece 1 adet kaldı."));
        assert!(blob.contains("Intel Core I5-13420H · 15.6 FHD 144Hz"));
        assert_eq!(card.buttons.len(), 1, "kart altında tek buton");
        let row = &card.buttons[0]["components"];
        assert_eq!(row.as_array().unwrap().len(), 1);
        assert_eq!(row[0]["url"], "https://www.amazon.com.tr/dp/B0GHR4L88D");
        assert_eq!(row[0]["label"], "Amazon'da aç");
    }

    #[test]
    fn no_strike_price_and_ordinary_stock_are_left_out() {
        let html = SAMPLE
            .replace(r#"data-a-strike="true""#, "")
            .replace("-%12", "")
            .replace("Stokta sadece 1 adet kaldı.", "Stokta var");
        let p = parse_product(&html, "https://www.amazon.com.tr/dp/B0GHR4L88D").unwrap();
        assert_eq!(p.price_was, None);
        assert_eq!(p.availability, None);
        let blob = serde_json::to_string(&build_card(&p).components).unwrap();
        assert!(!blob.contains("~~"));
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
    fn card_without_image_has_no_accessoryless_section() {
        let html = SAMPLE.replace("data-old-hires", "data-x");
        let p = parse_product(&html, "https://www.amazon.com.tr/dp/B0GHR4L88D").unwrap();
        assert_eq!(p.image, None);
        let card = build_card(&p);
        assert!(card.components.iter().all(|c| c["type"] != 9));
        assert!(card.components.iter().any(|c| {
            c["content"]
                .as_str()
                .is_some_and(|t| t.contains("49.911,94 TL"))
        }));
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

    #[test]
    fn missing_price_still_builds_a_card() {
        let html = SAMPLE.replace("apex-pricetopay-accessibility-label", "x");
        let p = parse_product(&html, "https://www.amazon.com.tr/dp/B0GHR4L88D").unwrap();
        assert_eq!(p.price_now, None);
        assert_eq!(build_card(&p).buttons.len(), 1);
    }
}
