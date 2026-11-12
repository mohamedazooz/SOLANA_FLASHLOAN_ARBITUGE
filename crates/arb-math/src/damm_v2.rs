//! Meteora DAMM v2 (`cp_amm`, `cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG`) Math.
//!
//! Models DAMM v2 pools which store Q64.64 `sqrt_price` and `liquidity` (with optional
//! concentrated range bounds `sqrt_min_price..=sqrt_max_price` or full-range CP-AMM),
//! plus base/dynamic fee schedules in `1e9` (`FEE_DENOMINATOR`) precision.

use ruint::aliases::U256;

/// Meteora DAMM v2 fee denominator (`10^9`).
pub const DAMM_V2_FEE_DENOMINATOR: u128 = 1_000_000_000;
/// Maximum capped fee numerator (50% = 5e8).
pub const DAMM_V2_MAX_FEE_NUMERATOR: u128 = 500_000_000;

/// Minimal decoded DAMM v2 fee configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DammV2FeeParams {
    /// Base fee numerator in `1e9` units.
    pub cliff_fee_numerator: u64,
    /// Fee scheduler mode (0 = linear, 1 = exponential).
    pub fee_scheduler_mode: u8,
    pub number_of_period: u16,
    pub period_frequency: u64,
    pub reduction_factor: u64,
    /// Dynamic fee enabled flag & volatility components.
    pub dynamic_fee_initialized: bool,
    pub variable_fee_control: u32,
    pub volatility_accumulator: u128,
    /// Collection mode: 0 = both tokens (input), 1 = token B only, 2 = token A only.
    pub collect_fee_mode: u8,
}

impl DammV2FeeParams {
    /// Compute effective swap fee numerator in `1e9` precision.
    pub fn effective_fee_numerator(&self, current_point: u64, activation_point: u64) -> u128 {
        let mut base = u128::from(self.cliff_fee_numerator);
        if self.number_of_period > 0
            && self.period_frequency > 0
            && current_point >= activation_point
        {
            let elapsed = current_point.saturating_sub(activation_point);
            let passed =
                u128::from((elapsed / self.period_frequency).min(u64::from(self.number_of_period)));
            if self.fee_scheduler_mode == 0 {
                // Linear reduction
                base =
                    base.saturating_sub(passed.saturating_mul(u128::from(self.reduction_factor)));
            } else {
                // Exponential decay: base * (1 - r / 10_000)^passed
                let keep = 10_000u128.saturating_sub(u128::from(self.reduction_factor.min(10_000)));
                for _ in 0..passed.min(64) {
                    base = base.saturating_mul(keep) / 10_000;
                }
            }
        }
        let variable = if self.dynamic_fee_initialized && self.variable_fee_control > 0 {
            let v = self.volatility_accumulator;
            let v2 = v.saturating_mul(v);
            v2.saturating_mul(u128::from(self.variable_fee_control)) / 100_000_000_000
        } else {
            0
        };
        base.saturating_add(variable).min(DAMM_V2_MAX_FEE_NUMERATOR)
    }

    /// Convert effective fee into conservative basis points.
    pub fn fee_bps(&self, current_point: u64, activation_point: u64) -> u16 {
        let num = self.effective_fee_numerator(current_point, activation_point);
        let bps = num.saturating_mul(10_000).div_ceil(DAMM_V2_FEE_DENOMINATOR);
        u16::try_from(bps).unwrap_or(10_000)
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
