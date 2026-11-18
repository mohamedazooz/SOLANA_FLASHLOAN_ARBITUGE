//! Original DLMM differential/property test suite, moved verbatim.

use super::*;
use ruint::aliases::U256;

fn create_test_parameters() -> DlmmParameters {
    DlmmParameters {
        bin_step: 25, // 0.25%
        base_factor: 1000,
        filter_period: 30,
        decay_period: 600,
        reduction_factor: 5000, // 50%
        variable_fee_control: 10,
        max_volatility_accumulator: 350_000,
        protocol_share: 0,
        base_fee_power_factor: 0,
        collect_fee_mode: 0,
    }
}

fn create_test_pool(active_id: i32) -> DlmmPoolState {
    let parameters = create_test_parameters();
    DlmmPoolState {
        active_id,
        bin_step: parameters.bin_step,
        status: 0,
        parameters,
        v_parameters: DlmmVariableParameters {
            volatility_accumulator: 0,
            volatility_reference: 0,
            index_reference: active_id,
            last_update_timestamp: 1_700_000_000,
        },
        ..Default::default()
    }
}

#[test]
fn pow_q64_rejects_unrepresentable_prices() {
    assert!(pow_q64(u128::MAX, 1).is_err());
    assert!(pow_q64(ONE_Q64 / 2, (MAX_EXPONENTIAL - 1) as i32).is_err());
}

#[test]
fn view_quote_does_not_mutate_the_supplied_bin_snapshot() {
    let pool = create_test_pool(10);
    let bins = vec![DlmmBin {
        id: 10,
        amount_x: 0,
        amount_y: 1_000_000,
        price_q64: 0,
        ..Default::default()
    }];
    let before = bins.clone();
    let _ = quote_dlmm_view(&pool, &bins, 10_000, true, None);
    assert_eq!(bins, before);
}

#[test]
fn test_bin_price_at_zero() {
    let price = get_price_from_id(0, 25).expect("price at bin 0");
    assert_eq!(price, ONE_Q64, "P_0 must equal exactly 1.0 in Q64.64");
}

#[test]
fn test_bin_price_positive_exponent() {
    let price_1 = get_price_from_id(1, 100).expect("price at bin 1");
    // 1.01 * 2^64
    let expected = (U256::from(ONE_Q64) * U256::from(101)) / U256::from(100);
    assert_eq!(U256::from(price_1), expected);

    let price_10 = get_price_from_id(10, 25).expect("price at bin 10");
    assert!(price_10 > ONE_Q64);
}

#[test]
fn test_bin_price_negative_exponent() {
    let price_neg1 = get_price_from_id(-1, 100).expect("price at bin -1");
    // 1/1.01 * 2^64
    assert!(price_neg1 < ONE_Q64);
    assert!(price_neg1 > 0);

    let price_1 = get_price_from_id(1, 100).expect("price at bin 1");
    let prod = (U256::from(price_neg1) * U256::from(price_1)) >> SCALE_OFFSET;
    // Product of P_1 and P_{-1} must be approximately 1.0 in Q64.64
    let diff = if prod > U256::from(ONE_Q64) {
        prod - U256::from(ONE_Q64)
    } else {
        U256::from(ONE_Q64) - prod
    };
    assert!(
        diff <= U256::from(100),
        "Reciprocal price product roundoff within 100 units"
    );
}

#[test]
fn test_fee_calculation() {
    let params = create_test_parameters();
    // base_fee = 1000 * 25 * 10 = 250_000 (0.025% = 2.5 bps)
    let base_fee = compute_base_fee(&params, params.bin_step).expect("base fee");
    assert_eq!(base_fee, 250_000);

    // variable_fee with va = 10_000:
    // (10_000 * 25)^2 = 6.25e10. vfc = 10 -> 6.25e11. ceil(6.25e11 / 1e11) = 7
    let var_fee = compute_variable_fee(&params, params.bin_step, 10_000).expect("variable fee");
    assert_eq!(var_fee, 7);

    let total_fee = compute_total_fee_rate(&params, params.bin_step, 10_000).expect("total fee");
    assert_eq!(total_fee, 250_007);
}

#[test]
fn test_fee_rate_capped_at_max() {
    let mut params = create_test_parameters();
    params.base_factor = 50_000;
    params.base_fee_power_factor = 2; // 50k * 25 * 10 * 100 = 1.25B > 100M cap
    let total = compute_total_fee_rate(&params, params.bin_step, 0).expect("total fee");
    assert_eq!(total, MAX_FEE_RATE, "Fee rate must be clamped at 10%");
}

#[test]
fn test_single_bin_swap_x_to_y_partial() {
    let _pool = create_test_pool(0);
    let mut bin = DlmmBin {
        id: 0,
        amount_x: 0,
        amount_y: 1_000_000,
        price_q64: ONE_Q64,
        ..Default::default()
    };

    let step = consume_bin_liquidity(
        &mut bin, 100_000, ONE_Q64, true,    // X -> Y
        250_000, // 2.5 bps fee
        0, true, // fee on input
        false,
    )
    .expect("swap step");

    assert_eq!(step.amount_in_consumed, 100_000);
    assert!(!step.drained_bin);
    assert!(step.fee_amount > 0);
    assert_eq!(step.amount_out, 100_000 - step.fee_amount);
    assert_eq!(bin.amount_y, 1_000_000 - step.amount_out);
    assert_eq!(bin.amount_x, 100_000 - step.fee_amount);
}

#[test]
fn test_single_bin_swap_x_to_y_full_drain() {
    let _pool = create_test_pool(0);
    let mut bin = DlmmBin {
        id: 0,
        amount_x: 0,
        amount_y: 100_000,
        price_q64: ONE_Q64,
        ..Default::default()
    };

    let step = consume_bin_liquidity(
        &mut bin, 1_000_000, // much larger than available 100k
        ONE_Q64, true, // X -> Y
        250_000, 0, true, // fee on input
        false,
    )
    .expect("swap step");

    assert!(step.drained_bin);
    assert_eq!(step.amount_out, 100_000);
    assert_eq!(bin.amount_y, 0);
    assert!(step.amount_in_consumed > 100_000); // includes fee
}

#[test]
fn test_multi_bin_traversal_x_to_y() {
    let pool = create_test_pool(10);
    let mut bins = vec![
        DlmmBin {
            id: 10,
            amount_x: 0,
            amount_y: 1_000_000,
            price_q64: 0,
            ..Default::default()
        },
        DlmmBin {
            id: 9,
            amount_x: 0,
            amount_y: 2_000_000,
            price_q64: 0,
            ..Default::default()
        },
        DlmmBin {
            id: 8,
            amount_x: 0,
            amount_y: 5_000_000,
            price_q64: 0,
            ..Default::default()
        },
    ];

    let quote = quote_dlmm_multi_bin(
        &pool, &mut bins, 2_000_000, true, // X -> Y
        None,
    )
    .expect("multi-bin quote");

    assert_eq!(quote.amount_in, 2_000_000);
    assert_eq!(quote.end_active_id, 9);
    assert_eq!(quote.bins_crossed, 1);
    assert!(quote.amount_out > 2_000_000); // Price is > 1.0 at bin 10 & 9
    assert_eq!(quote.fee_amount, 501);
}

#[test]
fn test_multi_bin_traversal_y_to_x() {
    let pool = create_test_pool(10);
    let mut bins = vec![
        DlmmBin {
            id: 10,
            amount_x: 1_000_000,
            amount_y: 0,
            price_q64: 0,
            ..Default::default()
        },
        DlmmBin {
            id: 11,
            amount_x: 2_000_000,
            amount_y: 0,
            price_q64: 0,
            ..Default::default()
        },
        DlmmBin {
            id: 12,
            amount_x: 5_000_000,
            amount_y: 0,
            price_q64: 0,
            ..Default::default()
        },
    ];

    let quote = quote_dlmm_multi_bin(
        &pool, &mut bins, 2_000_000, false, // Y -> X
        None,
    )
    .expect("multi-bin quote");

    assert_eq!(quote.amount_in, 2_000_000);
    assert_eq!(quote.end_active_id, 11);
    assert_eq!(quote.bins_crossed, 1);
    assert!(quote.amount_out < 2_000_000); // Token X is more expensive than Y
    assert_eq!(quote.fee_amount, 501);
}

#[test]
fn test_collect_fee_mode_matrix() {
    // Mode 0 = InputOnly: fee always on input, both directions.
    assert!(dlmm_fee_on_input(0, true).unwrap()); // X->Y
    assert!(dlmm_fee_on_input(0, false).unwrap()); // Y->X

    // Mode 1 = OnlyY.
    assert!(!dlmm_fee_on_input(1, true).unwrap()); // X->Y: fee on output Y
    assert!(dlmm_fee_on_input(1, false).unwrap()); // Y->X: fee on input Y

    // Invalid mode rejected.
    assert!(dlmm_fee_on_input(2, true).is_err());

    // Enum round-trip.
    assert_eq!(CollectFeeMode::InputOnly.as_u8(), 0);
    assert_eq!(CollectFeeMode::OnlyY.as_u8(), 1);
    assert_eq!(
        CollectFeeMode::from_u8(0).unwrap(),
        CollectFeeMode::InputOnly
    );
    assert_eq!(CollectFeeMode::from_u8(1).unwrap(), CollectFeeMode::OnlyY);

    // Regression: reversed enum values must not silently pass.
    assert_ne!(CollectFeeMode::from_u8(0).unwrap(), CollectFeeMode::OnlyY);
    assert_ne!(
        CollectFeeMode::from_u8(1).unwrap(),
        CollectFeeMode::InputOnly
    );
}

#[test]
fn test_volatility_decay() {
    let params = create_test_parameters();
    let mut v_params = DlmmVariableParameters {
        volatility_accumulator: 20_000,
        volatility_reference: 20_000,
        index_reference: 0,
        last_update_timestamp: 1_000,
    };

    // Within filter period (elapsed 10s < 30s) -> no change
    update_references_with_decay(&mut v_params, &params, 5, 1_010).expect("decay update");
    assert_eq!(v_params.volatility_reference, 20_000);
    assert_eq!(v_params.index_reference, 0);

    // Between filter & decay period (elapsed 100s, filter=30, decay=600) -> 50% decay
    update_references_with_decay(&mut v_params, &params, 5, 1_100).expect("decay update");
    assert_eq!(v_params.index_reference, 5);
    assert_eq!(v_params.volatility_reference, 10_000); // 20k * 50%

    // Past decay period (elapsed 700s > 600s) -> drops to 0
    update_references_with_decay(&mut v_params, &params, 5, 1_700).expect("decay update");
    assert_eq!(v_params.volatility_reference, 0);
}

#[test]
fn test_dlmm_active_bin_max_input() {
    // Price at bin 0 with 25 bps is 1.0 (ONE_Q64)
    // a_to_b: pool pays Y, so max input X depends on amount_y
    let cap_x = dlmm_active_bin_max_input(0, 25, 5_000_000, 10_000_000, true);
    assert_eq!(cap_x, Some(10_000_000));

    // b_to_a: pool pays X, so max input Y depends on amount_x
    let cap_y = dlmm_active_bin_max_input(0, 25, 5_000_000, 10_000_000, false);
    assert_eq!(cap_y, Some(5_000_000));

    // If active bin has zero output token liquidity, returns None
    assert_eq!(dlmm_active_bin_max_input(0, 25, 5_000_000, 0, true), None);
    assert_eq!(dlmm_active_bin_max_input(0, 25, 0, 10_000_000, false), None);
}

#[test]
fn test_dlmm_multi_bin_max_input_walks_past_empty_active_bin() {
    // Active bin 0 has ZERO output-side liquidity (post-crossing state).
    // The old active-bin-only capacity returned None -> INSUFFICIENT_LIQUIDITY
    // even though neighbouring bins hold deep liquidity.
    let bins = vec![
        DlmmBin {
            id: 2,
            amount_x: 0,
            amount_y: 0,
            price_q64: 0,
            ..Default::default()
        },
        DlmmBin {
            id: 1,
            amount_x: 0,
            amount_y: 0,
            price_q64: 0,
            ..Default::default()
        },
        DlmmBin {
            id: 0,
            amount_x: 5_000_000,
            amount_y: 0,
            price_q64: 0,
            ..Default::default()
        },
        DlmmBin {
            id: -1,
            amount_x: 0,
            amount_y: 10_000_000,
            price_q64: 0,
            ..Default::default()
        },
        DlmmBin {
            id: -2,
            amount_x: 0,
            amount_y: 20_000_000,
            price_q64: 0,
            ..Default::default()
        },
    ];
    // X->Y: bin 0 contributes 0 (amount_y=0); bin -1 gives 10M/price(-1);
    // bin -2 gives 20M/price(-2). Prices near 1.0 for 25bps steps.
    let cap_down = dlmm_multi_bin_max_input(0, 25, &bins, true).expect("capacity");
    assert!(
        cap_down > 29_000_000,
        "multi-bin capacity must exceed the single-bin zero: {cap_down}"
    );

    // Y->X from active bin 0: bin 0's X reserve (5M) absorbs input; bins
    // above (1, 2) hold no X and contribute 0. State complete -> Some.
    let cap_up = dlmm_multi_bin_max_input(0, 25, &bins, false).expect("state complete");
    assert_eq!(cap_up, 5_000_000);
}

#[test]
fn test_dlmm_multi_bin_max_input_fails_closed_on_missing_active_bin() {
    let bins = vec![
        DlmmBin {
            id: -3,
            amount_x: 0,
            amount_y: 9_000_000,
            price_q64: 0,
            ..Default::default()
        },
        DlmmBin {
            id: -2,
            amount_x: 0,
            amount_y: 9_000_000,
            price_q64: 0,
            ..Default::default()
        },
    ];
    // Active bin 0 not present in the window: incomplete state -> None.
    assert_eq!(dlmm_multi_bin_max_input(0, 25, &bins, true), None);
    assert_eq!(dlmm_multi_bin_max_input(0, 25, &[], true), None);
}

#[test]
fn test_dlmm_multi_bin_max_input_matches_single_bin_when_isolated() {
    // With only the active bin present, multi-bin == active-bin result.
    let bins = vec![DlmmBin {
        id: 0,
        amount_x: 5_000_000,
        amount_y: 10_000_000,
        price_q64: 0,
        ..Default::default()
    }];
    assert_eq!(
        dlmm_multi_bin_max_input(0, 25, &bins, true),
        dlmm_active_bin_max_input(0, 25, 5_000_000, 10_000_000, true)
    );
    assert_eq!(
        dlmm_multi_bin_max_input(0, 25, &bins, false),
        dlmm_active_bin_max_input(0, 25, 5_000_000, 10_000_000, false)
    );
}

#[test]
fn test_scale_dlmm_three_hop_input() {
    // Probe: 10,000 lamports giving 23,000 lamports profit
    let probe_in = 10_000;
    let probe_profit = 23_000;

    // Bottleneck is 1,000,000 across 3 hops
    let hop1_cap = 2_000_000;
    let hop2_cap = 1_000_000;
    let hop3_cap = 3_000_000;
    let max_wallet = 10_000_000;

    let (scaled_in, scaled_profit) = scale_dlmm_three_hop_input(
        probe_in,
        probe_profit,
        hop1_cap,
        hop2_cap,
        hop3_cap,
        max_wallet,
    );

    // Safe optimal in = (1,000,000 * 92) / 100 = 920,000
    assert_eq!(scaled_in, 920_000);
    // Scaled profit = 23,000 * 920,000 / 10,000 = 2,116,000 lamports!
    assert_eq!(scaled_profit, 2_116_000);
}
