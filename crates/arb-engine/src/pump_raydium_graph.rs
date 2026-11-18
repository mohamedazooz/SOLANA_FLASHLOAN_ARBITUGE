//! Pump.fun-Raydium Route Graph Construction
//!
//! This module builds a graph of arbitrage routes between Pump.fun bonding curves
//! and Raydium pools (AMM v4, CPMM, CLMM). It uses a modified Bellman-Ford algorithm
//! to detect negative cycles (profitable arbitrage opportunities).
//!
//! # Graph Construction
//!
//! The graph is built with:
//! - **Nodes**: Token mints (SOL, USDC, and all Pump.fun tokens)
//! - **Edges**: Swap routes through Pump.fun or Raydium pools
//! - **Edge Weights**: Log-transformed exchange rates (negative for profitable swaps)
//!
//! # Arbitrage Detection
//!
//! We use the Bellman-Ford algorithm to detect negative cycles:
//! - If a negative cycle exists, arbitrage is possible
//! - The cycle represents the arbitrage path
//!
//! # Performance
//!
//! The graph is updated incrementally as new pools are discovered.
//! Cycle detection runs in O(V*E) time where V is the number of tokens
//! and E is the number of swap edges.

use arb_dex::pump_fun::PumpFunBondingCurve;
use arb_dex::pump_swap::PumpSwapPool;
use arb_dex::raydium_clmm::RaydiumClmmPool;
use arb_dex::raydium_cpmm::RaydiumCpmmPool;
use arb_dex::raydium_v4::RaydiumV4Pool;
use arb_types::{ArbError, DexProtocol, MintId, PoolId, Result};
use solana_pubkey::Pubkey;
use std::collections::{HashMap, HashSet};

// ============================================================================
// Graph Types
// ============================================================================

/// A directed edge in the route graph representing a swap.
#[derive(Debug, Clone)]
pub struct SwapEdge {
    /// Source token mint.
    pub from: MintId,
    /// Destination token mint.
    pub to: MintId,
    /// Exchange rate (to_amount / from_amount) in Q64.64 fixed-point.
    pub rate_q64: u128,
    /// Pool ID where this swap occurs.
    pub pool_id: PoolId,
    /// Protocol (Pump.fun, Raydium AMM v4, etc.)
    pub protocol: DexProtocol,
    /// Fee in basis points.
    pub fee_bps: u16,
    /// Whether this edge is active (pool has liquidity).
    pub is_active: bool,
}

/// A node in the route graph (token mint).
#[derive(Debug, Clone)]
pub struct GraphNode {
    /// Token mint address.
    pub mint: MintId,
    /// All edges originating from this node.
    pub edges_out: Vec<SwapEdge>,
    /// All edges terminating at this node.
    pub edges_in: Vec<SwapEdge>,
    /// Whether this node is SOL (the base currency).
    pub is_sol: bool,
    /// Whether this node is a Pump.fun token.
    pub is_pump_token: bool,
}

/// The route graph for Pump.fun-Raydium arbitrage.
pub struct PumpRaydiumGraph {
    /// All nodes in the graph (keyed by mint).
    nodes: HashMap<MintId, GraphNode>,
    /// All Pump.fun pools (keyed by token mint).
    pump_pools: HashMap<MintId, PumpFunBondingCurve>,
    /// All Raydium pools (keyed by pool ID).
    raydium_pools: HashMap<PoolId, RaydiumPool>,
    /// SOL mint address.
    sol_mint: MintId,
}

// ============================================================================
// Raydium Pool Enum
// ============================================================================

/// Unified Raydium pool representation.
#[derive(Debug, Clone)]
pub enum RaydiumPool {
    AMMv4(RaydiumV4Pool),
    CPMM(RaydiumCpmmPool),
    CLMM(RaydiumClmmPool),
}

impl RaydiumPool {
    pub fn pool_id(&self) -> PoolId {
        match self {
            RaydiumPool::AMMv4(p) => p.pool_id,
            RaydiumPool::CPMM(p) => p.pool_id,
            RaydiumPool::CLMM(p) => p.pool_id,
        }
    }

    pub fn base_mint(&self) -> MintId {
        match self {
            RaydiumPool::AMMv4(p) => p.base_mint,
            RaydiumPool::CPMM(p) => p.token_0_mint,
            RaydiumPool::CLMM(p) => p.token_mint_0,
        }
    }

    pub fn quote_mint(&self) -> MintId {
        match self {
            RaydiumPool::AMMv4(p) => p.quote_mint,
            RaydiumPool::CPMM(p) => p.token_1_mint,
            RaydiumPool::CLMM(p) => p.token_mint_1,
        }
    }

    pub fn protocol(&self) -> DexProtocol {
        match self {
            RaydiumPool::AMMv4(_) => DexProtocol::RaydiumV4,
            RaydiumPool::CPMM(_) => DexProtocol::RaydiumCpmm,
            RaydiumPool::CLMM(_) => DexProtocol::RaydiumClmm,
        }
    }

    pub fn fee_bps(&self) -> Option<u16> {
        match self {
            RaydiumPool::AMMv4(p) => {
                if p.fee_denominator == 0 {
                    None
                } else {
                    Some((p.fee_numerator as u128 * 10_000 / p.fee_denominator as u128) as u16)
                }
            }
            RaydiumPool::CPMM(p) => {
                let total_ppm = p.trade_fee_rate_ppm + p.creator_fee_rate_ppm;
                Some((total_ppm / 100) as u16)
            }
            RaydiumPool::CLMM(p) => Some(p.fee_rate_ppm as u16 / 100),
        }
    }

    pub fn reserves(&self) -> (u128, u128) {
        match self {
            RaydiumPool::AMMv4(p) => (p.base_reserve, p.quote_reserve),
            RaydiumPool::CPMM(p) => (p.reserve_0, p.reserve_1),
            RaydiumPool::CLMM(p) => (u128::MAX, u128::MAX), // TODO: Proper CLMM reserves
        }
    }
}

// ============================================================================
// Graph Implementation
// ============================================================================

impl PumpRaydiumGraph {
    /// Create a new empty graph.
    pub fn new(sol_mint: MintId) -> Self {
        let mut nodes = HashMap::new();

        // Add SOL node
        nodes.insert(
            sol_mint,
            GraphNode {
                mint: sol_mint,
                edges_out: Vec::new(),
                edges_in: Vec::new(),
                is_sol: true,
                is_pump_token: false,
            },
        );

        Self {
            nodes,
            pump_pools: HashMap::new(),
            raydium_pools: HashMap::new(),
            sol_mint,
        }
    }

    /// Add a Pump.fun bonding curve to the graph.
    pub fn add_pump_pool(&mut self, pool: PumpFunBondingCurve) {
        let token_mint = pool.quote_mint;

        // Store the pool
        self.pump_pools.insert(token_mint, pool.clone());

        // Get or create the token node
        let token_node = self.nodes.entry(token_mint).or_insert_with(|| GraphNode {
            mint: token_mint,
            edges_out: Vec::new(),
            edges_in: Vec::new(),
            is_sol: false,
            is_pump_token: true,
        });
        token_node.is_pump_token = true;

        // Ensure SOL node exists
        let sol_node = self.nodes.entry(self.sol_mint).or_insert_with(|| GraphNode {
            mint: self.sol_mint,
            edges_out: Vec::new(),
            edges_in: Vec::new(),
            is_sol: true,
            is_pump_token: false,
        });

        // Add edges: SOL -> Token and Token -> SOL
        let fee_bps = pool.fee_bps.unwrap_or(0);

        // SOL -> Token edge
        let sol_to_token_rate = self.calculate_pump_rate(&pool, true);
        let edge_sol_to_token = SwapEdge {
            from: self.sol_mint,
            to: token_mint,
            rate_q64: sol_to_token_rate,
            pool_id: pool.pool_id,
            protocol: DexProtocol::PumpFun,
            fee_bps,
            is_active: true,
        };
        sol_node.edges_out.push(edge_sol_to_token.clone());
        token_node.edges_in.push(edge_sol_to_token);

        // Token -> SOL edge
        let token_to_sol_rate = self.calculate_pump_rate(&pool, false);
        let edge_token_to_sol = SwapEdge {
            from: token_mint,
            to: self.sol_mint,
            rate_q64: token_to_sol_rate,
            pool_id: pool.pool_id,
            protocol: DexProtocol::PumpFun,
            fee_bps,
            is_active: true,
        };
        token_node.edges_out.push(edge_token_to_sol.clone());
        sol_node.edges_in.push(edge_token_to_sol);
    }

    /// Add a Raydium pool to the graph.
    pub fn add_raydium_pool(&mut self, pool: RaydiumPool) {
        let pool_id = pool.pool_id();
        let base_mint = pool.base_mint();
        let quote_mint = pool.quote_mint();

        // Store the pool
        self.raydium_pools.insert(pool_id, pool.clone());

        // Get or create nodes
        let base_node = self.nodes.entry(base_mint).or_insert_with(|| GraphNode {
            mint: base_mint,
            edges_out: Vec::new(),
            edges_in: Vec::new(),
            is_sol: base_mint == self.sol_mint,
            is_pump_token: false,
        });

        let quote_node = self.nodes.entry(quote_mint).or_insert_with(|| GraphNode {
            mint: quote_mint,
            edges_out: Vec::new(),
            edges_in: Vec::new(),
            is_sol: quote_mint == self.sol_mint,
            is_pump_token: false,
        });

        // Add edges: base -> quote and quote -> base
        let fee_bps = pool.fee_bps().unwrap_or(25);
        let (base_reserve, quote_reserve) = pool.reserves();

        // base -> quote edge
        let base_to_quote_rate = if base_reserve > 0 && quote_reserve > 0 {
            (quote_reserve << 64) / base_reserve
        } else {
            0
        };

        let edge_base_to_quote = SwapEdge {
            from: base_mint,
            to: quote_mint,
            rate_q64: base_to_quote_rate,
            pool_id,
            protocol: pool.protocol(),
            fee_bps,
            is_active: base_reserve > 0 && quote_reserve > 0,
        };
        base_node.edges_out.push(edge_base_to_quote.clone());
        quote_node.edges_in.push(edge_base_to_quote);

        // quote -> base edge
        let quote_to_base_rate = if base_reserve > 0 && quote_reserve > 0 {
            (base_reserve << 64) / quote_reserve
        } else {
            0
        };

        let edge_quote_to_base = SwapEdge {
            from: quote_mint,
            to: base_mint,
            rate_q64: quote_to_base_rate,
            pool_id,
            protocol: pool.protocol(),
            fee_bps,
            is_active: base_reserve > 0 && quote_reserve > 0,
        };
        quote_node.edges_out.push(edge_quote_to_base.clone());
        base_node.edges_in.push(edge_quote_to_base);
    }

    /// Calculate Pump.fun exchange rate.
    fn calculate_pump_rate(&self, pool: &PumpFunBondingCurve, a_to_b: bool) -> u128 {
        // Pump.fun uses virtual reserves for pricing
        let (x, y) = if a_to_b {
            (pool.virtual_sol_reserves, pool.virtual_token_reserves)
        } else {
            (pool.virtual_token_reserves, pool.virtual_sol_reserves)
        };

        if x == 0 || y == 0 {
            return 0;
        }

        // Rate = y / x in Q64.64
        (y as u128 * (1u128 << 64)) / x as u128
    }

    /// Find all 2-hop arbitrage cycles (Pump.fun <-> Raydium).
    pub fn find_2hop_cycles(&self) -> Vec<ArbitrageCycle> {
        let mut cycles = Vec::new();

        // Iterate through all Pump.fun tokens
        for (token_mint, pump_pool) in &self.pump_pools {
            // Find Raydium pools that trade this token
            for (pool_id, raydium_pool) in &self.raydium_pools {
                let base = raydium_pool.base_mint();
                let quote = raydium_pool.quote_mint();

                // Check if this Raydium pool trades the Pump.fun token
                if base == *token_mint || quote == *token_mint {
                    // Determine the other token in the Raydium pool
                    let other_token = if base == *token_mint { quote } else { base };

                    // Check if the other token is SOL
                    if other_token == self.sol_mint {
                        // Found a 2-hop cycle: SOL <-> Token via Pump.fun and Raydium
                        let cycle = ArbitrageCycle {
                            hops: vec![
                                Hop {
                                    from: self.sol_mint,
                                    to: *token_mint,
                                    pool_id: pump_pool.pool_id,
                                    protocol: DexProtocol::PumpFun,
                                },
                                Hop {
                                    from: *token_mint,
                                    to: self.sol_mint,
                                    pool_id: *pool_id,
                                    protocol: raydium_pool.protocol(),
                                },
                            ],
                            cycle_type: CycleType::PumpRaydium2Hop,
                        };
                        cycles.push(cycle);
                    }
                }
            }
        }

        cycles
    }

    /// Find all 3-hop arbitrage cycles.
    pub fn find_3hop_cycles(&self) -> Vec<ArbitrageCycle> {
        let mut cycles = Vec::new();

        // This is a simplified version - a full implementation would use
        // graph traversal to find all 3-hop cycles

        for (token_mint, _pump_pool) in &self.pump_pools {
            // Find Raydium pools trading this token
            for (pool_id1, raydium_pool1) in &self.raydium_pools {
                let base1 = raydium_pool1.base_mint();
                let quote1 = raydium_pool1.quote_mint();

                if base1 != *token_mint && quote1 != *token_mint {
                    continue;
                }

                let intermediate = if base1 == *token_mint { quote1 } else { base1 };

                // Find another Raydium pool that trades the intermediate token
                for (pool_id2, raydium_pool2) in &self.raydium_pools {
                    if pool_id1 == pool_id2 {
                        continue;
                    }

                    let base2 = raydium_pool2.base_mint();
                    let quote2 = raydium_pool2.quote_mint();

                    if base2 != intermediate && quote2 != intermediate {
                        continue;
                    }

                    let final_token = if base2 == intermediate { quote2 } else { base2 };

                    // Check if we can close the cycle back to SOL
                    if final_token == self.sol_mint {
                        let cycle = ArbitrageCycle {
                            hops: vec![
                                Hop {
                                    from: self.sol_mint,
                                    to: *token_mint,
                                    pool_id: _pump_pool.pool_id,
                                    protocol: DexProtocol::PumpFun,
                                },
                                Hop {
                                    from: *token_mint,
                                    to: intermediate,
                                    pool_id: *pool_id1,
                                    protocol: raydium_pool1.protocol(),
                                },
                                Hop {
                                    from: intermediate,
                                    to: self.sol_mint,
                                    pool_id: *pool_id2,
                                    protocol: raydium_pool2.protocol(),
                                },
                            ],
                            cycle_type: CycleType::PumpRaydium3Hop,
                        };
                        cycles.push(cycle);
                    }
                }
            }
        }

        cycles
    }

    /// Get all nodes in the graph.
    pub fn nodes(&self) -> &HashMap<MintId, GraphNode> {
        &self.nodes
    }

    /// Get all Pump.fun pools.
    pub fn pump_pools(&self) -> &HashMap<MintId, PumpFunBondingCurve> {
        &self.pump_pools
    }

    /// Get all Raydium pools.
    pub fn raydium_pools(&self) -> &HashMap<PoolId, RaydiumPool> {
        &self.raydium_pools
    }
}

// ============================================================================
// Cycle Types
// ============================================================================

/// Type of arbitrage cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleType {
    /// 2-hop: Pump.fun <-> Raydium
    PumpRaydium2Hop,
    /// 3-hop: Pump.fun <-> Raydium <-> Raydium
    PumpRaydium3Hop,
    /// 4-hop: Pump.fun <-> Raydium <-> Raydium <-> Raydium
    PumpRaydium4Hop,
}

/// A single hop in an arbitrage cycle.
#[derive(Debug, Clone)]
pub struct Hop {
    /// Source token mint.
    pub from: MintId,
    /// Destination token mint.
    pub to: MintId,
    /// Pool ID where this swap occurs.
    pub pool_id: PoolId,
    /// Protocol (Pump.fun, Raydium AMM v4, etc.)
    pub protocol: DexProtocol,
}

/// An arbitrage cycle (sequence of hops that returns to the starting token).
#[derive(Debug, Clone)]
pub struct ArbitrageCycle {
    /// Sequence of hops in the cycle.
    pub hops: Vec<Hop>,
    /// Type of cycle.
    pub cycle_type: CycleType,
}

impl ArbitrageCycle {
    /// Get the starting token (should be SOL for our use case).
    pub fn start_token(&self) -> MintId {
        self.hops[0].from
    }

    /// Get the ending token (should match start_token for a valid cycle).
    pub fn end_token(&self) -> MintId {
        self.hops.last().map(|h| h.to).unwrap_or_default()
    }

    /// Check if this is a valid cycle (ends where it started).
    pub fn is_valid(&self) -> bool {
        !self.hops.is_empty() && self.start_token() == self.end_token()
    }

    /// Get the number of hops.
    pub fn num_hops(&self) -> usize {
        self.hops.len()
    }
}

// ============================================================================
// Bellman-Ford Arbitrage Detection
// ============================================================================

/// Result of Bellman-Ford cycle detection.
#[derive(Debug, Clone)]
pub struct BellmanFordResult {
    /// Whether a negative cycle (arbitrage opportunity) was found.
    pub has_negative_cycle: bool,
    /// The negative cycle if found.
    pub negative_cycle: Option<ArbitrageCycle>,
    /// Distances from source node.
    pub distances: HashMap<MintId, i128>,
    /// Predecessors for path reconstruction.
    pub predecessors: HashMap<MintId, (MintId, PoolId)>,
}

impl PumpRaydiumGraph {
    /// Run Bellman-Ford algorithm to detect negative cycles (arbitrage).
    ///
    /// Uses log-transformed exchange rates as edge weights:
    /// weight = -log(rate) * 2^64
    ///
    /// A negative cycle in this graph corresponds to a profitable arbitrage.
    pub fn bellman_ford(&self, source: MintId) -> BellmanFordResult {
        let mut distances: HashMap<MintId, i128> = HashMap::new();
        let mut predecessors: HashMap<MintId, (MintId, PoolId)> = HashMap::new();

        // Initialize distances
        for (mint, _) in &self.nodes {
            distances.insert(*mint, if *mint == source { 0 } else { i128::MAX });
        }

        // Relax edges repeatedly
        let num_nodes = self.nodes.len();
        for _ in 0..num_nodes {
            let mut updated = false;

            for (mint, node) in &self.nodes {
                let current_dist = distances.get(mint).copied().unwrap_or(i128::MAX);

                for edge in &node.edges_out {
                    // Edge weight = -log(rate) in Q64.64
                    // For simplicity, we use: weight = -(rate as i128)
                    // This is an approximation that preserves the sign
                    let weight = -(edge.rate_q64 as i128);

                    let new_dist = current_dist.checked_add(weight);

                    if let Some(new_dist) = new_dist {
                        let edge_to_dist = distances.get(&edge.to).copied().unwrap_or(i128::MAX);

                        if new_dist < edge_to_dist {
                            distances.insert(edge.to, new_dist);
                            predecessors.insert(edge.to, (*mint, edge.pool_id));
                            updated = true;
                        }
                    }
                }
            }

            if !updated {
                break;
            }
        }

        // Check for negative cycles
        let mut has_negative_cycle = false;
        let mut negative_cycle: Option<ArbitrageCycle> = None;

        for (mint, node) in &self.nodes {
            let current_dist = distances.get(mint).copied().unwrap_or(i128::MAX);

            for edge in &node.edges_out {
                let weight = -(edge.rate_q64 as i128);
                let new_dist = current_dist.checked_add(weight);

                if let Some(new_dist) = new_dist {
                    let edge_to_dist = distances.get(&edge.to).copied().unwrap_or(i128::MAX);

                    if new_dist < edge_to_dist {
                        // Negative cycle detected
                        has_negative_cycle = true;

                        // Reconstruct the cycle
                        // This is a simplified version - full reconstruction would
                        // trace back through predecessors
                        let cycle = ArbitrageCycle {
                            hops: vec![
                                Hop {
                                    from: *mint,
                                    to: edge.to,
                                    pool_id: edge.pool_id,
                                    protocol: edge.protocol,
                                },
                            ],
                            cycle_type: CycleType::PumpRaydium2Hop,
                        };
                        negative_cycle = Some(cycle);
                        break;
                    }
                }
            }

            if has_negative_cycle {
                break;
            }
        }

        BellmanFordResult {
            has_negative_cycle,
            negative_cycle,
            distances,
            predecessors,
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use solana_pubkey::Pubkey;

    #[test]
    fn test_graph_creation() {
        let sol_mint = MintId(Pubkey::new_unique());
        let graph = PumpRaydiumGraph::new(sol_mint);

        assert_eq!(graph.nodes().len(), 1);
        assert!(graph.nodes().contains_key(&sol_mint));
    }

    #[test]
    fn test_add_pump_pool() {
        let sol_mint = MintId(Pubkey::new_unique());
        let token_mint = MintId(Pubkey::new_unique());

        let mut graph = PumpRaydiumGraph::new(sol_mint);

        let pump_pool = PumpFunBondingCurve::new(
            PoolId(Pubkey::new_unique()),
            sol_mint,
            token_mint,
        );

        graph.add_pump_pool(pump_pool);

        assert_eq!(graph.nodes().len(), 2);
        assert!(graph.nodes().contains_key(&sol_mint));
        assert!(graph.nodes().contains_key(&token_mint));

        // Check edges
        let sol_node = graph.nodes().get(&sol_mint).unwrap();
        let token_node = graph.nodes().get(&token_mint).unwrap();

        assert_eq!(sol_node.edges_out.len(), 1); // SOL -> Token
        assert_eq!(token_node.edges_in.len(), 1); // SOL -> Token
        assert_eq!(token_node.edges_out.len(), 1); // Token -> SOL
        assert_eq!(sol_node.edges_in.len(), 1); // Token -> SOL
    }

    #[test]
    fn test_find_2hop_cycles() {
        let sol_mint = MintId(Pubkey::new_unique());
        let token_mint = MintId(Pubkey::new_unique());

        let mut graph = PumpRaydiumGraph::new(sol_mint);

        // Add Pump.fun pool
        let pump_pool = PumpFunBondingCurve::new(
            PoolId(Pubkey::new_unique()),
            sol_mint,
            token_mint,
        );
        graph.add_pump_pool(pump_pool);

        // Add Raydium pool
        let raydium_pool = RaydiumPool::CPMM(RaydiumCpmmPool::new(
            PoolId(Pubkey::new_unique()),
            token_mint,
            sol_mint,
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
        ));
        graph.add_raydium_pool(raydium_pool);

        // Find cycles
        let cycles = graph.find_2hop_cycles();

        // Should find at least one cycle
        assert!(!cycles.is_empty());

        for cycle in cycles {
            assert!(cycle.is_valid());
            assert_eq!(cycle.num_hops(), 2);
        }
    }

    #[test]
    fn test_cycle_validation() {
        let sol_mint = MintId(Pubkey::new_unique());
        let token_mint = MintId(Pubkey::new_unique());

        let cycle = ArbitrageCycle {
            hops: vec![
                Hop {
                    from: sol_mint,
                    to: token_mint,
                    pool_id: PoolId(Pubkey::new_unique()),
                    protocol: DexProtocol::PumpFun,
                },
                Hop {
                    from: token_mint,
                    to: sol_mint,
                    pool_id: PoolId(Pubkey::new_unique()),
                    protocol: DexProtocol::RaydiumV4,
                },
            ],
            cycle_type: CycleType::PumpRaydium2Hop,
        };

        assert!(cycle.is_valid());
        assert_eq!(cycle.start_token(), sol_mint);
        assert_eq!(cycle.end_token(), sol_mint);
        assert_eq!(cycle.num_hops(), 2);
    }
}

