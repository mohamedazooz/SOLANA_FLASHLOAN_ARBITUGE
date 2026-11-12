# 06 — Meteora DLMM BinArray Forensic Audit

Evidence base: `audit_work/06_DLMM_BIN_ARRAY.md` (all `file:line`), plus external protocol ground
truth in `14_EXTERNAL_RESEARCH_REFERENCES.md` §6 (70 bins/array; `["bin_array", lb_pair, i64 idx]`;
`["bitmap", lb_pair]`; default bitmap −512…511; `swap_for_y` is X-in semantics; output floor /
input ceil).

## 1. Bin index math — VERIFIED correct (all copies)

| Copy | Code | Semantics |
|---|---|---|
| arb-math/src/dlmm.rs:574-576 | `(bin_id as i64).div_euclid(70)` | Euclidean (floor) |
| arb-tx/src/lib.rs:1084-1086 | same | same |
| arb-tx/src/meteora_dlmm.rs:202-204 | same | same |
| arb-dex/src/meteora_dlmm.rs:85-89 | manual floor `(b − (n−1)) / n` | equivalent |
| arb-ingest/src/registry.rs:1853 | `bin_id_to_bin_array_index` | same family |
| arb-sim/src/meteora_fresh_pipeline.rs:33 | inline `.div_euclid(70)` | same |

`-1 → -1`, `-70 → -1`, `-71 → -2` verified with tests
(`meteora_dlmm.rs:538-543`, `dlmm_bin_array_boundary_tests.rs:54-61`). `BINS_PER_ARRAY = 70`
(arb-constants/src/lib.rs:222; `METEORA_BINS_PER_BIN_ARRAY`). Not 64. ✓ against protocol.

**Drift risk (OPEN):** ≥6 copies, no cross-implementation equality test; arb-dex copy has zero
negative-id tests.

## 2. BinArray PDA derivation — VERIFIED byte-exact vs SDK

Seeds `[b"bin_array", lb_pair, index_le_i64]`, program `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` —
arb-tx/lib.rs:1023-1032, arb-tx/meteora_dlmm.rs:206-214, arb-dex:91-98; byte-matched against the
official SDK on a captured mainnet snapshot, indices −2…2 (`meteora_sdk_differential_tests.rs:106-137`;
`meteora_mainnet_sdk_differential.rs:526-561`). Bitmap-extension PDA `[b"bitmap", lb_pair]` similarly
correct (lib.rs:1043-1050). Event authority `[b"__event_authority"]` hard-checked ==
`D1ZN9Wj1fRSUQfCjhvnu1hqDMT7hzjzBBpi12nVniYD6` (master_alt.rs:38-41).

**Ingest oracle bug (CONFIRMED):** `PoolExecutionAccounts::MeteoraDlmm { oracle: p.pool_id.0, … }`
(registry.rs:1054) — pool address used as oracle instead of the `[b"oracle", lb_pair]` PDA; any
instruction consuming ingest execution accounts reverts.

## 3. Active bin / bitmap scanning / extension — NO SCAN EXISTS

- `LbPair.bin_array_bitmap: [u64; 16]` is **never read**. All array selection is geometric around
  the active bin.
- Fetch/quote: symmetric ± radius (`bin_array_pdas_around_active`, arb-dex:100; budget
  `500u32 / bin_step + 1` bins → **up to 16 PDAs at bin_step=1, no cap**) — oversized PDAs can be
  attached to `execution_accounts` and only the post-signing MTU check catches them.
- Bitmap extension handling — **three contradictory policies, none verified**:
  1. arb-tx/meteora_dlmm.rs:281 computes `bitmap_extension_required`; its derivation caps arrays
     at **2** (`TooManyBinArrays`, :257-262) and is unused by the atomic path.
  2. arb-dex:282 / registry.rs:1056 always set `Some(pda)` writable — nonexistent extension
     account marked writable ⇒ any consumer wiring it reverts.
  3. **The actually-signed path passes `None` unconditionally** (meteora_atomic_builder.rs:222,256).
  Net effect: pools whose active array index is outside ±512 can never be traded, and nothing
  detects it. No code parses the extension account's contents (self-declared gap,
  `meteora_mainnet_sdk_differential.rs:723-730`).
- Empty (zero-liquidity) bins are **dropped by the parser** (arb-dex:182-188) then the quote
  hard-errors `BinNotFound` (dlmm.rs:835-838) instead of skipping — the on-chain program skips
  them silently. Fail-closed, but every route whose path includes an unloaded-but-crossed empty
  bin is refused. `bins_crossed` inflates on zero-consume bins (dlmm.rs:666-681,880-883).

## 4. BinArray selection — the under-provisioning bug (CONFIRMED CRITICAL for execution)

`meteora_bin_array_indices` (arb-tx/lib.rs:1059-1070) and `derive_directional_bin_arrays`
(meteora_dlmm.rs:239-288) compute:

```
arrays_needed = (bins_traversed as i64).div_euclid(70) + 1
```

**ignoring the starting bin's offset inside its array.** Required set is
`|idx(end) − idx(start)| + 1` with `end = start ∓ bins_crossed` (direction-dependent).

Counter-examples (independently derived):
1. active=0, X→Y, bins_crossed=1 → need `[0, −1]`; formula yields `[0]` → missing array → revert.
2. active=10, X→Y, bins_crossed=85 (end=−75, array −2) → need `[0, −1, −2]`; formula yields `2` → revert.
3. active=70, X→Y, bins_crossed=1 (end=69, array 0) → need `[1, 0]`; formula yields `1` → revert.

Identical formula/bug in `derive_directional_bin_arrays`, `registry.rs:1512`, arb-dex:104.
`meteora_bin_array_indices` has **zero unit tests**.

**Quote traversal vs instruction set never match** — the quote consumes whatever bin vec was
loaded (typically ±"depth" around active, or `2*depth+1` symmetric); the instruction re-derives an
undercounted directional list. A profitable quote can compile into a reverting instruction.
Ordering is active-first then direction order (consistent with SDK convention, but **untested and
unverifiable here** — ordering sensitivity is not checked by any test).

Hard limits: directional derivation caps at **2 arrays**; atomic builder is unbounded
("no upper bound… caller's MTU check is the only limit, fails closed",
meteora_atomic_builder.rs:124-126,295-300). Math tests traverse 4-5 arrays; instruction path 2.

## 5. Direction semantics (tested against SDK)

X→Y ⇒ bins decrement; Y→X ⇒ increment; `swap_for_y=true` = X in. Matches SDK; exact-out dust and
fee modes verified byte-equal on captured pool except the Exact-out dust rule
(`dlmm.rs:1043-1064` — unverified vs SDK, unfuzzed).

## 6. Quote vs instruction — the systemic finding

The single invariant "quote uses the same loaded array set that the instruction supplies" has no
test anywhere. No test feeds a quote's `bins_crossed`/`end_active_id` into the instruction builder
and asserts coverage of `[start..end]`. Everything else in this section (PDA math, per-bin
liquidity walk, fee rounding) is in unusually good shape — the fragility is selection and
enforcement.

## 7. Test coverage map

Covered: index math incl. negatives (`dlmm_bin_array_boundary_tests.rs:46-62, 65-286`);
4-5-array traversal in math (:289-487); SDK-parity PDAs (`meteora_sdk_differential_tests.rs`);
tolerance-0 quotes on captured pool (`meteora_mainnet_sdk_differential.rs`); directional X→Y/Y→X
array tests (`meteora_dlmm.rs:559-641`); 2-array cap; bitmap-exceeds at ±512/513.

Missing: boundary offset cases through the *instruction* path; extension-required end-to-end;
empty bins in traversal; wrong-direction array rejection; >2-array instructions; cross-impl index
equality; quote⇒instruction set equality (the critical one).

## 8. Does ALT help? (No — see 07)

Bin arrays are **dynamic, pool-specific** accounts; they cannot be globally pre-loaded in a shared
ALT (256 entries max; every pool has its own). ALT can only shrink their 32-byte key to a 1-byte
index when referenced — it does not remove them from the transaction, compute, or account locks.
Prefetching a larger neighborhood trades RPC/memory cost for latency; with no bitmap scan, the
"prefetch N arrays around active" policy is the only viable approach, and N must be sized from the
trade's `bins_crossed`, not fixed. An adaptive policy:

```
N(direction) = |idx(active_end) − idx(active_start)| + 1
active_end   = active − ceil(bins_crossed)  (X→Y) / active + bins_crossed (Y→X)
cache key    = (lb_pair, direction, slot_epoch); invalidate on active_bin change or slot > S_max
```

Cache the *parsed* bins (not raw accounts) to skip RPC on repeated quotes of the same pool within
a slot; always re-derive the instruction array set from the quote's end bin; cap by the 1232-byte
budget measured post-compile (today's compilers exceed it; the live path never reaches this).
Recommended sizes from fixtures: typical arb crosses 1-3 arrays per hop; prefetching ±4-5 arrays
covers the crossing tests with margin; prefetching ±16 (current bin_step=1 worst case) wastes
~10 RPC calls of decode per hop.
