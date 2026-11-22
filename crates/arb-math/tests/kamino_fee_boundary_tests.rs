//! Feature 1: Kamino KLend U68F60 Integer Fee Math — Tier 2 Boundary Tests.
//!
//! Validates exact integer arithmetic, boundary conditions, ties-away rounding,
//! sentinel disabling, and BorrowTooSmall enforcement for Kamino flash loan fees.

use arb_math::economic::{
    KaminoFeeError, calculate_flash_fee_raw, kamino_flash_fee_checked, kamino_flash_fee_unchecked,
};
use arb_types::error::ArbError;

/// Edge Case 1: Zero borrow amount and zero fee scaling factor.
#[test]
fn test_boundary_f1_zero_principal_and_fee_sf() {
    // Zero amount with zero fee factor
    assert_eq!(kamino_flash_fee_checked(0, 0), Ok(0));
    assert_eq!(kamino_flash_fee_unchecked(0, 0).unwrap(), 0);
    assert_eq!(calculate_flash_fee_raw(0, 0).unwrap(), 0);

    // Zero amount with 1 bps fee factor (11_529_215_046_068)
    let one_bps_sf = 11_529_215_046_068u64;
    assert_eq!(kamino_flash_fee_checked(0, one_bps_sf), Ok(0));
    assert_eq!(calculate_flash_fee_raw(0, one_bps_sf).unwrap(), 0);

    // Positive borrow amount with zero fee factor (free flash loan)
    assert_eq!(kamino_flash_fee_checked(10_000_000_000, 0), Ok(0));
    assert_eq!(calculate_flash_fee_raw(10_000_000_000, 0).unwrap(), 0);
}

/// Edge Case 2: 1-lamport borrow triggers BorrowTooSmall rejection.
/// For borrow = 1 and any positive fee_sf, fee_bits clamps to floor 1 << 60,
/// which equals amount << 60 (1 << 60). Thus fee_bits >= amount_bits, which
/// klend rejects as BorrowTooSmall.
#[test]
fn test_boundary_f1_one_lamport_borrow_too_small_rejection() {
    // Smallest positive fee_sf = 1
    assert_eq!(
        kamino_flash_fee_checked(1, 1),
        Err(KaminoFeeError::BorrowTooSmall)
    );

    // 1 bps fee factor
    let one_bps_sf = 11_529_215_046_068u64;
    assert_eq!(
        kamino_flash_fee_checked(1, one_bps_sf),
        Err(KaminoFeeError::BorrowTooSmall)
    );

    // calculate_flash_fee_raw fails closed with InvalidInput
    match calculate_flash_fee_raw(1, one_bps_sf) {
        Err(ArbError::InvalidInput(msg)) => {
            assert!(msg.contains("origination fee >= principal"));
        }
        other => panic!("expected InvalidInput, got {:?}", other),
    }
}

/// Edge Case 3: Smallest viable borrow principal.
/// For borrow = 2 and fee_sf = 1:
/// product = 2 * 1 < 1 << 60, so fee_bits = 1 << 60.
/// amount_bits = 2 << 60.
/// Since fee_bits (1 << 60) < amount_bits (2 << 60), BorrowTooSmall does not trigger.
/// Rounded fee = (1 << 60 + 1 << 59) >> 60 = 1.
#[test]
fn test_boundary_f1_smallest_viable_borrow_principal() {
    assert_eq!(kamino_flash_fee_checked(2, 1), Ok(1));
    assert_eq!(calculate_flash_fee_raw(2, 1).unwrap(), 1);
}

/// Edge Case 4: u64::MAX principal does not overflow U256 intermediate math.
#[test]
fn test_boundary_f1_u64_max_principal_no_overflow() {
    let one_bps_sf = 11_529_215_046_068u64; // ~1 bps
    let fee = kamino_flash_fee_checked(u64::MAX, one_bps_sf).unwrap();
    // Exact integer computation: (u64::MAX * 11_529_215_046_068 + 2^59) / 2^60 = 184_467_440_737_088
    assert_eq!(fee, 184_467_440_737_088);
    assert_eq!(
        calculate_flash_fee_raw(u64::MAX, one_bps_sf).unwrap(),
        184_467_440_737_088
    );
}

/// Edge Case 5: Sentinel value u64::MAX disables flash loans and fails closed.
#[test]
fn test_boundary_f1_disabled_sentinel_fails_closed() {
    for amount in [0, 1, 1_000_000_000, u64::MAX] {
        assert_eq!(
            kamino_flash_fee_checked(amount, u64::MAX),
            Err(KaminoFeeError::FlashLoanDisabled)
        );
        match calculate_flash_fee_raw(amount, u64::MAX) {
            Err(ArbError::InvalidInput(msg)) => {
                assert!(msg.contains("kamino flash loans disabled"));
            }
            other => panic!(
                "expected InvalidInput for amount {}, got {:?}",
                amount, other
            ),
        }
    }
}

/// Edge Case 6: Exact half ties-away from zero rounding.
/// For borrow = 1024 = 2^10:
/// sf_exact_half: product / 2^60 has fractional part exactly 0.5 (1 << 59).
/// It rounds UP to 8.
/// sf_below_half: product / 2^60 has fractional part 0.5 - eps.
/// It rounds DOWN to 7.
#[test]
fn test_boundary_f1_exact_half_ties_away_rounding() {
    let borrow = 1024u64; // 2^10
    // To make product / 2^60 = 7.5:
    // (borrow * sf) = 7.5 * 2^60 = 7 * 2^60 + 2^59.
    // sf = (7 * 2^60 + 2^59) / 2^10 = (7 << 50) + (1 << 49).
    let sf_exact_half = (7u64 << 50) + (1u64 << 49);
    let fee_half = kamino_flash_fee_checked(borrow, sf_exact_half).unwrap();
    assert_eq!(fee_half, 8, "7.5 must round up to 8 (ties away)");

    let sf_below_half = sf_exact_half - 1;
    let fee_below = kamino_flash_fee_checked(borrow, sf_below_half).unwrap();
    assert_eq!(fee_below, 7, "strictly below 7.5 must round down to 7");
}
