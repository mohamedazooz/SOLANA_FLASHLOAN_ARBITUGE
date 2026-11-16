use arb_math::clmm::{MAX_TICK_INDEX, MIN_TICK_INDEX, tick_index_to_sqrt_price_x64};

#[test]
fn tick_to_sqrt_price_stays_within_official_orca_rounding_bound() {
    let ticks = [
        MIN_TICK_INDEX,
        MIN_TICK_INDEX + 1,
        -100_000,
        -1,
        0,
        1,
        100_000,
        MAX_TICK_INDEX - 1,
        MAX_TICK_INDEX,
    ];

    let mut max_difference = 0u128;
    for tick in ticks
        .into_iter()
        .chain((MIN_TICK_INDEX..=MAX_TICK_INDEX).step_by(997))
    {
        let local = tick_index_to_sqrt_price_x64(tick)
            .unwrap_or_else(|error| panic!("local tick conversion failed at {tick}: {error}"));
        let official: u128 = orca_whirlpools_core::tick_index_to_sqrt_price(tick);

        max_difference = max_difference.max(local.abs_diff(official));
    }
    assert!(
        max_difference <= 16,
        "sqrt price differs by {max_difference} Q64.64 units; tolerance is 16"
    );
}
