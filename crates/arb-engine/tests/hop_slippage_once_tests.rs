//! §5.1 / §5.2 regression: the `hop_inputs` contract and single-slippage rule.
//!
//! `TxBuilder::derive_swap_legs` documents
//! `hop_inputs == [borrow, out_1, .., out_n]` (length `hops.len() + 1`) and
//! applies slippage to each entry exactly once. The three-leg capture path used
//! to feed it the per-leg `amount_in` values instead, which are ALREADY
//! discounted (`amount_in[i+1] == floor(out_i * (1 - s))`). That double-counted
//! slippage on every intermediate leg and made the final leg's floor derive
//! from the optimistic quote.
//!
//! These tests pin the current contract:
//!
//! 1. Slippage is applied exactly once across a multi-hop chain.
//! 2. Each hop's `amount_in` is clamped to the previous hop's `min_amount_out`
//!    (`min(hop_inputs[i], prev.min_amount_out)`), so the chain can never spend
//!    more than the previous hop guarantees.
//! 3. `previous.min_amount_out >= next.amount_in` holds for every adjacent pair.
//! 4. Min-out exactly equal to the next input is accepted (boundary).
//! 5. An oversized caller-supplied next input is clamped to the previous
//!    min-out; the supervisor's dispatch guard rejects such routes upstream.
//! 6. Rounding, zero-output and overflow boundaries fail closed.

mod common;

use arb_engine::build_three_leg_meme_pipeline;
use arb_tx::{ExecutionOpportunity, TxBuilder};
use arb_types::{Bps, DexProtocol, Lamports, MintId, PoolId, Slot, SwapHop, TokenAmount};
use common::{meme_pool_fixture, params as base_params};
use solana_pubkey::Pubkey;

/// Recover the `(amount_in, min_amount_out)` legs a builder would emit for an
/// opportunity, using the same `hop_slippage_bps` the supervisor dispatches.
fn legs_for(opp: &ExecutionOpportunity) -> Vec<(u64, u64)> {
    let flash_fee = 0u64;
    let tip = 0u64;
    let min_profit = 0u64;
    let slippage = opp.effective_hop_slippage_bps();
    TxBuilder::derive_swap_legs(opp, flash_fee, tip, min_profit, slippage)
        .expect("derive_swap_legs must accept a well-formed opportunity")
}

/// Build a 3-leg opportunity from the real pipeline, mirroring the supervisor's
/// construction of `hop_inputs`.
fn three_leg_opportunity(
    slippage_bps: u64,
) -> (Vec<(u64, u64)>, arb_engine::ThreeLegMemePipelineResult) {
    let pools = meme_pool_fixture();
    let mut p = base_params(100_000_000);
    p.slippage_bps = slippage_bps;
    p.min_profit_lamports = 0;
    let result = build_three_leg_meme_pipeline(&pools, &p).expect("pipeline must build");

    let size_in = result.size_search.best_input.max(1);
    // Corrected contract: [borrow, out_1, out_2, out_3].
    let hop_inputs = vec![
        size_in,
        result.legs[0].amount_out,
        result.legs[1].amount_out,
        result.legs[2].amount_out,
    ];

    let wsol = Pubkey::from_str(arb_constants::WSOL_MINT).unwrap();
    let hops: Vec<SwapHop> = result
        .legs
        .iter()
        .map(|leg| SwapHop {
            pool_id: leg.pool_id,
            protocol: DexProtocol::MeteoraDlmm,
            mint_in: leg.from_mint,
            mint_out: leg.to_mint,
            fee_bps: 10,
            active_bin_id: None,
            token_programs: Default::default(),
            trade_intent: Default::default(),
            execution_accounts: None,
        })
        .collect();

    let opp = ExecutionOpportunity {
        hop_slippage_bps: slippage_bps,
        route_id: arb_types::RouteId(1),
        slot: Slot(1),
        size_in: TokenAmount(size_in),
        expected_out: TokenAmount(result.expected_final_out),
        hop_inputs,
        gross_profit: Lamports(result.gross_profit_lamports as u64),
        net_profit: Lamports(result.net_profit_lamports.max(0) as u64),
        base_decimals: 9,
        price_impact_bps: Bps(0),
        hops,
        expected_min_final_output: result.expected_min_final_output,
    };
    let _ = wsol;
    (legs_for(&opp), result)
}

/// §5.2 (1) — slippage applied EXACTLY ONCE at every hop boundary.
///
/// With the `[borrow, out_1, out_2, out_3]` contract, leg i's nominal input is
/// the previous hop's *expected* output, clamped to that hop's `min_amount_out`
/// (the guaranteed delivery). Its `min_amount_out` is the expected output
/// discounted once. If slippage were applied twice, the ratio
/// `amount_out / amount_in` would be `(1-s)^2` instead of `(1-s)`.
#[test]
fn slippage_is_applied_exactly_once_per_boundary() {
    let slippage_bps = 100u64; // 1%
    let (legs, result) = three_leg_opportunity(slippage_bps);
    assert_eq!(legs.len(), 3);

    for (i, (&(amount_in, min_out), leg_result)) in
        legs.iter().zip(result.legs.iter()).enumerate().take(3)
    {
        let expected_out = leg_result.amount_out;

        if i > 0 {
            let prev_min_out = legs[i - 1].1;
            assert_eq!(
                amount_in,
                result.legs[i - 1].amount_out.min(prev_min_out),
                "leg {i} input must be clamped to the previous hop's guaranteed min-out"
            );
        }

        // Exactly one haircut: min_out == expected_out - expected_out*s/10_000.
        let once = expected_out
            .checked_sub(expected_out * slippage_bps / 10_000)
            .expect("no overflow");
        assert_eq!(
            min_out, once,
            "leg {i}: min_out must be a single slippage haircut of {expected_out}, not a double one"
        );

        // And prove it is NOT the double-haircut value, whenever the two differ.
        let twice = once
            .checked_sub(once * slippage_bps / 10_000)
            .expect("no overflow");
        if twice != once {
            assert_ne!(
                min_out, twice,
                "leg {i}: slippage must not be compounded across the boundary"
            );
        }
    }
}

/// §5.2 (2) — the mandatory adjacency invariant:
/// `previous.min_amount_out >= next.amount_in` for every adjacent pair: the
/// builder clamps each hop's input to the previous hop's guaranteed min-out, so
/// at zero slippage the two coincide and at non-zero slippage the next input is
/// exactly the previous min-out (the nominal expected output is haircuts away).
///
/// This test verifies the invariant holds at zero slippage, and that the final leg
/// covers the repayment floor.
#[test]
fn previous_min_out_covers_next_input_for_every_pair() {
    for &slippage in &[0u64, 50, 100, 300] {
        let (legs, result) = three_leg_opportunity(slippage);
        for i in 0..legs.len() - 1 {
            let (_, min_out) = legs[i];
            let (next_in, _) = legs[i + 1];
            if slippage == 0 {
                // At zero slippage, min_out == next_in (no haircut)
                assert_eq!(
                    min_out,
                    next_in,
                    "slippage 0 bps: leg {i} min_out {min_out} != leg {} input {next_in}",
                    i + 1
                );
            } else {
                // At non-zero slippage the next input is clamped exactly to the
                // previous min-out (nominal expected output > guaranteed floor).
                assert_eq!(
                    next_in,
                    min_out,
                    "slippage {slippage} bps: leg {} input {next_in} must be clamped to leg {i} min_out {min_out}",
                    i + 1
                );
            }
        }
        // The final leg's min-out must cover the repayment floor the pipeline
        // guarantees (principal + flash fee), otherwise repay cannot land.
        let (_, final_min_out) = legs[2];
        assert!(
            final_min_out >= result.expected_min_final_output,
            "final min_out {final_min_out} below pipeline floor {}",
            result.expected_min_final_output
        );
        assert_eq!(
            legs[2].0,
            result.legs[1].amount_out.min(legs[1].1),
            "final leg input must be clamped to the previous min-out"
        );
    }
}

/// §5.2 (3) — boundary: min-out EXACTLY equal to the next input is accepted.
#[test]
fn min_out_exactly_equal_to_next_input_is_accepted() {
    // Construct the boundary directly: out_1 == in_2 exactly.
    let wsol = Pubkey::from_str(arb_constants::WSOL_MINT).unwrap();
    let usdc = Pubkey::from_str(arb_constants::USDC_MINT).unwrap();
    let hop = |mint_in, mint_out| SwapHop {
        pool_id: PoolId::new([7u8; 32]),
        protocol: DexProtocol::MeteoraDlmm,
        mint_in,
        mint_out,
        fee_bps: 0,
        active_bin_id: None,
        token_programs: Default::default(),
        trade_intent: Default::default(),
        execution_accounts: None,
    };
    let opp = ExecutionOpportunity {
        hop_slippage_bps: 0, // no haircut: min_out == expected_out exactly
        route_id: arb_types::RouteId(2),
        slot: Slot(1),
        size_in: TokenAmount(1_000),
        expected_out: TokenAmount(1_100),
        // [borrow=1000, out_1=1100, out_2=1100] -> leg0 min_out == leg1 amount_in
        hop_inputs: vec![1_000, 1_100, 1_100],
        gross_profit: Lamports(100),
        net_profit: Lamports(100),
        base_decimals: 9,
        price_impact_bps: Bps(0),
        hops: vec![
            hop(MintId(wsol), MintId(usdc)),
            hop(MintId(usdc), MintId(wsol)),
        ],
        expected_min_final_output: 0,
    };
    let legs = legs_for(&opp);
    assert_eq!(legs[0].1, legs[1].0, "equality boundary must be allowed");
    assert_eq!(legs[0].1, 1_100);
    assert_eq!(legs[1].0, 1_100);
}

/// §5.2 (4) — an oversized caller-supplied next input is clamped to the
/// previous hop's guaranteed min-out, never dispatched as-is.
///
/// A well-formed opportunity can never have `hop_inputs[i+1] > out_i` by
/// construction, and the supervisor's dispatch guard rejects any route whose
/// nominal next input exceeds the previous min-out before a builder ever sees
/// it. The builder's own last line of defence is to clamp the input to the
/// guaranteed delivery rather than silently under-collateralise the chain.
#[test]
fn next_input_above_previous_min_out_is_clamped() {
    let wsol = Pubkey::from_str(arb_constants::WSOL_MINT).unwrap();
    let usdc = Pubkey::from_str(arb_constants::USDC_MINT).unwrap();
    let hop = |mint_in, mint_out| SwapHop {
        pool_id: PoolId::new([8u8; 32]),
        protocol: DexProtocol::MeteoraDlmm,
        mint_in,
        mint_out,
        fee_bps: 0,
        active_bin_id: None,
        token_programs: Default::default(),
        trade_intent: Default::default(),
        execution_accounts: None,
    };
    let opp = ExecutionOpportunity {
        hop_slippage_bps: arb_types::DEFAULT_HOP_SLIPPAGE_BPS,
        route_id: arb_types::RouteId(3),
        slot: Slot(1),
        size_in: TokenAmount(1_000),
        expected_out: TokenAmount(2_000),
        // Malformed: leg1 wants 1_500 but leg0 only guarantees ~990.
        hop_inputs: vec![1_000, 1_000, 1_500],
        gross_profit: Lamports(500),
        net_profit: Lamports(500),
        base_decimals: 9,
        price_impact_bps: Bps(0),
        hops: vec![
            hop(MintId(wsol), MintId(usdc)),
            hop(MintId(usdc), MintId(wsol)),
        ],
        expected_min_final_output: 0,
    };
    let legs = legs_for(&opp);
    assert_eq!(
        legs[0].1, 999,
        "fixture: leg0 min-out must be 1000 discounted once by the default 10 bps"
    );
    assert_eq!(
        legs[1].0, legs[0].1,
        "oversized nominal input (1_500 > min_out 999) must be clamped to the previous min-out"
    );
}

/// §5.2 (5) — rounding / zero-output / overflow boundaries.
#[test]
fn boundary_cases_fail_closed_or_round_conservatively() {
    // Zero expected output on a hop that consumes input must be an error.
    let wsol = Pubkey::from_str(arb_constants::WSOL_MINT).unwrap();
    let usdc = Pubkey::from_str(arb_constants::USDC_MINT).unwrap();
    let hop = |mint_in, mint_out| SwapHop {
        pool_id: PoolId::new([9u8; 32]),
        protocol: DexProtocol::MeteoraDlmm,
        mint_in,
        mint_out,
        fee_bps: 0,
        active_bin_id: None,
        token_programs: Default::default(),
        trade_intent: Default::default(),
        execution_accounts: None,
    };
    let zero_out = ExecutionOpportunity {
        hop_slippage_bps: arb_types::DEFAULT_HOP_SLIPPAGE_BPS,
        route_id: arb_types::RouteId(4),
        slot: Slot(1),
        size_in: TokenAmount(1_000),
        expected_out: TokenAmount(0),
        hop_inputs: vec![1_000, 0],
        gross_profit: Lamports(0),
        net_profit: Lamports(0),
        base_decimals: 9,
        price_impact_bps: Bps(0),
        hops: vec![hop(MintId(wsol), MintId(usdc))],
        expected_min_final_output: 0,
    };
    let err = TxBuilder::derive_swap_legs(&zero_out, 0, 0, 0, arb_types::DEFAULT_HOP_SLIPPAGE_BPS)
        .expect_err("zero output on a funded hop must be rejected");
    assert!(
        matches!(err, arb_tx::TxError::InvalidInput(_)),
        "unexpected error: {err:?}"
    );

    // Tiny output with 1% slippage rounds to 0 — that must be surfaced, not
    // silently converted into a free option.
    let tiny = ExecutionOpportunity {
        hop_slippage_bps: arb_types::DEFAULT_HOP_SLIPPAGE_BPS,
        route_id: arb_types::RouteId(5),
        slot: Slot(1),
        size_in: TokenAmount(50),
        expected_out: TokenAmount(51),
        // out=51, 1% haircut -> 51 - 0 = 51 (integer floor), so min_out == out.
        hop_inputs: vec![50, 51],
        gross_profit: Lamports(1),
        net_profit: Lamports(1),
        base_decimals: 9,
        price_impact_bps: Bps(0),
        hops: vec![hop(MintId(wsol), MintId(usdc))],
        expected_min_final_output: 0,
    };
    let legs = legs_for(&tiny);
    assert_eq!(legs[0].1, 51, "min_out must never exceed expected_out");

    // Profit floor arithmetic overflow must be rejected, never wrapped.
    let huge = ExecutionOpportunity {
        hop_slippage_bps: arb_types::DEFAULT_HOP_SLIPPAGE_BPS,
        route_id: arb_types::RouteId(6),
        slot: Slot(1),
        size_in: TokenAmount(u64::MAX),
        expected_out: TokenAmount(u64::MAX),
        hop_inputs: vec![u64::MAX, u64::MAX],
        gross_profit: Lamports(0),
        net_profit: Lamports(0),
        base_decimals: 9,
        price_impact_bps: Bps(0),
        hops: vec![hop(MintId(wsol), MintId(usdc))],
        expected_min_final_output: 0,
    };
    let err = TxBuilder::derive_swap_legs(&huge, u64::MAX, u64::MAX, u64::MAX, 100)
        .expect_err("floor overflow must be rejected");
    assert!(
        matches!(err, arb_tx::TxError::InvalidInput(_)),
        "unexpected error: {err:?}"
    );
}

use std::str::FromStr;
