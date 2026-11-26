//! Opportunity History Persistence and Deterministic Shadow Replay Engine.
//!
//! Solves requirement P6:
//! 1. Persists every evaluated candidate route and outcome to structured append-only logs.
//! 2. Tracks detection timestamp, pool states/slots, quotes, fee breakdown, gross/net profit,
//!    simulation results, disappearance time, rejection reason, quote freshness, and simulation latency.
//! 3. Provides a deterministic shadow replay engine so code changes can be verified against
//!    identical recorded market states.

use arb_types::currency::FeeItem;
use arb_types::{ArbError, PoolState, Result};
use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// Comprehensive historical opportunity record capturing complete market state and outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateOpportunityRecord {
    pub candidate_id: String,
    pub detection_timestamp_ms: u64,
    pub pool_a_id: Pubkey,
    pub pool_b_id: Pubkey,
    pub pool_a_slot: u64,
    pub pool_b_slot: u64,
    pub quote_slot: u64,
    pub pool_a_state: PoolState,
    pub pool_b_state: PoolState,
    pub direction: String,
    pub trade_size_in: u64,
    pub hop1_quote_out: u64,
    pub hop2_final_out: u64,
    pub fee_items: Vec<FeeItem>,
    pub gross_profit_quote_raw: i64,
    pub total_execution_cost_quote_raw: u64,
    pub net_profit_quote_raw: i64,
    pub net_profit_bps: i64,
    pub simulation_success: bool,
    pub simulation_slot: Option<u64>,
    pub simulation_latency_ms: u64,
    pub quote_freshness_cleared: bool,
    pub disappearance_time_ms: Option<u64>,
    pub candidate_lifetime_ms: Option<u64>,
    pub best_observed_net_profit: Option<i64>,
    pub rejection_reason: Option<String>,
    pub status: String,
}

impl CandidateOpportunityRecord {
    /// Typed qualification stage derived conservatively from the record's own
    /// fields. This is the canonical answer to "what has been proven about
    /// this candidate"; the free-form `status` string remains only for
    /// historical compatibility.
    ///
    /// Semantics:
    /// - no hop output ⇒ `Discovered`;
    /// - non-positive net profit ⇒ `QuoteValid` (economics NOT proven);
    /// - positive net profit without a preflight ⇒ `EconomicallyPositive`;
    /// - successful preflight ⇒ `Simulated` (the ladder's progression order
    ///   is NOT a logical implication: a negative-edge successful simulation
    ///   reports `Simulated` while its economics stay unproven);
    /// - `ShadowValidated` / `MainnetVerified` are NEVER derived here because
    ///   quote fields alone cannot prove continuity checks or settled
    ///   balance deltas.
    pub fn stage(&self) -> arb_types::OpportunityStage {
        use arb_types::OpportunityStage;

        if self.hop1_quote_out == 0 || self.hop2_final_out == 0 {
            return OpportunityStage::Discovered;
        }
        if self.simulation_success {
            return OpportunityStage::Simulated;
        }
        if self.net_profit_quote_raw > 0 {
            OpportunityStage::EconomicallyPositive
        } else {
            OpportunityStage::QuoteValid
        }
    }
}

pub struct OpportunityHistoryRecorder {
    log_dir: PathBuf,
}

impl OpportunityHistoryRecorder {
    pub fn new(log_dir: impl AsRef<Path>) -> Self {
        let p = log_dir.as_ref().to_path_buf();
        let _ = std::fs::create_dir_all(&p);
        Self { log_dir: p }
    }

    pub fn record_candidate(&self, record: &CandidateOpportunityRecord) -> Result<()> {
        let file_path = self.log_dir.join("candidates.jsonl");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)
            .map_err(|e| ArbError::Internal(format!("Failed to open candidate log: {e}")))?;

        let line = serde_json::to_string(record)
            .map_err(|e| ArbError::Internal(format!("Failed to serialize record: {e}")))?;
        writeln!(file, "{}", line)
            .map_err(|e| ArbError::Internal(format!("Failed to write candidate log: {e}")))?;

        // Also save single snapshot fixture for individual route replay
        let single_fixture_path = self.log_dir.join(format!("{}.json", record.candidate_id));
        if let Ok(mut sf) = File::create(single_fixture_path) {
            let pretty = serde_json::to_string_pretty(record).unwrap_or_default();
            let _ = sf.write_all(pretty.as_bytes());
        }

        Ok(())
    }

    pub fn load_candidates(&self) -> Result<Vec<CandidateOpportunityRecord>> {
        let file_path = self.log_dir.join("candidates.jsonl");
        if !file_path.exists() {
            return Ok(Vec::new());
        }
        let file = File::open(&file_path)
            .map_err(|e| ArbError::Internal(format!("Failed to read candidates: {e}")))?;
        let reader = BufReader::new(file);
        let mut records = Vec::new();
        for line in reader.lines() {
            let line = line.map_err(|e| ArbError::Internal(e.to_string()))?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(rec) = serde_json::from_str::<CandidateOpportunityRecord>(trimmed) {
                records.push(rec);
            }
        }
        Ok(records)
    }

    /// Replay an opportunity against recorded pool states and verify deterministic quote reproduction.
    pub fn replay_candidate_route(record: &CandidateOpportunityRecord) -> Result<bool> {
        let pair = arb_engine::MeteoraDlmmPoolPairDiscovery::discover_pairs(&[
            record.pool_a_state.clone(),
            record.pool_b_state.clone(),
        ]);
        if pair.distinct_eligible_pool_pairs == 0 {
            return Ok(false);
        }
        let eligible = &pair.eligible_pairs[0];
        let quote = if record.direction == "DirectionA" {
            arb_engine::MeteoraDlmmCrossPoolQuoteEngine::quote_direction_a(
                eligible,
                record.trade_size_in,
            )
        } else {
            arb_engine::MeteoraDlmmCrossPoolQuoteEngine::quote_direction_b(
                eligible,
                record.trade_size_in,
            )
        };

        if let Some(q) = quote {
            let match_out = q.hop1_amount_out == record.hop1_quote_out
                && q.hop2_final_amount_out == record.hop2_final_out;
            let match_gross = q.gross_profit == record.gross_profit_quote_raw;
            Ok(match_out && match_gross)
        } else {
            Ok(false)
        }
    }
}

#[cfg(test)]
mod stage_derivation_tests {
    use super::*;
    use arb_types::{
        DexProtocol, MintId, OpportunityStage, PoolId, PoolKind, Slot, TokenProgramKind,
    };

    fn test_pool_state() -> PoolState {
        PoolState {
            pool_id: PoolId(Pubkey::new_unique()),
            protocol: DexProtocol::MeteoraDlmm,
            slot: Slot(1),
            write_version: 1,
            base_mint: MintId(Pubkey::new_unique()),
            quote_mint: MintId(Pubkey::new_unique()),
            decimals_a: 9,
            decimals_b: 6,
            reserve_in: 50_000_000_000,
            reserve_out: 50_000_000_000,
            fee_numerator: 20,
            fee_denominator: 10_000,
            raydium_cpmm_fee: None,
            is_active: true,
            has_risky_token: false,
            kind: PoolKind::Dlmm,
            clmm: None,
            dlmm: None,
            pump_fun: None,
            damm_v2: None,
            transfer_fee_bps: 0,
            token_programs: [TokenProgramKind::SplToken, TokenProgramKind::SplToken],
            execution_accounts: None,
            last_update_slot: 1,
            last_update_ms: 0,
        }
    }

    fn base_record() -> CandidateOpportunityRecord {
        CandidateOpportunityRecord {
            candidate_id: "test-candidate".to_string(),
            detection_timestamp_ms: 0,
            pool_a_id: Pubkey::new_unique(),
            pool_b_id: Pubkey::new_unique(),
            pool_a_slot: 1,
            pool_b_slot: 1,
            quote_slot: 1,
            pool_a_state: test_pool_state(),
            pool_b_state: test_pool_state(),
            direction: "DirectionA".to_string(),
            trade_size_in: 1_000,
            hop1_quote_out: 0,
            hop2_final_out: 0,
            fee_items: vec![],
            gross_profit_quote_raw: 0,
            total_execution_cost_quote_raw: 0,
            net_profit_quote_raw: 0,
            net_profit_bps: 0,
            simulation_success: false,
            simulation_slot: None,
            simulation_latency_ms: 0,
            quote_freshness_cleared: false,
            disappearance_time_ms: None,
            candidate_lifetime_ms: None,
            best_observed_net_profit: None,
            rejection_reason: None,
            status: String::new(),
        }
    }

    #[test]
    fn no_hop_output_is_discovered() {
        let mut r = base_record();
        r.hop1_quote_out = 0;
        r.hop2_final_out = 500;
        assert_eq!(r.stage(), OpportunityStage::Discovered);
        r.hop1_quote_out = 500;
        r.hop2_final_out = 0;
        assert_eq!(r.stage(), OpportunityStage::Discovered);
    }

    #[test]
    fn non_positive_net_profit_never_reaches_economic_stage() {
        let mut r = base_record();
        r.hop1_quote_out = 900;
        r.hop2_final_out = 990;
        r.net_profit_quote_raw = 0;
        assert_eq!(r.stage(), OpportunityStage::QuoteValid);
        r.net_profit_quote_raw = -5;
        assert_eq!(r.stage(), OpportunityStage::QuoteValid);
    }

    #[test]
    fn positive_net_without_preflight_is_economically_positive() {
        let mut r = base_record();
        r.hop1_quote_out = 900;
        r.hop2_final_out = 1_100;
        r.net_profit_quote_raw = 100;
        assert_eq!(r.stage(), OpportunityStage::EconomicallyPositive);
    }

    #[test]
    fn failed_simulation_does_not_claim_simulated_stage() {
        let mut r = base_record();
        r.hop1_quote_out = 900;
        r.hop2_final_out = 1_100;
        r.net_profit_quote_raw = 100;
        r.simulation_success = false;
        r.simulation_slot = Some(2);
        r.simulation_latency_ms = 25;
        r.rejection_reason = Some("SimulationFailed".to_string());
        r.status = "REJECTED_SIM_FAIL".to_string();
        assert_eq!(r.stage(), OpportunityStage::EconomicallyPositive);
        assert_eq!(
            OpportunityStage::from_legacy_status(&r.status),
            Some(OpportunityStage::Simulated)
        );
    }

    #[test]
    fn successful_simulation_is_simulated_never_higher() {
        let mut r = base_record();
        r.hop1_quote_out = 900;
        r.hop2_final_out = 1_100;
        r.net_profit_quote_raw = 100;
        r.simulation_success = true;
        r.simulation_slot = Some(2);
        r.simulation_latency_ms = 25;
        r.status = "VERIFIED_PROFITABLE".to_string();
        assert_eq!(r.stage(), OpportunityStage::Simulated);
        assert!(r.stage() < OpportunityStage::ShadowValidated);
        assert!(r.stage() < OpportunityStage::MainnetVerified);
    }

    #[test]
    fn negative_edge_successful_simulation_reports_simulated() {
        let mut r = base_record();
        r.hop1_quote_out = 900;
        r.hop2_final_out = 850;
        r.net_profit_quote_raw = -50;
        r.simulation_success = true;
        r.simulation_slot = Some(2);
        r.simulation_latency_ms = 30;
        r.status = "SIMULATED_SUCCESS_NEGATIVE_EDGE".to_string();
        assert_eq!(r.stage(), OpportunityStage::Simulated);
    }

    #[test]
    fn legacy_jsonl_records_without_stage_field_still_deserialize() {
        let mut r = base_record();
        r.hop1_quote_out = 900;
        r.hop2_final_out = 1_100;
        r.net_profit_quote_raw = 100;
        r.simulation_success = true;
        let mut json = serde_json::to_string(&r).expect("serialize");
        assert!(
            !json.contains("stage"),
            "record schema must not embed a stage field"
        );
        let repaired = json.replace("\"status\":\"\"", "\"status\":\"VERIFIED_PROFITABLE\"");
        json = repaired;
        let parsed: CandidateOpportunityRecord = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.stage(), OpportunityStage::Simulated);
    }
}
