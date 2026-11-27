//! Unit tests for PumpSwap Independent Quote Engine & Protocol ID Resolution (Feature 14, Tier 1).
//!
//! Validates:
//! 1. Protocol ID resolves to `DexProtocol::PumpSwap` (never `DexProtocol::RaydiumCpmm`).
//! 2. Exact-in sell quotation (Base -> Quote) derives exact constant product output and fees.
//! 3. Exact-in buy quotation (Quote -> Base) derivates directional output without overflow.
//! 4. Inactive or disabled pool immediately fails closed (`ArbError::PoolDisabled`).
//! 5. Unrecognized foreign mint immediately fails closed (`ArbError::InvalidToken`).

use arb_dex::engines::PumpSwapQuoteEngine;
use arb_dex::quote_engine::{QuoteEngine, QuoteRequest};
use arb_types::{
    ArbError, DexProtocol, MintId, PoolId, PoolKind, PoolState, Slot, TokenProgramKind,
};
use solana_pubkey::Pubkey;

fn create_sample_pumpswap_pool(is_active: bool) -> (PoolState, MintId, MintId) {
    let base_mint = MintId(Pubkey::new_unique());
    let quote_mint = MintId(Pubkey::new_unique());
    let pool_state = PoolState {
        pool_id: PoolId::new([0x55; 32]),
        protocol: DexProtocol::PumpSwap,
        slot: Slot(100),
        write_version: 1,
        base_mint,
        quote_mint,
        decimals_a: 6,
        decimals_b: 9,
        reserve_in: 1_000_000_000, // Base token reserve (e.g. 1,000 tokens)
        reserve_out: 50_000_000_000, // Quote token reserve (e.g. 50 SOL)
        fee_numerator: 3_000,
        fee_denominator: 1_000_000,
        raydium_cpmm_fee: None,
        is_active,
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
    (pool_state, base_mint, quote_mint)
}

#[test]
fn test_pumpswap_quote_engine_protocol_id_resolution() {
    let engine = PumpSwapQuoteEngine::new();
    assert_eq!(engine.protocol_id(), DexProtocol::PumpSwap);
    assert_ne!(engine.protocol_id(), DexProtocol::RaydiumCpmm);
}

#[test]
fn test_pumpswap_quote_engine_fails_closed_without_fee_ratio() {
    let engine = PumpSwapQuoteEngine::new();
    let (mut pool, base_mint, quote_mint) = create_sample_pumpswap_pool(true);
    pool.fee_denominator = 0;
    let request = QuoteRequest {
        input_mint: base_mint,
        output_mint: quote_mint,
        amount: 10_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    assert!(engine.quote_exact_in(&pool, &request).is_err());
}
#[test]
fn test_pumpswap_quote_preserves_exact_sub_bps_fee_ratio() {
    let engine = PumpSwapQuoteEngine::new();
    let (mut pool, base_mint, quote_mint) = create_sample_pumpswap_pool(true);
    pool.fee_numerator = 50; // 50 ppm = 0.5 bps
    pool.fee_denominator = 1_000_000;
    let request = QuoteRequest {
        input_mint: quote_mint,
        output_mint: base_mint,
        amount: 10_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    let result = engine
        .quote_exact_in(&pool, &request)
        .expect("exact ppm quote");
    assert_eq!(result.fees_breakdown.total_fee, 502);
    assert!(result.price_impact.0 > 0);
}
#[test]
fn test_pumpswap_sell_fee_is_charged_on_quote_output() {
    let engine = PumpSwapQuoteEngine::new();
    let (mut pool, base_mint, quote_mint) = create_sample_pumpswap_pool(true);
    pool.fee_numerator = 50; // 50 ppm = 0.5 bps
    pool.fee_denominator = 1_000_000;
    let request = QuoteRequest {
        input_mint: base_mint,
        output_mint: quote_mint,
        amount: 10_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };
    let result = engine.quote_exact_in(&pool, &request).expect("sell quote");
    let gross_out = (pool.reserve_out * 10_000_000u128) / (pool.reserve_in + 10_000_000u128);
    let expected_fee = ((gross_out * 50).div_ceil(1_000_000)) + 2;
    assert_eq!(u128::from(result.fees_breakdown.total_fee), expected_fee);
}
#[test]
fn test_pumpswap_quote_engine_quote_exact_in_sell_base() {
    let engine = PumpSwapQuoteEngine::new();
    let (pool, base_mint, quote_mint) = create_sample_pumpswap_pool(true);

    let request = QuoteRequest {
        input_mint: base_mint,
        output_mint: quote_mint,
        amount: 10_000_000, // 10 tokens in
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };

    let result = engine
        .quote_exact_in(&pool, &request)
        .expect("quote succeeds");
    assert_eq!(
        result.fees_breakdown.lp_fee
            + result.fees_breakdown.protocol_fee
            + result.fees_breakdown.fund_fee
            + result.fees_breakdown.creator_fee,
        0,
        "generic PoolState does not contain verified PumpSwap fee attribution"
    );
    assert!(result.quote_validity);
    assert!(result.fees_breakdown.total_fee > 0);
}

#[test]
fn test_pumpswap_quote_engine_quote_exact_in_buy_base() {
    let engine = PumpSwapQuoteEngine::new();
    let (pool, base_mint, quote_mint) = create_sample_pumpswap_pool(true);

    let request = QuoteRequest {
        input_mint: quote_mint,
        output_mint: base_mint,
        amount: 500_000_000, // 0.5 SOL in
        slippage_bps: 100,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };

    let result = engine
        .quote_exact_in(&pool, &request)
        .expect("buy quote succeeds");
    assert_eq!(result.protocol, DexProtocol::PumpSwap);
    assert!(result.amount_out_raw > 0);
    assert!(result.amount_out_raw < pool.reserve_in as u64);
    assert!(result.quote_validity);
    assert!(result.fees_breakdown.total_fee > 0);
}

#[test]
fn test_pumpswap_quote_engine_disabled_pool_fails_closed() {
    let engine = PumpSwapQuoteEngine::new();
    let (pool, base_mint, quote_mint) = create_sample_pumpswap_pool(false); // is_active = false

    let request = QuoteRequest {
        input_mint: base_mint,
        output_mint: quote_mint,
        amount: 10_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };

    let res = engine.quote_exact_in(&pool, &request);
    assert!(matches!(res, Err(ArbError::PoolDisabled(_))));
}

#[test]
fn test_pumpswap_quote_engine_mismatched_mint_fails_closed() {
    let engine = PumpSwapQuoteEngine::new();
    let (pool, _base_mint, quote_mint) = create_sample_pumpswap_pool(true);

    let foreign_mint = MintId(Pubkey::new_unique());
    let request = QuoteRequest {
        input_mint: foreign_mint,
        output_mint: quote_mint,
        amount: 10_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };

    let res = engine.quote_exact_in(&pool, &request);
    assert!(matches!(res, Err(ArbError::InvalidToken(_))));
}
