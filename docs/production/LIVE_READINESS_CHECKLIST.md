# Live Readiness Checklist

Every line needs dated evidence. Nothing may be checked by declaration.
Live mode stays **disabled** (`ARB_LIVE=0`, `DRY_RUN=true`) until every
gate below is checked by the human owner.

## Gate 1 — Code and tests

- [ ] CI green on the merge commit (fmt, clippy `-D warnings`, full suite, web build).
- [ ] No critical/high unresolved finding in `docs/audit/` + `docs/audits/`.
- [ ] Recovery deterministic tests green (block-height evidence, restart, no-resubmit).
- [ ] Property/fuzz suites green (fee rounding, dust invariants, V1 serialization).

## Gate 2 — Shadow soak (≥24 h, required)

- [ ] ≥24 h continuous shadow run with `DRY_RUN=true`.
- [ ] Report: opportunity count, rejected count, simulation success rate,
      stale-state rejection rate, predicted-vs-observed discrepancies.
- [ ] Predicted outputs match observed pool state within tolerance.
- [ ] Memory/CPU stable over the soak (no growth trend, no task leaks).
- [ ] Daily reports and error logs retained.

## Gate 3 — Infrastructure

- [ ] Reverse proxy terminates TLS; bot API binds localhost only.
- [ ] `WEB_ADMIN_PASSWORD` set to a strong secret (in `.env`, never committed).
- [ ] Telegram allowlist configured; `/live` requires `CONFIRM-LIVE`.
- [ ] Database backups/restores exercised (SQLite file or PostgreSQL dump).
- [x] Store selection fail-closed verified (R-I): unreachable PostgreSQL or
  unopenable SQLite file aborts startup — no silent in-memory fallback;
  `sqlite://` URLs resolve to real file paths (regression tests in
  `arb-bot/src/supervisor.rs::tests`).
- [ ] Provider failover observed live (primary WSS drop → rotation, no stale fallback).

## Gate 4 — Authorized canary (human approval required)

- [ ] Explicit owner authorization recorded (who/when/scope).
- [ ] Controlled wallet funding: minimal, single-purpose hot wallet.
- [ ] Maximum position size configured (`strategy.max_borrow_size`).
- [ ] Maximum daily loss + failure circuit breaker configured and tested.
- [ ] Maximum concurrent in-flight transactions = 1 for the canary.
- [ ] Borrow minimum/maximum bounds verified against live Kamino liquidity.
- [ ] Slippage limits per hop verified in the built transaction.
- [ ] First canary transaction simulated end-to-end (borrow → 3 swaps → repay).
- [ ] Realized PnL independently reconciled from on-chain balances
      (never from bot accounting alone).
- [ ] Kill switch + emergency stop exercised live once (abort path proven).

## Gate 5 — Live operation

- [ ] All Gate 1–4 items checked with dated evidence.
- [ ] `ARB_LIVE=1` set ONLY at this point, by the owner, with `CONFIRM-LIVE`.
- [ ] Monitoring + Telegram alerts confirmed operational before first send.
- [ ] Rollback plan: `stop` → `emergency-stop` → restart restores paused shadow state.

## Standing prohibitions

- Never enable live mode automatically or on a timer.
- Never bypass the RiskGate or fire-time kill-switch gate.
- Never resubmit an ambiguous transaction (recovery never resubmits).
- Never claim landing/profit without on-chain signature/status/balance proof.
