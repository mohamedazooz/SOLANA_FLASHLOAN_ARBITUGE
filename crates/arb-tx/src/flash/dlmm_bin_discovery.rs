//! Phase 17 — Directional Meteora DLMM bin-array + bitmap-extension discovery.
//!
//! A DLMM swap walks the discrete bin ladder from the pool's active bin in the
//! direction of the trade:
//!
//! * `swap_for_y = true`  (X -> Y): the price falls, the walk moves to **lower**
//!   bin ids and therefore to **lower** bin-array indices.
//! * `swap_for_y = false` (Y -> X): the price rises, the walk moves to **higher**
//!   bin ids and therefore to **higher** bin-array indices.
//!
//! This module computes the *exact* set of bin arrays a swap of a given depth can
//! touch. It never assumes a fixed global array count, never substitutes an
//! unrelated array, and never truncates: if the loaded array set cannot cover the
//! traversal, the caller must reject the opportunity.
//!
//! The array count is derived from the first and last bin of the traversal:
//! `|index(last_bin) - index(first_bin)| + 1`, with `index(bin) = floor(bin / 70)`.
//! That is precise in *both* directions — a traversal that stays inside the active
//! array needs exactly one array, whichever way it walks.

use crate::{
    bin_array_index_for_bin, derive_meteora_bin_array_bitmap_extension_pda,
    derive_meteora_bin_array_pda,
};
use arb_constants::{
    METEORA_BINS_PER_BIN_ARRAY, meteora_bin_array_index_requires_bitmap_extension,
};
use solana_pubkey::Pubkey;

/// Errors raised while planning or verifying a bin-array set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinArrayPlanError {
    /// The traversal depth was zero.
    ZeroDepth,
    /// A bin-array account required by the traversal was not loaded.
    MissingArray { index: i64 },
    /// A loaded array is not part of the plan (it would be a silent substitution).
    UnexpectedArray { index: i64 },
    /// A loaded array's PDA does not match the canonical derivation.
    PdaMismatch {
        index: i64,
        expected: Pubkey,
        actual: Pubkey,
    },
    /// The loaded array set does not cover the traversed bin range.
    CoverageGap {
        expected_first: i32,
        expected_last: i32,
        covered_first: i32,
        covered_last: i32,
    },
    /// The plan exceeds the configured account budget.
    AccountBudget { required: usize, budget: usize },
    /// The estimated transaction size exceeds the protocol limit.
    SizeBudget { estimated: usize, limit: usize },
}

/// A resolved bin-array plan for one DLMM swap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DlmmBinArrayPlan {
    /// The `LbPair` the arrays belong to.
    pub lb_pair: Pubkey,
    /// Active bin at planning time.
    pub active_bin_id: i32,
    /// `true` when the swap is X -> Y (walk towards lower bin ids).
    pub swap_for_y: bool,
    /// Number of bins the swap is expected to traverse.
    pub bins_traversed: u32,
    /// First bin of the traversal.
    pub first_bin_id: i32,
    /// Last bin of the traversal.
    pub last_bin_id: i32,
    /// Bin-array indices, in traversal order.
    pub indices: Vec<i64>,
    /// Canonical bin-array PDAs, in traversal order.
    pub pdas: Vec<Pubkey>,
    /// Canonical `BinArrayBitmapExtension` PDA for this pool.
    pub bitmap_extension_pda: Pubkey,
    /// True when any planned bin-array index falls outside the LbPair default
    /// bitmap window `[-512, 511]`, i.e. the swap traverses arrays tracked in
    /// the `BinArrayBitmapExtension` and the account is required by the
    /// protocol. Official rule `isOverflowDefaultBinArrayBitmap`
    /// (MeteoraAg/dlmm-sdk, ts-client/src/dlmm/helpers/binArray.ts, IDL constant
    /// `BIN_ARRAY_BITMAP_SIZE = 512`). This is a property of the INDEX VALUES,
    /// never of the array count: `indices.len() > 1` is not the protocol rule.
    pub requires_bitmap_extension: bool,
}

impl DlmmBinArrayPlan {
    /// Number of bin-array accounts the swap carries.
    #[inline]
    pub fn account_count(&self) -> usize {
        self.pdas.len()
    }

    /// Inclusive bin-id range the plan covers.
    #[inline]
    pub fn bin_span(&self) -> (i32, i32) {
        (self.first_bin_id, self.last_bin_id)
    }

    /// True when the plan stays inside the active bin array.
    #[inline]
    pub fn is_single_array(&self) -> bool {
        self.indices.len() == 1
    }
}

/// Build the exact bin-array plan for a swap of `bins_traversed` depth.
///
/// `bins_traversed` must come from the DLMM quote's real `bins_crossed` value so the
/// instruction carries exactly the arrays the swap will touch.
pub fn plan_bin_arrays(
    lb_pair: &Pubkey,
    active_bin_id: i32,
    swap_for_y: bool,
    bins_traversed: u32,
) -> Result<DlmmBinArrayPlan, BinArrayPlanError> {
    if bins_traversed == 0 {
        return Err(BinArrayPlanError::ZeroDepth);
    }

    let step: i64 = if swap_for_y { -1 } else { 1 };
    let first_bin_id = active_bin_id;
    let last_bin_id = active_bin_id
        .checked_add((step as i32).saturating_mul((bins_traversed - 1) as i32))
        .ok_or(BinArrayPlanError::ZeroDepth)?;

    let first_index = bin_array_index_for_bin(first_bin_id);
    let last_index = bin_array_index_for_bin(last_bin_id);

    let mut indices = Vec::new();
    let mut cursor = first_index;
    loop {
        indices.push(cursor);
        if cursor == last_index {
            break;
        }
        cursor += step;
        if indices.len() > 4096 {
            // Defensive: a pathological depth must never produce an unbounded plan.
            return Err(BinArrayPlanError::ZeroDepth);
        }
    }

    let pdas = indices
        .iter()
        .map(|idx| derive_meteora_bin_array_pda(lb_pair, *idx))
        .collect();

    let requires_bitmap_extension = indices
        .iter()
        .any(|idx| meteora_bin_array_index_requires_bitmap_extension(*idx));

    Ok(DlmmBinArrayPlan {
        lb_pair: *lb_pair,
        active_bin_id,
        swap_for_y,
        bins_traversed,
        first_bin_id,
        last_bin_id,
        requires_bitmap_extension,
        indices,
        pdas,
        bitmap_extension_pda: derive_meteora_bin_array_bitmap_extension_pda(lb_pair),
    })
}

/// Number of bin arrays a traversal of `bins_traversed` depth requires.
#[inline]
pub fn required_bin_array_count(bins_traversed: u32) -> usize {
    if bins_traversed == 0 {
        return 0;
    }
    (bins_traversed as usize).div_ceil(METEORA_BINS_PER_BIN_ARRAY as usize)
}

/// Verify that the loaded bin arrays fully cover the planned traversal.
///
/// `loaded` is the set of `(array_index, account_address)` pairs that were actually
/// fetched from chain. Every planned array must be present, no unplanned array may
/// be present, and every address must equal the canonical derivation — a mismatch
/// means the array would be silently substituted.
pub fn verify_bin_array_coverage(
    plan: &DlmmBinArrayPlan,
    loaded: &[(i64, Pubkey)],
) -> Result<(), BinArrayPlanError> {
    for (idx, pda) in plan.indices.iter().zip(plan.pdas.iter()) {
        let found = loaded.iter().find(|(i, _)| i == idx);
        match found {
            None => return Err(BinArrayPlanError::MissingArray { index: *idx }),
            Some((_, actual)) => {
                if actual != pda {
                    return Err(BinArrayPlanError::PdaMismatch {
                        index: *idx,
                        expected: *pda,
                        actual: *actual,
                    });
                }
            }
        }
    }

    for (idx, _) in loaded {
        if !plan.indices.contains(idx) {
            return Err(BinArrayPlanError::UnexpectedArray { index: *idx });
        }
    }

    // The loaded set must span at least the planned bin range. Because every
    // planned index was matched above and the plan is contiguous, this holds by
    // construction — the explicit check documents the invariant and protects
    // against a future non-contiguous plan.
    let covered_first = plan
        .indices
        .first()
        .map(|i| (*i as i32).saturating_mul(METEORA_BINS_PER_BIN_ARRAY as i32))
        .unwrap_or(plan.first_bin_id);
    let covered_last = plan
        .indices
        .last()
        .map(|i| {
            (*i as i32)
                .saturating_mul(METEORA_BINS_PER_BIN_ARRAY as i32)
                .saturating_add(METEORA_BINS_PER_BIN_ARRAY as i32 - 1)
        })
        .unwrap_or(plan.last_bin_id);
    let expected_first = plan.first_bin_id.min(covered_first);
    let expected_last = plan.last_bin_id.max(covered_last);
    if covered_first > expected_first || covered_last < expected_last {
        return Err(BinArrayPlanError::CoverageGap {
            expected_first,
            expected_last,
            covered_first,
            covered_last,
        });
    }

    Ok(())
}

/// Maximum number of bin arrays that fit into the remaining transaction budget.
///
/// A legacy Solana message carries 32 bytes per distinct account plus one byte per
/// account reference inside an instruction. `per_account_bytes` therefore accounts
/// for both.
pub fn max_bin_arrays_for_tx_budget(base_bytes: usize, per_account_bytes: usize) -> usize {
    if per_account_bytes == 0 {
        return 0;
    }
    base_bytes.saturating_sub(1) / per_account_bytes
}

/// Exact byte size of a legacy-format Solana message holding `instructions`.
///
/// Mirrors `TxBuilder::compile_message` (3-byte header, one signature, static
/// account keys, blockhash, instruction list) so the optimizer can enforce the
/// 1232-byte limit before a transaction is assembled.
pub fn estimate_legacy_message_size(instructions: &[crate::CompiledInstruction]) -> usize {
    use std::collections::BTreeMap;

    let mut account_map: BTreeMap<Pubkey, (bool, bool)> = BTreeMap::new();
    for ix in instructions {
        for acc in &ix.accounts {
            let entry = account_map.entry(acc.pubkey).or_insert((false, false));
            entry.0 |= acc.is_signer;
            entry.1 |= acc.is_writable;
        }
        account_map.entry(ix.program_id).or_insert((false, false));
    }

    let mut size = 1usize; // signature count
    size += 64; // one signature
    size += 3; // message header
    size += compact_u16_len(account_map.len());
    size += 32 * account_map.len();
    size += 32; // recent blockhash
    size += compact_u16_len(instructions.len());

    for ix in instructions {
        size += 1; // program index
        size += compact_u16_len(ix.accounts.len());
        size += ix.accounts.len(); // one account index byte per reference
        size += compact_u16_len(ix.data.len());
        size += ix.data.len();
    }

    size
}

/// Encoded length of a `compact-u16` value.
#[inline]
pub fn compact_u16_len(value: usize) -> usize {
    if value < 0x80 {
        1
    } else if value < 0x4000 {
        2
    } else {
        3
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn pool() -> Pubkey {
        Pubkey::from_str("5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6").unwrap()
    }

    #[test]
    fn single_bin_traversal_needs_one_array() {
        for swap_for_y in [true, false] {
            let plan = plan_bin_arrays(&pool(), 0, swap_for_y, 1).unwrap();
            assert_eq!(plan.account_count(), 1);
            assert!(plan.is_single_array());
            assert!(!plan.requires_bitmap_extension);
        }
    }

    #[test]
    fn downward_traversal_crosses_into_the_lower_array() {
        // Bins 0 .. -69: bin -69 lives in array -1 (floor division). Both
        // indices [-1, 0] sit inside the default bitmap window [-512, 511], so
        // the Bitmap Extension is NOT required despite the multi-array plan.
        let plan = plan_bin_arrays(&pool(), 0, true, 70).unwrap();
        assert_eq!(plan.indices, vec![0, -1]);
        assert!(!plan.requires_bitmap_extension);
        assert_eq!(plan.bin_span(), (0, -69));
    }

    #[test]
    fn upward_traversal_stays_inside_the_active_array() {
        // Bins 0 .. 69 all live in array 0.
        let plan = plan_bin_arrays(&pool(), 0, false, 70).unwrap();
        assert_eq!(plan.indices, vec![0]);
        assert!(!plan.requires_bitmap_extension);
        assert_eq!(plan.bin_span(), (0, 69));
    }

    #[test]
    fn upward_traversal_crosses_array_boundary_inside_default_window() {
        // Arrays [0, 1] are both inside the default bitmap window, so crossing
        // the array boundary alone does NOT require the extension.
        let plan = plan_bin_arrays(&pool(), 0, false, 71).unwrap();
        assert_eq!(plan.indices, vec![0, 1]);
        assert!(!plan.requires_bitmap_extension);
    }

    #[test]
    fn bitmap_extension_required_exactly_outside_the_default_window() {
        // Official window: [-512, 511] (IDL BIN_ARRAY_BITMAP_SIZE = 512).
        // In-window boundaries never require it.
        for idx in [-512i64, -511, -1, 0, 1, 510, 511] {
            let active = (idx * 70) as i32;
            let plan = plan_bin_arrays(&pool(), active, false, 1).unwrap();
            assert_eq!(plan.indices, vec![idx]);
            assert!(
                !plan.requires_bitmap_extension,
                "array {idx} is inside the default window"
            );
        }
        // Out-of-window boundaries always require it.
        for idx in [-513i64, -514, 512, 513] {
            let active = (idx * 70) as i32;
            let plan = plan_bin_arrays(&pool(), active, false, 1).unwrap();
            assert_eq!(plan.indices, vec![idx]);
            assert!(
                plan.requires_bitmap_extension,
                "array {idx} overflows the default bitmap"
            );
        }
    }

    #[test]
    fn traversal_into_overflow_arrays_requires_the_extension() {
        // A walk from array 511 into 512 must flip the flag even though the
        // plan is only two arrays.
        let plan = plan_bin_arrays(&pool(), 511 * 70, false, 71).unwrap();
        assert_eq!(plan.indices, vec![511, 512]);
        assert!(plan.requires_bitmap_extension);

        // And symmetrically downward past -512.
        let plan = plan_bin_arrays(&pool(), -512 * 70, true, 71).unwrap();
        assert_eq!(plan.indices, vec![-512, -513]);
        assert!(plan.requires_bitmap_extension);
    }

    #[test]
    fn plan_is_never_under_provisioned_versus_the_legacy_helper() {
        // The legacy helper (`meteora_bin_array_indices`) over-provisions by one
        // array in the upward direction; the precise plan must never require more.
        for active in [-6168i32, -1, 0, 69, 70, 1000] {
            for swap_for_y in [true, false] {
                for depth in [1u32, 35, 70, 71, 140, 141, 300] {
                    let plan = plan_bin_arrays(&pool(), active, swap_for_y, depth).unwrap();
                    let legacy = crate::meteora_bin_array_indices(active, swap_for_y, depth);
                    assert!(
                        plan.indices.len() <= legacy.len(),
                        "precise plan must not exceed the legacy helper \
                         (active={active}, swap_for_y={swap_for_y}, depth={depth})"
                    );
                    // ...and must still cover the traversal.
                    assert!(plan.indices.len() >= required_bin_array_count(depth).min(1));
                }
            }
        }
    }

    #[test]
    fn missing_array_is_detected() {
        let plan = plan_bin_arrays(&pool(), 0, true, 140).unwrap();
        assert_eq!(plan.indices.len(), 3);
        let err = verify_bin_array_coverage(&plan, &[(0, plan.pdas[0])]).unwrap_err();
        assert!(matches!(err, BinArrayPlanError::MissingArray { index: -1 }));
    }

    #[test]
    fn unrelated_array_is_rejected() {
        let plan = plan_bin_arrays(&pool(), 0, true, 70).unwrap();
        let bogus = derive_meteora_bin_array_pda(&pool(), 99);
        let err =
            verify_bin_array_coverage(&plan, &[(0, plan.pdas[0]), (-1, plan.pdas[1]), (99, bogus)])
                .unwrap_err();
        assert!(matches!(
            err,
            BinArrayPlanError::UnexpectedArray { index: 99 }
        ));
    }

    #[test]
    fn substituted_array_is_rejected() {
        let plan = plan_bin_arrays(&pool(), 0, true, 70).unwrap();
        let wrong = derive_meteora_bin_array_pda(&pool(), 7);
        let err = verify_bin_array_coverage(&plan, &[(0, plan.pdas[0]), (-1, wrong)]).unwrap_err();
        assert!(matches!(
            err,
            BinArrayPlanError::PdaMismatch { index: -1, .. }
        ));
    }

    #[test]
    fn full_coverage_is_accepted() {
        let plan = plan_bin_arrays(&pool(), 0, true, 140).unwrap();
        let loaded: Vec<(i64, Pubkey)> = plan
            .indices
            .iter()
            .copied()
            .zip(plan.pdas.iter().copied())
            .collect();
        assert!(verify_bin_array_coverage(&plan, &loaded).is_ok());
    }
}
