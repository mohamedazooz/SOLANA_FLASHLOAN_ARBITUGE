//! Raydium CLMM dynamic-fee math, mirrored from the deployed program source
//! (`raydium-clmm/programs/amm/src/states/pool_fee.rs` and the
//! `compute_dynamic_fee_rate` / `get_total_fee_rate` helpers in
//! `instructions/swap.rs`), checked 2026-11-20.
//!
//! Constants are VERIFIED against the official crate:
//! - `MAX_FEE_RATE_NUMERATOR = 100_000`
//! - `VOLATILITY_ACCUMULATOR_SCALE = 10_000`
//! - `DYNAMIC_FEE_CONTROL_DENOMINATOR = 100_000`
//!
//! The on-chain effective fee is `min(base + dynamic, MAX_FEE_RATE_NUMERATOR)`
//! where `dynamic = ceil(control * (va * tick_spacing)^2 / (DENOM * SCALE^2))`
//! and `va` is the on-chain volatility accumulator, which can be updated every
//! swap. To stay fail-closed (a quote may never overstate output, i.e. never
//! understate the fee), the helpers here compute the fee rate at the pool's
//! `max_volatility_accumulator`: the largest fee the swap loop can charge in
//! the current account epoch. Any quote computed with this bound therefore
//! understates (never inflates) received output.

/// The dynamic-fee configuration must satisfy
/// `0 < control < DYNAMIC_FEE_CONTROL_DENOMINATOR` (`initialize_dynamic_fee_info`
/// verifier); a zero or out-of-range control means the dynamic fee is disabled.
pub const RAYDIUM_CLMM_DYNAMIC_FEE_CONTROL_DENOMINATOR: u32 = 100_000;
pub const RAYDIUM_CLMM_VOLATILITY_ACCUMULATOR_SCALE: u32 = 10_000;
pub const RAYDIUM_CLMM_MAX_FEE_RATE_NUMERATOR: u32 = 100_000;

/// Conservative worst-case dynamic fee rate (hundredths of a bip, denominator
/// 1e6) at the pool's `max_volatility_accumulator`. Returns 0 when the dynamic
/// fee is disabled. Pure integer math, matching `compute_dynamic_fee_rate`
/// bit-for-bit except that `va = max_volatility_accumulator` (upper bound; the
/// fee is monotonically increasing in `va`).
pub fn raydium_clmm_dynamic_fee_rate_upper_bound(
    dynamic_fee_control: u32,
    max_volatility_accumulator: u32,
    tick_spacing: u16,
) -> u32 {
    if dynamic_fee_control == 0
        || dynamic_fee_control >= RAYDIUM_CLMM_DYNAMIC_FEE_CONTROL_DENOMINATOR
    {
        return 0;
    }
    let crossed = u128::from(max_volatility_accumulator) * u128::from(tick_spacing);
    let squared = crossed * crossed;
    let denominator = u128::from(RAYDIUM_CLMM_DYNAMIC_FEE_CONTROL_DENOMINATOR)
        * u128::from(RAYDIUM_CLMM_VOLATILITY_ACCUMULATOR_SCALE)
        * u128::from(RAYDIUM_CLMM_VOLATILITY_ACCUMULATOR_SCALE);
    let numerator = u128::from(dynamic_fee_control) * squared;
    let fee_rate = numerator / denominator + u128::from(numerator % denominator != 0);
    u32::try_from(fee_rate.min(u128::from(RAYDIUM_CLMM_MAX_FEE_RATE_NUMERATOR)))
        .unwrap_or(RAYDIUM_CLMM_MAX_FEE_RATE_NUMERATOR)
}

/// Conservative total fee rate upper bound (base + worst-case dynamic), capped
/// at the on-chain `MAX_FEE_RATE_NUMERATOR`. `u16::MAX` resilience: fees above
/// the u64-`fee_rate` domain of the local swap math must be rejected by the
/// caller — checked with `u16::try_from` (never clamped).
pub fn raydium_clmm_total_fee_rate_upper_bound(
    base_fee_rate: u16,
    dynamic_fee_control: u32,
    max_volatility_accumulator: u32,
    tick_spacing: u16,
) -> u64 {
    (u64::from(base_fee_rate)
        + u64::from(raydium_clmm_dynamic_fee_rate_upper_bound(
            dynamic_fee_control,
            max_volatility_accumulator,
            tick_spacing,
        )))
    .min(u64::from(RAYDIUM_CLMM_MAX_FEE_RATE_NUMERATOR))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_dynamic_fee_is_zero() {
        assert_eq!(raydium_clmm_dynamic_fee_rate_upper_bound(0, 123, 10), 0);
        assert_eq!(
            raydium_clmm_dynamic_fee_rate_upper_bound(
                RAYDIUM_CLMM_DYNAMIC_FEE_CONTROL_DENOMINATOR,
                123,
                10
            ),
            0
        );
    }

    #[test]
    fn upper_bound_is_absolute_ceiling_of_the_exact_formula() {
        // Exact formula at va <= max must never exceed the upper bound.
        let (control, max_va, ts) = (5_000u32, 40_000u32, 64u16);
        let denominator = u128::from(RAYDIUM_CLMM_DYNAMIC_FEE_CONTROL_DENOMINATOR)
            * u128::from(RAYDIUM_CLMM_VOLATILITY_ACCUMULATOR_SCALE)
            * u128::from(RAYDIUM_CLMM_VOLATILITY_ACCUMULATOR_SCALE);
        let rate_at = |va: u32| {
            let numerator = u128::from(control) * (u128::from(va) * u128::from(ts)).pow(2);
            numerator / denominator + u128::from(numerator % denominator != 0)
        };
        let bound = u128::from(raydium_clmm_dynamic_fee_rate_upper_bound(
            control, max_va, ts,
        ));
        assert!(rate_at(12_345) <= bound);
        assert!(rate_at(max_va) <= bound);
        // The bound is the exact formula evaluated at the cap.
        assert_eq!(rate_at(max_va), bound);
    }

    #[test]
    fn caps_at_official_max_fee_rate_numerator() {
        let enorme = raydium_clmm_dynamic_fee_rate_upper_bound(99_999, u32::MAX, u16::MAX);
        assert_eq!(enorme, RAYDIUM_CLMM_MAX_FEE_RATE_NUMERATOR);
    }

    #[test]
    fn total_is_base_plus_dynamic_capped() {
        assert_eq!(
            raydium_clmm_total_fee_rate_upper_bound(2_500, 0, 0, 10),
            2_500
        );
        // control=50_000, max_va=100_000, ts=1: crossed=1e5, squared=1e10,
        // num=5e14, denom=1e13 -> q=50 rem=0; total=2_500+50.
        assert_eq!(
            raydium_clmm_total_fee_rate_upper_bound(2_500, 50_000, 100_000, 1),
            2_550
        );
        // Ceil case: control=10_000, max_va=10_000, ts=1: crossed=1e4,
        // squared=1e8, num=1e12, denom=1e13 -> q=0 rem=1e12>0 -> 1.
        assert_eq!(
            raydium_clmm_total_fee_rate_upper_bound(2_500, 10_000, 10_000, 1),
            2_501
        );
        // Combined base + dynamic must be capped at RAYDIUM_CLMM_MAX_FEE_RATE_NUMERATOR.
        assert_eq!(
            raydium_clmm_total_fee_rate_upper_bound(50_000, 99_999, u32::MAX, u16::MAX),
            u64::from(RAYDIUM_CLMM_MAX_FEE_RATE_NUMERATOR)
        );
    }
}
