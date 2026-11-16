# PROJECT_STATE.md — Solana Atomic Arbitrage Engine

> Current-state snapshot. Updated at the end of every engineering campaign.
> For file-level detail see [`architecture/CODE_TREE.md`](architecture/CODE_TREE.md);
> for strategy/economics see [`architecture/BUSINESS_LOGIC.md`](architecture/BUSINESS_LOGIC.md);
> for gates and verdict see [`quality/PRODUCTION_READINESS_2026-11-13.md`](quality/PRODUCTION_READINESS_2026-11-13.md).

## What the system is

A Rust HFT workspace (edition 2024, pinned 1.99.1, 13 crates) executing
atomic Solana DEX arbitrage funded 100% by Kamino flash loans inside ONE
transaction per trade: `borrow → swap(s) → repay`. Solana Transaction V1
(0x81, 4,096-byte wire limit, inline accounts, no ALT) is used when a
transaction exceeds the 1,232-byte legacy limit. A React web application and
a Telegram bot share one control service. Safety defaults: `ARB_LIVE=0`,
`DRY_RUN=true`, shadow mode.

## Canonical pipeline (single path, verified wired)

```
arb-ingest (WS/gRPC streams + RPC reconcile, fail-closed freshness)
  → arb-engine (route graph, Bellman-Ford, cycle enumeration, sizing)
  → arb-math (canonical economics + canonical Kamino klend fee)
  → arb-risk (risk gate, kill switch)
  → arb-tx (protocol builders → flash assemblers → adaptive V0/V1 compiler)
  → arb-sim (preflight simulation)
  → arb-bot supervisor::dispatch (mode gate → SWQoS/Jito, one signed tx,
     dispatch intent persisted before send, landing tracker, no blind resubmit)
  → arb-land + arb-store (settlement, SQLite/Postgres, reconciliation)
```

## Protocol matrix (all verified against official IDLs/SDKs)

Raydium V4 / CPMM / CLMM / LaunchLab, Orca Whirlpool, Meteora DLMM / DAMM v2,
Pump.fun, PumpSwap. Dynamic account resolution everywhere; zero hardcoded
pools; fail-closed `TxError::MissingExecutionAccounts`. See
[`architecture/PROTOCOL_COMPATIBILITY.md`](architecture/PROTOCOL_COMPATIBILITY.md).

## Verification status (last full campaign: 2026-11-13, branch `vibe/store-fail-closed-9d81d9`, PR #11)

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings`: 0 warnings.
- `cargo test --workspace --all-targets` (with `.env` sourced): **961 passed,
  0 failed**, including property/fuzz tests for the V1 path, SDK differential
  tests (Meteora/Kamino/Solana), fork simulation, economic invariants, and
  Solami WS-key resolution.
- Real Helius RPC/WSS connectivity tested; live-network tests skip cleanly
  when the upstream RPC returns 0 pools (environmental, documented).
- **R-I store fail-closed remediation verified live**: positive smoke run
  (release binary, shadow mode, real Helius) created a durable SQLite file +
  WAL; negative smoke run with an impossible store path refused startup
  (exit 101, explicit fail-closed message) instead of silently falling back
  to in-memory. See `quality/TEST_EVIDENCE.md` and
  `audit/DATABASE_AND_RECOVERY_AUDIT.md`.
- Web app: authenticated Axum API (constant-time login, rate-limited, session
  tokens, security headers, 64 KB body limit, localhost bind) + React UI
  (sections A–I, WS reconnect with exponential backoff + jitter).

## Remediation register (R-items)

| Item | Status |
|---|---|
| R2 — remove dead `arb-mcp` crate | ✅ done |
| R3 — single canonical Kamino fee (arb-math canonical, arb-tx delegates) | ✅ done |
| R9 — split supervisor monolith (2,184 L run.rs → run/strategy/opportunity/recovery) | ✅ done |
| R10 — split arb-tx lib (4,799 L → 4 domain groups, 31 modules) | ✅ done |
| R12 — durable dispatch intents + startup reconciliation | ✅ done |
| Web hardening (body limit, security headers, WS backoff) | ✅ done |
| Test isolation (env-var lock for parallel tests) | ✅ done |

## Known honest limitations

- **No live broadcast ever**: `ARB_LIVE=0`; all dispatch paths fail closed in
  shadow mode. Live requires explicit operator authorization.
- **≥24h shadow soak not yet run** — required before any live transition.
- **Cross-DEX 3-pool live loop**: builders are tested end-to-end in
  simulation/fork; the full live-loop wiring awaits the gated live plan
  ([`operations/LIVE_PLAN_3POOL_V1_FLASH_ARB.md`](operations/LIVE_PLAN_3POOL_V1_FLASH_ARB.md)).
- Files >1,000 lines that remain (`compiler.rs` 1,490, `master_alt.rs` 1,417,
  `opportunity.rs` ~1,170) are single-responsibility; splitting them further
  would be line-count cosmetics, not clarity.

## Logical next step

System is in DEVELOPMENT COMPLETE / shadow-mandatory state. Next gates (in
order): 24h shadow soak on the VPS → performance report from `latency_bench`
→ operator review of
[`quality/PRODUCTION_READINESS_2026-11-13.md`](quality/PRODUCTION_READINESS_2026-11-13.md)
→ explicit live authorization decision.
