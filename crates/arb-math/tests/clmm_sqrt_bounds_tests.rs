//! Boundary parity and per-protocol sqrt-price bounds for the shared CLMM math.
//!
//! Pins, with exact integer expectations:
//! 1. The shared envelope `MIN_SQRT_PRICE_X64..=MAX_SQRT_PRICE_X64` matches the
//!    module's own conversion endpoints.
//! 2. The Orca boundary delta vs. the official `orca_whirlpools_core` (pinned
//!    `=2.1.1` dev-dependency) stays within the 16-unit tolerance.
//! 3. Per-program official bounds (Orca / Raydium CLMM / Meteora DAMM v2) and
//!    the fail-closed per-protocol guard functions, verified 2026-11-19 against
//!    the official program sources.

use arb_constants::{
    CLMM_MIN_SQRT_PRICE_X64, METEORA_DAMM_V2_MAX_SQRT_PRICE_X64, ORCA_WHIRLPOOL_MAX_SQRT_PRICE_X64,
    RAYDIUM_CLMM_MAX_SQRT_PRICE_X64,
};
use arb_math::clmm::{
    ClmmMathError, MAX_SQRT_PRICE_X64, MAX_TICK_INDEX, MIN_SQRT_PRICE_X64, MIN_TICK_INDEX,
    sqrt_price_x64_in_meteora_damm_v2_bounds, sqrt_price_x64_in_orca_whirlpool_bounds,
    sqrt_price_x64_in_raydium_clmm_bounds, sqrt_price_x64_to_tick_index,
    tick_index_to_sqrt_price_x64,
};
use orca_whirlpools_core::MAX_SQRT_PRICE as ORCA_CORE_MAX_SQRT_PRICE;

#[test]
fn tick_conversion_endpoints_match_shared_envelope_exactly() {
    assert_eq!(
        tick_index_to_sqrt_price_x64(MIN_TICK_INDEX).expect("min tick conversion"),
        MIN_SQRT_PRICE_X64
    );
    assert_eq!(
        tick_index_to_sqrt_price_x64(MAX_TICK_INDEX).expect("max tick conversion"),
        MAX_SQRT_PRICE_X64
    );
    assert_eq!(MAX_SQRT_PRICE_X64, 79_226_673_515_401_279_992_447_579_061);
}

#[test]
fn max_endpoint_stays_within_orca_core_official_bound_plus_tolerance() {
    let local_max = tick_index_to_sqrt_price_x64(MAX_TICK_INDEX).expect("max tick conversion");
    assert!(
        local_max > ORCA_CORE_MAX_SQRT_PRICE,
        "guard against accidental constant drift: local endpoint must stay above pinned core bound"
    );
    let delta = local_max - ORCA_CORE_MAX_SQRT_PRICE;
    assert_eq!(delta, 6);
    assert!(delta <= 16);
    assert_eq!(ORCA_CORE_MAX_SQRT_PRICE, ORCA_WHIRLPOOL_MAX_SQRT_PRICE_X64);
}

#[test]
fn sharing_envelope_never_accepts_above_any_official_program_bound() {
    let local_max = tick_index_to_sqrt_price_x64(MAX_TICK_INDEX).expect("max tick conversion");
    assert!(local_max <= ORCA_WHIRLPOOL_MAX_SQRT_PRICE_X64 + 16);
    assert!(local_max < RAYDIUM_CLMM_MAX_SQRT_PRICE_X64);
    assert!(local_max < METEORA_DAMM_V2_MAX_SQRT_PRICE_X64);
    assert!(RAYDIUM_CLMM_MAX_SQRT_PRICE_X64 > ORCA_WHIRLPOOL_MAX_SQRT_PRICE_X64);
    assert_eq!(
        RAYDIUM_CLMM_MAX_SQRT_PRICE_X64,
        METEORA_DAMM_V2_MAX_SQRT_PRICE_X64
    );
    assert_eq!(MIN_SQRT_PRICE_X64, CLMM_MIN_SQRT_PRICE_X64);
}

#[test]
fn off_by_one_outside_official_bounds_is_rejected_per_protocol() {
    assert!(sqrt_price_x64_in_orca_whirlpool_bounds(
        ORCA_WHIRLPOOL_MAX_SQRT_PRICE_X64
    ));
    assert!(!sqrt_price_x64_in_orca_whirlpool_bounds(
        ORCA_WHIRLPOOL_MAX_SQRT_PRICE_X64 + 1
    ));
    assert!(sqrt_price_x64_in_raydium_clmm_bounds(
        RAYDIUM_CLMM_MAX_SQRT_PRICE_X64
    ));
    assert!(!sqrt_price_x64_in_raydium_clmm_bounds(
        RAYDIUM_CLMM_MAX_SQRT_PRICE_X64 + 1
    ));
    assert!(sqrt_price_x64_in_meteora_damm_v2_bounds(
        METEORA_DAMM_V2_MAX_SQRT_PRICE_X64
    ));
    assert!(!sqrt_price_x64_in_meteora_damm_v2_bounds(
        METEORA_DAMM_V2_MAX_SQRT_PRICE_X64 + 1
    ));
    assert!(!sqrt_price_x64_in_orca_whirlpool_bounds(
        CLMM_MIN_SQRT_PRICE_X64 - 1
    ));
    assert!(!sqrt_price_x64_in_raydium_clmm_bounds(0));
    assert!(!sqrt_price_x64_in_meteora_damm_v2_bounds(0));
}

#[test]
fn inverse_mapping_rejects_values_above_the_shared_envelope() {
    assert_eq!(
        sqrt_price_x64_to_tick_index(MAX_SQRT_PRICE_X64).expect("envelope max maps to max tick"),
        MAX_TICK_INDEX
    );
    assert_eq!(
        sqrt_price_x64_to_tick_index(MAX_SQRT_PRICE_X64 + 1),
        Err(ClmmMathError::SqrtPriceOutOfBounds)
    );
    assert_eq!(
        sqrt_price_x64_to_tick_index(RAYDIUM_CLMM_MAX_SQRT_PRICE_X64),
        Err(ClmmMathError::SqrtPriceOutOfBounds)
    );
    assert_eq!(
        sqrt_price_x64_to_tick_index(MIN_SQRT_PRICE_X64 - 1),
        Err(ClmmMathError::SqrtPriceOutOfBounds)
    );
}

#[test]
fn one_u128_above_envelope_never_wraps_into_bounds() {
    let last = u128::MAX;
    assert!(!sqrt_price_x64_in_orca_whirlpool_bounds(last));
    assert!(!sqrt_price_x64_in_raydium_clmm_bounds(last));
    assert!(!sqrt_price_x64_in_meteora_damm_v2_bounds(last));
}
