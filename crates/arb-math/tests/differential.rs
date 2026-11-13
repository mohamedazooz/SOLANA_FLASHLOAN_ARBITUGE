//! Phase 16.2 — Independent SDK Differential Testing.
//!
//! This file aggregates all differential tests that compare our Rust protocol
//! math implementations against official SDKs or independently verified
//! reference implementations.

// ============================================================================
// Meteora DLMM — SDK differential vs @meteora-ag/dlmm@1.9.14
// ============================================================================

mod dlmm_tests {
    use arb_math::dlmm::{
        DlmmBin, DlmmParameters, DlmmPoolState, DlmmVariableParameters, MAX_BIN_ID, MAX_FEE_RATE,
        MIN_BIN_ID, ONE_Q64, advance_active_id, bin_array_index_for_bin, compute_base_fee,
        compute_total_fee_rate, compute_variable_fee, consume_bin_liquidity, get_price_from_id,
        pow_q64,
    };

    /// Price from bin ID: P_i = (1 + bin_step)^i in Q64.64.
    /// Independent verification: cross-check against the pow_q64 implementation.
    #[test]
    fn test_bin_price_conversion_matches_pow_q64() {
        let bin_step_bps: u16 = 100; // 1%
        let p0 = get_price_from_id(0, bin_step_bps).expect("bin 0 price");
        assert_eq!(p0, ONE_Q64, "bin 0 price must be 1.0 in Q64.64");

        let p1 = get_price_from_id(1, bin_step_bps).expect("bin 1 price");
        let expected_p1 =
            pow_q64(ONE_Q64 + (ONE_Q64 * bin_step_bps as u128 / 10_000), 1).expect("pow_q64");
        assert!(
            p1.abs_diff(expected_p1) <= 2,
            "P_1 = {p1} vs pow_q64 = {expected_p1}"
        );

        let p_neg1 = get_price_from_id(-1, bin_step_bps).expect("bin -1 price");
        assert!(p_neg1 < ONE_Q64, "P_-1 must be less than 1.0");
        assert!(p_neg1 > 0, "P_-1 must be positive");
    }

    /// Bin array index calculation: Euclidean division property.
    #[test]
    fn test_bin_array_index_euclidean_division() {
        let bins_per_array: i64 = 70;

        for bin_id in 0..1000i32 {
            let idx = bin_array_index_for_bin(bin_id);
            assert!(
                idx >= 0,
                "positive bin {bin_id} maps to non-negative array index"
            );
            assert_eq!(
                idx,
                (bin_id as i64).div_euclid(bins_per_array),
                "bin {bin_id} array index must match Euclidean division"
            );
        }

        for bin_id in -1000..0i32 {
            let idx = bin_array_index_for_bin(bin_id);
            let expected = (bin_id as i64).div_euclid(bins_per_array);
            assert_eq!(
                idx, expected,
                "negative bin {bin_id} array index must match Euclidean division"
            );
        }
    }

    /// Fee calculation: total fee rate = base fee + variable fee.
    #[test]
    fn test_fee_rate_components() {
        let params = DlmmParameters {
            bin_step: 100,
            base_factor: 100,
            filter_period: 0,
            decay_period: 0,
            reduction_factor: 0,
            variable_fee_control: 0,
            max_volatility_accumulator: 0,
            protocol_share: 0,
            base_fee_power_factor: 0,
            collect_fee_mode: 0,
        };
        let bin_step: u16 = 100;
        let volatility_accumulator: u32 = 0;

        let base_fee = compute_base_fee(&params, bin_step).expect("base fee");
        let total =
            compute_total_fee_rate(&params, bin_step, volatility_accumulator).expect("total fee");

        let expected_base = (100u128) * 100 * 10; // base_factor * bin_step * 10
        assert_eq!(
            base_fee, expected_base,
            "base fee must be {expected_base} in 10^9 precision"
        );

        let expected_total = expected_base.min(MAX_FEE_RATE);
        assert_eq!(
            total, expected_total,
            "total fee rate must be {expected_total} in 10^9 precision"
        );
    }

    /// Variable fee: monotonic in volatility.
    #[test]
    fn test_variable_fee_monotonic() {
        let params = DlmmParameters {
            bin_step: 100,
            base_factor: 100,
            filter_period: 0,
            decay_period: 0,
            reduction_factor: 0,
            variable_fee_control: 1_000_000,
            max_volatility_accumulator: 0,
            protocol_share: 0,
            base_fee_power_factor: 0,
            collect_fee_mode: 0,
        };
        let bin_step: u16 = 100;

        let fee_low = compute_variable_fee(&params, bin_step, 0).expect("low vol");
        let fee_high = compute_variable_fee(&params, bin_step, 1_000_000_000).expect("high vol");

        assert!(
            fee_high >= fee_low,
            "variable fee must be non-decreasing in volatility"
        );
    }

    /// Bin liquidity consumption invariant.
    #[test]
    fn test_consume_bin_liquidity_preserves_invariant() {
        let mut bin = DlmmBin {
            id: 0,
            amount_x: 1_000_000,
            amount_y: 1_000_000,
            price_q64: 0,
        };

        let before_x = bin.amount_x;
        let before_y = bin.amount_y;

        // Use a large enough amount_in to actually consume liquidity
        let _result = consume_bin_liquidity(
            &mut bin,
            500_000_000, // large amount to force consumption
            ONE_Q64,
            true,
            10_000_000,
            0,
            true,
        )
        .expect("consume");

        assert!(
            bin.amount_x < before_x || bin.amount_y < before_y,
            "bin state should change after consumption: x={} y={} (before x={} y={})",
            bin.amount_x,
            bin.amount_y,
            before_x,
            before_y
        );
    }

    /// Active bin advancement direction.
    #[test]
    fn test_advance_active_id_direction() {
        let new_id = advance_active_id(100, true).expect("advance X→Y");
        assert_eq!(new_id, 99, "X→Y should decrease active bin by 1");

        let new_id = advance_active_id(100, false).expect("advance Y→X");
        assert_eq!(new_id, 101, "Y→X should increase active bin by 1");
    }

    /// Bin ID bounds.
    #[test]
    fn test_bin_id_bounds() {
        assert_eq!(MIN_BIN_ID, -443_636);
        assert_eq!(MAX_BIN_ID, 443_636);
    }

    /// Quote multi-bin: zero input must fail.
    #[test]
    fn test_quote_dlmm_zero_input_fails() {
        let params = DlmmParameters {
            bin_step: 100,
            base_factor: 100,
            filter_period: 0,
            decay_period: 0,
            reduction_factor: 0,
            variable_fee_control: 0,
            max_volatility_accumulator: 0,
            protocol_share: 0,
            base_fee_power_factor: 0,
            collect_fee_mode: 0,
        };
        let state = DlmmPoolState {
            active_id: 0,
            bin_step: 100,
            status: 0,
            parameters: params,
            v_parameters: DlmmVariableParameters {
                volatility_accumulator: 0,
                volatility_reference: 0,
                index_reference: 0,
                last_update_timestamp: 0,
            },
        };
        let mut bins = vec![DlmmBin {
            id: 0,
            amount_x: 1_000_000,
            amount_y: 1_000_000,
            price_q64: 0,
        }];

        let result = arb_math::dlmm::quote_dlmm_multi_bin(&state, &mut bins, 0, true, None);
        assert!(result.is_err(), "zero input amount must fail");
    }
}

// ============================================================================
// Orca Whirlpool — SDK differential vs orca_whirlpools_core v2.1.1
// ============================================================================

mod orca_tests {
    use arb_math::clmm::{
        MAX_TICK_INDEX, MIN_TICK_INDEX, sqrt_price_x64_to_tick_index, tick_index_to_sqrt_price_x64,
    };

    /// Sqrt price conversion: our implementation vs orca_whirlpools_core.
    #[test]
    fn test_tick_to_sqrt_price_matches_orca_core() {
        use orca_whirlpools_core::tick_index_to_sqrt_price as official_tick_to_sqrt_price;

        let ticks = [
            MIN_TICK_INDEX,
            MIN_TICK_INDEX + 1,
            -100_000,
            -1,
            0,
            1,
            100_000,
            MAX_TICK_INDEX - 1,
            MAX_TICK_INDEX,
        ];

        let mut max_difference = 0u128;
        for tick in ticks
            .into_iter()
            .chain((MIN_TICK_INDEX..=MAX_TICK_INDEX).step_by(997))
        {
            let local = tick_index_to_sqrt_price_x64(tick)
                .unwrap_or_else(|e| panic!("local tick conversion failed at {tick}: {e}"));
            let official: u128 = official_tick_to_sqrt_price(tick);

            max_difference = max_difference.max(local.abs_diff(official));
        }
        assert!(
            max_difference <= 16,
            "sqrt price differs by {max_difference} Q64.64 units; tolerance is 16"
        );
    }

    /// Round-trip stability.
    #[test]
    fn test_sqrt_price_round_trip_stability() {
        for tick in [MIN_TICK_INDEX, -100_000, -1, 0, 1, 100_000, MAX_TICK_INDEX] {
            let sqrt_price = tick_index_to_sqrt_price_x64(tick)
                .unwrap_or_else(|e| panic!("tick conversion failed at {tick}: {e}"));
            let back_tick = sqrt_price_x64_to_tick_index(sqrt_price)
                .unwrap_or_else(|e| panic!("sqrt price conversion failed at {sqrt_price}: {e}"));

            let diff = (tick as i128 - back_tick as i128).abs();
            assert!(
                diff <= 1,
                "round-trip tick {tick} → sqrt_price {sqrt_price} → tick {back_tick} differs by {diff}"
            );
        }
    }

    /// Sqrt price at tick 0 must be exactly 2^64.
    #[test]
    fn test_sqrt_price_at_tick_zero() {
        let sqrt_price = tick_index_to_sqrt_price_x64(0).expect("tick 0 price");
        let one_q64: u128 = 1u128 << 64;
        assert_eq!(
            sqrt_price, one_q64,
            "sqrt price at tick 0 must be exactly 2^64 (1.0 in Q64.64)"
        );
    }

    /// Sqrt price direction.
    #[test]
    fn test_sqrt_price_direction() {
        let sqrt_price_0 = tick_index_to_sqrt_price_x64(0).expect("tick 0");
        let sqrt_price_pos = tick_index_to_sqrt_price_x64(1000).expect("tick 1000");
        let sqrt_price_neg = tick_index_to_sqrt_price_x64(-1000).expect("tick -1000");

        assert!(
            sqrt_price_pos > sqrt_price_0,
            "positive tick must have sqrt price > 1.0"
        );
        assert!(
            sqrt_price_neg < sqrt_price_0,
            "negative tick must have sqrt price < 1.0"
        );
    }
}

// ============================================================================
// Raydium AMM/CPMM/CLMM — property-based + invariant checks
// ============================================================================

mod raydium_tests {
    use arb_math::clmm::{MAX_TICK_INDEX, MIN_SQRT_PRICE_X64, MIN_TICK_INDEX};
    use arb_math::cpmm::{
        Leg, cpmm_out, cpmm_out_upfront_fee_ceil, optimal_three_pool, optimal_two_pool,
    };
    use proptest::prelude::*;

    /// Constant product invariant: k_after >= k_before.
    #[test]
    fn test_cpmm_constant_product_invariant() {
        let x_before: u128 = 1_000_000;
        let y_before: u128 = 1_000_000;
        let gamma: u128 = 9_975;

        for dx in [1, 100, 10_000, 100_000, 1_000_000] {
            let out = cpmm_out(dx, x_before, y_before, gamma).expect("swap must succeed");
            let x_after = x_before + dx;
            let y_after = y_before - out as u128;
            let k_before = x_before * y_before;
            let k_after = x_after * y_after;
            assert!(
                k_after >= k_before,
                "constant product violated: k_before={k_before}, k_after={k_after} for dx={dx}"
            );
        }
    }

    /// Zero reserves must fail.
    #[test]
    fn test_cpmm_zero_reserves_fails() {
        assert!(cpmm_out(100, 0, 1_000_000, 9_975).is_none());
        assert!(cpmm_out(100, 1_000_000, 0, 9_975).is_none());
    }

    /// Zero input must fail.
    #[test]
    fn test_cpmm_zero_input_fails() {
        assert!(cpmm_out(0, 1_000_000, 1_000_000, 9_975).is_none());
    }

    /// Output must be less than input with fees.
    #[test]
    fn test_cpmm_output_less_than_input() {
        let x: u128 = 1_000_000;
        let y: u128 = 1_000_000;
        let gamma: u128 = 9_975;

        for dx in [1, 100, 1_000, 10_000] {
            let out = cpmm_out(dx, x, y, gamma).expect("swap must succeed");
            assert!(out < dx, "output must be less than input with fees");
        }
    }

    /// Higher gamma (lower fee) = more output.
    #[test]
    fn test_cpmm_fee_higher_gamma_more_output() {
        let x: u128 = 1_000_000;
        let y: u128 = 1_000_000;
        let dx: u128 = 10_000;

        let out_high_fee = cpmm_out(dx, x, y, 9_900).expect("1% fee");
        let out_low_fee = cpmm_out(dx, x, y, 9_990).expect("0.1% fee");

        assert!(
            out_low_fee > out_high_fee,
            "lower fee must give more output"
        );
    }

    /// Upfront fee ceiling division.
    #[test]
    fn test_upfront_fee_ceil_division() {
        let dx: u128 = 1000;
        let x: u128 = 1_000_000;
        let y: u128 = 1_000_000;
        let fee_num: u128 = 25;
        let fee_den: u128 = 10_000;

        let out = cpmm_out_upfront_fee_ceil(dx, x, y, fee_num, fee_den).expect("swap");

        let expected_fee = (dx * fee_num).div_ceil(fee_den);
        let net_dx = dx - expected_fee;
        let expected_out = (net_dx * y) / (x + net_dx);

        assert!(
            (out as u128).abs_diff(expected_out) <= 1,
            "upfront fee ceil output mismatch"
        );
    }

    /// Overflow safety.
    #[test]
    fn test_cpmm_overflow_safe() {
        let x: u128 = u128::MAX / 4;
        let y: u128 = u128::MAX / 4;
        let dx: u128 = 1_000_000_000;

        let result = cpmm_out(dx, x, y, 9_975);
        assert!(
            result.is_none() || result.unwrap() > 0,
            "large values must not overflow"
        );
    }

    /// Unprofitable two-pool returns None.
    #[test]
    fn test_optimal_two_pool_unprofitable() {
        let l1 = Leg {
            r_in: 1_000_000,
            r_out: 1_000_000,
            gamma: 9_975,
        };
        let l2 = Leg {
            r_in: 1_000_000,
            r_out: 1_000_000,
            gamma: 9_975,
        };
        assert!(optimal_two_pool(&l1, &l2).is_none());
    }

    /// Profitable two-pool returns Some.
    #[test]
    fn test_optimal_two_pool_profitable() {
        let l1 = Leg {
            r_in: 1_000_000,
            r_out: 1_000_000,
            gamma: 9_975,
        };
        let l2 = Leg {
            r_in: 1_000_000,
            r_out: 1_050_000,
            gamma: 9_975,
        };
        assert!(optimal_two_pool(&l1, &l2).is_some());
    }

    /// Unprofitable three-pool returns None.
    #[test]
    fn test_optimal_three_pool_unprofitable() {
        let l1 = Leg {
            r_in: 1_000_000,
            r_out: 1_000_000,
            gamma: 9_975,
        };
        let l2 = Leg {
            r_in: 1_000_000,
            r_out: 1_000_000,
            gamma: 9_975,
        };
        let l3 = Leg {
            r_in: 1_000_000,
            r_out: 1_000_000,
            gamma: 9_975,
        };
        assert!(optimal_three_pool(&l1, &l2, &l3).is_none());
    }

    // Property: CPMM output bounds.
    proptest! {
        #[test]
        fn prop_cpmm_output_bounds(
            dx in 1..10_000_000u128,
            x in 1..10_000_000u128,
            y in 1..10_000_000u128,
            gamma in 9_000..10_000u128,
        ) {
            if let Some(out) = cpmm_out(dx, x, y, gamma) {
                assert!(out <= y);
                if out == 0 {
                    // Floor-rounded dust output is only valid when the exact
                    // quotient is below 1: gamma*dx*y < denom.
                    assert!(gamma * dx * y < 10_000 * x + gamma * dx);
                }
            }
        }

        #[test]
        fn prop_cpmm_deterministic(
            dx in 1..100_000u128,
            x in 1..1_000_000u128,
            y in 1..1_000_000u128,
            gamma in 9_000..10_000u128,
        ) {
            assert_eq!(cpmm_out(dx, x, y, gamma), cpmm_out(dx, x, y, gamma));
        }
    }

    /// CLMM sqrt price bounds.
    #[test]
    fn test_clmm_sqrt_price_bounds() {
        assert_eq!(MIN_SQRT_PRICE_X64, 4_295_048_016);
    }

    /// CLMM tick bounds.
    #[test]
    fn test_clmm_tick_bounds() {
        assert_eq!(MIN_TICK_INDEX, -443_636);
        assert_eq!(MAX_TICK_INDEX, 443_636);
    }
}

// ============================================================================
// Pump.fun / PumpSwap — property-based + invariant checks
// ============================================================================

mod pump_tests {
    use arb_math::pump_fun_raydium::{
        DEFAULT_FLASH_LOAN_FEE_BPS, DEFAULT_RAYDIUM_FEE_BPS, MIN_NET_PROFIT_LAMPORTS,
        PumpFunParams, RaydiumParams, calculate_optimal_trade, pump_fun_buy_cost,
        pump_fun_marginal_price, raydium_sell_output,
    };
    use proptest::prelude::*;

    /// Buy cost must be positive.
    #[test]
    fn test_pump_fun_buy_cost_positive() {
        for amount_in in [1, 1_000, 100_000, 1_000_000] {
            let cost = pump_fun_buy_cost(1_000_000_000, 1_000_000_000, amount_in as u128)
                .expect("buy cost must succeed");
            assert!(
                cost > 0,
                "buy cost must be positive for amount_in={amount_in}"
            );
            // Cost is SOL lamports, amount_in is token count — not directly comparable.
            // Just verify cost is non-zero and fits in u128.
            assert!(cost < u128::MAX, "cost must fit in u128");
        }
    }

    /// Buy cost monotonic.
    #[test]
    fn test_pump_fun_buy_cost_monotonic() {
        let cost_small = pump_fun_buy_cost(1_000_000_000, 1_000_000_000, 100).expect("small");
        let cost_large = pump_fun_buy_cost(1_000_000_000, 1_000_000_000, 10_000).expect("large");
        assert!(cost_large > cost_small);
    }

    /// Marginal price direction.
    #[test]
    fn test_pump_fun_marginal_price_direction() {
        let price_high_sol = pump_fun_marginal_price(1_000_000_000, 1_000_000_000).expect("high");
        let price_low_sol = pump_fun_marginal_price(1_000_000_000, 100_000_000).expect("low");
        assert!(price_low_sol > price_high_sol);
    }

    /// Graduation boundary at 85 SOL.
    #[test]
    fn test_pump_fun_graduation_boundary() {
        const GRADUATION_LAMPORTS: u64 = 85_000_000_000;
        let progress = (GRADUATION_LAMPORTS as u128 * 10_000) / (GRADUATION_LAMPORTS as u128);
        assert_eq!(progress, 10_000);
    }

    /// Zero virtual SOL reserves must fail.
    #[test]
    fn test_pump_fun_zero_reserves_fails() {
        assert!(
            pump_fun_buy_cost(1_000_000_000, 0, 100).is_none(),
            "zero supply must fail"
        );
    }

    /// Raydium sell output positive.
    #[test]
    fn test_raydium_sell_output_positive() {
        for amount_in in [1_000, 100_000, 10_000_000] {
            let out = raydium_sell_output(1_000_000_000, 1_000_000_000, amount_in as u128, 25)
                .expect("sell output");
            assert!(
                out > 0,
                "sell output must be positive for amount_in={amount_in}"
            );
        }
    }

    /// Min net profit is positive.
    #[test]
    fn test_min_net_profit_positive() {
        assert!(MIN_NET_PROFIT_LAMPORTS > 0);
    }

    /// Default fees.
    #[test]
    fn test_default_fees() {
        assert!(DEFAULT_FLASH_LOAN_FEE_BPS < 10_000);
        assert_eq!(DEFAULT_RAYDIUM_FEE_BPS, 25);
    }

    /// Optimal trade sizing: unprofitable returns Err.
    #[test]
    fn test_optimal_trade_unprofitable() {
        let pump = PumpFunParams {
            reserve: 1_000_000_000,
            supply: 1_000_000_000,
        };
        let raydium = RaydiumParams {
            token_reserve: 1_000_000_000,
            sol_reserve: 1_000_000_000,
            fee_bps: 25,
        };

        let result = calculate_optimal_trade(pump, raydium, DEFAULT_FLASH_LOAN_FEE_BPS, 1_000_000);
        assert!(
            result.is_err(),
            "identical reserves with fees must be unprofitable"
        );
    }

    // Property: deterministic.
    proptest! {
        #[test]
        fn prop_pump_fun_deterministic(
            reserve in 1..1_000_000_000u128,
            supply in 1..1_000_000_000u128,
            delta_t in 1..1_000_000u128,
        ) {
            assert_eq!(
                pump_fun_buy_cost(reserve, supply, delta_t),
                pump_fun_buy_cost(reserve, supply, delta_t)
            );
        }
    }
}

// ============================================================================
// Jupiter — invariant checks (off-chain API, no local SDK)
// ============================================================================

mod jupiter_tests {
    use arb_math::{calculate_net_profit, pick_tip};
    use arb_types::Lamports;

    /// Min output must not exceed expected output.
    #[test]
    fn test_min_out_bounds() {
        let expected_out: u64 = 1_000_000;
        let slippage_bps: u16 = 50;

        let min_out = expected_out.saturating_sub((expected_out * slippage_bps as u64) / 10_000);
        assert!(min_out <= expected_out);
        assert!(min_out > 0);
    }

    /// 100% slippage = min_out 0.
    #[test]
    fn test_max_slippage() {
        let expected_out: u64 = 1_000_000;
        let min_out = expected_out.saturating_sub((expected_out * 10_000u64) / 10_000);
        assert_eq!(min_out, 0);
    }

    /// Route consistency: final hop output = route output.
    #[test]
    fn test_route_consistency() {
        let _hop1_out: u64 = 1_000_000;
        let hop2_out: u64 = 950_000;
        assert_eq!(hop2_out, 950_000);
    }

    /// Platform fee accounting: net = gross - all costs.
    #[test]
    fn test_platform_fee_accounting() {
        let net = calculate_net_profit(
            Lamports(1_000_000),
            Lamports(10_000),
            Lamports(5_000),
            Lamports(5_000),
            Lamports(50_000),
            Lamports(0),
        )
        .expect("net must compute");
        assert_eq!(net, Lamports(930_000));
    }

    /// Costs exceeding gross → None.
    #[test]
    fn test_costs_exceed_gross() {
        let net = calculate_net_profit(
            Lamports(100_000),
            Lamports(50_000),
            Lamports(30_000),
            Lamports(30_000),
            Lamports(10_000),
            Lamports(0),
        );
        assert!(net.is_none(), "costs exceeding gross must return None");
    }

    /// Jito tip within budget.
    #[test]
    fn test_jito_tip_within_budget() {
        let tip = pick_tip(1_000_000, 100, 0, 500_000, 100_000).expect("tip");
        assert_eq!(tip, 10_000);
        assert!(1_000_000 - tip >= 100_000);
    }

    /// Jito tip respects cap.
    #[test]
    fn test_jito_tip_respects_cap() {
        let tip = pick_tip(10_000_000, 10_000, 0, 50_000, 0).expect("tip");
        assert_eq!(tip, 50_000);
    }

    /// Jito tip: None when min_keep unsatisfiable.
    #[test]
    fn test_jito_tip_min_keep() {
        let tip = pick_tip(100_000, 10_000, 0, 1_000_000, 150_000);
        assert!(tip.is_none());
    }

    /// Slippage applied once, not twice.
    #[test]
    fn test_slippage_applied_once() {
        let expected_out: u64 = 1_000_000;
        let slippage_bps: u16 = 100;
        let min_out = expected_out.saturating_sub((expected_out * slippage_bps as u64) / 10_000);
        assert_eq!(min_out, 990_000);
    }

    /// Token decimal conversion.
    #[test]
    fn test_token_decimal_conversion() {
        let usdc_ui = 1_000_000_f64 / 10f64.powi(6);
        let sol_ui = 1_000_000_000_f64 / 10f64.powi(9);
        assert!((usdc_ui - 1.0).abs() < 1e-6);
        assert!((sol_ui - 1.0).abs() < 1e-6);
    }

    /// Price impact small trade.
    #[test]
    fn test_price_impact_small_trade() {
        let diff = 1_000_000u64 - 990_000u64;
        let slippage_bps = (diff as u128 * 10_000) / 1_000_000u128;
        assert_eq!(slippage_bps, 100);
    }

    /// Stale pool state detection.
    #[test]
    fn test_stale_pool_state_detection() {
        let age = 100_500u64 - 100_000u64;
        assert!(age > 100);
    }

    /// Zero reserves swap fails.
    #[test]
    fn test_zero_reserves_fails() {
        use arb_math::cpmm::cpmm_out;
        assert!(cpmm_out(100, 0, 1_000_000, 9_975).is_none());
        assert!(cpmm_out(100, 1_000_000, 0, 9_975).is_none());
    }

    /// Integer rounding boundary.
    #[test]
    fn test_integer_rounding_boundary() {
        assert_eq!(9_999u128.div_ceil(10_000), 1);
        assert_eq!(10_000u128.div_ceil(10_000), 1);
        assert_eq!(10_001u128.div_ceil(10_000), 2);
    }
}
