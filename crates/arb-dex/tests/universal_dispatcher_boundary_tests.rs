//! Feature 15: Universal Quote Dispatcher Full Protocol Coverage — Tier 2 Boundary Tests.
//!
//! Validates boundary handling across all 10 supported protocols:
//! 1. Invalid input token mint fails closed across all 10 protocols.
//! 2. Disabled pool (is_active = false) fails closed across all 10 protocols.
//! 3. Zero amount input fails closed across all 10 protocols.
//! 4. Exact-out exceeding reserves fails closed on standard protocols.
//! 5. Exact-out unsupported operation fails closed on Clmm, PumpFun, PumpSwap.

use arb_dex::engines::UniversalQuoteDispatcher;
use arb_dex::quote_engine::QuoteRequest;
use arb_types::pool::{ClmmState, DlmmState, PumpFunState};
use arb_types::{ArbError, DexProtocol, MintId, PoolId, PoolState, Slot, TokenProgramKind};
use solana_pubkey::Pubkey;

const ALL_10_PROTOCOLS: [DexProtocol; 10] = [
    DexProtocol::RaydiumV4,
    DexProtocol::RaydiumCpmm,
    DexProtocol::RaydiumClmm,
    DexProtocol::OrcaWhirlpool,
    DexProtocol::MeteoraDlmm,
    DexProtocol::MeteoraDammV2,
    DexProtocol::MeteoraDbc,
    DexProtocol::RaydiumLaunchLab,
    DexProtocol::PumpFun,
    DexProtocol::PumpSwap,
];

fn create_protocol_test_pool(
    protocol: DexProtocol,
    is_active: bool,
) -> (PoolState, MintId, MintId) {
    let base_mint = MintId(Pubkey::new_unique());
    let quote_mint = MintId(Pubkey::new_unique());

    let clmm = if matches!(
        protocol,
        DexProtocol::RaydiumClmm | DexProtocol::OrcaWhirlpool
    ) {
        Some(ClmmState {
            sqrt_price: 1u128 << 64,
            tick_current: 0,
            liquidity: 10_000_000_000,
            tick_spacing: 64,
            fee_rate: 2500,
            tick_arrays: vec![],
        })
    } else {
        None
    };

    let dlmm = if matches!(protocol, DexProtocol::MeteoraDlmm) {
        Some(DlmmState {
            active_bin_id: 0,
            status: 0,
            bin_step_bps: 10,
            params: Default::default(),
            volatility_accumulator: 0,
            volatility_reference: 0,
            index_reference: 0,
            last_update_timestamp: 0,
            bins: vec![],
            support_limit_order: false,
        })
    } else {
        None
    };

    let pump_fun = if matches!(protocol, DexProtocol::PumpFun) {
        Some(PumpFunState {
            virtual_token_reserves: 1_000_000_000_000,
            virtual_sol_reserves: 30_000_000_000,
            real_token_reserves: 793_100_000_000,
            real_sol_reserves: 30_000_000_000,
            token_total_supply: 1_000_000_000_000_000,
            complete: false,
            creator: Pubkey::new_unique(),
            is_mayhem_mode: false,
            is_cashback_coin: false,
            mode_flags_present: true,
        })
    } else {
        None
    };

    let pool_state = PoolState {
        pool_id: PoolId::new([0x88; 32]),
        protocol,
        slot: Slot(200),
        write_version: 1,
        base_mint,
        quote_mint,
        decimals_a: 6,
        decimals_b: 9,
        reserve_in: 10_000_000_000,
        reserve_out: 10_000_000_000,
        fee_numerator: if protocol == DexProtocol::RaydiumCpmm {
            2_500
        } else {
            25
        },
        fee_denominator: if protocol == DexProtocol::RaydiumCpmm {
            1_000_000
        } else {
            10_000
        },
        raydium_cpmm_fee: (protocol == DexProtocol::RaydiumCpmm).then_some(
            arb_types::RaydiumCpmmFeeState {
                trade_fee_rate_ppm: 2_500,
                creator_fee_rate_ppm: 0,
                protocol_fee_rate: 120_000,
                fund_fee_rate: 40_000,
                enable_creator_fee: false,
                creator_fee_on: 0,
            },
        ),
        is_active,
        has_risky_token: false,
        kind: protocol.family(),
        clmm,
        dlmm,
        pump_fun,
        transfer_fee_bps: 0,
        token_programs: [TokenProgramKind::SplToken; 2],
        execution_accounts: None,
        last_update_slot: 200,
        last_update_ms: 1_700_000_000_000,
    };
    (pool_state, base_mint, quote_mint)
}

/// Boundary Case 1: Invalid input token mint fails closed across all 10 protocols.
#[test]
fn test_universal_dispatcher_boundary_invalid_token_mint_all_10_protocols() {
    let dispatcher = UniversalQuoteDispatcher::new();
    let foreign_mint = MintId(Pubkey::new_unique());

    for protocol in ALL_10_PROTOCOLS {
        let (pool, _, _) = create_protocol_test_pool(protocol, true);
        let req = QuoteRequest {
            input_mint: foreign_mint,
            output_mint: MintId(Pubkey::default()),
            amount: 1_000_000,
            slippage_bps: 50,
            input_transfer_fee_config: None,
            output_transfer_fee_config: None,
        };
        let res = dispatcher.quote_exact_in(&pool, &req);
        match res {
            Err(ArbError::InvalidToken(msg)) => {
                assert!(
                    msg.contains("does not match") || msg.contains("not in pool"),
                    "protocol {:?} error: {}",
                    protocol,
                    msg
                );
            }
            other => panic!(
                "protocol {:?} expected InvalidToken, got {:?}",
                protocol, other
            ),
        }
    }
}

/// Boundary Case 2: Disabled pool (is_active = false) fails closed across all 10 protocols.
#[test]
fn test_universal_dispatcher_boundary_disabled_pool_all_10_protocols() {
    let dispatcher = UniversalQuoteDispatcher::new();

    for protocol in ALL_10_PROTOCOLS {
        let (pool, base, quote) = create_protocol_test_pool(protocol, false);
        let req = QuoteRequest {
            input_mint: base,
            output_mint: quote,
            amount: 1_000_000,
            slippage_bps: 50,
            input_transfer_fee_config: None,
            output_transfer_fee_config: None,
        };
        let res = dispatcher.quote_exact_in(&pool, &req);
        match res {
            Err(ArbError::PoolDisabled(msg)) => {
                assert!(
                    msg.contains("disabled"),
                    "protocol {:?} msg: {}",
                    protocol,
                    msg
                );
            }
            other => panic!(
                "protocol {:?} expected PoolDisabled, got {:?}",
                protocol, other
            ),
        }
    }
}

/// Boundary Case 3: Zero input amount fails closed with ZeroAmount across all 10 protocols.
#[test]
fn test_universal_dispatcher_boundary_zero_amount_all_10_protocols() {
    let dispatcher = UniversalQuoteDispatcher::new();

    for protocol in ALL_10_PROTOCOLS {
        let (pool, base, _) = create_protocol_test_pool(protocol, true);
        let req = QuoteRequest {
            input_mint: base,
            output_mint: MintId(Pubkey::default()),
            amount: 0,
            slippage_bps: 50,
            input_transfer_fee_config: None,
            output_transfer_fee_config: None,
        };
        let res = dispatcher.quote_exact_in(&pool, &req);
        assert!(
            matches!(res, Err(ArbError::ZeroAmount)),
            "protocol {:?} must return ZeroAmount, got {:?}",
            protocol,
            res
        );
    }
}

/// Boundary Case 4: Target output exceeding reserves in exact-out fails closed.
#[test]
fn test_universal_dispatcher_boundary_exact_out_exceeding_reserve() {
    let dispatcher = UniversalQuoteDispatcher::new();
    let exact_out_protocols = [DexProtocol::RaydiumV4, DexProtocol::RaydiumCpmm];

    for protocol in exact_out_protocols {
        let (pool, base, quote) = create_protocol_test_pool(protocol, true);
        let req = QuoteRequest {
            input_mint: base,
            output_mint: quote,
            amount: pool.reserve_out as u64 + 1, // Exceeds reserve_out
            slippage_bps: 50,
            input_transfer_fee_config: None,
            output_transfer_fee_config: None,
        };
        let res = dispatcher.quote_exact_out(&pool, &req);
        match res {
            Err(ArbError::InsufficientLiquidity(msg)) => {
                assert!(
                    msg.contains("exceeds pool reserve") || msg.contains("exceeds reserve"),
                    "protocol {:?} msg: {}",
                    protocol,
                    msg
                );
            }
            other => panic!(
                "protocol {:?} expected InsufficientLiquidity, got {:?}",
                protocol, other
            ),
        }
    }
}

/// Boundary Case 5: Exact out on unsupported protocols returns UnsupportedOperation.
#[test]
fn test_universal_dispatcher_boundary_exact_out_unsupported_protocols() {
    let dispatcher = UniversalQuoteDispatcher::new();
    let unsupported = [
        DexProtocol::RaydiumClmm,
        DexProtocol::PumpFun,
        DexProtocol::PumpSwap,
        DexProtocol::OrcaWhirlpool,
        DexProtocol::MeteoraDlmm,
        DexProtocol::MeteoraDammV2,
        DexProtocol::MeteoraDbc,
        DexProtocol::RaydiumLaunchLab,
    ];

    for protocol in unsupported {
        let (pool, base, quote) = create_protocol_test_pool(protocol, true);
        let req = QuoteRequest {
            input_mint: base,
            output_mint: quote,
            amount: 1_000_000,
            slippage_bps: 50,
            input_transfer_fee_config: None,
            output_transfer_fee_config: None,
        };
        let res = dispatcher.quote_exact_out(&pool, &req);
        match res {
            Err(ArbError::UnsupportedOperation(msg)) => {
                assert!(
                    msg.contains("not yet supported")
                        || msg.contains("lacks")
                        || msg.contains("not backed")
                        || msg.contains("disabled until"),
                    "protocol {:?} msg: {}",
                    protocol,
                    msg
                );
            }
            other => panic!(
                "protocol {:?} expected UnsupportedOperation, got {:?}",
                protocol, other
            ),
        }
    }
}
