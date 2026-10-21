//! Twitter / X gönderi verisi.
//!
//! Üç kaynak desteklenir:
//! - **Syndication** (`cdn.syndication.twimg.com/tweet-result`): X'in kendi uç noktası,
//!   üçüncü taraf yok, API anahtarı yok. Metin/görsel/video/alıntı/metrik verir.
//! - **FxTwitter** (`api.fxtwitter.com`): görüntülenme, repost, topluluk notu gibi ek alanlar.
//! - **VxTwitter** (`api.vxtwitter.com`): topluluk notu, alıntı ilişkisi ve X çevirisi.

use std::time::{Duration, Instant};

use regex::Regex;
use serde::Deserialize;
use std::sync::LazyLock;
use tracing::{debug, warn};

use super::media::card_image_url;
use super::translate::needs_translation;
use super::{
    Author, BuiltinPost, LinkPreview, Media, MediaKind, Stats, Translation, parse_timestamp,
    strip_trailing_media_links,
};
use crate::converters::parser::{ParsedPost, Platform};
use crate::settings::{EngineSettings, TwitterSource};

mod pipeline;
mod plan;
mod source_cache;

#[cfg(test)]
mod benchmarks;

const SYNDICATION_URL: &str = "https://cdn.syndication.twimg.com/tweet-result";
const FXTWITTER_URL: &str = "https://api.fxtwitter.com";
const VXTWITTER_URL: &str = "https://api.vxtwitter.com";

static TWEET_URL_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:twitter\.com|x\.com)/([A-Za-z0-9_]{1,20})/status/(\d{1,25})")
        .expect("geçerli tweet regex")
});

/// Hızlı yol (FxTwitter) bu süre içinde yanıt vermezse X syndication ucu **paralel**
/// başlatılır (hedge): yavaş veya askıda kalan uç kartın gelmesini geciktirmez.
const HEDGE_DELAY: Duration = Duration::from_millis(500);
/// Hedge sonrası X ucu kazandıysa, çeviri/metrikleri tamamlamak için FxTwitter'a tanınan
/// ek süre (gelmezse X verisiyle devam edilir).
const HEDGE_MERGE_GRACE: Duration = Duration::from_millis(300);
/// İç içe alıntı, ana gönderiyi bekletmemesi gereken en iyi çaba tamamlamasıdır.
const NESTED_QUOTE_TIMEOUT: Duration = Duration::from_millis(700);
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

/// Gönderi verisini seçilen kaynaktan çeker.
///
/// VxTwitter/otomatik çeviri yolunda dil ve alıntı bilgisi gelir gelmez içerik planı
/// çıkarılır. Yalnızca görünür ve eksik veriler istenir; bağımsız istekler paraleldir.
/// Açıkça seçilmiş FxTwitter ve çevirisiz syndication yolları korunur.
pub async fn fetch(
    client: &reqwest::Client,
    parsed: &ParsedPost,
    engine: &EngineSettings,
) -> Result<BuiltinPost, String> {
    let captures = TWEET_URL_REGEX
        .captures(&parsed.original_url)
        .ok_or_else(|| format!("Twitter kimliği çözümlenemedi: {}", parsed.original_url))?;
    let id = captures
        .get(2)
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| format!("Twitter gönderi kimliği yok: {}", parsed.original_url))?;

    // `max_video_bytes` platforma bağlı değildir; anahtar sözleşmeyi açık tutar.
    let video_budget =
        super::render::RenderOptions::from_engine(engine, "twitter").max_video_bytes();
    debug!(
        "X verisi çekiliyor (kaynak: {}, id: {id})",
        engine.twitter_source.key()
    );

    let source_started = Instant::now();
    let mut post =
        fetch_post_internal(client, &parsed.original_url, &id, engine, video_budget).await?;
    let source_elapsed = source_started.elapsed();

    // Medyasız zincirlerde X alıntının alıntısını da (3 seviye) gösterir.
    // Ancak ne Syndication ne de FxTwitter API'si tek yanıtta iç içe alıntıyı dönmez:
    // ilk seviye alıntı da medyasızsa, alıntılanan gönderinin kendisi çekilerek içteki
    // alıntı (ve medyası) tamamlanır.
    let nested_started = Instant::now();
    if !uses_planner(engine) && engine.show_quote && should_populate_nested_quote(&post) {
        populate_nested_quote(client, &mut post, engine, video_budget).await;
    }
    let nested_elapsed = nested_started.elapsed();

    // Gönderi yalnızca link içeriyorsa X'in kendi arayüzünde gösterdiği link kartı
    // (hedef sayfanın önizlemesi) da karta eklenir. Planlayıcı yolunda bu istek veri
    // fazının **içinde**, diğer isteklerle paralel yürür; burada ikinci kez istenmez.
    let preview_started = Instant::now();
    if !uses_planner(engine) {
        attach_link_preview(client, &mut post).await;
    }
    let preview_elapsed = preview_started.elapsed();

    debug!(
        "X veri fazları ({id}): kaynak {} ms, iç içe alıntı {} ms, link önizleme {} ms, toplam {} ms",
        source_elapsed.as_millis(),
        nested_elapsed.as_millis(),
        preview_elapsed.as_millis(),
        (source_elapsed + nested_elapsed + preview_elapsed).as_millis(),
    );

    Ok(post)
}

/// Gönderi yalnızca bir **harici bağlantı** içeriyorsa X'in "link kartı" önizlemesini ekler.
///
/// X, medyasız (ve alıntısız) gönderilerde metindeki harici adresin önizlemesini gösterir:
/// hedef sayfanın `og:`/`twitter:` etiketleri okunur. Hiçbir medya indirilmez; önizleme
/// görseli doğrudan CDN bağlantısıyla karta bağlanır. Sayfa yanıt vermezse, etiketler boşsa
/// ya da [`LINK_PREVIEW_TIMEOUT`] dolarsa kart önizlemesiz gönderilir.
async fn attach_link_preview(_client: &reqwest::Client, post: &mut BuiltinPost) {
    if !post.media.is_empty() || post.quote.is_some() {
        return;
    }
    let Some(url) = external_link(&post.text) else {
        return;
    };

    let (target, html) =
        match tokio::time::timeout(LINK_PREVIEW_TIMEOUT, super::safe_http::preview(&url)).await {
            Ok(Ok(result)) => result,
            Ok(Err(err)) => {
                debug!("Link önizlemesi alınamadı ({url}): {err}");
                return;
            }
            Err(_elapsed) => {
                debug!(
                    "Link önizlemesi {} ms içinde gelmedi: {url}",
                    LINK_PREVIEW_TIMEOUT.as_millis()
                );
                return;
            }
        };

    let Some(preview) = build_link_preview(&target, &html) else {
        debug!("Link önizlemesinde gösterilecek veri yok: {target}");
        return;
    };
    debug!(
        "Link kartı eklendi: {} ({})",
        preview.url,
        preview.site_name.as_deref().unwrap_or("-")
    );
    post.link_preview = Some(preview);
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
            let lower = url.to_ascii_lowercase();
            let host = lower.split("://").nth(1).unwrap_or_default();
            !OWN_HOSTS
                .iter()
                .any(|own| host == *own || host.starts_with(&format!("{own}/")))
        })
}

/// Adresten alan adını çıkarır (`www.` ve alt alan adı olmadan): `https://www.bbc.com/x` →
/// `bbc.com`. Site adı bildirmeyen sayfalar için yedek etikettir.
fn host_label(url: &str) -> Option<String> {
    let host = url
        .split("://")
        .nth(1)?
        .split(['/', '?', '#'])
        .next()?
        .rsplit('@')
        .next()?
        .split(':')
        .next()?
        .trim()
        .trim_start_matches("www.");
    if host.is_empty() || OWN_HOSTS.contains(&host) {
        return None;
    }
    Some(host.to_string())
}

/// Meta etiketi değerini karta uygun hale getirir: kırpılır, boşlar `None` olur, uzun
/// metinler sınıra kadar kesilir.
fn meta_value(value: Option<String>, limit: usize) -> Option<String> {
    value
        .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|text| !text.is_empty())
        .map(|text| crate::utils::truncate_chars(&text, limit))
}

/// Gönderi verisini seçilen kaynaktan çeker.
async fn fetch_post_internal(
    client: &reqwest::Client,
    original_url: &str,
    id: &str,
    engine: &EngineSettings,
    video_budget: u64,
) -> Result<BuiltinPost, String> {
    let source = engine.twitter_source;
    let translate = engine.auto_translate;

    if translate || source == TwitterSource::Vxtwitter {
        return pipeline::acquire(
            HttpFetcher {
                client: client.clone(),
                video_budget,
                translation_target: engine.translation_target.clone(),
                translate_english: engine.translate_english,
            },
            original_url,
            engine,
        )
        .await;
    }

    // 2) Hızlı yol kaynağı açıkça fxtwitter ise tek FxTwitter isteği (çeviri kapalı).
    if source == TwitterSource::Fxtwitter {
        return fetch_fx_fast(client, original_url, id, engine, video_budget).await;
    }

    // 2) X syndication + FxTwitter aynı anda: biri hata verse de diğeri kullanılır.
    let (x_result, fx_result) = tokio::join!(
        from_syndication(client, original_url, id, video_budget),
        from_fxtwitter(
            client,
            original_url,
            id,
            None,
            &engine.translation_target,
            video_budget,
            engine.translate_english,
        ),
    );

    // Syndication repost/görüntülenme/alıntı vermez: paralel istenen yanıttan tamamlanır.
    let fx_post = fx_result.as_ref().ok();
    let fx_stats = fx_post.map(|post| post.stats);
    let fx_community_note = fx_post.and_then(|post| post.community_note.clone());
    let mut post = match x_result {
        Ok(post) => post,
        Err(err) => {
            warn!("X syndication başarısız, FxTwitter sonucu kullanılıyor ({id}): {err}");
            fx_result.map_err(|fx_err| format!("{err} | {fx_err}"))?
        }
    };
    if let Some(stats) = fx_stats {
        merge_stats(&mut post.stats, &stats);
    }
    // Keep the fetched post complete even when rendering is configured to hide notes. This is
    // important for cached posts: visibility belongs to RenderOptions, not the fetch result.
    merge_community_note(&mut post.community_note, fx_community_note.as_deref());

    Ok(post)
}

/// Medyasız alıntı zincirlerinde iç içe alıntıyı (3. seviye) tamamlar.
///
/// X'in syndication ve FxTwitter API'leri performans gerekçesiyle alıntı içindeki alıntıyı
/// (2. derinlik) tek yanıtta dönmez. Gönderi ve ilk seviye alıntı medyasızsa, alıntılanan tweet
/// ayrı bir istekle çekilerek iç içe alıntı ve medyası (varsa) zincire eklenir.
async fn populate_nested_quote(
    client: &reqwest::Client,
    post: &mut BuiltinPost,
    engine: &EngineSettings,
    video_budget: u64,
) {
    let quote = match post.quote.as_deref_mut() {
        Some(q) if q.quote.is_none() => q,
        _ => return,
    };

    let quote_id = match extract_tweet_id(&quote.url) {
        Some(id) => id,
        None => return,
    };

    // Alıntılanan gönderiyi çekerek iç içe alıntıyı (3. seviye) ara.
    // Ana isteği gereksiz bekletmemek için kısa bir zaman aşımı uygulanır.
    let fetch_fut = fetch_post_internal(client, &quote.url, &quote_id, engine, video_budget);

    match tokio::time::timeout(NESTED_QUOTE_TIMEOUT, fetch_fut).await {
        Ok(Ok(full_quote)) => {
            *quote = full_quote;
        }
        Ok(Err(err)) => {
            debug!("İç içe alıntı tamamlanamadı ({quote_id}): {err}");
        }
        Err(_) => {
            debug!("İç içe alıntı isteği zaman aşımına uğradı ({quote_id})");
        }
    }
}

/// Tweet URL'sinden gönderi kimliğini (sayısal ID) çıkarır.
fn extract_tweet_id(url: &str) -> Option<String> {
    TWEET_URL_REGEX
        .captures(url)
        .and_then(|c| c.get(2))
        .map(|m| m.as_str().to_string())
}

/// İç içe alıntı isteğinin gerekli olup olmadığını belirler.
///
/// Ana gönderi medyasızsa ve ilk seviye alıntı da medyasızsa, eksik üçüncü seviyeyi
/// tamamlamak anlamlıdır. İlk seviye alıntıda medya gömülüyse yeni istek yalnızca
/// gecikme ekler; mevcut iç içe alıntı da yeniden çekilmez.
fn should_populate_nested_quote(post: &BuiltinPost) -> bool {
    post.media.is_empty()
        && post
            .quote
            .as_ref()
            .is_some_and(|quote| quote.media.is_empty() && quote.quote.is_none())
}

/// Hızlı yol: **tek** FxTwitter isteği + gecikme sigortası (hedge).
///
/// FxTwitter [`HEDGE_DELAY`] içinde yanıt vermezse X syndication ucu paralel başlatılır ve
/// ilk kullanılabilir veri kartı üretir; X ucu kazanırsa FxTwitter'a çeviri ve metrikleri
/// tamamlaması için [`HEDGE_MERGE_GRACE`] kadar ek süre tanınır. Topluluk notları açıksa, notu
/// kaybetmemek için bu süreden sonra da aynı FxTwitter isteği tamamlanana kadar beklenir. Böylece
/// notlar kapalıyken yavaş ya da askıda kalan uç kartı 12 sn'ye kadar (istek zaman aşımı) geciktiremez.
///
/// Hızlı yol **hata verirse** FxTwitter ikinci kez istenmez: kaynak `syndication` ise doğrudan
/// X ucu kullanılır (eskiden yavaş yol aynı isteği bir kez daha atıyordu).
async fn fetch_fx_fast(
    client: &reqwest::Client,
    original_url: &str,
    id: &str,
    engine: &EngineSettings,
    max_video_bytes: u64,
) -> Result<BuiltinPost, String> {
    let source = engine.twitter_source;
    let translate = engine.auto_translate;
    let translation_target = &engine.translation_target;
    let show_community_note = engine.show_community_note;
    let language = translate.then_some(translation_target.as_str());
    let fx = from_fxtwitter(
        client,
        original_url,
        id,
        language,
        translation_target,
        max_video_bytes,
        engine.translate_english,
    );
    tokio::pin!(fx);

    match tokio::time::timeout(HEDGE_DELAY, &mut fx).await {
        Ok(Ok(post)) => return Ok(post),
        Ok(Err(err)) => {
            warn!("FxTwitter isteği başarısız ({id}): {err}");
            if source == TwitterSource::Fxtwitter {
                // Dil ekli uç sorun çıkarmış olabilir: bir kez dil olmadan denenir.
                // Dil yoksa ilk istek zaten dilsizdi; aynı isteği tekrarlamak yalnızca gecikme ekler.
                if language.is_none() {
                    return Err(err);
                }
                return from_fxtwitter(
                    client,
                    original_url,
                    id,
                    None,
                    translation_target,
                    max_video_bytes,
                    engine.translate_english,
                )
                .await;
            }
            return from_syndication(client, original_url, id, max_video_bytes)
                .await
                .map_err(|sy_err| format!("{err} | {sy_err}"));
        }
        // Yanıt gecikti: hedge devreye girer (X ucu da istenir).
        Err(_elapsed) => debug!(
            "FxTwitter {} ms içinde yanıt vermedi ({id}): X ucu da paralel istendi",
            HEDGE_DELAY.as_millis()
        ),
    }

    let syndication = from_syndication(client, original_url, id, max_video_bytes);
    tokio::pin!(syndication);

    // İki uç yarışır; FxTwitter tercih edilir (çeviri, topluluk notu ve görüntülenme onda).
    let (fx_result, x_result) = tokio::select! {
        biased;
        result = &mut fx => (Some(result), None),
        result = &mut syndication => (None, Some(result)),
    };

    if let Some(result) = fx_result {
        return match result {
            Ok(post) => Ok(post),
            Err(err) => {
                debug!("FxTwitter hedge sonrası hata verdi ({id}): {err}");
                syndication
                    .await
                    .map_err(|sy_err| format!("{err} | {sy_err}"))
            }
        };
    }

    // X ucu kazandı ama hata verdiyse FxTwitter'ın kendi zaman aşımı dolana kadar beklenir: hızlı bir syndication hatası, hâlâ yolda olan geçerli yanıtı ek süre bitti diye düşürmemeli.
    let post = match x_result.expect("select her zaman bir dalı döner") {
        Ok(post) => post,
        Err(sy_err) => {
            debug!("X ucu hedge sonrası hata verdi ({id}): {sy_err}");
            return fx.await.map_err(|fx_err| format!("{fx_err} | {sy_err}"));
        }
    };

    // X ucu kazandı: veri hazır. Normalde yalnızca kısa bir ek süre tanınır; topluluk notları
    // açıksa aynı FxTwitter isteği tamamlanana kadar beklenir, aksi halde not kaybolur.
    match tokio::time::timeout(HEDGE_MERGE_GRACE, &mut fx).await {
        Ok(Ok(fx_post)) => return Ok(fx_post),
        Ok(Err(err)) => {
            debug!("FxTwitter hedge sonrası hata verdi ({id}): {err}");
        }
        Err(_) if show_community_note => {
            // A late FxTwitter response is the only source of a community note. When notes are
            // enabled, wait for that in-flight request before falling back to the fast Syndication
            // result; otherwise the note would disappear whenever the 300 ms grace was exceeded.
            if let Ok(fx_post) = fx.await {
                return Ok(fx_post);
            }
        }
        Err(_) => {}
    }
    Ok(post)
}

fn uses_planner(engine: &EngineSettings) -> bool {
    engine.auto_translate || engine.twitter_source == TwitterSource::Vxtwitter
}

#[derive(Clone)]
struct HttpFetcher {
    client: reqwest::Client,
    video_budget: u64,
    translation_target: String,
    translate_english: bool,
}

static SOURCE_RESULTS: LazyLock<source_cache::SourceCache<pipeline::Snapshot>> =
    LazyLock::new(|| source_cache::SourceCache::new(Duration::from_secs(600), 512));

impl HttpFetcher {
    fn key(&self, source: &str, id: &str, language: Option<&str>) -> String {
        format!(
            "{source}:{id}:{}:{}:{}:{}",
            language.unwrap_or("original"),
            self.translation_target,
            self.translate_english,
            self.video_budget
        )
    }
}

#[serenity::all::async_trait]
impl pipeline::Fetcher for HttpFetcher {
    async fn cached(&self, request: &pipeline::Request) -> Option<pipeline::Snapshot> {
        let (source, url, language) = match request {
            pipeline::Request::Vx { url, language } => ("vx", url, language.as_deref()),
            pipeline::Request::Fx { url } => ("fx", url, None),
            pipeline::Request::Syndication { .. } => return None,
        };
        let id = extract_tweet_id(url)?;
        SOURCE_RESULTS
            .get(&self.key(source, &id, language))
            .await
            .map(|snapshot| (*snapshot).clone())
    }

    async fn fetch(&self, request: pipeline::Request) -> Result<pipeline::Snapshot, String> {
        let (source, url, language) = match request {
            pipeline::Request::Vx { url, language } => ("vx", url, language),
            pipeline::Request::Fx { url } => ("fx", url, None),
            pipeline::Request::Syndication { url } => ("syndication", url, None),
        };
        let id = extract_tweet_id(&url).ok_or_else(|| "X request has no tweet id".to_string())?;
        // ID-based keys reuse a standalone tweet when it appears as a quote elsewhere.
        // Target/source/budget remain isolated; spoiler/NSFW decisions happen only later.
        let key = self.key(source, &id, language.as_deref());
        let started = Instant::now();
        let result = SOURCE_RESULTS
            .get_or_fetch(key, || async {
                if source == "vx" {
                    from_vxtwitter(
                        &self.client,
                        &url,
                        &id,
                        language.as_deref(),
                        &self.translation_target,
                        self.translate_english,
                    )
                    .await
                } else if source == "fx" {
                    from_fxtwitter(
                        &self.client,
                        &url,
                        &id,
                        None,
                        &self.translation_target,
                        self.video_budget,
                        self.translate_english,
                    )
                    .await
                    .map(|post| pipeline::Snapshot {
                        post,
                        nested_hint: plan::NestedHint::Unknown,
                    })
                } else {
                    from_syndication(&self.client, &url, &id, self.video_budget)
                        .await
                        .map(|post| pipeline::Snapshot {
                            post,
                            nested_hint: plan::NestedHint::Unknown,
                        })
                }
            })
            .await;
        debug!(
            "X source={source} id={id} language={} elapsed_ms={} ok={}",
            language.as_deref().unwrap_or("original"),
            started.elapsed().as_millis(),
            result.is_ok(),
        );
        result.map(|snapshot| (*snapshot).clone())
    }

    /// Harici link kartını **veri istekleriyle paralel** çözer (bütçe planlayıcıdadır).
    async fn preview(&self, url: &str) -> Option<LinkPreview> {
        let (target, html) = match tokio::time::timeout(
            LINK_PREVIEW_TIMEOUT,
            super::safe_http::preview(url),
        )
        .await
        {
            Ok(Ok(result)) => result,
            Ok(Err(err)) => {
                debug!("Link önizlemesi alınamadı ({url}): {err}");
                return None;
            }
            Err(_) => {
                debug!(
                    "Link önizlemesi {} ms içinde gelmedi: {url}",
                    LINK_PREVIEW_TIMEOUT.as_millis()
                );
                return None;
            }
        };
        build_link_preview(&target, &html)
    }
}

/// Gönderi verisini vxtwitter API'sinden çeker.
///
/// Dil eki (`/tr`) verildiğinde yanıt `translation` alanında X'in kendi Grok çevirisini
/// taşır; topluluk notu (`communityNote`) her yanıtta mevcuttur (yoksa null).
async fn from_vxtwitter(
    client: &reqwest::Client,
    original_url: &str,
    id: &str,
    language: Option<&str>,
    translation_target: &str,
    translate_english: bool,
) -> Result<pipeline::Snapshot, String> {
    let language_suffix = language.map(|lang| format!("/{lang}")).unwrap_or_default();
    let handle = TWEET_URL_REGEX
        .captures(original_url)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
        .filter(|handle| !handle.is_empty())
        // vx yoldaki kullanıcı adını yalnızca adres biçimi için kullanır; her segment çalışır.
        .unwrap_or_else(|| "Twitter".to_string());
    let url = format!("{VXTWITTER_URL}/{handle}/status/{id}{language_suffix}");
    let value = super::get_json(client, &url).await?;
    let nested_hint = vx_nested_hint(&value);
    let tweet: VxTweet = serde_json::from_value(value)
        .map_err(|e| format!("vxtwitter yanıtı çözümlenemedi ({id}): {e}"))?;

    if tweet.tweet_id.is_none() {
        return Err(format!("vxtwitter yanıtında gönderi kimliği yok: {id}"));
    }

    Ok(pipeline::Snapshot {
        post: vx_to_post(&tweet, original_url, translation_target, translate_english)?,
        nested_hint,
    })
}

/// An explicit qrtURL:null is evidence that the inline middle tweet has no quote.
/// A URL points straight to the third tweet; a missing field is unknown, not absent.
fn vx_nested_hint(value: &serde_json::Value) -> plan::NestedHint {
    let tweet = value
        .get("retweet")
        .filter(|value| value.is_object())
        .unwrap_or(value);
    let Some(quote) = tweet.get("qrt").filter(|value| value.is_object()) else {
        return plan::NestedHint::Absent;
    };
    match quote.get("qrtURL") {
        Some(serde_json::Value::Null) => plan::NestedHint::Absent,
        Some(serde_json::Value::String(url)) => {
            let valid = url::Url::parse(url).ok().is_some_and(|parsed| {
                matches!(parsed.scheme(), "https" | "http")
                    && matches!(
                        parsed.host_str(),
                        Some("x.com" | "twitter.com" | "www.x.com" | "www.twitter.com")
                    )
                    && extract_tweet_id(url).is_some()
            });
            if valid {
                plan::NestedHint::Present(url.clone())
            } else {
                plan::NestedHint::Unknown
            }
        }
        _ => plan::NestedHint::Unknown,
    }
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
/// Alıntı yalnızca **ilk seviye** inline gömülür (`qrt`); daha derin zincirler
/// [`populate_nested_quote`] akışındaki mevcut yolla tamamlanır.
fn vx_to_post(
    tweet: &VxTweet,
    fallback_url: &str,
    translation_target: &str,
    translate_english: bool,
) -> Result<BuiltinPost, String> {
    let handle = tweet.user_screen_name.clone().unwrap_or_default();

    // Saf retweet: vx gerçek içeriği `retweet` objesine koyar ve dış metni boşatır.
    if let Some(inner) = tweet.retweet.as_deref() {
        let mut inner_post =
            vx_to_post(inner, fallback_url, translation_target, translate_english)?;
        inner_post.quote = None; // retweet içinde alıntı zinciri gösterilmez.
        return Ok(inner_post);
    }

    let mut author = Author::new(
        tweet
            .user_name
            .clone()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| handle.clone()),
        handle.clone(),
    );
    author.avatar = tweet.user_profile_image_url.clone();
    author.profile_url = if handle.is_empty() {
        None
    } else {
        Some(format!("https://x.com/{handle}"))
    };

    let url = tweet
        .tweet_url
        .clone()
        .filter(|url| !url.trim().is_empty())
        .or_else(|| {
            let id = tweet
                .tweet_id
                .as_deref()
                .filter(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))?;
            let handle = if !handle.is_empty()
                && handle
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                handle.as_str()
            } else {
                "i"
            };
            Some(format!("https://x.com/{handle}/status/{id}"))
        })
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
        // vx görüntülenme sayısını bildirmez; FX akışı istenirse merge_stats ile tamamlar.
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
    let duration = item.duration_millis.map(|ms| ms.div_ceil(1000));

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

/// Eksik alanları doldurur; kaynakta 0/`None` olan değerleri taze veriyle değiştirir.
fn merge_stats(target: &mut Stats, fresh: &Stats) {
    if target.replies == 0 {
        target.replies = fresh.replies;
    }
    if target.reposts == 0 {
        target.reposts = fresh.reposts;
    }
    if target.likes == 0 {
        target.likes = fresh.likes;
    }
    if target.quotes == 0 {
        target.quotes = fresh.quotes;
    }
    if target.views.is_none() {
        target.views = fresh.views;
    }
}

/// Syndication yanıtında bulunmayan topluluk notunu FxTwitter yanıtından tamamlar.
/// Hedefte zaten bir not varsa onu korur; boş bir FxTwitter notu da karta eklenmez.
fn merge_community_note(target: &mut Option<String>, fresh: Option<&str>) {
    if target.is_none() {
        *target = fresh
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned);
    }
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
    let value = super::get_json(client, &url).await?;
    let tweet: SyndicationTweet = serde_json::from_value(value)
        .map_err(|e| format!("Syndication yanıtı çözümlenemedi ({id}): {e}"))?;

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
    let handle = user.and_then(|u| u.screen_name.clone()).unwrap_or_default();
    let mut author = Author::new(
        user.and_then(|u| u.name.clone())
            .unwrap_or_else(|| handle.clone()),
        handle.clone(),
    );
    author.avatar = user.and_then(|u| u.profile_image_url_https.clone());
    author.verified = user
        .and_then(|u| u.is_blue_verified.or(u.verified))
        .unwrap_or(false);
    author.profile_url = if handle.is_empty() {
        None
    } else {
        Some(format!("https://x.com/{handle}"))
    };

    let mut post = BuiltinPost::new(Platform::Twitter, url, author);

    let raw_text = tweet
        .note_tweet
        .as_ref()
        .and_then(|n| n.text.clone())
        .or_else(|| tweet.text.clone())
        .unwrap_or_default();
    // X metni `&amp;`/`&lt;`/`&gt;` kaçışlı verir, `display_text_range` ise kaçışsız metnin kod noktalarını sayar (`&` tek karakterdir).
    // Bu yüzden kesmeden önce çözülür; aksi halde her kaçış aralığı kaydırır ve metnin sonu kırpılır.
    let raw_text = super::unescape_html(&raw_text);
    let raw_text = if tweet.note_tweet.is_some() {
        raw_text
    } else {
        cut_to_display_range(&raw_text, tweet.display_text_range.as_deref())
    };
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

    if let Some(quoted) = tweet.quoted_tweet.as_deref() {
        let quoted_url = quoted.id_str.as_deref().map(|quoted_id| {
            let quoted_handle = quoted
                .user
                .as_ref()
                .and_then(|u| u.screen_name.as_deref())
                .filter(|s| !s.is_empty())
                .unwrap_or("i");
            format!("https://x.com/{quoted_handle}/status/{quoted_id}")
        });
        if let Some(quoted_url) = quoted_url
            && let Ok(quoted_post) = syndication_to_post(quoted, &quoted_url, max_video_bytes)
        {
            post.quote = Some(Box::new(quoted_post));
        }
    }

    Ok(post)
}

/// Gönderi metnini `display_text_range` sınırına kadar keser (metnin sonundaki medya
/// kısa linki bu aralığın dışında kalır).
fn cut_to_display_range(text: &str, range: Option<&[usize]>) -> String {
    match range {
        Some(range) if range.len() >= 2 => text
            .chars()
            .skip(range[0])
            .take(range[1].saturating_sub(range[0]))
            .collect(),
        _ => text.to_string(),
    }
}

/// `entities.urls` içindeki `t.co` kısa linklerini gerçek adresleriyle değiştirir.
fn expand_url_entities(text: &str, entities: Option<&SyndicationEntities>) -> String {
    let mut out = text.to_string();
    if let Some(urls) = entities.and_then(|e| e.urls.as_deref()) {
        for entity in urls {
            if let (Some(short), Some(expanded)) =
                (entity.url.as_deref(), entity.expanded_url.as_deref())
            {
                if short == expanded {
                    continue;
                }
                out = out.replace(short, expanded);
                out = out.replace(&short.replace("http://", "https://"), expanded);
            }
        }
    }
    out
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
            duration: video.duration_ms.map(|ms| ms / 1000),
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
                duration: video_info.duration_millis.map(|ms| ms / 1000),
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
    let value = super::get_json(client, &url).await?;
    let response: FxResponse = serde_json::from_value(value)
        .map_err(|e| format!("fxtwitter yanıtı çözümlenemedi ({id}): {e}"))?;

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
    let handle = user.and_then(|a| a.screen_name.clone()).unwrap_or_default();
    let mut author = Author::new(
        user.and_then(|a| a.name.clone())
            .unwrap_or_else(|| handle.clone()),
        handle.clone(),
    );
    author.avatar = user.and_then(|a| a.avatar_url.clone());
    author.verified = user
        .and_then(|a| a.verification.as_ref())
        .and_then(|v| v.verified)
        .unwrap_or(false);
    author.profile_url = if handle.is_empty() {
        None
    } else {
        Some(format!("https://x.com/{handle}"))
    };

    let url = tweet
        .url
        .clone()
        .unwrap_or_else(|| fallback_url.to_string())
        .replace("://twitter.com/", "://x.com/");

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

    if let Some(quoted) = tweet.quote.as_deref() {
        let quoted_url = quoted
            .url
            .as_deref()
            .filter(|u| !u.trim().is_empty())
            .map(|u| u.replace("://twitter.com/", "://x.com/"))
            .or_else(|| {
                let id = quoted.id.as_deref()?;
                let handle = quoted
                    .author
                    .as_ref()
                    .and_then(|a| a.screen_name.as_deref())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("i");
                Some(format!("https://x.com/{handle}/status/{id}"))
            })
            .unwrap_or_default();
        if !quoted_url.is_empty()
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
    let duration_ms = item
        .duration
        .map(|seconds| (seconds.max(0.0) * 1000.0).round() as u64);

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
        duration: item.duration.map(|seconds| seconds.round() as u64),
    })
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
    fn twitter_hedge_and_nested_quote_budgets_are_separate() {
        assert_eq!(HEDGE_DELAY, Duration::from_millis(500));
        assert_eq!(NESTED_QUOTE_TIMEOUT, Duration::from_millis(700));
    }

    #[test]
    fn vx_quote_topology_distinguishes_absent_unknown_and_direct_third_post() {
        assert_eq!(
            vx_nested_hint(&serde_json::json!({"qrt": null})),
            plan::NestedHint::Absent
        );
        assert_eq!(
            vx_nested_hint(&serde_json::json!({"qrt": {"qrtURL": null}})),
            plan::NestedHint::Absent
        );
        assert_eq!(
            vx_nested_hint(&serde_json::json!({"qrt": {}})),
            plan::NestedHint::Unknown
        );
        let url = "https://twitter.com/i/status/3";
        assert_eq!(
            vx_nested_hint(&serde_json::json!({"qrt": {"qrtURL": url}})),
            plan::NestedHint::Present(url.to_string()),
        );
        assert_eq!(
            vx_nested_hint(
                &serde_json::json!({"qrt": {"qrtURL": "https://untrusted.test/x.com/a/status/3"}})
            ),
            plan::NestedHint::Unknown,
        );
        assert_eq!(
            vx_nested_hint(&serde_json::json!({"retweet": {"qrt": {"qrtURL": url}}})),
            plan::NestedHint::Present(url.to_string()),
        );
    }

    #[test]
    fn nested_quote_refresh_only_runs_for_media_less_first_level_quotes() {
        let mut post = BuiltinPost::new(
            Platform::Twitter,
            "https://x.com/kullanici/status/1",
            Author::new("Kullanıcı", "kullanici"),
        );
        assert!(
            !should_populate_nested_quote(&post),
            "alıntı yoksa yenileme yok"
        );

        post.quote = Some(Box::new(BuiltinPost::new(
            Platform::Twitter,
            "https://x.com/alinti/status/2",
            Author::new("Alıntı", "alinti"),
        )));
        assert!(
            should_populate_nested_quote(&post),
            "medyasız alıntı tamamlanmalı"
        );

        post.quote.as_mut().unwrap().media.push(Media {
            kind: MediaKind::Image,
            url: "https://pbs.twimg.com/media/quote.jpg".to_string(),
            poster: None,
            width: None,
            height: None,
            duration: None,
        });
        assert!(
            !should_populate_nested_quote(&post),
            "gömülü ilk seviye medya yeni isteği gereksiz kılar"
        );

        post.quote.as_mut().unwrap().media.clear();
        post.quote.as_mut().unwrap().quote = Some(Box::new(BuiltinPost::new(
            Platform::Twitter,
            "https://x.com/inner/status/3",
            Author::new("İç", "inner"),
        )));
        assert!(
            !should_populate_nested_quote(&post),
            "mevcut iç içe alıntı yeniden çekilmemeli"
        );

        post.quote.as_mut().unwrap().quote = None;
        post.media.push(Media {
            kind: MediaKind::Image,
            url: "https://pbs.twimg.com/media/post.jpg".to_string(),
            poster: None,
            width: None,
            height: None,
            duration: None,
        });
        assert!(
            !should_populate_nested_quote(&post),
            "ana gönderide medya varsa zincir yenilenmemeli"
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
    fn missing_syndication_metrics_are_merged_without_overwriting_existing_ones() {
        // Syndication yalnızca yanıt + beğeni verir; repost/görüntülenme FxTwitter'dan gelir.
        let mut stats = Stats {
            replies: 87,
            reposts: 0,
            likes: 15_600,
            quotes: 0,
            views: None,
        };
        merge_stats(
            &mut stats,
            &Stats {
                replies: 90,
                reposts: 412,
                likes: 15_700,
                quotes: 12,
                views: Some(1_240_000),
            },
        );

        assert_eq!(stats.replies, 87, "mevcut değer korunur");
        assert_eq!(stats.likes, 15_600, "mevcut beğeni korunur");
        assert_eq!(stats.reposts, 412, "eksik repost tamamlanır");
        assert_eq!(stats.quotes, 12, "eksik alıntı tamamlanır");
        assert_eq!(
            stats.views,
            Some(1_240_000),
            "eksik görüntülenme tamamlanır"
        );
    }

    #[test]
    fn community_note_is_merged_from_fxtwitter_without_replacing_existing_note() {
        let mut note = None;
        merge_community_note(&mut note, Some("  Topluluk notu  "));
        assert_eq!(note.as_deref(), Some("Topluluk notu"));

        merge_community_note(&mut note, Some("başka not"));
        assert_eq!(note.as_deref(), Some("Topluluk notu"));
    }

    #[test]
    fn empty_fxtwitter_community_note_is_not_added() {
        let mut note = None;
        merge_community_note(&mut note, Some("  "));
        assert!(note.is_none());
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

    #[tokio::test]
    #[ignore = "requires live X/vxTwitter access; run explicitly"]
    async fn live_fetch_three_level_quote() {
        let client = reqwest::Client::builder()
            .tls_backend_rustls()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let parsed = ParsedPost {
            platform: Platform::Twitter,
            original_url: "https://x.com/firstshowing/status/2106814102412968268".to_string(),
            fixed_url: "https://x.com/firstshowing/status/2106814102412968268".to_string(),
            is_spoiler: false,
            user_comment: None,
        };
        let mut engine = EngineSettings {
            twitter_source: TwitterSource::Vxtwitter,
            show_quote: true,
            ..EngineSettings::default()
        };
        let post = fetch(&client, &parsed, &engine)
            .await
            .expect("tweet çekilmeli");

        assert!(post.quote.is_some(), "ilk alıntı (Mark_Hobin) bulunmalı");
        let middle = post.quote.as_ref().unwrap();
        assert_eq!(middle.author.handle, "Mark_Hobin");

        assert!(
            middle.quote.is_some(),
            "ikinci alıntı (DiscussingFilm) iç içe alıntı olarak tamamlanmalı"
        );
        let inner = middle.quote.as_ref().unwrap();
        assert_eq!(inner.author.handle, "DiscussingFilm");
        assert!(
            !inner.media.is_empty(),
            "DiscussingFilm görseli içermeli: {:?}",
            inner.media
        );

        // Modern V2 kart çıktısında 3 gönderi ve görsel de yer almalı:
        let render_opts =
            crate::converters::builtin::render::RenderOptions::from_engine(&engine, "twitter");
        let plan = crate::converters::builtin::render::plan_media(&post, &render_opts);
        assert!(!plan.remote_images.is_empty(), "plan görsel içermeli");

        let resolved =
            crate::converters::builtin::render::resolve_media(&client, &post, &plan, &render_opts)
                .await;
        let emojis = crate::settings::EmojiSettings::default();
        let card = crate::converters::builtin::v2::V2Card {
            post: &post,
            options: &render_opts,
            resolved: &resolved,
            emojis: &emojis,
            spoiler: false,
            sender: None,
            user_comment: None,
            impersonation: None,
            allowed_mentions: None,
        };
        let payload = crate::converters::builtin::v2::build_payload(&card);
        let payload_str = serde_json::to_string(&payload).unwrap();
        assert!(payload_str.contains("firstshowing"), "ana yazar olmalı");
        assert!(payload_str.contains("Mark_Hobin"), "ilk alıntı olmalı");
        assert!(
            payload_str.contains("DiscussingFilm"),
            "ikinci alıntı olmalı"
        );
        assert!(
            payload_str.contains("DIGGER"),
            "DiscussingFilm metni olmalı"
        );
        assert!(
            payload_str.contains("media_gallery") || payload_str.contains("HTy-9WNWYAAGzcK"),
            "galeri görseli olmalı"
        );

        // Syndication kaynağı da 3 seviyeli zinciri tamamlayabilmeli:
        engine.twitter_source = TwitterSource::Syndication;
        let post_syn = fetch(&client, &parsed, &engine)
            .await
            .expect("syndication tweet çekilmeli");
        let middle_syn = post_syn.quote.expect("syndication ilk alıntı bulunmalı");
        assert_eq!(middle_syn.author.handle, "Mark_Hobin");
        let inner_syn = middle_syn
            .quote
            .expect("syndication iç içe alıntı tamamlanmalı");
        assert_eq!(inner_syn.author.handle, "DiscussingFilm");
        assert!(
            !inner_syn.media.is_empty(),
            "syndication medya tamamlanmalı"
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
    fn syndication_text_is_unescaped_before_the_display_range_cut() {
        // `display_text_range` kaçışsız metni sayar: "a & <b>" 7 karakterdir.
        let tweet: SyndicationTweet = serde_json::from_value(serde_json::json!({
            "text": "a &amp; &lt;b&gt; https://t.co/medya",
            "display_text_range": [0, 7],
            "user": { "name": "Ad", "screen_name": "ad" }
        }))
        .unwrap();
        let post = syndication_to_post(&tweet, "https://x.com/ad/status/1", u64::MAX).unwrap();
        assert_eq!(post.text, "a & <b>");

        let doubly: SyndicationTweet = serde_json::from_value(serde_json::json!({
            "text": "&amp;lt;",
            "user": { "screen_name": "ad" }
        }))
        .unwrap();
        let post = syndication_to_post(&doubly, "https://x.com/ad/status/2", u64::MAX).unwrap();
        assert_eq!(post.text, "&lt;");
    }
}
