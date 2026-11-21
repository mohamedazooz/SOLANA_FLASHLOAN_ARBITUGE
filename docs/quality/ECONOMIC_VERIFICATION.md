# ECONOMIC_VERIFICATION — Canonical Economic Engine

**Baseline commit**: `25ffe606f255d4ae5f0ea52e550df96afee55ef4`
**Verification date**: 2026-11-21

---

## 1. Single source of truth

`crates/arb-math/src/economic.rs` is the only financial decision authority:
`build_economic_quote` (economic.rs:296), `calculate_net_profit_raw`
(economic.rs:512), and the Kamino fee family (`kamino_flash_fee_checked`
economic.rs:621, `kamino_fee_bits` economic.rs:646, `calculate_flash_fee_raw`
economic.rs:655). `arb-tx` re-exports; no strategy, engine, or supervisor module
implements an alternative profit calculation (verified by register in
`docs/active/DECISIONS.md` §1 and by this session's grep for parallel
estimators — none found).

## 2. Cost accounting contracts

`NetProfit = FinalOutput − BorrowPrincipal − FlashLoanFee − BaseFee −
PriorityFee − JitoTip − TokenTransferFees − OtherCosts − Contingency`,
evaluated in integer raw units with explicit cost-unit attribution
(`CostLedger`/`CostEntry`/`CostUnit`, economic.rs:95–126), including:

- `total_included_not_deducted` vs `total_deducted_by_unit` — prevents
  **double-counting** fees already embedded in DEX quote outputs (the exact
  ISSUE-P1-01 class).
- Dimensional typing: raw token units are never added to lamports without an
  explicit, provenance-carrying conversion (`base_raw_to_lamports`,
  economic.rs:266; `verified_config::rate_usdc_is_rejected_integral_only`).
- Realized-vs-estimated separation enforced at the metric level (PR #98):
  realized PnL is a signed gauge fed **only** by on-chain balance deltas;
  estimated landed rows stay in the estimate bucket; SQLite outcome rows carry
  provenance filters.

## 3. Articulable floor policy

A candidate must satisfy **both** floors:

- `NetProfit >= AbsoluteMinimumProfit` (absolute lamport/USD-safe floor,
  `min_net_profit_floor_matrix` exercised in the config suite), and
- `NetProfit >= RelativeMinimumProfit` (basis-point profitability),
- plus ROI sanity, overflow rejection (`no_overflow_is_accepted` property),
  missing-cost rejection, invalid-conversion rejection, and repayment-shortfall
  rejection. Fail-closed: any missing cost input rejects the route.

## 4. Kamino flash fee verification (U68F60)

Contract from the deployed klend semantics and preserved in `kamino_fee_bits`:

```
fee_bits = max(amount·sf, 2^60)        (exact U68F60 product)
fee      = round_ties_away(fee_bits / 2^60)
repayment= principal + fee
```

- `fee_bits >= amount << 60` → klend rejects with `BorrowTooSmall`; the guard
  compares **raw bits pre-rounding** (protected by regression tests
  economic.rs:931–935, incl. the `sf = 2^60 − 1` near-1.0 boundary).
- The official Kamino docs confirm `borrow_instruction_index` must address the
  flash-borrow instruction **position in the final transaction** — the V1
  compiler rewrites it after ComputeBudget stripping
  (`adaptive.rs::compile_v1_message`), preserving klend's
  `flash_repay_checks` sysvar assertion.

## 5. Executed evidence (this session)

- `cargo test --workspace --all-targets` → **1,398 passed / 0 failed / 2
  ignored**; economic, sizing, dedup, and PnL-provenance families all green,
  including (selection):
  - `config::min_profit_floor_tests::min_net_profit_floor_matrix`
  - `supervisor::route_dedup::tests::duplicate_route_keeps_highest_net_profit_candidate`
  - `test_matrix_02_supervisor_unprofitable_candidate_rejected`
  - `test_matrix_06_insufficient_kamino_liquidity_rejected`
  - `verified_config::rate_usdc_is_rejected_integral_only`
  - realized/estimated PnL separation metrics tests (arb-metrics / arb-store)
- Property suites (proptest, in `cpmm.rs` / `clmm.rs`) green in the same run.

## 6. Floating-point policy

`grep`-level audit of authoritative decision paths (quote → sizing → economic →
risk → dispatch): no `f64`/`f32` is used for financial decisions. The only
floats live in telemetry/latency estimation (`arb-math/src/latency.rs`,
RTT log-normal tracker) and display formatting, which do not approve or reject
opportunities.

## 7. On-chain enforcement status (honest)

`programs/arb-executor` compiles and is wired as the profit-guard program; the
off-chain pipeline additionally enforces minimum-output constraints derived
from the validated quote. The off-chain net-profit estimate is **not** an
on-chain guarantee; this limitation is carried into
`FINAL_PRODUCTION_READINESS_REPORT.md`.
