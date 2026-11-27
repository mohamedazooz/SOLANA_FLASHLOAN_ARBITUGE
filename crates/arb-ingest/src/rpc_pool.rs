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

impl MethodMetrics {}

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
    /// Classify provider capacity based on live measured benchmarks:
    /// - Alchemy: 20 req/s sustained, burst 25, max batch 100
    /// - RPCFast: 25 req/s sustained, burst 30, max batch 5 (hard RPC cap `-32602 max 5`)
    /// - Shyft: 8 req/s sustained, burst 10, max batch 100 (429 observed at 15-20 rps)
    /// - Solami `/Solana`: 2.0 req/s sustained, burst 3, max batch 100 (429 observed >=4 rps)
    /// - Solami `/sol`: 3.0 req/s sustained, burst 4, max batch 100 (429 observed at 5 rps)
    /// - Helius: 12 req/s sustained, burst 15, max batch 100
    pub fn classify_provider(idx: usize, url: &str) -> ProviderConfig {
        let lc = url.to_ascii_lowercase();
        // Never include API-key fragments in telemetry labels or logs.
        let instance = format!("#{}", idx + 1);

        if lc.contains("alchemy.com") {
            ProviderConfig {
                label: format!("Alchemy({instance})"),
                rps_limit: 20.0,
                burst_capacity: 25.0,
                max_batch_size: 100,
            }
        } else if lc.contains("rpcfast.com") {
            ProviderConfig {
                label: format!("RPCFast({instance})"),
                rps_limit: 25.0,
                burst_capacity: 30.0,
                max_batch_size: 5,
            }
        } else if lc.contains("shyft.to") {
            ProviderConfig {
                label: format!("Shyft({instance})"),
                rps_limit: 8.0,
                burst_capacity: 10.0,
                max_batch_size: 100,
            }
        } else if lc.contains("solami.dev/solana") {
            ProviderConfig {
                label: format!("Solami-Solana({instance})"),
                rps_limit: 2.0,
                burst_capacity: 3.0,
                max_batch_size: 100,
            }
        } else if lc.contains("solami.dev") {
            ProviderConfig {
                label: format!("Solami-sol({instance})"),
                rps_limit: 3.0,
                burst_capacity: 4.0,
                max_batch_size: 100,
            }
        } else if lc.contains("helius-rpc.com") {
            ProviderConfig {
                label: format!("Helius({instance})"),
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

        // 1. Explicit primary endpoint — MUST be first: `SOLANA_RPC_PRIMARY` is
        // the documented primary key (also read by `PacedAccountFetcher::from_env`),
        // but it was never collected here, so operators setting only
        // PRIMARY + FALLBACK_1/2 got an RpcPool containing just
        // `SOLANA_MAINNET_RPC_FALLBACK` (measured in soak5: pool of 1 =
        // api.mainnet-beta, permanently 429-cooled; discovery degraded
        // 162 -> 93 pools and dropped Raydium every ~90s cycle).
        if let Some(val) = lookup("SOLANA_RPC_PRIMARY") {
            push_unique(&val);
        }
        if let Some(val) = lookup("SOLANA_RPC_SECONDARY") {
            push_unique(&val);
        }

        // 1b. Explicit numbered priority slots (SOLANA_RPC_1_URL .. SOLANA_RPC_10_URL)
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

        // 2b. Explicit fallback URLs + role-separated Alchemy URL — same keys
        // `PacedAccountFetcher::from_env` honors. Omitting them here meant the
        // RpcPool and the paced fetcher disagreed on the configured provider
        // set, so discovery/health/Jito traffic never reached the operator's
        // declared fallbacks.
        for key in [
            "SOLANA_RPC_FALLBACK_1",
            "SOLANA_RPC_FALLBACK_2",
            "ALCHEMY_RPC_URL",
        ] {
            if let Some(val) = lookup(key) {
                push_unique(&val);
            }
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
    /// Builds a pool from operator-configured URLs merged with the
    /// environment endpoints (config first, env appended, deduped). When both
    /// sources are empty, falls back to the shared public endpoint with a
    /// loud warning so non-market-critical workers (e.g. Jito leader
    /// refresh, health checks) still function in degraded mode.
    pub fn from_config_and_env(config_urls: Vec<String>) -> Self {
        dotenvy::dotenv().ok();
        let mut seen = std::collections::HashSet::new();
        let mut endpoints: Vec<String> = Vec::new();
        for u in config_urls
            .into_iter()
            .chain(Self::collect_rpc_urls(|k| std::env::var(k).ok()))
        {
            let u = u.trim().to_string();
            let usable = !u.is_empty()
                && !u.contains("REPLACE_ME")
                && (u.starts_with("https://") || u.starts_with("http://"));
            if usable && seen.insert(u.clone()) {
                endpoints.push(u);
            }
        }
        if endpoints.is_empty() {
            tracing::warn!(
                "no RPC endpoints configured via TOML [rpc] url/failover_url or env; falling back to the shared public endpoint https://api.mainnet-beta.solana.com — heavily rate-limited (HTTP 429 expected, 100 req/10s per IP per official docs); NOT production-grade: configure [rpc].url in config TOML or ALCHEMY_RPC_URL / HELIUS_RPC_URL / SOLANA_RPC_*"
            );
            endpoints.push("https://api.mainnet-beta.solana.com".to_string());
        }
        Self::finish_from_env(endpoints)
    }

    fn dedupe_urls(urls: Vec<String>) -> Vec<String> {
        let mut seen = std::collections::HashSet::with_capacity(urls.len());
        urls.into_iter()
            .map(|url| url.trim().to_string())
            .filter(|url| !url.is_empty() && seen.insert(url.clone()))
            .collect()
    }
    pub fn from_env() -> Self {
        dotenvy::dotenv().ok();
        let endpoints = Self::dedupe_urls(Self::collect_rpc_urls(|k| std::env::var(k).ok()));
        Self::finish_from_env(endpoints)
    }

    fn finish_from_env(endpoints: Vec<String>) -> Self {
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
            .map(|url| {
                let authority = url
                    .split_once("://")
                    .map(|(_, remainder)| remainder)
                    .unwrap_or(url.as_str())
                    .split(['/', '?', '#'])
                    .next()
                    .unwrap_or("unknown");
                authority
                    .rsplit('@')
                    .next()
                    .filter(|host| !host.is_empty())
                    .unwrap_or("unknown")
                    .to_string()
            })
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
                            let code = error.get("code").and_then(|value| value.as_i64());
                            let message_lc = error
                                .get("message")
                                .and_then(|value| value.as_str())
                                .unwrap_or_default()
                                .to_ascii_lowercase();
                            let is_rl = code == Some(-32005)
                                || message_lc.contains("rate")
                                || message_lc.contains("limit")
                                || message_lc.contains("429");
                            let backoff_secs = if is_rl {
                                retry_after_secs.unwrap_or(5)
                            } else {
                                0
                            };
                            // Provider-controlled error text is not trusted for logs:
                            // retain the numeric code, but never echo arbitrary text.
                            let msg = format!("RPC error ({label}, code={code:?})");
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
                        // reqwest::Error may include the full request URL (and API key).
                        // Keep only a coarse category in telemetry and returned errors.
                        let category = if error.is_timeout() {
                            "timeout"
                        } else if error.is_connect() {
                            "connect"
                        } else {
                            "request"
                        };
                        let msg = format!("network {category} error ({label})");
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

    /// Fetch current block height. Used by recovery as positive evidence of
    /// blockhash expiration (current > persisted lastValidBlockHeight).
    pub async fn get_block_height(&self) -> Result<u64> {
        let res = self.call("getBlockHeight", json!([])).await?;
        let h = res
            .as_u64()
            .or_else(|| res.get("result").and_then(|r| r.as_u64()))
            .context("Missing block height in response")?;
        Ok(h)
    }

    /// Fetch batch account info and latest chain context slot.
    /// Routes 100-account batches to endpoints with `max_batch_size >= 100` (excluding quota-exhausted)
    /// and automatically falls back to 5-account chunks across remaining healthy endpoints (including RPCFast).
    ///
    /// DATA-01 honesty note: accounts in one call can come from DIFFERENT
    /// responses (multi-chunk merge or 5-key fallback fan-out), each with its
    /// own `context.slot`. This wrapper collapses them to the max — fine for
    /// whole-chunk stamping demos; freshness-critical callers must use
    /// [`Self::get_multiple_accounts_with_slots`] and stamp per account.
    pub async fn get_multiple_accounts_with_slot(
        &self,
        pubkeys: &[&str],
    ) -> Result<(Vec<Option<serde_json::Value>>, u64)> {
        let (accounts, slots) = self.get_multiple_accounts_with_slots(pubkeys).await?;
        let latest_slot = slots.iter().copied().max().unwrap_or(0);
        Ok((accounts, latest_slot))
    }

    /// Decode a getMultipleAccounts result without allowing a short or
    /// oversized value array to shift account bytes onto the wrong pubkey.
    /// Solana JSON-RPC returns exactly one value per requested address.
    fn decode_multiple_accounts_response(
        response: &serde_json::Value,
        expected_count: usize,
    ) -> Result<(Vec<Option<serde_json::Value>>, u64)> {
        let slot = response
            .pointer("/context/slot")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let values = response["value"]
            .as_array()
            .context("Invalid accounts array format")?;
        anyhow::ensure!(
            values.len() == expected_count,
            "Invalid getMultipleAccounts result length: expected {expected_count}, got {}",
            values.len()
        );
        Ok((
            values
                .iter()
                .map(|account| (!account.is_null()).then(|| account.clone()))
                .collect(),
            slot,
        ))
    }

    /// Like [`Self::get_multiple_accounts_with_slot`] but returns the
    /// `context.slot` of the response that carried EACH account, so a slow
    /// replica can never borrow a faster replica's slot and mask staleness
    /// (DATA-01). `0` means the response carried no slot — callers already
    /// treat that as "fall back to current slot".
    pub async fn get_multiple_accounts_with_slots(
        &self,
        pubkeys: &[&str],
    ) -> Result<(Vec<Option<serde_json::Value>>, Vec<u64>)> {
        if pubkeys.is_empty() {
            return Ok((Vec::new(), Vec::new()));
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
        let mut all_slots = Vec::with_capacity(pubkeys.len());

        if has_healthy_batch_100 {
            // chunks are fetched with bounded parallelism (4 in flight): the
            // dominant cost of a multi-chunk hydration is per-chunk RPC round
            // trips, and fetching them concurrently removes the serial RTT
            // chain (e.g. 3 chunks x ~45ms -> ~45ms). `buffered` preserves
            // input order, so the DATA-01 contract (per-account slot stamping
            // from the chunk response that carried it) is unchanged.
            const MAX_PARALLEL_GMA_CHUNKS: usize = 4;
            use futures::stream::{self, StreamExt};
            let chunk_results: Vec<anyhow::Result<serde_json::Value>> = stream::iter(
                pubkeys
                    .chunks(100)
                    .map(|chunk| {
                        self.call_filtered_idx(
                            "getMultipleAccounts",
                            json!([chunk, {"encoding": "base64", "commitment": "processed"}]),
                            |idx, _| {
                                self.telemetry.get(idx).is_some_and(|t| {
                                    t.config.max_batch_size >= 100
                                        && !t.quota_exhausted.load(Ordering::Acquire)
                                })
                            },
                        )
                    })
                    .collect::<Vec<_>>(),
            )
            .buffered(MAX_PARALLEL_GMA_CHUNKS)
            .collect()
            .await;

            for (chunk, res) in pubkeys.chunks(100).zip(chunk_results) {
                match res {
                    Ok(res) => {
                        match Self::decode_multiple_accounts_response(&res, chunk.len()) {
                            Ok((accounts, chunk_slot)) => {
                                all_slots.extend(std::iter::repeat_n(chunk_slot, accounts.len()));
                                all_accounts.extend(accounts);
                            }
                            Err(_) => {
                                // Do not associate malformed batch values with requested keys.
                                // Retry in bounded five-key chunks and validate each response.
                                for sub in chunk.chunks(5) {
                                    let res = self
                                        .call(
                                            "getMultipleAccounts",
                                            json!([sub, {"encoding": "base64", "commitment": "processed"}]),
                                        )
                                        .await?;
                                    let (accounts, sub_slot) =
                                        Self::decode_multiple_accounts_response(&res, sub.len())?;
                                    all_slots.extend(std::iter::repeat_n(sub_slot, accounts.len()));
                                    all_accounts.extend(accounts);
                                }
                            }
                        }
                    }
                    Err(_) => {
                        // Fallback this chunk into 5-key sub-chunks across all providers.
                        for sub in chunk.chunks(5) {
                            let res = self
                                .call(
                                    "getMultipleAccounts",
                                    json!([sub, {"encoding": "base64", "commitment": "processed"}]),
                                )
                                .await?;
                            let (accounts, sub_slot) =
                                Self::decode_multiple_accounts_response(&res, sub.len())?;
                            all_slots.extend(std::iter::repeat_n(sub_slot, accounts.len()));
                            all_accounts.extend(accounts);
                        }
                    }
                }
            }
            return Ok((all_accounts, all_slots));
        }

        for chunk in pubkeys.chunks(5) {
            let res = self
                .call(
                    "getMultipleAccounts",
                    json!([chunk, {"encoding": "base64", "commitment": "processed"}]),
                )
                .await?;
            let (accounts, chunk_slot) =
                Self::decode_multiple_accounts_response(&res, chunk.len())?;
            all_slots.extend(std::iter::repeat_n(chunk_slot, accounts.len()));
            all_accounts.extend(accounts);
        }
        Ok((all_accounts, all_slots))
    }

    /// Fetch batch account info for multiple pools/vaults
    pub async fn get_multiple_accounts(
        &self,
        pubkeys: &[&str],
    ) -> Result<Vec<Option<serde_json::Value>>> {
        let (accounts, _slot) = self.get_multiple_accounts_with_slot(pubkeys).await?;
        Ok(accounts)
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
        // Keep `context.slot` so callers can enforce quote freshness.
        Ok(res)
    }

    /// Query a transaction signature. This is deliberately separate from Jito bundle IDs.
    pub async fn get_signature_status(&self, signature: &str) -> Result<SignatureStatus> {
        let (status, _, _) = self.get_signature_status_detailed(signature).await?;
        Ok(status)
    }

    /// Detailed signature status query with slot and failure error message.
    pub async fn get_signature_status_detailed(
        &self,
        signature: &str,
    ) -> Result<(SignatureStatus, Option<u64>, Option<String>)> {
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
            return Ok((SignatureStatus::Pending, None, None));
        }
        let slot = value.get("slot").and_then(|s| s.as_u64());
        let err_str = value
            .get("err")
            .filter(|e| !e.is_null())
            .map(|e| e.to_string());
        if err_str.is_some() {
            return Ok((SignatureStatus::Failed, slot, err_str));
        }
        let status = match value
            .get("confirmationStatus")
            .and_then(|v| v.as_str())
            .unwrap_or("processed")
        {
            "processed" => SignatureStatus::Processed,
            "confirmed" => SignatureStatus::Confirmed,
            "finalized" => SignatureStatus::Finalized,
            _ => SignatureStatus::Pending,
        };
        Ok((status, slot, None))
    }

    /// Retrieve full on-chain transaction metadata (pre/post balances, CU consumed, logs, error).
    pub async fn get_transaction_with_metadata(
        &self,
        signature: &str,
    ) -> Result<Option<serde_json::Value>> {
        let res = self
            .call(
                "getTransaction",
                json!([
                    signature,
                    {
                        "encoding": "jsonParsed",
                        "maxSupportedTransactionVersion": 1,
                        "commitment": "confirmed"
                    }
                ]),
            )
            .await;
        match res {
            Ok(val) => {
                if val.is_null() {
                    Ok(None)
                } else {
                    Ok(Some(val))
                }
            }
            Err(e) => Err(e),
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

    /// Poll transaction signature tracking detailed status, slot, and error string.
    pub async fn poll_signature_with_details(
        &self,
        signature: &str,
        attempts: usize,
        interval: Duration,
    ) -> Result<(SignatureStatus, Option<u64>, Option<String>)> {
        let mut last_status = SignatureStatus::Pending;
        let mut last_slot = None;
        let mut last_err = None;

        for _ in 0..attempts {
            let (status, slot, err) = self.get_signature_status_detailed(signature).await?;
            last_status = status;
            last_slot = slot;
            last_err = err;
            if status.is_terminal() {
                break;
            }
            tokio::time::sleep(interval).await;
        }

        Ok((last_status, last_slot, last_err))
    }

    fn is_v1_endpoint_allowed(endpoint: &str, allowlist: &[String]) -> bool {
        allowlist.iter().any(|approved| approved == endpoint)
    }

    /// True only when at least one exact HTTP(S) endpoint is configured for V1 ingress.
    pub fn has_v1_rpc_allowlist(&self) -> bool {
        std::env::var("SOLANA_TX_V1_RPC_ALLOWLIST")
            .ok()
            .is_some_and(|raw| {
                raw.split(';').map(str::trim).any(|url| {
                    !url.is_empty()
                        && (url.starts_with("https://") || url.starts_with("http://"))
                        && self.endpoints.iter().any(|endpoint| endpoint == url)
                })
            })
    }

    /// Broadcast a V1 transaction only through endpoints explicitly verified for V1 ingress.
    /// Unlike the generic sender, this path never uses Helius Sender and never falls back
    /// to an unverified RPC endpoint. Configure exact endpoint URLs in
    /// SOLANA_TX_V1_RPC_ALLOWLIST, separated by semicolons. Keep secrets in the environment.
    pub async fn send_raw_transaction_v1(&self, b64_tx: &str) -> Result<String> {
        let allowlist = std::env::var("SOLANA_TX_V1_RPC_ALLOWLIST")
            .unwrap_or_default()
            .split(';')
            .map(str::trim)
            .filter(|url| {
                !url.is_empty() && (url.starts_with("https://") || url.starts_with("http://"))
            })
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if allowlist.is_empty() {
            bail!(
                "V1 RPC allowlist is empty; refusing to broadcast through an unverified endpoint"
            );
        }

        let res = self
            .call_filtered_idx(
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
                |_, endpoint| Self::is_v1_endpoint_allowed(endpoint, &allowlist),
            )
            .await?;
        res.as_str()
            .map(str::to_owned)
            .context("V1 RPC response missing transaction signature string")
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
        // DATA-11: never fabricate a fee. A missing or non-numeric estimate is
        // an error — the caller applies its own configured fallback.
        res.get("priorityFeeEstimate")
            .and_then(|v| v.as_f64().map(|f| f as u64).or_else(|| v.as_u64()))
            .context("priorityFeeEstimate missing or non-numeric in RPC response")
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
    fn v1_allowlist_matches_only_exact_verified_endpoints() {
        let allowlist = vec![
            "https://verified.example/rpc?token=secret".to_string(),
            "https://backup.example/rpc".to_string(),
        ];

        assert!(RpcPool::is_v1_endpoint_allowed(
            "https://verified.example/rpc?token=secret",
            &allowlist
        ));
        assert!(RpcPool::is_v1_endpoint_allowed(
            "https://backup.example/rpc",
            &allowlist
        ));
        assert!(!RpcPool::is_v1_endpoint_allowed(
            "https://unverified.example/rpc",
            &allowlist
        ));
        assert!(!RpcPool::is_v1_endpoint_allowed(
            "https://verified.example/rpc?token=other",
            &allowlist
        ));
        assert!(!RpcPool::is_v1_endpoint_allowed(
            "https://verified.example/rpc?token=secret/extra",
            &allowlist
        ));
    }

    #[test]
    fn test_provider_classification_enforces_measured_limits() {
        let alchemy =
            RpcPool::classify_provider(0, "https://solana-mainnet.g.alchemy.com/v2/key1234");
        assert_eq!(alchemy.max_batch_size, 100);
        assert_eq!(alchemy.rps_limit, 20.0);
        assert_eq!(alchemy.label, "Alchemy(#1)");

        let rpcfast =
            RpcPool::classify_provider(1, "https://solana-rpc.rpcfast.com/?api_key=abcd1234");
        assert_eq!(rpcfast.max_batch_size, 5);
        assert_eq!(rpcfast.rps_limit, 25.0);
        assert_eq!(rpcfast.label, "RPCFast(#2)");

        let shyft = RpcPool::classify_provider(2, "https://rpc.fra.shyft.to?api_key=efgh5678");
        assert_eq!(shyft.max_batch_size, 100);
        assert_eq!(shyft.rps_limit, 8.0);
        assert_eq!(shyft.label, "Shyft(#3)");

        let solami_upper =
            RpcPool::classify_provider(3, "https://rpc.solami.dev/Solana?api_key=sk_1234");
        assert_eq!(solami_upper.rps_limit, 2.0);
        assert_eq!(solami_upper.max_batch_size, 100);
        assert_eq!(solami_upper.label, "Solami-Solana(#4)");

        let solami_lower =
            RpcPool::classify_provider(4, "https://rpc.solami.dev/sol?api_key=q88_5678");
        assert_eq!(solami_lower.rps_limit, 3.0);
        assert_eq!(solami_lower.max_batch_size, 100);
        assert_eq!(solami_lower.label, "Solami-sol(#5)");
    }

    #[test]
    fn from_config_and_env_config_first_then_env_deduped() {
        let pool = RpcPool::from_config_and_env(vec![
            "https://rpc.example-private.com".to_string(),
            "https://api.mainnet-beta.solana.com".to_string(),
            "https://api.mainnet-beta.solana.com".to_string(),
        ]);
        assert_eq!(pool.endpoints()[0], "https://rpc.example-private.com");
        let public = pool
            .endpoints()
            .iter()
            .filter(|u| u.contains("api.mainnet-beta"))
            .count();
        assert_eq!(public, 1, "duplicate URLs must be deduped");
    }

    #[test]
    fn from_config_and_env_empty_config_still_yields_public_fallback() {
        let pool = RpcPool::from_config_and_env(Vec::new());
        assert!(
            !pool.endpoints().is_empty(),
            "empty pool must not starve leader refresh / health checks"
        );
        assert!(pool.endpoints().iter().all(|u| u.starts_with("https://")));
    }

    #[test]
    fn from_config_and_env_filters_placeholder_entries() {
        let pool = RpcPool::from_config_and_env(vec![
            "".to_string(),
            "REPLACE_ME_RPC_URL".to_string(),
            "ftp://not-a-rpc.example.com".to_string(),
        ]);
        assert!(
            pool.endpoints()
                .iter()
                .all(|u| !u.contains("REPLACE_ME") && !u.starts_with("ftp://")),
            "placeholders and non-http schemes must be dropped: {:?}",
            pool.endpoints()
        );
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

    /// Regression for the soak5 finding: an operator config of
    /// SOLANA_RPC_PRIMARY + SOLANA_RPC_FALLBACK_1/2 + ALCHEMY_RPC_URL
    /// (the exact key set `PacedAccountFetcher::from_env` honors) previously
    /// produced an RpcPool containing ONLY SOLANA_MAINNET_RPC_FALLBACK,
    /// starving discovery/health/Jito onto a single rate-limited endpoint.
    #[test]
    fn test_collect_rpc_urls_includes_primary_and_fallback_keys() {
        let mut vars = HashMap::new();
        vars.insert("SOLANA_RPC_PRIMARY", "https://solana-rpc.publicnode.com");
        vars.insert(
            "SOLANA_RPC_FALLBACK_1",
            "https://api.mainnet-beta.solana.com",
        );
        vars.insert(
            "SOLANA_RPC_FALLBACK_2",
            "https://solana-rpc.publicnode.com/?extra=dup",
        );
        vars.insert(
            "ALCHEMY_RPC_URL",
            "https://solana-mainnet.g.alchemy.com/v2/alch_url_key",
        );
        vars.insert(
            "SOLANA_MAINNET_RPC_FALLBACK",
            "https://api.mainnet-beta.solana.com",
        );

        let urls = RpcPool::collect_rpc_urls(|k| vars.get(k).map(|v| v.to_string()));
        assert_eq!(
            urls,
            vec![
                "https://solana-rpc.publicnode.com",
                "https://api.mainnet-beta.solana.com",
                "https://solana-rpc.publicnode.com/?extra=dup",
                "https://solana-mainnet.g.alchemy.com/v2/alch_url_key",
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
            "https://rpc.fra.shyft.to?api_key=must-not-leak".to_string(),
            "https://user:pass@rpc.example.net:443?token=hidden".to_string(),
        ]);
        let hosts = pool.redacted_hosts();
        assert_eq!(
            hosts,
            vec![
                "mainnet.helius-rpc.com".to_string(),
                "solana-mainnet.g.alchemy.com".to_string(),
                "rpc.fra.shyft.to".to_string(),
                "rpc.example.net:443".to_string(),
            ]
        );
        assert!(
            hosts
                .iter()
                .all(|host| !host.contains("api_key") && !host.contains("hidden"))
        );

        assert_eq!(
            RpcPool::dedupe_urls(vec![
                "https://one.example".into(),
                "https://one.example".into(),
                "https://one.example/?api-key=other".into(),
                "https://two.example".into(),
            ]),
            vec![
                "https://one.example".to_string(),
                "https://one.example/?api-key=other".to_string(),
                "https://two.example".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn network_errors_never_echo_rpc_url_credentials() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let secret = "rpc-key-must-not-leak";
        let pool = RpcPool::new(vec![format!("http://{addr}/?api_key={secret}")]);
        let err = pool
            .call("getSlot", serde_json::json!([]))
            .await
            .expect_err("closed local port must fail");

        assert!(!err.to_string().contains(secret));
        let last_error = pool.health_snapshot().providers[0]
            .last_error
            .clone()
            .unwrap_or_default();
        assert!(!last_error.contains(secret));
        assert!(last_error.contains("network"));
    }

    #[tokio::test]
    async fn rpc_error_text_is_not_echoed_into_telemetry() {
        let secret = "provider-reflected-secret-token";
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": { "code": -32000, "message": format!("upstream rejected api_key={secret}") }
        })
        .to_string();
        let (addr, server) = priority_estimate_server(vec![body]);
        let pool = RpcPool::new(vec![format!("http://{addr}")]);
        let err = pool
            .call("getSlot", serde_json::json!([]))
            .await
            .expect_err("RPC error response must fail");
        assert!(!err.to_string().contains(secret));
        let last_error = pool.health_snapshot().providers[0]
            .last_error
            .clone()
            .unwrap_or_default();
        assert!(!last_error.contains(secret));
        assert!(last_error.contains("code=Some(-32000)"));
        server.join().unwrap();
    }

    /// Every provider failing (HTTP 429 from the only endpoint) must fail closed:
    /// a single bounded error, no infinite retry storm, and the endpoint enters
    /// cooldown so the next call skips it immediately.
    #[tokio::test]
    async fn all_endpoints_failing_fails_closed_without_storm() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            use std::io::Write;
            if let Ok((mut stream, _)) = listener.accept() {
                let _ = stream.write_all(
                    b"HTTP/1.1 429 Too Many Requests\r\ncontent-length: 0\r\nretry-after: 60\r\nconnection: close\r\n\r\n",
                );
            }
        });

        let pool = RpcPool::new(vec![format!("http://{addr}")]);
        let started = std::time::Instant::now();
        let err = pool
            .call("getSlot", serde_json::json!([]))
            .await
            .expect_err("all-fail must return Err, not a stale fallback");

        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "fail-closed must be bounded, took {:?}",
            started.elapsed()
        );
        assert!(
            err.to_string()
                .contains("All 1 matching RPC endpoints failed"),
            "unexpected error: {err}"
        );
        // The endpoint entered 429 cooldown: a follow-up call must fail fast
        // without issuing another HTTP request (fail-closed, no retry storm).
        let started2 = std::time::Instant::now();
        let err2 = pool
            .call("getSlot", serde_json::json!([]))
            .await
            .unwrap_err();
        assert!(err2.to_string().contains("cooldown"));
        assert!(started2.elapsed() < std::time::Duration::from_secs(5));
        server.join().unwrap();
    }

    // DATA-01 regression: per-account slots must track the response that carried
    // each account, not a pool-wide max. A raw HTTP endpoint answers two
    // getMultipleAccounts chunks (100 + 50 keys <= the 100-key batch limit) with
    // different context.slot values (1000 then 2000); each account must be
    // stamped with its own chunk's slot.
    #[tokio::test]
    async fn per_account_slots_track_their_own_chunk() {
        let nulls_100 = vec!["null"; 100].join(",");
        let body1 = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"context\":{{\"slot\":1000}},\"value\":[{nulls_100}]}}}}"
        );
        let nulls_50 = vec!["null"; 50].join(",");
        let body2 = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{{\"context\":{{\"slot\":2000}},\"value\":[{nulls_50}]}}}}"
        );
        let bodies = std::sync::Arc::new(std::sync::Mutex::new(vec![body1, body2]));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let mut served = 0;
            while served < 2 {
                let (mut stream, _) = match listener.accept() {
                    Ok(s) => s,
                    Err(_) => break,
                };
                // Drain the request (headers + Content-Length body) before
                // answering, so closing the connection does not RST the client
                // mid-response.
                let mut raw = Vec::new();
                loop {
                    let mut buf = [0u8; 4096];
                    let n = match stream.read(&mut buf) {
                        Ok(n) => n,
                        Err(_) => break,
                    };
                    if n == 0 {
                        break;
                    }
                    raw.extend_from_slice(&buf[..n]);
                    let header_end = raw.windows(4).position(|w| w == b"\r\n\r\n");
                    if let Some(h) = header_end {
                        let headers = String::from_utf8_lossy(&raw[..h]);
                        let clen = headers
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if raw.len() >= h + 4 + clen {
                            break;
                        }
                    }
                }
                let body = {
                    let mut b = bodies.lock().unwrap();
                    let out = b[0].clone();
                    b.remove(0);
                    out
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
                served += 1;
            }
        });

        let url = format!("http://{addr}");
        let pool = RpcPool::new(vec![url]);
        let key = "Key".to_string();
        let keys: Vec<&str> = vec![key.as_str(); 150];
        let (accounts, slots) = match pool.get_multiple_accounts_with_slots(&keys).await {
            Ok(v) => v,
            Err(e) => panic!("call failed: {e:#}"),
        };

        assert_eq!(accounts.len(), 150);
        assert_eq!(slots.len(), 150);
        assert!(accounts.iter().all(|v| v.is_none()));
        assert!(slots[..100].iter().all(|&s| s == 1000));
        assert!(slots[100..].iter().all(|&s| s == 2000));
        server.join().unwrap();
    }

    #[tokio::test]
    async fn malformed_batch_account_count_retries_checked_subchunks() {
        let response = |slot: u64, count: usize| {
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "context": { "slot": slot },
                    "value": vec![serde_json::Value::Null; count]
                }
            })
            .to_string()
        };
        // The first (six-key) batch is malformed: it returns only five values.
        // The bounded fallback then returns a valid five-key and one-key result.
        let (addr, server) = priority_estimate_server(vec![
            response(1000, 5),
            response(2000, 5),
            response(3000, 1),
        ]);
        let pool = RpcPool::new(vec![format!("http://{addr}")]);
        let keys = ["key-1", "key-2", "key-3", "key-4", "key-5", "key-6"];
        let (accounts, slots) = pool
            .get_multiple_accounts_with_slots(&keys)
            .await
            .expect("malformed large response should retry in checked subchunks");

        assert_eq!(accounts.len(), keys.len());
        assert!(accounts.iter().all(Option::is_none));
        assert_eq!(&slots[..5], &[2000; 5]);
        assert_eq!(slots[5], 3000);
        server.join().unwrap();
    }

    #[test]
    fn get_multiple_accounts_rejects_oversized_value_arrays() {
        let response = serde_json::json!({
            "context": { "slot": 42 },
            "value": [null, null]
        });
        let error = RpcPool::decode_multiple_accounts_response(&response, 1)
            .expect_err("an extra value must not be silently discarded");
        assert!(error.to_string().contains("expected 1, got 2"));
    }

    // DATA-11 regression: get_priority_fee_estimate must surface the RPC's
    // estimate verbatim and must never fabricate a default (10,000 or else)
    // when the response lacks a numeric priorityFeeEstimate.
    fn priority_estimate_server(
        bodies: Vec<String>,
    ) -> (std::net::SocketAddr, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let bodies = std::sync::Arc::new(std::sync::Mutex::new(bodies));
        let to_serve = bodies.lock().unwrap().len();
        let handle = std::thread::spawn(move || {
            use std::io::{Read, Write};
            for _ in 0..to_serve {
                let (mut stream, _) = match listener.accept() {
                    Ok(s) => s,
                    Err(_) => break,
                };
                let mut raw = Vec::new();
                loop {
                    let mut buf = [0u8; 4096];
                    match stream.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => raw.extend_from_slice(&buf[..n]),
                    }
                    if let Some(h) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&raw[..h]);
                        let clen = headers
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if raw.len() >= h + 4 + clen {
                            break;
                        }
                    }
                }
                let body = {
                    let mut b = bodies.lock().unwrap();
                    if b.is_empty() {
                        break;
                    }
                    b.remove(0)
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        (addr, handle)
    }

    #[tokio::test]
    async fn priority_fee_estimate_returns_numeric_value() {
        let (addr, server) = priority_estimate_server(vec![
            json!({
                "jsonrpc": "2.0", "id": 1,
                "result": { "priorityFeeEstimate": 23_456 }
            })
            .to_string(),
        ]);
        let pool = RpcPool::new(vec![format!("http://{addr}")]);
        let fee = pool
            .get_priority_fee_estimate(&["SomeAccount".to_string()])
            .await
            .unwrap();
        assert_eq!(fee, 23_456);
        server.join().unwrap();
    }

    #[tokio::test]
    async fn priority_fee_estimate_fails_closed_without_fabricated_default() {
        let (addr, server) = priority_estimate_server(vec![
            json!({
                "jsonrpc": "2.0", "id": 1,
                "result": { "context": { "slot": 1 } }
            })
            .to_string(),
        ]);
        let pool = RpcPool::new(vec![format!("http://{addr}")]);
        let err = pool
            .get_priority_fee_estimate(&["SomeAccount".to_string()])
            .await
            .expect_err("missing estimate must error, not fabricate 10,000");
        assert!(err.to_string().contains("priorityFeeEstimate"));
        server.join().unwrap();
    }
}
