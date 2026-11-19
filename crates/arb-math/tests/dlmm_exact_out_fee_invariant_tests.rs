//! Fee-inclusion invariants for `quote_dlmm_exact_out` (Part E1).
//!
//! The official SDK's `maxInAmount` for exact-out swaps does NOT add the
//! trading fee / protocol fee on top of the fee-excluded input — integrators
//! get reverted on-chain unless they add the fee manually
//! (MeteoraAg/dlmm-sdk issues, solana.stackexchange.com/questions/20112).
//!
//! These tests pin, against independently re-derived expected values (pure
//! `U256` integer math written out here, never the engine's internals), that
//! `quote_dlmm_exact_out(...).amount_in` ALWAYS includes every fee:
//!
//! - `CollectFeeMode::OnlyY` (+ X→Y ⇒ fee on output): the required input pays
//!   for the fee-inclusive gross output `amount_out + fee_to_add(amount_out)`.
//! - `CollectFeeMode::InputOnly` (fee on input): the required input is the
//!   fee-excluded conversion input plus `fee_to_add(excluded_input)`.
//! - In both modes `amount_in` is strictly greater than the naive fee-less
//!   conversion input whenever the fee rate is non-zero.
//!
//! Everything is deterministic integer math; no floats, no RPC, no SDK.

use arb_math::dlmm::{
    DlmmBin, DlmmError, DlmmParameters, DlmmPoolState, DlmmVariableParameters,
    compute_fee_from_amount, compute_fee_to_add, compute_total_fee_rate, get_price_from_id,
    quote_dlmm_exact_out, quote_dlmm_multi_bin, update_volatility_accumulator,
};
use ruint::aliases::U256;

const SCALE_OFFSET: usize = 64;

fn params_with_mode(collect_fee_mode: u8) -> DlmmParameters {
    DlmmParameters {
        bin_step: 25,
        base_factor: 200, // base fee = 200 * 25 * 10 * 10^1 = 500_000 / 10^9 = 0.05 %
        filter_period: 60,
        decay_period: 600,
        reduction_factor: 5_000,
        variable_fee_control: 40_000,
        max_volatility_accumulator: 1_100_000,
        protocol_share: 1_000, // 10 % of the trading fee goes to the protocol
        base_fee_power_factor: 1,
        collect_fee_mode,
    }
}

fn pool_at(active_id: i32, collect_fee_mode: u8) -> DlmmPoolState {
    DlmmPoolState {
        active_id,
        bin_step: 25,
        status: 0,
        parameters: params_with_mode(collect_fee_mode),
        v_parameters: DlmmVariableParameters {
            volatility_accumulator: 0,
            volatility_reference: 0,
            index_reference: active_id,
            last_update_timestamp: 1_700_000_000,
        },
        support_limit_order: false,
    }
}

fn single_liquidity_bin(bin_id: i32, out_side_amount: u64) -> Vec<DlmmBin> {
    vec![DlmmBin {
        id: bin_id,
        amount_x: 0,
        amount_y: out_side_amount,
        ..Default::default()
    }]
}

/// Independent re-derivation of the fee-excluded input that produces
/// `gross_out` at `price_q64` (ceiling, X→Y direction: `ceil(out * 2^64 / P)`).
fn excluded_in_for_gross_out(gross_out: u64, price_q64: u128) -> u64 {
    let num = U256::from(gross_out) << SCALE_OFFSET;
    let (quot, rem) = num.div_rem(U256::from(price_q64));
    let value = if rem.is_zero() {
        quot
    } else {
        quot + U256::from(1)
    };
    u64::try_from(value).expect("fee-excluded input fits u64 in these scenarios")
}

/// The fee rate the engine will actually use at `bin_id`: it refreshes the
/// volatility accumulator (walk order mirrored here) before pricing the bin.
fn engine_fee_rate_at(pool: &DlmmPoolState, bin_id: i32) -> u128 {
    let mut v_params = pool.v_parameters;
    update_volatility_accumulator(&mut v_params, &pool.parameters, bin_id)
        .expect("volatility update");
    compute_total_fee_rate(
        &pool.parameters,
        pool.bin_step,
        v_params.volatility_accumulator,
    )
    .expect("fee rate")
}

fn expected_in_fee_on_output(pool: &DlmmPoolState, active_bin: i32, amount_out: u64) -> u64 {
    let price = get_price_from_id(active_bin, pool.bin_step).expect("price");
    let rate = engine_fee_rate_at(pool, active_bin);
    let gross_needed =
        amount_out + compute_fee_to_add(amount_out, rate).expect("fee_to_add on output target");
    excluded_in_for_gross_out(gross_needed, price)
}

fn expected_in_fee_on_input(pool: &DlmmPoolState, active_bin: i32, amount_out: u64) -> u64 {
    let price = get_price_from_id(active_bin, pool.bin_step).expect("price");
    let rate = engine_fee_rate_at(pool, active_bin);
    let excluded = excluded_in_for_gross_out(amount_out, price);
    excluded + compute_fee_to_add(excluded, rate).expect("fee_to_add on excluded input")
}

// ---------------------------------------------------------------------------
// 1. Exact-input equality against the independently derived fee-inclusive value
// ---------------------------------------------------------------------------

#[test]
fn exact_out_fee_on_output_amount_in_matches_manual_fee_inclusive_derivation() {
    let active_id = 5;
    let pool = pool_at(active_id, 1); // CollectFeeMode::OnlyY, X→Y ⇒ fee on output
    let amount_out: u64 = 1_000_000;
    // 5_000_000 yocto-reserve: far above the requested output, no drain.
    let bins = single_liquidity_bin(active_id, 50_000_000);

    let quote = quote_dlmm_exact_out(&pool, &bins, amount_out, true, None)
        .expect("exact-out quote, fee on output");

    let expected = expected_in_fee_on_output(&pool, active_id, amount_out);
    assert_eq!(
        quote.amount_in, expected,
        "amount_in must include the trading fee"
    );
    assert_eq!(quote.amount_out, amount_out);
    assert!(
        quote.fee_amount >= 1,
        "non-zero fee rate must surface a fee"
    );
}

#[test]
fn exact_out_fee_on_input_amount_in_matches_manual_fee_inclusive_derivation() {
    let active_id = 5;
    let pool = pool_at(active_id, 0); // CollectFeeMode::InputOnly ⇒ fee on input
    let amount_out: u64 = 1_000_000;
    let bins = single_liquidity_bin(active_id, 50_000_000);

    let quote = quote_dlmm_exact_out(&pool, &bins, amount_out, true, None)
        .expect("exact-out quote, fee on input");

    let expected = expected_in_fee_on_input(&pool, active_id, amount_out);
    assert_eq!(
        quote.amount_in, expected,
        "amount_in must include the input-side fee"
    );
    assert_eq!(quote.amount_out, amount_out);
    assert!(
        quote.fee_amount >= 1,
        "non-zero fee rate must surface a fee"
    );
}

// ---------------------------------------------------------------------------
// 2. Strict maxInAmount pitfall: fees are never silently omitted
// ---------------------------------------------------------------------------

#[test]
fn exact_out_amount_in_strictly_exceeds_naive_fee_less_input_in_both_fee_modes() {
    for collect_fee_mode in [0u8, 1u8] {
        let active_id = 5;
        let pool = pool_at(active_id, collect_fee_mode);
        let amount_out: u64 = 2_000_000;
        let bins = single_liquidity_bin(active_id, 50_000_000);

        let quote =
            quote_dlmm_exact_out(&pool, &bins, amount_out, true, None).expect("exact-out quote");

        let price = get_price_from_id(active_id, pool.bin_step).expect("price");
        let naive_fee_less = excluded_in_for_gross_out(amount_out, price);
        assert!(
            quote.amount_in > naive_fee_less,
            "mode {collect_fee_mode}: amount_in {} must exceed the fee-less input {naive_fee_less} \
             (SDK maxInAmount pitfall: omitting the fee under-provisions and reverts on-chain)",
            quote.amount_in
        );
    }
}

// ---------------------------------------------------------------------------
// 3. Round-trip: re-running exact-in with amount_in delivers the target
// ---------------------------------------------------------------------------

#[test]
fn exact_out_round_trip_exact_in_delivers_target_within_one_unit_both_fee_modes() {
    for collect_fee_mode in [0u8, 1u8] {
        let active_id = 5;
        let pool = pool_at(active_id, collect_fee_mode);
        let amount_out: u64 = 1_500_000;
        let bins = single_liquidity_bin(active_id, 50_000_000);

        let quote =
            quote_dlmm_exact_out(&pool, &bins, amount_out, true, None).expect("exact-out quote");
        let mut bins_round_trip = single_liquidity_bin(active_id, 50_000_000);
        let round_trip =
            quote_dlmm_multi_bin(&pool, &mut bins_round_trip, quote.amount_in, true, None)
                .expect("exact-in round trip");

        let delivered = round_trip.amount_out as i128;
        let target = amount_out as i128;
        assert!(
            (delivered - target).abs() <= 1,
            "mode {collect_fee_mode}: round trip delivered {delivered}, target {target} (dust ≤ 1)"
        );
        assert!(
            delivered >= target - 1,
            "exact-out must never under-provision beyond rounding dust"
        );
    }
}

// ---------------------------------------------------------------------------
// 4. Fee accounting: total fee covers the protocol share, bin after bin
// ---------------------------------------------------------------------------

#[test]
fn exact_out_fee_amount_covers_protocol_share_and_stays_consistent_multi_bin() {
    let active_id = 100;
    let pool = pool_at(active_id, 1); // fee on output
    // Bin 100 drains after ~4M; the remaining out is taken from bin 99 below.
    let bins = vec![
        DlmmBin {
            id: 100,
            amount_x: 0,
            amount_y: 4_000_000,
            price_q64: 0,
            open_order_amount: 0,
            processed_order_remaining_amount: 0,
            limit_order_ask_side: false,
        },
        DlmmBin {
            id: 99,
            amount_x: 0,
            amount_y: 900_000_000,
            price_q64: 0,
            open_order_amount: 0,
            processed_order_remaining_amount: 0,
            limit_order_ask_side: false,
        },
    ];
    let amount_out: u64 = 3_999_500;

    let quote = quote_dlmm_exact_out(&pool, &bins, amount_out, true, None)
        .expect("multi-bin exact-out quote");

    assert_eq!(quote.amount_out, amount_out);
    assert!(
        quote.fee_amount >= quote.protocol_fee,
        "total fee must cover the protocol share"
    );
    assert_eq!(quote.end_active_id, 99);
    assert_eq!(quote.bins_crossed, 1);
    assert!(quote.fee_amount >= 1);
    // Replicate the per-bin drain of bin 100: full gross minus its output fee.
    let rate_bin100 = engine_fee_rate_at(&pool, 100);
    let gross_bin100 = bins[0].amount_y;
    let net_bin100 =
        gross_bin100 - compute_fee_from_amount(gross_bin100, rate_bin100).expect("fee_from_amount");
    let remaining = amount_out - net_bin100;
    assert!(
        remaining > 0 && remaining < bins[1].amount_y,
        "scenario must straddle the bin boundary (remaining = {remaining})"
    );
    // Bin 99 delivers `remaining` net, fee-inclusive on the output side.
    let rate_bin99 = engine_fee_rate_at(&pool, 99);
    let f1 = compute_fee_to_add(remaining, rate_bin99).expect("fee_to_add for bin 99");
    let price99 = get_price_from_id(99, pool.bin_step).expect("price at 99");
    let in_bin99 = excluded_in_for_gross_out(remaining + f1, price99);
    let price100 = get_price_from_id(100, pool.bin_step).expect("price at 100");
    let in_bin100 = excluded_in_for_gross_out(gross_bin100, price100);
    assert_eq!(
        quote.amount_in,
        in_bin100 + in_bin99,
        "total amount_in must equal the sum of the per-bin fee-inclusive inputs"
    );
}

// ---------------------------------------------------------------------------
// 5. Fail-closed: a window that cannot deliver the target is an error, never a guess
// ---------------------------------------------------------------------------

#[test]
fn exact_out_fails_closed_when_window_has_no_more_liquidity() {
    let active_id = 5;
    let pool = pool_at(active_id, 1);
    // Bin 5 holds 1M; bin 4 exists but is empty; bin 3 is absent entirely.
    let bins = vec![
        DlmmBin {
            id: 5,
            amount_x: 0,
            amount_y: 1_000_000,
            price_q64: 0,
            open_order_amount: 0,
            processed_order_remaining_amount: 0,
            limit_order_ask_side: false,
        },
        DlmmBin {
            id: 4,
            amount_x: 0,
            amount_y: 0,
            price_q64: 0,
            open_order_amount: 0,
            processed_order_remaining_amount: 0,
            limit_order_ask_side: false,
        },
    ];
    let err = quote_dlmm_exact_out(&pool, &bins, 5_000_000, true, None)
        .expect_err("walk past the window end must fail closed");
    assert!(
        matches!(err, DlmmError::BinNotFound(3)),
        "expected BinNotFound(3), got {err:?}"
    );
}

#[test]
fn exact_out_rejects_zero_output_and_disabled_pools() {
    let pool = pool_at(5, 1);
    let bins = single_liquidity_bin(5, 1_000_000);

    assert!(matches!(
        quote_dlmm_exact_out(&pool, &bins, 0, true, None),
        Err(DlmmError::ZeroInput)
    ));

    let mut disabled = pool.clone();
    disabled.status = 1;
    assert!(matches!(
        quote_dlmm_exact_out(&disabled, &bins, 1_000, true, None),
        Err(DlmmError::PoolDisabled)
    ));
}
