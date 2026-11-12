//! Jito execution service.
//!
//! Wires the three independent signals that decide whether a tip is worth
//! paying into one decision:
//!
//! * the **market** — the landed tip distribution, polled by [`TipOracle`];
//! * the **evidence** — how often a bid at each rung actually landed, kept in
//!   [`LandingLedger`];
//! * the **schedule** — whether the leader of the target slot runs Jito-Solana
//!   at all, resolved by joining `getLeaderSchedule` with the kobe registry.
//!
//! Pricing without the third signal is how searchers burn tips against leaders
//! that have no Jito block engine in front of them, so a non-Jito leader
//! downgrades the route to the plain RPC path instead of paying for an auction
//! that does not exist.

use anyhow::{Context, Result};
use arb_ingest::RpcPool;
use arb_land::{
    AdaptiveBidder, BidderConfig, JitoLander, JitoValidatorSet, Landability, LandingLedger,
    LeaderSchedule, MIN_JITO_TIP_LAMPORTS, TipOracle, landability, now_ms,
};
use arb_types::{InflightStatus, Lamports, Percentile, RefuseReason, Slot, TipDecision, TipPlan};
use parking_lot::RwLock;
use serde_json::json;
use solana_pubkey::Pubkey;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

/// Public validator registry reporting `running_jito` per validator.
pub const KOBE_VALIDATORS_URL: &str = "https://kobe.mainnet.jito.network/api/v1/validators";

/// Where a signed transaction should be submitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionRoute {
    /// The slot's leader runs Jito-Solana: a bundle can be won.
    Bundle,
    /// The leader does not run Jito-Solana: a tip buys nothing, use plain RPC.
    PlainRpc,
    /// Direct SWQoS route with priority fee (fallback for profitable opportunities below Jito hurdle).
    SwqosDirect { priority_fee_lamports: u64 },
}

impl ExecutionRoute {
    /// True when the Jito bundle path applies.
    pub fn is_bundle(self) -> bool {
        matches!(self, Self::Bundle)
    }

    /// True when the SWQoS direct route applies.
    pub fn is_swqos(self) -> bool {
        matches!(self, Self::SwqosDirect { .. })
    }
}

/// A priced opportunity plus the route it should take.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionPlan {
    pub route: ExecutionRoute,
    pub tip: TipPlan,
}

impl ExecutionPlan {
    /// True when a bundle with the priced tip or direct SWQoS tx can be submitted.
    pub fn is_executable(&self) -> bool {
        (self.route.is_bundle() && self.tip.is_bid()) || self.route.is_swqos()
    }
}

/// Service configuration.
#[derive(Debug, Clone)]
pub struct JitoServiceConfig {
    /// Block-engine regions to fan out to. Empty routes to the global endpoint.
    pub regions: Vec<String>,
    /// Tip pricing policy.
    pub bidder: BidderConfig,
    /// Maximum age of a tip-floor snapshot before it is treated as absent.
    pub tip_floor_max_age_ms: u64,
    /// Tip-floor poll interval. The public endpoint allows one request per
    /// second per IP, so values below 1000 are clamped.
    pub tip_floor_poll_ms: u64,
    /// Validator registry endpoint.
    pub kobe_url: String,
    /// Where to persist the landing ledger. `None` keeps it in memory only.
    pub ledger_path: Option<PathBuf>,
    /// When the leader is unknown (stale schedule, failed registry fetch), the
    /// default must be fail-closed: do not send a bundle tip into a non-Jito
    /// slot. This remains overrideable by callers that intentionally want a
    /// legacy fail-open fallback.
    pub tip_on_unknown_leader: bool,
}

impl Default for JitoServiceConfig {
    fn default() -> Self {
        Self {
            regions: arb_land::DEFAULT_JITO_REGIONS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            bidder: BidderConfig::default(),
            tip_floor_max_age_ms: arb_land::DEFAULT_MAX_AGE_MS,
            tip_floor_poll_ms: 1_000,
            kobe_url: KOBE_VALIDATORS_URL.to_string(),
            ledger_path: None,
            tip_on_unknown_leader: false,
        }
    }
}

/// Owns tip pricing, leader gating and landing feedback.
pub struct JitoService {
    lander: JitoLander,
    oracle: Arc<TipOracle>,
    bidder: AdaptiveBidder,
    ledger: RwLock<LandingLedger>,
    schedule: RwLock<Option<LeaderSchedule>>,
    validators: RwLock<Option<JitoValidatorSet>>,
    config: JitoServiceConfig,
    http: reqwest::Client,
}

impl JitoService {
    /// Build the service, restoring the landing ledger from disk when configured.
    pub fn new(config: JitoServiceConfig) -> Result<Self> {
        let bidder = AdaptiveBidder::new(config.bidder)
            .map_err(|e| anyhow::anyhow!("invalid bidder config: {e}"))?;

        let ledger = match config.ledger_path.as_deref() {
            Some(path) => Self::load_ledger(path),
            None => LandingLedger::new(),
        };

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("build Jito service http client")?;

        Ok(Self {
            lander: JitoLander::new(config.regions.clone()),
            oracle: Arc::new(TipOracle::new(config.tip_floor_max_age_ms)),
            bidder,
            ledger: RwLock::new(ledger),
            schedule: RwLock::new(None),
            validators: RwLock::new(None),
            config,
            http,
        })
    }

    /// Tip-floor oracle, exposed so a caller can force a refresh before pricing.
    pub fn oracle(&self) -> Arc<TipOracle> {
        Arc::clone(&self.oracle)
    }

    /// Bundle lander.
    pub fn lander(&self) -> &JitoLander {
        &self.lander
    }

    /// Current landability of a slot.
    pub fn landability(&self, slot: Slot) -> Landability {
        let schedule = self.schedule.read();
        let validators = self.validators.read();
        landability(schedule.as_ref(), validators.as_ref(), slot)
    }

    /// True when a bundle sent for `slot` has an auction it can win.
    ///
    /// A known non-Jito leader has no block engine in front of it, so a tip
    /// buys nothing; an unknown leader is the configured fail-open case.
    pub fn bids_for(&self, slot: Slot) -> bool {
        match self.landability(slot) {
            Landability::Jito => true,
            Landability::NonJito => false,
            Landability::Unknown => self.config.tip_on_unknown_leader,
        }
    }

    /// Retrieve the validator identity leading the specified slot, if known.
    pub fn leader_for(&self, slot: Slot) -> Option<Pubkey> {
        self.schedule
            .read()
            .as_ref()
            .and_then(|s| s.leader_at(slot))
    }

    /// Retrieve the recommended Jito tip account for the target slot based on the leader schedule.
    ///
    /// Deterministically resolves the leader identity for the given slot from the inverted
    /// schedule, then maps that identity to one of the 8 verified Jito tip accounts.
    /// If the leader is unknown, falls back to deterministic slot-based indexing.
    pub fn tip_account_for_slot(&self, slot: Slot) -> Pubkey {
        let leader = self.leader_for(slot);
        arb_constants::jito_tip_account_for_slot(slot.0, leader.as_ref())
    }

    /// Price one opportunity and choose its route.
    ///
    /// `previous_tip` is the tip paid on the last attempt inside the same window;
    /// it bounds how far this attempt may escalate.
    pub fn plan(
        &self,
        slot: Slot,
        edge: Lamports,
        previous_tip: Option<Lamports>,
    ) -> ExecutionPlan {
        self.plan_with_context(slot, edge, previous_tip, 200_000, true)
    }

    /// Price one opportunity with runtime execution context (estimated CUs and pool contention).
    pub fn plan_with_context(
        &self,
        slot: Slot,
        edge: Lamports,
        previous_tip: Option<Lamports>,
        estimated_cu: u64,
        is_hot_write_lock: bool,
    ) -> ExecutionPlan {
        let market = self.oracle.current(now_ms());
        let tip = {
            let ledger = self.ledger.read();
            self.bidder.plan_with_context(
                edge,
                market,
                &ledger,
                previous_tip,
                estimated_cu,
                is_hot_write_lock,
            )
        };

        let route = if self.bids_for(slot) {
            if tip.is_bid() {
                ExecutionRoute::Bundle
            } else if !is_hot_write_lock && edge.0 >= 8_000 {
                // Hybrid SWQoS fallback: if trade is calm and profitable (covers 5k base + 2k priority + profit),
                // route directly to SWQoS!
                ExecutionRoute::SwqosDirect {
                    priority_fee_lamports: 2_000,
                }
            } else {
                ExecutionRoute::Bundle
            }
        } else if !is_hot_write_lock && edge.0 >= 8_000 {
            ExecutionRoute::SwqosDirect {
                priority_fee_lamports: 2_000,
            }
        } else {
            ExecutionRoute::PlainRpc
        };

        // Downgrading the route invalidates the price: there is no auction to
        // bid in, so the bid is refused rather than quietly sent elsewhere.
        // This outranks the bidder's own reason — the slot, not the market, is
        // what makes the opportunity unexecutable.
        if route == ExecutionRoute::PlainRpc {
            return ExecutionPlan {
                route,
                tip: TipPlan {
                    decision: TipDecision::Refuse {
                        reason: RefuseReason::LeaderNotJito,
                    },
                    inclusion_p: 0.0,
                    expected_value: 0,
                    ..tip
                },
            };
        }

        ExecutionPlan { route, tip }
    }

    /// Fold an observed bundle status into the landing ledger.
    ///
    /// Returns the evidence recorded, if any. `Pending` and `Invalid` carry none
    /// and leave the ledger untouched.
    pub fn record(&self, rung: Percentile, status: InflightStatus) -> Option<bool> {
        let evidence = status.evidence()?;
        self.ledger.write().record(rung, evidence);
        self.persist_ledger();
        Some(evidence)
    }

    /// Realised landing rate across every rung.
    pub fn landing_rate(&self) -> f64 {
        self.ledger.read().landing_rate()
    }

    /// Observations folded into the ledger.
    pub fn observations(&self) -> u64 {
        self.ledger.read().attempts()
    }

    /// Serialized ledger, for diagnostics.
    pub fn ledger_json(&self) -> Result<String> {
        self.ledger
            .read()
            .to_json()
            .context("serialize landing ledger")
    }

    /// Refresh the leader schedule and the Jito validator set.
    pub async fn refresh_leader(&self, rpc: &RpcPool) -> Result<()> {
        let info = rpc
            .call("getEpochInfo", json!([{ "commitment": "confirmed" }]))
            .await
            .context("getEpochInfo")?;

        // Solana's JSON-RPC returns these in camelCase (`absoluteSlot`,
        // `slotIndex`, `slotsInEpoch`). Reading only the snake_case spelling
        // made every leader refresh fail, which silently disabled the Jito
        // leader gate. Accept both so a snake_case mirror still works.
        let field = |obj: &serde_json::Value, camel: &str, snake: &str| -> Option<u64> {
            obj.get(camel)
                .or_else(|| obj.get(snake))
                .and_then(serde_json::Value::as_u64)
        };

        let epoch = field(&info, "epoch", "epoch").context("getEpochInfo missing epoch")?;
        let absolute_slot = field(&info, "absoluteSlot", "absolute_slot")
            .context("getEpochInfo missing absoluteSlot")?;
        let slot_index =
            field(&info, "slotIndex", "slot_index").context("getEpochInfo missing slotIndex")?;
        let slots_in_epoch = field(&info, "slotsInEpoch", "slots_in_epoch")
            .context("getEpochInfo missing slotsInEpoch")?;
        let epoch_start_slot = absolute_slot.saturating_sub(slot_index);

        // Several providers (Helius among them) return `null` for
        // `getLeaderSchedule(epoch)` and only serve the CURRENT epoch when the
        // method is called with no epoch argument. Asking for the epoch we just
        // read is still correct — but a null answer must fall back, not treat the
        // leader gate as broken forever.
        let schedule_json = match rpc
            .call(
                "getLeaderSchedule",
                json!([epoch, { "commitment": "confirmed" }]),
            )
            .await
        {
            Ok(v) if !v.is_null() => v,
            Ok(_) => rpc
                .call("getLeaderSchedule", json!([]))
                .await
                .context("getLeaderSchedule (current epoch fallback)")?,
            Err(e) => {
                return Err(e).context("getLeaderSchedule");
            }
        };
        let schedule =
            LeaderSchedule::from_rpc_json(&schedule_json, epoch, epoch_start_slot, slots_in_epoch)?;

        let validators = self.fetch_validators().await?;

        let indexed = schedule.len();
        *self.schedule.write() = Some(schedule);
        *self.validators.write() = Some(validators);

        info!(
            epoch,
            indexed_slots = indexed,
            jito_keys = self
                .validators
                .read()
                .as_ref()
                .map_or(0, JitoValidatorSet::len),
            "leader schedule refreshed"
        );
        Ok(())
    }

    /// Fetch the Jito validator registry.
    async fn fetch_validators(&self) -> Result<JitoValidatorSet> {
        let response = self
            .http
            .get(&self.config.kobe_url)
            .send()
            .await
            .context("kobe validators request")?;

        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("kobe validators returned HTTP {status}");
        }

        let body: serde_json::Value = response.json().await.context("kobe validators body")?;
        let set = JitoValidatorSet::from_kobe_json(&body)?;
        if set.is_empty() {
            anyhow::bail!("kobe returned no validator running Jito-Solana");
        }
        Ok(set)
    }

    /// Poll one bundle until it reaches a terminal status or the attempts run out.
    pub async fn poll_until_terminal(
        &self,
        bundle_id: &str,
        attempts: u32,
        interval: Duration,
    ) -> InflightStatus {
        for _ in 0..attempts {
            tokio::time::sleep(interval).await;
            match self.lander.get_inflight_statuses(&[bundle_id]).await {
                Ok(statuses) => {
                    if let Some(status) = statuses.first().copied()
                        && status.is_terminal()
                    {
                        return status;
                    }
                }
                Err(e) => warn!("inflight status poll failed: {e}"),
            }
        }
        InflightStatus::Pending
    }

    /// Comprehensive landing verification loop combining Jito `getBundleStatuses` and
    /// Solana RPC `getSignatureStatuses`.
    ///
    /// Cross-checks:
    /// - If Jito reports Landed, verifies on-chain execution with Solana RPC to confirm no revert.
    /// - If Jito API lags or drops bundle status from cache, checks Solana RPC directly via signature.
    /// - If Jito reports Dropped, double-checks RPC so confirmed on-chain transactions are not misclassified.
    pub async fn verify_landing(
        &self,
        rpc: &RpcPool,
        bundle_id: &str,
        tx_signature: Option<&str>,
        attempts: u32,
        interval: Duration,
    ) -> (arb_land::LandStatus, Option<u64>, Option<String>) {
        let mut landed_slot = None;
        let mut observed_err = None;

        for _ in 0..attempts {
            tokio::time::sleep(interval).await;

            // Track 1: Jito Block Engine getBundleStatuses
            if let Ok(records) = self.lander.get_bundle_statuses_detailed(bundle_id).await {
                if let Some(record) = records.first() {
                    if let Some(slot) = record.slot {
                        landed_slot = Some(slot);
                    }
                    if let Some(ref err) = record.err {
                        observed_err = Some(err.clone());
                    }

                    match record.status {
                        arb_land::LandStatus::Landed => {
                            // Cross-verify on-chain execution with Solana RPC if signature is available
                            let sig = tx_signature
                                .or_else(|| record.transactions.first().map(String::as_str));
                            if let Some(s) = sig {
                                if let Ok(sig_status) = rpc.get_signature_status(s).await {
                                    if sig_status == arb_ingest::SignatureStatus::Failed {
                                        return (
                                            arb_land::LandStatus::Failed,
                                            landed_slot,
                                            observed_err.or_else(|| {
                                                Some("transaction reverted on chain".into())
                                            }),
                                        );
                                    }
                                }
                            }
                            return (arb_land::LandStatus::Landed, landed_slot, None);
                        }
                        arb_land::LandStatus::Failed => {
                            return (arb_land::LandStatus::Failed, landed_slot, observed_err);
                        }
                        arb_land::LandStatus::Dropped | arb_land::LandStatus::Expired => {
                            // Before accepting drop/expiration, check if transaction landed on chain anyway
                            if let Some(s) = tx_signature {
                                if let Ok(sig_status) = rpc.get_signature_status(s).await {
                                    if sig_status.is_landed() {
                                        return (arb_land::LandStatus::Landed, landed_slot, None);
                                    }
                                }
                            }
                            return (record.status.clone(), landed_slot, observed_err);
                        }
                        arb_land::LandStatus::Sent => {}
                    }
                }
            }

            // Track 2: Direct on-chain verification via RPC getSignatureStatuses
            if let Some(s) = tx_signature {
                if let Ok(sig_status) = rpc.get_signature_status(s).await {
                    match sig_status {
                        arb_ingest::SignatureStatus::Confirmed
                        | arb_ingest::SignatureStatus::Finalized => {
                            return (arb_land::LandStatus::Landed, landed_slot, None);
                        }
                        arb_ingest::SignatureStatus::Failed => {
                            return (
                                arb_land::LandStatus::Failed,
                                landed_slot,
                                Some("transaction failed on chain".into()),
                            );
                        }
                        arb_ingest::SignatureStatus::Processed
                        | arb_ingest::SignatureStatus::Pending => {}
                    }
                }
            }
        }

        // Final check via RPC with history search
        if let Some(s) = tx_signature {
            if let Ok(sig_status) = rpc.get_signature_status(s).await {
                if sig_status.is_landed() {
                    return (arb_land::LandStatus::Landed, landed_slot, None);
                }
            }
        }

        (arb_land::LandStatus::Expired, landed_slot, observed_err)
    }

    /// Run the background refreshers until the task is dropped.
    pub async fn run(self: Arc<Self>, rpc: Arc<RpcPool>, leader_refresh: Duration) {
        let oracle = Arc::clone(&self.oracle);
        let poll = Duration::from_millis(self.config.tip_floor_poll_ms.max(1_000));
        tokio::spawn(async move { oracle.run(poll).await });

        let mut ticker = tokio::time::interval(leader_refresh);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Err(e) = self.refresh_leader(&rpc).await {
                warn!("leader refresh failed: {e:#}");
            }
        }
    }

    /// Minimum tip the block engine will consider.
    pub fn minimum_tip() -> Lamports {
        Lamports(MIN_JITO_TIP_LAMPORTS)
    }

    /// Restore a ledger, treating an unreadable or foreign snapshot as empty.
    fn load_ledger(path: &Path) -> LandingLedger {
        let Ok(json) = std::fs::read_to_string(path) else {
            return LandingLedger::new();
        };
        match LandingLedger::from_json(&json) {
            Ok(ledger) => {
                info!(
                    attempts = ledger.attempts(),
                    landings = ledger.landings(),
                    "landing ledger restored"
                );
                ledger
            }
            Err(e) => {
                warn!("landing ledger unreadable, starting fresh: {e}");
                LandingLedger::new()
            }
        }
    }

    /// Write the ledger back to disk after every observation.
    ///
    /// At the bot's order velocity this is a few hundred bytes a handful of
    /// times per hour, and an interrupted write is rejected on load rather than
    /// misread, because the schema gate runs after parsing.
    fn persist_ledger(&self) {
        let Some(path) = self.config.ledger_path.as_ref() else {
            return;
        };
        let json = match self.ledger.read().to_json() {
            Ok(json) => json,
            Err(e) => {
                warn!("landing ledger serialize failed: {e}");
                return;
            }
        };
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(path, json) {
            warn!("landing ledger persist failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_bidder_config() {
        let config = JitoServiceConfig {
            bidder: BidderConfig {
                keep_share: -1.0,
                ..BidderConfig::default()
            },
            ..JitoServiceConfig::default()
        };
        assert!(JitoService::new(config).is_err());
    }

    #[test]
    fn unknown_leader_routes_to_the_bundle_when_explicitly_allowed() {
        let config = JitoServiceConfig {
            tip_on_unknown_leader: true,
            ..JitoServiceConfig::default()
        };
        let service = JitoService::new(config).expect("service");
        // No schedule, no validator set: every slot is Unknown.
        assert_eq!(service.landability(Slot(1)), Landability::Unknown);

        let plan = service.plan(Slot(1), Lamports(1_000_000), None);
        assert_eq!(plan.route, ExecutionRoute::Bundle);
        // The snapshot is what is missing here, not the leader: the refusal
        // must blame the market so the operator debugs the right feed.
        assert_eq!(
            plan.tip.decision,
            TipDecision::Refuse {
                reason: RefuseReason::MarketUnavailable
            }
        );
        assert!(!plan.is_executable());
    }

    #[test]
    fn unknown_leader_refuses_by_default() {
        let service = JitoService::new(JitoServiceConfig::default()).expect("service");
        let plan = service.plan(Slot(1), Lamports(1_000_000), None);
        assert_eq!(plan.route, ExecutionRoute::PlainRpc);
        assert!(!plan.tip.is_bid());
        assert_eq!(
            plan.tip.decision,
            TipDecision::Refuse {
                reason: RefuseReason::LeaderNotJito
            }
        );
    }

    #[test]
    fn ledger_records_only_evidence() {
        let service = JitoService::new(JitoServiceConfig::default()).expect("service");
        assert_eq!(service.observations(), 0);

        assert_eq!(
            service.record(Percentile::P90, InflightStatus::Pending),
            None
        );
        assert_eq!(
            service.record(Percentile::P90, InflightStatus::Invalid),
            None
        );
        assert_eq!(service.observations(), 0);

        assert_eq!(
            service.record(Percentile::P90, InflightStatus::Landed { slot: 42 }),
            Some(true)
        );
        assert_eq!(
            service.record(Percentile::P90, InflightStatus::Failed),
            Some(false)
        );
        assert_eq!(service.observations(), 2);
        assert!((service.landing_rate() - 0.5).abs() < 1e-12);
    }

    #[test]
    fn minimum_tip_matches_the_block_engine_floor() {
        assert_eq!(JitoService::minimum_tip().0, 1_000);
    }

    /// The numbers `config.toml` ships with, so these tests price the way the
    /// bot actually does rather than against the library defaults.
    fn shipped_config() -> JitoServiceConfig {
        JitoServiceConfig {
            bidder: BidderConfig {
                keep_share: 0.85,
                max_tip_share: 0.70,
                profit_floor: Lamports(20_000),
                ladder_growth: 1.6,
            },
            ..JitoServiceConfig::default()
        }
    }

    #[test]
    fn refuses_when_the_ceiling_cannot_fund_the_minimum_tip() {
        let config = JitoServiceConfig {
            tip_on_unknown_leader: true,
            ..shipped_config()
        };
        let service = JitoService::new(config).expect("service");
        // Edge 22_500: with CU contention overhead the hurdle is 22_000, leaving a 500-lamport ceiling,
        // below the 1_000 the block engine accepts. Bidding it would either be
        // ignored by the auction or breach the risk budget.
        let plan = service.plan(Slot(1), Lamports(22_500), None);

        assert!(!plan.is_executable());
        assert_eq!(
            plan.tip.decision,
            TipDecision::Refuse {
                reason: RefuseReason::EdgeBelowMinimumTip
            }
        );
        assert_eq!(plan.tip.ceiling, Lamports(500));
    }

    #[test]
    fn refuses_when_the_tip_market_is_missing() {
        let config = JitoServiceConfig {
            tip_on_unknown_leader: true,
            ..shipped_config()
        };
        let service = JitoService::new(config).expect("service");
        // No oracle snapshot yet. The edge is comfortably above the hurdle, so
        // the only thing that stops a bid is the absent market — and inventing
        // a price here is what would pay 85% of a 1M-lamport edge as a tip.
        let plan = service.plan(Slot(1), Lamports(1_000_000), None);

        assert_eq!(plan.route, ExecutionRoute::Bundle);
        assert_eq!(
            plan.tip.decision,
            TipDecision::Refuse {
                reason: RefuseReason::MarketUnavailable
            }
        );
        assert_eq!(plan.tip.ceiling, Lamports(150_000));
        assert!(!plan.is_executable());
    }

    #[test]
    fn a_retry_cannot_escalate_past_the_ladder_cap() {
        let config = JitoServiceConfig {
            tip_on_unknown_leader: true,
            ..shipped_config()
        };
        let service = JitoService::new(config).expect("service");
        service.oracle().publish(arb_types::TipFloor {
            observed_at_ms: now_ms(),
            p25: Lamports(1_000),
            p50: Lamports(2_000),
            p75: Lamports(50_000),
            p95: Lamports(100_000),
            p99: Lamports(200_000),
            ema50: Lamports(2_000),
        });

        // Edge 1_000_000: ceiling 150_000, so the 100k p95 rung is affordable
        // and wins on EV (0.95 * 900_000 beats every cheaper rung).
        let first = service.plan(Slot(1), Lamports(1_000_000), None);
        let TipDecision::Bid {
            tip,
            rung,
            escalated,
        } = first.tip.decision
        else {
            panic!("expected a bid, got {:?}", first.tip.decision);
        };
        assert_eq!(tip, Lamports(100_000));
        assert_eq!(rung, Percentile::P95);
        assert!(!escalated);

        // The retry sees the same market, but the previous attempt bounds it:
        // 1.6 * 50_000 = 80_000, so the p95 rung is held back rather than
        // outbidding the attempt that already lost this slot.
        let retry = service.plan(Slot(1), Lamports(1_000_000), Some(Lamports(50_000)));
        let TipDecision::Bid { tip, escalated, .. } = retry.tip.decision else {
            panic!("expected a bid, got {:?}", retry.tip.decision);
        };
        assert_eq!(tip, Lamports(80_000));
        assert!(escalated);
        assert!(retry.is_executable());
    }

    #[test]
    fn ledger_survives_a_restart() {
        let dir = std::env::temp_dir().join(format!("arb-jito-ledger-test-{}", std::process::id()));
        let path = dir.join("ledger.json");
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::remove_file(&path);

        {
            let service = JitoService::new(JitoServiceConfig {
                ledger_path: Some(path.clone()),
                ..JitoServiceConfig::default()
            })
            .expect("service");
            for _ in 0..5 {
                service.record(Percentile::P75, InflightStatus::Landed { slot: 1 });
            }
            for _ in 0..5 {
                service.record(Percentile::P75, InflightStatus::Failed);
            }
            assert_eq!(service.observations(), 10);
        }

        let restored = JitoService::new(JitoServiceConfig {
            ledger_path: Some(path.clone()),
            ..JitoServiceConfig::default()
        })
        .expect("service");
        assert_eq!(restored.observations(), 10);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn default_config_has_multi_region_endpoints() {
        let config = JitoServiceConfig::default();
        assert_eq!(config.regions, vec!["amsterdam", "frankfurt", "ny"]);
    }

    #[test]
    fn tip_account_for_slot_determines_account_consistently() {
        let service = JitoService::new(JitoServiceConfig::default()).expect("service");
        let tip1 = service.tip_account_for_slot(Slot(100));
        let tip2 = service.tip_account_for_slot(Slot(100));
        assert_eq!(tip1, tip2);
        assert_ne!(tip1, Pubkey::new_from_array([0; 32]));
    }
}
