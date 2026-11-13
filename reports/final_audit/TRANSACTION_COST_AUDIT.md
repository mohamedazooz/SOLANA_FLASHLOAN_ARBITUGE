# CU, Fees and Transaction Cost Audit (PHASE J)

## CU estimation — dynamic, simulation-corrected

* `arb_tx::estimate_route_cu(true, &[MeteoraDlmm; 3], 3)` composes the estimate from the
  actual instruction mix (flash borrow/repay, per-leg swap, per-BinArray allowance,
  ATA ops) — not a single static number.
* `calculate_dynamic_cu_limit` adds a safety margin under the 350k protocol max
  (`COMPUTE_BUDGET_LIMIT_3HOP = 320_000` for the 3-hop route).
* The estimator is corrected by REAL simulation CU before dispatch (LiteSVM / fork sim),
  and historical reporting uses measured consumption, never the estimate.

## Measured evidence (live mainnet, previous phase)

| Metric | Value |
|---|---|
| Simulated CU consumed | 76,549 / 400,000 |
| V0 wire size | 942 bytes / 1,232 |
| Quote/sim slot lag | ≤ 1 (enforced ≤ 2) |
| Base fee | 5,000 lamports |
| Flash fee (WSOL reserve) | exactly 0.1 bps via `flash_loan_fee_sf` |

## Priority fee

`TipStrategy::adaptive_cu_price` (p75 of recent landed CU prices, escalated +25%,
bootstrap default 100,000 micro-lamports) × dynamic CU limit, converted with
`(price × cu) / 1_000_000` (V0) and ceil-encoded for V1. This audit unified the 3-leg
pipeline's cost model onto the same adaptive source as the dispatch gate.

## Jito tips

`pick_dynamic_tip` is bounded by the risk config (`max_tip_spend_ratio`); the tip account
is slot-rotated. The 3-leg pipeline now ranks with the VERIFIED tip instead of 0
(fixed this audit).

## Cost double-count check

The net-profit identity counts each cost exactly once: DEX fees are inside the swap
output (fee-on-input/fee-on-output semantics per pool), so they are NOT subtracted again;
flash fee, base fee, priority fee, tip, ATA rent are subtracted once each
(`arb-math/src/economic.rs::calculate_net_profit_raw`, CostLedger enforces
"included" vs "deducted" separation).
