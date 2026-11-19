# Flash Loan Execution Audit — Kamino Path

> Scope: master directive §3.3 and §12 (borrow sizing → repay validation).
> Every claim classified. Audited 2026-11-19 (P5 pass). Baseline: `f72afbc`.
> Companion: `docs/testing/complete-flashloan-test.md` (step-by-step test
> scenario status), `docs/audit/financial-calculation-audit.md` (fee math).

---

## 1. Provider & structure [code]

- **Provider**: Kamino (KLend) — the only flash-borrow path wired to execution;
  builders live in `arb-tx/src/flash/kamino_flash.rs` with a curated re-export
  in `flash/mod.rs` (curated list exists because `kamino.rs` and
  `kamino_flash.rs` both define wrapper functions — E0659 trap, see AGENTS.md).
- **Atomic order**: `FlashBorrow → Swap_1..m → FlashRepay` inside **one**
  transaction; `atomic_integrity::verify_atomic_integrity` is wired and is the
  structural checker (borrow-first, repay-last, no bundle splitting). [code]

## 2. Repayment math [code + test]

Canonical klend formula from `arb-math/src/economic.rs` (single source of
truth, re-exported by `arb-tx`):

- `fee_bits = max(amount * sf, 2^60)` (flat-fee floor 2^60 raw bits),
  **compared raw pre-rounding** in the `BorrowTooSmall` guard — the guard must
  never be "simplified" to compare the rounded fee.
- Round ties-away at the final step only.

Regression tests in `arb-math` (154 passing) pin: fee floor behavior, ordering
of round vs guard, exact repayment = principal + fee. [test]

## 3. Borrow sizing [code + test]

- Sizing is **profitability-driven**, not max-available: the engine tests
  candidate sizes through exact quoting and picks the best
  **expected-net-value** margin (P1/P2 work: closed-form CPMM optimum,
  piecewise CLMM/DLMM traversal; see `financial-calculation-audit.md` §cost
  ledger — every cost counted exactly once).
- **Hard liquidity bound**: `supervisor/strategy.rs`
  `kamino_available_flash_liquidity` decodes the live reserve supply-vault
  liquidity (from the account store snapshot at a fresh slot) and caps borrow
  below it; if liquidity cannot be verified → **skip opportunity** (fail-closed,
  invariant "الاقتراض على حسب عمق البول"). Config ceiling alone is never
  sufficient. [code]

## 4. Ordering & pre-sign validation [code + test]

The tx assembly path validates before signing: borrow present first, repay
last, repayment amount ≥ principal + raw-guarded fee, mint/decimals
consistency across legs, and program/account expectations; violations refuse
the transaction before it exists (fail-closed validator per directive §11.1).
Covered by `arb-tx` unit tests + supervisor integration tests. [test]

## 5. What is proven vs not proven

| Claim | Class | Evidence |
|---|---|---|
| Repayment math exact (klend formula, raw-guard, ties-away) | **test** | `arb-math` suite, 154 pass |
| Borrow bounded by decoded live liquidity; fail-closed on unknown | **test** | `arb-bot` supervisor tests |
| Atomic instruction ordering structurally validated | **test** | `arb-tx` + `atomic_integrity` tests |
| Full flash-loan transaction compiled end-to-end from live pool state | **test (sim/devnet-grade)** | integration tests; see testing doc |
| **Mainnet flash-loan transaction landed & repaid** | **cluster / unk — NOT PROVEN** | no E2E live run performed in this audit cycle |
| Realized profit from on-chain balances | **unk — NOT PROVEN** | storage path exists & is provenance-gated (f72afbc), but no live landing event has populated it |

## 6. Findings

| ID | Finding | Status |
|---|---|---|
| FL-1 | V1-envelope flash loan execution: transport verified by mainnet canaries (`reports/v1_canary/`, `V1_TRANSPORT_VERIFIED`, 3 reports, finalized; commit c922805) but `ARB_V1_TRANSPORT_SUPPORTED` env flag remains default-false by design (manual operator opt-in, fail-closed) | resolved-by-design |
| FL-2 | No full live flash-loan execution has been run; Definition-of-Done items 13-15 remain open pending P11/P12 | open → P11/P12 |
| FL-3 | Kamino reserve reads on hot path use a slot-age bound
(`KAMINO_RPC_SNAPSHOT_MAX_AGE_SLOTS = 1500`, `arb-constants/src/lib.rs:544`)
audited under PIPE-14 — acceptable, not a stale-data fallback | closed |

## 7. Evidence

- `crates/arb-tx/src/flash/kamino_flash.rs`, `flash/mod.rs` (curated re-exports).
- `crates/arb-math/src/economic.rs` — klend fee formula + `ExecutionCosts`
  unit-explicit ledger (MATH-04/05 fix @ 9a334f8) with
  `test_lamports_costs_convert_through_rate_exactly_once`.
- `crates/arb-bot/src/supervisor/strategy.rs` — `kamino_available_flash_liquidity`.
- `crates/arb-tx/src/atomic_integrity.rs` (wired per AGENTS.md invariants).
- `reports/v1_canary/canary_1794590368.json` — verdict `V1_TRANSPORT_VERIFIED`.
