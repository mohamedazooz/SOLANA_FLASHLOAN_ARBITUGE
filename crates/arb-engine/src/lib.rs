//!
//! Pure compute crate: Zero I/O, zero async networking.
//! Implements Base-Asset Anchoring (SOL / USDC) with epsilon threshold and optimal sizing.

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
    DlmmFlashArbRoute, DlmmHopQuote, DlmmPoolAllowlist, FlashArbRejection, enumerate_routes,
};
pub mod meteora_pair_discovery;
pub use meteora_pair_discovery::{
    MeteoraDlmmEligiblePair, MeteoraDlmmPairDiscoveryReport, MeteoraDlmmPoolPairDiscovery,
    PoolRejectionReason,
};
pub mod meteora_quote_engine;
pub use meteora_quote_engine::{MeteoraDlmmCrossPoolQuoteEngine, TwoHopDlmmQuote};
pub mod meteora_fresh_state;
pub mod meteora_size_optimizer;
pub mod three_leg_meme_pipeline;
pub mod flash_loan_sizing;
pub use flash_loan_sizing::{
    calculate_kamino_flash_fee, calculate_optimal_flash_loan, FlashLoanSizingConfig,
    FlashLoanSizingResult, KAMINO_FRACTION_FRAC_BITS, KAMINO_FRACTION_ONE_BITS,
};
pub use meteora_fresh_state::{FreshnessConfig, FreshnessVerdict, evaluate_freshness};
pub use meteora_size_optimizer::{
    DlmmArbitrageDirection, DlmmOptimizerConfig, MeteoraDlmmTradeSizeOptimizer,
    MeteoraOptimalSizeResult,
};
pub use three_leg_meme_pipeline::{
    ThreeLegMemeContextParams, ThreeLegMemeLegQuote, ThreeLegMemePipelineError,
    ThreeLegMemePipelineResult, ThreeLegMemeSizeSearchResult, build_three_leg_meme_pipeline,
    simulate_profitability_gate,
};

use arb_math::{
    DlmmBin as ArbDlmmBin, DlmmParameters, DlmmPoolState, DlmmVariableParameters, Leg,
    cpmm::clamp_at_marginal_ceiling_two_pool,
    cpmm_out, cpmm_price_impact_bps,
    economic::{
        EconomicConfig, base_raw_to_lamports, build_economic_quote, calculate_flash_fee_raw,
    },
    optimal_three_pool, optimal_two_pool, quote_clmm, quote_dlmm_view,
};
use arb_types::{
    Bps, DexProtocol, ExecutionOpportunity, Lamports, MintId, PoolExecutionAccounts, PoolId,
    PoolKind, PoolState, RouteId, Slot, SwapHop, TokenAmount, TokenProgramKind,
};
use std::collections::HashMap;

/// Minimum spread threshold to filter floating-point roundoff noise (10 bps = 0.1%).
pub const EPSILON_CYCLE_SPREAD_BPS: u64 = 10;
/// Minimum required net profit in lamports to justify execution and Jito tip.
pub const MIN_NET_PROFIT_LAMPORTS: u64 = 30_000;

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
    match kind {
        PoolKind::Cpmm => cpmm_out(dx, x, y, gamma).unwrap_or(0),
        PoolKind::Clmm => {
            if let Some(e) = edge {
                if let Some(c) = &e.clmm {
                    if c.liquidity > 0 && c.sqrt_price > 0 && !c.tick_arrays.is_empty() {
                        // Reject invalid fee metadata and values outside the quote
                        // engine's integer domain; never clamp into a different quote.
                        if gamma > 10_000 {
                            return 0;
                        }
                        let fee_rate_u128 = (10_000u128 - gamma) * 100;
                        let Ok(fee_rate) = u16::try_from(fee_rate_u128) else {
                            return 0;
                        };
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
                                    })
                                    .collect(),
                            })
                            .collect();
                        if let Ok(out) = quote_clmm(
                            amount,
                            c.sqrt_price,
                            c.tick_current,
                            c.liquidity,
                            c.tick_spacing,
                            fee_rate,
                            a_to_b,
                            &arrays,
                        ) {
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
                if let Some(d) = &e.dlmm {
                    if let Some(params) = &d.params {
                        if d.status != 0 || params.function_type != 0 || params.collect_fee_mode > 1
                        {
                            return 0;
                        }
                        if !d.bins.is_empty() {
                            let pool = DlmmPoolState {
                                active_id: d.active_bin_id,
                                bin_step: d.bin_step_bps,
                                status: d.status,
                                parameters: DlmmParameters {
                                    bin_step: d.bin_step_bps,
                                    base_factor: params.base_factor,
                                    filter_period: params.filter_period,
                                    decay_period: params.decay_period,
                                    reduction_factor: params.reduction_factor,
                                    variable_fee_control: params.variable_fee_control,
                                    max_volatility_accumulator: params.max_volatility_accumulator,
                                    protocol_share: params.protocol_share,
                                    base_fee_power_factor: params.base_fee_power_factor,
                                    collect_fee_mode: params.collect_fee_mode,
                                },
                                v_parameters: DlmmVariableParameters {
                                    volatility_accumulator: d.volatility_accumulator,
                                    volatility_reference: d.volatility_reference,
                                    index_reference: d.index_reference,
                                    last_update_timestamp: d.last_update_timestamp,
                                },
                            };
                            let bins: Option<Vec<ArbDlmmBin>> = d
                                .bins
                                .iter()
                                .map(|b| {
                                    Some(ArbDlmmBin {
                                        id: b.bin_id,
                                        amount_x: u64::try_from(b.amount_x).ok()?,
                                        amount_y: u64::try_from(b.amount_y).ok()?,
                                        price_q64: 0,
                                    })
                                })
                                .collect();
                            let Some(bins) = bins else {
                                return 0;
                            };
                            let Ok(amount) = u64::try_from(dx) else {
                                return 0;
                            };
                            let a_to_b = e.pool_base == e.from_mint;
                            if let Ok(q) = quote_dlmm_view(&pool, &bins, amount, a_to_b, None) {
                                return q.amount_out as u128;
                            }
                        }
                    }
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
        pools.push(PoolState {
            pool_id: e.pool_id,
            protocol: e.protocol,
            slot: Slot(0),
            write_version: 0,
            base_mint: e.pool_base,
            quote_mint: e.pool_quote,
            decimals_a: 0,
            decimals_b: 0,
            reserve_in: e.reserve_in,
            reserve_out: e.reserve_out,
            fee_numerator: e.fee_bps as u64,
            fee_denominator: 10_000,
            is_active: true,
            has_risky_token: false,
            kind: e.kind,
            clmm: e.clmm.clone(),
            dlmm: e.dlmm.clone(),
            pump_fun: e.pump_fun.clone(),
            transfer_fee_bps: e.transfer_fee_bps,
            token_programs: e.token_programs,
            execution_accounts: e.execution_accounts.clone(),
            last_update_slot: 0,
            last_update_ms: 0,
        });
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

/// Golden-section search for the input size maximizing profit on a (CPMM) cycle.
/// P1 #11: replaces the bisection that converged to the breakeven edge.
pub fn golden_section_size(path: &[&MarketEdge], lo: u64, hi: u64, iters: u32) -> u64 {
    if hi <= lo {
        return lo;
    }
    let phi = 0.6180339887498949f64;
    let mut a = lo as f64;
    let mut b = hi as f64;
    let mut c = b - (b - a) * phi;
    let mut d = a + (b - a) * phi;
    for _ in 0..iters {
        let fc = cycle_profit_at(path, c as u64) as f64;
        let fd = cycle_profit_at(path, d as u64) as f64;
        if fc < fd {
            a = c;
            c = d;
            d = a + (b - a) * phi;
        } else {
            b = d;
            d = c;
            c = b - (b - a) * phi;
        }
    }
    ((a + b) / 2.0) as u64
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

/// Directed multi-asset liquidity graph anchored to base currencies.
#[derive(Debug, Default, Clone)]
pub struct MarketGraph {
    pub edges_by_from: HashMap<MintId, Vec<MarketEdge>>,
    pub base_anchors: Vec<MintId>,
    /// Authoritative economic configuration. When `None` the graph cannot price
    /// opportunities and `find_base_cycles` returns empty (fail closed).
    pub economic_config: Option<EconomicConfig>,
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
        let quote_timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let quote = build_economic_quote(
            base_mint,
            config.base_decimals,
            TokenAmount(size_in),
            TokenAmount(final_out_u64),
            TokenAmount(final_out_u64),
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
        })
    }

    /// Rebuild graph from active pool states.
    pub fn build_from_pools(&mut self, pools: &[PoolState]) {
        self.edges_by_from.clear();

        for p in pools {
            if !p.is_active
                || p.reserve_in == 0
                || p.reserve_out == 0
                || p.has_risky_token
                || p.transfer_fee_bps != 0
                || !p.is_fee_valid()
            {
                continue;
            }
            let fee = ((p.fee_numerator as u128 * 10_000) / p.fee_denominator as u128) as u16;

            // Edge: base_mint -> quote_mint
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
                    clmm: p.clmm.clone(),
                    dlmm: p.dlmm.clone(),
                    pump_fun: p.pump_fun.clone(),
                    transfer_fee_bps: p.transfer_fee_bps,
                    token_programs: p.token_programs,
                    execution_accounts: p.execution_accounts.clone(),
                    active_bin_id: p.dlmm.as_ref().map(|d| d.active_bin_id),
                });

            // Edge: quote_mint -> base_mint
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
                    clmm: p.clmm.clone(),
                    dlmm: p.dlmm.clone(),
                    pump_fun: p.pump_fun.clone(),
                    transfer_fee_bps: p.transfer_fee_bps,
                    token_programs: [p.token_programs[1], p.token_programs[0]],
                    execution_accounts: p.execution_accounts.clone(),
                    active_bin_id: p.dlmm.as_ref().map(|d| d.active_bin_id),
                });
        }
    }

    /// Maximum input capacity for a market edge within its active tick/bin or real reserves.
    /// Supports Pump.fun, CLMM, and DLMM:
    /// - Pump.fun: limits input by real_token_reserves (Buy) or real_sol_reserves (Sell), returns 0 if complete.
    /// - CLMM: computes Delta X max (when a_to_b) or Delta Y max (when b_to_a).
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
                let c = edge.clmm.as_ref()?;
                let a_to_b = edge.pool_base == edge.from_mint;
                arb_math::clmm::clmm_active_tick_max_input(
                    c.sqrt_price,
                    c.tick_current,
                    c.tick_spacing,
                    c.liquidity,
                    a_to_b,
                )
            }
            PoolKind::Dlmm => {
                let d = edge.dlmm.as_ref()?;
                let active_bin = d.bins.iter().find(|b| b.bin_id == d.active_bin_id)?;
                let a_to_b = edge.pool_base == edge.from_mint;
                let ax = active_bin.amount_x.min(u64::MAX as u128) as u64;
                let ay = active_bin.amount_y.min(u64::MAX as u128) as u64;
                arb_math::dlmm::dlmm_active_bin_max_input(
                    d.active_bin_id,
                    d.bin_step_bps,
                    ax,
                    ay,
                    a_to_b,
                )
            }
            PoolKind::Cpmm => None,
        }
    }

    /// Detect profitable 2-hop (cross-DEX) and 3-hop (triangular) cycles anchored to base working assets.
    pub fn find_base_cycles(
        &self,
        current_slot: Slot,
        max_trade_size: u64,
    ) -> Vec<ExecutionOpportunity> {
        let mut opportunities = Vec::new();
        let mut route_counter = 1u32;

        for &base in &self.base_anchors {
            let Some(first_hops) = self.edges_by_from.get(&base) else {
                continue;
            };

            for e1 in first_hops {
                let inter1 = e1.to_mint;
                if inter1 == base {
                    continue;
                }

                let Some(second_hops) = self.edges_by_from.get(&inter1) else {
                    continue;
                };

                for e2 in second_hops {
                    // Case 1: 2-Hop Arbitrage (Base -> inter1 -> Base) across different pools
                    if e2.to_mint == base && e1.pool_id != e2.pool_id {
                        let l1 = Leg {
                            r_in: e1.reserve_in,
                            r_out: e1.reserve_out,
                            gamma: 10_000u128.saturating_sub(e1.fee_bps as u128),
                        };
                        let l2 = Leg {
                            r_in: e2.reserve_in,
                            r_out: e2.reserve_out,
                            gamma: 10_000u128.saturating_sub(e2.fee_bps as u128),
                        };

                        let non_cpmm_hops = (e1.kind != PoolKind::Cpmm) as usize
                            + (e2.kind != PoolKind::Cpmm) as usize;
                        let initial_size = optimal_two_pool(&l1, &l2).or(if non_cpmm_hops > 0 {
                            Some(100_000)
                        } else {
                            None
                        });

                        if let Some(opt_size) = initial_size {
                            let mut size_in = opt_size.min(max_trade_size);
                            // Two-sided CLMM & DLMM active tick/bin capacity clamping (Hop 1)
                            if let Some(c1) = Self::edge_max_input_capacity(e1) {
                                if c1 > 0 {
                                    size_in = size_in.min(c1);
                                }
                            }
                            if size_in > 0 {
                                let mut out1 = quote_pool(
                                    e1.kind,
                                    size_in as u128,
                                    l1.r_in,
                                    l1.r_out,
                                    l1.gamma,
                                    Some(e1),
                                );
                                // Two-sided CLMM & DLMM active tick/bin capacity clamping (Hop 2)
                                if let Some(c2) = Self::edge_max_input_capacity(e2) {
                                    if c2 > 0 && out1 > c2 as u128 {
                                        size_in = ((size_in as u128 * c2 as u128) / out1) as u64;
                                        out1 = quote_pool(
                                            e1.kind,
                                            size_in as u128,
                                            l1.r_in,
                                            l1.r_out,
                                            l1.gamma,
                                            Some(e1),
                                        );
                                    }
                                }
                                if size_in > 0 {
                                    let out2 = quote_pool(
                                        e2.kind,
                                        out1,
                                        l2.r_in,
                                        l2.r_out,
                                        l2.gamma,
                                        Some(e2),
                                    );
                                    if out2 > size_in as u128 {
                                        let impact = cpmm_price_impact_bps(
                                            size_in as u128,
                                            l1.r_in,
                                            l1.r_out,
                                            l1.gamma,
                                        );
                                        let hop_inputs = vec![size_in, out1 as u64, out2 as u64];
                                        let hops = vec![
                                            SwapHop {
                                                pool_id: e1.pool_id,
                                                protocol: e1.protocol,
                                                mint_in: e1.from_mint,
                                                mint_out: e1.to_mint,
                                                fee_bps: e1.fee_bps,
                                                active_bin_id: e1.active_bin_id,
                                                token_programs: e1.token_programs,
                                                trade_intent: Default::default(),
                                                execution_accounts: e1.execution_accounts.clone(),
                                            },
                                            SwapHop {
                                                pool_id: e2.pool_id,
                                                protocol: e2.protocol,
                                                mint_in: e2.from_mint,
                                                mint_out: e2.to_mint,
                                                fee_bps: e2.fee_bps,
                                                active_bin_id: e2.active_bin_id,
                                                token_programs: e2.token_programs,
                                                trade_intent: Default::default(),
                                                execution_accounts: e2.execution_accounts.clone(),
                                            },
                                        ];
                                        if let Some(opp) = self.make_economic_opportunity(
                                            base,
                                            current_slot,
                                            RouteId(route_counter),
                                            size_in,
                                            out2,
                                            hop_inputs,
                                            impact,
                                            hops,
                                        ) {
                                            opportunities.push(opp);
                                            route_counter += 1;
                                        }
                                    }
                                }
                            }
                        }
                        continue;
                    }

                    // Case 2: 3-Hop Arbitrage (Base -> inter1 -> inter2 -> Base)
                    let inter2 = e2.to_mint;
                    if inter2 == base || inter2 == inter1 {
                        continue;
                    }

                    let Some(third_hops) = self.edges_by_from.get(&inter2) else {
                        continue;
                    };

                    for e3 in third_hops {
                        if e3.to_mint != base {
                            continue;
                        }
                        if e1.pool_id == e2.pool_id
                            || e2.pool_id == e3.pool_id
                            || e1.pool_id == e3.pool_id
                        {
                            continue;
                        }

                        let l1 = Leg {
                            r_in: e1.reserve_in,
                            r_out: e1.reserve_out,
                            gamma: 10_000u128.saturating_sub(e1.fee_bps as u128),
                        };
                        let l2 = Leg {
                            r_in: e2.reserve_in,
                            r_out: e2.reserve_out,
                            gamma: 10_000u128.saturating_sub(e2.fee_bps as u128),
                        };
                        let l3 = Leg {
                            r_in: e3.reserve_in,
                            r_out: e3.reserve_out,
                            gamma: 10_000u128.saturating_sub(e3.fee_bps as u128),
                        };

                        let dlmm_hops = (e1.kind == PoolKind::Dlmm) as usize
                            + (e2.kind == PoolKind::Dlmm) as usize
                            + (e3.kind == PoolKind::Dlmm) as usize;
                        let initial_size = optimal_three_pool(&l1, &l2, &l3).or(if dlmm_hops > 0 {
                            Some(100_000)
                        } else {
                            None
                        });

                        if let Some(opt_size) = initial_size {
                            let mut size_in = opt_size.min(max_trade_size);

                            // Active-bin bottleneck expansion for Meteora DLMM routes
                            if dlmm_hops >= 2 {
                                if let (Some(c1), Some(c2), Some(c3)) = (
                                    Self::edge_max_input_capacity(e1),
                                    Self::edge_max_input_capacity(e2),
                                    Self::edge_max_input_capacity(e3),
                                ) {
                                    if c1 > 0 && c2 > 0 && c3 > 0 {
                                        let probe_in = 100_000u64.min(c1).min(max_trade_size);
                                        if probe_in > 0 {
                                            let p_out1 = quote_pool(
                                                e1.kind,
                                                probe_in as u128,
                                                l1.r_in,
                                                l1.r_out,
                                                l1.gamma,
                                                Some(e1),
                                            );
                                            let p_out2 = quote_pool(
                                                e2.kind,
                                                p_out1,
                                                l2.r_in,
                                                l2.r_out,
                                                l2.gamma,
                                                Some(e2),
                                            );
                                            let p_out3 = quote_pool(
                                                e3.kind,
                                                p_out2,
                                                l3.r_in,
                                                l3.r_out,
                                                l3.gamma,
                                                Some(e3),
                                            );
                                            if p_out3 > probe_in as u128 {
                                                let probe_profit =
                                                    (p_out3 - probe_in as u128) as u64;
                                                let hop2_in_equiv = (probe_in as u128 * c2 as u128)
                                                    .checked_div(p_out1)
                                                    .unwrap_or(0)
                                                    as u64;
                                                let hop3_in_equiv = (probe_in as u128 * c3 as u128)
                                                    .checked_div(p_out2)
                                                    .unwrap_or(0)
                                                    as u64;
                                                let (scaled_in, _) =
                                                    arb_math::dlmm::scale_dlmm_three_hop_input(
                                                        probe_in,
                                                        probe_profit,
                                                        c1,
                                                        hop2_in_equiv,
                                                        hop3_in_equiv,
                                                        max_trade_size,
                                                    );
                                                if scaled_in > size_in {
                                                    size_in = scaled_in;
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            // Two-sided CLMM & DLMM active tick/bin capacity clamping (Hop 1)
                            if let Some(c1) = Self::edge_max_input_capacity(e1) {
                                if c1 > 0 {
                                    size_in = size_in.min(c1);
                                }
                            }
                            if size_in > 0 {
                                let mut out1 = quote_pool(
                                    e1.kind,
                                    size_in as u128,
                                    l1.r_in,
                                    l1.r_out,
                                    l1.gamma,
                                    Some(e1),
                                );
                                // Two-sided CLMM & DLMM active tick/bin capacity clamping (Hop 2)
                                if let Some(c2) = Self::edge_max_input_capacity(e2) {
                                    if c2 > 0 && out1 > c2 as u128 {
                                        size_in = ((size_in as u128 * c2 as u128) / out1) as u64;
                                        out1 = quote_pool(
                                            e1.kind,
                                            size_in as u128,
                                            l1.r_in,
                                            l1.r_out,
                                            l1.gamma,
                                            Some(e1),
                                        );
                                    }
                                }
                                if size_in > 0 {
                                    let mut out2 = quote_pool(
                                        e2.kind,
                                        out1,
                                        l2.r_in,
                                        l2.r_out,
                                        l2.gamma,
                                        Some(e2),
                                    );
                                    // Two-sided CLMM & DLMM active tick/bin capacity clamping (Hop 3)
                                    if let Some(c3) = Self::edge_max_input_capacity(e3) {
                                        if c3 > 0 && out2 > c3 as u128 {
                                            size_in =
                                                ((size_in as u128 * c3 as u128) / out2) as u64;
                                            out1 = quote_pool(
                                                e1.kind,
                                                size_in as u128,
                                                l1.r_in,
                                                l1.r_out,
                                                l1.gamma,
                                                Some(e1),
                                            );
                                            out2 = quote_pool(
                                                e2.kind,
                                                out1,
                                                l2.r_in,
                                                l2.r_out,
                                                l2.gamma,
                                                Some(e2),
                                            );
                                        }
                                    }
                                    if size_in > 0 {
                                        let out3 = quote_pool(
                                            e3.kind,
                                            out2,
                                            l3.r_in,
                                            l3.r_out,
                                            l3.gamma,
                                            Some(e3),
                                        );

                                        if out3 > size_in as u128 {
                                            let impact = cpmm_price_impact_bps(
                                                size_in as u128,
                                                l1.r_in,
                                                l1.r_out,
                                                l1.gamma,
                                            );
                                            let hop_inputs = vec![
                                                size_in,
                                                out1 as u64,
                                                out2 as u64,
                                                out3 as u64,
                                            ];
                                            let hops = vec![
                                                SwapHop {
                                                    pool_id: e1.pool_id,
                                                    protocol: e1.protocol,
                                                    mint_in: e1.from_mint,
                                                    mint_out: e1.to_mint,
                                                    fee_bps: e1.fee_bps,
                                                    active_bin_id: e1.active_bin_id,
                                                    token_programs: e1.token_programs,
                                                    trade_intent: Default::default(),
                                                    execution_accounts: e1
                                                        .execution_accounts
                                                        .clone(),
                                                },
                                                SwapHop {
                                                    pool_id: e2.pool_id,
                                                    protocol: e2.protocol,
                                                    mint_in: e2.from_mint,
                                                    mint_out: e2.to_mint,
                                                    fee_bps: e2.fee_bps,
                                                    active_bin_id: e2.active_bin_id,
                                                    token_programs: e2.token_programs,
                                                    trade_intent: Default::default(),
                                                    execution_accounts: e2
                                                        .execution_accounts
                                                        .clone(),
                                                },
                                                SwapHop {
                                                    pool_id: e3.pool_id,
                                                    protocol: e3.protocol,
                                                    mint_in: e3.from_mint,
                                                    mint_out: e3.to_mint,
                                                    fee_bps: e3.fee_bps,
                                                    active_bin_id: e3.active_bin_id,
                                                    token_programs: e3.token_programs,
                                                    trade_intent: Default::default(),
                                                    execution_accounts: e3
                                                        .execution_accounts
                                                        .clone(),
                                                },
                                            ];
                                            if let Some(opp) = self.make_economic_opportunity(
                                                base,
                                                current_slot,
                                                RouteId(route_counter),
                                                size_in,
                                                out3,
                                                hop_inputs,
                                                impact,
                                                hops,
                                            ) {
                                                opportunities.push(opp);
                                                route_counter += 1;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Candidate Ranking: Sort opportunities descending by net profit
        opportunities.sort_by_key(|a| std::cmp::Reverse(a.net_profit.0));
        opportunities
    }

    /// Discover multi-hop arbitrage cycles up to `max_hops` (e.g. 4-hop, 5-hop) using DFS cycle traversal
    /// and golden-section / bisection optimal trade sizing.
    pub fn find_multi_hop_cycles(
        &self,
        current_slot: Slot,
        max_trade_size: u64,
        max_hops: usize,
    ) -> Vec<ExecutionOpportunity> {
        let mut opportunities = Vec::new();
        let mut route_counter = 1000u32;
        let clamped_max_hops = max_hops.clamp(2, 6);

        for &base in &self.base_anchors {
            let mut current_path: Vec<&MarketEdge> = Vec::new();
            let mut visited_pools: std::collections::HashSet<PoolId> =
                std::collections::HashSet::new();

            self.dfs_multi_hop(
                base,
                base,
                clamped_max_hops,
                &mut current_path,
                &mut visited_pools,
                current_slot,
                max_trade_size,
                &mut route_counter,
                &mut opportunities,
            );
        }

        // Candidate Ranking: Sort multi-hop opportunities descending by net profit
        opportunities.sort_by_key(|a| std::cmp::Reverse(a.net_profit.0));
        opportunities
    }

    #[allow(clippy::too_many_arguments)]
    fn dfs_multi_hop<'a>(
        &'a self,
        current_mint: MintId,
        base_anchor: MintId,
        max_hops: usize,
        path: &mut Vec<&'a MarketEdge>,
        visited_pools: &mut std::collections::HashSet<PoolId>,
        current_slot: Slot,
        max_trade_size: u64,
        route_counter: &mut u32,
        results: &mut Vec<ExecutionOpportunity>,
    ) {
        if path.len() >= 2 && current_mint == base_anchor {
            if let Some(opp) =
                self.evaluate_arbitrage_cycle(path, current_slot, max_trade_size, *route_counter)
            {
                results.push(opp);
                *route_counter = route_counter.saturating_add(1);
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

            // Safe Lower-Bound Pruning:
            // Calculate current path's marginal rate. If even an extreme 2.0x remaining rate cannot yield profit, prune!
            let mut current_rate = 1.0f64;
            for e in path.iter() {
                let gamma = (10_000u32.saturating_sub(e.fee_bps as u32)) as f64 / 10_000.0;
                current_rate *= (e.reserve_out as f64 / e.reserve_in as f64) * gamma;
            }
            let remaining_hops = max_hops.saturating_sub(path.len());
            let theoretical_max_remaining = 2.0f64.powi(remaining_hops as i32);

            if current_rate * theoretical_max_remaining >= 1.0010 {
                self.dfs_multi_hop(
                    edge.to_mint,
                    base_anchor,
                    max_hops,
                    path,
                    visited_pools,
                    current_slot,
                    max_trade_size,
                    route_counter,
                    results,
                );
            }

            path.pop();
            visited_pools.remove(&edge.pool_id);
        }
    }

    fn evaluate_arbitrage_cycle(
        &self,
        path: &[&MarketEdge],
        current_slot: Slot,
        max_trade_size: u64,
        route_id: u32,
    ) -> Option<ExecutionOpportunity> {
        // 1. Marginal rate check
        let mut marginal_product = 1.0f64;
        for e in path {
            let gamma = (10_000u32.saturating_sub(e.fee_bps as u32)) as f64 / 10_000.0;
            let rate = (e.reserve_out as f64 / e.reserve_in as f64) * gamma;
            marginal_product *= rate;
        }

        if marginal_product <= 1.0010 {
            return None;
        }

        // P1 #11: golden-section search for the peak (not the breakeven edge).
        let low = 1_000u64;
        let mut high = max_trade_size;
        if high <= low {
            high = low + 1;
        }
        let best_size = golden_section_size(path, low, high, 48);
        let best_profit = cycle_profit_at(path, best_size);
        // recompute final output at best_size
        let mut curr = best_size as u128;
        for e in path {
            let gamma = 10_000u128.saturating_sub(e.fee_bps as u128);
            curr = quote_pool(e.kind, curr, e.reserve_in, e.reserve_out, gamma, Some(e));
        }

        if best_profit == 0 || best_size == 0 {
            return None;
        }

        // Clamp at marginal degradation ceiling if economic config is available.
        let clamped_size = if let Some(config) = &self.economic_config {
            // Compute flash fee in bps for the best_size
            let flash_fee_raw =
                calculate_flash_fee_raw(best_size, config.flash_loan_fee_sf).ok()?;
            let flash_fee_bps = if best_size > 0 {
                ((flash_fee_raw as u128 * 10_000) / best_size as u128) as u64
            } else {
                0
            };

            // For 2-hop CPMM cycles, use the analytical clamp.
            if path.len() == 2 && path[0].kind == PoolKind::Cpmm && path[1].kind == PoolKind::Cpmm {
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
                clamp_at_marginal_ceiling_two_pool(best_size, &l1, &l2, flash_fee_bps)
                    .unwrap_or(best_size)
            } else {
                // For mixed/3+ hop cycles: general clamp using marginal derivative.
                // If the marginal profit derivative at best_size is negative, reduce size.
                // Use a simple heuristic: if flash fee exceeds 50% of gross profit, halve the size.
                let gross_profit = best_profit;
                if flash_fee_raw * 2 > gross_profit && best_size > low {
                    best_size / 2
                } else {
                    best_size
                }
            }
        } else {
            best_size
        };

        // Recompute final output at clamped_size
        let mut curr = clamped_size as u128;
        for e in path {
            let gamma = 10_000u128.saturating_sub(e.fee_bps as u128);
            curr = quote_pool(e.kind, curr, e.reserve_in, e.reserve_out, gamma, Some(e));
        }
        let final_out = curr as u64;

        // Recompute profit at clamped size (verify it's still positive)
        if u128::from(final_out) <= clamped_size as u128 {
            return None;
        };

        let first_edge = path[0];
        let gamma1 = 10_000u128.saturating_sub(first_edge.fee_bps as u128);
        let impact = cpmm_price_impact_bps(
            clamped_size as u128,
            first_edge.reserve_in,
            first_edge.reserve_out,
            gamma1,
        );

        let hops = path
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

        // Compute per-hop expected outputs for correct min_amount_out derivation.
        let mut hop_inputs = vec![clamped_size];
        let mut curr = clamped_size as u128;
        for e in path {
            let gamma = 10_000u128.saturating_sub(e.fee_bps as u128);
            curr = quote_pool(e.kind, curr, e.reserve_in, e.reserve_out, gamma, Some(e));
            hop_inputs.push(curr as u64);
        }

        // Authoritative economic calculation: no hardcoded flash loan fee,
        // no fabricated tip fraction, no hardcoded decimals. Fail closed when
        // no economic config is set or the profit floor is not met.
        let base_mint = path[0].from_mint;
        self.make_economic_opportunity(
            base_mint,
            current_slot,
            RouteId(route_id),
            clamped_size,
            final_out as u128,
            hop_inputs,
            impact,
            hops,
        )
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
    fn graph_2hop_detection() {
        let base = MintId::new([1; 32]);
        let _token_b = MintId::new([2; 32]);

        // Pool 1: 1M Base / 1M TokenB
        let p1 = make_pool(10, 1, 2, 1_000_000, 1_000_000);
        // Pool 2: 1M TokenB / 1.05M Base (5% arb spread)
        let p2 = make_pool(20, 2, 1, 1_000_000, 1_050_000);

        let mut graph = MarketGraph::new(vec![base]).with_economic_config(test_config());
        graph.build_from_pools(&[p1, p2]);

        let opps = graph.find_base_cycles(Slot(1), 100_000);
        assert!(!opps.is_empty());
        assert!(opps[0].gross_profit.as_u64() > 0);
        assert_eq!(opps[0].hops.len(), 2);
    }

    #[test]
    fn graph_4hop_multi_detection() {
        let base = MintId::new([1; 32]);

        // 4-Hop cycle: Base(1) -> TokenA(2) -> TokenB(3) -> TokenC(4) -> Base(1)
        let p1 = make_pool(1, 1, 2, 10_000_000, 10_000_000);
        let p2 = make_pool(2, 2, 3, 10_000_000, 10_000_000);
        let p3 = make_pool(3, 3, 4, 10_000_000, 10_000_000);
        // Pool 4 offers 10% premium back to base
        let p4 = make_pool(4, 4, 1, 10_000_000, 11_200_000);

        let mut graph = MarketGraph::new(vec![base]).with_economic_config(test_config());
        graph.build_from_pools(&[p1, p2, p3, p4]);

        let opps = graph.find_multi_hop_cycles(Slot(1), 100_000, 4);
        assert!(!opps.is_empty(), "Must detect 4-hop profitable cycle");
        let four_hop = opps.iter().find(|o| o.hops.len() == 4);
        assert!(four_hop.is_some(), "Found 4-hop opportunity");
        assert!(four_hop.unwrap().gross_profit.as_u64() > 0);
    }

    #[test]
    fn test_chronos_opportunity_scoring() {
        let base = MintId::new([1; 32]);
        let p1 = make_pool(10, 1, 2, 1_000_000, 1_000_000);
        let p2 = make_pool(20, 2, 1, 1_000_000, 1_050_000);

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

    #[test]
    fn test_single_pool_trap_exclusion() {
        let base = MintId::new([1; 32]);
        // Only 1 pool exists between Base and TokenB (single-pool trap)
        let p1 = make_pool(10, 1, 2, 1_000_000, 1_000_000);

        let mut graph = MarketGraph::new(vec![base]);
        graph.build_from_pools(&[p1]);

        let opps = graph.find_base_cycles(Slot(1), 100_000);
        assert!(
            opps.is_empty(),
            "Single pool trap must be strictly excluded (e1.pool_id != e2.pool_id)"
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
