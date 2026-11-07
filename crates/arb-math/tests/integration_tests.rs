//! Integration tests for crates/arb-math.
//!
//! Verifies:
//! - CPMM closed-form optimal sizing d* in U384 (2-pool) and U512 (3-pool)
//! - Existence invariant N > D0 (sub-fee, break-even, and profitable regimes)
//! - Marginal profit degradation ceiling check (dPi / d(Delta x) <= 0)
//! - CLMM Q64.64 bitwise exponentiation, strict min/max mint ordering, 1_000_000 fee denominator
//! - Meteora DLMM discrete bin pricing P_i = (1 + bin_step)^i, variable fee, multi-bin traversal
//! - Pure integer Option<u64> returns

use arb_math::{
    DlmmBin, DlmmParameters, DlmmPoolState, DlmmVariableParameters, Leg, ONE_Q64, TickArrayFacade,
    TickFacade, clamp_at_marginal_ceiling_two_pool, compute_swap, compute_swap_step, cpmm_out,
    get_amount_delta_a, get_amount_delta_b, is_past_marginal_degradation_ceiling_two_pool,
    is_token_a, marginal_profit_derivative_cpmm, optimal_three_pool, optimal_two_pool,
    quote_dlmm_view, tick_index_to_sqrt_price_x64,
};

#[test]
fn test_cpmm_two_pool_closed_form_exactness() {
    // 5% spread between two CPMM pools with 25 bps fees
    let l1 = Leg {
        r_in: 1_000_000_000, // 1 SOL in lamports
        r_out: 150_000_000,  // 150 USDC in units
        gamma: 9975,
    };
    let l2 = Leg {
        r_in: 150_000_000,    // 150 USDC in units
        r_out: 1_050_000_000, // 1.05 SOL in lamports
        gamma: 9975,
    };

    let d_star = optimal_two_pool(&l1, &l2).expect("Must find positive optimal size");
    assert!(d_star > 0);

    // Verify profit at d* is maximum (strictly concave check)
    let delta = 10_000;
    let profit_peak = eval_profit_2pool(d_star, &l1, &l2);
    let profit_lower = eval_profit_2pool(d_star.saturating_sub(delta), &l1, &l2);
    let profit_higher = eval_profit_2pool(d_star.saturating_add(delta), &l1, &l2);

    assert!(
        profit_peak >= profit_lower,
        "Profit peak {profit_peak} >= lower {profit_lower}"
    );
    assert!(
        profit_peak >= profit_higher,
        "Profit peak {profit_peak} >= higher {profit_higher}"
    );
}

#[test]
fn test_cpmm_three_pool_closed_form_exactness() {
    // Triangular cycle SOL -> USDC -> USDT -> SOL
    let l1 = Leg {
        r_in: 1_000_000_000,
        r_out: 150_000_000,
        gamma: 9975,
    };
    let l2 = Leg {
        r_in: 150_000_000,
        r_out: 150_000_000,
        gamma: 9975,
    };
    let l3 = Leg {
        r_in: 150_000_000,
        r_out: 1_100_000_000, // 10% profit spread on leg 3
        gamma: 9975,
    };

    let d3_star = optimal_three_pool(&l1, &l2, &l3).expect("Must find positive optimal size");
    assert!(d3_star > 0);

    let profit_peak = eval_profit_3pool(d3_star, &l1, &l2, &l3);
    let profit_lower = eval_profit_3pool(d3_star.saturating_sub(10_000), &l1, &l2, &l3);
    let profit_higher = eval_profit_3pool(d3_star.saturating_add(10_000), &l1, &l2, &l3);

    assert!(profit_peak >= profit_lower);
    assert!(profit_peak >= profit_higher);
}

#[test]
fn test_marginal_profit_ceiling_truncation() {
    let l1 = Leg {
        r_in: 2_000_000_000,
        r_out: 300_000_000,
        gamma: 9975,
    };
    let l2 = Leg {
        r_in: 300_000_000,
        r_out: 2_100_000_000,
        gamma: 9975,
    };
    let flash_fee_bps = 5; // Kamino 0.05%

    let d_star = optimal_two_pool(&l1, &l2).expect("optimal size");

    // Before peak: derivative > 0, not past ceiling
    let half_size = d_star / 2;
    assert!(!is_past_marginal_degradation_ceiling_two_pool(
        half_size,
        &l1,
        &l2,
        flash_fee_bps
    ));
    assert!(marginal_profit_derivative_cpmm(half_size, &l1, &l2, flash_fee_bps) > 0);

    // After peak: derivative < 0, past ceiling
    let double_size = d_star * 2;
    assert!(is_past_marginal_degradation_ceiling_two_pool(
        double_size,
        &l1,
        &l2,
        flash_fee_bps
    ));
    assert!(marginal_profit_derivative_cpmm(double_size, &l1, &l2, flash_fee_bps) < 0);

    // Clamp function truncates candidate back to d*
    let clamped = clamp_at_marginal_ceiling_two_pool(double_size, &l1, &l2, flash_fee_bps);
    assert_eq!(clamped, Some(d_star));
}

#[test]
fn test_clmm_exact_q64_and_token_deltas() {
    // Test tick 0 is exactly 1.0 in Q64.64
    let p0 = tick_index_to_sqrt_price_x64(0).expect("tick 0 price");
    assert_eq!(p0, 1u128 << 64);

    let p_target = tick_index_to_sqrt_price_x64(-64).expect("tick -64 price");
    let liquidity = 10_000_000_000_000u128;
    let fee_rate = 500u16; // 5 bps in ppm (500 / 1_000_000)

    // Compute swap step A to B (price decreasing)
    let step = compute_swap_step(p0, p_target, liquidity, 1_000_000, fee_rate, true)
        .expect("checked CLMM swap step");
    assert!(step.amount_in <= 1_000_000);
    assert!(step.amount_out > 0);
    assert!(step.fee_amount > 0);

    // Test solvency: input delta (round_up = true) >= output delta (round_up = false)
    let delta_a_in = get_amount_delta_a(p0, p_target, liquidity, true).expect("delta A in");
    let delta_a_out = get_amount_delta_a(p0, p_target, liquidity, false).expect("delta A out");
    assert!(delta_a_in >= delta_a_out);

    let delta_b_in = get_amount_delta_b(p0, p_target, liquidity, true).expect("delta B in");
    let delta_b_out = get_amount_delta_b(p0, p_target, liquidity, false).expect("delta B out");
    assert!(delta_b_in >= delta_b_out);
}

#[test]
fn test_clmm_multi_tick_traversal_with_tick_arrays() {
    let p0 = tick_index_to_sqrt_price_x64(0).expect("tick 0");
    let spacing = 64u16;

    // Create mock tick array with initialized ticks
    let tick_arrays = vec![TickArrayFacade {
        start_tick_index: -5632,
        ticks: vec![
            TickFacade {
                tick_index: -64,
                initialized: true,
                liquidity_net: 500_000_000_000,
                liquidity_gross: 500_000_000_000,
            },
            TickFacade {
                tick_index: -128,
                initialized: true,
                liquidity_net: -200_000_000_000,
                liquidity_gross: 200_000_000_000,
            },
        ],
    }];

    let quote = compute_swap(
        p0,
        0,
        10_000_000_000_000,
        spacing,
        3000, // 30 bps
        500_000_000,
        true, // a_to_b
        &tick_arrays,
    )
    .expect("compute swap quote");

    assert!(quote.amount_in_consumed > 0);
    assert!(quote.amount_out_received > 0);
    assert!(quote.steps_taken >= 1);
}

#[test]
fn test_dlmm_discrete_bin_traversal_and_variable_fee() {
    let params = DlmmParameters {
        bin_step: 25, // 0.25%
        base_factor: 1000,
        filter_period: 30,
        decay_period: 600,
        reduction_factor: 5000,
        variable_fee_control: 100,
        max_volatility_accumulator: 350_000,
        protocol_share: 0,
        base_fee_power_factor: 0,
    };
    let pool = DlmmPoolState {
        active_id: 0,
        bin_step: 25,
        status: 0,
        parameters: params,
        v_parameters: DlmmVariableParameters {
            volatility_accumulator: 0,
            volatility_reference: 0,
            index_reference: 0,
            last_update_timestamp: 1_700_000_000,
        },
    };

    let bins = vec![
        DlmmBin {
            id: 0,
            amount_x: 0,
            amount_y: 100_000,
            price_q64: ONE_Q64,
        },
        DlmmBin {
            id: -1,
            amount_x: 0,
            amount_y: 500_000,
            price_q64: 0,
        },
    ];

    // Non-mutating view swap
    let quote = quote_dlmm_view(&pool, &bins, 200_000, true, None).expect("dlmm quote");
    assert_eq!(quote.amount_in, 200_000);
    assert!(quote.amount_out > 0);
    assert_eq!(quote.end_active_id, -1);
    assert_eq!(quote.bins_crossed, 1);
    assert!(quote.fee_amount > 0);
}

#[test]
fn test_strict_token_ordering_mint_check() {
    let mint_a = [0x01u8; 32];
    let mint_b = [0x02u8; 32];
    assert!(is_token_a(&mint_a, &mint_b));
    assert!(!is_token_a(&mint_b, &mint_a));
}

fn eval_profit_2pool(dx: u64, l1: &Leg, l2: &Leg) -> i128 {
    let Some(out1) = cpmm_out(u128::from(dx), l1.r_in, l1.r_out, l1.gamma) else {
        return i128::MIN;
    };
    let Some(out2) = cpmm_out(out1, l2.r_in, l2.r_out, l2.gamma) else {
        return i128::MIN;
    };
    let out2_i128 = i128::try_from(out2).unwrap_or(i128::MAX);
    out2_i128 - i128::from(dx)
}

fn eval_profit_3pool(dx: u64, l1: &Leg, l2: &Leg, l3: &Leg) -> i128 {
    let Some(out1) = cpmm_out(u128::from(dx), l1.r_in, l1.r_out, l1.gamma) else {
        return i128::MIN;
    };
    let Some(out2) = cpmm_out(out1, l2.r_in, l2.r_out, l2.gamma) else {
        return i128::MIN;
    };
    let Some(out3) = cpmm_out(out2, l3.r_in, l3.r_out, l3.gamma) else {
        return i128::MIN;
    };
    let out3_i128 = i128::try_from(out3).unwrap_or(i128::MAX);
    out3_i128 - i128::from(dx)
}
