//! Feature 10: Raydium CLMM Sqrt-Price Limit & Tick Transitions — Tier 2 Boundary Tests.
//!
//! Validates min/max tick conversions, zero liquidity and price handling,
//! exact-out unsupported operation, disabled pool rejection, and mismatched mint fail-closed.

use arb_dex::engines::RaydiumClmmQuoteEngine;
use arb_dex::quote_engine::{QuoteEngine, QuoteRequest};
use arb_math::clmm::{
    MAX_TICK_INDEX, MIN_SQRT_PRICE_X64, MIN_TICK_INDEX, RAYDIUM_MAX_SQRT_PRICE_X64,
    sqrt_price_x64_in_raydium_clmm_bounds, tick_index_to_sqrt_price_x64,
};
use arb_types::pool::ClmmState;
use arb_types::{
    ArbError, DexProtocol, MintId, PoolId, PoolKind, PoolState, Slot, TokenProgramKind,
};
use solana_pubkey::Pubkey;

fn make_clmm_pool(
    liquidity: u128,
    sqrt_price: u128,
    is_active: bool,
) -> (PoolState, MintId, MintId) {
    let base_mint = MintId(Pubkey::new_unique());
    let quote_mint = MintId(Pubkey::new_unique());
    let clmm = ClmmState {
        sqrt_price,
        tick_current: 0,
        liquidity,
        tick_spacing: 64,
        fee_rate: 2500, // 2500 ppm = 25 bps
        tick_arrays: vec![],
    };
    let state = PoolState {
        pool_id: PoolId::new([0x10; 32]),
        protocol: DexProtocol::RaydiumClmm,
        slot: Slot(100),
        write_version: 1,
        base_mint,
        quote_mint,
        decimals_a: 6,
        decimals_b: 9,
        reserve_in: 10_000_000_000,
        reserve_out: 10_000_000_000,
        fee_numerator: 2500,
        fee_denominator: 1_000_000,
        raydium_cpmm_fee: None,
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

/// Boundary Case 1: Min and Max tick index endpoints convert within Raydium CLMM bounds.
#[test]
fn test_raydium_clmm_min_max_tick_boundary_conversions() {
    let min_sqrt = tick_index_to_sqrt_price_x64(MIN_TICK_INDEX).unwrap();
    assert!(
        sqrt_price_x64_in_raydium_clmm_bounds(min_sqrt),
        "min tick must map within Raydium CLMM bounds"
    );
    assert!(
        min_sqrt >= MIN_SQRT_PRICE_X64,
        "min tick sqrt_price must be >= MIN_SQRT_PRICE_X64"
    );

    let max_sqrt = tick_index_to_sqrt_price_x64(MAX_TICK_INDEX).unwrap();
    assert!(
        sqrt_price_x64_in_raydium_clmm_bounds(max_sqrt),
        "max tick must map within Raydium CLMM bounds"
    );
    assert!(
        max_sqrt <= RAYDIUM_MAX_SQRT_PRICE_X64,
        "max tick sqrt_price must be <= RAYDIUM_MAX_SQRT_PRICE_X64"
    );
}

/// Boundary Case 2: Zero liquidity or zero sqrt price fails closed with InsufficientLiquidity.
#[test]
fn test_raydium_clmm_zero_liquidity_and_zero_price_fails_closed() {
    let engine = RaydiumClmmQuoteEngine::new();

    // Sub-case A: liquidity = 0
    let (pool_zero_liq, base_liq, quote_liq) = make_clmm_pool(0, 1u128 << 64, true);
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
            assert!(msg.contains("Raydium CLMM zero liquidity/price"));
        }
        other => panic!("expected InsufficientLiquidity, got {:?}", other),
    }

    // Sub-case B: sqrt_price = 0
    let (pool_zero_price, base_price, quote_price) = make_clmm_pool(10_000_000, 0, true);
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
            assert!(msg.contains("Raydium CLMM zero liquidity/price"));
        }
        other => panic!("expected InsufficientLiquidity, got {:?}", other),
    }
}

/// Boundary Case 3: quote_exact_out returns UnsupportedOperation on CLMM.
#[test]
fn test_raydium_clmm_exact_out_unsupported_operation() {
    let engine = RaydiumClmmQuoteEngine::new();
    let (pool, base, quote) = make_clmm_pool(10_000_000, 1u128 << 64, true);
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

/// Boundary Case 4: Inactive pool fails closed with PoolDisabled.
#[test]
fn test_raydium_clmm_disabled_pool_fails_closed() {
    let engine = RaydiumClmmQuoteEngine::new();
    let (pool, base, quote) = make_clmm_pool(10_000_000, 1u128 << 64, false);
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

/// Boundary Case 5: Unrecognized input mint fails closed with InvalidToken.
#[test]
fn test_raydium_clmm_mismatched_mint_fails_closed() {
    let engine = RaydiumClmmQuoteEngine::new();
    let (pool, _, quote) = make_clmm_pool(10_000_000, 1u128 << 64, true);
    let foreign_mint = MintId(Pubkey::new_unique());
    let req = QuoteRequest {
        input_mint: foreign_mint,
        output_mint: quote,
        amount: 1_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    match engine.quote_exact_in(&pool, &req) {
        Err(ArbError::InvalidToken(msg)) => {
            assert!(msg.contains("does not match pool mints"));
        }
        other => panic!("expected InvalidToken, got {:?}", other),
    }
}
