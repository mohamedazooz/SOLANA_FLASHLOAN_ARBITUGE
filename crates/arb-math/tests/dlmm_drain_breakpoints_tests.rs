//! Engine-verified invariants for `dlmm_drain_breakpoints` (Part E2).
//!
//! For a pool whose bins hold discrete liquidity, the quote output as a
//! function of the fee-included input is piecewise-linear with exact
//! breakpoints at the cumulative inputs that fully drain each traversed bin.
//! These tests verify the enumerator against the quote engine itself:
//!
//! - Quoting `cumulative_input` for breakpoint `k` must drain every bin up to
//!   and including `bin_id` exactly (all output-side layers at zero) and leave
//!   everything beyond untouched.
//! - Quoting `cumulative_input - 1` must leave the bin at that breakpoint
//!   partially filled — the breakpoint is the exact discrete transition.
//! - Empty bins never produce breakpoints and never perturb the fee rate
//!   sequence (the engine skips them without touching the accumulator).

use arb_math::dlmm::{
    DlmmBin, DlmmError, DlmmParameters, DlmmPoolState, DlmmVariableParameters,
    dlmm_breakpoint_seed_sizes, dlmm_drain_breakpoints, quote_dlmm_multi_bin,
};

fn params() -> DlmmParameters {
    DlmmParameters {
        bin_step: 25,
        base_factor: 200,
        filter_period: 60,
        decay_period: 600,
        reduction_factor: 5_000,
        variable_fee_control: 40_000,
        max_volatility_accumulator: 1_100_000,
        protocol_share: 1_000,
        base_fee_power_factor: 1,
        collect_fee_mode: 0,
    }
}

fn pool_at(active_id: i32) -> DlmmPoolState {
    DlmmPoolState {
        active_id,
        bin_step: 25,
        status: 0,
        parameters: params(),
        v_parameters: DlmmVariableParameters {
            volatility_accumulator: 0,
            volatility_reference: 0,
            index_reference: active_id,
            last_update_timestamp: 1_700_000_000,
        },
        support_limit_order: false,
    }
}

/// `swap_for_y = true` walks toward decreasing bin ids, so a forward ladder
/// (active, active-1, active-2, ...) is what the walk drains first.
fn forward_bins(active_id: i32, depth: i32, per_bin_out_side: u64) -> Vec<DlmmBin> {
    (0..depth)
        .map(|d| DlmmBin {
            id: active_id - d,
            amount_x: 0,
            amount_y: per_bin_out_side,
            ..Default::default()
        })
        .collect()
}

fn output_side_amount(bin: &DlmmBin, swap_for_y: bool) -> (u64, u64, u64) {
    if swap_for_y {
        (
            bin.amount_y,
            bin.processed_order_remaining_amount,
            bin.open_order_amount,
        )
    } else {
        (
            bin.amount_x,
            bin.processed_order_remaining_amount,
            bin.open_order_amount,
        )
    }
}

fn assert_drained(bin: &DlmmBin, swap_for_y: bool) {
    let (mm, processed, open) = output_side_amount(bin, swap_for_y);
    assert_eq!(
        (mm, processed, open),
        (0, 0, 0),
        "bin {} must be fully drained",
        bin.id
    );
}

fn assert_has_liquidity(bin: &DlmmBin, swap_for_y: bool) {
    let (mm, processed, open) = output_side_amount(bin, swap_for_y);
    assert!(
        mm + processed + open > 0,
        "bin {} must retain liquidity before its breakpoint",
        bin.id
    );
}

#[test]
fn breakpoints_exactly_match_engine_drain_transitions() {
    let active_id = 100;
    let swap_for_y = true;
    let pool = pool_at(active_id);
    let intact = forward_bins(active_id, 3, 20_000_000);

    let breakpoints = dlmm_drain_breakpoints(&pool, &intact, swap_for_y).expect("breakpoints");
    assert_eq!(
        breakpoints.len(),
        3,
        "one breakpoint per liquidity-bearing bin"
    );
    assert_eq!(
        breakpoints.iter().map(|b| b.bin_id).collect::<Vec<_>>(),
        vec![100, 99, 98]
    );
    for (i, pair) in breakpoints.windows(2).enumerate() {
        assert!(
            pair[1].cumulative_input > pair[0].cumulative_input,
            "breakpoints must be strictly increasing (window {i})"
        );
        assert_eq!(pair[1].bins_drained, pair[0].bins_drained + 1);
    }

    for k in 0..breakpoints.len() {
        // At the breakpoint: every bin up to and including bin k is drained.
        let mut at = forward_bins(active_id, 3, 20_000_000);
        let quote_at = quote_dlmm_multi_bin(
            &pool,
            &mut at,
            breakpoints[k].cumulative_input,
            swap_for_y,
            None,
        )
        .expect("quote at breakpoint");
        for bin in at.iter().take(k + 1) {
            assert_drained(bin, swap_for_y);
        }
        assert_eq!(quote_at.end_active_id, breakpoints[k].bin_id);
        // One unit below: bin k retains liquidity — the exact discrete knee.
        let mut below = forward_bins(active_id, 3, 20_000_000);
        quote_dlmm_multi_bin(
            &pool,
            &mut below,
            breakpoints[k].cumulative_input - 1,
            swap_for_y,
            None,
        )
        .expect("quote below breakpoint");
        for bin in below.iter().take(k) {
            assert_drained(bin, swap_for_y);
        }
        assert_has_liquidity(&below[k], swap_for_y);
    }
}

#[test]
fn breakpoints_skip_empty_bins_without_perturbing_fee_walk() {
    let active_id = 50;
    let swap_for_y = true;
    let pool = pool_at(active_id);
    // Liquidity at 50 and 48; bin 49 is empty in between.
    let intact = vec![
        DlmmBin {
            id: 50,
            amount_x: 0,
            amount_y: 10_000_000,
            ..Default::default()
        },
        DlmmBin {
            id: 49,
            amount_x: 0,
            amount_y: 0,
            ..Default::default()
        },
        DlmmBin {
            id: 48,
            amount_x: 0,
            amount_y: 30_000_000,
            ..Default::default()
        },
    ];

    let breakpoints = dlmm_drain_breakpoints(&pool, &intact, swap_for_y).expect("breakpoints");
    assert_eq!(
        breakpoints.len(),
        2,
        "empty bins must not produce breakpoints"
    );
    assert_eq!(breakpoints[0].bin_id, 50);
    assert_eq!(breakpoints[1].bin_id, 48);
    assert_eq!(breakpoints[1].bins_drained, 2);

    // The second breakpoint must drive the quote through the empty bin and
    // drain bin 48 exactly (engine walk skips empty bins untouched).
    let mut at = intact.clone();
    let quote_at = quote_dlmm_multi_bin(
        &pool,
        &mut at,
        breakpoints[1].cumulative_input,
        swap_for_y,
        None,
    )
    .expect("quote through empty bin");
    assert_drained(&at[0], swap_for_y);
    assert_drained(&at[2], swap_for_y);
    assert_eq!(quote_at.end_active_id, 48);

    let mut below = intact.clone();
    quote_dlmm_multi_bin(
        &pool,
        &mut below,
        breakpoints[1].cumulative_input - 1,
        swap_for_y,
        None,
    )
    .expect("quote below second breakpoint");
    assert_drained(&below[0], swap_for_y);
    assert_has_liquidity(&below[2], swap_for_y);
}

#[test]
fn breakpoints_consume_every_limit_order_layer_for_supported_pools() {
    let active_id = 10;
    let swap_for_y = true;
    let mut pool = pool_at(active_id);
    pool.support_limit_order = true;
    // X→Y fills bid-side layers (limit_order_ask_side = false).
    let intact = vec![DlmmBin {
        id: active_id,
        amount_x: 0,
        amount_y: 5_000_000,
        open_order_amount: 2_000_000,
        processed_order_remaining_amount: 3_000_000,
        limit_order_ask_side: false,
        price_q64: 0,
    }];

    let breakpoints = dlmm_drain_breakpoints(&pool, &intact, swap_for_y).expect("breakpoints");
    assert_eq!(breakpoints.len(), 1);

    let mut at = intact.clone();
    quote_dlmm_multi_bin(
        &pool,
        &mut at,
        breakpoints[0].cumulative_input,
        swap_for_y,
        None,
    )
    .expect("quote at limit-order breakpoint");
    assert_drained(&at[0], swap_for_y);

    let mut below = intact.clone();
    quote_dlmm_multi_bin(
        &pool,
        &mut below,
        breakpoints[0].cumulative_input - 1,
        swap_for_y,
        None,
    )
    .expect("quote below limit-order breakpoint");
    assert_has_liquidity(&below[0], swap_for_y);
}

#[test]
fn breakpoints_support_the_reverse_direction() {
    let active_id = 7;
    let swap_for_y = false; // walks toward increasing bin ids
    let pool = pool_at(active_id);
    let intact: Vec<DlmmBin> = (0..2)
        .map(|d| DlmmBin {
            id: active_id + d,
            amount_x: 15_000_000,
            amount_y: 0,
            ..Default::default()
        })
        .collect();

    let breakpoints = dlmm_drain_breakpoints(&pool, &intact, swap_for_y).expect("breakpoints");
    assert_eq!(breakpoints.len(), 2);
    assert_eq!(breakpoints[0].bin_id, 7);
    assert_eq!(breakpoints[1].bin_id, 8);

    let mut at = intact.clone();
    quote_dlmm_multi_bin(
        &pool,
        &mut at,
        breakpoints[1].cumulative_input,
        swap_for_y,
        None,
    )
    .expect("quote at reverse-direction breakpoint");
    assert_drained(&at[0], swap_for_y);
    assert_drained(&at[1], swap_for_y);
}

#[test]
fn breakpoints_reject_disabled_and_stop_at_window_end() {
    let active_id = 0;
    let mut pool = pool_at(active_id);
    pool.status = 2;
    let bins = forward_bins(active_id, 2, 1_000_000);
    assert!(matches!(
        dlmm_drain_breakpoints(&pool, &bins, true),
        Err(DlmmError::PoolDisabled)
    ));
}

#[test]
fn breakpoint_seed_sizes_clamp_dedupe_and_spread() {
    let raw = [0, 5, 5, 7, 100, 300, 900, 1_500];
    // Inside the band only; dedupe keeps [5, 7, 100, 300, 900, 1500].
    assert_eq!(
        dlmm_breakpoint_seed_sizes(&raw, 5, 1_500, 64),
        vec![5, 7, 100, 300, 900, 1_500]
    );
    // Out-of-band values (0, 900) are dropped.
    assert_eq!(
        dlmm_breakpoint_seed_sizes(&raw, 10, 1_500, 64),
        vec![100, 300, 900, 1_500]
    );
    // Truncation keeps an evenly spaced sample including both endpoints.
    let wide: Vec<u64> = (1..=200).map(|i| i as u64 * 1_000).collect();
    let seeds = dlmm_breakpoint_seed_sizes(&wide, 1, 200_000, 10);
    assert_eq!(seeds.len(), 10);
    assert_eq!(seeds[0], 1_000);
    assert_eq!(seeds[9], 200_000);
    assert!(seeds.windows(2).all(|w| w[0] < w[1]));
    // Degenerate requests.
    assert!(dlmm_breakpoint_seed_sizes(&raw, 10, 20, 0).is_empty());
    assert!(dlmm_breakpoint_seed_sizes(&raw, 10, 20, 64).is_empty());
}
