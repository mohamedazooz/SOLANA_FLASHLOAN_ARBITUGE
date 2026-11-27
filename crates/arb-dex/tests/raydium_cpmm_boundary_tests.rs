//! Feature 9: Raydium CPMM Fee Tiers & Instruction Assembly — Tier 2 Boundary Tests.
//!
//! Validates extreme fee tiers (0 ppm vs 100,000 ppm), zero input and fee exhaustion,
//! extreme reserve ratios, and exact out exceeding reserves.

use arb_dex::engines::RaydiumCpmmQuoteEngine;
use arb_dex::quote_engine::{QuoteEngine, QuoteRequest};
use arb_types::{
    ArbError, DexProtocol, MintId, PoolId, PoolKind, PoolState, Slot, TokenProgramKind,
};
use solana_pubkey::Pubkey;

fn make_cpmm_pool(
    reserve_in: u128,
    reserve_out: u128,
    trade_fee_rate: u64,
) -> (PoolState, MintId, MintId) {
    let base_mint = MintId(Pubkey::new_unique());
    let quote_mint = MintId(Pubkey::new_unique());
    let state = PoolState {
        pool_id: PoolId::new([1u8; 32]),
        protocol: DexProtocol::RaydiumCpmm,
        slot: Slot(100),
        write_version: 1,
        base_mint,
        quote_mint,
        decimals_a: 6,
        decimals_b: 9,
        reserve_in,
        reserve_out,
        fee_numerator: trade_fee_rate,
        fee_denominator: 1_000_000,
        raydium_cpmm_fee: Some(arb_types::RaydiumCpmmFeeState {
            trade_fee_rate_ppm: trade_fee_rate,
            creator_fee_rate_ppm: 0,
            protocol_fee_rate: 120_000,
            fund_fee_rate: 40_000,
            enable_creator_fee: false,
            creator_fee_on: 0,
        }),
        is_active: true,
        has_risky_token: false,
        kind: PoolKind::Cpmm,
        clmm: None,
        dlmm: None,
        pump_fun: None,
        damm_v2: None,
        transfer_fee_bps: 0,
        token_programs: [TokenProgramKind::SplToken; 2],
        token2022_fee_context: None,
        execution_accounts: None,
        last_update_slot: 100,
        last_update_ms: 1_700_000_000_000,
    };
    (state, base_mint, quote_mint)
}

/// Boundary Case 1: Extreme fee tiers: 0 ppm (free) vs 100,000 ppm (10% max).
#[test]
fn test_cpmm_quote_engine_extreme_fee_tiers_zero_and_max() {
    let engine = RaydiumCpmmQuoteEngine::new();

    // 0 ppm fee tier
    let (pool_zero_fee, base, quote_zero) = make_cpmm_pool(10_000_000_000, 10_000_000_000, 0);
    let req = QuoteRequest {
        input_mint: base,
        output_mint: quote_zero,
        amount: 1_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    let quote_zero = engine.quote_exact_in(&pool_zero_fee, &req).unwrap();
    assert_eq!(
        quote_zero.fees_breakdown.total_fee, 0,
        "0 ppm must charge zero fee"
    );
    // dy = (10^10 * 10^6) / (10^10 + 10^6) = 999900
    assert_eq!(quote_zero.amount_out_raw, 999_900);

    // 100,000 ppm fee tier (10%)
    let (pool_max_fee, base_max, quote_max) =
        make_cpmm_pool(10_000_000_000, 10_000_000_000, 100_000);
    let req_max = QuoteRequest {
        input_mint: base_max,
        output_mint: quote_max,
        amount: 1_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    let quote_max = engine.quote_exact_in(&pool_max_fee, &req_max).unwrap();
    assert_eq!(
        quote_max.fees_breakdown.total_fee, 100_000,
        "10% fee on 1,000,000 must be 100,000"
    );
    // input after fee = 900_000; dy = (10^10 * 900_000) / (10^10 + 900_000) = 899919
    assert_eq!(quote_max.amount_out_raw, 899_919);
}

/// Regression: 0.005% is 0.5 bps and must not be rounded down to a free swap.
#[test]
fn test_cpmm_quote_preserves_sub_basis_point_fee_precision() {
    let engine = RaydiumCpmmQuoteEngine::new();
    let (pool, base, quote) = make_cpmm_pool(10_000_000_000, 10_000_000_000, 50);
    let request = QuoteRequest {
        input_mint: base,
        output_mint: quote,
        amount: 1_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    let result = engine.quote_exact_in(&pool, &request).unwrap();
    assert_eq!(result.fees_breakdown.total_fee, 50);
    assert_eq!(result.amount_out_raw, 999_850);
}

/// Boundary Case 2: Zero input amount fails closed with ZeroAmount.
#[test]
fn test_cpmm_quote_engine_zero_input_and_exhaustion() {
    let engine = RaydiumCpmmQuoteEngine::new();
    let (pool, base, quote) = make_cpmm_pool(10_000_000_000, 10_000_000_000, 2500);

    let req_zero = QuoteRequest {
        input_mint: base,
        output_mint: quote,
        amount: 0,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    assert!(
        matches!(
            engine.quote_exact_in(&pool, &req_zero),
            Err(ArbError::ZeroAmount)
        ),
        "amount = 0 must return ZeroAmount"
    );
}

/// Boundary Case 3: Extreme reserve ratios and zero reserves.
#[test]
fn test_cpmm_quote_engine_extreme_reserve_ratios() {
    let engine = RaydiumCpmmQuoteEngine::new();

    // Sub-case A: r_in = 1, r_out = 10^18 (massive reserve out)
    let (pool_deep, base_deep, quote_deep) = make_cpmm_pool(1, 1_000_000_000_000_000_000, 0);
    let req = QuoteRequest {
        input_mint: base_deep,
        output_mint: quote_deep,
        amount: 1,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    // dy = (10^18 * 1) / (1 + 1) = 5 * 10^17
    let quote = engine.quote_exact_in(&pool_deep, &req).unwrap();
    assert_eq!(quote.amount_out_raw, 500_000_000_000_000_000);

    // Sub-case B: r_in = 10^18, r_out = 1 (output is 0, must fail closed)
    let (pool_shallow, base_shallow, quote_shallow) =
        make_cpmm_pool(1_000_000_000_000_000_000, 1, 0);
    let req_shallow = QuoteRequest {
        input_mint: base_shallow,
        output_mint: quote_shallow,
        amount: 1_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    match engine.quote_exact_in(&pool_shallow, &req_shallow) {
        Err(ArbError::InsufficientLiquidity(msg)) => {
            assert!(msg.contains("returned 0 output tokens"));
        }
        other => panic!("expected InsufficientLiquidity, got {:?}", other),
    }

    // Sub-case C: r_in = 0, r_out = 10^10
    let (pool_zero_res, base_zero_res, quote_zero_res) = make_cpmm_pool(0, 10_000_000_000, 2500);
    let req_zero_res = QuoteRequest {
        input_mint: base_zero_res,
        output_mint: quote_zero_res,
        amount: 100_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    match engine.quote_exact_in(&pool_zero_res, &req_zero_res) {
        Err(ArbError::InsufficientLiquidity(msg)) => {
            assert!(msg.contains("has zero reserves"));
        }
        other => panic!("expected InsufficientLiquidity, got {:?}", other),
    }
}

/// Boundary Case 4: quote_exact_out with target_dy >= r_out fails closed.
#[test]
fn test_cpmm_quote_engine_exact_out_boundary_exceeds_reserve() {
    let engine = RaydiumCpmmQuoteEngine::new();
    let (pool, base, quote) = make_cpmm_pool(10_000_000_000, 5_000_000_000, 2500);

    // Exact equality: target_dy == r_out (5_000_000_000)
    let req_equal = QuoteRequest {
        input_mint: base,
        output_mint: quote,
        amount: 5_000_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    match engine.quote_exact_out(&pool, &req_equal) {
        Err(ArbError::InsufficientLiquidity(msg)) => {
            assert!(msg.contains("Exact out exceeds pool reserve"));
        }
        other => panic!("expected InsufficientLiquidity, got {:?}", other),
    }

    // Strictly exceeding: target_dy == r_out + 1
    let req_exceed = QuoteRequest {
        input_mint: base,
        output_mint: quote,
        amount: 5_000_000_001,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    match engine.quote_exact_out(&pool, &req_exceed) {
        Err(ArbError::InsufficientLiquidity(msg)) => {
            assert!(msg.contains("Exact out exceeds pool reserve"));
        }
        other => panic!("expected InsufficientLiquidity, got {:?}", other),
    }
}

/// Boundary Case 5: Disabled pool fails closed.
#[test]
fn test_cpmm_quote_engine_disabled_pool_fails_closed() {
    let engine = RaydiumCpmmQuoteEngine::new();
    let (mut pool, base, quote) = make_cpmm_pool(10_000_000_000, 10_000_000_000, 2500);
    pool.is_active = false;

    let req = QuoteRequest {
        input_mint: base,
        output_mint: quote,
        amount: 1_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    match engine.quote_exact_in(&pool, &req) {
        Err(ArbError::PoolDisabled(msg)) => {
            assert!(msg.contains("disabled"));
        }
        other => panic!("expected PoolDisabled, got {:?}", other),
    }
}

#[test]
fn creator_fee_matches_pool_mode_for_both_swap_directions() {
    let engine = RaydiumCpmmQuoteEngine::new();
    let amount = 1_000_000;

    for (mode, a_to_b, expected_input_side) in [
        (0u8, true, true),
        (0u8, false, true),
        (1u8, true, true),
        (1u8, false, false),
        (2u8, true, false),
        (2u8, false, true),
    ] {
        let (mut pool, base, quote) = make_cpmm_pool(10_000_000_000, 20_000_000_000, 2_500);
        pool.raydium_cpmm_fee = Some(arb_types::RaydiumCpmmFeeState {
            trade_fee_rate_ppm: 2_500,
            creator_fee_rate_ppm: 500,
            protocol_fee_rate: 120_000,
            fund_fee_rate: 40_000,
            enable_creator_fee: true,
            creator_fee_on: mode,
        });
        let (input_mint, output_mint) = if a_to_b { (base, quote) } else { (quote, base) };
        let request = QuoteRequest {
            input_mint,
            output_mint,
            amount,
            slippage_bps: 50,
            input_transfer_fee_config: None,
            output_transfer_fee_config: None,
        };
        let result = engine
            .quote_exact_in(&pool, &request)
            .expect("creator-fee quote");
        assert!(
            result.fees_breakdown.creator_fee > 0,
            "mode={mode} a_to_b={a_to_b}"
        );
        assert_eq!(
            result.creator_fee_mint,
            if expected_input_side {
                input_mint
            } else {
                output_mint
            },
            "creator fee denomination must follow the actual collection side"
        );
        assert_eq!(result.total_fee_mint, input_mint);
        assert_eq!(
            result.fees_breakdown.total_fee,
            if expected_input_side { 3_000 } else { 2_500 },
            "input-side fee total must include creator fee only when the mode charges input"
        );
        assert_eq!(
            result.fees_breakdown.lp_fee
                + result.fees_breakdown.protocol_fee
                + result.fees_breakdown.fund_fee,
            2_500,
            "protocol/fund splits are derived from trade fee only"
        );
        if expected_input_side {
            assert_eq!(
                result.transfer_fee_breakdown.output_curve_reserve_debit,
                result.transfer_fee_breakdown.output_gross_from_pool,
                "input-side creator fee does not change the output reserve debit"
            );
        } else {
            assert!(
                result.transfer_fee_breakdown.output_curve_reserve_debit
                    > result.transfer_fee_breakdown.output_gross_from_pool,
                "output-side creator fee remains in the vault and must be excluded from curve reserves"
            );
        }
        assert_eq!(
            result.fees_breakdown.creator_fee,
            if expected_input_side {
                500
            } else {
                let (reserve_in, reserve_out) = if a_to_b {
                    (10_000_000_000, 20_000_000_000)
                } else {
                    (20_000_000_000, 10_000_000_000)
                };
                let gross = arb_math::cpmm::cpmm_out_upfront_fee_ceil(
                    amount as u128,
                    reserve_in,
                    reserve_out,
                    2_500,
                    1_000_000,
                )
                .unwrap();
                u64::try_from((gross as u128 * 500).div_ceil(1_000_000)).unwrap()
            },
            "creator fee must use the correct side and protocol rounding"
        );
    }
}

#[test]
fn creator_fee_exact_out_fails_closed_until_inverse_fee_path_is_supported() {
    let engine = RaydiumCpmmQuoteEngine::new();
    let (mut pool, base, quote) = make_cpmm_pool(10_000_000_000, 20_000_000_000, 2_500);
    pool.raydium_cpmm_fee = Some(arb_types::RaydiumCpmmFeeState {
        trade_fee_rate_ppm: 2_500,
        creator_fee_rate_ppm: 500,
        protocol_fee_rate: 120_000,
        fund_fee_rate: 40_000,
        enable_creator_fee: true,
        creator_fee_on: 0,
    });
    let request = QuoteRequest {
        input_mint: base,
        output_mint: quote,
        amount: 1_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    assert!(matches!(
        engine.quote_exact_out(&pool, &request),
        Err(ArbError::UnsupportedOperation(_))
    ));
}
