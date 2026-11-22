# Arbitrage Engine Audit — Phase 4 (Mission §9)

- **Date:** 2026-11-22 · Code-only verification against the named source files; live profitability of any route remains UNPROVEN (no live tx permitted).

## 1. Architecture of the pipeline

| Stage | Implementation | Notes |
|---|---|---|
| Cycle discovery | `arb-engine/src/bellman_ford.rs` (Moore-Bellman-Ford on `-ln(rate×gamma)` graph, f64) | Candidate discovery ONLY. `evaluate_cycle` (line 404) uses the f64 product solely as a pre-screen (`product <= 1.0 → skip`); all pricing/sizing/gating re-derive from integer math below |
| Route graph & dedup | `route_graph.rs`, `route_dedup.rs` | `RouteSignature` preserves direction/hop order (ISSUE-P0-01 fixed, PR #93) |
| Exact re-quote & sizing | `compute_cycle_profit` (integer `Leg`/closed-form `optimal_{2,3,4}_pool`, `arb-math`), joint route×size bounded search with per-hop re-quotes + bin/tick boundaries (ISSUE-P0-02 resolution), Kamino vault-cap clamp | Returns best-OBSERVED candidate; explicitly not a global optimum |
| Net economics | `arb-math::economic::build_economic_quote` + `calculate_net_profit_raw` (economic.rs:552) — full CostLedger: gross − principal − flash fee − DEX fees − transfer fees − ATA rent − other verified costs − network fee − priority fee − Jito tip, converted to base-raw by slot-tagged rate | Integer-only in decisions |
| Gate ladder | `opportunity.rs`: implausibility → net-profit floor → EV gate → RiskGate (USD notional, limits) → invariant checks | Documented rejected opportunities surface via telegram/metrics |
| Assembly | `TxBuilder::derive_swap_legs` (final-hop economic floor) / split guard `split_return_min_out` (Phase 2) | Same fee formula instance as scanner |
| Sim & realization | preflight typed simulation (fail-closed), realized PnL = signed on-chain balance-delta gauge, estimates quarantined (ISSUE-P1-03 fixed) | DB rows carry provenance (estimate vs realized) |

## 2. Executability of routes (not just math)

- Every hop carries `execution_accounts` + `token_programs` from ingestion hydration; builders fail closed when required accounts are absent (`swap_hop.rs` per-protocol arms).
- Executor-CPI routes compile with the on-chain delta guard preceding repay (Phase 2 audit §3) — a mathematically profitable but maliciously/toxic-liquidity route cannot repay without producing ≥ market output.
- Quote dependency manifests + `max_slot_skew = 0` reject stale/mixed-slot routes (ISSUE-P0-03 closed).

## 3. Consistency of economic assumptions (single-source ladder)

| Consumer | Where the net-profit assumption comes from | Consistent? |
|---|---|---|
| Scanner gate | `calculate_flash_fee_raw` + `calculate_net_profit_raw` (`opportunity.rs:181/468`) vs `min_net_base_raw` | ✅ |
| Sizing | same `flash_loan_fee_sf` closed forms + vault cap | ✅ |
| TX builder | `derive_swap_legs` floor = borrow+fee+tip+min_profit (same fee instance) | ✅ |
| Simulator | executes the actual compiled instructions (not re-derived math); typed fail-closed statuses | ✅ |
| Risk gate | consumes the same `real_net_base_raw` ladder | ✅ |
| On-chain executor | independent re-computation: `min_final_wsol_delta` + profit-guard equation | ✅ (defense-in-depth) |
| Realized PnL | balance-delta gauge only; estimates excluded | ✅ |

## 4. Findings

- **ARB-F-2026-11-22-N1 (P3):** DLMM two-pool dedicated optimizer (`dlmm_flash_arbitrage.rs`) is not reachable from `arb-bot` (see FL-F-2026-11-22-N2 in Phase 2). Its `ProfitBelowFloor` gate is quote-time only. Not a live defect; wire-up precondition registered.
- **ARB-F-2026-11-22-N2 (P3):** f64 `-ln` graph weights are unsuitable for formal verification of cycle profitability, but by design they only pre-screen; exact integer evaluation re-derives everything. Zero known precision-loss escape path; keep the `product <= 1.0` early-exit documented.
- **ARB-F-2026-11-22-N3 (P3):** "best observed" sizing is openly NOT a proven global optimum (multi-basin search with budget) — accepted limitation, documented in ISSUE-P0-02 resolution; no silent over-claim.

## 5. Evidence

```text
cargo test -p arb-engine → 190 passed / 0 failed
cargo test -p arb-risk   → 22 passed / 0 failed
```
Key named suites: `three_leg_net_economics_tests`, `route_graph_boundary_tests`, `trade_size_optimization_*`, `cross_dex_three_pool_live_loop`, `guarded/guard_01_execution_tests`, `hop_slippage_once_tests`.
