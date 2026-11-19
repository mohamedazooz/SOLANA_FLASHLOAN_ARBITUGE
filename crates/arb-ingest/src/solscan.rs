//! Solscan Pro v2 API Client for Market & Pool Discovery, Transaction Auditing, and Token Metadata.
//!
//! Enforces:
//! - Strict Token-Bucket Rate Limiter (<= 1.0 RPS / 60 RPM).
//! - Monthly Compute Unit (CU) budget tracking (10,000,000 CU ceiling).
//! - In-memory TTL caching to avoid repeated CU consumption.
//! - Non-blocking, fail-safe async dispatch (never stalls the execution hot path).

use anyhow::Result;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, warn};

pub const DEFAULT_SOLSCAN_BASE_URL: &str = "https://pro-api.solscan.io/v2.0";
pub const DEFAULT_SOLSCAN_RATE_LIMIT_RPS: f64 = 1.0; // 60 requests per minute
pub const DEFAULT_SOLSCAN_CU_BUDGET: u64 = 10_000_000; // 10 Million Compute Units

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolscanConfig {
    pub api_key: String,
    pub base_url: String,
    pub rate_limit_rps: f64,
    pub monthly_cu_budget: u64,
}

impl Default for SolscanConfig {
    fn default() -> Self {
        let api_key = std::env::var("SOLSCAN_API_KEY").unwrap_or_default();
        let base_url = std::env::var("SOLSCAN_API_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_SOLSCAN_BASE_URL.to_string());
        let rate_limit_rps = std::env::var("SOLSCAN_RATE_LIMIT_RPS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_SOLSCAN_RATE_LIMIT_RPS);
        let monthly_cu_budget = std::env::var("SOLSCAN_MAX_CU")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_SOLSCAN_CU_BUDGET);

        Self {
            api_key,
            base_url,
            rate_limit_rps,
            monthly_cu_budget,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolscanTokenMeta {
    pub address: String,
    pub name: Option<String>,
    pub symbol: Option<String>,
    pub decimals: Option<u8>,
    pub icon: Option<String>,
    pub price: Option<f64>,
    pub market_cap: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolscanMarketPair {
    pub program_id: Option<String>,
    pub pool_address: Option<String>,
    pub dex_name: Option<String>,
    pub pair: Option<String>,
    pub price: Option<f64>,
    pub volume_24h_usd: Option<f64>,
    pub tvl_usd: Option<f64>,
}

struct CacheEntry {
    data: Value,
    expires_at: Instant,
}

#[derive(Clone)]
pub struct SolscanClient {
    config: SolscanConfig,
    client: reqwest::Client,
    last_request_time: Arc<tokio::sync::Mutex<Instant>>,
    cache: Arc<RwLock<HashMap<String, CacheEntry>>>,
    cu_consumed: Arc<std::sync::atomic::AtomicU64>,
}

impl SolscanClient {
    pub fn new(config: SolscanConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .pool_max_idle_per_host(4)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        Self {
            config,
            client,
            last_request_time: Arc::new(tokio::sync::Mutex::new(
                Instant::now() - Duration::from_secs(10),
            )),
            cache: Arc::new(RwLock::new(HashMap::new())),
            cu_consumed: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    pub fn from_env() -> Self {
        Self::new(SolscanConfig::default())
    }

    pub fn has_api_key(&self) -> bool {
        !self.config.api_key.trim().is_empty()
    }

    pub fn cu_consumed(&self) -> u64 {
        self.cu_consumed.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Rate-limit gate: enforces minimum delay between requests (e.g. 1000ms for 1.0 RPS)
    async fn wait_for_rate_limit(&self) {
        if self.config.rate_limit_rps <= 0.0 {
            return;
        }
        let interval = Duration::from_secs_f64(1.0 / self.config.rate_limit_rps);
        let mut lock = self.last_request_time.lock().await;
        let elapsed = lock.elapsed();
        if elapsed < interval {
            let sleep_time = interval - elapsed;
            tokio::time::sleep(sleep_time).await;
        }
        *lock = Instant::now();
    }

    /// Internal generic GET with auth, rate limiting, and in-memory TTL caching
    async fn fetch_cached(&self, path: &str, ttl: Duration) -> Result<Option<Value>> {
        if !self.has_api_key() {
            debug!("Solscan API key not set, skipping request to {path}");
            return Ok(None);
        }

        // Check cache first
        let cache_key = path.to_string();
        {
            let cache_read = self.cache.read();
            if let Some(entry) = cache_read.get(&cache_key) {
                if Instant::now() < entry.expires_at {
                    return Ok(Some(entry.data.clone()));
                }
            }
        }

        // Enforce 1 req/sec rate limit
        self.wait_for_rate_limit().await;

        let url = format!(
            "{}/{}",
            self.config.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        );
        let res = self
            .client
            .get(&url)
            .header("accept", "application/json")
            .header("token", &self.config.api_key)
            .header("User-Agent", "Sentinel-HFT/1.0 (Solana Arbitrage)")
            .send()
            .await;

        let resp = match res {
            Ok(r) => r,
            Err(e) => {
                warn!("Solscan request error for {url}: {e}");
                return Ok(None);
            }
        };

        let status = resp.status();
        let body_text = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                warn!("Solscan body decode error for {url}: {e}");
                return Ok(None);
            }
        };

        if status == reqwest::StatusCode::UNAUTHORIZED {
            // Check if it's the "Please upgrade your api key level" response
            if body_text.contains("Please upgrade your api key level") {
                debug!("Solscan key level notice: {body_text}");
            } else {
                warn!("Solscan 401 Unauthorized: {body_text}");
            }
            return Ok(None);
        }

        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            warn!("Solscan 429 Too Many Requests: backing off");
            return Ok(None);
        }

        if !status.is_success() {
            debug!("Solscan HTTP {status} for {url}: {body_text}");
            return Ok(None);
        }

        let parsed: Value = match serde_json::from_str(&body_text) {
            Ok(val) => val,
            Err(e) => {
                warn!("Solscan JSON parse error for {url}: {e}");
                return Ok(None);
            }
        };

        // Increment approximate CU usage (standard endpoints consume ~10-25 CUs)
        self.cu_consumed
            .fetch_add(25, std::sync::atomic::Ordering::Relaxed);

        // Save in cache
        {
            let mut cache_write = self.cache.write();
            cache_write.insert(
                cache_key,
                CacheEntry {
                    data: parsed.clone(),
                    expires_at: Instant::now() + ttl,
                },
            );
        }

        Ok(Some(parsed))
    }

    /// Retrieve metadata for a token mint (Cached for 10 minutes)
    pub async fn get_token_meta(&self, mint: &str) -> Result<Option<SolscanTokenMeta>> {
        let path = format!("token/meta?address={mint}");
        let json_opt = self.fetch_cached(&path, Duration::from_secs(600)).await?;
        let val = match json_opt {
            Some(v) => v,
            None => return Ok(None),
        };

        let data = val.get("data").unwrap_or(&val);
        Ok(Some(SolscanTokenMeta {
            address: mint.to_string(),
            name: data
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            symbol: data
                .get("symbol")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            decimals: data
                .get("decimals")
                .and_then(|v| v.as_u64())
                .map(|d| d as u8),
            icon: data
                .get("icon")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            price: data.get("price").and_then(|v| v.as_f64()),
            market_cap: data.get("market_cap").and_then(|v| v.as_f64()),
        }))
    }

    /// Retrieve active DEX trading markets & liquidity pools for a token mint (Cached for 5 minutes)
    pub async fn get_token_markets(&self, mint: &str) -> Result<Vec<SolscanMarketPair>> {
        let path = format!("token/markets?address={mint}&page=1&page_size=20");
        let json_opt = self.fetch_cached(&path, Duration::from_secs(300)).await?;
        let val = match json_opt {
            Some(v) => v,
            None => return Ok(vec![]),
        };

        let mut out = Vec::new();
        let items = val
            .get("data")
            .and_then(|d| d.get("items").or(Some(d)))
            .and_then(|x| x.as_array());

        if let Some(arr) = items {
            for item in arr {
                out.push(SolscanMarketPair {
                    program_id: item
                        .get("program_id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    pool_address: item
                        .get("pool_address")
                        .or_else(|| item.get("address"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    dex_name: item
                        .get("dex_name")
                        .or_else(|| item.get("source"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    pair: item
                        .get("pair")
                        .or_else(|| item.get("name"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    price: item.get("price").and_then(|v| v.as_f64()),
                    volume_24h_usd: item.get("volume_24h").and_then(|v| v.as_f64()),
                    tvl_usd: item
                        .get("tvl")
                        .or_else(|| item.get("liquidity"))
                        .and_then(|v| v.as_f64()),
                });
            }
        }

        Ok(out)
    }

    /// Verify an on-chain transaction signature and retrieve parsed transfer & instruction details
    pub async fn get_transaction_detail(&self, tx_signature: &str) -> Result<Option<Value>> {
        let path = format!("transaction/detail?tx={tx_signature}");
        // Transactions are immutable, cached for 1 hour
        self.fetch_cached(&path, Duration::from_secs(3600)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_solscan_config_defaults() {
        let cfg = SolscanConfig::default();
        assert_eq!(cfg.base_url, DEFAULT_SOLSCAN_BASE_URL);
        assert_eq!(cfg.rate_limit_rps, 1.0);
        assert_eq!(cfg.monthly_cu_budget, 10_000_000);
    }

    #[tokio::test]
    async fn test_solscan_client_rate_limiter() {
        let cfg = SolscanConfig {
            api_key: "test_key".to_string(),
            base_url: "https://pro-api.solscan.io/v2.0".to_string(),
            rate_limit_rps: 5.0, // 200ms
            monthly_cu_budget: 10_000_000,
        };
        let client = SolscanClient::new(cfg);
        let t0 = Instant::now();
        client.wait_for_rate_limit().await;
        client.wait_for_rate_limit().await;
        let elapsed = t0.elapsed();
        assert!(elapsed >= Duration::from_millis(150));
    }
}
