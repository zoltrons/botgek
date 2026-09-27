//! Kısa ömürlü gönderi önbelleği.
//!
//! Aynı link kısa süre içinde tekrar paylaşıldığında (veya aynı gönderi başka bir kanalda
//! tekrar yazıldığında) ağ isteği hiç yapılmaz: dönüşüm anında tamamlanır. Önbellek
//! `Instant` tabanlı TTL ile temizlenir ve boyutu sınırlıdır.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use tokio::sync::Mutex;

use super::BuiltinPost;

/// Gönderi verisinin önbellekte kalma süresi (metrikler bu süre içinde güncellenmez).
const TTL: Duration = Duration::from_secs(600);
/// Önbellekte tutulacak en fazla gönderi sayısı.
const MAX_ENTRIES: usize = 512;

/// Önbellek girdisi: gönderi + eklenme zamanı.
static CACHE: Lazy<Mutex<HashMap<String, (Instant, Arc<BuiltinPost>)>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Önbellekten gönderiyi döndürür (süresi dolmuşsa siler).
pub async fn get(key: &str) -> Option<Arc<BuiltinPost>> {
    let mut cache = CACHE.lock().await;
    match cache.get(key) {
        Some((stored_at, post)) if stored_at.elapsed() < TTL => Some(post.clone()),
        Some(_) => {
            cache.remove(key);
            None
        }
        None => None,
    }
}

/// Gönderiyi önbelleğe yazar; sınır aşılırsa süresi dolanlar ve en eski girdiler atılır.
pub async fn put(key: String, post: &BuiltinPost) {
    let mut cache = CACHE.lock().await;

    if cache.len() >= MAX_ENTRIES {
        cache.retain(|_, (stored_at, _)| stored_at.elapsed() < TTL);
    }
    if cache.len() >= MAX_ENTRIES {
        // Hâlâ doluysa en eski girdi çıkarılır (tek giriş).
        if let Some(oldest) = cache
            .iter()
            .min_by_key(|(_, (stored_at, _))| *stored_at)
            .map(|(key, _)| key.clone())
        {
            cache.remove(&oldest);
        }
    }

    cache.insert(key, (Instant::now(), Arc::new(post.clone())));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::converters::builtin::Author;
    use crate::converters::parser::Platform;

    fn post() -> BuiltinPost {
        BuiltinPost::new(
            Platform::Twitter,
            "https://x.com/kullanici/status/1",
            Author::new("Ad", "kullanici"),
        )
    }

    #[tokio::test]
    async fn cached_posts_are_returned_until_they_expire() {
        let key = "test-key-webui".to_string();
        assert!(get(&key).await.is_none());

        put(key.clone(), &post()).await;
        assert!(get(&key).await.is_some());

        // Farklı anahtar etkilenmez
        assert!(get("baska-anahtar").await.is_none());

        // Aynı anahtar tazelenebilir
        put(key.clone(), &post()).await;
        assert!(get(&key).await.is_some());
    }
}