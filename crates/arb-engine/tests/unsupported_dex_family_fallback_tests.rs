//! Legacy route-graph adapter regression tests.
//!
//! These protocols must not inherit CPMM/CLMM formulas merely because a
//! normalized PoolState carries reserves or a CLMM-shaped state object.

use arb_engine::{
    HopRequest, execution_capability, protocol_quote_adapter, quote_hop, quote_route,
};
use arb_types::{DexProtocol, MintId, PoolId, PoolState, Slot, TokenProgramKind};
use solana_pubkey::Pubkey;

fn pool(protocol: DexProtocol) -> PoolState {
    PoolState {
        pool_id: PoolId::new([protocol as u8 + 1; 32]),
        protocol,
        slot: Slot(200),
        write_version: 1,
        base_mint: MintId(Pubkey::new_unique()),
        quote_mint: MintId(Pubkey::new_unique()),
        decimals_a: 6,
        decimals_b: 9,
        reserve_in: 2_000_000_000,
        reserve_out: 40_000_000_000,
        fee_numerator: 25,
        fee_denominator: 10_000,
        is_active: true,
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
    }
}

#[test]
fn legacy_family_adapters_cannot_quote_the_four_unverified_venues() {
    let protocols = [
        DexProtocol::MeteoraDammV2,
        DexProtocol::MeteoraDbc,
        DexProtocol::MeteoraDlmmPro,
        DexProtocol::RaydiumLaunchLab,
    ];

    for protocol in protocols {
        let mut state = pool(protocol);
        let before = (state.reserve_in, state.reserve_out);

        assert!(
            execution_capability(&state).is_err(),
            "{protocol:?} must remain quote-only"
        );

        let adapter = protocol_quote_adapter(protocol);
        assert_eq!(adapter.protocol(), protocol);
        assert!(
            adapter
                .quote_curve(&mut state.clone(), 10_000, true)
                .is_err(),
            "{protocol:?} must not inherit a generic family quote"
        );

        assert!(
            quote_hop(&mut state, 10_000, true).is_err(),
            "{protocol:?} must fail before generic quote math"
        );
        assert_eq!((state.reserve_in, state.reserve_out), before);

        let route_state = pool(protocol);
        let hops = [HopRequest {
            pool_id: route_state.pool_id,
            mint_in: route_state.base_mint,
        }];
        let mut route_pools = [route_state];
        assert!(
            quote_route(&mut route_pools, &hops, 10_000).is_err(),
            "{protocol:?} must fail closed through quote_route too"
        );
    }
}
