//! Constant Product Market Maker (CPMM) Math & Optimal Sizing Engine.
//!
//! Enforces:
//! - Exact basis-point fee scaling (D = 10,000)
//! - Strict existence invariant check (N > D0)
//! - Wide-integer un-factored radicand sizing: U384 (2-hop, <=293 bits), U512 (3-hop, <=441 bits)
//! - Pure integer return types (`Option<u64>`) with zero lossy float casts
//! - Marginal profit degradation ceiling check (dPi / d(Delta x) <= 0)

use arb_types::{Bps, TokenAmount};
use ruint::Uint;
use ruint::aliases::U256;

pub type U384 = Uint<384, 6>;
pub type U512 = Uint<512, 8>;
pub type U768 = Uint<768, 12>;

pub const BASIS_POINTS_DIVISOR: u64 = 10_000;
pub const NANO_SCALE: u128 = 1_000_000_000;

/// Quote result for a swap on a CPMM pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpmmQuote {
    pub amount_out: TokenAmount,
    pub price_impact_bps: Bps,
}

/// Pool dimension parameters for optimal trade sizing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Leg {
    pub r_in: u128,
    pub r_out: u128,
    pub gamma: u128, // fee complement in basis points, e.g. 9975 for 0.25% fee
}

/// Calculate CPMM output with standard 10,000 basis points fee denominator.
/// Returns `None` on overflow or invalid inputs (fail-closed).
pub fn cpmm_out(dx: u128, x: u128, y: u128, gamma: u128) -> Option<u128> {
    cpmm_out_fee(dx, x, y, gamma, 10_000)
}

/// Calculate CPMM output with explicit fee numerator (gamma) and fee denominator (D).
/// Returns `None` on overflow or invalid inputs (fail-closed).
pub fn cpmm_out_fee(
    dx: u128,
    x: u128,
    y: u128,
    gamma: u128,
    fee_denominator: u128,
) -> Option<u128> {
    if dx == 0 || x == 0 || y == 0 || fee_denominator == 0 {
        return None;
    }
    let dx_u = U256::from(dx);
    let x_u = U256::from(x);
    let y_u = U256::from(y);
    let gamma_u = U256::from(gamma);
    let d_u = U256::from(fee_denominator);

    let num = gamma_u * dx_u * y_u;
    let denom = (d_u * x_u) + (gamma_u * dx_u);
    if denom.is_zero() {
        return None;
    }
    let out = num / denom;
    if out > U256::from(u128::MAX) {
        None
    } else {
        Some(out.to::<u128>())
    }
}

/// Convenient helper matching on-chain Raydium v4 quote interface.
/// Returns `None` on overflow or invalid inputs (fail-closed).
pub fn quote_cpmm(amount_in: u64, reserve_in: u64, reserve_out: u64, fee_bps: u64) -> Option<u64> {
    let gamma = u128::from(BASIS_POINTS_DIVISOR).saturating_sub(u128::from(fee_bps));
    let out = cpmm_out(
        u128::from(amount_in),
        u128::from(reserve_in),
        u128::from(reserve_out),
        gamma,
    )?;
    u64::try_from(out).ok()
}

/// Quote CPMM output along with price impact.
/// Returns `None` on overflow or invalid inputs (fail-closed).
pub fn quote_cpmm_full(
    amount_in: u64,
    reserve_in: u64,
    reserve_out: u64,
    fee_bps: u64,
) -> Option<CpmmQuote> {
    let out = quote_cpmm(amount_in, reserve_in, reserve_out, fee_bps)?;
    let gamma = u128::from(BASIS_POINTS_DIVISOR).saturating_sub(u128::from(fee_bps));
    let impact = cpmm_price_impact_bps(
        u128::from(amount_in),
        u128::from(reserve_in),
        u128::from(reserve_out),
        gamma,
    );
    Some(CpmmQuote {
        amount_out: TokenAmount(out),
        price_impact_bps: impact,
    })
}

/// Calculate price impact in Basis Points (bps).
/// Returns `Bps(10000)` (100% impact) on invalid inputs (fail-closed).
pub fn cpmm_price_impact_bps(dx: u128, x: u128, y: u128, gamma: u128) -> Bps {
    if dx == 0 || x == 0 || y == 0 {
        return Bps(10_000);
    }
    // Price impact is the curve effect ONLY, fees excluded (B16 fix).
    // `gamma` is retained in the signature for API stability but the fee-free
    // output is used so fees are never double-counted.
    let _ = gamma;
    let Some(out) = cpmm_out(dx, x, y, 10_000) else {
        return Bps(10_000);
    };
    if out == 0 {
        return Bps(10_000);
    }

    let out_u = U256::from(out);
    let x_u = U256::from(x);
    let dx_u = U256::from(dx);
    let y_u = U256::from(y);

    let exec_prod = out_u * x_u;
    let spot_prod = dx_u * y_u;
    if spot_prod.is_zero() || exec_prod >= spot_prod {
        return Bps(0);
    }

    let diff = spot_prod - exec_prod;
    let bps = (diff * U256::from(10_000)) / spot_prod;
    let bps_val = u16::try_from(bps.to::<u64>().min(10_000)).unwrap_or(10_000);
    Bps(bps_val)
}

/// Fee paid on a CPMM swap where the fee is computed as
/// `ceil(amount_in * fee_bps / 10_000)` (Raydium V4 convention).
pub fn cpmm_fee_paid(amount_in: u64, fee_bps: u64) -> u64 {
    cpmm_fee_paid_with_denom(amount_in, fee_bps, 10_000)
}

/// Fee paid on a CPMM swap with configurable denominator, computed as
/// `ceil(amount_in * fee_rate / fee_denominator)`.
///
/// Standard conventions:
/// - Raydium V4: fee_denominator = 10_000, fee_rate in basis points (bps).
/// - Raydium CPMM: fee_denominator = 1_000_000, fee_rate in parts per million (PPM).
pub fn cpmm_fee_paid_with_denom(amount_in: u64, fee_rate: u64, fee_denominator: u64) -> u64 {
    if fee_rate == 0 || fee_denominator == 0 {
        return 0;
    }
    (amount_in as u128 * fee_rate as u128).div_ceil(fee_denominator as u128) as u64
}
pub fn isqrt_u384(val: U384) -> U384 {
    if val <= U384::from(1) {
        return val;
    }
    let bits = val.bit_len();
    let mut x0 = U384::from(1) << bits.div_ceil(2);
    let mut x1 = (x0 + val / x0) >> 1;
    while x1 < x0 {
        x0 = x1;
        x1 = (x0 + val / x0) >> 1;
    }
    x0
}

/// Integer square root using Newton-Raphson for U512.
pub fn isqrt_u512(val: U512) -> U512 {
    if val <= U512::from(1) {
        return val;
    }
    let bits = val.bit_len();
    let mut x0 = U512::from(1) << bits.div_ceil(2);
    let mut x1 = (x0 + val / x0) >> 1;
    while x1 < x0 {
        x0 = x1;
        x1 = (x0 + val / x0) >> 1;
    }
    x0
}

/// Integer square root using Newton-Raphson for U768.
pub fn isqrt_u768(val: U768) -> U768 {
    if val <= U768::from(1) {
        return val;
    }
    let bits = val.bit_len();
    let mut x0 = U768::from(1) << bits.div_ceil(2);
    let mut x1 = (x0 + val / x0) >> 1;
    while x1 < x0 {
        x0 = x1;
        x1 = (x0 + val / x0) >> 1;
    }
    x0
}

/// Closed-form optimal input trade size d* for a 2-pool CPMM cross-arbitrage route.
///
/// Mathematical Formulation:
/// D0 = D^2 · x1 · y2
/// D1 = D · gamma1 · y2 + gamma1 · gamma2 · y1
/// N  = gamma1 · gamma2 · y1 · x2
///
/// Strict Existence Invariant: If N <= D0 -> None
/// Closed-Form Sizing: d* = (isqrt(N · D0) - D0) / D1
/// Direct Radicand: Computed in U384 (<= 293 bits) without double-truncation.
/// Return Type: Pure integer Option<u64>.
pub fn optimal_two_pool(l1: &Leg, l2: &Leg) -> Option<u64> {
    if l1.r_in == 0
        || l1.r_out == 0
        || l2.r_in == 0
        || l2.r_out == 0
        || l1.gamma == 0
        || l2.gamma == 0
    {
        return None;
    }

    let d = U384::from(BASIS_POINTS_DIVISOR);
    let x1 = U384::from(l1.r_in);
    let y1 = U384::from(l1.r_out);
    let g1 = U384::from(l1.gamma);

    let y2 = U384::from(l2.r_in);
    let x2 = U384::from(l2.r_out);
    let g2 = U384::from(l2.gamma);

    // D0 = D^2 · x1 · y2
    let d0 = d * d * x1 * y2;

    // D1 = D · gamma1 · y2 + gamma1 · gamma2 · y1
    let d1 = (d * g1 * y2) + (g1 * g2 * y1);

    // N = gamma1 · gamma2 · y1 · x2
    let n = g1 * g2 * y1 * x2;

    // Strict Arbitrage Existence Invariant
    if n <= d0 || d1.is_zero() {
        return None;
    }

    // Direct un-factored radicand in U384 (eliminates factored double-truncation)
    let radicand = n * d0;
    let sqrt_n_d0 = isqrt_u384(radicand);

    if sqrt_n_d0 <= d0 {
        return None;
    }

    let num = sqrt_n_d0 - d0;
    let d_star = num / d1;

    if d_star.is_zero() {
        None
    } else {
        u64::try_from(d_star).ok()
    }
}

/// Closed-form optimal input trade size d3* for a 3-hop CPMM triangular route.
///
/// Mathematical Formulation:
/// D0,3 = D^3 · x1 · y2 · z3
/// D1,3 = gamma1 · [ D^2 · y2 · z3 + D · gamma2 · y1 · z3 + gamma2 · gamma3 · y1 · z2 ]
/// N3   = gamma1 · gamma2 · gamma3 · y1 · z2 · x3
///
/// Strict Existence Invariant: If N3 <= D0,3 -> None
/// Closed-Form Sizing: d3* = (isqrt(N3 · D0,3) - D0,3) / D1,3
/// Direct Radicand: Computed in U512 (<= 441 bits) without double-truncation.
/// Return Type: Pure integer Option<u64>.
pub fn optimal_three_pool(l1: &Leg, l2: &Leg, l3: &Leg) -> Option<u64> {
    if l1.r_in == 0
        || l1.r_out == 0
        || l2.r_in == 0
        || l2.r_out == 0
        || l3.r_in == 0
        || l3.r_out == 0
        || l1.gamma == 0
        || l2.gamma == 0
        || l3.gamma == 0
    {
        return None;
    }

    let d = U512::from(BASIS_POINTS_DIVISOR);
    let x1 = U512::from(l1.r_in);
    let y1 = U512::from(l1.r_out);
    let g1 = U512::from(l1.gamma);

    let y2 = U512::from(l2.r_in);
    let z2 = U512::from(l2.r_out);
    let g2 = U512::from(l2.gamma);

    let z3 = U512::from(l3.r_in);
    let x3 = U512::from(l3.r_out);
    let g3 = U512::from(l3.gamma);

    // D0,3 = D^3 · x1 · y2 · z3
    let d0_3 = d * d * d * x1 * y2 * z3;

    // D1,3 = gamma1 · [ D^2 · y2 · z3 + D · gamma2 · y1 · z3 + gamma2 · gamma3 · y1 · z2 ]
    let bracket = (d * d * y2 * z3) + (d * g2 * y1 * z3) + (g2 * g3 * y1 * z2);
    let d1_3 = g1 * bracket;

    // N3 = gamma1 · gamma2 · gamma3 · y1 · z2 · x3
    let n_3 = g1 * g2 * g3 * y1 * z2 * x3;

    // Strict Arbitrage Existence Invariant
    if n_3 <= d0_3 || d1_3.is_zero() {
        return None;
    }

    // Direct un-factored radicand in U512 (eliminates factored double-truncation)
    let radicand = n_3 * d0_3;
    let sqrt_prod = isqrt_u512(radicand);

    if sqrt_prod <= d0_3 {
        return None;
    }

    let num = sqrt_prod - d0_3;
    let d3_star = num / d1_3;

    if d3_star.is_zero() {
        None
    } else {
        u64::try_from(d3_star).ok()
    }
}

/// Closed-form optimal input trade size d4* for a 4-hop CPMM cycle using composite Möbius transformations.
///
/// Mathematical Formulation:
/// M12 = M2 * M1, M34 = M4 * M3, M_cycle = M34 * M12
/// A_cycle = gamma1 * gamma2 * gamma3 * gamma4 * y1 * z2 * w3 * x4
/// D_cycle = D^4 * x1 * y2 * z3 * w4
/// C_cycle = C34 * A12 + D34 * C12
///
/// Strict Existence Invariant: If A_cycle <= D_cycle -> None
/// Closed-Form Sizing: d4* = (isqrt(A_cycle * D_cycle) - D_cycle) / C_cycle
/// Direct Radicand: Computed in U768 (<= 618 bits) without intermediate truncation.
/// Return Type: Pure integer Option<u64>.
pub fn optimal_four_pool(l1: &Leg, l2: &Leg, l3: &Leg, l4: &Leg) -> Option<u64> {
    if l1.r_in == 0
        || l1.r_out == 0
        || l2.r_in == 0
        || l2.r_out == 0
        || l3.r_in == 0
        || l3.r_out == 0
        || l4.r_in == 0
        || l4.r_out == 0
        || l1.gamma == 0
        || l2.gamma == 0
        || l3.gamma == 0
        || l4.gamma == 0
    {
        return None;
    }

    let d = U768::from(BASIS_POINTS_DIVISOR);
    let x1 = U768::from(l1.r_in);
    let y1 = U768::from(l1.r_out);
    let g1 = U768::from(l1.gamma);

    let y2 = U768::from(l2.r_in);
    let z2 = U768::from(l2.r_out);
    let g2 = U768::from(l2.gamma);

    let z3 = U768::from(l3.r_in);
    let w3 = U768::from(l3.r_out);
    let g3 = U768::from(l3.gamma);

    let w4 = U768::from(l4.r_in);
    let x4 = U768::from(l4.r_out);
    let g4 = U768::from(l4.gamma);

    // M12 = M2 * M1
    let a12 = g2 * z2 * g1 * y1;
    let d12 = d * d * x1 * y2;
    let c12 = g1 * (g2 * y1 + d * y2);

    // M34 = M4 * M3
    let a34 = g4 * x4 * g3 * w3;
    let d34 = d * d * z3 * w4;
    let c34 = g3 * (g4 * w3 + d * w4);

    // M_cycle = M34 * M12
    let a_cycle = a34 * a12;
    let d_cycle = d34 * d12;
    let c_cycle = (c34 * a12) + (d34 * c12);

    // Strict Arbitrage Existence Invariant
    if a_cycle <= d_cycle || c_cycle.is_zero() {
        return None;
    }

    // Direct un-factored radicand in U768 (<= 618 bits)
    let radicand = a_cycle * d_cycle;
    let sqrt_prod = isqrt_u768(radicand);

    if sqrt_prod <= d_cycle {
        return None;
    }

    let num = sqrt_prod - d_cycle;
    let d4_star = num / c_cycle;

    if d4_star.is_zero() {
        None
    } else {
        u64::try_from(d4_star).ok()
    }
}

/// Evaluates whether a proposed trade size is past the marginal profit degradation ceiling:
/// dPi / d(Delta x) <= 0
///
/// Returns true if trade size is past the profit peak and should be truncated.
pub fn is_past_marginal_degradation_ceiling_two_pool(
    size: u64,
    l1: &Leg,
    l2: &Leg,
    flash_fee_bps: u64,
) -> bool {
    if size == 0 {
        return false;
    }

    let d = U384::from(BASIS_POINTS_DIVISOR);
    let x1 = U384::from(l1.r_in);
    let y1 = U384::from(l1.r_out);
    let g1 = U384::from(l1.gamma);

    let y2 = U384::from(l2.r_in);
    let x2 = U384::from(l2.r_out);
    let g2 = U384::from(l2.gamma);

    let d0 = d * d * x1 * y2;
    let d1 = (d * g1 * y2) + (g1 * g2 * y1);
    let n = g1 * g2 * y1 * x2;

    if n <= d0 {
        return true; // Unprofitable even at Delta x -> 0
    }

    let delta_x = U384::from(size);
    let denom_term = d0 + (d1 * delta_x);

    // Cross-multiplication invariant:
    // LHS = N · D0 · D
    // RHS = (D0 + D1 · Delta x)^2 · (D + flash_fee_bps)
    let lhs = n * d0 * d;
    let cost_multiplier = d + U384::from(flash_fee_bps);
    let rhs = denom_term * denom_term * cost_multiplier;

    // Past ceiling if LHS <= RHS (meaning marginal return <= marginal cost)
    lhs <= rhs
}

/// Computes the exact analytical marginal return derivative:
/// dPi / d(Delta x) = (N · D0) / (D0 + D1 · Delta x)^2 - (1 + flash_fee_bps / 10,000)
///
/// Output is scaled by 10^9 (nano-units):
/// - Positive: profit is still increasing with trade size
/// - Zero: exact profit peak
/// - Negative: trade size has exceeded profit peak (degrading return)
pub fn marginal_profit_derivative_cpmm(size: u64, l1: &Leg, l2: &Leg, flash_fee_bps: u64) -> i128 {
    let d = U384::from(BASIS_POINTS_DIVISOR);
    let x1 = U384::from(l1.r_in);
    let y1 = U384::from(l1.r_out);
    let g1 = U384::from(l1.gamma);

    let y2 = U384::from(l2.r_in);
    let x2 = U384::from(l2.r_out);
    let g2 = U384::from(l2.gamma);

    let d0 = d * d * x1 * y2;
    let d1 = (d * g1 * y2) + (g1 * g2 * y1);
    let n = g1 * g2 * y1 * x2;

    let delta_x = U384::from(size);
    let denom_term = d0 + (d1 * delta_x);
    let denom_sq = denom_term * denom_term;

    if denom_sq.is_zero() {
        return i128::MIN;
    }

    let scale = U384::from(NANO_SCALE);
    let marginal_rev_scaled = (n * d0 * scale) / denom_sq;
    let marginal_cost_scaled = (scale * (d + U384::from(flash_fee_bps))) / d;

    let rev_i128 = i128::try_from(marginal_rev_scaled.to::<u128>()).unwrap_or(i128::MAX);
    let cost_i128 = i128::try_from(marginal_cost_scaled.to::<u128>()).unwrap_or(i128::MAX);

    rev_i128.saturating_sub(cost_i128)
}

/// Generic discrete marginal derivative evaluator:
/// Evaluates marginal net return derivative dPi / d(Delta x) given candidate borrow size and output.
pub fn marginal_profit_derivative(size: u64, out: u64, flash_fee_bps: u64) -> i128 {
    if size == 0 {
        return 0;
    }
    let size_u = u128::from(size);
    let out_u = u128::from(out);
    let cost_bps = u128::from(BASIS_POINTS_DIVISOR) + u128::from(flash_fee_bps);

    // marginal revenue per unit borrowed * 10^9
    let rev_scaled = (out_u * NANO_SCALE) / size_u;
    let cost_scaled = (NANO_SCALE * cost_bps) / u128::from(BASIS_POINTS_DIVISOR);

    let rev_i128 = i128::try_from(rev_scaled).unwrap_or(i128::MAX);
    let cost_i128 = i128::try_from(cost_scaled).unwrap_or(i128::MAX);

    rev_i128.saturating_sub(cost_i128)
}

/// Clamps candidate borrow size at the marginal degradation ceiling.
/// If candidate_size exceeds d*, it truncates to d* to preserve peak profit.
pub fn clamp_at_marginal_ceiling_two_pool(
    candidate_size: u64,
    l1: &Leg,
    l2: &Leg,
    flash_fee_bps: u64,
) -> Option<u64> {
    let optimal = optimal_two_pool(l1, l2)?;
    let clamped = candidate_size.min(optimal);
    if is_past_marginal_degradation_ceiling_two_pool(clamped, l1, l2, flash_fee_bps)
        && clamped > optimal
    {
        Some(optimal)
    } else {
        Some(clamped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::any;

    #[test]
    fn test_cpmm_out_fee_scaling() {
        // dx=10, x=1000, y=1000, gamma=9975 (0.25% fee)
        // num = 9975 * 10 * 1000 = 99,750,000
        // denom = 10,000 * 1000 + 9975 * 10 = 10,000,000 + 99,750 = 10,099,750
        // out = 99,750,000 / 10,099,750 = 9
        let out = cpmm_out(10, 1000, 1000, 9975).expect("valid inputs");
        assert_eq!(out, 9);

        // Zero checks return None
        assert_eq!(cpmm_out(0, 1000, 1000, 9975), None);
        assert_eq!(cpmm_out(10, 0, 1000, 9975), None);
        assert_eq!(cpmm_out(10, 1000, 0, 9975), None);
    }

    #[test]
    fn test_quote_cpmm_interface() {
        let out = quote_cpmm(10, 1000, 1000, 25).expect("valid inputs"); // 25 bps fee
        assert_eq!(out, 9);
    }

    #[test]
    fn test_optimal_two_pool_existence_invariant() {
        // Sub-fee spread: 1M / 1M vs 1M / 1.001M (0.10% spread vs 2 * 0.25% fees)
        // N <= D0 -> Must return None
        let l1 = Leg {
            r_in: 1_000_000,
            r_out: 1_000_000,
            gamma: 9975,
        };
        let l2 = Leg {
            r_in: 1_000_000,
            r_out: 1_001_000,
            gamma: 9975,
        };
        assert_eq!(optimal_two_pool(&l1, &l2), None);

        // Exact break-even spread: N <= D0 -> Must return None
        // D0 = 10^8 * 10^6 * 10^6 = 10^20
        // N = 9975 * 9975 * 10^6 * x2. For N <= D0: x2 <= 10^14 / 9975^2 = 1,005,018.8
        let l_even = Leg {
            r_in: 1_000_000,
            r_out: 1_005_018,
            gamma: 9975,
        };
        assert_eq!(optimal_two_pool(&l1, &l_even), None);

        // 10% losing trade -> Must return None
        let l_loss = Leg {
            r_in: 1_000_000,
            r_out: 900_000,
            gamma: 9975,
        };
        assert_eq!(optimal_two_pool(&l1, &l_loss), None);
    }

    #[test]
    fn test_optimal_two_pool_sizing_accuracy() {
        // 5% spread: 1M / 1M vs 1M / 1.05M with 25 bps fees
        let l1 = Leg {
            r_in: 1_000_000,
            r_out: 1_000_000,
            gamma: 9975,
        };
        let l2 = Leg {
            r_in: 1_000_000,
            r_out: 1_050_000,
            gamma: 9975,
        };

        let opt = optimal_two_pool(&l1, &l2);
        assert!(opt.is_some());
        let d_star = opt.expect("invariant");
        assert!(d_star > 0);

        // Verify d* is the profit peak:
        // Profit at d* must be strictly higher than at d* - 1000 and d* + 1000
        let p_peak = round_trip_profit_2pool(d_star, &l1, &l2);
        let p_lower = round_trip_profit_2pool(d_star.saturating_sub(1000), &l1, &l2);
        let p_higher = round_trip_profit_2pool(d_star.saturating_add(1000), &l1, &l2);

        assert!(
            p_peak >= p_lower,
            "Peak profit {p_peak} should be >= lower profit {p_lower}"
        );
        assert!(
            p_peak >= p_higher,
            "Peak profit {p_peak} should be >= higher profit {p_higher}"
        );
    }

    #[test]
    fn test_u384_overflow_protection_293_bits() {
        // 10^18 reserves (standard 9-decimal SPL token with 1B supply)
        // Radicand is ~10^88 (293 bits). Must NOT overflow or panic!
        let r18 = 1_000_000_000_000_000_000u128; // 10^18
        let r18_out = 1_050_000_000_000_000_000u128; // 1.05 * 10^18

        let l1 = Leg {
            r_in: r18,
            r_out: r18,
            gamma: 9975,
        };
        let l2 = Leg {
            r_in: r18,
            r_out: r18_out,
            gamma: 9975,
        };

        let opt = optimal_two_pool(&l1, &l2);
        assert!(opt.is_some());
        let d_star = opt.expect("invariant");
        assert!(d_star > 1_000_000_000_000_000); // Expect ~1.1 * 10^16 units
    }

    #[test]
    fn test_optimal_three_pool_sizing_and_overflow_441_bits() {
        // Triangular route: 10^18 reserves on all 3 legs
        // Radicand reaches 441 bits. U512 must NOT overflow or panic!
        let r18 = 1_000_000_000_000_000_000u128;
        let r18_high = 1_100_000_000_000_000_000u128; // 10% profit spread on leg 3

        let l1 = Leg {
            r_in: r18,
            r_out: r18,
            gamma: 9975,
        };
        let l2 = Leg {
            r_in: r18,
            r_out: r18,
            gamma: 9975,
        };
        let l3 = Leg {
            r_in: r18,
            r_out: r18_high,
            gamma: 9975,
        };

        let opt = optimal_three_pool(&l1, &l2, &l3);
        assert!(opt.is_some());
        let d3_star = opt.expect("invariant");
        assert!(d3_star > 0);
    }

    #[test]
    fn test_marginal_profit_degradation_ceiling() {
        let l1 = Leg {
            r_in: 1_000_000,
            r_out: 1_000_000,
            gamma: 9975,
        };
        let l2 = Leg {
            r_in: 1_000_000,
            r_out: 1_050_000,
            gamma: 9975,
        };
        let flash_fee_bps = 5; // 0.05%

        let d_star = optimal_two_pool(&l1, &l2).expect("invariant");

        // 1. Below ceiling: d* / 2 -> Not past ceiling, derivative > 0
        let small_size = d_star / 2;
        assert!(!is_past_marginal_degradation_ceiling_two_pool(
            small_size,
            &l1,
            &l2,
            flash_fee_bps
        ));
        let deriv_small = marginal_profit_derivative_cpmm(small_size, &l1, &l2, flash_fee_bps);
        assert!(
            deriv_small > 0,
            "Marginal return should be positive before peak: {deriv_small}"
        );

        // 2. Beyond ceiling: 2 * d* -> Past ceiling, derivative < 0
        let large_size = d_star * 2;
        assert!(is_past_marginal_degradation_ceiling_two_pool(
            large_size,
            &l1,
            &l2,
            flash_fee_bps
        ));
        let deriv_large = marginal_profit_derivative_cpmm(large_size, &l1, &l2, flash_fee_bps);
        assert!(
            deriv_large < 0,
            "Marginal return should be negative past peak: {deriv_large}"
        );

        // 3. Clamping function truncates 2 * d* back to d*
        let clamped = clamp_at_marginal_ceiling_two_pool(large_size, &l1, &l2, flash_fee_bps);
        assert_eq!(clamped, Some(d_star));
    }

    #[test]
    fn test_discrete_marginal_profit_derivative() {
        // Borrow 10,000, receive 10,500, flash fee 5 bps
        // Gross return = 5%, cost = 0.05% -> derivative is positive
        let d_pos = marginal_profit_derivative(10_000, 10_500, 5);
        assert!(d_pos > 0);

        // Borrow 10,000, receive 10,002, flash fee 5 bps
        // Gross return = 2 bps, cost = 5 bps -> derivative is negative
        let d_neg = marginal_profit_derivative(10_000, 10_002, 5);
        assert!(d_neg < 0);
    }

    fn round_trip_profit_2pool(dx: u64, l1: &Leg, l2: &Leg) -> i128 {
        let Some(out1) = cpmm_out(u128::from(dx), l1.r_in, l1.r_out, l1.gamma) else {
            return i128::MIN;
        };
        let Some(out2) = cpmm_out(out1, l2.r_in, l2.r_out, l2.gamma) else {
            return i128::MIN;
        };
        let out2_i128 = i128::try_from(out2).unwrap_or(i128::MAX);
        let dx_i128 = i128::from(dx);
        out2_i128 - dx_i128
    }

    #[test]
    fn test_cpmm_fee_paid_denominators() {
        // Raydium V4: D=10_000, 25 bps fee on 10_000 in -> 25
        assert_eq!(cpmm_fee_paid_with_denom(10_000, 25, 10_000), 25);
        // Raydium V4 ceiling rounding: 1 in with 25 bps fee -> ceil(25 / 10000) = 1
        assert_eq!(cpmm_fee_paid_with_denom(1, 25, 10_000), 1);
        // Raydium CPMM: D=1_000_000, 2500 PPM (0.25%) on 1_000_000 in -> 2500
        assert_eq!(cpmm_fee_paid_with_denom(1_000_000, 2_500, 1_000_000), 2_500);
        // Raydium CPMM ceiling rounding: 1 in with 2500 PPM -> ceil(2500 / 1e6) = 1
        assert_eq!(cpmm_fee_paid_with_denom(1, 2_500, 1_000_000), 1);
    }

    // -------------------------------------------------------------------------
    // Property-based tests (proptest)
    // -------------------------------------------------------------------------

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(2_000))]

        #[test]
        fn cpmm_out_never_exceeds_y(dx in any::<u128>(), x in any::<u128>(), y in any::<u128>()) {
            let out = cpmm_out(dx, x, y, 9_975);
            // Output can never exceed the output reserve, whatever the inputs.
            if let Some(out_val) = out {
                proptest::prop_assert!(out_val <= y);
            }
            // Degenerate pools / zero input must return None, never panic.
            if dx == 0 || x == 0 || y == 0 {
                proptest::prop_assert_eq!(out, None);
            }
        }

        #[test]
        fn optimal_two_pool_never_panics(
            r1in in 1u128..u128::MAX,
            r1out in 1u128..u128::MAX,
            r2in in 1u128..u128::MAX,
            r2out in 1u128..u128::MAX,
            gamma1 in 1u128..=10_000u128,
            gamma2 in 1u128..=10_000u128,
            dx in 0u64..u64::MAX,
        ) {
            let l1 = Leg { r_in: r1in, r_out: r1out, gamma: gamma1 };
            let l2 = Leg { r_in: r2in, r_out: r2out, gamma: gamma2 };
            // Sizing must be total: Some(_) or None, never a panic.
            let _ = optimal_two_pool(&l1, &l2);
            let _ = round_trip_profit_2pool(dx, &l1, &l2);
        }
    }
}
