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
        is_active: true,
        has_risky_token: false,
        kind: PoolKind::Cpmm,
        clmm: None,
        dlmm: None,
        pump_fun: None,
        transfer_fee_bps: 0,
        token_programs: [TokenProgramKind::SplToken; 2],
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
    let (pool_zero_fee, base, _) = make_cpmm_pool(10_000_000_000, 10_000_000_000, 0);
    let req = QuoteRequest {
        input_mint: base,
        output_mint: MintId(Pubkey::default()),
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
    let (pool_max_fee, base_max, _) = make_cpmm_pool(10_000_000_000, 10_000_000_000, 100_000);
    let req_max = QuoteRequest {
        input_mint: base_max,
        output_mint: MintId(Pubkey::default()),
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

/// Boundary Case 2: Zero input amount fails closed with ZeroAmount.
#[test]
fn test_cpmm_quote_engine_zero_input_and_exhaustion() {
    let engine = RaydiumCpmmQuoteEngine::new();
    let (pool, base, _) = make_cpmm_pool(10_000_000_000, 10_000_000_000, 2500);

    let req_zero = QuoteRequest {
        input_mint: base,
        output_mint: MintId(Pubkey::default()),
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
    let (pool_deep, base_deep, _) = make_cpmm_pool(1, 1_000_000_000_000_000_000, 0);
    let req = QuoteRequest {
        input_mint: base_deep,
        output_mint: MintId(Pubkey::default()),
        amount: 1,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    // dy = (10^18 * 1) / (1 + 1) = 5 * 10^17
    let quote = engine.quote_exact_in(&pool_deep, &req).unwrap();
    assert_eq!(quote.amount_out_raw, 500_000_000_000_000_000);

    // Sub-case B: r_in = 10^18, r_out = 1 (output is 0, must fail closed)
    let (pool_shallow, base_shallow, _) = make_cpmm_pool(1_000_000_000_000_000_000, 1, 0);
    let req_shallow = QuoteRequest {
        input_mint: base_shallow,
        output_mint: MintId(Pubkey::default()),
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
    let (pool_zero_res, base_zero_res, _) = make_cpmm_pool(0, 10_000_000_000, 2500);
    let req_zero_res = QuoteRequest {
        input_mint: base_zero_res,
        output_mint: MintId(Pubkey::default()),
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
    let (mut pool, base, _) = make_cpmm_pool(10_000_000_000, 10_000_000_000, 2500);
    pool.is_active = false;

    let req = QuoteRequest {
        input_mint: base,
        output_mint: MintId(Pubkey::default()),
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
