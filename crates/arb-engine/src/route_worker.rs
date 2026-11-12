//! Autonomous Route Architect Worker & Convex Split-Routing Engine.
//! Continuously monitors live pool reserves and solves the closed-form
//! Equal Marginal Price Impact convex optimization problem:
//! Splits flash loans across multiple DEX pools (Raydium, Orca, Meteora)
//! to completely eliminate slippage and maximize net atomic profit.

use arb_ingest::RpcPool;
use arb_types::{DexProtocol, ExecutionOpportunity, Lamports, PoolId, PoolState};
use base64::prelude::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;
use tracing::debug;

/// Sized Split Leg specifying exact input allocation per venue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitLeg {
    pub pool_id: PoolId,
    pub protocol: DexProtocol,
    pub amount_in: u64,
    pub expected_out: u64,
}

/// Sized atomic multi-leg split opportunity designed by the worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitOpportunity {
    pub total_loan_lamports: u64,
    pub split_legs: Vec<SplitLeg>,
    pub return_pool_id: PoolId,
    pub return_protocol: DexProtocol,
    pub expected_total_out: u64,
    pub gross_profit: Lamports,
    pub net_profit: Lamports,
    pub jito_tip: Lamports,
}

/// Result of connecting the Route Architect split engine to the main pipeline:
/// the optimal equal-marginal-price-impact allocation across N parallel pools
/// for an opportunity whose single-pool price impact exceeds the threshold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitAllocation {
    /// One leg per pool, in allocation order. `Σ leg.amount_in == total_loan`.
    pub legs: Vec<SplitLeg>,
}

/// Dynamic pool reserves state.
#[derive(Debug, Clone)]
pub struct PoolDepth {
    pub pool_id: PoolId,
    pub protocol: DexProtocol,
    pub reserve_in: u128,
    pub reserve_out: u128,
    pub fee_bps: u64,
}

pub struct RouteArchitectWorker {
    #[allow(dead_code)]
    rpc_pool: RpcPool,
    running: Arc<AtomicBool>,
}

impl RouteArchitectWorker {
    pub fn new(rpc_pool: RpcPool) -> Self {
        Self {
            rpc_pool,
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Calculate the closed-form Equal Marginal Price Impact split for a loan X
    /// across two parallel AMM liquidity pools (e.g. Raydium and Orca).
    /// Maximizes combined output Y = y1(x1) + y2(x2) where x1 + x2 = X.
    pub fn calculate_optimal_split_two_pools(
        pool1: &PoolDepth,
        pool2: &PoolDepth,
        total_loan: u64,
    ) -> (u64, u64) {
        let x_total = total_loan as f64;
        if x_total <= 0.0 {
            return (0, 0);
        }

        let r1_x = pool1.reserve_in as f64;
        let r1_y = pool1.reserve_out as f64;
        let gamma1 = (10_000.0 - pool1.fee_bps as f64) / 10_000.0;

        let r2_x = pool2.reserve_in as f64;
        let r2_y = pool2.reserve_out as f64;
        let gamma2 = (10_000.0 - pool2.fee_bps as f64) / 10_000.0;

        if r1_x <= 0.0 || r1_y <= 0.0 || r2_x <= 0.0 || r2_y <= 0.0 {
            return (total_loan, 0);
        }

        // Closed-form marginal multiplier C:
        // C = (sqrt(r1_x * r1_y / gamma1) + sqrt(r2_x * r2_y / gamma2)) / (X + r1_x/gamma1 + r2_x/gamma2)
        let num_c = (r1_x * r1_y / gamma1).sqrt() + (r2_x * r2_y / gamma2).sqrt();
        let den_c = x_total + (r1_x / gamma1) + (r2_x / gamma2);

        if den_c <= 0.0 {
            return (total_loan / 2, total_loan - total_loan / 2);
        }

        let c = num_c / den_c;

        // x1* = (1 / gamma1) * (sqrt(r1_x * r1_y * gamma1) / C - r1_x)
        let x1_raw = (1.0 / gamma1) * (((r1_x * r1_y * gamma1).sqrt() / c) - r1_x);
        let x1_bounded = x1_raw.clamp(0.0, x_total);
        let x2_bounded = x_total - x1_bounded;

        (x1_bounded.round() as u64, x2_bounded.round() as u64)
    }

    /// Calculate CPMM output tokens for a given leg
    pub fn cpmm_output(amount_in: u64, r_in: u128, r_out: u128, fee_bps: u64) -> u64 {
        if amount_in == 0 || r_in == 0 || r_out == 0 {
            return 0;
        }
        let gamma = 10_000u128.saturating_sub(fee_bps as u128);
        let amount_in_u128 = amount_in as u128;
        let num = r_out.saturating_mul(amount_in_u128).saturating_mul(gamma);
        let den = r_in
            .saturating_mul(10_000)
            .saturating_add(amount_in_u128.saturating_mul(gamma));
        num.checked_div(den).map(|v| v as u64).unwrap_or(0)
    }

    /// Equal-marginal-price-impact split of `total_loan` across N parallel pools.
    ///
    /// Maximises `Y = Σ y_i(x_i)` subject to `Σ x_i = X`, where
    /// `y_i(x) = γ_i·x·r_i_y / (D·r_i_x + γ_i·x)`. The first-order condition
    /// `dy_i/dx_i = C` (a common marginal `C`) gives
    /// `x_i(C) = (√(γ_i·r_i_y·D·r_i_x / C) − D·r_i_x) / γ_i`.
    /// `Σ x_i(C)` decreases monotonically in `C`, so `C` is found by bisection.
    ///
    /// Returns the per-pool allocation; the sum equals `total_loan` exactly.
    pub fn calculate_optimal_split_n_pools(pools: &[PoolDepth], total_loan: u64) -> Vec<u64> {
        let n = pools.len();
        if n == 0 || total_loan == 0 {
            return vec![0; n];
        }
        if n == 1 {
            return vec![total_loan];
        }

        let d = 10_000.0f64;
        // (r_in, r_out, gamma) per pool; gamma is the fee complement (0..1).
        let terms: Vec<(f64, f64, f64)> = pools
            .iter()
            .map(|p| {
                (
                    p.reserve_in as f64,
                    p.reserve_out as f64,
                    (10_000.0 - p.fee_bps as f64) / 10_000.0,
                )
            })
            .collect();

        // x_i(C) for a common marginal C; 0 when the pool is already drained
        // (the radicand goes negative, i.e. C is above that pool's spot marginal).
        let x_at = |(rx, ry, g): (f64, f64, f64), c: f64| -> f64 {
            if g <= 0.0 || rx <= 0.0 || ry <= 0.0 || c <= 0.0 {
                return 0.0;
            }
            let inner = g * ry * d * rx / c;
            if inner <= 0.0 {
                return 0.0;
            }
            ((inner.sqrt() - d * rx) / g).max(0.0)
        };

        let x_total = total_loan as f64;
        // Bisection on C: Σ x_i(C) is strictly decreasing, so widen until the
        // bracket straddles x_total, then converge.
        let mut lo = 1e-12f64;
        let mut hi = 1e24f64;
        // Ensure f(lo) > x_total > f(hi).
        let mut guard = 0;
        while terms.iter().map(|t| x_at(*t, lo)).sum::<f64>() < x_total && guard < 80 {
            lo /= 10.0;
            guard += 1;
        }
        guard = 0;
        while terms.iter().map(|t| x_at(*t, hi)).sum::<f64>() > x_total && guard < 80 {
            hi *= 10.0;
            guard += 1;
        }
        for _ in 0..200 {
            let mid = 0.5 * (lo + hi);
            let sum: f64 = terms.iter().map(|t| x_at(*t, mid)).sum();
            if sum > x_total {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let c = 0.5 * (lo + hi);

        let mut alloc: Vec<u64> = terms.iter().map(|t| x_at(*t, c).round() as u64).collect();

        // Reconcile integer rounding so the allocation sums to `total_loan`.
        let sum: u64 = alloc.iter().sum();
        if sum != total_loan {
            if let Some((idx, _)) = alloc.iter().enumerate().max_by_key(|(_, v)| **v) {
                if sum < total_loan {
                    alloc[idx] = alloc[idx].saturating_add(total_loan - sum);
                } else {
                    alloc[idx] = alloc[idx].saturating_sub(sum - total_loan);
                }
            }
        }
        alloc
    }

    /// Build a `PoolDepth` snapshot from a live `PoolState`. Returns `None` when
    /// the pool's fee is invalid (zero denominator) — the pool must be skipped,
    /// never priced with a guessed default.
    pub fn pool_depth_from_state(state: &PoolState) -> Option<PoolDepth> {
        if !state.is_fee_valid() {
            return None;
        }
        let fee = ((state.fee_numerator as u128 * 10_000) / state.fee_denominator as u128) as u64;
        Some(PoolDepth {
            pool_id: state.pool_id,
            protocol: state.protocol,
            reserve_in: state.reserve_in,
            reserve_out: state.reserve_out,
            fee_bps: fee,
        })
    }

    /// Connect the Route Architect split engine to the main pipeline.
    ///
    /// When an opportunity's single-pool price impact exceeds `max_price_impact_bps`,
    /// look for a parallel pool offering the same mint pair (same reserves direction)
    /// and compute the optimal two-pool equal-marginal-price-impact split allocation
    /// using the closed-form `calculate_optimal_split_two_pools` solver. Returns the
    /// sized split if a viable parallel pool is found, otherwise `None`.
    pub fn maybe_split_allocation(
        active_pools: &[PoolState],
        opp: &ExecutionOpportunity,
        max_price_impact_bps: u64,
    ) -> Option<SplitAllocation> {
        // Only split when the single-pool price impact actually breaches the limit.
        if (opp.price_impact_bps.0 as u64) <= max_price_impact_bps || opp.hops.is_empty() {
            return None;
        }

        let first = &opp.hops[0];
        let total_loan = opp.size_in.as_u64();
        if total_loan == 0 {
            return None;
        }

        // Collect the primary pool plus EVERY parallel pool for the same mint
        // pair, all in the SAME direction as the first hop (base -> quote).
        let candidates: Vec<&PoolState> = active_pools
            .iter()
            .filter(|p| {
                p.is_active
                    && p.reserve_in > 0
                    && p.reserve_out > 0
                    && p.is_fee_valid()
                    && p.base_mint == first.mint_in
                    && p.quote_mint == first.mint_out
            })
            .collect();

        // Need the routed pool plus at least one parallel pool to split.
        if candidates.len() < 2 || !candidates.iter().any(|p| p.pool_id == first.pool_id) {
            return None;
        }

        let depths: Vec<PoolDepth> = candidates
            .iter()
            .filter_map(|p| Self::pool_depth_from_state(p))
            .collect();
        if depths.len() < 2 {
            return None;
        }

        let alloc = Self::calculate_optimal_split_n_pools(&depths, total_loan);

        let mut legs = Vec::new();
        for (d, amount) in depths.iter().zip(alloc.iter()) {
            if *amount == 0 {
                continue;
            }
            let out = Self::cpmm_output(*amount, d.reserve_in, d.reserve_out, d.fee_bps);
            legs.push(SplitLeg {
                pool_id: d.pool_id,
                protocol: d.protocol,
                amount_in: *amount,
                expected_out: out,
            });
        }
        if legs.is_empty() {
            return None;
        }
        Some(SplitAllocation { legs })
    }

    /// Continuously poll depth and synthesize split opportunities, broadcasting to supervisor.
    ///
    /// DISABLED: the original body synthesized opportunities with a hardcoded 5 bps
    /// Kamino fee and a 50%-of-profit Jito tip (FIN-01 / FIN-03) - exactly the
    /// fabricated economics the remediation forbids. The signature is kept so no
    /// caller breaks, but the loop does nothing. See docs/PRODUCTION_READINESS.md.
    pub async fn start_worker_loop(
        self: Arc<Self>,
        _tx_opportunity: UnboundedSender<SplitOpportunity>,
    ) {
        debug!("start_worker_loop disabled: fabricated-economics synthesizer removed");
        loop {
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

#[allow(dead_code)]
fn decode_account_data(acc: &serde_json::Value) -> Option<Vec<u8>> {
    let data_str = acc.get("data")?.get(0)?.as_str()?;
    BASE64_STANDARD.decode(data_str).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_pubkey::Pubkey;
    #[test]
    fn test_calculate_optimal_split_two_pools() {
        // Case 1: Proportional split with identical fees
        let pool1 = PoolDepth {
            pool_id: PoolId(Pubkey::new_from_array([1; 32])),
            protocol: DexProtocol::RaydiumV4,
            reserve_in: 100_000_000_000_000,
            reserve_out: 10_000_000_000_000,
            fee_bps: 25,
        };
        let pool2 = PoolDepth {
            pool_id: PoolId(Pubkey::new_from_array([2; 32])),
            protocol: DexProtocol::OrcaWhirlpool,
            reserve_in: 200_000_000_000_000,
            reserve_out: 20_000_000_000_000,
            fee_bps: 25,
        };

        let total_loan = 30_000_000_000; // 30 SOL
        let (x1, x2) =
            RouteArchitectWorker::calculate_optimal_split_two_pools(&pool1, &pool2, total_loan);
        assert_eq!(x1 + x2, total_loan);
        assert_eq!(x1, 10_000_000_000); // 10 SOL
        assert_eq!(x2, 20_000_000_000); // 20 SOL

        // Case 2: Large loan with fee differential (600 SOL)
        let pool2_low_fee = PoolDepth {
            fee_bps: 5,
            ..pool2
        };
        let large_loan = 600_000_000_000; // 600 SOL
        let (lx1, lx2) = RouteArchitectWorker::calculate_optimal_split_two_pools(
            &pool1,
            &pool2_low_fee,
            large_loan,
        );
        assert_eq!(lx1 + lx2, large_loan);
        assert!(lx1 > 0);
        assert!(lx2 > lx1);
    }

    #[test]
    fn n_pool_split_reduces_to_two_pools() {
        // The N-pool solver must agree with the closed-form 2-pool solver.
        let p1 = PoolDepth {
            pool_id: PoolId(Pubkey::new_from_array([1; 32])),
            protocol: DexProtocol::RaydiumV4,
            reserve_in: 100_000_000_000_000,
            reserve_out: 10_000_000_000_000,
            fee_bps: 25,
        };
        let p2 = PoolDepth {
            pool_id: PoolId(Pubkey::new_from_array([2; 32])),
            protocol: DexProtocol::OrcaWhirlpool,
            reserve_in: 200_000_000_000_000,
            reserve_out: 20_000_000_000_000,
            fee_bps: 25,
        };
        let loan = 30_000_000_000u64;
        let (x1, x2) = RouteArchitectWorker::calculate_optimal_split_two_pools(&p1, &p2, loan);
        let alloc = RouteArchitectWorker::calculate_optimal_split_n_pools(&[p1, p2], loan);
        assert_eq!(alloc.iter().sum::<u64>(), loan);
        // Within 1% of the closed form.
        assert!((alloc[0] as i128 - x1 as i128).abs() <= (loan / 100) as i128);
        assert!((alloc[1] as i128 - x2 as i128).abs() <= (loan / 100) as i128);
    }

    #[test]
    fn n_pool_split_sums_exactly_and_biases_to_cheaper_pool() {
        let expensive = PoolDepth {
            pool_id: PoolId(Pubkey::new_from_array([3; 32])),
            protocol: DexProtocol::RaydiumV4,
            reserve_in: 100_000_000_000_000,
            reserve_out: 10_000_000_000_000,
            fee_bps: 30,
        };
        let cheap = PoolDepth {
            pool_id: PoolId(Pubkey::new_from_array([4; 32])),
            protocol: DexProtocol::OrcaWhirlpool,
            reserve_in: 100_000_000_000_000,
            reserve_out: 10_000_000_000_000,
            fee_bps: 1,
        };
        let loan = 50_000_000_000u64;
        let alloc =
            RouteArchitectWorker::calculate_optimal_split_n_pools(&[expensive, cheap], loan);
        assert_eq!(
            alloc.iter().sum::<u64>(),
            loan,
            "allocation must sum exactly"
        );
        // The cheaper pool should receive the larger share.
        assert!(alloc[1] > alloc[0], "cheaper pool must get more");
    }

    #[test]
    fn n_pool_split_handles_three_pools_and_zero() {
        let mk = |seed: u8, rx: u128, fee: u64| PoolDepth {
            pool_id: PoolId(Pubkey::new_from_array([seed; 32])),
            protocol: DexProtocol::RaydiumV4,
            reserve_in: rx,
            reserve_out: rx / 10,
            fee_bps: fee,
        };
        let pools = [
            mk(5, 1_000_000_000_000, 25),
            mk(6, 2_000_000_000_000, 4),
            mk(7, 500_000_000_000, 30),
        ];
        let loan = 40_000_000_000u64;
        let alloc = RouteArchitectWorker::calculate_optimal_split_n_pools(&pools, loan);
        assert_eq!(alloc.len(), 3);
        assert_eq!(alloc.iter().sum::<u64>(), loan);
        assert_eq!(
            RouteArchitectWorker::calculate_optimal_split_n_pools(&[], 100),
            Vec::<u64>::new()
        );
        assert_eq!(
            RouteArchitectWorker::calculate_optimal_split_n_pools(&pools, 0),
            vec![0, 0, 0]
        );
    }
}
