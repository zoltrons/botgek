//! Twitter / X gönderi verisi.
//!
//! - **FxTwitter** (`api.fxtwitter.com`): **ana kaynak**. Tek yanıtta tam metin (uzun "note"
//!   gönderileri dahil), medya, ilk seviye alıntı, görüntülenme dahil tüm metrikler ve topluluk
//!   notu gelir; dil ekli uç (`/tr`) kök gönderiyi ve alıntıyı X'in Grok çevirisiyle verir.
//! - **Syndication** (`cdn.syndication.twimg.com/tweet-result`): X'in kendi ucu. FxTwitter hata
//!   verir ya da gecikirse yedektir (görüntülenme ve çeviri yok, uzun metin kesik).
//! - **VxTwitter** (`api.vxtwitter.com`): yalnızca `/dönüştür kaynak:vx` ile elle seçilir;
//!   FxTwitter'ın medyayı (ör. GIF) bozduğu gönderiler içindir.
//!
//! Alıntı zinciri yalnızca ilk seviyede gösterilir (alıntının alıntısı çekilmez).

use std::future::Future;
use std::time::{Duration, Instant};

use regex::Regex;
use serde::Deserialize;
use std::sync::LazyLock;
use tracing::{debug, warn};

use super::media::card_image_url;
use super::preview::meta_value;
use super::translate::needs_translation;
use super::{
    Author, BuiltinPost, LinkPreview, Media, MediaKind, Stats, Translation, parse_timestamp,
    strip_trailing_media_links,
};
use crate::converters::parser::{ParsedPost, Platform};
use crate::settings::EngineSettings;

#[cfg(test)]
mod benchmarks;

const SYNDICATION_URL: &str = "https://cdn.syndication.twimg.com/tweet-result";
const FXTWITTER_URL: &str = "https://api.fxtwitter.com";
const VXTWITTER_URL: &str = "https://api.vxtwitter.com";

static TWEET_URL_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:twitter\.com|x\.com)/([A-Za-z0-9_]{1,20})/status/(\d{1,25})")
        .expect("geçerli tweet regex")
});

/// Ana kaynağa (FxTwitter) tanınan süre. X syndication ucu baştan paralel istenir: ana kaynak bu
/// sürede yanıt vermezse kart hazır syndication verisiyle (görüntülenmesiz, çevirisiz) gider,
/// yavaş ya da askıda kalan uç kartı geciktirmez (ölçülen FxTwitter: sıcak ~0,3 sn, önbelleğinde
/// olmayan gönderide ~0,9 sn'ye kadar).
const PRIMARY_PATIENCE: Duration = Duration::from_millis(800);
/// Çeviri gereken gönderide dil ekli yanıta, isteklerin başından itibaren tanınan süre. Dolarsa
/// kart orijinal metinle gider (ölçülen FxTwitter `/tr`: ~0,5-0,8 sn).
const TRANSLATION_BUDGET: Duration = Duration::from_millis(1_200);
/// Harici link önizlemesi (X'in "link kartı") için tanınan süre: hedef sayfa bu süre
/// içinde yanıt vermezse kart önizlemesiz gönderilir (kart asla bekletilmez).
const LINK_PREVIEW_TIMEOUT: Duration = Duration::from_millis(500);
/// Önizleme metinlerinin kart bütçesini yormaması için üst sınırlar.
const LINK_PREVIEW_TITLE_LIMIT: usize = 180;
const LINK_PREVIEW_DESCRIPTION_LIMIT: usize = 400;

/// Metindeki bağlantıları bulur (X'in kendi adresleri ayrıca ayıklanır).
static URL_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"https?://[^\s<>()\[\]]+").expect("geçerli URL regex"));
/// X/Twitter'a ait adresler link kartına konmaz (gönderi/alıntı/medya bağlantıları).
static OWN_HOSTS: [&str; 5] = [
    "x.com",
    "twitter.com",
    "t.co",
    "pic.twitter.com",
    "twimg.com",
];

/// Çekilen X gönderisi.
pub struct Fetched {
    pub post: BuiltinPost,
    /// `false`: sonuç eksik (çeviri gerekiyordu ama gelmedi ya da yedek syndication verisi
    /// kullanıldı). Önbelleğe yazılmamalı: sonraki paylaşım ana kaynağı yeniden dener.
    pub cacheable: bool,
}

/// X verisinin kaynağı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    /// Ana kaynak: her zaman kullanılır.
    Fx,
    /// Yalnızca `/dönüştür kaynak:vx` ile ([`EngineSettings::x_alternative_source`]).
    Vx,
}

/// Gönderi verisini çeker: ana kaynak FxTwitter (elle seçilirse vxTwitter), yedek X syndication.
///
/// Düz yanıt ve (otomatik çeviri açıksa) dil ekli yanıt **paralel** istenir: gönderinin dili yanıt
/// gelmeden bilinmez, dil ekli yanıt ise düz yanıtın tamamını da içerir. Çeviri gerekmiyorsa kart
/// düz yanıtla hemen gider ve dil ekli istek bırakılır; gerekiyorsa [`TRANSLATION_BUDGET`] kadar
/// beklenir (bkz. [`acquire`]).
pub async fn fetch(
    client: &reqwest::Client,
    parsed: &ParsedPost,
    engine: &EngineSettings,
) -> Result<Fetched, String> {
    let id = extract_tweet_id(&parsed.original_url)
        .ok_or_else(|| format!("Twitter kimliği çözümlenemedi: {}", parsed.original_url))?;
    let url = parsed.original_url.as_str();

    // X videosu indirilip yüklenmez, CDN'den uzaktan oynar: Discord yükleme limiti ve
    // `max_video_mb` geçerli değil, her zaman en yüksek bit hızlı (1080p) varyant seçilir.
    let video_budget = u64::MAX;
    let source = if engine.x_alternative_source {
        Source::Vx
    } else {
        Source::Fx
    };
    debug!("X verisi çekiliyor (kaynak: {source:?}, id: {id})");

    let started = Instant::now();
    let translated = engine.auto_translate.then(|| {
        request(
            client,
            source,
            url,
            &id,
            Some(engine.translation_target.as_str()),
            engine,
            video_budget,
        )
    });
    let acquired = acquire(
        engine,
        request(client, source, url, &id, None, engine, video_budget),
        translated,
        from_syndication(client, url, &id, video_budget),
        PRIMARY_PATIENCE,
        TRANSLATION_BUDGET,
    );
    let mut fetched = tokio::time::timeout(super::FETCH_FIRST_DATA_BUDGET, acquired)
        .await
        .map_err(|_| format!("X verisi zamanında gelmedi: {id}"))??;
    let data_elapsed = started.elapsed();

    // Gönderi yalnızca link içeriyorsa X'in kendi arayüzünde gösterdiği link kartı (hedef
    // sayfanın önizlemesi) da karta eklenir.
    attach_link_preview(&mut fetched.post).await;

    debug!(
        "X veri fazları ({id}): veri {} ms, link önizleme {} ms, önbelleğe uygun: {}",
        data_elapsed.as_millis(),
        (started.elapsed() - data_elapsed).as_millis(),
        fetched.cacheable,
    );
    Ok(fetched)
}

/// Kaynağa tek istek; `language` verilirse dil ekli (çevirili) uç kullanılır.
async fn request(
    client: &reqwest::Client,
    source: Source,
    url: &str,
    id: &str,
    language: Option<&str>,
    engine: &EngineSettings,
    video_budget: u64,
) -> Result<BuiltinPost, String> {
    match source {
        Source::Fx => {
            from_fxtwitter(
                client,
                url,
                id,
                language,
                &engine.translation_target,
                video_budget,
                engine.translate_english,
            )
            .await
        }
        Source::Vx => {
            from_vxtwitter(
                client,
                url,
                id,
                language,
                &engine.translation_target,
                engine.translate_english,
            )
            .await
        }
    }
}

/// Kaynak yanıtlarını birleştirir. Ağdan bağımsızdır: testler sahte future'larla çalıştırır.
///
/// - İlk veri [`first_data`] ile alınır (`primary` ile `fallback` paralel; `primary` tercih edilir).
/// - `translated` düz yanıttan önce gelirse doğrudan kullanılır (düz yanıtın üst kümesidir).
/// - Düz yanıtta çevrilecek metin yoksa `translated` beklenmez ve bırakılır; varsa başlangıçtan
///   itibaren `budget` dolana kadar beklenir, dolarsa orijinal metinle devam edilir.
async fn acquire<P, T, F>(
    engine: &EngineSettings,
    primary: P,
    translated: Option<T>,
    fallback: F,
    patience: Duration,
    budget: Duration,
) -> Result<Fetched, String>
where
    P: Future<Output = Result<BuiltinPost, String>>,
    T: Future<Output = Result<BuiltinPost, String>>,
    F: Future<Output = Result<BuiltinPost, String>>,
{
    let deadline = tokio::time::Instant::now() + budget;
    let first = first_data(primary, fallback, patience);
    tokio::pin!(first);
    let Some(translated) = translated else {
        let (post, from_fallback) = first.await?;
        return Ok(Fetched {
            post,
            cacheable: !from_fallback,
        });
    };
    tokio::pin!(translated);

    let (post, from_fallback) = tokio::select! {
        biased;
        result = &mut first => match result {
            Ok(first) => first,
            // Düz yanıt da yedek de başarısız: dil ekli yanıt hâlâ gönderinin tamamını getirebilir.
            Err(err) => {
                return translated
                    .await
                    .map(|post| Fetched {
                        cacheable: !translation_missing(&post, engine),
                        post,
                    })
                    .map_err(|tr_err| format!("{err} | {tr_err}"));
            }
        },
        result = &mut translated => match result {
            Ok(post) => {
                return Ok(Fetched {
                    cacheable: !translation_missing(&post, engine),
                    post,
                });
            }
            Err(err) => {
                debug!("Dil ekli X isteği başarısız: {err}");
                let (post, from_fallback) = first.await?;
                return Ok(Fetched {
                    cacheable: !from_fallback && !translation_missing(&post, engine),
                    post,
                });
            }
        },
    };

    if !translation_missing(&post, engine) {
        return Ok(Fetched {
            post,
            cacheable: !from_fallback,
        });
    }
    match tokio::time::timeout_at(deadline, &mut translated).await {
        Ok(Ok(translated)) => Ok(Fetched {
            cacheable: !translation_missing(&translated, engine),
            post: translated,
        }),
        Ok(Err(err)) => {
            debug!("Dil ekli X isteği başarısız, kart çevirisiz gidiyor: {err}");
            Ok(Fetched {
                post,
                cacheable: false,
            })
        }
        Err(_) => {
            debug!(
                "X çevirisi {} ms içinde gelmedi, kart çevirisiz gidiyor: {}",
                budget.as_millis(),
                post.url
            );
            Ok(Fetched {
                post,
                cacheable: false,
            })
        }
    }
}

/// İlk kullanılabilir veri. `primary` ve `fallback` **baştan paralel** yürür:
///
/// - `patience` dolana kadar `primary` beklenir (`fallback` erken gelse bile); gelirse kazanır.
/// - Süre dolarsa ilk başarılı yanıt kazanır: `fallback` hazırsa kart hiç beklemez.
/// - `primary` hata verirse doğrudan `fallback`'e düşülür.
///
/// İkinci değer: sonuç yedekten mi geldi.
async fn first_data<P, F>(
    primary: P,
    fallback: F,
    patience: Duration,
) -> Result<(BuiltinPost, bool), String>
where
    P: Future<Output = Result<BuiltinPost, String>>,
    F: Future<Output = Result<BuiltinPost, String>>,
{
    tokio::pin!(primary);
    tokio::pin!(fallback);
    let deadline = tokio::time::Instant::now() + patience;
    let mut early_fallback = None;

    let primary_result = loop {
        tokio::select! {
            biased;
            result = &mut primary => break Some(result),
            result = &mut fallback, if early_fallback.is_none() => early_fallback = Some(result),
            _ = tokio::time::sleep_until(deadline) => break None,
        }
    };
    match primary_result {
        Some(Ok(post)) => return Ok((post, false)),
        Some(Err(err)) => {
            warn!("X ana kaynağı başarısız, syndication kullanılıyor: {err}");
            let fallback_result = match early_fallback {
                Some(result) => result,
                None => fallback.await,
            };
            return fallback_result
                .map(|post| (post, true))
                .map_err(|fb_err| format!("{err} | {fb_err}"));
        }
        None => debug!(
            "X ana kaynağı {} ms içinde yanıt vermedi; syndication verisi kullanılabilir",
            patience.as_millis()
        ),
    }

    // Ana kaynağa tanınan süre doldu: ilk başarılı yanıt kazanır.
    match early_fallback {
        Some(Ok(post)) => return Ok((post, true)),
        Some(Err(fb_err)) => {
            return primary
                .await
                .map(|post| (post, false))
                .map_err(|err| format!("{err} | {fb_err}"));
        }
        None => {}
    }
    tokio::select! {
        biased;
        result = &mut primary => match result {
            Ok(post) => Ok((post, false)),
            Err(err) => fallback
                .await
                .map(|post| (post, true))
                .map_err(|fb_err| format!("{err} | {fb_err}")),
        },
        result = &mut fallback => match result {
            Ok(post) => Ok((post, true)),
            Err(fb_err) => primary
                .await
                .map(|post| (post, false))
                .map_err(|err| format!("{err} | {fb_err}")),
        },
    }
}

/// Gönderide ya da görünür alıntısında çevrilmesi gereken ama çevrilmemiş metin var mı?
fn translation_missing(post: &BuiltinPost, engine: &EngineSettings) -> bool {
    let missing = |post: &BuiltinPost| {
        post.translation.is_none()
            && !post.text.trim().is_empty()
            && needs_translation(
                post.language.as_deref(),
                &engine.translation_target,
                engine.translate_english,
            )
    };
    engine.auto_translate
        && (missing(post) || (engine.show_quote && post.quote.as_deref().is_some_and(missing)))
}

/// Gönderi yalnızca bir **harici bağlantı** içeriyorsa X'in "link kartı" önizlemesini ekler.
///
/// X, medyasız (ve alıntısız) gönderilerde metindeki harici adresin önizlemesini gösterir:
/// hedef sayfanın `og:`/`twitter:` etiketleri okunur. Hiçbir medya indirilmez; önizleme
/// görseli doğrudan CDN bağlantısıyla karta bağlanır. Sayfa yanıt vermezse, etiketler boşsa
/// ya da [`LINK_PREVIEW_TIMEOUT`] dolarsa kart önizlemesiz gönderilir.
async fn attach_link_preview(post: &mut BuiltinPost) {
    if !post.media.is_empty() || post.quote.is_some() {
        return;
    }
    let Some(url) = external_link(&post.text) else {
        return;
    };
    let Some(preview) = fetch_link_preview(&url).await else {
        return;
    };
    debug!(
        "Link kartı eklendi: {} ({})",
        preview.url,
        preview.site_name.as_deref().unwrap_or("-")
    );
    post.link_preview = Some(preview);
}

/// Harici sayfanın link kartını [`LINK_PREVIEW_TIMEOUT`] içinde çözer; sayfa yanıt vermezse
/// ya da gösterilecek etiket yoksa `None` (kart önizlemesiz gönderilir).
async fn fetch_link_preview(url: &str) -> Option<LinkPreview> {
    let (target, html) =
        match tokio::time::timeout(LINK_PREVIEW_TIMEOUT, super::safe_http::preview(url)).await {
            Ok(Ok(result)) => result,
            Ok(Err(err)) => {
                debug!("Link önizlemesi alınamadı ({url}): {err}");
                return None;
            }
            Err(_elapsed) => {
                debug!(
                    "Link önizlemesi {} ms içinde gelmedi: {url}",
                    LINK_PREVIEW_TIMEOUT.as_millis()
                );
                return None;
            }
        };
    let preview = build_link_preview(&target, &html);
    if preview.is_none() {
        debug!("Link önizlemesinde gösterilecek veri yok: {target}");
    }
    preview
}

/// Çekilen sayfanın OG etiketlerinden karta konacak link önizlemesini üretir.
///
/// Değerler üçüncü taraf sayfadan geldiği için Markdown'a güvenli hale getirilir: başlık V2 kartında zaten kaçırıldığından yalnızca düz metne indirgenir, açıklama ve site adı ise ham basıldığından burada kaçırılır.
/// Bağlantı hedefi `[etiket](url)` içinde durduğundan `)`/`[`/`]` yüzde kodlanır; aksi halde sayfa sahte bir maskeli bağlantı enjekte edebilir.
fn build_link_preview(target: &str, html: &str) -> Option<LinkPreview> {
    let page = url::Url::parse(target).ok()?;
    let graph = super::opengraph::parse_open_graph(html);
    // The fetched, validated URL is canonical here; untrusted og:url cannot replace it.
    let preview = LinkPreview {
        site_name: meta_value(graph.site_name, 80)
            .or_else(|| host_label(target))
            .map(|site| super::preview::escape_preview_markdown(&site)),
        title: meta_value(graph.title, LINK_PREVIEW_TITLE_LIMIT)
            .map(|title| super::preview::plain_text(&title)),
        description: meta_value(graph.description, LINK_PREVIEW_DESCRIPTION_LIMIT)
            .map(|description| super::preview::escape_preview_markdown(&description)),
        image: graph.image.and_then(|raw| link_preview_image(&page, &raw)),
        url: super::preview::link_url(&page),
    };
    (!preview.is_empty()).then_some(preview)
}

/// `og:image` değerini sayfa adresine göre mutlak bir http(s) adresine çözer.
///
/// Göreli (`/x.png`, `//cdn/x.png`) ya da kırpılmış bir adres Discord'un tüm kartı 400 ile reddetmesine yol açar; bu yüzden adresler asla kısaltılmaz, sınırı aşan görsel bırakılır.
fn link_preview_image(page: &url::Url, raw: &str) -> Option<String> {
    const MAX_IMAGE_URL_LEN: usize = 2048;
    let image = page.join(raw.trim()).ok()?;
    (matches!(image.scheme(), "http" | "https")
        && image.host_str().is_some()
        && image.as_str().len() <= MAX_IMAGE_URL_LEN)
        .then(|| image.to_string())
}

/// Metindeki ilk **harici** bağlantıyı döndürür (X/Twitter adresleri atlanır).
fn external_link(text: &str) -> Option<String> {
    URL_REGEX
        .find_iter(text)
        .map(|found| {
            found
                .as_str()
                .trim_end_matches(['.', ',', ';', ':', '!', '?'])
                .to_string()
        })
        .find(|url| {
            url::Url::parse(url)
                .ok()
                .and_then(|url| url.host_str().map(|host| !is_own_host(host)))
                .unwrap_or(false)
        })
}

/// X/Twitter'a ait alan adı mı? Alt alan adları da sayılır (`mobile.twitter.com`,
/// `www.x.com`, `pbs.twimg.com`): bunlar link kartı değil, gönderi/medya bağlantısıdır.
fn is_own_host(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    OWN_HOSTS.iter().any(|own| {
        host == *own
            || host
                .strip_suffix(own)
                .is_some_and(|prefix| prefix.ends_with('.'))
    })
}

/// Adresten alan adını çıkarır (`www.` ve alt alan adı olmadan): `https://www.bbc.com/x` →
/// `bbc.com`. Site adı bildirmeyen sayfalar için yedek etikettir.
fn host_label(url: &str) -> Option<String> {
    let url = url::Url::parse(url).ok()?;
    let host = url.host_str()?.trim_start_matches("www.");
    (!host.is_empty() && !is_own_host(host)).then(|| host.to_string())
}

/// Üç kaynağın ortak yazar biçimi: boş görünen ad kullanıcı adına düşer, profil adresi
/// yalnızca kullanıcı adı varsa kurulur.
fn x_author(name: Option<&str>, handle: &str, avatar: Option<String>, verified: bool) -> Author {
    let name = name
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(handle);
    let mut author = Author::new(name, handle);
    author.avatar = avatar;
    author.verified = verified;
    author.profile_url = (!handle.is_empty()).then(|| format!("https://x.com/{handle}"));
    author
}

/// Kimlik ve kullanıcı adından kanonik gönderi adresi. Kimlik sayısal olmalıdır; adres
/// biçimine uymayan (ya da boş) kullanıcı adı yerine X'in `i` yolu kullanılır.
fn status_url(handle: Option<&str>, id: &str) -> Option<String> {
    if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let handle = handle
        .filter(|handle| {
            !handle.is_empty()
                && handle
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
        .unwrap_or("i");
    Some(format!("https://x.com/{handle}/status/{id}"))
}

/// Kaynakların `twitter.com` adreslerini kartta tek biçime (`x.com`) indirir.
fn canonical_x_url(url: &str) -> String {
    url.replace("://twitter.com/", "://x.com/")
}

/// Milisaniye süreyi saniyeye yukarı yuvarlar; tüm kaynaklarda aynı süre gösterilir
/// (13,65 sn'lik video her yerde "0:14").
fn duration_secs(millis: u64) -> u64 {
    millis.div_ceil(1000)
}

/// Metindeki tweet bağlantısını `https://x.com/<kullanıcı>/status/<id>` olarak bulur.
///
/// Bot kartında gönderinin kendi linki `[Link](...)` satırındadır; alıntı başlığı
/// (`[Quote](...)`) ondan önce geldiği için önce o satır aranır, yoksa ilk tweet linki
/// alınır. fxtwitter/vxtwitter/fixupx gibi alan adları da eşleşir.
pub fn find_tweet_url(text: &str) -> Option<String> {
    let canonical = |captures: regex::Captures<'_>| {
        format!("https://x.com/{}/status/{}", &captures[1], &captures[2])
    };
    text.split("[Link](")
        .skip(1)
        .filter_map(|rest| rest.split(')').next())
        .find_map(|link| TWEET_URL_REGEX.captures(link))
        .or_else(|| TWEET_URL_REGEX.captures(text))
        .map(canonical)
}

/// `/gif-yap` için tweet'in videolarının (ve GIF'lerinin) FxEmbed GIF bağlantıları.
///
/// Her video için **480p** mp4 seçilir (yoksa altındaki en yakın varyant): servis videoyu
/// kare kare GIF'e çevirdiğinden 1080p kaynak onlarca MB'lık, Discord'da gömülemeyen
/// dosyalar üretir.
/// Gönderinin kendi videosu yoksa alıntılanan gönderininkiler kullanılır.
pub async fn gif_links(client: &reqwest::Client, url: &str) -> Result<Vec<String>, String> {
    let id = extract_tweet_id(url).ok_or("Geçerli bir X/Twitter gönderi bağlantısı değil.")?;
    let response: FxResponse =
        super::get_json_as(client, &format!("{FXTWITTER_URL}/i/status/{id}")).await?;
    let tweet = response
        .tweet
        .ok_or_else(|| format!("fxtwitter yanıtında gönderi yok: {id}"))?;

    let links = fx_gif_links(&tweet);
    if links.is_empty()
        && let Some(quote) = tweet.quote.as_deref()
    {
        return Ok(fx_gif_links(quote));
    }
    Ok(links)
}

fn fx_gif_links(tweet: &FxTweet) -> Vec<String> {
    let videos = tweet
        .media
        .as_ref()
        .and_then(|media| media.videos.as_deref())
        .unwrap_or_default();
    videos
        .iter()
        .filter_map(|video| {
            let chosen = video
                .formats
                .as_deref()
                .unwrap_or_default()
                .iter()
                .filter(|format| format.container.as_deref() == Some("mp4"))
                .min_by_key(|format| gif_variant_rank(format))
                .and_then(|format| format.url.as_deref());
            chosen
                .or(video.url.as_deref())
                .and_then(super::media::fx_gif_url)
        })
        .collect()
}

/// GIF kaynağı olarak hedeflenen kısa kenar (480p).
const GIF_TARGET_SHORT_SIDE: u64 = 480;

/// Varyant URL'sindeki çözünürlük: `.../vid/640x360/...` veya `.../vid/avc1/480x852/...`.
static VARIANT_SIZE_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/(\d{2,5})x(\d{2,5})/").expect("geçerli çözünürlük regex"));

/// Küçük sıra daha iyi: kısa kenarı 480'i aşmayan en büyük varyant (480p varsa o), yoksa
/// 480'in üstündeki en küçük. Çözünürlüğü bilinmeyen varyantlarda en düşük bit hızı.
fn gif_variant_rank(format: &FxFormat) -> (u8, u64) {
    let from_url = format.url.as_deref().and_then(|url| {
        let captures = VARIANT_SIZE_REGEX.captures(url)?;
        Some((captures[1].parse().ok()?, captures[2].parse().ok()?))
    });
    let size = match (format.width, format.height) {
        (Some(width), Some(height)) => Some((u64::from(width), u64::from(height))),
        _ => from_url,
    };
    match size.map(|(width, height)| width.min(height)) {
        Some(short) if short <= GIF_TARGET_SHORT_SIDE => (0, GIF_TARGET_SHORT_SIDE - short),
        Some(short) => (1, short),
        None => (2, format.bitrate.unwrap_or(u64::MAX)),
    }
}

/// Tweet URL'sinden gönderi kimliğini (sayısal ID) çıkarır.
fn extract_tweet_id(url: &str) -> Option<String> {
    TWEET_URL_REGEX
        .captures(url)
        .and_then(|c| c.get(2))
        .map(|m| m.as_str().to_string())
}

/// Gönderi verisini vxtwitter API'sinden çeker.
///
/// Dil eki (`/tr`) verildiğinde yanıt `translation` alanında X'in kendi Grok çevirisini
/// taşır; topluluk notu (`communityNote`) her yanıtta mevcuttur (yoksa null).
///
/// **Metin eksik olabilir:** uzun (note) gönderilerde vx metni ilk bağlantıda keser ve geri
/// kalanını hiç döndürmez (bkz. Wario64/2113285272523968617); görüntülenme de vermez. Bu yüzden
/// yalnızca elle seçilen yedek kaynaktır.
async fn from_vxtwitter(
    client: &reqwest::Client,
    original_url: &str,
    id: &str,
    language: Option<&str>,
    translation_target: &str,
    translate_english: bool,
) -> Result<BuiltinPost, String> {
    let language_suffix = language.map(|lang| format!("/{lang}")).unwrap_or_default();
    let handle = TWEET_URL_REGEX
        .captures(original_url)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
        .filter(|handle| !handle.is_empty())
        // vx yoldaki kullanıcı adını yalnızca adres biçimi için kullanır; her segment çalışır.
        .unwrap_or_else(|| "Twitter".to_string());
    let url = format!("{VXTWITTER_URL}/{handle}/status/{id}{language_suffix}");
    let tweet: VxTweet = super::get_json_as(client, &url).await?;

    if tweet.tweet_id.is_none() {
        return Err(format!("vxtwitter yanıtında gönderi kimliği yok: {id}"));
    }

    vx_to_post(&tweet, original_url, translation_target, translate_english)
}

/// vxtwitter JSON gönderi şeması. Şema içeriğe göre değişkendir (görsel/video/alıntı);
/// tüm alanlar `Option` + `#[serde(default)]` ile toleranslı çözümlenir.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct VxTweet {
    #[serde(rename = "tweetID")]
    tweet_id: Option<String>,
    #[serde(rename = "tweetURL")]
    tweet_url: Option<String>,
    text: Option<String>,
    #[serde(rename = "user_name")]
    user_name: Option<String>,
    #[serde(rename = "user_screen_name")]
    user_screen_name: Option<String>,
    #[serde(rename = "user_profile_image_url")]
    user_profile_image_url: Option<String>,
    #[serde(rename = "date_epoch")]
    date_epoch: Option<i64>,
    likes: Option<u64>,
    retweets: Option<u64>,
    replies: Option<u64>,
    lang: Option<String>,
    possibly_sensitive: Option<bool>,
    #[serde(rename = "media_extended")]
    media_extended: Vec<VxMedia>,
    qrt: Option<Box<VxTweet>>,
    #[serde(rename = "communityNote")]
    community_note: Option<String>,
    translation: Option<VxTranslation>,
    /// Saf retweet: vx gerçek içeriği buraya koyar, dış metni boşatır.
    retweet: Option<Box<VxTweet>>,
}

/// vxtwitter gönderisini [`BuiltinPost`]'a çevirir.
///
/// Alıntı yalnızca **ilk seviye** inline gömülür (`qrt`).
fn vx_to_post(
    tweet: &VxTweet,
    fallback_url: &str,
    translation_target: &str,
    translate_english: bool,
) -> Result<BuiltinPost, String> {
    let handle = tweet.user_screen_name.as_deref().unwrap_or_default();

    // Saf retweet: vx gerçek içeriği `retweet` objesine koyar ve dış metni boşatır.
    if let Some(inner) = tweet.retweet.as_deref() {
        let mut inner_post =
            vx_to_post(inner, fallback_url, translation_target, translate_english)?;
        inner_post.quote = None; // retweet içinde alıntı zinciri gösterilmez.
        return Ok(inner_post);
    }

    let author = x_author(
        tweet.user_name.as_deref(),
        handle,
        tweet.user_profile_image_url.clone(),
        false,
    );
    let url = tweet
        .tweet_url
        .clone()
        .filter(|url| !url.trim().is_empty())
        .or_else(|| status_url(Some(handle), tweet.tweet_id.as_deref()?))
        .unwrap_or_else(|| fallback_url.to_string());
    let mut post = BuiltinPost::new(Platform::Twitter, url, author);

    let raw_text = tweet.text.clone().unwrap_or_default();
    post.text = strip_trailing_media_links(&super::unescape_html(&raw_text));
    post.language = tweet.lang.clone().filter(|lang| !lang.trim().is_empty());
    post.created_at = tweet.date_epoch;
    post.stats = Stats {
        replies: tweet.replies.unwrap_or(0),
        reposts: tweet.retweets.unwrap_or(0),
        likes: tweet.likes.unwrap_or(0),
        quotes: 0,
        // vx görüntülenme sayısını bildirmez.
        views: None,
    };
    post.sensitive = tweet.possibly_sensitive.unwrap_or(false);
    post.media = tweet.media_extended.iter().filter_map(vx_media).collect();

    if let Some(quote) = tweet.qrt.as_deref()
        && let Ok(quote_post) = vx_to_post(quote, "", translation_target, translate_english)
    {
        post.quote = Some(Box::new(quote_post));
    }

    post.community_note = tweet
        .community_note
        .clone()
        .map(|note| note.trim().to_owned())
        .filter(|note| !note.is_empty());

    apply_vx_translation(&mut post, tweet, translation_target, translate_english);
    Ok(post)
}

#[derive(Debug, Deserialize, Default)]
struct VxMedia {
    url: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    size: Option<VxSize>,
    #[serde(rename = "thumbnail_url")]
    thumbnail_url: Option<String>,
    #[serde(rename = "duration_millis")]
    duration_millis: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
struct VxSize {
    width: Option<u32>,
    height: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct VxTranslation {
    text: Option<String>,
    #[serde(rename = "source_language")]
    source_language: Option<String>,
}

/// vxtwitter medya öğesini [`Media`]'ya çevirir.
///
/// Görsel: `url` doğrudan `pbs.twimg.com` adresidir. Video: `poster` küçük görsel varyantı,
/// süre milisaniyedir. GIF: vx de `tweet_video` mp4'ünü verir; kart bunu vxTwitter **gifconvert**
/// servisine türetilmiş animasyonlu AVIF bağlantısıyla göstermeye devam eder.
fn vx_media(item: &VxMedia) -> Option<Media> {
    let url = item.url.as_deref()?.to_string();
    let (width, height) = item
        .size
        .as_ref()
        .map_or((None, None), |s| (s.width, s.height));
    let duration = item.duration_millis.map(duration_secs);

    let kind = match item.kind.as_deref() {
        Some("video") => MediaKind::Video,
        Some("gif") => MediaKind::Gif,
        Some("image") | None => MediaKind::Image,
        Some(other) => {
            debug!("Bilinmeyen vx medya türü atlandı ({other}): {url}");
            return None;
        }
    };

    Some(Media {
        kind,
        url,
        poster: item.thumbnail_url.clone(),
        width,
        height,
        duration,
    })
}

/// Dil ekli vxtwitter yanıtındaki X çevirisini gönderiye uygular.
///
/// Eski FX akışındaki [Grok kuralı] burada ücretsiz sağlanır: vx'in `translation` alanı
/// yapısal olarak yalnızca X'in `grok_translated_post_with_availability` verisiyle dolduğu
/// için sağlayıcı kontrolüne gerek kalmaz. Varsayılan akışta yalnızca gönderi
/// Türkçe/İngilizce değilse uygulanır; `translate_english` ile İngilizce de uygulanır.
fn apply_vx_translation(
    post: &mut BuiltinPost,
    tweet: &VxTweet,
    translation_target: &str,
    translate_english: bool,
) {
    if !needs_translation(
        post.language.as_deref(),
        translation_target,
        translate_english,
    ) {
        return;
    }

    let Some(translation) = tweet.translation.as_ref() else {
        return;
    };

    let Some(text) = translation
        .text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    else {
        return;
    };

    post.text = strip_trailing_media_links(&super::unescape_html(text));
    post.translation = Some(Translation {
        source: translation
            .source_language
            .clone()
            .or_else(|| post.language.clone()),
    });
}

/// X'in syndication uç noktası token'ı doğrulamıyor; istemci tarafında üretilen
/// deterministik token yeterli oluyor (referans JS: `((id / 1e15) * Math.PI).toString(36)`).
fn syndication_token(id: &str) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";

    let mut value = (id.parse::<f64>().unwrap_or(0.0) / 1e15) * std::f64::consts::PI;
    let integral = value.trunc();
    let mut whole = integral as u64;
    let mut stack: Vec<u8> = Vec::new();
    if whole == 0 {
        stack.push(b'0');
    }
    while whole > 0 {
        stack.push(DIGITS[(whole % 36) as usize]);
        whole /= 36;
    }

    let mut out = String::new();
    for digit in stack.iter().rev() {
        out.push(*digit as char);
    }

    value -= integral;
    if value > 0.0 {
        out.push('.');
        for _ in 0..18 {
            value *= 36.0;
            let digit = value.trunc() as usize;
            out.push(DIGITS[digit % 36] as char);
            value -= value.trunc();
            if value <= 0.0 {
                break;
            }
        }
    }

    // JS referansındaki `.replace(/(0+|\.)/g, '')` ile aynı davranış
    out.replace(['.', '0'], "")
}

// =====================================================================================
// SYNDICATION
// =====================================================================================

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SyndicationTweet {
    id_str: Option<String>,
    text: Option<String>,
    created_at: Option<String>,
    favorite_count: Option<u64>,
    conversation_count: Option<u64>,
    possibly_sensitive: Option<bool>,
    display_text_range: Option<Vec<usize>>,
    lang: Option<String>,
    user: Option<SyndicationUser>,
    entities: Option<SyndicationEntities>,
    photos: Option<Vec<SyndicationPhoto>>,
    video: Option<SyndicationVideo>,
    #[serde(rename = "mediaDetails")]
    media_details: Option<Vec<SyndicationMediaDetail>>,
    quoted_tweet: Option<Box<SyndicationTweet>>,
    note_tweet: Option<SyndicationNoteTweet>,
}

impl SyndicationTweet {
    /// Silinmiş/korunmuş gönderilerde tüm alanlar boş gelir.
    fn is_empty(&self) -> bool {
        self.text.is_none()
            && self.user.is_none()
            && self.media_details.is_none()
            && self.photos.is_none()
            && self.video.is_none()
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SyndicationUser {
    name: Option<String>,
    screen_name: Option<String>,
    profile_image_url_https: Option<String>,
    is_blue_verified: Option<bool>,
    verified: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SyndicationEntities {
    urls: Option<Vec<SyndicationUrlEntity>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SyndicationUrlEntity {
    url: Option<String>,
    expanded_url: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SyndicationPhoto {
    url: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SyndicationVideo {
    poster: Option<String>,
    variants: Option<Vec<SyndicationVariant>>,
    #[serde(rename = "durationMs")]
    duration_ms: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SyndicationVariant {
    src: Option<String>,
    url: Option<String>,
    content_type: Option<String>,
    bitrate: Option<u64>,
}

impl SyndicationVariant {
    fn location(&self) -> Option<&str> {
        self.src.as_deref().or(self.url.as_deref())
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SyndicationMediaDetail {
    #[serde(rename = "type")]
    kind: Option<String>,
    media_url_https: Option<String>,
    original_info: Option<SyndicationOriginalInfo>,
    video_info: Option<SyndicationVideoInfo>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SyndicationOriginalInfo {
    width: Option<u32>,
    height: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SyndicationVideoInfo {
    variants: Option<Vec<SyndicationVariant>>,
    duration_millis: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SyndicationNoteTweet {
    text: Option<String>,
}

/// Bir mp4 adayı (ağırlık hesabı için).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VideoCandidate<'a> {
    url: &'a str,
    /// Bit hızı (bit/sn); kaynak vermezse 0.
    bitrate: u64,
    /// Dosya boyutu: kaynak tam boyut veriyorsa o, yoksa bit hızı × süre tahmini.
    bytes: Option<u64>,
}

/// Bit hızı (bit/sn) ve süreden dosya boyutu tahmini (bayt).
fn estimated_bytes(bitrate: Option<u64>, duration_ms: Option<u64>) -> Option<u64> {
    let bitrate = bitrate.filter(|bitrate| *bitrate > 0)?;
    let duration_ms = duration_ms.filter(|ms| *ms > 0)?;
    Some(bitrate.saturating_mul(duration_ms) / 8_000)
}

/// Adaylar arasından **en uygun** mp4'ü seçer (HLS/m3u8 oynatılamaz, adaylara girmez).
///
/// **Ağırlık: kalite ÷ boyut.** `max_bytes` (Discord limiti × ayar) altında kalan adaylar
/// arasından en yüksek bit hızlı olan seçilir: kart, oynatıcıyı kilitleyen dev varyant
/// yerine **anında oynayan** en iyi kaliteyi kullanır (X'in kendisi de 1080p yerine
/// izleyicinin hızına göre daha küçük varyant verir). Hiçbiri bütçeye sığmıyorsa **en küçük**
/// dosya seçilir; böylece indirme hızlı tamamlanır ve video kartta oynar.
///
/// `prefer_tweet_video`: X'in GIF kaynakları (`video.twimg.com/tweet_video/…`) varsa yalnızca
/// onlar arasından seçilir; FxEmbed GIF servisinin adresi bu varyanttan türetilir.
fn pick_video(
    candidates: Vec<VideoCandidate<'_>>,
    max_bytes: u64,
    prefer_tweet_video: bool,
) -> Option<String> {
    if candidates.is_empty() {
        return None;
    }

    let pool: Vec<VideoCandidate> =
        if prefer_tweet_video && candidates.iter().any(|c| c.url.contains("tweet_video")) {
            candidates
                .into_iter()
                .filter(|c| c.url.contains("tweet_video"))
                .collect()
        } else {
            candidates
        };

    // 1) Bütçeye sığanlar arasından en yüksek bit hızlı aday (boyut bilinmiyorsa sığar sayılır).
    if let Some(best) = pool
        .iter()
        .filter(|c| c.bytes.map(|bytes| bytes <= max_bytes).unwrap_or(true))
        .max_by_key(|c| (c.bitrate, c.bytes.unwrap_or(0)))
    {
        return Some(best.url.to_string());
    }

    // 2) Hiçbiri bütçeye sığmıyorsa en küçük dosya (en hızlı indirme / en hızlı oynatma).
    pool.iter()
        .min_by_key(|c| c.bytes.unwrap_or(u64::MAX))
        .map(|c| c.url.to_string())
}

/// X syndication varyant listesini [`pick_video`] ağırlığına çevirir (HLS oynatılamaz).
fn best_mp4(
    variants: Option<&[SyndicationVariant]>,
    duration_ms: Option<u64>,
    max_bytes: u64,
    prefer_tweet_video: bool,
) -> Option<String> {
    let candidates: Vec<VideoCandidate> = variants?
        .iter()
        .filter(|variant| {
            variant
                .content_type
                .as_deref()
                .unwrap_or("video/mp4")
                .contains("mp4")
        })
        .filter_map(|variant| {
            Some(VideoCandidate {
                url: variant.location()?,
                bitrate: variant.bitrate.unwrap_or(0),
                bytes: estimated_bytes(variant.bitrate, duration_ms),
            })
        })
        .collect();

    pick_video(candidates, max_bytes, prefer_tweet_video)
}

async fn from_syndication(
    client: &reqwest::Client,
    original_url: &str,
    id: &str,
    max_video_bytes: u64,
) -> Result<BuiltinPost, String> {
    let url = format!(
        "{SYNDICATION_URL}?id={id}&token={}&lang=en",
        syndication_token(id)
    );
    let tweet: SyndicationTweet = super::get_json_as(client, &url).await?;

    if tweet.is_empty() {
        return Err(format!(
            "Gönderi bulunamadı (silinmiş veya korumalı olabilir): {id}"
        ));
    }

    syndication_to_post(&tweet, original_url, max_video_bytes)
}

fn syndication_to_post(
    tweet: &SyndicationTweet,
    url: &str,
    max_video_bytes: u64,
) -> Result<BuiltinPost, String> {
    let user = tweet.user.as_ref();
    let author = x_author(
        user.and_then(|u| u.name.as_deref()),
        user.and_then(|u| u.screen_name.as_deref())
            .unwrap_or_default(),
        user.and_then(|u| u.profile_image_url_https.clone()),
        user.and_then(|u| u.is_blue_verified.or(u.verified))
            .unwrap_or(false),
    );

    let mut post = BuiltinPost::new(Platform::Twitter, url, author);

    let raw_text = tweet
        .note_tweet
        .as_ref()
        .and_then(|n| n.text.clone())
        .or_else(|| tweet.text.clone())
        .unwrap_or_default();
    // `display_text_range` X'in verdiği **kaçışlı** metnin UTF-16 birimlerini sayar (`&gt;` dört,
    // 🥰 iki birim): önce kesilir, sonra kaçışlar çözülür. Kaçışsız metinde kod noktası saymak
    // her emoji/kaçış için aralığı kaydırır ve sona kısa linkin başı (` h`, ` https:`) taşar.
    let raw_text = if tweet.note_tweet.is_some() {
        raw_text
    } else {
        cut_to_display_range(&raw_text, tweet.display_text_range.as_deref())
    };
    let raw_text = super::unescape_html(&raw_text);
    post.text =
        strip_trailing_media_links(&expand_url_entities(&raw_text, tweet.entities.as_ref()));
    post.language = tweet.lang.clone().filter(|lang| !lang.trim().is_empty());

    post.created_at = tweet.created_at.as_deref().and_then(parse_timestamp);
    post.stats = Stats {
        replies: tweet.conversation_count.unwrap_or(0),
        reposts: 0,
        likes: tweet.favorite_count.unwrap_or(0),
        quotes: 0,
        views: None,
    };
    post.sensitive = tweet.possibly_sensitive.unwrap_or(false);
    post.media = syndication_media(tweet, max_video_bytes);

    if let Some(quoted) = tweet.quoted_tweet.as_deref()
        && let Some(quoted_url) = quoted.id_str.as_deref().and_then(|quoted_id| {
            status_url(
                quoted.user.as_ref().and_then(|u| u.screen_name.as_deref()),
                quoted_id,
            )
        })
        && let Ok(quoted_post) = syndication_to_post(quoted, &quoted_url, max_video_bytes)
    {
        post.quote = Some(Box::new(quoted_post));
    }

    Ok(post)
}

/// Gönderi metnini `display_text_range` sınırına kadar keser (metnin sonundaki medya
/// kısa linki bu aralığın dışında kalır). Sınırlar UTF-16 birimidir; bir vekil çiftin
/// ortasına düşen sınır karakteri bölmez, karakter dahil edilir.
fn cut_to_display_range(text: &str, range: Option<&[usize]>) -> String {
    let Some(&[start, end, ..]) = range else {
        return text.to_string();
    };
    let mut offset = 0;
    text.chars()
        .filter(|ch| {
            let units = ch.len_utf16();
            let keep = offset + units > start && offset < end;
            offset += units;
            keep
        })
        .collect()
}

/// `entities.urls` içindeki `t.co` kısa linklerini gerçek adresleriyle değiştirir.
///
/// Metin tek geçişte taranır ve her kısa link **tam** eşleşmeyle aranır: art arda
/// `replace` çağrıları hem metni her bağlantı için yeniden kopyalıyor hem de bir kısa
/// link başka birinin önekiyse (`t.co/abc` ↔ `t.co/abcd`) uzun olanı bozuyordu.
fn expand_url_entities(text: &str, entities: Option<&SyndicationEntities>) -> String {
    static SHORT_LINK: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"https?://t\.co/[A-Za-z0-9]+").expect("geçerli t.co regex"));
    let Some(urls) = entities.and_then(|e| e.urls.as_deref()) else {
        return text.to_string();
    };
    // Şema farkı yok sayılır: X metinde `https://`, varlıkta bazen `http://` verir.
    let path = |url: &str| {
        url.split_once("://")
            .map_or(url, |(_, rest)| rest)
            .to_owned()
    };
    let expansions: std::collections::HashMap<String, &str> = urls
        .iter()
        .filter_map(|entity| {
            Some((
                path(entity.url.as_deref()?),
                entity.expanded_url.as_deref()?,
            ))
        })
        .collect();
    if expansions.is_empty() {
        return text.to_string();
    }
    SHORT_LINK
        .replace_all(text, |caps: &regex::Captures<'_>| {
            expansions
                .get(&path(&caps[0]))
                .map_or_else(|| caps[0].to_string(), |expanded| (*expanded).to_string())
        })
        .into_owned()
}

/// `mediaDetails` (varsa) veya `photos` + `video` alanlarından medya listesi üretir.
fn syndication_media(tweet: &SyndicationTweet, max_video_bytes: u64) -> Vec<Media> {
    if let Some(details) = tweet.media_details.as_deref()
        && !details.is_empty()
    {
        return details
            .iter()
            .filter_map(|detail| media_from_detail(detail, max_video_bytes))
            .collect();
    }

    let mut media = Vec::new();
    if let Some(photos) = tweet.photos.as_deref() {
        for photo in photos {
            if let Some(url) = photo.url.as_deref() {
                media.push(Media {
                    kind: MediaKind::Image,
                    url: card_image_url(url),
                    poster: None,
                    width: photo.width,
                    height: photo.height,
                    duration: None,
                });
            }
        }
    }
    if let Some(video) = tweet.video.as_ref()
        && let Some(url) = best_mp4(
            video.variants.as_deref(),
            video.duration_ms,
            max_video_bytes,
            true,
        )
    {
        media.push(Media {
            kind: MediaKind::Video,
            url,
            poster: video.poster.as_deref().map(card_image_url),
            width: None,
            height: None,
            duration: video.duration_ms.map(duration_secs),
        });
    }
    media
}

fn media_from_detail(detail: &SyndicationMediaDetail, max_video_bytes: u64) -> Option<Media> {
    let kind = detail.kind.as_deref()?;
    let info = detail.original_info.as_ref();
    let width = info.and_then(|i| i.width);
    let height = info.and_then(|i| i.height);

    match kind {
        "photo" => Some(Media {
            kind: MediaKind::Image,
            url: card_image_url(detail.media_url_https.as_deref()?),
            poster: None,
            width,
            height,
            duration: None,
        }),
        "video" | "animated_gif" => {
            let video_info = detail.video_info.as_ref()?;
            let url = best_mp4(
                video_info.variants.as_deref(),
                video_info.duration_millis,
                max_video_bytes,
                // GIF kaynağı `tweet_video` varyantından türetilir: o varsa tercih edilir.
                true,
            )?;
            Some(Media {
                kind: if kind == "animated_gif" {
                    MediaKind::Gif
                } else {
                    MediaKind::Video
                },
                url,
                poster: detail.media_url_https.as_deref().map(card_image_url),
                width,
                height,
                duration: video_info.duration_millis.map(duration_secs),
            })
        }
        _ => None,
    }
}

// =====================================================================================
// FXTWITTER (api.fxtwitter.com)
// =====================================================================================

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FxResponse {
    code: Option<u16>,
    message: Option<String>,
    tweet: Option<FxTweet>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FxTweet {
    id: Option<String>,
    url: Option<String>,
    text: Option<String>,
    lang: Option<String>,
    created_at: Option<String>,
    created_timestamp: Option<i64>,
    author: Option<FxAuthor>,
    likes: Option<u64>,
    retweets: Option<u64>,
    replies: Option<u64>,
    quotes: Option<u64>,
    views: Option<u64>,
    possibly_sensitive: Option<bool>,
    community_note: Option<FxCommunityNote>,
    media: Option<FxMedia>,
    quote: Option<Box<FxTweet>>,
    translation: Option<FxTranslation>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FxAuthor {
    name: Option<String>,
    screen_name: Option<String>,
    avatar_url: Option<String>,
    verification: Option<FxVerification>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FxVerification {
    verified: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FxCommunityNote {
    text: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FxMedia {
    all: Option<Vec<FxMediaItem>>,
    photos: Option<Vec<FxMediaItem>>,
    videos: Option<Vec<FxMediaItem>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FxMediaItem {
    #[serde(rename = "type")]
    kind: Option<String>,
    url: Option<String>,
    thumbnail_url: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    duration: Option<f64>,
    /// Video varyantları: bit hızı, boyut ve kodek bilgisi buradan gelir (ağırlıklı seçim).
    formats: Option<Vec<FxFormat>>,
}

/// FxTwitter'ın video varyantı (`formats[]`): mp4/m3u8 + bit hızı + (varsa) tam boyut.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FxFormat {
    url: Option<String>,
    container: Option<String>,
    bitrate: Option<u64>,
    size: Option<u64>,
    width: Option<u32>,
    height: Option<u32>,
}

/// FxTwitter'ın dil ekli ucundan gelen X çevirisi (`provider` X'in çeviri servisidir).
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FxTranslation {
    text: Option<String>,
    source_lang: Option<String>,
    target_lang: Option<String>,
    /// Çeviriyi üreten sağlayıcı: X'in kendi çevirisi `grok`, FxEmbed'in yedek çevirisi
    /// `deepl` gibi başka bir değer olur.
    provider: Option<String>,
}

async fn from_fxtwitter(
    client: &reqwest::Client,
    original_url: &str,
    id: &str,
    language: Option<&str>,
    translation_target: &str,
    max_video_bytes: u64,
    translate_english: bool,
) -> Result<BuiltinPost, String> {
    // Dil eki (`/tr`) verildiğinde yanıt orijinal metinle birlikte X'in çevirisini de içerir.
    let language_suffix = language.map(|lang| format!("/{lang}")).unwrap_or_default();
    let url = format!("{FXTWITTER_URL}/i/status/{id}{language_suffix}");
    let response: FxResponse = super::get_json_as(client, &url).await?;

    if let Some(code) = response.code
        && code != 200
    {
        return Err(format!(
            "fxtwitter gönderiyi döndürmedi ({id}, kod {code}): {}",
            response.message.unwrap_or_default()
        ));
    }

    let tweet = response
        .tweet
        .ok_or_else(|| format!("fxtwitter yanıtında gönderi yok: {id}"))?;

    fx_to_post_with_target(
        &tweet,
        original_url,
        max_video_bytes,
        translation_target,
        translate_english,
    )
}

fn fx_to_post_with_target(
    tweet: &FxTweet,
    fallback_url: &str,
    max_video_bytes: u64,
    translation_target: &str,
    translate_english: bool,
) -> Result<BuiltinPost, String> {
    let user = tweet.author.as_ref();
    let author = x_author(
        user.and_then(|a| a.name.as_deref()),
        user.and_then(|a| a.screen_name.as_deref())
            .unwrap_or_default(),
        user.and_then(|a| a.avatar_url.clone()),
        user.and_then(|a| a.verification.as_ref())
            .and_then(|v| v.verified)
            .unwrap_or(false),
    );
    let url = canonical_x_url(tweet.url.as_deref().unwrap_or(fallback_url));

    let mut post = BuiltinPost::new(Platform::Twitter, url, author);
    post.text = strip_trailing_media_links(&tweet.text.clone().unwrap_or_default());
    post.language = tweet.lang.clone().filter(|lang| !lang.trim().is_empty());
    post.created_at = tweet
        .created_timestamp
        .or_else(|| tweet.created_at.as_deref().and_then(parse_timestamp));
    post.stats = Stats {
        replies: tweet.replies.unwrap_or(0),
        reposts: tweet.retweets.unwrap_or(0),
        likes: tweet.likes.unwrap_or(0),
        quotes: tweet.quotes.unwrap_or(0),
        views: tweet.views,
    };
    post.sensitive = tweet.possibly_sensitive.unwrap_or(false);
    post.community_note = tweet
        .community_note
        .as_ref()
        .and_then(|note| note.text.clone())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty());
    post.media = fx_media(tweet.media.as_ref(), max_video_bytes);

    if let Some(quoted) = tweet.quote.as_deref()
        && let Some(quoted_url) = quoted
            .url
            .as_deref()
            .filter(|u| !u.trim().is_empty())
            .map(canonical_x_url)
            .or_else(|| {
                status_url(
                    quoted
                        .author
                        .as_ref()
                        .and_then(|a| a.screen_name.as_deref()),
                    quoted.id.as_deref()?,
                )
            })
        && let Ok(quoted_post) = fx_to_post_with_target(
            quoted,
            &quoted_url,
            max_video_bytes,
            translation_target,
            translate_english,
        )
    {
        post.quote = Some(Box::new(quoted_post));
    }

    // Alıntılanan gönderi de kendi `fx_to_post` çağrısında çevrilir.
    apply_fx_translation(&mut post, tweet, translation_target, translate_english);

    Ok(post)
}

/// Dil ekli FxTwitter yanıtındaki X çevirisini gönderiye uygular.
///
/// Varsayılan akışta yalnızca gönderi Türkçe/İngilizce değilse uygulanır (`translate_english`
/// ile İngilizce de) **ve çeviri Grok'tan gelmelidir** (X'in kendi çeviri servisi). FxEmbed,
/// X çevirisi yoksa kendi yedek servisine (ör. DeepL) düşer; o çeviri **kullanılmaz** —
/// gönderi orijinal metniyle ve çeviri notu olmadan
/// gönderilir (bayrak reaksiyonuyla istenirse proxy linki üzerinden çevrilebilir).
fn apply_fx_translation(
    post: &mut BuiltinPost,
    tweet: &FxTweet,
    translation_target: &str,
    translate_english: bool,
) {
    if !needs_translation(
        post.language.as_deref(),
        translation_target,
        translate_english,
    ) {
        return;
    }

    let Some(translation) = tweet.translation.as_ref() else {
        return;
    };

    if !is_grok_translation(translation.provider.as_deref()) {
        debug!(
            "Çeviri Grok'tan gelmedi (sağlayıcı: {:?}), kullanılmıyor: {}",
            translation.provider.as_deref().unwrap_or("bilinmiyor"),
            post.url
        );
        return;
    }

    let Some(text) = translation
        .text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    else {
        return;
    };

    post.text = strip_trailing_media_links(text);
    post.translation = Some(Translation {
        source: translation
            .source_lang
            .clone()
            .or_else(|| post.language.clone()),
    });
}

/// Çeviri **Grok** (X'in kendi çeviri servisi) tarafından mı üretildi?
///
/// Sağlayıcı bilinmiyorsa da kabul edilmez: "her zaman Grok" kuralı gereği yalnızca
/// sağlayıcısı Grok olan çeviri karta yazılır (ör. `deepl`, `google` yok sayılır).
fn is_grok_translation(provider: Option<&str>) -> bool {
    provider
        .map(|provider| {
            let provider = provider.trim().to_ascii_lowercase();
            provider.contains("grok") || provider == "xai" || provider == "x-ai"
        })
        .unwrap_or(false)
}

/// `media.all` (sıralı) varsa onu, yoksa `photos` + `videos` listelerini kullanır.
fn fx_media(media: Option<&FxMedia>, video_budget: u64) -> Vec<Media> {
    let Some(media) = media else {
        return Vec::new();
    };

    let mut items: Vec<&FxMediaItem> = media.all.as_deref().unwrap_or_default().iter().collect();
    if items.is_empty() {
        items.extend(media.photos.as_deref().unwrap_or_default());
        items.extend(media.videos.as_deref().unwrap_or_default());
    }

    items
        .iter()
        .filter_map(|item| fx_media_item(item, video_budget))
        .collect()
}

/// FxTwitter'ın `formats` listesinden **anında oynayan** en uygun mp4'ü seçer.
///
/// `None` dönerse gönderinin kendi `url` alanı kullanılır (tek varyant verilmiş demektir).
fn pick_fx_video(item: &FxMediaItem, max_bytes: u64) -> Option<String> {
    let duration_ms = fx_duration_ms(item);

    let candidates: Vec<VideoCandidate> = item
        .formats
        .as_deref()?
        .iter()
        .filter(|format| format.container.as_deref() == Some("mp4"))
        .filter_map(|format| {
            Some(VideoCandidate {
                url: format.url.as_deref()?,
                bitrate: format.bitrate.unwrap_or(0),
                // Kaynak tam boyut veriyorsa o, yoksa bit hızı × süre tahmini kullanılır.
                bytes: format
                    .size
                    .or_else(|| estimated_bytes(format.bitrate, duration_ms)),
            })
        })
        .collect();

    pick_video(candidates, max_bytes, false)
}

fn fx_media_item(item: &FxMediaItem, video_budget: u64) -> Option<Media> {
    let url = item.url.as_deref()?;
    let kind = match item.kind.as_deref()? {
        "photo" => MediaKind::Image,
        "gif" => MediaKind::Gif,
        "video" => MediaKind::Video,
        _ => return None,
    };

    Some(Media {
        kind,
        url: match kind {
            // Görsel kartta CDN'den geldiği için vx/fxTwitter gibi tam çözünürlüklü varyantı (name=orig) kullanılır.
            MediaKind::Image => card_image_url(url),
            // Video: `formats` varsa ağırlıklı seçim (1080p dev varyant yerine hızlı oynayan).
            MediaKind::Video => {
                pick_fx_video(item, video_budget).unwrap_or_else(|| url.to_string())
            }
            // GIF kaynağı `tweet_video` mp4'ü olarak kalmalı: FxEmbed GIF adresi ondan türetilir.
            MediaKind::Gif => url.to_string(),
        },
        poster: item.thumbnail_url.clone(),
        width: item.width,
        height: item.height,
        duration: fx_duration_ms(item).map(duration_secs),
    })
}

/// FxTwitter süreyi saniye (ondalıklı) verir; diğer kaynaklarla aynı birime çevrilir.
fn fx_duration_ms(item: &FxMediaItem) -> Option<u64> {
    item.duration
        .filter(|seconds| seconds.is_finite())
        .map(|seconds| (seconds.max(0.0) * 1000.0).round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FxTwitter'ın dil ekli (`/tr`) ucundan alınan yanıtın alan yapısı (canlı yanıttan).
    fn translated_tweet(lang: &str) -> serde_json::Value {
        serde_json::json!({
            "url": "https://x.com/kullanici/status/1",
            "text": "Guten Morgen zusammen",
            "lang": lang,
            "created_timestamp": 1_714_998_896,
            "author": { "name": "Kullanıcı", "screen_name": "kullanici" },
            "translation": {
                "text": "Herkese günaydın",
                "source_lang": "de",
                "target_lang": "tr",
                "source_lang_en": "German",
                "provider": "grok"
            }
        })
    }

    fn post_from(json: &serde_json::Value) -> BuiltinPost {
        let tweet: FxTweet =
            serde_json::from_value(json.clone()).expect("örnek JSON geçerli olmalı");
        fx_to_post_with_target(
            &tweet,
            "https://x.com/kullanici/status/1",
            8 * 1024 * 1024,
            "tr",
            false,
        )
        .expect("gönderi üretilmeli")
    }

    /// vxtwitter API yanıtının alan yapısı (canlı yanıttan; görsel içerikli gönderi).
    fn vx_tweet_json(lang: &str) -> serde_json::Value {
        serde_json::json!({
            "text": "Guten Morgen zusammen",
            "tweetID": "1112223334445556666",
            "tweetURL": "https://twitter.com/kullanici/status/1112223334445556666",
            "user_name": "Kullanıcı",
            "user_screen_name": "kullanici",
            "user_profile_image_url": "https://pbs.twimg.com/profile_images/1/a_normal.jpg",
            "date_epoch": 1_714_998_896,
            "likes": 12,
            "retweets": 3,
            "replies": 4,
            "lang": lang,
            "possibly_sensitive": false,
            "media_extended": [{
                "url": "https://pbs.twimg.com/media/AaBbCcD.jpg",
                "type": "image",
                "size": { "width": 1200, "height": 800 },
                "thumbnail_url": "https://pbs.twimg.com/media/AaBbCcD.jpg",
                "altText": null,
                "id_str": "100"
            }],
            "communityNote": null,
            "translation": null,
            "qrt": null,
            "retweet": null
        })
    }

    fn vx_post_from(json: &serde_json::Value) -> BuiltinPost {
        let tweet: VxTweet =
            serde_json::from_value(json.clone()).expect("örnek vx JSON geçerli olmalı");
        vx_to_post(
            &tweet,
            "https://x.com/kullanici/status/1112223334445556666",
            "tr",
            false,
        )
        .expect("vxtwitter gönderisi üretilmeli")
    }

    #[test]
    fn vx_parses_text_author_stats_and_media() {
        let post = vx_post_from(&vx_tweet_json("en"));

        assert_eq!(post.author.handle, "kullanici");
        assert_eq!(post.author.name, "Kullanıcı");
        assert_eq!(
            post.author.profile_url.as_deref(),
            Some("https://x.com/kullanici")
        );
        assert_eq!(
            post.author.avatar.as_deref(),
            Some("https://pbs.twimg.com/profile_images/1/a_normal.jpg")
        );
        assert_eq!(post.created_at, Some(1_714_998_896));
        assert_eq!(post.stats.likes, 12);
        assert_eq!(post.stats.reposts, 3);
        assert_eq!(post.stats.replies, 4);
        assert_eq!(post.media.len(), 1);
        assert_eq!(post.media[0].kind, MediaKind::Image);
        assert_eq!(post.media[0].width, Some(1200));
        assert_eq!(post.media[0].height, Some(800));
    }

    #[test]
    fn vx_video_and_gif_media_map_to_the_right_kinds() {
        let mut json = vx_tweet_json("en");
        json["media_extended"] = serde_json::json!([
            {
                "url": "https://video.twimg.com/ext_tw_video/1/pu/vid/762x528/x.mp4",
                "type": "video",
                "size": { "width": 762, "height": 528 },
                "duration_millis": 13_650,
                "thumbnail_url": "https://pbs.twimg.com/ext_tw_video_thumb/1/pu/img/x.jpg",
                "altText": null,
                "id_str": "101"
            },
            {
                "url": "https://video.twimg.com/tweet_video/AaBbCcD.mp4",
                "type": "gif",
                "size": { "width": 500, "height": 500 },
                "duration_millis": 0,
                "thumbnail_url": "https://pbs.twimg.com/tweet_video_thumb/AaBbCcD.jpg",
                "altText": null,
                "id_str": "102"
            },
            { "type": "unknown_kind", "url": "https://pbs.twimg.com/media/z.mp4" }
        ]);

        let post = vx_post_from(&json);
        assert_eq!(post.media.len(), 2, "bilinmeyen tür atlanır");

        let video = &post.media[0];
        assert_eq!(video.kind, MediaKind::Video);
        assert_eq!(
            video.duration,
            Some(14),
            "13_650 ms → 14 sn (yukarı yuvarlar)"
        );
        assert_eq!(
            video.poster.as_deref(),
            Some("https://pbs.twimg.com/ext_tw_video_thumb/1/pu/img/x.jpg")
        );

        let gif = &post.media[1];
        assert_eq!(gif.kind, MediaKind::Gif);
        assert_eq!(gif.url, "https://video.twimg.com/tweet_video/AaBbCcD.mp4");
        // gifconvert türetimi (`media::gif_convert_url`) tweet_video yolunu kabul eder.
        assert!(crate::converters::builtin::media::gif_convert_url(&gif.url).is_some());
    }

    #[test]
    fn vx_translation_applies_only_for_foreign_languages() {
        // Almanca → hedef Türkçe: X'in Grok çevirisi uygulanır ve metin değişir.
        let mut german = vx_tweet_json("de");
        german["translation"] = serde_json::json!({
            "text": "Herkese günaydın",
            "source_language": "de"
        });
        let post = vx_post_from(&german);
        assert_eq!(post.text, "Herkese günaydın");
        assert_eq!(
            post.translation,
            Some(Translation {
                source: Some("de".into())
            })
        );

        // Türkçe gönderi: çeviri alanı dolu olsa bile uygulanmaz.
        let mut turkish = vx_tweet_json("tr");
        turkish["translation"] = german["translation"].clone();
        let post = vx_post_from(&turkish);
        assert_eq!(post.text, "Guten Morgen zusammen");
        assert!(post.translation.is_none());

        // Çeviri metni boşluk ise yok sayılır.
        let mut blank = vx_tweet_json("de");
        blank["translation"]["text"] = serde_json::json!("   ");
        let post = vx_post_from(&blank);
        assert_eq!(post.text, "Guten Morgen zusammen");
        assert!(post.translation.is_none());
    }

    #[test]
    fn vx_community_note_is_kept_but_blank_notes_dropped() {
        let mut json = vx_tweet_json("en");
        json["communityNote"] = serde_json::json!("Bu gönderi bağlamı eksik: ...");
        let post = vx_post_from(&json);
        assert_eq!(
            post.community_note.as_deref(),
            Some("Bu gönderi bağlamı eksik: ...")
        );

        let mut blank = vx_tweet_json("en");
        blank["communityNote"] = serde_json::json!("   ");
        assert!(vx_post_from(&blank).community_note.is_none());
    }

    #[test]
    fn vx_quote_is_embedded_one_level_deep() {
        let mut json = vx_tweet_json("en");
        json["qrt"] = serde_json::json!({
            "tweetID": "999888777666555444",
            "text": "iç alıntı metni",
            "user_name": "Alinti",
            "user_screen_name": "alinti_user",
            "media_extended": [],
        });

        let post = vx_post_from(&json);
        let quote = post.quote.as_ref().expect("alıntı gömülmeli");
        assert_eq!(quote.author.handle, "alinti_user");
        assert_eq!(quote.text, "iç alıntı metni");
        assert_eq!(
            quote.url,
            "https://x.com/alinti_user/status/999888777666555444"
        );
    }

    #[test]
    fn vx_retweet_unwraps_the_inner_content() {
        let mut json = vx_tweet_json("en");
        json["text"] = serde_json::json!("");
        json["retweet"] = serde_json::json!({
            "tweetID": "888999111222333444",
            "text": "asıl içerik",
            "user_name": "Asıl",
            "user_screen_name": "asil_user",
            "media_extended": [],
        });

        let post = vx_post_from(&json);
        assert_eq!(post.author.handle, "asil_user");
        assert_eq!(post.text, "asıl içerik");
        assert!(post.quote.is_none());
    }

    #[test]
    fn fx_video_uses_the_variant_that_plays_instantly() {
        let mut json = translated_tweet("en");
        json["media"] = serde_json::json!({
            "all": [{
                "type": "video",
                "url": "https://video.twimg.com/amplify_video/1/vid/1920x1080/hi.mp4",
                "duration": 61.494,
                "width": 1920,
                "height": 1080,
                "formats": [
                    { "url": "https://video.twimg.com/amplify_video/1/pl/x.m3u8", "container": "m3u8" },
                    { "url": "https://video.twimg.com/amplify_video/1/vid/480x270/lo.mp4", "container": "mp4", "bitrate": 288000 },
                    { "url": "https://video.twimg.com/amplify_video/1/vid/1920x1080/hi.mp4", "container": "mp4", "bitrate": 10368000 }
                ]
            }]
        });

        let post = post_from(&json);

        assert_eq!(post.media[0].kind, MediaKind::Video);
        // 8 MB bütçe: 480p (≈2,2 MB) seçilir; 1080p (≈79 MB) oynatıcıyı kilitlemez.
        assert_eq!(
            post.media[0].url,
            "https://video.twimg.com/amplify_video/1/vid/480x270/lo.mp4"
        );
    }

    #[test]
    fn fx_photos_use_orig_variant_in_the_card() {
        let mut json = translated_tweet("en");
        json["media"] = serde_json::json!({
            "all": [{
                "type": "photo",
                "url": "https://pbs.twimg.com/media/ABC.jpg?name=orig",
                "width": 1919,
                "height": 894
            }]
        });

        let post = post_from(&json);
        assert_eq!(
            post.media[0].url, "https://pbs.twimg.com/media/ABC.jpg?name=orig",
            "kart görseli vx/fxTwitter gibi tam kalitede (name=orig) tutulur"
        );
    }

    #[test]
    fn fx_gifs_keep_the_tweet_video_source_for_the_gif_service() {
        let mut json = translated_tweet("en");
        json["media"] = serde_json::json!({
            "all": [{
                "type": "gif",
                "url": "https://video.twimg.com/tweet_video/HTEtNFcXEAAY4Dv.mp4",
                "duration": 3.2,
                "formats": [{
                    "url": "https://video.twimg.com/tweet_video/HTEtNFcXEAAY4Dv.mp4",
                    "container": "mp4",
                    "bitrate": 800000
                }]
            }]
        });

        let post = post_from(&json);

        assert_eq!(post.media[0].kind, MediaKind::Gif);
        // FxEmbed GIF adresi bu mp4'ten türetilir (`gif.fxtwitter.com/tweet_video/...gif`).
        assert_eq!(
            post.media[0].url,
            "https://video.twimg.com/tweet_video/HTEtNFcXEAAY4Dv.mp4"
        );
    }

    #[test]
    fn fx_community_note_is_copied_to_the_builtin_post() {
        let mut json = translated_tweet("en");
        json["community_note"] = serde_json::json!({
            "text": "Readers added context"
        });

        let post = post_from(&json);

        assert_eq!(
            post.community_note.as_deref(),
            Some("Readers added context")
        );
    }

    #[test]
    fn fxtwitter_payload_supplies_the_missing_metrics() {
        // Zenginleştirme akışı bu alanlarla repost/görüntülenme/alıntıyı tamamlar.
        let mut json = translated_tweet("en");
        json["retweets"] = serde_json::json!(412);
        json["quotes"] = serde_json::json!(12);
        json["views"] = serde_json::json!(1_240_000);

        let post = post_from(&json);
        assert_eq!(post.stats.reposts, 412);
        assert_eq!(post.stats.quotes, 12);
        assert_eq!(post.stats.views, Some(1_240_000));
    }

    #[test]
    fn foreign_post_uses_x_translation() {
        let post = post_from(&translated_tweet("de"));

        assert_eq!(post.language.as_deref(), Some("de"));
        assert_eq!(
            post.text, "Herkese günaydın",
            "Grok (X) çevirisi metne yazılmalı"
        );
        assert_eq!(
            post.translation.as_ref().and_then(|t| t.source.as_deref()),
            Some("de")
        );
    }

    #[test]
    fn non_grok_translations_are_ignored() {
        // FxEmbed, X çevirisi yoksa kendi yedek servisine (ör. DeepL) düşer; o çeviri
        // kullanılmaz: gönderi orijinal metniyle ve çeviri notu olmadan kalır.
        for provider in ["deepl", "google", "microsoft"] {
            let mut json = translated_tweet("ja");
            json["translation"]["provider"] = serde_json::json!(provider);

            let post = post_from(&json);

            assert_eq!(
                post.text, "Guten Morgen zusammen",
                "orijinal metin korunmalı ({provider})"
            );
            assert!(
                post.translation.is_none(),
                "Grok dışı çeviri karta yazılmamalı ({provider})"
            );
        }
    }

    #[test]
    fn missing_provider_is_not_treated_as_grok() {
        let mut json = translated_tweet("de");
        json["translation"]
            .as_object_mut()
            .expect("çeviri nesnesi")
            .remove("provider");

        let post = post_from(&json);

        assert!(
            post.translation.is_none(),
            "sağlayıcı bilinmiyorsa Grok sayılmaz"
        );
        assert_eq!(post.text, "Guten Morgen zusammen");
    }

    #[test]
    fn only_grok_providers_are_accepted() {
        for provider in ["grok", "Grok", "xai", "x-ai", "grok-3"] {
            assert!(
                is_grok_translation(Some(provider)),
                "{provider} kabul edilmeli"
            );
        }
        for provider in ["deepl", "google", "microsoft", ""] {
            assert!(
                !is_grok_translation(Some(provider)),
                "{provider} reddedilmeli"
            );
        }
        assert!(!is_grok_translation(None), "sağlayıcı yoksa kabul edilmez");
    }

    #[test]
    fn turkish_and_english_posts_keep_original_text() {
        for lang in ["en", "tr", "en-GB"] {
            let post = post_from(&translated_tweet(lang));
            assert_eq!(
                post.text, "Guten Morgen zusammen",
                "orijinal metin korunmalı ({lang})"
            );
            assert!(
                post.translation.is_none(),
                "tr/en gönderiler çevrilmemeli ({lang})"
            );
        }
    }

    #[test]
    fn quoted_post_is_translated_separately() {
        let mut json = translated_tweet("ja");
        json["text"] = serde_json::json!("asıl gönderi");
        json["translation"] = serde_json::json!({
            "text": "asıl gönderinin çevirisi",
            "source_lang": "ja",
            "target_lang": "tr",
            "provider": "grok"
        });
        json["quote"] = serde_json::json!({
            "url": "https://x.com/diger/status/2",
            "text": "引用テキスト",
            "lang": "ja",
            "author": { "name": "Diğer", "screen_name": "diger" },
            "translation": {
                "text": "alıntı metni",
                "source_lang": "ja",
                "target_lang": "tr",
                // Yalnızca Grok (X'in kendi çevirisi) kabul edilir.
                "provider": "grok"
            }
        });

        let post = post_from(&json);
        assert_eq!(post.text, "asıl gönderinin çevirisi");

        let quote = post.quote.expect("alıntı eklenmeli");
        assert_eq!(quote.text, "alıntı metni");
        assert_eq!(
            quote.translation.as_ref().and_then(|t| t.source.as_deref()),
            Some("ja")
        );
    }

    #[test]
    fn trailing_media_link_is_removed_from_translation() {
        let mut json = translated_tweet("de");
        json["translation"]["text"] = serde_json::json!("Herkese günaydın https://t.co/abc123");

        let post = post_from(&json);
        assert_eq!(post.text, "Herkese günaydın");
    }

    #[test]
    fn missing_or_empty_translation_keeps_original() {
        let mut without_translation = translated_tweet("de");
        without_translation
            .as_object_mut()
            .expect("obje")
            .remove("translation");
        let post = post_from(&without_translation);
        assert_eq!(post.text, "Guten Morgen zusammen");
        assert!(post.translation.is_none());

        let mut empty_translation = translated_tweet("de");
        empty_translation["translation"]["text"] = serde_json::json!("   ");
        let post = post_from(&empty_translation);
        assert_eq!(post.text, "Guten Morgen zusammen");
        assert!(post.translation.is_none());
    }

    /// Ağırlık testleri için syndication video varyantı üretir.
    fn variant(url: &str, content_type: &str, bitrate: Option<u64>) -> SyndicationVariant {
        SyndicationVariant {
            src: Some(url.to_string()),
            url: None,
            content_type: Some(content_type.to_string()),
            bitrate,
        }
    }

    #[test]
    fn video_variant_weights_quality_against_estimated_size() {
        let variants = vec![
            variant(
                "https://video.twimg.com/amplify_video/1/vid/1920x1080/hi.mp4",
                "video/mp4",
                Some(5_000_000),
            ),
            variant(
                "https://video.twimg.com/amplify_video/1/vid/640x360/lo.mp4",
                "video/mp4",
                Some(600_000),
            ),
            // HLS/m3u8 oynatılamaz: bit hızı en yüksek olsa bile değerlendirilmez.
            variant(
                "https://video.twimg.com/amplify_video/1/pl/playlist.m3u8",
                "application/x-mpegURL",
                Some(9_000_000),
            ),
        ];

        // 8 MB bütçe + 12 sn video: 5 Mbps ≈ 7.5 MB sığar → en yüksek kalite indirilir.
        assert_eq!(
            best_mp4(Some(&variants), Some(12_000), 8 * 1024 * 1024, false).as_deref(),
            Some("https://video.twimg.com/amplify_video/1/vid/1920x1080/hi.mp4")
        );

        // Bütçe darsa hiçbir varyant sığmaz: **en küçük** dosya seçilir, kart beklemez.
        assert_eq!(
            best_mp4(Some(&variants), Some(12_000), 500 * 1024, false).as_deref(),
            Some("https://video.twimg.com/amplify_video/1/vid/640x360/lo.mp4")
        );
    }

    #[test]
    fn gif_command_picks_the_480p_variant() {
        let format = |url: &str, bitrate: u64| FxFormat {
            url: Some(url.to_string()),
            container: Some("mp4".to_string()),
            bitrate: Some(bitrate),
            ..FxFormat::default()
        };
        let tweet = |formats: Vec<FxFormat>| FxTweet {
            media: Some(FxMedia {
                videos: Some(vec![FxMediaItem {
                    formats: Some(formats),
                    ..FxMediaItem::default()
                }]),
                ..FxMedia::default()
            }),
            ..FxTweet::default()
        };

        // Dikey video: 480x852 tam 480p.
        let portrait = tweet(vec![
            format(
                "https://video.twimg.com/amplify_video/1/vid/avc1/320x568/a.mp4",
                1,
            ),
            format(
                "https://video.twimg.com/amplify_video/1/vid/avc1/480x852/b.mp4",
                2,
            ),
            format(
                "https://video.twimg.com/amplify_video/1/vid/avc1/720x1280/c.mp4",
                3,
            ),
        ]);
        assert_eq!(
            fx_gif_links(&portrait),
            ["https://gif.fxtwitter.com/amplify_video/1/vid/avc1/480x852/b.gif"]
        );

        // Yatay videoda 480p yoksa altındaki en yakın (360p) seçilir.
        let landscape = tweet(vec![
            format(
                "https://video.twimg.com/ext_tw_video/1/pu/vid/480x270/a.mp4?tag=14",
                1,
            ),
            format(
                "https://video.twimg.com/ext_tw_video/1/pu/vid/640x360/b.mp4?tag=14",
                2,
            ),
            format(
                "https://video.twimg.com/ext_tw_video/1/pu/vid/1280x720/c.mp4?tag=14",
                3,
            ),
        ]);
        assert_eq!(
            fx_gif_links(&landscape),
            ["https://gif.fxtwitter.com/ext_tw_video/1/pu/vid/640x360/b.gif"]
        );
    }

    #[test]
    fn gif_sources_prefer_the_tweet_video_variant() {
        let variants = vec![
            variant(
                "https://video.twimg.com/amplify_video/1/vid/720x1280/amplify.mp4",
                "video/mp4",
                Some(2_000_000),
            ),
            variant(
                "https://video.twimg.com/tweet_video/HTEtNFcXEAAY4Dv.mp4",
                "video/mp4",
                Some(800_000),
            ),
        ];

        // GIF kaynağı `tweet_video` varyantından türetilir (FxEmbed GIF servisi): tercih edilir.
        assert_eq!(
            best_mp4(Some(&variants), Some(3_000), 8 * 1024 * 1024, true).as_deref(),
            Some("https://video.twimg.com/tweet_video/HTEtNFcXEAAY4Dv.mp4")
        );
        // Tercih kapalıyken yalnızca ağırlık kuralı işler: en yüksek bit hızı kazanır.
        assert_eq!(
            best_mp4(Some(&variants), Some(3_000), 8 * 1024 * 1024, false).as_deref(),
            Some("https://video.twimg.com/amplify_video/1/vid/720x1280/amplify.mp4")
        );
    }

    #[test]
    fn size_estimates_need_both_bitrate_and_duration() {
        // 1 Mbps × 8 sn = 1 MB
        assert_eq!(
            estimated_bytes(Some(1_000_000), Some(8_000)),
            Some(1_000_000)
        );
        assert_eq!(estimated_bytes(None, Some(8_000)), None);
        assert_eq!(estimated_bytes(Some(1_000_000), None), None);
        assert_eq!(estimated_bytes(Some(0), Some(8_000)), None);

        // Boyut bilinmiyorsa varyant yine de kullanılabilir ("sığar" sayılır).
        let unknown = vec![variant(
            "https://video.twimg.com/amplify_video/1/vid/x.mp4",
            "video/mp4",
            None,
        )];
        assert_eq!(
            best_mp4(Some(&unknown), None, 8 * 1024 * 1024, false).as_deref(),
            Some("https://video.twimg.com/amplify_video/1/vid/x.mp4")
        );
    }

    // =================================================================================
    // KAYNAK BİRLEŞTİRME (acquire)
    // =================================================================================

    fn sourced(lang: &str, text: &str) -> BuiltinPost {
        let mut post = BuiltinPost::new(
            Platform::Twitter,
            "https://x.com/kullanici/status/1",
            Author::new("Kullanıcı", "kullanici"),
        );
        post.language = Some(lang.to_string());
        post.text = text.to_string();
        post
    }

    fn translated(lang: &str, text: &str) -> BuiltinPost {
        let mut post = sourced(lang, text);
        post.translation = Some(Translation {
            source: Some(lang.to_string()),
        });
        post
    }

    async fn after(ms: u64, result: Result<BuiltinPost, String>) -> Result<BuiltinPost, String> {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        result
    }

    const HEDGE: Duration = Duration::from_millis(300);
    const BUDGET: Duration = Duration::from_millis(400);

    #[tokio::test]
    async fn english_post_is_sent_without_waiting_for_the_translation() {
        let started = Instant::now();
        let fetched = acquire(
            &EngineSettings::default(),
            after(10, Ok(sourced("en", "Hello"))),
            Some(after(2_000, Ok(translated("en", "Merhaba")))),
            after(10, Err("kullanılmamalı".into())),
            HEDGE,
            BUDGET,
        )
        .await
        .unwrap();

        assert!(started.elapsed() < Duration::from_millis(300));
        assert_eq!(fetched.post.text, "Hello");
        assert!(fetched.cacheable);
    }

    #[tokio::test]
    async fn foreign_post_uses_the_translation_within_the_budget() {
        let fetched = acquire(
            &EngineSettings::default(),
            after(10, Ok(sourced("de", "Guten Morgen"))),
            Some(after(80, Ok(translated("de", "Günaydın")))),
            after(10, Err("kullanılmamalı".into())),
            HEDGE,
            BUDGET,
        )
        .await
        .unwrap();

        assert_eq!(fetched.post.text, "Günaydın");
        assert!(fetched.post.translation.is_some());
        assert!(fetched.cacheable);
    }

    #[tokio::test]
    async fn late_translation_keeps_the_original_and_is_not_cached() {
        let started = Instant::now();
        let fetched = acquire(
            &EngineSettings::default(),
            after(10, Ok(sourced("de", "Guten Morgen"))),
            Some(after(3_000, Ok(translated("de", "Günaydın")))),
            after(10, Err("kullanılmamalı".into())),
            HEDGE,
            BUDGET,
        )
        .await
        .unwrap();

        assert!(started.elapsed() < Duration::from_millis(1_000));
        assert_eq!(fetched.post.text, "Guten Morgen");
        assert!(!fetched.cacheable, "çevirisiz sonuç önbelleğe yazılmamalı");
    }

    /// X Grok çevirisini ilk istekte henüz üretmemiş olabilir: dil ekli yanıt çevirisiz gelir.
    #[tokio::test]
    async fn translation_not_yet_available_is_not_cached() {
        let fetched = acquire(
            &EngineSettings::default(),
            after(10, Ok(sourced("de", "Guten Morgen"))),
            Some(after(40, Ok(sourced("de", "Guten Morgen")))),
            after(10, Err("kullanılmamalı".into())),
            HEDGE,
            BUDGET,
        )
        .await
        .unwrap();

        assert!(!fetched.cacheable);
    }

    #[tokio::test]
    async fn translated_response_arriving_first_is_used_directly() {
        let started = Instant::now();
        let fetched = acquire(
            &EngineSettings::default(),
            after(250, Ok(sourced("de", "Guten Morgen"))),
            Some(after(10, Ok(translated("de", "Günaydın")))),
            after(10, Err("kullanılmamalı".into())),
            HEDGE,
            BUDGET,
        )
        .await
        .unwrap();

        assert!(started.elapsed() < Duration::from_millis(200));
        assert_eq!(fetched.post.text, "Günaydın");
        assert!(fetched.cacheable);
    }

    #[tokio::test]
    async fn untranslated_quote_also_waits_for_the_translation() {
        let mut plain = sourced("en", "Bak");
        plain.quote = Some(Box::new(sourced("ja", "こんにちは")));
        let mut done = sourced("en", "Bak");
        done.quote = Some(Box::new(translated("ja", "Merhaba")));

        let fetched = acquire(
            &EngineSettings::default(),
            after(10, Ok(plain)),
            Some(after(60, Ok(done))),
            after(10, Err("kullanılmamalı".into())),
            HEDGE,
            BUDGET,
        )
        .await
        .unwrap();

        let quote = fetched.post.quote.expect("alıntı korunmalı");
        assert_eq!(quote.text, "Merhaba");
        assert!(fetched.cacheable);
    }

    #[tokio::test]
    async fn primary_failure_falls_back_to_syndication_without_caching() {
        let fetched = acquire(
            &EngineSettings::default(),
            after(5, Err("fx 500".into())),
            None::<std::future::Ready<Result<BuiltinPost, String>>>,
            after(5, Ok(sourced("en", "syndication"))),
            HEDGE,
            BUDGET,
        )
        .await
        .unwrap();

        assert_eq!(fetched.post.text, "syndication");
        assert!(!fetched.cacheable, "yedek veri önbelleğe yazılmamalı");
    }

    #[tokio::test]
    async fn slow_primary_is_hedged_with_syndication() {
        let started = Instant::now();
        let fetched = acquire(
            &EngineSettings::default(),
            after(3_000, Ok(sourced("en", "fx"))),
            None::<std::future::Ready<Result<BuiltinPost, String>>>,
            after(10, Ok(sourced("en", "syndication"))),
            Duration::from_millis(50),
            BUDGET,
        )
        .await
        .unwrap();

        assert!(started.elapsed() < Duration::from_millis(500));
        assert_eq!(fetched.post.text, "syndication");
        assert!(!fetched.cacheable);
    }

    #[tokio::test]
    async fn primary_is_preferred_over_a_faster_fallback_within_patience() {
        let fetched = acquire(
            &EngineSettings::default(),
            after(60, Ok(sourced("en", "fx"))),
            None::<std::future::Ready<Result<BuiltinPost, String>>>,
            after(5, Ok(sourced("en", "syndication"))),
            HEDGE,
            BUDGET,
        )
        .await
        .unwrap();

        assert_eq!(fetched.post.text, "fx");
        assert!(fetched.cacheable);
    }

    #[tokio::test]
    async fn ready_fallback_is_used_as_soon_as_patience_runs_out() {
        let started = Instant::now();
        let fetched = acquire(
            &EngineSettings::default(),
            after(3_000, Ok(sourced("en", "fx"))),
            None::<std::future::Ready<Result<BuiltinPost, String>>>,
            after(5, Ok(sourced("en", "syndication"))),
            Duration::from_millis(100),
            BUDGET,
        )
        .await
        .unwrap();

        let elapsed = started.elapsed();
        assert!(
            elapsed >= Duration::from_millis(100),
            "ana kaynağa süre tanınmalı"
        );
        assert!(elapsed < Duration::from_millis(400));
        assert_eq!(fetched.post.text, "syndication");
        assert!(!fetched.cacheable);
    }

    #[tokio::test]
    async fn every_source_failing_reports_all_errors() {
        let error = acquire(
            &EngineSettings::default(),
            after(5, Err("fx 500".into())),
            Some(after(5, Err("fx/tr 500".into()))),
            after(5, Err("syndication 404".into())),
            HEDGE,
            BUDGET,
        )
        .await
        .err()
        .expect("hata dönmeli");

        assert!(error.contains("fx 500") && error.contains("syndication 404"));
    }

    #[test]
    fn disabled_translation_never_counts_as_missing() {
        let engine = EngineSettings {
            auto_translate: false,
            ..EngineSettings::default()
        };
        assert!(!translation_missing(
            &sourced("de", "Guten Morgen"),
            &engine
        ));
        assert!(translation_missing(
            &sourced("de", "Guten Morgen"),
            &EngineSettings::default()
        ));
        assert!(!translation_missing(
            &sourced("tr", "Günaydın"),
            &EngineSettings::default()
        ));
    }

    #[tokio::test]
    #[ignore = "requires live FxTwitter/vxTwitter access; run explicitly"]
    async fn live_fetch_main_and_alternative_sources() {
        let client = reqwest::Client::builder()
            .tls_backend_rustls()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let parsed = ParsedPost {
            platform: Platform::Twitter,
            original_url: "https://x.com/firstshowing/status/2106814102412968268".to_string(),
            is_spoiler: false,
            user_comment: None,
        };

        let main = fetch(&client, &parsed, &EngineSettings::default())
            .await
            .expect("FxTwitter verisi çekilmeli");
        assert!(
            main.post.stats.views.is_some(),
            "görüntülenme FxTwitter'dan gelmeli"
        );
        let quote = main.post.quote.as_deref().expect("ilk alıntı bulunmalı");
        assert_eq!(quote.author.handle, "Mark_Hobin");
        assert!(quote.quote.is_none(), "alıntının alıntısı çekilmez");

        let engine = EngineSettings {
            x_alternative_source: true,
            ..EngineSettings::default()
        };
        let alternative = fetch(&client, &parsed, &engine)
            .await
            .expect("vxTwitter verisi çekilmeli");
        assert_eq!(
            alternative
                .post
                .quote
                .as_deref()
                .map(|quote| quote.author.handle.as_str()),
            Some("Mark_Hobin")
        );
        assert!(
            alternative.post.stats.views.is_none(),
            "vxTwitter görüntülenme vermez"
        );
    }

    // =================================================================================
    // LİNK KARTI (harici bağlantı önizlemesi)
    // =================================================================================

    #[test]
    fn link_preview_skips_x_addresses_and_picks_the_external_one() {
        let text = "yeni yazı: https://t.co/abc ve alıntı https://x.com/biri/status/9 \
                    ardından https://example.com/haber?x=1";
        assert_eq!(
            external_link(text).as_deref(),
            Some("https://example.com/haber?x=1"),
            "X bağlantıları atlanır, harici adres seçilir"
        );
        assert!(external_link("sadece https://x.com/biri/status/9").is_none());
        assert!(external_link("link içermeyen metin").is_none());
        // X'in alt alan adları da harici bağlantı sayılmaz.
        assert!(external_link("bak https://mobile.twitter.com/biri/status/9").is_none());
        assert!(external_link("https://pbs.twimg.com/media/a.jpg https://www.x.com/i").is_none());
        // Benzer ama yabancı alan adları atlanmaz.
        assert_eq!(
            external_link("https://notx.com/haber").as_deref(),
            Some("https://notx.com/haber")
        );
    }

    #[test]
    fn link_preview_falls_back_to_the_host_name() {
        assert_eq!(
            host_label("https://www.bbc.com.tr/haber/1?x=2").as_deref(),
            Some("bbc.com.tr")
        );
        assert_eq!(
            host_label("https://example.com:8443/x").as_deref(),
            Some("example.com")
        );
        assert!(host_label("https://x.com/biri/status/9").is_none());
    }

    #[test]
    fn link_preview_meta_values_are_cleaned() {
        assert_eq!(
            meta_value(Some("  çok   boşluk  ".to_string()), 40).as_deref(),
            Some("çok boşluk"),
            "fazla boşluklar sadeleşir"
        );
        assert!(meta_value(Some("   ".to_string()), 40).is_none());
        assert_eq!(
            meta_value(Some("başlık".to_string()), 3).as_deref(),
            Some("ba…"),
            "uzun meta metni karta sığacak şekilde kesilir"
        );
    }

    #[test]
    fn link_preview_metadata_cannot_inject_masked_links() {
        let html = r#"<meta property="og:title" content="Başlık @everyone">
            <meta property="og:description" content="[Claim](https://phish)">
            <meta property="og:site_name" content="[Site](https://phish)">"#;
        let preview = build_link_preview("https://example.com/a_(b)[c]", html).expect("önizleme");
        assert_eq!(preview.url, "https://example.com/a_%28b%29%5Bc%5D");
        assert_eq!(
            preview.description.as_deref(),
            Some("\\[Claim\\](https://phish)")
        );
        assert_eq!(
            preview.site_name.as_deref(),
            Some("\\[Site\\](https://phish)")
        );
        // Başlık V2 kartında zaten kaçırılır: burada yalnızca bahsetme etkisizleşir (çift kaçış yok).
        assert_eq!(preview.title.as_deref(), Some("Başlık @\u{200b}everyone"));
    }

    #[test]
    fn link_preview_image_is_resolved_and_never_truncated() {
        let page = url::Url::parse("https://example.com/haber/1").unwrap();
        assert_eq!(
            link_preview_image(&page, "/x.png").as_deref(),
            Some("https://example.com/x.png")
        );
        assert_eq!(
            link_preview_image(&page, "//cdn.example.com/x.png").as_deref(),
            Some("https://cdn.example.com/x.png")
        );
        assert!(link_preview_image(&page, "javascript:alert(1)").is_none());
        assert!(link_preview_image(&page, "data:image/png;base64,AAAA").is_none());
        let long = format!("https://cdn.example.com/{}.png", "a".repeat(600));
        assert_eq!(link_preview_image(&page, &long), Some(long.clone()));
        let too_long = format!("https://cdn.example.com/{}.png", "a".repeat(2100));
        assert!(link_preview_image(&page, &too_long).is_none());
    }

    #[test]
    fn syndication_text_is_unescaped_after_the_display_range_cut() {
        // `display_text_range` kaçışlı metni sayar: "a &amp; &lt;b&gt;" 17 birimdir.
        let tweet: SyndicationTweet = serde_json::from_value(serde_json::json!({
            "text": "a &amp; &lt;b&gt; https://t.co/medya",
            "display_text_range": [0, 17],
            "user": { "name": "Ad", "screen_name": "ad" }
        }))
        .unwrap();
        let post = syndication_to_post(&tweet, "https://x.com/ad/status/1", u64::MAX).unwrap();
        assert_eq!(post.text, "a & <b>");

        // Gerçek yanıt (king40085151/2114278300113916196): 🥰 iki UTF-16 birimidir; kod noktası
        // saymak metnin sonuna kısa linkin başını (" h") taşıyordu.
        let emoji: SyndicationTweet = serde_json::from_value(serde_json::json!({
            "text": "我会永远爱你😍，我们🥰 https://t.co/B0hVcFEVom",
            "display_text_range": [0, 13],
            "user": { "screen_name": "ad" }
        }))
        .unwrap();
        let post = syndication_to_post(&emoji, "https://x.com/ad/status/3", u64::MAX).unwrap();
        assert_eq!(post.text, "我会永远爱你😍，我们🥰");
        assert_eq!(cut_to_display_range("a😍b", Some(&[1, 2])), "😍");

        let doubly: SyndicationTweet = serde_json::from_value(serde_json::json!({
            "text": "&amp;lt;",
            "user": { "screen_name": "ad" }
        }))
        .unwrap();
        let post = syndication_to_post(&doubly, "https://x.com/ad/status/2", u64::MAX).unwrap();
        assert_eq!(post.text, "&lt;");
    }

    #[test]
    fn short_links_expand_exactly_even_when_one_prefixes_another() {
        let entities: SyndicationEntities = serde_json::from_value(serde_json::json!({
            "urls": [
                { "url": "https://t.co/abc", "expanded_url": "https://kisa.test" },
                { "url": "http://t.co/abcd", "expanded_url": "https://uzun.test/yol" },
                { "url": "https://t.co/zzz" }
            ]
        }))
        .unwrap();
        assert_eq!(
            expand_url_entities(
                "bak https://t.co/abcd ve https://t.co/abc, https://t.co/zzz https://t.co/yok",
                Some(&entities)
            ),
            "bak https://uzun.test/yol ve https://kisa.test, https://t.co/zzz https://t.co/yok"
        );
        assert_eq!(expand_url_entities("düz metin", None), "düz metin");
    }

    #[test]
    fn status_urls_are_canonical_and_validated() {
        assert_eq!(
            status_url(Some("biri_1"), "42").as_deref(),
            Some("https://x.com/biri_1/status/42")
        );
        // Adres biçimine uymayan ya da boş kullanıcı adı X'in `i` yoluna düşer.
        assert_eq!(
            status_url(Some("kötü/../ad"), "42").as_deref(),
            Some("https://x.com/i/status/42")
        );
        assert_eq!(
            status_url(None, "42").as_deref(),
            Some("https://x.com/i/status/42")
        );
        assert_eq!(status_url(Some("biri"), "42?x=1"), None);
        assert_eq!(status_url(Some("biri"), ""), None);
        assert_eq!(
            canonical_x_url("https://twitter.com/a/status/1"),
            "https://x.com/a/status/1"
        );
    }

    #[test]
    fn blank_display_names_fall_back_to_the_handle() {
        let author = x_author(Some("   "), "kullanici", None, true);
        assert_eq!(author.name, "kullanici");
        assert!(author.verified);
        assert_eq!(
            author.profile_url.as_deref(),
            Some("https://x.com/kullanici")
        );
        assert!(x_author(None, "", None, false).profile_url.is_none());
    }

    #[test]
    fn video_durations_round_the_same_way_for_every_source() {
        assert_eq!(duration_secs(13_650), 14);
        assert_eq!(duration_secs(13_000), 13);
        assert_eq!(duration_secs(0), 0);

        let item = |seconds: f64| FxMediaItem {
            duration: Some(seconds),
            ..FxMediaItem::default()
        };
        assert_eq!(fx_duration_ms(&item(13.65)).map(duration_secs), Some(14));
        assert_eq!(fx_duration_ms(&item(f64::NAN)), None);
        assert_eq!(fx_duration_ms(&item(-3.0)), Some(0));
    }
}
