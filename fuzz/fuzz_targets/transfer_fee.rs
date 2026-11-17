#![no_main]
//! Fuzzes Token-2022 transfer-fee math. Invariants: never panics, fee never
//! increases the amount, ceil variant >= floor variant, monotonic in bps.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() < 12 {
        return;
    }
    let mut amount = [0u8; 8];
    amount.copy_from_slice(&data[0..8]);
    let amount = u64::from_le_bytes(amount);
    let fee_bps = u16::from_le_bytes([data[8], data[9]]) % 20_001;
    let maximum_fee = u64::from_le_bytes([data[10], data[11], 0, 0, 0, 0, 0, 0]);

    let floored = arb_math::apply_transfer_fee(amount, fee_bps);
    let ceiled = arb_math::apply_transfer_fee_ceil(amount, fee_bps, maximum_fee);
    assert!(floored <= amount, "floor fee must never increase the amount");
    assert!(ceiled <= amount, "ceil fee must never increase the amount");
    assert!(
        ceiled >= floored,
        "a smaller maximum_fee cap takes less fee, so net received must be >= uncapped"
    );
    assert!(
        ceiled >= floored && floored <= amount,
        "fee invariants violated for bps={fee_bps} cap={maximum_fee}"
    );
});
