//! Meteora DLMM Trade Size Optimizer (`MeteoraDlmmTradeSizeOptimizer`).
//!
//! Evaluates a configurable range of trade sizes across both arbitrage directions
//! using discrete coarse-to-fine search. Does NOT assume unimodality; refines around
//! multiple promising candidate basins preserving checked integer arithmetic.
//!
//! Records for every tested size:
//! - Hop 1 output
//! - Hop 2 output
//! - DEX fees (Hop 1, Hop 2, total embedded)
//! - Network costs (converted to quote currency)
//! - Net profit (in quote currency raw units)
//! - Net profit bps
//! - Compute units
//! - Required Bin Arrays (Hop 1, Hop 2, total)

use crate::meteora_pair_discovery::MeteoraDlmmEligiblePair;
use crate::meteora_quote_engine::{MeteoraDlmmCrossPoolQuoteEngine, TwoHopDlmmQuote};
use serde::{Deserialize, Serialize};

/// Arbitrage direction across the two pools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DlmmArbitrageDirection {
    /// Direction A: Pool A (Quote -> Base) -> Pool B (Base -> Quote)
    DirectionA,
    /// Direction B: Pool B (Quote -> Base) -> Pool A (Base -> Quote)
    DirectionB,
}

/// Evaluation record for an individual tested trade size.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestedSizeEvaluation {
    pub direction: DlmmArbitrageDirection,
    pub trade_size: u64,
    pub hop1_out: u64,
    pub hop2_out: u64,
    pub fees_hop1_raw: u64,
    pub fees_hop2_raw: u64,
    pub total_fees_raw: u64,
    pub network_costs_quote_raw: u64,
    pub net_profit_quote_raw: i64,
    pub net_profit_bps: i64,
    pub compute_units: u32,
    pub required_bin_arrays_hop1: u32,
    pub required_bin_arrays_hop2: u32,
    pub total_bin_arrays: u32,
}

/// Optimal trade size result across all evaluated candidate points.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeteoraOptimalSizeResult {
    pub direction: DlmmArbitrageDirection,
    pub optimal_amount_in: u64,
    pub expected_hop1_out: u64,
    pub expected_final_out: u64,
    pub expected_gross_profit: i64,
    pub expected_net_profit: i64,
    pub spread_bps: u64,
    pub net_profit_bps: i64,
    pub hop1_bins_traversed: u32,
    pub hop2_bins_traversed: u32,
    pub estimated_cu: u32,
    pub required_bin_arrays: u32,
    pub is_profitable: bool,
    pub tested_evaluations: Vec<TestedSizeEvaluation>,
    pub quote: TwoHopDlmmQuote,
    // ---- Search diagnostics (Phase 15.2) -----------------------------------
    /// Lower bound of the searched size range.
    pub search_lower_bound: u64,
    /// Upper bound of the searched size range.
    pub search_upper_bound: u64,
    /// Number of coarse samples requested by configuration.
    pub coarse_sample_count: usize,
    /// Exact coarse grid points evaluated.
    pub coarse_points: Vec<u64>,
    /// Candidate brackets `(lo, hi)` discovered from coarse local maxima / edges.
    pub candidate_basins: Vec<(u64, u64)>,
    /// Fine-search intervals actually swept.
    pub fine_search_intervals: Vec<(u64, u64)>,
    /// Total number of distinct candidates evaluated.
    pub evaluation_count: usize,
    /// Human-readable stopping condition.
    pub stopping_condition: String,
    /// True when the search was bounded by the configured limits rather than
    /// proving exhaustion of the range.
    pub bounded_early: bool,
    /// This implementation reports `best_observed_candidate`, never a proven
    /// global optimum. Kept explicit for machine consumers.
    pub result_kind: String,
}

/// Configuration for the trade size optimizer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DlmmOptimizerConfig {
    pub min_input: u64,
    pub max_input: u64,
    pub coarse_samples: usize,
    pub max_candidate_basins: usize,
    pub refine_steps_per_basin: usize,
    pub estimated_network_cost_lamports: u64,
    pub sol_usd_price: u64,
    pub quote_decimals: u8,
    pub is_quote_sol: bool,
}

impl Default for DlmmOptimizerConfig {
    fn default() -> Self {
        Self {
            min_input: 10_000,     // 0.01 USDC (6 dec)
            max_input: 10_000_000, // 10.0 USDC (6 dec)
            coarse_samples: 24,
            max_candidate_basins: 4,
            refine_steps_per_basin: 10,
            estimated_network_cost_lamports: 9_000, // 5k base + 4k priority
            sol_usd_price: 180,                     // Conservative default
            quote_decimals: 6,
            is_quote_sol: false,
        }
    }
}

pub struct MeteoraDlmmTradeSizeOptimizer;

impl MeteoraDlmmTradeSizeOptimizer {
    /// Effective network cost lamports.
    pub fn effective_network_cost(cfg: &DlmmOptimizerConfig) -> u64 {
        if cfg.estimated_network_cost_lamports > 0 {
            cfg.estimated_network_cost_lamports
        } else {
            9_000
        }
    }

    /// Effective refine steps.
    pub fn effective_refine_steps(cfg: &DlmmOptimizerConfig) -> usize {
        if cfg.refine_steps_per_basin > 0 {
            cfg.refine_steps_per_basin
        } else {
            8
        }
    }

    /// Convert SOL-denominated network costs into quote currency base units.
    pub fn network_cost_in_quote_raw(
        lamports: u64,
        is_quote_sol: bool,
        sol_usd_price: u64,
        quote_decimals: u8,
    ) -> u64 {
        if is_quote_sol {
            lamports
        } else {
            let num = (lamports as u128)
                .saturating_mul(sol_usd_price as u128)
                .saturating_mul(10u128.pow(quote_decimals as u32));
            (num / 1_000_000_000) as u64
        }
    }

    /// Estimate compute units based on bin arrays touched.
    pub fn estimate_compute_units(total_bin_arrays: u32) -> u32 {
        const BASELINE_CU: u32 = 65_000;
        const PER_BIN_ARRAY_CU: u32 = 15_000;
        const MAX_CU: u32 = 400_000;
        BASELINE_CU
            .saturating_add(total_bin_arrays.saturating_mul(PER_BIN_ARRAY_CU))
            .min(MAX_CU)
    }

    /// Build a single `TestedSizeEvaluation` from a quote.
    pub fn evaluate_size(
        direction: DlmmArbitrageDirection,
        size: u64,
        quote: &TwoHopDlmmQuote,
        cfg: &DlmmOptimizerConfig,
    ) -> TestedSizeEvaluation {
        let arr1 = (quote.hop1_bins_traversed as u64 / 70) as u32 + 1;
        let arr2 = (quote.hop2_bins_traversed as u64 / 70) as u32 + 1;
        let total_arrays = arr1 + arr2;
        let cu = Self::estimate_compute_units(total_arrays);

        let lamports = Self::effective_network_cost(cfg);
        let net_cost_quote = Self::network_cost_in_quote_raw(
            lamports,
            cfg.is_quote_sol,
            cfg.sol_usd_price.max(1),
            cfg.quote_decimals,
        );

        let net_profit = quote.gross_profit - (net_cost_quote as i64);
        let net_bps = if size > 0 {
            (net_profit * 10_000) / (size as i64)
        } else {
            0
        };

        TestedSizeEvaluation {
            direction,
            trade_size: size,
            hop1_out: quote.hop1_amount_out,
            hop2_out: quote.hop2_final_amount_out,
            fees_hop1_raw: quote.hop1_fee,
            fees_hop2_raw: quote.hop2_fee,
            total_fees_raw: quote.hop1_fee.saturating_add(quote.hop2_fee),
            network_costs_quote_raw: net_cost_quote,
            net_profit_quote_raw: net_profit,
            net_profit_bps: net_bps,
            compute_units: cu,
            required_bin_arrays_hop1: arr1,
            required_bin_arrays_hop2: arr2,
            total_bin_arrays: total_arrays,
        }
    }

    /// Optimize trade size for a specific direction on an eligible pair without unimodal assumptions.
    pub fn optimize_direction(
        pair: &MeteoraDlmmEligiblePair,
        direction: DlmmArbitrageDirection,
        cfg: &DlmmOptimizerConfig,
    ) -> Option<MeteoraOptimalSizeResult> {
        let lo = cfg.min_input;
        let hi = cfg.max_input;
        if lo == 0 || hi < lo {
            return None;
        }

        let quote_fn = |amt: u64| -> Option<TwoHopDlmmQuote> {
            match direction {
                DlmmArbitrageDirection::DirectionA => {
                    MeteoraDlmmCrossPoolQuoteEngine::quote_direction_a(pair, amt)
                }
                DlmmArbitrageDirection::DirectionB => {
                    MeteoraDlmmCrossPoolQuoteEngine::quote_direction_b(pair, amt)
                }
            }
        };

        let mut evaluated_sizes = Vec::new();
        let mut tested_evaluations = Vec::new();
        let mut quote_map: std::collections::HashMap<u64, TwoHopDlmmQuote> =
            std::collections::HashMap::new();

        // 1. Multi-modal Coarse Sampling: Logarithmic progression + uniform grid
        let mut coarse_points = Vec::with_capacity(128);
        coarse_points.push(lo);
        let mut cur = lo;
        while cur < hi && coarse_points.len() < 128 {
            // Growth multiplier of 1.5 preserves coverage across bin arrays
            let next_u128 = (cur as u128 * 15 / 10).max(cur as u128 + 1);
            let next = match u64::try_from(next_u128) {
                Ok(v) if v > cur => v,
                _ => break,
            };
            if next > hi {
                break;
            }
            coarse_points.push(next);
            cur = next;
        }
        if *coarse_points.last().unwrap_or(&0) != hi {
            coarse_points.push(hi);
        }

        // Linear subdivision fill
        let span = hi - lo;
        let step = span / (cfg.coarse_samples.max(2) as u64);
        if step > 0 {
            for i in 1..cfg.coarse_samples {
                let pt = lo + (i as u64) * step;
                if pt <= hi {
                    coarse_points.push(pt);
                }
            }
        }
        coarse_points.sort_unstable();
        coarse_points.dedup();

        for &pt in &coarse_points {
            if let Some(q) = quote_fn(pt) {
                let eval = Self::evaluate_size(direction, pt, &q, cfg);
                tested_evaluations.push(eval);
                evaluated_sizes.push(pt);
                quote_map.insert(pt, q);
            }
        }

        if evaluated_sizes.is_empty() {
            return None;
        }

        // 2. Multi-Basin Candidate Detection (Rejecting global unimodality)
        // Find local maxima and high-yield candidate points across the coarse grid
        let mut candidate_basins: Vec<(u64, u64)> = Vec::new(); // (bracket_lo, bracket_hi)
        for i in 0..evaluated_sizes.len() {
            let left_val = if i > 0 {
                tested_evaluations[i - 1].net_profit_quote_raw
            } else {
                i64::MIN
            };
            let right_val = if i + 1 < evaluated_sizes.len() {
                tested_evaluations[i + 1].net_profit_quote_raw
            } else {
                i64::MIN
            };
            let cur_val = tested_evaluations[i].net_profit_quote_raw;

            // Local maximum or edge boundary peak
            if cur_val >= left_val && cur_val >= right_val {
                let b_lo = if i > 0 {
                    evaluated_sizes[i - 1]
                } else {
                    evaluated_sizes[i]
                };
                let b_hi = if i + 1 < evaluated_sizes.len() {
                    evaluated_sizes[i + 1]
                } else {
                    evaluated_sizes[i]
                };
                candidate_basins.push((b_lo, b_hi));
            }
        }

        // If no strict local peaks found, take top candidate points by net profit
        if candidate_basins.is_empty() {
            let mut indexed: Vec<(usize, i64)> = tested_evaluations
                .iter()
                .enumerate()
                .map(|(i, e)| (i, e.net_profit_quote_raw))
                .collect();
            indexed.sort_by_key(|&(_, p)| std::cmp::Reverse(p));
            for &(idx, _) in indexed.iter().take(cfg.max_candidate_basins) {
                let b_lo = if idx > 0 {
                    evaluated_sizes[idx - 1]
                } else {
                    evaluated_sizes[idx]
                };
                let b_hi = if idx + 1 < evaluated_sizes.len() {
                    evaluated_sizes[idx + 1]
                } else {
                    evaluated_sizes[idx]
                };
                candidate_basins.push((b_lo, b_hi));
            }
        }
        candidate_basins.truncate(cfg.max_candidate_basins);

        // 3. Discrete Fine Search around EACH candidate basin (Integer arithmetic only)
        let mut fine_search_intervals = Vec::new();
        for &(b_lo, b_hi) in &candidate_basins {
            if b_hi <= b_lo + 1 {
                continue;
            }
            fine_search_intervals.push((b_lo, b_hi));
            let b_span = b_hi - b_lo;
            let steps = Self::effective_refine_steps(cfg).max(2);
            let fine_step = (b_span / (steps as u64)).max(1);

            for s in 1..steps {
                let pt = b_lo + (s as u64) * fine_step;
                if pt >= b_hi || quote_map.contains_key(&pt) {
                    continue;
                }
                if let Some(q) = quote_fn(pt) {
                    let eval = Self::evaluate_size(direction, pt, &q, cfg);
                    tested_evaluations.push(eval);
                    quote_map.insert(pt, q);
                }
            }
        }

        // 4. Select the global discrete best across all tested points
        tested_evaluations.sort_by_key(|e| e.trade_size);
        let best_eval = tested_evaluations
            .iter()
            .max_by_key(|e| e.net_profit_quote_raw)?
            .clone();

        let best_quote = quote_map.get(&best_eval.trade_size).cloned()?;

        let bounded_early = coarse_points.len() >= 128;
        Some(MeteoraOptimalSizeResult {
            direction,
            optimal_amount_in: best_eval.trade_size,
            expected_hop1_out: best_eval.hop1_out,
            expected_final_out: best_eval.hop2_out,
            expected_gross_profit: best_quote.gross_profit,
            expected_net_profit: best_eval.net_profit_quote_raw,
            spread_bps: best_quote.spread_bps,
            net_profit_bps: best_eval.net_profit_bps,
            hop1_bins_traversed: best_quote.hop1_bins_traversed,
            hop2_bins_traversed: best_quote.hop2_bins_traversed,
            estimated_cu: best_eval.compute_units,
            required_bin_arrays: best_eval.total_bin_arrays,
            is_profitable: best_eval.net_profit_quote_raw > 0,
            evaluation_count: tested_evaluations.len(),
            tested_evaluations,
            quote: best_quote,
            search_lower_bound: lo,
            search_upper_bound: hi,
            coarse_sample_count: cfg.coarse_samples,
            coarse_points,
            candidate_basins,
            fine_search_intervals,
            stopping_condition: "coarse logarithmic+linear grid, multi-basin local maxima, \
                bounded fine sweep; best observed candidate selected"
                .to_string(),
            bounded_early,
            result_kind: "best_observed_candidate".to_string(),
        })
    }

    /// Optimize both directions and return the best overall candidate opportunity.
    pub fn find_best_opportunity(
        pair: &MeteoraDlmmEligiblePair,
        cfg: &DlmmOptimizerConfig,
    ) -> Option<MeteoraOptimalSizeResult> {
        let res_a = Self::optimize_direction(pair, DlmmArbitrageDirection::DirectionA, cfg);
        let res_b = Self::optimize_direction(pair, DlmmArbitrageDirection::DirectionB, cfg);

        match (res_a, res_b) {
            (Some(a), Some(b)) => {
                if a.expected_net_profit >= b.expected_net_profit {
                    Some(a)
                } else {
                    Some(b)
                }
            }
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
    }
}
