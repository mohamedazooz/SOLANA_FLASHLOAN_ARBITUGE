//! Feature 17: Global Bounded Optimal Borrow Sizing — Tier 2 Boundary Tests.
//!
//! Validates:
//! 1. Search bounds strictly respect max vault capacity (never evaluates or exceeds capacity).
//! 2. Zero-derivative flat curve selects minimal input without division by zero.
//! 3. Strictly concave/unprofitable curve returns None (fails closed).
//! 4. Single-lamport vault bound evaluates point 1 without fractional division.
//! 5. Large u64 vault bound (u64::MAX - 1) prevents integer overflow panics.

use arb_engine::optimize_bounded_non_monotonic;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Boundary Case 1: Optimizer strictly respects max vault capacity ceiling.
/// Candidates evaluated must never exceed max_vault, and optimal borrow <= max_vault.
#[test]
fn test_size_optimizer_search_bounds_at_max_vault_capacity() {
    let max_vault = 10_000_000u64;
    let exceeded_vault = AtomicBool::new(false);
    let max_evaluated = AtomicU64::new(0);

    let result = optimize_bounded_non_monotonic(
        max_vault,
        32,
        4,
        8,
        &[max_vault.saturating_sub(1), max_vault],
        |size_in| {
            if size_in > max_vault {
                exceeded_vault.store(true, Ordering::SeqCst);
            }
            max_evaluated.fetch_max(size_in, Ordering::SeqCst);

            // Synthetic concave profit curve peaking at 8_000_000
            let score = if size_in <= max_vault {
                1_000_000i128 - i128::from(size_in.abs_diff(8_000_000))
            } else {
                -1_000_000i128
            };
            Some((size_in, score))
        },
    );

    assert!(
        !exceeded_vault.load(Ordering::SeqCst),
        "Optimizer must never evaluate a borrow size exceeding max vault capacity"
    );
    assert!(
        max_evaluated.load(Ordering::SeqCst) <= max_vault,
        "Maximum evaluated input must be <= max_vault"
    );

    let res = result.expect("profitable trade size candidate expected");
    assert!(
        res.input <= max_vault,
        "Selected optimal borrow size {} must be <= max_vault {}",
        res.input,
        max_vault
    );
    assert!(
        res.input.abs_diff(8_000_000) <= 50_000,
        "Optimizer must refine candidate close to true peak, got {}",
        res.input
    );
}

/// Boundary Case 2: Zero-derivative flat curve selects the minimal input achieving peak score.
#[test]
fn test_size_optimizer_zero_derivative_flat_curve() {
    let max_input = 50_000u64;

    let result = optimize_bounded_non_monotonic(max_input, 24, 4, 8, &[], |input| {
        // Flat score everywhere
        Some((input, 100i128))
    });

    let res = result.expect("candidate expected for flat curve");
    assert_eq!(res.score, 100);
    // In tie-breaking: max_by_key((score, Reverse(input))) guarantees minimal input
    assert_eq!(
        res.input, 1,
        "Flat curve tie-breaking must pick the smallest input"
    );
}

/// Boundary Case 3: Strictly unprofitable curve fails closed (returns None).
#[test]
fn test_size_optimizer_strictly_concave_unprofitable_curve() {
    let max_input = 100_000u64;

    let result = optimize_bounded_non_monotonic(max_input, 16, 4, 8, &[], |_input| {
        // Unprofitable routes return None in real pricing pipeline
        None::<(u64, i128)>
    });

    assert!(
        result.is_none(),
        "Strictly unprofitable curve returning None for all candidates must fail closed"
    );
}

/// Boundary Case 4: Single-lamport vault bound (max_input = 1).
#[test]
fn test_size_optimizer_single_lamport_vault_bound() {
    let max_input = 1u64;

    // Subcase 4a: profitable at 1 lamport
    let res_profitable =
        optimize_bounded_non_monotonic(max_input, 12, 2, 4, &[], |input| Some((input, 50i128)));
    assert!(res_profitable.is_some());
    let r = res_profitable.unwrap();
    assert_eq!(r.input, 1);
    assert_eq!(r.score, 50);

    // Subcase 4b: unprofitable at 1 lamport fails closed
    let res_unprofitable =
        optimize_bounded_non_monotonic(max_input, 12, 2, 4, &[], |_input| None::<(u64, i128)>);
    assert!(res_unprofitable.is_none());
}

/// Boundary Case 5: Large u64 vault bound (u64::MAX - 1) overflow prevention.
#[test]
fn test_size_optimizer_large_u64_vault_overflow_prevention() {
    let max_input = u64::MAX - 1;
    let eval_count = AtomicU64::new(0);

    let result = optimize_bounded_non_monotonic(max_input, 16, 2, 4, &[], |input| {
        eval_count.fetch_add(1, Ordering::SeqCst);
        assert!(
            input <= max_input,
            "candidate {} > max_input {}",
            input,
            max_input
        );
        // Only return Some for candidate 1 to ensure execution finishes cleanly
        if input == 1 {
            Some((input, 1i128))
        } else {
            None
        }
    });

    assert!(eval_count.load(Ordering::SeqCst) > 0);
    assert!(result.is_some());
    assert_eq!(result.unwrap().input, 1);
}
