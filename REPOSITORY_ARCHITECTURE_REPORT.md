# REPOSITORY_ARCHITECTURE_REPORT.md

**Repository:** SOLANA_FLASHLOAN_ARBITUGE · Branch: newrepair · Date: 2026-11-16

## Workspace Structure

14 crates under `crates/`:

| Crate | Role | Dep direction |
|---|---|---|
| arb-types | leaf types (Lamports, Slot, Pubkey wrappers) | none |
| arb-constants | leaf constants (mints, program IDs) | none |
| arb-math | all quote math + canonical economics (leaf) | arb-types |
| arb-dex | protocol adapters (decode + quote) | arb-math, arb-types |
| arb-ingest | market data, provider supervision, freshness | arb-dex |
| arb-engine | route graph, discovery, sizing optimizer | arb-dex, arb-math |
| arb-risk | risk gate, kill switch, Chronos EV | arb-types |
| arb-tx | instruction builders, V0/V1 compiler, flash assemblers | arb-math, arb-types |
| arb-sim | simulation | arb-tx |
| arb-land | landing (Jito/RPC) | arb-tx |
| arb-store | persistence (SQLite/Postgres/Redis) | — |
| arb-metrics | telemetry | — |
| arb-config | config loading/validation | — |
| arb-bot | binary: main.rs wires everything; supervisor/ run loop | all |

## Execution Flow

`main.rs` → Config load → Wallet init → RPC init → Supervisor → `run.rs` (per-tick) → `strategy.rs` → `opportunity.rs` (gate ladder) → `dispatch.rs` → `recovery.rs` (dispatch-intent reconciliation).

## Invariants (from AGENTS.md, verified structurally)

- Atomicity: borrow+swaps+repay in ONE tx; `atomic_integrity::verify_atomic_integrity` wired.
- Repayment: `principal + fee`, fee from canonical klend formula; `BorrowTooSmall` compares RAW bits pre-rounding.
- Borrow sizing: bounded by decoded supply-vault liquidity; fail closed if unverifiable.
- Net profit: integer math, mint-aware, no f64 in authoritative decisions.
- V1: 0x81 discriminator, 4096-byte ceiling, 64 inline accounts, no ALT.
- No stale fallbacks: fail closed on stream failure.
- Ambiguous dispatch ≠ failure: check signature status first.

## Known traps (documented)

- Env vars process-global → ENV_LOCK in web_api tests.
- Glob re-exports collide → kamino curated list.
- `use super::*` vs `use crate::*` convention.
- Evidence files test-written (timestamp churn).
- Live-network tests skip on 0 pools (env, not bug).
- Supervisor.ingest.current_slot() returns u64 → wrap in Slot().
- axum 0.8 middleware/from_fn, DefaultBodyLimit.
- gh token cannot merge PRs.

## Verification status

Architecture claims are from source review + existing docs. No build/test run this session (cargo unavailable). Circular deps / race conditions / memory leaks not dynamically profiled.
