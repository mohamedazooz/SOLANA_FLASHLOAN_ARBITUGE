//! Property-based fuzzing for economic/decimal boundaries (directive §18).
//!
//! Targets the exact fuzz classes the master directive calls out that the
//! existing suites do not cover:
//! * Extreme token decimals (0..=255) into `build_economic_quote`.
//! * Very small / very large borrow amounts (`u64` boundaries, `u64::MAX`).
//! * Integer boundaries in fee math (`calculate_flash_fee_raw`,
//!   `calculate_net_profit_raw`) — never panic, never saturate a loss into a
//!   gain; overflow must return `Err`, not a plausible-looking number.
//!
//! All assertions hold only on the `Ok` path; `Err(ArbError::Overflow | _)` is
//! an acceptable outcome (fail closed), panic is not.

use arb_math::{build_economic_quote, calculate_flash_fee_raw, calculate_net_profit_raw};
use arb_types::{ArbError, Lamports, MintId, Slot, TokenAmount};
use proptest::prelude::*;
use solana_pubkey::Pubkey;
use std::str::FromStr;

fn wsol_mint() -> MintId {
    MintId(Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    // 1. Extreme decimals: > 18 must be rejected with InvalidInput, never
    //    accepted (fail closed) and never panic.
    #[test]
    fn fuzz_extreme_decimals_rejected_above_18(decimals in 19u8..=255) {
        let r = build_economic_quote(
            wsol_mint(),
            decimals,
            TokenAmount(1_000),
            TokenAmount(1_100),
            TokenAmount(1_100),
            TokenAmount(0),
            TokenAmount(0),
            TokenAmount(0),
            TokenAmount(0),
            TokenAmount(0),
            Lamports(0),
            Lamports(0),
            Lamports(0),
            1,
            1,
            TokenAmount(1),
            Slot(1),
            1,
        );
        match r {
            Err(ArbError::InvalidInput(msg)) => {
                prop_assert!(msg.contains("decimals"), "wrong rejection reason: {msg}");
            }
            Ok(_) => prop_assert!(false, "decimals={decimals} accepted above the 18 ceiling"),
            Err(other) => prop_assert!(false, "unexpected error for decimals={decimals}: {other:?}"),
        }
    }

    // 2. Supported decimals 0..=18 with arbitrary raw amounts up to u64::MAX:
    //    Ok(path invariants hold) or Err(fail closed) — never panic, never a
    //    Profit quote whose minimum net is negative (profit decision must use
    //    the conservative minimum, not the expectation).
    #[test]
    fn fuzz_economic_quote_integer_boundaries(
        decimals in 0u8..=18,
        borrow in any::<u64>(),
        expected in any::<u64>(),
        minimum in any::<u64>(),
        flash_fee in any::<u64>(),
        dex_fees in any::<u64>(),
        transfer_fees in any::<u64>(),
        acct_cost in any::<u64>(),
        other_costs in any::<u64>(),
        rate_num in 1u128..=u128::MAX,
        rate_denom in 1u128..=u128::MAX,
        min_profit in any::<u64>(),
    ) {
        let r = build_economic_quote(
            wsol_mint(),
            decimals,
            TokenAmount(borrow),
            TokenAmount(expected),
            TokenAmount(minimum),
            TokenAmount(flash_fee),
            TokenAmount(dex_fees),
            TokenAmount(transfer_fees),
            TokenAmount(acct_cost),
            TokenAmount(other_costs),
            Lamports(1), // non-zero so the lamport->raw conversion path is exercised
            Lamports(1),
            Lamports(1),
            rate_num,
            rate_denom,
            TokenAmount(min_profit),
            Slot(42),
            1_000,
        );
        if let Ok(q) = r {
            // minimum output <= expected output is pre-validated by the ctor.
            prop_assert!(q.minimum_final_amount_raw.as_u64() <= q.expected_final_amount_raw.as_u64());
            // Positive projections of signed values, exactly (no saturation drift).
            prop_assert_eq!(q.net_profit_raw.as_u64(), q.expected_net_profit_signed_raw.max(0) as u64);
            prop_assert_eq!(q.gross_profit_raw.as_u64(), q.gross_profit_signed_raw.max(0) as u64);
            // Costs are non-negative → net (conservative) <= expected net.
            prop_assert!(q.minimum_net_profit_signed_raw <= q.expected_net_profit_signed_raw);
            // Profitability decision rests on the conservative minimum only.
            prop_assert_eq!(
                q.is_profitable(),
                q.meets_min_profit && q.minimum_net_profit_signed_raw > 0
            );
            // The floor comparison is exact: meets_min_profit == conservative net >= floor.
            prop_assert_eq!(
                q.meets_min_profit,
                q.minimum_net_profit_signed_raw >= i128::from(min_profit)
            );
        }
    }

    // 3. calculate_net_profit_raw: signed-vs-unsigned boundary. A loss (final <
    //    principal) must return Err — the u64 return contract forbids storing a
    //    negative PnL as a small positive number.
    #[test]
    fn fuzz_net_profit_raw_loss_is_not_positive(
        final_received in any::<u64>(),
        borrow in any::<u64>(),
        fees_sum in any::<u64>(),
    ) {
        let r = calculate_net_profit_raw(
            final_received,
            borrow,
            fees_sum, 0, 0, 0, 0,
            Lamports(0),
            Lamports(0),
            Lamports(0),
            1,
            1,
        );
        if final_received < borrow {
            match r {
                Err(_) => { /* Overflow on negative gross: correct fail-closed */ }
                Ok(net) => prop_assert!(false, "loss stored as positive net: {net}"),
            }
        }
    }

    // 4. Flash fee: disabled sentinel, origination guard, and rounding at
    //    integer boundaries — never panic, fee never exceeds the principal on Ok.
    #[test]
    fn fuzz_flash_fee_never_exceeds_principal(borrow in any::<u64>(), fee_sf in any::<u64>()) {
        match calculate_flash_fee_raw(borrow, fee_sf) {
            Ok(fee) => {
                prop_assert!(fee <= borrow, "flash fee {fee} exceeds principal {borrow}");
            }
            Err(ArbError::InvalidInput(_)) => {
                // Disabled (fee_sf == u64::MAX) or borrow too small: fail closed.
            }
            Err(ArbError::Overflow) => { /* escalated overflow: fail closed */ }
            Err(other) => prop_assert!(false, "unexpected error: {other:?}"),
        }
    }
}
