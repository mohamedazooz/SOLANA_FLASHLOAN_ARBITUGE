//! Adversarial Empirical Stress Tests for crates/arb-math (Milestone M1).
//!
//! Rigorously challenges:
//! 1. Orca Whirlpool CLMM:
//!    - Multi-tick boundary traversal (1, 2, and 3 tick boundaries crossed).
//!    - Extreme tick limits (MIN_TICK_INDEX = -443636, MAX_TICK_INDEX = 443636) and out-of-bounds guards.
//!    - Directional rounding solvency: pool never overpays (ceiling on inflows, floor on outflows).
//!    - Strict Token A/B ordering invariance & antisymmetry.
//! 2. Meteora DLMM:
//!    - Extreme bin steps (1 to 1000 bps) and multi-bin active crossings.
//!    - Dynamic variable fee surge under volatile accumulator ($v_a$) values up to u32::MAX.
//!    - Strict 10% maximum fee rate clamping (MAX_FEE_RATE = 100_000_000).
//!    - Zero-liquidity bin traversal and sparse liquidity gaps.

use arb_math::{
    ClmmMathError, DlmmBin, DlmmError, DlmmParameters, DlmmPoolState, DlmmVariableParameters,
    MAX_FEE_RATE, MAX_SQRT_PRICE_X64, MAX_TICK_INDEX, MIN_SQRT_PRICE_X64, MIN_TICK_INDEX, ONE_Q64,
    TickArrayFacade, TickFacade, compute_swap, compute_total_fee_rate, compute_variable_fee,
    get_amount_delta_a, get_amount_delta_b, get_next_sqrt_price_from_a_input,
    get_next_sqrt_price_from_b_input, get_price_from_id, is_token_a, is_token_a_pubkey,
    quote_dlmm_view, sqrt_price_x64_to_tick_index, tick_index_to_sqrt_price_x64,
};
use solana_pubkey::Pubkey;

// ============================================================================
// 1. ORCA WHIRLPOOL CLMM EMPIRICAL CHALLENGES
// ============================================================================

#[test]
fn challenge_clmm_cross_one_tick_boundary_a_to_b() {
    let p0 = tick_index_to_sqrt_price_x64(0).expect("Price at tick 0");
    let spacing = 64u16;
    let liquidity = 10_000_000_000_000u128;
    let fee_rate = 500u16; // 5 bps (500 / 1_000_000)

    // Tick array with initialized tick at -64
    let tick_arrays = vec![TickArrayFacade {
        start_tick_index: -5632,
        ticks: vec![TickFacade {
            tick_index: -64,
            initialized: true,
            liquidity_net: 2_000_000_000_000,
            liquidity_gross: 2_000_000_000_000,
        }],
    }];

    // Compute input needed to reach exactly tick -64
    let p_target = tick_index_to_sqrt_price_x64(-64).expect("Target price");
    let delta_a_exact = get_amount_delta_a(p0, p_target, liquidity, true).expect("invariant");
    // Provide 50% more input to cross the boundary into the next tick interval
    let amount_in = delta_a_exact.saturating_mul(15) / 10;

    let quote = compute_swap(
        p0,
        0,
        liquidity,
        spacing,
        fee_rate,
        amount_in,
        true, // A to B
        &tick_arrays,
    )
    .expect("Swap should succeed");

    assert!(
        quote.steps_taken >= 2,
        "Swap must take >= 2 steps to cross tick boundary, took {}",
        quote.steps_taken
    );
    assert!(
        quote.final_tick_index < -64,
        "Final tick index ({}) must be strictly past boundary -64",
        quote.final_tick_index
    );
    assert_eq!(
        quote.amount_in_consumed, amount_in,
        "Must consume full input"
    );
    assert!(quote.amount_out_received > 0, "Must deliver output");
}

#[test]
fn challenge_clmm_cross_one_tick_boundary_b_to_a() {
    let p0 = tick_index_to_sqrt_price_x64(0).expect("Price at tick 0");
    let spacing = 64u16;
    let liquidity = 10_000_000_000_000u128;
    let fee_rate = 500u16;

    // Tick array with initialized tick at +64
    let tick_arrays = vec![TickArrayFacade {
        start_tick_index: 0,
        ticks: vec![TickFacade {
            tick_index: 64,
            initialized: true,
            liquidity_net: 2_000_000_000_000,
            liquidity_gross: 2_000_000_000_000,
        }],
    }];

    let p_target = tick_index_to_sqrt_price_x64(64).expect("Target price");
    let delta_b_exact = get_amount_delta_b(p0, p_target, liquidity, true).expect("invariant");
    let amount_in = delta_b_exact.saturating_mul(15) / 10;

    let quote = compute_swap(
        p0,
        0,
        liquidity,
        spacing,
        fee_rate,
        amount_in,
        false, // B to A
        &tick_arrays,
    )
    .expect("Swap should succeed");

    assert!(
        quote.steps_taken >= 2,
        "Swap must take >= 2 steps to cross tick boundary, took {}",
        quote.steps_taken
    );
    assert!(
        quote.final_tick_index >= 64,
        "Final tick index ({}) must be at or past boundary 64",
        quote.final_tick_index
    );
    assert_eq!(quote.amount_in_consumed, amount_in);
    assert!(quote.amount_out_received > 0);
}

#[test]
fn challenge_clmm_cross_two_and_three_tick_boundaries() {
    let p0 = tick_index_to_sqrt_price_x64(0).expect("Price at tick 0");
    let spacing = 64u16;
    let initial_liquidity = 10_000_000_000_000u128;
    let fee_rate = 300u16; // 3 bps

    // 3 initialized ticks at -64, -128, -192
    let tick_arrays = vec![TickArrayFacade {
        start_tick_index: -5632,
        ticks: vec![
            TickFacade {
                tick_index: -64,
                initialized: true,
                liquidity_net: 2_000_000_000_000, // L becomes 8T
                liquidity_gross: 2_000_000_000_000,
            },
            TickFacade {
                tick_index: -128,
                initialized: true,
                liquidity_net: -1_000_000_000_000, // L becomes 9T
                liquidity_gross: 1_000_000_000_000,
            },
            TickFacade {
                tick_index: -192,
                initialized: true,
                liquidity_net: 3_000_000_000_000, // L becomes 6T
                liquidity_gross: 3_000_000_000_000,
            },
        ],
    }];

    // 1. Two-boundary crossing: reach past -128 but before -192
    let p_64 = tick_index_to_sqrt_price_x64(-64).expect("invariant");
    let p_128 = tick_index_to_sqrt_price_x64(-128).expect("invariant");
    let d1 = get_amount_delta_a(p0, p_64, initial_liquidity, true).expect("invariant");
    let d2 = get_amount_delta_a(p_64, p_128, 8_000_000_000_000, true).expect("invariant");
    // Account for fee buffer (0.03%) plus extra amount to cross into interval [-192, -128)
    let net_needed_2 = d1 + d2 + 50_000;
    let amount_in_2_boundaries = net_needed_2 + (net_needed_2 * 300 / 1_000_000) + 10_000;

    let quote_2 = compute_swap(
        p0,
        0,
        initial_liquidity,
        spacing,
        fee_rate,
        amount_in_2_boundaries,
        true,
        &tick_arrays,
    )
    .expect("2-boundary swap should succeed");

    assert!(
        quote_2.steps_taken >= 3,
        "Crossing 2 boundaries must take >= 3 steps, took {}",
        quote_2.steps_taken
    );
    assert!(
        quote_2.final_tick_index < -128 && quote_2.final_tick_index >= -192,
        "Final tick {} must be between -128 and -192",
        quote_2.final_tick_index
    );

    // 2. Three-boundary crossing: reach past -192
    let p_192 = tick_index_to_sqrt_price_x64(-192).expect("invariant");
    let d3 = get_amount_delta_a(p_128, p_192, 9_000_000_000_000, true).expect("invariant");
    let net_needed_3 = d1 + d2 + d3 + 100_000;
    let amount_in_3_boundaries = net_needed_3 + (net_needed_3 * 300 / 1_000_000) + 20_000;

    let quote_3 = compute_swap(
        p0,
        0,
        initial_liquidity,
        spacing,
        fee_rate,
        amount_in_3_boundaries,
        true,
        &tick_arrays,
    )
    .expect("3-boundary swap should succeed");

    assert!(
        quote_3.steps_taken >= 4,
        "Crossing 3 boundaries must take >= 4 steps, took {}",
        quote_3.steps_taken
    );
    assert!(
        quote_3.final_tick_index < -192,
        "Final tick {} must be past boundary -192",
        quote_3.final_tick_index
    );
}

#[test]
fn challenge_clmm_extreme_price_ticks() {
    // 1. Min tick bounds
    let p_min = tick_index_to_sqrt_price_x64(MIN_TICK_INDEX).expect("Min tick price");
    assert_eq!(p_min, MIN_SQRT_PRICE_X64);
    let tick_from_p_min = sqrt_price_x64_to_tick_index(p_min).expect("Inverse min tick");
    assert_eq!(tick_from_p_min, MIN_TICK_INDEX);

    // 2. Max tick bounds
    let p_max = tick_index_to_sqrt_price_x64(MAX_TICK_INDEX).expect("Max tick price");
    assert!(p_max <= MAX_SQRT_PRICE_X64);
    let tick_from_p_max = sqrt_price_x64_to_tick_index(p_max).expect("Inverse max tick");
    assert_eq!(tick_from_p_max, MAX_TICK_INDEX);

    // 3. Out-of-bounds rejection
    assert_eq!(
        tick_index_to_sqrt_price_x64(MIN_TICK_INDEX - 1),
        Err(ClmmMathError::TickIndexOutOfBounds(MIN_TICK_INDEX - 1))
    );
    assert_eq!(
        tick_index_to_sqrt_price_x64(MAX_TICK_INDEX + 1),
        Err(ClmmMathError::TickIndexOutOfBounds(MAX_TICK_INDEX + 1))
    );

    assert_eq!(
        sqrt_price_x64_to_tick_index(MIN_SQRT_PRICE_X64 - 1),
        Err(ClmmMathError::SqrtPriceOutOfBounds)
    );
    assert_eq!(
        sqrt_price_x64_to_tick_index(MAX_SQRT_PRICE_X64 + 1),
        Err(ClmmMathError::SqrtPriceOutOfBounds)
    );

    // 4. Safe termination at boundaries
    let quote_at_min = compute_swap(
        p_min,
        MIN_TICK_INDEX,
        1_000_000,
        64,
        500,
        100_000,
        true, // attempting to push below min tick
        &[],
    );
    assert!(quote_at_min.is_ok());

    let quote_at_max = compute_swap(
        p_max,
        MAX_TICK_INDEX,
        1_000_000,
        64,
        500,
        100_000,
        false, // attempting to push above max tick
        &[],
    );
    assert!(quote_at_max.is_ok());
}

#[test]
fn challenge_clmm_rounding_solvency_invariants() {
    // Solvency test: For ANY valid prices and liquidity:
    // Inflow delta (round_up = true) MUST be >= Outflow delta (round_up = false)
    // Pool must NEVER pay out more tokens than mathematically deposited.
    let test_ticks = [
        (-100_000, -80_000),
        (-10_000, -5_000),
        (-64, 0),
        (0, 64),
        (5_000, 10_000),
        (80_000, 100_000),
    ];
    let liquidities = [
        1u128,
        1_000_000u128,
        10_000_000_000u128,
        1_000_000_000_000u128,
    ];

    for (t1, t2) in test_ticks {
        let p1 = tick_index_to_sqrt_price_x64(t1).expect("invariant");
        let p2 = tick_index_to_sqrt_price_x64(t2).expect("invariant");

        for &l in &liquidities {
            // Delta A solvency
            if let (Ok(da_in), Ok(da_out)) = (
                get_amount_delta_a(p1, p2, l, true),
                get_amount_delta_a(p1, p2, l, false),
            ) {
                assert!(
                    da_in >= da_out,
                    "Delta A solvency violation: in {da_in} < out {da_out} at ticks ({t1}, {t2})"
                );
            }

            // Delta B solvency
            if let (Ok(db_in), Ok(db_out)) = (
                get_amount_delta_b(p1, p2, l, true),
                get_amount_delta_b(p1, p2, l, false),
            ) {
                assert!(
                    db_in >= db_out,
                    "Delta B solvency violation: in {db_in} < out {db_out} at ticks ({t1}, {t2})"
                );
            }

            // Next sqrt price precision solvency
            if l >= 1_000_000 {
                let amount_in = 50_000u64;
                if let Ok(next_p_a) = get_next_sqrt_price_from_a_input(p2, l, amount_in) {
                    assert!(next_p_a <= p2, "A->B must decrease price");
                    if let Ok(calc_in_a) = get_amount_delta_a(next_p_a, p2, l, true) {
                        assert!(
                            calc_in_a <= amount_in,
                            "Required inflow {calc_in_a} cannot exceed available input {amount_in}"
                        );
                    }
                }

                if let Ok(next_p_b) = get_next_sqrt_price_from_b_input(p1, l, amount_in) {
                    assert!(next_p_b >= p1, "B->A must increase price");
                    if let Ok(calc_in_b) = get_amount_delta_b(p1, next_p_b, l, true) {
                        assert!(
                            calc_in_b <= amount_in,
                            "Required inflow {calc_in_b} cannot exceed available input {amount_in}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn challenge_clmm_token_ordering_invariance() {
    // 1. Strict antisymmetry
    let m1 = Pubkey::new_unique();
    let m2 = Pubkey::new_unique();
    assert_ne!(m1, m2);

    let is_a_12 = is_token_a_pubkey(&m1, &m2);
    let is_a_21 = is_token_a_pubkey(&m2, &m1);
    assert!(
        is_a_12 ^ is_a_21,
        "Strict antisymmetry: exactly one must be token A"
    );
    assert!(
        !is_token_a_pubkey(&m1, &m1),
        "Reflexivity: same token is not token A of itself"
    );

    // Array form
    let a1: [u8; 32] = m1.to_bytes();
    let a2: [u8; 32] = m2.to_bytes();
    assert_eq!(is_token_a(&a1, &a2), is_a_12);
    assert_eq!(is_token_a(&a2, &a1), is_a_21);

    // 2. Round-trip thermodynamic conservation (no free money pump)
    let p0 = tick_index_to_sqrt_price_x64(0).expect("invariant");
    let initial_l = 10_000_000_000_000u128;
    let fee_rate = 500u16; // 5 bps

    let amount_a_in = 1_000_000u64;
    // Step 1: A -> B
    let quote_ab =
        compute_swap(p0, 0, initial_l, 64, fee_rate, amount_a_in, true, &[]).expect("invariant");
    assert!(quote_ab.amount_out_received > 0);

    // Step 2: Swap output B -> A at the updated price
    let quote_ba = compute_swap(
        quote_ab.final_sqrt_price_x64,
        quote_ab.final_tick_index,
        initial_l,
        64,
        fee_rate,
        quote_ab.amount_out_received,
        false,
        &[],
    )
    .expect("invariant");

    // Roundtrip output MUST be strictly less than original input due to fees and bid-ask spread
    assert!(
        quote_ba.amount_out_received < amount_a_in,
        "Round-trip output {} must be < input {} (no free energy)",
        quote_ba.amount_out_received,
        amount_a_in
    );
}

// ============================================================================
// 2. METEORA DLMM EMPIRICAL CHALLENGES
// ============================================================================

#[test]
fn challenge_dlmm_extreme_bin_steps_and_active_bin_crossing() {
    let extreme_steps: [u16; 5] = [1, 10, 100, 500, 1000]; // 0.01% to 10% per bin

    for &bs in &extreme_steps {
        // Verify price computation does not overflow or produce zero
        let p_zero = get_price_from_id(0, bs).expect("P(0)");
        assert_eq!(p_zero, ONE_Q64, "P(0) must always be 1.0 in Q64.64");

        let p_pos = get_price_from_id(100, bs).expect("P(100)");
        assert!(p_pos > p_zero, "Positive bin must have P > 1.0");

        let p_neg = get_price_from_id(-100, bs).expect("P(-100)");
        assert!(p_neg < p_zero, "Negative bin must have P < 1.0");

        // Multi-bin crossing across 5 discrete bins
        let bins = vec![
            DlmmBin {
                id: -2,
                amount_x: 0,
                amount_y: 500_000,
                price_q64: 0,
            },
            DlmmBin {
                id: -1,
                amount_x: 0,
                amount_y: 500_000,
                price_q64: 0,
            },
            DlmmBin {
                id: 0,
                amount_x: 0,
                amount_y: 500_000,
                price_q64: 0,
            },
            DlmmBin {
                id: 1,
                amount_x: 500_000,
                amount_y: 0,
                price_q64: 0,
            },
            DlmmBin {
                id: 2,
                amount_x: 500_000,
                amount_y: 0,
                price_q64: 0,
            },
        ];

        let pool = DlmmPoolState {
            active_id: 0,
            bin_step: bs,
            status: 0,
            parameters: DlmmParameters {
                bin_step: bs,
                base_factor: 1000,
                filter_period: 30,
                decay_period: 600,
                reduction_factor: 5000,
                variable_fee_control: 10,
                max_volatility_accumulator: 350_000,
                protocol_share: 0,
                base_fee_power_factor: 0,
            },
            v_parameters: DlmmVariableParameters {
                volatility_accumulator: 0,
                volatility_reference: 0,
                index_reference: 0,
                last_update_timestamp: 1_000_000,
            },
        };

        // Swap X -> Y: drains bin 0, bin -1, and partially bin -2
        let amount_in_drain = 1_500_000u64;
        let quote_xy = quote_dlmm_view(&pool, &bins, amount_in_drain, true, Some(1_000_005))
            .expect("DLMM X->Y multi-bin swap");

        assert!(
            quote_xy.bins_crossed >= 2,
            "Must cross >= 2 bins, crossed {}",
            quote_xy.bins_crossed
        );
        assert_eq!(quote_xy.end_active_id, -2, "Must end in bin -2");
        assert!(
            quote_xy.amount_out > 1_000_000,
            "Must deliver reserves from drained bins"
        );

        // Swap Y -> X: drains bin 1, partially fills bin 2
        let pool_for_yx = DlmmPoolState {
            active_id: 1,
            ..pool
        };
        let amount_in_yx = 700_000u64;
        let quote_yx = quote_dlmm_view(&pool_for_yx, &bins, amount_in_yx, false, Some(1_000_005))
            .expect("DLMM Y->X multi-bin swap");
        assert!(quote_yx.bins_crossed >= 1, "Must cross bins moving upwards");
        assert_eq!(quote_yx.end_active_id, 2, "Must end in bin 2");
        assert!(
            quote_yx.amount_out > 500_000,
            "Must deliver reserves from drained bin 1 and part of bin 2"
        );
    }
}

#[test]
fn challenge_dlmm_variable_fee_surge_under_high_volatility() {
    let params = DlmmParameters {
        bin_step: 25,
        base_factor: 1000,
        filter_period: 30,
        decay_period: 600,
        reduction_factor: 5000,
        variable_fee_control: 50,
        max_volatility_accumulator: 500_000,
        protocol_share: 0,
        base_fee_power_factor: 0,
    };

    // Test monotonicity across accumulator values
    let mut prev_fee = 0u128;
    let va_test_points = [
        0u32, 100, 1_000, 2_000, 5_000, 10_000, 50_000, 100_000, 350_000,
    ];

    for &va in &va_test_points {
        let v_fee =
            compute_variable_fee(&params, params.bin_step, va).expect("Compute variable fee");
        if va == 0 {
            assert_eq!(v_fee, 0);
        } else {
            // Due to ceil division div_ceil(num, 10^11), any non-zero numerator < 10^11 rounds to 1
            assert!(
                v_fee >= prev_fee,
                "Variable fee must be monotonically non-decreasing: {v_fee} >= {prev_fee} at va={va}"
            );
            if va >= 2_000 {
                assert!(
                    v_fee > prev_fee,
                    "Variable fee must strictly increase once num > 10^11: {v_fee} > {prev_fee} at va={va}"
                );
            }
        }
        prev_fee = v_fee;
    }

    // Stress test with maximum possible u32 volatility accumulator
    let extreme_v_fee = compute_variable_fee(&params, params.bin_step, u32::MAX);
    assert!(
        extreme_v_fee.is_ok(),
        "Must handle u32::MAX without overflow"
    );
}

#[test]
fn challenge_dlmm_maximum_fee_rate_clamping() {
    // 1. Extreme base fee alone
    let high_base_params = DlmmParameters {
        bin_step: 1000,
        base_factor: 65535,
        filter_period: 30,
        decay_period: 600,
        reduction_factor: 5000,
        variable_fee_control: 0,
        max_volatility_accumulator: 350_000,
        protocol_share: 0,
        base_fee_power_factor: 1, // Scaled by 10
    };
    let total_fee_high_base =
        compute_total_fee_rate(&high_base_params, 1000, 0).expect("Base fee clamping");
    assert_eq!(
        total_fee_high_base, MAX_FEE_RATE,
        "Base fee exceeding 10% must be strictly clamped to MAX_FEE_RATE (100_000_000)"
    );

    // 2. Extreme variable fee alone
    let high_var_params = DlmmParameters {
        bin_step: 100,
        base_factor: 10,
        filter_period: 30,
        decay_period: 600,
        reduction_factor: 5000,
        variable_fee_control: 1_000_000,
        max_volatility_accumulator: 2_000_000,
        protocol_share: 0,
        base_fee_power_factor: 0,
    };
    let total_fee_high_var =
        compute_total_fee_rate(&high_var_params, 100, 1_000_000).expect("Var fee clamping");
    assert_eq!(
        total_fee_high_var, MAX_FEE_RATE,
        "Variable fee exceeding 10% must be strictly clamped to MAX_FEE_RATE"
    );

    // 3. Both fees combined exceeding MAX_FEE_RATE
    let combined_params = DlmmParameters {
        bin_step: 100,
        base_factor: 60_000, // 6% base fee = 60_000_000 in 10^9
        filter_period: 30,
        decay_period: 600,
        reduction_factor: 5000,
        variable_fee_control: 100_000, // variable fee surge will add > 50_000_000
        max_volatility_accumulator: 350_000,
        protocol_share: 0,
        base_fee_power_factor: 0,
    };
    let total_combined =
        compute_total_fee_rate(&combined_params, 100, 100_000).expect("Combined clamping");
    assert_eq!(total_combined, MAX_FEE_RATE);
}

#[test]
fn challenge_dlmm_zero_liquidity_bins_and_empty_bin_traversal() {
    let bin_step = 25u16;

    // Pool configuration where active bin (0) and next bin (-1) have ZERO liquidity
    // Liquidity is located in bin -2
    let bins = vec![
        DlmmBin {
            id: -2,
            amount_x: 0,
            amount_y: 2_000_000,
            price_q64: 0,
        },
        DlmmBin {
            id: -1,
            amount_x: 0,
            amount_y: 0,
            price_q64: 0,
        }, // EMPTY
        DlmmBin {
            id: 0,
            amount_x: 0,
            amount_y: 0,
            price_q64: 0,
        }, // EMPTY
    ];

    let pool = DlmmPoolState {
        active_id: 0,
        bin_step,
        status: 0,
        parameters: DlmmParameters {
            bin_step,
            base_factor: 1000,
            filter_period: 30,
            decay_period: 600,
            reduction_factor: 5000,
            variable_fee_control: 10,
            max_volatility_accumulator: 350_000,
            protocol_share: 0,
            base_fee_power_factor: 0,
        },
        v_parameters: DlmmVariableParameters {
            volatility_accumulator: 0,
            volatility_reference: 0,
            index_reference: 0,
            last_update_timestamp: 1_000_000,
        },
    };

    // Swap X for Y starting at empty bin 0
    let quote = quote_dlmm_view(&pool, &bins, 500_000, true, Some(1_000_001))
        .expect("Should traverse empty bins and fill in bin -2");

    assert_eq!(
        quote.end_active_id, -2,
        "Must cross both empty bins (0 and -1) and end in bin -2"
    );
    assert!(quote.bins_crossed >= 2, "Must count empty bin crossings");
    assert!(quote.amount_out > 0, "Must receive liquidity from bin -2");

    // All empty test: when NO bins have liquidity, it should return BinNotFound once it walks out of range
    let all_empty_bins = vec![
        DlmmBin {
            id: -1,
            amount_x: 0,
            amount_y: 0,
            price_q64: 0,
        },
        DlmmBin {
            id: 0,
            amount_x: 0,
            amount_y: 0,
            price_q64: 0,
        },
    ];
    let empty_quote = quote_dlmm_view(&pool, &all_empty_bins, 100_000, true, None);
    assert_eq!(
        empty_quote,
        Err(DlmmError::BinNotFound(-2)),
        "Must exit cleanly with BinNotFound when walking past available empty bins"
    );
}
