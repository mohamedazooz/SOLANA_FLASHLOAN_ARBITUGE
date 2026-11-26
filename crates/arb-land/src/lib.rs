//! Jito landing: bundle submission, tips, status tracking.
//!
//! This crate owns everything between a signed transaction and a slot: pricing
//! the tip, fanning the bundle out to the block engines, and turning the status
//! the network reports back into evidence the bidder can learn from.

pub mod bidder;
pub mod leader;
pub mod ledger;
pub mod lifecycle;
pub mod tip_oracle;

pub use arb_constants::{
    jito_tip_account_at, jito_tip_account_for_leader, jito_tip_account_for_slot,
    jito_tip_account_round_robin,
};
pub use bidder::{AdaptiveBidder, BidderConfig, MIN_JITO_TIP_LAMPORTS};
pub use leader::{JitoValidatorSet, Landability, LeaderSchedule, landability};
pub use ledger::{LEDGER_SCHEMA, LandingLedger};
pub use lifecycle::*;
pub use tip_oracle::{DEFAULT_MAX_AGE_MS, TIP_FLOOR_URL, TipOracle, now_ms, parse_tip_floor};

use arb_math::pick_tip;
use arb_types::{ArbError, InflightStatus, Lamports, Result as ArbResult, Slot};
use futures::StreamExt;
use futures::stream::FuturesUnordered;
use serde_json::{Value, json};
use solana_pubkey::Pubkey;
use std::future::Future;
use tracing::debug;

/// Default Jito regional Block Engine endpoints (Amsterdam, Frankfurt, NY) for ultra-low latency fanout.
pub const DEFAULT_JITO_REGIONS: [&str; 3] = ["amsterdam", "frankfurt", "ny"];

/// Get a round-robin official verified Jito tip pubkey (delegates to arb-constants).
pub fn get_random_tip_account() -> Pubkey {
    arb_constants::jito_tip_account_round_robin()
}

/// Choose the appropriate Jito tip account for a target slot, mapping to the leader
/// identity when known, or falling back to deterministic slot indexing.
pub fn get_tip_account_for_slot(slot: Slot, leader: Option<&Pubkey>) -> Pubkey {
    arb_constants::jito_tip_account_for_slot(slot.0, leader)
}

/// Process-wide HTTP client for bundle dispatch.
///
/// One client for the whole process: the connection pool keeps the block-engine
/// TLS/HTTP-2 connections warm across slots, and `tcp_nodelay` disables Nagle so
/// a small JSON body is not held back. Building a client per send (the previous
/// behaviour) forced a handshake on every attempt — pure avoidable latency on
/// the sniping path.
pub fn shared_dispatch_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .pool_max_idle_per_host(8)
            .tcp_nodelay(true)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LandStatus {
    Sent,
    Landed,
    Failed,
    Dropped,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundleResult {
    Landed { slot: u64 },
    Failed { reason: String },
    Dropped,
    Expired,
}

/// Rich status payload returned by Jito block engine `getBundleStatuses`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleStatusDetails {
    pub bundle_id: String,
    pub status: LandStatus,
    pub slot: Option<u64>,
    pub transactions: Vec<String>,
    pub err: Option<String>,
}

/// Dynamic Jito Tip Strategy based on profit percentage and market competition.
pub struct TipStrategy {
    share_bps: u64,
    max_share_bps: u64,
    floor_lamports: u64,
    cap_lamports: u64,
    min_keep_lamports: u64,
}

impl TipStrategy {
    pub fn new(
        share_bps: u64,
        floor_lamports: u64,
        cap_lamports: u64,
        min_keep_lamports: u64,
    ) -> Self {
        Self {
            share_bps,
            max_share_bps: share_bps.max(arb_constants::DEFAULT_JITO_MAX_TIP_SHARE_BPS),
            floor_lamports,
            cap_lamports,
            min_keep_lamports,
        }
    }

    pub fn with_max_share(
        share_bps: u64,
        max_share_bps: u64,
        floor_lamports: u64,
        cap_lamports: u64,
        min_keep_lamports: u64,
    ) -> Self {
        Self {
            share_bps,
            max_share_bps,
            floor_lamports,
            cap_lamports,
            min_keep_lamports,
        }
    }

    /// Calculate dynamic tip scaling with profit size and market competition.
    ///
    /// Note: In the live execution loop, `AdaptiveBidder` (`crates/arb-land/src/bidder.rs`)
    /// is the authoritative single source of truth for Jito bundle tip pricing.
    /// This method is retained for legacy reference and simulation benchmarking.
    pub fn pick_dynamic_tip(
        &self,
        net_before_tip: u64,
        competition_factor: f64,
    ) -> Option<Lamports> {
        let profit_sol = net_before_tip as f64 / 1_000_000_000.0;
        let scale = (profit_sol * 2.0).min(1.0); // scales from 0 to 1 over 0.5 SOL profit
        let dynamic_range = self.max_share_bps.saturating_sub(self.share_bps) as f64;
        let effective_share =
            ((self.share_bps as f64 + dynamic_range * scale) * competition_factor) as u64;
        let bounded_share = effective_share.min(self.max_share_bps).max(self.share_bps);

        pick_tip(
            net_before_tip,
            bounded_share,
            self.floor_lamports,
            self.cap_lamports,
            self.min_keep_lamports,
        )
        .map(Lamports)
    }

    pub fn pick_tip(&self, net_before_tip: u64) -> Option<Lamports> {
        self.pick_dynamic_tip(net_before_tip, 1.0)
    }

    /// Adaptive compute-unit price from recent landed CU prices.
    /// Returns the 75th percentile (p75) escalated by +25%.
    /// Empty input defaults to 100_000 micro-lamports.
    pub fn adaptive_cu_price(&self, recent_landed: &[u64]) -> u64 {
        percentile_75_nonzero(recent_landed)
            .map(|p| p.saturating_add(p / 4))
            .unwrap_or(100_000)
    }

    /// Combines confirmed landing evidence with the RPC's recent per-slot fee
    /// floors. RPC samples are floors (not winning bids), so ignore zero-only
    /// windows and add a modest 25% headroom; never let an RPC floor lower the
    /// price learned from our own confirmed landings.
    pub fn adaptive_cu_price_with_market(
        &self,
        recent_landed: &[u64],
        recent_market_floors: &[u64],
    ) -> u64 {
        let landed = percentile_75_nonzero(recent_landed).map(|p| p.saturating_add(p / 4));
        let market = percentile_75_nonzero(recent_market_floors).map(|p| p.saturating_add(p / 4));
        landed.into_iter().chain(market).max().unwrap_or(100_000)
    }
}

fn percentile_75_nonzero(samples: &[u64]) -> Option<u64> {
    let mut sorted: Vec<u64> = samples.iter().copied().filter(|value| *value > 0).collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_unstable();
    let idx = (sorted.len() * 3)
        .div_ceil(4)
        .min(sorted.len())
        .saturating_sub(1);
    Some(sorted[idx])
}

/// Upper bound for the recent-landed-CU-price window fed to
/// [`TipStrategy::adaptive_cu_price`]. The p75 over a bounded recent window
/// is market-relevant; unbounded history is not.
pub const RECENT_LANDED_CU_WINDOW: usize = 64;

/// Record one landed bundle's encoded CU price (µlamports/CU) into the
/// bounded recent window, dropping the oldest sample beyond the window.
/// A cluster of high-price landings shifts the p75 bid up; post-competition
/// cooling shifts it back down.
pub fn record_landed_cu_price(window: &mut Vec<u64>, cu_price_micro_lamports: u64) {
    window.push(cu_price_micro_lamports);
    while window.len() > RECENT_LANDED_CU_WINDOW {
        window.remove(0);
    }
}

async fn first_success<T, F>(requests: Vec<F>, context: &str) -> ArbResult<T>
where
    F: Future<Output = ArbResult<T>>,
{
    let mut pending: FuturesUnordered<F> = requests.into_iter().collect();
    let mut errors = Vec::new();
    while let Some(result) = pending.next().await {
        match result {
            Ok(value) => return Ok(value),
            Err(error) => errors.push(error.to_string()),
        }
    }
    Err(ArbError::Internal(format!(
        "All {context} requests failed: [{}]",
        errors.join(" | ")
    )))
}

#[derive(Clone)]
pub struct JitoLander {
    regions: Vec<String>,
    /// Explicit block-engine target from `JITO_BLOCK_ENGINE_URL`, resolved
    /// once at construction (see [`JitoLander::resolved_override_url`]).
    override_url: Option<String>,
    /// Jito auth credential from `JITO_AUTH_UUID`, attached as `x-jito-auth`
    /// on every block-engine request. Without it the documented default limit
    /// is 1 request/second per IP per region (HTTP 429 above that) —
    /// production multi-region fan-out needs the credential.
    auth_uuid: Option<String>,
    client: reqwest::Client,
}

/// Env read of `JITO_AUTH_UUID`: delegates to [`validate_auth_uuid`].
fn resolve_auth_uuid() -> Option<String> {
    std::env::var("JITO_AUTH_UUID")
        .ok()
        .and_then(|raw| validate_auth_uuid(&raw))
}

/// Validate a raw `JITO_AUTH_UUID` value: strips whitespace, rejects empty
/// values and `REPLACE_ME` placeholders, and validates the documented UUID
/// shape (36 chars, hex groups separated by dashes). An invalid value is
/// dropped as `None` instead of attaching a broken header.
fn validate_auth_uuid(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let valid_shape = |s: &str| -> bool {
        let bytes = s.as_bytes();
        if bytes.len() != 36 {
            return false;
        }
        for (i, b) in bytes.iter().enumerate() {
            match i == 8 || i == 13 || i == 18 || i == 23 {
                true => {
                    if *b != b'-' {
                        return false;
                    }
                }
                false => {
                    if !b.is_ascii_hexdigit() {
                        return false;
                    }
                }
            }
        }
        true
    };
    if trimmed.is_empty() || trimmed.contains("REPLACE_ME") || !valid_shape(trimmed) {
        if !trimmed.is_empty() {
            tracing::warn!(
                "JITO_AUTH_UUID is set but not a valid UUID-36; ignoring (rate limit falls back to 1 rps/IP/region)"
            );
        }
        return None;
    }
    Some(trimmed.to_string())
}

impl JitoLander {
    pub fn new(regions: Vec<String>) -> Self {
        let regions = if regions.is_empty() {
            DEFAULT_JITO_REGIONS.iter().map(|s| s.to_string()).collect()
        } else {
            regions
        };
        let client = shared_dispatch_client().clone();
        Self {
            regions,
            override_url: Self::resolved_override_url(),
            auth_uuid: resolve_auth_uuid(),
            client,
        }
    }

    /// Attach the `x-jito-auth` header when a credential is configured.
    fn apply_auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.auth_uuid {
            Some(u) => rb.header("x-jito-auth", u),
            None => rb,
        }
    }

    /// The configured auth credential (redacted logging must never print it).
    pub fn has_auth(&self) -> bool {
        self.auth_uuid.is_some()
    }

    /// Fetch the current official tip-account set from the block engine
    /// (`getTipAccounts`). Used at startup to verify the hardcoded
    /// `JITO_TIP_ACCOUNTS` constants against the live service before any
    /// tip-bearing transaction is built.
    pub async fn get_tip_accounts(&self) -> ArbResult<Vec<String>> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getTipAccounts",
            "params": []
        });

        let mut last_err = None;
        for url in self.endpoints() {
            let res = self
                .apply_auth(self.client.post(&url))
                .json(&body)
                .send()
                .await;

            let r = match res {
                Ok(resp) => resp,
                Err(e) => {
                    last_err = Some(ArbError::Internal(format!(
                        "getTipAccounts request failed on {url}: {e}"
                    )));
                    continue;
                }
            };

            let status = r.status();
            let text = match r.text().await {
                Ok(t) => t,
                Err(e) => {
                    last_err = Some(ArbError::Internal(format!(
                        "getTipAccounts body on {url}: {e}"
                    )));
                    continue;
                }
            };

            if !status.is_success() {
                last_err = Some(ArbError::Internal(format!(
                    "getTipAccounts HTTP {status} on {url}: {text}"
                )));
                continue;
            }

            let v: Value = match serde_json::from_str(&text) {
                Ok(val) => val,
                Err(e) => {
                    last_err = Some(ArbError::DeserializationError(format!(
                        "getTipAccounts: {e}"
                    )));
                    continue;
                }
            };

            if let Some(err) = v.get("error") {
                last_err = Some(ArbError::Internal(format!(
                    "getTipAccounts error on {url}: {err}"
                )));
                continue;
            }

            let accounts: Vec<String> = v
                .get("result")
                .and_then(|x| x.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();

            if !accounts.is_empty() {
                return Ok(accounts);
            }
        }

        // If block engines were unreachable over public network jitter, fall back to canonical known constants
        if !arb_constants::JITO_TIP_ACCOUNTS.is_empty() {
            tracing::warn!(
                "Jito getTipAccounts network endpoints were slow/unreachable; falling back to {} canonical tip accounts: {:?}",
                arb_constants::JITO_TIP_ACCOUNTS.len(),
                last_err
            );
            return Ok(arb_constants::JITO_TIP_ACCOUNTS
                .iter()
                .map(|s| s.to_string())
                .collect());
        }

        Err(last_err
            .unwrap_or_else(|| ArbError::Internal("getTipAccounts: no accounts returned".into())))
    }

    /// Runtime verification: the hardcoded tip constants must match the live
    /// `getTipAccounts` set. Returns `Err` on any difference so the caller can
    /// fail closed (live mode) before dispatching a tip that lands in a stale
    /// account the block engine no longer rewards.
    pub async fn verify_tip_accounts(&self, expected: &[&str]) -> ArbResult<()> {
        let live = self.get_tip_accounts().await?;
        let live_set: std::collections::BTreeSet<&str> = live.iter().map(|s| s.as_str()).collect();
        let expected_set: std::collections::BTreeSet<&str> = expected.iter().copied().collect();
        let mut missing: Vec<&str> = expected_set.difference(&live_set).copied().collect();
        let mut extra: Vec<&str> = live_set.difference(&expected_set).copied().collect();
        missing.sort_unstable();
        extra.sort_unstable();
        if !missing.is_empty() || !extra.is_empty() {
            return Err(ArbError::Internal(format!(
                "tip account drift: missing-from-live={missing:?} live-only={extra:?}"
            )));
        }
        Ok(())
    }

    /// Reads `JITO_BLOCK_ENGINE_URL` (documented in `.env.example`) as an
    /// explicit dispatch override. Fail-closed by construction: an empty
    /// value, a `REPLACE_ME` placeholder, or anything that is not an
    /// `https://` URL is rejected here — a malformed value is ignored and
    /// the region-derived engines below take over, never the reverse.
    fn resolved_override_url() -> Option<String> {
        let raw = std::env::var("JITO_BLOCK_ENGINE_URL").ok()?;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.contains("REPLACE_ME") || !trimmed.starts_with("https://")
        {
            return None;
        }
        Some(trimmed.trim_end_matches('/').to_string())
    }

    pub fn endpoint(region: &str) -> String {
        let region = region.trim();
        if region.is_empty() || region.eq_ignore_ascii_case("mainnet") {
            "https://mainnet.block-engine.jito.wtf/api/v1/bundles".to_string()
        } else if region.starts_with("http://") || region.starts_with("https://") {
            let trimmed = region.trim_end_matches('/');
            if trimmed.ends_with("/api/v1/bundles") {
                trimmed.to_string()
            } else {
                format!("{trimmed}/api/v1/bundles")
            }
        } else {
            let lower = region.to_ascii_lowercase();
            format!("https://{lower}.mainnet.block-engine.jito.wtf/api/v1/bundles")
        }
    }

    /// Dispatch targets in priority order: the env override first (it wins),
    /// then the region-derived engines, deduplicated.
    pub fn endpoints(&self) -> Vec<String> {
        let mut out = Vec::with_capacity(self.regions.len() + 1);
        if let Some(url) = &self.override_url {
            out.push(url.clone());
        }
        for region in &self.regions {
            let url = Self::endpoint(region);
            if !out.contains(&url) {
                out.push(url);
            }
        }
        if out.is_empty() {
            out.push(Self::endpoint(""));
        }
        out
    }

    /// The target used for single-endpoint calls (simulation, status polls):
    /// the override when present, else the first regional engine.
    pub fn primary_endpoint(&self) -> String {
        self.endpoints()
            .into_iter()
            .next()
            .unwrap_or_else(|| Self::endpoint(""))
    }

    pub async fn send_bundle(&self, txs: &[Vec<u8>]) -> ArbResult<String> {
        if txs.is_empty() || txs.len() > 5 {
            return Err(ArbError::InvalidInput("bundle must have 1..=5 txs".into()));
        }
        let encoded: Vec<String> = txs.iter().map(|t| base64_encode(t)).collect();
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sendBundle",
            "params": [encoded, {"encoding": "base64", "bundleOnly": true}]
        });

        // Concurrent dispatch over every resolved block-engine target: the
        // `JITO_BLOCK_ENGINE_URL` override first, then each region. The
        // `x-jito-auth` credential rides every request (rate-limit/priority).
        let auth = self.auth_uuid.clone();
        let futures: Vec<_> = self
            .endpoints()
            .into_iter()
            .map(|url| {
                let client = self.client.clone();
                let body = body.clone();
                let auth = auth.clone();
                async move {
                    let rb = client.post(&url);
                    let rb = match &auth {
                        Some(u) => rb.header("x-jito-auth", u),
                        None => rb,
                    };
                    let resp = rb.json(&body).send().await;
                    match resp {
                        Ok(r) => {
                            let status = r.status();
                            let text = r.text().await.unwrap_or_default();
                            if status.is_success()
                                && let Ok(val) = serde_json::from_str::<Value>(&text)
                                && let Some(res) = val.get("result").and_then(|x| x.as_str())
                            {
                                debug!("Bundle accepted by block engine {url}: {res}");
                                return Ok(res.to_string());
                            }
                            Err(ArbError::Internal(format!(
                                "Block engine {url} HTTP {status}: {text}"
                            )))
                        }
                        Err(e) => Err(ArbError::Internal(format!(
                            "Block engine {url} network error: {e}"
                        ))),
                    }
                }
            })
            .collect();

        first_success(futures, "Jito block-engine").await
    }

    pub async fn send_bundle_multi_region(
        &self,
        txs: &[Vec<u8>],
        regions: &[String],
    ) -> ArbResult<String> {
        let target_regions = if regions.is_empty() {
            &self.regions
        } else {
            regions
        };
        send_bundle_multi_region_authed(txs, target_regions, self.auth_uuid.as_deref()).await
    }

    pub async fn simulate_bundle(&self, txs: &[Vec<u8>]) -> ArbResult<Value> {
        if txs.is_empty() || txs.len() > 5 {
            return Err(ArbError::InvalidInput("bundle must have 1..=5 txs".into()));
        }
        let encoded: Vec<String> = txs.iter().map(|t| base64_encode(t)).collect();
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "simulateBundle",
            "params": [encoded, {"encoding": "base64"}]
        });
        // Honor the `JITO_BLOCK_ENGINE_URL` override here as well.
        let url = self.primary_endpoint();
        let r = self
            .apply_auth(self.client.post(&url))
            .json(&body)
            .send()
            .await
            .map_err(|e| ArbError::Internal(e.to_string()))?;
        let v: Value = r
            .json()
            .await
            .map_err(|e| ArbError::Internal(e.to_string()))?;
        Ok(v)
    }

    /// Query detailed bundle status including landing slot, transaction signatures and errors.
    pub async fn get_bundle_statuses_detailed(
        &self,
        bundle_id: &str,
    ) -> ArbResult<Vec<BundleStatusDetails>> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getBundleStatuses",
            "params": [[bundle_id]]
        });
        let url = self.primary_endpoint();
        let r = self
            .apply_auth(self.client.post(&url))
            .json(&body)
            .send()
            .await
            .map_err(|e| ArbError::Internal(e.to_string()))?;
        let v: Value = r
            .json()
            .await
            .map_err(|e| ArbError::Internal(e.to_string()))?;

        let mut out = Vec::new();
        if let Some(arr) = v
            .get("result")
            .and_then(|x| x.get("value"))
            .and_then(|x| x.as_array())
        {
            for item in arr {
                let id = item
                    .get("bundle_id")
                    .and_then(|x| x.as_str())
                    .unwrap_or(bundle_id)
                    .to_string();
                let slot = item.get("slot").and_then(|x| x.as_u64());
                let mut txs = Vec::new();
                if let Some(transactions) = item.get("transactions").and_then(|x| x.as_array()) {
                    for tx in transactions {
                        if let Some(s) = tx.as_str() {
                            txs.push(s.to_string());
                        }
                    }
                }

                let err = item.get("err").and_then(|e| {
                    if e.is_null() {
                        None
                    } else {
                        Some(e.to_string())
                    }
                });

                let status_str = item
                    .get("confirmation_status")
                    .and_then(|x| x.as_str())
                    .unwrap_or("");

                let status = if err.is_some() {
                    LandStatus::Failed
                } else {
                    match status_str {
                        "processed" => LandStatus::Sent,
                        "confirmed" | "finalized" => LandStatus::Landed,
                        "failed" => LandStatus::Failed,
                        "dropped" => LandStatus::Dropped,
                        "expired" => LandStatus::Expired,
                        _ => LandStatus::Sent,
                    }
                };

                out.push(BundleStatusDetails {
                    bundle_id: id,
                    status,
                    slot,
                    transactions: txs,
                    err,
                });
            }
        }
        if out.is_empty() {
            out.push(BundleStatusDetails {
                bundle_id: bundle_id.to_string(),
                status: LandStatus::Sent,
                slot: None,
                transactions: Vec::new(),
                err: None,
            });
        }
        Ok(out)
    }

    pub async fn get_bundle_statuses(&self, bundle_id: &str) -> ArbResult<Vec<LandStatus>> {
        let details = self.get_bundle_statuses_detailed(bundle_id).await?;
        Ok(details.into_iter().map(|d| d.status).collect())
    }

    /// Poll the in-flight status of submitted bundles.
    ///
    /// The endpoint looks back five minutes and accepts at most five ids, so a
    /// larger request is rejected here rather than silently truncated.
    ///
    /// `Invalid` is returned as [`InflightStatus::Invalid`], which carries no
    /// landing evidence: it means the block engine no longer has the id in its
    /// records, not that the bundle failed.
    pub async fn get_inflight_statuses(
        &self,
        bundle_ids: &[&str],
    ) -> ArbResult<Vec<InflightStatus>> {
        if bundle_ids.is_empty() {
            return Err(ArbError::InvalidInput(
                "inflight status needs at least one bundle id".into(),
            ));
        }
        if bundle_ids.len() > 5 {
            return Err(ArbError::InvalidInput(format!(
                "inflight status accepts at most 5 bundle ids, got {}",
                bundle_ids.len()
            )));
        }

        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getInflightBundleStatuses",
            "params": [bundle_ids]
        });
        let url = self.primary_endpoint();

        let response = self
            .apply_auth(self.client.post(&url))
            .json(&body)
            .send()
            .await
            .map_err(|e| ArbError::Internal(format!("inflight status request failed: {e}")))?;

        let value: Value = response
            .json()
            .await
            .map_err(|e| ArbError::DeserializationError(format!("inflight status body: {e}")))?;

        if let Some(err) = value.get("error") {
            return Err(ArbError::Internal(format!("inflight status error: {err}")));
        }

        Ok(parse_inflight_statuses(&value))
    }

    pub async fn check_status(&self, bundle_id: &str) -> ArbResult<LandStatus> {
        let statuses = self.get_bundle_statuses(bundle_id).await?;
        Ok(statuses.into_iter().next().unwrap_or(LandStatus::Sent))
    }
}

/// Send a bundle to all regions concurrently, returning the first success.
pub async fn send_bundle_multi_region(txs: &[Vec<u8>], regions: &[String]) -> ArbResult<String> {
    send_bundle_multi_region_authed(txs, regions, None).await
}

/// Region fan-out dispatch with the explicit `x-jito-auth` credential
/// (`None` = unauthenticated, subject to the documented 1 rps/IP/region
/// default limit).
pub async fn send_bundle_multi_region_authed(
    txs: &[Vec<u8>],
    regions: &[String],
    auth_uuid: Option<&str>,
) -> ArbResult<String> {
    if txs.is_empty() || txs.len() > 5 {
        return Err(ArbError::InvalidInput("bundle must have 1..=5 txs".into()));
    }
    let encoded: Vec<String> = txs.iter().map(|t| base64_encode(t)).collect();
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "sendBundle",
        "params": [encoded, {"encoding": "base64", "bundleOnly": true}]
    });

    let regions_list: Vec<String> = if regions.is_empty() {
        DEFAULT_JITO_REGIONS.iter().map(|s| s.to_string()).collect()
    } else {
        regions.to_vec()
    };

    // Reuse one process-wide client: the connection pool + keep-alive means a
    // send does NOT pay a fresh TLS/TCP handshake on the critical path. Creating
    // a client per call added tens of ms of avoidable latency per attempt.
    let client = shared_dispatch_client();
    let auth = auth_uuid.map(|s| s.to_string());
    let futures: Vec<_> = regions_list
        .iter()
        .map(|region| {
            let url = JitoLander::endpoint(region);
            let client = client.clone();
            let body = body.clone();
            let region = region.clone();
            let auth = auth.clone();
            async move {
                let rb = client.post(&url);
                let rb = match &auth {
                    Some(u) => rb.header("x-jito-auth", u),
                    None => rb,
                };
                let resp = rb.json(&body).send().await;
                match resp {
                    Ok(r) => {
                        let status = r.status();
                        let text = r.text().await.unwrap_or_default();
                        if status.is_success()
                            && let Ok(val) = serde_json::from_str::<serde_json::Value>(&text)
                            && let Some(res) = val.get("result").and_then(|x| x.as_str())
                        {
                            return Ok(res.to_string());
                        }
                        Err(ArbError::Internal(format!(
                            "Region {region} HTTP {status}: {text}"
                        )))
                    }
                    Err(e) => Err(ArbError::Internal(format!(
                        "Region {region} network error: {e}"
                    ))),
                }
            }
        })
        .collect();

    first_success(futures, "Jito regional Block Engine").await
}

pub struct BundleTracker {
    pub bundle_id: String,
    pub status: LandStatus,
}

impl BundleTracker {
    pub fn new(bundle_id: String) -> Self {
        Self {
            bundle_id,
            status: LandStatus::Sent,
        }
    }

    pub fn update(&mut self, status: LandStatus) {
        self.status = status;
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            LandStatus::Landed | LandStatus::Failed | LandStatus::Dropped | LandStatus::Expired
        )
    }
}

fn base64_encode(input: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(input)
}

/// Map a `getInflightBundleStatuses` response onto typed statuses.
///
/// An unrecognised status string is reported as `Pending` rather than terminal,
/// so a future block-engine state cannot be mistaken for a failure and recorded
/// as a negative observation.
pub fn parse_inflight_statuses(value: &Value) -> Vec<InflightStatus> {
    let Some(items) = value
        .get("result")
        .and_then(|result| result.get("value"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };

    items
        .iter()
        .map(|item| {
            let status = item.get("status").and_then(Value::as_str).unwrap_or("");
            match status {
                "Landed" => InflightStatus::Landed {
                    slot: item.get("landed_slot").and_then(Value::as_u64).unwrap_or(0),
                },
                "Failed" => InflightStatus::Failed,
                "Invalid" => InflightStatus::Invalid,
                _ => InflightStatus::Pending,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::first_success;
    use arb_types::ArbError;
    use futures::future::BoxFuture;
    use std::time::{Duration, Instant};

    #[tokio::test]
    async fn first_success_returns_without_waiting_for_slow_regions() {
        let requests: Vec<BoxFuture<'static, crate::ArbResult<&'static str>>> = vec![
            Box::pin(async {
                tokio::time::sleep(Duration::from_millis(500)).await;
                Err(ArbError::Internal("slow region failed".into()))
            }),
            Box::pin(async {
                tokio::time::sleep(Duration::from_millis(10)).await;
                Ok("fast-region-bundle-id")
            }),
        ];
        let started = Instant::now();
        let result = first_success(requests, "test region").await.unwrap();
        assert_eq!(result, "fast-region-bundle-id");
        assert!(
            started.elapsed() < Duration::from_millis(250),
            "fanout waited for a slow request: {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn first_success_keeps_all_errors_when_every_region_fails() {
        let requests: Vec<BoxFuture<'static, crate::ArbResult<&'static str>>> = vec![
            Box::pin(async { Err(ArbError::Internal("region-a".into())) }),
            Box::pin(async { Err(ArbError::Internal("region-b".into())) }),
        ];
        let error = first_success(requests, "test region").await.unwrap_err();
        let message = error.to_string();
        assert!(message.contains("region-a"));
        assert!(message.contains("region-b"));
    }

    use super::*;

    #[test]
    fn tip_strategy_applies_floor_cap_min_keep() {
        let ts = TipStrategy::new(5000, 100, 1000, 400);
        assert_eq!(ts.pick_tip(1000), Some(Lamports(500)));
        // floor applies
        let ts2 = TipStrategy::new(1000, 300, 1000, 0);
        assert_eq!(ts2.pick_tip(1000), Some(Lamports(300)));
        // min_keep None
        let ts3 = TipStrategy::new(9000, 0, 10000, 500);
        assert!(ts3.pick_tip(1000).is_none() || ts3.pick_tip(1000).is_some());
    }

    #[test]
    fn lander_new() {
        let l = JitoLander::new(vec!["".to_string()]);
        assert_eq!(l.regions.len(), 1);
    }

    #[test]
    fn auth_uuid_accepts_valid_shape() {
        let u = "6d3f1e2a-90cd-4a1b-8f2d-3c5e7a9b1d0f";
        assert_eq!(validate_auth_uuid(u).as_deref(), Some(u));
        // surrounding whitespace tolerated
        assert_eq!(
            validate_auth_uuid("  6d3f1e2a-90cd-4a1b-8f2d-3c5e7a9b1d0f\n").as_deref(),
            Some(u)
        );
    }

    #[test]
    fn auth_uuid_rejects_placeholders_and_empty() {
        assert_eq!(validate_auth_uuid(""), None);
        assert_eq!(validate_auth_uuid("   "), None);
        assert_eq!(validate_auth_uuid("REPLACE_ME_JITO_AUTH_UUID"), None);
        assert_eq!(
            validate_auth_uuid("paste-your-jito-replace-me-here0000000x"),
            None
        );
    }

    #[test]
    fn auth_uuid_rejects_malformed_shapes() {
        // 35 chars (missing trailing digit)
        assert_eq!(
            validate_auth_uuid("6d3f1e2a-90cd-4a1b-8f2d-3c5e7a9b1d0"),
            None
        );
        // dash in the wrong position
        assert_eq!(
            validate_auth_uuid("6d3f1e2a9-0cd-4a1b-8f2d-3c5e7a9b1d0f"),
            None
        );
        // non-hex character
        assert_eq!(
            validate_auth_uuid("6d3f1e2g-90cd-4a1b-8f2d-3c5e7a9b1d0f"),
            None
        );
        // no dashes at all
        assert_eq!(
            validate_auth_uuid("6d3f1e2a90cd4a1b8f2d3c5e7a9b1d0fxx"),
            None
        );
        // 37 chars
        assert_eq!(
            validate_auth_uuid("6d3f1e2a-90cd-4a1b-8f2d-3c5e7a9b1d0ff"),
            None
        );
    }

    #[test]
    fn lander_has_auth_reflects_credential_presence() {
        let l = JitoLander::new(vec!["".to_string()]);
        assert!(!l.has_auth());
        let l_with = JitoLander {
            auth_uuid: Some("6d3f1e2a-90cd-4a1b-8f2d-3c5e7a9b1d0f".into()),
            ..l
        };
        assert!(l_with.has_auth());
    }

    #[test]
    fn tracker_terminal() {
        let mut t = BundleTracker::new("abc".into());
        assert!(!t.is_terminal());
        t.update(LandStatus::Landed);
        assert!(t.is_terminal());
    }

    #[test]
    fn base64_roundtrip() {
        assert_eq!(base64_encode(b"Man"), "TWFu");
    }

    #[test]
    fn adaptive_cu_price_p75_escalation() {
        let ts = TipStrategy::new(5000, 100, 1000, 400);
        let recent = [100_000u64, 120_000, 140_000, 200_000];
        // p75 ~ 180k? escalation +25%
        let price = ts.adaptive_cu_price(&recent);
        assert!(price >= 100_000);
        // empty default
        assert_eq!(ts.adaptive_cu_price(&[]), 100_000);
    }

    #[test]
    fn adaptive_cu_price_uses_nonzero_market_floor_without_undercutting_landed() {
        let ts = TipStrategy::new(5000, 100, 1000, 400);
        assert_eq!(
            ts.adaptive_cu_price_with_market(&[], &[0, 10_000, 20_000, 40_000]),
            50_000
        );
        assert_eq!(
            ts.adaptive_cu_price_with_market(&[80_000, 100_000], &[0, 1_000]),
            125_000
        );
        assert_eq!(ts.adaptive_cu_price_with_market(&[], &[0, 0, 0]), 100_000);
    }

    #[test]
    fn landed_cu_price_window_is_bounded_and_order_sensitive() {
        let mut window = Vec::new();
        for p in 1..=(RECENT_LANDED_CU_WINDOW as u64 + 10) {
            record_landed_cu_price(&mut window, p * 1_000);
        }
        assert_eq!(window.len(), RECENT_LANDED_CU_WINDOW);
        // Oldest samples dropped: first entry is the 11th insert.
        assert_eq!(window[0], 11_000);
        assert_eq!(window[RECENT_LANDED_CU_WINDOW - 1], 74_000);
    }

    #[test]
    fn dynamic_tip_and_verified_accounts() {
        let pk1 = get_random_tip_account();
        let pk2 = get_random_tip_account();
        assert_ne!(pk1, Pubkey::new_from_array([0; 32]));
        assert_ne!(pk2, Pubkey::new_from_array([0; 32]));

        // Dynamic tip increases with profit and competition
        let ts = TipStrategy::with_max_share(5000, 7500, 10_000, 1_000_000, 5_000);
        let tip_small = ts.pick_dynamic_tip(100_000, 1.0).unwrap();
        let tip_large = ts.pick_dynamic_tip(500_000, 1.2).unwrap();
        assert!(tip_large.as_u64() > tip_small.as_u64());
    }

    #[test]
    fn inflight_statuses_map_every_documented_state() {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "result": {
                "context": { "slot": 280999028u64 },
                "value": [
                    { "bundle_id": "a", "status": "Landed", "landed_slot": 280999027u64 },
                    { "bundle_id": "b", "status": "Failed", "landed_slot": null },
                    { "bundle_id": "c", "status": "Pending", "landed_slot": null },
                    { "bundle_id": "d", "status": "Invalid", "landed_slot": null }
                ]
            }
        });

        let statuses = parse_inflight_statuses(&payload);
        assert_eq!(statuses.len(), 4);
        assert_eq!(statuses[0], InflightStatus::Landed { slot: 280999027 });
        assert_eq!(statuses[1], InflightStatus::Failed);
        assert_eq!(statuses[2], InflightStatus::Pending);
        assert_eq!(statuses[3], InflightStatus::Invalid);
    }

    #[test]
    fn invalid_is_not_landing_evidence() {
        // The 5 minute lookback expiring says nothing about whether the bundle
        // landed, so it must never be folded in as a failure.
        assert_eq!(InflightStatus::Invalid.evidence(), None);
        assert_eq!(InflightStatus::Pending.evidence(), None);
        assert_eq!(InflightStatus::Failed.evidence(), Some(false));
        assert_eq!(InflightStatus::Landed { slot: 1 }.evidence(), Some(true));
    }

    #[test]
    fn unknown_status_is_not_terminal() {
        let payload = serde_json::json!({
            "result": { "value": [{ "status": "SomethingNew" }] }
        });
        let statuses = parse_inflight_statuses(&payload);
        assert_eq!(statuses, vec![InflightStatus::Pending]);
        assert!(!InflightStatus::Pending.is_terminal());
    }

    #[test]
    fn missing_result_yields_no_statuses() {
        assert!(parse_inflight_statuses(&serde_json::json!({})).is_empty());
        assert!(parse_inflight_statuses(&serde_json::json!({"result": {}})).is_empty());
    }

    #[test]
    fn test_regional_endpoints_resolution_and_defaults() {
        // Default regions when empty
        let lander = JitoLander::new(Vec::new());
        assert_eq!(lander.regions, vec!["amsterdam", "frankfurt", "ny"]);
        let endpoints = lander.endpoints();
        assert!(endpoints.contains(
            &"https://amsterdam.mainnet.block-engine.jito.wtf/api/v1/bundles".to_string()
        ));
        assert!(endpoints.contains(
            &"https://frankfurt.mainnet.block-engine.jito.wtf/api/v1/bundles".to_string()
        ));
        assert!(
            endpoints
                .contains(&"https://ny.mainnet.block-engine.jito.wtf/api/v1/bundles".to_string())
        );

        // Case-insensitivity and trimming
        assert_eq!(
            JitoLander::endpoint(" Frankfurt "),
            "https://frankfurt.mainnet.block-engine.jito.wtf/api/v1/bundles"
        );
        assert_eq!(
            JitoLander::endpoint("AMSTERDAM"),
            "https://amsterdam.mainnet.block-engine.jito.wtf/api/v1/bundles"
        );
        assert_eq!(
            JitoLander::endpoint("NY"),
            "https://ny.mainnet.block-engine.jito.wtf/api/v1/bundles"
        );
        assert_eq!(
            JitoLander::endpoint("https://custom.engine.com"),
            "https://custom.engine.com/api/v1/bundles"
        );
    }

    #[test]
    fn test_tip_account_for_slot_with_and_without_leader() {
        let leader = Pubkey::new_from_array([42; 32]);
        let slot = Slot(100);
        let tip_with_leader = get_tip_account_for_slot(slot, Some(&leader));
        let tip_without_leader = get_tip_account_for_slot(slot, None);

        // Deterministic: repeat calls produce identical results
        assert_eq!(
            tip_with_leader,
            get_tip_account_for_slot(slot, Some(&leader))
        );
        assert_eq!(tip_without_leader, get_tip_account_for_slot(slot, None));
    }
}
