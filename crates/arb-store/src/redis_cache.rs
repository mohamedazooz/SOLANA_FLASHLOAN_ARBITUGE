//! High-performance short-TTL Redis and In-Memory Cache with aggressive automatic eviction.
//! Ensures minimal memory footprint by clearing out stale pool and quote data immediately.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::{debug, info};

/// Cache entry with explicit timestamp for expiration.
#[derive(Debug, Clone)]
struct CacheEntry {
    value: Vec<u8>,
    expires_at: Instant,
}

/// Short-TTL Cache with proactive memory cleanup.
pub struct ShortTtlCache {
    default_ttl: Duration,
    entries: Mutex<HashMap<String, CacheEntry>>,
}

impl Default for ShortTtlCache {
    fn default() -> Self {
        Self::new(Duration::from_secs(10))
    }
}

impl ShortTtlCache {
    /// Create new cache with specific default TTL (e.g. 10 seconds).
    pub fn new(default_ttl: Duration) -> Self {
        Self {
            default_ttl,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Set a key with default TTL. Cleans up expired items opportunistically.
    pub fn set(&self, key: &str, value: Vec<u8>) {
        self.set_with_ttl(key, value, self.default_ttl);
    }

    /// Set a key with custom TTL in seconds.
    pub fn set_with_ttl(&self, key: &str, value: Vec<u8>, ttl: Duration) {
        let mut map = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();

        // Evict expired entries to maintain compact footprint
        map.retain(|_, entry| entry.expires_at > now);

        map.insert(
            key.to_string(),
            CacheEntry {
                value,
                expires_at: now + ttl,
            },
        );
    }

    /// Get value if not expired. If expired, immediately drops it.
    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        let mut map = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();

        if let Some(entry) = map.get(key)
            && entry.expires_at > now
        {
            return Some(entry.value.clone());
        }
        // Expired or missing: remove
        map.remove(key);
        None
    }

    /// Delete a key explicitly.
    pub fn delete(&self, key: &str) {
        let mut map = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        map.remove(key);
    }

    /// Proactively purge all expired entries to free RAM/disk space.
    pub fn prune_expired(&self) -> usize {
        let mut map = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        let initial_len = map.len();
        map.retain(|_, entry| entry.expires_at > now);
        let purged = initial_len - map.len();
        if purged > 0 {
            debug!("🧹 ShortTtlCache pruned {} expired entries", purged);
        }
        purged
    }

    /// Current number of unexpired entries.
    pub fn len(&self) -> usize {
        let mut map = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        map.retain(|_, entry| entry.expires_at > now);
        map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Redis-backed or short-TTL Cache Manager.
pub struct RedisStore {
    cache: ShortTtlCache,
    redis_url: Option<String>,
    redis_client: Option<redis::Client>,
}

impl RedisStore {
    pub fn new(redis_url: Option<String>, default_ttl_secs: u64) -> Self {
        let redis_client = if let Some(ref url) = redis_url {
            match redis::Client::open(url.as_str()) {
                Ok(client) => {
                    info!("Connected to live Redis Cache endpoint: {}", url);
                    Some(client)
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to initialize Redis client ({}), using local cache",
                        e
                    );
                    None
                }
            }
        } else {
            info!("Initialized In-Memory Short-TTL Cache with active space reclamation");
            None
        };

        Self {
            cache: ShortTtlCache::new(Duration::from_secs(default_ttl_secs)),
            redis_url,
            redis_client,
        }
    }

    pub fn set_json<T: serde::Serialize>(
        &self,
        key: &str,
        data: &T,
        ttl_secs: u64,
    ) -> anyhow::Result<()> {
        let json_str = serde_json::to_string(data)?;
        let bytes = json_str.as_bytes().to_vec();

        // 1. If Redis is available, persist with SETEX to auto-evict at the server level
        if let Some(ref client) = self.redis_client
            && let Ok(mut conn) = client.get_connection()
        {
            use redis::Commands;
            let _: Result<(), _> = conn.set_ex(key, &json_str, ttl_secs);
        }

        // 2. Also update local high-speed short-TTL cache
        self.cache
            .set_with_ttl(key, bytes, Duration::from_secs(ttl_secs));
        Ok(())
    }

    pub fn get_json<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<T> {
        // 1. Try local cache first for sub-millisecond retrieval
        if let Some(bytes) = self.cache.get(key)
            && let Ok(val) = serde_json::from_slice(&bytes)
        {
            return Some(val);
        }

        // 2. Fall back to live Redis server
        if let Some(ref client) = self.redis_client
            && let Ok(mut conn) = client.get_connection()
        {
            use redis::Commands;
            if let Ok(Some(json_str)) = conn.get::<_, Option<String>>(key)
                && let Ok(val) = serde_json::from_str::<T>(&json_str)
            {
                return Some(val);
            }
        }

        None
    }

    pub fn delete(&self, key: &str) {
        if let Some(ref client) = self.redis_client
            && let Ok(mut conn) = client.get_connection()
        {
            use redis::Commands;
            let _: Result<(), _> = conn.del(key);
        }
        self.cache.delete(key);
    }

    pub fn redis_url(&self) -> Option<&str> {
        self.redis_url.as_deref()
    }

    /// Whether a live Redis server connection is active (vs. local in-memory only).
    pub fn is_live_redis(&self) -> bool {
        self.redis_client.is_some()
    }

    /// Store raw bytes (e.g. a pre-warmed transaction wire template) under `key`.
    ///
    /// The bytes are base64-encoded so the Redis value stays a printable string,
    /// and the local short-TTL cache is updated too for a sub-ms read on the hot
    /// path. This is the persistence layer for pre-signed / pre-warmed bundles:
    /// the expensive build+sign work happens ahead of the slot and is restored
    /// (or shared across processes) without redoing it at fire time.
    pub fn set_bytes(&self, key: &str, data: &[u8], ttl_secs: u64) -> anyhow::Result<()> {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(data);
        if let Some(ref client) = self.redis_client
            && let Ok(mut conn) = client.get_connection()
        {
            use redis::Commands;
            let _: Result<(), _> = conn.set_ex(key, &encoded, ttl_secs);
        }
        self.cache
            .set_with_ttl(key, data.to_vec(), Duration::from_secs(ttl_secs));
        Ok(())
    }

    /// Retrieve raw bytes stored by `set_bytes`.
    pub fn get_bytes(&self, key: &str) -> Option<Vec<u8>> {
        // Local cache first (sub-millisecond, no network hop).
        if let Some(bytes) = self.cache.get(key) {
            return Some(bytes);
        }
        if let Some(ref client) = self.redis_client
            && let Ok(mut conn) = client.get_connection()
        {
            use base64::Engine;
            use redis::Commands;
            if let Ok(Some(encoded)) = conn.get::<_, Option<String>>(key)
                && let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(encoded)
            {
                return Some(bytes);
            }
        }
        None
    }

    /// Sub-millisecond deduplication guard for pool/quote cache entries.
    /// Returns `true` when the key is fresh (accepted) and `false` when a prior
    /// entry with the same key is still within its TTL (duplicate suppressed).
    pub fn try_dedupe(&self, key: &str, ttl_secs: u64) -> bool {
        // Local cache check is sub-millisecond (no network hop).
        if self.cache.get(key).is_some() {
            return false; // already seen within TTL
        }
        self.cache
            .set_with_ttl(key, Vec::new(), Duration::from_secs(ttl_secs));
        true
    }

    /// Proactively reclaim memory from expired cache entries.
    pub fn prune(&self) -> usize {
        self.cache.prune_expired()
    }

    /// Current number of unexpired local cache entries.
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_short_ttl_cache_eviction() {
        let cache = ShortTtlCache::new(Duration::from_millis(50));
        cache.set("pool_1", b"data_1".to_vec());
        assert_eq!(cache.get("pool_1"), Some(b"data_1".to_vec()));

        // Sleep to let TTL expire
        std::thread::sleep(Duration::from_millis(70));
        assert_eq!(cache.get("pool_1"), None);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn test_prune_expired() {
        let cache = ShortTtlCache::new(Duration::from_millis(20));
        cache.set("k1", b"v1".to_vec());
        cache.set("k2", b"v2".to_vec());
        std::thread::sleep(Duration::from_millis(30));
        let pruned = cache.prune_expired();
        assert_eq!(pruned, 2);
        assert!(cache.is_empty());
    }
}
