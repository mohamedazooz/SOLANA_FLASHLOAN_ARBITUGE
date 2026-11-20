//! Regression tests for the official Orca `FixedTickArray` binary layout and
//! for swap-loop behaviour the on-chain program exhibits but the old quote path
//! did not.
//!
//! Live mainnet layout (verified 2026-11-19 against three tick arrays of
//! `Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE`):
//!   [0..8]   Anchor discriminator
//!   [8..12]  start_tick_index (i32 LE)
//!   [12..9956] 88 packed ticks, 113 bytes each
//!   [9956..9988] whirlpool Pubkey
//!
//! The previous decoder read `start_tick_index` at `[40..44]` and ticks at
//! `44 + i * 113` (the Raydium CLMM layout), silently producing a bogus array
//! for every Orca pool.

use arb_math::clmm::{
    ClmmTickMath, MAX_TICK_INDEX, MIN_TICK_INDEX, TICK_ARRAY_SIZE, TickArrayFacade, TickFacade,
    WHIRLPOOL_TICK_ARRAY_SIZE, compute_swap, is_valid_tick_array_start_tick,
    parse_tick_array_account, tick_index_to_sqrt_price_x64,
};

fn build_orca_tick_array(start_tick: i32, spacing: i32, ticks: &[(i32, i128, u128)]) -> Vec<u8> {
    let mut data = vec![0u8; WHIRLPOOL_TICK_ARRAY_SIZE];
    data[0..8].copy_from_slice(&[0x45, 0x61, 0xbd, 0xbe, 0x6e, 0x07, 0x42, 0xbb]);
    data[8..12].copy_from_slice(&start_tick.to_le_bytes());
    for (tick_index, liquidity_net, liquidity_gross) in ticks {
        let i = ((tick_index - start_tick) / spacing) as usize;
        assert!(i < TICK_ARRAY_SIZE, "tick {tick_index} outside array");
        let offset = 12 + i * 113;
        data[offset] = 1;
        data[offset + 1..offset + 17].copy_from_slice(&liquidity_net.to_le_bytes());
        data[offset + 17..offset + 33].copy_from_slice(&liquidity_gross.to_le_bytes());
    }
    // Trailing whirlpool key (content is not validated by the parser).
    for (i, byte) in data[9956..9988].iter_mut().enumerate() {
        *byte = (i as u8).wrapping_mul(7).wrapping_add(3);
    }
    data
}

#[test]
fn decodes_official_fixed_tick_array_offsets() {
    let start = -26_400;
    let mut data = build_orca_tick_array(
        start,
        4,
        &[
            (-26_400, -261_559_516_025, 858_992_910_299),
            (-26_396, 53_612_935_351, 53_612_935_351),
        ],
    );

    // Poison the old (Raydium) offsets: the decoder must not read them.
    data[40..44].copy_from_slice(&0i32.to_le_bytes());

    let parsed = parse_tick_array_account(&data, 4).expect("valid Orca tick array");
    assert_eq!(parsed.start_tick_index, start);
    assert_eq!(parsed.ticks.len(), TICK_ARRAY_SIZE);
    assert_eq!(parsed.ticks[0].tick_index, start);
    assert_eq!(parsed.ticks[0].liquidity_net, -261_559_516_025);
    assert_eq!(parsed.ticks[0].liquidity_gross, 858_992_910_299);
    assert_eq!(parsed.ticks[1].tick_index, start + 4);
    assert!(parsed.ticks[0].initialized);
    assert!(!parsed.ticks[2].initialized);
}

#[test]
fn rejects_non_multiple_start_tick() {
    let mut data = build_orca_tick_array(0, 4, &[]);
    data[8..12].copy_from_slice(&3i32.to_le_bytes());
    assert!(parse_tick_array_account(&data, 4).is_err());
}

#[test]
fn left_edge_tick_array_start_is_valid_like_the_program() {
    // Official: MIN_TICK_INDEX - (MIN_TICK_INDEX % ticks_in_array + ticks_in_array)
    // for spacing 4 -> -443636 - (-116 + 352) = -443872.
    assert!(is_valid_tick_array_start_tick(-443_872, 4));
    assert!(!is_valid_tick_array_start_tick(-444_224, 4));
    assert!(is_valid_tick_array_start_tick(0, 4));
    assert!(is_valid_tick_array_start_tick(443_520, 4));
    assert!(!is_valid_tick_array_start_tick(443_636, 4));
    assert!(!is_valid_tick_array_start_tick(0, 0));
}

#[test]
fn zero_liquidity_gap_is_crossed_for_free_not_aborted() {
    // Current price at tick 0 with zero active liquidity. The next initialized
    // tick at -64 injects liquidity; the swap must jump to it and then trade.
    let p0 = tick_index_to_sqrt_price_x64(0).expect("tick 0");
    let spacing = 64u16;
    let injected = 10_000_000_000_000u128;
    let tick_arrays = vec![
        TickArrayFacade {
            start_tick_index: 0,
            ticks: vec![],
        },
        TickArrayFacade {
            start_tick_index: -5_632,
            ticks: vec![TickFacade {
                tick_index: -64,
                initialized: true,
                liquidity_net: -(injected as i128),
                liquidity_gross: injected,
                limit_order_unfilled: 0,
            }],
        },
    ];

    let quote = compute_swap(
        p0,
        0,
        0, // zero active liquidity at the start
        spacing,
        500,
        1_000_000_000,
        true,
        &tick_arrays,
    )
    .expect("zero-liquidity gap must be traversed, not reported as incomplete");

    assert_eq!(quote.final_liquidity, injected);
    assert!(quote.final_tick_index < -64);
    assert!(quote.amount_out_received > 0);
    assert!(quote.steps_taken >= 2);
}

#[test]
fn orca_and_raydium_quote_paths_use_distinct_tick_tables() {
    // At tick 100_000 the two official programs differ by ~4.37e9 Q64.64 units.
    let orca = tick_index_to_sqrt_price_x64(100_000).expect("orca");
    let raydium = ClmmTickMath::Raydium
        .sqrt_price_from_tick(100_000)
        .expect("raydium");
    assert_ne!(orca, raydium);
    assert!(orca.abs_diff(raydium) > 4_000_000_000);

    // Both inverses round-trip their own endpoint.
    assert_eq!(
        ClmmTickMath::Orca
            .tick_from_sqrt_price(orca)
            .expect("orca inverse"),
        100_000
    );
    assert_eq!(
        ClmmTickMath::Raydium
            .tick_from_sqrt_price(raydium)
            .expect("raydium inverse"),
        100_000
    );
}

#[test]
fn extreme_tick_conversion_stays_in_bounds() {
    for tick in [
        MIN_TICK_INDEX,
        MIN_TICK_INDEX + 1,
        MAX_TICK_INDEX - 1,
        MAX_TICK_INDEX,
    ] {
        let orca = tick_index_to_sqrt_price_x64(tick).expect("orca");
        let raydium = ClmmTickMath::Raydium
            .sqrt_price_from_tick(tick)
            .expect("raydium");
        assert!(orca > 0 && raydium > 0);
    }
}
