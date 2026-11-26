# Repository Inventory — Baseline (`origin/main` @ `b9b516c`)

**Audit Date**: 2026-11-26  
**Baseline Commit**: `b9b516c` (Merge of PR #171 on `origin/main`)  
**Working Branch**: `phase/00-baseline-audit`  
**Toolchain**: `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, `cargo 1.99.1 (5f94df478 2026-08-27)`, `node v24.18.0`, `npm 11.16.0`

---

## 1. Workspace & Package Topology

The repository is a multi-package workspace containing **15 Rust library/binary crates** in `crates/*`, **1 native Solana on-chain program** in `programs/arb-executor`, **1 standalone Rust webhook service** (`webhook-service/`), **1 TypeScript/React web dashboard** (`web/`), and an excluded fuzz package (`fuzz/`).

| Component | Path | Type | Primary Role |
|---|---|---|---|
| `arb-constants` | `crates/arb-constants` | Library (leaf) | Program IDs, canonical mints, Jito tip accounts, discriminators |
| `arb-types` | `crates/arb-types` | Library (leaf) | Domain types (`Slot`, `Lamports`, `SignedLamports`, `UsdMicros`, `PoolState`, `SwapHop`, `ExecutionOpportunity`) |
| `arb-math` | `crates/arb-math` | Library (leaf math) | Canonical economics (`economic.rs`, Kamino fee math), CPMM, CLMM, DLMM, DAMM v2 integer quote math |
| `arb-config` | `crates/arb-config` | Library | Strict TOML config schema (`base`, `development`, `staging`, `production`, `testing`), freshness validation |
| `arb-risk` | `crates/arb-risk` | Library | Fail-closed risk gate, kill switch, BPS profit floors |
| `arb-store` | `crates/arb-store` | Library | SQLite (WAL), Postgres, and Redis persistence; dispatch intents & PnL |
| `arb-metrics` | `crates/arb-metrics` | Library | Prometheus text metrics exporter |
| `arb-dex` | `crates/arb-dex` | Library | DEX account decoders, Token-2022 extension parser, protocol quote engines |
| `arb-tx` | `crates/arb-tx` | Library | DEX swap IX builders, Kamino flash-loan assemblers, V0 & SIMD-0385 V1 compiler, atomic integrity verifier |
| `arb-land` | `crates/arb-land` | Library | Jito Block Engine client, SWQoS/RPC transport selector, adaptive tip/priority fee estimator |
| `arb-ingest` | `crates/arb-ingest` | Library | Helius WebSocket stream, snapshot hydration, slot/write-version integrity, provider supervision |
| `arb-engine` | `crates/arb-engine` | Library | `MarketGraph` route graph, cycle discovery, non-monotonic integer borrow sizer |
| `arb-sim` | `crates/arb-sim` | Library | RPC `simulateTransaction` client & response validator (`litesvm` optional feature) |
| `arb-idl-gen` | `crates/arb-idl-gen` | Library / Tool | IDL type generation helper |
| `arb-bot` | `crates/arb-bot` | Library + 7 Binaries | Supervisor loop (`run`, `strategy`, `opportunity`, `dispatch`, `recovery`), Web API, Telegram bot, CLI binaries |
| `arb-executor` | `programs/arb-executor` | Native Solana Program | On-chain 3-leg DLMM CPI executor (`pinocchio`/`solana-program`, **INACTIVE / undeployed**) |
| `webhook-service` | `webhook-service/` | Standalone Rust crate | Standalone Axum webhook listener (HMAC verification, Prometheus metrics, Docker/unit tests) |
| `web` | `web/` | Vite + React + TypeScript | Operator control dashboard (`npm run build`, `tsc --noEmit`, `security-contract.test.mjs`) |

---

## 2. Binaries in `crates/arb-bot`

Defined explicitly in `crates/arb-bot/Cargo.toml` (`autobins = false`):
- `arb-bot` (`src/main.rs`) — main daemon (`run` / `observe` / `simulate` / `canary` / `live` guarded by `ARB_LIVE=1` + `shadow-only` compile feature).
- `alchemy-webhook` (`src/bin/alchemy_webhook.rs`) — webhook consumer binary.
- `live-scanner` (`src/bin/live_scanner.rs`) — read-only scanner diagnostic.
- `validate` (`src/bin/validate.rs`) — live read-only config/quote/cycle validator.
- `discover` (`src/bin/discover.rs`) — pool discovery utility.
- `latency-bench` (`src/bin/latency_bench.rs`) — read-only RPC slot latency diagnostic.
- `replay` (`src/bin/replay.rs`) — historical replay binary.

---

## 3. Configuration, CI/CD, Deployment & Documentation Layout

- **Config profiles**: `config/{base,config.example,development,production,staging,testing}.toml`, `.env.example` (secrets live only in gitignored `.env`).
- **CI workflows**: `.github/workflows/rust-ci.yml` (`rust`, `webhook-service`, `security`, `secrets`, `web` jobs), `.github/workflows/deploy-vps.yml` (manual `workflow_dispatch`, `environment: production`, systemd `UnsetEnvironment` shadow enforcement, SHA-pinned checkout).
- **Branches preserved at audit start**:
  - `origin/main` at `b9b516c` (PR #171, 2026-11-26)
  - `phase/00-baseline-audit` branched cleanly from `origin/main` (`b9b516c`)
  - `fix/comprehensive-hft-remediation` (`9444bbc`, diverged earlier from `4b97cf1`) preserved untouched
- **Documentation drift observed**:
  - Root `README.md` contains stale counts ("13 crates"), false protocol claims (lists Raydium CLMM as fully simulated/supported), wrong on-chain stack claim ("Anchor (program)"), and false license claim ("No license file currently tracked" even though `LICENSE` is tracked).
  - Both `docs/active/*` and legacy top-level `docs/{architecture,audit,performance,quality,research,adr}/*` + root `AUDIT_BASELINE.md` / `FINAL_PRODUCTION_READINESS_REPORT.md` / `docs/DAMM_V2_FEE_SCHEDULER_AUDIT.md` exist simultaneously, violating clean `ACTIVE / INACTIVE / ARCHIVE` separation.
