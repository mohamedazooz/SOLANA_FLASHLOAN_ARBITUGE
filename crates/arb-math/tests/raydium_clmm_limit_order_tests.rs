//! Raydium CLMM limit-order traversal tests (Phase 4).
//!
//! VERIFIED facts (deployed program source `raydium-clmm/programs/amm/src/states/tick_state.rs`,
//! checked 2026-11-20):
//! - `TickState` fields at the tail: `order_phase` u64 @116,
//!   `orders_amount` u64 @124..132, `part_filled_orders_remaining` u64
//!   @132..140, `unfilled_ratio_x64` u128 @140..156; `LEN = 168`.
//! - `is_initialized() = has_liquidity() || has_limit_orders()`: a tick with
//!   zero liquidity but unpaid limit orders must still stop the AMM
//!   traversal (on-chain, limit orders match at the exact tick price).

use arb_math::clmm::{
    ClmmMathError, RAYDIUM_CLMM_TICK_ARRAY_ACCOUNT_SIZE, RAYDIUM_CLMM_TICK_STRUCT_SIZE,
    TickArrayFacade, TickFacade, compute_swap_with_ticks_per_array,
    parse_raydium_clmm_tick_array_account,
};

const SQRT_X64_TICK_0: u128 = 1u128 << 64;
const TICKS_PER_ARRAY: usize = 60;

fn tick_array_bytes(start_tick_index: i32, write: impl Fn(&mut Vec<u8>)) -> Vec<u8> {
    let mut data = vec![0u8; RAYDIUM_CLMM_TICK_ARRAY_ACCOUNT_SIZE];
    data[40..44].copy_from_slice(&start_tick_index.to_le_bytes());
    write(&mut data);
    data
}

fn put_tick(
    data: &mut [u8],
    start_tick_index: i32,
    slot: usize,
    tick_spacing: i32,
    liquidity_net: i128,
    liquidity_gross: u128,
    orders_amount: u64,
    part_filled: u64,
) {
    let tick_index = start_tick_index + (slot as i32) * tick_spacing;
    let offset = 44 + slot * RAYDIUM_CLMM_TICK_STRUCT_SIZE;
    data[offset..offset + 4].copy_from_slice(&tick_index.to_le_bytes());
    data[offset + 4..offset + 20].copy_from_slice(&liquidity_net.to_le_bytes());
    data[offset + 20..offset + 36].copy_from_slice(&liquidity_gross.to_le_bytes());
    data[offset + 124..offset + 132].copy_from_slice(&orders_amount.to_le_bytes());
    data[offset + 132..offset + 140].copy_from_slice(&part_filled.to_le_bytes());
}

#[test]
fn parsed_tick_with_only_limit_orders_is_initialized() {
    let data = tick_array_bytes(0, |d| {
        put_tick(d, 0, 40, 1, 0, 0, 500, 0);
    });
    let parsed = parse_raydium_clmm_tick_array_account(&data, 1).expect("array parses");
    let tick = parsed
        .ticks
        .iter()
        .find(|t| t.tick_index == 40)
        .expect("tick present");
    assert_eq!(tick.liquidity_gross, 0);
    assert_eq!(tick.limit_order_unfilled, 500);
    assert!(
        tick.initialized,
        "official is_initialized(): has_limit_orders()"
    );
}

#[test]
fn parsed_tick_with_liquidity_only_has_zero_unfilled_orders() {
    let data = tick_array_bytes(0, |d| {
        put_tick(d, 0, 10, 1, 1_000, 1_000, 0, 0);
    });
    let parsed = parse_raydium_clmm_tick_array_account(&data, 1).expect("array parses");
    let tick = parsed
        .ticks
        .iter()
        .find(|t| t.tick_index == 10)
        .expect("tick present");
    assert!(tick.initialized);
    assert_eq!(tick.limit_order_unfilled, 0);
}

#[test]
fn parsed_tick_uninitialized_without_liquidity_or_orders() {
    let data = tick_array_bytes(0, |d| {
        put_tick(d, 0, 30, 1, 0, 0, 0, 0);
    });
    let parsed = parse_raydium_clmm_tick_array_account(&data, 1).expect("array parses");
    let tick = parsed
        .ticks
        .iter()
        .find(|t| t.tick_index == 30)
        .expect("tick present");
    assert!(!tick.initialized);
}

#[test]
fn orders_amount_plus_part_filled_overflow_fails_closed() {
    let data = tick_array_bytes(0, |d| {
        put_tick(d, 0, 5, 1, 0, 0, u64::MAX, 1);
    });
    assert_eq!(
        parse_raydium_clmm_tick_array_account(&data, 1).unwrap_err(),
        ClmmMathError::IntegerOverflow
    );
}

#[test]
fn swap_crossing_limit_order_tick_fails_closed() {
    // Liquidity sits below tick 30; the swap (zero_for_one, price moves up)
    // has enough input to reach tick 30 exactly, where an unfilled limit
    // order must abort the pure-AMM traversal.
    let data = tick_array_bytes(0, |d| {
        put_tick(d, 0, 30, 1, 1 << 60, 1 << 60, 1_000_000, 500);
    });
    let array = parse_raydium_clmm_tick_array_account(&data, 1).expect("array parses");
    let err = compute_swap_with_ticks_per_array(
        SQRT_X64_TICK_0,
        0,
        1 << 60,
        1,
        2_500,
        1_000_000_000_000_000_000,
        false,
        &[array],
        TICKS_PER_ARRAY,
    )
    .unwrap_err();
    assert_eq!(err, ClmmMathError::LimitOrderTickInSwapPath(30));
}

#[test]
fn swap_crossing_plain_tick_without_orders_still_quotes() {
    let data = tick_array_bytes(0, |d| {
        put_tick(d, 0, 30, 1, 1 << 60, 1 << 60, 0, 0);
    });
    let array = parse_raydium_clmm_tick_array_account(&data, 1).expect("array parses");
    let quote = compute_swap_with_ticks_per_array(
        SQRT_X64_TICK_0,
        0,
        1 << 60,
        1,
        2_500,
        2_000_000_000_000_000,
        false,
        &[array],
        TICKS_PER_ARRAY,
    )
    .expect("orders-free tick traversal quotes");
    assert!(quote.amount_out_received > 0);
    assert!(quote.steps_taken >= 1);
}

#[test]
fn unfilled_order_tick_below_price_range_does_not_block_quote() {
    // A limit-order tick exists above the traversed range but the input is too
    // small to reach it: no crossing, no forced failure.
    let array = TickArrayFacade {
        start_tick_index: 0,
        ticks: (0..TICKS_PER_ARRAY)
            .map(|i| TickFacade {
                tick_index: i as i32,
                initialized: i == 50,
                liquidity_net: 0,
                liquidity_gross: if i == 50 { 1 << 60 } else { 0 },
                limit_order_unfilled: if i == 50 { 700 } else { 0 },
            })
            .collect(),
    };
    let quote = compute_swap_with_ticks_per_array(
        SQRT_X64_TICK_0,
        0,
        1 << 60,
        1,
        2_500,
        10_000,
        false,
        &[array],
        TICKS_PER_ARRAY,
    )
    .expect("input exhausts before the limit-order tick");
    assert!(quote.amount_out_received > 0);
}
