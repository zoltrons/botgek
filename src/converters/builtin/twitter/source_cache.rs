//! Bounded, short-lived storage for successful, already parsed source results.
//!
//! The caller supplies a non-secret source/id/target/budget key. Coordination is
//! namespaced separately from whole-post fetches; result storage is instance-local.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

type Entries<T> = HashMap<String, (Instant, Arc<T>)>;

/// TTL starts when a successful fetch completes. Full caches evict the oldest
/// stored result; reads do not extend its lifetime. Zero capacity or TTL disables
/// result reuse.
pub(super) struct SourceCache<T> {
    ttl: Duration,
    capacity: usize,
    entries: Mutex<Entries<T>>,
}

impl<T> SourceCache<T> {
    pub(super) fn new(ttl: Duration, capacity: usize) -> Self {
        Self {
            ttl,
            capacity,
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub(super) async fn get_or_fetch<F, Fut>(&self, key: String, fetch: F) -> Result<Arc<T>, String>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, String>>,
    {
        if let Some(value) = self.get(&key).await {
            return Ok(value);
        }

        let _single_flight =
            super::super::coordination::lock(format!("twitter:source-result:{key}")).await;
        // Another caller may have populated this cache while we waited. Other
        // cache instances can share the lock, but never share their result maps.
        if let Some(value) = self.get(&key).await {
            return Ok(value);
        }

        // No in-flight state is stored. Dropping or unwinding this future drops
        // the coordination guard, so cancellation, panic and errors can retry.
        // The entries mutex is never held across the adapter fetch.
        let value = Arc::new(fetch().await?);
        self.insert(key, Arc::clone(&value)).await;
        Ok(value)
    }

    /// Inspect successful source data before starting speculative network requests.
    pub(super) async fn get(&self, key: &str) -> Option<Arc<T>> {
        let mut entries = self.entries.lock().await;
        match entries.get(key) {
            Some((stored_at, value)) if stored_at.elapsed() < self.ttl => Some(Arc::clone(value)),
            Some(_) => {
                entries.remove(key);
                None
            }
            None => None,
        }
    }

    async fn insert(&self, key: String, value: Arc<T>) {
        if self.capacity == 0 || self.ttl.is_zero() {
            return;
        }

        let mut entries = self.entries.lock().await;
        let now = Instant::now();
        if entries.len() >= self.capacity {
            entries.retain(|_, (stored_at, _)| now.duration_since(*stored_at) < self.ttl);
        }
        if entries.len() >= self.capacity
            && let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, (stored_at, _))| *stored_at)
                .map(|(key, _)| key.clone())
        {
            entries.remove(&oldest);
        }
        entries.insert(key, (now, value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::Poll;
    use tokio::sync::oneshot;

    const TEST_TIMEOUT: Duration = Duration::from_secs(5);
    const TTL: Duration = Duration::from_secs(60);

    // Poll a caller once to establish a concurrent miss while the first fetch
    // is blocked. No wall-clock sleeps or scheduler timing assumptions are used.
    async fn assert_pending<F: Future>(mut future: Pin<&mut F>) {
        std::future::poll_fn(|cx| {
            assert!(future.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
    }

    async fn wait_for_fetch_start<F: Future>(
        mut future: Pin<&mut F>,
        started: oneshot::Receiver<()>,
    ) {
        tokio::time::timeout(TEST_TIMEOUT, async {
            tokio::select! {
                biased;
                _ = future.as_mut() => panic!("fetch completed before being released"),
                result = started => result.expect("fetch did not send its start signal"),
            }
        })
        .await
        .expect("fetch did not start");
    }

    #[tokio::test]
    async fn cache_hit_reuses_arc_without_running_fetch() {
        // Results need not implement Clone.
        struct ParsedResult(&'static str);

        let cache = SourceCache::new(TTL, 2);
        let first = cache
            .get_or_fetch("hit".into(), || async { Ok(ParsedResult("parsed")) })
            .await
            .unwrap();
        let second = cache
            .get_or_fetch("hit".into(), || async { panic!("cache hit fetched again") })
            .await
            .unwrap();

        assert_eq!(second.0, "parsed");
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[tokio::test]
    async fn concurrent_same_key_fetches_are_coalesced() {
        let cache = SourceCache::new(TTL, 2);
        let invocations = AtomicUsize::new(0);
        let (started, start_signal) = oneshot::channel();
        let (release, released) = oneshot::channel();
        let mut first = Box::pin(cache.get_or_fetch("coalescing".into(), || async {
            invocations.fetch_add(1, Ordering::SeqCst);
            started.send(()).unwrap();
            released.await.unwrap();
            Ok(42)
        }));
        wait_for_fetch_start(first.as_mut(), start_signal).await;

        let mut waiters = Vec::new();
        for _ in 0..8 {
            let mut waiter = Box::pin(cache.get_or_fetch("coalescing".into(), || async {
                invocations.fetch_add(1, Ordering::SeqCst);
                Ok(99)
            }));
            assert_pending(waiter.as_mut()).await;
            waiters.push(waiter);
        }
        assert_eq!(invocations.load(Ordering::SeqCst), 1);

        release.send(()).unwrap();
        let first = first.await.unwrap();
        for waiter in waiters {
            let value = tokio::time::timeout(TEST_TIMEOUT, waiter)
                .await
                .expect("same-key waiter remained blocked")
                .unwrap();
            assert_eq!(*value, 42);
            assert!(Arc::ptr_eq(&first, &value));
        }
        assert_eq!(invocations.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn different_keys_do_not_wait_for_each_other() {
        let cache = SourceCache::new(TTL, 2);
        let (started, start_signal) = oneshot::channel();
        let mut blocked = Box::pin(cache.get_or_fetch("independent:blocked".into(), || async {
            started.send(()).unwrap();
            std::future::pending::<Result<usize, String>>().await
        }));
        wait_for_fetch_start(blocked.as_mut(), start_signal).await;

        let independent = tokio::time::timeout(
            TEST_TIMEOUT,
            cache.get_or_fetch("independent:ready".into(), || async { Ok(7) }),
        )
        .await
        .expect("a different key was blocked by the pending fetch")
        .unwrap();
        assert_eq!(*independent, 7);
    }

    #[tokio::test]
    async fn failed_fetches_retry_without_poisoning_the_cache() {
        let cache = SourceCache::<usize>::new(TTL, 2);
        let invocations = AtomicUsize::new(0);
        let failure = cache
            .get_or_fetch("retry".into(), || async {
                invocations.fetch_add(1, Ordering::SeqCst);
                Err("temporary failure".into())
            })
            .await
            .unwrap_err();
        assert_eq!(failure, "temporary failure");
        assert!(cache.entries.lock().await.is_empty());

        let recovered = cache
            .get_or_fetch("retry".into(), || async {
                invocations.fetch_add(1, Ordering::SeqCst);
                Ok(8)
            })
            .await
            .unwrap();
        let hit = cache
            .get_or_fetch("retry".into(), || async {
                panic!("successful retry was not cached")
            })
            .await
            .unwrap();
        assert_eq!(invocations.load(Ordering::SeqCst), 2);
        assert_eq!(*recovered, 8);
        assert!(Arc::ptr_eq(&recovered, &hit));
    }

    #[tokio::test]
    async fn capacity_evicts_the_oldest_stored_result() {
        let cache = SourceCache::new(TTL, 2);
        let oldest = cache
            .get_or_fetch("eviction:oldest".into(), || async { Ok(1) })
            .await
            .unwrap();
        // Make the eviction order deterministic even on coarse system clocks.
        cache
            .entries
            .lock()
            .await
            .get_mut("eviction:oldest")
            .unwrap()
            .0 = Instant::now() - Duration::from_secs(1);
        let recent = cache
            .get_or_fetch("eviction:recent".into(), || async { Ok(2) })
            .await
            .unwrap();
        cache
            .get_or_fetch("eviction:newest".into(), || async { Ok(3) })
            .await
            .unwrap();

        {
            let entries = cache.entries.lock().await;
            assert_eq!(entries.len(), 2);
            assert!(!entries.contains_key("eviction:oldest"));
            assert!(entries.contains_key("eviction:recent"));
            assert!(entries.contains_key("eviction:newest"));
        }
        let hit = cache
            .get_or_fetch("eviction:recent".into(), || async {
                panic!("recent entry was evicted")
            })
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&recent, &hit));
        let refetched = cache
            .get_or_fetch("eviction:oldest".into(), || async { Ok(4) })
            .await
            .unwrap();
        assert_eq!(*refetched, 4);
        assert!(!Arc::ptr_eq(&oldest, &refetched));
        assert_eq!(cache.entries.lock().await.len(), 2);
    }

    #[tokio::test]
    async fn expired_results_are_removed_on_read_and_refetched() {
        let cache = SourceCache::new(TTL, 2);
        let first = cache
            .get_or_fetch("expiry".into(), || async { Ok(1) })
            .await
            .unwrap();
        cache.entries.lock().await.get_mut("expiry").unwrap().0 = Instant::now() - TTL;

        let failure = cache
            .get_or_fetch("expiry".into(), || async {
                Err("fresh fetch failed".into())
            })
            .await
            .unwrap_err();
        assert_eq!(failure, "fresh fetch failed");
        assert!(cache.entries.lock().await.is_empty());

        let fresh = cache
            .get_or_fetch("expiry".into(), || async { Ok(2) })
            .await
            .unwrap();
        assert_eq!(*fresh, 2);
        assert!(!Arc::ptr_eq(&first, &fresh));
    }

    #[tokio::test]
    async fn full_cache_prunes_expired_results_before_evicting_live_ones() {
        let cache = SourceCache::new(TTL, 2);
        let live = cache
            .get_or_fetch("pruning:live".into(), || async { Ok(1) })
            .await
            .unwrap();
        cache
            .get_or_fetch("pruning:expired".into(), || async { Ok(2) })
            .await
            .unwrap();
        cache
            .entries
            .lock()
            .await
            .get_mut("pruning:expired")
            .unwrap()
            .0 = Instant::now() - TTL;
        cache
            .get_or_fetch("pruning:newest".into(), || async { Ok(3) })
            .await
            .unwrap();

        let hit = cache
            .get_or_fetch("pruning:live".into(), || async {
                panic!("live entry was evicted")
            })
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&live, &hit));
        let entries = cache.entries.lock().await;
        assert_eq!(entries.len(), 2);
        assert!(!entries.contains_key("pruning:expired"));
    }

    #[tokio::test]
    async fn cancelled_fetch_releases_lock_and_waiter_recovers() {
        let cache = SourceCache::new(TTL, 2);
        let invocations = AtomicUsize::new(0);
        let (started, start_signal) = oneshot::channel();
        let mut cancelled = Box::pin(cache.get_or_fetch("cancellation".into(), || async {
            invocations.fetch_add(1, Ordering::SeqCst);
            started.send(()).unwrap();
            std::future::pending::<Result<usize, String>>().await
        }));
        wait_for_fetch_start(cancelled.as_mut(), start_signal).await;
        let mut waiter = Box::pin(cache.get_or_fetch("cancellation".into(), || async {
            invocations.fetch_add(1, Ordering::SeqCst);
            Ok(9)
        }));
        assert_pending(waiter.as_mut()).await;
        assert_eq!(invocations.load(Ordering::SeqCst), 1);
        assert!(cache.entries.lock().await.is_empty());

        // Dropping a pending get_or_fetch future is cancellation, including its
        // currently awaited adapter future and the held coordination guard.
        drop(cancelled);
        let recovered = tokio::time::timeout(TEST_TIMEOUT, waiter)
            .await
            .expect("cancelled fetch left the key locked")
            .unwrap();
        assert_eq!(*recovered, 9);
        assert_eq!(invocations.load(Ordering::SeqCst), 2);
        let hit = cache
            .get_or_fetch("cancellation".into(), || async {
                panic!("recovered result was not cached")
            })
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&recovered, &hit));
    }

    #[tokio::test]
    async fn panicking_fetch_does_not_poison_the_cache_or_lock() {
        let cache = Arc::new(SourceCache::<usize>::new(TTL, 2));
        let task_cache = Arc::clone(&cache);
        let panic = tokio::spawn(async move {
            task_cache
                .get_or_fetch("panic".into(), || async {
                    tokio::task::yield_now().await;
                    panic!("adapter panicked");
                })
                .await
        })
        .await
        .unwrap_err();
        assert!(panic.is_panic());
        assert!(cache.entries.lock().await.is_empty());

        let recovered = tokio::time::timeout(
            TEST_TIMEOUT,
            cache.get_or_fetch("panic".into(), || async { Ok(10) }),
        )
        .await
        .expect("panicking fetch left the key locked")
        .unwrap();
        assert_eq!(*recovered, 10);
    }

    #[tokio::test]
    async fn instances_keep_results_isolated_even_with_identical_keys() {
        let first_cache = SourceCache::new(TTL, 2);
        let second_cache = SourceCache::new(TTL, 2);
        let first = first_cache
            .get_or_fetch("isolation".into(), || async { Ok(1) })
            .await
            .unwrap();
        let second = second_cache
            .get_or_fetch("isolation".into(), || async { Ok(2) })
            .await
            .unwrap();
        assert_eq!(*first, 1);
        assert_eq!(*second, 2);
        assert!(!Arc::ptr_eq(&first, &second));

        let first_hit = first_cache
            .get_or_fetch("isolation".into(), || async {
                panic!("first instance lost its result")
            })
            .await
            .unwrap();
        let second_hit = second_cache
            .get_or_fetch("isolation".into(), || async {
                panic!("second instance lost its result")
            })
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&first, &first_hit));
        assert!(Arc::ptr_eq(&second, &second_hit));
    }

    #[tokio::test]
    async fn zero_capacity_or_ttl_disables_result_storage() {
        for (ttl, capacity) in [(TTL, 0), (Duration::ZERO, 2)] {
            let cache = SourceCache::new(ttl, capacity);
            let invocations = AtomicUsize::new(0);
            for expected in 0..2 {
                let value = cache
                    .get_or_fetch("disabled".into(), || async {
                        Ok(invocations.fetch_add(1, Ordering::SeqCst))
                    })
                    .await
                    .unwrap();
                assert_eq!(*value, expected);
            }
            assert_eq!(invocations.load(Ordering::SeqCst), 2);
            assert!(cache.entries.lock().await.is_empty());
        }
    }
}
