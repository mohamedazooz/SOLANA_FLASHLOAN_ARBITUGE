use super::SCALE_OFFSET;
use super::price::{advance_active_id, get_price_from_id};
use super::types::DlmmBin;
use ruint::aliases::U256;

/// Compute the maximum absorbable input liquidity in an active bin.
///
/// In Meteora DLMM:
/// - When `a_to_b` is true (User deposits Token X, receives Token Y):
///   The pool pays out Token Y from the active bin.
///   The maximum Token X deposit that can be absorbed before active bin's
///   Token Y (`amount_y`) is depleted is: `(amount_y << 64) / price_q64`.
/// - When `a_to_b` is false (User deposits Token Y, receives Token X):
///   The pool pays out Token X from the active bin.
///   The maximum Token Y deposit that can be absorbed before active bin's
///   Token X (`amount_x`) is depleted is: `(amount_x * price_q64) >> 64`.
pub fn dlmm_active_bin_max_input(
    active_bin_id: i32,
    bin_step_bps: u16,
    active_bin_amount_x: u64,
    active_bin_amount_y: u64,
    a_to_b: bool,
) -> Option<u64> {
    let price_q64 = get_price_from_id(active_bin_id, bin_step_bps).ok()?;
    if price_q64 == 0 {
        return None;
    }

    if a_to_b {
        if active_bin_amount_y == 0 {
            return None;
        }
        let y_wide = U256::from(active_bin_amount_y) << SCALE_OFFSET;
        let max_x = y_wide / U256::from(price_q64);
        let cap_u64: u64 = max_x.try_into().ok()?;
        Some(cap_u64)
    } else {
        if active_bin_amount_x == 0 {
            return None;
        }
        let x_wide = U256::from(active_bin_amount_x);
        let max_y = (x_wide * U256::from(price_q64)) >> SCALE_OFFSET;
        let cap_u64: u64 = max_y.try_into().ok()?;
        Some(cap_u64)
    }
}

/// Multi-bin executable input capacity: the maximum input absorbable by the
/// provided bin window, walking bins in the exact traversal order used by
/// [`quote_dlmm_multi_bin`] (X->Y descends from the active bin, Y->X ascends).
///
/// Math (per bin i, price P_i in Q64.64):
/// * X->Y (swap_for_y): the bin's Y reserve absorbs `max_x_i = floor(amount_y_i << 64 / P_i)` input X.
/// * Y->X: the bin's X reserve absorbs `max_y_i = floor(amount_x_i * P_i >> 64)` input Y.
///
/// Capacity = sum of `max_*_i` over every bin present in the window. This is
/// exactly the executable depth of [`quote_dlmm_multi_bin`] on the same
/// window (quote depth == execution depth); traversal beyond the window
/// errors `BinNotFound` on-chain-quote side, so it must not be counted.
///
/// Returns `None` (fail closed) only when the window is empty, does not
/// contain the active bin (incomplete state), or a price derivation fails.
/// An empty OUTPUT side of the active bin is NOT a rejection: the walk
/// continues into neighbouring bins, unlike [`dlmm_active_bin_max_input`].
pub fn dlmm_multi_bin_max_input(
    active_bin_id: i32,
    bin_step_bps: u16,
    bins: &[DlmmBin],
    swap_for_y: bool,
) -> Option<u64> {
    if bins.is_empty() || !bins.iter().any(|b| b.id == active_bin_id) {
        return None;
    }
    let mut total: u128 = 0;
    let mut current = active_bin_id;
    while let Some(bin) = bins.iter().find(|b| b.id == current) {
        let price_q64 = if bin.price_q64 != 0 {
            bin.price_q64
        } else {
            get_price_from_id(current, bin_step_bps).ok()?
        };
        if price_q64 == 0 {
            return None;
        }
        let absorbable: u128 = if swap_for_y {
            if bin.amount_y == 0 {
                0
            } else {
                let y_wide = U256::from(bin.amount_y) << SCALE_OFFSET;
                let max_x = y_wide / U256::from(price_q64);
                max_x.try_into().ok()?
            }
        } else {
            if bin.amount_x == 0 {
                0
            } else {
                let x_wide = U256::from(bin.amount_x);
                let max_y = (x_wide * U256::from(price_q64)) >> SCALE_OFFSET;
                max_y.try_into().ok()?
            }
        };
        total = total.saturating_add(absorbable);
        if total >= u64::MAX as u128 {
            return Some(u64::MAX);
        }
        current = match advance_active_id(current, swap_for_y) {
            Ok(next) => next,
            Err(_) => break,
        };
    }
    u64::try_from(total).ok()
}

/// Scale trade input across a 3-hop Meteora DLMM cycle to extract maximum active bin liquidity.
///
/// In Meteora DLMM, price is constant-sum inside each bin (zero price impact within the active bin).
/// This calculates the bottleneck across all three hops, applies a 92% safety factor to avoid
/// crossing into suboptimal bins, and scales the gross edge linearly.
pub fn scale_dlmm_three_hop_input(
    probe_amount_in: u64,
    probe_gross_profit: u64,
    hop1_active_bin_max_in: u64,
    hop2_active_bin_max_in_equiv: u64,
    hop3_active_bin_max_in_equiv: u64,
    max_wallet_cap: u64,
) -> (u64, u64) {
    let bottleneck_in = hop1_active_bin_max_in
        .min(hop2_active_bin_max_in_equiv)
        .min(hop3_active_bin_max_in_equiv)
        .min(max_wallet_cap);

    let safe_optimal_in = (bottleneck_in * 92) / 100;

    if safe_optimal_in <= probe_amount_in || probe_amount_in == 0 {
        return (probe_amount_in, probe_gross_profit);
    }

    let scale_factor = safe_optimal_in as u128;
    let scaled_profit =
        ((probe_gross_profit as u128) * scale_factor / (probe_amount_in as u128)) as u64;

    (safe_optimal_in, scaled_profit)
}
