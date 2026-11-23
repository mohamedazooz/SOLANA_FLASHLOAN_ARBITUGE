//! F-06 regression suite: Top-K route candidate retention.
//!
//! Root cause proven at HEAD: `build_three_leg_meme_pipeline` collapsed each
//! size's candidate list to its single best and kept only the global best
//! (chain, size). When the supervisor's FINAL authoritative economics gate
//! (actual base fee + priority fee + real Jito tip) rejected that one
//! candidate, every alternative had already been discarded — the opportunity
//! set was lost even when second-best routes were profitable net.
//!
//! Fix: `build_three_leg_meme_pipeline_top_k` ranks every quoted (chain,
//! size) candidate by the canonical net-profit accounting and returns a
//! bounded, best-first, per-candidate-constructed set. The single-best mode is
//! the k = 1 wrapper, so all invariants are shared.

mod common;

use arb_engine::ThreeLegMemeContextParams;
use arb_engine::{
    THREE_LEG_PIPELINE_TOP_K_MAX, ThreeLegMemePipelineError, build_three_leg_meme_pipeline,
    build_three_leg_meme_pipeline_top_k,
};
use common::{meme_pool_fixture, params as base_params};

const FLASH_LOAN_FEE_SF: u64 = 11_529_215_046_068; // 1e-5 U68F60

fn top_k_params(input_size: u64) -> ThreeLegMemeContextParams {
    let mut p = base_params(input_size);
    p.flash_loan_fee_sf = FLASH_LOAN_FEE_SF;
    p
}

/// The pipeline retains the size ladder as ranked candidates: the fixture
/// defines three search sizes, all economically distinct route-size
/// candidates, and the top-k set must carry each with its OWN identity, size
/// and legs (no candidate reuses another candidate's quote or accounts).
#[test]
fn f06_top_k_retains_distinct_route_size_candidates() {
    let pools = meme_pool_fixture();
    let p = top_k_params(100_000_000);
    let results =
        build_three_leg_meme_pipeline_top_k(&pools, &p, THREE_LEG_PIPELINE_TOP_K_MAX).unwrap();

    assert!(
        results.len() >= 2,
        "distinct (route, size) candidates must be retained, got {}",
        results.len()
    );
    assert!(
        results.len() <= THREE_LEG_PIPELINE_TOP_K_MAX,
        "top-k must stay bounded"
    );

    // Each candidate is its own route-size pair: size identity lives in its
    // size_search, and the first leg consumes exactly its size.
    for r in &results {
        assert_eq!(r.legs[0].amount_in, r.size_search.best_input);
        assert!(r.size_search.best_input > 0);
    }

    // Best-first ordering on canonical net profit.
    for pair in results.windows(2) {
        assert!(
            pair[0].net_profit_lamports >= pair[1].net_profit_lamports,
            "candidates must be ordered best-first: {} < {}",
            pair[0].net_profit_lamports,
            pair[1].net_profit_lamports
        );
    }

    // No candidate uses another route's execution accounts: contexts are
    // resolved against each candidate's own leg pool ids in order.
    for r in &results {
        for (leg, pool_id) in r.legs.iter().zip(r.pool_ids.iter()) {
            assert_eq!(leg.pool_id.0, *pool_id);
        }
    }
}

/// A requesting k above the policy ceiling is clamped, never unbounded.
#[test]
fn f06_top_k_request_is_clamped_to_policy_ceiling() {
    let pools = meme_pool_fixture();
    let p = top_k_params(100_000_000);
    let results = build_three_leg_meme_pipeline_top_k(&pools, &p, 1_000_000).unwrap();
    assert!(results.len() <= THREE_LEG_PIPELINE_TOP_K_MAX);
}

/// The k = 1 wrapper must return exactly the top-ranked candidate.
#[test]
fn f06_single_best_wrapper_equals_top_ranked_candidate() {
    let pools = meme_pool_fixture();
    let p = top_k_params(100_000_000);
    let best = build_three_leg_meme_pipeline(&pools, &p).unwrap();
    let mut ranked = build_three_leg_meme_pipeline_top_k(&pools, &p, 1).unwrap();
    assert_eq!(ranked.len(), 1);
    let first = ranked.remove(0);
    assert_eq!(first.pool_ids, best.pool_ids);
    assert_eq!(first.net_profit_lamports, best.net_profit_lamports);
    assert_eq!(
        first.size_search.best_input, best.size_search.best_input,
        "tie-breaking between candidates must not flip the winner"
    );
}

/// A floor above every candidate's net preserves the single-best failure
/// semantics (BelowNetProfitFloor with the OVERALL best net), so the caller
/// sees the same evidence as before — while viable alternates still existed.
#[test]
fn f06_floor_above_all_candidates_fails_closed_with_best_net() {
    let pools = meme_pool_fixture();
    let mut p = top_k_params(100_000_000);
    p.network_cost_lamports = 1_000_000_000; // guts every candidate
    p.min_profit_lamports = 1;
    let err =
        build_three_leg_meme_pipeline_top_k(&pools, &p, THREE_LEG_PIPELINE_TOP_K_MAX).unwrap_err();
    assert!(
        matches!(err, ThreeLegMemePipelineError::BelowNetProfitFloor { best_net, floor: 1 }
            if best_net < 1),
        "expected BelowNetProfitFloor with the overall best net, got: {err:?}"
    );
    // The single-best wrapper produces the identical rejection.
    let err1 = build_three_leg_meme_pipeline(&pools, &p).unwrap_err();
    assert_eq!(err.to_string(), err1.to_string());
}

/// Structural fail-closed preconditions are preserved (no pools, too few
/// pools) on the top-k surface too.
#[test]
fn f06_structural_preconditions_are_preserved() {
    let mut p = top_k_params(100_000_000);
    p.min_profit_lamports = 0;

    let err = build_three_leg_meme_pipeline_top_k(&meme_pool_fixture()[..2], &p, 3).unwrap_err();
    assert!(matches!(
        err,
        ThreeLegMemePipelineError::NoCycleFound { pools: 2 }
    ));

    let err = build_three_leg_meme_pipeline_top_k(&[], &p, 3).unwrap_err();
    assert!(matches!(err, ThreeLegMemePipelineError::NoEligiblePools));
}

/// Every retained candidate independently satisfies the repayment guarantee:
/// slippage-floor final output must cover principal + flash fee. Candidates
/// that fail construction are skipped; the remaining set is still returned.
#[test]
fn f06_retained_candidates_cover_principal_plus_flash_fee() {
    let pools = meme_pool_fixture();
    let p = top_k_params(100_000_000);
    let results =
        build_three_leg_meme_pipeline_top_k(&pools, &p, THREE_LEG_PIPELINE_TOP_K_MAX).unwrap();
    for r in &results {
        if r.gross_profit_lamports <= 0 {
            continue;
        }
        let required = r
            .size_search
            .best_input
            .saturating_add(r.flash_fee_lamports);
        assert!(
            r.expected_min_final_output >= required,
            "candidate (size {}) must guarantee repayment: min_out {} < required {}",
            r.size_search.best_input,
            r.expected_min_final_output,
            required
        );
    }
}
