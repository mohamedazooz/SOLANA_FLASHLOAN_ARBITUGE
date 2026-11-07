//! High-performance two-tier deduplication engine combining an in-memory `DashMap`
//! and an asynchronous distributed Redis connection manager (`SET NX PX`).
//!
//! Eliminates multi-provider race conditions (e.g. Helius vs Bloxroute) within sub-millisecond latencies.

use anyhow::Result;
use dashmap::DashMap;
use redis::aio::ConnectionManager;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

/// Deduplication coordinator managing local and distributed caches.
#[derive(Clone)]
pub struct DeduplicationEngine {
    /// Ultra-low latency in-memory signature cache storing timestamp of first observation.
    local_cache: Arc<DashMap<String, Instant>>,
    /// Asynchronous distributed Redis connection manager.
    redis_conn: Option<ConnectionManager>,
    /// Active local cache window duration.
    local_ttl: Duration,
    /// Redis distributed key expiration in milliseconds.
    redis_ttl_ms: u64,
}

impl DeduplicationEngine {
    /// Initializes the deduplication engine and connects to Redis if available.
    pub async fn new(
        redis_url: &str,
        local_ttl_secs: u64,
        redis_ttl_ms: u64,
    ) -> Result<Self> {
        let local_cache = Arc::new(DashMap::new());
        let local_ttl = Duration::from_secs(local_ttl_secs);

        let redis_conn = match redis::Client::open(redis_url) {
            Ok(client) => match ConnectionManager::new(client).await {
                Ok(manager) => {
                    info!(target: "dedup", "Connected successfully to Redis connection manager at {}", redis_url);
                    Some(manager)
                }
                Err(e) => {
                    warn!(target: "dedup", "Failed to connect to Redis: {}. Running with in-memory DashMap only.", e);
                    None
                }
            },
            Err(e) => {
                warn!(target: "dedup", "Invalid Redis URL: {}. Running with in-memory DashMap only.", e);
                None
            }
        };

        Ok(Self {
            local_cache,
            redis_conn,
            local_ttl,
            redis_ttl_ms,
        })
    }

    /// Spawns a background pruning task that evicts expired keys from the in-memory DashMap.
    pub fn start_pruning_task(&self, prune_interval: Duration) -> tokio::task::JoinHandle<()> {
        let cache = Arc::clone(&self.local_cache);
        let ttl = self.local_ttl;

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(prune_interval);
            loop {
                ticker.tick().await;
                let now = Instant::now();
                // Retain only entries within the active TTL window
                cache.retain(|_, &mut inserted_at| now.duration_since(inserted_at) < ttl);
                debug!(target: "dedup", current_entries = cache.len(), "Local DashMap pruned");
            }
        })
    }

    /// Evaluates if the signature is duplicate.
    ///
    /// Returns `Ok(true)` if the signature is already processed or being processed (Duplicate, MUST DROP).
    /// Returns `Ok(false)` if the signature is novel and successfully acquired (Process transaction).
    pub async fn is_duplicate_and_set(&self, signature: &str) -> Result<bool> {
        let now = Instant::now();

        // 1. Fast Path: Local DashMap In-Memory Check (< 50 nanoseconds)
        if let Some(entry) = self.local_cache.get(signature) {
            if now.duration_since(*entry.value()) < self.local_ttl {
                return Ok(true);
            }
        }

        // 2. Distributed Path: Atomic Redis SET key 1 NX PX ttl_ms
        if let Some(ref manager) = self.redis_conn {
            let mut conn = manager.clone();
            let key = format!("sol:sig:{}", signature);

            let res: redis::RedisResult<Option<String>> = redis::cmd("SET")
                .arg(&key)
                .arg("1")
                .arg("NX")
                .arg("PX")
                .arg(self.redis_ttl_ms)
                .query_async(&mut conn)
                .await;

            match res {
                Ok(Some(ref reply)) if reply == "OK" => {
                    // Successfully claimed the lock in Redis!
                    self.local_cache.insert(signature.to_string(), now);
                    Ok(false)
                }
                Ok(Some(_)) => {
                    // Non-OK reply means key was not set or already exists
                    self.local_cache.insert(signature.to_string(), now);
                    Ok(true)
                }
                Ok(None) => {
                    // Key already existed in Redis! Another provider pushed earlier.
                    self.local_cache.insert(signature.to_string(), now);
                    Ok(true)
                }
                Err(e) => {
                    error!(target: "dedup", error = %e, "Redis atomic CAS failed, falling back to local DashMap");
                    // Fallback to local DashMap only
                    let is_dup = self.local_cache.contains_key(signature);
                    self.local_cache.insert(signature.to_string(), now);
                    Ok(is_dup)
                }
            }
        } else {
            // Redis not enabled or unreachable: Pure DashMap atomic insertion
            use dashmap::mapref::entry::Entry;
            match self.local_cache.entry(signature.to_string()) {
                Entry::Occupied(mut occ) => {
                    if now.duration_since(*occ.get()) < self.local_ttl {
                        Ok(true)
                    } else {
                        occ.insert(now);
                        Ok(false)
                    }
                }
                Entry::Vacant(vac) => {
                    vac.insert(now);
                    Ok(false)
                }
            }
        }
    }
}
