# Flash Loan on a Single DLMM Platform — Forensic Review (Kamino KLend × Meteora DLMM)

Scope (per the review directive): flash-loan invocation, repayment, and account metas on a
single DLMM platform; bin-array / bitmap / bin-array selection for the swap action; profit
directions (`swap_for_y`) per leg; per-bin account/liquidity counts → borrow value and pool
depth. Reverse-engineered from the repo builders and cross-checked against the official
MeteoraAg/dlmm-sdk TypeScript client (cloned at review time) and the KLend program rules
verified earlier in `KAMINO_FLASH_LOAN_AUDIT.md`.

All findings below reference exact files/lines at commit `eaa0111` + the canary fix commit.

---

## 1. Flash-loan invocation & repayment — VERIFIED

### 1.1 Instruction layout (3-leg, single platform = Meteora DLMM only)

`crates/arb-tx/src/flash/dlmm_3leg_flash_arb.rs::assemble_flash_funded_three_pool`:

```text
[0] ComputeBudget::SetComputeUnitLimit
[1] ComputeBudget::SetComputeUnitPrice
[2] ATA CreateIdempotent  (settlement = WSOL ATA of payer)
[3] ATA CreateIdempotent  (intermediate A)
[4] ATA CreateIdempotent  (intermediate B)
[5] klend::flash_borrow_reserve_liquidity   ← borrow_instruction_index = 5
[6] lb_clmm::swap2  Pool 1: WSOL → A
[7] lb_clmm::swap2  Pool 2: A → B
[8] lb_clmm::swap2  Pool 3: B → WSOL
[9] klend::flash_repay_reserve_liquidity    ← borrow_instruction_index = 5
[10] Jito tip (System transfer, marked)
```

First instruction = borrow, last economic instruction = repay — matches the KLend
`flash_borrow_reserve_liquidity` (index 0) / `flash_repay_reserve_liquidity` (last index)
contract enforced via the instructions sysvar.

### 1.2 Borrow account metas (12, verified against Anchor IDL)

`crates/arb-tx/src/flash/kamino.rs:85-119` — `flash_borrow_reserve_liquidity`:

| # | Account | Meta |
|---|---|---|
| 0 | user_authority (payer) | signer, readonly |
| 1 | lending_market_authority | readonly |
| 2 | lending_market | readonly |
| 3 | reserve | **writable** |
| 4 | reserve_liquidity_mint | readonly |
| 5 | reserve_supply_vault | **writable** |
| 6 | user destination ATA (= settlement) | **writable** |
| 7 | reserve_fee_vault | **writable** |
| 8 | referrer_token_state (or program_id sentinel) | readonly |
| 9 | referrer_account (or program_id sentinel) | readonly |
| 10 | Sysvar1nstructions…1111111111111111111111111 | readonly |
| 11 | token_program | readonly |

### 1.3 Repay semantics — the two traps, both handled

1. **Amount argument = PRINCIPAL ONLY.** KLend's `flash_repay_checks` requires
   `liquidity_amount == borrow amount` (else `InvalidFlashRepay` 6033); the flash fee is
   debited **separately, on top**, by the handler from the same source account.
   `dlmm_3leg_flash_arb.rs:459-467` passes `params.borrow_amount` (principal), and the
   repayment-coverage figure `repay_amount() = principal + fee` is enforced as a *separate*
   invariant against leg 3's minimum output (`Invariant 6`, lines 253-272).
   The doc comment on `FlashArb3LegParams::repay_amount` (lines 262-272) states this
   distinction explicitly — this is the single most common flash-loan integration bug and it
   is correctly avoided.
2. **borrow_instruction_index remap.** In V1 the compiler strips leading ComputeBudget
   no-ops; `dlmm_3leg_flash_arb.rs:655-665` rewrites the reported indices through
   `v1_executed_instruction_index`, and `adaptive.rs` rewrites the repay's embedded index
   byte (`transport/adaptive.rs:418-443`). Without this, every V1 flash-loan tx would revert
   with `InvalidFlashRepay`.

### 1.4 Settlement-account isolation (no inventory subsidy)

The borrow destination, pool-3 output, and repay source are **one** account: the payer's
flash-mint ATA (`settlement_account`, lines 476-478 + `kamino.user_destination_liquidity`
override at line 484). Leg 1 consumes the principal in full (`amount_in = borrow`), so a
pre-existing balance can never subsidize the repayment or fake a profit. Each intermediate
leg's `amount_in` is bound to the previous leg's **conservative minimum** output
(Invariant 6), not the optimistic expected output.

### 1.5 WSOL specifics

No `SyncNative` is required anywhere in this flow: the flash mint is **wrapped** WSOL
(SPL token), the Kamino vaults hold wrapped WSOL, and every transfer is a plain SPL token
transfer between token accounts. `SyncNative` is only needed for native-SOL-coalescing
accounts, which this route never touches. The two-pool builder (`meteora_atomic_builder.rs`)
likewise needs no SyncNative.

### 1.6 Atomicity

`verify_atomic_integrity` (borrow first, repay last, coverage ≥ principal+fee) runs before
signing (`dlmm_3leg_flash_arb.rs:516-531`). Any repay shortfall reverts the entire
transaction including the borrow — the atomic rollback guard is structural (single tx), not
a soft check.

---

## 2. Bin array / bitmap / swap bin-array selection — VERIFIED vs official SDK

### 2.1 Selection rule

`crates/arb-tx/src/flash/dlmm_bin_discovery.rs::plan_bin_arrays` (line 113):

* `swap_for_y = true` (X→Y, price down): step −1; `false` (Y→X, price up): step +1.
* Span: `first = active_bin_id`, `last = active ± (bins_traversed−1)`;
  array indices from `bin_array_index_for_bin` (floor division, correct for negatives)
  walked from the active array toward the last array **in traversal order**.
* Bitmap extension required iff any planned index is outside the default window
  **[−512, 511]** (`METEORA_BIN_ARRAY_BITMAP_SIZE = 512`) — the BUG-001 fix. The engine
  passes the extension PDA only when required (`dlmm_flash_arbitrage.rs:814/830`).

### 2.2 SDK cross-check (MeteoraAg/dlmm-sdk, ts-client)

* `getBinArrayForSwap(swapForY, count)` (index.ts:2370) walks from the active bin array in
  the swap direction via `findNextBinArrayIndexWithLiquidity`, collecting arrays in
  **traversal order** — identical convention to `plan_bin_arrays` (first = active array,
  then step in direction).
* `getBinArrayIndexesCoverage(lowerBinId, upperBinId)` (helpers/positions/index.ts:19) is
  used only for position/liquidity coverage, not swap account order — not applicable here.
* `binIdToBinArrayIndex` = floor division by 70 — matches `bin_array_index_for_bin`
  (`div_euclid`, meteora_accounts.rs:83-85).
* The SDK's `swap()` passes the caller's `binArraysPubkey` through **unreordered** as
  writable remaining accounts (index.ts:6116-6123) — confirming the program consumes them
  in the supplied traversal order, which is what we emit.

### 2.3 swap2 account matrix

`crates/arb-tx/src/protocols/meteora_accounts.rs:87+` mirrors the generated official client
(`Swap2`): 16 fixed accounts (lb_pair, bitmap-extension slot with optional-account sentinel,
reserve_x/y, user_token_in/out, token_x/y_mint, oracle, user, user source/destination,
token_x/y_program, memo_program, event_authority, host_fee sentinel) + bin arrays as
remaining accounts, all writable. Verified against live mainnet transactions earlier in the
audit (see METEORA_BIN_ARRAY_AUDIT.md).

---

## 3. Profit directions (`swap_for_y`) — VERIFIED

Derivation chain (single source of truth, `crates/arb-engine/src/dlmm_flash_arbitrage.rs:154-162`):

```text
swap_for_y(from, to) = token_x_mint == from && token_y_mint == to   → true  (X→Y)
                     = token_y_mint == from && token_x_mint == to   → false (Y→X)
                     = otherwise                                    → None (reject leg)
```

For the WSOL → A → B → WSOL cycle:

| Leg | from → to | swap_for_y derivation |
|---|---|---|
| 1 | WSOL → A | WSOL == leg1.token_x ? true : false (resolved against the pool's **on-chain** X/Y ordering, not the human naming) |
| 2 | A → B | same rule against pool 2's on-chain ordering |
| 3 | B → WSOL | same rule against pool 3's on-chain ordering |

Key correctness property: the direction is resolved from each pool's **on-chain**
`token_x_mint`/`token_y_mint` fields, never from the route's textual mint order. A pool
whose on-chain ordering is (A, WSOL) is handled with no special case — covered by the
`reversed_mint_ordering_is_supported` test in `dlmm_flash_arb.rs`.

`token_in()`/`token_out()`/`token_in_program()`/`token_out_program()` on
`FlashArb3LegPoolLeg` (lines 96-133) are derived from the same flag, so the mint chain,
ATA sides, and token programs cannot drift from the direction.

---

## 4. Per-bin depth → borrow value — VERIFIED

### 4.1 Single-leg capacity (per bin)

`crates/arb-math/src/dlmm.rs:303-332` — `dlmm_active_bin_max_input`:

* X→Y (a_to_b): `max_in_x = amount_y · 2^64 / price_q64` (U256, floor) — the active bin's
  entire Y reserve converted at the bin's exact Q64.64 price.
* Y→X: `max_in_y = (amount_x · price_q64) >> 64` (floor).
* Zero reserve on the output side → `None` → leg capacity 0 → route rejected. Fails closed.

Inside a DLMM bin the swap is constant-sum (zero price impact within the bin), so the
active-bin reserve is the exact capacity of the zero-impact tranche. This is a
**conservative floor** on true pool depth (deeper traversal crosses more bins), which is the
correct direction for sizing: never overstate depth.

### 4.2 Chain simulation and bottleneck

`crates/arb-engine/src/flash_loan_sizing.rs`:

* `get_leg_capacity` (line 213): per-pool active-bin capacity in the leg's direction.
* `simulate_three_leg_chain` (line 230): walks `borrow → leg1 quote → conservative min_out
  (apply_slippage) → leg2 → … → leg3 → final_output`, accumulating `bins_crossed` and DEX
  fees per leg. The chain's **bottleneck** is the minimum leg capacity propagated forward.
* Borrow is capped by the bottleneck (with a 92% safety factor to stay inside the active
  bin), by Kamino's available liquidity, and by the configured maximum exposure; the size
  optimizer then searches candidates maximizing **net** profit
  (`net_profit_lamports`, line 315: `final_output − borrow − flash_fee(U68F60) − network_cost`),
  with confidence derived from the capacity ratio (3000 bps floor).
* `bins_crossed` per leg feeds `plan_bin_arrays`, so the borrow value directly determines
  the account footprint (arrays needed) — and the account footprint feeds back as a
  constraint through transaction size and loaded-data caps. The loop is closed: borrow ↑ →
  bins ↑ → arrays ↑ → size/CU ↑ → capped.

### 4.3 Verdict

The borrow value is derived from verified on-chain per-bin reserves, the Kamino reserve's
available liquidity, and the net-profit optimizer — no hardcoded trade size anywhere in the
chain (HARDCODED_VALUES.csv tracks the remaining engineering limits, all classified).

---

## 5. Findings summary

| # | Area | Status | Evidence |
|---|---|---|---|
| 1 | Borrow/repay metas (12 accounts each) | PASS | kamino.rs:85-119 vs Anchor IDL; layout test `layout_is_borrow_swaps_repay` |
| 2 | Repay amount = principal only | PASS | dlmm_3leg_flash_arb.rs:459-467 + doc on repay_amount() |
| 3 | V1 borrow-index remap | PASS | adaptive.rs:418-443; dlmm_3leg_flash_arb.rs:655-665 |
| 4 | Settlement isolation / no subsidy | PASS | Invariants 4-6, tests |
| 5 | SyncNative not needed (wrapped WSOL) | PASS | route uses SPL transfers only |
| 6 | Bin-array order = SDK traversal order | PASS | plan_bin_arrays vs getBinArrayForSwap |
| 7 | Bitmap-extension rule (outside [−512,511]) | PASS | BUG-001 fix + 5 boundary tests |
| 8 | swap_for_y from on-chain ordering | PASS | dlmm_flash_arbitrage.rs:154-162 + reversed-ordering test |
| 9 | Per-bin capacity math (Q64.64, U256) | PASS | arb-math/dlmm.rs:303-332, oracle-verified |
| 10 | Borrow sizing from depth + net profit | PASS | flash_loan_sizing.rs:213-340 |

No new defects found in this review. The one structural caveat remains as documented in
`THREE_POOL_EXECUTION_AUDIT.md`: the standalone 3-leg builder (`dlmm_3leg_flash_arb.rs`) is
the reference/V1-capable implementation, while the live dispatch path assembles 3-leg routes
through the engine's `dlmm_flash_arbitrage.rs` pipeline (which carries the same invariants
and the bitmap-extension fix). Any dispatch-path change must keep the two in sync or
consolidate onto one builder.
