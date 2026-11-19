//! Exact bin-drain breakpoints for the DLMM piecewise-linear quote curve.
//!
//! Within one bin, the output of a DLMM swap is linear in the (fee-adjusted)
//! input; the marginal conversion price changes exactly when a bin's
//! deliverable liquidity is exhausted. The inputs at which each bin in the
//! walk direction is fully drained are therefore the exact breakpoints of the
//! piecewise-linear profit/size curve used by cross-pool arbitrage optimizers,
//! and the global optimum of a two-pool cycle rests on one of them.
//!
//! Mirrors the official engine semantics reproduced by this crate:
//! - Liquidity layers are filled MM reserves first, then processed limit
//!   orders, then open limit orders; a bin is drained exactly when every
//!   layer received its full `input_for_gross_out(layer_capacity)` (each
//!   ceiling-rounded per layer).
//! - `CollectFeeMode::InputOnly` charges the fee on the input side, so the
//!   fee-included drain input is the fee-excluded drain input plus
//!   `compute_fee_to_add(excluded, rate)`: the quote engine then re-derives
//!   `ceil(included * rate / 10^9) == compute_fee_to_add(excluded, rate)` and
//!   consumes exactly the fee-excluded amount, exhausting every layer.
//! - `CollectFeeMode::OnlyY` charges the fee on the output side, which never
//!   changes the required drain input.
//! - The volatility accumulator is refreshed only at bins that can deliver
//!   output (`max_out > 0`), in walk order, exactly as
//!   `quote_dlmm_multi_bin` / `quote_dlmm_exact_out` do. Callers that quote
//!   with a live `current_timestamp` must decay the snapshot first
//!   (`update_references_with_decay`) for the breakpoints to stay aligned.

use super::error::{DlmmError, dlmm_fee_on_input};
use super::fee::{compute_fee_to_add, compute_total_fee_rate, update_volatility_accumulator};
use super::price::{advance_active_id, get_price_from_id};
use super::swap::{bin_max_amount_out, input_for_gross_out, limit_order_amounts_by_direction};
use super::types::{DlmmBin, DlmmPoolState};

/// One piecewise-linear breakpoint: the cumulative fee-included input (in the
/// pool's inbound token base units) at which the walk has fully drained every
/// liquidity-bearing bin up to and including `bin_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DlmmDrainBreakpoint {
    /// Cumulative fee-included input that fully drains through `bin_id`.
    pub cumulative_input: u64,
    /// Bin drained at this breakpoint.
    pub bin_id: i32,
    /// Number of liquidity-bearing bins fully drained at this point
    /// (1-based depth: the first liquidity-bearing bin drained is 1).
    pub bins_drained: u32,
}

/// Exact fee-included input that fully drains one bin, layer by layer.
///
/// The engine's cascade consumes `min(remaining, layer_max_in)` per layer, so
/// the minimum input that exhausts every layer is the sum of the per-layer
/// ceiling inputs (each `input_for_gross_out(layer_capacity)`), not a single
/// ceil over the summed capacity.
fn bin_drain_excluded_input(
    bin: &DlmmBin,
    swap_for_y: bool,
    support_limit_order: bool,
    price_q64: u128,
) -> Result<u64, DlmmError> {
    let mm_amount = if swap_for_y {
        bin.amount_y
    } else {
        bin.amount_x
    };
    let mut excluded_needed = input_for_gross_out(mm_amount, price_q64, swap_for_y)?;

    if support_limit_order {
        let (open_order_amount, processed_order_remaining_amount) =
            limit_order_amounts_by_direction(bin, swap_for_y);
        let processed_input =
            input_for_gross_out(processed_order_remaining_amount, price_q64, swap_for_y)?;
        excluded_needed = excluded_needed
            .checked_add(processed_input)
            .ok_or(DlmmError::MathOverflow)?;
        let open_input = input_for_gross_out(open_order_amount, price_q64, swap_for_y)?;
        excluded_needed = excluded_needed
            .checked_add(open_input)
            .ok_or(DlmmError::MathOverflow)?;
    }

    Ok(excluded_needed)
}

/// Enumerate the exact cumulative-input breakpoints at which each traversed
/// bin is fully drained, walking from the active bin in the swap direction.
///
/// Bins that cannot deliver output in this direction (`max_out == 0`, empty
/// MM reserves *and* empty matching limit-order layers) are skipped without
/// producing a breakpoint, and without touching the volatility accumulator,
/// exactly as the quote engines do. The walk stops honestly at the end of the
/// caller-supplied window (missing bin), never guesses beyond it.
pub fn dlmm_drain_breakpoints(
    pool: &DlmmPoolState,
    bins: &[DlmmBin],
    swap_for_y: bool,
) -> Result<Vec<DlmmDrainBreakpoint>, DlmmError> {
    if pool.status != 0 {
        return Err(DlmmError::PoolDisabled);
    }

    let mut v_params = pool.v_parameters;
    let fee_on_input = dlmm_fee_on_input(pool.parameters.collect_fee_mode, swap_for_y)?;

    let mut current_bin_id = pool.active_id;
    let mut cumulative_input = 0u64;
    let mut bins_drained = 0u32;
    let mut breakpoints = Vec::new();

    while let Some(bin) = bins.iter().find(|b| b.id == current_bin_id) {
        let max_out = bin_max_amount_out(bin, swap_for_y, pool.support_limit_order);
        if max_out > 0 {
            update_volatility_accumulator(&mut v_params, &pool.parameters, current_bin_id)?;
            let fee_rate = compute_total_fee_rate(
                &pool.parameters,
                pool.bin_step,
                v_params.volatility_accumulator,
            )?;

            let price_q64 = if bin.price_q64 != 0 {
                bin.price_q64
            } else {
                get_price_from_id(bin.id, pool.bin_step)?
            };

            let excluded_needed =
                bin_drain_excluded_input(bin, swap_for_y, pool.support_limit_order, price_q64)?;
            let included_needed = if fee_on_input {
                let fee = compute_fee_to_add(excluded_needed, fee_rate)?;
                excluded_needed
                    .checked_add(fee)
                    .ok_or(DlmmError::MathOverflow)?
            } else {
                excluded_needed
            };

            cumulative_input = cumulative_input
                .checked_add(included_needed)
                .ok_or(DlmmError::MathOverflow)?;
            bins_drained = bins_drained.saturating_add(1);
            breakpoints.push(DlmmDrainBreakpoint {
                cumulative_input,
                bin_id: current_bin_id,
                bins_drained,
            });
        }

        current_bin_id = advance_active_id(current_bin_id, swap_for_y)?;
    }

    Ok(breakpoints)
}

/// Shape raw breakpoint seeds into an optimized candidate list.
///
/// - Rejects zero sizes and anything outside `[lo, hi]`.
/// - Sorts and dedupes.
/// - When more than `max_seeds` remain, keeps an evenly spaced sample that
///   always includes the smallest and the largest surviving breakpoint (a
///   limit-order-free pool with 81 bins must not explode the optimizer's
///   evaluation budget).
pub fn dlmm_breakpoint_seed_sizes(
    raw_seeds: &[u64],
    lo: u64,
    hi: u64,
    max_seeds: usize,
) -> Vec<u64> {
    let mut seeds: Vec<u64> = raw_seeds
        .iter()
        .copied()
        .filter(|&s| s > 0 && s >= lo && s <= hi)
        .collect();
    seeds.sort_unstable();
    seeds.dedup();

    if max_seeds == 0 || seeds.len() <= max_seeds {
        return seeds;
    }

    let last = seeds.len() - 1;
    let mut sampled = Vec::with_capacity(max_seeds);
    for k in 0..max_seeds {
        let idx = (k * last) / (max_seeds - 1);
        if sampled.last() != Some(&seeds[idx]) {
            sampled.push(seeds[idx]);
        }
    }
    if sampled.last() != Some(&seeds[last]) {
        sampled.push(seeds[last]);
    }
    sampled
}
