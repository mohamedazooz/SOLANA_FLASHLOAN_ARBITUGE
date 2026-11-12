//! Regression test for the worked example in `AUDIT_AND_ENGINE_DESIGN.md` §4.2.
//!
//! Raydium 20,000 SOL / 3,400,000 USDC vs Orca 12,000 SOL / 2,076,000 USDC.
//! Buying SOL on Raydium and selling it on Orca must size to ~7,999 USDC and
//! return ~49 USDC of gross profit once both 25 bps fees are applied.

use arb_math::cpmm::{Leg, cpmm_out, optimal_two_pool};

fn leg(r_in: u128, r_out: u128, fee_bps: u128) -> Leg {
    Leg {
        r_in,
        r_out,
        gamma: 10_000u128.saturating_sub(fee_bps),
    }
}

#[test]
fn fee_aware_optimum_matches_worked_example() {
    // Pool A: Raydium, in = USDC, out = SOL.
    let a = leg(3_400_000_000_000, 20_000_000_000_000, 25);
    // Pool B: Orca, in = SOL, out = USDC.
    let b = leg(12_000_000_000_000, 2_076_000_000_000, 25);

    let d_star = optimal_two_pool(&a, &b).expect("arbitrage must exist");

    // 8,000 USDC = 8_000_000_000 atomic units (6 decimals).
    let expected: u64 = 8_000_000_000;
    let tolerance: u64 = expected / 100; // 1%

    assert!(
        d_star.abs_diff(expected) <= tolerance,
        "optimal size {d_star} not within 1% of {expected}"
    );

    let Some(sol_out) = cpmm_out(u128::from(d_star), a.r_in, a.r_out, a.gamma) else {
        panic!("cpmm_out failed for optimal size");
    };
    let Some(usdc_back) = cpmm_out(sol_out, b.r_in, b.r_out, b.gamma) else {
        panic!("cpmm_out failed for optimal size");
    };

    let gross = usdc_back.saturating_sub(u128::from(d_star));

    // ~49.4 USDC of gross profit => 49_400_000 micro-USD, allow 5%.
    let gross_usd = gross as f64 / 1e6;
    assert!(
        (gross_usd - 49.4).abs() < 2.5,
        "gross profit {gross_usd} USDC not within tolerance of 49.4"
    );
}

#[test]
fn no_arbitrage_returns_none_when_prices_match() {
    let a = leg(3_400_000_000_000, 20_000_000_000_000, 25);
    let b = leg(12_000_000_000_000, 2_040_000_000_000, 25); // same 170 price
    assert!(optimal_two_pool(&a, &b).is_none());
}
