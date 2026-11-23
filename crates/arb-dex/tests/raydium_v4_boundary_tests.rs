//! Feature 11: Raydium AMM V4 Swap Quotes & OpenBook Accounts — Tier 2 Boundary Tests.
//!
//! Validates zero reserves fail-closed, maximum 10,000 bps slippage handling,
//! exact-out exceeding pool reserve rejection, zero input amount, and disabled pool checks.

use arb_dex::engines::RaydiumAmmV4QuoteEngine;
use arb_dex::quote_engine::{QuoteEngine, QuoteRequest};
use arb_types::{
    ArbError, DexProtocol, MintId, PoolId, PoolKind, PoolState, Slot, TokenProgramKind,
};
use solana_pubkey::Pubkey;

fn make_v4_pool(
    reserve_in: u128,
    reserve_out: u128,
    is_active: bool,
) -> (PoolState, MintId, MintId) {
    let base_mint = MintId(Pubkey::new_unique());
    let quote_mint = MintId(Pubkey::new_unique());
    let state = PoolState {
        pool_id: PoolId::new([0x44; 32]),
        protocol: DexProtocol::RaydiumV4,
        slot: Slot(100),
        write_version: 1,
        base_mint,
        quote_mint,
        decimals_a: 6,
        decimals_b: 9,
        reserve_in,
        reserve_out,
        fee_numerator: 25,
        fee_denominator: 10_000,
        is_active,
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

/// Boundary Case 1: Zero reserves in Raydium V4 pool immediately fails closed.
#[test]
fn test_raydium_v4_quote_zero_reserves_fails_closed() {
    let engine = RaydiumAmmV4QuoteEngine::new();

    // Sub-case A: reserve_in = 0
    let (pool_zero_in, base_in, quote_in) = make_v4_pool(0, 10_000_000_000, true);
    let req = QuoteRequest {
        input_mint: base_in,
        output_mint: quote_in,
        amount: 1_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    match engine.quote_exact_in(&pool_zero_in, &req) {
        Err(ArbError::InsufficientLiquidity(msg)) => {
            assert!(msg.contains("has zero reserves"));
        }
        other => panic!("expected InsufficientLiquidity, got {:?}", other),
    }

    // Sub-case B: reserve_out = 0
    let (pool_zero_out, base_out, quote_out) = make_v4_pool(10_000_000_000, 0, true);
    let req2 = QuoteRequest {
        input_mint: base_out,
        output_mint: quote_out,
        amount: 1_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    match engine.quote_exact_in(&pool_zero_out, &req2) {
        Err(ArbError::InsufficientLiquidity(msg)) => {
            assert!(msg.contains("has zero reserves"));
        }
        other => panic!("expected InsufficientLiquidity, got {:?}", other),
    }
}

/// Boundary Case 2: Maximum 10,000 bps (100%) slippage produces minimum_out = 0 without underflow.
#[test]
fn test_raydium_v4_quote_max_slippage_10000_bps() {
    let engine = RaydiumAmmV4QuoteEngine::new();
    let (pool, base, quote) = make_v4_pool(10_000_000_000, 10_000_000_000, true);
    let req = QuoteRequest {
        input_mint: base,
        output_mint: quote,
        amount: 1_000_000,
        slippage_bps: 10_000, // 100% slippage
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    let quote = engine.quote_exact_in(&pool, &req).unwrap();
    assert_eq!(
        quote.minimum_amount_out_raw, 0,
        "100% slippage must evaluate to minimum_out = 0"
    );
    assert!(
        quote.amount_out_raw > 0,
        "expected output must remain positive"
    );
}

/// Boundary Case 3: quote_exact_out with target_dy >= reserve_out fails closed.
#[test]
fn test_raydium_v4_quote_exact_out_exceeds_reserve() {
    let engine = RaydiumAmmV4QuoteEngine::new();
    let (pool, base, quote) = make_v4_pool(10_000_000_000, 5_000_000_000, true);

    // Sub-case A: target_dy == reserve_out
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

    // Sub-case B: target_dy == reserve_out + 1
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

/// Boundary Case 4: Zero input amount returns ZeroAmount error.
#[test]
fn test_raydium_v4_zero_input_fails_closed() {
    let engine = RaydiumAmmV4QuoteEngine::new();
    let (pool, base, quote) = make_v4_pool(10_000_000_000, 10_000_000_000, true);
    let req = QuoteRequest {
        input_mint: base,
        output_mint: quote,
        amount: 0,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    assert!(
        matches!(
            engine.quote_exact_in(&pool, &req),
            Err(ArbError::ZeroAmount)
        ),
        "zero input must return ZeroAmount"
    );
}

/// Boundary Case 5: Disabled pool and mismatched mint fail closed.
#[test]
fn test_raydium_v4_disabled_pool_and_mismatched_mint() {
    let engine = RaydiumAmmV4QuoteEngine::new();

    // Sub-case A: is_active = false
    let (pool_inactive, base, quote_inactive) = make_v4_pool(10_000_000_000, 10_000_000_000, false);
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

    // Sub-case B: mismatched input mint
    let (pool_active, _, quote_active) = make_v4_pool(10_000_000_000, 10_000_000_000, true);
    let foreign = MintId(Pubkey::new_unique());
    let req_foreign = QuoteRequest {
        input_mint: foreign,
        output_mint: quote_active,
        amount: 1_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    match engine.quote_exact_in(&pool_active, &req_foreign) {
        Err(ArbError::InvalidToken(msg)) => {
            assert!(msg.contains("does not match pool mints"));
        }
        other => panic!("expected InvalidToken, got {:?}", other),
    }
}
