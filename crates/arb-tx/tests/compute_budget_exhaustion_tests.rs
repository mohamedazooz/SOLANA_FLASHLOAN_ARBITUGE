//! Compute-budget exhaustion invariants.
//!
//! What is proven here (offline, deterministic):
//! 1. The dynamic CU limit never returns below the measured requirement plus
//!    the safety buffer, unless clamped — and the only clamp is the Solana
//!    runtime consensus maximum (1,400,000).
//! 2. A route whose measured CU exceeds the runtime max is clamped to exactly
//!    1,400,000 — the on-chain "computational budget exceeded" rejection for
//!    such a route is a runtime decision, so the builder must never claim a
//!    higher limit than the runtime accepts.
//! 3. The estimator can never produce a self-exhausting budget: the dynamic
//!    limit derived from its own estimate always covers the estimate.
//! 4. Under exhaustion-level CU limits, the priority-fee cap still bounds the
//!    total cost to the approved profit share.

use arb_tx::{
    MAX_COMPUTE_UNIT_LIMIT, build_compute_unit_limit, calculate_dynamic_cu_limit,
    cap_cu_price_by_profit, estimate_minimal_cu, priority_fee_lamports,
};

#[test]
fn measured_cu_over_runtime_max_is_clamped_to_exactly_1_400_000() {
    let clamped = calculate_dynamic_cu_limit(1_400_001);
    assert_eq!(
        clamped, MAX_COMPUTE_UNIT_LIMIT,
        "exhaustion-level demand must clamp to the runtime max, never exceed it"
    );
    assert_eq!(MAX_COMPUTE_UNIT_LIMIT, 1_400_000);
}

#[test]
fn dynamic_limit_always_covers_measured_cu_plus_buffer_unless_clamped() {
    for measured in [1u32, 50_000, 98_450, 500_000, 1_000_000, 1_216_000] {
        let limit = calculate_dynamic_cu_limit(measured);
        let with_buffer = (measured as u64 + (measured as u64 * 15) / 100) as u32;
        if with_buffer <= MAX_COMPUTE_UNIT_LIMIT {
            assert_eq!(
                limit, with_buffer,
                "limit must be measured + 15% for measured={measured}"
            );
        } else {
            assert_eq!(limit, MAX_COMPUTE_UNIT_LIMIT);
        }
    }
}

#[test]
fn estimator_can_never_produce_a_self_exhausting_budget() {
    for hops in 2..=3 {
        for has_flash in [false, true] {
            let estimate = estimate_minimal_cu(has_flash, hops);
            let limit = calculate_dynamic_cu_limit(estimate);
            assert!(
                limit >= estimate,
                "dynamic limit ({limit}) must cover its own estimate ({estimate}) for hops={hops} flash={has_flash}"
            );
        }
    }
}

#[test]
fn priority_fee_cost_stays_bounded_at_exhaustion_level_cu_limits() {
    let profit_lamports: u64 = 1_000_000;
    let share_bps: u64 = 2_000;
    let desired_price: u64 = 500_000;

    let price = cap_cu_price_by_profit(
        profit_lamports,
        MAX_COMPUTE_UNIT_LIMIT,
        desired_price,
        share_bps,
    );
    let cost = priority_fee_lamports(price, MAX_COMPUTE_UNIT_LIMIT);
    let budget = profit_lamports * share_bps / 10_000;
    assert!(
        cost <= budget,
        "at a 1.4M CU limit the capped priority cost ({cost}) must stay within the profit budget ({budget})"
    );

    let one_more = priority_fee_lamports(price + 1, MAX_COMPUTE_UNIT_LIMIT);
    assert!(
        one_more > budget,
        "price+1 must breach the budget — proves the cap is exact, not loose"
    );
}

#[test]
fn cu_limit_instruction_encodes_the_full_runtime_max_verbatim() {
    let ix = build_compute_unit_limit(MAX_COMPUTE_UNIT_LIMIT);
    assert_eq!(ix.data[0], 2);
    assert_eq!(&ix.data[1..5], &1_400_000u32.to_le_bytes());
}
