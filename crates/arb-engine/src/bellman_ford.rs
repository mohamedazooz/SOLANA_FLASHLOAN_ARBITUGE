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

use arb_math::economic::{
    EconomicConfig, base_raw_to_lamports, build_economic_quote, calculate_flash_fee_raw,
    slippage_haircut_min_final,
};
use arb_types::{
    DexProtocol, ExecutionOpportunity, Lamports, MintId, PoolId, PoolKind, RouteId, Slot, SwapHop,
    TokenAmount,
};
use std::collections::HashMap;
use tracing::debug;

/// Return the conservative fee-retention multiplier used only by the floating-point
/// negative-cycle pre-screen. Exact amount sizing and profitability are evaluated
/// separately with protocol-native integer quotes.
///
/// CLMM fee rates are stored in their native millionth denominator; all other
/// protocols use the exact numerator/denominator carried by the verified snapshot.
/// Raydium CPMM creator fees are an additional volume fee and may be collected
/// on either side of the swap, depending on the pool's creator-fee mode.
fn exact_fee_retention(
    protocol: DexProtocol,
    kind: PoolKind,
    fee_numerator: u64,
    fee_denominator: u64,
    clmm_fee_rate: Option<u16>,
    raydium_cpmm_fee: Option<&arb_types::RaydiumCpmmFeeState>,
    a_to_b: bool,
) -> Option<f64> {
    let (numerator, denominator) = if kind == PoolKind::Clmm {
        (u64::from(clmm_fee_rate?), 1_000_000u64)
    } else {
        (fee_numerator, fee_denominator)
    };
    if denominator == 0 || numerator >= denominator {
        return None;
    }
    let trade_retention = 1.0 - numerator as f64 / denominator as f64;
    if !trade_retention.is_finite() || trade_retention <= 0.0 {
        return None;
    }

    if protocol != DexProtocol::RaydiumCpmm {
        return Some(trade_retention);
    }

    let fees = raydium_cpmm_fee?;
    if !fees.is_valid() {
        return None;
    }
    let creator_rate = fees.effective_creator_fee_rate();
    if creator_rate == 0 {
        return Some(trade_retention);
    }
    let creator_retention = 1.0 - creator_rate as f64 / 1_000_000.0;
    if creator_retention <= 0.0 {
        return None;
    }
    match fees.creator_fee_on_input(a_to_b)? {
        // The two input-denominated volume rates are both deducted before
        // curve math; their exact quote implementation remains authoritative.
        true => {
            let combined = numerator.checked_add(creator_rate)?;
            if combined >= denominator {
                None
            } else {
                Some(1.0 - combined as f64 / denominator as f64)
            }
        }
        // Output-side creator fees compound after the curve output.
        false => Some(trade_retention * creator_retention),
    }
}

/// A directed edge in the exchange-rate graph for Bellman-Ford.
/// Weight = -ln(rate × exact fee-retention estimate); BPS is metadata only.
#[derive(Debug, Clone)]
pub struct BfEdge {
    pub pool_id: PoolId,
    pub protocol: DexProtocol,
    pub kind: PoolKind,
    pub from_mint: MintId,
    pub to_mint: MintId,
    pub pool_base: MintId,
    pub pool_quote: MintId,
    pub reserve_in: u128,
    pub reserve_out: u128,
    pub fee_bps: u16,
    pub fee_numerator: u64,
    pub fee_denominator: u64,
    /// Exact Raydium CPMM dynamic/creator fee metadata used by the quote adapter.
    pub raydium_cpmm_fee: Option<arb_types::RaydiumCpmmFeeState>,
    /// -ln(rate × gamma) — negative when the edge is favourable (output > input)
    pub weight: f64,
    pub clmm: Option<arb_types::ClmmState>,
    pub dlmm: Option<arb_types::DlmmState>,
    pub pump_fun: Option<arb_types::PumpFunState>,
    pub damm_v2: Option<arb_types::DammV2State>,
    pub transfer_fee_bps: u16,
    pub token_programs: [arb_types::TokenProgramKind; 2],
    pub token2022_fee_context: Option<arb_types::Token2022FeeContext>,
    pub execution_accounts: Option<arb_types::PoolExecutionAccounts>,
    pub active_bin_id: Option<i32>,
}

impl BfEdge {
    fn fee_retention(&self) -> Option<f64> {
        exact_fee_retention(
            self.protocol,
            self.kind,
            self.fee_numerator,
            self.fee_denominator,
            self.clmm.as_ref().map(|state| state.fee_rate),
            self.raydium_cpmm_fee.as_ref(),
            self.from_mint == self.pool_base,
        )
    }

    pub fn from_pool_state(p: &arb_types::PoolState) -> Self {
        // BPS is retained for telemetry and legacy API compatibility only.
        // The screening weight uses the protocol's native fee precision.
        let fee = p.fee_bps().unwrap_or(10_000);
        let gamma = exact_fee_retention(
            p.protocol,
            p.kind,
            p.fee_numerator,
            p.fee_denominator,
            p.clmm.as_ref().map(|state| state.fee_rate),
            p.raydium_cpmm_fee.as_ref(),
            true,
        )
        .unwrap_or(0.0);
        let rate = if p.reserve_in > 0 {
            p.reserve_out as f64 / p.reserve_in as f64
        } else {
            0.0
        };
        // This is a candidate-discovery pre-screen only. Every accepted cycle is
        // sized and gated by protocol-native integer quotes below.
        let rg = (rate * gamma).max(1e-18);
        let weight = -rg.ln();
        let active_bin_id = p.dlmm.as_ref().map(|d| d.active_bin_id);
        Self {
            pool_id: p.pool_id,
            protocol: p.protocol,
            kind: p.kind,
            from_mint: p.base_mint,
            to_mint: p.quote_mint,
            pool_base: p.base_mint,
            pool_quote: p.quote_mint,
            reserve_in: p.reserve_in,
            reserve_out: p.reserve_out,
            fee_bps: fee,
            fee_numerator: p.fee_numerator,
            fee_denominator: p.fee_denominator,
            raydium_cpmm_fee: p.raydium_cpmm_fee,
            weight,
            clmm: p.clmm.clone(),
            dlmm: p.dlmm.clone(),
            pump_fun: p.pump_fun.clone(),
            damm_v2: p.damm_v2.clone(),
            transfer_fee_bps: p.transfer_fee_bps,
            token_programs: p.token_programs,
            token2022_fee_context: p.token2022_fee_context,
            execution_accounts: p.execution_accounts.clone(),
            active_bin_id,
        }
    }

    pub fn to_market_edge(&self) -> crate::MarketEdge {
        crate::MarketEdge {
            pool_id: self.pool_id,
            protocol: self.protocol,
            kind: self.kind,
            from_mint: self.from_mint,
            to_mint: self.to_mint,
            pool_base: self.pool_base,
            pool_quote: self.pool_quote,
            reserve_in: self.reserve_in,
            reserve_out: self.reserve_out,
            fee_bps: self.fee_bps,
            fee_numerator: self.fee_numerator,
            fee_denominator: self.fee_denominator,
            raydium_cpmm_fee: self.raydium_cpmm_fee,
            clmm: self.clmm.clone(),
            dlmm: self.dlmm.clone(),
            pump_fun: self.pump_fun.clone(),
            damm_v2: self.damm_v2.clone(),
            transfer_fee_bps: self.transfer_fee_bps,
            token_programs: self.token_programs,
            token2022_fee_context: self.token2022_fee_context,
            execution_accounts: self.execution_accounts.clone(),
            active_bin_id: self.active_bin_id,
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
        let est_capacity = pools.len().saturating_mul(2);
        let mut graph: HashMap<MintId, Vec<BfEdge>> = HashMap::with_capacity(est_capacity);
        let mut node_set = std::collections::HashSet::with_capacity(est_capacity);

        for p in pools {
            if !p.is_active
                || p.reserve_in == 0
                || p.reserve_out == 0
                || p.has_risky_token
                || !crate::pool_token_context_supported(p)
                || !p.is_fee_valid()
            {
                continue;
            }
            let Some(_forward_gamma) = exact_fee_retention(
                p.protocol,
                p.kind,
                p.fee_numerator,
                p.fee_denominator,
                p.clmm.as_ref().map(|state| state.fee_rate),
                p.raydium_cpmm_fee.as_ref(),
                true,
            ) else {
                continue;
            };
            let Some(reverse_gamma) = exact_fee_retention(
                p.protocol,
                p.kind,
                p.fee_numerator,
                p.fee_denominator,
                p.clmm.as_ref().map(|state| state.fee_rate),
                p.raydium_cpmm_fee.as_ref(),
                false,
            ) else {
                continue;
            };
            // Forward edge: base_mint → quote_mint
            let fwd = BfEdge::from_pool_state(p);
            node_set.insert(fwd.from_mint);
            node_set.insert(fwd.to_mint);
            graph.entry(fwd.from_mint).or_default().push(fwd.clone());

            // Reverse edge: quote_mint → base_mint (swap reserves). Creator-fee
            // collection can be side-dependent, so recompute its exact-rate screen.
            let fee = fwd.fee_bps;
            let rev_rate = p.reserve_in as f64 / p.reserve_out as f64;
            let rev_weight = -((rev_rate * reverse_gamma).max(1e-18)).ln();
            let rev = BfEdge {
                pool_id: p.pool_id,
                protocol: p.protocol,
                kind: p.kind,
                from_mint: p.quote_mint,
                to_mint: p.base_mint,
                pool_base: p.base_mint,
                pool_quote: p.quote_mint,
                reserve_in: p.reserve_out,
                reserve_out: p.reserve_in,
                fee_bps: fee,
                fee_numerator: p.fee_numerator,
                fee_denominator: p.fee_denominator,
                raydium_cpmm_fee: p.raydium_cpmm_fee,
                weight: rev_weight,
                clmm: p.clmm.clone(),
                dlmm: p.dlmm.clone(),
                pump_fun: p.pump_fun.clone(),
                damm_v2: p.damm_v2.clone(),
                transfer_fee_bps: p.transfer_fee_bps,
                token_programs: [p.token_programs[1], p.token_programs[0]],
                token2022_fee_context: p.token2022_fee_context,
                execution_accounts: p.execution_accounts.clone(),
                active_bin_id: fwd.active_bin_id,
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

            // Kamino's configured flash reserve and transaction builders use
            // WSOL raw units. A USDC-anchored candidate would otherwise receive
            // a WSOL-sized amount interpreted as USDC raw units. Reject cycles
            // that cannot be rotated to a WSOL borrow/repay anchor.
            if let Some(pos) = cycle_edges
                .iter()
                .position(|e| e.from_mint.0 == arb_constants::wsol_mint())
            {
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
    ///
    /// ACCOUNTING PRECEDENCE: all profit/size decisions here come from
    /// `build_economic_quote` (the official integer CostLedger model) and u128
    /// integer sizing. The `f64` product computed by the caller is ONLY an
    /// initial screening signal to skip obviously unprofitable cycles before
    /// exact evaluation; it is never used for pricing, sizing, or gating.
    fn evaluate_cycle(
        &self,
        cycle: &[BfEdge],
        current_slot: Slot,
        max_trade_size: u64,
        route_counter: u32,
    ) -> Option<ExecutionOpportunity> {
        let mut product = 1.0f64;
        for edge in cycle {
            let gamma = edge.fee_retention()?;
            let rate = if edge.reserve_in > 0 {
                edge.reserve_out as f64 / edge.reserve_in as f64
            } else {
                return None;
            };
            product *= (rate * gamma).max(1e-18);
        }

        if !product.is_finite() || product <= 1.0 {
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
                active_bin_id: e.active_bin_id,
                token_programs: e.token_programs,
                trade_intent: Default::default(),
                execution_accounts: e.execution_accounts.clone(),
            })
            .collect();

        let mut hop_inputs = vec![best_size];
        let mut curr = best_size as u128;
        for e in cycle {
            let gamma = 10_000u128.saturating_sub(e.fee_bps as u128);
            let me = e.to_market_edge();
            curr = crate::quote_pool(e.kind, curr, e.reserve_in, e.reserve_out, gamma, Some(&me));
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
        // ECON-002: gate on a compounded per-hop slippage haircut of the
        // expected final output, not `expected_out` itself; fail closed when
        // the haircut arithmetic fails.
        let minimum_final_out =
            slippage_haircut_min_final(final_out, hops.len(), config.hop_slippage_bps).ok()?;
        let quote = match build_economic_quote(
            base_mint,
            config.base_decimals,
            TokenAmount(best_size),
            TokenAmount(final_out),
            TokenAmount(minimum_final_out),
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
            hop_slippage_bps: config.hop_slippage_bps,
            cu_estimate_units: 0,
            cu_shape_key: String::new(),
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

            expected_min_final_output: minimum_final_out,
        })
    }
}

/// Optimize one route against the same protocol-native integer quote path used
/// by the final cycle evaluator. The bounded search is explicit: it reports its
/// best observed candidate rather than claiming a global proof outside an
/// exhaustively enumerated raw-unit domain.
fn compute_cycle_profit(
    path: &[BfEdge],
    max_trade_size: u64,
    flash_loan_fee_sf: u64,
) -> (u64, u64) {
    if path.is_empty() || max_trade_size == 0 {
        return (0, 0);
    }

    let market_edges: Vec<crate::MarketEdge> = path.iter().map(BfEdge::to_market_edge).collect();
    let best = crate::route_sizing::optimize_bounded_non_monotonic_with_budget(
        max_trade_size,
        24,
        8,
        10,
        &[],
        crate::ROUTE_SIZE_SEARCH_BUDGET,
        |size| {
            let mut amount = u128::from(size);
            for (edge, market_edge) in path.iter().zip(&market_edges) {
                let quote = crate::quote_pool_detailed(
                    edge.kind,
                    amount,
                    edge.reserve_in,
                    edge.reserve_out,
                    10_000,
                    Some(market_edge),
                )?;
                amount = quote.amount_out;
                if amount == 0 {
                    return None;
                }
            }

            let final_output = u64::try_from(amount).ok()?;
            let flash_fee = calculate_flash_fee_raw(size, flash_loan_fee_sf).ok()?;
            let net_score = i128::from(final_output) - i128::from(size) - i128::from(flash_fee);
            Some((final_output, net_score))
        },
    );

    let Some(best) = best.filter(|candidate| candidate.score > 0) else {
        return (0, 0);
    };
    let gross_profit = best.candidate.saturating_sub(best.input);
    if gross_profit == 0 {
        (0, 0)
    } else {
        (best.input, gross_profit)
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
            priority_fee_micro_lamports: 0,
            base_decimals: 9,
            lamports_to_base_mint_rate: 1,
            lamports_to_base_rate_denom: 1,
            min_profit_raw: TokenAmount::ZERO,
            hop_slippage_bps: arb_types::DEFAULT_HOP_SLIPPAGE_BPS,
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
            raydium_cpmm_fee: None,
            is_active: true,
            has_risky_token: false,
            last_update_slot: 0,
            last_update_ms: 0,
            kind: PoolKind::Cpmm,
            clmm: None,
            dlmm: None,
            pump_fun: None,
            damm_v2: None,
            transfer_fee_bps: 0,
            token_programs: Default::default(),
            token2022_fee_context: None,
            execution_accounts: None,
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

    #[test]
    fn bellman_ford_weight_uses_native_sub_bps_fee_ratio() {
        let mut pool = make_pool(10, 1, 2, 100, 200, 1);
        pool.fee_numerator = 50;
        pool.fee_denominator = 1_000_000;

        let edge = BfEdge::from_pool_state(&pool);
        let exact = -(2.0f64 * (1.0 - 50.0 / 1_000_000.0)).ln();
        let rounded_bps = -(2.0f64 * (1.0 - 1.0 / 10_000.0)).ln();

        assert!((edge.weight - exact).abs() < 1e-12);
        assert!(
            (edge.weight - rounded_bps).abs() > 1e-6,
            "BPS ceiling must not replace the native sub-bps fee ratio"
        );
    }

    #[test]
    fn cycle_sizing_uses_protocol_quotes_for_every_candidate_size() {
        let mut first = make_pool(10, 1, 2, 1_000_000, 1_100_000, 1);
        let mut second = make_pool(20, 2, 1, 1_000_000, 1_100_000, 1);
        first.fee_numerator = 50;
        first.fee_denominator = 1_000_000;
        second.fee_numerator = 50;
        second.fee_denominator = 1_000_000;

        let path = [
            BfEdge::from_pool_state(&first),
            BfEdge::from_pool_state(&second),
        ];
        let (selected_size, selected_profit) = compute_cycle_profit(&path, 64, 0);
        assert!(selected_size > 0);
        assert!(selected_profit > 0);

        let market_edges: Vec<_> = path.iter().map(BfEdge::to_market_edge).collect();
        let selected_score = i128::from(selected_profit);
        for size in 1..=64u64 {
            let mut amount = u128::from(size);
            let mut valid = true;
            for (edge, market) in path.iter().zip(&market_edges) {
                match crate::quote_pool_detailed(
                    edge.kind,
                    amount,
                    edge.reserve_in,
                    edge.reserve_out,
                    10_000,
                    Some(market),
                ) {
                    Some(quote) => amount = quote.amount_out,
                    None => {
                        valid = false;
                        break;
                    }
                }
            }
            if valid {
                if let Ok(final_output) = u64::try_from(amount) {
                    let score = i128::from(final_output) - i128::from(size);
                    assert!(
                        selected_score >= score,
                        "selected exact-quote score {selected_score} was worse than size {size} score {score}"
                    );
                }
            }
        }
    }
}
