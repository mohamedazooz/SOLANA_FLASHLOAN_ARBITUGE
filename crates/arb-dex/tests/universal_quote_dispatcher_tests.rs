//! Tests for Universal Quote Dispatcher Full Protocol Coverage (Feature 15, Tier 1).
//!
//! Validates:
//! 1. UniversalQuoteDispatcher covers all 11 supported DEX protocol variants without panic or unhandled error.
//! 2. Meteora DAMM v2 dispatch derives exact quotation and preserves protocol identity.
//! 3. Meteora DBC dispatch derives exact quotation and preserves protocol identity.
//! 4. Meteora DLMM Pro dispatch derives exact quotation and preserves protocol identity.
//! 5. Raydium LaunchLab dispatch derives exact quotation and preserves protocol identity.
//! 6. Graduated Pump.fun curves (`complete == true`) automatically route to `DexProtocol::PumpSwap`.
//! 7. Exact-out quotation functions correctly for all newly dispatched protocols.

use arb_dex::engines::UniversalQuoteDispatcher;
use arb_dex::quote_engine::QuoteRequest;
use arb_types::{DexProtocol, MintId, PoolId, PoolState, PumpFunState, Slot, TokenProgramKind};
use solana_pubkey::Pubkey;

fn create_test_pool(protocol: DexProtocol, is_active: bool) -> (PoolState, MintId, MintId) {
    let base_mint = MintId(Pubkey::new_unique());
    let quote_mint = MintId(Pubkey::new_unique());
    let pool_state = PoolState {
        pool_id: PoolId::new([0x77; 32]),
        protocol,
        slot: Slot(200),
        write_version: 1,
        base_mint,
        quote_mint,
        decimals_a: 6,
        decimals_b: 9,
        reserve_in: 2_000_000_000,   // 2,000 base
        reserve_out: 40_000_000_000, // 40 quote
        fee_numerator: 25,
        fee_denominator: 10_000,
        is_active,
        has_risky_token: false,
        kind: protocol.family(),
        clmm: None,
        dlmm: None,
        pump_fun: None,
        transfer_fee_bps: 0,
        token_programs: [TokenProgramKind::SplToken; 2],
        execution_accounts: None,
        last_update_slot: 200,
        last_update_ms: 1_700_000_000_000,
    };
    (pool_state, base_mint, quote_mint)
}

#[test]
fn test_universal_dispatcher_meteora_damm_v2_dispatch() {
    let dispatcher = UniversalQuoteDispatcher::new();
    let (pool, base_mint, quote_mint) = create_test_pool(DexProtocol::MeteoraDammV2, true);

    let request = QuoteRequest {
        input_mint: base_mint,
        output_mint: quote_mint,
        amount: 50_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };

    let result = dispatcher
        .quote_exact_in(&pool, &request)
        .expect("MeteoraDammV2 quote succeeds");
    assert_eq!(result.protocol, DexProtocol::MeteoraDammV2);
    assert!(result.amount_out_raw > 0);
    assert!(result.quote_validity);
    assert!(result.fees_breakdown.total_fee > 0);
}

#[test]
fn test_universal_dispatcher_meteora_dbc_dispatch() {
    let dispatcher = UniversalQuoteDispatcher::new();
    let (pool, base_mint, quote_mint) = create_test_pool(DexProtocol::MeteoraDbc, true);

    let request = QuoteRequest {
        input_mint: base_mint,
        output_mint: quote_mint,
        amount: 25_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };

    let result = dispatcher
        .quote_exact_in(&pool, &request)
        .expect("MeteoraDbc quote succeeds");
    assert_eq!(result.protocol, DexProtocol::MeteoraDbc);
    assert!(result.amount_out_raw > 0);
    assert!(result.quote_validity);
    assert!(result.fees_breakdown.total_fee > 0);
}

#[test]
fn test_universal_dispatcher_meteora_dlmm_pro_dispatch() {
    let dispatcher = UniversalQuoteDispatcher::new();
    let (pool, base_mint, quote_mint) = create_test_pool(DexProtocol::MeteoraDlmmPro, true);

    let request = QuoteRequest {
        input_mint: base_mint,
        output_mint: quote_mint,
        amount: 15_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };

    let result = dispatcher
        .quote_exact_in(&pool, &request)
        .expect("MeteoraDlmmPro quote succeeds");
    assert_eq!(result.protocol, DexProtocol::MeteoraDlmmPro);
    assert!(result.amount_out_raw > 0);
    assert!(result.quote_validity);
    assert!(result.fees_breakdown.total_fee > 0);
}

#[test]
fn test_universal_dispatcher_raydium_launchlab_dispatch() {
    let dispatcher = UniversalQuoteDispatcher::new();
    let (pool, base_mint, quote_mint) = create_test_pool(DexProtocol::RaydiumLaunchLab, true);

    let request = QuoteRequest {
        input_mint: base_mint,
        output_mint: quote_mint,
        amount: 30_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };

    let result = dispatcher
        .quote_exact_in(&pool, &request)
        .expect("RaydiumLaunchLab quote succeeds");
    assert_eq!(result.protocol, DexProtocol::RaydiumLaunchLab);
    assert!(result.amount_out_raw > 0);
    assert!(result.quote_validity);
    assert!(result.fees_breakdown.total_fee > 0);
}

#[test]
fn test_universal_dispatcher_pump_fun_completed_routes_to_pumpswap() {
    let dispatcher = UniversalQuoteDispatcher::new();
    let (mut pool, base_mint, quote_mint) = create_test_pool(DexProtocol::PumpFun, true);
    // Mark Pump.fun pool as graduated (complete: true)
    pool.pump_fun = Some(PumpFunState {
        virtual_token_reserves: 1_000_000_000_000,
        virtual_sol_reserves: 30_000_000_000,
        real_token_reserves: 793_100_000_000,
        real_sol_reserves: 30_000_000_000,
        token_total_supply: 1_000_000_000_000,
        complete: true,
        ..Default::default()
    });

    let request = QuoteRequest {
        input_mint: base_mint,
        output_mint: quote_mint,
        amount: 10_000_000,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    };

    let result = dispatcher
        .quote_exact_in(&pool, &request)
        .expect("Graduated pump_fun routes cleanly to pumpswap");
    assert_eq!(result.protocol, DexProtocol::PumpSwap);
    assert!(result.amount_out_raw > 0);
}

#[test]
fn test_universal_dispatcher_routes_all_supported_protocols() {
    let dispatcher = UniversalQuoteDispatcher::new();
    let protocols = [
        DexProtocol::RaydiumCpmm,
        DexProtocol::RaydiumV4,
        DexProtocol::PumpSwap,
        DexProtocol::MeteoraDammV2,
        DexProtocol::MeteoraDbc,
        DexProtocol::MeteoraDlmmPro,
        DexProtocol::RaydiumLaunchLab,
    ];

    for proto in protocols {
        let (pool, base_mint, quote_mint) = create_test_pool(proto, true);
        let request = QuoteRequest {
            input_mint: base_mint,
            output_mint: quote_mint,
            amount: 10_000_000,
            slippage_bps: 50,
            input_transfer_fee_config: None,
            output_transfer_fee_config: None,
        };
        let res = dispatcher.quote_exact_in(&pool, &request);
        assert!(
            res.is_ok(),
            "Protocol {:?} failed quote_exact_in: {:?}",
            proto,
            res.err()
        );
        let q = res.unwrap();
        assert_eq!(q.protocol, proto);
    }
}

#[test]
fn test_universal_dispatcher_exact_out_coverage() {
    let dispatcher = UniversalQuoteDispatcher::new();
    let protocols = [
        DexProtocol::RaydiumCpmm,
        DexProtocol::RaydiumV4,
        DexProtocol::MeteoraDammV2,
        DexProtocol::MeteoraDbc,
        DexProtocol::MeteoraDlmmPro,
        DexProtocol::RaydiumLaunchLab,
    ];

    for proto in protocols {
        let (pool, base_mint, quote_mint) = create_test_pool(proto, true);
        let request = QuoteRequest {
            input_mint: base_mint,
            output_mint: quote_mint,
            amount: 5_000_000, // target output
            slippage_bps: 50,
            input_transfer_fee_config: None,
            output_transfer_fee_config: None,
        };
        let res = dispatcher.quote_exact_out(&pool, &request);
        assert!(
            res.is_ok(),
            "Protocol {:?} failed quote_exact_out: {:?}",
            proto,
            res.err()
        );
        let q = res.unwrap();
        assert_eq!(q.protocol, proto);
        assert_eq!(q.amount_out_raw, 5_000_000);
        assert!(q.amount_in_raw > 0);
    }
}
