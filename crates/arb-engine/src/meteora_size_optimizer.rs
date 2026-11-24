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
use arb_types::MintId;
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
    pub fees_hop1_mint: MintId,
    pub fees_hop2_raw: u64,
    pub fees_hop2_mint: MintId,
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
    pub slippage_bps: u16,
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
            min_input: 100_000,    // 0.10 USDC (6 dec)
            max_input: 10_000_000, // 10.0 USDC (6 dec)
            slippage_bps: arb_constants::DEFAULT_HOP_SLIPPAGE_BPS as u16,
            coarse_samples: 8,
            max_candidate_basins: 4,
            refine_steps_per_basin: 6,
            estimated_network_cost_lamports: arb_constants::SOLANA_BASE_FEE_PER_SIGNATURE_LAMPORTS,
            sol_usd_price: 0, // Must be supplied for non-SOL quote currencies.
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
            arb_constants::SOLANA_BASE_FEE_PER_SIGNATURE_LAMPORTS
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
            if sol_usd_price == 0 || quote_decimals > 18 {
                return u64::MAX;
            }
            let num = u128::from(lamports)
                .checked_mul(u128::from(sol_usd_price))
                .and_then(|value| value.checked_mul(10u128.pow(u32::from(quote_decimals))));
            num.and_then(|value| u64::try_from(value / 1_000_000_000).ok())
                .unwrap_or(u64::MAX)
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
            cfg.sol_usd_price,
            cfg.quote_decimals,
        );

        let net_profit = quote
            .minimum_gross_profit
            .saturating_sub(i64::try_from(net_cost_quote).unwrap_or(i64::MAX));
        let net_bps = if size > 0 {
            i64::try_from(i128::from(net_profit) * 10_000 / i128::from(size))
                .unwrap_or(if net_profit >= 0 { i64::MAX } else { i64::MIN })
        } else {
            0
        };

        TestedSizeEvaluation {
            direction,
            trade_size: size,
            hop1_out: quote.hop1_amount_out,
            hop2_out: quote.hop2_final_amount_out,
            fees_hop1_raw: quote.hop1_fee,
            fees_hop1_mint: quote.hop1_fee_mint,
            fees_hop2_raw: quote.hop2_fee,
            fees_hop2_mint: quote.hop2_fee_mint,
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
        if lo == 0
            || hi < lo
            || cfg.coarse_samples < 2
            || cfg.max_candidate_basins == 0
            || cfg.refine_steps_per_basin == 0
            || cfg.quote_decimals > 18
            || (!cfg.is_quote_sol && cfg.sol_usd_price == 0)
        {
            return None;
        }

        let quote_fn = |amt: u64| -> Option<TwoHopDlmmQuote> {
            match direction {
                DlmmArbitrageDirection::DirectionA => {
                    MeteoraDlmmCrossPoolQuoteEngine::quote_direction_a_with_slippage(
                        pair,
                        amt,
                        cfg.slippage_bps,
                    )
                }
                DlmmArbitrageDirection::DirectionB => {
                    MeteoraDlmmCrossPoolQuoteEngine::quote_direction_b_with_slippage(
                        pair,
                        amt,
                        cfg.slippage_bps,
                    )
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

        // 2. Multi-basin detection. Rank every discovered basin by its coarse
        // net profit before applying the work cap; encounter order is size order
        // and must not decide which profitable regions get refined.
        let candidate_basins = select_candidate_basins(
            &evaluated_sizes,
            &tested_evaluations,
            cfg.max_candidate_basins,
        );

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
            spread_bps: best_quote.minimum_spread_bps,
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

fn select_candidate_basins(
    sizes: &[u64],
    evaluations: &[TestedSizeEvaluation],
    max_basins: usize,
) -> Vec<(u64, u64)> {
    if sizes.is_empty() || sizes.len() != evaluations.len() || max_basins == 0 {
        return Vec::new();
    }

    let mut peaks: Vec<usize> = (0..sizes.len())
        .filter(|&i| {
            let value = evaluations[i].net_profit_quote_raw;
            let left = i
                .checked_sub(1)
                .map(|j| evaluations[j].net_profit_quote_raw)
                .unwrap_or(i64::MIN);
            let right = evaluations
                .get(i + 1)
                .map(|next| next.net_profit_quote_raw)
                .unwrap_or(i64::MIN);
            value >= left && value >= right
        })
        .collect();

    if peaks.is_empty() {
        peaks.extend(0..sizes.len());
    }

    peaks.sort_by_key(|&i| {
        (
            std::cmp::Reverse(evaluations[i].net_profit_quote_raw),
            sizes[i],
        )
    });
    peaks.truncate(max_basins);
    peaks
        .into_iter()
        .map(|i| {
            (
                sizes[i.saturating_sub(1)],
                sizes[(i + 1).min(sizes.len() - 1)],
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{TestedSizeEvaluation, select_candidate_basins};
    use crate::meteora_size_optimizer::DlmmArbitrageDirection;
    use arb_types::MintId;
    use solana_pubkey::Pubkey;

    fn evaluation(size: u64, profit: i64) -> TestedSizeEvaluation {
        TestedSizeEvaluation {
            direction: DlmmArbitrageDirection::DirectionA,
            trade_size: size,
            hop1_out: 0,
            hop2_out: 0,
            fees_hop1_raw: 0,
            fees_hop1_mint: MintId(Pubkey::default()),
            fees_hop2_raw: 0,
            fees_hop2_mint: MintId(Pubkey::default()),
            network_costs_quote_raw: 0,
            net_profit_quote_raw: profit,
            net_profit_bps: 0,
            compute_units: 0,
            required_bin_arrays_hop1: 0,
            required_bin_arrays_hop2: 0,
            total_bin_arrays: 0,
        }
    }

    #[test]
    fn refines_highest_profit_basins_even_when_they_appear_late() {
        let sizes = [10, 20, 30, 40, 50, 60, 70, 80, 90];
        let profits = [1, 10, 1, 20, 1, 30, 1, 40, 1];
        let evaluations: Vec<_> = sizes
            .iter()
            .zip(profits)
            .map(|(&size, profit)| evaluation(size, profit))
            .collect();

        assert_eq!(
            select_candidate_basins(&sizes, &evaluations, 3),
            vec![(70, 90), (50, 70), (30, 50)]
        );
    }

    #[test]
    fn invalid_or_empty_coarse_data_has_no_basins() {
        assert!(select_candidate_basins(&[], &[], 4).is_empty());
        assert!(select_candidate_basins(&[1], &[], 4).is_empty());
        assert!(select_candidate_basins(&[1], &[evaluation(1, 2)], 0).is_empty());
    }
}
