# Solana Atomic Arbitrage Engine — Kamino Flash-Loan Funded, Meteora DLMM

A Rust HFT workspace for atomic Solana DEX arbitrage: market-data ingestion,
route discovery, integer quote math, flash-loan sizing, transaction
construction (Solana Transaction V1 where required), simulation, dispatch,
landing reconciliation, persistence, and a React web control application.

## What it does

One canonical pipeline, one execution entry point:

```
Market Data (WS / gRPC / RPC reconcile)
  → Pool State Normalization (arb-ingest)
  → Unified Route Graph + Opportunity Discovery (arb-engine)
  → Unified Quote + Net-Profit Economics (arb-math)
  → Lightweight Risk Gate (arb-risk)
  → Transaction Planning + V0/V1 Compilation (arb-tx)
  → Preflight Simulation (arb-sim)
  → Signing + Dispatch (SWQoS / Jito) (arb-bot supervisor::dispatch)
  → Landing Reconciliation + Persistence (arb-land, arb-store)
```

Every trade is 100% funded by a Kamino flash loan inside ONE atomic
transaction: `borrow → swap A → swap B → (swap C) → repay`. Borrow sizing is
bounded by actual validated reserve supply-vault liquidity — never by a
configured USD ceiling alone. Net profit is enforced after every cost
(flash fee, DEX fees, priority fee, tip, base fee) with a configurable
minimum (`MIN_NET_PROFIT_USD`, default floor $0.05-style micro-arb).

## Status

**Shadow mode is mandatory.** `ARB_LIVE=0` / `DRY_RUN=true` are enforced by
configuration; live execution is disabled and requires an explicit operator
decision after all gates in
[`docs/quality/PRODUCTION_READINESS_2026-11-13.md`](docs/quality/PRODUCTION_READINESS_2026-11-13.md).
Current verdict and remaining limitations live there — do not treat quotes,
simulations, or RPC acceptance as realized profit.

## Documentation

| Path | Purpose |
|---|---|
| [`docs/README.md`](docs/README.md) | Documentation index (start here) |
| [`docs/PROJECT_STATE.md`](docs/PROJECT_STATE.md) | Current state snapshot |
| [`docs/architecture/ARCHITECTURE.md`](docs/architecture/ARCHITECTURE.md) | System architecture |
| [`docs/architecture/CODE_TREE.md`](docs/architecture/CODE_TREE.md) | Annotated file tree of every crate |
| [`docs/architecture/BUSINESS_LOGIC.md`](docs/architecture/BUSINESS_LOGIC.md) | Business analysis, strategy, and decision logic |
| [`docs/architecture/EXECUTION_LIFECYCLE.md`](docs/architecture/EXECUTION_LIFECYCLE.md) | Transaction lifecycle stages |
| [`docs/architecture/PROTOCOL_COMPATIBILITY.md`](docs/architecture/PROTOCOL_COMPATIBILITY.md) | Per-protocol support matrix |
| [`docs/architecture/WEB_APPLICATION.md`](docs/architecture/WEB_APPLICATION.md) | Web app + control service |
| [`docs/operations/DEPLOYMENT_RUNBOOK.md`](docs/operations/DEPLOYMENT_RUNBOOK.md) | Deployment (VPS, Docker, HTTPS) |
| [`docs/operations/OPERATIONS_RUNBOOK.md`](docs/operations/OPERATIONS_RUNBOOK.md) | Day-2 operations, incidents |
| [`docs/economics/ECONOMICS_AND_SIZING.md`](docs/economics/ECONOMICS_AND_SIZING.md) | Cost model and borrow sizing |
| [`docs/quality/TEST_EVIDENCE.md`](docs/quality/TEST_EVIDENCE.md) | Test campaign evidence |
| [`docs/quality/SECURITY.md`](docs/quality/SECURITY.md) | Secret inventory and rotation |
| [`docs/adr/`](docs/adr/) | Architecture decision records |
| [`docs/archive/`](docs/archive/) | Historical phase/audit reports (provenance only) |

## Workspace crates

| Crate | Responsibility |
|---|---|
| `arb-types` | Domain types (TokenAmount, mints, hops), dimensional accounting |
| `arb-constants` | Single source of truth for program IDs, seeds, discriminators |
| `arb-math` | AMM/CLMM/DLMM quote math, canonical Kamino fee, economic calculator |
| `arb-dex` | DEX quote adapters and account decoding |
| `arb-ingest` | WS/gRPC ingestion, provider supervision, state freshness |
| `arb-engine` | Route graph, discovery, sizing optimizer, guarded quotes |
| `arb-risk` | Risk gates, kill switch |
| `arb-tx` | Instruction builders, V0/V1 compiler, flash-loan assemblers |
| `arb-sim` | Simulation interfaces (optional `litesvm` feature) |
| `arb-land` | Landing/sender integrations |
| `arb-store` | SQLite WAL / Postgres persistence, dispatch intents |
| `arb-metrics` | Telemetry counters |
| `arb-bot` | Runtime: supervisor, config, web API, Telegram, control service |

## Build and validation

Rust edition 2024, pinned to **1.99.1** via `rust-toolchain.toml`. **Linux is
the reference build target** (a gRPC dependency is Unix-only; use Linux, WSL2,
or Docker).

```bash
bash scripts/verify-linux.sh          # one-shot: fmt, check, clippy, tests
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
```

Live-network tests read credentials from a git-ignored `.env` (see
`.env.example`). Never commit `.env`.

## Configuration and operation

- `config/config.example.toml` is the tracked reference; `config/config.toml`
  is deployment-provided and gitignored.
- `.env.example` documents every environment variable; secrets stay out of git.
- `KILL_SWITCH_FILE` (default `data/kill_switch`) is the durable pause state.
- The web API binds to localhost by default and is exposed through an
  authenticated HTTPS reverse proxy (Caddy/Nginx) — see the deployment runbook.
- Telegram and the web app share ONE control service; commands are identical.

## Safety policy

Shadow mode by default. No mainnet broadcasting, no live wallet signing, no
bundle submission without explicit operator authorization. Never report RPC
acceptance as landing, or simulation success as realized profit.
