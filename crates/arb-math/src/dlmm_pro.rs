//! Meteora DLMM Pro Mathematical Model.
//!
//! Implements continuous tick concentrated liquidity ($P = 1.0001^{\text{tick}}$)
//! combined with resting limit-order taker matching.
//!
//! Architectural Invariants:
//! - Exact Q64.64 integer math using wide integers (`U256`) and binary exponentiation.
//! - Directional rounding: ceiling on user inputs, floor on user outputs.
//! - Resting limit orders matched at tick boundaries with deterministic priority.
//! - Zero floating point arithmetic in authoritative quote decisions.

use crate::clmm::{ClmmMathError, MAX_TICK, MIN_TICK, compute_swap_step, tick_to_sqrt_price_x64};
use ruint::aliases::U256;

/// A single initialized tick or resting limit order in DLMM Pro.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DlmmProTick {
    pub tick_index: i32,
    pub liquidity_gross: u128,
    pub liquidity_net: i128,
    /// Resting limit-order volume available to be taken at this tick.
    pub resting_order_amount: u64,
}

/// A contiguous array of ticks in DLMM Pro.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DlmmProTickArray {
    pub start_tick_index: i32,
    pub ticks: Vec<DlmmProTick>,
}

/// DLMM Pro pool state for quote execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DlmmProPoolState {
    pub current_tick: i32,
    pub current_sqrt_price_x64: u128,
    pub active_liquidity: u128,
    pub tick_spacing: u16,
    pub fee_bps: u16,
    pub tick_arrays: Vec<DlmmProTickArray>,
}

impl DlmmProPoolState {
    pub fn new(
        current_tick: i32,
        current_sqrt_price_x64: u128,
        active_liquidity: u128,
        tick_spacing: u16,
        fee_bps: u16,
    ) -> Self {
        Self {
            current_tick,
            current_sqrt_price_x64,
            active_liquidity,
            tick_spacing,
            fee_bps,
            tick_arrays: Vec::new(),
        }
    }

    /// Add a tick array to the state.
    pub fn add_tick_array(&mut self, array: DlmmProTickArray) {
        self.tick_arrays.push(array);
        self.tick_arrays.sort_by_key(|a| a.start_tick_index);
    }
}

/// Quote an exact-in swap through DLMM Pro with resting order matching.
///
/// Returns `(amount_out, fee_paid, resting_order_filled)`.
pub fn quote_dlmm_pro_exact_in(
    state: &DlmmProPoolState,
    amount_in: u64,
    a_to_b: bool,
) -> Result<(u64, u64, u64), ClmmMathError> {
    if amount_in == 0 {
        return Ok((0, 0, 0));
    }
    let mut current_sqrt_p = state.current_sqrt_price_x64;
    let mut current_tick = state.current_tick;
    let mut active_liquidity = state.active_liquidity;
    let mut amount_remaining = amount_in;
    let mut total_amount_out: u64 = 0;
    let mut total_fee_paid: u64 = 0;
    let mut total_resting_filled: u64 = 0;

    let fee_rate = state.fee_bps;

    let mut iterations = 0;
    while amount_remaining > 0 {
        iterations += 1;
        if iterations > 256 {
            return Err(ClmmMathError::IterationLimitExceeded);
        }

        // Find next initialized tick boundary in direction of swap
        let next_tick = if a_to_b {
            // Price moving down, tick moving down
            let spacing = i32::from(state.tick_spacing);
            (current_tick / spacing) * spacing - spacing
        } else {
            // Price moving up, tick moving up
            let spacing = i32::from(state.tick_spacing);
            (current_tick / spacing) * spacing + spacing
        };

        let clamped_tick = next_tick.clamp(MIN_TICK, MAX_TICK);
        let target_sqrt_p = tick_to_sqrt_price_x64(clamped_tick)?;

        // 1. Taker matching: check for resting limit orders at current tick
        for arr in &state.tick_arrays {
            for tick in &arr.ticks {
                if tick.tick_index == current_tick && tick.resting_order_amount > 0 {
                    let resting_avail = tick.resting_order_amount;
                    // Match resting order directly (1:1 taker fill)
                    let fill_amount = amount_remaining.min(resting_avail);
                    if fill_amount > 0 {
                        // Calculate output from resting order at current price
                        let out = if a_to_b {
                            // in = A, out = B: out = in * P
                            let num = U256::from(fill_amount) * U256::from(current_sqrt_p);
                            let scaled = (num * U256::from(current_sqrt_p)) >> 128;
                            u64::try_from(scaled).unwrap_or(0)
                        } else {
                            // in = B, out = A: out = in / P
                            let num = U256::from(fill_amount) << 128;
                            let denom = U256::from(current_sqrt_p) * U256::from(current_sqrt_p);
                            if denom.is_zero() {
                                0
                            } else {
                                u64::try_from(num / denom).unwrap_or(0)
                            }
                        };
                        amount_remaining -= fill_amount;
                        total_amount_out = total_amount_out.saturating_add(out);
                        total_resting_filled = total_resting_filled.saturating_add(fill_amount);
                        if amount_remaining == 0 {
                            break;
                        }
                    }
                }
            }
            if amount_remaining == 0 {
                break;
            }
        }

        if amount_remaining == 0 {
            break;
        }

        // 2. Continuous CLMM swap step between current_sqrt_p and target_sqrt_p
        if active_liquidity == 0 {
            // Cannot traverse without liquidity
            if current_sqrt_p == target_sqrt_p {
                break;
            }
            current_sqrt_p = target_sqrt_p;
            current_tick = clamped_tick;
            continue;
        }

        let step_res = compute_swap_step(
            current_sqrt_p,
            target_sqrt_p,
            active_liquidity,
            amount_remaining,
            fee_rate,
            a_to_b,
        )?;

        amount_remaining = amount_remaining
            .checked_sub(step_res.amount_in)
            .ok_or(ClmmMathError::IntegerOverflow)?;
        total_amount_out = total_amount_out
            .checked_add(step_res.amount_out)
            .ok_or(ClmmMathError::IntegerOverflow)?;
        total_fee_paid = total_fee_paid
            .checked_add(step_res.fee_amount)
            .ok_or(ClmmMathError::IntegerOverflow)?;
        current_sqrt_p = step_res.next_sqrt_price_x64;

        if current_sqrt_p == target_sqrt_p {
            current_tick = clamped_tick;
            // Cross tick and update liquidity from tick arrays
            for arr in &state.tick_arrays {
                for tick in &arr.ticks {
                    if tick.tick_index == current_tick {
                        if a_to_b {
                            if tick.liquidity_net < 0 {
                                active_liquidity =
                                    active_liquidity.saturating_sub((-tick.liquidity_net) as u128);
                            } else {
                                active_liquidity =
                                    active_liquidity.saturating_add(tick.liquidity_net as u128);
                            }
                        } else {
                            if tick.liquidity_net < 0 {
                                active_liquidity =
                                    active_liquidity.saturating_add((-tick.liquidity_net) as u128);
                            } else {
                                active_liquidity =
                                    active_liquidity.saturating_sub(tick.liquidity_net as u128);
                            }
                        }
                    }
                }
            }
        } else {
            break;
        }
    }

    Ok((total_amount_out, total_fee_paid, total_resting_filled))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dlmm_pro_single_step_quote() {
        let sqrt_p_0 = tick_to_sqrt_price_x64(0).unwrap();
        let mut pool = DlmmProPoolState::new(0, sqrt_p_0, 10_000_000_000, 60, 25);
        let tick_arr = DlmmProTickArray {
            start_tick_index: -120,
            ticks: vec![DlmmProTick {
                tick_index: -60,
                liquidity_gross: 5_000_000,
                liquidity_net: 5_000_000,
                resting_order_amount: 1_000_000,
            }],
        };
        pool.add_tick_array(tick_arr);

        let quote = quote_dlmm_pro_exact_in(&pool, 500_000, true).unwrap();
        assert!(quote.0 > 0, "expected positive amount_out");
        assert!(quote.1 > 0, "expected positive fee");
    }
}
