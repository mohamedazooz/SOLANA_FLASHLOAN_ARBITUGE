//! Ordered RPC failover with per-provider token-bucket rate limiting.
//! Providers are attempted in configured priority order; no stale-cache or public-RPC fallback.
//! Detects HTTP 429 Too Many Requests, backs off failing endpoints,
//! and fails the request when all configured providers are unavailable.

use anyhow::{Context, Result, bail};
use parking_lot::RwLock;
use reqwest::Client;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue, USER_AGENT};
use serde_json::json;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Thread-safe Token Bucket Rate Limiter enforcing precise requests-per-second ceilings.
#[derive(Debug)]
pub struct TokenBucket {
    pub rate_per_sec: f64,
    pub capacity: f64,
    pub state: parking_lot::Mutex<(f64, Instant)>,
}

impl TokenBucket {
    pub fn new(rate_per_sec: f64, capacity: f64) -> Self {
        Self {
            rate_per_sec,
            capacity,
            state: parking_lot::Mutex::new((capacity, Instant::now())),
        }
    }

    /// Try acquire a permit. Returns true if acquired, false if exhausted.
    pub fn try_acquire(&self) -> bool {
        let mut guard = self.state.lock();
        let now = Instant::now();
        let elapsed = now.duration_since(guard.1).as_secs_f64();
        guard.0 = (guard.0 + elapsed * self.rate_per_sec).min(self.capacity);
        guard.1 = now;

        if guard.0 >= 1.0 {
            guard.0 -= 1.0;
            true
        } else {
            false
        }
    }

    /// Wait until a token is replenished (micro-sleep backoff).
    pub async fn acquire(&self) {
        loop {
            if self.try_acquire() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureStatus {
    Processed,
    Confirmed,
    Finalized,
    Failed,
    Pending,
}

#[derive(Clone)]
pub struct RpcPool {
    endpoints: Vec<String>,
    client: Client,
    backoffs: std::sync::Arc<RwLock<HashMap<usize, Instant>>>,
    limiters: std::sync::Arc<Vec<TokenBucket>>,
}

impl Default for RpcPool {
    fn default() -> Self {
        Self::from_env()
    }
}

impl RpcPool {
    /// Runtime JSON-RPC over HTTP is intentionally disabled for the Solana market-data
    /// plane. Bootstrap and live account state are supplied by Yellowstone gRPC; auxiliary
    /// Solana PubSub feeds use the configured WebSocket slots. `new()` remains available
    /// only for isolated tests and legacy code paths until they are migrated.
    pub fn from_env() -> Self {
        Self::new_with_limits(Vec::new(), 0.0, 0.0)
    }

    pub fn new(endpoints: Vec<String>) -> Self {
        Self::new_with_limits(endpoints, 20.0, 20.0)
    }

    fn new_with_limits(endpoints: Vec<String>, rate_per_sec: f64, burst: f64) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static("Sentinel-ArbBot-MultiRpc/1.0"),
        );

        let client = Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(45))
            .build()
            .unwrap_or_default();

        // Per-provider limits come from deployment configuration. Invalid/missing
        // production values produce zero permits and therefore fail closed.
        let limiters = endpoints
            .iter()
            .map(|_| TokenBucket::new(rate_per_sec, burst))
            .collect();

        Self {
            endpoints,
            client,
            backoffs: std::sync::Arc::new(RwLock::new(HashMap::new())),
            limiters: std::sync::Arc::new(limiters),
        }
    }

    pub fn endpoints_count(&self) -> usize {
        self.endpoints.len()
    }

    /// Dispatch JSON-RPC request with automated rate-limiting, 429 detection & failover.
    pub async fn call(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        let total = self.endpoints.len();
        if total == 0 {
            bail!("No RPC endpoints configured in pool");
        }

        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });

        let mut last_err = String::new();

        // Strict ordered failover: provider 0 is always tried first. Never rotate
        // the primary and never return cached/stale market data on failure.
        for idx in 0..total {
            let url = &self.endpoints[idx];
            let cooled_down = self
                .backoffs
                .read()
                .get(&idx)
                .is_some_and(|until| Instant::now() < *until);
            if cooled_down {
                last_err = format!("provider {idx} is in cooldown");
                continue;
            }
            if !self.limiters[idx].try_acquire() {
                last_err = format!("provider {idx} rate limit exhausted");
                self.backoffs
                    .write()
                    .insert(idx, Instant::now() + Duration::from_secs(1));
                continue;
            }

            match self.client.post(url).json(&body).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if status.as_u16() == 429 || status.is_server_error() {
                        self.backoffs
                            .write()
                            .insert(idx, Instant::now() + Duration::from_secs(2));
                        last_err = format!("HTTP {status} from provider {idx}");
                        continue;
                    }
                    if !status.is_success() {
                        last_err = format!("HTTP {status} from provider {idx}");
                        continue;
                    }
                    let val: serde_json::Value = match resp.json().await {
                        Ok(value) => value,
                        Err(error) => {
                            last_err = format!("invalid JSON from provider {idx}: {error}");
                            continue;
                        }
                    };
                    if let Some(error) = val.get("error") {
                        last_err = format!("RPC error from provider {idx}: {error}");
                        continue;
                    }
                    if val.get("result").is_none() {
                        last_err = format!("missing result from provider {idx}");
                        continue;
                    }
                    return Ok(val["result"].clone());
                }
                Err(error) => {
                    self.backoffs
                        .write()
                        .insert(idx, Instant::now() + Duration::from_secs(2));
                    last_err = format!("network error from provider {idx}: {error}");
                }
            }
        }

        bail!(
            "All {} RPC endpoints failed or rate-limited. Last error: {}",
            total,
            last_err
        );
    }

    /// Get current slot across the pool
    pub async fn get_slot(&self) -> Result<u64> {
        let res = self.call("getSlot", json!([])).await?;
        res.as_u64().context("Invalid slot format in response")
    }

    /// Get balance in lamports for a pubkey with processed commitment
    pub async fn get_balance(&self, pubkey: &solana_pubkey::Pubkey) -> Result<u64> {
        let res = self
            .call(
                "getBalance",
                json!([pubkey.to_string(), {"commitment": "processed"}]),
            )
            .await?;
        res["value"]
            .as_u64()
            .or_else(|| res.as_u64())
            .context("Invalid balance in RPC response")
    }

    /// Fetch latest blockhash with processed commitment
    pub async fn get_latest_blockhash(&self) -> Result<String> {
        let res = self
            .call("getLatestBlockhash", json!([{"commitment": "processed"}]))
            .await?;
        let bh = res["value"]["blockhash"]
            .as_str()
            .context("Missing blockhash in response")?;
        Ok(bh.to_string())
    }

    /// Fetch batch account info for multiple pools/vaults in a single call
    pub async fn get_multiple_accounts(
        &self,
        pubkeys: &[&str],
    ) -> Result<Vec<Option<serde_json::Value>>> {
        let res = self
            .call(
                "getMultipleAccounts",
                json!([pubkeys, {"encoding": "base64"}]),
            )
            .await?;
        let accounts = res["value"]
            .as_array()
            .context("Invalid accounts array format")?;

        Ok(accounts
            .iter()
            .map(|acc| {
                if acc.is_null() {
                    None
                } else {
                    Some(acc.clone())
                }
            })
            .collect())
    }

    /// Simulate transaction with processed commitment
    pub async fn simulate_transaction(&self, b64_tx: &str) -> Result<serde_json::Value> {
        let res = self
            .call(
                "simulateTransaction",
                json!([
                    b64_tx,
                    {
                        "encoding": "base64",
                        "commitment": "processed",
                        "replaceRecentBlockhash": true
                    }
                ]),
            )
            .await?;
        Ok(res["value"].clone())
    }

    /// Query a transaction signature. This is deliberately separate from Jito bundle IDs.
    pub async fn get_signature_status(&self, signature: &str) -> Result<SignatureStatus> {
        let res = self
            .call(
                "getSignatureStatuses",
                json!([[signature], {"searchTransactionHistory": true}]),
            )
            .await?;
        let Some(value) = res
            .get("value")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
        else {
            bail!("malformed getSignatureStatuses response");
        };
        if value.is_null() {
            return Ok(SignatureStatus::Pending);
        }
        if value.get("err").is_some_and(|err| !err.is_null()) {
            return Ok(SignatureStatus::Failed);
        }
        match value
            .get("confirmationStatus")
            .and_then(|v| v.as_str())
            .unwrap_or("processed")
        {
            "processed" => Ok(SignatureStatus::Processed),
            "confirmed" => Ok(SignatureStatus::Confirmed),
            "finalized" => Ok(SignatureStatus::Finalized),
            _ => Ok(SignatureStatus::Pending),
        }
    }

    /// Poll a direct transaction signature until finalized/failed or timeout.
    pub async fn poll_signature_until_terminal(
        &self,
        signature: &str,
        attempts: usize,
        interval: Duration,
    ) -> Result<SignatureStatus> {
        let mut last = SignatureStatus::Pending;
        for _ in 0..attempts {
            last = self.get_signature_status(signature).await?;
            match last {
                SignatureStatus::Finalized | SignatureStatus::Failed => return Ok(last),
                SignatureStatus::Processed
                | SignatureStatus::Confirmed
                | SignatureStatus::Pending => {}
            }
            tokio::time::sleep(interval).await;
        }
        Ok(last)
    }

    /// Broadcast signed transaction directly via RPC with processed commitment (SWQoS path).
    pub async fn send_raw_transaction(&self, b64_tx: &str) -> Result<String> {
        let res = self
            .call(
                "sendTransaction",
                json!([
                    b64_tx,
                    {
                        "encoding": "base64",
                        "skipPreflight": true,
                        "preflightCommitment": "processed",
                        "maxRetries": 0
                    }
                ]),
            )
            .await?;
        let sig = res
            .as_str()
            .context("RPC response missing transaction signature string")?;
        Ok(sig.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_bucket_permits_up_to_capacity() {
        let bucket = TokenBucket::new(20.0, 20.0);
        for _ in 0..20 {
            assert!(bucket.try_acquire());
        }
        // 21st must fail until replenishment
        assert!(!bucket.try_acquire());
    }

    #[test]
    fn test_rpc_pool_preserves_priority_and_limits() {
        let pool = RpcPool::new(vec![
            "http://127.0.0.1:8899".to_string(),
            "http://127.0.0.1:8898".to_string(),
        ]);
        assert_eq!(pool.endpoints_count(), 2);
        assert_eq!(pool.limiters.len(), 2);
    }
}
