//! Ordered RPC failover with per-provider token-bucket rate limiting, batch-size enforcement,
//! quota-exhaustion detection, and live telemetry.
//! Providers are attempted in configured priority order with round-robin balancing across healthy nodes.
//! Detects HTTP 429 Too Many Requests (`Retry-After` + `max usage` quota exhaustion), backs off
//! failing endpoints cleanly, and fails the request when all configured providers are unavailable.

use anyhow::{Context, Result, bail};
use parking_lot::RwLock;
use reqwest::Client;
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue, RETRY_AFTER, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
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

impl SignatureStatus {
    /// True when the transaction reached a confirmed or finalized block.
    pub fn is_landed(self) -> bool {
        matches!(self, Self::Confirmed | Self::Finalized)
    }

    /// True when polling may stop.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Confirmed | Self::Finalized | Self::Failed)
    }
}

/// Lifecycle state of an RPC endpoint.
/// Endpoints start in `Configured` and are NEVER labeled active until successfully probed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EndpointStatus {
    /// Endpoint is defined in configuration or environment, but has not yet been probed.
    Configured,
    /// Actively undergoing health probe check.
    Probed,
    /// Operating nominally: latency < 350ms, no 429s, no severe errors.
    Healthy,
    /// Operating with elevated latency (> 350ms), sporadic timeouts, or non-terminal errors.
    Degraded,
    /// Received HTTP 429 Too Many Requests or quota/credits exhaustion.
    QuotaExhausted,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MethodMetrics {
    pub call_count: u64,
    pub success_count: u64,
    pub error_count: u64,
    pub total_latency_ms: u64,
}

impl MethodMetrics {
    pub fn avg_latency_ms(&self) -> f64 {
        if self.call_count == 0 {
            0.0
        } else {
            self.total_latency_ms as f64 / self.call_count as f64
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub label: String,
    pub rps_limit: f64,
    pub burst_capacity: f64,
    pub max_batch_size: usize,
}

#[derive(Debug)]
struct ProviderTelemetry {
    config: ProviderConfig,
    status: RwLock<EndpointStatus>,
    is_probed: AtomicBool,
    requests_ok: AtomicU64,
    rate_limited_429: AtomicU64,
    timeouts: AtomicU64,
    errors_other: AtomicU64,
    retries_used: AtomicU64,
    recoveries: AtomicU64,
    quota_exhausted: AtomicBool,
    last_error: RwLock<Option<String>>,
    latencies_ms: parking_lot::Mutex<Vec<u32>>,
    method_metrics: parking_lot::Mutex<HashMap<String, MethodMetrics>>,
}

impl ProviderTelemetry {
    fn new(config: ProviderConfig) -> Self {
        Self {
            config,
            status: RwLock::new(EndpointStatus::Configured),
            is_probed: AtomicBool::new(false),
            requests_ok: AtomicU64::new(0),
            rate_limited_429: AtomicU64::new(0),
            timeouts: AtomicU64::new(0),
            errors_other: AtomicU64::new(0),
            retries_used: AtomicU64::new(0),
            recoveries: AtomicU64::new(0),
            quota_exhausted: AtomicBool::new(false),
            last_error: RwLock::new(None),
            latencies_ms: parking_lot::Mutex::new(Vec::with_capacity(256)),
            method_metrics: parking_lot::Mutex::new(HashMap::new()),
        }
    }

    fn record_call(&self, method: &str, elapsed_ms: u32, ok: bool, is_429: bool, is_timeout: bool) {
        {
            let mut lats = self.latencies_ms.lock();
            if lats.len() >= 256 {
                lats.remove(0);
            }
            lats.push(elapsed_ms);
        }
        {
            let mut methods = self.method_metrics.lock();
            let entry = methods.entry(method.to_string()).or_default();
            entry.call_count += 1;
            entry.total_latency_ms += elapsed_ms as u64;
            if ok {
                entry.success_count += 1;
            } else {
                entry.error_count += 1;
            }
        }
        if is_timeout {
            self.timeouts.fetch_add(1, Ordering::Relaxed);
        }
        if is_429 {
            self.rate_limited_429.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn latency_percentiles(&self) -> (u32, u32, u32) {
        let guard = self.latencies_ms.lock();
        if guard.is_empty() {
            return (0, 0, 0);
        }
        let mut sorted = guard.clone();
        sorted.sort_unstable();
        let len = sorted.len();
        let p50 = sorted[(len * 50) / 100];
        let p95 = sorted[((len * 95) / 100).min(len - 1)];
        let p99 = sorted[((len * 99) / 100).min(len - 1)];
        (p50, p95, p99)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderHealthReport {
    pub label: String,
    pub status: EndpointStatus,
    pub rps_limit: f64,
    pub max_batch_size: usize,
    pub ok_count: u64,
    pub rate_limited_429: u64,
    pub error_count: u64,
    pub timeout_count: u64,
    pub retry_count: u64,
    pub recovery_count: u64,
    pub p50_latency_ms: u32,
    pub p95_latency_ms: u32,
    pub p99_latency_ms: u32,
    pub cooldown_remaining_secs: u64,
    pub quota_exhausted: bool,
    pub last_error: Option<String>,
    pub method_metrics: HashMap<String, MethodMetrics>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcPoolHealth {
    pub total: usize,
    pub configured_count: usize,
    pub probed_count: usize,
    pub healthy_count: usize,
    pub degraded_count: usize,
    pub quota_exhausted_count: usize,
    pub in_cooldown: usize,
    pub total_ok: u64,
    pub total_429: u64,
    pub total_errors: u64,
    pub providers: Vec<ProviderHealthReport>,
    /// Backwards compatibility: count of healthy endpoints
    pub healthy: usize,
}

#[derive(Clone)]
pub struct RpcPool {
    endpoints: Vec<String>,
    sender_endpoint: Option<String>,
    client: Client,
    backoffs: Arc<RwLock<HashMap<usize, Instant>>>,
    limiters: Arc<Vec<TokenBucket>>,
    telemetry: Arc<Vec<ProviderTelemetry>>,
    next_index: Arc<AtomicUsize>,
}

impl Default for RpcPool {
    fn default() -> Self {
        Self::from_env()
    }
}

impl RpcPool {
    fn key_suffix_hint(url: &str) -> Option<String> {
        let clean = url.trim_end_matches('/');
        if let Some((_, q)) = clean.split_once("api_key=") {
            let k = q.split('&').next().unwrap_or("");
            if k.len() >= 4 {
                return Some(k[k.len() - 4..].to_string());
            }
        } else if let Some((_, q)) = clean.split_once("api-key=") {
            let k = q.split('&').next().unwrap_or("");
            if k.len() >= 4 {
                return Some(k[k.len() - 4..].to_string());
            }
        } else if let Some((_, k)) = clean.rsplit_once("/v2/") {
            if k.len() >= 4 {
                return Some(k[k.len() - 4..].to_string());
            }
        }
        None
    }

    /// Classify provider capacity based on live measured benchmarks:
    /// - Alchemy: 20 req/s sustained, burst 25, max batch 100
    /// - RPCFast: 25 req/s sustained, burst 30, max batch 5 (hard RPC cap `-32602 max 5`)
    /// - Shyft: 8 req/s sustained, burst 10, max batch 100 (429 observed at 15-20 rps)
    /// - Solami `/Solana`: 2.0 req/s sustained, burst 3, max batch 100 (429 observed >=4 rps)
    /// - Solami `/sol`: 3.0 req/s sustained, burst 4, max batch 100 (429 observed at 5 rps)
    /// - Helius: 12 req/s sustained, burst 15, max batch 100
    pub fn classify_provider(idx: usize, url: &str) -> ProviderConfig {
        let lc = url.to_ascii_lowercase();
        let hint = Self::key_suffix_hint(url)
            .map(|h| format!("..{h}"))
            .unwrap_or_else(|| format!("#{}", idx + 1));

        if lc.contains("alchemy.com") {
            ProviderConfig {
                label: format!("Alchemy({hint})"),
                rps_limit: 20.0,
                burst_capacity: 25.0,
                max_batch_size: 100,
            }
        } else if lc.contains("rpcfast.com") {
            ProviderConfig {
                label: format!("RPCFast({hint})"),
                rps_limit: 25.0,
                burst_capacity: 30.0,
                max_batch_size: 5,
            }
        } else if lc.contains("shyft.to") {
            ProviderConfig {
                label: format!("Shyft({hint})"),
                rps_limit: 8.0,
                burst_capacity: 10.0,
                max_batch_size: 100,
            }
        } else if lc.contains("solami.dev/solana") {
            ProviderConfig {
                label: format!("Solami-Solana({hint})"),
                rps_limit: 2.0,
                burst_capacity: 3.0,
                max_batch_size: 100,
            }
        } else if lc.contains("solami.dev") {
            ProviderConfig {
                label: format!("Solami-sol({hint})"),
                rps_limit: 3.0,
                burst_capacity: 4.0,
                max_batch_size: 100,
            }
        } else if lc.contains("helius-rpc.com") {
            ProviderConfig {
                label: format!("Helius({hint})"),
                rps_limit: 12.0,
                burst_capacity: 15.0,
                max_batch_size: 100,
            }
        } else {
            let host = url.split('/').nth(2).unwrap_or("CustomRPC");
            ProviderConfig {
                label: format!("{host}(#{})", idx + 1),
                rps_limit: 10.0,
                burst_capacity: 15.0,
                max_batch_size: 100,
            }
        }
    }

    /// Collect and deduplicate RPC endpoints from environment lookup.
    pub fn collect_rpc_urls<F>(lookup: F) -> Vec<String>
    where
        F: Fn(&str) -> Option<String>,
    {
        let mut endpoints = Vec::new();
        let mut push_unique = |raw: &str| {
            let trimmed = raw.trim();
            if !trimmed.is_empty()
                && !trimmed.starts_with("REPLACE_ME")
                && !trimmed.contains("REPLACE_ME")
                && (trimmed.starts_with("https://") || trimmed.starts_with("http://"))
                && !endpoints.iter().any(|u| u == trimmed)
            {
                endpoints.push(trimmed.to_string());
            }
        };

        // 1. Explicit numbered priority slots (SOLANA_RPC_1_URL .. SOLANA_RPC_10_URL)
        for n in 1..=10 {
            let key = format!("SOLANA_RPC_{n}_URL");
            if let Some(val) = lookup(&key) {
                push_unique(&val);
            }
        }

        // 2. FAST_RPC_API / FAST_ENDPOINT (supports either full URL or raw RPCFast API key)
        if let Some(fast_api) = lookup("FAST_RPC_API") {
            let k = fast_api.trim();
            if !k.is_empty() && !k.contains("REPLACE_ME") {
                if k.starts_with("http") {
                    push_unique(k);
                } else {
                    push_unique(&format!("https://solana-rpc.rpcfast.com/?api_key={k}"));
                }
            }
        }
        if let Some(fast_ep) = lookup("FAST_ENDPOINT") {
            push_unique(&fast_ep);
        }

        // 3. Alchemy keys (synthesized if not already present via numbered slots)
        for alch_key in ["ALCHEMY_API_KEY", "alchemyapi"] {
            if let Some(val) = lookup(alch_key) {
                let k = val.trim();
                if !k.is_empty() && !k.contains("REPLACE_ME") {
                    if k.starts_with("http") {
                        push_unique(k);
                    } else {
                        push_unique(&format!("https://solana-mainnet.g.alchemy.com/v2/{k}"));
                    }
                }
            }
        }

        // 4. Shyft fallback & key
        if let Some(shyft_fb) = lookup("SOLANA_MAINNET_RPC_FALLBACK") {
            push_unique(&shyft_fb);
        }
        if let Some(shyft_key) = lookup("SHYFT_API_KEY") {
            let k = shyft_key.trim();
            if !k.is_empty() && !k.contains("REPLACE_ME") && !k.starts_with("http") {
                push_unique(&format!("https://rpc.fra.shyft.to?api_key={k}"));
            }
        }

        // 5. Solami RPC variants
        for key in ["SOLAMI_RPC_URL", "solamrpc"] {
            if let Some(solami_url) = lookup(key) {
                let s = solami_url.trim();
                if s.starts_with("http") {
                    push_unique(s);
                } else if !s.is_empty() && !s.contains("REPLACE_ME") {
                    push_unique(&format!("https://rpc.solami.dev/sol?api_key={s}"));
                }
            }
        }
        if let Some(solami_key) = lookup("SOLAMI_API_KEY") {
            let k = solami_key.trim();
            if !k.is_empty() && !k.contains("REPLACE_ME") && !k.starts_with("http") {
                push_unique(&format!("https://rpc.solami.dev/Solana?api_key={k}"));
            }
        }

        // 6. General / Helius fallback keys
        for key in ["SOLANA_RPC_URL", "RPC_URL", "HELIUS_RPC_URL", "HELIUS_API"] {
            if let Some(val) = lookup(key) {
                push_unique(&val);
            }
        }
        if let Some(helius_key) = lookup("HELIUS_API_KEY") {
            let k = helius_key.trim();
            if !k.is_empty() && !k.contains("REPLACE_ME") {
                let clean_key = if let Some(p) = k.split("api-key=").nth(1) {
                    p.split('&').next().unwrap_or(p)
                } else {
                    k
                };
                if !clean_key.is_empty() && !clean_key.starts_with("http") {
                    push_unique(&format!(
                        "https://mainnet.helius-rpc.com/?api-key={clean_key}"
                    ));
                }
            }
        }

        endpoints
    }

    /// Initialize multi-RPC pool from environment with per-provider rate limits
    /// and round-robin switching (load balancing across healthy providers).
    pub fn from_env() -> Self {
        dotenvy::dotenv().ok();
        let endpoints = Self::collect_rpc_urls(|k| std::env::var(k).ok());

        let sender_endpoint = std::env::var("HELIUS_SENDER_REGIONAL")
            .ok()
            .or_else(|| std::env::var("HELIUS_SENDER_URL").ok())
            .map(|u| {
                let s = u.trim();
                if !s.ends_with("/fast") && !s.contains("/fast?") {
                    format!("{s}/fast")
                } else {
                    s.to_string()
                }
            })
            .filter(|u| !u.is_empty() && !u.starts_with("REPLACE_ME") && u.starts_with("http"));

        Self::new_with_per_provider_limits(endpoints, sender_endpoint)
    }

    pub fn new(endpoints: Vec<String>) -> Self {
        Self::new_with_per_provider_limits(endpoints, None)
    }

    fn new_with_per_provider_limits(
        endpoints: Vec<String>,
        sender_endpoint: Option<String>,
    ) -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static("Sentinel-ArbBot-MultiRpc/1.1"),
        );

        let client = Client::builder()
            .default_headers(headers)
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_default();

        let configs: Vec<ProviderConfig> = endpoints
            .iter()
            .enumerate()
            .map(|(idx, url)| Self::classify_provider(idx, url))
            .collect();

        let limiters = configs
            .iter()
            .map(|cfg| TokenBucket::new(cfg.rps_limit, cfg.burst_capacity))
            .collect();

        let telemetry = configs.into_iter().map(ProviderTelemetry::new).collect();

        Self {
            endpoints,
            sender_endpoint,
            client,
            backoffs: Arc::new(RwLock::new(HashMap::new())),
            limiters: Arc::new(limiters),
            telemetry: Arc::new(telemetry),
            next_index: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn endpoints_count(&self) -> usize {
        self.endpoints.len()
    }

    pub fn endpoints(&self) -> &[String] {
        &self.endpoints
    }

    pub fn provider_labels(&self) -> Vec<String> {
        self.telemetry
            .iter()
            .map(|t| {
                format!(
                    "{}[{:.0}rps/b{}]",
                    t.config.label, t.config.rps_limit, t.config.max_batch_size
                )
            })
            .collect()
    }

    /// Return ordered endpoints suitable for transaction simulation:
    /// healthy batch-100 / sim-capable providers first, then other non-exhausted providers.
    pub fn healthy_endpoints_for_sim(&self) -> Vec<String> {
        let now = Instant::now();
        let backoffs = self.backoffs.read();
        let mut active = Vec::new();
        let mut fallback = Vec::new();
        for (idx, url) in self.endpoints.iter().enumerate() {
            let exhausted = self.telemetry[idx].quota_exhausted.load(Ordering::Acquire);
            if exhausted {
                continue;
            }
            let in_cooldown = backoffs.get(&idx).is_some_and(|until| now < *until);
            if !in_cooldown {
                active.push(url.clone());
            } else {
                fallback.push(url.clone());
            }
        }
        active.extend(fallback);
        if active.is_empty() {
            self.endpoints.clone()
        } else {
            active
        }
    }

    pub fn health_snapshot(&self) -> RpcPoolHealth {
        let now = Instant::now();
        let backoffs = self.backoffs.read();
        let mut providers = Vec::with_capacity(self.endpoints.len());
        let mut configured_count = 0usize;
        let mut probed_count = 0usize;
        let mut healthy_count = 0usize;
        let mut degraded_count = 0usize;
        let mut quota_exhausted_count = 0usize;
        let mut in_cooldown = 0usize;
        let mut total_ok = 0u64;
        let mut total_429 = 0u64;
        let mut total_errors = 0u64;

        for (idx, tel) in self.telemetry.iter().enumerate() {
            let ok_count = tel.requests_ok.load(Ordering::Relaxed);
            let r429 = tel.rate_limited_429.load(Ordering::Relaxed);
            let errs = tel.errors_other.load(Ordering::Relaxed);
            let timeouts = tel.timeouts.load(Ordering::Relaxed);
            let retries = tel.retries_used.load(Ordering::Relaxed);
            let recoveries = tel.recoveries.load(Ordering::Relaxed);
            let is_probed = tel.is_probed.load(Ordering::Acquire);
            let exhausted = tel.quota_exhausted.load(Ordering::Acquire);
            let cooldown_remaining_secs = backoffs
                .get(&idx)
                .and_then(|until| until.checked_duration_since(now))
                .map(|d| d.as_secs())
                .unwrap_or(0);

            total_ok = total_ok.saturating_add(ok_count);
            total_429 = total_429.saturating_add(r429);
            total_errors = total_errors.saturating_add(errs).saturating_add(timeouts);

            let status = if !is_probed {
                configured_count += 1;
                EndpointStatus::Configured
            } else if exhausted {
                probed_count += 1;
                quota_exhausted_count += 1;
                EndpointStatus::QuotaExhausted
            } else if cooldown_remaining_secs > 0 {
                probed_count += 1;
                in_cooldown += 1;
                degraded_count += 1;
                EndpointStatus::Degraded
            } else {
                probed_count += 1;
                let cur = *tel.status.read();
                match cur {
                    EndpointStatus::Healthy => healthy_count += 1,
                    EndpointStatus::Degraded => degraded_count += 1,
                    EndpointStatus::QuotaExhausted => quota_exhausted_count += 1,
                    _ => {
                        healthy_count += 1;
                    }
                }
                cur
            };

            let (p50, p95, p99) = tel.latency_percentiles();
            let method_metrics = tel.method_metrics.lock().clone();

            providers.push(ProviderHealthReport {
                label: tel.config.label.clone(),
                status,
                rps_limit: tel.config.rps_limit,
                max_batch_size: tel.config.max_batch_size,
                ok_count,
                rate_limited_429: r429,
                error_count: errs,
                timeout_count: timeouts,
                retry_count: retries,
                recovery_count: recoveries,
                p50_latency_ms: p50,
                p95_latency_ms: p95,
                p99_latency_ms: p99,
                cooldown_remaining_secs,
                quota_exhausted: exhausted,
                last_error: tel.last_error.read().clone(),
                method_metrics,
            });
        }

        RpcPoolHealth {
            total: self.endpoints.len(),
            configured_count,
            probed_count,
            healthy_count,
            degraded_count,
            quota_exhausted_count,
            in_cooldown,
            total_ok,
            total_429,
            total_errors,
            providers,
            healthy: healthy_count,
        }
    }

    pub fn format_compact_status(&self) -> String {
        let h = self.health_snapshot();
        if h.total == 0 {
            return "No endpoints configured".to_string();
        }
        let exhausted_names: Vec<String> = h
            .providers
            .iter()
            .filter(|p| p.status == EndpointStatus::QuotaExhausted)
            .map(|p| p.label.clone())
            .collect();
        let cooling_names: Vec<String> = h
            .providers
            .iter()
            .filter(|p| p.status != EndpointStatus::QuotaExhausted && p.cooldown_remaining_secs > 0)
            .map(|p| format!("{}:{}s", p.label, p.cooldown_remaining_secs))
            .collect();

        let mut parts = vec![format!(
            "{}/{} Healthy ({}/{} Probed)",
            h.healthy_count, h.total, h.probed_count, h.total
        )];
        parts.push(format!("OK:{}", h.total_ok));
        if h.total_429 > 0 {
            parts.push(format!("429:{}", h.total_429));
        }
        if h.degraded_count > 0 {
            parts.push(format!("Degraded:{}", h.degraded_count));
        }
        if !cooling_names.is_empty() {
            parts.push(format!("Cool:[{}]", cooling_names.join(",")));
        }
        if !exhausted_names.is_empty() {
            parts.push(format!("QuotaDead:[{}]", exhausted_names.join(",")));
        }
        parts.join(" | ")
    }

    pub fn redact_url(url: &str) -> String {
        if let Some((base, _)) = url.split_once('?') {
            format!("{base}?***")
        } else if let Some((prefix, _)) = url.rsplit_once("/v2/") {
            format!("{prefix}/v2/***")
        } else {
            url.to_string()
        }
    }

    pub fn redacted_hosts(&self) -> Vec<String> {
        self.endpoints
            .iter()
            .map(|u| u.split('/').nth(2).unwrap_or("unknown").to_string())
            .collect()
    }

    fn record_error(
        &self,
        idx: usize,
        method: &str,
        elapsed_ms: u32,
        err: String,
        is_429: bool,
        exhausted: bool,
        is_timeout: bool,
        backoff_secs: u64,
    ) {
        if backoff_secs > 0 {
            self.backoffs
                .write()
                .insert(idx, Instant::now() + Duration::from_secs(backoff_secs));
        }
        if let Some(tel) = self.telemetry.get(idx) {
            tel.is_probed.store(true, Ordering::Release);
            tel.record_call(method, elapsed_ms, false, is_429, is_timeout);
            if !is_429 {
                tel.errors_other.fetch_add(1, Ordering::Relaxed);
            }
            if exhausted || is_429 {
                tel.quota_exhausted.store(true, Ordering::Release);
                *tel.status.write() = EndpointStatus::QuotaExhausted;
            } else {
                *tel.status.write() = EndpointStatus::Degraded;
            }
            *tel.last_error.write() = Some(err);
        }
    }

    fn record_success(&self, idx: usize, method: &str, elapsed_ms: u32) {
        if let Some(tel) = self.telemetry.get(idx) {
            tel.is_probed.store(true, Ordering::Release);
            tel.record_call(method, elapsed_ms, true, false, false);
            tel.requests_ok.fetch_add(1, Ordering::Relaxed);
            if tel.quota_exhausted.swap(false, Ordering::AcqRel) {
                tel.recoveries.fetch_add(1, Ordering::Relaxed);
                *tel.last_error.write() = None;
            }
            let (p50, _, _) = tel.latency_percentiles();
            let new_status = if p50 > 350 {
                EndpointStatus::Degraded
            } else {
                EndpointStatus::Healthy
            };
            *tel.status.write() = new_status;
        }
    }

    /// Dispatch JSON-RPC request with automated rate-limiting, 429 detection & round-robin failover.
    pub async fn call(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        self.call_filtered_idx(method, params, |_, _| true).await
    }

    /// Dispatch JSON-RPC request across endpoints matching `allow_endpoint` URL predicate.
    pub async fn call_filtered<F>(
        &self,
        method: &str,
        params: serde_json::Value,
        allow_endpoint: F,
    ) -> Result<serde_json::Value>
    where
        F: Fn(&str) -> bool,
    {
        self.call_filtered_idx(method, params, |_, url| allow_endpoint(url))
            .await
    }

    async fn call_filtered_idx<F>(
        &self,
        method: &str,
        params: serde_json::Value,
        allow_fn: F,
    ) -> Result<serde_json::Value>
    where
        F: Fn(usize, &str) -> bool,
    {
        let total = self.endpoints.len();
        if total == 0 {
            bail!("No RPC endpoints configured in pool");
        }
        let matching_count = self
            .endpoints
            .iter()
            .enumerate()
            .filter(|(idx, u)| allow_fn(*idx, u))
            .count();
        if matching_count == 0 {
            bail!("No matching RPC endpoints for filter in pool of {total}");
        }

        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });

        let mut last_err = String::new();
        let mut attempted = vec![false; total];

        // Round-robin load balancing: alternate starting endpoint across healthy providers
        let start_idx = self.next_index.fetch_add(1, Ordering::Relaxed) % total;

        for pass in 0..2 {
            for step in 0..total {
                let idx = (start_idx + step) % total;
                if attempted[idx] {
                    continue;
                }
                let url = &self.endpoints[idx];
                if !allow_fn(idx, url) {
                    continue;
                }
                let label = self
                    .telemetry
                    .get(idx)
                    .map(|t| t.config.label.as_str())
                    .unwrap_or("RPC");

                let cooled_down = self
                    .backoffs
                    .read()
                    .get(&idx)
                    .is_some_and(|until| Instant::now() < *until);
                if cooled_down {
                    last_err = format!("{label} in cooldown");
                    continue;
                }

                // Prefer non-exhausted endpoints first; only probe a previously quota-exhausted
                // endpoint once its long cooldown expires and only on pass 1 if non-exhausted failed.
                let is_quota_dead = self
                    .telemetry
                    .get(idx)
                    .is_some_and(|t| t.quota_exhausted.load(Ordering::Acquire));
                if pass == 0 && is_quota_dead {
                    last_err = format!("{label} quota exhausted");
                    continue;
                }

                if !self.limiters[idx].try_acquire() {
                    if pass == 0 {
                        last_err = format!("{label} token bucket momentarily empty");
                        continue;
                    }
                    self.limiters[idx].acquire().await;
                }

                attempted[idx] = true;
                let start_req = Instant::now();

                match self.client.post(url).json(&body).send().await {
                    Ok(resp) => {
                        let elapsed_ms = start_req.elapsed().as_millis() as u32;
                        let status = resp.status();
                        let retry_after_secs = resp
                            .headers()
                            .get(RETRY_AFTER)
                            .and_then(|v| v.to_str().ok())
                            .and_then(|s| s.trim().parse::<u64>().ok())
                            .map(|s| s.clamp(1, 120));

                        if status.as_u16() == 429 {
                            let body_txt = resp.text().await.unwrap_or_default();
                            let body_lc = body_txt.to_ascii_lowercase();
                            let exhausted = body_lc.contains("max usage")
                                || body_lc.contains("quota")
                                || body_lc.contains("credits")
                                || body_lc.contains("exhausted");
                            let backoff_secs = if exhausted {
                                1800
                            } else {
                                retry_after_secs.unwrap_or(5)
                            };
                            let msg = if exhausted {
                                format!("HTTP 429 Quota Exhausted ({label}): max usage reached")
                            } else {
                                format!("HTTP 429 Rate Limited ({label}, backoff {backoff_secs}s)")
                            };
                            self.record_error(
                                idx,
                                method,
                                elapsed_ms,
                                msg.clone(),
                                true,
                                exhausted,
                                false,
                                backoff_secs,
                            );
                            last_err = msg;
                            continue;
                        }
                        if status.as_u16() == 403 || status.as_u16() == 401 {
                            let msg = format!("HTTP {status} Auth/Quota error ({label})");
                            self.record_error(
                                idx,
                                method,
                                elapsed_ms,
                                msg.clone(),
                                false,
                                true,
                                false,
                                600,
                            );
                            last_err = msg;
                            continue;
                        }
                        if status.is_server_error() || !status.is_success() {
                            let msg = format!("HTTP {status} ({label})");
                            self.record_error(
                                idx,
                                method,
                                elapsed_ms,
                                msg.clone(),
                                false,
                                false,
                                false,
                                3,
                            );
                            last_err = msg;
                            continue;
                        }
                        let val: serde_json::Value = match resp.json().await {
                            Ok(value) => value,
                            Err(error) => {
                                let msg = format!("invalid JSON ({label}): {error}");
                                self.record_error(
                                    idx,
                                    method,
                                    elapsed_ms,
                                    msg.clone(),
                                    false,
                                    false,
                                    false,
                                    2,
                                );
                                last_err = msg;
                                continue;
                            }
                        };
                        if let Some(error) = val.get("error") {
                            let err_str = error.to_string();
                            let err_lc = err_str.to_ascii_lowercase();
                            let is_rl = err_lc.contains("rate")
                                || err_lc.contains("limit")
                                || err_lc.contains("-32005")
                                || err_lc.contains("429");
                            let backoff_secs = if is_rl {
                                retry_after_secs.unwrap_or(5)
                            } else {
                                0
                            };
                            let msg = format!("RPC error ({label}): {err_str}");
                            if is_rl {
                                self.record_error(
                                    idx,
                                    method,
                                    elapsed_ms,
                                    msg.clone(),
                                    true,
                                    false,
                                    false,
                                    backoff_secs,
                                );
                            } else {
                                self.record_error(
                                    idx,
                                    method,
                                    elapsed_ms,
                                    msg.clone(),
                                    false,
                                    false,
                                    false,
                                    0,
                                );
                            }
                            last_err = msg;
                            continue;
                        }
                        if val.get("result").is_none() {
                            let msg = format!("missing result ({label})");
                            self.record_error(
                                idx,
                                method,
                                elapsed_ms,
                                msg.clone(),
                                false,
                                false,
                                false,
                                1,
                            );
                            last_err = msg;
                            continue;
                        }
                        self.record_success(idx, method, elapsed_ms);
                        return Ok(val["result"].clone());
                    }
                    Err(error) => {
                        let elapsed_ms = start_req.elapsed().as_millis() as u32;
                        let msg = format!("network error ({label}): {error}");
                        self.record_error(
                            idx,
                            method,
                            elapsed_ms,
                            msg.clone(),
                            false,
                            false,
                            error.is_timeout(),
                            2,
                        );
                        last_err = msg;
                    }
                }
            }
            if pass == 0 {
                let has_retry_candidate = self.endpoints.iter().enumerate().any(|(i, u)| {
                    allow_fn(i, u)
                        && !attempted[i]
                        && !self
                            .backoffs
                            .read()
                            .get(&i)
                            .is_some_and(|until| Instant::now() < *until)
                });
                if !has_retry_candidate {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
        }

        bail!(
            "All {} matching RPC endpoints failed or rate-limited. Last error: {}",
            matching_count,
            last_err
        );
    }

    /// Get current slot across the pool with processed commitment
    pub async fn get_slot(&self) -> Result<u64> {
        let res = self
            .call("getSlot", json!([{"commitment": "processed"}]))
            .await?;
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

    /// Fetch batch account info and latest chain context slot.
    /// Routes 100-account batches to endpoints with `max_batch_size >= 100` (excluding quota-exhausted)
    /// and automatically falls back to 5-account chunks across remaining healthy endpoints (including RPCFast).
    pub async fn get_multiple_accounts_with_slot(
        &self,
        pubkeys: &[&str],
    ) -> Result<(Vec<Option<serde_json::Value>>, u64)> {
        if pubkeys.is_empty() {
            return Ok((Vec::new(), 0));
        }

        let has_healthy_batch_100 = self.telemetry.iter().enumerate().any(|(idx, t)| {
            t.config.max_batch_size >= 100
                && !t.quota_exhausted.load(Ordering::Acquire)
                && !self
                    .backoffs
                    .read()
                    .get(&idx)
                    .is_some_and(|until| Instant::now() < *until)
        });

        let mut all_accounts = Vec::with_capacity(pubkeys.len());
        let mut latest_slot = 0u64;

        if has_healthy_batch_100 {
            for chunk in pubkeys.chunks(100) {
                match self
                    .call_filtered_idx(
                        "getMultipleAccounts",
                        json!([chunk, {"encoding": "base64", "commitment": "processed"}]),
                        |idx, _| {
                            self.telemetry.get(idx).is_some_and(|t| {
                                t.config.max_batch_size >= 100
                                    && !t.quota_exhausted.load(Ordering::Acquire)
                            })
                        },
                    )
                    .await
                {
                    Ok(res) => {
                        if let Some(s) = res.pointer("/context/slot").and_then(|v| v.as_u64()) {
                            latest_slot = latest_slot.max(s);
                        }
                        let accounts = res["value"]
                            .as_array()
                            .context("Invalid accounts array format")?;
                        for acc in accounts {
                            all_accounts.push(if acc.is_null() {
                                None
                            } else {
                                Some(acc.clone())
                            });
                        }
                    }
                    Err(_) => {
                        // Fallback this chunk into 5-key sub-chunks across ALL remaining endpoints (including RPCFast)
                        for sub in chunk.chunks(5) {
                            let res = self
                                .call(
                                    "getMultipleAccounts",
                                    json!([sub, {"encoding": "base64", "commitment": "processed"}]),
                                )
                                .await?;
                            if let Some(s) = res.pointer("/context/slot").and_then(|v| v.as_u64()) {
                                latest_slot = latest_slot.max(s);
                            }
                            let accounts = res["value"]
                                .as_array()
                                .context("Invalid accounts array format")?;
                            for acc in accounts {
                                all_accounts.push(if acc.is_null() {
                                    None
                                } else {
                                    Some(acc.clone())
                                });
                            }
                        }
                    }
                }
            }
            return Ok((all_accounts, latest_slot));
        }

        for chunk in pubkeys.chunks(5) {
            let res = self
                .call(
                    "getMultipleAccounts",
                    json!([chunk, {"encoding": "base64", "commitment": "processed"}]),
                )
                .await?;
            if let Some(s) = res.pointer("/context/slot").and_then(|v| v.as_u64()) {
                latest_slot = latest_slot.max(s);
            }
            let accounts = res["value"]
                .as_array()
                .context("Invalid accounts array format")?;
            for acc in accounts {
                all_accounts.push(if acc.is_null() {
                    None
                } else {
                    Some(acc.clone())
                });
            }
        }
        Ok((all_accounts, latest_slot))
    }

    /// Fetch batch account info for multiple pools/vaults
    pub async fn get_multiple_accounts(
        &self,
        pubkeys: &[&str],
    ) -> Result<Vec<Option<serde_json::Value>>> {
        let (accounts, _slot) = self.get_multiple_accounts_with_slot(pubkeys).await?;
        Ok(accounts)
    }

    /// Actively probe all endpoints with `getSlot` to establish baseline latency and readiness.
    /// Unprobed endpoints are strictly classified as `Configured` and never labeled active.
    pub async fn probe_all_endpoints(&self) -> Vec<(String, EndpointStatus, u32)> {
        let mut results = Vec::with_capacity(self.endpoints.len());
        for (idx, url) in self.endpoints.iter().enumerate() {
            let label = self
                .telemetry
                .get(idx)
                .map(|t| t.config.label.clone())
                .unwrap_or_default();
            let start = Instant::now();
            let body = json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "getSlot",
                "params": [{"commitment": "processed"}],
            });
            let resp_res = self.client.post(url).json(&body).send().await;
            let elapsed_ms = start.elapsed().as_millis() as u32;

            if let Some(tel) = self.telemetry.get(idx) {
                tel.is_probed.store(true, Ordering::Release);
                match resp_res {
                    Ok(resp) if resp.status().is_success() => {
                        tel.record_call("getSlot", elapsed_ms, true, false, false);
                        tel.requests_ok.fetch_add(1, Ordering::Relaxed);
                        let status = if elapsed_ms > 350 {
                            EndpointStatus::Degraded
                        } else {
                            EndpointStatus::Healthy
                        };
                        *tel.status.write() = status;
                        results.push((label, status, elapsed_ms));
                    }
                    Ok(resp) if resp.status().as_u16() == 429 => {
                        tel.record_call("getSlot", elapsed_ms, false, true, false);
                        *tel.status.write() = EndpointStatus::QuotaExhausted;
                        results.push((label, EndpointStatus::QuotaExhausted, elapsed_ms));
                    }
                    Ok(_) => {
                        tel.record_call("getSlot", elapsed_ms, false, false, false);
                        *tel.status.write() = EndpointStatus::Degraded;
                        results.push((label, EndpointStatus::Degraded, elapsed_ms));
                    }
                    Err(e) => {
                        tel.record_call("getSlot", elapsed_ms, false, false, e.is_timeout());
                        *tel.status.write() = EndpointStatus::Degraded;
                        results.push((label, EndpointStatus::Degraded, elapsed_ms));
                    }
                }
            }
        }
        results
    }

    /// Dispatch simulation requests prioritizing healthy endpoints with lowest P50 latency.
    pub async fn call_simulation_priority(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let mut candidate_indices: Vec<(usize, u32)> = self
            .telemetry
            .iter()
            .enumerate()
            .filter_map(|(idx, tel)| {
                let status = *tel.status.read();
                let cooled = self
                    .backoffs
                    .read()
                    .get(&idx)
                    .is_some_and(|until| Instant::now() < *until);
                if status == EndpointStatus::Healthy && !cooled {
                    let (p50, _, _) = tel.latency_percentiles();
                    Some((idx, p50))
                } else {
                    None
                }
            })
            .collect();

        candidate_indices.sort_by_key(|&(_, p50)| p50);

        for (idx, _) in candidate_indices {
            let url = &self.endpoints[idx];
            let label = self
                .telemetry
                .get(idx)
                .map(|t| t.config.label.as_str())
                .unwrap_or("RPC");
            let body = json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": method,
                "params": params,
            });

            let start_req = Instant::now();
            if let Ok(resp) = self.client.post(url).json(&body).send().await {
                let elapsed_ms = start_req.elapsed().as_millis() as u32;
                if resp.status().is_success() {
                    if let Ok(val) = resp.json::<serde_json::Value>().await {
                        if let Some(res) = val.get("result") {
                            self.record_success(idx, method, elapsed_ms);
                            return Ok(res.clone());
                        }
                    }
                }
                self.record_error(
                    idx,
                    method,
                    elapsed_ms,
                    format!("Priority sim call non-success on {label}"),
                    false,
                    false,
                    false,
                    2,
                );
            }
        }

        // Fallback to standard resilient round-robin call
        self.call(method, params).await
    }

    /// Simulate transaction with processed commitment and priority routing.
    ///
    /// `max_supported_transaction_version` is the highest RPC transaction version
    /// accepted (0 = v0, 1 = v1; legacy is unversioned).
    /// Solana RPC refuses to deserialize versioned transactions above this
    /// value (the default allows legacy only), so v0/v1 pre-flight simulation
    /// must pass it explicitly or every versioned tx is rejected with
    /// `UnsupportedTransactionVersion`.
    pub async fn simulate_transaction(
        &self,
        b64_tx: &str,
        max_supported_transaction_version: Option<u8>,
    ) -> Result<serde_json::Value> {
        let mut config = serde_json::Map::new();
        config.insert("encoding".into(), json!("base64"));
        config.insert("commitment".into(), json!("processed"));
        config.insert("replaceRecentBlockhash".into(), json!(true));
        if let Some(version) = max_supported_transaction_version {
            config.insert("maxSupportedTransactionVersion".into(), json!(version));
        }
        let res = self
            .call_simulation_priority(
                "simulateTransaction",
                json!([b64_tx, serde_json::Value::Object(config)]),
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
            if last.is_terminal() {
                return Ok(last);
            }
            tokio::time::sleep(interval).await;
        }
        Ok(last)
    }

    /// Broadcast signed transaction directly via RPC with processed commitment (SWQoS path).
    /// When Helius Sender endpoint is configured (HELIUS_SENDER_REGIONAL / HELIUS_SENDER_URL),
    /// submits through Helius Sender for simultaneous validator + Jito dual routing with 0 API credits.
    pub async fn send_raw_transaction(&self, b64_tx: &str) -> Result<String> {
        let payload = json!({
            "jsonrpc": "2.0",
            "id": "1",
            "method": "sendTransaction",
            "params": [
                b64_tx,
                {
                    "encoding": "base64",
                    "skipPreflight": true,
                    "preflightCommitment": "processed",
                    "maxRetries": 0
                }
            ]
        });

        if let Some(ref sender_url) = self.sender_endpoint {
            tracing::info!(
                "Submitting transaction via Helius Sender: {}",
                Self::redact_url(sender_url)
            );
            if let Ok(resp) = self.client.post(sender_url).json(&payload).send().await {
                if resp.status().is_success() {
                    if let Ok(val) = resp.json::<serde_json::Value>().await {
                        if let Some(sig) = val.get("result").and_then(|r| r.as_str()) {
                            return Ok(sig.to_string());
                        }
                    }
                }
            }
            tracing::warn!("Helius Sender submit returned non-success; falling back to RPC pool");
        }

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

    /// Query dynamic priority fee estimate from Helius for given account keys.
    pub async fn get_priority_fee_estimate(&self, account_keys: &[String]) -> Result<u64> {
        let req_params = json!([
            {
                "accountKeys": account_keys,
                "options": {
                    "priorityLevel": "High"
                }
            }
        ]);
        let res = self.call("getPriorityFeeEstimate", req_params).await?;
        let fee = res
            .get("priorityFeeEstimate")
            .and_then(|v| v.as_f64().map(|f| f as u64).or_else(|| v.as_u64()))
            .unwrap_or(10_000);
        Ok(fee)
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

    #[test]
    fn test_provider_classification_enforces_measured_limits() {
        let alchemy =
            RpcPool::classify_provider(0, "https://solana-mainnet.g.alchemy.com/v2/key1234");
        assert_eq!(alchemy.max_batch_size, 100);
        assert_eq!(alchemy.rps_limit, 20.0);

        let rpcfast =
            RpcPool::classify_provider(1, "https://solana-rpc.rpcfast.com/?api_key=abcd1234");
        assert_eq!(rpcfast.max_batch_size, 5);
        assert_eq!(rpcfast.rps_limit, 25.0);

        let shyft = RpcPool::classify_provider(2, "https://rpc.fra.shyft.to?api_key=efgh5678");
        assert_eq!(shyft.max_batch_size, 100);
        assert_eq!(shyft.rps_limit, 8.0);

        let solami_upper =
            RpcPool::classify_provider(3, "https://rpc.solami.dev/Solana?api_key=sk_1234");
        assert_eq!(solami_upper.rps_limit, 2.0);
        assert_eq!(solami_upper.max_batch_size, 100);

        let solami_lower =
            RpcPool::classify_provider(4, "https://rpc.solami.dev/sol?api_key=q88_5678");
        assert_eq!(solami_lower.rps_limit, 3.0);
        assert_eq!(solami_lower.max_batch_size, 100);
    }

    #[test]
    fn test_collect_rpc_urls_synthesizes_fast_rpc_api_and_alchemy_keys() {
        let mut vars = HashMap::new();
        vars.insert("FAST_RPC_API", "test_fast_key_9999");
        vars.insert("ALCHEMY_API_KEY", "alch_key_1111");
        vars.insert("alchemyapi", "alch_key_2222");

        let urls = RpcPool::collect_rpc_urls(|k| vars.get(k).map(|v| v.to_string()));
        assert_eq!(
            urls,
            vec![
                "https://solana-rpc.rpcfast.com/?api_key=test_fast_key_9999",
                "https://solana-mainnet.g.alchemy.com/v2/alch_key_1111",
                "https://solana-mainnet.g.alchemy.com/v2/alch_key_2222",
            ]
        );
    }

    #[test]
    fn test_redact_url_and_hosts_hide_credentials() {
        assert_eq!(
            RpcPool::redact_url("https://mainnet.helius-rpc.com/?api-key=secret-123"),
            "https://mainnet.helius-rpc.com/?***"
        );
        assert_eq!(
            RpcPool::redact_url("https://solana-mainnet.g.alchemy.com/v2/alch_secret"),
            "https://solana-mainnet.g.alchemy.com/v2/***"
        );
        let pool = RpcPool::new(vec![
            "https://mainnet.helius-rpc.com/?api-key=secret-123".to_string(),
            "https://solana-mainnet.g.alchemy.com/v2/alch_secret".to_string(),
        ]);
        assert_eq!(
            pool.redacted_hosts(),
            vec![
                "mainnet.helius-rpc.com".to_string(),
                "solana-mainnet.g.alchemy.com".to_string(),
            ]
        );
    }
}
