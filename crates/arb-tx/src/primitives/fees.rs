//! Priority-fee unit conversion — the single source of truth.
//!
//! A `ComputeBudget` priority price is expressed in **micro-lamports per compute
//! unit**, while the total fee charged is
//! `ceil(price_micro_lamports * compute_unit_limit / 1_000_000)` lamports.
//!
//! Two failure modes this module exists to prevent:
//!
//! * Dividing micro-lamports by `1_000_000` directly (`price / 1e6`) truncates
//!   every realistic price (e.g. 50 000 µ-lamports) to **zero**, silently
//!   dropping the priority fee to nothing and losing the leader race.
//! * Forgetting the `compute_unit_limit` factor under-reports the fee by the
//!   whole CU budget (up to 1.4 M), which under-funds the account and can make
//!   the transaction fail fee validation.
//!
//! Ceiling rounding is used so a non-zero price never rounds down to a zero
//! total fee.

/// Total priority fee in lamports for `micro_lamports_per_cu × cu_limit`.
///
/// Returns `u64::MAX` (fail closed, "absurdly expensive" so the caller rejects
/// the plan) if the product exceeds `u64::MAX` lamports.
#[inline]
pub fn total_priority_fee_lamports(micro_lamports_per_cu: u64, cu_limit: u32) -> u64 {
    let numerator = u128::from(micro_lamports_per_cu).saturating_mul(u128::from(cu_limit));
    let total = numerator.saturating_add(999_999) / 1_000_000;
    u64::try_from(total).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_price_and_budget_to_total_lamports() {
        // 1 micro-lamport/CU over 1_000_000 CU = 1 lamport.
        assert_eq!(total_priority_fee_lamports(1, 1_000_000), 1);
        // 50_000 micro-lamports/CU over 200_000 CU = 10 000 lamports.
        assert_eq!(total_priority_fee_lamports(50_000, 200_000), 10_000);
    }

    #[test]
    fn never_truncates_a_nonzero_price_to_zero() {
        // The legacy `price / 1_000_000` bug produced 0 here.
        assert_eq!(total_priority_fee_lamports(1, 1), 1);
        assert_eq!(total_priority_fee_lamports(500_000, 1), 1);
    }

    #[test]
    fn zero_price_is_zero_fee() {
        assert_eq!(total_priority_fee_lamports(0, 1_400_000), 0);
    }

    #[test]
    fn overflow_fails_closed() {
        assert_eq!(total_priority_fee_lamports(u64::MAX, u32::MAX), u64::MAX);
    }
}
