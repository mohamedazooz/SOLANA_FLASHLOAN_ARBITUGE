//! Paced single-account RPC fetcher: sequential requests, strict semaphore
//! rate limits, inter-request pacing and automatic endpoint failover.

use anyhow::{Context, Result, bail};
use base64::prelude::*;
use solana_pubkey::Pubkey;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio::time::sleep;
use tracing::{debug, warn};

/// Cross-module counter of consecutive RPC failures (reset on every success).
pub(crate) static CONSECUTIVE_RPC_FAILURES: AtomicU64 = AtomicU64::new(0);

/// Thread-safe Paced Single-Account RPC Fetcher enforcing sequential requests,
/// strict semaphore rate limits, inter-request pacing, and automatic endpoint failover.
#[derive(Clone)]
pub struct PacedAccountFetcher {
    endpoints: Arc<Vec<String>>,
    client: reqwest::Client,
    semaphore: Arc<Semaphore>,
    min_interval: Duration,
    current_idx: Arc<AtomicUsize>,
    last_call: Arc<parking_lot::Mutex<Instant>>,
    endpoint_cooldowns: Arc<parking_lot::Mutex<std::collections::HashMap<usize, Instant>>>,
}

impl PacedAccountFetcher {
    pub fn new(endpoints: Vec<String>, max_concurrent: usize, min_interval_ms: u64) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(6))
            .connect_timeout(Duration::from_secs(3))
            .build()
            .unwrap_or_default();

        Self {
            endpoints: Arc::new(endpoints),
            client,
            semaphore: Arc::new(Semaphore::new(max_concurrent.max(1))),
            min_interval: Duration::from_millis(min_interval_ms),
            current_idx: Arc::new(AtomicUsize::new(0)),
            last_call: Arc::new(parking_lot::Mutex::new(
                Instant::now() - Duration::from_secs(1),
            )),
            endpoint_cooldowns: Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
        }
    }

    pub fn from_env() -> Self {
        let mut endpoints = Vec::new();
        let keys = [
            "ALCHEMY_RPC_URL",
            "SOLANA_RPC_PRIMARY",
            "SOLANA_RPC_FALLBACK_1",
            "SOLANA_RPC_FALLBACK_2",
            "SOLANA_RPC_1_URL",
            "SOLANA_RPC_2_URL",
            "SOLANA_RPC_3_URL",
            "SOLANA_RPC_4_URL",
            "SOLANA_RPC_5_URL",
            "SOLANA_RPC_6_URL",
            "SOLANA_RPC_7_URL",
            "SOLANA_RPC_URL",
            "HELIUS_RPC_URL",
            "SOLAMI_RPC_URL",
            "SOLANA_MAINNET_RPC_FALLBACK",
        ];
        for key in keys {
            if let Ok(val) = std::env::var(key) {
                let trimmed = val.trim().to_string();
                if !trimmed.is_empty()
                    && !trimmed.contains("REPLACE_ME")
                    && (trimmed.starts_with("https://") || trimmed.starts_with("http://"))
                    && !endpoints.contains(&trimmed)
                {
                    endpoints.push(trimmed);
                }
            }
        }
        if endpoints.is_empty() {
            tracing::warn!(
                "no RPC endpoints configured via env; falling back to the shared public endpoint https://api.mainnet-beta.solana.com — heavily rate-limited (HTTP 429 expected); NOT production-grade: configure ALCHEMY_RPC_URL / HELIUS_RPC_URL / SOLANA_RPC_*"
            );
            endpoints.push("https://api.mainnet-beta.solana.com".to_string());
        }

        // Prioritize high-throughput, generous-limit endpoints first (Alchemy and Shyft)
        endpoints.sort_by_key(|e| {
            if e.contains("alchemy.com") {
                0
            } else if e.contains("shyft.to") {
                1
            } else if e.contains("helius") {
                3
            } else {
                2
            }
        });

        let max_concurrent: usize = std::env::var("RPC_MAX_CONCURRENT_REQUESTS")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(4);

        let min_interval_ms: u64 = std::env::var("RPC_MIN_INTERVAL_MS")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(30);

        Self::new(endpoints, max_concurrent, min_interval_ms)
    }

    /// Process-shared instance (DATA-10): `from_env` builds a fresh
    /// `reqwest::Client` per call, which discards the previous connection pool
    /// and forces new TLS handshakes on every discovery/hydration use. The
    /// fetcher packs all state in `Arc` fields, so cloning is nearly free —
    /// one instance is initialized lazily from env and cloned everywhere.
    /// The shared semaphore also makes the global request pacing (interval +
    /// endpoint cooldowns) coherent across concurrent callers instead of
    /// multiplicative per construction site.
    pub fn shared() -> Self {
        static SHARED: std::sync::OnceLock<PacedAccountFetcher> = std::sync::OnceLock::new();
        SHARED.get_or_init(Self::from_env).clone()
    }

    /// Places endpoint `idx` on cooldown and emits the failure warning ONLY on
    /// the no-cooldown → cooldown state transition.
    ///
    /// Why: with `max_concurrent` in-flight requests, several tasks can select
    /// the same endpoint before any cooldown insert lands; each 429 response
    /// then hits this path. Under a rate-limited fallback this produced
    /// ~39,500 WARN lines in a 3-minute soak (~660/min). Gating the warn on
    /// the transition (checked under the same `endpoint_cooldowns` lock as the
    /// insert) collapses that to at most one line per cooldown cycle per
    /// endpoint (60s for HTTP 429/5xx, 15s for transport errors) while
    /// preserving the failover behavior exactly.
    fn cooldown_and_warn(&self, idx: usize, url: &str, cooldown: Duration, detail: String) {
        let mut guard = self.endpoint_cooldowns.lock();
        let was_active = guard
            .get(&idx)
            .is_some_and(|cooldown_end| Instant::now() < *cooldown_end);
        guard.insert(idx, Instant::now() + cooldown);
        drop(guard);
        if !was_active {
            warn!(
                "RPC endpoint {} failed ({}); placing on {}s cooldown and rotating",
                url,
                detail,
                cooldown.as_secs()
            );
        }
    }

    pub async fn fetch_single_account(&self, pubkey: &Pubkey) -> Result<(u64, Vec<u8>)> {
        let _permit = self.semaphore.acquire().await.context("semaphore closed")?;

        let now = Instant::now();
        let wait_dur = {
            let mut guard = self.last_call.lock();
            let elapsed = now.duration_since(*guard);
            if elapsed < self.min_interval {
                let wait = self.min_interval - elapsed;
                *guard = now + wait;
                Some(wait)
            } else {
                *guard = now;
                None
            }
        };
        if let Some(w) = wait_dur {
            sleep(w).await;
        }

        let total_endpoints = self.endpoints.len();
        let max_attempts = total_endpoints.saturating_mul(2).max(1);
        let mut last_err = anyhow::anyhow!("no endpoints available");

        for _ in 0..max_attempts {
            let idx = {
                let mut guard = self.endpoint_cooldowns.lock();
                let now = Instant::now();
                let mut chosen = None;
                for offset in 0..total_endpoints {
                    let candidate = (self.current_idx.fetch_add(1, Ordering::Relaxed) + offset)
                        % total_endpoints;
                    if let Some(cooldown_end) = guard.get(&candidate) {
                        if now < *cooldown_end {
                            continue; // skip endpoint during cooldown
                        } else {
                            guard.remove(&candidate);
                        }
                    }
                    chosen = Some(candidate);
                    break;
                }
                chosen.unwrap_or_else(|| self.current_idx.load(Ordering::Relaxed) % total_endpoints)
            };
            let url = &self.endpoints[idx];

            let req_body_zstd = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "getAccountInfo",
                "params": [
                    pubkey.to_string(),
                    {
                        "encoding": "base64+zstd",
                        "commitment": "processed"
                    }
                ]
            });

            let resp_res = self.client.post(url).json(&req_body_zstd).send().await;
            match resp_res {
                Ok(resp) => {
                    let status = resp.status();
                    if status.as_u16() == 429 || status.is_server_error() {
                        self.cooldown_and_warn(
                            idx,
                            url,
                            Duration::from_secs(60),
                            format!("HTTP {status}"),
                        );
                        continue;
                    }
                    if let Ok(json_val) = resp.json::<serde_json::Value>().await {
                        if let Some(err_obj) = json_val.get("error") {
                            debug!(
                                "RPC returned error {:?} on base64+zstd, falling back to base64",
                                err_obj
                            );
                            let req_body_b64 = serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": 1,
                                "method": "getAccountInfo",
                                "params": [
                                    pubkey.to_string(),
                                    {
                                        "encoding": "base64",
                                        "commitment": "processed"
                                    }
                                ]
                            });
                            if let Ok(b64_resp) =
                                self.client.post(url).json(&req_body_b64).send().await
                            {
                                if let Ok(b64_val) = b64_resp.json::<serde_json::Value>().await {
                                    return Self::parse_account_info_response(&b64_val, false);
                                }
                            }
                        } else {
                            match Self::parse_account_info_response(&json_val, true) {
                                Ok(res) => return Ok(res),
                                Err(e) => {
                                    last_err = e;
                                    break;
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    self.cooldown_and_warn(
                        idx,
                        url,
                        Duration::from_secs(15),
                        format!("transport error: {e}"),
                    );
                    last_err = anyhow::anyhow!(e);
                    sleep(Duration::from_millis(50)).await;
                }
            }
        }

        Err(last_err)
    }

    fn parse_account_info_response(
        val: &serde_json::Value,
        is_zstd: bool,
    ) -> Result<(u64, Vec<u8>)> {
        let slot = val
            .pointer("/result/context/slot")
            .and_then(|s| s.as_u64())
            .unwrap_or(0);

        let value_node = val.pointer("/result/value");
        if value_node.is_none() || value_node.unwrap().is_null() {
            bail!("account not found (null value)");
        }

        let raw_b64 = value_node
            .and_then(|v| v.get("data"))
            .and_then(|d| d.get(0))
            .and_then(|s| s.as_str())
            .context("missing data[0] in account info")?;

        let decoded_bytes = BASE64_STANDARD
            .decode(raw_b64)
            .context("base64 decode failed")?;

        if is_zstd {
            match zstd::decode_all(&decoded_bytes[..]) {
                Ok(decompressed) => Ok((slot, decompressed)),
                Err(_) => Ok((slot, decoded_bytes)),
            }
        } else {
            Ok((slot, decoded_bytes))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DATA-10: `shared()` must hand out clones of ONE lazily-initialized
    /// instance (Arc-packed state), never rebuild a `reqwest::Client`
    /// (and its TLS/connection pool) per call.
    #[test]
    fn shared_returns_clones_of_one_instance() {
        let a = PacedAccountFetcher::shared();
        let b = PacedAccountFetcher::shared();
        assert!(Arc::ptr_eq(&a.semaphore, &b.semaphore));
        assert!(Arc::ptr_eq(&a.endpoints, &b.endpoints));
        assert!(Arc::ptr_eq(&a.endpoint_cooldowns, &b.endpoint_cooldowns));
        assert_eq!(a.min_interval, b.min_interval);
    }
}
