//! Comprehensive Property-Based Fuzzing Suite for arb-math.
//!
//! Fuzzes:
//! 1. CPMM Math (cpmm_out, optimal_two_pool, optimal_three_pool, optimal_four_pool)
//! 2. Newton-Raphson Wide Integer Square Roots (isqrt_u384, isqrt_u512, isqrt_u768)
//! 3. CLMM Tick-to-Price Bitwise Exponentiation & Traversal
//! 4. DLMM Bin Price Computation & Invariants
//! 5. Pump.fun <-> Raydium Optimal Trade Math

use arb_math::{
    MAX_SQRT_PRICE_X64, MAX_TICK_INDEX, MIN_SQRT_PRICE_X64, MIN_TICK_INDEX,
    clmm::tick_index_to_sqrt_price_x64,
    cpmm::{
        Leg, U384, U512, U768, cpmm_out, cpmm_out_fee, isqrt_u384, isqrt_u512, isqrt_u768,
        optimal_four_pool, optimal_three_pool, optimal_two_pool,
    },
    dlmm::{MAX_BIN_ID, MIN_BIN_ID, get_price_from_id},
    pump_fun_raydium::{
        DEFAULT_FLASH_LOAN_FEE_BPS, PumpFunParams, RaydiumParams, calculate_optimal_trade_integer,
        pump_fun_buy_cost, raydium_sell_output,
    },
};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    // =========================================================================
    // 1. CPMM Out Properties
    // =========================================================================

    #[test]
    fn fuzz_cpmm_out_never_panics(
        dx in any::<u128>(),
        x in any::<u128>(),
        y in any::<u128>(),
        gamma in 0u128..=10_000,
    ) {
        let res = cpmm_out(dx, x, y, gamma);
        if dx == 0 || x == 0 || y == 0 || gamma == 0 {
            prop_assert!(res.is_none());
        }
        if let Some(out) = res {
            // Output can never exceed the pool's total output reserve
            prop_assert!(out < y);
        }
    }

    #[test]
    fn fuzz_cpmm_out_monotonicity(
        dx1 in 1u64..1_000_000,
        delta in 1u64..1_000_000,
        x in 1_000_000u64..1_000_000_000,
        y in 1_000_000u64..1_000_000_000,
        gamma in 9_000u128..=10_000,
    ) {
        let dx2 = dx1 + delta;
        let out1 = cpmm_out(dx1 as u128, x as u128, y as u128, gamma);
        let out2 = cpmm_out(dx2 as u128, x as u128, y as u128, gamma);

        if let (Some(o1), Some(o2)) = (out1, out2) {
            prop_assert!(o2 >= o1, "Larger input must produce at least equal or larger output");
        }
    }

    #[test]
    fn fuzz_cpmm_out_fee_arbitrary_denominators(
        dx in 1u64..100_000_000,
        x in 100_000u64..10_000_000_000,
        y in 100_000u64..10_000_000_000,
        denom in 1u128..=1_000_000,
    ) {
        let gamma = denom / 2; // 50% fee
        let res = cpmm_out_fee(dx as u128, x as u128, y as u128, gamma, denom);
        if let Some(out) = res {
            prop_assert!(out < y as u128);
        }
    }

    // =========================================================================
    // 2. CPMM Optimal Sizing Invariants
    // =========================================================================

    #[test]
    fn fuzz_optimal_two_pool_invariants(
        r1_in in 1_000u128..=10_000_000_000_000,
        r1_out in 1_000u128..=10_000_000_000_000,
        gamma1 in 9_000u128..=10_000,
        r2_in in 1_000u128..=10_000_000_000_000,
        r2_out in 1_000u128..=10_000_000_000_000,
        gamma2 in 9_000u128..=10_000,
    ) {
        let l1 = Leg { r_in: r1_in, r_out: r1_out, gamma: gamma1 };
        let l2 = Leg { r_in: r2_in, r_out: r2_out, gamma: gamma2 };

        let opt = optimal_two_pool(&l1, &l2);

        // Compute N and D0 to verify the fundamental existence invariant
        let d = U384::from(10_000);
        let x1 = U384::from(r1_in);
        let y1 = U384::from(r1_out);
        let g1 = U384::from(gamma1);
        let y2 = U384::from(r2_in);
        let x2 = U384::from(r2_out);
        let g2 = U384::from(gamma2);

        let d0 = d * d * x1 * y2;
        let n = g1 * g2 * y1 * x2;

        if n <= d0 {
            // Unprofitable route invariant: MUST be None
            prop_assert_eq!(opt, None, "Unprofitable route N <= D0 must yield None");
        }

        if let Some(d_star) = opt {
            prop_assert!(d_star > 0, "Optimal size must be strictly positive");

            // Verify that executing d_star produces a trade without math overflow
            let out1 = cpmm_out(d_star as u128, r1_in, r1_out, gamma1);
            prop_assert!(out1.is_some());
            let out1 = out1.unwrap();
            let out2 = cpmm_out(out1, r2_in, r2_out, gamma2);
            prop_assert!(out2.is_some());
            let out2 = out2.unwrap();

            // Total output out2 must exceed input d_star (pure gross profit)
            prop_assert!(out2 > d_star as u128, "d* must yield gross arbitrage profit: out2 > d_star");
        }
    }

    #[test]
    fn fuzz_optimal_three_pool_never_panics(
        r1_in in 100u128..10_000_000_000,
        r1_out in 100u128..10_000_000_000,
        gamma1 in 9_000u128..=10_000,
        r2_in in 100u128..10_000_000_000,
        r2_out in 100u128..10_000_000_000,
        gamma2 in 9_000u128..=10_000,
        r3_in in 100u128..10_000_000_000,
        r3_out in 100u128..10_000_000_000,
        gamma3 in 9_000u128..=10_000,
    ) {
        let l1 = Leg { r_in: r1_in, r_out: r1_out, gamma: gamma1 };
        let l2 = Leg { r_in: r2_in, r_out: r2_out, gamma: gamma2 };
        let l3 = Leg { r_in: r3_in, r_out: r3_out, gamma: gamma3 };

        let opt = optimal_three_pool(&l1, &l2, &l3);
        if let Some(d_star) = opt {
            prop_assert!(d_star > 0);
        }
    }

    #[test]
    fn fuzz_optimal_four_pool_never_panics(
        r1 in 100u128..1_000_000_000,
        r2 in 100u128..1_000_000_000,
        r3 in 100u128..1_000_000_000,
        r4 in 100u128..1_000_000_000,
    ) {
        let l1 = Leg { r_in: r1, r_out: r2, gamma: 9975 };
        let l2 = Leg { r_in: r2, r_out: r3, gamma: 9975 };
        let l3 = Leg { r_in: r3, r_out: r4, gamma: 9975 };
        let l4 = Leg { r_in: r4, r_out: r1 * 2, gamma: 9975 };

        let opt = optimal_four_pool(&l1, &l2, &l3, &l4);
        if let Some(d_star) = opt {
            prop_assert!(d_star > 0);
        }
    }

    // =========================================================================
    // 3. Wide Integer Square Roots (U384, U512, U768)
    // =========================================================================

    #[test]
    fn fuzz_isqrt_u384_correctness(val_low in any::<u128>(), val_mid in any::<u128>()) {
        let val = U384::from(val_low) + (U384::from(val_mid) << 128);
        let s = isqrt_u384(val);

        // Invariant: s * s <= val
        prop_assert!(s * s <= val, "s^2 must be <= val");

        // Invariant: (s + 1)^2 > val (checked unless s is maximum)
        if s < U384::MAX {
            let s_plus_1 = s + U384::from(1);
            prop_assert!(s_plus_1 * s_plus_1 > val, "(s+1)^2 must be > val");
        }
    }

    #[test]
    fn fuzz_isqrt_u512_correctness(val_low in any::<u128>(), val_high in any::<u128>()) {
        let val = U512::from(val_low) + (U512::from(val_high) << 256);
        let s = isqrt_u512(val);

        prop_assert!(s * s <= val, "s^2 must be <= val");
        if s < U512::MAX {
            let s_plus_1 = s + U512::from(1);
            prop_assert!(s_plus_1 * s_plus_1 > val, "(s+1)^2 must be > val");
        }
    }

    #[test]
    fn fuzz_isqrt_u768_correctness(val_low in any::<u128>(), val_high in any::<u128>()) {
        let val = U768::from(val_low) + (U768::from(val_high) << 384);
        let s = isqrt_u768(val);

        prop_assert!(s * s <= val, "s^2 must be <= val");
        if s < U768::MAX {
            let s_plus_1 = s + U768::from(1);
            prop_assert!(s_plus_1 * s_plus_1 > val, "(s+1)^2 must be > val");
        }
    }

    // =========================================================================
    // 4. CLMM Tick-to-Sqrt-Price Math
    // =========================================================================

    #[test]
    fn fuzz_clmm_tick_to_sqrt_price_valid_bounds(tick in MIN_TICK_INDEX..=MAX_TICK_INDEX) {
        let res = tick_index_to_sqrt_price_x64(tick);
        prop_assert!(res.is_ok());
        let sqrt_p = res.unwrap();
        prop_assert!(sqrt_p >= MIN_SQRT_PRICE_X64, "Must be >= MIN_SQRT_PRICE_X64");
        prop_assert!(sqrt_p <= MAX_SQRT_PRICE_X64, "Must be <= MAX_SQRT_PRICE_X64");
    }

    #[test]
    fn fuzz_clmm_tick_to_sqrt_price_out_of_bounds(tick in any::<i32>()) {
        if !(MIN_TICK_INDEX..=MAX_TICK_INDEX).contains(&tick) {
            let res = tick_index_to_sqrt_price_x64(tick);
            prop_assert!(res.is_err(), "Out of bounds tick must return Err");
        }
    }

    // =========================================================================
    // 5. DLMM Bin Price Invariants
    // =========================================================================

    #[test]
    fn fuzz_dlmm_get_price_from_id_valid(
        bin_step in 1u16..=100,
        bin_id in -10_000i32..=10_000i32,
    ) {
        let res = get_price_from_id(bin_id, bin_step);
        // Under extreme bin exponentiation, it may overflow Q64.64 and return MathOverflow.
        // Invariant: must never panic, and when Ok, price must be strictly positive.
        if let Ok(price_q64) = res {
            prop_assert!(price_q64 > 0, "Price in Q64.64 must be positive");
        }
    }

    #[test]
    fn fuzz_dlmm_get_price_from_id_common_range(
        bin_step in 1u16..=25,
        bin_id in -1_000i32..=1_000i32,
    ) {
        let res = get_price_from_id(bin_id, bin_step);
        prop_assert!(res.is_ok(), "Practical DLMM pool bins must always be computable");
        prop_assert!(res.unwrap() > 0);
    }

    #[test]
    fn fuzz_dlmm_get_price_from_id_out_of_bounds(bin_id in any::<i32>()) {
        if !(MIN_BIN_ID..=MAX_BIN_ID).contains(&bin_id) {
            let res = get_price_from_id(bin_id, 10);
            prop_assert!(res.is_err(), "Out of range bin ID must return DlmmError");
        }
    }

    // =========================================================================
    // 6. Pump.fun <-> Raydium Math
    // =========================================================================

    #[test]
    fn fuzz_pump_fun_buy_cost_bounds(
        reserve in 1_000u128..1_000_000_000_000,
        supply in 1_000u128..1_000_000_000_000,
        delta_t in any::<u128>(),
    ) {
        let cost = pump_fun_buy_cost(reserve, supply, delta_t);
        if delta_t == 0 {
            prop_assert_eq!(cost, Some(0));
        } else if delta_t >= supply {
            prop_assert_eq!(cost, None);
        } else if let Some(c) = cost {
            // Buying delta_t < supply must have positive cost when reserve * delta_t > 0
            prop_assert!(c >= (reserve * delta_t) / (supply - delta_t));
        }
    }

    #[test]
    fn fuzz_raydium_sell_output_bounds(
        sol_reserve in 1_000u128..1_000_000_000_000,
        token_reserve in 1_000u128..1_000_000_000_000,
        delta_t in any::<u128>(),
        fee_bps in 0u64..=10_000,
    ) {
        let out = raydium_sell_output(sol_reserve, token_reserve, delta_t, fee_bps);
        if let Some(s) = out {
            if delta_t == 0 {
                prop_assert_eq!(s, 0);
            } else {
                // Sell output cannot exceed sol_reserve
                prop_assert!(s <= sol_reserve);
            }
        }
    }

    #[test]
    fn fuzz_calculate_optimal_trade_fails_closed_when_unprofitable(
        pump_res in 1_000u128..1_000_000_000_000,
        pump_sup in 1_000u128..1_000_000_000_000,
        ray_tok in 1_000u128..1_000_000_000_000,
        ray_sol in 1_000u128..1_000_000_000_000,
    ) {
        let pump = PumpFunParams { reserve: pump_res, supply: pump_sup };
        let ray = RaydiumParams { token_reserve: ray_tok, sol_reserve: ray_sol, fee_bps: 25 };

        // calculate_optimal_trade_integer must fail closed on unprofitable trades
        let trade = calculate_optimal_trade_integer(pump, ray, DEFAULT_FLASH_LOAN_FEE_BPS, 1_000_000);
        if let Some(t) = trade {
            prop_assert!(t.is_profitable(), "Optimal trade integer MUST be profitable");
            prop_assert!(t.delta_t > 0, "Optimal trade delta_t must be > 0");
        }
    }
}
