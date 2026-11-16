//! Meteora Dynamic Bonding Curve (DBC) Mathematical Model.
//!
//! Reimplemented from official Meteora DBC specifications (`dynamic_bonding_curve`).
//!
//! Architectural Invariants:
//! - Concentrated liquidity piecewise curve: Delta x = L * (1/sqrt_p_lower - 1/sqrt_p_upper)
//!   and Delta y = L * (sqrt_p_upper - sqrt_p_lower).
//! - Deterministic integer arithmetic using Q64.64 fixed-point math and wide integers (`U256`).
//! - Fee precision in 1e9 (`FEE_DENOMINATOR = 1_000_000_000`). Max fee cap 99%.
//! - Graduation threshold detection for automated migration arbitrage.

use ruint::aliases::U256;

/// DBC fee denominator (1e9).
pub const DBC_FEE_DENOMINATOR: u128 = 1_000_000_000;
/// Maximum fee rate enforced on-chain (99% = 990,000,000 in 1e9 units).
pub const DBC_MAX_FEE_NUMERATOR: u128 = 990_000_000;

/// 1.0 in Q64.64 fixed point ($2^{64}$).
pub const ONE_Q64: u128 = 1u128 << 64;

/// A single segment in the 16-point piecewise bonding curve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DbcCurveSegment {
    pub sqrt_price_lower_x64: u128,
    pub sqrt_price_upper_x64: u128,
    pub liquidity: u128,
}

impl DbcCurveSegment {
    pub fn new(lower_x64: u128, upper_x64: u128, liquidity: u128) -> Self {
        Self {
            sqrt_price_lower_x64: lower_x64,
            sqrt_price_upper_x64: upper_x64,
            liquidity,
        }
    }

    /// Calculate quote token needed (Delta y) to traverse between two sqrt prices.
    pub fn quote_amount_delta(&self, lower_x64: u128, upper_x64: u128) -> Option<u64> {
        if lower_x64 >= upper_x64 || self.liquidity == 0 {
            return Some(0);
        }
        let delta_sqrt_p = upper_x64.checked_sub(lower_x64)?;
        let num = U256::from(self.liquidity) * U256::from(delta_sqrt_p);
        let res = num >> 64;
        u64::try_from(res).ok()
    }

    /// Calculate base token sold (Delta x) to traverse between two sqrt prices.
    pub fn base_amount_delta(&self, lower_x64: u128, upper_x64: u128) -> Option<u64> {
        if lower_x64 >= upper_x64 || self.liquidity == 0 {
            return Some(0);
        }
        let num = (U256::from(self.liquidity) << 64) * U256::from(upper_x64 - lower_x64);
        let denom = U256::from(lower_x64) * U256::from(upper_x64);
        if denom.is_zero() {
            return None;
        }
        let res = num / denom;
        u64::try_from(res).ok()
    }
}

/// Dynamic Bonding Curve state and multi-segment quote engine.
#[derive(Debug, Clone, Default)]
pub struct DbcCurve {
    pub current_sqrt_price_x64: u128,
    pub segments: Vec<DbcCurveSegment>,
    pub fee_numerator: u64,
    pub migration_quote_threshold: u64,
    pub quote_reserve: u64,
    pub base_reserve: u64,
    pub migrated: bool,
}

impl DbcCurve {
    /// Check whether the bonding curve has reached its migration threshold.
    pub fn is_migration_ready(&self) -> bool {
        self.migration_quote_threshold > 0 && self.quote_reserve >= self.migration_quote_threshold
    }

    /// Compute swap fee for an input amount.
    pub fn calculate_fee(&self, amount_in: u64) -> u64 {
        if self.fee_numerator == 0 || amount_in == 0 {
            return 0;
        }
        let fee_rate = u128::from(self.fee_numerator).min(DBC_MAX_FEE_NUMERATOR);
        let num = U256::from(amount_in) * U256::from(fee_rate);
        let denom = U256::from(DBC_FEE_DENOMINATOR);
        let fee = (num + denom - U256::from(1)) / denom; // ceil div
        u64::try_from(fee).unwrap_or(amount_in)
    }

    /// Quote an exact-in swap through the bonding curve.
    ///
    /// `a_to_b`:
    /// - true: buying base with quote (spending quote token, receiving base token; price increases).
    /// - false: selling base for quote (spending base token, receiving quote token; price decreases).
    pub fn quote_exact_in(&self, amount_in: u64, a_to_b: bool) -> Option<(u64, u64)> {
        if amount_in == 0 {
            return None;
        }
        let fee_paid = self.calculate_fee(amount_in);
        let net_in = amount_in.checked_sub(fee_paid)?;
        if net_in == 0 {
            return Some((0, fee_paid));
        }

        if a_to_b {
            // Spending quote, receiving base (price goes up)
            let mut remaining_quote = net_in;
            let mut total_base_out = 0u64;
            let mut current_p = self.current_sqrt_price_x64;

            for seg in &self.segments {
                if current_p >= seg.sqrt_price_upper_x64 {
                    continue;
                }
                let seg_lower = current_p.max(seg.sqrt_price_lower_x64);
                let seg_upper = seg.sqrt_price_upper_x64;
                let max_quote_seg = seg.quote_amount_delta(seg_lower, seg_upper)?;

                if remaining_quote >= max_quote_seg {
                    let base_seg = seg.base_amount_delta(seg_lower, seg_upper)?;
                    total_base_out = total_base_out.checked_add(base_seg)?;
                    remaining_quote -= max_quote_seg;
                    current_p = seg_upper;
                } else {
                    // Partial traversal within segment
                    let delta_p = (U256::from(remaining_quote) << 64) / U256::from(seg.liquidity);
                    let target_p = u128::try_from(U256::from(seg_lower) + delta_p).ok()?;
                    let base_seg = seg.base_amount_delta(seg_lower, target_p)?;
                    total_base_out = total_base_out.checked_add(base_seg)?;
                    remaining_quote = 0;
                    break;
                }
                if remaining_quote == 0 {
                    break;
                }
            }

            if remaining_quote > 0 && total_base_out == 0 {
                return None;
            }
            Some((total_base_out, fee_paid))
        } else {
            // Selling base, receiving quote (price goes down)
            let mut remaining_base = net_in;
            let mut total_quote_out = 0u64;
            let mut current_p = self.current_sqrt_price_x64;

            for seg in self.segments.iter().rev() {
                if current_p <= seg.sqrt_price_lower_x64 {
                    continue;
                }
                let seg_upper = current_p.min(seg.sqrt_price_upper_x64);
                let seg_lower = seg.sqrt_price_lower_x64;
                let max_base_seg = seg.base_amount_delta(seg_lower, seg_upper)?;

                if remaining_base >= max_base_seg {
                    let quote_seg = seg.quote_amount_delta(seg_lower, seg_upper)?;
                    total_quote_out = total_quote_out.checked_add(quote_seg)?;
                    remaining_base -= max_base_seg;
                    current_p = seg_lower;
                } else {
                    // Partial step downwards
                    let num = U256::from(seg.liquidity) << 64;
                    let denom: U256 = (num / U256::from(seg_upper)) + U256::from(remaining_base);
                    if denom.is_zero() {
                        return None;
                    }
                    let target_p = u128::try_from(num / denom).ok()?;
                    let quote_seg = seg.quote_amount_delta(target_p, seg_upper)?;
                    total_quote_out = total_quote_out.checked_add(quote_seg)?;
                    remaining_base = 0;
                    break;
                }
                if remaining_base == 0 {
                    break;
                }
            }

            if remaining_base > 0 && total_quote_out == 0 {
                return None;
            }
            Some((total_quote_out, fee_paid))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dbc_single_segment_quote_exact_in() {
        // sqrt price from 1.0 to 2.0 in Q64.64
        let p_low = ONE_Q64;
        let p_high = ONE_Q64 * 2;
        let liquidity = 1_000_000_000u128; // 1e9 liquidity
        let seg = DbcCurveSegment::new(p_low, p_high, liquidity);

        let curve = DbcCurve {
            current_sqrt_price_x64: p_low,
            segments: vec![seg],
            fee_numerator: 25_000_000, // 2.5%
            migration_quote_threshold: 500_000_000,
            quote_reserve: 100_000_000,
            base_reserve: 1_000_000_000,
            migrated: false,
        };

        let (out, fee) = curve.quote_exact_in(10_000_000, true).expect("quote works");
        assert!(out > 0);
        assert_eq!(fee, 250_000); // 2.5% of 10M
        assert!(!curve.is_migration_ready());
    }

    #[test]
    fn test_dbc_migration_threshold_detection() {
        let curve = DbcCurve {
            current_sqrt_price_x64: ONE_Q64,
            segments: vec![],
            fee_numerator: 0,
            migration_quote_threshold: 100_000_000,
            quote_reserve: 100_000_000,
            base_reserve: 500_000_000,
            migrated: false,
        };
        assert!(curve.is_migration_ready());
    }
}
