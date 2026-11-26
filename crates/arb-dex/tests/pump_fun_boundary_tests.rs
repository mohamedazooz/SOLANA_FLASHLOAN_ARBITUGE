//! Feature 13: Pump.fun Virtual Curve & Migration Boundaries — Tier 2 Boundary Tests.
//!
//! Validates graduated curve migration boundary (complete = true), zero virtual reserves,
//! 100% token supply purchase ceiling, zero input amount, and exact-out unsupported operation.

use arb_dex::engines::PumpBondingCurveQuoteEngine;
use arb_dex::quote_engine::{QuoteEngine, QuoteRequest};
use arb_types::pool::PumpFunState;
use arb_types::{
    ArbError, DexProtocol, MintId, PoolId, PoolKind, PoolState, Slot, TokenProgramKind,
};
use solana_pubkey::Pubkey;

fn make_pump_fun_pool(
    v_token: u64,
    v_sol: u64,
    r_token: u64,
    complete: bool,
    is_active: bool,
) -> (PoolState, MintId, MintId) {
    let base_mint = MintId(Pubkey::new_unique()); // WSOL (SOL)
    let quote_mint = MintId(Pubkey::new_unique()); // Meme Token
    let pump_fun = PumpFunState {
        virtual_token_reserves: v_token,
        virtual_sol_reserves: v_sol,
        real_token_reserves: r_token,
        real_sol_reserves: 30_000_000_000,
        token_total_supply: 1_000_000_000_000_000,
        complete,
        creator: Pubkey::new_unique(),
        is_mayhem_mode: false,
        is_cashback_coin: false,
        mode_flags_present: true,
    };
    let state = PoolState {
        pool_id: PoolId::new([0x66; 32]),
        protocol: DexProtocol::PumpFun,
        slot: Slot(100),
        write_version: 1,
        base_mint,
        quote_mint,
        decimals_a: 9,
        decimals_b: 6,
        reserve_in: v_sol as u128,
        reserve_out: v_token as u128,
        fee_numerator: 125, // documented 1.25% SOL bonding-curve fee
        fee_denominator: 10_000,
        raydium_cpmm_fee: None,
        is_active,
        has_risky_token: false,
        kind: PoolKind::Cpmm,
        clmm: None,
        dlmm: None,
        pump_fun: Some(pump_fun),
        transfer_fee_bps: 0,
        token_programs: [TokenProgramKind::SplToken; 2],
        execution_accounts: None,
        last_update_slot: 100,
        last_update_ms: 1_700_000_000_000,
    };
    (state, base_mint, quote_mint)
}

/// Boundary Case 1: Graduated curve (`complete = true`) immediately fails closed with PoolMigrated.
#[test]
fn test_pump_fun_graduated_migration_boundary() {
    let engine = PumpBondingCurveQuoteEngine::new();
    let (pool_migrated, base, quote) = make_pump_fun_pool(
        1_073_000_000_000_000,
        30_000_000_000,
        793_100_000_000_000,
        true, // complete
        true,
    );
    let req = QuoteRequest {
        input_mint: base,
        output_mint: quote,
        amount: 1_000_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    match engine.quote_exact_in(&pool_migrated, &req) {
        Err(ArbError::PoolMigrated(msg)) => {
            assert!(msg.contains("has graduated; use PumpSwap quote engine"));
        }
        other => panic!("expected PoolMigrated, got {:?}", other),
    }
}

/// Boundary Case 2: Zero virtual reserves fails closed with InsufficientLiquidity.
#[test]
fn test_pump_fun_zero_virtual_reserves_fails_closed() {
    let engine = PumpBondingCurveQuoteEngine::new();

    // Sub-case A: v_sol = 0
    let (pool_zero_sol, base_sol, quote_sol) = make_pump_fun_pool(
        1_073_000_000_000_000,
        0, // v_sol = 0
        793_100_000_000_000,
        false,
        true,
    );
    let req = QuoteRequest {
        input_mint: base_sol,
        output_mint: quote_sol,
        amount: 1_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    match engine.quote_exact_in(&pool_zero_sol, &req) {
        Err(ArbError::InsufficientLiquidity(msg)) => {
            assert!(msg.contains("Pump.fun virtual reserves are 0"));
        }
        other => panic!("expected InsufficientLiquidity, got {:?}", other),
    }

    // Sub-case B: v_token = 0
    let (pool_zero_token, base_token, quote_token) = make_pump_fun_pool(
        0, // v_token = 0
        30_000_000_000,
        793_100_000_000_000,
        false,
        true,
    );
    let req2 = QuoteRequest {
        input_mint: base_token,
        output_mint: quote_token,
        amount: 1_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    match engine.quote_exact_in(&pool_zero_token, &req2) {
        Err(ArbError::InsufficientLiquidity(msg)) => {
            assert!(msg.contains("Pump.fun virtual reserves are 0"));
        }
        other => panic!("expected InsufficientLiquidity, got {:?}", other),
    }
}

/// Boundary Case 3: Massive buy order strictly capped by real_token_reserves ceiling.
#[test]
fn test_pump_fun_100_percent_token_supply_purchase_ceiling() {
    let engine = PumpBondingCurveQuoteEngine::new();
    let r_token = 500_000_000_000u64;
    let (pool, base, quote) =
        make_pump_fun_pool(1_073_000_000_000_000, 30_000_000_000, r_token, false, true);

    // Massive buy of 1,000,000 SOL
    let req = QuoteRequest {
        input_mint: base,
        output_mint: quote,
        amount: 1_000_000_000_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    let quote = engine.quote_exact_in(&pool, &req).unwrap();
    assert!(
        quote.amount_out_raw <= r_token,
        "tokens out ({}) must not exceed real_token_reserves ({})",
        quote.amount_out_raw,
        r_token
    );
    assert_eq!(
        quote.amount_out_raw, r_token,
        "massive buy must drain available real_token_reserves exactly"
    );
}

/// Boundary Case 4: Zero input amount fails closed with ZeroAmount.
#[test]
fn test_pump_fun_zero_input_and_fee_exhaustion() {
    let engine = PumpBondingCurveQuoteEngine::new();
    let (pool, base, quote) = make_pump_fun_pool(
        1_073_000_000_000_000,
        30_000_000_000,
        793_100_000_000_000,
        false,
        true,
    );

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

/// Boundary Case 5: quote_exact_out returns UnsupportedOperation on Pump.fun bonding curves.
#[test]
fn test_pump_fun_exact_out_unsupported_operation() {
    let engine = PumpBondingCurveQuoteEngine::new();
    let (pool, base, quote) = make_pump_fun_pool(
        1_073_000_000_000_000,
        30_000_000_000,
        793_100_000_000_000,
        false,
        true,
    );

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
            assert!(msg.contains("Pump.fun bonding curve exact out not yet supported"));
        }
        other => panic!("expected UnsupportedOperation, got {:?}", other),
    }
}
