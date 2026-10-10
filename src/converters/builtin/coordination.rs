//! Keyed single-flight locks. Idle keys hold no payload or secret and are reclaimed.
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Weak},
};
use tokio::sync::{Mutex, OwnedMutexGuard};

type Locks = Mutex<HashMap<String, Weak<Mutex<()>>>>;
static LOCKS: LazyLock<Locks> = LazyLock::new(|| Mutex::new(HashMap::new()));

pub async fn lock(key: String) -> OwnedMutexGuard<()> {
    let lock = {
        let mut locks = LOCKS.lock().await;
        locks.retain(|_, value| value.strong_count() > 0);
        match locks.get(&key).and_then(Weak::upgrade) {
            Some(lock) => lock,
            None => {
                let lock = Arc::new(Mutex::new(()));
                locks.insert(key, Arc::downgrade(&lock));
                lock
            }
        }
    };
    lock.lock_owned().await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn same_key_waits_while_other_keys_do_not() {
        let first = lock("test:same".into()).await;
        let waiting = lock("test:same".into());
        tokio::pin!(waiting);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut waiting)
                .await
                .is_err()
        );
        let independent = lock("test:other".into()).await;
        drop(first);
        let acquired = tokio::time::timeout(std::time::Duration::from_secs(1), waiting)
            .await
            .unwrap();
        drop((acquired, independent));
    }
}
