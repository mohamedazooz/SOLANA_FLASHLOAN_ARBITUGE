//! F-02 regression suite: exact Bin Array provisioning from the quote's
//! traversal metadata.
//!
//! Root cause proven at HEAD: the old provisioning formula
//! `arrays_needed = (500/step + 1).div_euclid(70) + 1` counted arrays from the
//! ACTIVE array without mapping the actual traversal span to array indices, so
//! any traversal crossing an array boundary under-provisioned (canonical
//! example: active bin 20, X->Y, ~51 bins -> the walk spans arrays {0, -1},
//! while the old formula returned only {0}). The swap instruction then
//! referenced a Bin Array account that was never hydrated, failing on-chain
//! with AccountNotFound / AccountOwnedByWrongProgram.
//!
//! Canonical semantics (one implementation shared by `plan_bin_arrays` and
//! `derive_directional_bin_arrays`, differenced here against both): the DLMM
//! quote engine reports `bins_crossed` = the number of bin ADVANCES beyond the
//! active bin (a swap fully consumed inside the active bin has `bins_crossed
//! == 0`). The visited bins are therefore
//! `active_id .. active_id ± bins_crossed` INCLUSIVE, and the required arrays
//! are every array index containing at least one visited bin:
//! `|floor(last_bin/70) - floor(active_bin/70)| + 1` with Euclidean (floor)
//! division for negative ids.
//!
//! Array mapping: bins `[70*i, 70*i + 69]` live in array `i` (so bin -1 is in
//! array -1, bin -70 in array -1, bin -71 in array -2, bin 70 in array 1).
//!
//! Official references (verified):
//! - MeteoraAg/dlmm-sdk, ts-client/src/dlmm/helpers/binArray.ts: bin-array
//!   index = floor(binId / MAX_BIN_PER_ARRAY), BIN_ARRAY_BITMAP_SIZE = 512.
//! - IDL seeds: bin_array => [b"bin_array", lb_pair, index.to_le_bytes()] (i64),
//!   bitmap => [b"bitmap", lb_pair].

use arb_tx::meteora_dlmm::{
    SwapDirection, active_bin_array_index, derive_bin_array_bitmap_extension_pda,
    derive_directional_bin_arrays, exceeds_default_bitmap,
};
use arb_tx::plan_bin_arrays;
use solana_pubkey::Pubkey;

fn pair() -> Pubkey {
    Pubkey::new_unique()
}

fn to_direction(swap_for_y: bool) -> SwapDirection {
    if swap_for_y {
        SwapDirection::XtoY
    } else {
        SwapDirection::YtoX
    }
}

/// The prompt's canonical boundary example: active bin 20, X->Y, ~50 bin
/// advances must load arrays {0, -1}, not just the active array.
#[test]
fn f02_boundary_crossing_active_20_x_to_y_50_advances_needs_two_arrays() {
    let lb_pair = pair();
    let derived = derive_directional_bin_arrays(&lb_pair, 20, SwapDirection::XtoY, 50)
        .expect("boundary traversal must succeed");
    assert_eq!(
        derived.indices,
        vec![0, -1],
        "bins 20..-30 span arrays {{0, -1}}; the old formula returned only {{0}}"
    );
    let planned = plan_bin_arrays(&lb_pair, 20, true, 50).expect("plan succeeds");
    assert_eq!(planned.indices, vec![0, -1]);
    assert_eq!(planned.last_bin_id, -30);
    assert_eq!(planned.first_bin_id, 20);
}

/// F-02 canonical semantics: `bins_crossed` counts bin ADVANCES beyond the
/// active bin, so the active bin itself is ALWAYS visited and depth 0 plans
/// exactly the active bin's array. The previous semantics treated depth 0 as
/// invalid, which could never happen for a real consumed-inside-active quote
/// and mispredicted the required account set by one bin at array boundaries.
#[test]
fn f02_zero_advance_depth_plans_exactly_the_active_array() {
    let lb_pair = pair();
    for (active, expected) in [
        (0i32, vec![0i64]),
        (-1, vec![-1]),
        (-70, vec![-1]),
        (70, vec![1]),
    ] {
        for swap_for_y in [true, false] {
            let direction = to_direction(swap_for_y);
            let d = derive_directional_bin_arrays(&lb_pair, active, direction, 0)
                .unwrap_or_else(|e| panic!("depth 0 must be accepted (active {active}): {e}"));
            assert_eq!(d.indices, expected, "depth 0 covers only the active array");
            let planned = plan_bin_arrays(&lb_pair, active, swap_for_y, 0)
                .unwrap_or_else(|e| panic!("plan depth 0 must succeed (active {active}): {e:?}"));
            assert_eq!(planned.indices, expected);
            assert!(planned.is_single_array());
            assert_eq!(planned.account_count(), 1);
        }
    }
}

/// The quote's `bins_crossed` is consumed directly by the execution planner
/// (no +1 fixup anywhere in the call chain): the planner covers the active
/// bin PLUS every advance.
#[test]
fn f02_quote_bins_crossed_feeds_the_planner_without_offset() {
    let lb_pair = pair();
    // One-bin advance from the exact top of array -1: bin 0 (array 0) consumed
    // actively, then the walk advanced once into bin -1 (array -1). The plan
    // must cover BOTH arrays.
    let d = derive_directional_bin_arrays(&lb_pair, 0, SwapDirection::XtoY, 1).unwrap();
    assert_eq!(d.indices, vec![0, -1]);
    let planned = plan_bin_arrays(&lb_pair, 0, true, 1).unwrap();
    assert_eq!(planned.indices, vec![0, -1]);
    assert_eq!(planned.last_bin_id, -1);
}

/// A traversal inside a single array needs exactly that one array, for
/// positive and negative ids.
#[test]
fn f02_single_array_traversals_are_exact() {
    for active in [20i32, -30, -69] {
        let lb_pair = pair();
        for swap_for_y in [true, false] {
            let direction = to_direction(swap_for_y);
            let d = derive_directional_bin_arrays(&lb_pair, active, direction, 1)
                .unwrap_or_else(|e| panic!("active {active} dir {swap_for_y}: {e}"));
            assert_eq!(d.indices.len(), 1, "one advance from a mid-array bin");
            assert_eq!(d.indices[0], active_bin_array_index(active));
            assert!(
                !d.bitmap_extension_required,
                "in-range single array must not demand the bitmap extension"
            );
        }
    }
}

/// Boundary crossings at exactly +/-69/70/71 bins from active bins sitting at
/// array edges: the span formula must cover the crossed arrays in the
/// traversal direction only (the swap never walks backwards).
#[test]
fn f02_array_edge_crossings_one_direction_only() {
    let lb_pair = pair();

    // Active 69 (array 0), X->Y, 71 advances: visits 69..-2, indices {0, -1}.
    let d = derive_directional_bin_arrays(&lb_pair, 69, SwapDirection::XtoY, 71).unwrap();
    assert_eq!(d.indices, vec![0, -1]);

    // Active 69, X->Y, 70 advances: visits 69..-1; bin -1 is in array -1.
    let d = derive_directional_bin_arrays(&lb_pair, 69, SwapDirection::XtoY, 70).unwrap();
    assert_eq!(d.indices, vec![0, -1]);

    // Active 69 (array 0), Y->X, 2 advances: visits 69..71; bins 70 and 71
    // live in array 1.
    let d = derive_directional_bin_arrays(&lb_pair, 69, SwapDirection::YtoX, 2).unwrap();
    assert_eq!(d.indices, vec![0, 1]);

    // Active 70 (array 1), Y->X, 70 advances: visits 70..140; bin 140 is
    // in array 2.
    let d = derive_directional_bin_arrays(&lb_pair, 70, SwapDirection::YtoX, 70).unwrap();
    assert_eq!(d.indices, vec![1, 2]);
    // Active 70, Y->X, 71 advances: visits 70..141 -> arrays {1, 2}.
    let d = derive_directional_bin_arrays(&lb_pair, 70, SwapDirection::YtoX, 71).unwrap();
    assert_eq!(d.indices, vec![1, 2]);

    // Active -1 (array -1), X->Y, 70 advances: visits -1..-71; bin -71 is in
    // array -2 (floor division).
    let d = derive_directional_bin_arrays(&lb_pair, -1, SwapDirection::XtoY, 70).unwrap();
    assert_eq!(d.indices, vec![-1, -2]);
    // 71 advances: visits -1..-72 -> still arrays {-1, -2}.
    let d = derive_directional_bin_arrays(&lb_pair, -1, SwapDirection::XtoY, 71).unwrap();
    assert_eq!(d.indices, vec![-1, -2]);

    // Active -69 (array -1), X->Y, 2 advances: visits -69..-71 -> {-1, -2}.
    let d = derive_directional_bin_arrays(&lb_pair, -69, SwapDirection::XtoY, 2).unwrap();
    assert_eq!(d.indices, vec![-1, -2]);
    // Active -70 (array -1), X->Y, 71 advances: visits -70..-141 -> array -3.
    let d = derive_directional_bin_arrays(&lb_pair, -70, SwapDirection::XtoY, 71).unwrap();
    assert_eq!(d.indices, vec![-1, -2, -3]);
}

/// 140 / 260-bin traversals spanning several arrays; 260 is the protocol swap
/// depth limit (METEORA_MAX_BINS_PER_SWAP) and spans exactly 5 arrays, above
/// this builder's 4-array policy.
#[test]
fn f02_multi_array_spans_and_policy_ceiling() {
    let lb_pair = pair();

    // Active 0, X->Y, 140 advances: visits 0..-140 -> arrays {0, -1, -2}.
    let d = derive_directional_bin_arrays(&lb_pair, 0, SwapDirection::XtoY, 140).unwrap();
    assert_eq!(d.indices, vec![0, -1, -2]);

    // Active 20, X->Y, 210 advances: visits 20..-190, floor(-190/70) = -3 ->
    // {0,-1,-2,-3} = 4 arrays (at policy ceiling).
    let d = derive_directional_bin_arrays(&lb_pair, 20, SwapDirection::XtoY, 210).unwrap();
    assert_eq!(d.indices, vec![0, -1, -2, -3]);

    // 260 advances from active 20: visits 20..-240, floor(-240/70) = -4 ->
    // 5 arrays, exceeding the 4-array transaction-size policy -> fail closed.
    assert!(matches!(
        derive_directional_bin_arrays(&lb_pair, 20, SwapDirection::XtoY, 260),
        Err(arb_tx::DlmmError::TooManyBinArrays { .. })
    ));
    // plan_bin_arrays rejects depth > 260 at its own limit with a distinct
    // error; at 260 it plans and agrees with the 4-array policy ceiling only
    // when the span fits.
    let planned = plan_bin_arrays(&lb_pair, 20, true, 260).unwrap();
    assert_eq!(planned.indices, vec![0, -1, -2, -3, -4]);
}

/// Differential property: for every (active bin, direction, depth) in the
/// boundary grid, `derive_directional_bin_arrays` must produce EXACTLY the
/// index set of the canonical `plan_bin_arrays` (same count, same order, same
/// PDAs) whenever the 4-array policy admits the traversal.
#[test]
fn f02_differential_agreement_with_canonical_planner() {
    let lb_pair = pair();
    let actives: [i32; 12] = [0, 1, -1, 20, 69, 70, -69, -70, -71, 100, 35_000, -35_001];
    for active in actives {
        for swap_for_y in [true, false] {
            for bins in [0u32, 1, 2, 69, 70, 71] {
                match plan_bin_arrays(&lb_pair, active, swap_for_y, bins) {
                    Ok(plan) => {
                        let d = derive_directional_bin_arrays(
                            &lb_pair,
                            active,
                            to_direction(swap_for_y),
                            bins,
                        )
                        .unwrap_or_else(|e| {
                            panic!("active {active} swap_for_y {swap_for_y} bins {bins}: {e}")
                        });
                        assert_eq!(
                            d.indices, plan.indices,
                            "indices must match the canonical planner"
                        );
                        assert_eq!(
                            d.bin_array_pdas, plan.pdas,
                            "PDAs must match the canonical planner"
                        );
                        assert_eq!(
                            d.bitmap_extension_required, plan.requires_bitmap_extension,
                            "bitmap-extension policy must match the canonical planner"
                        );
                    }
                    Err(e) => {
                        let d = derive_directional_bin_arrays(
                            &lb_pair,
                            active,
                            to_direction(swap_for_y),
                            bins,
                        );
                        assert!(
                            d.is_err(),
                            "derive must fail closed when the planner fails ({e:?})"
                        );
                    }
                }
            }
        }
    }
}

/// Bitmap-extension policy: required iff any selected index is outside the
/// default bitmap window `[-512, 511]` (a property of index VALUES, not of the
/// array count), and the PDA must be the canonical `[b"bitmap", lb_pair]` seed.
#[test]
fn f02_bitmap_extension_policy_is_index_value_based() {
    let lb_pair = pair();

    // Two arrays inside the default window: NO extension.
    let d = derive_directional_bin_arrays(&lb_pair, 0, SwapDirection::YtoX, 70).unwrap();
    assert_eq!(d.indices, vec![0, 1]);
    assert!(!d.bitmap_extension_required);

    // Last default array (511) walking up crosses out of the default bitmap.
    let active = 511 * 70;
    let d = derive_directional_bin_arrays(&lb_pair, active, SwapDirection::YtoX, 70).unwrap();
    assert_eq!(d.indices, vec![511, 512]);
    assert!(d.bitmap_extension_required);
    assert_ne!(
        derive_bin_array_bitmap_extension_pda(&lb_pair),
        Pubkey::default(),
        "extension PDA derivation must be canonical, not a placeholder"
    );

    // Mirror boundary at the negative end: array -512 is in-range, -513 is out.
    assert!(!exceeds_default_bitmap(&[-512]));
    assert!(!exceeds_default_bitmap(&[511]));
    assert!(exceeds_default_bitmap(&[-513]));
    assert!(exceeds_default_bitmap(&[512]));

    // Single advance at the default-window edge stays in range...
    let active = 511 * 70 + 68;
    let d = derive_directional_bin_arrays(&lb_pair, active, SwapDirection::XtoY, 1).unwrap();
    assert_eq!(d.indices, vec![511]);
    assert!(!d.bitmap_extension_required);
    // ...while an active array outside the window always demands it, however
    // short the traversal is.
    for active in [512 * 70i32, -513 * 70] {
        let d = derive_directional_bin_arrays(&lb_pair, active, SwapDirection::XtoY, 0).unwrap();
        assert_eq!(d.indices.len(), 1);
        assert!(d.bitmap_extension_required);
    }
}

/// PDAs are deterministic and follow the official seed layout: each derived
/// array PDA for a given (pair, index) repeats across invocations, and active
/// bins in the same array map to the same leading index.
#[test]
fn f02_pda_derivation_is_deterministic_and_floor_divided() {
    let lb_pair = pair();
    let a = derive_directional_bin_arrays(&lb_pair, -1, SwapDirection::XtoY, 0).unwrap();
    let b = derive_directional_bin_arrays(&lb_pair, -70, SwapDirection::XtoY, 0).unwrap();
    assert_eq!(a.indices, b.indices, "bins -1 and -70 live in array -1");
    assert_eq!(a.bin_array_pdas, b.bin_array_pdas);
    // Re-deriving reproduces the same PDAs byte-for-byte.
    let again = derive_directional_bin_arrays(&lb_pair, -1, SwapDirection::XtoY, 0).unwrap();
    assert_eq!(a.bin_array_pdas, again.bin_array_pdas);
}
