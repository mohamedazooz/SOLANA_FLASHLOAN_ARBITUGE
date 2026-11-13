# Meteora DLMM Bin Array Audit (PHASE C/D)

## Official constants (verified from the deployed program IDL this audit)

Source: MeteoraAg/dlmm-sdk (cloned main), `ts-client/src/dlmm/idl/idl.json` constants:

| Constant | Value | Meaning |
|---|---|---|
| `MAX_BIN_PER_ARRAY` | 70 | bins per BinArray |
| `BIN_ARRAY_BITMAP_SIZE` | 512 | default bitmap half-width in bin-array indices |
| `EXTENSION_BINARRAY_BITMAP_SIZE` | 12 | extension window size |
| seed `BIN_ARRAY` | `"bin_array"` | BinArray PDA seed |
| seed `BIN_ARRAY_BITMAP_SEED` | `"bitmap"` | BitmapExtension PDA seed |

Program ID: `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` (matches repo constant).

## 1. Bin-array index = mathematical floor division

`index = floor(bin_id / 70)`. Implementation: `i64::div_euclid(70)` in
`arb-tx/src/protocols/meteora_accounts.rs:83`, `arb-dex/src/dlmm_validation.rs:87`,
`arb-math/src/dlmm.rs:574` — all identical, all pinned by
`crates/arb-ingest/src/registry.rs::bin_array_index_uses_floor_division`:

| Bin ID | Array index |
|---|---|
| 0 | 0 |
| 69 | 0 |
| 70 | 1 |
| −1 | **−1** (truncation would give 0 — the historical bug class) |
| −70 | −1 |
| −71 | −2 |
| −5840 | −84 |

## 2. BinArray PDA seeds (verified byte-for-byte)

`seeds = [b"bin_array", lb_pair.as_ref(), index.to_le_bytes()]` (i64 little-endian).
`crates/arb-engine/tests/phase17_pda_differential.rs` and `meteora_sdk_differential_tests.rs`
assert byte-identical derivations vs the official SDK for indices 0, 1, 2, −1, −2, plus
Bitmap Extension and Oracle. PASS.

## 3. Bitmap Extension — BUG FOUND AND FIXED

**Defect (BUG-001, HIGH)**: `crates/arb-tx/src/flash/dlmm_bin_discovery.rs:151` used

```rust
requires_bitmap_extension: indices.len() > 1
```

This is the exact forbidden condition. It is wrong in both directions:
* A two-array traversal entirely inside the default window (e.g. bins 0..−69 → arrays [0,−1]) was flagged as requiring the extension — it does not.
* A single-array traversal at bin-array index 600 was flagged as not requiring it — it does.

**Official rule** (`isOverflowDefaultBinArrayBitmap`, ts-client/src/dlmm/helpers/binArray.ts):
the extension is required iff any traversed bin-array index is outside the default window
`[-BIN_ARRAY_BITMAP_SIZE, BIN_ARRAY_BITMAP_SIZE − 1]` = **[−512, 511]**.

**Fix applied**:
* `arb-constants`: added `METEORA_BIN_ARRAY_BITMAP_SIZE = 512`, window bounds, and `meteora_bin_array_index_requires_bitmap_extension()`.
* `plan_bin_arrays` now computes the flag as `indices.iter().any(outside_window)`.
* `arb-engine/dlmm_flash_arbitrage.rs` now passes the extension account to the swap legs only when the plan requires it (previously always `Some(...)`, wasting an account slot and risking an uninitialized optional-account mismatch).
* Regression tests added: `bitmap_extension_required_exactly_outside_the_default_window` (boundary indices ±512/±511/±513 in both directions) and `traversal_into_overflow_arrays_requires_the_extension` (511→512 and −512→−513 crossings).

## 4. Active Bin

`active_id` is decoded from the on-chain LbPair account (offset-verified decoder in
`arb-ingest/src/registry.rs`: bin_step u16 @80, status u8 @82) with signed i32 handling,
slot-stamped, and freshness-gated. It is never inferred from liquidity or volume. The
quote engine consumes exactly this value; stale slots fail closed.

## 5. Traversal planning

`plan_bin_arrays` derives first/last bin from the quote's real `bins_crossed` and walks
array indices contiguously (`|index(last) − index(first)| + 1`), never
`ceil(bins/70)` — the naive ceil formula under-provisions when the active bin starts
mid-array. `verify_bin_array_coverage` rejects missing arrays, substituted arrays
(PDA mismatch), and unplanned arrays (silent substitution). PASS (11/11 unit tests +
integration coverage in phase17 tests).
