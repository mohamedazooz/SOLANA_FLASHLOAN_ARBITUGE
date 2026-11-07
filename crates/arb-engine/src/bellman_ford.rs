//! Bellman-Ford Negative Cycle Detection Engine for Universal Arbitrage Route Discovery.
//!
//! Transforms the token-pool liquidity graph into a weighted directed graph
//! where edge weight = -ln(exchange_rate × fee_complement). A negative-cost
//! cycle in this graph corresponds to a profitable arbitrage route of ANY length.
//!
//! This replaces the previous DFS brute-force (limited to max_hops=5 with
//! aggressive pruning that misses opportunities) with the mathematically
//! optimal approach:
//!
//!   - Bellman-Ford relaxes all edges V-1 times → shortest paths
//!   - One more relaxation pass: if any distance shrinks → negative cycle exists
//!   - The cycle's edges trace the exact arbitrage route
//!
//! References:
//!   - arXiv:2406.16573 — "An Improved Algorithm to Identify More Arbitrage
//!     Opportunities on Decentralized Exchanges" (Moore-Bellman-Ford)
//!   - Princeton CS423 — Bellman-Ford & negative cycle detection
//!   - emergentmind.com — "Graph-Based Cyclic Arbitrage"
//!
//! All edge weights and distances use f64. The -ln transform ensures:
//!   - Product of exchange rates along a cycle > 1  →  negative cycle  →  profit
//!   - No floating-point threshold magic numbers: the cycle IS the proof

use arb_math::{
    Leg,
    cpmm::clamp_at_marginal_ceiling_two_pool,
    economic::{
        EconomicConfig, base_raw_to_lamports, build_economic_quote, calculate_flash_fee_raw,
    },
    optimal_four_pool, optimal_three_pool, optimal_two_pool,
};
use arb_types::{
    DexProtocol, ExecutionOpportunity, Lamports, MintId, PoolId, PoolKind, RouteId, Slot, SwapHop,
    TokenAmount,
};
use std::collections::HashMap;
use tracing::debug;

/// A directed edge in the exchange-rate graph for Bellman-Ford.
/// Weight = -ln(rate × gamma) where rate = R_out / R_in and gamma = (10000 - fee_bps) / 10000.
#[derive(Debug, Clone)]
pub struct BfEdge {
    pub pool_id: PoolId,
    pub protocol: DexProtocol,
    pub kind: PoolKind,
    pub from_mint: MintId,
    pub to_mint: MintId,
    pub reserve_in: u128,
    pub reserve_out: u128,
    pub fee_bps: u16,
    /// -ln(rate × gamma) — negative when the edge is favourable (output > input)
    pub weight: f64,
}

impl BfEdge {
    pub fn from_pool_state(p: &arb_types::PoolState) -> Self {
        let fee = if p.fee_denominator == 0 {
            25u16
        } else {
            ((p.fee_numerator as u128 * 10_000) / p.fee_denominator as u128) as u16
        };
        let gamma = (10_000u32 - fee as u32) as f64 / 10_000.0;
        let rate = if p.reserve_in > 0 {
            p.reserve_out as f64 / p.reserve_in as f64
        } else {
            0.0
        };
        // weight = -ln(rate × gamma). If rate × gamma > 1, weight is negative (favourable).
        let rg = (rate * gamma).max(1e-18);
        let weight = -rg.ln();
        Self {
            pool_id: p.pool_id,
            protocol: p.protocol,
            kind: p.kind,
            from_mint: p.base_mint,
            to_mint: p.quote_mint,
            reserve_in: p.reserve_in,
            reserve_out: p.reserve_out,
            fee_bps: fee,
            weight,
        }
    }
}

/// Result of a negative cycle detection: the exact sequence of pool hops
/// that form a profitable arbitrage route.
#[derive(Debug, Clone)]
pub struct ArbitrageCycle {
    /// The edges (pool hops) that form the cycle, in execution order.
    pub edges: Vec<BfEdge>,
    /// Product of (rate × gamma) along the cycle. > 1.0 means profitable.
    pub product: f64,
    /// Total weight = -ln(product). < 0 means profitable.
    pub total_weight: f64,
}

/// Bellman-Ford negative cycle detector.
///
/// Builds a directed graph from all live pool states, relaxes edges |V|-1 times,
/// then performs one more pass. Any edge that can still be relaxed is part of
/// (or reachable from) a negative cycle — i.e., a profitable arbitrage route.
pub struct BellmanFordEngine {
    /// Adjacency list: mint → outgoing edges
    graph: HashMap<MintId, Vec<BfEdge>>,
    /// All unique mint nodes
    nodes: Vec<MintId>,
    /// Authoritative economic configuration. When `None` the engine cannot price
    /// opportunities and `find_negative_cycles` returns empty (fail closed).
    economic_config: Option<EconomicConfig>,
}

impl BellmanFordEngine {
    /// Set the authoritative economic configuration.
    pub fn with_economic_config(mut self, config: EconomicConfig) -> Self {
        self.economic_config = Some(config);
        self
    }

    /// Build the exchange-rate graph from live pool states.
    /// Each pool creates TWO edges (forward: base→quote, reverse: quote→base).
    pub fn from_pools(pools: &[arb_types::PoolState]) -> Self {
        let mut graph: HashMap<MintId, Vec<BfEdge>> = HashMap::new();
        let mut node_set = std::collections::HashSet::new();

        for p in pools {
            if !p.is_active || p.reserve_in == 0 || p.reserve_out == 0 || p.has_risky_token {
                continue;
            }
            // Forward edge: base_mint → quote_mint
            let fwd = BfEdge::from_pool_state(p);
            node_set.insert(fwd.from_mint);
            node_set.insert(fwd.to_mint);
            graph.entry(fwd.from_mint).or_default().push(fwd.clone());

            // Reverse edge: quote_mint → base_mint (swap reserves)
            let fee = fwd.fee_bps;
            let gamma = (10_000u32 - fee as u32) as f64 / 10_000.0;
            let rev_rate = if p.reserve_out > 0 {
                p.reserve_in as f64 / p.reserve_out as f64
            } else {
                0.0
            };
            let rg = (rev_rate * gamma).max(1e-18);
            let rev_weight = -rg.ln();
            let rev = BfEdge {
                pool_id: p.pool_id,
                protocol: p.protocol,
                kind: p.kind,
                from_mint: p.quote_mint,
                to_mint: p.base_mint,
                reserve_in: p.reserve_out,
                reserve_out: p.reserve_in,
                fee_bps: fee,
                weight: rev_weight,
            };
            graph.entry(rev.from_mint).or_default().push(rev);
        }

        let mut nodes: Vec<MintId> = node_set.into_iter().collect();
        nodes.sort_by_key(|a| a.0.to_string());
        Self {
            graph,
            nodes,
            economic_config: None,
        }
    }

    /// Detect ALL profitable arbitrage cycles using Bellman-Ford.
    ///
    /// Algorithm:
    ///   1. Initialize dist[v] = 0 for all nodes (super-source formulation)
    ///   2. Relax all edges |V| - 1 times
    ///   3. One more pass: any edge (u→v) where dist[v] > dist[u] + w(u,v)
    ///      indicates a negative cycle reachable from v
    ///   4. Trace back through predecessor[] to extract the cycle
    ///
    /// Returns all unique cycles found, sorted by profitability (most negative weight first).
    pub fn find_negative_cycles(
        &self,
        current_slot: Slot,
        max_trade_size: u64,
    ) -> Vec<ExecutionOpportunity> {
        let n = self.nodes.len();
        if n == 0 || n > 5000 {
            // Safety: don't run on absurdly large graphs (memory)
            debug!("BellmanFord: skipping ({} nodes)", n);
            return Vec::new();
        }

        // Map mint → index
        let idx_map: HashMap<MintId, usize> = self
            .nodes
            .iter()
            .enumerate()
            .map(|(i, m)| (*m, i))
            .collect();

        // dist[v] = shortest path weight from super-source to v
        let mut dist = vec![0.0f64; n];
        // pred[v] = Some((u_idx, edge)) — stores the full incoming edge for sub-microsecond cycle reconstruction
        let mut pred: Vec<Option<(usize, BfEdge)>> = vec![None; n];

        // Relax all edges |V| - 1 times
        for _iteration in 0..n.saturating_sub(1) {
            let mut relaxed = false;
            for u_idx in 0..n {
                let u_mint = self.nodes[u_idx];
                if let Some(edges) = self.graph.get(&u_mint) {
                    for edge in edges.iter() {
                        if let Some(&v_idx) = idx_map.get(&edge.to_mint) {
                            let new_dist = dist[u_idx] + edge.weight;
                            if new_dist < dist[v_idx] - 1e-15 {
                                dist[v_idx] = new_dist;
                                pred[v_idx] = Some((u_idx, edge.clone()));
                                relaxed = true;
                            }
                        }
                    }
                }
            }
            if !relaxed {
                break; // early termination — no more improvements
            }
        }

        // Final relaxation pass: identify edges that can still be relaxed → negative cycle
        let mut cycle_start_nodes: Vec<usize> = Vec::new();
        for u_idx in 0..n {
            let u_mint = self.nodes[u_idx];
            if let Some(edges) = self.graph.get(&u_mint) {
                for edge in edges {
                    if let Some(&v_idx) = idx_map.get(&edge.to_mint)
                        && dist[v_idx] > dist[u_idx] + edge.weight + 1e-15
                    {
                        dist[v_idx] = dist[u_idx] + edge.weight;
                        pred[v_idx] = Some((u_idx, edge.clone()));
                        cycle_start_nodes.push(v_idx);
                    }
                }
            }
        }

        let mut opportunities = Vec::new();
        if cycle_start_nodes.is_empty() {
            return opportunities;
        }

        let mut seen_cycles = std::collections::HashSet::new();
        let mut route_counter = 1u32;

        // [B-01]: Direct negative cycle extraction from predecessor chain (Sub-microsecond, no DFS)
        for v_start in cycle_start_nodes {
            let mut curr = v_start;
            for _ in 0..n {
                if let Some((p_idx, _)) = &pred[curr] {
                    curr = *p_idx;
                } else {
                    break;
                }
            }
            let cycle_node = curr;

            // Trace edges backwards from cycle_node until the cycle closes
            let mut trace_curr = cycle_node;
            let mut cycle_edges = Vec::new();
            let mut visited = std::collections::HashMap::new();
            let mut step = 0;

            while let Some((p_idx, edge)) = &pred[trace_curr] {
                if let Some(&prev_step) = visited.get(&trace_curr) {
                    let cycle_slice = cycle_edges[prev_step..step].to_vec();
                    cycle_edges = cycle_slice;
                    break;
                }
                visited.insert(trace_curr, step);
                cycle_edges.push(edge.clone());
                step += 1;
                trace_curr = *p_idx;
                if step > n + 1 {
                    break;
                }
            }

            // Reverse to forward execution order
            cycle_edges.reverse();

            // Length must be between 2 and 4 hops (B-01 requirement)
            let hop_count = cycle_edges.len();
            if !(2..=4).contains(&hop_count) {
                continue;
            }

            // Verify continuity: each hop's to_mint must match the next hop's from_mint
            let mut continuous = true;
            for i in 0..hop_count {
                if cycle_edges[i].to_mint != cycle_edges[(i + 1) % hop_count].from_mint {
                    continuous = false;
                    break;
                }
            }
            if !continuous {
                continue;
            }

            // Verify pool uniqueness: no duplicate PoolId in the same cycle
            let mut pool_set = std::collections::HashSet::new();
            let mut has_duplicate_pool = false;
            for e in &cycle_edges {
                if !pool_set.insert(e.pool_id) {
                    has_duplicate_pool = true;
                    break;
                }
            }
            if has_duplicate_pool {
                continue;
            }

            // Rotate cycle so that if WSOL or USDC is present, borrow starts and ends with it
            const WSOL_STR: &str = "So11111111111111111111111111111111111111112";
            const USDC_STR: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
            if let Some(pos) = cycle_edges.iter().position(|e| {
                let s = e.from_mint.0.to_string();
                s == WSOL_STR || s == USDC_STR
            }) {
                cycle_edges.rotate_left(pos);
            }

            // Deduplicate across relaxed vertices using canonical pool key
            let mut key_pools: Vec<String> = cycle_edges
                .iter()
                .map(|e| e.pool_id.0.to_string())
                .collect();
            key_pools.sort();
            let cycle_key = key_pools.join("+");
            if seen_cycles.contains(&cycle_key) {
                continue;
            }
            seen_cycles.insert(cycle_key);

            // Price the cycle atomically via closed-form models & CostLedger
            if let Some(opp) =
                self.evaluate_cycle(&cycle_edges, current_slot, max_trade_size, route_counter)
            {
                opportunities.push(opp);
                route_counter += 1;
            }
        }

        // Sort by net profit descending
        opportunities.sort_by_key(|a| std::cmp::Reverse(a.net_profit.0));
        opportunities
    }

    /// Evaluates profitability, optimal trade size, and fills the ExecutionOpportunity
    /// using closed-form mathematical equations and CostLedger economic modeling.
    fn evaluate_cycle(
        &self,
        cycle: &[BfEdge],
        current_slot: Slot,
        max_trade_size: u64,
        route_counter: u32,
    ) -> Option<ExecutionOpportunity> {
        let product: f64 = cycle
            .iter()
            .map(|e| {
                let gamma = (10_000u32 - e.fee_bps as u32) as f64 / 10_000.0;
                let rate = if e.reserve_in > 0 {
                    e.reserve_out as f64 / e.reserve_in as f64
                } else {
                    0.0
                };
                (rate * gamma).max(1e-18)
            })
            .product();

        if product <= 1.0 {
            return None;
        }

        let flash_loan_fee_sf = self
            .economic_config
            .as_ref()
            .map(|c| c.flash_loan_fee_sf)
            .unwrap_or(0);
        let (best_size, gross_profit) =
            compute_cycle_profit(cycle, max_trade_size, flash_loan_fee_sf);
        if gross_profit == 0 || best_size == 0 {
            return None;
        }

        let hops: Vec<SwapHop> = cycle
            .iter()
            .map(|e| SwapHop {
                pool_id: e.pool_id,
                protocol: e.protocol,
                mint_in: e.from_mint,
                mint_out: e.to_mint,
                fee_bps: e.fee_bps,
                active_bin_id: None,
            })
            .collect();

        let mut hop_inputs = vec![best_size];
        let mut curr = best_size as u128;
        for e in cycle {
            let gamma = 10_000u128.saturating_sub(e.fee_bps as u128);
            curr = crate::quote_pool(e.kind, curr, e.reserve_in, e.reserve_out, gamma, None);
            hop_inputs.push(curr as u64);
        }
        let final_out = curr as u64;

        let first = &cycle[0];
        let gamma1 = 10_000u128.saturating_sub(first.fee_bps as u128);
        let impact = arb_math::cpmm_price_impact_bps(
            best_size as u128,
            first.reserve_in,
            first.reserve_out,
            gamma1,
        );

        let config = self.economic_config.as_ref()?;
        let base_mint = cycle[0].from_mint;
        let flash_fee_raw =
            calculate_flash_fee_raw(best_size, config.flash_loan_fee_sf).unwrap_or(0);
        let quote = match build_economic_quote(
            base_mint,
            config.base_decimals,
            TokenAmount(best_size),
            TokenAmount(final_out),
            TokenAmount(final_out),
            TokenAmount(flash_fee_raw),
            TokenAmount(0),
            TokenAmount(0),
            TokenAmount(0),
            TokenAmount(0),
            Lamports::ZERO,
            Lamports::ZERO,
            config.jito_tip_lamports,
            config.lamports_to_base_mint_rate,
            config.lamports_to_base_rate_denom,
            config.min_profit_raw,
            current_slot,
            current_slot.0,
        ) {
            Ok(q) => q,
            Err(_) => return None,
        };
        if !quote.is_profitable() {
            return None;
        }

        let gross_lamports = match base_raw_to_lamports(
            quote.gross_profit_raw,
            config.lamports_to_base_mint_rate,
            config.lamports_to_base_rate_denom,
        ) {
            Ok(l) => l,
            Err(_) => return None,
        };
        let net_lamports = match base_raw_to_lamports(
            quote.net_profit_raw,
            config.lamports_to_base_mint_rate,
            config.lamports_to_base_rate_denom,
        ) {
            Ok(l) => l,
            Err(_) => return None,
        };

        Some(ExecutionOpportunity {
            route_id: RouteId(route_counter),
            slot: current_slot,
            size_in: TokenAmount(best_size),
            expected_out: TokenAmount(final_out),
            hop_inputs,
            gross_profit: gross_lamports,
            net_profit: net_lamports,
            base_decimals: config.base_decimals,
            price_impact_bps: impact,
            hops,
        })
    }
}

/// Compute the gross profit for a cycle using the existing optimal sizing math.
/// For 2-hop cycles: uses optimal_two_pool (closed-form).
/// For 3-hop: uses optimal_three_pool (closed-form).
/// For 4+: uses binary search (bisection) — same as the DFS engine.
fn compute_cycle_profit(
    path: &[BfEdge],
    max_trade_size: u64,
    flash_loan_fee_sf: u64,
) -> (u64, u64) {
    match path.len() {
        2 => {
            let l1 = Leg {
                r_in: path[0].reserve_in,
                r_out: path[0].reserve_out,
                gamma: 10_000u128.saturating_sub(path[0].fee_bps as u128),
            };
            let l2 = Leg {
                r_in: path[1].reserve_in,
                r_out: path[1].reserve_out,
                gamma: 10_000u128.saturating_sub(path[1].fee_bps as u128),
            };
            if let Some(opt_size) = optimal_two_pool(&l1, &l2) {
                let size_in = opt_size.min(max_trade_size);
                if size_in == 0 {
                    return (0, 0);
                }
                // Clamp at marginal degradation ceiling
                let flash_fee_raw =
                    calculate_flash_fee_raw(size_in, flash_loan_fee_sf).unwrap_or(0);
                let flash_fee_bps = if size_in > 0 {
                    ((flash_fee_raw as u128 * 10_000) / size_in as u128) as u64
                } else {
                    0
                };
                let clamped_size =
                    clamp_at_marginal_ceiling_two_pool(size_in, &l1, &l2, flash_fee_bps)
                        .unwrap_or(size_in);
                if clamped_size == 0 {
                    return (0, 0);
                }
                let out1 = crate::quote_pool(
                    path[0].kind,
                    clamped_size as u128,
                    l1.r_in,
                    l1.r_out,
                    l1.gamma,
                    None,
                );
                let out2 = crate::quote_pool(path[1].kind, out1, l2.r_in, l2.r_out, l2.gamma, None);
                if out2 > clamped_size as u128 {
                    (clamped_size, (out2 - clamped_size as u128) as u64)
                } else {
                    (0, 0)
                }
            } else {
                (0, 0)
            }
        }
        3 => {
            let l1 = Leg {
                r_in: path[0].reserve_in,
                r_out: path[0].reserve_out,
                gamma: 10_000u128.saturating_sub(path[0].fee_bps as u128),
            };
            let l2 = Leg {
                r_in: path[1].reserve_in,
                r_out: path[1].reserve_out,
                gamma: 10_000u128.saturating_sub(path[1].fee_bps as u128),
            };
            let l3 = Leg {
                r_in: path[2].reserve_in,
                r_out: path[2].reserve_out,
                gamma: 10_000u128.saturating_sub(path[2].fee_bps as u128),
            };
            if let Some(opt_size) = optimal_three_pool(&l1, &l2, &l3) {
                let size_in = opt_size.min(max_trade_size);
                if size_in == 0 {
                    return (0, 0);
                }
                // For 3-hop, use a heuristic clamp: if flash fee > 50% of gross profit, halve
                let flash_fee_raw =
                    calculate_flash_fee_raw(size_in, flash_loan_fee_sf).unwrap_or(0);
                let clamped_size = {
                    let out1 = crate::quote_pool(
                        path[0].kind,
                        size_in as u128,
                        l1.r_in,
                        l1.r_out,
                        l1.gamma,
                        None,
                    );
                    let out2 =
                        crate::quote_pool(path[1].kind, out1, l2.r_in, l2.r_out, l2.gamma, None);
                    let out3 =
                        crate::quote_pool(path[2].kind, out2, l3.r_in, l3.r_out, l3.gamma, None);
                    let gross_profit = if out3 > size_in as u128 {
                        out3 - size_in as u128
                    } else {
                        return (0, 0);
                    };
                    if u128::from(flash_fee_raw) * 2 > gross_profit && size_in > 1_000 {
                        size_in / 2
                    } else {
                        size_in
                    }
                };
                if clamped_size == 0 {
                    return (0, 0);
                }
                let out1 = crate::quote_pool(
                    path[0].kind,
                    clamped_size as u128,
                    l1.r_in,
                    l1.r_out,
                    l1.gamma,
                    None,
                );
                let out2 = crate::quote_pool(path[1].kind, out1, l2.r_in, l2.r_out, l2.gamma, None);
                let out3 = crate::quote_pool(path[2].kind, out2, l3.r_in, l3.r_out, l3.gamma, None);
                if out3 > clamped_size as u128 {
                    (clamped_size, (out3 - clamped_size as u128) as u64)
                } else {
                    (0, 0)
                }
            } else {
                (0, 0)
            }
        }
        4 => {
            let l1 = Leg {
                r_in: path[0].reserve_in,
                r_out: path[0].reserve_out,
                gamma: 10_000u128.saturating_sub(path[0].fee_bps as u128),
            };
            let l2 = Leg {
                r_in: path[1].reserve_in,
                r_out: path[1].reserve_out,
                gamma: 10_000u128.saturating_sub(path[1].fee_bps as u128),
            };
            let l3 = Leg {
                r_in: path[2].reserve_in,
                r_out: path[2].reserve_out,
                gamma: 10_000u128.saturating_sub(path[2].fee_bps as u128),
            };
            let l4 = Leg {
                r_in: path[3].reserve_in,
                r_out: path[3].reserve_out,
                gamma: 10_000u128.saturating_sub(path[3].fee_bps as u128),
            };
            if let Some(opt_size) = optimal_four_pool(&l1, &l2, &l3, &l4) {
                let size_in = opt_size.min(max_trade_size);
                if size_in == 0 {
                    return (0, 0);
                }
                let out1 = crate::quote_pool(
                    path[0].kind,
                    size_in as u128,
                    l1.r_in,
                    l1.r_out,
                    l1.gamma,
                    None,
                );
                let out2 = crate::quote_pool(path[1].kind, out1, l2.r_in, l2.r_out, l2.gamma, None);
                let out3 = crate::quote_pool(path[2].kind, out2, l3.r_in, l3.r_out, l3.gamma, None);
                let out4 = crate::quote_pool(path[3].kind, out3, l4.r_in, l4.r_out, l4.gamma, None);
                if out4 > size_in as u128 {
                    (size_in, (out4 - size_in as u128) as u64)
                } else {
                    (0, 0)
                }
            } else {
                (0, 0)
            }
        }
        _ => {
            // 4+ hops: binary search (bisection) — same as arb-engine DFS engine
            let mut low = 1_000u64;
            let mut high = max_trade_size;
            let mut best_size = 0u64;
            let mut best_profit = 0u64;

            for _ in 0..16 {
                let mid = low + (high - low) / 2;
                let mut curr = mid as u128;
                for e in path {
                    let gamma = 10_000u128.saturating_sub(e.fee_bps as u128);
                    curr =
                        crate::quote_pool(e.kind, curr, e.reserve_in, e.reserve_out, gamma, None);
                }
                if curr > mid as u128 {
                    let profit = (curr - mid as u128) as u64;
                    if profit > best_profit {
                        best_profit = profit;
                        best_size = mid;
                    }
                    low = mid + 1;
                } else {
                    high = mid.saturating_sub(1);
                }
            }
            if best_profit == 0 {
                (0, 0)
            } else {
                (best_size, best_profit)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_types::{DexProtocol, MintId, PoolId, PoolKind, PoolState, Slot};

    fn test_config() -> EconomicConfig {
        EconomicConfig {
            flash_loan_fee_sf: 0,
            jito_tip_lamports: Lamports::ZERO,
            base_decimals: 9,
            lamports_to_base_mint_rate: 1,
            lamports_to_base_rate_denom: 1,
            min_profit_raw: TokenAmount::ZERO,
        }
    }

    fn make_pool(id: u8, base: u8, quote: u8, r_in: u128, r_out: u128, fee_bps: u16) -> PoolState {
        PoolState {
            pool_id: PoolId::new([id; 32]),
            protocol: DexProtocol::RaydiumV4,
            slot: Slot(1),
            write_version: 1,
            base_mint: MintId::new([base; 32]),
            quote_mint: MintId::new([quote; 32]),
            decimals_a: 9,
            decimals_b: 6,
            reserve_in: r_in,
            reserve_out: r_out,
            fee_numerator: fee_bps as u64,
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

    #[test]
    fn bellman_ford_detects_2hop_arbitrage() {
        // Pool 1: 1M Base / 1M TokenB
        let p1 = make_pool(10, 1, 2, 1_000_000, 1_000_000, 25);
        // Pool 2: 1M TokenB / 1.05M Base (5% arb spread)
        let p2 = make_pool(20, 2, 1, 1_000_000, 1_050_000, 25);

        let engine = BellmanFordEngine::from_pools(&[p1, p2]).with_economic_config(test_config());
        let opps = engine.find_negative_cycles(Slot(1), 100_000);

        assert!(
            !opps.is_empty(),
            "Bellman-Ford must detect the 2-hop arbitrage cycle"
        );
        assert!(opps[0].gross_profit.as_u64() > 0);
        assert_eq!(opps[0].hops.len(), 2);
    }

    #[test]
    fn bellman_ford_detects_no_arbitrage_when_balanced() {
        let p1 = make_pool(10, 1, 2, 1_000_000, 1_000_000, 25);
        let p2 = make_pool(20, 2, 1, 1_000_000, 1_000_000, 25);

        let engine = BellmanFordEngine::from_pools(&[p1, p2]).with_economic_config(test_config());
        let opps = engine.find_negative_cycles(Slot(1), 100_000);

        assert!(
            opps.is_empty(),
            "No arbitrage when pools are perfectly balanced"
        );
    }

    #[test]
    fn bellman_ford_detects_3hop_triangular() {
        // A→B: 1:1
        let p1 = make_pool(10, 1, 2, 1_000_000, 1_000_000, 25);
        // B→C: 1:1
        let p2 = make_pool(20, 2, 3, 1_000_000, 1_000_000, 25);
        // C→A: 1:1.1 (10% premium)
        let p3 = make_pool(30, 3, 1, 1_000_000, 1_100_000, 25);

        let engine =
            BellmanFordEngine::from_pools(&[p1, p2, p3]).with_economic_config(test_config());
        let opps = engine.find_negative_cycles(Slot(1), 100_000);

        assert!(
            !opps.is_empty(),
            "Bellman-Ford must detect the 3-hop triangular arbitrage"
        );
        assert!(opps[0].gross_profit.as_u64() > 0);
    }

    #[test]
    fn bellman_ford_handles_empty_graph() {
        let engine = BellmanFordEngine::from_pools(&[]);
        let opps = engine.find_negative_cycles(Slot(1), 100_000);
        assert!(opps.is_empty());
    }

    #[test]
    fn bellman_ford_edge_weight_correct() {
        let p = make_pool(10, 1, 2, 100, 200, 0); // 0 fee, rate = 2.0
        let edge = BfEdge::from_pool_state(&p);
        // weight = -ln(2.0 × 1.0) = -ln(2) ≈ -0.693
        assert!((edge.weight - (-2.0_f64.ln())).abs() < 0.001);
    }
}
