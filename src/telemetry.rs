//! Bounded, process-local telemetry for debugging.
//!
//! Tasarım ilkeleri:
//!
//! - **Sıfır yeni bağımlılık:** sabit kovalı histogramlar ve atomik sayaçlar yeterli;
//!   OpenMetrics/Prometheus dışa aktarıcısı veya global recorder katmanı yok.
//! - **Sınırlı etiket kümesi:** rota/sınıf adları sabit sayıda; Discord kanal/webhook
//!   kimlikleri metrik etiketi olarak tutulmaz (kardinalite patlaması olmaz).
//! - **Sıcak yolda kilit yok:** her gözlem birkaç atomik işlemdir. JSON anlık görüntüsü
//!   yalnızca MCP/WebUI istediğinde üretilir.
//! - **Süreç ömrü:** sayaçlar yeniden başlatmada sıfırlanır; kalıcı veri değildir.

use std::sync::{
    LazyLock, OnceLock,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

const RELAXED: Ordering = Ordering::Relaxed;

/// Mikrosaniye cinsinden üst sınırlar. Son kova `+Inf`.
const BUCKETS_US: [u64; 12] = [
    1_000,
    2_500,
    5_000,
    10_000,
    25_000,
    50_000,
    100_000,
    250_000,
    500_000,
    1_000_000,
    2_500_000,
    u64::MAX,
];
const BUCKET_COUNT: usize = BUCKETS_US.len();

// =====================================================================================
// MEVCUT SAYAÇLAR (geriye dönük uyumlu)
// =====================================================================================

pub static DISCORD_READY: AtomicBool = AtomicBool::new(false);
pub static CACHE_HITS: AtomicU64 = AtomicU64::new(0);
pub static OVERLOADS: AtomicU64 = AtomicU64::new(0);
pub static CONVERSION_QUEUE_ENQUEUED: AtomicU64 = AtomicU64::new(0);
pub static CONVERSION_QUEUE_COMPLETED: AtomicU64 = AtomicU64::new(0);
pub static CONVERSION_QUEUE_SATURATED: AtomicU64 = AtomicU64::new(0);
pub static CONVERSION_QUEUE_DEPTH: AtomicU64 = AtomicU64::new(0);

const LIMITS: [u64; 8] = [100, 250, 500, 1_000, 2_500, 5_000, 10_000, u64::MAX];
const PLATFORMS: [&str; 6] = [
    "twitter",
    "tiktok",
    "instagram",
    "reddit",
    "bluesky",
    "spotify",
];

struct Counters {
    ok: AtomicU64,
    failed: AtomicU64,
    buckets: [AtomicU64; 8],
}

static COUNTERS: LazyLock<[Counters; 6]> = LazyLock::new(|| {
    std::array::from_fn(|_| Counters {
        ok: AtomicU64::new(0),
        failed: AtomicU64::new(0),
        buckets: std::array::from_fn(|_| AtomicU64::new(0)),
    })
});

pub fn record(platform: &str, ok: bool, elapsed: Duration) {
    let Some(index) = PLATFORMS.iter().position(|p| *p == platform) else {
        return;
    };
    let counters = &COUNTERS[index];
    if ok { &counters.ok } else { &counters.failed }.fetch_add(1, RELAXED);
    let millis = elapsed.as_millis().min(u64::MAX as u128) as u64;
    for (limit, bucket) in LIMITS.iter().zip(&counters.buckets) {
        if millis <= *limit {
            bucket.fetch_add(1, RELAXED);
        }
    }
}

// =====================================================================================
// HİSTOGRAM
// =====================================================================================

/// Sabit kovalı, atomik histogram. `p50/p90/p95/p99` yaşam boyu kovalardan **yaklaşık**;
/// `recent_*` alanları son 64 gözlemin **tam** yüzdeleridir. `buckets` kesin dağılımı verir.
pub struct Histogram {
    buckets: [AtomicU64; BUCKET_COUNT],
    count: AtomicU64,
    sum_us: AtomicU64,
    min_us: AtomicU64,
    max_us: AtomicU64,
    recent: [AtomicU64; RECENT_CAPACITY],
    recent_cursor: AtomicU64,
}

const RECENT_CAPACITY: usize = 64;

impl Histogram {
    fn new() -> Self {
        Self {
            buckets: std::array::from_fn(|_| AtomicU64::new(0)),
            count: AtomicU64::new(0),
            sum_us: AtomicU64::new(0),
            min_us: AtomicU64::new(u64::MAX),
            max_us: AtomicU64::new(0),
            recent: std::array::from_fn(|_| AtomicU64::new(0)),
            recent_cursor: AtomicU64::new(0),
        }
    }

    pub fn record(&self, elapsed: Duration) {
        let micros = elapsed.as_micros().min(u64::MAX as u128) as u64;
        self.count.fetch_add(1, RELAXED);
        self.sum_us.fetch_add(micros, RELAXED);
        self.min_us.fetch_min(micros, RELAXED);
        self.max_us.fetch_max(micros, RELAXED);
        let cursor = self.recent_cursor.fetch_add(1, RELAXED) as usize;
        self.recent[cursor % RECENT_CAPACITY].store(micros, RELAXED);
        let index = BUCKETS_US
            .iter()
            .position(|upper| micros <= *upper)
            .unwrap_or(BUCKET_COUNT - 1);
        self.buckets[index].fetch_add(1, RELAXED);
    }

    /// Son `RECENT_CAPACITY` gözlemin sıralı mikrosaniye değerleri.
    fn recent_samples(&self) -> Vec<u64> {
        let cursor = self.recent_cursor.load(RELAXED) as usize;
        let count = self.count.load(RELAXED) as usize;
        let valid = count.min(RECENT_CAPACITY);
        let start = cursor.saturating_sub(valid);
        let mut samples: Vec<u64> = (0..valid)
            .map(|offset| self.recent[(start + offset) % RECENT_CAPACITY].load(RELAXED))
            .collect();
        samples.sort_unstable();
        samples
    }

    #[cfg(test)]
    pub fn count(&self) -> u64 {
        self.count.load(RELAXED)
    }

    pub fn snapshot(&self) -> Value {
        let mut buckets = Vec::with_capacity(BUCKET_COUNT);
        let mut loaded = [0u64; BUCKET_COUNT];
        for (index, bucket) in self.buckets.iter().enumerate() {
            loaded[index] = bucket.load(RELAXED);
            buckets.push(json!({
                "le_ms": if BUCKETS_US[index] == u64::MAX {
                    Value::String("inf".into())
                } else {
                    json!(BUCKETS_US[index].div_ceil(1_000))
                },
                "count": loaded[index]
            }));
        }
        let count = self.count.load(RELAXED);
        let recent = self.recent_samples();
        if count == 0 {
            return json!({
                "count": 0,
                "sum_ms": 0.0,
                "min_ms": Value::Null,
                "p50_ms": Value::Null,
                "p90_ms": Value::Null,
                "p95_ms": Value::Null,
                "p99_ms": Value::Null,
                "max_ms": Value::Null,
                "recent_count": 0,
                "recent_p50_ms": Value::Null,
                "recent_p95_ms": Value::Null,
                "recent_max_ms": Value::Null,
                "buckets": buckets
            });
        }
        let max_us = self.max_us.load(RELAXED);
        let percentile = |fraction: f64| percentile_ms(&loaded, count, fraction, max_us);
        json!({
            "count": count,
            "sum_ms": self.sum_us.load(RELAXED) as f64 / 1_000.0,
            "min_ms": self.min_us.load(RELAXED).div_ceil(1_000),
            "p50_ms": percentile(0.50),
            "p90_ms": percentile(0.90),
            "p95_ms": percentile(0.95),
            "p99_ms": percentile(0.99),
            "max_ms": max_us.div_ceil(1_000),
            "recent_count": recent.len(),
            "recent_p50_ms": exact_percentile_ms(&recent, 0.50),
            "recent_p95_ms": exact_percentile_ms(&recent, 0.95),
            "recent_max_ms": recent.last().map_or(Value::Null, |value| json!(value.div_ceil(1_000))),
            "buckets": buckets
        })
    }
}

fn exact_percentile_ms(samples: &[u64], fraction: f64) -> Value {
    if samples.is_empty() {
        return Value::Null;
    }
    let index = (((samples.len() - 1) as f64) * fraction).round() as usize;
    json!(samples[index].div_ceil(1_000))
}

fn percentile_ms(counts: &[u64; BUCKET_COUNT], count: u64, fraction: f64, max_us: u64) -> u64 {
    let target = ((count as f64) * fraction).ceil().max(1.0) as u64;
    let mut cumulative = 0u64;
    for (index, observed) in counts.iter().enumerate() {
        cumulative = cumulative.saturating_add(*observed);
        if cumulative >= target {
            return if BUCKETS_US[index] == u64::MAX {
                max_us.div_ceil(1_000)
            } else {
                BUCKETS_US[index].div_ceil(1_000)
            };
        }
    }
    max_us.div_ceil(1_000)
}

// =====================================================================================
// DÖNÜŞÜM AŞAMALARI
// =====================================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum Phase {
    QueueWait = 0,
    Fetch = 1,
    Media = 2,
    Render = 3,
    WebhookWait = 4,
    WebhookResolve = 5,
    WebhookSend = 6,
    Send = 7,
    Delete = 8,
    Reaction = 9,
    Total = 10,
    Preview = 11,
}

impl Phase {
    pub const ALL: [Phase; 12] = [
        Phase::QueueWait,
        Phase::Fetch,
        Phase::Media,
        Phase::Render,
        Phase::WebhookWait,
        Phase::WebhookResolve,
        Phase::WebhookSend,
        Phase::Send,
        Phase::Delete,
        Phase::Reaction,
        Phase::Total,
        Phase::Preview,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Phase::QueueWait => "queue_wait",
            Phase::Fetch => "fetch",
            Phase::Media => "media",
            Phase::Render => "render",
            Phase::WebhookWait => "webhook_wait",
            Phase::WebhookResolve => "webhook_resolve",
            Phase::WebhookSend => "webhook_send",
            Phase::Send => "send",
            Phase::Delete => "delete",
            Phase::Reaction => "reaction",
            Phase::Total => "total",
            Phase::Preview => "preview_total",
        }
    }
}

static PHASES: LazyLock<[Histogram; 12]> =
    LazyLock::new(|| std::array::from_fn(|_| Histogram::new()));

pub fn record_phase(phase: Phase, elapsed: Duration) {
    PHASES[phase as usize].record(elapsed);
}

/// Kapsam dışına çıkıldığında (erken `return`/`?`/panik) süreyi kaydeden koruma.
pub struct PhaseTimer {
    phase: Phase,
    started: Instant,
}

impl PhaseTimer {
    pub fn new(phase: Phase) -> Self {
        Self {
            phase,
            started: Instant::now(),
        }
    }
}

impl Drop for PhaseTimer {
    fn drop(&mut self) {
        record_phase(self.phase, self.started.elapsed());
    }
}

// =====================================================================================
// DISCORD REST
// =====================================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum RouteClass {
    ChannelMessage = 0,
    WebhookExecute = 1,
    WebhookList = 2,
    WebhookCreate = 3,
    InteractionEdit = 4,
    Other = 5,
}

impl RouteClass {
    pub const ALL: [RouteClass; 6] = [
        RouteClass::ChannelMessage,
        RouteClass::WebhookExecute,
        RouteClass::WebhookList,
        RouteClass::WebhookCreate,
        RouteClass::InteractionEdit,
        RouteClass::Other,
    ];

    pub fn classify(route: &str) -> Self {
        if route.starts_with("channel:") {
            RouteClass::ChannelMessage
        } else if route.starts_with("hook_exec:") || route.starts_with("hook:") {
            RouteClass::WebhookExecute
        } else if route.starts_with("hook_list:") {
            RouteClass::WebhookList
        } else if route.starts_with("hook_create:") {
            RouteClass::WebhookCreate
        } else if route.starts_with("interaction:") {
            RouteClass::InteractionEdit
        } else {
            RouteClass::Other
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            RouteClass::ChannelMessage => "channel_message",
            RouteClass::WebhookExecute => "webhook_execute",
            RouteClass::WebhookList => "webhook_list",
            RouteClass::WebhookCreate => "webhook_create",
            RouteClass::InteractionEdit => "interaction_edit",
            RouteClass::Other => "other",
        }
    }
}

/// Yaygın Discord durum kodları; diğerleri `2xx`/`other_4xx`/`other_5xx` altında toplanır.
const STATUS_CODES: [u16; 10] = [400, 401, 403, 404, 408, 429, 500, 502, 503, 504];
const STATUS_SLOT_COUNT: usize = STATUS_CODES.len() + 3;
const OTHER_2XX: usize = STATUS_CODES.len();
const OTHER_4XX: usize = STATUS_CODES.len() + 1;
const OTHER_5XX: usize = STATUS_CODES.len() + 2;

struct RouteMetrics {
    requests: AtomicU64,
    attempts: AtomicU64,
    success: AtomicU64,
    rejected: AtomicU64,
    server_error: AtomicU64,
    rate_limited: AtomicU64,
    retries: AtomicU64,
    ambiguous: AtomicU64,
    timeouts: AtomicU64,
    slow: AtomicU64,
    invalid: AtomicU64,
    statuses: [AtomicU64; STATUS_SLOT_COUNT],
    attempt_latency: Histogram,
    total_latency: Histogram,
    wait_latency: Histogram,
}

impl RouteMetrics {
    fn new() -> Self {
        Self {
            requests: AtomicU64::new(0),
            attempts: AtomicU64::new(0),
            success: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
            server_error: AtomicU64::new(0),
            rate_limited: AtomicU64::new(0),
            retries: AtomicU64::new(0),
            ambiguous: AtomicU64::new(0),
            timeouts: AtomicU64::new(0),
            slow: AtomicU64::new(0),
            invalid: AtomicU64::new(0),
            statuses: std::array::from_fn(|_| AtomicU64::new(0)),
            attempt_latency: Histogram::new(),
            total_latency: Histogram::new(),
            wait_latency: Histogram::new(),
        }
    }
}

static ROUTES: LazyLock<[RouteMetrics; 6]> =
    LazyLock::new(|| std::array::from_fn(|_| RouteMetrics::new()));

fn status_slot(status: u16) -> usize {
    STATUS_CODES
        .iter()
        .position(|code| *code == status)
        .unwrap_or(if (200..300).contains(&status) {
            OTHER_2XX
        } else if (400..500).contains(&status) {
            OTHER_4XX
        } else {
            OTHER_5XX
        })
}

pub fn rest_request(class: RouteClass) {
    ROUTES[class as usize].requests.fetch_add(1, RELAXED);
}

/// Tek bir HTTP denemesinin sonucu. `status` ağ hatasında `None`.
pub fn rest_attempt(class: RouteClass, elapsed: Duration, status: Option<u16>) {
    let metrics = &ROUTES[class as usize];
    metrics.attempts.fetch_add(1, RELAXED);
    metrics.attempt_latency.record(elapsed);
    let Some(status) = status else {
        return;
    };
    metrics.statuses[status_slot(status)].fetch_add(1, RELAXED);
    if (200..300).contains(&status) {
        metrics.success.fetch_add(1, RELAXED);
    } else if status == 429 {
        metrics.rate_limited.fetch_add(1, RELAXED);
    } else if (400..500).contains(&status) && status != 408 {
        metrics.rejected.fetch_add(1, RELAXED);
    } else if status >= 500 {
        metrics.server_error.fetch_add(1, RELAXED);
    }
}

pub fn rest_retry(class: RouteClass) {
    ROUTES[class as usize].retries.fetch_add(1, RELAXED);
}

pub fn rest_rate_limit_wait(class: RouteClass, wait: Duration) {
    ROUTES[class as usize].wait_latency.record(wait);
}

pub fn rest_total(class: RouteClass, elapsed: Duration) {
    ROUTES[class as usize].total_latency.record(elapsed);
}

pub fn rest_ambiguous(class: RouteClass) {
    ROUTES[class as usize].ambiguous.fetch_add(1, RELAXED);
}

pub fn rest_timeout(class: RouteClass) {
    ROUTES[class as usize].timeouts.fetch_add(1, RELAXED);
}

pub fn rest_slow(class: RouteClass) {
    ROUTES[class as usize].slow.fetch_add(1, RELAXED);
}

pub fn rest_invalid(class: RouteClass) {
    ROUTES[class as usize].invalid.fetch_add(1, RELAXED);
}

// =====================================================================================
// UPSTREAM HTTP
// =====================================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum HttpHost {
    VxTwitter = 0,
    FxTwitter = 1,
    Syndication = 2,
    Discord = 3,
    TwitterCdn = 4,
    Instagram = 5,
    Reddit = 6,
    Spotify = 7,
    Other = 8,
}

impl HttpHost {
    pub const ALL: [HttpHost; 9] = [
        HttpHost::VxTwitter,
        HttpHost::FxTwitter,
        HttpHost::Syndication,
        HttpHost::Discord,
        HttpHost::TwitterCdn,
        HttpHost::Instagram,
        HttpHost::Reddit,
        HttpHost::Spotify,
        HttpHost::Other,
    ];

    pub fn classify(url: &str) -> Self {
        let lower = url.to_ascii_lowercase();
        if lower.contains("vxtwitter") {
            HttpHost::VxTwitter
        } else if lower.contains("fxtwitter") {
            HttpHost::FxTwitter
        } else if lower.contains("syndication.twimg.com") {
            HttpHost::Syndication
        } else if lower.contains("discord.com") || lower.contains("discordapp.com") {
            HttpHost::Discord
        } else if lower.contains("twimg.com") {
            HttpHost::TwitterCdn
        } else if lower.contains("instagram.com")
            || lower.contains("cdninstagram.com")
            || lower.contains("fbcdn.net")
        {
            HttpHost::Instagram
        } else if lower.contains("reddit.com")
            || lower.contains("redd.it")
            || lower.contains("redditmedia.com")
        {
            HttpHost::Reddit
        } else if lower.contains("spotify.com") || lower.contains("scdn.co") {
            HttpHost::Spotify
        } else {
            HttpHost::Other
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            HttpHost::VxTwitter => "vxtwitter",
            HttpHost::FxTwitter => "fxtwitter",
            HttpHost::Syndication => "twitter_syndication",
            HttpHost::Discord => "discord",
            HttpHost::TwitterCdn => "twitter_cdn",
            HttpHost::Instagram => "instagram",
            HttpHost::Reddit => "reddit",
            HttpHost::Spotify => "spotify",
            HttpHost::Other => "other",
        }
    }
}

struct HostMetrics {
    requests: AtomicU64,
    success: AtomicU64,
    errors: AtomicU64,
    bytes: AtomicU64,
    latency: Histogram,
}

impl HostMetrics {
    fn new() -> Self {
        Self {
            requests: AtomicU64::new(0),
            success: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            latency: Histogram::new(),
        }
    }
}

static HOSTS: LazyLock<[HostMetrics; 9]> =
    LazyLock::new(|| std::array::from_fn(|_| HostMetrics::new()));

pub fn http_request(host: HttpHost) {
    HOSTS[host as usize].requests.fetch_add(1, RELAXED);
}

pub fn http_success(host: HttpHost, elapsed: Duration, bytes: usize) {
    let metrics = &HOSTS[host as usize];
    metrics.success.fetch_add(1, RELAXED);
    metrics.bytes.fetch_add(bytes as u64, RELAXED);
    metrics.latency.record(elapsed);
}

pub fn http_error(host: HttpHost, elapsed: Duration) {
    let metrics = &HOSTS[host as usize];
    metrics.errors.fetch_add(1, RELAXED);
    metrics.latency.record(elapsed);
}

// =====================================================================================
// ÖNBELLEKLER
// =====================================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum CacheKind {
    Post = 0,
    Source = 1,
    Webhook = 2,
}

impl CacheKind {
    pub const ALL: [CacheKind; 3] = [CacheKind::Post, CacheKind::Source, CacheKind::Webhook];

    pub fn name(self) -> &'static str {
        match self {
            CacheKind::Post => "post",
            CacheKind::Source => "source",
            CacheKind::Webhook => "webhook",
        }
    }
}

struct CacheMetrics {
    hit: AtomicU64,
    miss: AtomicU64,
    expired: AtomicU64,
    put: AtomicU64,
    evict: AtomicU64,
    negative_hit: AtomicU64,
}

impl CacheMetrics {
    fn new() -> Self {
        Self {
            hit: AtomicU64::new(0),
            miss: AtomicU64::new(0),
            expired: AtomicU64::new(0),
            put: AtomicU64::new(0),
            evict: AtomicU64::new(0),
            negative_hit: AtomicU64::new(0),
        }
    }
}

static CACHES: LazyLock<[CacheMetrics; 3]> =
    LazyLock::new(|| std::array::from_fn(|_| CacheMetrics::new()));

pub fn cache_hit(kind: CacheKind) {
    CACHES[kind as usize].hit.fetch_add(1, RELAXED);
}

pub fn cache_miss(kind: CacheKind) {
    CACHES[kind as usize].miss.fetch_add(1, RELAXED);
}

pub fn cache_expired(kind: CacheKind) {
    CACHES[kind as usize].expired.fetch_add(1, RELAXED);
}

pub fn cache_put(kind: CacheKind) {
    CACHES[kind as usize].put.fetch_add(1, RELAXED);
}

pub fn cache_evict(kind: CacheKind) {
    CACHES[kind as usize].evict.fetch_add(1, RELAXED);
}

pub fn cache_negative_hit(kind: CacheKind) {
    CACHES[kind as usize].negative_hit.fetch_add(1, RELAXED);
}

// =====================================================================================
// VERİTABANI
// =====================================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum DbOp {
    Remember = 0,
    Forget = 1,
    Author = 2,
    Contains = 3,
    Count = 4,
    Prune = 5,
    Query = 6,
    Healthy = 7,
}

impl DbOp {
    pub const ALL: [DbOp; 8] = [
        DbOp::Remember,
        DbOp::Forget,
        DbOp::Author,
        DbOp::Contains,
        DbOp::Count,
        DbOp::Prune,
        DbOp::Query,
        DbOp::Healthy,
    ];

    pub fn name(self) -> &'static str {
        match self {
            DbOp::Remember => "remember",
            DbOp::Forget => "forget",
            DbOp::Author => "author",
            DbOp::Contains => "contains",
            DbOp::Count => "count",
            DbOp::Prune => "prune",
            DbOp::Query => "query",
            DbOp::Healthy => "healthy",
        }
    }
}

struct DbMetrics {
    count: AtomicU64,
    errors: AtomicU64,
    latency: Histogram,
}

impl DbMetrics {
    fn new() -> Self {
        Self {
            count: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            latency: Histogram::new(),
        }
    }
}

static DB_OPS: LazyLock<[DbMetrics; 8]> =
    LazyLock::new(|| std::array::from_fn(|_| DbMetrics::new()));

pub fn db_op(op: DbOp, elapsed: Duration, ok: bool) {
    let metrics = &DB_OPS[op as usize];
    metrics.count.fetch_add(1, RELAXED);
    if !ok {
        metrics.errors.fetch_add(1, RELAXED);
    }
    metrics.latency.record(elapsed);
}

// =====================================================================================
// GATEWAY / SHARD
// =====================================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum GatewayEvent {
    Ready = 0,
    Message = 1,
    Reaction = 2,
    Interaction = 3,
    ShardStage = 4,
    Other = 5,
}

impl GatewayEvent {
    pub const ALL: [GatewayEvent; 6] = [
        GatewayEvent::Ready,
        GatewayEvent::Message,
        GatewayEvent::Reaction,
        GatewayEvent::Interaction,
        GatewayEvent::ShardStage,
        GatewayEvent::Other,
    ];

    pub fn name(self) -> &'static str {
        match self {
            GatewayEvent::Ready => "ready",
            GatewayEvent::Message => "message",
            GatewayEvent::Reaction => "reaction",
            GatewayEvent::Interaction => "interaction",
            GatewayEvent::ShardStage => "shard_stage",
            GatewayEvent::Other => "other",
        }
    }
}

static GATEWAY_EVENTS: LazyLock<[AtomicU64; 6]> =
    LazyLock::new(|| std::array::from_fn(|_| AtomicU64::new(0)));
static LAST_GATEWAY_EVENT_MS: AtomicU64 = AtomicU64::new(0);

pub fn gateway_event(kind: GatewayEvent) {
    GATEWAY_EVENTS[kind as usize].fetch_add(1, RELAXED);
    LAST_GATEWAY_EVENT_MS.store(now_ms(), RELAXED);
}

// =====================================================================================
// SÜREÇ / ANLIK GÖRÜNTÜ
// =====================================================================================

static STARTED: OnceLock<Instant> = OnceLock::new();

/// Süreç başlangıcını işaretler (uptime için). Birden fazla çağrı zararsızdır.
pub fn mark_started() {
    let _ = STARTED.set(Instant::now());
}

fn uptime_seconds() -> u64 {
    STARTED
        .get()
        .map(|started| started.elapsed().as_secs())
        .unwrap_or(0)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

fn histogram_map() -> Value {
    let mut object = serde_json::Map::new();
    for phase in Phase::ALL {
        object.insert(phase.name().to_string(), PHASES[phase as usize].snapshot());
    }
    Value::Object(object)
}

fn rest_snapshot() -> Value {
    let routes: Vec<Value> = RouteClass::ALL
        .iter()
        .map(|class| {
            let metrics = &ROUTES[*class as usize];
            let mut statuses = serde_json::Map::new();
            for (index, counter) in metrics.statuses.iter().enumerate() {
                let count = counter.load(RELAXED);
                if count == 0 {
                    continue;
                }
                let key = if index == OTHER_2XX {
                    "2xx".to_string()
                } else if index == OTHER_4XX {
                    "other_4xx".to_string()
                } else if index == OTHER_5XX {
                    "other_5xx".to_string()
                } else {
                    STATUS_CODES[index].to_string()
                };
                statuses.insert(key, json!(count));
            }
            json!({
                "route": class.name(),
                "requests": metrics.requests.load(RELAXED),
                "attempts": metrics.attempts.load(RELAXED),
                "success": metrics.success.load(RELAXED),
                "rejected_4xx": metrics.rejected.load(RELAXED),
                "server_5xx": metrics.server_error.load(RELAXED),
                "rate_limited_429": metrics.rate_limited.load(RELAXED),
                "retries": metrics.retries.load(RELAXED),
                "ambiguous": metrics.ambiguous.load(RELAXED),
                "timeouts": metrics.timeouts.load(RELAXED),
                "slow_calls": metrics.slow.load(RELAXED),
                "invalid_payloads": metrics.invalid.load(RELAXED),
                "status_codes": statuses,
                "attempt_latency_ms": metrics.attempt_latency.snapshot(),
                "total_latency_ms": metrics.total_latency.snapshot(),
                "rate_limit_wait_ms": metrics.wait_latency.snapshot()
            })
        })
        .collect();
    json!({"routes": routes})
}

fn http_snapshot() -> Value {
    let hosts: Vec<Value> = HttpHost::ALL
        .iter()
        .map(|host| {
            let metrics = &HOSTS[*host as usize];
            json!({
                "host": host.name(),
                "requests": metrics.requests.load(RELAXED),
                "success": metrics.success.load(RELAXED),
                "errors": metrics.errors.load(RELAXED),
                "bytes": metrics.bytes.load(RELAXED),
                "latency_ms": metrics.latency.snapshot()
            })
        })
        .collect();
    json!({"hosts": hosts})
}

fn cache_snapshot() -> Value {
    let mut object = serde_json::Map::new();
    for kind in CacheKind::ALL {
        let metrics = &CACHES[kind as usize];
        object.insert(
            kind.name().to_string(),
            json!({
                "hit": metrics.hit.load(RELAXED),
                "miss": metrics.miss.load(RELAXED),
                "expired": metrics.expired.load(RELAXED),
                "put": metrics.put.load(RELAXED),
                "evict": metrics.evict.load(RELAXED),
                "negative_hit": metrics.negative_hit.load(RELAXED)
            }),
        );
    }
    Value::Object(object)
}

fn db_snapshot() -> Value {
    let ops: Vec<Value> = DbOp::ALL
        .iter()
        .map(|op| {
            let metrics = &DB_OPS[*op as usize];
            json!({
                "op": op.name(),
                "count": metrics.count.load(RELAXED),
                "errors": metrics.errors.load(RELAXED),
                "latency_ms": metrics.latency.snapshot()
            })
        })
        .collect();
    json!({"ops": ops})
}

pub fn gateway_snapshot() -> Value {
    let mut events = serde_json::Map::new();
    for kind in GatewayEvent::ALL {
        events.insert(
            kind.name().to_string(),
            json!(GATEWAY_EVENTS[kind as usize].load(RELAXED)),
        );
    }
    let last_event_ms = LAST_GATEWAY_EVENT_MS.load(RELAXED);
    json!({
        "events": events,
        "last_event_ms": if last_event_ms == 0 { Value::Null } else { json!(last_event_ms) },
        "last_event_age_seconds": if last_event_ms == 0 {
            Value::Null
        } else {
            json!(now_ms().saturating_sub(last_event_ms) / 1_000)
        }
    })
}

/// Linux `/proc` ve cgroup v2'den süreç/kaynak özeti. Diğer platformlarda alanlar null.
pub fn process_snapshot() -> Value {
    let mut object = serde_json::Map::new();
    object.insert("os".into(), json!(std::env::consts::OS));
    object.insert("arch".into(), json!(std::env::consts::ARCH));
    object.insert("pid".into(), json!(std::process::id()));
    object.insert("uptime_seconds".into(), json!(uptime_seconds()));

    #[cfg(target_os = "linux")]
    {
        if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
            for (key, field) in [
                ("VmRSS:", "rss_kb"),
                ("VmHWM:", "peak_rss_kb"),
                ("VmSize:", "virtual_kb"),
                ("Threads:", "threads"),
            ] {
                if let Some(value) = status
                    .lines()
                    .find(|line| line.starts_with(key))
                    .and_then(|line| line.split_whitespace().nth(1))
                    .and_then(|value| value.parse::<u64>().ok())
                {
                    object.insert(field.into(), json!(value));
                }
            }
        }
        if let Ok(entries) = std::fs::read_dir("/proc/self/fd") {
            object.insert("open_fds".into(), json!(entries.count()));
        }
        if let Ok(io) = std::fs::read_to_string("/proc/self/io") {
            for (key, field) in [
                ("read_bytes:", "io_read_bytes"),
                ("write_bytes:", "io_write_bytes"),
            ] {
                if let Some(value) = io
                    .lines()
                    .find(|line| line.starts_with(key))
                    .and_then(|line| line.split_whitespace().nth(1))
                    .and_then(|value| value.parse::<u64>().ok())
                {
                    object.insert(field.into(), json!(value));
                }
            }
        }
        object.insert("cpu_seconds".into(), json!(linux_cpu_seconds()));
        if let Some(current) = read_u64("/sys/fs/cgroup/memory.current") {
            object.insert("cgroup_memory_current_bytes".into(), json!(current));
        }
        if let Ok(max) = std::fs::read_to_string("/sys/fs/cgroup/memory.max") {
            let max = max.trim();
            object.insert(
                "cgroup_memory_max_bytes".into(),
                if max == "max" {
                    Value::Null
                } else {
                    max.parse::<u64>().map_or(Value::Null, |value| json!(value))
                },
            );
        }
        if let Ok(stat) = std::fs::read_to_string("/sys/fs/cgroup/cpu.stat") {
            for (key, field) in [
                ("usage_usec", "cgroup_cpu_usage_us"),
                ("throttled_usec", "cgroup_cpu_throttled_us"),
                ("nr_throttled", "cgroup_cpu_nr_throttled"),
            ] {
                if let Some(value) = stat
                    .lines()
                    .find(|line| line.starts_with(key))
                    .and_then(|line| line.split_whitespace().nth(1))
                    .and_then(|value| value.parse::<u64>().ok())
                {
                    object.insert(field.into(), json!(value));
                }
            }
        }
        if let Some(pids) = read_u64("/sys/fs/cgroup/pids.current") {
            object.insert("cgroup_pids_current".into(), json!(pids));
        }
    }

    Value::Object(object)
}

#[cfg(target_os = "linux")]
fn read_u64(path: &str) -> Option<u64> {
    std::fs::read_to_string(path)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
}

/// `/proc/self/stat` alan 14+15 (utime+stime), `sysconf(_SC_CLK_TCK)` varsayımı 100.
#[cfg(target_os = "linux")]
fn linux_cpu_seconds() -> f64 {
    let Ok(stat) = std::fs::read_to_string("/proc/self/stat") else {
        return 0.0;
    };
    let Some((_, rest)) = stat.rsplit_once(')') else {
        return 0.0;
    };
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let ticks = fields
        .get(11)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
        .saturating_add(
            fields
                .get(12)
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0),
        );
    ticks as f64 / 100.0
}

// =====================================================================================
// ANLIK GÖRÜNTÜ (mevcut + ayrıntılı)
// =====================================================================================

pub fn snapshot() -> Value {
    json!({
        "cache_hits": CACHE_HITS.load(RELAXED),
        "overloads": OVERLOADS.load(RELAXED),
        "conversion_queue": {
            "capacity": crate::converters::CONVERSION_QUEUE_CAPACITY,
            "enqueued": CONVERSION_QUEUE_ENQUEUED.load(RELAXED),
            "completed": CONVERSION_QUEUE_COMPLETED.load(RELAXED),
            "saturated": CONVERSION_QUEUE_SATURATED.load(RELAXED),
            "depth": CONVERSION_QUEUE_DEPTH.load(RELAXED),
        },
        "native_conversions": PLATFORMS.iter().zip(COUNTERS.iter()).map(|(platform, c)| serde_json::json!({
            "platform": platform, "success": c.ok.load(RELAXED), "failed": c.failed.load(RELAXED),
            "duration_ms_buckets": LIMITS.iter().zip(&c.buckets).map(|(limit, count)| serde_json::json!({"le":limit,"count":count.load(RELAXED)})).collect::<Vec<_>>()
        })).collect::<Vec<_>>(),
        "rest": rest_snapshot(),
        "phases": histogram_map(),
        "http": http_snapshot(),
        "caches": cache_snapshot(),
        "db": db_snapshot(),
        "gateway": gateway_snapshot(),
        "process": process_snapshot()
    })
}

/// `snapshot()` ile aynı; MCP'de ayrı bir araç adı için niyet belgesi.
pub fn detailed_snapshot() -> Value {
    snapshot()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_percentiles_are_bounded_and_monotonic() {
        let histogram = Histogram::new();
        for micros in [500, 1_500, 3_000, 20_000, 600_000] {
            histogram.record(Duration::from_micros(micros));
        }
        let snapshot = histogram.snapshot();
        assert_eq!(snapshot["count"], 5);
        assert_eq!(snapshot["min_ms"], 1);
        assert_eq!(snapshot["max_ms"], 600);
        let p50 = snapshot["p50_ms"].as_u64().unwrap();
        let p90 = snapshot["p90_ms"].as_u64().unwrap();
        let p95 = snapshot["p95_ms"].as_u64().unwrap();
        assert!(p50 <= p90 && p90 <= p95);
        assert!(p50 >= 1 && p95 >= 10);
        assert_eq!(snapshot["recent_count"], 5);
        assert_eq!(snapshot["recent_p50_ms"], 3);
        assert_eq!(snapshot["recent_p95_ms"], 600);
        assert_eq!(snapshot["recent_max_ms"], 600);
        assert_eq!(snapshot["buckets"].as_array().unwrap().len(), BUCKET_COUNT);
    }

    #[test]
    fn empty_histogram_reports_nulls() {
        let snapshot = Histogram::new().snapshot();
        assert_eq!(snapshot["count"], 0);
        assert!(snapshot["p95_ms"].is_null());
        assert!(snapshot["recent_p95_ms"].is_null());
    }

    #[test]
    fn success_status_is_not_counted_as_server_error() {
        assert_eq!(status_slot(200), OTHER_2XX);
        assert_eq!(status_slot(201), OTHER_2XX);
        assert_eq!(status_slot(403), 2);
        assert_eq!(status_slot(429), 5);
        assert_eq!(status_slot(418), OTHER_4XX);
        assert_eq!(status_slot(501), OTHER_5XX);
    }

    #[test]
    fn phase_timer_records_on_scope_exit() {
        let before = PHASES[Phase::Render as usize].count();
        {
            let _timer = PhaseTimer::new(Phase::Render);
        }
        assert_eq!(PHASES[Phase::Render as usize].count(), before + 1);
    }

    #[test]
    fn route_and_host_classification_is_stable() {
        assert_eq!(
            RouteClass::classify("channel:123"),
            RouteClass::ChannelMessage
        );
        assert_eq!(
            RouteClass::classify("hook_exec:456"),
            RouteClass::WebhookExecute
        );
        assert_eq!(
            RouteClass::classify("hook_list:789"),
            RouteClass::WebhookList
        );
        assert_eq!(
            RouteClass::classify("hook_create:789"),
            RouteClass::WebhookCreate
        );
        assert_eq!(
            RouteClass::classify("interaction:1"),
            RouteClass::InteractionEdit
        );
        assert_eq!(RouteClass::classify("bilinmeyen"), RouteClass::Other);
        assert_eq!(
            HttpHost::classify("https://api.vxtwitter.com/i/status/1"),
            HttpHost::VxTwitter
        );
        assert_eq!(
            HttpHost::classify("https://cdn.syndication.twimg.com/tweet-result"),
            HttpHost::Syndication
        );
        assert_eq!(
            HttpHost::classify("https://pbs.twimg.com/media/a.jpg"),
            HttpHost::TwitterCdn
        );
    }

    #[test]
    fn snapshot_exposes_all_debug_sections() {
        let snapshot = snapshot();
        for key in [
            "cache_hits",
            "overloads",
            "conversion_queue",
            "native_conversions",
            "rest",
            "phases",
            "http",
            "caches",
            "db",
            "gateway",
            "process",
        ] {
            assert!(snapshot.get(key).is_some(), "eksik bölüm: {key}");
        }
        assert!(snapshot["rest"]["routes"].as_array().unwrap().len() >= 5);
        assert!(snapshot["phases"]["total"].is_object());
        assert!(snapshot["process"]["os"].is_string());
    }

    #[test]
    fn counters_are_recorded_per_route_and_cache() {
        let before = ROUTES[RouteClass::ChannelMessage as usize]
            .requests
            .load(RELAXED);
        rest_request(RouteClass::ChannelMessage);
        assert_eq!(
            ROUTES[RouteClass::ChannelMessage as usize]
                .requests
                .load(RELAXED),
            before + 1
        );

        let before = CACHES[CacheKind::Post as usize].hit.load(RELAXED);
        cache_hit(CacheKind::Post);
        assert_eq!(
            CACHES[CacheKind::Post as usize].hit.load(RELAXED),
            before + 1
        );
    }
}
