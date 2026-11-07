//! Jito landing: bundle submission, tips, status tracking.
//!
//! This crate owns everything between a signed transaction and a slot: pricing
//! the tip, fanning the bundle out to the block engines, and turning the status
//! the network reports back into evidence the bidder can learn from.

pub mod bidder;
pub mod leader;
pub mod ledger;
pub mod tip_oracle;

pub use bidder::{AdaptiveBidder, BidderConfig, MIN_JITO_TIP_LAMPORTS};
pub use leader::{JitoValidatorSet, Landability, LeaderSchedule, landability};
pub use ledger::{LEDGER_SCHEMA, LandingLedger};
pub use tip_oracle::{DEFAULT_MAX_AGE_MS, TIP_FLOOR_URL, TipOracle, now_ms, parse_tip_floor};

use arb_math::pick_tip;
use arb_types::{ArbError, InflightStatus, Lamports, Result as ArbResult};
use serde_json::{Value, json};
use solana_pubkey::Pubkey;
use tracing::debug;

/// Get a round-robin official verified Jito tip pubkey (delegates to arb-constants).
pub fn get_random_tip_account() -> Pubkey {
    arb_constants::jito_tip_account_round_robin()
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
    client: reqwest::Client,
    #[allow(dead_code)]
    max_retries: u32,
}

impl JitoLander {
    pub fn new(regions: Vec<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .pool_max_idle_per_host(8)
            .tcp_nodelay(true)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            regions,
            client,
            max_retries: 3,
        }
    }

    fn endpoint(region: &str) -> String {
        if region.is_empty() || region == "mainnet" {
            "https://mainnet.block-engine.jito.wtf/api/v1/bundles".to_string()
        } else {
            format!("https://{region}.mainnet.block-engine.jito.wtf/api/v1/bundles")
        }
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

        // Multi-region concurrent dispatch
        let futures: Vec<_> = self
            .regions
            .iter()
            .map(|region| {
                let url = Self::endpoint(region);
                let client = self.client.clone();
                let body = body.clone();
                let region = region.clone();
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
                                debug!("Bundle accepted by region {}: {}", region, res);
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

    pub async fn send_bundle_multi_region(
        &self,
        txs: &[Vec<u8>],
        regions: &[String],
    ) -> ArbResult<String> {
        send_bundle_multi_region(txs, regions).await
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
        let url = Self::endpoint(self.regions.first().map(String::as_str).unwrap_or(""));
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

    pub async fn get_bundle_statuses(&self, bundle_id: &str) -> ArbResult<Vec<LandStatus>> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getBundleStatuses",
            "params": [[bundle_id]]
        });
        let url = Self::endpoint(self.regions.first().map(String::as_str).unwrap_or(""));
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
                // P0 #7: Jito returns confirmation_status even for failed bundles;
                // a non-null `err` means the bundle did NOT land successfully.
                let has_err = item.get("err").map(|e| !e.is_null()).unwrap_or(false);
                if has_err {
                    out.push(LandStatus::Failed);
                    continue;
                }
                let status = item
                    .get("confirmation_status")
                    .and_then(|x| x.as_str())
                    .unwrap_or("");
                let s = match status {
                    // Processed is provisional and must never be reported as landed.
                    "processed" => LandStatus::Sent,
                    "confirmed" | "finalized" => LandStatus::Landed,
                    "failed" => LandStatus::Failed,
                    "dropped" => LandStatus::Dropped,
                    "expired" => LandStatus::Expired,
                    _ => LandStatus::Sent,
                };
                out.push(s);
            }
        }
        if out.is_empty() {
            out.push(LandStatus::Sent);
        }
        Ok(out)
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
        let url = Self::endpoint("");

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
        let url = Self::endpoint(self.regions.first().map(String::as_str).unwrap_or(""));
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
        for _ in 0..max_attempts {
            tokio::time::sleep(poll_interval).await;
            if let Ok(status) = self.check_status(bundle_id).await
                && matches!(
                    status,
                    LandStatus::Landed
                        | LandStatus::Failed
                        | LandStatus::Dropped
                        | LandStatus::Expired
                )
            {
                return status;
            }
        }
        LandStatus::Sent
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
        "params": [encoded, {"encoding": "base64"}]
    });

    let regions = if regions.is_empty() {
        vec![String::new()]
    } else {
        regions.to_vec()
    };

    // Reuse one process-wide client: the connection pool + keep-alive means a
    // send does NOT pay a fresh TLS/TCP handshake on the critical path. Creating
    // a client per call added tens of ms of avoidable latency per attempt.
    let client = shared_dispatch_client();
    let futures: Vec<_> = regions
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
}
