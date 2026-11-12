//! Phase 17 utility — dump the canonical Meteora DLMM accounts for a set of pools.
//!
//! Prints the derived oracle / reserve / bin-array / bitmap-extension addresses so
//! an operator (or the capture script) can fetch the exact accounts from RPC.
//!
//! ```text
//! cargo run -p arb-tx --example phase17_dump_dlmm_accounts -- \
//!     5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6 \
//!     BGm1tav58oGcsQJehL9WXBFXF7D27vZsKefj4xJKD5Y
//! ```

use arb_tx::dlmm_bin_discovery::plan_bin_arrays;
use solana_pubkey::Pubkey;
use std::str::FromStr;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: phase17_dump_dlmm_accounts <pool> [pool ...]");
        std::process::exit(2);
    }

    for arg in &args {
        let pool = match Pubkey::from_str(arg.trim()) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("invalid pool address {arg}: {e}");
                std::process::exit(2);
            }
        };
        println!("pool {pool}");
        println!(
            "  oracle           {}",
            arb_tx::derive_meteora_oracle_pda(&pool)
        );
        println!(
            "  bitmap_extension {}",
            arb_tx::derive_meteora_bin_array_bitmap_extension_pda(&pool)
        );
        println!(
            "  event_authority  {}",
            arb_tx::derive_meteora_event_authority()
        );
        for idx in -130i64..=130 {
            let pda = arb_tx::derive_meteora_bin_array_pda(&pool, idx);
            println!("  bin_array[{idx:>4}] {pda}");
        }
    }

    // Directional windows for a traversal of 70 bins (one array in each direction).
    for arg in &args {
        let pool = Pubkey::from_str(arg.trim()).expect("valid pool");
        for (active, swap_for_y) in [(0i32, true), (0, false)] {
            if let Ok(plan) = plan_bin_arrays(&pool, active, swap_for_y, 140) {
                println!(
                    "window pool={pool} active={active} swap_for_y={swap_for_y} indices={:?}",
                    plan.indices
                );
            }
        }
    }
}
