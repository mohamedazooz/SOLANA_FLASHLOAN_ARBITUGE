//!
//! Covers:
//! - Exact fee-scaled CPMM quotes (x·y=k with fee denominator D=10,000)
//! - Optimal closed-form trade sizing for 2-pool CPMM with N > D0 condition in U384
//! - Optimal closed-form Möbius sizing for 3-hop triangular cycles in U512
//! - Pure integer `Option<u64>` returns with zero lossy float casts
//! - Marginal profit degradation ceiling check (dPi / d(Delta x) <= 0)
//! - Concentrated Liquidity (CLMM) math with Q64.64 bitwise binary exponentiation and tick arrays
//! - Meteora DLMM discrete bin pricing P_i = (1 + bin_step)^i and variable fee calculation
//! - Token-2022 transfer fee deduction
//! - Invariant checking (constant product & slippage bounds)

pub mod chronos;
pub mod clmm;
pub mod cpmm;
pub mod damm_v2;
pub mod dlmm;
pub mod economic;
pub mod economic_integration;
pub mod latency;
pub mod launchlab;
pub mod pump_fun_raydium;

pub use chronos::{
    BPS_DIVISOR_F64, ExecutionTimeline, GateDecision, GateInput, GateOutput, LatencyMetrics,
    LatencyWindow, RiskAdjustedScore, StateFreshness, TimestampedEvent, VolatilityEstimator,
    VolatilityObservation, chronos_edge_weight, compute_risk_adjusted_score, evaluate_gate,
};
pub use economic::{
    CostEntry, CostId, CostLedger, CostScope, CostType, CostUnit, ECONOMIC_CALCULATION_VERSION,
    EconomicConfig, EconomicQuote, LAMPORTS_PER_SOL, base_raw_to_lamports, build_economic_quote,
    calculate_flash_fee_raw, calculate_net_profit_raw,
};
pub use economic_integration::{
    ExecutionCosts, UnifiedEconomicCalculator, UnifiedEconomicConfig, UnifiedEconomicResult,
};
pub use latency::{
    COLD_MARGIN_MS, FirePlan, MAX_MARGIN_MS, MAX_SLOT_PAIRS, RttTracker, SLOT_MS, SlotClock,
    SlotFire, plan_fire,
};
pub use pump_fun_raydium::{
    DEFAULT_FLASH_LOAN_FEE_BPS, DEFAULT_RAYDIUM_FEE_BPS, MIN_NET_PROFIT_LAMPORTS, OptimalTrade,
    PumpFunParams, RaydiumParams, calculate_optimal_trade, calculate_optimal_trade_integer,
    pump_fun_buy_cost, pump_fun_marginal_price, raydium_sell_output, verify_marginal_decreasing,
};

pub use clmm::{
    ClmmMathError, MAX_SQRT_PRICE_X64, MAX_TICK, MAX_TICK_INDEX, MIN_SQRT_PRICE_X64, MIN_TICK,
    MIN_TICK_INDEX, Q64, StepSwapResult, TickArrayFacade, TickFacade, WhirlpoolQuote,
    clmm_active_tick_max_input, clmm_max_delta_x_in_tick, clmm_max_delta_y_in_tick,
    clmm_max_input_in_tick, compute_swap, compute_swap_step, compute_swap_step_checked,
    find_next_tick_boundary, get_amount_delta_a, get_amount_delta_b, get_delta_x, get_delta_y,
    get_next_sqrt_price_from_a_input, get_next_sqrt_price_from_b_input, get_start_tick_index,
    is_token_a, is_token_a_pubkey, parse_tick_array_account, quote_clmm,
    sqrt_price_x64_to_tick_index, tick_index_to_sqrt_price_x64, tick_to_sqrt_price_x64,
    update_liquidity_on_cross,
};
pub use cpmm::*;
pub use damm_v2::{
    DAMM_V2_FEE_DENOMINATOR, DammV2FeeParams, damm_v2_virtual_reserves, quote_damm_v2_exact_in,
};
pub use dlmm::{
    DlmmBin, DlmmError, DlmmParameters, DlmmPoolState, DlmmSwapQuote, DlmmSwapStepResult,
    DlmmVariableParameters, MAX_BIN_ID, MAX_FEE_RATE, MIN_BIN_ID, ONE_Q64, advance_active_id,
    compute_base_fee, compute_total_fee_rate, compute_variable_fee, consume_bin_liquidity,
    get_price_from_id, pow_q64, quote_dlmm_multi_bin, quote_dlmm_view,
};
pub use launchlab::{quote_launchlab_buy_exact_in, quote_launchlab_sell_exact_in};

use arb_types::Lamports;

/// Apply Token-2022 transfer fee deduction with ceiling rounding and max-fee cap.
pub fn apply_transfer_fee_ceil(amount: u64, fee_bps: u16, maximum_fee: u64) -> u64 {
    if amount == 0 || fee_bps == 0 {
        return amount;
    }
    let num = u128::from(amount).saturating_mul(u128::from(fee_bps));
    let raw_fee = num.saturating_add(9_999) / 10_000;
    let capped_fee = u64::try_from(raw_fee).unwrap_or(amount).min(maximum_fee);
    amount.saturating_sub(capped_fee)
}

/// Apply Token-2022 transfer fee deduction (bps-only convenience wrapper).
pub fn apply_transfer_fee(amount: u64, fee_bps: u16) -> u64 {
    apply_transfer_fee_ceil(amount, fee_bps, u64::MAX)
}

/// Select Jito tip bounded by profit, floor, cap, and minimum retained profit.
pub fn pick_tip(
    net_before_tip: u64,
    share_bps: u64,
    floor: u64,
    cap: u64,
    min_keep: u64,
) -> Option<u64> {
    let want = net_before_tip.checked_mul(share_bps)? / BASIS_POINTS_DIVISOR;
    let tip = want.max(floor).min(cap);
    let remaining = net_before_tip.checked_sub(tip)?;
    if remaining >= min_keep {
        Some(tip)
    } else {
        None
    }
}

/// Calculate net profit in Lamports after all fees, gas, ATA rent, and Jito tip.
pub fn calculate_net_profit(
    gross: Lamports,
    flash_fee: Lamports,
    priority_fee: Lamports,
    base_fee: Lamports,
    tip: Lamports,
    rent_ata: Lamports,
) -> Option<Lamports> {
    let total_costs = flash_fee
        .checked_add(priority_fee)?
        .checked_add(base_fee)?
        .checked_add(tip)?
        .checked_add(rent_ata)?;
    gross.checked_sub(total_costs)
}

#[cfg(test)]
use ruint::aliases::U256;

/// Invariant check: Verify Constant Product Invariant (k_after >= k_before).
#[cfg(test)]
pub fn check_constant_product_invariant(
    x_before: u128,
    y_before: u128,
    x_after: u128,
    y_after: u128,
) -> bool {
    let k_before = U256::from(x_before) * U256::from(y_before);
    let k_after = U256::from(x_after) * U256::from(y_after);
    k_after >= k_before
}

/// Invariant check: Verify that execution output respects slippage tolerance.
#[cfg(test)]
pub fn check_slippage_bound(expected_out: u64, min_out: u64, max_slippage_bps: u16) -> bool {
    if min_out > expected_out || expected_out == 0 {
        return false;
    }
    let diff = expected_out.saturating_sub(min_out);
    let slippage_bps =
        (u128::from(diff) * u128::from(BASIS_POINTS_DIVISOR)) / u128::from(expected_out);
    slippage_bps <= u128::from(max_slippage_bps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cpmm_out_basic() {
        let out = cpmm_out(10, 1000, 1000, 9975).expect("valid inputs");
        assert!(out > 0);
        assert!(out < 10);
    }

    #[test]
    fn test_optimal_two_pool_profitable() {
        // Pool 1: 1M SOL / 1M USDC, gamma=9975 (0.25% fee)
        // Pool 2: 1M USDC / 1.05M SOL, gamma=9975 (5% spread)
        let l1 = Leg {
            r_in: 1_000_000,
            r_out: 1_000_000,
            gamma: 9975,
        };
        let l2 = Leg {
            r_in: 1_000_000,
            r_out: 1_050_000,
            gamma: 9975,
        };

        let opt = optimal_two_pool(&l1, &l2);
        assert!(opt.is_some());
        assert!(opt.expect("invariant") > 0);
    }

    #[test]
    fn test_optimal_two_pool_unprofitable_returns_none() {
        // Spread too small to cover 2 * 0.25% fees
        let l1 = Leg {
            r_in: 1_000_000,
            r_out: 1_000_000,
            gamma: 9975,
        };
        let l2 = Leg {
            r_in: 1_000_000,
            r_out: 1_001_000,
            gamma: 9975,
        };

        assert!(optimal_two_pool(&l1, &l2).is_none());
    }

    #[test]
    fn test_optimal_three_pool_basic() {
        let l1 = Leg {
            r_in: 1_000_000,
            r_out: 1_000_000,
            gamma: 9975,
        };
        let l2 = Leg {
            r_in: 1_000_000,
            r_out: 1_000_000,
            gamma: 9975,
        };
        let l3 = Leg {
            r_in: 1_000_000,
            r_out: 1_100_000,
            gamma: 9975,
        };

        let opt = optimal_three_pool(&l1, &l2, &l3);
        assert!(opt.is_some());
        assert!(opt.expect("invariant") > 0);
    }

    #[test]
    fn test_transfer_fee_deduction() {
        assert_eq!(apply_transfer_fee(10_000, 100), 9_900); // 1% fee
        assert_eq!(apply_transfer_fee(10_000, 0), 10_000);
    }

    #[test]
    fn test_constant_product_invariant() {
        assert!(check_constant_product_invariant(1000, 1000, 1010, 1000));
        assert!(!check_constant_product_invariant(1000, 1000, 990, 1000));
    }

    #[test]
    fn test_slippage_bound() {
        assert!(check_slippage_bound(1000, 990, 100)); // 1% slippage <= 1%
        assert!(!check_slippage_bound(1000, 950, 100)); // 5% slippage > 1%
    }
}
