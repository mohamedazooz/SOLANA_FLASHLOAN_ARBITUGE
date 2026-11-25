//! UniversalQuoteDispatcher contract tests.
//!
//! The dispatcher must emit a quote only when the snapshot contains the exact
//! curve/fee state consumed by that protocol implementation. Incomplete protocol
//! models fail closed instead of borrowing a generic constant-product formula.

use arb_dex::engines::UniversalQuoteDispatcher;
use arb_dex::quote_engine::QuoteRequest;
use arb_types::{
    ArbError, DexProtocol, MintId, PoolId, PoolState, PumpFunState, Slot, TokenProgramKind,
};
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
        reserve_in: 2_000_000_000,
        reserve_out: 40_000_000_000,
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

fn request(input_mint: MintId, output_mint: MintId, amount: u64) -> QuoteRequest {
    QuoteRequest {
        input_mint,
        output_mint,
        amount,
        slippage_bps: 50,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    }
}

#[test]
fn raydium_cpmm_and_v4_dispatch_with_reserve_state() {
    let dispatcher = UniversalQuoteDispatcher::new();
    for protocol in [DexProtocol::RaydiumCpmm, DexProtocol::RaydiumV4] {
        let (pool, base, quote) = create_test_pool(protocol, true);
        let result = dispatcher
            .quote_exact_in(&pool, &request(base, quote, 10_000_000))
            .expect("reserve-backed CPMM quote succeeds");
        assert_eq!(result.protocol, protocol);
        assert!(result.amount_out_raw > 0);
        assert!(result.quote_validity);
    }
}

#[test]
fn graduated_pump_fun_routes_to_pumpswap() {
    let dispatcher = UniversalQuoteDispatcher::new();
    let (mut pool, base, quote) = create_test_pool(DexProtocol::PumpFun, true);
    pool.pump_fun = Some(PumpFunState {
        virtual_token_reserves: 1_000_000_000_000,
        virtual_sol_reserves: 30_000_000_000,
        real_token_reserves: 793_100_000_000,
        real_sol_reserves: 30_000_000_000,
        token_total_supply: 1_000_000_000_000,
        complete: true,
        ..Default::default()
    });
    let result = dispatcher
        .quote_exact_in(&pool, &request(base, quote, 10_000_000))
        .expect("graduated curve routes to PumpSwap");
    assert_eq!(result.protocol, DexProtocol::PumpSwap);
    assert!(result.amount_out_raw > 0);
}

#[test]
fn incomplete_protocol_specific_states_fail_closed_instead_of_using_cpmm_fallbacks() {
    let dispatcher = UniversalQuoteDispatcher::new();
    let protocols = [
        DexProtocol::MeteoraDammV2,
        DexProtocol::MeteoraDbc,
        DexProtocol::RaydiumLaunchLab,
    ];
    for protocol in protocols {
        let (pool, base, quote) = create_test_pool(protocol, true);
        let result = dispatcher.quote_exact_in(&pool, &request(base, quote, 10_000_000));
        assert!(
            matches!(result, Err(ArbError::UnsupportedOperation(_))),
            "{protocol:?} must refuse an incomplete protocol-specific state: {result:?}"
        );
    }
}

#[test]
fn exact_out_is_only_enabled_for_models_with_verified_reverse_math() {
    let dispatcher = UniversalQuoteDispatcher::new();
    for protocol in [DexProtocol::RaydiumCpmm, DexProtocol::RaydiumV4] {
        let (pool, base, quote) = create_test_pool(protocol, true);
        let result = dispatcher
            .quote_exact_out(&pool, &request(base, quote, 5_000_000))
            .expect("implemented CPMM exact-out model succeeds");
        assert_eq!(result.protocol, protocol);
        assert_eq!(result.amount_out_raw, 5_000_000);
        assert!(result.amount_in_raw > 0);
    }
}

#[test]
fn exact_out_missing_protocol_math_is_explicitly_unsupported() {
    let dispatcher = UniversalQuoteDispatcher::new();
    let protocols = [
        DexProtocol::RaydiumClmm,
        DexProtocol::OrcaWhirlpool,
        DexProtocol::MeteoraDlmm,
        DexProtocol::MeteoraDammV2,
        DexProtocol::MeteoraDbc,
        DexProtocol::RaydiumLaunchLab,
        DexProtocol::PumpFun,
        DexProtocol::PumpSwap,
    ];
    for protocol in protocols {
        let (pool, base, quote) = create_test_pool(protocol, true);
        let result = dispatcher.quote_exact_out(&pool, &request(base, quote, 5_000_000));
        assert!(
            matches!(result, Err(ArbError::UnsupportedOperation(_))),
            "{protocol:?} exact-out must not use a guessed reverse formula: {result:?}"
        );
    }
}

#[test]
fn request_validation_rejects_invalid_slippage_and_foreign_output_mint() {
    let dispatcher = UniversalQuoteDispatcher::new();
    let (pool, base, quote) = create_test_pool(DexProtocol::RaydiumCpmm, true);
    let mut invalid_slippage = request(base, quote, 1_000_000);
    invalid_slippage.slippage_bps = 10_001;
    assert!(matches!(
        dispatcher.quote_exact_in(&pool, &invalid_slippage),
        Err(ArbError::InvalidInput(_))
    ));

    let foreign = MintId(Pubkey::new_unique());
    assert!(matches!(
        dispatcher.quote_exact_in(&pool, &request(base, foreign, 1_000_000)),
        Err(ArbError::InvalidToken(_))
    ));
}
