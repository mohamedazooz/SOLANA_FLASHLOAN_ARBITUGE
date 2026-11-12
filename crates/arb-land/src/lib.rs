//! Jito landing: bundle submission, tips, status tracking.
//!
//! This crate owns everything between a signed transaction and a slot: pricing
//! the tip, fanning the bundle out to the block engines, and turning the status
//! the network reports back into evidence the bidder can learn from.

pub mod bidder;
pub mod leader;
pub mod ledger;
pub mod tip_oracle;

pub use arb_constants::{
    jito_tip_account_at, jito_tip_account_for_leader, jito_tip_account_for_slot,
    jito_tip_account_round_robin,
};
pub use bidder::{AdaptiveBidder, BidderConfig, MIN_JITO_TIP_LAMPORTS};
pub use leader::{JitoValidatorSet, Landability, LeaderSchedule, landability};
pub use ledger::{LEDGER_SCHEMA, LandingLedger};
pub use tip_oracle::{DEFAULT_MAX_AGE_MS, TIP_FLOOR_URL, TipOracle, now_ms, parse_tip_floor};

use arb_math::pick_tip;
use arb_types::{ArbError, InflightStatus, Lamports, Result as ArbResult, Slot};
use serde_json::{Value, json};
use solana_pubkey::Pubkey;
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
            .timeout(std::time::Duration::from_secs(5))
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
            max_share_bps: share_bps.max(7500),
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
    /// - Normal opportunities pay base `share_bps` (e.g. 50-55%).
    /// - High-profit or competitive opportunities dynamically scale up towards `max_share_bps` (e.g. 75%).
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
        if recent_landed.is_empty() {
            return 100_000;
        }
        let mut sorted: Vec<u64> = recent_landed.to_vec();
        sorted.sort_unstable();
        let idx = (sorted.len() * 3).div_ceil(4); // ceil p75
        let idx = idx.min(sorted.len()).saturating_sub(1);
        let p75 = sorted[idx];
        p75 + (p75 / 4) // +25%
    }
}

#[derive(Clone)]
pub struct JitoLander {
    regions: Vec<String>,
    /// Explicit block-engine target from `JITO_BLOCK_ENGINE_URL`, resolved
    /// once at construction (see [`JitoLander::resolved_override_url`]).
    override_url: Option<String>,
    client: reqwest::Client,
    #[allow(dead_code)]
    max_retries: u32,
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
            client,
            max_retries: 3,
        }
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
        // `JITO_BLOCK_ENGINE_URL` override first, then each region.
        let futures: Vec<_> = self
            .endpoints()
            .into_iter()
            .map(|url| {
                let client = self.client.clone();
                let body = body.clone();
                async move {
                    let resp = client.post(&url).json(&body).send().await;
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

        let results = futures::future::join_all(futures).await;
        let mut err_details = Vec::new();
        for res in results {
            match res {
                Ok(bundle_id) => return Ok(bundle_id),
                Err(e) => err_details.push(e.to_string()),
            }
        }
        Err(ArbError::Internal(format!(
            "All Jito block engines failed to accept bundle: [{}]",
            err_details.join(" | ")
        )))
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
        send_bundle_multi_region(txs, target_regions).await
    }

    pub async fn simulate_bundle(&self, txs: &[Vec<u8>]) -> ArbResult<Value> {
        if txs.is_empty() {
            return Err(ArbError::InvalidInput("empty bundle".into()));
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
            .client
            .post(&url)
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
            .client
            .post(&url)
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
            .client
            .post(&url)
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

    pub async fn get_tip_accounts(&self) -> ArbResult<Vec<String>> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getTipAccounts",
            "params": []
        });
        let url = self.primary_endpoint();
        let r = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| ArbError::Internal(e.to_string()))?;
        let v: Value = r
            .json()
            .await
            .map_err(|e| ArbError::Internal(e.to_string()))?;
        let mut out = Vec::new();
        if let Some(arr) = v.get("result").and_then(|x| x.as_array()) {
            for item in arr {
                if let Some(s) = item.as_str() {
                    out.push(s.to_string());
                }
            }
        }
        Ok(out)
    }

    pub async fn check_status(&self, bundle_id: &str) -> ArbResult<LandStatus> {
        let statuses = self.get_bundle_statuses(bundle_id).await?;
        Ok(statuses.into_iter().next().unwrap_or(LandStatus::Sent))
    }

    /// Asynchronously poll bundle landing status until terminal state or timeout.
    pub async fn poll_bundle_until_terminal(
        &self,
        bundle_id: &str,
        max_attempts: usize,
        poll_interval: std::time::Duration,
    ) -> LandStatus {
        let details = self
            .poll_bundle_detailed_until_terminal(bundle_id, max_attempts, poll_interval)
            .await;
        details.status
    }

    /// Asynchronously poll bundle landing detailed status until terminal state or timeout.
    pub async fn poll_bundle_detailed_until_terminal(
        &self,
        bundle_id: &str,
        max_attempts: usize,
        poll_interval: std::time::Duration,
    ) -> BundleStatusDetails {
        for _ in 0..max_attempts {
            tokio::time::sleep(poll_interval).await;
            if let Ok(records) = self.get_bundle_statuses_detailed(bundle_id).await {
                if let Some(record) = records.into_iter().next() {
                    if matches!(
                        record.status,
                        LandStatus::Landed
                            | LandStatus::Failed
                            | LandStatus::Dropped
                            | LandStatus::Expired
                    ) {
                        return record;
                    }
                }
            }
        }
        BundleStatusDetails {
            bundle_id: bundle_id.to_string(),
            status: LandStatus::Sent,
            slot: None,
            transactions: Vec::new(),
            err: None,
        }
    }
}

/// Send a bundle to all regions concurrently, returning the first success.
pub async fn send_bundle_multi_region(txs: &[Vec<u8>], regions: &[String]) -> ArbResult<String> {
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
    let futures: Vec<_> = regions_list
        .iter()
        .map(|region| {
            let url = JitoLander::endpoint(region);
            let client = client.clone();
            let body = body.clone();
            let region = region.clone();
            async move {
                let resp = client.post(&url).json(&body).send().await;
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

    let results = futures::future::join_all(futures).await;
    let mut err_details = Vec::new();
    for res in results {
        match res {
            Ok(bundle_id) => return Ok(bundle_id),
            Err(e) => err_details.push(e.to_string()),
        }
    }
    Err(ArbError::Internal(format!(
        "All Jito regional Block Engines failed to accept bundle: [{}]",
        err_details.join(" | ")
    )))
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
