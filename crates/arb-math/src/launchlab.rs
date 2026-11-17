//! Raydium LaunchLab (`LanMV9sAd7wArD4vJFi2qDdfnVhFxYSUg6eADduJ3uj`) Bonding Curve Math.
//!
//! LaunchLab pools trade a base meme token against a quote asset (WSOL/USDC) on a
//! constant-product curve over virtual + real reserves until graduation (`status != 0`).

use ruint::aliases::U256;

/// LaunchLab fee denominator (`10_000` bps or `1_000_000` depending on platform config;
/// standard LaunchLab protocol + platform + creator fee rates use `10_000` bps or `1_000_000` ppm).
pub const LAUNCHLAB_FEE_DENOM: u128 = 10_000;
pub const LAUNCHLAB_FEE_DENOM_PPM: u128 = 1_000_000;

/// Quote `buy_exact_in`: spend `quote_amount_in` (after upfront fee deduction) to
/// receive base tokens out of `virtual_base - real_base` / `virtual_quote + real_quote` curve,
/// capped by `real_base_reserves`.
pub fn quote_launchlab_buy_exact_in(
    quote_amount_in: u64,
    virtual_base: u64,
    virtual_quote: u64,
    real_base: u64,
    real_quote: u64,
    total_fee_bps: u16,
) -> Option<u64> {
    if quote_amount_in == 0 || real_base == 0 {
        return Some(0);
    }
    if total_fee_bps >= 10_000 {
        return None;
    }
    let eff_base = u128::from(virtual_base).checked_add(u128::from(real_base))?;
    let eff_quote = u128::from(virtual_quote).checked_add(u128::from(real_quote))?;
    if eff_base == 0 || eff_quote == 0 {
        return None;
    }
    let dx = U256::from(quote_amount_in);
    let fee = (dx * U256::from(total_fee_bps) + U256::from(9_999u64)) / U256::from(10_000u64);
    if fee >= dx {
        return Some(0);
    }
    let net_in = dx - fee;
    let out = (net_in * U256::from(eff_base)) / (U256::from(eff_quote) + net_in);
    let out_u64 = u64::try_from(out).ok()?;
    if out_u64 > real_base {
        // Cannot exceed physical remaining tokens on bonding curve before graduation
        return None;
    }
    Some(out_u64)
}

/// Quote `sell_exact_in`: sell `base_amount_in` into the curve, receive quote tokens
/// less upfront/output fees, capped by `real_quote_reserves`.
pub fn quote_launchlab_sell_exact_in(
    base_amount_in: u64,
    virtual_base: u64,
    virtual_quote: u64,
    real_base: u64,
    real_quote: u64,
    total_fee_bps: u16,
) -> Option<u64> {
    if base_amount_in == 0 || real_quote == 0 {
        return Some(0);
    }
    if total_fee_bps >= 10_000 {
        return None;
    }
    let eff_base = u128::from(virtual_base).checked_add(u128::from(real_base))?;
    let eff_quote = u128::from(virtual_quote).checked_add(u128::from(real_quote))?;
    if eff_base == 0 || eff_quote == 0 {
        return None;
    }
    let dx = U256::from(base_amount_in);
    let gross_out = (dx * U256::from(eff_quote)) / (U256::from(eff_base) + dx);
    let fee =
        (gross_out * U256::from(total_fee_bps) + U256::from(9_999u64)) / U256::from(10_000u64);
    let net_out = gross_out.saturating_sub(fee);
    let out_u64 = u64::try_from(net_out).ok()?;
    if out_u64 > real_quote {
        return None;
    }
    Some(out_u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_launchlab_buy_zero_returns_zero() {
        let out = quote_launchlab_buy_exact_in(0, 1_000_000, 1_000_000, 500_000, 500_000, 25);
        assert_eq!(out, Some(0));
    }

    #[test]
    fn test_launchlab_buy_fee_deduction() {
        let out =
            quote_launchlab_buy_exact_in(10_000, 10_000_000, 10_000_000, 5_000_000, 5_000_000, 100);
        assert!(out.is_some());
        assert!(out.unwrap() > 0);
    }

    #[test]
    fn test_launchlab_sell_zero_returns_zero() {
        let out = quote_launchlab_sell_exact_in(0, 1_000_000, 1_000_000, 500_000, 500_000, 25);
        assert_eq!(out, Some(0));
    }

    #[test]
    fn test_launchlab_sell_exceeding_reserves_returns_none() {
        // Massive swap that exhausts real reserves
        let out =
            quote_launchlab_sell_exact_in(1_000_000_000, 1_000_000, 1_000_000, 500_000, 100, 25);
        assert_eq!(out, None);
    }
}
