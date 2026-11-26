//! Feature 14: PumpSwap Engine & Protocol ID Resolution — Tier 2 Boundary Tests.
//!
//! Validates overflow check on reserve_out exceeding u64::MAX (testing the remediation of silent truncation),
//! zero reserves fail-closed, Token-2022 100% transfer fee exhaustion, exact-out rejection,
//! and large u64 input amount arithmetic safety.

use arb_dex::engines::PumpSwapQuoteEngine;
use arb_dex::quote_engine::{QuoteEngine, QuoteRequest};
use arb_dex::token2022::TransferFeeConfig;
use arb_types::{
    ArbError, DexProtocol, MintId, PoolId, PoolKind, PoolState, Slot, TokenProgramKind,
};
use solana_pubkey::Pubkey;

fn make_pumpswap_pool(
    reserve_in: u128,
    reserve_out: u128,
    is_active: bool,
) -> (PoolState, MintId, MintId) {
    let base_mint = MintId(Pubkey::new_unique());
    let quote_mint = MintId(Pubkey::new_unique());
    let state = PoolState {
        pool_id: PoolId::new([0x55; 32]),
        protocol: DexProtocol::PumpSwap,
        slot: Slot(100),
        write_version: 1,
        base_mint,
        quote_mint,
        decimals_a: 6,
        decimals_b: 9,
        reserve_in,
        reserve_out,
        fee_numerator: 30,
        fee_denominator: 10_000,
        raydium_cpmm_fee: None,
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

/// Boundary Case 1: Overflow check when output exceeds u64::MAX (testing checked conversion remediation).
#[test]
fn test_pumpswap_boundary_reserve_out_exceeding_u64_max_overflow_check() {
    let engine = PumpSwapQuoteEngine::new();
    // reserve_out is 10^24 (well above u64::MAX ~ 1.84 * 10^19)
    let massive_reserve_out = 1_000_000_000_000_000_000_000_000u128;
    let (pool, base, quote) = make_pumpswap_pool(1_000_000_000, massive_reserve_out, true);

    let req = QuoteRequest {
        input_mint: base,
        output_mint: quote,
        amount: 1_000_000_000, // dx' ~ 10^9, output would be ~ massive_reserve_out / 2 >> u64::MAX
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };

    // Before remediation: truncated via `as u64`.
    // After remediation: returns `Err(ArbError::Overflow)`.
    assert!(
        matches!(engine.quote_exact_in(&pool, &req), Err(ArbError::Overflow)),
        "output exceeding u64::MAX must return ArbError::Overflow"
    );
}

/// Boundary Case 2: Zero reserves fails closed with InsufficientLiquidity.
#[test]
fn test_pumpswap_boundary_zero_reserves_fails_closed() {
    let engine = PumpSwapQuoteEngine::new();

    // Sub-case A: reserve_in = 0
    let (pool_zero_in, base_in, quote_in) = make_pumpswap_pool(0, 10_000_000_000, true);
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
            assert!(msg.contains("PumpSwap pool has 0 reserves"));
        }
        other => panic!("expected InsufficientLiquidity, got {:?}", other),
    }

    // Sub-case B: reserve_out = 0
    let (pool_zero_out, base_out, quote_out) = make_pumpswap_pool(10_000_000_000, 0, true);
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
            assert!(msg.contains("PumpSwap pool has 0 reserves"));
        }
        other => panic!("expected InsufficientLiquidity, got {:?}", other),
    }
}

/// Boundary Case 3: Token-2022 100% transfer fee exhausts input and fails closed with ZeroAmount.
#[test]
fn test_pumpswap_boundary_token_2022_100_percent_transfer_fee() {
    let engine = PumpSwapQuoteEngine::new();
    let (pool, base, quote) = make_pumpswap_pool(10_000_000_000, 10_000_000_000, true);

    let fee_config_100_percent = TransferFeeConfig {
        transfer_fee_basis_points: 10_000, // 100% fee
        maximum_fee: u64::MAX,
    };

    let req = QuoteRequest {
        input_mint: base,
        output_mint: quote,
        amount: 1_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: Some(fee_config_100_percent),
        output_transfer_fee_config: None,
    };

    assert!(
        matches!(
            engine.quote_exact_in(&pool, &req),
            Err(ArbError::ZeroAmount)
        ),
        "100% transfer fee must exhaust input and return ZeroAmount"
    );
}

/// Boundary Case 4: quote_exact_out returns UnsupportedOperation on PumpSwap.
#[test]
fn test_pumpswap_boundary_exact_out_unsupported_operation() {
    let engine = PumpSwapQuoteEngine::new();
    let (pool, base, quote) = make_pumpswap_pool(10_000_000_000, 10_000_000_000, true);

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

/// Boundary Case 5: Large u64 input amount evaluates safely without arithmetic panic.
#[test]
fn test_pumpswap_boundary_max_u64_input_amount() {
    let engine = PumpSwapQuoteEngine::new();
    let (pool, base, quote) = make_pumpswap_pool(100_000_000_000, 100_000_000_000, true);

    // Large amount near u64::MAX - 1000
    let req = QuoteRequest {
        input_mint: base,
        output_mint: quote,
        amount: u64::MAX - 1000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };

    let quote = engine.quote_exact_in(&pool, &req).unwrap();
    assert!(
        quote.amount_out_raw <= 100_000_000_000,
        "amount out must be bounded by reserve_out"
    );
    assert!(quote.amount_out_raw > 0);
}
