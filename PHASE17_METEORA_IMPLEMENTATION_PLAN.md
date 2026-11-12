# PHASE17_METEORA_IMPLEMENTATION_PLAN.md

**Phase:** 17 — Meteora DLMM Two-Pool Flash Loan Arbitrage
**Repository:** `D:\work_new`
**Branch:** `feat/dlmm-cross-pool-strategy`
**Mode:** SHADOW — live execution disabled
**Date:** 2026-11-11

---

## 1. Mission

Execute, inside **ONE** Solana transaction:

```
Kamino Flash Borrow (WSOL)
   -> Meteora DLMM Pool A swap   (WSOL -> TOKEN)
   -> Meteora DLMM Pool B swap   (TOKEN -> WSOL)
   -> Kamino Flash Repay (WSOL principal + flash fee)
```

Every phase below is an implementation requirement, not a document. The repository
already contains a complete DLMM math engine, a Kamino flash-loan integration, a
two-swap atomic builder and a cross-pool optimizer; **Phase 17 wires those existing
pieces into one flash-funded, direction-correct, constraint-checked path and closes the
gaps that the Phase 16 audit recorded** (no flash leg in the DLMM atomic builder, no
directional bin-array discovery bound to quote depth, no Kamino-fee-aware sizing, no
on-chain account validation).

---

## 2. Phase 1 — Repository audit (what already exists)

| Concern | Existing implementation | Reuse decision |
|---|---|---|
| DLMM quote engine (Q64.64, integer-only) | `crates/arb-math/src/dlmm.rs` | **Reuse.** 16/16 zero-tolerance SDK differential cases already pass (`@meteora-ag/dlmm@1.9.14`). No second quote engine is written. |
| LbPair / BinArray decoding | `crates/arb-dex/src/meteora_dlmm.rs` | **Reuse** the decoders; add validation on top (Phase 3). |
| DLMM PDA helpers | `crates/arb-tx/src/lib.rs` (`derive_meteora_*_pda`) | **Reuse** as canonical; add a differential test against the `arb-dex` copy. |
| Directional bin-array index walk | `crates/arb-tx/src/lib.rs` (`meteora_bin_array_indices`) | **Reuse**; extend into an explicit requirement set with coverage validation (Phase 4). |
| Kamino flash borrow/repay instructions | `crates/arb-tx/src/kamino.rs` + `lib.rs` | **Reuse.** 12-account matrix, `flash_loan_fee_sf` Q64.64 fee math, fail-closed reserve decoder. |
| Kamino flash fee | `crates/arb-tx/src/kamino.rs::calculate_kamino_flash_fee` | **Reuse** in the optimizer and the transaction builder. |
| Two-swap atomic DLMM builder | `crates/arb-tx/src/meteora_atomic_builder.rs` | **Reuse** the account-resolution and instruction-shape logic; **extend** with the flash borrow/repay legs (Phase 7). |
| Cross-pool sizing optimizer | `crates/arb-engine/src/meteora_size_optimizer.rs` | **Reuse** the coarse-to-fine, multi-basin search shape; **extend** with Kamino fee, liquidity, CU, size and profit-floor constraints (Phase 6). |
| Two-pool quote engine | `crates/arb-engine/src/meteora_quote_engine.rs` | **Reuse** `quote_single_hop` / `quote_two_hops`. |
| Atomicity verifier | `crates/arb-tx/src/atomic_integrity.rs` | **Reuse** `verify_atomic_integrity` as the builder's self-check. |
| Economic integration | `crates/arb-math/src/economic.rs`, `crates/arb-engine/src/economic_integration.rs` | **Reuse** cost-ledger semantics; the Phase 17 cost model is expressed in raw WSOL units. |

**Phase 16 audit gaps this phase closes** (`PHASE16_AUDIT.md`, `docs/PROJECT_STATE.md`):

1. DLMM atomic builder had **no flash-loan leg** — a Kamino-funded two-pool route did not exist.
2. Bin-array provisioning was not bound to the quote's actual traversal depth.
3. No on-chain account validation (ownership / PDA derivation / pool status) before pricing.
4. Sizing optimizer ignored the flash fee and the flash-loan liquidity ceiling.

---

## 3. Phase 2 — Two real Meteora DLMM pools

Pools are selected from an **explicit allowlist** of on-chain-verified addresses. The
allowlist is populated from a live `getMultipleAccounts` capture of the `LbPair`
accounts (owner `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`, `status == 0`,
`function_type == 0`).

Captured at slot **468191440** (confirmed commitment):

| Pool | bin_step | active_id | var_fee_control | protocol_share | token_x | token_y | status |
|---|---|---|---|---|---|---|---|
| `5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6` | 4 | −6168 | 120000 | 1000 | WSOL | USDC | 0 |
| `BGm1tav58oGcsQJehL9WXBFXF7D27vZsKefj4xJKD5Y` | 10 | −2468 | 40000 | 1000 | WSOL | USDC | 0 |
| `HTvjzsfX3yU6BUodCjZ5vZkUrAxMDTrBs3CJaq43ashR` | 1 | −24673 | 2000000 | 1000 | WSOL | USDC | 0 |
| `CgqwPLSFfht89pF5RSKGUUMFj5zRxoUt4861w2SkXaqY` | 80 | −310 | 7500 | 1000 | WSOL | USDC | 0 |

**Primary pair:** `5rCf1DM8…` (Pool A) x `BGm1tav5…` (Pool B) — both WSOL/USDC,
both active, both with executable depth.

**Mint ordering is NOT assumed.** Each pool's `token_x_mint` / `token_y_mint` are read
from the on-chain `LbPair` at runtime, and the hop direction (`swap_for_y`) is derived
per pool from that ordering. A pool whose on-chain ordering is (USDC, WSOL) is handled
by the same code path with the opposite `swap_for_y` flag. The validation layer
asserts the two pools share the *same unordered* mint pair and the same token programs
per mint, and that the flash-loan mint (WSOL) is present in both pools.

The strategy supports **both** route directions:

* Direction 1: WSOL→TOKEN on A, TOKEN→WSOL on B.
* Direction 2: WSOL→TOKEN on B, TOKEN→WSOL on A.

No API price is used anywhere in the decision path. Every number comes from on-chain
account state or from the deterministic quote engine.

---

## 4. Phase 3 — DLMM account validation

New module `crates/arb-dex/src/dlmm_validation.rs` validates, for every pool:

* account **owner** == Meteora DLMM program,
* `LbPair` account length >= minimum,
* `status == 0` (pool enabled),
* `token_x_mint != token_y_mint`,
* `oracle == PDA([b"oracle", lb_pair])`,
* `reserve_x == PDA([lb_pair, token_x_mint])`, `reserve_y == PDA([lb_pair, token_y_mint])`,
* bin-step range, active-bin range, `min_bin_id`/`max_bin_id` sanity,
* `collect_fee_mode` / `function_type` accepted values,
* BinArray account owner, and the BinArray's **own stored `array_index`** equal to the
  requested index (data-driven check, no PDA arithmetic duplicated),
* BinArrayBitmapExtension owner,
* token program identity per mint (SPL Token vs Token-2022) across both pools,
* mint extensions / transfer fees accounted in the quote.

All PDA derivations use the official MeteoraAg/dlmm-sdk `commons/src/pda.rs` seeds.
The `arb-dex` copy and the `arb-tx` copy are proven equal by a differential test
(`crates/arb-engine/tests/phase17_pda_differential.rs`).

---

## 5. Phase 4 — Bin array + bitmap discovery

New module `crates/arb-tx/src/dlmm_bin_discovery.rs`:

* **Directional** discovery: for `swap_for_y = true` (X→Y, price down) the arrays
  walk to *lower* indices; for `swap_for_y = false` (Y→X, price up) to *higher*
  indices. Starting index is the active bin's array.
* **Dynamic count**: `ceil(bins_traversed / 70) + 1`, derived from the quote's real
  `bins_crossed` — never a fixed global count.
* **Bitmap extension**: the `BinArrayBitmapExtension` PDA (`[b"bitmap", lb_pair]`) is
  included whenever the traversal leaves the active array (i.e. more than one array is
  required), and the on-chain `bin_array_bitmap` is used to confirm the target array
  exists.
* **Missing-account detection**: an array that the pool's bitmap says does not exist,
  or that cannot be fetched, rejects the opportunity.
* **Coverage validation**: the loaded array set must cover every bin the quote
  traverses; a short set is a hard error, never a silent truncation.
* **Budget constraints**: `required_bin_arrays <= max_bin_arrays` and the resulting
  serialized transaction size `<= 1232` bytes.

---

## 6. Phase 5 — Exact quoting

* Primary quote source: `arb_math::dlmm::quote_dlmm_view` (exact-in) and
  `quote_dlmm_exact_out` (exact-out), unchanged.
* Fee model: base fee, variable fee with live volatility accumulator + decay, protocol
  fee share, `CollectFeeMode` (fee on input vs fee on Y).
* Token-2022: transfer fee applied on the **input** side and on the **output** side,
  each exactly once (`apply_transfer_fee_ceil` returns the *net* amount — no double
  subtraction).
* Integer / fixed point only. No float in any execution calculation.
* Differential: the existing 16/16 zero-tolerance mainnet SDK differential suite
  (`crates/arb-engine/tests/meteora_mainnet_sdk_differential.rs`) continues to run
  unchanged; Phase 17 adds mainnet-captured LbPair field verification
  (`phase17_mainnet_state_verification.rs`).

---

## 7. Phase 6 — Cross-pool optimization

New module `crates/arb-engine/src/dlmm_flash_arbitrage.rs`.

For every candidate borrow amount `B`:

1. `flash_fee = ceil(B * flash_loan_fee_sf / 2^64)` (Kamino, Q64.64).
2. Quote hop A with `B` -> `mid`.
3. Quote hop B with the **actual** `mid` -> `final`.
4. `gross = final - B`; `costs = flash_fee + network_cost + priority_fee`;
   `net = gross - costs` (conservative: intermediate slippage is applied to the
   hop-A minimum output before hop B is priced).

Objective: **maximize `net(B)`**. Profit is *not* assumed monotonic: a coarse
logarithmic + linear grid is followed by multi-basin local-maximum detection and a
bounded integer fine sweep.

Hard constraints (all fail closed):

* `B <= kamino_available_liquidity`
* `B <= pool A executable liquidity` (active-bin capacity, plus the quote must succeed)
* `mid <= pool B executable liquidity`
* `required_bin_arrays <= max_bin_arrays`
* `serialized_tx_bytes <= 1232`
* `estimated_cu <= cu_limit`
* `final >= B + flash_fee + costs + min_profit`

Optimizer is checked against exhaustive search on bounded test domains
(`phase17_two_pool_flash_arb_tests.rs`).

---

## 8. Phase 7 — Kamino flash loan + atomic transaction

New module `crates/arb-tx/src/dlmm_flash_arb.rs` builds the single transaction:

```
[0] ComputeBudget::SetComputeUnitLimit
[1] ComputeBudget::SetComputeUnitPrice
[2] ATA CreateIdempotent  (settlement WSOL)
[3] ATA CreateIdempotent  (intermediate TOKEN)
[4] klend::flash_borrow_reserve_liquidity      <-- borrow_instruction_index = 4
[5] lb_clmm::swap2   (Pool A)
[6] lb_clmm::swap2   (Pool B)
[7] klend::flash_repay_reserve_liquidity       <-- borrow_instruction_index = 4
```

* Borrow principal = `B`; repay amount = `B + flash_fee` (ceil-rounded, never
  understated).
* **Settlement ATA isolation**: the flash principal lands in a dedicated WSOL ATA that
  is simultaneously the flash-borrow destination, the final hop output destination and
  the flash-repay source. The intermediate TOKEN ATA receives hop A's output and is
  the only source of hop B's input; hop B's `amount_in` is bound to hop A's
  *conservative minimum* output, so pre-existing inventory can never subsidize the
  route (no-inventory-subsidy invariant).
* `verify_atomic_integrity(ExecutionMode::SingleTransaction, …)` is executed on the
  assembled instruction list as a builder self-check.
* The transaction is serialized and size-checked against the 1232-byte limit.

---

## 9. Deliverables

| Artifact | Path |
|---|---|
| This plan | `PHASE17_METEORA_IMPLEMENTATION_PLAN.md` |
| DLMM account validation | `crates/arb-dex/src/dlmm_validation.rs` |
| Bin-array + bitmap discovery | `crates/arb-tx/src/dlmm_bin_discovery.rs` |
| Atomic flash builder | `crates/arb-tx/src/dlmm_flash_arb.rs` |
| Strategy / optimizer | `crates/arb-engine/src/dlmm_flash_arbitrage.rs` |
| On-chain pool fetcher | `crates/arb-ingest/src/dlmm_pool_fetcher.rs` |
| Captured pool state | `fixtures/phase17_meteora_pools.json` |
| Integration tests | `crates/arb-engine/tests/phase17_two_pool_flash_arb_tests.rs` |
| On-chain verification | `crates/arb-engine/tests/phase17_mainnet_state_verification.rs` |
| PDA differential | `crates/arb-engine/tests/phase17_pda_differential.rs` |

**Safety:** the repository stays in SHADOW mode. Phase 17 adds no broadcast path, no
Jito submission and no live authorization; the new builder produces an unsigned/
signed *transaction artifact* that is only ever simulated.

---

## 10. Verification gates

* `cargo fmt --all -- --check`
* `cargo check --workspace --all-targets`
* `cargo clippy --workspace --all-targets -- -D warnings`
* `cargo test --workspace --all-targets --no-fail-fast`
* `cargo test -p arb-engine --test phase17_two_pool_flash_arb_tests`
* `cargo test -p arb-engine --test phase17_mainnet_state_verification`
