//! Paper-Trading Statistical Evaluation Report (Gate H — economic evidence).
//!
//! Consumes the append-only opportunity history written by
//! [`crate::opportunity_recorder::OpportunityHistoryRecorder`] and produces the
//! measured statistics required before any live-trading trust:
//!
//! - Detection / acceptance / rejection counts with rejection-reason breakdown.
//! - Quote accuracy: deterministic replay of each recorded candidate against
//!   its captured pool states (via
//!   [`crate::opportunity_recorder::OpportunityHistoryRecorder::replay_candidate_route`]).
//! - Opportunity decay: detection → disappearance lifetime distribution.
//! - Economics: gross vs net totals, execution costs, PnL distribution and
//!   cumulative drawdown over the recorded sequence.
//! - A sample-size confidence note — never a fabricated significance claim.
//!
//! This module only reports on recorded data. It never signs, submits, or
//! authorizes anything, and it never treats a positive theoretical quote as
//! realized profit.

use crate::opportunity_recorder::CandidateOpportunityRecord;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Minimum sample size below which no statistical significance is claimed.
const MIN_SIGNIFICANT_SAMPLE: usize = 100;

/// Acceptance status emitted by the profitability pipeline.
const STATUS_ACCEPTED: &str = "VERIFIED_PROFITABLE";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Counts {
    pub detected: usize,
    pub accepted: usize,
    pub rejected: usize,
    pub simulation_failed: usize,
    pub stale_quote: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QuoteAccuracy {
    /// Candidates with recorded pool states eligible for deterministic replay.
    pub replayed: usize,
    /// Replays whose outputs exactly matched the recorded quotes.
    pub reproduced: usize,
    /// `reproduced / replayed` in basis points (0 when `replayed == 0`).
    pub reproduction_rate_bps: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DecayStats {
    /// Candidates whose disappearance was observed.
    pub observed: usize,
    /// Median candidate lifetime in ms (0 when unobserved).
    pub median_lifetime_ms: u64,
    /// p90 candidate lifetime in ms.
    pub p90_lifetime_ms: u64,
    /// Candidates with no observed disappearance within the recording window.
    pub still_alive_at_end: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PnlStats {
    /// Sum of gross profits (quote-mint raw units) over all records.
    pub gross_total_raw: i64,
    /// Sum of net profits (quote-mint raw units) over all records.
    pub net_total_raw: i64,
    /// Sum of total execution costs (quote-mint raw units).
    pub execution_cost_total_raw: u64,
    pub positive_net_count: usize,
    pub negative_net_count: usize,
    /// Mean net profit over accepted candidates only (raw units, 0 when none).
    pub mean_net_per_accepted_raw: i64,
    /// Maximum peak-to-trough decline of the cumulative net-profit curve.
    pub max_drawdown_raw: i64,
    /// Net profit of the single best record.
    pub best_net_raw: i64,
    /// Net profit of the single worst record.
    pub worst_net_raw: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConfidenceAssessment {
    pub sample_size: usize,
    pub min_significant_sample: usize,
    pub statistically_meaningful: bool,
    pub note: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PaperTradingReport {
    pub generated_at_unix: u64,
    pub counts: Counts,
    pub quote_accuracy: QuoteAccuracy,
    pub decay: DecayStats,
    pub pnl: PnlStats,
    /// Rejection reasons with occurrence counts, descending.
    pub rejection_reasons: Vec<(String, usize)>,
    pub confidence: ConfidenceAssessment,
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let n = sorted.len();
    let idx = ((n as f64 * p).ceil() as usize)
        .saturating_sub(1)
        .min(n - 1);
    sorted[idx]
}

/// Maximum peak-to-trough decline of the cumulative sum of `series`.
fn max_drawdown(series: &[i64]) -> i64 {
    let mut cumulative: i64 = 0;
    let mut peak: i64 = 0;
    let mut drawdown: i64 = 0;
    for v in series {
        cumulative = cumulative.saturating_add(*v);
        if cumulative > peak {
            peak = cumulative;
        }
        let dd = peak - cumulative;
        if dd > drawdown {
            drawdown = dd;
        }
    }
    drawdown
}

fn replay_all(records: &[CandidateOpportunityRecord]) -> QuoteAccuracy {
    let mut replayed = 0usize;
    let mut reproduced = 0usize;
    for rec in records {
        replayed += 1;
        if let Ok(true) =
            crate::opportunity_recorder::OpportunityHistoryRecorder::replay_candidate_route(rec)
        {
            reproduced += 1;
        }
    }
    let rate = if replayed == 0 {
        0
    } else {
        (reproduced as u64 * 10_000) / replayed as u64
    };
    QuoteAccuracy {
        replayed,
        reproduced,
        reproduction_rate_bps: rate,
    }
}

/// Build the full evaluation report from recorded candidates.
pub fn build_report(records: &[CandidateOpportunityRecord], now_unix: u64) -> PaperTradingReport {
    let mut counts = Counts {
        detected: records.len(),
        ..Counts::default()
    };
    let mut rejection_reasons: HashMap<String, usize> = HashMap::new();
    let mut lifetimes: Vec<u64> = Vec::new();
    let mut net_series: Vec<i64> = Vec::new();
    let mut pnl = PnlStats::default();

    for rec in records {
        match rec.status.as_str() {
            STATUS_ACCEPTED => counts.accepted += 1,
            "REJECTED_SIM_FAIL" => {
                counts.rejected += 1;
                counts.simulation_failed += 1;
                *rejection_reasons
                    .entry("simulation_failed".to_string())
                    .or_insert(0) += 1;
            }
            "STALE_QUOTE" => {
                counts.rejected += 1;
                counts.stale_quote += 1;
                *rejection_reasons
                    .entry("stale_quote".to_string())
                    .or_insert(0) += 1;
            }
            other => {
                counts.rejected += 1;
                *rejection_reasons.entry(other.to_string()).or_insert(0) += 1;
            }
        }

        if let Some(lt) = rec.candidate_lifetime_ms {
            lifetimes.push(lt);
        }

        pnl.gross_total_raw = pnl
            .gross_total_raw
            .saturating_add(rec.gross_profit_quote_raw);
        pnl.execution_cost_total_raw = pnl
            .execution_cost_total_raw
            .saturating_add(rec.total_execution_cost_quote_raw);
        pnl.net_total_raw = pnl.net_total_raw.saturating_add(rec.net_profit_quote_raw);
        if rec.net_profit_quote_raw >= 0 {
            pnl.positive_net_count += 1;
        } else {
            pnl.negative_net_count += 1;
        }
        if rec.net_profit_quote_raw > pnl.best_net_raw {
            pnl.best_net_raw = rec.net_profit_quote_raw;
        }
        if rec.net_profit_quote_raw < pnl.worst_net_raw {
            pnl.worst_net_raw = rec.net_profit_quote_raw;
        }
        net_series.push(rec.net_profit_quote_raw);
    }

    pnl.max_drawdown_raw = max_drawdown(&net_series);
    if counts.accepted > 0 {
        pnl.mean_net_per_accepted_raw = pnl.net_total_raw / counts.accepted as i64;
    }

    lifetimes.sort_unstable();
    let decay = DecayStats {
        observed: lifetimes.len(),
        median_lifetime_ms: percentile(&lifetimes, 0.5),
        p90_lifetime_ms: percentile(&lifetimes, 0.9),
        still_alive_at_end: records
            .iter()
            .filter(|r| r.candidate_lifetime_ms.is_none())
            .count(),
    };

    let mut reasons: Vec<(String, usize)> = rejection_reasons.into_iter().collect();
    reasons.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    let statistically_meaningful = records.len() >= MIN_SIGNIFICANT_SAMPLE;
    let note = if records.is_empty() {
        "No recorded candidates: no economic evidence exists yet.".to_string()
    } else if statistically_meaningful {
        format!(
            "Sample size {} >= {}; descriptive statistics are reportable. \
             This is paper/simulation evidence, not realized on-chain profit.",
            records.len(),
            MIN_SIGNIFICANT_SAMPLE
        )
    } else {
        format!(
            "Sample size {} < {}; a short favorable sample is NOT proof of a \
             profitable strategy. Continue recording across varied market conditions.",
            records.len(),
            MIN_SIGNIFICANT_SAMPLE
        )
    };

    PaperTradingReport {
        generated_at_unix: now_unix,
        counts,
        quote_accuracy: replay_all(records),
        decay,
        pnl,
        rejection_reasons: reasons,
        confidence: ConfidenceAssessment {
            sample_size: records.len(),
            min_significant_sample: MIN_SIGNIFICANT_SAMPLE,
            statistically_meaningful,
            note,
        },
    }
}

/// Render the report as a human-readable Markdown document.
pub fn render_markdown(r: &PaperTradingReport) -> String {
    let mut s = String::new();
    s.push_str("# Paper-Trading Evaluation Report\n\n");
    s.push_str(&format!(
        "Generated at UNIX `{}` from `{}` recorded candidates.\n\n",
        r.generated_at_unix, r.counts.detected
    ));
    s.push_str("## Funnel\n\n| Stage | Count |\n|---|---|\n");
    s.push_str(&format!("| Detected | {} |\n", r.counts.detected));
    s.push_str(&format!(
        "| Accepted ({}-equivalent) | {} |\n",
        STATUS_ACCEPTED, r.counts.accepted
    ));
    s.push_str(&format!("| Rejected | {} |\n", r.counts.rejected));
    s.push_str(&format!(
        "| - simulation failed | {} |\n",
        r.counts.simulation_failed
    ));
    s.push_str(&format!("| - stale quote | {} |\n", r.counts.stale_quote));
    s.push_str("\n## Quote accuracy (deterministic replay)\n\n");
    s.push_str(&format!(
        "- Replayed: {} · Reproduced exactly: {} · Rate: {} bps\n",
        r.quote_accuracy.replayed,
        r.quote_accuracy.reproduced,
        r.quote_accuracy.reproduction_rate_bps
    ));
    s.push_str("\n## Opportunity decay\n\n");
    s.push_str(&format!(
        "- Observed disappearances: {} · still alive at end: {}\n- Median lifetime: {} ms · p90: {} ms\n",
        r.decay.observed, r.decay.still_alive_at_end, r.decay.median_lifetime_ms, r.decay.p90_lifetime_ms
    ));
    s.push_str("\n## Economics (quote-mint raw units)\n\n");
    s.push_str(&format!(
        "- Gross total: {} · Execution costs: {} · Net total: {}\n",
        r.pnl.gross_total_raw, r.pnl.execution_cost_total_raw, r.pnl.net_total_raw
    ));
    s.push_str(&format!(
        "- Positive-net records: {} · Negative: {}\n- Mean net per accepted: {} · Best: {} · Worst: {}\n- Max drawdown (cumulative net curve): {}\n",
        r.pnl.positive_net_count, r.pnl.negative_net_count,
        r.pnl.mean_net_per_accepted_raw, r.pnl.best_net_raw, r.pnl.worst_net_raw,
        r.pnl.max_drawdown_raw
    ));
    if !r.rejection_reasons.is_empty() {
        s.push_str("\n## Rejection reasons\n\n| Reason | Count |\n|---|---|\n");
        for (reason, n) in &r.rejection_reasons {
            s.push_str(&format!("| {} | {} |\n", reason, n));
        }
    }
    s.push_str("\n## Confidence\n\n");
    s.push_str(&format!("- {}\n", r.confidence.note));
    s
}

/// Persist `paper_trading_report.json` and `paper_trading_report.md` next to
/// the candidate log (same directory convention as the recorder).
pub fn write_report(log_dir: impl AsRef<Path>, report: &PaperTradingReport) -> std::io::Result<()> {
    let dir = log_dir.as_ref();
    std::fs::create_dir_all(dir)?;
    let json_path = dir.join("paper_trading_report.json");
    std::fs::write(
        &json_path,
        serde_json::to_string_pretty(report).unwrap_or_default(),
    )?;
    let md_path = dir.join("paper_trading_report.md");
    std::fs::write(&md_path, render_markdown(report))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opportunity_recorder::CandidateOpportunityRecord;
    use arb_types::{DexProtocol, MintId, PoolId, PoolKind, PoolState, Slot, TokenProgramKind};
    use solana_pubkey::Pubkey;

    fn test_pool_state() -> PoolState {
        PoolState {
            pool_id: PoolId(Pubkey::new_unique()),
            protocol: DexProtocol::MeteoraDlmm,
            slot: Slot(100),
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
            last_update_slot: 100,
            last_update_ms: 0,
        }
    }

    fn base_record(id: &str, status: &str, net: i64) -> CandidateOpportunityRecord {
        CandidateOpportunityRecord {
            candidate_id: id.to_string(),
            detection_timestamp_ms: 1_000,
            pool_a_id: Pubkey::new_unique(),
            pool_b_id: Pubkey::new_unique(),
            pool_a_slot: 100,
            pool_b_slot: 100,
            quote_slot: 101,
            pool_a_state: test_pool_state(),
            pool_b_state: test_pool_state(),
            direction: "DirectionA".to_string(),
            trade_size_in: 1_000_000,
            hop1_quote_out: 1_010_000,
            hop2_final_out: 1_020_000,
            fee_items: vec![],
            gross_profit_quote_raw: 20_000,
            total_execution_cost_quote_raw: 5_000,
            net_profit_quote_raw: net,
            net_profit_bps: 200,
            simulation_success: true,
            simulation_slot: Some(101),
            simulation_latency_ms: 12,
            quote_freshness_cleared: true,
            disappearance_time_ms: None,
            candidate_lifetime_ms: None,
            best_observed_net_profit: None,
            rejection_reason: None,
            status: status.to_string(),
        }
    }

    #[test]
    fn empty_records_produce_empty_report_with_no_significance() {
        let r = build_report(&[], 42);
        assert_eq!(r.counts.detected, 0);
        assert!(!r.confidence.statistically_meaningful);
        assert!(r.confidence.note.contains("No recorded candidates"));
        assert_eq!(r.quote_accuracy.reproduction_rate_bps, 0);
    }

    #[test]
    fn funnel_and_economics_are_counted_correctly() {
        let records = vec![
            base_record("a", "VERIFIED_PROFITABLE", 15_000),
            base_record("b", "VERIFIED_PROFITABLE", -3_000),
            base_record("c", "REJECTED_SIM_FAIL", 0),
            base_record("d", "STALE_QUOTE", 0),
            base_record("e", "SIMULATED_SUCCESS_NEGATIVE_EDGE", 0),
        ];
        let r = build_report(&records, 42);
        assert_eq!(r.counts.detected, 5);
        assert_eq!(r.counts.accepted, 2);
        assert_eq!(r.counts.rejected, 3);
        assert_eq!(r.counts.simulation_failed, 1);
        assert_eq!(r.counts.stale_quote, 1);
        assert_eq!(r.pnl.net_total_raw, 12_000);
        assert_eq!(r.pnl.positive_net_count, 4);
        assert_eq!(r.pnl.negative_net_count, 1);
        assert_eq!(r.pnl.mean_net_per_accepted_raw, 6_000);
        assert_eq!(r.pnl.best_net_raw, 15_000);
        assert_eq!(r.pnl.worst_net_raw, -3_000);
        assert_eq!(r.rejection_reasons[0].1, 1);
    }

    #[test]
    fn drawdown_is_peak_to_trough_of_cumulative_curve() {
        let records = vec![
            base_record("a", "VERIFIED_PROFITABLE", 10_000),
            base_record("b", "VERIFIED_PROFITABLE", -4_000),
            base_record("c", "VERIFIED_PROFITABLE", -8_000),
            base_record("d", "VERIFIED_PROFITABLE", 9_000),
        ];
        let r = build_report(&records, 42);
        assert_eq!(r.pnl.max_drawdown_raw, 12_000);
    }

    #[test]
    fn decay_percentiles_use_observed_lifetimes() {
        let mut records = Vec::new();
        for i in 0..10u64 {
            let mut rec = base_record(&format!("r{i}"), "VERIFIED_PROFITABLE", 100);
            rec.candidate_lifetime_ms = Some((i + 1) * 100);
            records.push(rec);
        }
        let mut alive = base_record("alive", "VERIFIED_PROFITABLE", 100);
        alive.candidate_lifetime_ms = None;
        records.push(alive);

        let r = build_report(&records, 42);
        assert_eq!(r.decay.observed, 10);
        assert_eq!(r.decay.still_alive_at_end, 1);
        assert_eq!(r.decay.median_lifetime_ms, 500);
        assert_eq!(r.decay.p90_lifetime_ms, 900);
    }

    #[test]
    fn markdown_renders_all_sections() {
        let records = vec![base_record("a", "VERIFIED_PROFITABLE", 15_000)];
        let r = build_report(&records, 42);
        let md = render_markdown(&r);
        assert!(md.contains("# Paper-Trading Evaluation Report"));
        assert!(md.contains("## Funnel"));
        assert!(md.contains("## Quote accuracy"));
        assert!(md.contains("## Opportunity decay"));
        assert!(md.contains("## Economics"));
        assert!(md.contains("## Confidence"));
        assert!(md.contains("NOT proof of a profitable strategy"));
    }

    #[test]
    fn write_report_emits_json_and_md() {
        let tmp = std::env::temp_dir().join(format!("ptr_{}", std::process::id()));
        let records = vec![base_record("a", "VERIFIED_PROFITABLE", 15_000)];
        let r = build_report(&records, 42);
        write_report(&tmp, &r).expect("write");
        assert!(tmp.join("paper_trading_report.json").exists());
        assert!(tmp.join("paper_trading_report.md").exists());
        let parsed: PaperTradingReport = serde_json::from_str(
            &std::fs::read_to_string(tmp.join("paper_trading_report.json")).unwrap(),
        )
        .expect("valid json");
        assert_eq!(parsed.counts.detected, 1);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
