# Competitor Activity Audit — Observable Competition Signals

> Scope: master directive §6 (competitor estimation, §6.1 market-activity
> metrics, §6.2 executable depth). Classification per claim as elsewhere in
> this audit series. Audited 2026-11-19 (P9 pass). Baseline: `f72afbc`.

---

## 1. Honest visibility boundary (directive-mandated)

**No searcher can enumerate all pending competing transactions via public RPC
or gRPC** — there is no public pending-tx stream. The directive (§6) forbids
claiming otherwise. The project's competition model therefore relies only on
**observables**: Jito tip auction floors, own-bundle landing outcomes,
leader schedules, and pool write-lock state. This document justifies each
signal and marks what is **not** implemented.

## 2. Implemented signals [code]

| Signal | Where | Mechanism |
|---|---|---|
| Tip auction level | `TipOracle` (`supervisor/jito_service.rs`) | Polls Jito `getTipFloor` (1 s cadence, `arb_land::DEFAULT_MAX_AGE_MS` freshness gate — stale floors stop bidding) |
| Competitive pressure factor | `AdaptiveBidder::competition_factor` (`jito_service.rs:215-216`) | Derived from tip-floor distribution (percentile rungs); clamps bid escalation |
| Own landing outcomes | Landing ledger `record` / `landing_rate` (`jito_service.rs:305,313`) | Per-rung `InflightStatus` outcomes → measured inclusion rate feeds calibrated inclusion probability (`:296`) |
| Leader awareness | `plan_with_context` (`jito_service.rs:233`) + `Landability` | Bids **refuse** on non-Jito leaders (`TipDecision::Refuse{LeaderNotJito}`); routes demote to `SwqosDirect`/`PlainRpc` only above the `swqos_fallback_hurdle_lamports` edge and only when not write-lock-hot (`:257,266`) |
| Write-lock contention | `is_hot_write_lock` input to planning | Hot account locks block SWQoS/plain demotion (avoids paying plain-RPC for lost races) |
| Order-flow / spread | `pump_spread_bps`, `pump_opportunities_total` gauges (`arb-metrics`) | Spread as activity proxy on pump pools |

Verified against official sources in
`docs/research/2026-solana-hft-best-practices.md` (live floors 2026-11-18:
p50 = 1,371 lamports, p99 = 7.53e-4 SOL, absolute min 1,000 lamports) [docs].

## 3. Detailed competitive signals — not implemented [code/assum]

Directive §6 also asks for: recent transactions touching pool accounts, pool
update frequency, failed-tx frequency, slot-level activity, repeated-pattern /
wallet clustering, and trader classification (Confirmed Bot / Probable /
MM / Unknown). Current code implements **none** of the wallet-clustering or
per-pool swap-rate monitors. This is an honest gap, recorded as:

- **COMP-1** [open → P9]: per-pool observed swap rate + failed-tx ratio from
  authored tx scraping is not implemented.
- **COMP-2** [open → P9]: trader classification (bot/MM/wallet) not implemented;
  no claim is made anywhere in code or docs that it exists.

The mitigation in the meantime is structural: the bidding/ladder logic treats
competition **implicitly** — if competitors outbid, our bundles fail to land
and `landing_rate` (measured, not assumed) drops, which drives the adaptive
bidder's calibrated probability down and refuses economically-local bids
(`tip never consumes the min-profit floor` invariant, see
`jito-submission-audit.md` §Tip-cap).

## 4. Market-activity metrics (§6.1) status [code]

| Metric | Status |
|---|---|
| Trades per second | not implemented [unk] |
| Unique active traders | not implemented [unk] |
| Pool update rate | partially — `account_update_lag_ms` + slot-gap counters exist; per-pool rate not aggregated |
| Failed transaction ratio | derivable from landing ledger (our own only) [code] |
| Landing probability | `calibrated_inclusion_probability` [code] |
| Price volatility | not implemented as a metric [unk] |

## 5. Executable depth (§6.2) [code]

Depth handling is **per-route exact quoting**, not a fixed SOL threshold:
each candidate route is quoted at multiple candidate borrow sizes through the
CLMM tick / DLMM bin traversing quote engine (`arb-dex` adapters, `arb-math`
sizing), and sizing is additionally bounded by live Kamino supply-vault
liquidity (fail-closed if unverifiable). Directive-conformant: no global
"5/10/20 SOL" constant exists in the sizing path; the only SOL-based constant
is `RANKING_SOL_USD` (DexScreener **ranking only**, named and documented —
DATA-06 fix, never a sizing/profit input).

## 6. Findings table

| ID | Finding | Severity | Status |
|---|---|---|---|
| COMP-1 | Per-pool swap-rate / failed-tx competition monitor absent | capability gap, P9 | open |
| COMP-2 | Trader classification absent (explicitly not claimed) | capability gap, P9 | open |
| COMP-3 | Competition model is implicit (via measured landing outcomes + tip floors) — valid and fail-safe, but less predictive | design note | accepted for now |

## 7. Evidence

- `crates/arb-bot/src/supervisor/jito_service.rs:101,119,215,233,257,266,296,305,313`
- `crates/arb-land/src/lib.rs` — `DEFAULT_MAX_AGE_MS`, tip-floor provider plumbing.
- `crates/arb-metrics/src/lib.rs:25-40` — spread / opportunity counters.
- `docs/research/2026-solana-hft-best-practices.md` — live Jito floors with fetch date.
- `DATA-06` fix: `crates/arb-ingest/src/pumpfun_discovery.rs` (`RANKING_SOL_USD` scoping).
