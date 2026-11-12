# Audit 06 - Meteora DLMM Bin-Array Logic (READ-ONLY)

**Scope.** `crates/arb-math/src/dlmm.rs`, `crates/arb-dex/src/meteora_dlmm.rs`, `crates/arb-tx/src/meteora_dlmm.rs`, `crates/arb-engine/src/meteora_quote_engine.rs`, `crates/arb-engine/src/meteora_fresh_state.rs`, `crates/arb-engine/src/meteora_pair_discovery.rs`, `crates/arb-tx/src/meteora_atomic_builder.rs`, plus tests `crates/arb-math/tests/dlmm_bin_array_boundary_tests.rs`, `crates/arb-engine/tests/meteora_mainnet_sdk_differential.rs`, `crates/arb-engine/tests/meteora_sdk_differential_tests.rs`. Cross-references: `crates/arb-tx/src/lib.rs`, `crates/arb-constants/src/lib.rs`, `crates/arb-ingest/src/registry.rs`, `crates/arb-sim/src/meteora_fresh_pipeline.rs`, `crates/arb-execution/src/merger.rs`. Read-only; `cargo` was **not** run; no code changed.

**Program ground truth used.** Meteora DLMM (`lb_clmm 0.12.0`) mainnet program `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`; `BIN_ARRAY_SIZE = 70`; `BIN_ARRAY_BITMAP_SIZE = 512` (internal `[u64; 16]` bitmap covers array indices `-512..=511`); `MIN/MAX_BIN_ID = -+443636`; PDA seeds `[b"bin_array", lb_pair, index_le_i64]`, `[b"bitmap", lb_pair]`, `[b"oracle", lb_pair]` per the official TS/Rust SDK. Line numbers cited against the repo as of today.

---

## 1. `bin_id -> bin_array_index`: division semantics

**Four duplicate implementations, all floor/Euclidean. The one used by the transaction path is correct; the one used by the instruction-span math is not** (see section 4).

| Location | Code | Semantics |
|---|---|---|
| `crates/arb-math/src/dlmm.rs:574-576` | `(bin_id as i64).div_euclid(70)` | Euclidean (floor for positive divisor) |
| `crates/arb-tx/src/lib.rs:1084-1086` (`bin_array_index_for_bin`) | same | same |
| `crates/arb-tx/src/meteora_dlmm.rs:202-204` (`active_bin_array_index`) | `(active_id as i64).div_euclid(MAX_BIN_PER_ARRAY)` | same |
| `crates/arb-dex/src/meteora_dlmm.rs:85-89` (`bin_id_to_array_index`) | `if b >= 0 { b / n } else { (b - (n - 1)) / n }` | manual floor, mathematically equivalent |
| `crates/arb-sim/src/meteora_fresh_pipeline.rs:33` | inline `active_bin_id.div_euclid(70)` | same |

Rust `i64::div_euclid` with a positive divisor rounds toward negative infinity, so negative bin IDs map correctly: `-1 -> -1`, `-70 -> -1`, `-71 -> -2` (verified in `crates/arb-tx/src/meteora_dlmm.rs:538-543` and `crates/arb-math/tests/dlmm_bin_array_boundary_tests.rs:54-61, 529-531`). The arb-dex manual form agrees (`-1: (-1-69)/70 = -1`, `-70: -139/70 = -1` trunc-toward-zero, `-71: -140/70 = -2`) but has **zero negative-id tests** of its own.

`BINS_PER_ARRAY = 70` is confirmed: `crates/arb-constants/src/lib.rs:222` (`METEORA_BINS_PER_BIN_ARRAY: i64 = 70`), re-exported at `crates/arb-tx/src/meteora_dlmm.rs:24` (`MAX_BIN_PER_ARRAY`). This matches the DLMM protocol constant (`BIN_ARRAY_SIZE = 70`), **not** 64 - the source comment at `meteora_dlmm.rs:23` states this explicitly.

**Finding 1.1 (drift risk).** The same formula lives in at least 5 places (arb-math, arb-tx `lib.rs::bin_array_index_for_bin`, arb-tx `meteora_dlmm.rs::active_bin_array_index`, arb-dex `bin_id_to_array_index`, arb-sim inline, arb-ingest `dlmm_bin_array_pdas` at `registry.rs:1509`). No test asserts cross-implementation equality; a future edit to one silently diverges quoting from instruction building.

---

## 2. BinArray PDA derivation

* Seeds - every implementation uses `[b"bin_array", lb_pair.as_ref(), &index.to_le_bytes()]` with a **signed i64 LE** index, against program `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`:
  * `crates/arb-tx/src/lib.rs:1023-1032` (`derive_meteora_bin_array_pda`),
  * `crates/arb-tx/src/meteora_dlmm.rs:206-214` (`derive_bin_array_pda`),
  * `crates/arb-dex/src/meteora_dlmm.rs:91-98` (`MeteoraDlmmPool::derive_bin_array_pda`).
* Bitmap-extension PDA `[b"bitmap", lb_pair]`: `crates/arb-tx/src/lib.rs:1043-1050`, `crates/arb-tx/src/meteora_dlmm.rs:216-222`, `crates/arb-dex/src/meteora_dlmm.rs:117-123`.
* Oracle `[b"oracle", lb_pair]`: `crates/arb-tx/src/lib.rs:1072-1079`. Event authority `[b"__event_authority"]`: `lib.rs:1034-1041`.
* Program ID is consistently `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` (arb-tx `meteora_dlmm.rs:27`; `lib.rs` via `METEORA_DLMM_PROGRAM_ID`; arb-dex `meteora_dlmm.rs:1-4`).
* PDAs are compared **byte-for-byte against the official SDK** on a captured Mainnet snapshot: pass - `meteora_sdk_differential_tests.rs:106-137` (indices -2,-1,0,1,2 + oracle + bitmap ext) and `meteora_mainnet_sdk_differential.rs:526-561` (per-array checks emitted into `reports/phase15_2/sdk_differential_manifest.json`).

**Finding 2.1 (out-of-scope but critical).** `crates/arb-ingest/src/registry.rs:1054` sets DLMM `PoolExecutionAccounts::MeteoraDlmm { oracle: p.pool_id.0, ... }` - the **pool's own address is used as the oracle account** instead of the derived `[b"oracle", lb_pair]` PDA. Any instruction path consuming ingest's execution accounts would pass a wrong, writable oracle account and revert.

**Finding 2.2.** `crates/arb-dex/src/meteora_dlmm.rs:92` and `:120` use `unwrap_or_default()` on `Pubkey::from_str(METEORA_DLMM_PROGRAM_ID)` - an unparseable constant silently becomes `Pubkey::default()`, i.e. PDAs derived against the system program. Constant is valid today, so this is latent, not live.

---

## 3. Active bin detection, bitmap scanning, bitmap extension

**There is no bitmap scanning anywhere in the audited code.** The LbPair's `bin_array_bitmap: [u64; 16]` (account layout comment, `crates/arb-dex/src/meteora_dlmm.rs:367`) is never read. All array selection is geometric (radius around `active_id`), not bitmap-driven:

* Fetch/quote path: symmetric radius, `depth` -> `2*depth+1` PDAs - `crates/arb-sim/src/meteora_fresh_pipeline.rs:33-52`; symmetric both directions (`center +/- offset`), not swap-direction-aware.
* dex helper: `bin_array_pdas_around_active` (`crates/arb-dex/src/meteora_dlmm.rs:100-115`) - derives a ~500 bps price-impact budget (`500u32 / step + 1` bins) and loads **both** directions (`idx - i` and `idx + i`); for `bin_step = 1` that is `arrays_needed = 501/70 + 1 = 8` -> **16 PDAs, no cap** - can exceed MTU / account-limit when fed to the instruction builder.
* ingest: `dlmm_bin_array_pdas` (`registry.rs:1508-1519`) - same symmetric +/- pattern, dedup via BTreeSet, no cap.
* instruction path: swap-direction-only via `meteora_bin_array_indices` (`crates/arb-tx/src/lib.rs:1059-1070`) / `derive_directional_bin_arrays` (`crates/arb-tx/src/meteora_dlmm.rs:239-288`, capped at **2 arrays**, `TooManyBinArrays`).

**Bitmap extension handling - inconsistent, and the path actually used is fail-open w.r.t. detection (fails closed only at the VM):**

1. `crates/arb-tx/src/meteora_dlmm.rs:46-60` documents the `-512..=511` default bitmap and provides `exceeds_default_bitmap`; `default_bitmap_covers` at `:194-196`; `derive_directional_bin_arrays` computes `bitmap_extension_required` (`:281`) - but this function **caps arrays at 2** (`:257-262`) and is **not used** by the atomic-builder path, which instead calls `meteora_bin_array_indices` (`crates/arb-tx/src/lib.rs:1059`), which computes **nothing** about the bitmap.
2. `crates/arb-tx/src/lib.rs:1043-1050` derives the extension PDA; `build_meteora_dlmm_swap_instruction_with_bin_arrays` (`lib.rs:1731-1784`) accepts `bitmap_extension: Option<Pubkey>` - but both call sites in `crates/arb-tx/src/meteora_atomic_builder.rs:222` and `:256` pass **`None` unconditionally**, even when the selected indices exceed the default bitmap. `meteora_bin_array_indices` does not flag it. Result: for a pool whose active array index is outside `+/-512` (possible for far-from-parity pools), the instruction is built **without** the extension account -> on-chain the program reverts (fail-closed at the VM, but the bot can never trade those pools, and nothing detects the mis-selection locally).
3. `crates/arb-dex/src/meteora_dlmm.rs:282` sets `bitmap_extension: Some(self.derive_bin_array_bitmap_extension_pda())` **always**, even for in-range pools that have no extension account. If a caller feeds this into `DlmmSwapAccountSet::to_account_metas` (`crates/arb-tx/src/meteora_dlmm.rs:133-136`), the extension PDA is encoded as **writable** - a non-existent account marked writable -> transaction fails. Meanwhile `arb-execution/src/merger.rs` propagates `bitmap_extension` verbatim into the 3-leg instruction accounts (leg bitmap_extension assignments; pushed into the account list at the `if let Some(ext) = leg.bitmap_extension` block). So the same flag is "always Some" in dex/ingest (`registry.rs:1056`), "conditionally Some with a computed requirement" in arb-tx's directional derivation, and "always None" in the atomic builder. Three uncoordinated policies; no path verifies the extension account actually exists on-chain.
4. No code parses the extension account's contents anywhere - the mainnet differential manifest explicitly declares this gap: `meteora_mainnet_sdk_differential.rs:723-730` ("PDA_DERIVATION_ONLY ... extension-account parsing and array-index selection beyond the default bitmap are not exercised").
5. A bin with zero liquidity is **dropped by the parser** - `crates/arb-dex/src/meteora_dlmm.rs:182-188` pushes only bins with `amount_x > 0 || amount_y > 0`. Traversal in `quote_dlmm_multi_bin` requires an exact `id` match (`crates/arb-math/src/dlmm.rs:835-838`) -> `DlmmError::BinNotFound(current_bin_id)` - the whole quote errors out instead of skipping the empty bin (the on-chain program skips zero-liquidity bins). This is fail-closed but yields false negatives on any route whose path contains an all-zero bin; it also makes the loaded-array set effectively required to be **complete and contiguous**.

**Net verdict:** bitmap is never scanned; extension is treated as optional everywhere but with three contradictory policies; detection of "extension required" exists only in the unused `derive_directional_bin_arrays`.

---

## 4. Bin-array selection per swap; hardcoded limits; quote vs instruction set

**Quote path** prices from the in-memory `bins: Vec<DlmmBin>` loaded symmetrically with `depth = requested_bin_array_depth` (arb-sim pipeline, section 3) or dex `bin_array_pdas_around_active`; hardcoded budget `500u32 / bin_step + 1` bins (`meteora_dlmm.rs:103`), no cap on array count (section 3). No enforcement that the loaded set covers the trade.

**Instruction path** (`meteora_atomic_builder.rs`):

* `build_atomic_two_swap` passes a fixed span of `METEORA_BINS_PER_BIN_ARRAY` (70 bins) as `bins_traversed` for each hop -> `arrays_needed = 70/70 + 1 = 2` arrays per hop - "one bin array ahead of the active array in the swap direction (2 arrays)" (`meteora_atomic_builder.rs:82-87, 101-117`).
* `build_atomic_two_swap_with_traversal` uses the quote's `bins_crossed` per hop (`:200-210, 234-244`) but computes the span as `(bins_traversed as i64).div_euclid(70) + 1` (`crates/arb-tx/src/lib.rs:1066`), **ignoring the starting bin's offset inside its array**. Correct requirement is `|idx(end_bin) - idx(start_bin)| + 1` with `end = start -/+ bins_crossed`.
  * Counter-example 1 (crosses boundary at small size): `active_id = 0`, `swap_for_y = true`, quote `bins_crossed = 1` (end bin `-1`). Required arrays `[0, -1]`; formula yields `[0]` -> missing array account -> swap reverts.
  * Counter-example 2 (mid-array large swap): `active_id = 10`, `swap_for_y = true`, `bins_crossed = 85` (end bin `-75`, array `-2`). Required `[0, -1, -2]`; formula yields `85/70 + 1 = 2` -> `[0, -1]` -> missing array -> reverts.
  * Counter-example 3: `active_id = 70`, X->Y, `bins_crossed = 1` (end `69`, array `0`): required `[1, 0]`; formula yields `1` -> `[1]` -> reverts. (The comment in that function claims floor-division hygiene but never mentions the offset term.)
  The same formula and same bug exist in `derive_directional_bin_arrays` (`crates/arb-tx/src/meteora_dlmm.rs:254`) and in the ingest/dex symmetric variants (`registry.rs:1512`, `arb-dex meteora_dlmm.rs:104`).
* Consequence: **quote traversal and instruction `remaining_accounts` never use the same ordered set.** The quote consumes whatever bin vec was loaded around the active bin; the instruction re-derives an (undercounted) directional list. A profitable quote can therefore compile into an instruction that reverts. The quoted `bins_crossed` is consumed only through this lossy formula.
* Ordering: both builders emit the active array first, then direction order (`lib.rs:1067-1069`; `meteora_dlmm.rs:267-276`) - consistent with the SDK's traversal-order passing. Order-sensitivity is untested (the DLMM program locates bins by PDA so order is probably irrelevant; unverified).
* Hard limits: `derive_directional_bin_arrays` caps at **2 arrays** and returns `TooManyBinArrays` (`meteora_dlmm.rs:257-262`) - far stricter than the math tests, which traverse 4-5 arrays (`dlmm_bin_array_boundary_tests.rs:289-487`). The atomic-builder path has **no** array-count cap by design ("There is deliberately no upper bound here; the caller's transaction-size check is the only limit, and it fails closed", `meteora_atomic_builder.rs:124-126`) and enforces MTU <= 1232 (`:295-300`) after signing. `meteora_bin_array_indices` itself is unbounded (`lib.rs:1059`). The dex/ingest symmetric fetchers are unbounded too (16 PDAs at bin_step=1) - nothing stops an oversized `execution_accounts` from being attached before the MTU check.

---

## 5. `swap_for_y`, X/Y direction, mint derivation

* Semantics: `swap_for_y = true` => Token X in / Token Y out ("price moves down, bin indices decrease" - `crates/arb-tx/src/meteora_dlmm.rs:63-69`; `crates/arb-math/src/dlmm.rs:642-644`); `false` => Y in / X out.
* Direction is derived **from mint addresses, never symbol names**: the quote engine compares `pool.base_mint == from_mint && pool.quote_mint == to_mint` etc. (`crates/arb-engine/src/meteora_quote_engine.rs:109-115`). The atomic builder compares `first_pool.base_mint.0 == token_in` / `second_pool.base_mint.0 == token_mid` (`meteora_atomic_builder.rs:198, 234`). Both are mint-equality checks against the pool's decoded `token_x_mint`/`token_y_mint` (`arb-dex meteora_dlmm.rs:399-400`). `canonical_mint_x/y` in discovery are also pubkey-ordered, not name-based (`crates/arb-engine/src/meteora_pair_discovery.rs:82-88`).
* Hop direction over the two-hop route: `token_in = first_pool.quote_mint`, `token_mid = first_pool.base_mint` (`meteora_atomic_builder.rs:169-171`) - hop 1 is by construction the pool's own Quote->Base leg; `hop1_swap_for_y` is effectively `false` unless base==quote (degenerate), and `hop2_swap_for_y = (second_pool.base == token_mid)` correctly distinguishes both second-pool orientations. Correct, but opaque - it silently assumes hop 1 always trades the pool's quote mint, with no independent check that hop 2's input mint is in the second pool (guarded only implicitly by mint equality).
* Input/output mint->account wiring is mint-based: `user_token_in/mid/out` via `get_associated_token_address_with_program_id(payer, mint, prog)` (`meteora_atomic_builder.rs:177-182`); `token_x_program`/`token_y_program` placed per mint program, `prog_mid` -> `token_x_program`, `prog_in/prog_out` -> `token_y_program` (`:217-218, 251-252`) - matches the X/Y ordering of the DLMM instruction account layout.

---

## 6. Traversal direction and quote math

**Traversal:** X->Y decrements the active bin (`advance_active_id`, `crates/arb-math/src/dlmm.rs:558-563`), Y->X increments; instruction-side arrays step the same way (`swap_for_y => -1`, `lib.rs:1065`; `meteora_dlmm.rs:248-251`) - consistent. `advance_active_id` enforces `MIN/MAX_BIN_ID +/-443636` (`dlmm.rs:34, 37, 565`) - matches DLMM.

**Pricing (Q64.64):** `P_i = (1 + bin_step/10_000)^i` (`get_price_from_id`, `dlmm.rs:279-290`); `pow_q64` inverts the base for negative exponents and pre-inverts when `base >= 1.0` to keep squaring in u128 range, iterating 19 bits (`:223-273`); zero results rejected as underflow (`:260-262, 267-269`); `MAX_EXPONENTIAL = 0x80_000` caps exp (`:40, 231`). Negative exponents unit-tested (`dlmm.rs:1250-1268`) and SDK-checked (`meteora_mainnet_sdk_differential.rs:261-279`, tolerance 0).

**Swap math per bin** (`consume_bin_liquidity`, `dlmm.rs:649-795`):

| Direction | Gross out for net in | Input for gross out (exact-out) |
|---|---|---|
| X->Y | `floor(net_in * P / 2^64)` (`:625-627`) | `ceil(out * 2^64 / P)` (`:592-604`) |
| Y->X | `floor(net_in * 2^64 / P)` (`:628-636`) | `ceil(out * P / 2^64)` (`:606-616`) |

**Rounding conforms to "protocol keeps the dust":** outputs floored, inputs/fees ceiling (module doc, `dlmm.rs:9-12`). Fees: `compute_fee_from_amount(amount_with_fee, rate)` ceil (`:488-502`), `compute_fee_to_add` ceil (`:439-456`), `included_fee_amount` ceil (`:464-483`), `input_for_gross_out` ceil (`:598-604, 609-616`). Protocol fee = `fee * protocol_share / 10_000` **floored** (`compute_protocol_fee`, `:505-513`).

**Variable/dynamic fee:** `ceil(vfc * (va * bin_step)^2 / 1e11)` (`compute_variable_fee`, `:395-422`); base fee `base_factor * bin_step * 10 * 10^power_factor` (`compute_base_fee`, `:372-389`); total = base + variable clamped at `MAX_FEE_RATE = 100_000_000` (10%) (`compute_total_fee_rate`, `:425-434`). Volatility accumulator per bin: `va = min(volatility_reference + |index_reference - active_id| * 10_000, max_va)` (`update_volatility_accumulator`, `:539-555`) with filter/decay reference update (`update_references_with_decay`, `:516-536`). Byte-exact match against the official SDK on a captured Mainnet pool across 16 cases (both fee modes x both directions x exact-in/out x small/deep, tolerance 0): `meteora_mainnet_sdk_differential.rs:412-467`; within 1 raw unit against the official-SDK fixture in `meteora_sdk_differential_tests.rs:197-265`.

**In-bin cap:** `dlmm_active_bin_max_input` (`dlmm.rs:303-332`) - X->Y: `floor(amount_y << 64 / P)`; Y->X: `floor(amount_x * P >> 64)` - both floored, conservative. Used by `scale_dlmm_three_hop_input`, which applies an arbitrary **92% safety factor** (`:352-353`) - bias downward (safe), but arbitrary.

**Exact-out dust rule:** rounding surplus beyond 1 unit is added to **both** `protocol_fee` and `fee_amount` (`dlmm.rs:1043-1064`) with an explicit comment claiming it mirrors the SDK's `swapExactOutQuoteAtBin`. Plausible and consistent with the tolerance-0 exact-out differential, but **not independently verified** and not fuzzed at the boundary where `step.amount_out == remaining_out + 1`.

**Exact-out bin probe:** `quote_dlmm_exact_out` probes a bin with `u64::MAX` input when the desired output exceeds it, draining the bin fully (`dlmm.rs:1005-1021`) - matches the documented SDK re-quote behaviour; not independently verified.

**Q8 check (truncation bias / float):** no float arithmetic anywhere in the amount/fee paths (all `U256`/`u128` integer); `price_impact_bps` is integer-only with `u16` clamp to 10_000 bps (`dlmm.rs:1146-1175`); `spread_bps` uses `saturating_mul` (`meteora_quote_engine.rs:158-163`); output floors / input-fee ceils / protocol-fee floors are all in the pool's favour, i.e. an arbitrageur's quoted profit is **understated** (conservative). The one overstatement vector is the transfer-fee / CollectFeeMode pair of bugs in section 7.

**Empty bins:** a bin present in the vec with the paying side at zero returns `amount_in_consumed = 0` but **still increments `bins_crossed`** (`dlmm.rs:666-681, 880-883`) - no infinite loop, amounts stay correct, but the `bins_crossed` metric (and any array-span formula derived from it) is inflated. Empty bins absent from the vec hard-error (section 3.5, section 9).

---

## 7. Fee accounting

Components: base fee + variable fee -> total rate (ceil'd variable, section 6); fee skimmed from input (`fee_on_input == true`, CollectFeeMode 0 or mode-1 Y->X) or from output (`fee_on_input == false`, OnlyY + X->Y) via `dlmm_fee_on_input` (`dlmm.rs:103-110`); protocol share carved out of the total fee, floored (section 6). Referral/host fee has **no math anywhere** - `host_fee_in` exists only as an account meta (`crates/arb-tx/src/meteora_dlmm.rs:97-105, 137-140`); the engine never subtracts a host/referral fee, so for pools configured with one, quoted profit is overstated (no such pool in fixtures; undetectable by this code).

**7.1 FINDING (critical, quote correctness) - transfer-fee double subtraction in the cross-pool engine.** `apply_transfer_fee_ceil` returns the **net** amount, not the fee - `crates/arb-math/src/lib.rs:69-77` (`amount - capped_fee`). `MeteoraDlmmCrossPoolQuoteEngine::quote_single_hop` treats the return value as the fee and subtracts again:

```
crates/arb-engine/src/meteora_quote_engine.rs:118-123   let fee = apply_transfer_fee_ceil(amount_in, ...); amount_in.saturating_sub(fee)
crates/arb-engine/src/meteora_quote_engine.rs:134-139   same for outbound
```

So `effective_amount_in = ceil(fee_of(amount_in))` - for a 1% fee the engine tries to swap **the fee itself**, not 99% of the input. Any pool with `pool.transfer_fee_bps > 0` produces catastrophically understated quotes (the arbitrage is refused, or sized wrong), and `pool.transfer_fee_bps` is a single field applied to both mints rather than per-mint (per-mint bps exist on the dex pool: `crates/arb-dex/src/meteora_dlmm.rs:48-49`). The dex `quote()` uses the same helper correctly (`net_in = apply_transfer_fee_ceil(...)`, output side `Ok(apply_transfer_fee_ceil(q.amount_out, ...))`, `meteora_dlmm.rs:426, 475`) - the contrast proves intent. No test passes non-zero `pool.transfer_fee_bps` through `quote_single_hop` (token-2022 is verified standalone only, `meteora_sdk_differential_tests.rs:361-384`).

**7.2 FINDING - `status` and `collect_fee_mode` are hardcoded to "enabled / InputOnly" in the quote engine.** `crates/arb-engine/src/meteora_quote_engine.rs:61` sets `status: 0` and `:72` sets `collect_fee_mode: 0`, ignoring `PoolState::is_active` and `dlmm.params.collect_fee_mode` (the field exists, `arb-types/src/lib.rs:786`, populated by decode, `arb-dex meteora_dlmm.rs:378, 390`). A CollectFeeMode-1 (OnlyY) pool is priced with InputOnly semantics - fee taken from input instead of output; fee amount and protocol/LP split are mis-denominated in exactly the direction that hides the real cost for X->Y OnlyY swaps. The mainnet differential explicitly simulates mode 1 over a mode-0 pool (`meteora_mainnet_sdk_differential.rs:731-738`, "SIMULATED_ONLY") and validates the math branch, but the engine never reaches it because `build_dlmm_pool_and_bins` discards the value. Also `is_active` is ignored, so a disabled pool quotes as enabled.

**7.3 Fee isolation across hops - OK.** Hop 2 input is `hop1_min_out` (the conservative floor, `meteora_atomic_builder.rs:185-187, 225, 253`); excess mid-token stays in the ATA. No protocol/referral double-count in exact-in; the exact-out dust path adds surplus to **both** total and protocol (`dlmm.rs:1053-1063`) - if the SDK does not do this, `fee_amount` (and `protocol_fee`) is inflated by the dust (flagged unverified, section 6).

**7.4 FINDING - truncation to bps / u16.** dex `fee_bps()` floors `scaled * 10_000 / 1e9` to u16 (`arb-dex meteora_dlmm.rs:229-238`) - 2.5 bps reports as `2`. `pair_discovery` casts `fee_numerator as u16` unchecked (`crates/arb-engine/src/meteora_pair_discovery.rs:217-218`) - wraps mod 65536 (moot for typical denominators, still an unchecked narrowing).

**7.5** Fees are computed per traversed bin with the *current* accumulator at each step; the accumulator is re-derived per bin from the snapshot `index_reference` (never the post-swap one) - consistent with SDK snapshot-time behaviour when `current_timestamp == None`. When `Some(last_update_timestamp)` is passed (dex `quote`, `meteora_dlmm.rs:468-472`), `elapsed == 0` so the decay branch never fires (`dlmm.rs:524`) - passing the **last** update time as "now" understates decay for stale caches; subtle, conservative, undocumented.

**7.6** In `consume_bin_liquidity`, the fee-on-input full-drain branch compares `amount_in >= gross_in_max` where `gross_in_max = net_in_max + ceil(fee)` (`dlmm.rs:685-690`) - correct SDK-style inclusion of the fee in the max drain. The partial branch computes `net_in = amount_in - fee` (`:718`), floors output, caps to `bin_max_out` (`:720`) - correct. No double count of protocol fee into LP fee: `fee_amount` is the total, `protocol_fee` a subset (documented, `meteora_mainnet_sdk_differential.rs:26-29`). The `OnlyY, X->Y` (fee-on-output) branch takes the fee from the gross output and reduces user-out accordingly (`dlmm.rs:751-794`) - correct, and byte-matched against the SDK on the captured pool (tolerance 0).

---

## 8. Integer-division truncation and floating point

Covered in sections 6/7: variable-fee division is **ceil** (never under-charges); outputs floor; protocol-fee division floors (LP side favourable - consistent with on-chain semantics, empirically confirmed). No `f32/f64` anywhere in `arb-math/src/dlmm.rs`, `arb-dex/src/meteora_dlmm.rs`, `arb-tx/src/meteora_dlmm.rs`; `price_impact_bps` and `spread_bps` are integer-only with saturating/checked conversions (`dlmm.rs:1162-1174`; `meteora_quote_engine.rs:158-163`).

**Profit-bias summary:** ceiling on input fees and outputs floor => quoted profit understated (conservative); 92% sizing factor => understated; empty-bin `bins_crossed` inflation => slightly overstates traversal cost in sizing heuristics; section 7.1 => garbage understatement for transfer-fee pools; section 7.2 (OnlyY as InputOnly) => **overstatement** for OnlyY X->Y pools. No float-induced or truncation-induced profitable-looking-but-losing systematic vector found beyond 7.1/7.2; all others are unit-scale and protocol-favourable.

**Numeric limit not enforced:** `bin_step` is clamped to >= 1 by dex/ingest decode (`arb-dex:397`; `registry.rs:1045`) but **arb-math itself does not clamp** - `get_price_from_id(0, 0)` returns `ONE_Q64` exactly (base = 1.0 == ONE, `pow_q64` pre-inverts to `u128::MAX/ONE = 2^64-1` then drifts below 1.0 over 19 squarings) - no error, silently wrong price; latent only (dex always clamps).

---

## 9. Cross-check vs Meteora DLMM program semantics (from knowledge)

| Item | This code | DLMM program / SDK | Verdict |
|---|---|---|---|
| Program ID | `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo` everywhere | same (mainnet) | [OK] |
| Bins per array | 70 (`arb-constants:222`) | `BIN_ARRAY_SIZE = 70` | [OK] |
| Min/max bin id | +/-443636 (`dlmm.rs:34,37`) | `MIN/MAX_BIN_ID = -+443636` | [OK] |
| BinArray seeds | `[b"bin_array", lb_pair, le_i64]` | identical | [OK] (byte-matched vs fixture) |
| Bitmap ext seeds | `[b"bitmap", lb_pair]` | identical | [OK] (PDA only; account never parsed) |
| Default bitmap range | `-512..=511` (`meteora_dlmm.rs:52, 59`) | `BIN_ARRAY_BITMAP_SIZE=512`, 1024-slot `[u64;16]` | [OK] |
| `div_euclid` for negative ids | floor | SDK `floor(bin_id / 70)` | [OK] |
| Base fee `base_factor*bin_step*10*10^p` | `dlmm.rs:372-389` | SDK same shape | [OK] (SDK-equal on fixture) |
| Variable fee `ceil(vfc*(va*bs)^2 / 1e11)` | `dlmm.rs:408-419` | SDK formula (ceil) | [OK] at fixture accumulator; **not swept** |
| Total fee cap 10% | `MAX_FEE_RATE` clamp (`dlmm.rs:31, 433`) | protocol max 10% | [OK] assumed/consistent |
| CollectFeeMode 0/1 fee on input/output | `dlmm_fee_on_input` (`:103-110`) | SDK `getFeeMode` (unit-tested, `meteora_sdk_differential_tests.rs:339-359`) | [OK] |
| Volatility accumulator `ref + abs(idx_ref - active)*10_000`, capped | `dlmm.rs:539-555` | SDK identical | [OK] empirically |
| Volatility decay on filter/decay period | `dlmm.rs:516-536` | SDK similar (floor decay) | [OK] unit-tested |
| X->Y decreases active id, Y->X increases | `advance_active_id` (`:558-570`) | confirmed | [OK] |
| Output floor / input fee ceil / protocol fee floor | section 6 | on-chain rounds in protocol's favour | [OK] empirically (tolerance 0) |
| Exact-out dust -> protocol + total fee | `dlmm.rs:1043-1064` | claimed mirror of SDK | [WARN] unverified, not fuzzed |
| Zero-liquidity bins skipped on-chain | engine **hard-errors** `BinNotFound` (`:838`) after parser drops empty bins (`arb-dex:182-188`) | program skips silently | [MISMATCH] semantic divergence (fail-closed, false negatives) |
| `bins_crossed` increments on zero-consume bins | `dlmm.rs:666-681, 880-883` | n/a | [WARN] metric inflates |
| Ingest oracle | `registry.rs:1054`: `oracle = pool_id` | `[b"oracle", lb_pair]` | [MISMATCH] |
| Status byte | engine ignores (`status: 0`, quote_engine.rs:61); dex reads `data[82]` and honours `PoolDisabled` (`dlmm.rs:815-817`) | disabled pools must not quote | [MISMATCH] engine, [OK] dex |
| swap2 / swap_exact_out2 discriminators | pinned to IDL values (`meteora_dlmm.rs:351-355`; golden tests `:773-790`) | plausibly correct | [WARN] not independently re-derived |
| host_fee_in / referral fee absent from quotes | accounts plumbed, math absent (section 7) | SDK supports optional host fee | [WARN] overstates profit on host-fee pools |
| Token-2022 hook remaining-accounts | hardcoded empty `[0,0,0,0]` (`lib.rs:1747`; `meteora_dlmm.rs:349`) | hooks must be declared | [OK] SPL-only pools; [MISMATCH] hook pools unsupported (declared, `meteora_mainnet_sdk_differential.rs:699-707`) |
| Instruction `bin_array` count formula | `bins_traversed/70 + 1`, offset-unaware (section 4) | program requires arrays for every touched bin | [MISMATCH] under-provisions (fail-closed at VM) |
| Max-2-array policy | `derive_directional_bin_arrays` cap (`meteora_dlmm.rs:257-262`) | not a protocol limit | [WARN] arbitrary policy vs 4-5 arrays in math tests |
| Empty `RemainingAccountsInfo` vs `swap2` arg layout | `:359-364, 380-385` | `remaining_accounts_info` vec of slices | [OK] shape matches IDL encoding |
| `swap2` account order (16 base + arrays) | pinned, tested vs official builder (`meteora_dlmm.rs:111-164, 644-699`) | official `Swap2` account order | [OK] (by construction + golden vectors) |
| `swap_exact_out2` arg order `max_in, out_amount` | `meteora_dlmm.rs:393-400, 414-425` | IDL `swap_exact_out2` | [OK] golden-vector tested |
| Sentinel for absent optional accounts | `meteora_dlmm.rs:38-44, 133-140`; lib.rs `:1751-1755, 1767` | official `derivePlaceHolderAccountMeta` | [OK] consistent across both builders |

---

## 10. Tests: what is and isn't covered

**Enumerated coverage**

* `crates/arb-math/tests/dlmm_bin_array_boundary_tests.rs`
  * Index arithmetic incl. negatives (`:46-62`); crossings 69<->70, -70<->-71, -1<->0 (`:65-286`); 4-5-array exact-in/exact-out over `(-75..=75)` and `0..=280` (`:289-487`); bitmap-extension index range **via a locally re-implemented closure**, not production `exceeds_default_bitmap` (`:490-532`).
  * These tests feed `Vec<DlmmBin>` directly to the math engine - array-index logic is exercised only through the free function, never through the fetchers/builders.
* `crates/arb-math/src/dlmm.rs` unit tests (`:1177-1515`): price +/-1 exponent handling, base/variable fee values, max-fee clamp, collect-fee matrix (incl. enum round-trip regression), multi-bin traversal both directions, non-mutating `quote_dlmm_view`, decay/accumulator logic, `dlmm_active_bin_max_input`.
* `crates/arb-engine/tests/meteora_sdk_differential_tests.rs`: byte-equal PDAs for BinArray `-2,-1,0,1,2`, oracle, bitmap ext (`:106-137`); exact-in +/-1 lamport (`:140-265`); exact-out required-in +/-1 (`:267-336`); CollectFeeMode fixture matrix (`:339-359`); Token-2022 transfer-fee standalone (`:361-384`).
* `crates/arb-engine/tests/meteora_mainnet_sdk_differential.rs`: price differential tolerance 0 (`:261-279`); 16 quote cases (both fee modes x both directions x exact-in/out x small/deep), tolerance **0**, plus PDA checks, aggregated into `reports/phase15_2/sdk_differential_manifest.json` (`:412-865`); explicitly self-declares gaps (`:685-758`).
* `crates/arb-tx/src/meteora_dlmm.rs` tests (`:500-897`): negative-index floor division (`:536-544`), PDA seed i64-LE determinism (`:547-556`), directional arrays X->Y lower / Y->X higher incl. second-negative-id cases (`:559-641`), 2-array cap & `TooManyBinArrays` (`:618-626`), `exceeds_default_bitmap` at +/-512/+/-513 (`:608-615`), `swap2` account order golden tests (`:644-699`), optional-account sentinel proofs (`:701-721, 852-896`), `swap_exact_out2` discriminator golden vector (`:752-771`), remaining-accounts encoding golden vectors (`:773-850`), exact-out arg layout (`:737-745`).

**NOT tested / gaps (explicit)**

1. **The `meteora_bin_array_indices` / `derive_directional_bin_arrays` offset under-count** (section 4) - no test calls these with a non-zero in-array start offset near a boundary; `bins_crossed >= 1` starting at bin 0 or bin 70 is unguarded. `meteora_bin_array_indices` has **no unit tests at all**.
2. **Bitmap extension required-case through the instruction builder** - every test passes `bitmap_extension: None`; the `Some` writable path in `to_account_metas:133-136` and lib.rs `:1751-1755` is never exercised end-to-end; no test asserts that a pool with indices outside `+/-512` gets the extension account or is rejected loudly.
3. **Empty bins inside a traversal path** - parser drops zero bins (`arb-dex:182-188`), quote then hard-errors (`dlmm.rs:835-838`); no test asserts the skip-or-fail behaviour.
4. **CollectFeeMode 1 against a real pool config through the engine** - engine hardcodes mode 0 (`quote_engine.rs:72`); differential mode-1 cases reuse mode-0 bin data (declared SIMULATED_ONLY, `meteora_mainnet_sdk_differential.rs:731-738`).
5. **Transfer-fee-inside-quote** - the section 7.1 bug is untested; Token-2022 verified standalone only, not composed with a DLMM quote (declared gap, `meteora_mainnet_sdk_differential.rs:699-707`).
6. **Wrong-direction array reuse** - no test asserts that arrays ordered for the opposite direction (or a subset missing the active array) is rejected locally; the program would revert, but nothing checks it.
7. **Array ordering sensitivity** - no test verifies active-first vs ascending order in `remaining_accounts`; SDK acceptance order is assumed, not tested.
8. **Larger-than-3-array instruction** - boundary tests traverse 4-5 arrays *in math*, but `derive_directional_bin_arrays` caps at 2 and no test shows an instruction carrying >=3 arrays; the with-traversal builder's output for >2 arrays is untested.
9. **Bitmap-extension account decoding** - declared NOT_COVERED (`meteora_mainnet_sdk_differential.rs:723-730`); array-index selection beyond the default bitmap is entirely unexercised.
10. **Exhaustion behaviour** - quote exhausting all loaded arrays (`BinNotFound`) vs the SDK's partial-fill/error is declared NOT_COVERED (`:740-749`).
11. **Synthetic variable-fee sweep** - declared NOT_COVERED (`:708-721`); only the live accumulator is exercised.
12. **Five-array live snapshot** - declared NOT_COVERED (`:686-698`).
13. **Cross-implementation index equality** (arb-math vs arb-tx vs arb-dex vs arb-sim) - no test (section 1).
14. **`pair_discovery` fee_numerator `as u16`** and `fee_bps()` flooring (section 7.4) - untested.
15. **Quote-vs-instruction array set equality** - the single most important invariant for this subsystem (section 4) has no test; no test feeds a quote's `bins_crossed`/`end_active_id` into the instruction builder and asserts the included PDAs cover `[start..end]`.

**Bottom line.** The math core (pricing, fees, rounding) is in unusually good shape - SDK-byte-equal on a real Mainnet pool, tolerance 0 - and bin-array index division is floor/Euclidean everywhere with negative-id tests. The fragile layer is **selection and enforcement outside the math**: no bitmap scan, three contradictory extension policies with the atomic path passing `None` unconditionally, an offset-unaware array-span formula in the instruction path that under-provisions accounts near boundaries, a 2-array cap in the unused-but-available derivation, a cross-pool quote engine with a catastrophic transfer-fee double-subtraction and hardcoded InputOnly/enabled status, a parser/quote interaction that turns empty bins into hard failures, and an ingest oracle wiring bug. All of these are fail-closed at the VM (transactions revert) rather than fund-loss bugs - with the exception of quoted-profit mispricing (sections 7.1, 7.2), which can mark a losing route as profitable or a profitable one as losing before simulation ever runs.
