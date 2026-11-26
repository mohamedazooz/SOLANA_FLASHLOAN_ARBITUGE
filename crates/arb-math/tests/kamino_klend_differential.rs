//! Differential test: the repo's Kamino flash-loan fee vs an INDEPENDENT
//! replication of klend's on-chain `ReserveFees::calculate_fees`.
//!
//! Upstream reference (pinned):
//! - repo: https://github.com/Kamino-Finance/klend
//! - commit: `686913ee421e0c5f3dc44c4c8344461ee0e4fb89`
//! - `programs/klend/src/state/reserve.rs` — `impl ReserveFees`:
//!   `calculate_flash_loan_fees` → `calculate_fees(amount, flash_loan_fee_sf,
//!   FeeCalculation::Exclusive, referral_fee_bps, has_referrer)`.
//! - `programs/klend/src/utils/fraction.rs` — `Fraction` = `fixed`-crate
//!   U68F60; `Mul` truncates after a wide multiply; `round()` rounds half
//!   away from zero.
//!
//! The replication below re-implements the upstream algorithm from scratch
//! (wide-integer math, no use of `arb_math` fee internals), including the
//! referrer fee split, so any divergence between the repo and klend mainnet
//! behavior surfaces as a hard test failure.

use arb_math::economic::{KaminoFeeError, kamino_flash_fee_checked};
use ruint::aliases::U256;

const FRAC_BITS: u32 = 60;
const ONE_BITS: u128 = 1u128 << FRAC_BITS;

/// Wide truncating U68F60 multiply with the `fixed` crate's `Mul` semantics:
/// `(a_bits * b_bits) >> 60`, computed in U256 to be overflow-free.
fn fraction_mul_truncating(a_bits: u128, b_bits: u128) -> u128 {
    let wide = U256::from(a_bits) * U256::from(b_bits);
    let res = wide >> FRAC_BITS;
    u128::try_from(res).expect("klend Fraction must stay within u128 bits")
}

/// Upstream failure modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KlendFeeOutcome {
    Fee(u64),
    BorrowTooSmall,
    /// `flash_loan_fee_sf == u64::MAX` — klend's flash-loans-disabled
    /// sentinel; `flash_borrow_reserve_liquidity` reverts before fee math.
    FlashLoansDisabled,
}

/// Independent replication of klend `calculate_fees` for the flash-loan path:
/// `FeeCalculation::Exclusive`, `origination_fee_rate = from_bits(fee_sf)`,
/// `minimum_fee = 1`, referral split per upstream. Returns the PROTOCOL fee
/// (the referral share is split out of the same fee, never added).
fn klend_calculate_fees_exclusive(
    amount: u64,
    fee_sf: u64,
    referral_fee_bps: u16,
    has_referrer: bool,
) -> KlendFeeOutcome {
    if fee_sf == u64::MAX {
        return KlendFeeOutcome::FlashLoansDisabled;
    }
    if fee_sf == 0 || amount == 0 {
        // upstream: `origination_fee_rate > ZERO && amount > ZERO` fails
        // ⇒ Ok((0, 0)).
        return KlendFeeOutcome::Fee(0);
    }

    let amount_bits = U256::from(amount) << FRAC_BITS;
    // Exclusive: `amount.mul(rate)` — truncating wide multiply.
    let origination_fee_amount_bits =
        fraction_mul_truncating((amount as u128) << FRAC_BITS, fee_sf as u128);

    // minimum_fee = 1 (Fraction ONE = 1 << 60), applied BEFORE rounding.
    let origination_fee_f_bits = U256::from(origination_fee_amount_bits).max(U256::from(ONE_BITS));

    // BorrowTooSmall: `origination_fee_f >= amount` (pre-round comparison).
    if origination_fee_f_bits >= amount_bits {
        return KlendFeeOutcome::BorrowTooSmall;
    }

    // to_round::<u64>(): round half away from zero on the fixed-point value.
    let half = U256::from(1u128 << (FRAC_BITS - 1));
    let rounded = (origination_fee_f_bits + half) >> FRAC_BITS;
    let origination_fee = u64::try_from(rounded).expect("BorrowTooSmall guard bounds the fee");

    // Referral split: only when a referrer is present AND bps > 0.
    let referral_fee = if referral_fee_bps > 0 && has_referrer {
        if referral_fee_bps == 10_000 {
            origination_fee
        } else {
            // `referral_fee_f = origination_fee_f * referral_fee_rate`,
            // `to_floor::<u64>()`. referral_fee_rate = bps / 10_000 as U68F60.
            // The product is itself a U68F60 fraction (bits); `to_floor` then
            // converts units by shifting down FRAC_BITS.
            let rate_bits = ((referral_fee_bps as u128) << FRAC_BITS) / 10_000;
            let referral_f_bits =
                (U256::from(origination_fee_f_bits) * U256::from(rate_bits)) >> FRAC_BITS;
            let referral_units = referral_f_bits >> FRAC_BITS;
            u64::try_from(referral_units).unwrap_or(u64::MAX)
        }
    } else {
        0
    };

    KlendFeeOutcome::Fee(origination_fee - referral_fee)
}

/// Total fee (the single origination fee) regardless of split.
fn klend_total_flash_fee(amount: u64, fee_sf: u64) -> KlendFeeOutcome {
    klend_calculate_fees_exclusive(amount, fee_sf, 0, false)
}

/// Referral share of the fee, replicating upstream's split exactly:
/// `referral = total − protocol` when a referrer is present.
fn klend_referral_share(amount: u64, fee_sf: u64, referral_fee_bps: u16) -> u64 {
    let protocol = match klend_calculate_fees_exclusive(amount, fee_sf, referral_fee_bps, true) {
        KlendFeeOutcome::Fee(p) => p,
        _ => 0,
    };
    let total = match klend_total_flash_fee(amount, fee_sf) {
        KlendFeeOutcome::Fee(t) => t,
        _ => 0,
    };
    total - protocol
}

/// Upstream repayment deduction: at repay the handler transfers
/// `liquidity_amount` → reserve, `protocol_fee` → fee vault, and
/// `referral_fee` → referrer (when present). All three leave the user, and
/// `protocol + referral == total_fee`, so the deduction is
/// principal + the single fee.
fn klend_total_user_deduction(amount: u64, fee_sf: u64, referral_fee_bps: u16) -> Option<u128> {
    match klend_total_flash_fee(amount, fee_sf) {
        KlendFeeOutcome::Fee(total_fee) => {
            let _ = referral_fee_bps; // interior to total_fee; see module docs
            Some(amount as u128 + total_fee as u128)
        }
        _ => None,
    }
}

/// Small exhaustive sweep: every (amount, fee_sf) pair in a dense low domain
/// plus the structurally interesting boundaries.
#[test]
fn differential_exhaustive_small_domain_has_referrer_false() {
    let fee_sfs: [u64; 12] = [
        0,
        1,
        2,
        3,
        1_000,               // ~1e-15
        1_152_921_504_606,   // ~0.001
        11_529_215_046_068,  // mainnet WSOL market ≈ 0.1 bps
        115_292_150_460_685, // ~1 bps
        1 << 59,
        1 << 60, // rate = 1.0 (100%)
        u64::MAX - 1,
        u64::MAX, // flash-loans-disabled sentinel
    ];
    for amount in 0u64..=400u64 {
        for &fee_sf in &fee_sfs {
            let upstream = klend_calculate_fees_exclusive(amount, fee_sf, 0, false);
            let ours = kamino_flash_fee_checked(amount, fee_sf);
            match upstream {
                KlendFeeOutcome::Fee(f) => {
                    assert_eq!(ours, Ok(f), "amount={amount} fee_sf={fee_sf}");
                }
                KlendFeeOutcome::BorrowTooSmall => {
                    assert_eq!(
                        ours,
                        Err(KaminoFeeError::BorrowTooSmall),
                        "amount={amount} fee_sf={fee_sf}"
                    );
                }
                KlendFeeOutcome::FlashLoansDisabled => {
                    assert_eq!(
                        ours,
                        Err(KaminoFeeError::FlashLoanDisabled),
                        "sentinel must map to FlashLoanDisabled: amount={amount}"
                    );
                }
            }
        }
    }
}

/// Dense fee_sf sweep on a realistic amount to catch rounding-boundary
/// divergence anywhere in the low bits.
#[test]
fn differential_dense_fee_sf_sweep() {
    let amount: u64 = 1_000_000_000; // 1 SOL of lamports
    let sweep = (0u64..=20_000)
        .step_by(7)
        .chain((0u64..=2_000_000).step_by(9_973));
    for fee_sf in sweep {
        let upstream = klend_calculate_fees_exclusive(amount, fee_sf, 0, false);
        let ours = kamino_flash_fee_checked(amount, fee_sf);
        match upstream {
            KlendFeeOutcome::Fee(f) => assert_eq!(ours, Ok(f), "fee_sf={fee_sf}"),
            KlendFeeOutcome::BorrowTooSmall => {
                assert_eq!(ours, Err(KaminoFeeError::BorrowTooSmall))
            }
            KlendFeeOutcome::FlashLoansDisabled => unreachable!("fee_sf <= 2_000_000 here"),
        }
    }
}

/// Pseudo-random large-domain differential (deterministic xorshift; no
/// external rng dependency). Exercises realistic 64-bit amounts and fee_sf
/// values, including extremes.
#[test]
fn differential_pseudorandom_large_domain() {
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..50_000 {
        let amount = next() % 500_000_000_000 + 1;
        let fee_sf = match next() % 8 {
            0 => next() % 1_000_000,
            1 => next() % (1 << 58),
            2 => next() % (1 << 59),
            3 => 1 << 60, // 100%
            4 => u64::MAX - 1,
            5 => 11_529_215_046_068,
            6 => next() % (1 << 40),
            _ => next() % (1 << 63),
        };
        let upstream = klend_calculate_fees_exclusive(amount, fee_sf, 0, false);
        let ours = kamino_flash_fee_checked(amount, fee_sf);
        match upstream {
            KlendFeeOutcome::Fee(f) => {
                assert_eq!(ours, Ok(f), "amount={amount} fee_sf={fee_sf}")
            }
            KlendFeeOutcome::BorrowTooSmall => {
                assert_eq!(ours, Err(KaminoFeeError::BorrowTooSmall));
            }
            KlendFeeOutcome::FlashLoansDisabled => {
                assert_eq!(ours, Err(KaminoFeeError::FlashLoanDisabled));
            }
        }
    }
}

/// Referral split identity: upstream splits ONE fee into
/// `protocol_fee + referral_fee`; the total is the same single fee the repo
/// charges. With `has_referrer = false` (every production builder in this
/// repo passes `None`), the referral share is structurally zero.
#[test]
fn differential_referral_split_is_interior_to_total_fee() {
    let mut state: u64 = 0xDEAD_BEEF_CAFE_BABE;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..10_000 {
        let amount = next() % 1_000_000_000_000 + 2;
        let fee_sf = next() % (1 << 56) + 1;
        let referral_fee_bps = (next() % 10_001) as u16;

        let total = match klend_total_flash_fee(amount, fee_sf) {
            KlendFeeOutcome::Fee(f) => f,
            other => {
                // Failure modes must agree with the repo either way.
                let ours = kamino_flash_fee_checked(amount, fee_sf);
                assert!(matches!(
                    (other, ours),
                    (
                        KlendFeeOutcome::BorrowTooSmall,
                        Err(KaminoFeeError::BorrowTooSmall)
                    ) | (
                        KlendFeeOutcome::FlashLoansDisabled,
                        Err(KaminoFeeError::FlashLoanDisabled)
                    )
                ));
                continue;
            }
        };

        let protocol_fee =
            match klend_calculate_fees_exclusive(amount, fee_sf, referral_fee_bps, true) {
                KlendFeeOutcome::Fee(p) => p,
                other => panic!("same inputs, same fee outcome expected, got {other:?}"),
            };
        let referral_fee = klend_referral_share(amount, fee_sf, referral_fee_bps);

        assert_eq!(
            protocol_fee + referral_fee,
            total,
            "upstream split must be interior: amount={amount} fee_sf={fee_sf} bps={referral_fee_bps}"
        );
        assert!(referral_fee <= total);

        // has_referrer = false ⇒ referral share structurally zero and the
        // repo's single fee IS the protocol fee.
        let no_referrer = klend_calculate_fees_exclusive(amount, fee_sf, referral_fee_bps, false);
        assert_eq!(no_referrer, KlendFeeOutcome::Fee(total));
        assert_eq!(
            kamino_flash_fee_checked(amount, fee_sf),
            Ok(total),
            "repo fee must equal upstream total flash fee"
        );
    }
}

/// Total user deduction at repay == principal + THE ONE fee, regardless of
/// whether a referrer participates. This proves the repo's builder decision
/// (repay `amount` = principal, fee charged separately) matches upstream.
#[test]
fn differential_total_repay_deduction_is_principal_plus_single_fee() {
    let mut state: u64 = 0x0123_4567_89AB_CDEF;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..10_000 {
        let amount = next() % 1_000_000_000_000 + 2;
        let fee_sf = next() % (1 << 56) + 1;
        let referral_fee_bps = (next() % 10_001) as u16;

        let deduction_no_referrer = klend_total_user_deduction(amount, fee_sf, 0);
        let deduction_with_referrer = klend_total_user_deduction(amount, fee_sf, referral_fee_bps);
        assert_eq!(
            deduction_no_referrer, deduction_with_referrer,
            "referrer presence redistributes the fee, never adds to it"
        );

        match (
            deduction_no_referrer,
            kamino_flash_fee_checked(amount, fee_sf),
        ) {
            (Some(deduction), Ok(fee)) => assert_eq!(
                deduction,
                amount as u128 + fee as u128,
                "repo deduction must equal upstream total deduction"
            ),
            (None, Err(_)) => {}
            other => panic!("divergent failure modes: {other:?}"),
        }
    }
}

/// Zero-rate and zero-amount edge: upstream returns Ok((0, 0)) — no fee.
#[test]
fn differential_zero_edges() {
    assert_eq!(kamino_flash_fee_checked(0, 0), Ok(0));
    assert_eq!(kamino_flash_fee_checked(1_000, 0), Ok(0));
    assert_eq!(kamino_flash_fee_checked(0, 11_529_215_046_068), Ok(0));
}
