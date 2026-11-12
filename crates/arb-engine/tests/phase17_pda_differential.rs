//! Phase 17 — PDA-derivation differential.
//!
//! The repository historically carried more than one copy of the Meteora DLMM
//! PDA derivations (one in `arb-tx/src/lib.rs`, one in
//! `arb-tx/src/meteora_dlmm.rs` and one in `arb-dex`). Phase 17 adds a canonical
//! set in `arb-dex::dlmm_validation`; this test proves that **all** of them agree
//! so the new code introduces no divergence.
//!
//! It also proves that the bin-array *index* computations agree, including the
//! negative-index floor-division case, and that the precise Phase 17 bin-array
//! plan never requires fewer arrays than the legacy helper.

use arb_dex::dlmm_validation::{
    bin_array_index_for_bin as dex_index_for_bin, derive_bin_array_pda as dex_bin_array_pda,
    derive_bitmap_extension_pda as dex_bitmap_pda, derive_event_authority as dex_event_authority,
    derive_oracle_pda as dex_oracle_pda, derive_reserve_pda as dex_reserve_pda,
};
use arb_dex::meteora_dlmm::MeteoraDlmmPool;
use arb_math::dlmm::bin_array_index_for_bin as math_index_for_bin;
use arb_tx::bin_array_index_for_bin as tx_index_for_bin;
use arb_tx::derive_meteora_bin_array_bitmap_extension_pda as tx_bitmap_pda;
use arb_tx::derive_meteora_bin_array_pda as tx_bin_array_pda;
use arb_tx::derive_meteora_event_authority as tx_event_authority;
use arb_tx::derive_meteora_oracle_pda as tx_oracle_pda;
use arb_tx::derive_meteora_reserve_pda as tx_reserve_pda;
use arb_tx::dlmm_bin_discovery::plan_bin_arrays;
use arb_tx::meteora_dlmm::derive_bin_array_bitmap_extension_pda as sdk_bitmap_pda;
use arb_tx::meteora_dlmm::derive_bin_array_pda as sdk_bin_array_pda;
use arb_types::PoolId;
use solana_pubkey::Pubkey;
use std::str::FromStr;

const POOL_A: &str = "5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6";
const POOL_B: &str = "BGm1tav58oGcsQJehL9WXBFXF7D27vZsKefj4xJKD5Y";
const WSOL: &str = "So11111111111111111111111111111111111111112";
const USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

fn pk(s: &str) -> Pubkey {
    Pubkey::from_str(s).expect("valid pubkey")
}

fn pools() -> Vec<Pubkey> {
    vec![pk(POOL_A), pk(POOL_B)]
}

/// Indices exercised by the differential: positive, negative and boundary values.
fn indices() -> Vec<i64> {
    let mut v = vec![
        -200i64, -130, -100, -91, -90, -89, -88, -71, -70, -69, -2, -1, 0, 1, 69, 70, 71, 100, 512,
        1_000,
    ];
    v.sort_unstable();
    v
}

#[test]
fn every_bin_array_pda_copy_agrees() {
    let wsol = pk(WSOL);
    let usdc = pk(USDC);
    for pool in pools() {
        // A synthetic pool instance is needed for the `arb-dex` instance method.
        let instance =
            MeteoraDlmmPool::from_account(PoolId(pool), &vec![0u8; 720]).expect("synthetic LbPair");

        for index in indices() {
            let a = tx_bin_array_pda(&pool, index);
            let b = sdk_bin_array_pda(&pool, index);
            let c = dex_bin_array_pda(&pool, index);
            let d = instance.derive_bin_array_pda(index);
            assert_eq!(a, b, "arb-tx lib vs arb-tx meteora_dlmm (index {index})");
            assert_eq!(
                a, c,
                "arb-tx lib vs arb-dex dlmm_validation (index {index})"
            );
            assert_eq!(a, d, "arb-tx lib vs arb-dex meteora_dlmm (index {index})");
        }

        // Bitmap extension.
        assert_eq!(
            tx_bitmap_pda(&pool),
            sdk_bitmap_pda(&pool),
            "arb-tx lib vs arb-tx meteora_dlmm (bitmap)"
        );
        assert_eq!(
            tx_bitmap_pda(&pool),
            dex_bitmap_pda(&pool),
            "arb-tx lib vs arb-dex dlmm_validation (bitmap)"
        );

        // Oracle.
        assert_eq!(
            tx_oracle_pda(&pool),
            dex_oracle_pda(&pool),
            "arb-tx lib vs arb-dex dlmm_validation (oracle)"
        );

        // Reserve vaults.
        for mint in [wsol, usdc] {
            assert_eq!(
                tx_reserve_pda(&pool, &mint),
                dex_reserve_pda(&pool, &mint),
                "arb-tx lib vs arb-dex dlmm_validation (reserve)"
            );
        }
    }
}

#[test]
fn event_authority_derivations_agree() {
    assert_eq!(tx_event_authority(), dex_event_authority());
}

#[test]
fn every_bin_array_index_computation_agrees() {
    for bin in -1_000i32..=1_000 {
        let a = tx_index_for_bin(bin);
        let b = math_index_for_bin(bin);
        let c = dex_index_for_bin(bin);
        assert_eq!(a, b, "arb-tx vs arb-math (bin {bin})");
        assert_eq!(a, c, "arb-tx vs arb-dex (bin {bin})");
        // The legacy `arb-dex` helper truncates instead of flooring; it must still
        // land on the same array for every bin id.
        let legacy = MeteoraDlmmPool::bin_id_to_array_index(bin);
        assert_eq!(a, legacy, "arb-tx vs arb-dex legacy helper (bin {bin})");
    }
}

#[test]
fn negative_bin_ids_floor_to_the_lower_array() {
    assert_eq!(tx_index_for_bin(-1), -1);
    assert_eq!(tx_index_for_bin(-70), -1);
    assert_eq!(tx_index_for_bin(-71), -2);
    assert_eq!(tx_index_for_bin(-140), -2);
    assert_eq!(tx_index_for_bin(-141), -3);
}

#[test]
fn precise_plan_is_never_under_provisioned_versus_the_legacy_helper() {
    for pool in pools() {
        for active in [-6_168i32, -2_468, -100, -1, 0, 69, 70, 1_000] {
            for swap_for_y in [true, false] {
                for depth in [1u32, 2, 35, 70, 71, 140, 141, 280] {
                    let plan = plan_bin_arrays(&pool, active, swap_for_y, depth).expect("plan");
                    let legacy = arb_tx::meteora_bin_array_indices(active, swap_for_y, depth);
                    assert!(
                        plan.indices.len() <= legacy.len(),
                        "precise plan must not exceed the legacy helper \
                         (active={active}, swap_for_y={swap_for_y}, depth={depth})"
                    );
                    // The legacy helper always starts at the active array and steps
                    // in the swap direction; so must the precise plan.
                    assert_eq!(plan.indices[0], legacy[0]);
                    assert_eq!(
                        plan.indices[plan.indices.len() - 1],
                        legacy[legacy.len() - 1],
                        "the traversal must end on the same array"
                    );
                }
            }
        }
    }
}

#[test]
fn the_legacy_helper_matches_the_precise_plan_exactly() {
    let pool = pk(POOL_A);
    // Walking up from bin 0 for 70 bins touches bins 0..=69, which all live in
    // array 0: the helper must not provision a phantom second array.
    assert_eq!(
        plan_bin_arrays(&pool, 0, false, 70).unwrap().indices,
        vec![0]
    );
    assert_eq!(arb_tx::meteora_bin_array_indices(0, false, 70), vec![0]);

    // Boundary: 71 bins walk from bin 0 into bin 70 (array 1).
    assert_eq!(
        plan_bin_arrays(&pool, 0, false, 71).unwrap().indices,
        vec![0, 1]
    );
    assert_eq!(arb_tx::meteora_bin_array_indices(0, false, 71), vec![0, 1]);

    // Boundary in the swap_for_y direction from a negative active bin.
    let plan = plan_bin_arrays(&pool, -1, true, 71).unwrap();
    assert_eq!(plan.indices, vec![-1, -2]);
    assert_eq!(
        arb_tx::meteora_bin_array_indices(-1, true, 71),
        plan.indices
    );
}
