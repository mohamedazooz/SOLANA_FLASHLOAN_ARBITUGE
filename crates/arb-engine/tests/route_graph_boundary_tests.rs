//! Feature 16: Universal Cross-DEX Cycle Pathfinder — Tier 2 Boundary Tests.
//!
//! Validates disconnected token graphs, max hop limit clamping (2..=8) and depth ceilings,
//! duplicate pool suppression in DFS walk, empty graphs / zero routes, and loss pricing.

use arb_engine::UnifiedRouteGraph;
use arb_types::primitives::{MintId, PoolId, Slot};
use arb_types::{DexProtocol, PoolKind, PoolState, TokenProgramKind};
use solana_pubkey::Pubkey;

fn mint(n: u8) -> MintId {
    MintId(Pubkey::new_from_array([n; 32]))
}

fn make_pool(id: u8, a: MintId, b: MintId, r_a: u128, r_b: u128) -> PoolState {
    PoolState {
        pool_id: PoolId::new([id; 32]),
        protocol: DexProtocol::RaydiumV4,
        slot: Slot(1),
        write_version: 1,
        base_mint: a,
        quote_mint: b,
        decimals_a: 9,
        decimals_b: 9,
        reserve_in: r_a,
        reserve_out: r_b,
        fee_numerator: 25,
        fee_denominator: 10_000,
        is_active: true,
        has_risky_token: false,
        kind: PoolKind::Cpmm,
        clmm: None,
        dlmm: None,
        pump_fun: None,
        transfer_fee_bps: 0,
        token_programs: [TokenProgramKind::SplToken; 2],
        execution_accounts: None,
        last_update_slot: 1,
        last_update_ms: 1_700_000_000_000,
    }
}

/// Boundary Case 1: Disconnected token graph returns empty cycles without panic.
#[test]
fn test_cycle_pathfinder_disconnected_token_graph() {
    let a = mint(1);
    let b = mint(2);
    let c = mint(3);
    let d = mint(4);

    let p1 = make_pool(1, a, b, 10_000_000, 10_000_000);
    let p2 = make_pool(2, c, d, 10_000_000, 10_000_000);

    let graph = UnifiedRouteGraph::new(vec![p1, p2]);
    let cycles = graph.enumerate_cycles(a, 4, 10);
    assert!(
        cycles.is_empty(),
        "disconnected components must produce 0 cycles"
    );
}

/// Boundary Case 2: Max hop limit boundary and clamping (4-hop vs 5-hop, 1 clamped to 2).
#[test]
fn test_cycle_pathfinder_max_hop_limit_boundary_and_clamping() {
    let a = mint(1);
    let b = mint(2);
    let c = mint(3);
    let d = mint(4);
    let e = mint(5);

    // 5-hop linear cycle: A -> B -> C -> D -> E -> A
    let p1 = make_pool(1, a, b, 10_000_000, 10_000_000);
    let p2 = make_pool(2, b, c, 10_000_000, 10_000_000);
    let p3 = make_pool(3, c, d, 10_000_000, 10_000_000);
    let p4 = make_pool(4, d, e, 10_000_000, 10_000_000);
    let p5 = make_pool(5, e, a, 10_000_000, 10_000_000);

    let graph = UnifiedRouteGraph::new(vec![p1, p2, p3, p4, p5]);

    // At max_hops = 4, 5-hop cycle cannot be reached
    let cycles_4 = graph.enumerate_cycles(a, 4, 10);
    assert!(
        cycles_4.is_empty(),
        "5-hop cycle must not be found when max_hops = 4"
    );

    // At max_hops = 5, 5-hop cycle is discovered
    let cycles_5 = graph.enumerate_cycles(a, 5, 10);
    assert_eq!(
        cycles_5.len(),
        1,
        "5-hop cycle must be discovered when max_hops = 5"
    );
    assert_eq!(cycles_5[0].hops.len(), 5);

    // Testing max_hops = 1 clamping to 2: 2-hop cycle discovered
    let p_rev = make_pool(6, b, a, 10_000_000, 10_000_000);
    let p_direct = make_pool(7, a, b, 10_000_000, 10_000_000);
    let graph_2hop = UnifiedRouteGraph::new(vec![p_direct, p_rev]);
    let cycles_clamped = graph_2hop.enumerate_cycles(a, 1, 10); // clamped to 2
    assert_eq!(
        cycles_clamped.len(),
        1,
        "clamped to 2 must find 2-hop cycle"
    );
    assert_eq!(cycles_clamped[0].hops.len(), 2);
}

/// Boundary Case 3: Duplicate pool suppression in DFS walk (no reusing same pool in route).
#[test]
fn test_cycle_pathfinder_duplicate_pool_rejection_in_dfs() {
    let a = mint(1);
    let b = mint(2);
    let c = mint(3);

    // Only two pools: A <-> B and B <-> C.
    // Cannot form a valid cycle A -> B -> C -> B -> A without reusing pool 1 and pool 2!
    let p1 = make_pool(1, a, b, 10_000_000, 10_000_000);
    let p2 = make_pool(2, b, c, 10_000_000, 10_000_000);

    let graph = UnifiedRouteGraph::new(vec![p1, p2]);
    let cycles = graph.enumerate_cycles(a, 4, 10);
    assert!(
        cycles.is_empty(),
        "cannot reuse pools to form back-and-forth cycle"
    );
}

/// Boundary Case 4: Empty graph, single-pool graph, and zero routes return empty Vec.
#[test]
fn test_cycle_pathfinder_empty_graph_and_zero_routes() {
    let a = mint(1);
    let b = mint(2);

    // Sub-case A: empty graph
    let empty_graph = UnifiedRouteGraph::new(vec![]);
    assert!(empty_graph.enumerate_cycles(a, 3, 10).is_empty());

    // Sub-case B: single pool (cannot close cycle)
    let single_graph = UnifiedRouteGraph::new(vec![make_pool(1, a, b, 10_000, 10_000)]);
    assert!(single_graph.enumerate_cycles(a, 3, 10).is_empty());

    // Sub-case C: max_routes = 0
    let p1 = make_pool(1, a, b, 10_000, 10_000);
    let p2 = make_pool(2, b, a, 10_000, 10_000);
    let valid_graph = UnifiedRouteGraph::new(vec![p1, p2]);
    assert!(valid_graph.enumerate_cycles(a, 3, 0).is_empty());
}

/// Boundary Case 5: Zero-profit or losing cycle pricing evaluation via quote_candidate.
#[test]
fn test_cycle_pathfinder_zero_profit_or_loss_pricing_rejection() {
    let a = mint(1);
    let b = mint(2);

    // Equal 1:1 reserves on both pools with 25 bps fee each -> guaranteed loss
    let p1 = make_pool(1, a, b, 10_000_000_000, 10_000_000_000);
    let p2 = make_pool(2, b, a, 10_000_000_000, 10_000_000_000);

    let graph = UnifiedRouteGraph::new(vec![p1, p2]);
    let cycles = graph.enumerate_cycles(a, 2, 10);
    assert_eq!(cycles.len(), 1);

    let quote = graph.quote_candidate(&cycles[0], 1_000_000).unwrap();
    // Input is 1,000,000. Output will be strictly less due to CPMM formula and 25 bps fees
    assert!(
        quote.amount_out < 1_000_000,
        "loss route must yield output < input"
    );
    let gross_profit = (quote.amount_out as i128) - (quote.amount_in as i128);
    assert!(
        gross_profit <= 0,
        "gross profit must be <= 0 for losing cycle"
    );
}
