# Phase 16 — Shadow Soak Report

**Date:** 2026-11-13
**Status:** ❌ **NOT RUN** — no 24-hour evidence exists.
**Verdict:** a soak cannot be asserted, and no numbers are invented here.

## 1. Why it did not run

| Requirement | Availability |
|---|---|
| ≥24 h continuous process | ❌ no persistent Linux host in this environment |
| RPC/WSS credentials | ⚠️ only a parent-directory RPC URL; no repo `.env` |
| Funded simulation payer | ❌ no `SOLANA_PRIVATE_KEY` |
| `ARB_LIVE=0` / `DRY_RUN=true` | ✅ shadow is the default, but moot without a run |
| Rust 1.99.1 Linux target | ⚠️ Windows host verified; WSL Ubuntu has no cargo |

A "soak" produced here would be a fabricated table. It is not produced.

## 2. Required soak protocol (ready to execute on a VPS)

Preconditions from `docs/production/SHADOW_MODE_RUNBOOK.md`:

- Linux + Rust 1.99.1; `.env` present (git-ignored) with Helius RPC/WSS.
- `ARB_LIVE=0`, `DRY_RUN=true`.
- Start: `set -a; . ./.env; set +a; cargo run --release -p arb-bot --bin arb-bot`
- Keep the process for **≥24 h**, retain logs, dashboard snapshots, and the
  daily report files.

## 3. Metrics that must be captured

The runbook and `LIVE_READINESS_CHECKLIST.md` require:

- opportunities detected (and per-route);
- gross and net expected profit (SOL and USD);
- quote freshness / slot lag at decision time;
- simulation success vs failure, with failure class;
- CU consumption (limit and consumed);
- estimated priority fee and Jito tip;
- rejection reasons (stage + reason);
- slot and execution latency (ingest→quote, risk, assembly, signing, RTT,
  landing);
- memory/CPU trend over the window (no leak, no task growth).

## 4. Known instrumentation gap (blocker B-4)

`crates/arb-store/migrations/001_init.sql` persists only:

- `attempts(id, slot, route_id, expected_net, tip_lamports, decision)`
- `outcomes(attempt_id, status, realized_net)`
- `fills(attempt_id, token, amount)`
- `dispatch_intents(...)`

It does **not** persist gross edge, full cost breakdown
(base/priority/tip/flash/DEX/rent), CU limit/consumed, rejection stage/reason,
or latency buckets. A 24 h soak can therefore only produce the metrics the
process emits to logs/telemetry, not a queryable evidence store. This must be
fixed (schema + writer) before the soak report can be independently verified.

## 5. Exit criteria for this deliverable

This report becomes valid only when all of the following are attached:

1. Process uptime ≥24 h (start/end timestamps, no restart, or restart log).
2. A machine-readable metric export covering §3 for the full window.
3. Stale-state rejection rate and simulated-vs-observed discrepancy summary.
4. Stable memory/CPU trend with the raw series.
5. A signed statement of the host and commit SHA used.

Until then, Gate 2 of `LIVE_READINESS_CHECKLIST.md` stays **unchecked**.
