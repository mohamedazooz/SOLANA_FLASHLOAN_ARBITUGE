//! Feature 12: Orca Whirlpools Tick Arrays & Fee Math — Tier 2 Boundary Tests.
//!
//! Validates empty tick arrays handling, zero liquidity and price handling,
//! fee rate boundary limits (0 and >= 1,000,000 rejected), exact-out unsupported operation,
//! and disabled pool fail-closed behavior.

use arb_dex::engines::OrcaWhirlpoolQuoteEngine;
use arb_dex::quote_engine::{QuoteEngine, QuoteRequest};
use arb_types::pool::ClmmState;
use arb_types::{
    ArbError, DexProtocol, MintId, PoolId, PoolKind, PoolState, Slot, TokenProgramKind,
};
use solana_pubkey::Pubkey;

fn make_whirlpool_pool(
    liquidity: u128,
    sqrt_price: u128,
    fee_rate: u16,
    is_active: bool,
) -> (PoolState, MintId, MintId) {
    let base_mint = MintId(Pubkey::new_unique());
    let quote_mint = MintId(Pubkey::new_unique());
    let clmm = ClmmState {
        sqrt_price,
        tick_current: 0,
        liquidity,
        tick_spacing: 64,
        fee_rate,
        tick_arrays: vec![],
    };
    let state = PoolState {
        pool_id: PoolId::new([0x33; 32]),
        protocol: DexProtocol::OrcaWhirlpool,
        slot: Slot(100),
        write_version: 1,
        base_mint,
        quote_mint,
        decimals_a: 6,
        decimals_b: 9,
        reserve_in: 10_000_000_000,
        reserve_out: 10_000_000_000,
        fee_numerator: fee_rate as u64,
        fee_denominator: 1_000_000,
        is_active,
        has_risky_token: false,
        kind: PoolKind::Clmm,
        clmm: Some(clmm),
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

/// Boundary Case 1: Empty tick arrays fails closed with CalculationError or IncompleteState.
#[test]
fn test_orca_whirlpool_empty_or_uninitialized_tick_arrays() {
    let engine = OrcaWhirlpoolQuoteEngine::new();
    let (pool, base, quote) = make_whirlpool_pool(10_000_000, 1u128 << 64, 2500, true);
    let req = QuoteRequest {
        input_mint: base,
        output_mint: quote,
        amount: 1_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    match engine.quote_exact_in(&pool, &req) {
        Err(ArbError::CalculationError(msg)) => {
            assert!(msg.contains("Orca Whirlpool step swap failed"));
        }
        other => panic!(
            "expected CalculationError for empty tick arrays, got {:?}",
            other
        ),
    }
}

/// Boundary Case 2: Zero liquidity or zero price fails closed with InsufficientLiquidity.
#[test]
fn test_orca_whirlpool_zero_liquidity_and_price_fails_closed() {
    let engine = OrcaWhirlpoolQuoteEngine::new();

    // Sub-case A: liquidity = 0
    let (pool_zero_liq, base_liq, quote_liq) = make_whirlpool_pool(0, 1u128 << 64, 2500, true);
    let req = QuoteRequest {
        input_mint: base_liq,
        output_mint: quote_liq,
        amount: 1_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    match engine.quote_exact_in(&pool_zero_liq, &req) {
        Err(ArbError::InsufficientLiquidity(msg)) => {
            assert!(msg.contains("Orca Whirlpool zero liquidity/price"));
        }
        other => panic!("expected InsufficientLiquidity, got {:?}", other),
    }

    // Sub-case B: sqrt_price = 0
    let (pool_zero_price, base_price, quote_price) = make_whirlpool_pool(10_000_000, 0, 2500, true);
    let req2 = QuoteRequest {
        input_mint: base_price,
        output_mint: quote_price,
        amount: 1_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    match engine.quote_exact_in(&pool_zero_price, &req2) {
        Err(ArbError::InsufficientLiquidity(msg)) => {
            assert!(msg.contains("Orca Whirlpool zero liquidity/price"));
        }
        other => panic!("expected InsufficientLiquidity, got {:?}", other),
    }
}

/// Boundary Case 3: Fee rate boundary limits (fee_rate = 0 fails closed).
#[test]
fn test_orca_whirlpool_fee_rate_boundary_limits() {
    let engine = OrcaWhirlpoolQuoteEngine::new();
    let (mut pool, base, quote) = make_whirlpool_pool(10_000_000, 1u128 << 64, 0, true);
    pool.fee_numerator = 0; // ensure no fallback
    pool.fee_denominator = 0;

    let req = QuoteRequest {
        input_mint: base,
        output_mint: quote,
        amount: 1_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    match engine.quote_exact_in(&pool, &req) {
        Err(ArbError::IncompleteState(msg)) => {
            assert!(msg.contains("has no decoded fee_rate"));
        }
        Err(ArbError::InvalidInput(msg)) => {
            assert!(msg.contains("out of bounds"));
        }
        other => panic!(
            "expected IncompleteState or InvalidInput for zero fee_rate, got {:?}",
            other
        ),
    }
}

/// Boundary Case 4: quote_exact_out returns UnsupportedOperation.
#[test]
fn test_orca_whirlpool_exact_out_unsupported_operation() {
    let engine = OrcaWhirlpoolQuoteEngine::new();
    let (pool, base, quote) = make_whirlpool_pool(10_000_000, 1u128 << 64, 2500, true);
    let req = QuoteRequest {
        input_mint: base,
        output_mint: quote,
        amount: 1_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    match engine.quote_exact_out(&pool, &req) {
        Err(ArbError::UnsupportedOperation(msg)) => {
            assert!(msg.contains("quote_exact_out not yet supported"));
        }
        other => panic!("expected UnsupportedOperation, got {:?}", other),
    }
}

/// Boundary Case 5: Disabled pool and zero input amount fail closed.
#[test]
fn test_orca_whirlpool_disabled_pool_and_zero_input() {
    let engine = OrcaWhirlpoolQuoteEngine::new();

    // Sub-case A: is_active = false
    let (pool_inactive, base, quote_inactive) =
        make_whirlpool_pool(10_000_000, 1u128 << 64, 2500, false);
    let req = QuoteRequest {
        input_mint: base,
        output_mint: quote_inactive,
        amount: 1_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    match engine.quote_exact_in(&pool_inactive, &req) {
        Err(ArbError::PoolDisabled(msg)) => {
            assert!(msg.contains("disabled"));
        }
        other => panic!("expected PoolDisabled, got {:?}", other),
    }

    // Sub-case B: amount = 0
    let (pool_active, base_active, quote_active) =
        make_whirlpool_pool(10_000_000, 1u128 << 64, 2500, true);
    let req_zero = QuoteRequest {
        input_mint: base_active,
        output_mint: quote_active,
        amount: 0,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    assert!(
        matches!(
            engine.quote_exact_in(&pool_active, &req_zero),
            Err(ArbError::ZeroAmount)
        ),
        "zero amount must return ZeroAmount"
    );
}
