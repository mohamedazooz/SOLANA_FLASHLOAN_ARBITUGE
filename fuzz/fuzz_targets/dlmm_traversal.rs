#![no_main]
//! Fuzzes DLMM bin traversal: active-bin and multi-bin max input. Invariants:
//! no panic, results are finite, multi-bin max >= active-bin-only max when the
//! active bin exists and both are Some.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() < 32 {
        return;
    }
    let le32 = |off: usize| -> i32 {
        let mut buf = [0u8; 4];
        buf.copy_from_slice(&data[off..off + 4]);
        i32::from_le_bytes(buf) % 1_000_000
    };
    let le64 = |off: usize| -> u64 {
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&data[off..off + 8]);
        u64::from_le_bytes(buf)
    };
    let active_id = le32(0);
    let bin_step_bps = u16::from_le_bytes([data[4], data[5]]) % 1_001;
    let amount_x = le64(8);
    let amount_y = le64(16);

    let bins: Vec<arb_math::DlmmBin> = (0..8)
        .filter_map(|i| {
            let off = 24 + i * 8;
            if off + 8 > data.len() {
                return None;
            }
            let raw = u64::from_le_bytes(data[off..off + 8].try_into().ok()?);
            Some(arb_math::DlmmBin {
                id: active_id + i32::from(i as i8) * 2,
                amount_x: if i == 0 { amount_x } else { (raw & 0xFFFF) as u64 },
                amount_y: if i == 0 { amount_y } else { ((raw >> 16) & 0xFFFF) as u64 },
                price_q64: 0,
                open_order_amount: 0,
                processed_order_remaining_amount: 0,
                limit_order_ask_side: false,
            })
        })
        .collect();

    let single = arb_math::dlmm::dlmm_active_bin_max_input(
        active_id,
        bin_step_bps,
        amount_x,
        amount_y,
        true,
    );
    let multi = arb_math::dlmm::dlmm_multi_bin_max_input(active_id, bin_step_bps, &bins, true);
    if let (Some(s), Some(m)) = (single, multi) {
        assert!(m >= s, "multi-bin max input must cover the active bin alone");
    }
});
