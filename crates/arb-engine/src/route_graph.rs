//! Unified, protocol-neutral route graph.
//!
//! The graph discovers paths from the mint topology only. Every hop is then
//! priced by the protocol-specific adapter registry in `adapters`; it never
//! assumes that two pools in the same AMM family share fee or state semantics.

use crate::{HopRequest, RouteQuote, quote_route};
use arb_types::{MintId, PoolId, PoolState, Result};
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct RouteCandidate {
    pub input_mint: MintId,
    pub hops: Vec<HopRequest>,
    pub pool_ids: Vec<PoolId>,
}

#[derive(Debug, Clone)]
pub struct QuotedCycle {
    pub candidate: RouteCandidate,
    pub quote: RouteQuote,
    pub gross_profit_raw: i128,
}

#[derive(Debug, Clone)]
pub struct UnifiedRouteGraph {
    pools: Vec<PoolState>,
}

impl UnifiedRouteGraph {
    pub fn new(pools: Vec<PoolState>) -> Self {
        Self { pools }
    }

    pub fn pools(&self) -> &[PoolState] {
        &self.pools
    }

    /// Enumerate simple closed cycles from `anchor_mint`.
    ///
    /// `max_hops` is clamped to 2..=8 and `max_routes` bounds CPU/memory use.
    /// Each pool may appear at most once in a candidate. Discovery includes
    /// every configured protocol; unsupported adapters are rejected later by
    /// quote construction, not silently mapped to a different curve.
    pub fn enumerate_cycles(
        &self,
        anchor_mint: MintId,
        max_hops: usize,
        max_routes: usize,
    ) -> Vec<RouteCandidate> {
        if max_routes == 0 || self.pools.len() < 2 {
            return Vec::new();
        }
        let max_hops = max_hops.clamp(2, 8);
        let mut out = Vec::new();
        let mut hops = Vec::with_capacity(max_hops);
        let mut used_pools = HashSet::with_capacity(max_hops);
        let mut visited_mints = HashSet::with_capacity(max_hops);
        visited_mints.insert(anchor_mint);
        self.walk(
            anchor_mint,
            anchor_mint,
            max_hops,
            max_routes,
            &mut hops,
            &mut used_pools,
            &mut visited_mints,
            &mut out,
        );
        out.sort_by(|a, b| {
            a.hops.len().cmp(&b.hops.len()).then_with(|| {
                a.pool_ids
                    .iter()
                    .map(|id| id.to_bytes())
                    .cmp(b.pool_ids.iter().map(|id| id.to_bytes()))
            })
        });
        out.truncate(max_routes);
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn walk(
        &self,
        anchor: MintId,
        current: MintId,
        max_hops: usize,
        max_routes: usize,
        hops: &mut Vec<HopRequest>,
        used_pools: &mut HashSet<PoolId>,
        visited_mints: &mut HashSet<MintId>,
        out: &mut Vec<RouteCandidate>,
    ) {
        if out.len() >= max_routes || hops.len() >= max_hops {
            return;
        }
        for pool in &self.pools {
            if out.len() >= max_routes || used_pools.contains(&pool.pool_id) || !pool.is_active {
                continue;
            }
            let next = if pool.base_mint == current {
                pool.quote_mint
            } else if pool.quote_mint == current {
                pool.base_mint
            } else {
                continue;
            };

            used_pools.insert(pool.pool_id);
            hops.push(HopRequest {
                pool_id: pool.pool_id,
                mint_in: current,
            });

            if next == anchor && hops.len() >= 2 {
                let route_hops = hops.clone();
                let pool_ids = route_hops.iter().map(|h| h.pool_id).collect();
                out.push(RouteCandidate {
                    input_mint: anchor,
                    hops: route_hops,
                    pool_ids,
                });
            } else if hops.len() < max_hops && visited_mints.insert(next) {
                self.walk(
                    anchor,
                    next,
                    max_hops,
                    max_routes,
                    hops,
                    used_pools,
                    visited_mints,
                    out,
                );
                visited_mints.remove(&next);
            }

            hops.pop();
            used_pools.remove(&pool.pool_id);
        }
    }

    /// Quote all discovered cycles against private route-local pool copies.
    /// Failed/unsupported candidates are skipped, never priced by a fallback.
    pub fn quote_cycles(
        &self,
        anchor_mint: MintId,
        amount_in: u64,
        max_hops: usize,
        max_routes: usize,
    ) -> Vec<QuotedCycle> {
        let candidates = self.enumerate_cycles(anchor_mint, max_hops, max_routes);
        let mut quoted = Vec::new();
        for candidate in candidates {
            let mut route_pools = self.pools.clone();
            if let Ok(quote) = quote_route(&mut route_pools, &candidate.hops, amount_in) {
                if quote.output_mint != candidate.input_mint {
                    continue;
                }
                let gross_profit_raw = i128::from(quote.amount_out) - i128::from(amount_in);
                quoted.push(QuotedCycle {
                    candidate,
                    quote,
                    gross_profit_raw,
                });
            }
        }
        quoted.sort_by_key(|a| std::cmp::Reverse(a.gross_profit_raw));
        quoted
    }

    /// Re-price one previously discovered candidate. This function is pure
    /// with respect to the graph snapshot: caller state is not mutated.
    pub fn quote_candidate(
        &self,
        candidate: &RouteCandidate,
        amount_in: u64,
    ) -> Result<RouteQuote> {
        let mut route_pools = self.pools.clone();
        let quote = quote_route(&mut route_pools, &candidate.hops, amount_in)?;
        if quote.input_mint != quote.output_mint {
            return Err(arb_types::ArbError::InvalidInput(
                "candidate is not a closed arbitrage cycle".into(),
            ));
        }
        Ok(quote)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_types::{DexProtocol, MintId, PoolKind, Slot};
    use solana_pubkey::Pubkey;

    fn mint(n: u8) -> MintId {
        MintId(Pubkey::new_from_array([n; 32]))
    }

    fn cp_pool(id: u8, a: MintId, b: MintId) -> PoolState {
        PoolState {
            pool_id: PoolId::new([id; 32]),
            protocol: DexProtocol::RaydiumV4,
            slot: Slot(1),
            write_version: 1,
            base_mint: a,
            quote_mint: b,
            decimals_a: 9,
            decimals_b: 9,
            reserve_in: 1_000_000_000,
            reserve_out: 1_000_000_000,
            fee_numerator: 25,
            fee_denominator: 10_000,
            is_active: true,
            has_risky_token: false,
            kind: PoolKind::Cpmm,
            clmm: None,
            dlmm: None,
            pump_fun: None,
            transfer_fee_bps: 0,
            token_programs: Default::default(),
            execution_accounts: None,
            last_update_slot: 1,
            last_update_ms: 1,
        }
    }

    #[test]
    fn graph_discovers_two_pool_cycle_and_preserves_mint_continuity() {
        let a = mint(1);
        let b = mint(2);
        let graph = UnifiedRouteGraph::new(vec![cp_pool(1, a, b), cp_pool(2, a, b)]);
        let cycles = graph.enumerate_cycles(a, 4, 16);
        assert!(!cycles.is_empty());
        assert!(cycles.iter().all(|c| c.hops.len() == 2));
        assert!(cycles.iter().all(|c| c.pool_ids[0] != c.pool_ids[1]));
    }

    #[test]
    fn graph_does_not_join_disconnected_mints() {
        let a = mint(1);
        let b = mint(2);
        let c = mint(3);
        let graph = UnifiedRouteGraph::new(vec![cp_pool(1, a, b), cp_pool(2, c, a)]);
        // A->B cannot continue to C because no pool connects B to C.
        assert!(graph.enumerate_cycles(b, 4, 16).is_empty());
    }
}

/// Request for one hop through a concrete, protocol-owned `AmmPool` instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdapterHopRequest {
    pub pool_id: PoolId,
    pub mint_in: MintId,
}

#[derive(Debug, Clone)]
pub struct AdapterRouteCandidate {
    pub input_mint: MintId,
    pub hops: Vec<AdapterHopRequest>,
}

#[derive(Debug, Clone)]
pub struct AdapterHopQuote {
    pub pool_id: PoolId,
    pub protocol: arb_types::DexProtocol,
    pub mint_in: MintId,
    pub mint_out: MintId,
    pub amount_in: u64,
    pub amount_out: u64,
    /// False means quote-only: the pool decoder did not provide all accounts
    /// required by the corresponding transaction builder.
    pub execution_ready: bool,
}

#[derive(Debug, Clone)]
pub struct AdapterRouteQuote {
    pub input_mint: MintId,
    pub output_mint: MintId,
    pub amount_in: u64,
    pub amount_out: u64,
    pub hops: Vec<AdapterHopQuote>,
    pub all_hops_execution_ready: bool,
}

/// Route graph over concrete `arb_dex::AmmPool` implementations. Unlike the
/// compact `PoolState` graph, this graph invokes each venue's own decoder-backed
/// `quote()` implementation, so DAMM v2, LaunchLab, and PumpSwap do not fall
/// through to a generic CPMM/CLMM formula.
pub struct AdapterRouteGraph<'a> {
    pools: Vec<&'a dyn arb_dex::AmmPool>,
}

impl<'a> AdapterRouteGraph<'a> {
    pub fn new(pools: Vec<&'a dyn arb_dex::AmmPool>) -> Self {
        Self { pools }
    }

    pub fn enumerate_cycles(
        &self,
        anchor_mint: MintId,
        max_hops: usize,
        max_routes: usize,
    ) -> Vec<AdapterRouteCandidate> {
        if self.pools.len() < 2 || max_routes == 0 {
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut hops = Vec::new();
        let mut used = HashSet::new();
        let mut visited = HashSet::new();
        visited.insert(anchor_mint);
        self.walk_adapter(
            anchor_mint,
            anchor_mint,
            max_hops.clamp(2, 8),
            max_routes,
            &mut hops,
            &mut used,
            &mut visited,
            &mut out,
        );
        out.sort_by(|a, b| {
            a.hops.len().cmp(&b.hops.len()).then_with(|| {
                a.hops
                    .iter()
                    .map(|h| h.pool_id.to_bytes())
                    .cmp(b.hops.iter().map(|h| h.pool_id.to_bytes()))
            })
        });
        out.truncate(max_routes);
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn walk_adapter(
        &self,
        anchor: MintId,
        current: MintId,
        max_hops: usize,
        max_routes: usize,
        hops: &mut Vec<AdapterHopRequest>,
        used: &mut HashSet<PoolId>,
        visited: &mut HashSet<MintId>,
        out: &mut Vec<AdapterRouteCandidate>,
    ) {
        if out.len() >= max_routes || hops.len() >= max_hops {
            return;
        }
        for pool in &self.pools {
            let id = pool.pool_id();
            if used.contains(&id) || out.len() >= max_routes {
                continue;
            }
            let (next, connected) = if pool.base_mint() == current {
                (pool.quote_mint(), true)
            } else if pool.quote_mint() == current {
                (pool.base_mint(), true)
            } else {
                (current, false)
            };
            if !connected {
                continue;
            }
            used.insert(id);
            hops.push(AdapterHopRequest {
                pool_id: id,
                mint_in: current,
            });
            if next == anchor && hops.len() >= 2 {
                out.push(AdapterRouteCandidate {
                    input_mint: anchor,
                    hops: hops.clone(),
                });
            } else if hops.len() < max_hops && visited.insert(next) {
                self.walk_adapter(anchor, next, max_hops, max_routes, hops, used, visited, out);
                visited.remove(&next);
            }
            hops.pop();
            used.remove(&id);
        }
    }

    pub fn quote_candidate(
        &self,
        candidate: &AdapterRouteCandidate,
        amount_in: u64,
    ) -> Result<AdapterRouteQuote> {
        if amount_in == 0 || candidate.hops.is_empty() {
            return Err(arb_types::ArbError::InvalidInput(
                "zero input or empty adapter route".into(),
            ));
        }
        let mut current_mint = candidate.input_mint;
        let mut current_amount = amount_in;
        let mut hop_quotes = Vec::with_capacity(candidate.hops.len());
        let mut all_execution_ready = true;
        let mut used = HashSet::with_capacity(candidate.hops.len());

        for hop in &candidate.hops {
            if hop.mint_in != current_mint || !used.insert(hop.pool_id) {
                return Err(arb_types::ArbError::InvalidInput(
                    "adapter route is discontinuous or reuses a pool".into(),
                ));
            }
            let pool = self
                .pools
                .iter()
                .copied()
                .find(|p| p.pool_id() == hop.pool_id)
                .ok_or(arb_types::ArbError::PoolNotFound(hop.pool_id))?;
            let a_to_b = if pool.base_mint() == current_mint {
                true
            } else if pool.quote_mint() == current_mint {
                false
            } else {
                return Err(arb_types::ArbError::InvalidInput(
                    "hop input mint is not part of selected pool".into(),
                ));
            };
            if pool.has_risky_token() || pool.transfer_fees_bps() != (0, 0) {
                return Err(arb_types::ArbError::InvalidInput(
                    "Token-2022 transfer fees/extensions require epoch-aware mint-level quote support".into(),
                ));
            }
            if pool.fee_bps().is_none() {
                return Err(arb_types::ArbError::InvalidInput(
                    "pool fee configuration is missing".into(),
                ));
            }
            let output = pool.quote(current_amount, a_to_b)?;
            if output == 0 {
                return Err(arb_types::ArbError::InvalidInput(
                    "adapter returned zero output".into(),
                ));
            }
            let mint_out = if a_to_b {
                pool.quote_mint()
            } else {
                pool.base_mint()
            };
            let execution_ready = pool.execution_accounts().is_some();
            all_execution_ready &= execution_ready;
            hop_quotes.push(AdapterHopQuote {
                pool_id: hop.pool_id,
                protocol: pool.protocol(),
                mint_in: current_mint,
                mint_out,
                amount_in: current_amount,
                amount_out: output,
                execution_ready,
            });
            current_amount = output;
            current_mint = mint_out;
        }
        if current_mint != candidate.input_mint {
            return Err(arb_types::ArbError::InvalidInput(
                "adapter route does not close back to its input mint".into(),
            ));
        }
        Ok(AdapterRouteQuote {
            input_mint: candidate.input_mint,
            output_mint: current_mint,
            amount_in,
            amount_out: current_amount,
            hops: hop_quotes,
            all_hops_execution_ready: all_execution_ready,
        })
    }
}

#[cfg(test)]
mod adapter_tests {
    use super::*;
    use arb_types::MintId;
    use solana_pubkey::Pubkey;

    fn mint(n: u8) -> MintId {
        MintId(Pubkey::new_from_array([n; 32]))
    }

    #[test]
    fn concrete_protocol_adapters_quote_two_pool_cycle() {
        use arb_dex::{AmmPool, RaydiumV4Pool};
        let a = mint(11);
        let b = mint(12);
        let mut p1 = RaydiumV4Pool::new(
            PoolId::new([11; 32]),
            a,
            b,
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
        );
        let mut p2 = RaydiumV4Pool::new(
            PoolId::new([12; 32]),
            a,
            b,
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
        );
        p1.update_reserves(1_000_000_000, 1_100_000_000);
        p2.update_reserves(1_000_000_000, 1_200_000_000);
        for pool in [&mut p1, &mut p2] {
            pool.fee_numerator = 25;
            pool.fee_denominator = 10_000;
            pool.status = 6;
        }
        let pools: Vec<&dyn AmmPool> = vec![&p1, &p2];
        let graph = AdapterRouteGraph::new(pools);
        let candidates = graph.enumerate_cycles(a, 3, 8);
        assert!(!candidates.is_empty());
        let quote = graph.quote_candidate(&candidates[0], 10_000).unwrap();
        assert_eq!(quote.input_mint, quote.output_mint);
        assert_eq!(quote.hops.len(), 2);
    }
}
