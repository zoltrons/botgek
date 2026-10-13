//! Bounded process-local counters; no user IDs, URLs, tokens or payloads are retained.
use std::sync::{
    LazyLock,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Duration;
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
    if ok { &counters.ok } else { &counters.failed }.fetch_add(1, Ordering::Relaxed);
    let millis = elapsed.as_millis().min(u64::MAX as u128) as u64;
    for (limit, bucket) in LIMITS.iter().zip(&counters.buckets) {
        if millis <= *limit {
            bucket.fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub fn snapshot() -> serde_json::Value {
    serde_json::json!({
        "cache_hits": CACHE_HITS.load(Ordering::Relaxed),
        "overloads": OVERLOADS.load(Ordering::Relaxed),
        "conversion_queue": {
            "capacity": crate::converters::CONVERSION_QUEUE_CAPACITY,
            "enqueued": CONVERSION_QUEUE_ENQUEUED.load(Ordering::Relaxed),
            "completed": CONVERSION_QUEUE_COMPLETED.load(Ordering::Relaxed),
            "saturated": CONVERSION_QUEUE_SATURATED.load(Ordering::Relaxed),
            "depth": CONVERSION_QUEUE_DEPTH.load(Ordering::Relaxed),
        },
        "native_conversions": PLATFORMS.iter().zip(COUNTERS.iter()).map(|(platform, c)| serde_json::json!({
            "platform": platform, "success": c.ok.load(Ordering::Relaxed), "failed": c.failed.load(Ordering::Relaxed),
            "duration_ms_buckets": LIMITS.iter().zip(&c.buckets).map(|(limit, count)| serde_json::json!({"le":limit,"count":count.load(Ordering::Relaxed)})).collect::<Vec<_>>()
        })).collect::<Vec<_>>()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_exposes_bounded_conversion_queue_metrics() {
        let snapshot = snapshot();
        let queue = snapshot
            .get("conversion_queue")
            .expect("conversion queue metrics should be present");
        assert_eq!(
            queue.get("capacity").and_then(serde_json::Value::as_u64),
            Some(crate::converters::CONVERSION_QUEUE_CAPACITY as u64)
        );
        for name in ["enqueued", "completed", "saturated", "depth"] {
            assert!(
                queue
                    .get(name)
                    .and_then(serde_json::Value::as_u64)
                    .is_some()
            );
        }
    }
}
