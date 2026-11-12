//! # MeteoraDlmmCrossPoolArbitrageStrategy
//!
//! First-class two-pool Meteora DLMM arbitrage (P0 first working strategy).
//!
//! Enumerates candidates as pairs of **distinct** LbPairs sharing the same
//! unordered token pair, evaluates **both** directions with the real
//! bin-traversal quote engine (`quote_pool` ⇒ `arb_math::dlmm::quote_dlmm_view`
//! — never x*y=k), and runs a bounded integer candidate-size optimizer:
//!
//! 1. Coarse logarithmic sampling (power-of-two doubling ladder with linear
//!    fill) between the capacity floor and the configured / on-chain ceiling.
//! 2. Discrete DLMM quote evaluation of every candidate through both pools.
//! 3. Integer golden-section local refinement inside the bracket around the
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

use arb_math::cpmm_price_impact_bps;
use arb_types::{
    DexProtocol, ExecutionOpportunity, MintId, PoolId, PoolKind, RouteId, Slot, SwapHop,
    TokenProgramKind,
};
use std::collections::{HashMap, HashSet};

use crate::{EPSILON_CYCLE_SPREAD_BPS, MarketEdge, MarketGraph, quote_pool};

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
    pub min_liquidity_bin_raw: u128,
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
        let out1 = quote_pool(
            PoolKind::Dlmm,
            cur_size as u128,
            e1.reserve_in,
            e1.reserve_out,
            10_000u128.saturating_sub(e1.fee_bps as u128),
            Some(e1),
        );
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
        let final_out = quote_pool(
            PoolKind::Dlmm,
            out1,
            e2.reserve_in,
            e2.reserve_out,
            10_000u128.saturating_sub(e2.fee_bps as u128),
            Some(e2),
        );
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
) -> Option<(u64, u128, u128, u128)> {
    if lo == 0 || hi < lo {
        return None;
    }

    let eval_point = |size: u64| -> Option<(u64, u128, u128, i128)> {
        let (s, o1, fo) = two_hop_quote(e1, e2, size, cap2)?;
        let p = (fo as i128).checked_sub(s as i128)?;
        Some((s, o1, fo, p))
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
    if params.function_type != 0 {
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
    let lo = cfg
        .min_input
        .max(1)
        .max(cfg.min_liquidity_bin_raw.min(u64::MAX as u128) as u64);
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

    let Some((size, out1, final_out, gross)) = optimize_size(l, r, lo, hi, cfg, cap2) else {
        return Err(CrossPoolRejectionReason::EdgeBelowHurdle);
    };

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
                    match evaluate_direction(l, r, cfg, max_trade_size) {
                        Ok((size, out1, final_out, _gross)) => {
                            let hops = vec![swap_hop_of(l), swap_hop_of(r)];
                            let hop_inputs = vec![size, conv_u64(out1), conv_u64(final_out)];
                            let impact = cpmm_price_impact_bps(
                                size as u128,
                                l.reserve_in,
                                l.reserve_out,
                                10_000u128.saturating_sub(l.fee_bps as u128),
                            );
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
