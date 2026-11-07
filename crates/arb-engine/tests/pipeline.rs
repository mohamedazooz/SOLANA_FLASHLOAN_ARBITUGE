//! End-to-end checks on the pricing pipeline: pool state -> graph -> cycle ->
//! optimal size. These go through the public API exactly as the supervisor and
//! the sizing bench do, so a change in any layer that breaks the chain fails here
//! rather than only in a unit test that stubs the layer next to it.

use arb_engine::{MarketEdge, MarketGraph, cycle_profit_at, golden_section_size};
use arb_math::economic::EconomicConfig;
use arb_types::{DexProtocol, Lamports, MintId, PoolId, PoolKind, PoolState, Slot, TokenAmount};

const BASE: u8 = 1;
const TOKEN: u8 = 2;

fn mint(seed: u8) -> MintId {
    MintId::new([seed; 32])
}

/// One directional edge. Reserves are already oriented `from -> to`, which is
/// what the CPMM quoter reads.
fn edge(id: u8, from: u8, to: u8, reserve_in: u128, reserve_out: u128) -> MarketEdge {
    MarketEdge {
        pool_id: PoolId::new([id; 32]),
        protocol: DexProtocol::RaydiumV4,
        kind: PoolKind::Cpmm,
        from_mint: mint(from),
        to_mint: mint(to),
        pool_base: mint(from),
        pool_quote: mint(to),
        reserve_in,
        reserve_out,
        fee_bps: 25,
        clmm: None,
        dlmm: None,
        pump_fun: None,
        transfer_fee_bps: 0,
    }
}

fn pool(id: u8, base: u8, quote: u8, reserve_base: u128, reserve_quote: u128) -> PoolState {
    PoolState {
        pool_id: PoolId::new([id; 32]),
        protocol: DexProtocol::RaydiumV4,
        slot: Slot(1),
        write_version: 1,
        base_mint: mint(base),
        quote_mint: mint(quote),
        decimals_a: 9,
        decimals_b: 6,
        reserve_in: reserve_base,
        reserve_out: reserve_quote,
        fee_numerator: 25,
        fee_denominator: 10_000,
        is_active: true,
        has_risky_token: false,
        last_update_slot: 0,
        last_update_ms: 0,
        kind: PoolKind::Cpmm,
        clmm: None,
        dlmm: None,
        pump_fun: None,
        transfer_fee_bps: 0,
    }
}

fn config(min_profit_raw: u64, jito_tip: u64) -> EconomicConfig {
    EconomicConfig {
        flash_loan_fee_sf: 0,
        jito_tip_lamports: Lamports(jito_tip),
        base_decimals: 9,
        lamports_to_base_mint_rate: 1,
        lamports_to_base_rate_denom: 1,
        min_profit_raw: TokenAmount(min_profit_raw),
    }
}

/// The two pools of a 6% cross-pool spread, as graph input.
fn spread_pools() -> [PoolState; 2] {
    [
        pool(10, BASE, TOKEN, 1_000_000, 1_000_000),
        pool(20, BASE, TOKEN, 1_060_000, 1_000_000),
    ]
}

#[test]
fn cycle_profit_peaks_at_an_interior_size() {
    let forward = edge(10, BASE, TOKEN, 1_000_000, 1_000_000);
    let backward = edge(20, TOKEN, BASE, 1_000_000, 1_060_000);
    let path = [&forward, &backward];

    let lo = 1_000u64;
    let hi = 100_000u64;
    let optimal = golden_section_size(&path, lo, hi, 60);
    let best = cycle_profit_at(&path, optimal);

    assert!(best > 0, "a 6% spread must be profitable, got {best}");
    assert!(
        optimal > lo && optimal < hi,
        "the optimum must be interior, got {optimal} in ({lo}, {hi})"
    );
    assert!(
        best > cycle_profit_at(&path, lo),
        "must beat the lower bound"
    );
    assert!(
        best > cycle_profit_at(&path, hi),
        "must beat the upper bound"
    );

    // Concave around the peak: both a much smaller and a much larger size lose.
    let smaller = optimal / 4;
    let larger = (optimal * 3).min(hi - 1);
    assert!(
        cycle_profit_at(&path, smaller) < best,
        "smaller size {smaller} must not beat {optimal}"
    );
    assert!(
        cycle_profit_at(&path, larger) < best,
        "larger size {larger} must not beat {optimal}"
    );
}

#[test]
fn fees_turn_a_flat_market_into_a_zero_profit_cycle() {
    // Same reserves both ways: there is no spread, so only the 25 bps per hop is
    // left and every size must price to zero rather than to roundoff profit.
    let forward = edge(10, BASE, TOKEN, 1_000_000, 1_000_000);
    let backward = edge(20, TOKEN, BASE, 1_000_000, 1_000_000);
    let path = [&forward, &backward];

    for size in [1_000u64, 10_000, 50_000, 100_000] {
        assert_eq!(
            cycle_profit_at(&path, size),
            0,
            "flat market must not show profit at size {size}"
        );
    }
}

#[test]
fn graph_detects_the_spread_and_reports_net_below_gross() {
    let mut graph = MarketGraph::new(vec![mint(BASE)]).with_economic_config(config(0, 0));
    graph.build_from_pools(&spread_pools());

    let opps = graph.find_base_cycles(Slot(1), 100_000);
    assert!(!opps.is_empty(), "6% spread must be detected");
    let opp = &opps[0];
    assert_eq!(opp.hops.len(), 2);
    assert!(opp.gross_profit.as_u64() > 0);
    assert!(
        opp.net_profit.as_u64() <= opp.gross_profit.as_u64(),
        "net {} must not exceed gross {}",
        opp.net_profit.as_u64(),
        opp.gross_profit.as_u64()
    );

    // The same market with a floor above the edge must produce nothing: the
    // minimum-profit gate is applied before an opportunity is ever emitted.
    let mut sceptical =
        MarketGraph::new(vec![mint(BASE)]).with_economic_config(config(10_000_000, 0));
    sceptical.build_from_pools(&spread_pools());
    assert!(
        sceptical.find_base_cycles(Slot(1), 100_000).is_empty(),
        "a 10M floor must reject a ~1k edge"
    );
}

#[test]
fn graph_without_economic_config_fails_closed() {
    let mut graph = MarketGraph::new(vec![mint(BASE)]);
    graph.build_from_pools(&spread_pools());

    assert!(
        !graph.edges_by_from.is_empty(),
        "the pools themselves are still indexed"
    );
    assert!(
        graph.find_base_cycles(Slot(1), 100_000).is_empty(),
        "an unconfigured graph must not price opportunities"
    );
}

#[test]
fn risky_and_inactive_pools_never_reach_the_graph() {
    let mut risky = spread_pools()[0].clone();
    risky.has_risky_token = true;
    let mut inactive = spread_pools()[1].clone();
    inactive.is_active = false;

    let mut graph = MarketGraph::new(vec![mint(BASE)]).with_economic_config(config(0, 0));
    graph.build_from_pools(&[risky, inactive]);

    assert!(graph.edges_by_from.is_empty());
    assert!(graph.find_base_cycles(Slot(1), 100_000).is_empty());
}
