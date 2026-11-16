# CPI Phase 2 — Economic & Quantity Accounting Remediation Report

**Report date:** 2026-11-16
**Status:** FORENSIS COMPLETE / **CODE FIXES NOT APPLIED** (see §6 — blocker).
**Method:** read-only source inspection; line numbers are from the current checkout
(`newrepair @ b9553c1e`). No test was run and no numeric result below is presented as
a test result — the worked examples are hand-computed arithmetic over the formulas
as written in the code.

> This report is the deliverable required by directive §14 Phase 2. It deliberately
> does **not** claim the defects are fixed. The environment has no shell/toolchain,
> so any edit to money-handling Rust would be unverifiable; per `AGENTS.md`
> ("Verify before claiming") and directive §15, unverified changes are not applied.

---

## 0. One-sentence summary

Six confirmed defects (B1–B6) and three suspected defects (S1–S3) were found in the
quote → plan → cost → wire path. The most serious are a **double application of
slippage** at two hop boundaries (producing a wire `min_amount_out` that violates
the directive's mandatory `prev.min_out >= next.amount_in` invariant) and a
**priority-fee double charge** where the transaction encodes a higher CU price than
the value deducted from profit.

---

## 1. Authoritative data flow (as built today)

```
three_leg_meme_pipeline::quote_chain
  inputs  = [borrow, floor(out1), floor(out2)]     // "conservative intermediate input"
  out     = [out1, out2, out3]                     // optimistic per-leg outputs
  legs[].min_out = floor(leg_out * (1 - slippage_bps/10_000))
                                                    // dedicated context path only
    ↓ (live path only passes Vec<u64>)
supervisor/run.rs: hop_inputs = [legs[0].amount_in, legs[1].amount_in,
                                legs[2].amount_in, expected_out]
    ↓
arb-tx primitives/builder.rs::derive_swap_legs
  amount_in[i]   = inputs[i]
  expected_out[i]= inputs[i+1]
  min_out[i]     = expected_out[i] - expected_out[i]*slippage/10_000
    ↓
ComputeBudget price = cap_cu_price_by_profit(...)  [supervisor]
                       + min(gross/25_000, 500_000) [builder — SECOND charge]
    ↓
wire instructions
```

The dedicated `ThreeLegMemeArbitrageContext` / `LegSwapParams` path
(`three_leg_meme_pipeline.rs:807-856`, `compiler.rs:498-577`) is correct but is
**not consumed by the supervisor** — it is test-only. The live path goes through
the generic `derive_swap_legs`, which is where the defects live.

---

## 2. Confirmed defects

### B1 — `hop_inputs` contract violated (P0)

- **Contract:** `crates/arb-tx/src/primitives/builder.rs:286-313` —
  "`hop_inputs` must be `[borrow, out_1, ..., out_n]`"; `amount_in[i]=inputs[i]`,
  `expected_out[i]=inputs[i+1]`.
- **Actual:** `crates/arb-bot/src/supervisor/run.rs:355-360` passes
  `[input, in2, in3, out3]` where `in2=floor(out1·(1−s))`, `in3=floor(out2·(1−s))`.
- **Consequence:** positions 1 and 2 are *discounted inputs*, not *expected outputs*.
  `expected_out[0]` becomes `in2` instead of `out1`; `expected_out[1]` becomes `in3`
  instead of `out2`.

### B2 — Slippage applied twice at the same boundary (P0)

- First haircut: `three_leg_meme_pipeline.rs:336-344` (`slippage_floor` → `in2`, `in3`).
- Second haircut: `builder.rs:331-336` re-applies slippage to the already-floored
  `inputs[i+1]`.
- The two haircuts need not even use the same rate: pipeline uses
  `params.slippage_bps` (`run.rs:238`), builder uses
  `effective_hop_slippage_bps()` with `DEFAULT_HOP_SLIPPAGE_BPS = 10`
  (`arb-constants/src/lib.rs:462`; `arb-types/src/lib.rs:1157-1164`).

**Worked regression case (hand-computed, `borrow = 1_000_000_000`, `s = 1%`,
`out1 = 1_010_000_000`, `out2` irrelevant for leg 0):**

| quantity | correct (source quote) | current wire (B1+B2) |
|---|---|---|
| `out1` | 1,010,000,000 | — |
| `in2 = floor(out1·0.99)` | 999,900,000 | 999,900,000 |
| `min_out_0 = floor(expected_out_0·0.99)` | floor(1,010,000,000·0.99) = **999,900,000** | floor(999,900,000·0.99) = **989,901,000** |
| next hop `amount_in` (leg 1) | 999,900,000 | 999,900,000 |
| invariant `min_out_0 ≥ next.amount_in` | 999,900,000 ≥ 999,900,000 ✅ | 989,901,000 < 999,900,000 ❌ **shortfall 9,999,000 lamports** |

This is exactly the condition directive §5.2 declares mandatory. It permits leg 0 to
succeed while delivering less than leg 1 requires; the deficit is silently drawn
from pre-existing inventory, which directive §5.3 forbids.

**Required regression tests (to add when a toolchain exists):**
1% slippage; multiple consecutive hops; `min_out == next.amount_in` (boundary);
`min_out < next.amount_in` (must reject); rounding boundaries (`7/3`, ties);
zero output; `u64::MAX` overflow; final repayment floor; repeated and inverted route
orientations.

### B3 — Profit gate ranks on optimistic final output (P1)

- `three_leg_meme_pipeline.rs:621-630` passes `quote.out[2]` (raw `out3`) into
  `net_profit_of`; ranking/floor check at `:553-558`.
- `flash_loan_sizing.rs:141-146` passes `leg_min_outputs[2]` (floored).
- **Result:** two authoritative profit calculators disagree. The pipeline reports
  and ranks profit optimistically on the last hop (2× the slippage haircut of
  difference between optimistic `out3` and conservative `floor(out3·(1−s))`).
  Repayment solvency still uses `expected_min_final_output` (`:591-600`), so this
  is a *reported/ranked* inflation, not an insolvency — but ranking an inflated
  opportunity above a genuinely better one is a real capital-allocation defect.

### B4 — Priority fee charged twice (P0)

- Supervisor: `opportunity.rs:116-135` computes
  `cu_price = min(desired, ceil? budget·1e6/cu_limit)` with
  `budget = profit·max_share_bps/10_000`, then deducts
  `ceil(cu_price·cu_limit / 1_000_000)` from net (`opportunity.rs:155`).
- Builder: `builder.rs:372-379` (and `188-195`) calls
  `calculate_dynamic_cu_price(gross, priority_fee_micro_lamports)` which adds
  `min(gross/25_000, 500_000)` (`cu.rs:25-28`) and encodes **that** price.

**Worked regression case (hand-computed):**
`gross = 100_000_000 lam`, `cu_limit = 200_000`, `desired = 5_000_000 µlam/CU`,
`max_share_bps = 2_000` (20%).
- cap budget = 100,000,000 × 2000 / 10000 = 20,000,000 lam
- max price = 20,000,000 × 1,000,000 / 200,000 = 100,000,000 µlam/CU
- capped price = min(5,000,000, 100,000,000) = **5,000,000**
- deducted priority fee = ceil(5,000,000 × 200,000 / 1,000,000) = **1,000,000 lam**
- builder elevation = min(100,000,000 / 25,000, 500,000) = **4,000 µlam/CU**
- **wire price = 5,004,000 → wire fee = 1,000,800 lam → 800 lam under-deducted.**

At `gross = 12.5 SOL` the elevation saturates at 500,000 µlam/CU and the
under-deduction reaches **100,000 lamports** on a 200k-CU transaction (10% of the
deducted fee). The 20%-of-gross cap is defeated and the addend is driven by *gross*,
not *net*. `assemble_atomic_bundle` (`builder.rs:67-68`) uses the raw value — so the
same opportunity is priced differently depending on which builder is called.

**Required fix shape:** encode exactly `priority_fee_micro_lamports` in both builder
sites; delete the elevation call; make `calculate_dynamic_cu_price` non-authoritative
(test/diagnostic only) or remove it; keep `div_ceil` for the lamport conversion.

### B5 — Authoritative Kamino fee path uses unchecked wrapper (P1)

- Canonical formula `arb-math/src/economic.rs:510-542` is **correct**:
  `fee_bits = max(amount·sf, 2^60)`, `BorrowTooSmall` when
  `fee_bits ≥ amount·2^60` checked **pre-rounding** (`:524-525`), ties-away
  `(fee_bits + 2^59) >> 60` (`:528`).
- But `flash_loan_sizing.rs:412-417` calls
  `arb_tx::kamino::calculate_kamino_flash_fee` (the *unchecked* wrapper,
  `kamino.rs:209-218`, `unwrap_or(u64::MAX)` and no `BorrowTooSmall` guard). This
  feeds sizing, the three-leg pipeline (`three_leg_meme_pipeline.rs:575-576`) and
  `derive_swap_legs` (`builder.rs:77, 449`).
- **Result:** a too-small borrow is not rejected at the economics layer; it reaches
  the wire and is rejected on-chain by klend, burning the signature fee. Fail-closed
  is currently provided only by the on-chain program, not by the gate.

### B6 — No canonical execution-plan representation (P0 structural)

`HopExecutionPlan` does not exist anywhere (zero grep matches). Competing encodings:
`hop_inputs: Vec<u64>` (`arb-types/src/lib.rs:1128`), `PlannedHop`
(`arb-types/src/lib.rs:693-710`, already documents the correct rule and is **unused
by the builder**), `LegSwapParams` and `ThreeLegMemeArbitrageContext`
(`compiler.rs:201-238`), private `TwoSwapPlan` (`meteora_atomic_builder.rs:129`).
This duplication is the root cause of B1/B2/B3.

---

## 3. Suspected defects (need route-level unit analysis; NOT claimed as confirmed)

- **S1 — unit conflation in solvency floor.** `builder.rs:316-321` adds lamport
  `tip_raw` to `borrow + flash_fee_raw + min_profit_raw`, which are in the *base
  token's* raw units. For a WSOL base (9 decimals) lamports and base-raw coincide;
  for any other base they do not. The specialized route is WSOL-anchored, so the
  live path is currently safe by coincidence, not by construction.
- **S2 — f64 in a decision path.** `arb-land/src/lib.rs:137-158`
  `pick_dynamic_tip` uses f64 to choose the tip actually paid.
- **S3 — zero account-creation cost.** `opportunity.rs:352-355` passes `0` for
  account-creation costs; ATA rent is reclaimed in-tx today, so this is correct
  only while every ephemeral ATA is closed.

---

## 4. Canonical target model (to implement when a toolchain is available)

```
HopExecutionPlan {
    pool_id, protocol, mint_in, mint_out,
    amount_in, expected_out, min_amount_out, direction,
    token_programs, required_accounts, state_slot / freshness_evidence,
}
```
Rules:
1. `min_amount_out = floor(expected_out · (1 − s))` — **exactly once**, in one place.
2. `next.amount_in = previous.expected_out` for quoting; the sizing path may choose
   `next.amount_in = previous.min_amount_out` as a *conservative* choice, but if it
   does, that same value is the authoritative plan amount for both hops — no
   second haircut.
3. Enforce `previous.min_amount_out >= next.amount_in` for every adjacent pair;
   reject otherwise.
4. Final profitability = `floor(final_out·(1−s))` (conservative), using the same
   Kamino **checked** fee and the same encoded CU price as the wire transaction.
5. `NetProfit = FinalOut − Borrow − KaminoFee − BaseFee − PriorityFee − JitoTip
   − TokenTransferFees − NonRecoverableRent − Contingency`, all integer, one ledger,
   each cost source/unit/rounding documented (directive §6.4).

---

## 5. Before / after formula summary

| Item | Before (current code) | After (required) |
|---|---|---|
| Hop input | `[input, floor(o1), floor(o2), o3]` | `[borrow, o1, o2, o3]` or one authoritative conservative plan |
| Min out | `floor(floor(oᵢ·(1−s))·(1−s))` (2 haircuts) | `floor(oᵢ·(1−s))` (1 haircut) |
| `prev.min_out ≥ next.in` | violated by ~1% of out | enforced, reject otherwise |
| Reported profit | `net(out3)` (optimistic) | `net(floor(out3·(1−s)))` |
| CU price on wire | `cap + min(gross/25k,500k)` | exactly `cap` (one price) |
| Kamino fee gate | unchecked wrapper | checked, `BorrowTooSmall` propagates |
| Plan type | `Vec<u64>` + 4 copies | one `HopExecutionPlan` |

---

## 6. Why no code fix was applied (BLOCKER)

The command-execution tool in this session cannot spawn a shell (`pwsh.exe`:
access denied), so `cargo fmt`, `cargo clippy -- -D warnings`, and
`cargo test --workspace --all-targets` **cannot be run**. Per `AGENTS.md` and
directive §§0/15, shipping edits to profit/sizing/priority-fee code that has not
been compiled or tested — into a system that handles real funds — is not
acceptable. The fixes above are therefore specified precisely but **not applied**.

An operator with a working Linux toolchain should, in order:
1. Apply B4 first (single-line-per-site change, lowest risk, highest certainty).
2. Introduce `HopExecutionPlan` (B6), migrate supervisor and builder, then B1/B2/B3.
3. Switch sizing to the checked Kamino fee (B5) and add error propagation.
4. Add all regression tests from §2, then B1–S3 unit analysis.
5. Re-run the full CI gate before any wiring of the on-chain executor.

**No claim of "economic invariants pass" is made. Phase 2 is NOT complete.**
