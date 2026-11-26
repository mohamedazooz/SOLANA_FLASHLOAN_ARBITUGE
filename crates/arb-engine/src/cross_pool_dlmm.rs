//! # MeteoraDlmmCrossPoolArbitrageStrategy
//!
//! First-class two-pool Meteora DLMM arbitrage (P0 first working strategy).
//!
//! Enumerates candidates as pairs of **distinct** LbPairs sharing the same
//! unordered token pair, evaluates **both** directions with the real
//! bin-traversal quote engine (`quote_pool` ⇒ `arb_math::dlmm::quote_dlmm_view`
//! — never x*y=k), and runs a bounded integer candidate-size optimizer:
//!
//! 1. Exact bin-drain breakpoint seeding: the profit/size curve of a two-pool
//!    DLMM cycle is piecewise-linear with knees exactly at the cumulative
//!    inputs that drain each pool's bins — leg-1 breakpoints are used directly
//!    and leg-2 breakpoints are mapped through leg-1's exact-out inverse.
//! 2. Coarse logarithmic sampling (power-of-two doubling ladder with linear
//!    fill) between the capacity floor and the configured / on-chain ceiling,
//!    with the breakpoints blended in as additional candidates.
//! 3. Discrete DLMM quote evaluation of every candidate through both pools.
//! 4. Integer golden-section local refinement inside the bracket around the
//!    best coarse candidate.
//!
//! All executable amounts are exact integer token base units. No floats are
//! used for any candidate size or output amount.
//!
//! Fail-closed rules:
//! - Missing fee parameters, empty bins, or a missing active bin ⇒ not priced.
//! - Missing execution accounts ⇒ `MissingAccounts`.
//! - Token-program mismatch across hops sharing a mint ⇒ rejected.
//! - Zero profitable directions is an acceptable, honest result.

use arb_math::economic::{
    EconomicConfig, calculate_flash_fee_raw, calculate_net_profit_raw_signed,
    slippage_haircut_min_final,
};
use arb_types::{
    DexProtocol, ExecutionOpportunity, Lamports, MintId, PoolId, PoolKind, RouteId, Slot, SwapHop,
    TokenProgramKind,
};
use std::collections::{HashMap, HashSet};

use crate::{
    EPSILON_CYCLE_SPREAD_BPS, MarketEdge, MarketGraph, dlmm_quote_components, quote_pool_detailed,
};

/// Tuning knobs for the cross-pool DLMM strategy. All defaults are safe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DlmmCrossPoolConfig {
    /// Maximum distinct pool *pairs* evaluated per scan (bounding CPU).
    pub max_pairs: usize,
    /// Minimum candidate input size in base-mint raw units.
    pub min_input: u64,
    /// Hard candidate input ceiling in base-mint raw units (risk limit).
    pub max_input: u64,
    /// Coarse candidate samples: power-of-two ladder plus linear fill.
    pub coarse_samples: usize,
    /// Local refinement iterations (integer golden section).
    pub refine_samples: usize,
    /// Minimum gross cycle spread in bps before economics are evaluated.
    pub min_spread_bps: u64,
    /// Skip candidate sizes below this raw base-unit input (dust guard).
    /// `u64` matches the deserializable bot-config field (`toml` cannot
    /// deserialize `u128`) and raw Solana token amounts, which fit in `u64`.
    pub min_liquidity_bin_raw: u64,
}

impl Default for DlmmCrossPoolConfig {
    fn default() -> Self {
        Self {
            max_pairs: 256,
            min_input: 100_000,
            max_input: u64::MAX,
            coarse_samples: 10,
            refine_samples: 8,
            min_spread_bps: EPSILON_CYCLE_SPREAD_BPS,
            min_liquidity_bin_raw: 1_000_000,
        }
    }
}

/// Structured rejection reason, one per evaluated direction that lost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CrossPoolRejectionReason {
    /// Cycle spread below the configured hurdle at every candidate size.
    EdgeBelowHurdle,
    /// Missing fee params, empty bins, missing active bin, no capacity.
    InsufficientLiquidity,
    /// Pool/bin state absent or otherwise unpriced.
    StalePoolState,
    /// SwapHop(s) would build without resolved execution accounts.
    MissingAccounts,
    /// Feasible size band empty because of configured size limits.
    MaxTradeSizeExceeded,
    /// Token program for a shared mint differs across the two hops.
    TokenProgramMismatch,
    /// Economic gate (net-profit floor / config) rejected the best candidate.
    EconomicGateRejected,
    /// An exact per-hop quote failed while validating a candidate or its impact.
    QuoteFailed,
}

impl CrossPoolRejectionReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EdgeBelowHurdle => "EDGE_BELOW_HURDLE",
            Self::InsufficientLiquidity => "INSUFFICIENT_LIQUIDITY",
            Self::StalePoolState => "STALE_POOL_STATE",
            Self::MissingAccounts => "MISSING_ACCOUNTS",
            Self::MaxTradeSizeExceeded => "MAX_TRADE_SIZE_EXCEEDED",
            Self::TokenProgramMismatch => "TOKEN_PROGRAM_MISMATCH",
            Self::EconomicGateRejected => "ECONOMIC_GATE_REJECTED",
            Self::QuoteFailed => "QUOTE_FAILED",
        }
    }

    fn from_edge_detail(detail: &str) -> Self {
        match detail {
            "missing DLMM state" => Self::StalePoolState,
            _ => Self::InsufficientLiquidity,
        }
    }
}

/// One structured rejection with debug context.
#[derive(Debug, Clone)]
pub struct CrossPoolRejection {
    pub base: String,
    pub mid: String,
    pub pool_first: PoolId,
    pub pool_second: PoolId,
    /// 1 = pool_first hops first, 2 = pool_second hops first.
    pub direction: u8,
    pub reason: CrossPoolRejectionReason,
    pub detail: String,
}

/// Aggregate scan statistics (surfaced in logs and op reports).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CrossPoolScanStats {
    pub pairs_scanned: u64,
    pub directions_evaluated: u64,
    pub opportunities: u64,
    /// Rejection reason name → occurrence count, sorted asc.
    pub rejections_by_reason: Vec<(String, u64)>,
}

impl CrossPoolScanStats {
    fn from_rejections(
        pairs_scanned: u64,
        directions_evaluated: u64,
        opportunities: u64,
        rejections: &[CrossPoolRejection],
    ) -> Self {
        let mut m: HashMap<&'static str, u64> = HashMap::new();
        for rej in rejections {
            *m.entry(rej.reason.as_str()).or_insert(0) += 1;
        }
        let mut pairs: Vec<(String, u64)> =
            m.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        pairs.sort();
        Self {
            pairs_scanned,
            directions_evaluated,
            opportunities,
            rejections_by_reason: pairs,
        }
    }
}

/// Full result of one strategy scan.
#[derive(Debug, Clone, Default)]
pub struct CrossPoolScan {
    pub opportunities: Vec<ExecutionOpportunity>,
    pub rejections: Vec<CrossPoolRejection>,
    pub stats: CrossPoolScanStats,
}

/// Token program of `mint` as seen from `edge` (from_mint ⇒ programs[0]).
fn edge_program_of(edge: &MarketEdge, mint: MintId) -> TokenProgramKind {
    if edge.from_mint == mint {
        edge.token_programs[0]
    } else {
        edge.token_programs[1]
    }
}

/// Bounded attempts for the second-hop capacity rescale.
const MAX_RESIZE_PASSES: usize = 4;

/// Upper bound on breakpoint-derived candidate seeds injected per direction.
/// The coarse ladder and refinement steps stay in charge of the final choice;
/// seeding only restores the exact piecewise-linear knees the ladder misses.
const MAX_BREAKPOINT_SEEDS: usize = 24;

/// Exact bin-drain candidate seeds for the two-leg cycle (the knees of its
/// piecewise-linear profit/size curve):
///
/// - Leg-1 breakpoints are cumulative fee-included inputs — already in cycle
///   size space.
/// - Leg-2 breakpoints are leg-2 input (= mid-mint) values; each is mapped
///   through leg-1's `quote_dlmm_exact_out` inverse — the exact leg-1 input
///   whose net output equals that mid-mint amount — so no slope heuristic is
///   used anywhere in the mapping.
/// - Leg-2 breakpoints above the second hop's capacity ceiling (when known)
///   are dropped: `two_hop_quote` already rescales such sizes down onto the
///   same knee.
///
/// The result is shaped by `dlmm_breakpoint_seed_sizes` (band filter, dedupe,
/// even spread, `MAX_BREAKPOINT_SEEDS` cap). A failed quote only drops a
/// seed — seeds are hints; every optimizer evaluation stays fail-closed.
fn pair_breakpoint_seeds(
    e1: &MarketEdge,
    e2: &MarketEdge,
    lo: u64,
    hi: u64,
    cap2: Option<u64>,
) -> Vec<u64> {
    let mut raw = Vec::new();

    if let Some((pool1, bins1)) = dlmm_quote_components(e1) {
        let dir1 = e1.pool_base == e1.from_mint;
        if let Ok(breaks1) = arb_math::dlmm::dlmm_drain_breakpoints(&pool1, &bins1, dir1) {
            raw.extend(breaks1.iter().map(|b| b.cumulative_input));
        }

        if let Some((pool2, bins2)) = dlmm_quote_components(e2) {
            let dir2 = e2.pool_base == e2.from_mint;
            if let Ok(breaks2) = arb_math::dlmm::dlmm_drain_breakpoints(&pool2, &bins2, dir2) {
                for b in breaks2 {
                    if let Some(cap) = cap2
                        && cap > 0
                        && b.cumulative_input > cap
                    {
                        continue;
                    }
                    if let Ok(q) = arb_math::dlmm::quote_dlmm_exact_out(
                        &pool1,
                        &bins1,
                        b.cumulative_input,
                        dir1,
                        None,
                    ) {
                        if arb_math::dlmm::validate_swap_depth(q.bins_crossed).is_ok() {
                            raw.push(q.amount_in);
                        }
                    }
                }
            }
        }
    }

    arb_math::dlmm::dlmm_breakpoint_seed_sizes(&raw, lo, hi, MAX_BREAKPOINT_SEEDS)
}

/// Complete the two-leg integer quote chain for one direction at input `size`,
/// honoring the second pool's active-bin capacity via proportional rescaling.
///
/// Returns `(effective_size, out1, final_out)`, or `None` when any quote fails
/// (fail-closed: no constant-product fallback).
fn two_hop_quote(
    e1: &MarketEdge,
    e2: &MarketEdge,
    size: u64,
    cap2: Option<u64>,
) -> Option<(u64, u128, u128)> {
    let mut cur_size = size;
    for _ in 0..MAX_RESIZE_PASSES {
        let out1 = quote_pool_detailed(
            PoolKind::Dlmm,
            cur_size as u128,
            e1.reserve_in,
            e1.reserve_out,
            10_000u128.saturating_sub(e1.fee_bps as u128),
            Some(e1),
        )?
        .amount_out;
        if out1 == 0 {
            return None;
        }
        if let Some(cap) = cap2
            && cap > 0
            && out1 > cap as u128
        {
            let rescaled = ((cur_size as u128 * cap as u128) / out1) as u64;
            if rescaled == 0 || rescaled >= cur_size {
                // Cannot shrink further; this size class has no feasible hop 2.
                return None;
            }
            cur_size = rescaled;
            continue;
        }
        let final_out = quote_pool_detailed(
            PoolKind::Dlmm,
            out1,
            e2.reserve_in,
            e2.reserve_out,
            10_000u128.saturating_sub(e2.fee_bps as u128),
            Some(e2),
        )?
        .amount_out;
        if final_out == 0 {
            return None;
        }
        return Some((cur_size, out1, final_out));
    }
    None
}

/// Bounded candidate-size optimizer (Part D4).
///
/// 1. Coarse sampling: power-of-two doubling ladder from `lo` to `hi` joined
///    with a linear fill of `coarse_samples` points.
/// 2. Discrete DLMM evaluation of every candidate through both pools.
/// 3. Integer golden-section refinement inside the bracket `[best/2, best*3/2]`
///    clamped to `[lo, hi]`.
///
/// Returns the best candidate `(effective_size, out1, final_out, gross_profit)`
/// where `gross_profit = final_out - effective_size` (checked).
fn optimize_size(
    e1: &MarketEdge,
    e2: &MarketEdge,
    lo: u64,
    hi: u64,
    cfg: &DlmmCrossPoolConfig,
    cap2: Option<u64>,
    economics: &EconomicConfig,
) -> Option<(u64, u128, u128, u128)> {
    if lo == 0 || hi < lo {
        return None;
    }

    let eval_point = |size: u64| -> Option<(u64, u128, u128, i128)> {
        let (s, o1, fo) = two_hop_quote(e1, e2, size, cap2)?;
        let final_out = u64::try_from(fo).ok()?;
        let minimum_final =
            slippage_haircut_min_final(final_out, 2, economics.hop_slippage_bps).ok()?;
        let flash_fee = calculate_flash_fee_raw(s, economics.flash_loan_fee_sf).ok()?;
        // DEX fees are already embedded in the exact bin-traversal outputs.
        // Discovery has no verified per-route base/priority sample yet; Jito
        // tip and the verified rational base conversion are still included.
        let net = calculate_net_profit_raw_signed(
            minimum_final,
            s,
            flash_fee,
            0,
            0,
            0,
            0,
            Lamports(arb_constants::SOLANA_BASE_FEE_LAMPORTS),
            Lamports::ZERO,
            economics.jito_tip_lamports,
            economics.lamports_to_base_mint_rate,
            economics.lamports_to_base_rate_denom,
        )
        .ok()?;
        Some((s, o1, fo, net))
    };

    let mut evaluated: Vec<(u64, u128, u128, i128)> = Vec::new();

    // ── 1: coarse multi-basin sampling (powers of 1.5 + linear grid) ──
    let mut coarse_points = Vec::with_capacity(128);
    coarse_points.push(lo);
    let mut c = lo;
    while c < hi && coarse_points.len() < 128 {
        let next_u128 = ((c as u128 * 15) / 10).max(c as u128 + 1);
        let next = match u64::try_from(next_u128) {
            Ok(v) if v > c => v,
            _ => break,
        };
        if next > hi {
            break;
        }
        coarse_points.push(next);
        c = next;
    }
    if hi > lo {
        coarse_points.push(hi);
    }
    let fill = cfg.coarse_samples.clamp(2, 64);
    for k in 1..fill - 1 {
        let cv = lo as u128 + ((hi as u128 - lo as u128) * (k as u128 + 1)) / fill as u128;
        if let Ok(v) = u64::try_from(cv) {
            coarse_points.push(v);
        }
    }
    // Exact piecewise-linear knees from both legs' bin-drain enumeration.
    coarse_points.extend(pair_breakpoint_seeds(e1, e2, lo, hi, cap2));

    coarse_points.sort_unstable();
    coarse_points.dedup();

    for pt in coarse_points {
        if let Some(res) = eval_point(pt) {
            evaluated.push(res);
        }
    }

    if evaluated.is_empty() {
        return None;
    }

    evaluated.sort_by_key(|&(s, _, _, _)| s);
    evaluated.dedup_by_key(|&mut (s, _, _, _)| s);

    // Snapshot profits separately so peak detection never borrows `evaluated`
    // while it is still mutably held by sort/dedup.
    let profits: Vec<i128> = evaluated.iter().map(|&(_, _, _, p)| p).collect();
    let sizes: Vec<u64> = evaluated.iter().map(|&(s, _, _, _)| s).collect();

    // ── 2: Multi-Basin local peak detection (Rejecting global unimodality) ──
    let mut candidate_basins = Vec::new();
    for i in 0..evaluated.len() {
        let left_p = if i > 0 { profits[i - 1] } else { i128::MIN };
        let right_p = if i + 1 < evaluated.len() {
            profits[i + 1]
        } else {
            i128::MIN
        };
        let cur_p = profits[i];

        if cur_p >= left_p && cur_p >= right_p {
            let b_lo = if i > 0 { sizes[i - 1] } else { sizes[i] };
            let b_hi = if i + 1 < evaluated.len() {
                sizes[i + 1]
            } else {
                sizes[i]
            };
            candidate_basins.push((b_lo, b_hi));
        }
    }

    // ── 3: Discrete integer fine search across candidate basins ──
    let steps = cfg.refine_samples.max(4);
    for (b_lo, b_hi) in candidate_basins {
        if b_hi <= b_lo + 1 {
            continue;
        }
        let b_span = b_hi - b_lo;
        let step = (b_span / steps as u64).max(1);
        for s in 1..steps {
            let pt = b_lo + (s as u64) * step;
            if pt < b_hi
                && let Some(res) = eval_point(pt)
            {
                evaluated.push(res);
            }
        }
    }

    let (s, o1, fo, _) = evaluated.into_iter().max_by_key(|&(_, _, _, p)| p)?;
    let gross = fo.checked_sub(s as u128)?;
    Some((s, o1, fo, gross))
}

/// Validate one edge against the DLMM quoting preconditions (fail-closed).
fn dlmm_edge_ok(e: &MarketEdge) -> Result<(), &'static str> {
    if e.protocol != DexProtocol::MeteoraDlmm || e.kind != PoolKind::Dlmm {
        return Err("not a DLMM edge");
    }
    let Some(d) = &e.dlmm else {
        return Err("missing DLMM state");
    };
    let Some(params) = d.params.as_ref() else {
        return Err("missing fee params");
    };
    // ConcreteFunctionType 1 (LiquidityMining) is quote-equivalent to 0
    // (supportLimitOrder=false => zero order liquidity); 2+ (LimitOrder) has
    // no differential-parity evidence and is rejected fail-closed, matching
    // the LbPair-validation gate in arb-dex.
    if params.function_type > 1 {
        return Err("unsupported DLMM function type");
    }
    if d.bins.is_empty() {
        return Err("no bin liquidity decoded");
    }
    if !d.bins.iter().any(|b| b.bin_id == d.active_bin_id) {
        return Err("active bin missing from decoded bins");
    }
    Ok(())
}

/// Evaluate one direction `(l, r)` of a distinct-pool pair. Returns
/// `(size, out1, final_out, gross_profit)` of the best candidate size.
fn evaluate_direction(
    l: &MarketEdge,
    r: &MarketEdge,
    cfg: &DlmmCrossPoolConfig,
    max_trade_size: u64,
    economics: &EconomicConfig,
) -> Result<(u64, u128, u128, u128), CrossPoolRejectionReason> {
    if let Err(d) = dlmm_edge_ok(l) {
        return Err(CrossPoolRejectionReason::from_edge_detail(d));
    }
    if let Err(d) = dlmm_edge_ok(r) {
        return Err(CrossPoolRejectionReason::from_edge_detail(d));
    }
    // Hops must chain leg-to-leg through the same intermediate mint.
    let mid = l.to_mint;
    if mid != r.from_mint || l.from_mint != r.to_mint {
        return Err(CrossPoolRejectionReason::StalePoolState);
    }
    // Token program consistency for both shared mints.
    if edge_program_of(l, l.from_mint) != edge_program_of(r, l.from_mint)
        || edge_program_of(l, mid) != edge_program_of(r, mid)
    {
        return Err(CrossPoolRejectionReason::TokenProgramMismatch);
    }
    // Execution accounts required on both hops (tx build re-checks fail-closed).
    if l.execution_accounts.is_none() || r.execution_accounts.is_none() {
        return Err(CrossPoolRejectionReason::MissingAccounts);
    }

    let cap1 = MarketGraph::edge_max_input_capacity(l);
    let cap2 = MarketGraph::edge_max_input_capacity(r);
    let global_cap = max_trade_size.min(cfg.max_input);
    let lo = cfg.min_input.max(1).max(cfg.min_liquidity_bin_raw);
    let hi = match cap1 {
        Some(c) if c > 0 => global_cap.min(c),
        None => global_cap,
        Some(_) => return Err(CrossPoolRejectionReason::InsufficientLiquidity),
    };
    if hi < lo {
        return if max_trade_size < cfg.min_input {
            Err(CrossPoolRejectionReason::MaxTradeSizeExceeded)
        } else {
            Err(CrossPoolRejectionReason::InsufficientLiquidity)
        };
    }

    let Some((size, out1, final_out, gross)) = optimize_size(l, r, lo, hi, cfg, cap2, economics)
    else {
        return Err(CrossPoolRejectionReason::EdgeBelowHurdle);
    };

    let final_out_u64 =
        u64::try_from(final_out).map_err(|_| CrossPoolRejectionReason::EconomicGateRejected)?;
    let minimum_final = slippage_haircut_min_final(final_out_u64, 2, economics.hop_slippage_bps)
        .map_err(|_| CrossPoolRejectionReason::EconomicGateRejected)?;
    let flash_fee = calculate_flash_fee_raw(size, economics.flash_loan_fee_sf)
        .map_err(|_| CrossPoolRejectionReason::EconomicGateRejected)?;
    let conservative_net = calculate_net_profit_raw_signed(
        minimum_final,
        size,
        flash_fee,
        0,
        0,
        0,
        0,
        Lamports(arb_constants::SOLANA_BASE_FEE_LAMPORTS),
        Lamports::ZERO,
        economics.jito_tip_lamports,
        economics.lamports_to_base_mint_rate,
        economics.lamports_to_base_rate_denom,
    )
    .map_err(|_| CrossPoolRejectionReason::EconomicGateRejected)?;
    if conservative_net < i128::from(economics.min_profit_raw.as_u64()) {
        return Err(CrossPoolRejectionReason::EconomicGateRejected);
    }

    if final_out <= size as u128 || gross == 0 {
        return Err(CrossPoolRejectionReason::EdgeBelowHurdle);
    }
    let spread_bps = (gross * 10_000) / size.max(1) as u128;
    if spread_bps < cfg.min_spread_bps as u128 {
        return Err(CrossPoolRejectionReason::EdgeBelowHurdle);
    }
    Ok((size, out1, final_out, gross))
}

fn conv_u64(v: u128) -> u64 {
    u64::try_from(v).unwrap_or(u64::MAX)
}

fn swap_hop_of(edge: &MarketEdge) -> SwapHop {
    SwapHop {
        pool_id: edge.pool_id,
        protocol: edge.protocol,
        mint_in: edge.from_mint,
        mint_out: edge.to_mint,
        fee_bps: edge.fee_bps,
        active_bin_id: edge.active_bin_id,
        token_programs: edge.token_programs,
        trade_intent: Default::default(),
        execution_accounts: edge.execution_accounts.clone(),
    }
}

impl MarketGraph {
    /// Enumerate every distinct same-pair `(poolA, poolB)` DLMM combination
    /// anchored to a base mint, optimize both directions with real bin
    /// traversal, and return economics-cleared opportunities sorted by net
    /// profit (desc), plus all structured rejections and scan statistics.
    ///
    /// Both directions are covered naturally: pairing a leaving edge
    /// `base→mid` of pool A with a returning edge `mid→base` of pool B is
    /// direction A; the mirrored combo (pool B leaves, pool A returns) is a
    /// distinct `(l, r)` evaluation of the same unordered pair.
    pub fn find_cross_pool_dlmm_cycles(
        &mut self,
        current_slot: Slot,
        max_trade_size: u64,
        cfg: &DlmmCrossPoolConfig,
    ) -> CrossPoolScan {
        let mut rejections: Vec<CrossPoolRejection> = Vec::new();
        let mut opportunities = Vec::new();
        let mut route_counter = 9_000_001u32;
        let mut pairs_scanned = 0u64;
        let mut directions_evaluated = 0u64;
        let mut seen_unordered: HashSet<(PoolId, PoolId, MintId)> = HashSet::new();

        if self.economic_config.is_none() {
            // Fail closed: no economic config ⇒ nothing can be priced honestly.
            let stats = CrossPoolScanStats::default();
            return CrossPoolScan {
                opportunities,
                rejections,
                stats,
            };
        }

        // The economic configuration was checked above; use its verified
        // fee, slippage and base-mint conversion for every size candidate.
        let economics = self.economic_config.as_ref().expect("checked above");

        'anchors: for &base in &self.base_anchors {
            let leaving: Vec<MarketEdge> = self
                .edges_by_from
                .get(&base)
                .map(|edges| {
                    edges
                        .iter()
                        .filter(|e| e.protocol == DexProtocol::MeteoraDlmm && e.to_mint != base)
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();

            for l in &leaving {
                let mid = l.to_mint;
                let returning: Vec<MarketEdge> = self
                    .edges_by_from
                    .get(&mid)
                    .map(|edges| {
                        edges
                            .iter()
                            .filter(|e| {
                                e.protocol == DexProtocol::MeteoraDlmm
                                    && e.to_mint == base
                                    && e.pool_id != l.pool_id
                            })
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();

                for r in &returning {
                    directions_evaluated += 1;

                    // Unordered-pair scalpr for the scan stats; both directions
                    // of the same pool pair are separate evaluations.
                    let unordered_key = if l.pool_id.0 < r.pool_id.0 {
                        (l.pool_id, r.pool_id, mid)
                    } else {
                        (r.pool_id, l.pool_id, mid)
                    };
                    if seen_unordered.insert(unordered_key) {
                        pairs_scanned += 1;
                    }
                    if pairs_scanned > cfg.max_pairs as u64 {
                        break 'anchors;
                    }

                    let direction: u8 = if l.pool_id.0 < r.pool_id.0 { 1 } else { 2 };
                    match evaluate_direction(l, r, cfg, max_trade_size, economics) {
                        Ok((size, out1, final_out, _gross)) => {
                            let hops = vec![swap_hop_of(l), swap_hop_of(r)];
                            let hop_inputs = vec![size, conv_u64(out1), conv_u64(final_out)];
                            // Discovery seed impact is protocol-native for both DLMM hops.
                            // The Supervisor still re-quotes and resizes this route canonically.
                            let first_quote = quote_pool_detailed(
                                PoolKind::Dlmm,
                                u128::from(size),
                                l.reserve_in,
                                l.reserve_out,
                                10_000u128.saturating_sub(u128::from(l.fee_bps)),
                                Some(l),
                            );
                            let Some(first_quote) = first_quote else {
                                rejections.push(CrossPoolRejection {
                                    base: base.0.to_string(),
                                    mid: mid.0.to_string(),
                                    pool_first: l.pool_id,
                                    pool_second: r.pool_id,
                                    direction,
                                    reason: CrossPoolRejectionReason::QuoteFailed,
                                    detail:
                                        "first DLMM seed quote failed during impact calculation"
                                            .into(),
                                });
                                continue;
                            };
                            let second_quote = quote_pool_detailed(
                                PoolKind::Dlmm,
                                first_quote.amount_out,
                                r.reserve_in,
                                r.reserve_out,
                                10_000u128.saturating_sub(u128::from(r.fee_bps)),
                                Some(r),
                            );
                            let Some(second_quote) = second_quote else {
                                rejections.push(CrossPoolRejection {
                                    base: base.0.to_string(),
                                    mid: mid.0.to_string(),
                                    pool_first: l.pool_id,
                                    pool_second: r.pool_id,
                                    direction,
                                    reason: CrossPoolRejectionReason::QuoteFailed,
                                    detail:
                                        "second DLMM seed quote failed during impact calculation"
                                            .into(),
                                });
                                continue;
                            };
                            let impact = first_quote
                                .price_impact_bps
                                .max(second_quote.price_impact_bps);
                            if let Some(opp) = self.make_economic_opportunity(
                                base,
                                current_slot,
                                RouteId(route_counter),
                                size,
                                final_out,
                                hop_inputs,
                                impact,
                                hops,
                            ) {
                                route_counter += 1;
                                opportunities.push(opp);
                            } else {
                                rejections.push(CrossPoolRejection {
                                    base: base.0.to_string(),
                                    mid: mid.0.to_string(),
                                    pool_first: l.pool_id,
                                    pool_second: r.pool_id,
                                    direction,
                                    reason: CrossPoolRejectionReason::EconomicGateRejected,
                                    detail: "economic gate rejected best candidate".into(),
                                });
                            }
                        }
                        Err(reason) => {
                            rejections.push(CrossPoolRejection {
                                base: base.0.to_string(),
                                mid: mid.0.to_string(),
                                pool_first: l.pool_id,
                                pool_second: r.pool_id,
                                direction,
                                reason,
                                detail: "direction rejected".into(),
                            });
                        }
                    }
                }
            }
        }

        opportunities.sort_by_key(|a| std::cmp::Reverse(a.net_profit));
        let stats = CrossPoolScanStats::from_rejections(
            pairs_scanned,
            directions_evaluated,
            opportunities.len() as u64,
            &rejections,
        );
        CrossPoolScan {
            opportunities,
            rejections,
            stats,
        }
    }

    /// Ordered intermediate mints of a route (mints strictly between the
    /// endpoints). The executor must verify these accounts start **empty** at
    /// execution time so pre-existing payer inventory can never subsidize an
    /// unprofitable route (no-inventory-subsidy invariant, Part D5).
    pub fn cross_pool_intermediate_mints(opp: &ExecutionOpportunity) -> Vec<MintId> {
        cross_pool_intermediate_mints(opp)
    }
}

/// Ordered intermediate mints of a route (mints strictly between the
/// endpoints). The executor must verify these accounts start **empty** at
/// execution time so pre-existing payer inventory can never subsidize an
/// unprofitable route (no-inventory-subsidy invariant, Part D5).
pub fn cross_pool_intermediate_mints(opp: &ExecutionOpportunity) -> Vec<MintId> {
    let mut out = Vec::new();
    let (Some(first), Some(last)) = (opp.hops.first(), opp.hops.last()) else {
        return out;
    };
    for hop in &opp.hops {
        if hop.mint_in != first.mint_in && hop.mint_in != last.mint_out {
            if !out.contains(&hop.mint_in) {
                out.push(hop.mint_in);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_dex::engines::MeteoraDlmmQuoteEngine;
    use arb_math::dlmm::{DlmmParameters, DlmmPoolState, DlmmVariableParameters, quote_dlmm_view};
    use arb_types::{DexProtocol, MintId, PoolId, TokenProgramKind};
    use solana_pubkey::Pubkey;

    const BIN_STEP_BPS: u16 = 25;

    fn mint(tag: u64) -> MintId {
        let mut bytes = [0u8; 32];
        bytes[0..8].copy_from_slice(&(0xA11CEu64 + tag).to_le_bytes());
        MintId(Pubkey::new_from_array(bytes))
    }

    fn fee_params() -> arb_types::DlmmFeeParams {
        arb_types::DlmmFeeParams {
            base_factor: 200,
            filter_period: 60,
            decay_period: 600,
            reduction_factor: 5_000,
            variable_fee_control: 40_000,
            max_volatility_accumulator: 1_100_000,
            protocol_share: 1_000,
            base_fee_power_factor: 1,
            function_type: 0,
            collect_fee_mode: 0,
        }
    }

    /// Leg-1-style edge: base(X) → quote(Y), `swap_for_y = true` (walks down).
    /// Leg-2-style edge: quote → base, `swap_for_y = false` (walks up).
    fn dlmm_edge(
        tag: u64,
        base_to_quote: bool,
        active_id: i32,
        per_bin: u64,
        depth: i32,
    ) -> MarketEdge {
        let base = mint(tag);
        let quote = mint(tag + 100);
        let (from_mint, to_mint) = if base_to_quote {
            (base, quote)
        } else {
            (quote, base)
        };
        let step = if base_to_quote { -1_i32 } else { 1_i32 };
        let bins: Vec<arb_types::DlmmBin> = (0..depth)
            .map(|d| arb_types::DlmmBin {
                bin_id: active_id + d * step,
                amount_x: if base_to_quote { 0 } else { per_bin as u128 },
                amount_y: if base_to_quote { per_bin as u128 } else { 0 },
                ..Default::default()
            })
            .collect();
        let pool_id = PoolId(Pubkey::new_from_array([tag as u8; 32]));
        let bin_array_pdas: Vec<Pubkey> = bins
            .iter()
            .filter(|bin| bin.amount_x != 0 || bin.amount_y != 0)
            .map(|bin| {
                let index = MeteoraDlmmQuoteEngine::bin_id_to_bin_array_index(bin.bin_id);
                MeteoraDlmmQuoteEngine::derive_bin_array_pda(&pool_id.0, index).0
            })
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let execution_accounts = arb_types::PoolExecutionAccounts::MeteoraDlmm {
            reserve_x: Pubkey::new_unique(),
            reserve_y: Pubkey::new_unique(),
            oracle: Pubkey::new_unique(),
            bin_array_pdas,
            bitmap_extension: None,
            pool_token_x: base.0,
            pool_token_y: quote.0,
        };
        MarketEdge {
            pool_id,
            protocol: DexProtocol::MeteoraDlmm,
            kind: PoolKind::Dlmm,
            from_mint,
            to_mint,
            pool_base: base,
            pool_quote: quote,
            reserve_in: 1_000_000_000,
            reserve_out: 1_000_000_000,
            fee_bps: 5,
            fee_numerator: 5,
            fee_denominator: 10_000,
            raydium_cpmm_fee: None,
            clmm: None,
            dlmm: Some(arb_types::DlmmState {
                active_bin_id: active_id,
                status: 0,
                bin_step_bps: BIN_STEP_BPS,
                bins,
                params: Some(fee_params()),
                volatility_accumulator: 0,
                volatility_reference: 0,
                index_reference: active_id,
                last_update_timestamp: 1_700_000_000,
                support_limit_order: false,
            }),
            pump_fun: None,
            damm_v2: None,
            transfer_fee_bps: 0,
            token_programs: [TokenProgramKind::SplToken, TokenProgramKind::SplToken],
            execution_accounts: Some(execution_accounts),
            active_bin_id: Some(active_id),
        }
    }

    /// Independent hand-built arb-math view of an edge (mirrors the
    /// conversion under test without calling it).
    fn math_view(e: &MarketEdge) -> (DlmmPoolState, Vec<arb_math::dlmm::DlmmBin>) {
        let d = e.dlmm.as_ref().expect("dlmm state");
        let params = d.params.as_ref().expect("fee params");
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
            support_limit_order: d.support_limit_order,
        };
        let bins = d
            .bins
            .iter()
            .map(|b| arb_math::dlmm::DlmmBin {
                id: b.bin_id,
                amount_x: b.amount_x as u64,
                amount_y: b.amount_y as u64,
                ..Default::default()
            })
            .collect();
        (pool, bins)
    }

    fn leg1_breakpoints(e: &MarketEdge) -> Vec<arb_math::dlmm::DlmmDrainBreakpoint> {
        let (pool, bins) = math_view(e);
        arb_math::dlmm::dlmm_drain_breakpoints(&pool, &bins, e.pool_base == e.from_mint)
            .expect("leg-1 breakpoints")
    }

    #[test]
    fn seeds_cover_exactly_the_leg1_drain_breakpoints_when_leg2_has_no_window() {
        let e1 = dlmm_edge(1, true, 100, 20_000_000, 3);
        let mut e2 = dlmm_edge(2, false, 200, 30_000_000, 2);
        if let Some(d) = &mut e2.dlmm {
            d.bins.clear();
        }

        let seeds = pair_breakpoint_seeds(&e1, &e2, 1, u64::MAX, None);
        let expected: Vec<u64> = leg1_breakpoints(&e1)
            .iter()
            .map(|b| b.cumulative_input)
            .collect();
        assert_eq!(seeds, expected);
        assert!(seeds.windows(2).all(|w| w[0] < w[1]));
        assert!(seeds.len() <= MAX_BREAKPOINT_SEEDS);
    }

    #[test]
    fn seeds_map_leg2_breakpoints_exactly_through_the_leg1_inverse() {
        let e1 = dlmm_edge(1, true, 100, 20_000_000, 3);
        let e2 = dlmm_edge(2, false, 200, 30_000_000, 2);

        let leg1_set: Vec<u64> = leg1_breakpoints(&e1)
            .iter()
            .map(|b| b.cumulative_input)
            .collect();
        let (pool2, bins2) = math_view(&e2);
        let leg2_breaks =
            arb_math::dlmm::dlmm_drain_breakpoints(&pool2, &bins2, e2.pool_base == e2.from_mint)
                .expect("leg-2 breakpoints");
        let leg2_inputs: Vec<u64> = leg2_breaks.iter().map(|b| b.cumulative_input).collect();

        let seeds = pair_breakpoint_seeds(&e1, &e2, 1, u64::MAX, None);
        // Every leg-1 breakpoint survives, sorted and deduped.
        for v in &leg1_set {
            assert!(seeds.contains(v), "leg-1 breakpoint {v} missing from seeds");
        }
        // Every mapped seed is the exact leg-1 input whose net output equals
        // some leg-2 breakpoint input (re-verified through the quote engine).
        let mapped: Vec<u64> = seeds
            .iter()
            .copied()
            .filter(|s| !leg1_set.contains(s))
            .collect();
        assert!(
            !mapped.is_empty(),
            "leg-2 breakpoints must contribute mapped seeds"
        );
        let (pool1, bins1) = math_view(&e1);
        let dir1 = e1.pool_base == e1.from_mint;
        for s in &mapped {
            let q = quote_dlmm_view(&pool1, &bins1, *s, dir1, None).expect("leg-1 quote");
            assert!(
                leg2_inputs.contains(&q.amount_out),
                "seed {s} produces out1 = {} which is not a leg-2 breakpoint input {leg2_inputs:?}",
                q.amount_out
            );
        }
    }

    #[test]
    fn seeds_respect_band_and_second_hop_capacity() {
        let e1 = dlmm_edge(1, true, 100, 20_000_000, 3);
        let e2 = dlmm_edge(2, false, 200, 30_000_000, 2);
        let leg1_set: Vec<u64> = leg1_breakpoints(&e1)
            .iter()
            .map(|b| b.cumulative_input)
            .collect();

        // Band: breakpoints below `lo` are dropped.
        let lo_above_first = leg1_set[0] + 1;
        let seeds = pair_breakpoint_seeds(&e1, &e2, lo_above_first, u64::MAX, None);
        assert!(seeds.iter().all(|&s| s >= lo_above_first));
        assert!(!seeds.contains(&leg1_set[0]));

        // Second-hop capacity: breakpoints above `cap2` would be rescaled by
        // `two_hop_quote` onto the same knee, so they are not seeded.
        let (pool2, bins2) = math_view(&e2);
        let leg2_first =
            arb_math::dlmm::dlmm_drain_breakpoints(&pool2, &bins2, e2.pool_base == e2.from_mint)
                .expect("leg-2 breakpoints")[0]
                .cumulative_input;
        let seeds_capped = pair_breakpoint_seeds(&e1, &e2, 1, u64::MAX, Some(leg2_first - 1));
        let seeds_full = pair_breakpoint_seeds(&e1, &e2, 1, u64::MAX, None);
        assert!(seeds_capped.len() < seeds_full.len());
        assert_eq!(seeds_capped, leg1_set);
    }

    #[test]
    fn seeds_fail_closed_to_empty_when_leg1_cannot_be_priced() {
        let e1 = dlmm_edge(1, true, 100, 20_000_000, 3);
        let e2 = dlmm_edge(2, false, 200, 30_000_000, 2);
        let mut disabled = e1.clone();
        if let Some(d) = &mut disabled.dlmm {
            d.status = 1;
        }
        assert!(pair_breakpoint_seeds(&disabled, &e2, 1, u64::MAX, None).is_empty());
        // Missing fee params must behave the same way.
        let mut unpriced = e1.clone();
        if let Some(d) = &mut unpriced.dlmm {
            d.params = None;
        }
        assert!(pair_breakpoint_seeds(&unpriced, &e2, 1, u64::MAX, None).is_empty());
    }
}
