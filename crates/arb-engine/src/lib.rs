//!
//! Pure compute crate: Zero I/O, zero async networking.
//! Implements Base-Asset Anchoring (SOL / USDC) with epsilon threshold and optimal sizing.

use arb_dex::engines::MeteoraDlmmQuoteEngine;

pub mod route_worker;
pub use route_worker::{
    PoolDepth, RouteArchitectWorker, SplitAllocation, SplitLeg, SplitOpportunity,
};
pub mod adapters;
pub use adapters::*;
pub mod route_graph;
pub use route_graph::{
    AdapterHopQuote, AdapterHopRequest, AdapterRouteCandidate, AdapterRouteGraph,
    AdapterRouteQuote, QuotedCycle, RouteCandidate, UnifiedRouteGraph,
};
pub mod guarded;
pub use guarded::{
    GuardedQuoteError, GuardedRouteQuote, quote_route_guarded, revalidate_before_dispatch,
};
pub mod bellman_ford;
pub use bellman_ford::{ArbitrageCycle, BellmanFordEngine, BfEdge};
pub mod cross_pool_dlmm;
pub use cross_pool_dlmm::{
    CrossPoolRejection, CrossPoolRejectionReason, CrossPoolScan, CrossPoolScanStats,
    DlmmCrossPoolConfig,
};
pub mod dlmm_flash_arbitrage;
pub use dlmm_flash_arbitrage::{
    DlmmFlashArbConfig, DlmmFlashArbEvaluation, DlmmFlashArbEvaluator, DlmmFlashArbPool,
    DlmmFlashArbRoute, DlmmHopQuote, DlmmPoolAllowlist, FlashArbRejection, QuoteGateRejection,
    enumerate_routes, flash_borrow_mint_from_env_value,
};
pub mod meteora_pair_discovery;
pub use meteora_pair_discovery::{
    CrossProtocolEligiblePair, CrossProtocolPairDiscovery, CrossProtocolPairDiscoveryReport,
    MeteoraDlmmEligiblePair, MeteoraDlmmPairDiscoveryReport, MeteoraDlmmPoolPairDiscovery,
    PoolRejectionReason,
};
pub mod meteora_quote_engine;
pub use meteora_quote_engine::{MeteoraDlmmCrossPoolQuoteEngine, TwoHopDlmmQuote};
pub mod flash_loan_sizing;
pub mod meteora_fresh_state;
pub mod meteora_size_optimizer;
pub mod three_leg_meme_pipeline;
pub use flash_loan_sizing::{
    FeeAmountByMint, FlashLoanSizingConfig, FlashLoanSizingResult, KAMINO_FRACTION_FRAC_BITS,
    KAMINO_FRACTION_ONE_BITS, calculate_kamino_flash_fee, calculate_optimal_flash_loan,
    net_profit_lamports,
};
pub use meteora_fresh_state::{FreshnessConfig, FreshnessVerdict, evaluate_freshness};
pub use meteora_size_optimizer::{
    DlmmArbitrageDirection, DlmmOptimizerConfig, MeteoraDlmmTradeSizeOptimizer,
    MeteoraOptimalSizeResult,
};
pub use three_leg_meme_pipeline::{
    THREE_LEG_PIPELINE_TOP_K_MAX, ThreeLegMemeContextParams, ThreeLegMemeLegQuote,
    ThreeLegMemePipelineError, ThreeLegMemePipelineResult, ThreeLegMemeSizeSearchResult,
    build_three_leg_meme_pipeline, build_three_leg_meme_pipeline_top_k, diagnose_missing_cycles,
    simulate_profitability_gate,
};
pub mod three_pool_router;
pub use three_pool_router::*;
pub mod route_sizing;
pub use route_sizing::{SizeSearchResult, build_size_ladder, optimize_bounded_non_monotonic};
pub mod units;
pub use units::{net_lamports_equiv, wsol_mint_pubkey};

use arb_math::{
    DlmmBin as ArbDlmmBin, DlmmPoolState,
    clmm::ClmmTickMath,
    cpmm_out, cpmm_price_impact_bps,
    economic::{
        EconomicConfig, base_raw_to_lamports, build_economic_quote, calculate_flash_fee_raw,
        slippage_haircut_min_final,
    },
    quote_clmm, quote_clmm_with_ticks_per_array_and_math, quote_dlmm_view,
};
use arb_types::{
    Bps, DexProtocol, ExecutionOpportunity, Lamports, MintId, PoolExecutionAccounts, PoolId,
    PoolKind, PoolState, RouteId, Slot, SwapHop, TokenAmount, TokenProgramKind,
};
use std::collections::HashMap;

/// Strictly-positive-gross epsilon for the coarse cross-pool scan. Integer math
/// has no roundoff, so 1 bps only filters dust (`gross * 10_000 / size < 1`);
/// acceptance is decided downstream by exact net-profit economics, never here.
pub const EPSILON_CYCLE_SPREAD_BPS: u64 = 1;
/// Hard per-slot candidate ceiling for the single production route scanner.
pub const MAX_ROUTE_CANDIDATES_PER_TICK: usize = 4096;
/// Minimum required net profit in lamports to justify execution and Jito tip.
pub const MIN_NET_PROFIT_LAMPORTS: u64 = arb_constants::DEFAULT_MIN_PROFIT_LAMPORTS;

/// Fail-closed conversion of a DLMM edge into the `arb-math` quote view
/// (`DlmmPoolState` + route-local bins). Single source of truth for every
/// consumer that prices DLMM liquidity (`quote_pool`, cross-pool breakpoint
/// seeds): returns `None` when the pool cannot be priced honestly (disabled
/// pool, unsupported function/fee mode, or any bin amount outside the `u64`
/// quote domain). Capacity estimation (`edge_max_input_capacity`) keeps its
/// own saturating clamp by design and must not switch to this helper.
type DlmmQuoteComponents = (DlmmPoolState, Vec<ArbDlmmBin>);
type DlmmQuoteCache = HashMap<PoolId, Option<DlmmQuoteComponents>>;

pub(crate) fn dlmm_quote_components(edge: &MarketEdge) -> Option<DlmmQuoteComponents> {
    if edge.protocol != DexProtocol::MeteoraDlmm
        || edge.transfer_fee_bps != 0
        || edge.token_programs.contains(&TokenProgramKind::Token2022)
    {
        return None;
    }
    let dlmm = edge.dlmm.as_ref()?;
    MeteoraDlmmQuoteEngine::validate_bin_array_snapshot(
        edge.pool_id,
        dlmm,
        edge.execution_accounts.as_ref(),
    )
    .ok()?;
    MeteoraDlmmQuoteEngine::build_dlmm_math_state_from_parts(dlmm, edge.pool_id).ok()
}

/// Profit (output - input) for a cycle at a given input size d (P1 #11: golden-section target).
///
/// Returns `0` (not an error) when the pool cannot be priced honestly — callers
/// treat zero output as "not profitable" and reject the route. This is fail-closed:
/// CLMM/DLMM pools never fall back to the constant-product formula.
pub fn quote_pool(
    kind: PoolKind,
    dx: u128,
    x: u128,
    y: u128,
    gamma: u128,
    edge: Option<&MarketEdge>,
) -> u128 {
    quote_pool_with_dlmm_cache(kind, dx, x, y, gamma, edge, None)
}

/// Internal quote path that reuses validated DLMM math state across every
/// sizing sample for one immutable route snapshot. `Some(cache)` also caches
/// invalid snapshots as `None`, avoiding repeated 600+ bin validation work.
fn quote_pool_with_dlmm_cache(
    kind: PoolKind,
    dx: u128,
    x: u128,
    y: u128,
    gamma: u128,
    edge: Option<&MarketEdge>,
    dlmm_cache: Option<&DlmmQuoteCache>,
) -> u128 {
    match kind {
        PoolKind::Cpmm => {
            if let Some(edge) = edge {
                let Ok(amount_in) = u64::try_from(dx) else {
                    return 0;
                };
                let pool = edge.to_pool_state();
                let a_to_b = edge.pool_base == edge.from_mint;
                crate::adapters::quote_via_dispatcher(&pool, amount_in, a_to_b)
                    .map(|quote| u128::from(quote.amount_out_raw))
                    .unwrap_or(0)
            } else {
                cpmm_out(dx, x, y, gamma).unwrap_or(0)
            }
        }
        PoolKind::Clmm => {
            if let Some(e) = edge {
                if let Some(c) = &e.clmm {
                    if c.liquidity > 0 && c.sqrt_price > 0 && !c.tick_arrays.is_empty() {
                        // Reject invalid fee metadata and values outside the quote
                        // engine's integer domain; never clamp into a different quote.
                        if gamma > 10_000 {
                            return 0;
                        }
                        let fee_rate = if e.protocol == DexProtocol::OrcaWhirlpool {
                            c.fee_rate
                        } else {
                            let fee_rate_u128 = (10_000u128 - gamma) * 100;
                            let Ok(rate) = u16::try_from(fee_rate_u128) else {
                                return 0;
                            };
                            rate
                        };
                        if fee_rate == 0 {
                            return 0;
                        }
                        let Ok(amount) = u64::try_from(dx) else {
                            return 0;
                        };
                        let a_to_b = e.pool_base == e.from_mint;
                        let arrays: Vec<arb_math::clmm::TickArrayFacade> = c
                            .tick_arrays
                            .iter()
                            .map(|ta| arb_math::clmm::TickArrayFacade {
                                start_tick_index: ta.start_tick_index,
                                ticks: ta
                                    .ticks
                                    .iter()
                                    .map(|t| arb_math::clmm::TickFacade {
                                        tick_index: t.tick_index,
                                        initialized: t.initialized,
                                        liquidity_net: t.liquidity_net,
                                        liquidity_gross: 0,
                                        limit_order_unfilled: t.limit_order_unfilled,
                                    })
                                    .collect(),
                            })
                            .collect();
                        let quote = if e.protocol == DexProtocol::OrcaWhirlpool {
                            arb_math::compute_whirlpool_swap(
                                c.sqrt_price,
                                c.tick_current,
                                c.liquidity,
                                c.tick_spacing,
                                fee_rate,
                                amount,
                                a_to_b,
                                &arrays,
                            )
                            .map(|q| q.amount_out_received)
                        } else if e.protocol == DexProtocol::RaydiumClmm {
                            quote_clmm_with_ticks_per_array_and_math(
                                amount,
                                c.sqrt_price,
                                c.tick_current,
                                c.liquidity,
                                c.tick_spacing,
                                fee_rate,
                                a_to_b,
                                &arrays,
                                arb_math::clmm::RAYDIUM_CLMM_TICK_ARRAY_SIZE,
                                ClmmTickMath::Raydium,
                            )
                        } else {
                            quote_clmm(
                                amount,
                                c.sqrt_price,
                                c.tick_current,
                                c.liquidity,
                                c.tick_spacing,
                                fee_rate,
                                a_to_b,
                                &arrays,
                            )
                        };
                        if let Ok(out) = quote {
                            return out as u128;
                        }
                    }
                }
            }
            // Fail-closed: Never fall back to constant-product for CLMM
            0
        }
        PoolKind::Dlmm => {
            if let Some(e) = edge {
                let quote_components = |pool: &DlmmPoolState, bins: &[ArbDlmmBin]| {
                    if bins.is_empty() {
                        return None;
                    }
                    let amount = u64::try_from(dx).ok()?;
                    let a_to_b = e.pool_base == e.from_mint;
                    let quote = quote_dlmm_view(pool, bins, amount, a_to_b, None).ok()?;
                    arb_math::dlmm::validate_swap_depth(quote.bins_crossed)
                        .ok()
                        .map(|()| quote.amount_out as u128)
                };
                let output = match dlmm_cache {
                    Some(cache) => cache
                        .get(&e.pool_id)
                        .and_then(Option::as_ref)
                        .and_then(|(pool, bins)| quote_components(pool, bins)),
                    None => dlmm_quote_components(e)
                        .and_then(|(pool, bins)| quote_components(&pool, &bins)),
                };
                if let Some(output) = output {
                    return output;
                }
            }
            // Fail-closed: Never fall back to linear/CPMM for DLMM
            0
        }
    }
}

pub fn cycle_profit_at(path: &[&MarketEdge], d: u64) -> u64 {
    if d == 0 || path.is_empty() {
        return 0;
    }

    // Materialise one mutable pool per unique pool id so a pool reused twice in
    // the same route is quoted against its already-advanced state.
    let mut pools: Vec<PoolState> = Vec::new();
    for e in path {
        if pools.iter().any(|p| p.pool_id == e.pool_id) {
            continue;
        }
        pools.push(e.to_pool_state());
    }

    let hops: Vec<HopRequest> = path
        .iter()
        .map(|e| HopRequest {
            pool_id: e.pool_id,
            mint_in: e.from_mint,
        })
        .collect();

    match quote_route(&mut pools, &hops, d) {
        Ok(q) if q.amount_out > d => q.amount_out - d,
        _ => 0,
    }
}

/// Directed edge in the market liquidity graph.
#[derive(Debug, Clone)]
pub struct MarketEdge {
    pub pool_id: PoolId,
    pub protocol: DexProtocol,
    pub kind: PoolKind,
    pub from_mint: MintId,
    pub to_mint: MintId,
    /// Canonical base mint of the pool (needed to resolve swap direction).
    pub pool_base: MintId,
    /// Canonical quote mint of the pool.
    pub pool_quote: MintId,
    pub reserve_in: u128,
    pub reserve_out: u128,
    pub fee_bps: u16,
    /// Original protocol fee ratio; BPS is display/pruning metadata only.
    pub fee_numerator: u64,
    pub fee_denominator: u64,
    /// Exact Raydium CPMM fee schedule, including creator-fee side routing.
    pub raydium_cpmm_fee: Option<arb_types::RaydiumCpmmFeeState>,
    /// Concentrated-liquidity state (CLMM). None for CPMM/DLMM.
    pub clmm: Option<arb_types::ClmmState>,
    /// Discrete-bin state (DLMM). None for CPMM/CLMM.
    pub dlmm: Option<arb_types::DlmmState>,
    /// Pump.fun bonding curve state. None for other protocols.
    pub pump_fun: Option<arb_types::PumpFunState>,
    /// Token-2022 transfer fee in bps.
    pub transfer_fee_bps: u16,
    /// Token programs for (from_mint, to_mint).
    pub token_programs: [TokenProgramKind; 2],
    /// Verified on-chain execution accounts resolved for this pool.
    pub execution_accounts: Option<PoolExecutionAccounts>,
    /// Active bin ID for DLMM pools.
    pub active_bin_id: Option<i32>,
}

impl MarketEdge {
    fn to_pool_state(&self) -> PoolState {
        PoolState {
            pool_id: self.pool_id,
            protocol: self.protocol,
            slot: Slot(0),
            write_version: 0,
            base_mint: self.pool_base,
            quote_mint: self.pool_quote,
            decimals_a: 0,
            decimals_b: 0,
            // MarketEdge reserves are oriented from -> to; PoolState reserves
            // are always canonical base -> quote. Restore canonical order before
            // handing the state to a protocol adapter that applies direction.
            reserve_in: if self.from_mint == self.pool_base {
                self.reserve_in
            } else {
                self.reserve_out
            },
            reserve_out: if self.from_mint == self.pool_base {
                self.reserve_out
            } else {
                self.reserve_in
            },
            fee_numerator: self.fee_numerator,
            fee_denominator: self.fee_denominator,
            raydium_cpmm_fee: self.raydium_cpmm_fee,
            is_active: true,
            has_risky_token: false,
            kind: self.kind,
            clmm: self.clmm.clone(),
            dlmm: self.dlmm.clone(),
            pump_fun: self.pump_fun.clone(),
            transfer_fee_bps: self.transfer_fee_bps,
            token_programs: self.token_programs,
            execution_accounts: self.execution_accounts.clone(),
            last_update_slot: 0,
            last_update_ms: 0,
        }
    }
}

/// Directed multi-asset liquidity graph anchored to base currencies.
#[derive(Debug, Default, Clone)]
pub struct MarketGraph {
    pub edges_by_from: HashMap<MintId, Vec<MarketEdge>>,
    pub base_anchors: Vec<MintId>,
    /// Authoritative economic configuration. When `None` the graph cannot price
    /// opportunities and `find_base_cycles` returns empty (fail closed).
    pub economic_config: Option<EconomicConfig>,
}

/// Why pool states did or did not enter the candidate topology.
/// This is a discovery report, not a quote or execution capability verdict.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PoolGraphBuildStats {
    pub pools_seen: usize,
    pub pools_indexed: usize,
    pub edges_indexed: usize,
    pub excluded_inactive: usize,
    pub excluded_raydium_clmm: usize,
    pub excluded_missing_reserves: usize,
    pub excluded_risky_token: usize,
    pub excluded_unmodeled_transfer_fee: usize,
    pub excluded_invalid_fee: usize,
    pub excluded_unsupported_protocol: usize,
}
/// Per-tick scan telemetry for the canonical cycle search.
///
/// Without this, a log line of "0 candidate route(s)" is ambiguous between
/// two fundamentally different causes that demand different fixes:
/// - `gross_profitable == 0`: the market genuinely had no cycle whose raw
///   output exceeded its input (nothing to do — correct fail-closed).
/// - `gross_profitable > 0 && gate_rejected > 0`: cycles existed but the
///   economic gate (flash fee, min profit, Jito tip) rejected them —
///   tuning/observability problem, NOT a market absence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedHopDiagnostic {
    pub pool_id: PoolId,
    pub protocol: DexProtocol,
    pub kind: PoolKind,
    pub from_mint: MintId,
    pub to_mint: MintId,
    /// Informational pool fee tier; already reflected in the quoted output.
    pub fee_bps: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedCycleDiagnostic {
    pub route_id: u32,
    pub base_mint: MintId,
    pub borrow_amount_raw: u64,
    pub expected_final_amount_raw: u64,
    pub gross_profit_raw: u64,
    /// Amount at each route boundary: borrow input followed by each hop output.
    pub hop_amounts_raw: Vec<u64>,
    pub hops: Vec<RejectedHopDiagnostic>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BaseCycleScanStats {
    pub base_anchors: usize,
    pub two_hop_gross_profitable: u64,
    pub three_hop_gross_profitable: u64,
    pub longer_hop_gross_profitable: u64,
    pub gate_rejected: u64,
    pub quote_rejected: u64,
    pub cycles_evaluated: u64,
    pub opportunities: u64,
    pub truncated: bool,
    /// Best strictly-positive-gross sample per rejected concrete route.
    /// Diagnostic only; these samples are not executable opportunities.
    pub rejected_cycles: Vec<RejectedCycleDiagnostic>,
}

struct RouteOptimizationOutcome {
    opportunity: Option<ExecutionOpportunity>,
    gross_profitable_sample: Option<(u64, u64, Vec<u64>)>,
}

impl MarketGraph {
    pub fn new(base_anchors: Vec<MintId>) -> Self {
        Self {
            edges_by_from: HashMap::new(),
            base_anchors,
            economic_config: None,
        }
    }

    /// Set the authoritative economic configuration (flash fee, tip, decimals,
    /// min profit) sourced from verified on-chain / market data.
    pub fn with_economic_config(mut self, config: EconomicConfig) -> Self {
        self.economic_config = Some(config);
        self
    }

    /// Replace the authoritative economic configuration in place (used by the
    /// supervisor to inject freshly verified inputs each cycle without
    /// rebuilding the graph).
    pub fn set_economic_config(&mut self, config: EconomicConfig) {
        self.economic_config = Some(config);
    }

    /// Minimum acceptable profit in base-mint raw units, or 0 if unconfigured.
    pub fn min_profit_raw(&self) -> TokenAmount {
        self.economic_config
            .as_ref()
            .map(|c| c.min_profit_raw)
            .unwrap_or(TokenAmount::ZERO)
    }

    /// Build an `ExecutionOpportunity` using the single authoritative economic
    /// calculation. Returns `None` when no economic config is set (fail closed)
    /// or when the opportunity does not clear the minimum profit floor.
    fn make_economic_opportunity(
        &self,
        base_mint: MintId,
        current_slot: Slot,
        route_id: RouteId,
        size_in: u64,
        final_out: u128,
        hop_inputs: Vec<u64>,
        impact: Bps,
        hops: Vec<SwapHop>,
    ) -> Option<ExecutionOpportunity> {
        let config = self.economic_config.as_ref()?;

        let flash_fee_raw = calculate_flash_fee_raw(size_in, config.flash_loan_fee_sf).ok()?;
        let final_out_u64 = u64::try_from(final_out).ok()?;
        // ECON-002: the profitability gate and the carried min-output floor are
        // both anchored to a compounded per-hop slippage haircut of the expected
        // final output, never to `expected_out` itself. Fail closed on any
        // haircut arithmetic failure by rejecting the opportunity.
        let minimum_final_out =
            slippage_haircut_min_final(final_out_u64, hops.len(), config.hop_slippage_bps).ok()?;
        let quote_timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let quote = build_economic_quote(
            base_mint,
            config.base_decimals,
            TokenAmount(size_in),
            TokenAmount(final_out_u64),
            TokenAmount(minimum_final_out),
            TokenAmount(flash_fee_raw),
            TokenAmount(0),
            TokenAmount(0),
            TokenAmount(0),
            TokenAmount(0),
            Lamports(arb_constants::SOLANA_BASE_FEE_LAMPORTS),
            Lamports::ZERO,
            config.jito_tip_lamports,
            config.lamports_to_base_mint_rate,
            config.lamports_to_base_rate_denom,
            config.min_profit_raw,
            current_slot,
            quote_timestamp_ms,
        )
        .ok()?;

        if !quote.is_profitable() {
            return None;
        }

        let gross_lamports = base_raw_to_lamports(
            quote.gross_profit_raw,
            config.lamports_to_base_mint_rate,
            config.lamports_to_base_rate_denom,
        )
        .ok()?;
        let net_lamports = base_raw_to_lamports(
            quote.net_profit_raw,
            config.lamports_to_base_mint_rate,
            config.lamports_to_base_rate_denom,
        )
        .ok()?;

        Some(ExecutionOpportunity {
            hop_slippage_bps: config.hop_slippage_bps,
            route_id,
            slot: current_slot,
            size_in: TokenAmount(size_in),
            expected_out: TokenAmount(final_out_u64),
            hop_inputs,
            gross_profit: gross_lamports,
            net_profit: net_lamports,
            base_decimals: config.base_decimals,
            price_impact_bps: impact,
            hops,

            expected_min_final_output: minimum_final_out,
        })
    }

    /// Jointly quote and size one concrete directed route using the canonical
    /// economic gate. Liquidity boundaries from each hop become explicit search
    /// seeds, and every sampled size is re-quoted through the complete path.
    fn optimize_route_candidate(
        &self,
        base_mint: MintId,
        current_slot: Slot,
        route_id: RouteId,
        max_trade_size: u64,
        edges: &[&MarketEdge],
    ) -> Option<RouteOptimizationOutcome> {
        if edges.len() < 2 || edges.len() > 6 || max_trade_size == 0 {
            return None;
        }

        let dlmm_cache: DlmmQuoteCache = edges
            .iter()
            .filter(|edge| edge.kind == PoolKind::Dlmm)
            .map(|edge| (edge.pool_id, dlmm_quote_components(edge)))
            .collect();
        let capacities: Vec<Option<u64>> = edges
            .iter()
            .map(|edge| Self::edge_max_input_capacity(edge))
            .collect();
        let mut upper = max_trade_size;
        let mut boundary_seeds = Vec::new();

        // Back-propagate each hop's input ceiling to the route's initial input.
        // Swap output is monotone in input for supported adapters; the final
        // candidate is always re-quoted exactly before eligibility is evaluated.
        for (hop_index, capacity) in capacities.iter().enumerate() {
            let Some(capacity) = capacity else {
                continue;
            };
            if *capacity == 0 {
                return None;
            }
            let mut low = 0u64;
            let mut high = upper;
            while low < high {
                let mid = low + (high - low).div_ceil(2);
                let amount = Self::amount_before_hop(edges, mid, hop_index, Some(&dlmm_cache));
                if amount.is_some_and(|value| value <= u128::from(*capacity)) {
                    low = mid;
                } else {
                    high = mid - 1;
                }
            }
            upper = low;
            if upper > 0 {
                boundary_seeds.push(upper);
                boundary_seeds.push(upper.saturating_sub(1));
            }
        }
        if upper == 0 {
            return None;
        }
        boundary_seeds.push(upper);

        let best_gross_sample = std::cell::RefCell::new(None::<(u64, u64, Vec<u64>)>);
        let result = route_sizing::optimize_bounded_non_monotonic(
            upper,
            32,
            4,
            8,
            &boundary_seeds,
            |size_in| {
                let hop_inputs =
                    Self::quote_route_input_ladder(edges, size_in, &capacities, Some(&dlmm_cache))?;
                let final_out = u128::from(*hop_inputs.last()?);
                if final_out <= u128::from(size_in) {
                    return None;
                }
                let gross = u64::try_from(final_out - u128::from(size_in)).ok()?;
                let replace = best_gross_sample
                    .borrow()
                    .as_ref()
                    .is_none_or(|(_, best, _)| gross > *best);
                if replace {
                    let mut amounts = Vec::with_capacity(hop_inputs.len() + 1);
                    amounts.push(size_in);
                    amounts.extend(hop_inputs.iter().copied());
                    *best_gross_sample.borrow_mut() = Some((size_in, gross, amounts));
                }

                let first = edges.first()?;
                let impact = cpmm_price_impact_bps(
                    u128::from(size_in),
                    first.reserve_in,
                    first.reserve_out,
                    10_000u128.saturating_sub(u128::from(first.fee_bps)),
                );
                let hops = edges
                    .iter()
                    .map(|edge| SwapHop {
                        pool_id: edge.pool_id,
                        protocol: edge.protocol,
                        mint_in: edge.from_mint,
                        mint_out: edge.to_mint,
                        fee_bps: edge.fee_bps,
                        active_bin_id: edge.active_bin_id,
                        token_programs: edge.token_programs,
                        trade_intent: Default::default(),
                        execution_accounts: edge.execution_accounts.clone(),
                    })
                    .collect();
                let opportunity = self.make_economic_opportunity(
                    base_mint,
                    current_slot,
                    route_id,
                    size_in,
                    final_out,
                    hop_inputs,
                    impact,
                    hops,
                )?;
                Some((opportunity.clone(), i128::from(opportunity.net_profit.0)))
            },
        );
        let opportunity = result.map(|result| {
            debug_assert!(result.evaluated_count > 0);
            debug_assert_eq!(result.input, result.candidate.size_in.as_u64());
            debug_assert_eq!(result.score, i128::from(result.candidate.net_profit.0));
            result.candidate
        });
        Some(RouteOptimizationOutcome {
            opportunity,
            gross_profitable_sample: best_gross_sample.borrow().clone(),
        })
    }

    fn amount_before_hop(
        edges: &[&MarketEdge],
        size_in: u64,
        hop_index: usize,
        dlmm_cache: Option<&DlmmQuoteCache>,
    ) -> Option<u128> {
        let mut amount = u128::from(size_in);
        for edge in edges.iter().take(hop_index) {
            let gamma = 10_000u128.saturating_sub(u128::from(edge.fee_bps));
            amount = quote_pool_with_dlmm_cache(
                edge.kind,
                amount,
                edge.reserve_in,
                edge.reserve_out,
                gamma,
                Some(edge),
                dlmm_cache,
            );
        }
        Some(amount)
    }

    /// Return per-hop inputs followed by the final output, rejecting any
    /// candidate that exceeds a hop's active tick/bin input capacity.
    fn quote_route_input_ladder(
        edges: &[&MarketEdge],
        size_in: u64,
        capacities: &[Option<u64>],
        dlmm_cache: Option<&DlmmQuoteCache>,
    ) -> Option<Vec<u64>> {
        let mut amount = u128::from(size_in);
        let mut ladder = Vec::with_capacity(edges.len() + 1);
        ladder.push(size_in);

        for (index, edge) in edges.iter().enumerate() {
            let amount_u64 = u64::try_from(amount).ok()?;
            if capacities
                .get(index)
                .copied()
                .flatten()
                .is_some_and(|capacity| amount_u64 > capacity)
            {
                return None;
            }
            let gamma = 10_000u128.saturating_sub(u128::from(edge.fee_bps));
            amount = quote_pool_with_dlmm_cache(
                edge.kind,
                amount,
                edge.reserve_in,
                edge.reserve_out,
                gamma,
                Some(edge),
                dlmm_cache,
            );
            if amount == 0 {
                return None;
            }
            ladder.push(u64::try_from(amount).ok()?);
        }
        Some(ladder)
    }

    /// Rebuild the route graph from the one verified pool snapshot for this slot.
    /// Unsupported or incomplete states are counted and excluded, never approximated.
    pub fn build_from_pools(&mut self, pools: &[PoolState]) -> PoolGraphBuildStats {
        self.edges_by_from.clear();
        let mut stats = PoolGraphBuildStats {
            pools_seen: pools.len(),
            ..Default::default()
        };

        for p in pools {
            if !p.is_active {
                stats.excluded_inactive += 1;
                continue;
            }
            if p.protocol == DexProtocol::RaydiumClmm {
                stats.excluded_raydium_clmm += 1;
                continue;
            }
            let supported_family = matches!(
                (p.protocol, p.kind),
                (DexProtocol::RaydiumV4, PoolKind::Cpmm)
                    | (DexProtocol::RaydiumCpmm, PoolKind::Cpmm)
                    | (DexProtocol::PumpFun, PoolKind::Cpmm)
                    | (DexProtocol::OrcaWhirlpool, PoolKind::Clmm)
                    | (DexProtocol::MeteoraDlmm, PoolKind::Dlmm)
            );
            if !supported_family {
                stats.excluded_unsupported_protocol += 1;
                continue;
            }
            if p.protocol == DexProtocol::PumpFun
                && (!p
                    .pump_fun
                    .as_ref()
                    .is_some_and(arb_types::PumpFunState::is_tradeable)
                    || p.fee_numerator == 0
                    || p.fee_denominator == 0
                    || p.fee_numerator > p.fee_denominator)
            {
                stats.excluded_unsupported_protocol += 1;
                continue;
            }
            if p.protocol == DexProtocol::RaydiumCpmm
                && !p.raydium_cpmm_fee.is_some_and(|fees| fees.is_valid())
            {
                stats.excluded_invalid_fee += 1;
                continue;
            }
            if p.reserve_in == 0 || p.reserve_out == 0 {
                stats.excluded_missing_reserves += 1;
                continue;
            }
            if p.has_risky_token {
                stats.excluded_risky_token += 1;
                continue;
            }
            if p.transfer_fee_bps != 0 {
                stats.excluded_unmodeled_transfer_fee += 1;
                continue;
            }
            if !p.is_fee_valid() {
                stats.excluded_invalid_fee += 1;
                continue;
            }
            let fee = p.fee_bps().expect("fee validity was checked above");
            stats.pools_indexed += 1;

            self.edges_by_from
                .entry(p.base_mint)
                .or_default()
                .push(MarketEdge {
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
                    clmm: p.clmm.clone(),
                    dlmm: p.dlmm.clone(),
                    pump_fun: p.pump_fun.clone(),
                    transfer_fee_bps: p.transfer_fee_bps,
                    token_programs: p.token_programs,
                    execution_accounts: p.execution_accounts.clone(),
                    active_bin_id: p.dlmm.as_ref().map(|d| d.active_bin_id),
                });
            self.edges_by_from
                .entry(p.quote_mint)
                .or_default()
                .push(MarketEdge {
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
                    clmm: p.clmm.clone(),
                    dlmm: p.dlmm.clone(),
                    pump_fun: p.pump_fun.clone(),
                    transfer_fee_bps: p.transfer_fee_bps,
                    token_programs: [p.token_programs[1], p.token_programs[0]],
                    execution_accounts: p.execution_accounts.clone(),
                    active_bin_id: p.dlmm.as_ref().map(|d| d.active_bin_id),
                });
        }

        for edges in self.edges_by_from.values_mut() {
            edges.sort_by(|a, b| {
                a.to_mint
                    .0
                    .to_bytes()
                    .cmp(&b.to_mint.0.to_bytes())
                    .then_with(|| a.pool_id.0.to_bytes().cmp(&b.pool_id.0.to_bytes()))
            });
        }
        stats.edges_indexed = self.edges_by_from.values().map(Vec::len).sum();
        stats
    }

    /// Maximum input capacity for a market edge within its active tick/bin or real reserves.
    /// Supports Pump.fun, CLMM, and DLMM:
    /// - Pump.fun: limits input by real_token_reserves (Buy) or real_sol_reserves (Sell), returns 0 if complete.
    /// - CLMM: no active-tick-only cap; exact quoting traverses loaded tick arrays and fails closed at missing coverage.
    /// - DLMM: limits input by active bin liquidity amount_x (a_to_b) or amount_y (b_to_a).
    pub fn edge_max_input_capacity(edge: &MarketEdge) -> Option<u64> {
        if edge.protocol == DexProtocol::PumpFun {
            if let Some(st) = &edge.pump_fun {
                if st.complete {
                    return Some(0);
                }
                let a_to_b = edge.pool_base == edge.from_mint;
                if a_to_b {
                    // Buy: Base (WSOL) -> Quote (Meme Token)
                    if st.real_token_reserves == 0 {
                        return Some(0);
                    }
                    if st.virtual_token_reserves > st.real_token_reserves {
                        let num = (st.real_token_reserves as u128)
                            .saturating_mul(st.virtual_sol_reserves as u128)
                            .saturating_mul(10_000);
                        let denom = 9_900u128.saturating_mul(
                            (st.virtual_token_reserves - st.real_token_reserves) as u128,
                        );
                        if let Some(quot) = num.checked_div(denom) {
                            let max_sol = quot.min(u64::MAX as u128) as u64;
                            return Some(max_sol);
                        }
                    }
                    return Some(st.virtual_sol_reserves);
                } else {
                    // Sell: Quote (Meme Token) -> Base (WSOL)
                    if st.real_sol_reserves == 0 {
                        return Some(0);
                    }
                    if st.virtual_sol_reserves > st.real_sol_reserves {
                        let num = (st.real_sol_reserves as u128)
                            .saturating_mul(st.virtual_token_reserves as u128)
                            .saturating_mul(10_000);
                        let denom = 9_900u128.saturating_mul(
                            (st.virtual_sol_reserves - st.real_sol_reserves) as u128,
                        );
                        if let Some(quot) = num.checked_div(denom) {
                            let max_tok = quot.min(u64::MAX as u128) as u64;
                            return Some(max_tok);
                        }
                    }
                    return Some(st.virtual_token_reserves);
                }
            }
        }
        match edge.kind {
            PoolKind::Clmm => {
                // Active-tick capacity truncates liquidity at the first tick
                // even when more tick arrays are loaded. Let exact quote
                // feasibility traverse those arrays and fail closed at their edge.
                None
            }
            PoolKind::Dlmm => {
                let d = edge.dlmm.as_ref()?;
                let a_to_b = edge.pool_base == edge.from_mint;
                let bins: Vec<arb_math::dlmm::DlmmBin> = d
                    .bins
                    .iter()
                    .map(|b| arb_math::dlmm::DlmmBin {
                        id: b.bin_id,
                        amount_x: b.amount_x.min(u64::MAX as u128) as u64,
                        amount_y: b.amount_y.min(u64::MAX as u128) as u64,
                        price_q64: 0,
                        open_order_amount: b.open_order_amount,
                        processed_order_remaining_amount: b.processed_order_remaining_amount,
                        limit_order_ask_side: b.limit_order_ask_side,
                    })
                    .collect();
                arb_math::dlmm::dlmm_multi_bin_max_input(
                    d.active_bin_id,
                    d.bin_step_bps,
                    &bins,
                    a_to_b,
                )
            }
            PoolKind::Cpmm => None,
        }
    }

    /// Compatibility wrapper for older callers. Production uses the one
    /// bounded cycle scanner for every configured hop count.
    pub fn find_base_cycles(
        &self,
        current_slot: Slot,
        max_trade_size: u64,
    ) -> Vec<ExecutionOpportunity> {
        self.find_cycles_with_stats(
            current_slot,
            max_trade_size,
            3,
            MAX_ROUTE_CANDIDATES_PER_TICK,
        )
        .0
    }

    pub fn find_base_cycles_with_stats(
        &self,
        current_slot: Slot,
        max_trade_size: u64,
    ) -> (Vec<ExecutionOpportunity>, BaseCycleScanStats) {
        self.find_cycles_with_stats(
            current_slot,
            max_trade_size,
            3,
            MAX_ROUTE_CANDIDATES_PER_TICK,
        )
    }

    /// Sole production route-discovery engine. It enumerates simple cycles
    /// from borrow anchors and exact-quotes every completed path through the
    /// same protocol-aware optimizer and net-economic gate.
    pub fn find_cycles_with_stats(
        &self,
        current_slot: Slot,
        max_trade_size: u64,
        max_hops: usize,
        max_candidates: usize,
    ) -> (Vec<ExecutionOpportunity>, BaseCycleScanStats) {
        let mut opportunities = Vec::new();
        let mut stats = BaseCycleScanStats {
            base_anchors: self.base_anchors.len(),
            ..Default::default()
        };
        let max_hops = max_hops.clamp(2, 6);
        let max_candidates = max_candidates.min(MAX_ROUTE_CANDIDATES_PER_TICK);
        if max_trade_size == 0 || max_candidates == 0 || self.economic_config.is_none() {
            return (opportunities, stats);
        }

        let mut route_counter = 1u32;
        for &base in &self.base_anchors {
            let mut path = Vec::with_capacity(max_hops);
            let mut visited = std::collections::HashSet::with_capacity(max_hops);
            self.walk_cycles(
                base,
                base,
                max_hops,
                max_candidates,
                &mut path,
                &mut visited,
                current_slot,
                max_trade_size,
                &mut route_counter,
                &mut stats,
                &mut opportunities,
            );
            if stats.truncated {
                break;
            }
        }

        opportunities.sort_by_key(|opportunity| std::cmp::Reverse(opportunity.net_profit.0));
        stats.opportunities = opportunities.len() as u64;
        (opportunities, stats)
    }

    #[allow(clippy::too_many_arguments)]
    fn walk_cycles<'a>(
        &'a self,
        current_mint: MintId,
        base_anchor: MintId,
        max_hops: usize,
        max_candidates: usize,
        path: &mut Vec<&'a MarketEdge>,
        visited_pools: &mut std::collections::HashSet<PoolId>,
        current_slot: Slot,
        max_trade_size: u64,
        route_counter: &mut u32,
        stats: &mut BaseCycleScanStats,
        results: &mut Vec<ExecutionOpportunity>,
    ) {
        if stats.cycles_evaluated as usize >= max_candidates {
            stats.truncated = true;
            return;
        }
        if path.len() >= 2 && current_mint == base_anchor {
            let route_id = RouteId(*route_counter);
            *route_counter = (*route_counter).saturating_add(1);
            stats.cycles_evaluated += 1;
            match self.optimize_route_candidate(
                base_anchor,
                current_slot,
                route_id,
                max_trade_size,
                path,
            ) {
                Some(outcome) => {
                    let gross_sample = outcome.gross_profitable_sample;
                    if gross_sample.is_some() {
                        match path.len() {
                            2 => stats.two_hop_gross_profitable += 1,
                            3 => stats.three_hop_gross_profitable += 1,
                            _ => stats.longer_hop_gross_profitable += 1,
                        }
                    }
                    if let Some(opportunity) = outcome.opportunity {
                        results.push(opportunity);
                    } else if let Some((borrow, gross, hop_amounts)) = gross_sample {
                        stats.gate_rejected += 1;
                        if stats.rejected_cycles.len() < 32 {
                            stats.rejected_cycles.push(RejectedCycleDiagnostic {
                                route_id: route_id.0,
                                base_mint: base_anchor,
                                borrow_amount_raw: borrow,
                                expected_final_amount_raw: borrow.saturating_add(gross),
                                gross_profit_raw: gross,
                                hop_amounts_raw: hop_amounts,
                                hops: path
                                    .iter()
                                    .map(|edge| RejectedHopDiagnostic {
                                        pool_id: edge.pool_id,
                                        protocol: edge.protocol,
                                        kind: edge.kind,
                                        from_mint: edge.from_mint,
                                        to_mint: edge.to_mint,
                                        fee_bps: edge.fee_bps,
                                    })
                                    .collect(),
                            });
                        }
                    } else {
                        stats.quote_rejected += 1;
                    }
                }
                None => stats.quote_rejected += 1,
            }
            return;
        }
        if path.len() >= max_hops {
            return;
        }
        let Some(next_edges) = self.edges_by_from.get(&current_mint) else {
            return;
        };
        for edge in next_edges {
            if visited_pools.contains(&edge.pool_id) {
                continue;
            }
            visited_pools.insert(edge.pool_id);
            path.push(edge);
            self.walk_cycles(
                edge.to_mint,
                base_anchor,
                max_hops,
                max_candidates,
                path,
                visited_pools,
                current_slot,
                max_trade_size,
                route_counter,
                stats,
                results,
            );
            path.pop();
            visited_pools.remove(&edge.pool_id);
            if stats.truncated {
                break;
            }
        }
    }

    /// Re-price a directed route discovered by another detector using this
    /// graph's canonical pool adapters, per-hop capacity checks and net-PnL
    /// optimizer. The supported flash-loan route must close in borrowed WSOL.
    pub fn evaluate_directed_route(
        &self,
        current_slot: Slot,
        max_trade_size: u64,
        route_id: RouteId,
        hops: &[SwapHop],
    ) -> Option<ExecutionOpportunity> {
        if !(2..=6).contains(&hops.len()) || max_trade_size == 0 {
            return None;
        }
        let first = hops.first()?;
        let last = hops.last()?;
        if first.mint_in != last.mint_out
            || first.mint_in.0 != arb_constants::wsol_mint()
            || hops
                .windows(2)
                .any(|pair| pair[0].mint_out != pair[1].mint_in)
        {
            return None;
        }

        let mut seen = std::collections::HashSet::with_capacity(hops.len());
        let mut edges = Vec::with_capacity(hops.len());
        for hop in hops {
            if !seen.insert(hop.pool_id) {
                return None;
            }
            let edge = self.edges_by_from.get(&hop.mint_in)?.iter().find(|edge| {
                edge.pool_id == hop.pool_id
                    && edge.protocol == hop.protocol
                    && edge.from_mint == hop.mint_in
                    && edge.to_mint == hop.mint_out
            })?;
            edges.push(edge);
        }

        self.optimize_route_candidate(
            first.mint_in,
            current_slot,
            route_id,
            max_trade_size,
            &edges,
        )?
        .opportunity
    }

    /// Compatibility wrapper over the canonical route scanner; no parallel DFS.
    pub fn find_multi_hop_cycles(
        &self,
        current_slot: Slot,
        max_trade_size: u64,
        max_hops: usize,
    ) -> Vec<ExecutionOpportunity> {
        self.find_cycles_with_stats(
            current_slot,
            max_trade_size,
            max_hops,
            MAX_ROUTE_CANDIDATES_PER_TICK,
        )
        .0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> EconomicConfig {
        EconomicConfig {
            flash_loan_fee_sf: 0,
            jito_tip_lamports: Lamports::ZERO,
            base_decimals: 9,
            lamports_to_base_mint_rate: 1,
            lamports_to_base_rate_denom: 1,
            min_profit_raw: TokenAmount::ZERO,
            hop_slippage_bps: arb_types::DEFAULT_HOP_SLIPPAGE_BPS,
        }
    }
    use arb_types::{DexProtocol, MintId, PoolId, PoolKind, Slot};

    /// Compute Chronos spatiotemporal score for an execution opportunity.
    /// Incorporates logarithmic profit expectation, latency penalty (lambda * tau), and hop count penalty.
    /// Lower (more negative) score indicates superior execution probability and risk-adjusted edge.
    fn score_opportunity_chronos(
        opp: &ExecutionOpportunity,
        latency_ms: f64,
        lambda_decay: f64,
        beta_risk: f64,
    ) -> f64 {
        let hops_count = opp.hops.len() as f64;
        let scale = 10f64.powi(i32::from(opp.base_decimals));
        let net_profit_sol = opp.net_profit.0 as f64 / scale;
        // Logarithmic financial return: -ln(profit). Higher profit -> more negative
        let profit_term = -(net_profit_sol.max(1e-9)).ln();
        // Latency degradation penalty
        let temporal_term = lambda_decay * (latency_ms / 1_000.0);
        // Multi-hop execution risk penalty
        let risk_term = beta_risk * hops_count;

        profit_term + temporal_term + risk_term
    }

    #[test]
    fn market_graph_excludes_quote_only_protocols_instead_of_using_cpmm_math() {
        let protocols = [
            DexProtocol::MeteoraDammV2,
            DexProtocol::MeteoraDbc,
            DexProtocol::RaydiumLaunchLab,
            DexProtocol::PumpSwap,
        ];
        let pools = protocols
            .into_iter()
            .enumerate()
            .map(|(index, protocol)| {
                let mut pool = make_pool(90 + index as u8, 1, 2, 1_000_000, 2_000_000);
                pool.protocol = protocol;
                pool.kind = PoolKind::Cpmm;
                pool
            })
            .collect::<Vec<_>>();
        let mut graph = MarketGraph::default();
        let stats = graph.build_from_pools(&pools);
        assert_eq!(stats.excluded_unsupported_protocol, protocols.len());
        assert!(graph.edges_by_from.is_empty());
    }

    #[test]
    fn market_graph_rejects_stale_cached_raydium_clmm_pool_states() {
        let base = MintId::new([31; 32]);
        let quote = MintId::new([32; 32]);
        let pool = PoolState {
            pool_id: PoolId::new([33; 32]),
            protocol: DexProtocol::RaydiumClmm,
            slot: Slot(100),
            write_version: 1,
            base_mint: base,
            quote_mint: quote,
            decimals_a: 6,
            decimals_b: 9,
            reserve_in: 1_000_000,
            reserve_out: 2_000_000,
            fee_numerator: 2_500,
            fee_denominator: 1_000_000,
            raydium_cpmm_fee: None,
            is_active: true,
            has_risky_token: false,
            kind: PoolKind::Clmm,
            clmm: Some(arb_types::ClmmState::default()),
            dlmm: None,
            pump_fun: None,
            transfer_fee_bps: 0,
            token_programs: [TokenProgramKind::SplToken; 2],
            execution_accounts: None,
            last_update_slot: 100,
            last_update_ms: 1_700_000_000_000,
        };
        let mut graph = MarketGraph::default();
        graph.build_from_pools(&[pool]);
        assert!(graph.edges_by_from.is_empty());
    }
    #[test]
    fn raydium_clmm_edge_quotes_with_60_tick_span_in_graph_quoting() {
        // Regression for the discovery path: quote_pool previously routed
        // every CLMM edge through the Whirlpool-shaped `quote_clmm` (88-tick
        // span, Orca tick rounding). Raydium arrays start on 60*spacing
        // multiples, so any hydrated Raydium CLMM edge whose active tick sits
        // in such an array failed closed and Raydium liquidity never entered
        // cycle discovery.
        let edge = MarketEdge {
            pool_id: PoolId::new([9; 32]),
            protocol: DexProtocol::RaydiumClmm,
            kind: PoolKind::Clmm,
            from_mint: MintId::new([1; 32]),
            to_mint: MintId::new([2; 32]),
            pool_base: MintId::new([1; 32]),
            pool_quote: MintId::new([2; 32]),
            reserve_in: 1_000_000_000_000,
            reserve_out: 1_000_000,
            fee_bps: 25,
            fee_numerator: 2_500,
            fee_denominator: 10_000,
            raydium_cpmm_fee: None,
            clmm: Some(arb_types::ClmmState {
                sqrt_price: 1u128 << 64,
                liquidity: 1_000_000_000_000,
                tick_current: 70,
                tick_spacing: 1,
                fee_rate: 2_500,
                tick_arrays: vec![
                    arb_types::ClmmTickArray {
                        start_tick_index: 60,
                        ticks: vec![],
                    },
                    arb_types::ClmmTickArray {
                        start_tick_index: 120,
                        ticks: vec![],
                    },
                ],
            }),
            dlmm: None,
            pump_fun: None,
            transfer_fee_bps: 0,
            token_programs: Default::default(),
            execution_accounts: None,
            active_bin_id: None,
        };
        let out = quote_pool(
            PoolKind::Clmm,
            10_000,
            edge.reserve_in,
            edge.reserve_out,
            9_970,
            Some(&edge),
        );
        assert!(out > 0, "Raydium CLMM edge must quote, got 0 (fail-closed)");
    }

    fn make_pool(id: u8, base: u8, quote: u8, r_in: u128, r_out: u128) -> PoolState {
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
            fee_numerator: 25,
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
            transfer_fee_bps: 0,
            token_programs: Default::default(),
            execution_accounts: None,
        }
    }

    #[test]
    fn external_route_requote_requires_contiguous_wsol_cycle() {
        let wsol = MintId(arb_constants::wsol_mint());
        let intermediate = MintId::new([77; 32]);
        let mut first_pool = make_pool(71, 1, 2, 1_000_000, 1_000_000);
        first_pool.base_mint = wsol;
        first_pool.quote_mint = intermediate;
        let mut second_pool = make_pool(72, 2, 1, 1_000_000, 1_500_000);
        second_pool.base_mint = intermediate;
        second_pool.quote_mint = wsol;

        let first_hop = SwapHop {
            pool_id: first_pool.pool_id,
            protocol: DexProtocol::RaydiumV4,
            mint_in: wsol,
            mint_out: intermediate,
            fee_bps: 25,
            active_bin_id: None,
            token_programs: Default::default(),
            trade_intent: Default::default(),
            execution_accounts: None,
        };
        let second_hop = SwapHop {
            pool_id: second_pool.pool_id,
            protocol: DexProtocol::RaydiumV4,
            mint_in: intermediate,
            mint_out: wsol,
            fee_bps: 25,
            active_bin_id: None,
            token_programs: Default::default(),
            trade_intent: Default::default(),
            execution_accounts: None,
        };
        let mut graph = MarketGraph::new(vec![wsol]).with_economic_config(test_config());
        graph.build_from_pools(&[first_pool, second_pool]);

        let valid = vec![first_hop.clone(), second_hop.clone()];
        assert!(
            graph
                .evaluate_directed_route(Slot(2), 100_000, RouteId(91), &valid)
                .is_some()
        );

        let mut broken_chain = valid.clone();
        broken_chain[1].mint_in = MintId::new([78; 32]);
        assert!(
            graph
                .evaluate_directed_route(Slot(2), 100_000, RouteId(92), &broken_chain)
                .is_none()
        );

        let mut non_wsol = valid;
        let usdc = MintId(arb_constants::usdc_mint());
        non_wsol[0].mint_in = usdc;
        non_wsol[1].mint_out = usdc;
        assert!(
            graph
                .evaluate_directed_route(Slot(2), 100_000, RouteId(93), &non_wsol)
                .is_none()
        );
    }

    #[test]
    fn graph_economic_gate_includes_base_network_fee() {
        let base = MintId::new([90; 32]);
        let intermediate = MintId::new([91; 32]);
        let hops = vec![
            SwapHop {
                pool_id: PoolId::new([92; 32]),
                protocol: DexProtocol::RaydiumV4,
                mint_in: base,
                mint_out: intermediate,
                fee_bps: 25,
                active_bin_id: None,
                token_programs: Default::default(),
                trade_intent: Default::default(),
                execution_accounts: None,
            },
            SwapHop {
                pool_id: PoolId::new([93; 32]),
                protocol: DexProtocol::RaydiumV4,
                mint_in: intermediate,
                mint_out: base,
                fee_bps: 25,
                active_bin_id: None,
                token_programs: Default::default(),
                trade_intent: Default::default(),
                execution_accounts: None,
            },
        ];
        let graph = MarketGraph::new(vec![base]).with_economic_config(test_config());
        // A 4,000-raw-unit gross edge is wiped out by the 5,000-lamport
        // base transaction fee (plus the conservative min-out haircut).
        assert!(
            graph
                .make_economic_opportunity(
                    base,
                    Slot(1),
                    RouteId(94),
                    10_000,
                    14_000,
                    vec![10_000, 12_000, 14_000],
                    Bps(0),
                    hops,
                )
                .is_none()
        );
    }

    #[test]
    fn graph_2hop_detection() {
        let base = MintId::new([1; 32]);
        let _token_b = MintId::new([2; 32]);

        let p1 = make_pool(10, 1, 2, 1_000_000, 1_000_000);
        let p2 = make_pool(20, 2, 1, 1_000_000, 1_250_000);

        let mut graph = MarketGraph::new(vec![base]).with_economic_config(test_config());
        graph.build_from_pools(&[p1, p2]);

        let opps = graph.find_base_cycles(Slot(1), 100_000);
        assert!(!opps.is_empty());
        assert!(opps[0].gross_profit.as_u64() > 0);
        assert_eq!(opps[0].hops.len(), 2);
        assert!(opps[0].size_in.as_u64() <= 100_000);
        assert_eq!(opps[0].hop_inputs.len(), opps[0].hops.len() + 1);
        assert_eq!(
            opps[0].hop_inputs.last().copied(),
            Some(opps[0].expected_out.as_u64())
        );
    }

    #[test]
    fn canonical_route_scanner_reports_candidate_budget_exhaustion() {
        let base = MintId::new([41; 32]);
        let p1 = make_pool(41, 41, 42, 1_000_000, 1_000_000);
        let p2 = make_pool(42, 41, 42, 1_000_000, 1_200_000);
        let p3 = make_pool(43, 41, 42, 1_000_000, 1_150_000);
        let mut graph = MarketGraph::new(vec![base]).with_economic_config(test_config());
        graph.build_from_pools(&[p1, p2, p3]);

        let (_opportunities, stats) = graph.find_cycles_with_stats(Slot(1), 100_000, 4, 1);
        assert_eq!(stats.cycles_evaluated, 1);
        assert!(stats.truncated, "budget exhaustion must be observable");
    }

    #[test]
    fn graph_4hop_multi_detection() {
        let base = MintId::new([1; 32]);

        let p1 = make_pool(1, 1, 2, 10_000_000, 10_000_000);
        let p2 = make_pool(2, 2, 3, 10_000_000, 10_000_000);
        let p3 = make_pool(3, 3, 4, 10_000_000, 10_000_000);
        let p4 = make_pool(4, 4, 1, 10_000_000, 11_200_000);

        let mut graph = MarketGraph::new(vec![base]).with_economic_config(test_config());
        graph.build_from_pools(&[p1, p2, p3, p4]);

        let opps = graph.find_multi_hop_cycles(Slot(1), 100_000, 4);
        assert!(!opps.is_empty(), "Must detect 4-hop profitable cycle");
        let four_hop = opps.iter().find(|o| o.hops.len() == 4);
        assert!(four_hop.is_some(), "Found 4-hop opportunity");
        let four_hop = four_hop.unwrap();
        assert!(four_hop.gross_profit.as_u64() > 0);
        assert!(four_hop.size_in.as_u64() <= 100_000);
        assert_eq!(four_hop.hop_inputs.len(), four_hop.hops.len() + 1);
        assert_eq!(
            four_hop.hop_inputs.last().copied(),
            Some(four_hop.expected_out.as_u64())
        );
    }

    #[test]
    fn test_chronos_opportunity_scoring() {
        let base = MintId::new([1; 32]);
        let p1 = make_pool(10, 1, 2, 1_000_000, 1_000_000);
        let p2 = make_pool(20, 2, 1, 1_000_000, 1_250_000);

        let mut graph = MarketGraph::new(vec![base]).with_economic_config(test_config());
        graph.build_from_pools(&[p1, p2]);

        let opps = graph.find_base_cycles(Slot(1), 100_000);
        assert!(!opps.is_empty());

        let fresh_score = score_opportunity_chronos(&opps[0], 5.0, 3.0, 0.0005);
        let stale_score = score_opportunity_chronos(&opps[0], 350.0, 3.0, 0.0005);

        assert!(
            fresh_score < stale_score,
            "Stale quote must receive a worse (higher) score than fresh quote"
        );
    }

    // ECON-002 regression: exact route quotes carry the compounded min-out haircut.
    #[test]
    fn opportunities_carry_slippage_haircut_min_final_output() {
        let base = MintId::new([1; 32]);
        let p1 = make_pool(10, 1, 2, 1_000_000, 1_000_000);
        let p2 = make_pool(20, 2, 1, 1_000_000, 1_250_000);

        let mut economic = test_config();
        economic.hop_slippage_bps = 50;
        let mut graph = MarketGraph::new(vec![base]).with_economic_config(economic);
        graph.build_from_pools(&[p1, p2]);

        let opps = graph.find_base_cycles(Slot(1), 100_000);
        assert!(!opps.is_empty());
        let opp = &opps[0];
        let expected = opp.expected_out.as_u64();
        let min_final = opp.expected_min_final_output;
        assert_eq!(opp.hop_slippage_bps, 50);
        assert!(min_final > 0, "haircut floor must be set");
        assert!(
            min_final < expected,
            "haircut floor must be below expected output"
        );
        assert_eq!(
            min_final,
            arb_math::economic::slippage_haircut_min_final(expected, opp.hops.len(), 50).unwrap(),
            "floor must equal the configured compounded haircut"
        );
    }
    #[test]
    fn test_min_net_profit_enforcement() {
        let base = MintId::new([1; 32]);
        // Pool 1 & Pool 2 with small spread.
        let p1 = make_pool(10, 1, 2, 1_000_000, 1_000_000);
        let p2 = make_pool(20, 2, 1, 1_000_000, 1_050_000);

        // Floor set above any achievable profit → all opportunities rejected.
        let config = EconomicConfig {
            flash_loan_fee_sf: 0,
            jito_tip_lamports: Lamports::ZERO,
            base_decimals: 9,
            lamports_to_base_mint_rate: 1,
            lamports_to_base_rate_denom: 1,
            min_profit_raw: TokenAmount(u64::MAX),
            hop_slippage_bps: arb_types::DEFAULT_HOP_SLIPPAGE_BPS,
        };
        let mut graph = MarketGraph::new(vec![base]).with_economic_config(config);
        graph.build_from_pools(&[p1, p2]);

        let opps = graph.find_base_cycles(Slot(1), 100_000);
        assert!(
            opps.is_empty(),
            "Profit below min profit threshold must be strictly rejected"
        );
    }

    #[test]
    fn test_pump_fun_raydium_2hop_cycle() {
        let base = MintId::new([1; 32]); // WSOL
        let _meme = MintId::new([2; 32]); // Meme Token

        // Pump.fun pool (Pool 1): 30 SOL virtual / 1,073,000,000 Meme Token virtual
        // Price: ~35.7M Token per SOL (tokens cheaper on bonding curve)
        let mut p1 = make_pool(10, 1, 2, 30_000_000_000, 1_073_000_000_000_000);
        p1.protocol = DexProtocol::PumpFun;
        p1.fee_numerator = 100;
        p1.fee_denominator = 10_000;
        p1.pump_fun = Some(arb_types::PumpFunState {
            virtual_token_reserves: 1_073_000_000_000_000,
            virtual_sol_reserves: 30_000_000_000,
            real_token_reserves: 793_100_000_000_000,
            real_sol_reserves: 5_000_000_000,
            token_total_supply: 1_000_000_000_000_000,
            complete: false,
            creator: Default::default(),
            is_mayhem_mode: false,
            is_cashback_coin: false,
            mode_flags_present: true,
        });

        // Raydium V4 pool (Pool 2): 35 SOL / 1,000,000,000 Meme Token (higher token valuation on Raydium)
        let mut p2 = make_pool(20, 1, 2, 35_000_000_000, 1_000_000_000_000_000);
        p2.protocol = DexProtocol::RaydiumV4;
        p2.fee_numerator = 25;
        p2.fee_denominator = 10_000;

        let mut graph = MarketGraph::new(vec![base]).with_economic_config(test_config());
        graph.build_from_pools(&[p1, p2]);

        let opps = graph.find_base_cycles(Slot(1), 500_000_000);
        assert!(
            !opps.is_empty(),
            "Must find 2-hop arbitrage cycle between Pump.fun and Raydium V4"
        );
        let opp = &opps[0];
        assert_eq!(opp.hops.len(), 2);
        assert!(opp.gross_profit.as_u64() > 0);
        assert!(opp.net_profit.as_u64() > 0);
        let protocols: Vec<DexProtocol> = opp.hops.iter().map(|h| h.protocol).collect();
        assert!(protocols.contains(&DexProtocol::PumpFun));
        assert!(protocols.contains(&DexProtocol::RaydiumV4));
    }
}

// Pump.fun-Raydium Arbitrage Engine
//core modules (arb/sizing/graph) are gated: they were merged unbuildable in
//PR #75 (56 compile errors, no callers) — repair behind the feature before
//promoting them; the on-chain profit guard compiles and stays public.
#[cfg(feature = "pump-raydium-legacy-engine")]
pub mod pump_raydium_arb;
#[cfg(feature = "pump-raydium-legacy-engine")]
pub use pump_raydium_arb::{
    ArbitrageDirection, PumpRaydiumConfig, PumpRaydiumEngine, PumpRaydiumOpportunity, RaydiumPool,
    RaydiumPoolType,
};
#[cfg(feature = "pump-raydium-legacy-engine")]
pub mod pump_raydium_sizing;
#[cfg(feature = "pump-raydium-legacy-engine")]
pub use pump_raydium_sizing::{
    SizingConfig, SizingResult, size_brents_method, size_cpmm_cpmm, size_golden_section,
    size_newton_raphson,
};
#[cfg(feature = "pump-raydium-legacy-engine")]
pub mod pump_raydium_graph;
#[cfg(feature = "pump-raydium-legacy-engine")]
pub use pump_raydium_graph::{
    ArbitrageCycle, BellmanFordResult, CycleType, GraphNode, Hop, PumpRaydiumGraph, SwapEdge,
};
pub mod pump_raydium_guard;
pub use pump_raydium_guard::{
    AssertMinimumProfit, ProfitGuardBuilder, ProfitGuardConfig, ProfitGuardError,
    ProfitGuardValidator,
};
