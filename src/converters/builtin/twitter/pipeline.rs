//! Content-aware acquisition: classify early, fetch only missing visible data, merge by id.
use std::collections::{HashMap, HashSet};

use serenity::all::async_trait;
use tokio::task::{AbortHandle, JoinSet};
use tracing::debug;

use super::plan::{FetchPlan, NestedHint, QuoteFetch, decide};
use super::{
    BuiltinPost, EngineSettings, LinkPreview, extract_tweet_id, merge_community_note, merge_stats,
};

#[derive(Clone)]
pub(super) struct Snapshot {
    pub post: BuiltinPost,
    pub nested_hint: NestedHint,
}

#[derive(Debug, Clone)]
pub(super) enum Request {
    Vx {
        url: String,
        language: Option<String>,
    },
    /// Yalnızca görüntülenme gibi eksik metrikleri tamamlayan FxTwitter isteği.
    Fx {
        url: String,
    },
    Syndication {
        url: String,
    },
}

#[async_trait]
pub(super) trait Fetcher: Clone + Send + Sync + 'static {
    async fn fetch(&self, request: Request) -> Result<Snapshot, String>;
    async fn cached(&self, _request: &Request) -> Option<Snapshot> {
        None
    }
    /// Harici bağlantı kartı (yalnızca medyasız ve alıntısız gönderilerde kullanılır).
    ///
    /// Önizleme, kaynak istekleriyle **paralel** çalışır; sonuç karta sonradan eklenir.
    async fn preview(&self, _url: &str) -> Option<LinkPreview> {
        None
    }
}

/// Tamamlanan işin iki türlü çıktısı: gönderi verisi ya da harici link kartı.
/// Gönderi verisi kutulanır: varyantlar arasındaki boyut farkı büyük.
enum Outcome {
    Snapshot(Box<Snapshot>),
    Preview(Option<LinkPreview>),
}

#[derive(Clone)]
enum Work {
    Rich,
    TranslatedRoot,
    Metadata,
    /// Kaynağın vermediği metrikleri (özellikle görüntülenme) FxTwitter'dan tamamlar.
    Metrics,
    Translation(String),
    Quote(QuoteFetch),
    Preview(String),
}

impl Work {
    fn key(&self) -> String {
        match self {
            Self::Rich => "rich".to_string(),
            Self::TranslatedRoot => "translated-root".to_string(),
            Self::Metadata => "metadata".to_string(),
            Self::Metrics => "metrics".to_string(),
            Self::Translation(url) => format!("translation:{}", identity(url)),
            Self::Quote(QuoteFetch::Direct(url)) => format!("direct:{}", identity(url)),
            Self::Quote(QuoteFetch::Inspect(url)) => format!("inspect:{}", identity(url)),
            Self::Preview(url) => format!("preview:{}", identity(url)),
        }
    }
}

fn identity(url: &str) -> String {
    extract_tweet_id(url).unwrap_or_else(|| url.to_string())
}

fn spawn<F: Fetcher>(
    tasks: &mut JoinSet<(Work, Result<Outcome, String>)>,
    pending: &mut HashMap<String, (Work, AbortHandle)>,
    fetcher: F,
    work: Work,
    request: Request,
) {
    let key = work.key();
    let in_flight = work.clone();
    let handle = tasks.spawn(async move {
        let result = if matches!(work, Work::Quote(QuoteFetch::Inspect(_))) {
            tokio::time::timeout(super::NESTED_QUOTE_TIMEOUT, fetcher.fetch(request))
                .await
                .unwrap_or_else(|_| Err("quote inspection timed out".to_string()))
        } else {
            fetcher.fetch(request).await
        };
        (
            work,
            result.map(|snapshot| Outcome::Snapshot(Box::new(snapshot))),
        )
    });
    pending.insert(key, (in_flight, handle));
}

/// Harici link kartı isteğini **diğer işlerle paralel** başlatır.
///
/// Eskiden önizleme, tüm veri fazı bittikten sonra sırayla çalışıyordu: medyasız bir
/// gönderide kartın süresine 500 ms'ye kadar ekliyordu. Artık ilk veri gelir gelmez
/// başlar ve zenginleştirme bütçesiyle sınırlıdır.
fn spawn_preview<F: Fetcher>(
    tasks: &mut JoinSet<(Work, Result<Outcome, String>)>,
    pending: &mut HashMap<String, (Work, AbortHandle)>,
    fetcher: F,
    url: String,
) {
    let work = Work::Preview(url.clone());
    let key = work.key();
    let in_flight = work.clone();
    let handle = tasks.spawn(async move {
        let preview = fetcher.preview(&url).await;
        (work, Ok(Outcome::Preview(preview)))
    });
    pending.insert(key, (in_flight, handle));
}

fn translation_needed(plan: &FetchPlan, url: &str, checked: &HashSet<String>) -> bool {
    let id = identity(url);
    !checked.contains(&id)
        && plan
            .translation_urls
            .iter()
            .any(|candidate| identity(candidate) == id)
}

/// No fixed merge-grace delay. The base endpoint races a speculative translated root
/// when enabled: English/media posts do not wait for /tr, foreign posts do not pay an
/// extra serial RTT to discover their language. Obsolete read-only work is cancelled.
/// Two deadlines bound the wait: optional enrichment stops once card data exists, while
/// a first snapshot may take longer so a slow source cannot drop the card entirely.
pub(super) async fn acquire<F: Fetcher>(
    fetcher: F,
    url: &str,
    engine: &EngineSettings,
) -> Result<BuiltinPost, String> {
    acquire_with_budgets(
        fetcher,
        url,
        engine,
        super::super::FETCH_ENRICH_BUDGET,
        super::super::FETCH_FIRST_DATA_BUDGET,
    )
    .await
}

/// Tek bütçeli sürüm: hem ilk veri hem zenginleştirme aynı sınıra tabidir (testler).
#[cfg(test)]
async fn acquire_with_budget<F: Fetcher>(
    fetcher: F,
    url: &str,
    engine: &EngineSettings,
    budget: std::time::Duration,
) -> Result<BuiltinPost, String> {
    acquire_with_budgets(fetcher, url, engine, budget, budget).await
}

async fn acquire_with_budgets<F: Fetcher>(
    fetcher: F,
    url: &str,
    engine: &EngineSettings,
    enrich: std::time::Duration,
    first: std::time::Duration,
) -> Result<BuiltinPost, String> {
    let started = tokio::time::Instant::now();
    let enrich_deadline = started + enrich;
    let first_deadline = started + first;
    let mut tasks = JoinSet::new();
    let mut pending = HashMap::new();
    let base_request = Request::Vx {
        url: url.to_string(),
        language: None,
    };
    let mut root = fetcher.cached(&base_request).await;
    let mut checked = HashSet::new();
    if engine.auto_translate
        && root.as_ref().is_none_or(|snapshot| {
            !decide(&snapshot.post, engine, &snapshot.nested_hint)
                .translation_urls
                .is_empty()
        })
        && let Some(snapshot) = fetcher
            .cached(&Request::Vx {
                url: url.to_string(),
                language: Some(engine.translation_target.clone()),
            })
            .await
    {
        checked.insert(identity(&snapshot.post.url));
        if let Some(quote) = snapshot.post.quote.as_deref() {
            checked.insert(identity(&quote.url));
        }
        apply(&mut root, Work::TranslatedRoot, snapshot);
    }
    let mut rich_finished = root.is_some();
    // Görüntülenme sayısını vx/syndication vermez; kartta metrikler açıksa ve önbellekte
    // henüz bir değer yoksa FxTwitter yalnızca bu eksiği tamamlamak için paralel istenir.
    // İsteğe bağlıdır: zenginleştirme bütçesi dolunca kart görüntülenmesiz gönderilir.
    if engine.show_metrics
        && root
            .as_ref()
            .is_none_or(|snapshot| snapshot.post.stats.views.is_none())
    {
        spawn(
            &mut tasks,
            &mut pending,
            fetcher.clone(),
            Work::Metrics,
            Request::Fx {
                url: url.to_string(),
            },
        );
    }
    if !rich_finished {
        spawn(
            &mut tasks,
            &mut pending,
            fetcher.clone(),
            Work::Rich,
            base_request,
        );
        spawn(
            &mut tasks,
            &mut pending,
            fetcher.clone(),
            Work::Metadata,
            Request::Syndication {
                url: url.to_string(),
            },
        );
        if engine.auto_translate {
            spawn(
                &mut tasks,
                &mut pending,
                fetcher.clone(),
                Work::TranslatedRoot,
                Request::Vx {
                    url: url.to_string(),
                    language: Some(engine.translation_target.clone()),
                },
            );
        }
    }
    let mut attempted = HashSet::new();
    let mut failures = Vec::new();

    loop {
        if let Some(snapshot) = root.as_ref() {
            let plan = decide(&snapshot.post, engine, &snapshot.nested_hint);
            debug!(tweet = %identity(&snapshot.post.url), media_ready = plan.media_ready,
                translations = plan.translation_urls.len(), quote = ?plan.quote,
                preview = plan.preview_url.is_some(), "X content-aware fetch plan");
            for translation_url in &plan.translation_urls {
                // The root /language response includes its inline first quote. Do not
                // request their translations again while that same work is in flight.
                let covered_by_root = pending.contains_key("translated-root")
                    && (identity(translation_url) == identity(&snapshot.post.url)
                        || snapshot.post.quote.as_deref().is_some_and(|quote| {
                            identity(translation_url) == identity(&quote.url)
                        }));
                let work = Work::Translation(translation_url.clone());
                if !covered_by_root
                    && !checked.contains(&identity(translation_url))
                    && attempted.insert(work.key())
                {
                    spawn(
                        &mut tasks,
                        &mut pending,
                        fetcher.clone(),
                        work,
                        Request::Vx {
                            url: translation_url.clone(),
                            language: Some(engine.translation_target.clone()),
                        },
                    );
                }
            }
            // Vx provides explicit qrtURL evidence. Do not inspect an arbitrary middle
            // quote from Syndication while that evidence is already on the way.
            let inspecting_translation = matches!(plan.quote.as_ref(), Some(QuoteFetch::Inspect(url))
                if pending.contains_key(&Work::Translation(url.clone()).key())
                    || (pending.contains_key("translated-root") && translation_needed(&plan, url, &checked)));
            if rich_finished
                && !inspecting_translation
                && let Some(quote) = plan.quote.as_ref()
            {
                let work = Work::Quote(quote.clone());
                if attempted.insert(work.key()) {
                    let quote_url = match quote {
                        QuoteFetch::Direct(url) | QuoteFetch::Inspect(url) => url,
                    };
                    spawn(
                        &mut tasks,
                        &mut pending,
                        fetcher.clone(),
                        work,
                        Request::Vx {
                            url: quote_url.clone(),
                            language: None,
                        },
                    );
                }
            }

            // Harici link kartı, kaynak/çeviri/alıntı işleriyle **paralel** istenir: eskiden
            // veri fazı bittikten sonra sırayla çalışıyor ve kartı 500 ms'ye kadar geciktiriyordu.
            if let Some(preview_url) = plan.preview_url.as_ref() {
                let work = Work::Preview(preview_url.clone());
                if attempted.insert(work.key()) {
                    spawn_preview(
                        &mut tasks,
                        &mut pending,
                        fetcher.clone(),
                        preview_url.clone(),
                    );
                }
            }

            let need_topology = !rich_finished && plan.quote.is_some();
            let need_rich = !rich_finished
                && (engine.show_metrics || engine.show_community_note || need_topology);
            let mut needed = false;
            for (work, handle) in pending.values() {
                let relevant = match work {
                    Work::Rich => need_rich,
                    Work::TranslatedRoot => {
                        need_rich
                            || plan.translation_urls.iter().any(|url| {
                                translation_needed(&plan, url, &checked)
                                    && (identity(url) == identity(&snapshot.post.url)
                                        || snapshot.post.quote.as_deref().is_some_and(|quote| {
                                            identity(url) == identity(&quote.url)
                                        }))
                            })
                    }
                    Work::Metadata => false,
                    // Görüntülenme zaten geldiyse ya da kartta metrikler gizliyse bekleme.
                    Work::Metrics => engine.show_metrics && snapshot.post.stats.views.is_none(),
                    Work::Translation(url) => translation_needed(&plan, url, &checked),
                    Work::Quote(quote) => plan.quote.as_ref() == Some(quote),
                    // İsteğe bağlı ama ucuz: bütçe dolana kadar karta eklenmesini bekle.
                    Work::Preview(_) => true,
                };
                if relevant {
                    needed = true;
                } else {
                    handle.abort();
                }
            }
            // Unknown quote topology still needs the rich response even if metrics/notes
            // are hidden, otherwise an existing third quote could be lost.
            if !needed && !need_topology {
                break;
            }
        }

        // Kart verisi hazırsa isteğe bağlı zenginleştirme için tanınan süre geçerlidir;
        // hiç veri yoksa kartın büsbütün düşmemesi için daha uzun ilk-veri sınırı beklenir.
        let deadline = if root.is_some() {
            enrich_deadline
        } else {
            first_deadline
        };
        let next = match tokio::time::timeout_at(deadline, tasks.join_next()).await {
            Ok(next) => next,
            Err(_) => {
                debug!(
                    enrich_ms = enrich.as_millis(),
                    first_ms = first.as_millis(),
                    pending = pending.len(),
                    "X acquisition deadline reached; keeping acquired data"
                );
                break;
            }
        };
        let Some(next) = next else { break };
        let (work, result) = match next {
            Ok(result) => result,
            // Aborted obsolete tasks have no data to publish. A panic also cannot leave
            // us stuck forever because all work shares the total deadline.
            Err(_) => continue,
        };
        pending.remove(&work.key());
        if matches!(work, Work::Rich | Work::TranslatedRoot) {
            rich_finished |= matches!(&result, Ok(Outcome::Snapshot(_)))
                || (!pending.contains_key("rich") && !pending.contains_key("translated-root"));
        }
        if matches!(work, Work::TranslatedRoot) {
            // Even an error must not cause the exact same language URL to be retried
            // immediately as a per-node translation. Other quoted IDs may still succeed.
            checked.insert(identity(url));
            if let Ok(Outcome::Snapshot(snapshot)) = &result {
                checked.insert(identity(&snapshot.post.url));
                if let Some(quote) = snapshot.post.quote.as_deref() {
                    checked.insert(identity(&quote.url));
                }
            }
        }
        if let Work::Translation(url) = &work {
            checked.insert(identity(url));
        }
        match result {
            Ok(Outcome::Snapshot(snapshot)) => apply(&mut root, work, *snapshot),
            Ok(Outcome::Preview(preview)) => {
                if let Work::Preview(preview_url) = &work {
                    apply_preview(&mut root, preview_url, preview);
                }
            }
            Err(reason) => {
                // Önizleme isteğe bağlıdır: başarısızlığı hata listesine yazılmaz.
                if !matches!(work, Work::Preview(_)) {
                    debug!(work = %work.key(), "X acquisition request failed: {reason}");
                    failures.push(reason);
                }
            }
        }
    }
    tasks.abort_all();
    root.map(|snapshot| snapshot.post).ok_or_else(|| {
        if failures.is_empty() {
            "X acquisition timed out".to_string()
        } else {
            failures.join(" | ")
        }
    })
}

fn apply(root: &mut Option<Snapshot>, work: Work, snapshot: Snapshot) {
    match work {
        Work::Rich | Work::TranslatedRoot => {
            let mut rich = snapshot;
            if let Some(earlier) = root.take() {
                if matches!(&earlier.nested_hint, NestedHint::Present(_))
                    && !matches!(&rich.nested_hint, NestedHint::Present(_))
                {
                    rich.nested_hint = earlier.nested_hint;
                }
                merge_post(&mut rich.post, &earlier.post);
            }
            *root = Some(rich);
        }
        Work::Metadata => match root {
            Some(current) => merge_post(&mut current.post, &snapshot.post),
            None => *root = Some(snapshot),
        },
        // Yalnızca eksik metrikleri tamamlar; metin/çeviri/medyayı ezmez. Kart hiç
        // oluşmadıysa FxTwitter'ın tam yanıtı yedek başlangıç verisi olarak kullanılır.
        Work::Metrics => match root {
            Some(current) if identity(&current.post.url) == identity(&snapshot.post.url) => {
                merge_post(&mut current.post, &snapshot.post);
            }
            None => *root = Some(snapshot),
            _ => {}
        },
        Work::Translation(url) => {
            if identity(&url) != identity(&snapshot.post.url) {
                return;
            }
            if let Some(root) = root {
                merge_at(&mut root.post, &url, &snapshot.post, 0);
                if root.nested_hint == NestedHint::Unknown
                    && root
                        .post
                        .quote
                        .as_deref()
                        .is_some_and(|quote| identity(&quote.url) == identity(&url))
                {
                    // A translated standalone middle response also answers the quote
                    // topology question. Never fetch it again just for inspection.
                    root.nested_hint = NestedHint::Absent;
                }
            }
        }
        Work::Quote(QuoteFetch::Direct(url)) => {
            if identity(&url) != identity(&snapshot.post.url) {
                return;
            }
            if let Some(root) = root
                && let Some(quote) = root.post.quote.as_deref_mut()
            {
                if let Some(nested) = quote.quote.as_deref_mut() {
                    merge_post(nested, &snapshot.post);
                } else {
                    quote.quote = Some(Box::new(snapshot.post));
                }
            }
        }
        Work::Quote(QuoteFetch::Inspect(url)) => {
            if let Some(root) = root
                && let Some(quote) = root.post.quote.as_deref_mut()
                && identity(&quote.url) == identity(&url)
                && identity(&snapshot.post.url) == identity(&url)
            {
                merge_post(quote, &snapshot.post);
                // This was a standalone response for the middle tweet, not its truncated
                // inline representation; a missing quote now really means no third tweet.
                root.nested_hint = NestedHint::Absent;
            }
        }
        // Önizleme kendi çıktısıyla (`Outcome::Preview`) uygulanır.
        Work::Preview(_) => {}
    }
}

/// Harici link kartını yalnızca hâlâ geçerliyse karta ekler.
///
/// Veri fazı sürerken metin değişebilir (çeviri) veya gönderiye medya/alıntı eklenebilir;
/// bu durumda eski metinden üretilen önizleme karta konmaz.
fn apply_preview(root: &mut Option<Snapshot>, url: &str, preview: Option<LinkPreview>) {
    let Some(preview) = preview else { return };
    let Some(root) = root else { return };
    if !root.post.media.is_empty() || root.post.quote.is_some() {
        return;
    }
    if super::external_link(&root.post.text).as_deref() != Some(url) {
        return;
    }
    if root
        .post
        .link_preview
        .as_ref()
        .is_some_and(|existing| !existing.is_empty())
    {
        return;
    }
    root.post.link_preview = Some(preview);
}

fn merge_at(target: &mut BuiltinPost, url: &str, fresh: &BuiltinPost, depth: usize) {
    if identity(&target.url) == identity(url) {
        merge_post(target, fresh);
    } else if depth < 2
        && let Some(quote) = target.quote.as_deref_mut()
    {
        merge_at(quote, url, fresh, depth + 1);
    }
}

/// Merge only the same tweet. Rich/translated responses may omit media or an already
/// completed third quote: neither a response race nor an empty translation may erase it.
fn merge_post(target: &mut BuiltinPost, fresh: &BuiltinPost) {
    if identity(&target.url) != identity(&fresh.url) {
        return;
    }
    if fresh.translation.is_some() || target.text.is_empty() {
        target.text.clone_from(&fresh.text);
        target.translation.clone_from(&fresh.translation);
    }
    if target.media.is_empty() {
        target.media.clone_from(&fresh.media);
    }
    if target.language.is_none() {
        target.language.clone_from(&fresh.language);
    }
    if target.created_at.is_none() {
        target.created_at = fresh.created_at;
    }
    target.sensitive |= fresh.sensitive;
    target.source_spoiler |= fresh.source_spoiler;
    target.author.verified |= fresh.author.verified;
    merge_stats(&mut target.stats, &fresh.stats);
    merge_community_note(&mut target.community_note, fresh.community_note.as_deref());
    match (target.quote.as_deref_mut(), fresh.quote.as_deref()) {
        (Some(quote), Some(fresh_quote)) => merge_post(quote, fresh_quote),
        (None, Some(quote)) => target.quote = Some(Box::new(quote.clone())),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::{Author, Media, MediaKind, Stats, Translation};
    use super::*;
    use crate::converters::parser::Platform;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    type Responses = HashMap<String, (Duration, Result<Snapshot, String>)>;
    type Previews = HashMap<String, (Duration, Option<LinkPreview>)>;

    #[derive(Clone, Default)]
    struct FakeFetcher {
        responses: Arc<Responses>,
        calls: Arc<Mutex<Vec<String>>>,
        previews: Arc<Previews>,
        preview_calls: Arc<Mutex<Vec<String>>>,
    }

    fn request_key(request: &Request) -> String {
        match request {
            Request::Vx { url, language } => format!(
                "vx:{}:{}",
                identity(url),
                language.as_deref().unwrap_or("original")
            ),
            Request::Fx { url } => format!("fx:{}", identity(url)),
            Request::Syndication { url } => format!("meta:{}", identity(url)),
        }
    }

    #[async_trait]
    impl Fetcher for FakeFetcher {
        async fn fetch(&self, request: Request) -> Result<Snapshot, String> {
            let key = request_key(&request);
            self.calls.lock().unwrap().push(key.clone());
            let (delay, response) = self
                .responses
                .get(&key)
                .cloned()
                .unwrap_or((Duration::ZERO, Err(format!("unexpected request: {key}"))));
            tokio::time::sleep(delay).await;
            response
        }

        async fn preview(&self, url: &str) -> Option<LinkPreview> {
            self.preview_calls.lock().unwrap().push(url.to_string());
            let (delay, preview) = self
                .previews
                .get(url)
                .cloned()
                .unwrap_or((Duration::ZERO, None));
            tokio::time::sleep(delay).await;
            preview
        }
    }

    impl FakeFetcher {
        fn new(responses: Vec<(&str, u64, Result<Snapshot, String>)>) -> Self {
            Self {
                responses: Arc::new(
                    responses
                        .into_iter()
                        .map(|(key, ms, result)| {
                            (key.to_string(), (Duration::from_millis(ms), result))
                        })
                        .collect(),
                ),
                calls: Arc::new(Mutex::new(Vec::new())),
                previews: Arc::new(Previews::new()),
                preview_calls: Arc::new(Mutex::new(Vec::new())),
            }
        }
        fn count(&self, key: &str) -> usize {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|call| call.as_str() == key)
                .count()
        }
        fn with_previews(mut self, previews: Vec<(&str, u64, Option<LinkPreview>)>) -> Self {
            self.previews = Arc::new(
                previews
                    .into_iter()
                    .map(|(url, ms, preview)| {
                        (url.to_string(), (Duration::from_millis(ms), preview))
                    })
                    .collect(),
            );
            self
        }
        fn preview_count(&self) -> usize {
            self.preview_calls.lock().unwrap().len()
        }
    }

    fn post(id: u64, language: &str) -> BuiltinPost {
        let mut post = BuiltinPost::new(
            Platform::Twitter,
            format!("https://x.com/test/status/{id}"),
            Author::new("Test", "test"),
        );
        post.text = format!("original {id}");
        post.language = Some(language.to_string());
        post
    }

    fn snapshot(post: BuiltinPost, nested_hint: NestedHint) -> Snapshot {
        Snapshot { post, nested_hint }
    }

    fn translated(mut post: BuiltinPost) -> BuiltinPost {
        post.text = format!("translated {}", identity(&post.url));
        post.translation = Some(Translation {
            source: post.language.clone(),
        });
        post
    }

    fn media(kind: MediaKind) -> Media {
        Media {
            kind,
            url: "https://cdn.test/media.mp4".to_string(),
            poster: Some("https://cdn.test/poster.jpg".to_string()),
            width: Some(720),
            height: Some(720),
            duration: Some(10),
        }
    }

    fn engine() -> EngineSettings {
        EngineSettings {
            auto_translate: true,
            // Bu akış testleri çeviri/alıntıya odaklı; metrik zenginleştirmesinin
            // kendi testleri var ve isteğe bağlı ek bir istek üretir.
            show_metrics: false,
            ..EngineSettings::default()
        }
    }

    fn metrics_engine() -> EngineSettings {
        EngineSettings {
            show_metrics: true,
            ..engine()
        }
    }

    #[test]
    fn late_root_response_does_not_erase_known_third_quote_or_translated_media() {
        let mut original = post(1, "en");
        original.quote = Some(Box::new(post(2, "en")));
        let third = post(3, "ja");
        let mut root = Some(snapshot(
            original.clone(),
            NestedHint::Present(third.url.clone()),
        ));
        apply(
            &mut root,
            Work::TranslatedRoot,
            snapshot(original.clone(), NestedHint::Absent),
        );
        assert_eq!(
            root.as_ref().unwrap().nested_hint,
            NestedHint::Present(third.url.clone())
        );
        let mut ready_third = translated(third.clone());
        ready_third.media.push(media(MediaKind::Image));
        apply(
            &mut root,
            Work::Quote(QuoteFetch::Direct(third.url)),
            snapshot(ready_third.clone(), NestedHint::Absent),
        );
        apply(
            &mut root,
            Work::Rich,
            snapshot(original, NestedHint::Unknown),
        );
        assert_eq!(
            root.unwrap().post.quote.unwrap().quote.unwrap().as_ref(),
            &ready_third
        );
    }

    #[tokio::test]
    async fn english_media_does_not_wait_for_unneeded_language_response() {
        let mut root = post(1, "en");
        root.media.push(media(MediaKind::Video));
        let fetcher = FakeFetcher::new(vec![
            ("meta:1", 5, Ok(snapshot(root.clone(), NestedHint::Unknown))),
            (
                "vx:1:original",
                20,
                Ok(snapshot(root.clone(), NestedHint::Absent)),
            ),
            (
                "vx:1:tr",
                500,
                Ok(snapshot(root.clone(), NestedHint::Absent)),
            ),
        ]);
        let result = tokio::time::timeout(
            Duration::from_millis(300),
            acquire(fetcher.clone(), &root.url, &engine()),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(result.media, root.media);
        assert!(result.translation.is_none());
        assert_eq!(
            fetcher.count("vx:1:tr"),
            1,
            "no duplicate translation fetch"
        );
    }

    #[tokio::test]
    async fn japanese_translation_races_without_an_extra_discovery_roundtrip() {
        let mut root = post(1, "ja");
        root.media.push(media(MediaKind::Gif));
        root.sensitive = true;
        root.community_note = Some("keep note".to_string());
        let mut translated_root = translated(root.clone());
        translated_root.media.clear();
        translated_root.community_note = None;
        translated_root.sensitive = false;
        let fetcher = FakeFetcher::new(vec![
            ("meta:1", 5, Ok(snapshot(root.clone(), NestedHint::Unknown))),
            (
                "vx:1:original",
                20,
                Ok(snapshot(root.clone(), NestedHint::Absent)),
            ),
            (
                "vx:1:tr",
                60,
                Ok(snapshot(translated_root, NestedHint::Absent)),
            ),
        ]);
        let result = acquire(fetcher.clone(), &root.url, &engine())
            .await
            .unwrap();
        assert_eq!(result.text, "translated 1");
        assert!(result.translation.is_some());
        assert_eq!(result.media, root.media);
        assert!(result.sensitive);
        assert_eq!(result.community_note, root.community_note);
        assert_eq!(fetcher.count("vx:1:tr"), 1);
    }

    #[tokio::test]
    async fn known_third_quote_is_fetched_directly_without_refetching_the_middle() {
        let mut root = post(1, "en");
        root.quote = Some(Box::new(post(2, "en")));
        let mut third = post(3, "en");
        third.media.push(media(MediaKind::Image));
        let fetcher = FakeFetcher::new(vec![
            ("meta:1", 5, Ok(snapshot(root.clone(), NestedHint::Unknown))),
            (
                "vx:1:original",
                20,
                Ok(snapshot(
                    root.clone(),
                    NestedHint::Present(third.url.clone()),
                )),
            ),
            (
                "vx:1:tr",
                500,
                Ok(snapshot(
                    root.clone(),
                    NestedHint::Present(third.url.clone()),
                )),
            ),
            (
                "vx:3:original",
                20,
                Ok(snapshot(third.clone(), NestedHint::Absent)),
            ),
        ]);
        let result = acquire(fetcher.clone(), &root.url, &engine())
            .await
            .unwrap();
        assert_eq!(result.quote.unwrap().quote.unwrap().media, third.media);
        assert_eq!(fetcher.count("vx:2:original"), 0);
        assert_eq!(fetcher.count("vx:3:original"), 1);
    }

    #[tokio::test]
    async fn translated_root_already_supplies_japanese_first_quote_without_duplicate_work() {
        let mut root = post(1, "en");
        let mut quote = post(2, "ja");
        quote.media.push(media(MediaKind::Video));
        root.quote = Some(Box::new(quote.clone()));
        let mut ready = root.clone();
        ready.quote = Some(Box::new(translated(quote)));
        let fetcher = FakeFetcher::new(vec![
            ("meta:1", 5, Ok(snapshot(root.clone(), NestedHint::Unknown))),
            (
                "vx:1:original",
                20,
                Ok(snapshot(root.clone(), NestedHint::Absent)),
            ),
            ("vx:1:tr", 60, Ok(snapshot(ready, NestedHint::Absent))),
        ]);
        let result = acquire(fetcher.clone(), &root.url, &engine())
            .await
            .unwrap();
        let quote = result.quote.unwrap();
        assert_eq!(quote.text, "translated 2");
        assert!(!quote.media.is_empty());
        assert_eq!(fetcher.count("vx:2:tr"), 0);
        assert_eq!(fetcher.count("vx:2:original"), 0);
    }

    #[tokio::test]
    async fn first_quote_media_does_not_hide_a_known_translated_third_quote() {
        let mut root = post(1, "en");
        let mut quote = post(2, "en");
        quote.media.push(media(MediaKind::Video));
        root.quote = Some(Box::new(quote));
        let third = post(3, "ja");
        let fetcher = FakeFetcher::new(vec![
            ("meta:1", 5, Ok(snapshot(root.clone(), NestedHint::Unknown))),
            (
                "vx:1:original",
                20,
                Ok(snapshot(
                    root.clone(),
                    NestedHint::Present(third.url.clone()),
                )),
            ),
            (
                "vx:1:tr",
                500,
                Ok(snapshot(
                    root.clone(),
                    NestedHint::Present(third.url.clone()),
                )),
            ),
            (
                "vx:3:original",
                20,
                Ok(snapshot(third.clone(), NestedHint::Absent)),
            ),
            (
                "vx:3:tr",
                20,
                Ok(snapshot(translated(third), NestedHint::Absent)),
            ),
        ]);
        let result = acquire(fetcher.clone(), &root.url, &engine())
            .await
            .unwrap();
        let quote = result.quote.unwrap();
        assert_eq!(quote.media[0].kind, MediaKind::Video);
        assert_eq!(quote.quote.unwrap().text, "translated 3");
        assert_eq!(fetcher.count("vx:3:tr"), 1);
    }

    #[tokio::test]
    async fn disabled_translation_and_hidden_quotes_do_not_spawn_enrichment() {
        let mut root = post(1, "ja");
        root.quote = Some(Box::new(post(2, "ja")));
        let fetcher = FakeFetcher::new(vec![
            ("meta:1", 5, Ok(snapshot(root.clone(), NestedHint::Unknown))),
            (
                "vx:1:original",
                20,
                Ok(snapshot(
                    root.clone(),
                    NestedHint::Present(post(3, "ja").url),
                )),
            ),
        ]);
        let settings = EngineSettings {
            auto_translate: false,
            show_quote: false,
            ..engine()
        };
        let result = acquire(fetcher.clone(), &root.url, &settings)
            .await
            .unwrap();
        assert!(result.translation.is_none());
        assert_eq!(fetcher.calls.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn absent_quote_evidence_avoids_probing_a_middle_tweet() {
        let mut root = post(1, "en");
        root.quote = Some(Box::new(post(2, "en")));
        let fetcher = FakeFetcher::new(vec![
            ("meta:1", 5, Ok(snapshot(root.clone(), NestedHint::Unknown))),
            (
                "vx:1:original",
                20,
                Ok(snapshot(root.clone(), NestedHint::Absent)),
            ),
            (
                "vx:1:tr",
                500,
                Ok(snapshot(root.clone(), NestedHint::Absent)),
            ),
        ]);
        let result = acquire(fetcher.clone(), &root.url, &engine())
            .await
            .unwrap();
        assert!(result.quote.unwrap().quote.is_none());
        assert_eq!(fetcher.count("vx:2:original"), 0);
    }

    #[tokio::test]
    async fn fast_source_errors_do_not_cancel_another_valid_source() {
        let root = post(1, "en");
        let fetcher = FakeFetcher::new(vec![
            ("meta:1", 1, Err("metadata unavailable".to_string())),
            (
                "vx:1:original",
                3,
                Err("base temporarily unavailable".to_string()),
            ),
            (
                "vx:1:tr",
                30,
                Ok(snapshot(root.clone(), NestedHint::Absent)),
            ),
        ]);
        let result = acquire(fetcher, &root.url, &engine()).await.unwrap();
        assert_eq!(result.text, root.text);
    }

    #[tokio::test]
    async fn unavailable_translation_keeps_media_and_original_without_blind_retry() {
        let mut root = post(1, "ja");
        root.media.push(media(MediaKind::Gif));
        let fetcher = FakeFetcher::new(vec![
            ("meta:1", 5, Ok(snapshot(root.clone(), NestedHint::Unknown))),
            (
                "vx:1:original",
                20,
                Ok(snapshot(root.clone(), NestedHint::Absent)),
            ),
            ("vx:1:tr", 30, Err("translation unavailable".to_string())),
        ]);
        let result = acquire(fetcher.clone(), &root.url, &engine())
            .await
            .unwrap();
        assert_eq!(result.text, root.text);
        assert_eq!(result.media, root.media);
        assert!(result.translation.is_none());
        assert_eq!(fetcher.count("vx:1:tr"), 1);
    }

    #[tokio::test]
    async fn all_source_failures_are_reported() {
        let fetcher = FakeFetcher::new(vec![
            ("meta:1", 1, Err("metadata failure".to_string())),
            ("vx:1:original", 2, Err("base failure".to_string())),
            ("vx:1:tr", 3, Err("language failure".to_string())),
        ]);
        let reason = acquire(fetcher, &post(1, "en").url, &engine())
            .await
            .unwrap_err();
        assert!(reason.contains("base failure"));
        assert!(reason.contains("metadata failure"));
        assert!(reason.contains("language failure"));
    }

    #[tokio::test]
    async fn a_shared_deadline_does_not_stack_translation_timeouts_or_discard_media() {
        let mut root = post(1, "ja");
        root.media.push(media(MediaKind::Video));
        let fetcher = FakeFetcher::new(vec![
            ("meta:1", 5, Ok(snapshot(root.clone(), NestedHint::Unknown))),
            (
                "vx:1:original",
                20,
                Ok(snapshot(root.clone(), NestedHint::Absent)),
            ),
            (
                "vx:1:tr",
                500,
                Ok(snapshot(translated(root.clone()), NestedHint::Absent)),
            ),
        ]);
        let result = tokio::time::timeout(
            Duration::from_millis(300),
            acquire_with_budget(
                fetcher.clone(),
                &root.url,
                &engine(),
                Duration::from_millis(80),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(result.media, root.media);
        assert_eq!(result.text, root.text);
        assert!(result.translation.is_none());
        assert_eq!(fetcher.count("vx:1:tr"), 1);
    }

    /// Kart verisi hazır olduğunda isteğe bağlı işler **zenginleştirme bütçesinde** kesilir:
    /// yavaş üçüncü seviye alıntı, kartı saniyelerce bekletmez.
    #[tokio::test]
    async fn optional_enrichment_stops_at_the_enrichment_budget() {
        let mut root = post(1, "en");
        root.quote = Some(Box::new(post(2, "en")));
        let fetcher = FakeFetcher::new(vec![
            ("meta:1", 5, Ok(snapshot(root.clone(), NestedHint::Unknown))),
            (
                "vx:1:original",
                20,
                Ok(snapshot(root.clone(), NestedHint::Unknown)),
            ),
            (
                "vx:1:tr",
                500,
                Ok(snapshot(root.clone(), NestedHint::Unknown)),
            ),
            (
                "vx:2:original",
                900,
                Ok(snapshot(post(2, "en"), NestedHint::Absent)),
            ),
        ]);
        let started = Instant::now();
        let result = acquire_with_budgets(
            fetcher.clone(),
            &root.url,
            &engine(),
            Duration::from_millis(100),
            Duration::from_millis(1_000),
        )
        .await
        .unwrap();
        assert!(
            started.elapsed() < Duration::from_millis(400),
            "bütçe aşıldı: {:?}",
            started.elapsed()
        );
        assert_eq!(result.text, root.text);
        assert!(
            result.quote.as_deref().unwrap().quote.is_none(),
            "bütçe dolduktan sonra alıntı derinliği tamamlanmamalı"
        );
    }

    /// **Logdaki ana neden:** metrik/topluluk notu açıkken kart, hızlı syndication verisi
    /// hazır olmasına rağmen yavaş vx yanıtına bağlanıyordu. Artık kart bütçede gönderilir.
    #[tokio::test]
    async fn slow_rich_source_does_not_pin_the_card_when_metadata_is_ready() {
        let root = post(1, "ja");
        let mut rich = root.clone();
        rich.stats.views = Some(1_234);
        rich.community_note = Some("not".to_string());
        let fetcher = FakeFetcher::new(vec![
            ("meta:1", 20, Ok(snapshot(root.clone(), NestedHint::Absent))),
            (
                "vx:1:original",
                1_900,
                Ok(snapshot(rich.clone(), NestedHint::Absent)),
            ),
            (
                "vx:1:tr",
                1_900,
                Ok(snapshot(translated(rich), NestedHint::Absent)),
            ),
        ]);
        let started = Instant::now();
        let result = acquire(fetcher.clone(), &root.url, &engine())
            .await
            .unwrap();
        assert!(
            started.elapsed() < Duration::from_millis(800),
            "kart yavaş kaynağa bağlı kaldı: {:?}",
            started.elapsed()
        );
        assert_eq!(result.text, root.text, "çeviri geç geldi, orijinal kalmalı");
        assert_eq!(result.stats.views, None, "geç gelen metrik karta girmemeli");
    }

    /// vx/syndication görüntülenme sayısını vermez. Kartta metrikler açıkken eksik değer
    /// FxTwitter'dan paralel tamamlanır ve mevcut metrikler korunur.
    #[tokio::test]
    async fn missing_views_are_completed_from_fxtwitter() {
        let root = post(1, "en");
        let mut source = root.clone();
        source.stats = Stats {
            replies: 3,
            reposts: 4,
            likes: 5,
            quotes: 1,
            views: None,
        };
        let mut metrics = root.clone();
        metrics.stats = Stats {
            views: Some(1_240_000),
            ..source.stats
        };
        let fetcher = FakeFetcher::new(vec![
            ("meta:1", 5, Ok(snapshot(root.clone(), NestedHint::Absent))),
            (
                "vx:1:original",
                20,
                Ok(snapshot(source, NestedHint::Absent)),
            ),
            ("fx:1", 60, Ok(snapshot(metrics, NestedHint::Unknown))),
        ]);

        let result = acquire(fetcher.clone(), &root.url, &metrics_engine())
            .await
            .unwrap();

        assert_eq!(result.stats.views, Some(1_240_000));
        assert_eq!(result.stats.likes, 5, "mevcut metrikler korunmalı");
        assert_eq!(fetcher.count("fx:1"), 1);
    }

    /// Metrikler kartta gizliyse görüntülenme için ek istek atılmaz.
    #[tokio::test]
    async fn metrics_are_not_requested_when_the_card_hides_them() {
        let root = post(1, "en");
        let fetcher = FakeFetcher::new(vec![
            ("meta:1", 5, Ok(snapshot(root.clone(), NestedHint::Absent))),
            (
                "vx:1:original",
                20,
                Ok(snapshot(root.clone(), NestedHint::Absent)),
            ),
        ]);

        let result = acquire(fetcher.clone(), &root.url, &engine())
            .await
            .unwrap();

        assert_eq!(fetcher.count("fx:1"), 0);
        assert_eq!(result.stats.views, None);
    }

    /// Görüntülenme isteği isteğe bağlıdır: zenginleştirme bütçesini aşan yanıt kartı bekletmez.
    #[tokio::test]
    async fn a_slow_metrics_source_does_not_delay_the_card() {
        let root = post(1, "en");
        let mut metrics = root.clone();
        metrics.stats.views = Some(9_000);
        let fetcher = FakeFetcher::new(vec![
            ("meta:1", 5, Ok(snapshot(root.clone(), NestedHint::Absent))),
            (
                "vx:1:original",
                20,
                Ok(snapshot(root.clone(), NestedHint::Absent)),
            ),
            ("fx:1", 600, Ok(snapshot(metrics, NestedHint::Unknown))),
        ]);

        let started = Instant::now();
        let result = acquire_with_budgets(
            fetcher,
            &root.url,
            &metrics_engine(),
            Duration::from_millis(120),
            Duration::from_millis(1_000),
        )
        .await
        .unwrap();

        assert!(
            started.elapsed() < Duration::from_millis(400),
            "görüntülenme isteği kartı bekletti: {:?}",
            started.elapsed()
        );
        assert_eq!(
            result.stats.views, None,
            "bütçe dolduktan sonra geç gelen görüntülenme karta girmemeli"
        );
    }

    /// Diğer kaynaklar düşerse FxTwitter'ın tam yanıtı kartı tek başına kurabilir.
    #[tokio::test]
    async fn metrics_response_can_create_the_card_when_other_sources_fail() {
        let root = post(1, "en");
        let mut metrics = root.clone();
        metrics.stats.views = Some(9_000);
        let fetcher = FakeFetcher::new(vec![(
            "fx:1",
            40,
            Ok(snapshot(metrics, NestedHint::Unknown)),
        )]);

        let result = acquire(fetcher, &root.url, &metrics_engine())
            .await
            .unwrap();

        assert_eq!(result.url, root.url);
        assert_eq!(result.stats.views, Some(9_000));
    }

    /// Hiç veri yokken zenginleştirme bütçesi kartı düşürmez: ilk veri kendi (daha uzun)
    /// sınırına kadar beklenir, çünkü bekleyen işler kartı üretebilecek tek şeydir.
    #[tokio::test]
    async fn a_missing_first_snapshot_waits_past_the_enrichment_budget() {
        let root = post(1, "en");
        let fetcher = FakeFetcher::new(vec![(
            "vx:1:original",
            200,
            Ok(snapshot(root.clone(), NestedHint::Absent)),
        )]);
        let started = Instant::now();
        let result = acquire_with_budgets(
            fetcher,
            &root.url,
            &engine(),
            Duration::from_millis(50),
            Duration::from_millis(1_000),
        )
        .await
        .unwrap();
        assert!(
            started.elapsed() >= Duration::from_millis(150),
            "ilk veri zenginleştirme bütçesinde kesilmemeli: {:?}",
            started.elapsed()
        );
        assert_eq!(result.text, root.text);
    }

    /// Harici link kartı kaynak istekleriyle **paralel** çalışır: kart iki sürenin toplamı
    /// kadar değil, en yavaşı kadar bekler.
    #[tokio::test]
    async fn link_preview_runs_in_parallel_with_source_fetch() {
        let mut root = post(1, "en");
        root.text = "bak https://example.com/article".to_string();
        let preview = LinkPreview {
            url: "https://example.com/article".to_string(),
            title: Some("Başlık".to_string()),
            ..LinkPreview::default()
        };
        let fetcher = FakeFetcher::new(vec![
            ("meta:1", 5, Ok(snapshot(root.clone(), NestedHint::Absent))),
            (
                "vx:1:original",
                200,
                Ok(snapshot(root.clone(), NestedHint::Absent)),
            ),
        ])
        .with_previews(vec![(
            "https://example.com/article",
            200,
            Some(preview.clone()),
        )]);
        let started = Instant::now();
        let result = acquire_with_budgets(
            fetcher.clone(),
            &root.url,
            &engine(),
            Duration::from_millis(400),
            Duration::from_millis(1_000),
        )
        .await
        .unwrap();
        assert_eq!(result.link_preview, Some(preview));
        assert_eq!(fetcher.preview_count(), 1);
        assert!(
            started.elapsed() < Duration::from_millis(320),
            "önizleme sırayla çalışmış olabilir: {:?}",
            started.elapsed()
        );
    }

    /// Medya veya alıntı taşıyan kartta harici link önizlemesi istenmez (X de göstermez).
    #[tokio::test]
    async fn media_posts_do_not_request_a_link_preview() {
        let mut root = post(1, "en");
        root.text = "bak https://example.com/article".to_string();
        root.media.push(media(MediaKind::Video));
        let fetcher = FakeFetcher::new(vec![
            ("meta:1", 5, Ok(snapshot(root.clone(), NestedHint::Absent))),
            (
                "vx:1:original",
                20,
                Ok(snapshot(root.clone(), NestedHint::Absent)),
            ),
        ]);
        let result = acquire(fetcher.clone(), &root.url, &engine())
            .await
            .unwrap();
        assert!(result.link_preview.is_none());
        assert_eq!(fetcher.preview_count(), 0);
    }

    #[test]
    fn stale_or_redundant_link_previews_are_rejected() {
        let mut root = Some(snapshot(post(1, "en"), NestedHint::Absent));
        root.as_mut().unwrap().post.text = "yeni https://example.com/b".to_string();
        let preview = LinkPreview {
            url: "https://example.com/a".to_string(),
            title: Some("Eski".to_string()),
            ..LinkPreview::default()
        };

        // Metin değiştiyse eski adresin önizlemesi karta konmaz.
        apply_preview(&mut root, "https://example.com/a", Some(preview.clone()));
        assert!(root.as_ref().unwrap().post.link_preview.is_none());

        // Medya eklendiyse de önizleme gösterilmez.
        let mut rooted = Some(snapshot(post(1, "en"), NestedHint::Absent));
        rooted.as_mut().unwrap().post.text = "https://example.com/a".to_string();
        rooted
            .as_mut()
            .unwrap()
            .post
            .media
            .push(media(MediaKind::Image));
        apply_preview(&mut rooted, "https://example.com/a", Some(preview));
        assert!(rooted.as_ref().unwrap().post.link_preview.is_none());
    }

    #[tokio::test]
    async fn a_translated_middle_response_also_completes_unknown_quote_topology() {
        let mut root = post(1, "en");
        root.quote = Some(Box::new(post(2, "ja")));
        let mut middle = translated(post(2, "ja"));
        middle.quote = Some(Box::new(post(3, "en")));
        let fetcher = FakeFetcher::new(vec![
            ("meta:1", 1, Err("metadata unavailable".to_string())),
            (
                "vx:1:original",
                5,
                Ok(snapshot(root.clone(), NestedHint::Unknown)),
            ),
            (
                "vx:1:tr",
                10,
                Err("root translation unavailable".to_string()),
            ),
            ("vx:2:tr", 20, Ok(snapshot(middle, NestedHint::Absent))),
        ]);
        let result = acquire(fetcher.clone(), &root.url, &engine())
            .await
            .unwrap();
        assert_eq!(result.quote.as_ref().unwrap().text, "translated 2");
        assert!(result.quote.unwrap().quote.is_some());
        assert_eq!(fetcher.count("vx:2:original"), 0);
    }

    #[derive(Clone)]
    struct CachedFetcher {
        inner: FakeFetcher,
        cache: Arc<super::super::source_cache::SourceCache<Snapshot>>,
    }

    #[async_trait]
    impl Fetcher for CachedFetcher {
        async fn fetch(&self, request: Request) -> Result<Snapshot, String> {
            self.cache
                .get_or_fetch(request_key(&request), || self.inner.fetch(request))
                .await
                .map(|snapshot| (*snapshot).clone())
        }
        async fn cached(&self, request: &Request) -> Option<Snapshot> {
            self.cache
                .get(&request_key(request))
                .await
                .map(|snapshot| (*snapshot).clone())
        }
    }

    #[tokio::test]
    async fn cached_source_data_makes_the_next_triple_quote_fetch_network_free() {
        let mut root = post(1, "en");
        root.quote = Some(Box::new(post(2, "en")));
        let mut third = post(3, "en");
        third.media.push(media(MediaKind::Image));
        let inner = FakeFetcher::new(vec![
            ("meta:1", 5, Ok(snapshot(root.clone(), NestedHint::Unknown))),
            (
                "vx:1:original",
                20,
                Ok(snapshot(
                    root.clone(),
                    NestedHint::Present(third.url.clone()),
                )),
            ),
            (
                "vx:1:tr",
                500,
                Ok(snapshot(
                    root.clone(),
                    NestedHint::Present(third.url.clone()),
                )),
            ),
            ("vx:3:original", 20, Ok(snapshot(third, NestedHint::Absent))),
        ]);
        let fetcher = CachedFetcher {
            inner: inner.clone(),
            cache: Arc::new(super::super::source_cache::SourceCache::new(
                Duration::from_secs(600),
                16,
            )),
        };
        let first = acquire(fetcher.clone(), &root.url, &engine())
            .await
            .unwrap();
        let calls = inner.calls.lock().unwrap().len();
        let second = acquire(fetcher, &root.url, &engine()).await.unwrap();
        assert_eq!(second, first);
        assert_eq!(
            inner.calls.lock().unwrap().len(),
            calls,
            "warm cache must not start speculative HTTP requests"
        );
    }

    #[tokio::test]
    async fn two_different_parent_tweets_reuse_the_same_third_tweet_source() {
        let mut root = post(1, "en");
        root.quote = Some(Box::new(post(2, "en")));
        let mut other = root.clone();
        other.url = post(4, "en").url;
        let third = post(3, "en");
        let inner = FakeFetcher::new(vec![
            ("meta:1", 1, Ok(snapshot(root.clone(), NestedHint::Unknown))),
            (
                "vx:1:original",
                5,
                Ok(snapshot(
                    root.clone(),
                    NestedHint::Present(third.url.clone()),
                )),
            ),
            (
                "meta:4",
                1,
                Ok(snapshot(other.clone(), NestedHint::Unknown)),
            ),
            (
                "vx:4:original",
                5,
                Ok(snapshot(
                    other.clone(),
                    NestedHint::Present(third.url.clone()),
                )),
            ),
            ("vx:3:original", 5, Ok(snapshot(third, NestedHint::Absent))),
        ]);
        let fetcher = CachedFetcher {
            inner: inner.clone(),
            cache: Arc::new(super::super::source_cache::SourceCache::new(
                Duration::from_secs(600),
                16,
            )),
        };
        let settings = EngineSettings {
            auto_translate: false,
            ..engine()
        };
        let first = acquire(fetcher.clone(), &root.url, &settings)
            .await
            .unwrap();
        let second = acquire(fetcher, &other.url, &settings).await.unwrap();
        assert_ne!(first.url, second.url);
        assert_eq!(first.quote, second.quote);
        assert_eq!(
            inner.count("vx:3:original"),
            1,
            "not just a whole-parent cache hit"
        );
    }

    // Reference for facfe5a's Vx path: language request -> 125 ms hedge -> up to
    // 300 ms merge grace, then a serial middle-tweet request for any bare quote.
    // Only used with successful scripted endpoints in the comparison below.
    async fn legacy_reference(
        fetcher: &FakeFetcher,
        root_url: &str,
        settings: &EngineSettings,
    ) -> BuiltinPost {
        let language = settings
            .auto_translate
            .then(|| settings.translation_target.clone());
        let primary = fetcher.fetch(Request::Vx {
            url: root_url.to_string(),
            language: language.clone(),
        });
        tokio::pin!(primary);
        let mut root = match tokio::time::timeout(Duration::from_millis(125), &mut primary).await {
            Ok(result) => result.unwrap().post,
            Err(_) => {
                let metadata = fetcher.fetch(Request::Syndication {
                    url: root_url.to_string(),
                });
                tokio::pin!(metadata);
                tokio::select! {
                    biased;
                    result = &mut primary => result.unwrap().post,
                    result = &mut metadata => {
                        match tokio::time::timeout(Duration::from_millis(300), &mut primary).await {
                            Ok(result) => result.unwrap().post,
                            Err(_) => result.unwrap().post,
                        }
                    }
                }
            }
        };
        if settings.show_quote && super::super::should_populate_nested_quote(&root) {
            let middle = root.quote.as_deref_mut().unwrap();
            if let Ok(Ok(result)) = tokio::time::timeout(
                super::super::NESTED_QUOTE_TIMEOUT,
                fetcher.fetch(Request::Vx {
                    url: middle.url.clone(),
                    language,
                }),
            )
            .await
            {
                *middle = result.post;
            }
        }
        root
    }

    #[tokio::test]
    #[ignore = "manual simulated endpoint comparison; --ignored --nocapture"]
    async fn scripted_latency_comparison() {
        for scenario in [
            "english-video",
            "japanese-video",
            "triple-image",
            "translated-triple",
        ] {
            let foreign = matches!(scenario, "japanese-video" | "translated-triple");
            let triple = matches!(scenario, "triple-image" | "translated-triple");
            let mut root = post(1, if foreign { "ja" } else { "en" });
            let mut middle = post(2, "en");
            let mut third = post(3, if foreign { "ja" } else { "en" });
            third.media.push(media(MediaKind::Image));
            let hint = if triple {
                root.quote = Some(Box::new(middle.clone()));
                middle.quote = Some(Box::new(if foreign {
                    translated(third.clone())
                } else {
                    third.clone()
                }));
                NestedHint::Present(third.url.clone())
            } else {
                root.media.push(media(MediaKind::Video));
                NestedHint::Absent
            };
            let mut before = Vec::new();
            let mut after = Vec::new();
            for sample in 0..3 {
                let routes = vec![
                    (
                        "meta:1",
                        40,
                        Ok(snapshot(root.clone(), NestedHint::Unknown)),
                    ),
                    (
                        "vx:1:original",
                        60,
                        Ok(snapshot(root.clone(), hint.clone())),
                    ),
                    (
                        "vx:1:tr",
                        240,
                        Ok(snapshot(
                            if foreign {
                                translated(root.clone())
                            } else {
                                root.clone()
                            },
                            hint.clone(),
                        )),
                    ),
                    (
                        "vx:2:tr",
                        240,
                        Ok(snapshot(middle.clone(), NestedHint::Absent)),
                    ),
                    (
                        "vx:3:original",
                        60,
                        Ok(snapshot(third.clone(), NestedHint::Absent)),
                    ),
                    (
                        "vx:3:tr",
                        240,
                        Ok(snapshot(translated(third.clone()), NestedHint::Absent)),
                    ),
                ];
                let baseline = FakeFetcher::new(routes.clone());
                let candidate = FakeFetcher::new(routes);
                let started = Instant::now();
                let old = legacy_reference(&baseline, &root.url, &engine()).await;
                before.push(started.elapsed().as_millis());
                let started = Instant::now();
                let new = acquire(candidate.clone(), &root.url, &engine())
                    .await
                    .unwrap();
                after.push(started.elapsed().as_millis());
                assert_eq!(new, old, "same visible content is required: {scenario}");
                println!(
                    "scripted scenario={scenario} sample={sample} before_ms={} after_ms={} middle_requests={}",
                    before[sample],
                    after[sample],
                    candidate.count("vx:2:tr") + candidate.count("vx:2:original")
                );
            }
            before.sort_unstable();
            after.sort_unstable();
            println!(
                "scripted scenario={scenario} median_before_ms={} median_after_ms={}",
                before[1], after[1]
            );
            if scenario != "japanese-video" {
                assert!(after[1] < before[1]);
            }
        }
    }
}
