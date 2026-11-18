# Financial Calculation Audit — P0 (2026-11-18)

Every monetary computation from price discovery to realized profit, audited.
Tags: **[code]** / **[docs]** / **[sim]** / **[chain]** / **[assum]** / **[unk]**.

## 1. Verified-correct computations (evidence first)

### 1.1 Flash loan fee — Kamino (klend) U68F60
`arb-math/src/economic.rs` + `arb-tx/src/flash/kamino.rs` **[code] [chain]**

```
raw_fee_bits = max(borrow_amount.saturating_mul(sf), 2^60)
fee          = round_ties_away_from_zero(raw_fee_bits / 2^60)
```

- `sf` is klend's `flash_loan_fee_sf` read live from the reserve account at offset
  **4904** (layout verified: mint@128, decimals@272, status@4856, supply vault@160,
  fee vault@192).
- `sf == u64::MAX` sentinel ⇒ flash loans disabled ⇒ fail-closed skip.
- `BorrowTooSmall` compares **raw pre-rounding bits** (not the rounded fee) — the
  guard is stricter than a rounded-fee comparison by construction.
- Fee source is single: `arb-math`, re-exported by `arb-tx` (`kamino.rs` wrapper vs
  `kamino_flash.rs` real builder divergence **only** in the wrapper name; the fee
  path itself has exactly one implementation).
- The repaid **argument** equals principal; fee is debited on top by the program,
  and the opportunity math charges fee + principal separately (correct — no
  omission, no double count).

### 1.2 Signed expected PnL
`build_economic_quote` computes `gross − Σcosts` in **i128**, propagating losses,
before any decision. **[code]**

### 1.3 Swap fees
Each adapter's quote returns output **net** of the protocol fee, matching the
program's deduct-order. Swap fees are therefore **contained in quotes** and are
never re-subtracted in the ledger. Audited all cost sites: no double count found
**[code]**.

### 1.4 Slippage
Applied exactly once per hop as a `min_out` **floor** (protection bound, not a
fee), derived from the curve quote — never from spot, never deducted from
expected profit. **[code]**

### 1.5 Tip
Deducted exactly once, after the ladder clamp enforces
`net_after_tip ≥ MIN_NET_PROFIT`; tip can never cross the profit floor
(`opportunity.rs:167-220`). **[code]**

### 1.6 Borrow sizing bound
`B ≤ supply_vault_liquidity` decoded live from the reserve; unverifiable
liquidity ⇒ skip (never assume). **[code] [chain]**

### 1.7 Priority fee / CU
V1: total-lamports fee inside `Message.config`; adaptive compiler strips budget
instructions and remaps instruction indices; priority fee computed **once**
(PI-verified single site per assembler). **[code]**

## 2. Defects

### MATH-01 [P0] Realized PnL cannot represent a loss
- **Where:** `arb-bot/src/supervisor/dispatch.rs:869-881`
  (`u64::try_from(net_profit_i128)`), `:929-932` (`.or(Some(net_profit_val))`
  — on parse failure the value silently falls back to the **estimate**),
  `postgres_store.rs:190`, store `lib.rs:205,331`.
- **Why broken:** storage column is unsigned; a losing lands transaction
  (tip > spread, or adverse fill) maps to drop-then-estimate, so the ledger
  reports a **profit or zero** where a loss occurred — the worst possible
  sign of a stealth-bleeding bot.
- **Impact:** `realized_net` statistics systematically biased upward; kill-switch
  daily-loss logic under-sends; operator sees fake solvency.
- **Fix:** widen persistence to signed (i64), thread i128 through dispatch,
  delete the estimate fallback on landing with actual data present; provenance
  tag distinguishes `actual | estimated | simulated | canary`.
- **Test:** regression — a landed transaction with net −12,345 lamports
  round-trips store as −12,345; estimate fallback forbidden when actual exists.

### MATH-02 [P1] Metrics consume pre-dispatch estimate
- **Where:** `dispatch.rs:947` — `add_net_profit(profit_sol_val)` called on the
  opportunity's **expected** value at dispatch time.
- **Why broken:** metrics present expected-as-actual; combined with MATH-01 the
  dashboard can show profit while the chain shows loss.
- **Fix:** metrics emit on on-chain confirmation (actual mode) or explicitly
  labeled estimate series; never both into one gauge.

### PIPE-01 [P1] `pnl_daily` provenance
- **Where:** store `lib.rs:305-334`, `postgres_store.rs:260-290`.
- **Why broken:** aggregation has no `status='landed'` and no mode filter;
  simulated and canary rows inflate daily PnL.
- **Fix:** aggregate only `status='landed' AND execution_mode='live'`; expose
  the filtered series to the dashboard.

### MATH-03 [P1] Pump-path flash-fee approximation
- **Where:** `strategy.rs:454`, `crates/arb-engine/src/pump_boost_arb.rs`,
  `pump_fun_raydium.rs` — `sf >> 32` used as a bps proxy.
- **Why broken:** the canonical low-sf regime (WSOL reserve
  `sf = 11_529_215_046_068` ≈ 0.1030 bps) under `>> 32` yields ~ульс fractions
  near ~166 bps boundary behavior — the approximation underprices fee by orders
  of magnitude in the low-fee regime, overpricing at the top: opportunities and
  costs computed off-formula.
- **Fix:** pipe raw `sf` into `arb-math::fee_from_sf` (canonical U68F60) for
  every pump path; delete the shift.
- **Test:** regression — `fee_from_sf(11_529_215_046_068, X)` golden equals
  klend's own rounding; property test across sf spectrum vs `>> 32` diff.

### MATH-04 [P2] `ExecutionCosts` unit ambiguity
All production paths are WSOL-denominated today; the struct does not pin its
unit. Enforce typed units (`Lamports` vs `BaseTokenAmount`) before any non-WSOL
quote-mint path lands. **[code, latent]**

### MATH-05 [P2] `CostLedger` dead abstraction
Struct + builder exist, nothing consumes it on the production path. Either wire
it into `opportunity.rs` as the single cost site or remove it. Decision: wire
(goal "one cost ledger" in AGENTS.md).

### MATH-06 [P2] Kelly sizing is f64
Confined to search-guidance (confidence ordering only); binary financial
decision stays integer. Acceptable — documented; keep the boundary tested.

### MATH-07 [P2] PumpBOOST empirical constants
Creator-fee and boost parameters are empirical (protocol-official values
pending); flagged as `[assum]` until the protocol doc confirms. Guarded by
conservative clamps today.

## 3. Double-count matrix (directive §7.7 targets)

| Possible double count | Audited result |
|---|---|
| Price impact re-added after quote | **Absent** — quotes net of impact; ledger adds none |
| Swap fee outside quote | **Absent** — quotes net of fee |
| Slippage as fee | **Absent** — min-out floor only |
| Flash fee double charge | **Absent** — program-side debit counted once + repay principal once |
| Tip double count | **Absent** — single site, clamp-gated |
| Fee on input and output asymmetry | Each hop matches its protocol's actual side (Pump per-direction bps, Raydium numerator/denominator, CLMM both sides on legs per protocol math) |

## 4. Units / decimals

Decimals carried in snapshots, converted once with `10^d` scaling, assert-guarded
against zero; WSOL↔SOL conversion 1:1 with lamports scaling — no mismatch found
**[code]**. Rent: transient ATAs closed (reclaim), creator-fee accounts priced
from constants — no missing account-creation cost on the production path.
