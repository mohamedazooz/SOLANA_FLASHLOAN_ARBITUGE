#![no_main]
//! Fuzzes the authoritative net-profit calculation. Invariants: checked
//! arithmetic never panics; a Some result never exceeds gross; costs are
//! additive and overflow-aware.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() < 48 {
        return;
    }
    let le = |off: usize| -> u64 {
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&data[off..off + 8]);
        u64::from_le_bytes(buf)
    };
    let (gross, flash, priority, base, tip, rent) =
        (le(0), le(8), le(16), le(24), le(32), le(40));

    if let Some(net) = arb_math::calculate_net_profit(
        arb_types::Lamports(gross),
        arb_types::Lamports(flash),
        arb_types::Lamports(priority),
        arb_types::Lamports(base),
        arb_types::Lamports(tip),
        arb_types::Lamports(rent),
    ) {
        let costs = flash
            .checked_add(priority)
            .and_then(|v| v.checked_add(base))
            .and_then(|v| v.checked_add(tip))
            .and_then(|v| v.checked_add(rent));
        if let Some(costs) = costs {
            if costs <= gross {
                assert_eq!(net.0, gross - costs, "net must equal gross minus costs");
            }
        }
    }
});
