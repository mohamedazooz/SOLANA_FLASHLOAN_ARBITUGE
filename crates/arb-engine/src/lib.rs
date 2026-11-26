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
pub use route_sizing::{
    SearchProofStatus, SizeSearchResult, build_size_ladder, optimize_bounded_non_monotonic,
    optimize_bounded_non_monotonic_with_budget,
};
pub mod units;
pub use units::{net_lamports_equiv, wsol_mint_pubkey};

use arb_math::{
    ClmmTickMath, DlmmBin as ArbDlmmBin, DlmmPoolState, TickArrayFacade, TickFacade, cpmm_out,
    cpmm_price_impact_bps,
    economic::{
        EconomicConfig, base_raw_to_lamports, build_economic_quote, calculate_flash_fee_raw,
        slippage_haircut_min_final,
    },
    quote_clmm_with_ticks_per_array_and_math,
};
use arb_types::{
    Bps, DexProtocol, ExecutionOpportunity, Lamports, MintId, PoolExecutionAccounts, PoolId,
    PoolKind, PoolState, RouteId, Slot, SwapHop, TokenAmount, TokenProgramKind,
};
use std::collections::{HashMap, VecDeque};

/// Strictly-positive-gross epsilon for the coarse cross-pool scan. Integer math
/// has no roundoff, so 1 bps only filters dust (`gross * 10_000 / size < 1`);
/// acceptance is decided downstream by exact net-profit economics, never here.
pub const EPSILON_CYCLE_SPREAD_BPS: u64 = 1;
/// Hard per-slot candidate ceiling for the single production route scanner.
pub const MAX_ROUTE_CANDIDATES_PER_TICK: usize = 4096;
/// Hard raw-quote budget per optimized route; sized for the 2-vCPU deployment.
pub const ROUTE_SIZE_SEARCH_BUDGET: usize = 128;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolQuote {
    pub amount_out: u128,
    pub price_impact_bps: Bps,
    /// DLMM discrete boundary count; None for continuous-curve venues.
    pub discrete_bins_crossed: Option<u32>,
    /// CLMM tick-step count from the protocol-native quote; None if unavailable.
    pub clmm_steps_taken: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RouteQuoteLadder {
    amounts: Vec<u64>,
    /// Compounded, fee-excluded impact across every hop in the route.
    route_price_impact_bps: Bps,
    /// Total DLMM bin traversals observed across the route quote.
    discrete_bins_crossed: u32,
    /// Total CLMM tick steps observed across the route quote.
    clmm_steps_taken: u32,
}

/// Compose per-hop curve-impact ratios. Per-hop impact is rounded up and the
/// retained spot ratio is rounded down, so risk is never understated by bps
/// truncation. All arithmetic remains integer-only.
fn compose_route_price_impact(retained_spot_bps: u128, hop_impact: Bps) -> u128 {
    let hop_retained_bps = 10_000u128.saturating_sub(u128::from(hop_impact.0));
    let numerator = retained_spot_bps.saturating_mul(hop_retained_bps);
    (numerator / 10_000).min(10_000)
}

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

/// Checked exact-in quote for one pool hop.
///
/// Returns None when the pool cannot be priced honestly. Concentrated and
/// discrete-liquidity venues never fall back to constant-product math.
pub fn quote_pool_detailed(
    kind: PoolKind,
    dx: u128,
    x: u128,
    y: u128,
    gamma: u128,
    edge: Option<&MarketEdge>,
) -> Option<PoolQuote> {
    let quote = quote_pool_with_dlmm_cache(kind, dx, x, y, gamma, edge, None)?;
    (quote.amount_out > 0).then_some(quote)
}

/// Backwards-compatible amount-only API. New route discovery code must prefer
/// quote_pool_detailed so a rejected/invalid quote cannot be confused with dust.
pub fn quote_pool(
    kind: PoolKind,
    dx: u128,
    x: u128,
    y: u128,
    gamma: u128,
    edge: Option<&MarketEdge>,
) -> u128 {
    quote_pool_detailed(kind, dx, x, y, gamma, edge)
        .map(|quote| quote.amount_out)
        .unwrap_or(0)
}

/// The only route-sizing quote interface. Each executable protocol keeps its
/// own mathematical adapter; the graph only carries output and curve impact.
fn quote_pool_with_dlmm_cache(
    kind: PoolKind,
    dx: u128,
    x: u128,
    y: u128,
    gamma: u128,
    edge: Option<&MarketEdge>,
    dlmm_cache: Option<&DlmmQuoteCache>,
) -> Option<PoolQuote> {
    match kind {
        PoolKind::Cpmm => {
            if let Some(edge) = edge {
                let amount_in = u64::try_from(dx).ok()?;
                let pool = edge.to_pool_state();
                let a_to_b = edge.pool_base == edge.from_mint;
                let quote = crate::adapters::quote_via_dispatcher(&pool, amount_in, a_to_b).ok()?;
                Some(PoolQuote {
                    amount_out: u128::from(quote.amount_out_raw),
                    price_impact_bps: quote.price_impact,
                    discrete_bins_crossed: None,
                    clmm_steps_taken: quote.clmm_steps_taken,
                })
            } else {
                Some(PoolQuote {
                    amount_out: cpmm_out(dx, x, y, gamma)?,
                    price_impact_bps: cpmm_price_impact_bps(dx, x, y, 10_000),
                    discrete_bins_crossed: None,
                    clmm_steps_taken: None,
                })
            }
        }
        PoolKind::Clmm => {
            let edge = edge?;
            let amount_in = u64::try_from(dx).ok()?;
            let a_to_b = edge.pool_base == edge.from_mint;

            // Raydium CLMM remains quote-only discovery: its protocol-native
            // 60*tick_spacing arrays are not Whirlpool's 88-tick arrays and
            // its live execution capability remains disabled. Preserve the
            // verified quote for cycle discovery, but use an unknown-impact
            // sentinel so the candidate cannot pass the production risk gate.
            if edge.protocol == DexProtocol::RaydiumClmm {
                let state = edge.clmm.as_ref()?;
                if state.liquidity == 0
                    || state.sqrt_price == 0
                    || state.tick_arrays.is_empty()
                    || gamma > 10_000
                {
                    return None;
                }
                let fee_rate =
                    u16::try_from(10_000u128.checked_sub(gamma)?.checked_mul(100)?).ok()?;
                if fee_rate == 0 {
                    return None;
                }
                let arrays: Vec<TickArrayFacade> = state
                    .tick_arrays
                    .iter()
                    .map(|array| TickArrayFacade {
                        start_tick_index: array.start_tick_index,
                        ticks: array
                            .ticks
                            .iter()
                            .map(|tick| TickFacade {
                                tick_index: tick.tick_index,
                                initialized: tick.initialized,
                                liquidity_net: tick.liquidity_net,
                                liquidity_gross: 0,
                                limit_order_unfilled: tick.limit_order_unfilled,
                            })
                            .collect(),
                    })
                    .collect();
                let amount_out = quote_clmm_with_ticks_per_array_and_math(
                    amount_in,
                    state.sqrt_price,
                    state.tick_current,
                    state.liquidity,
                    state.tick_spacing,
                    fee_rate,
                    a_to_b,
                    &arrays,
                    arb_math::clmm::RAYDIUM_CLMM_TICK_ARRAY_SIZE,
                    ClmmTickMath::Raydium,
                )
                .ok()?;
                return Some(PoolQuote {
                    amount_out: u128::from(amount_out),
                    price_impact_bps: Bps(10_000),
                    discrete_bins_crossed: None,
                    clmm_steps_taken: None,
                });
            }

            let pool = edge.to_pool_state();
            let quote = crate::adapters::quote_via_dispatcher(&pool, amount_in, a_to_b).ok()?;
            Some(PoolQuote {
                amount_out: u128::from(quote.amount_out_raw),
                price_impact_bps: quote.price_impact,
                discrete_bins_crossed: None,
                clmm_steps_taken: quote.clmm_steps_taken,
            })
        }
        PoolKind::Dlmm => {
            let edge = edge?;
            let amount = u64::try_from(dx).ok()?;
            let a_to_b = edge.pool_base == edge.from_mint;
            let quote = match dlmm_cache {
                Some(cache) => {
                    let (pool, bins) = cache.get(&edge.pool_id)?.as_ref()?;
                    MeteoraDlmmQuoteEngine::quote_exact_in_from_view(pool, bins, amount, a_to_b)
                        .ok()?
                }
                None => {
                    let (pool, bins) = dlmm_quote_components(edge)?;
                    MeteoraDlmmQuoteEngine::quote_exact_in_from_view(&pool, &bins, amount, a_to_b)
                        .ok()?
                }
            };
            Some(PoolQuote {
                amount_out: u128::from(quote.amount_out),
                price_impact_bps: quote.price_impact_bps,
                discrete_bins_crossed: Some(quote.bins_crossed),
                clmm_steps_taken: None,
            })
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
    pub damm_v2: Option<arb_types::DammV2State>,
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
            // DAMM v2 needs the exact same-context Clock slot after graph projection.
            slot: Slot(self.damm_v2.as_ref().map_or(0, |damm| damm.clock_slot)),
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
            damm_v2: self.damm_v2.clone(),
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
    /// Dynamic CU-price policy shared with the final supervisor profitability gate.
    pub priority_fee_policy: Option<PriorityFeePolicy>,
}

/// Bounded recent CU samples for one exact route/amount shape.
///
/// We retain the maximum of the last eight successful simulations, not the
/// latest or average, to avoid lowering a CU limit after a low-usage outlier.
/// The sample is usable only for the next eight slots; simulation remains the
/// authoritative final gate.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CuMeasurement {
    /// Successful simulation samples retain their own slot so each expires
    /// independently; late completions cannot make old evidence appear fresh.
    pub samples: VecDeque<(Slot, u32)>,
    /// Monotonic high-water slot used for bounded-cache eviction.
    pub last_slot: Slot,
}

impl CuMeasurement {
    pub const MAX_SAMPLES: usize = 8;
    pub const MAX_AGE_SLOTS: u64 = 8;

    pub fn record(&mut self, units_consumed: u32, slot: Slot) {
        if units_consumed == 0 || slot.0 < self.last_slot.0 {
            return;
        }

        self.samples
            .retain(|(sample_slot, _)| slot.0.saturating_sub(sample_slot.0) <= Self::MAX_AGE_SLOTS);
        if self.samples.len() == Self::MAX_SAMPLES {
            self.samples.pop_front();
        }
        self.samples.push_back((slot, units_consumed));
        self.last_slot = slot;
    }

    #[must_use]
    pub fn conservative_units(&self, current_slot: Slot) -> Option<u32> {
        if current_slot.0 < self.last_slot.0
            || current_slot.0.saturating_sub(self.last_slot.0) > Self::MAX_AGE_SLOTS
        {
            return None;
        }

        self.samples
            .iter()
            .filter(|(sample_slot, _)| {
                current_slot.0 >= sample_slot.0
                    && current_slot.0.saturating_sub(sample_slot.0) <= Self::MAX_AGE_SLOTS
            })
            .map(|(_, units)| *units)
            .max()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriorityFeePolicy {
    pub desired_cu_price_micro_lamports: u64,
    pub max_share_bps: u64,
    pub has_flash_loan: bool,
    /// Recent measurements keyed by the exact route/amount/impact shape key.
    pub measured_route_cu: HashMap<String, CuMeasurement>,
}

impl PriorityFeePolicy {
    /// Use the exact shape-aware fallback estimate unless a fresh measured
    /// sample exists for this same quote shape. Priority-fee cap and ceiling
    /// rounding are shared with transaction construction.
    pub fn estimate_lamports(
        &self,
        gross_profit_lamports: u64,
        estimated_cu_units: u32,
        route_signature: &str,
        current_slot: Slot,
    ) -> u64 {
        let cu_limit = self
            .measured_route_cu
            .get(route_signature)
            .and_then(|measurement| measurement.conservative_units(current_slot))
            .map(|measured| arb_tx::calculate_dynamic_cu_limit_with_buffer(measured, 1_000))
            .unwrap_or_else(|| arb_tx::calculate_dynamic_cu_limit(estimated_cu_units));
        let cu_price = arb_tx::cap_cu_price_by_profit(
            gross_profit_lamports,
            cu_limit,
            self.desired_cu_price_micro_lamports,
            self.max_share_bps,
        );
        arb_tx::priority_fee_lamports(cu_price, cu_limit)
    }
}

/// Stable ordered route key shared by graph sizing and diagnostics. This key
/// intentionally identifies topology only; it must not be used to reuse CU.
pub fn route_compute_signature(hops: &[SwapHop]) -> String {
    format!(
        "cu-route:{}",
        hops.iter()
            .map(|hop| format!("{}-{:?}", hop.pool_id.0, hop.protocol))
            .collect::<Vec<_>>()
            .join("_")
    )
}

/// Deterministic CU-cache key for the exact quote shape. It includes ordered
/// pool/program/mint topology, every raw hop amount, and the quote's impact.
/// This avoids reusing a measurement from a route that crosses different
/// discrete bins/tick regions or uses a different ATA/token-program layout.
pub fn route_compute_shape_signature(
    hops: &[SwapHop],
    size_in: u64,
    hop_amounts: &[u64],
    price_impact_bps: Bps,
) -> String {
    route_compute_shape_signature_with_traversal(hops, size_in, hop_amounts, price_impact_bps, 0, 0)
}

/// Deterministic CU cache key including exact quote-time DLMM/CLMM traversal.
pub fn route_compute_shape_signature_with_traversal(
    hops: &[SwapHop],
    size_in: u64,
    hop_amounts: &[u64],
    price_impact_bps: Bps,
    dlmm_bins_crossed: u32,
    clmm_steps_taken: u32,
) -> String {
    fn absorb(hash: &mut u64, bytes: &[u8]) {
        for byte in bytes {
            *hash ^= u64::from(*byte);
            *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    absorb(&mut hash, b"solana-arb-cu-shape-v2");
    absorb(&mut hash, &size_in.to_le_bytes());
    absorb(&mut hash, &price_impact_bps.0.to_le_bytes());
    absorb(&mut hash, &dlmm_bins_crossed.to_le_bytes());
    absorb(&mut hash, &clmm_steps_taken.to_le_bytes());
    absorb(&mut hash, &(hops.len() as u64).to_le_bytes());
    for hop in hops {
        absorb(&mut hash, hop.pool_id.0.as_ref());
        absorb(&mut hash, hop.mint_in.0.as_ref());
        absorb(&mut hash, hop.mint_out.0.as_ref());
        absorb(&mut hash, &hop.fee_bps.to_le_bytes());
        absorb(
            &mut hash,
            &hop.active_bin_id.unwrap_or(i32::MIN).to_le_bytes(),
        );
        absorb(&mut hash, format!("{:?}", hop.protocol).as_bytes());
        absorb(&mut hash, format!("{:?}", hop.trade_intent).as_bytes());
        for token_program in hop.token_programs {
            let program_id = token_program.program_id();
            absorb(&mut hash, program_id.as_ref());
        }
        if let Some(accounts) = &hop.execution_accounts {
            absorb(&mut hash, format!("{accounts:?}").as_bytes());
        } else {
            absorb(&mut hash, b"no-execution-account-snapshot");
        }
    }
    absorb(&mut hash, &(hop_amounts.len() as u64).to_le_bytes());
    for amount in hop_amounts {
        absorb(&mut hash, &amount.to_le_bytes());
    }
    format!("cu:v2:{hash:016x}")
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
    /// Quote-only protocols intentionally kept out of executable cycle discovery.
    pub excluded_quote_only: usize,
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
            priority_fee_policy: None,
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

    pub fn set_priority_fee_policy(&mut self, policy: PriorityFeePolicy) {
        self.priority_fee_policy = Some(policy);
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
        discrete_bins_crossed: u32,
        clmm_steps_taken: u32,
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

        let gross_profit_raw = final_out_u64.checked_sub(size_in)?;
        let gross_profit_lamports = base_raw_to_lamports(
            TokenAmount(gross_profit_raw),
            config.lamports_to_base_mint_rate,
            config.lamports_to_base_rate_denom,
        )
        .ok()?
        .0;
        let protocols: Vec<DexProtocol> = hops.iter().map(|hop| hop.protocol).collect();
        let ata_counts = arb_tx::route_ata_instruction_counts(&hops).ok()?;
        let has_flash_loan = self
            .priority_fee_policy
            .as_ref()
            .map(|policy| policy.has_flash_loan)
            .unwrap_or(true);
        let estimated_cu = arb_tx::estimate_route_cu_with_shape(
            has_flash_loan,
            &protocols,
            ata_counts,
            discrete_bins_crossed,
            clmm_steps_taken,
            0,
        );
        let route_signature = route_compute_shape_signature_with_traversal(
            &hops,
            size_in,
            &hop_inputs,
            impact,
            discrete_bins_crossed,
            clmm_steps_taken,
        );
        let priority_fee_lamports = self
            .priority_fee_policy
            .as_ref()
            .map(|policy| {
                policy.estimate_lamports(
                    gross_profit_lamports,
                    estimated_cu,
                    &route_signature,
                    current_slot,
                )
            })
            .unwrap_or(0);

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
            Lamports(priority_fee_lamports),
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
            cu_estimate_units: estimated_cu,
            cu_shape_key: route_signature,
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

        // Capacity estimates are conservative on pool-side amounts and may
        // omit input-side swap fees. If the upper endpoint is not exactly
        // executable, locate a valid point with bounded geometric probes,
        // then binary-search the highest fully quoted route input.
        if Self::quote_route_input_ladder(edges, upper, &capacities, Some(&dlmm_cache)).is_none() {
            let mut probe = 1u64;
            let mut first_feasible = None;
            loop {
                if Self::quote_route_input_ladder(edges, probe, &capacities, Some(&dlmm_cache))
                    .is_some()
                {
                    first_feasible = Some(probe);
                    break;
                }
                if probe >= upper {
                    break;
                }
                probe = probe.saturating_mul(2).min(upper);
            }
            let mut low = first_feasible?;
            let mut high = upper;
            while low < high {
                let mid = low + (high - low).div_ceil(2);
                if Self::quote_route_input_ladder(edges, mid, &capacities, Some(&dlmm_cache))
                    .is_some()
                {
                    low = mid;
                } else {
                    high = mid - 1;
                }
            }
            upper = low;
        }

        // Map exact DLMM bin-drain inputs back to the route's initial input.
        // At most two breakpoints per DLMM hop are sampled, then neighbors are
        // added so integer rounding on either side of a discontinuity is tested.
        // Every global seed is re-quoted through the full route by the optimizer.
        for (hop_index, edge) in edges
            .iter()
            .enumerate()
            .filter(|(_, edge)| edge.kind == PoolKind::Dlmm)
        {
            let Some(Some((pool, bins))) = dlmm_cache.get(&edge.pool_id) else {
                continue;
            };
            let Ok(breakpoints) = arb_math::dlmm::dlmm_drain_breakpoints(
                pool,
                bins,
                edge.pool_base == edge.from_mint,
            ) else {
                continue;
            };
            let Some(local_ceiling) =
                Self::amount_before_hop(edges, upper, hop_index, Some(&dlmm_cache))
                    .and_then(|amount| u64::try_from(amount).ok())
            else {
                continue;
            };
            let local_breakpoints: Vec<u64> = breakpoints
                .iter()
                .map(|point| point.cumulative_input)
                .collect();
            let sampled_local =
                arb_math::dlmm::dlmm_breakpoint_seed_sizes(&local_breakpoints, 1, local_ceiling, 2);
            for local_input in sampled_local {
                if let Some(route_input) = Self::route_input_for_hop_input_threshold(
                    edges,
                    upper,
                    hop_index,
                    local_input,
                    &dlmm_cache,
                ) {
                    boundary_seeds.extend([
                        route_input.saturating_sub(1),
                        route_input,
                        route_input.saturating_add(1).min(upper),
                    ]);
                }
            }
        }
        boundary_seeds = arb_math::dlmm::dlmm_breakpoint_seed_sizes(&boundary_seeds, 1, upper, 48);
        let best_gross_sample = std::cell::RefCell::new(None::<(u64, u64, Vec<u64>)>);
        let result = route_sizing::optimize_bounded_non_monotonic_with_budget(
            upper,
            32,
            4,
            8,
            &boundary_seeds,
            ROUTE_SIZE_SEARCH_BUDGET,
            |size_in| {
                let route_quote =
                    Self::quote_route_input_ladder(edges, size_in, &capacities, Some(&dlmm_cache))?;
                tracing::trace!(
                    size_in,
                    bins_crossed = route_quote.discrete_bins_crossed,
                    clmm_steps = route_quote.clmm_steps_taken,
                    "route quote traversal shape"
                );
                let final_out = u128::from(*route_quote.amounts.last()?);
                let discrete_bins_crossed = route_quote.discrete_bins_crossed;
                let clmm_steps_taken = route_quote.clmm_steps_taken;
                let hop_inputs = route_quote.amounts;
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

                let impact = route_quote.route_price_impact_bps;
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
                    discrete_bins_crossed,
                    clmm_steps_taken,
                    hops,
                )?;
                Some((opportunity.clone(), i128::from(opportunity.net_profit.0)))
            },
        );
        if let Some(search) = result.as_ref() {
            tracing::debug!(
                evaluated = search.evaluated_count,
                budget = search.evaluation_budget,
                complete = search.search_complete,
                proof = ?search.proof_status,
                "route size search finished (sampled search is not a global-optimum proof unless exhaustive)"
            );
        }
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

    fn route_input_for_hop_input_threshold(
        edges: &[&MarketEdge],
        upper: u64,
        hop_index: usize,
        target_hop_input: u64,
        dlmm_cache: &DlmmQuoteCache,
    ) -> Option<u64> {
        if target_hop_input == 0 || upper == 0 {
            return None;
        }
        let reaches_threshold = |route_input: u64| {
            Self::amount_before_hop(edges, route_input, hop_index, Some(dlmm_cache))
                .is_some_and(|amount| amount >= u128::from(target_hop_input))
        };
        if !reaches_threshold(upper) {
            return None;
        }

        // Upstream exact-in quotes are monotone in input for the immutable
        // route snapshot. Find the first route input that reaches this hop's
        // exact DLMM drain boundary, then test +/- one raw unit around it.
        let mut low = 1u64;
        let mut high = upper;
        while low < high {
            let mid = low + (high - low) / 2;
            if reaches_threshold(mid) {
                high = mid;
            } else {
                low = mid + 1;
            }
        }
        Some(low)
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
            )?
            .amount_out;
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
    ) -> Option<RouteQuoteLadder> {
        let mut amount = u128::from(size_in);
        let mut ladder = Vec::with_capacity(edges.len() + 1);
        let mut retained_spot_bps = 10_000u128;
        let mut discrete_bins_crossed = 0u32;
        let mut clmm_steps_taken = 0u32;
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
            let quote = quote_pool_with_dlmm_cache(
                edge.kind,
                amount,
                edge.reserve_in,
                edge.reserve_out,
                gamma,
                Some(edge),
                dlmm_cache,
            )?;
            if quote.amount_out == 0 {
                return None;
            }
            retained_spot_bps =
                compose_route_price_impact(retained_spot_bps, quote.price_impact_bps);
            discrete_bins_crossed =
                discrete_bins_crossed.saturating_add(quote.discrete_bins_crossed.unwrap_or(0));
            clmm_steps_taken = clmm_steps_taken.saturating_add(quote.clmm_steps_taken.unwrap_or(0));
            amount = quote.amount_out;
            ladder.push(u64::try_from(amount).ok()?);
        }
        Some(RouteQuoteLadder {
            amounts: ladder,
            route_price_impact_bps: Bps(u16::try_from(
                10_000u128.saturating_sub(retained_spot_bps),
            )
            .unwrap_or(10_000)),
            discrete_bins_crossed,
            clmm_steps_taken,
        })
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
            if p.protocol == DexProtocol::MeteoraDammV2 {
                // It can be priced by the specialized dispatcher, but exact deployed swap
                // account/min-out/rounding parity is not yet proven. Never issue an
                // ExecutionOpportunity for this quote-only venue.
                stats.excluded_quote_only += 1;
                continue;
            }
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
                    damm_v2: p.damm_v2.clone(),
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
                    damm_v2: p.damm_v2.clone(),
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
    #[test]
    fn route_price_impact_compounds_each_hop_conservatively() {
        let retained = compose_route_price_impact(10_000, Bps(30));
        let retained = compose_route_price_impact(retained, Bps(30));
        assert_eq!(10_000u128 - retained, 60);

        let retained = compose_route_price_impact(10_000, Bps(10_000));
        assert_eq!(retained, 0);
        let retained = compose_route_price_impact(10_000, Bps(0));
        assert_eq!(retained, 10_000);
    }

    #[test]
    fn detailed_quote_reports_curve_impact_and_rejects_invalid_output() {
        let quote = quote_pool_detailed(PoolKind::Cpmm, 1_000, 100_000, 100_000, 9_900, None)
            .expect("valid CPMM quote");
        assert!(quote.amount_out > 0);
        assert!(quote.price_impact_bps.0 > 0);
        assert!(quote.discrete_bins_crossed.is_none());

        assert!(quote_pool_detailed(PoolKind::Cpmm, 0, 100_000, 100_000, 9_900, None).is_none());
    }

    #[test]
    fn measured_cu_uses_recent_maximum_and_expires_by_slot() {
        let mut measurement = CuMeasurement::default();
        for (slot, units) in (100u64..110).zip([400, 410, 405, 420, 415, 430, 425, 440, 435, 450]) {
            measurement.record(units, Slot(slot));
        }
        assert_eq!(measurement.samples.len(), CuMeasurement::MAX_SAMPLES);
        assert_eq!(measurement.conservative_units(Slot(109)), Some(450));
        assert_eq!(measurement.conservative_units(Slot(117)), Some(450));
        assert_eq!(measurement.conservative_units(Slot(118)), None);
        assert_eq!(measurement.conservative_units(Slot(108)), None);

        // A delayed result from an older slot must not roll the cache clock
        // backwards or resurrect an expired high-CU observation.
        measurement.record(900, Slot(100));
        assert_eq!(measurement.last_slot, Slot(109));
        assert_eq!(measurement.conservative_units(Slot(117)), Some(450));
    }

    #[test]
    fn compute_shape_signature_changes_with_raw_amount_and_impact() {
        let hop = SwapHop {
            pool_id: PoolId::new([1; 32]),
            protocol: DexProtocol::MeteoraDlmm,
            mint_in: MintId::new([2; 32]),
            mint_out: MintId::new([3; 32]),
            fee_bps: 25,
            active_bin_id: Some(7),
            token_programs: [TokenProgramKind::SplToken; 2],
            trade_intent: Default::default(),
            execution_accounts: None,
        };
        let base = route_compute_shape_signature(
            std::slice::from_ref(&hop),
            1_000,
            &[1_000, 990],
            Bps(20),
        );
        assert_eq!(
            base,
            route_compute_shape_signature(
                std::slice::from_ref(&hop),
                1_000,
                &[1_000, 990],
                Bps(20)
            )
        );
        assert_ne!(
            base,
            route_compute_shape_signature(
                std::slice::from_ref(&hop),
                1_001,
                &[1_001, 991],
                Bps(20)
            )
        );
        assert_ne!(
            base,
            route_compute_shape_signature(
                std::slice::from_ref(&hop),
                1_000,
                &[1_000, 990],
                Bps(21)
            )
        );
        assert_ne!(
            base,
            route_compute_shape_signature_with_traversal(
                std::slice::from_ref(&hop),
                1_000,
                &[1_000, 990],
                Bps(20),
                2,
                0,
            )
        );
        assert_ne!(
            base,
            route_compute_shape_signature_with_traversal(
                &[hop],
                1_000,
                &[1_000, 990],
                Bps(20),
                0,
                1,
            )
        );
    }

    #[test]
    fn priority_fee_policy_uses_route_compute_estimate_and_profit_cap() {
        let policy = PriorityFeePolicy {
            desired_cu_price_micro_lamports: 10_000_000,
            max_share_bps: 500,
            has_flash_loan: true,
            measured_route_cu: HashMap::new(),
        };
        let fee = policy.estimate_lamports(1_000_000, 350_000, "unmeasured-route", Slot(100));
        assert!(fee > 0);
        assert!(
            fee <= 50_000,
            "priority fee exceeded its 5% profit share: {fee}"
        );
        assert_eq!(
            policy.estimate_lamports(0, 350_000, "unmeasured-route", Slot(100)),
            0
        );
    }

    #[test]
    fn priority_fee_policy_reuses_measured_route_compute_limit() {
        let route = "cu:pool-a-OrcaWhirlpool_pool-b-MeteoraDlmm";
        let mut measured_route_cu = HashMap::new();
        let mut measurement = CuMeasurement::default();
        measurement.record(510_000, Slot(100));
        measured_route_cu.insert(route.to_string(), measurement);
        let policy = PriorityFeePolicy {
            desired_cu_price_micro_lamports: 10_000,
            max_share_bps: 500,
            has_flash_loan: true,
            measured_route_cu,
        };
        let gross_profit = 2_000_000u64;
        let cu_limit = arb_tx::calculate_dynamic_cu_limit_with_buffer(510_000, 1_000);
        let cu_price = arb_tx::cap_cu_price_by_profit(
            gross_profit,
            cu_limit,
            policy.desired_cu_price_micro_lamports,
            policy.max_share_bps,
        );
        let expected = arb_tx::priority_fee_lamports(cu_price, cu_limit);

        assert_eq!(
            policy.estimate_lamports(gross_profit, 700_000, route, Slot(101)),
            expected,
        );
        assert_ne!(
            policy.estimate_lamports(gross_profit, 700_000, "different-route", Slot(101)),
            expected,
            "different routes must not reuse this route's measured CU"
        );
    }

    #[test]
    fn priority_fee_policy_zero_share_never_bids() {
        let policy = PriorityFeePolicy {
            desired_cu_price_micro_lamports: u64::MAX,
            max_share_bps: 0,
            has_flash_loan: true,
            measured_route_cu: HashMap::new(),
        };
        assert_eq!(
            policy.estimate_lamports(1_000_000, 250_000, "unmeasured-route", Slot(100),),
            0,
        );
    }

    use super::*;

    fn make_damm_pool_state(collect_fee_mode: u8) -> PoolState {
        let pool_id = PoolId::new([70; 32]);
        let mint_a = MintId::new([71; 32]);
        let mint_b = MintId::new([72; 32]);
        let authority = solana_pubkey::Pubkey::new_from_array([73; 32]);
        let vault_a = solana_pubkey::Pubkey::new_from_array([74; 32]);
        let vault_b = solana_pubkey::Pubkey::new_from_array([75; 32]);
        let event_authority = solana_pubkey::Pubkey::new_from_array([76; 32]);
        let compounding = collect_fee_mode == 2;
        let state = arb_types::DammV2State {
            pool_status: 0,
            fee_version: 0,
            layout_version: if compounding { 1 } else { 0 },
            collect_fee_mode,
            activation_type: 1,
            activation_point: 1_800_000_000,
            current_point: 1_800_000_100,
            current_timestamp: 1_800_000_100,
            clock_slot: 55,
            clock_epoch: 1097,
            sqrt_price_x64: 1u128 << 64,
            liquidity: 1_000_000_000_000,
            sqrt_min_price_x64: 1u128 << 63,
            sqrt_max_price_x64: 1u128 << 65,
            token_a_amount: if compounding { Some(10_000_000) } else { None },
            token_b_amount: if compounding { Some(20_000_000) } else { None },
            cliff_fee_numerator: 100_000,
            fee_scheduler_mode: 2,
            protocol_fee_percent: 10,
            referral_fee_percent: 25,
            compounding_fee_bps: 2000,
            account_graph_verified: true,
            ..Default::default()
        };
        PoolState {
            pool_id,
            protocol: DexProtocol::MeteoraDammV2,
            slot: Slot(55),
            write_version: 1,
            base_mint: mint_a,
            quote_mint: mint_b,
            decimals_a: 9,
            decimals_b: 6,
            reserve_in: if compounding {
                10_000_000
            } else {
                1_000_000_000_000
            },
            reserve_out: if compounding {
                20_000_000
            } else {
                1_000_000_000_000
            },
            fee_numerator: 100_000,
            fee_denominator: 1_000_000_000,
            raydium_cpmm_fee: None,
            is_active: true,
            has_risky_token: false,
            kind: if compounding {
                PoolKind::Cpmm
            } else {
                PoolKind::Clmm
            },
            clmm: None,
            dlmm: None,
            pump_fun: None,
            damm_v2: Some(state),
            transfer_fee_bps: 0,
            token_programs: [TokenProgramKind::SplToken; 2],
            execution_accounts: Some(PoolExecutionAccounts::MeteoraDammV2 {
                token_a_mint: mint_a.0,
                token_b_mint: mint_b.0,
                authority,
                vault_a,
                vault_b,
                event_authority,
            }),
            last_update_slot: 55,
            last_update_ms: 1,
        }
    }

    fn edge_from_damm_pool(pool: &PoolState, reverse: bool) -> MarketEdge {
        let (from_mint, to_mint, reserve_in, reserve_out) = if reverse {
            (
                pool.quote_mint,
                pool.base_mint,
                pool.reserve_out,
                pool.reserve_in,
            )
        } else {
            (
                pool.base_mint,
                pool.quote_mint,
                pool.reserve_in,
                pool.reserve_out,
            )
        };
        MarketEdge {
            pool_id: pool.pool_id,
            protocol: pool.protocol,
            kind: pool.kind,
            from_mint,
            to_mint,
            pool_base: pool.base_mint,
            pool_quote: pool.quote_mint,
            reserve_in,
            reserve_out,
            fee_bps: pool.fee_bps().unwrap(),
            fee_numerator: pool.fee_numerator,
            fee_denominator: pool.fee_denominator,
            raydium_cpmm_fee: None,
            clmm: None,
            dlmm: None,
            pump_fun: None,
            damm_v2: pool.damm_v2.clone(),
            transfer_fee_bps: 0,
            token_programs: [TokenProgramKind::SplToken; 2],
            execution_accounts: pool.execution_accounts.clone(),
            active_bin_id: None,
        }
    }

    #[test]
    fn market_edge_quote_uses_damm_v2_adapter_for_both_curve_types_and_directions() {
        for mode in [0u8, 2u8] {
            let state = make_damm_pool_state(mode);
            let kind = state.kind;
            for reverse in [false, true] {
                let edge = edge_from_damm_pool(&state, reverse);
                let output = quote_pool(
                    kind,
                    100_000,
                    edge.reserve_in,
                    edge.reserve_out,
                    9_970,
                    Some(&edge),
                );
                assert!(
                    output > 0,
                    "DAMM v2 mode={mode} reverse={reverse} should quote through the universal dispatcher"
                );
            }
        }
    }

    #[test]
    fn production_cycle_graph_keeps_damm_v2_quote_only_until_execution_parity() {
        let state = make_damm_pool_state(0);
        let mut graph = MarketGraph::new(vec![state.base_mint]);
        let stats = graph.build_from_pools(&[state]);
        assert_eq!(stats.pools_seen, 1);
        assert_eq!(stats.excluded_quote_only, 1);
        assert_eq!(stats.pools_indexed, 0);
        assert!(graph.edges_by_from.is_empty());
    }
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
        assert_eq!(stats.excluded_unsupported_protocol, protocols.len() - 1);
        assert_eq!(stats.excluded_quote_only, 1);
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
            damm_v2: None,
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
            damm_v2: None,
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
            damm_v2: None,
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
                    0,
                    0,
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
