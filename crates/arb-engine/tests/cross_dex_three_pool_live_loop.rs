//! Cross-DEX three-pool live-loop coverage: a WSOL -> A -> B -> WSOL cycle
//! across THREE distinct protocols (Meteora DLMM -> Raydium CPMM -> Raydium V4)
//! must be discovered by the same `MarketGraph::find_multi_hop_cycles` path the
//! Supervisor uses, quoted per-protocol by the adapter registry, and carried
//! with correct per-hop protocol/mint metadata into `ExecutionOpportunity`.
//!
//! This is the test backing the "cross-DEX 3-pool live loop" readiness item:
//! the specialized 3-leg optimizer remains DLMM-only, but the unified graph
//! path discovers, sizes, and prices mixed-protocol 3-pool cycles through the
//! single execution pipeline.

mod common;

use arb_engine::{MarketGraph, quote_hop};
use arb_math::economic::EconomicConfig;
use arb_types::{
    ClmmState, ClmmTickArray, DexProtocol, Lamports, MintId, PoolId, PoolKind, PoolState, Slot,
    TokenAmount,
};
use common::{make_dlmm_pool, meme_a_mint, meme_b_mint, wsol_mint};
use solana_pubkey::Pubkey;

fn economic_config() -> EconomicConfig {
    EconomicConfig {
        flash_loan_fee_sf: 0,
        jito_tip_lamports: Lamports::ZERO,
        base_decimals: 9,
        lamports_to_base_mint_rate: 1,
        lamports_to_base_rate_denom: 1,
        min_profit_raw: TokenAmount::ZERO,
    }
}

fn cpmm_pool(id: u8, a: MintId, b: MintId, reserve_out: u128) -> arb_types::PoolState {
    arb_types::PoolState {
        pool_id: arb_types::PoolId::new([id; 32]),
        protocol: DexProtocol::RaydiumCpmm,
        slot: Slot(467_000_000),
        write_version: 1,
        base_mint: a,
        quote_mint: b,
        decimals_a: 9,
        decimals_b: 9,
        reserve_in: 10_000_000_000,
        reserve_out,
        fee_numerator: 25,
        fee_denominator: 10_000,
        is_active: true,
        has_risky_token: false,
        kind: arb_types::PoolKind::Cpmm,
        clmm: None,
        dlmm: None,
        pump_fun: None,
        transfer_fee_bps: 0,
        token_programs: Default::default(),
        execution_accounts: None,
        last_update_slot: 467_000_000,
        last_update_ms: 1_700_000_000_000,
    }
}

fn v4_pool(id: u8, a: MintId, b: MintId, reserve_out: u128) -> arb_types::PoolState {
    arb_types::PoolState {
        pool_id: arb_types::PoolId::new([id; 32]),
        protocol: DexProtocol::RaydiumV4,
        slot: Slot(467_000_000),
        write_version: 1,
        base_mint: a,
        quote_mint: b,
        decimals_a: 9,
        decimals_b: 9,
        reserve_in: 10_000_000_000,
        reserve_out,
        fee_numerator: 25,
        fee_denominator: 10_000,
        is_active: true,
        has_risky_token: false,
        kind: arb_types::PoolKind::Cpmm,
        clmm: None,
        dlmm: None,
        pump_fun: None,
        transfer_fee_bps: 0,
        token_programs: Default::default(),
        execution_accounts: None,
        last_update_slot: 467_000_000,
        last_update_ms: 1_700_000_000_000,
    }
}

/// WSOL -> MemeA on Meteora DLMM, MemeA -> MemeB on Raydium CPMM, MemeB -> WSOL
/// on Raydium V4, with the final leg priced rich enough for a positive cycle.
fn cross_dex_three_pool_fixture() -> Vec<arb_types::PoolState> {
    let wsol = wsol_mint();
    let ma = meme_a_mint();
    let mb = meme_b_mint();
    vec![
        make_dlmm_pool(1, -100, 10, 100, wsol, ma, 10_u128.pow(12)),
        cpmm_pool(2, ma, mb, 10_000_000_000),
        // Final leg returns 12% more WSOL per MemeB than parity: profit,
        // mirroring the `graph_4hop_multi_detection` fixture pattern.
        v4_pool(3, mb, wsol, 11_200_000_000),
    ]
}

#[test]
fn cross_dex_three_pool_cycle_discovered_with_per_protocol_hops() {
    let wsol = wsol_mint();
    let ma = meme_a_mint();
    let mb = meme_b_mint();
    let pools = cross_dex_three_pool_fixture();

    let mut graph = MarketGraph::new(vec![wsol]).with_economic_config(economic_config());
    graph.build_from_pools(&pools);

    let opps = graph.find_multi_hop_cycles(Slot(467_000_001), 1_000_000_000, 4);
    let three_hop = opps
        .iter()
        .find(|o| o.hops.len() == 3)
        .expect("cross-DEX 3-pool cycle must be discovered by the unified graph path");

    // Mint continuity: WSOL -> A -> B -> WSOL.
    assert_eq!(three_hop.hops[0].mint_in, wsol);
    assert_eq!(three_hop.hops[0].mint_out, ma);
    assert_eq!(three_hop.hops[1].mint_in, ma);
    assert_eq!(three_hop.hops[1].mint_out, mb);
    assert_eq!(three_hop.hops[2].mint_in, mb);
    assert_eq!(three_hop.hops[2].mint_out, wsol);

    // One distinct protocol per hop — genuinely cross-DEX.
    let protocols: Vec<_> = three_hop.hops.iter().map(|h| h.protocol).collect();
    assert_eq!(
        protocols,
        vec![
            DexProtocol::MeteoraDlmm,
            DexProtocol::RaydiumCpmm,
            DexProtocol::RaydiumV4
        ]
    );
    assert_eq!(three_hop.hops[0].pool_id.0, pools[0].pool_id.0);
    assert_eq!(three_hop.hops[1].pool_id.0, pools[1].pool_id.0);
    assert_eq!(three_hop.hops[2].pool_id.0, pools[2].pool_id.0);

    // Positive gross edge at the optimized size.
    assert!(three_hop.gross_profit.as_u64() > 0);
}

#[test]
fn cross_dex_three_pool_hop_quotes_chain_through_adapter_registry() {
    let wsol = wsol_mint();
    let ma = meme_a_mint();
    let mb = meme_b_mint();
    let mut pools = cross_dex_three_pool_fixture();

    // Per-protocol adapter quoting must chain WSOL -> A -> B -> WSOL with a
    // positive final output, matching the graph's curve model per hop.
    let out1 = quote_hop(&mut pools[0], 1_000_000_000, true).expect("DLMM hop quote");
    assert_eq!(out1.protocol, DexProtocol::MeteoraDlmm);
    assert_eq!(out1.mint_in, wsol);
    assert_eq!(out1.mint_out, ma);
    assert!(out1.amount_out > 0);

    let out2 = quote_hop(&mut pools[1], out1.amount_out, true).expect("CPMM hop quote");
    assert_eq!(out2.protocol, DexProtocol::RaydiumCpmm);
    assert_eq!(out2.mint_in, ma);
    assert_eq!(out2.mint_out, mb);
    assert!(out2.amount_out > 0);

    let out3 = quote_hop(&mut pools[2], out2.amount_out, true).expect("V4 hop quote");
    assert_eq!(out3.protocol, DexProtocol::RaydiumV4);
    assert_eq!(out3.mint_in, mb);
    assert_eq!(out3.mint_out, wsol);
    assert!(out3.amount_out > 0);
}

fn meme_c_mint() -> MintId {
    MintId(Pubkey::new_from_array([92u8; 32]))
}

/// Orca Whirlpool fixture at price 1.0 (sqrt_price = 2^64) with the active-tick
/// tick arrays supplied, mirroring the adapter-test fixture pattern.
fn orca_clmm_pool(id: u8, a: MintId, b: MintId) -> PoolState {
    PoolState {
        pool_id: PoolId::new([id; 32]),
        protocol: DexProtocol::OrcaWhirlpool,
        slot: Slot(467_000_000),
        write_version: 1,
        base_mint: a,
        quote_mint: b,
        decimals_a: 9,
        decimals_b: 9,
        reserve_in: 1_000_000_000_000,
        reserve_out: 1_000_000_000_000,
        fee_numerator: 30,
        fee_denominator: 10_000,
        is_active: true,
        has_risky_token: false,
        kind: PoolKind::Clmm,
        clmm: Some(ClmmState {
            sqrt_price: 1u128 << 64,
            liquidity: 1_000_000_000_000,
            tick_current: 0,
            tick_spacing: 4,
            fee_rate: 3_000,
            tick_arrays: vec![
                ClmmTickArray {
                    start_tick_index: -88 * 4,
                    ticks: vec![],
                },
                ClmmTickArray {
                    start_tick_index: 0,
                    ticks: vec![],
                },
            ],
        }),
        dlmm: None,
        pump_fun: None,
        transfer_fee_bps: 0,
        token_programs: Default::default(),
        execution_accounts: None,
        last_update_slot: 467_000_000,
        last_update_ms: 1_700_000_000_000,
    }
}

/// ALL executable platforms connected in ONE four-pool cycle:
/// WSOL -DLMM-> A -CPMM-> B -OrcaCLMM-> C -RaydiumV4-> WSOL. The unified graph
/// path must enumerate mixed-protocol routes and carry each hop's own protocol
/// and per-pool fee into the opportunity.
#[test]
fn cross_protocol_four_pool_cycle_connects_all_platforms() {
    let wsol = wsol_mint();
    let ma = meme_a_mint();
    let mb = meme_b_mint();
    let mc = meme_c_mint();
    let pools = vec![
        make_dlmm_pool(1, -100, 10, 100, wsol, ma, 10_u128.pow(12)),
        cpmm_pool(2, ma, mb, 10_000_000_000),
        orca_clmm_pool(4, mb, mc),
        v4_pool(3, mc, wsol, 11_200_000_000),
    ];

    let mut graph = MarketGraph::new(vec![wsol]).with_economic_config(economic_config());
    graph.build_from_pools(&pools);

    let opps = graph.find_multi_hop_cycles(Slot(467_000_001), 1_000_000_000, 4);
    let four_hop = opps
        .iter()
        .find(|o| o.hops.len() == 4)
        .expect("4-protocol cycle must be discovered by the unified graph path");

    let protocols: Vec<_> = four_hop.hops.iter().map(|h| h.protocol).collect();
    assert_eq!(
        protocols,
        vec![
            DexProtocol::MeteoraDlmm,
            DexProtocol::RaydiumCpmm,
            DexProtocol::OrcaWhirlpool,
            DexProtocol::RaydiumV4
        ]
    );
    assert_eq!(four_hop.hops[0].mint_in, wsol);
    assert_eq!(four_hop.hops[3].mint_out, wsol);
    assert!(four_hop.gross_profit.as_u64() > 0);
}

/// Fee awareness: with two CPMM pools between the same mints (cheap 25 bps vs
/// expensive 200 bps, both individually profitable inside the cycle), both
/// cycle variants are explored (multigraph) and the cheap pool MUST rank first
/// by net profit — per-pool fees drive ranking.
#[test]
fn per_pool_fee_determines_route_preference_between_parallel_pools() {
    let wsol = wsol_mint();
    let ma = meme_a_mint();
    let mb = meme_b_mint();
    let mut cheap = cpmm_pool(2, ma, mb, 10_000_000_000);
    cheap.fee_numerator = 25;
    let mut expensive = cpmm_pool(5, ma, mb, 10_000_000_000);
    expensive.fee_numerator = 200;
    let pools = vec![
        make_dlmm_pool(1, -100, 10, 100, wsol, ma, 10_u128.pow(12)),
        cheap,
        expensive,
        // Larger premium so BOTH variants clear the 1.0010 marginal-rate gate.
        v4_pool(3, mb, wsol, 12_500_000_000),
    ];

    let mut graph = MarketGraph::new(vec![wsol]).with_economic_config(economic_config());
    graph.build_from_pools(&pools);

    let opps = graph.find_multi_hop_cycles(Slot(467_000_001), 1_000_000_000, 4);
    let three_hops: Vec<_> = opps.iter().filter(|o| o.hops.len() == 3).collect();
    assert!(
        three_hops.len() >= 2,
        "both parallel-pool cycle variants must be explored"
    );
    assert_eq!(
        three_hops[0].hops[1].pool_id.0,
        Pubkey::new_from_array([2u8; 32]),
        "top-ranked 3-hop cycle must route through the cheap (25 bps) pool"
    );
    assert!(three_hops[0].net_profit.0 > three_hops[1].net_profit.0);
}

/// Per-pool fee accounting: each protocol's HopQuote must charge that pool's
/// own configured fee, not a global average.
#[test]
fn hop_quotes_charge_each_pool_own_fee() {
    let wsol = wsol_mint();
    let ma = meme_a_mint();
    let mut cheap = cpmm_pool(2, wsol, ma, 10_000_000_000);
    cheap.fee_numerator = 100;
    let q = quote_hop(&mut cheap, 10_000_000, true).expect("CPMM quote");
    let expected = 10_000_000u64 * 100 / 10_000;
    assert_eq!(
        q.fee_paid, expected,
        "CPMM fee must be the pool's own 100 bps"
    );

    let mut clmm = orca_clmm_pool(4, wsol, ma);
    let qc = quote_hop(&mut clmm, 10_000_000, true).expect("CLMM quote");
    assert!(
        qc.fee_paid > 0 && qc.fee_paid <= 10_000_000 * 30 / 10_000,
        "CLMM fee must reflect the Whirlpool native fee rate"
    );
}
