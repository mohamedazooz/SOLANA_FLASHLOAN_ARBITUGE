//! Feature 8: Meteora DLMM Exact Q64.64 Math & Dynamic Volatility — Tier 2 Boundary Tests.
//!
//! Validates volatility accumulator saturation, variable fee rate caps at MAX_FEE_RATE,
//! pow_q64 exponent boundaries, empty bin slice rejection, and zero-liquidity bin traversal.

use arb_math::dlmm::{
    DlmmBin, DlmmError, DlmmParameters, DlmmPoolState, DlmmVariableParameters, MAX_FEE_RATE,
    ONE_Q64, compute_total_fee_rate, compute_variable_fee, pow_q64, quote_dlmm_multi_bin,
    update_volatility_accumulator,
};

fn default_test_params() -> (DlmmParameters, DlmmVariableParameters) {
    let params = DlmmParameters {
        bin_step: 10,
        base_factor: 10,
        filter_period: 30,
        decay_period: 600,
        reduction_factor: 5000,
        variable_fee_control: 40_000,
        max_volatility_accumulator: 350_000,
        protocol_share: 1000,
        base_fee_power_factor: 0,
        collect_fee_mode: 0,
    };
    let v_params = DlmmVariableParameters {
        volatility_accumulator: 0,
        volatility_reference: 0,
        index_reference: 0,
        last_update_timestamp: 0,
    };
    (params, v_params)
}

/// Boundary Case 1: Volatility accumulator saturation at max_volatility_accumulator.
#[test]
fn test_boundary_f8_volatility_accumulator_saturation() {
    let (params, mut v_params) = default_test_params();
    // Large bin jump: active_id = 50_000 from index_reference = 0.
    // delta = 50_000 * 10_000 = 500_000_000 > max_volatility_accumulator (350_000).
    update_volatility_accumulator(&mut v_params, &params, 50_000).unwrap();

    assert_eq!(
        v_params.volatility_accumulator, params.max_volatility_accumulator,
        "accumulator must saturate exactly at max_volatility_accumulator (350_000)"
    );
}

/// Boundary Case 2: Variable fee rate is strictly capped at MAX_FEE_RATE (10% = 100_000_000).
#[test]
fn test_boundary_f8_variable_fee_saturation_capped_at_max() {
    let (mut params, _) = default_test_params();
    params.variable_fee_control = 1_000_000;
    let bin_step = 100;
    let massive_va = 350_000;

    let var_fee = compute_variable_fee(&params, bin_step, massive_va).unwrap();
    assert!(var_fee > MAX_FEE_RATE, "uncapped variable fee exceeds max");

    let total_fee = compute_total_fee_rate(&params, bin_step, massive_va).unwrap();
    assert_eq!(
        total_fee, MAX_FEE_RATE,
        "total fee rate must be strictly clamped to MAX_FEE_RATE"
    );
}

/// Boundary Case 3: pow_q64 extreme exponent and overflow boundaries.
#[test]
fn test_boundary_f8_pow_q64_extreme_exponent_boundaries() {
    // Sub-case A: exp == 0 yields 1.0 (ONE_Q64)
    assert_eq!(pow_q64(ONE_Q64, 0).unwrap(), ONE_Q64);
    assert_eq!(pow_q64(123456789, 0).unwrap(), ONE_Q64);

    // Sub-case B: extreme exponent >= MAX_EXPONENTIAL (524_288) returns MathOverflow
    assert_eq!(
        pow_q64(ONE_Q64 + 100, 524_288),
        Err(DlmmError::MathOverflow)
    );
    assert_eq!(
        pow_q64(ONE_Q64 + 100, -524_288),
        Err(DlmmError::MathOverflow)
    );

    // Sub-case C: base = 0 with positive exp returns underflow/overflow error
    assert_eq!(pow_q64(0, 1), Err(DlmmError::MathOverflow));
}

/// Boundary Case 4: Empty bin array slice fails closed with BinNotFound.
#[test]
fn test_boundary_f8_empty_bin_array_slice_fails_closed() {
    let (params, v_params) = default_test_params();
    let pool = DlmmPoolState {
        active_id: 100,
        bin_step: 10,
        status: 0,
        parameters: params,
        v_parameters: v_params,
        support_limit_order: false,
    };
    let mut empty_bins: Vec<DlmmBin> = Vec::new();

    let res = quote_dlmm_multi_bin(&pool, &mut empty_bins, 1_000_000, true, None);
    assert_eq!(res.err(), Some(DlmmError::BinNotFound(100)));
}

/// Boundary Case 5: Traversal across zero-liquidity bins skips accumulator bump and continues.
#[test]
fn test_boundary_f8_zero_liquidity_bins_traversal() {
    let (params, v_params) = default_test_params();
    let pool = DlmmPoolState {
        active_id: 100,
        bin_step: 10,
        status: 0,
        parameters: params,
        v_parameters: v_params,
        support_limit_order: false,
    };

    // Bin 100 is empty (0 liquidity), Bin 99 has sufficient liquidity
    let mut bins = vec![
        DlmmBin {
            id: 100,
            amount_x: 0,
            amount_y: 0,
            price_q64: 0,
            open_order_amount: 0,
            processed_order_remaining_amount: 0,
            limit_order_ask_side: false,
        },
        DlmmBin {
            id: 99,
            amount_x: 0,
            amount_y: 10_000_000,
            price_q64: 0,
            open_order_amount: 0,
            processed_order_remaining_amount: 0,
            limit_order_ask_side: false,
        },
    ];

    let quote = quote_dlmm_multi_bin(&pool, &mut bins, 1_000, true, None).unwrap();
    assert!(
        quote.amount_out > 0,
        "must successfully cross empty active bin and consume liquidity from next bin"
    );
    assert_eq!(quote.bins_crossed, 1, "exactly one bin transition crossed");
}
