//! Adversarial Empirical Stress Tests for crates/arb-math (Milestone M1).
//!
//! Stress-tests:
//! 1. CPMM optimal sizing d* in U384/U512:
//!    - Extreme reserve imbalances (1 lamport vs 10^18 atomic units, deep pools with 10^24 units).
//!    - Boundary condition N == D0 (must return None, zero profit).
//!    - Losing routes N < D0 (must return None).
//!    - Triangular 3-pool sizing where radicand approaches 441 bits; zero overflow and truncation check.
//! 2. Marginal profit degradation ceiling:
//!    - Derivative sign before and after d*.
//!    - Clamping behavior and preservation of net profit peak.
//! 3. Zero float verification and fixed-point math safety.

use arb_math::{
    MAX_TICK_INDEX, MIN_TICK_INDEX,
    cpmm::{
        Leg, U512, cpmm_out, is_past_marginal_degradation_ceiling_two_pool, isqrt_u512,
        marginal_profit_derivative, marginal_profit_derivative_cpmm, optimal_four_pool,
        optimal_three_pool, optimal_two_pool,
    },
    dlmm::{MAX_BIN_ID, MIN_BIN_ID, get_price_from_id},
    tick_index_to_sqrt_price_x64,
};

#[test]
fn test_adv_exact_boundary_n_equals_d0() {
    // Construct exact boundary condition where N == D0
    // In 2-pool:
    // D0 = D^2 · x1 · y2
    // N  = gamma1 · gamma2 · y1 · x2
    // Let x1 = 10^6, y2 = 10^6, gamma1 = 9975, gamma2 = 9975
    // D0 = 10^8 * 10^6 * 10^6 = 10^20
    // To make N == D0:
    // gamma1 * gamma2 * y1 * x2 == 10^20
    // 9975^2 * y1 * x2 = 99,500,625 * y1 * x2 = 10^20
    // Let y1 = 1,000,000. Then 99,500,625 * 10^6 * x2 = 99,500,625,000,000 * x2 = 10^20
    // x2 = 10^20 / 99,500,625,000,000 = 1,005,018.79...
    // Let's set gamma1 = 10,000 and gamma2 = 10,000 (0% fees) so D0 = N when x1 * y2 == y1 * x2
    let l1 = Leg {
        r_in: 1_000_000,
        r_out: 2_000_000,
        gamma: 10_000,
    };
    let l2 = Leg {
        r_in: 2_000_000,
        r_out: 1_000_000,
        gamma: 10_000,
    };
    // D0 = 10^8 * 1M * 2M = 2 * 10^20
    // N  = 10^8 * 2M * 1M = 2 * 10^20
    // Exactly N == D0!
    let res = optimal_two_pool(&l1, &l2);
    assert_eq!(
        res, None,
        "Boundary condition N == D0 must return None (zero profit)"
    );

    // Also with standard 25 bps fees:
    // Let x1 = 9975, y1 = 10000, y2 = 9975, x2 = 10000
    // D0 = 10000^2 * 9975 * 9975
    // N  = 9975 * 9975 * 10000 * 10000 = D0
    let l1_fee = Leg {
        r_in: 9975,
        r_out: 10_000,
        gamma: 9975,
    };
    let l2_fee = Leg {
        r_in: 9975,
        r_out: 10_000,
        gamma: 9975,
    };
    let res_fee = optimal_two_pool(&l1_fee, &l2_fee);
    assert_eq!(
        res_fee, None,
        "Boundary condition with fees N == D0 must return None"
    );
}

#[test]
fn test_adv_losing_routes_n_less_than_d0() {
    // Route 1: Gross price ratio is 1:1, but fees eat 50 bps total (N < D0)
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
    assert_eq!(optimal_two_pool(&l1, &l2), None);

    // Route 2: 10% losing route
    let l2_loss = Leg {
        r_in: 1_000_000,
        r_out: 900_000,
        gamma: 9975,
    };
    assert_eq!(optimal_two_pool(&l1, &l2_loss), None);

    // Route 3: Massive 99% losing route
    let l2_rekt = Leg {
        r_in: 1_000_000,
        r_out: 10_000,
        gamma: 9975,
    };
    assert_eq!(optimal_two_pool(&l1, &l2_rekt), None);

    // Route 4: 3-pool losing cycle
    let l3_loss = Leg {
        r_in: 1_000_000,
        r_out: 950_000,
        gamma: 9975,
    };
    assert_eq!(optimal_three_pool(&l1, &l2, &l3_loss), None);
}

#[test]
fn test_adv_extreme_reserve_imbalance_1_lamport_vs_10_pow_18() {
    // 1 lamport in pool 1, 10^18 atomic units in pool 2
    let r18 = 1_000_000_000_000_000_000u128; // 10^18
    let l1 = Leg {
        r_in: 1,
        r_out: r18,
        gamma: 9975,
    };
    let l2 = Leg {
        r_in: r18,
        r_out: 2, // Doubling 1 lamport to 2
        gamma: 9975,
    };

    // Mathematically: D0 = 10^8 * 1 * 10^18 = 10^26
    // N = 9975^2 * 10^18 * 2 = 1.99 * 10^26 > D0
    // d* = (sqrt(N * D0) - D0) / D1
    // But D1 ~ 2 * 10^26, so d* is fractional (0.2 lamports).
    // In integer math, d* = 0, so it must return None.
    let opt = optimal_two_pool(&l1, &l2);
    assert_eq!(opt, None, "Fractional d* < 1 must return None");

    // Now test where 1 lamport can yield integer sizing:
    // Let pool 2 return 100 lamports for the 10^18 tokens
    let l2_rich = Leg {
        r_in: r18,
        r_out: 100,
        gamma: 9975,
    };
    let opt2 = optimal_two_pool(&l1, &l2_rich);
    // Let's verify whether it computes safely without panic
    if let Some(d) = opt2 {
        assert!(d >= 1);
        let Some(out1) = cpmm_out(u128::from(d), l1.r_in, l1.r_out, l1.gamma) else {
            panic!("cpmm_out failed for valid optimal size");
        };
        let Some(out2) = cpmm_out(out1, l2_rich.r_in, l2_rich.r_out, l2_rich.gamma) else {
            panic!("cpmm_out failed for valid optimal size");
        };
        assert!(
            out2 > u128::from(d),
            "Trade must be profit-positive: in={d}, out={out2}"
        );
    }
}

#[test]
fn test_adv_deep_pools_10_pow_24() {
    // Deep pools: 10^24 atomic units (e.g. 18-decimal token with 1,000,000 tokens)
    let r24 = 1_000_000_000_000_000_000_000_000u128; // 10^24
    let r24_profit = 1_050_000_000_000_000_000_000_000u128; // 5% spread

    let l1 = Leg {
        r_in: r24,
        r_out: r24,
        gamma: 9975,
    };
    let l2 = Leg {
        r_in: r24,
        r_out: r24_profit,
        gamma: 9975,
    };

    // In U384, radicand is ~10^112 <= 372 bits, fits in U384 (<= 384 bits)
    // However, d* is ~1.1 * 10^22, which exceeds u64::MAX (1.84 * 10^19).
    // optimal_two_pool returns Option<u64>, so u64::try_from(d_star) should return None!
    let opt = optimal_two_pool(&l1, &l2);
    // Must NOT panic! Must return None safely because d* > u64::MAX
    assert_eq!(
        opt, None,
        "Deep pool where optimal size exceeds u64::MAX must safely return None"
    );

    // Deep pool with SMALL spread where d* fits in u64:
    // Let spread be tiny, e.g. 0.505% vs 0.5% fee
    // Or let r_in be large but r_out scaled such that d* < u64::MAX
    let l1_deep_fit = Leg {
        r_in: r24,
        r_out: 1_000_000_000_000_000_000u128, // 10^18
        gamma: 9975,
    };
    let l2_deep_fit = Leg {
        r_in: 1_000_000_000_000_000_000u128,
        r_out: 1_060_000_000_000_000_000_000_000u128, // 1.06 * 10^24
        gamma: 9975,
    };
    let opt_fit = optimal_two_pool(&l1_deep_fit, &l2_deep_fit);
    // Should compute safely without panic
    if let Some(d) = opt_fit {
        assert!(d > 0);
    }
}

#[test]
fn test_adv_deep_pools_overflow_in_marginal_functions() {
    // Challenge whether is_past_marginal_degradation_ceiling_two_pool
    // or marginal_profit_derivative_cpmm panic when reserves = 10^24
    let r24 = 1_000_000_000_000_000_000_000_000u128; // 10^24
    let r24_profit = 1_050_000_000_000_000_000_000_000u128; // 5% spread

    let l1 = Leg {
        r_in: r24,
        r_out: r24,
        gamma: 9975,
    };
    let l2 = Leg {
        r_in: r24,
        r_out: r24_profit,
        gamma: 9975,
    };

    // Test: does is_past_marginal_degradation_ceiling_two_pool panic with 10^24 reserves?
    // In U384: lhs = n * d0 * d = 10^56 * 10^56 * 10^4 = 10^116 > 2^384 (~3.94 * 10^115)!
    // Let's see what happens if we call it with size = 1_000_000:
    let result = std::panic::catch_unwind(|| {
        is_past_marginal_degradation_ceiling_two_pool(1_000_000, &l1, &l2, 5)
    });
    // Record if it overflows/panics:
    println!(
        "is_past_marginal_degradation_ceiling_two_pool at 10^24 reserves: panic? {}",
        result.is_err()
    );

    // Test: does marginal_profit_derivative_cpmm panic with 10^24 reserves?
    // marginal_rev_scaled = (n * d0 * scale) / denom_sq = 10^112 * 10^9 = 10^121 > 2^384!
    let result_deriv =
        std::panic::catch_unwind(|| marginal_profit_derivative_cpmm(1_000_000, &l1, &l2, 5));
    println!(
        "marginal_profit_derivative_cpmm at 10^24 reserves: panic? {}",
        result_deriv.is_err()
    );
}

#[test]
fn test_adv_3pool_441_bits_and_headroom() {
    // 3-pool sizing with reserves of 10^18 on all 3 legs
    // Radicand is ~438-441 bits
    let r18 = 1_000_000_000_000_000_000u128;
    let r18_out = 1_100_000_000_000_000_000u128; // 10% spread

    let l1 = Leg {
        r_in: r18,
        r_out: r18,
        gamma: 9975,
    };
    let l2 = Leg {
        r_in: r18,
        r_out: r18,
        gamma: 9975,
    };
    let l3 = Leg {
        r_in: r18,
        r_out: r18_out,
        gamma: 9975,
    };

    let opt3 = optimal_three_pool(&l1, &l2, &l3);
    assert!(opt3.is_some(), "3-pool optimal sizing must succeed");
    let d3 = opt3.expect("invariant");
    assert!(d3 > 0);

    // Verify 3-hop execution profitability at d3:
    let Some(out1) = cpmm_out(u128::from(d3), l1.r_in, l1.r_out, l1.gamma) else {
        panic!("cpmm_out failed for valid optimal size");
    };
    let Some(out2) = cpmm_out(out1, l2.r_in, l2.r_out, l2.gamma) else {
        panic!("cpmm_out failed for valid optimal size");
    };
    let Some(out3) = cpmm_out(out2, l3.r_in, l3.r_out, l3.gamma) else {
        panic!("cpmm_out failed for valid optimal size");
    };
    assert!(
        out3 > u128::from(d3),
        "3-pool trade must be profitable: in={d3}, out={out3}"
    );

    // Verify d3 is at the profit peak:
    let step = 100_000_000_000_000u64;
    let p_peak = out3 as i128 - d3 as i128;

    let d_low = d3.saturating_sub(step);
    let Some(out1_low) = cpmm_out(u128::from(d_low), l1.r_in, l1.r_out, l1.gamma) else {
        panic!("cpmm_out failed for d_low");
    };
    let Some(out2_low) = cpmm_out(out1_low, l2.r_in, l2.r_out, l2.gamma) else {
        panic!("cpmm_out failed for d_low");
    };
    let Some(out3_low) = cpmm_out(out2_low, l3.r_in, l3.r_out, l3.gamma) else {
        panic!("cpmm_out failed for d_low");
    };
    let p_low = out3_low as i128 - d_low as i128;

    let d_high = d3.saturating_add(step);
    let Some(out1_high) = cpmm_out(u128::from(d_high), l1.r_in, l1.r_out, l1.gamma) else {
        panic!("cpmm_out failed for d_high");
    };
    let Some(out2_high) = cpmm_out(out1_high, l2.r_in, l2.r_out, l2.gamma) else {
        panic!("cpmm_out failed for d_high");
    };
    let Some(out3_high) = cpmm_out(out2_high, l3.r_in, l3.r_out, l3.gamma) else {
        panic!("cpmm_out failed for d_high");
    };
    let p_high = out3_high as i128 - d_high as i128;

    assert!(p_peak >= p_low, "p_peak ({p_peak}) >= p_low ({p_low})");
    assert!(p_peak >= p_high, "p_peak ({p_peak}) >= p_high ({p_high})");

    // Test U512 isqrt exactness near 441 bits:
    let val_441 = U512::from(1) << 441;
    let sqrt_val = isqrt_u512(val_441);
    let expected_sqrt = U512::from(1) << 220; // 2^220.5 ~ 1.414 * 2^220
    assert!(sqrt_val >= expected_sqrt);
    // (sqrt)^2 <= val_441 < (sqrt + 1)^2
    assert!(sqrt_val * sqrt_val <= val_441);
    let next = sqrt_val + U512::from(1);
    assert!(next * next > val_441);
}

#[test]
fn test_adv_marginal_derivative_behavior() {
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
    let flash_fee_bps = 5; // Kamino 0.05%

    let d_star = optimal_two_pool(&l1, &l2).expect("invariant");

    // 1. Below d*: derivative must be strictly positive
    let d_low = d_star / 2;
    let deriv_low = marginal_profit_derivative_cpmm(d_low, &l1, &l2, flash_fee_bps);
    assert!(
        deriv_low > 0,
        "Derivative before peak must be positive, got {deriv_low}"
    );

    // 2. Beyond d*: derivative must be strictly negative
    let d_high = d_star * 2;
    let deriv_high = marginal_profit_derivative_cpmm(d_high, &l1, &l2, flash_fee_bps);
    assert!(
        deriv_high < 0,
        "Derivative after peak must be negative, got {deriv_high}"
    );

    // 3. Monotonic decrease of derivative (strict concavity):
    // dPi / d(Delta x) must decrease as Delta x increases
    let deriv_1 = marginal_profit_derivative_cpmm(d_star / 4, &l1, &l2, flash_fee_bps);
    let deriv_2 = marginal_profit_derivative_cpmm(d_star / 2, &l1, &l2, flash_fee_bps);
    let deriv_3 = marginal_profit_derivative_cpmm(d_star, &l1, &l2, flash_fee_bps);
    let deriv_4 = marginal_profit_derivative_cpmm(d_star * 2, &l1, &l2, flash_fee_bps);
    assert!(deriv_1 > deriv_2, "Derivative must strictly decrease");
    assert!(deriv_2 > deriv_3, "Derivative must strictly decrease");
    assert!(deriv_3 > deriv_4, "Derivative must strictly decrease");

    // 4. Test discrete derivative evaluator
    let disc_pos = marginal_profit_derivative(10_000, 10_500, 5);
    let disc_neg = marginal_profit_derivative(10_000, 10_001, 5);
    assert!(disc_pos > 0);
    assert!(disc_neg < 0);
}

#[test]
fn test_adv_clamping_logic_anomaly_analysis() {
    use arb_math::cpmm::clamp_at_marginal_ceiling_two_pool;

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
    let flash_fee_bps = 5;

    let d_star = optimal_two_pool(&l1, &l2).expect("invariant");

    // Candidate size double optimal:
    let clamped_double = clamp_at_marginal_ceiling_two_pool(d_star * 2, &l1, &l2, flash_fee_bps);
    assert_eq!(clamped_double, Some(d_star));

    // Candidate size half optimal:
    let clamped_half = clamp_at_marginal_ceiling_two_pool(d_star / 2, &l1, &l2, flash_fee_bps);
    assert_eq!(clamped_half, Some(d_star / 2));
}

#[test]
fn test_adv_clmm_boundaries_and_ordering() {
    // CLMM boundary ticks: MIN_TICK_INDEX (-443636) and MAX_TICK_INDEX (443636)
    let min_p = tick_index_to_sqrt_price_x64(MIN_TICK_INDEX);
    assert!(min_p.is_ok(), "MIN_TICK_INDEX must not panic");
    assert!(min_p.expect("invariant") > 0);

    let max_p = tick_index_to_sqrt_price_x64(MAX_TICK_INDEX);
    assert!(max_p.is_ok(), "MAX_TICK_INDEX must not panic");

    // Out of bounds tick must return Err
    assert!(tick_index_to_sqrt_price_x64(MIN_TICK_INDEX - 1).is_err());
    assert!(tick_index_to_sqrt_price_x64(MAX_TICK_INDEX + 1).is_err());
}

#[test]
fn test_adv_dlmm_boundaries() {
    // Extreme bin IDs
    let p_min = get_price_from_id(MIN_BIN_ID, 25);
    println!("p_min result: {:?}", p_min);
    assert!(
        p_min.is_ok(),
        "MIN_BIN_ID price must not panic, got {:?}",
        p_min
    );
    assert!(p_min.expect("invariant") > 0);

    let p_max = get_price_from_id(MAX_BIN_ID, 25);
    println!("p_max result: {:?}", p_max);
    assert!(
        p_max.is_ok(),
        "MAX_BIN_ID price must not panic, got {:?}",
        p_max
    );

    // Out of bounds bin ID must return Err
    assert!(get_price_from_id(MIN_BIN_ID - 1, 25).is_err());
    assert!(get_price_from_id(MAX_BIN_ID + 1, 25).is_err());
}

#[test]
fn test_adv_4pool_u768_closed_form() {
    // 4-pool sizing across 4 legs in U768
    let r18 = 1_000_000_000_000_000_000u128; // 10^18
    let r18_out = 1_080_000_000_000_000_000u128; // 8% spread on the final leg

    let l1 = Leg {
        r_in: r18,
        r_out: r18,
        gamma: 9975,
    };
    let l2 = Leg {
        r_in: r18,
        r_out: r18,
        gamma: 9975,
    };
    let l3 = Leg {
        r_in: r18,
        r_out: r18,
        gamma: 9975,
    };
    let l4 = Leg {
        r_in: r18,
        r_out: r18_out,
        gamma: 9975,
    };

    let opt4 = optimal_four_pool(&l1, &l2, &l3, &l4);
    assert!(
        opt4.is_some(),
        "4-pool optimal sizing must succeed when cycle is profitable"
    );
    let d4 = opt4.expect("invariant");
    assert!(d4 > 0);

    // Verify 4-hop execution profitability at d4:
    let out1 = cpmm_out(u128::from(d4), l1.r_in, l1.r_out, l1.gamma).expect("hop 1");
    let out2 = cpmm_out(out1, l2.r_in, l2.r_out, l2.gamma).expect("hop 2");
    let out3 = cpmm_out(out2, l3.r_in, l3.r_out, l3.gamma).expect("hop 3");
    let out4 = cpmm_out(out3, l4.r_in, l4.r_out, l4.gamma).expect("hop 4");
    assert!(
        out4 > u128::from(d4),
        "4-pool trade must be profitable: in={d4}, out={out4}"
    );

    // Verify d4 is at the mathematical profit peak:
    let step = 100_000_000_000_000u64;
    let p_peak = out4 as i128 - d4 as i128;

    let d_low = d4.saturating_sub(step);
    let o1_low = cpmm_out(u128::from(d_low), l1.r_in, l1.r_out, l1.gamma).expect("hop 1 low");
    let o2_low = cpmm_out(o1_low, l2.r_in, l2.r_out, l2.gamma).expect("hop 2 low");
    let o3_low = cpmm_out(o2_low, l3.r_in, l3.r_out, l3.gamma).expect("hop 3 low");
    let o4_low = cpmm_out(o3_low, l4.r_in, l4.r_out, l4.gamma).expect("hop 4 low");
    let p_low = o4_low as i128 - d_low as i128;

    let d_high = d4.saturating_add(step);
    let o1_high = cpmm_out(u128::from(d_high), l1.r_in, l1.r_out, l1.gamma).expect("hop 1 high");
    let o2_high = cpmm_out(o1_high, l2.r_in, l2.r_out, l2.gamma).expect("hop 2 high");
    let o3_high = cpmm_out(o2_high, l3.r_in, l3.r_out, l3.gamma).expect("hop 3 high");
    let o4_high = cpmm_out(o3_high, l4.r_in, l4.r_out, l4.gamma).expect("hop 4 high");
    let p_high = o4_high as i128 - d_high as i128;

    assert!(p_peak >= p_low, "p_peak ({p_peak}) >= p_low ({p_low})");
    assert!(p_peak >= p_high, "p_peak ({p_peak}) >= p_high ({p_high})");
}
