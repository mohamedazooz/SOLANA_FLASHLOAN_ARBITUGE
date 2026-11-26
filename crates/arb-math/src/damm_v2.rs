//! Meteora DAMM v2 (`cp_amm`, `cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG`) Math.
//!
//! Models DAMM v2 pools which store Q64.64 `sqrt_price` and `liquidity` (with optional
//! concentrated range bounds `sqrt_min_price..=sqrt_max_price` or full-range CP-AMM),
//! plus base/dynamic fee schedules in `1e9` (`FEE_DENOMINATOR`) precision.

use ruint::aliases::U256;

/// Meteora DAMM v2 fee denominator (`10^9`).
pub const DAMM_V2_FEE_DENOMINATOR: u128 = 1_000_000_000;
/// Maximum capped fee numerator for legacy V0 pool layout (50%).
pub const DAMM_V2_MAX_FEE_NUMERATOR_V0: u128 = 500_000_000;
/// Maximum capped fee numerator for V1 pool layout (99%).
pub const DAMM_V2_MAX_FEE_NUMERATOR_V1: u128 = 990_000_000;
/// Backwards-compatible alias for legacy V0.
pub const DAMM_V2_MAX_FEE_NUMERATOR: u128 = DAMM_V2_MAX_FEE_NUMERATOR_V0;
const DAMM_V2_ONE_Q64: u128 = 1u128 << 64;
const DAMM_V2_BPS_DENOMINATOR: u128 = 10_000;
const DAMM_V2_DYNAMIC_FEE_DENOMINATOR: u128 = 100_000_000_000;

/// Official DAMM v2 BaseFeeMode values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DammV2BaseFeeMode {
    TimeLinear = 0,
    TimeExponential = 1,
    RateLimiter = 2,
    MarketCapLinear = 3,
    MarketCapExponential = 4,
}

/// Minimal decoded DAMM v2 fee configuration. Fields mirror the deployed
/// cp_amm PoolFeesStruct and scheduler parameter layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DammV2FeeParams {
    /// Base fee numerator in 1e9 units.
    pub cliff_fee_numerator: u64,
    /// Official BaseFeeMode discriminator (0..=4).
    pub fee_scheduler_mode: u8,
    pub number_of_period: u16,
    /// Time scheduler period frequency, in slots or seconds per activation_type.
    pub period_frequency: u64,
    /// Scheduler reduction factor, in basis points (1/10,000).
    pub reduction_factor: u64,
    /// Market-cap scheduler price step in basis points.
    pub sqrt_price_step_bps: u32,
    /// Market-cap scheduler expiry duration, in activation points.
    pub scheduler_expiration_duration: u32,
    /// Initial sqrt price captured when pool was initialized.
    pub init_sqrt_price_x64: u128,
    /// Pool fee version: 0 => 50% cap, 1 => 99% cap.
    pub fee_version: u8,
    pub protocol_fee_percent: u8,
    pub referral_fee_percent: u8,
    pub compounding_fee_bps: u16,
    /// Dynamic fee state as stored in PoolFeesStruct::dynamic_fee.
    pub dynamic_fee_initialized: bool,
    pub variable_fee_control: u32,
    pub max_volatility_accumulator: u32,
    pub bin_step: u16,
    pub volatility_accumulator: u128,
    /// Collection mode: 0 BothToken, 1 OnlyB, 2 Compounding.
    pub collect_fee_mode: u8,
}

/// Exact allocation of a single DAMM v2 trading-fee amount. Components are
/// mutually exclusive and sum to the trading fee; they must never be charged
/// again on top of the already fee-adjusted swap output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DammV2FeeBreakdown {
    pub lp_claiming_fee: u64,
    pub compounding_fee: u64,
    pub protocol_fee: u64,
    pub referral_fee: u64,
}
impl DammV2FeeParams {
    /// Match Meteora FeeMode::get_fee_mode for fee placement.
    /// AtoB means token A is input; BtoA means token B is input.
    pub fn fees_on_input(&self, a_to_b: bool) -> Option<bool> {
        match (self.collect_fee_mode, a_to_b) {
            (0, _) => Some(false),
            (1, true) | (2, true) => Some(false),
            (1, false) | (2, false) => Some(true),
            _ => None,
        }
    }
    /// Split the already-calculated trading fee like PoolFeesStruct::split_fees.
    pub fn split_trading_fee(
        &self,
        trading_fee: u64,
        has_referral: bool,
    ) -> Option<DammV2FeeBreakdown> {
        if self.protocol_fee_percent > 100
            || self.referral_fee_percent > 100
            || self.compounding_fee_bps > 10_000
        {
            return None;
        }
        let protocol_total = u128::from(trading_fee)
            .checked_mul(u128::from(self.protocol_fee_percent))?
            .checked_div(100)?;
        let protocol_total = u64::try_from(protocol_total).ok()?;
        let lp_fee = trading_fee.checked_sub(protocol_total)?;
        let compounding_fee = u128::from(lp_fee)
            .checked_mul(u128::from(self.compounding_fee_bps))?
            .checked_div(10_000)?;
        let compounding_fee = u64::try_from(compounding_fee).ok()?;
        let lp_claiming_fee = lp_fee.checked_sub(compounding_fee)?;
        let referral_fee = if has_referral {
            u64::try_from(
                u128::from(protocol_total)
                    .checked_mul(u128::from(self.referral_fee_percent))?
                    .checked_div(100)?,
            )
            .ok()?
        } else {
            0
        };
        let protocol_fee = protocol_total.checked_sub(referral_fee)?;
        let breakdown = DammV2FeeBreakdown {
            lp_claiming_fee,
            compounding_fee,
            protocol_fee,
            referral_fee,
        };
        let total = breakdown
            .lp_claiming_fee
            .checked_add(breakdown.compounding_fee)?
            .checked_add(breakdown.protocol_fee)?
            .checked_add(breakdown.referral_fee)?;
        (total == trading_fee).then_some(breakdown)
    }
    /// Compute the exact Q64.64 exponential factor used by Meteora.
    /// The exponentiation-by-squaring order and floor shifts mirror
    /// programs/cp-amm/src/math/fee_math.rs::get_fee_in_period.
    pub fn exponential_fee_numerator(cliff: u64, reduction_bps: u64, period: u16) -> Option<u64> {
        if reduction_bps > 10_000 {
            return None;
        }
        if reduction_bps == 0 || period == 0 {
            return Some(cliff);
        }
        let reduction_q64 = u128::from(reduction_bps)
            .checked_shl(64)?
            .checked_div(DAMM_V2_BPS_DENOMINATOR)?;
        let mut factor = DAMM_V2_ONE_Q64.checked_sub(reduction_q64)?;
        let mut result = DAMM_V2_ONE_Q64;
        let mut exponent = u32::from(period);
        while exponent > 0 {
            if exponent & 1 == 1 {
                result = result.checked_mul(factor)? >> 64;
            }
            exponent >>= 1;
            if exponent > 0 {
                factor = factor.checked_mul(factor)? >> 64;
            }
        }
        if result == 0 {
            return None;
        }
        u64::try_from(result.checked_mul(u128::from(cliff))? >> 64).ok()
    }

    pub fn max_fee_numerator(&self) -> Option<u128> {
        match self.fee_version {
            0 => Some(DAMM_V2_MAX_FEE_NUMERATOR_V0),
            1 => Some(DAMM_V2_MAX_FEE_NUMERATOR_V1),
            _ => None,
        }
    }

    fn base_fee_numerator(
        &self,
        current_point: u64,
        activation_point: u64,
        current_sqrt_price_x64: u128,
    ) -> Option<u128> {
        let max_period = u64::from(self.number_of_period);
        let period = match self.fee_scheduler_mode {
            0 | 1 => {
                if self.period_frequency == 0 {
                    return Some(u128::from(self.cliff_fee_numerator));
                }
                // Meteora charges the terminal/minimum scheduled fee before
                // activation (alpha-vault pre-activation path).
                if current_point < activation_point {
                    max_period
                } else {
                    current_point
                        .saturating_sub(activation_point)
                        .checked_div(self.period_frequency)?
                        .min(max_period)
                }
            }
            2 => return Some(u128::from(self.cliff_fee_numerator)),
            3 | 4 => {
                let expiration =
                    activation_point.checked_add(u64::from(self.scheduler_expiration_duration))?;
                if current_point < activation_point || current_point > expiration {
                    max_period
                } else if current_sqrt_price_x64 <= self.init_sqrt_price_x64
                    || self.sqrt_price_step_bps == 0
                {
                    0
                } else {
                    let delta = U256::from(current_sqrt_price_x64 - self.init_sqrt_price_x64);
                    let numerator = delta.checked_mul(U256::from(10_000u64))?;
                    let denominator = U256::from(self.init_sqrt_price_x64)
                        .checked_mul(U256::from(self.sqrt_price_step_bps))?;
                    if denominator.is_zero() {
                        return None;
                    }
                    let passed = numerator / denominator;
                    u64::try_from(passed).ok()?.min(max_period)
                }
            }
            _ => return None,
        };

        let period = u16::try_from(period).ok()?;
        let fee = match self.fee_scheduler_mode {
            0 | 3 => u128::from(self.cliff_fee_numerator)
                .checked_sub(u128::from(self.reduction_factor).checked_mul(u128::from(period))?)?,
            1 | 4 => u128::from(Self::exponential_fee_numerator(
                self.cliff_fee_numerator,
                self.reduction_factor,
                period,
            )?),
            _ => return None,
        };
        Some(fee)
    }

    /// Minimum base fee after the full configured schedule has elapsed.
    pub fn min_base_fee_numerator(&self) -> Option<u64> {
        let periods = self.number_of_period;
        match self.fee_scheduler_mode {
            0 | 3 => u64::try_from(u128::from(self.cliff_fee_numerator).checked_sub(
                u128::from(self.reduction_factor).checked_mul(u128::from(periods))?,
            )?)
            .ok(),
            1 | 4 => Self::exponential_fee_numerator(
                self.cliff_fee_numerator,
                self.reduction_factor,
                periods,
            ),
            2 => Some(self.cliff_fee_numerator),
            _ => None,
        }
    }
    /// Exact total trading fee numerator (base + dynamic, capped by pool version).
    pub fn effective_fee_numerator_at(
        &self,
        current_point: u64,
        activation_point: u64,
        current_sqrt_price_x64: u128,
    ) -> Option<u128> {
        let base =
            self.base_fee_numerator(current_point, activation_point, current_sqrt_price_x64)?;
        let dynamic = if self.dynamic_fee_initialized {
            let vfa_bin = self
                .volatility_accumulator
                .checked_mul(u128::from(self.bin_step))?;
            let square = vfa_bin.checked_mul(vfa_bin)?;
            square
                .checked_mul(u128::from(self.variable_fee_control))?
                .checked_add(DAMM_V2_DYNAMIC_FEE_DENOMINATOR - 1)?
                .checked_div(DAMM_V2_DYNAMIC_FEE_DENOMINATOR)?
        } else {
            0
        };
        // The on-chain program casts the sum to u64 before applying the cap.
        // Preserve that failure mode rather than silently capping an overflow.
        let total = u64::try_from(base.checked_add(dynamic)?).ok()?;
        Some(u128::from(total).min(self.max_fee_numerator()?))
    }

    /// Compatibility helper for time-scheduler unit tests and callers that
    /// only possess a time point. Returns None for non-time scheduler modes.
    pub fn effective_fee_numerator(
        &self,
        current_point: u64,
        activation_point: u64,
    ) -> Option<u128> {
        if self.fee_scheduler_mode > 2 {
            return None;
        }
        self.effective_fee_numerator_at(current_point, activation_point, self.init_sqrt_price_x64)
    }

    /// Convert fee numerator to conservative bps, rounding upward.
    pub fn fee_bps_at(
        &self,
        current_point: u64,
        activation_point: u64,
        current_sqrt_price_x64: u128,
    ) -> Option<u16> {
        let num = self.effective_fee_numerator_at(
            current_point,
            activation_point,
            current_sqrt_price_x64,
        )?;
        let bps = num.checked_mul(10_000)?.div_ceil(DAMM_V2_FEE_DENOMINATOR);
        u16::try_from(bps).ok()
    }
}
/// Quote an exact-in swap on a Meteora DAMM v2 pool using virtual reserves
/// derived from `(sqrt_price_x64, liquidity)` bounded by `[sqrt_min_price, sqrt_max_price]`.
pub fn quote_damm_v2_exact_in(
    amount_in: u64,
    a_to_b: bool,
    sqrt_price_x64: u128,
    liquidity: u128,
    sqrt_min_price_x64: u128,
    sqrt_max_price_x64: u128,
    fee_numerator_1e9: u128,
) -> Option<u64> {
    quote_damm_v2_exact_in_with_fee_mode(
        amount_in,
        a_to_b,
        sqrt_price_x64,
        liquidity,
        sqrt_min_price_x64,
        sqrt_max_price_x64,
        fee_numerator_1e9,
        false, // default to BothToken mode: fee deducted from output
    )
}

/// Quote an exact-in swap with explicit fee collection mode:
/// - `fees_on_input = false` (standard for BothToken mode): fees deducted from output.
/// - `fees_on_input = true` (OnlyB/Compounding when selling token B): fees deducted from input.
pub fn quote_damm_v2_exact_in_with_fee_mode(
    amount_in: u64,
    a_to_b: bool,
    sqrt_price_x64: u128,
    liquidity: u128,
    sqrt_min_price_x64: u128,
    sqrt_max_price_x64: u128,
    fee_numerator_1e9: u128,
    fees_on_input: bool,
) -> Option<u64> {
    if amount_in == 0 || sqrt_price_x64 == 0 || liquidity == 0 {
        return Some(0);
    }
    if fee_numerator_1e9 >= DAMM_V2_FEE_DENOMINATOR {
        return None;
    }

    let dx_u = U256::from(amount_in);
    let net_in = if fees_on_input {
        let fee_u = (dx_u * U256::from(fee_numerator_1e9)
            + U256::from(DAMM_V2_FEE_DENOMINATOR - 1))
            / U256::from(DAMM_V2_FEE_DENOMINATOR);
        if fee_u >= dx_u {
            return Some(0);
        }
        dx_u - fee_u
    } else {
        dx_u
    };

    let l_u = U256::from(liquidity);
    let sp_u = U256::from(sqrt_price_x64);
    let q64 = U256::from(1u128 << 64);

    let gross_out = if a_to_b {
        // Selling token A reduces sqrt_price: sp_next = (L * sp * Q64) / (L * Q64 + net_in * sp)
        let l_q64 = l_u << 64usize;
        let denom = l_q64.checked_add(net_in.checked_mul(sp_u)?)?;
        if denom.is_zero() {
            return None;
        }
        let sp_next = (l_q64.checked_mul(sp_u)?) / denom;
        let min_sp = if sqrt_min_price_x64 > 0 {
            U256::from(sqrt_min_price_x64)
        } else {
            U256::from(1u64)
        };
        let clamped_next = sp_next.max(min_sp);
        if clamped_next >= sp_u {
            return Some(0);
        }
        let delta_sp = sp_u - clamped_next;
        let out = (l_u * delta_sp) / q64;
        u64::try_from(out).ok()?
    } else {
        // Selling token B increases sqrt_price: sp_next = sp + (net_in * Q64) / L
        let delta_sp = (net_in << 64usize) / l_u;
        let sp_next = sp_u.checked_add(delta_sp)?;
        let clamped_next = if sqrt_max_price_x64 > 0 {
            sp_next.min(U256::from(sqrt_max_price_x64))
        } else {
            sp_next
        };
        if clamped_next <= sp_u {
            return Some(0);
        }
        // out_a = L * (sp_next - sp) * Q64 / (sp * sp_next)
        let num = l_u * (clamped_next - sp_u);
        let num_shifted = num << 64usize;
        let denom = sp_u.checked_mul(clamped_next)?;
        if denom.is_zero() {
            return None;
        }
        let out = num_shifted / denom;
        u64::try_from(out).ok()?
    };

    if !fees_on_input {
        let gross_out_u = U256::from(gross_out);
        let fee_u = (gross_out_u * U256::from(fee_numerator_1e9)
            + U256::from(DAMM_V2_FEE_DENOMINATOR - 1))
            / U256::from(DAMM_V2_FEE_DENOMINATOR);
        let net_out = gross_out_u.saturating_sub(fee_u);
        u64::try_from(net_out).ok()
    } else {
        Some(gross_out)
    }
}

/// Derive virtual CPMM-equivalent reserves `(reserve_a, reserve_b)` from `(sqrt_price_x64, liquidity)`.
pub fn damm_v2_virtual_reserves(sqrt_price_x64: u128, liquidity: u128) -> (u128, u128) {
    if sqrt_price_x64 == 0 || liquidity == 0 {
        return (0, 0);
    }
    let l = U256::from(liquidity);
    let sp = U256::from(sqrt_price_x64);
    let r_a = (l << 64usize) / sp;
    let r_b = (l * sp) >> 64usize;
    (
        u128::try_from(r_a).unwrap_or(u128::MAX),
        u128::try_from(r_b).unwrap_or(u128::MAX),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fee_bps_from_cliff_numerator() {
        // 10_000_000 / 1e9 = 1% = 100 bps.
        let f = DammV2FeeParams {
            cliff_fee_numerator: 10_000_000,
            ..Default::default()
        };
        assert_eq!(f.fee_bps_at(0, 0, 0), Some(100));
    }

    #[test]
    fn fee_bps_rounds_up_to_never_understate() {
        // 1 lamport of 1e9 fee = 0.0000001% -> ceil to at least 1 bps? No:
        // 1 * 10_000 / 1e9 ceil = 1 (since div_ceil of 0.00001 -> 1).
        let f = DammV2FeeParams {
            cliff_fee_numerator: 1,
            ..Default::default()
        };
        assert_eq!(f.fee_bps_at(0, 0, 0), Some(1));
    }

    #[test]
    fn effective_fee_is_capped_at_fifty_percent() {
        let f = DammV2FeeParams {
            cliff_fee_numerator: u64::MAX,
            dynamic_fee_initialized: true,
            variable_fee_control: u32::MAX,
            volatility_accumulator: u128::MAX,
            ..Default::default()
        };
        assert_eq!(
            f.effective_fee_numerator(0, 0),
            Some(DAMM_V2_MAX_FEE_NUMERATOR)
        );
        assert_eq!(f.fee_bps_at(0, 0, 0), Some(5_000));
    }

    #[test]
    fn linear_scheduler_reduces_fee_by_periods_passed() {
        // cliff 1%, reduction 100_000/1e9 per period, 2 periods passed.
        let f = DammV2FeeParams {
            cliff_fee_numerator: 10_000_000,
            fee_scheduler_mode: 0,
            number_of_period: 5,
            period_frequency: 10,
            reduction_factor: 100_000,
            ..Default::default()
        };
        // 0 elapsed -> no reduction.
        assert_eq!(f.effective_fee_numerator(0, 0), Some(10_000_000));
        // 25 elapsed -> 2 full periods -> 10_000_000 - 200_000.
        assert_eq!(f.effective_fee_numerator(25, 0), Some(9_800_000));
        // Elapsed beyond all periods clamps at number_of_period.
        assert_eq!(f.effective_fee_numerator(1_000, 0), Some(9_500_000));
    }

    #[test]
    fn quote_rejects_fee_at_or_above_denominator() {
        assert!(
            quote_damm_v2_exact_in(1_000, true, 1 << 64, 1_000, 0, 0, DAMM_V2_FEE_DENOMINATOR)
                .is_none()
        );
        assert!(quote_damm_v2_exact_in(1_000, true, 1 << 64, 1_000, 0, 0, u128::MAX).is_none());
    }

    #[test]
    fn quote_zero_inputs_or_liquidity_yields_zero() {
        assert_eq!(
            quote_damm_v2_exact_in(0, true, 1 << 64, 1_000, 0, 0, 0),
            Some(0)
        );
        assert_eq!(
            quote_damm_v2_exact_in(1_000, true, 0, 1_000, 0, 0, 0),
            Some(0)
        );
        assert_eq!(
            quote_damm_v2_exact_in(1_000, true, 1 << 64, 0, 0, 0, 0),
            Some(0)
        );
    }

    #[test]
    fn quote_both_directions_agree_with_virtual_reserves_for_small_trades() {
        // Full-range pool: sqrt_price = 1.0 (Q64.64), L = 1e12.
        let (sp, l) = (1u128 << 64, 1_000_000_000_000u128);
        let (r_a, r_b) = damm_v2_virtual_reserves(sp, l);
        // r_a == r_b == L at sqrt_price 1.0.
        assert_eq!(r_a, l);
        assert_eq!(r_b, l);

        let dx = 1_000u64;
        let out_a_to_b = quote_damm_v2_exact_in(dx, true, sp, l, 0, 0, 0).expect("quote");
        let out_b_to_a = quote_damm_v2_exact_in(dx, false, sp, l, 0, 0, 0).expect("quote");
        // Symmetric reserves -> symmetric quotes (constant-product invariant,
        // small rounding drift from the sqrt formulation is <= 1 unit).
        assert!(
            out_a_to_b.abs_diff(out_b_to_a) <= 1,
            "a_to_b={out_a_to_b} b_to_a={out_b_to_a}"
        );
        // Constant-product sanity: dx * (r_b - out) approx out * (r_a + dx).
        assert!(out_a_to_b > 0 && out_a_to_b < dx);
    }

    #[test]
    fn price_impact_is_monotone_in_amount() {
        // Amounts far above the 1-unit truncation floor so the monotone
        // price-impact signal is not drowned by integer rounding.
        let (sp, l) = (1u128 << 64, 1_000_000_000_000_000u128);
        let small = quote_damm_v2_exact_in(1_000_000_000, true, sp, l, 0, 0, 0).unwrap();
        let large = quote_damm_v2_exact_in(100_000_000_000, true, sp, l, 0, 0, 0).unwrap();
        let rate_small = small as f64 / 1_000_000_000.0;
        let rate_large = large as f64 / 100_000_000_000.0;
        assert!(
            rate_large < rate_small,
            "larger trade must get a worse rate"
        );
    }

    #[test]
    fn range_clamp_limits_max_output() {
        // Bounded range: selling B cannot push sqrt_price past sqrt_max.
        let (sp, l) = (1u128 << 64, 1_000_000_000_000u128);
        let sqrt_max = (1u128 << 64) + (1u128 << 32);
        let unbounded = quote_damm_v2_exact_in(u64::MAX, false, sp, l, 0, 0, 0).unwrap();
        let bounded = quote_damm_v2_exact_in(u64::MAX, false, sp, l, 0, sqrt_max, 0).unwrap();
        assert!(bounded < unbounded, "range cap must reduce max output");
        assert!(bounded > 0);
    }

    #[test]
    fn fees_on_input_and_output_are_close_for_small_fees() {
        let (sp, l) = (1u128 << 64, 1_000_000_000_000u128);
        let dx = 1_000_000u64;
        let fee = 5_000_000u128; // 0.5%
        let on_output =
            quote_damm_v2_exact_in_with_fee_mode(dx, false, sp, l, 0, 0, fee, false).unwrap();
        let on_input =
            quote_damm_v2_exact_in_with_fee_mode(dx, false, sp, l, 0, 0, fee, true).unwrap();
        // Same fee rate applied at either end differs only by second-order
        // price-impact on the (smaller) effective input.
        assert!(on_input <= on_output);
        assert!(on_output - on_input <= on_output / 1_000 + 1);
    }

    #[test]
    fn exponential_scheduler_matches_q64_period_decay() {
        assert_eq!(
            DammV2FeeParams::exponential_fee_numerator(10_000_000, 5_000, 0),
            Some(10_000_000)
        );
        assert_eq!(
            DammV2FeeParams::exponential_fee_numerator(10_000_000, 5_000, 1),
            Some(5_000_000)
        );
        assert_eq!(
            DammV2FeeParams::exponential_fee_numerator(10_000_000, 5_000, 2),
            Some(2_500_000)
        );
        assert_eq!(
            DammV2FeeParams::exponential_fee_numerator(10_000_000, 10_001, 1),
            None
        );
    }

    #[test]
    fn time_scheduler_uses_terminal_fee_before_activation() {
        let f = DammV2FeeParams {
            cliff_fee_numerator: 10_000_000,
            fee_scheduler_mode: 1,
            number_of_period: 2,
            period_frequency: 10,
            reduction_factor: 5_000,
            ..Default::default()
        };
        assert_eq!(f.effective_fee_numerator(99, 100), Some(2_500_000));
        assert_eq!(f.effective_fee_numerator(100, 100), Some(10_000_000));
        assert_eq!(f.effective_fee_numerator(120, 100), Some(2_500_000));
    }

    #[test]
    fn market_cap_exponential_scheduler_uses_sqrt_price_steps_and_expiry() {
        let initial = 1u128 << 64;
        let f = DammV2FeeParams {
            cliff_fee_numerator: 10_000_000,
            fee_scheduler_mode: 4,
            number_of_period: 2,
            sqrt_price_step_bps: 900,
            scheduler_expiration_duration: 100,
            reduction_factor: 5_000,
            init_sqrt_price_x64: initial,
            ..Default::default()
        };
        assert_eq!(
            f.effective_fee_numerator_at(50, 0, initial),
            Some(10_000_000)
        );
        assert_eq!(
            f.effective_fee_numerator_at(50, 0, initial + initial / 10),
            Some(5_000_000)
        );
        assert_eq!(
            f.effective_fee_numerator_at(101, 0, initial),
            Some(2_500_000)
        );
    }

    #[test]
    fn dynamic_fee_matches_official_ceil_formula() {
        let f = DammV2FeeParams {
            cliff_fee_numerator: 10_000_000,
            dynamic_fee_initialized: true,
            variable_fee_control: 1_000_000,
            bin_step: 10,
            volatility_accumulator: 100,
            ..Default::default()
        };
        assert_eq!(f.effective_fee_numerator(0, 0), Some(10_000_010));
    }

    #[test]
    fn pool_fee_version_controls_total_fee_cap() {
        let v0 = DammV2FeeParams {
            cliff_fee_numerator: 990_000_000,
            fee_version: 0,
            ..Default::default()
        };
        let v1 = DammV2FeeParams {
            fee_version: 1,
            ..v0
        };
        assert_eq!(v0.effective_fee_numerator(0, 0), Some(500_000_000));
        assert_eq!(v1.effective_fee_numerator(0, 0), Some(990_000_000));
    }

    #[test]
    fn unknown_base_fee_mode_fails_closed() {
        let f = DammV2FeeParams {
            fee_scheduler_mode: 5,
            ..Default::default()
        };
        assert_eq!(f.effective_fee_numerator(0, 0), None);
    }

    #[test]
    fn fee_split_allocations_sum_to_the_single_trading_fee() {
        let params = DammV2FeeParams {
            protocol_fee_percent: 20,
            referral_fee_percent: 25,
            compounding_fee_bps: 5_000,
            ..Default::default()
        };
        let split = params.split_trading_fee(1_000, true).unwrap();
        assert_eq!(split.protocol_fee, 150);
        assert_eq!(split.referral_fee, 50);
        assert_eq!(split.compounding_fee, 400);
        assert_eq!(split.lp_claiming_fee, 400);
        assert_eq!(
            split.protocol_fee + split.referral_fee + split.compounding_fee + split.lp_claiming_fee,
            1_000
        );
        let no_referral = params.split_trading_fee(1_000, false).unwrap();
        assert_eq!(no_referral.protocol_fee, 200);
        assert_eq!(no_referral.referral_fee, 0);
    }

    #[test]
    fn collect_fee_mode_matches_official_directional_fee_placement() {
        let both = DammV2FeeParams {
            collect_fee_mode: 0,
            ..Default::default()
        };
        let only_b = DammV2FeeParams {
            collect_fee_mode: 1,
            ..Default::default()
        };
        let compounding = DammV2FeeParams {
            collect_fee_mode: 2,
            ..Default::default()
        };
        assert_eq!(both.fees_on_input(true), Some(false));
        assert_eq!(both.fees_on_input(false), Some(false));
        assert_eq!(only_b.fees_on_input(true), Some(false));
        assert_eq!(only_b.fees_on_input(false), Some(true));
        assert_eq!(compounding.fees_on_input(true), Some(false));
        assert_eq!(compounding.fees_on_input(false), Some(true));
        let invalid = DammV2FeeParams {
            collect_fee_mode: 3,
            ..Default::default()
        };
        assert_eq!(invalid.fees_on_input(true), None);
    }

    #[test]
    fn unknown_pool_fee_version_fails_closed() {
        let f = DammV2FeeParams {
            fee_version: 2,
            ..Default::default()
        };
        assert_eq!(f.effective_fee_numerator(0, 0), None);
    }
}
