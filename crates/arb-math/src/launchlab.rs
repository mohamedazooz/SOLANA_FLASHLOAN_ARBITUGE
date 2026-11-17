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

    /// Canonical LaunchLab-style curve: 1e9 virtual base, 1e9 virtual quote,
    /// 1e9 real base, 1e9 real quote (effective reserves 2e9 each).
    fn curve() -> (u64, u64, u64, u64) {
        (1_000_000_000, 1_000_000_000, 1_000_000_000, 1_000_000_000)
    }

    #[test]
    fn buy_zero_input_yields_zero_and_zero_real_base_yields_zero() {
        let (vb, vq, rb, rq) = curve();
        assert_eq!(
            quote_launchlab_buy_exact_in(0, vb, vq, rb, rq, 100),
            Some(0)
        );
        assert_eq!(
            quote_launchlab_buy_exact_in(1_000, vb, vq, 0, rq, 100),
            Some(0)
        );
    }

    #[test]
    fn buy_rejects_fee_at_or_above_100pct() {
        let (vb, vq, rb, rq) = curve();
        assert!(quote_launchlab_buy_exact_in(1_000, vb, vq, rb, rq, 10_000).is_none());
        assert!(quote_launchlab_buy_exact_in(1_000, vb, vq, rb, rq, u16::MAX).is_none());
    }

    #[test]
    fn buy_applies_upfront_fee_before_curve() {
        let (vb, vq, rb, rq) = curve();
        let dx = 1_000_000u64;
        let gross = quote_launchlab_buy_exact_in(dx, vb, vq, rb, rq, 0).unwrap();
        let net = quote_launchlab_buy_exact_in(dx, vb, vq, rb, rq, 100).unwrap();
        // 100 bps = 1% upfront fee on the input (ceil): net_in = dx - 10_000.
        // Independent recomputation of the same curve on the net input.
        let fee = (u128::from(dx) * 100).div_ceil(10_000);
        let net_in = u128::from(dx) - fee;
        let expected = (net_in * 2_000_000_000u128) / (2_000_000_000u128 + net_in);
        assert_eq!(u128::from(net), expected);
        // Concavity: the output loss is strictly smaller than the fee taken.
        assert!(net < gross);
        assert!(
            u128::from(gross - net) <= fee,
            "loss {} exceeds fee {fee}",
            gross - net
        );
    }

    #[test]
    fn buy_cannot_exceed_real_base_reserves() {
        let (vb, vq, rb, rq) = curve();
        // Drain attempt: huge input must be rejected, not fabricated.
        assert!(quote_launchlab_buy_exact_in(u64::MAX, vb, vq, rb, rq, 0).is_none());
    }

    #[test]
    fn sell_zero_input_yields_zero_and_zero_real_quote_yields_zero() {
        let (vb, vq, rb, rq) = curve();
        assert_eq!(
            quote_launchlab_sell_exact_in(0, vb, vq, rb, rq, 100),
            Some(0)
        );
        assert_eq!(
            quote_launchlab_sell_exact_in(1_000, vb, vq, rb, 0, 100),
            Some(0)
        );
    }

    #[test]
    fn sell_rejects_fee_at_or_above_100pct() {
        let (vb, vq, rb, rq) = curve();
        assert!(quote_launchlab_sell_exact_in(1_000, vb, vq, rb, rq, 10_000).is_none());
    }

    #[test]
    fn sell_cannot_exceed_real_quote_reserves() {
        let (vb, vq, rb, rq) = curve();
        assert!(quote_launchlab_sell_exact_in(u64::MAX, vb, vq, rb, rq, 0).is_none());
    }

    #[test]
    fn buy_then_sell_round_trip_strictly_loses() {
        let (vb, vq, rb, rq) = curve();
        let start_quote = 10_000_000u64;
        let bought = quote_launchlab_buy_exact_in(start_quote, vb, vq, rb, rq, 100).unwrap();
        assert!(bought > 0);
        let recovered = quote_launchlab_sell_exact_in(bought, vb, vq, rb, rq, 100).unwrap();
        assert!(
            recovered < start_quote,
            "fees + curve spread must make the round trip a strict loss: {recovered} >= {start_quote}"
        );
    }

    /// Official docs invariant (docs.raydium.io/products/launchlab/bonding-curve):
    /// `(V_q + real_quote_in_after_fee) x (V_b + real_base_remaining - base_out)
    ///  = V_q x V_b` — i.e. with effective reserves E_q, E_b:
    /// `(E_q + net_in) x (E_b - out) == E_q x E_b` under exact arithmetic; the
    /// integer quote may only round the output DOWN (never up).
    #[test]
    fn buy_preserves_official_xyk_invariant() {
        let (vb, vq, rb, rq) = curve();
        let (eff_base, eff_quote) = (
            u128::from(vb) + u128::from(rb),
            u128::from(vq) + u128::from(rq),
        );
        for dx in [1u64, 9_999, 1_000_000, 777_777_777] {
            let out = quote_launchlab_buy_exact_in(dx, vb, vq, rb, rq, 100).unwrap();
            let fee = (u128::from(dx) * 100).div_ceil(10_000);
            let net_in = u128::from(dx) - fee;
            let lhs = (eff_quote + net_in) * (eff_base - u128::from(out));
            let rhs = eff_quote * eff_base;
            // Truncation of `out` can only leave lhs >= rhs (out rounded down),
            // and by less than one full output unit of the product's divisor.
            assert!(
                lhs >= rhs && lhs - rhs < eff_quote + net_in,
                "dx={dx}: invariant violated lhs={lhs} rhs={rhs}"
            );
        }
    }

    #[test]
    fn price_impact_is_monotone_in_buy_amount() {
        let (vb, vq, rb, rq) = curve();
        let small = quote_launchlab_buy_exact_in(1_000_000, vb, vq, rb, rq, 0).unwrap();
        let large = quote_launchlab_buy_exact_in(100_000_000, vb, vq, rb, rq, 0).unwrap();
        let rate_small = small as f64 / 1_000_000.0;
        let rate_large = large as f64 / 100_000_000.0;
        assert!(rate_large < rate_small, "larger buy must get a worse rate");
    }
}
