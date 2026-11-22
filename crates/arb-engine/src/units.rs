//! Cross-denomination profit normalization — for REPORTING and CROSS-ROUTE
//! RANKING only.
//!
//! Per-route economic gates stay in the route's NATIVE flash-mint raw units
//! (a net quoted in WSOL remains lamports, a net quoted in USDC remains raw
//! USDC); the per-route profitability floor is never evaluated here. This
//! module exists because raw units are dimensionally incompatible ACROSS
//! mints: 1 raw USDC is 10⁻⁶ token units while 1 lamport is 10⁻⁹ WSOL — a
//! factor of 1000× per unit — so any cross-mint sweep/ranking that compares
//! raw nets as-is is silently biased by 10^(9−decimals).
//!
//! Contract (fail-closed, integer-only):
//! - WSOL flash mints pass through unchanged: 1 raw WSOL unit IS 1 lamport by
//!   definition (9 decimals), so no price is needed and none is consulted.
//! - Every other mint REQUIRES a positive integer SOL/USD price. Missing,
//!   zero, or an over-wide (decimals > 18) denomination returns `None`, which
//!   callers MUST treat as "cross-unit ranking refused" — never as zero.
//! - The conversion uses 128-bit intermediates and `div_euclid` (rounds toward
//!   negative infinity), so a negative net is never understated by more than
//!   the last lamport: the error is conservative in the fail-safe direction.
//!
//! `SOURCE_VERIFIED` for the 9-decimals WSOL raw-unit identity:
//! SPL Token mint So11111111111111111111111111111111111111112 declares 9
//! decimals (spl-token-2022 `token_metadata` / Solana Explorer), matching
//! `arb_constants::WSOL_MINT` used repo-wide as the flash/mid denomination.

use arb_types::MintId;
use solana_pubkey::Pubkey;
use std::str::FromStr;

/// Canonical WSOL mint pubkey (repo-wide single source: `arb_constants`).
#[must_use]
pub fn wsol_mint_pubkey() -> Pubkey {
    Pubkey::from_str(arb_constants::WSOL_MINT).expect("valid WSOL mint")
}

/// Convert a route's net profit from its native flash-mint raw units into
/// WSOL-lamports-equivalent, or `None` when the required SOL price bridge is
/// missing/invalid (cross-unit ranking refused, fail-closed).
///
/// `net_raw` may be negative (a loss); the denomination is identified by
/// `flash_mint` with `flash_mint_decimals`; `sol_usd_price` is the integer
/// USD price for one SOL (token units, USD-pegged mints assumed for non-WSOL
/// flash assets, e.g. USDC/USDT). The result is `net_lamports ≈
/// net_raw × 10⁹ / (10^decimals × price)`, floored toward −∞.
#[must_use]
pub fn net_lamports_equiv(
    net_raw: i64,
    flash_mint: MintId,
    flash_mint_decimals: u8,
    sol_usd_price: Option<u64>,
) -> Option<i64> {
    if flash_mint == MintId(wsol_mint_pubkey()) {
        return Some(net_raw);
    }
    let price = sol_usd_price.filter(|p| *p > 0)?;
    if flash_mint_decimals > 18 {
        return None;
    }
    let denomination = 10i128
        .checked_pow(u32::from(flash_mint_decimals))?
        .checked_mul(i128::from(price))?;
    // div_euclid floors toward −∞ so a loss is never understated by the
    // conversion (conservative direction for ranking).
    let scaled = i128::from(net_raw)
        .checked_mul(1_000_000_000i128)?
        .div_euclid(denomination);
    i64::try_from(scaled).ok()
}

#[cfg(test)]
mod tests {
    use super::{MintId, Pubkey, net_lamports_equiv, wsol_mint_pubkey};
    use std::str::FromStr;

    #[test]
    fn wsol_raw_units_are_lamports_and_need_no_price() {
        let wsol = MintId(wsol_mint_pubkey());
        assert_eq!(net_lamports_equiv(-143_680, wsol, 9, None), Some(-143_680));
        assert_eq!(
            net_lamports_equiv(i64::MAX, wsol, 9, Some(180)),
            Some(i64::MAX)
        );
    }

    #[test]
    fn usdc_raw_units_convert_at_the_supplied_sol_price() {
        let usdc =
            MintId(Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").expect("usdc"));
        // 1_000_000 raw USDC = 1 USDC = 1/180 SOL = 5,555,555.55… lamports.
        assert_eq!(
            net_lamports_equiv(1_000_000, usdc, 6, Some(180)),
            Some(5_555_555)
        );
        // Negative nets floor TOWARD MORE NEGATIVE (div_euclid): a loss is
        // never understated by the conversion.
        assert_eq!(
            net_lamports_equiv(-1_000_000, usdc, 6, Some(180)),
            Some(-5_555_556)
        );
    }

    #[test]
    fn missing_zero_price_or_wide_denomination_refuses_conversion() {
        let usdc =
            MintId(Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").expect("usdc"));
        assert_eq!(net_lamports_equiv(-101_956, usdc, 6, None), None);
        assert_eq!(net_lamports_equiv(-101_956, usdc, 6, Some(0)), None);
        assert_eq!(net_lamports_equiv(-101_956, usdc, 19, Some(180)), None);
    }

    #[test]
    fn unrepresentable_conversion_refuses_instead_of_saturating() {
        let zero_decimal =
            MintId(Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").expect("usdc"));
        // decimals = 0 with price 1 maps raw 1:1 into 10⁹ token-lamports; an
        // i64-scale raw value cannot fit and MUST refuse, not saturate.
        assert_eq!(net_lamports_equiv(i64::MAX, zero_decimal, 0, Some(1)), None);
    }
}
