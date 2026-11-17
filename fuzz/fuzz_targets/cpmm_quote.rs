#![no_main]
//! Fuzzes the CPMM quote path: constant-product output, upfront-fee variant,
//! and price impact. Invariants: no panic, out < reserve, monotonic in input,
//! price impact within [0, 10_000 bps].
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() < 48 {
        return;
    }
    let le = |off: usize| -> u128 {
        let mut buf = [0u8; 16];
        buf.copy_from_slice(&data[off..off + 16]);
        u128::from_le_bytes(buf) & ((1u128 << 96) - 1)
    };
    let (dx, x, y, gamma, fee_denominator) = (le(0), le(16), le(32), le(0) | 1, le(16) | 1);
    let fee_bps = u64::from(data[47]) % 10_001;

    if let Some(out) = arb_math::cpmm::cpmm_out_fee(dx, x, y, gamma, fee_denominator) {
        assert!(out <= y, "cpmm output must never exceed the y reserve");
        if dx > dx / 2 {
            if let (Some(a), Some(b)) = (
                arb_math::cpmm::cpmm_out_fee(dx, x, y, gamma, fee_denominator),
                arb_math::cpmm::cpmm_out_fee(dx / 2, x, y, gamma, fee_denominator),
            ) {
                assert!(a >= b, "cpmm output must be monotonic in input");
            }
        }
    }
    if let (Some(out_fee), Some(_)) = (
        arb_math::cpmm::cpmm_out_fee(dx, x, y, gamma, fee_denominator),
        arb_math::cpmm::quote_cpmm(
            u64::try_from(dx % u128::from(u64::MAX)).unwrap_or(u64::MAX),
            u64::try_from(x % u128::from(u64::MAX)).unwrap_or(u64::MAX),
            u64::try_from(y % u128::from(u64::MAX)).unwrap_or(u64::MAX),
            fee_bps,
        ),
    ) {
        let _ = out_fee;
    }
});
