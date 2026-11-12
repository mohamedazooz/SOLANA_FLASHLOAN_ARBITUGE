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
