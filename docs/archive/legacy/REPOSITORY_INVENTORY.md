> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Repository Inventory — 2026-11-16

## Workspace members (`crates/*`)

| Crate | Role |
|---|---|
| `arb-bot` | Supervisor/binary: dispatch, Jito service, control service, Telegram control, heartbeats, health monitor |
| `arb-config` | Centralized layered TOML config + env overlays + startup validation |
| `arb-constants` | Shared constants (program IDs, fees, limits) |
| `arb-dex` | DEX adapters / account decoders |
| `arb-engine` | Route graph + opportunity discovery |
| `arb-ingest` | Market-data ingestion (WS / gRPC / RPC reconcile) |
| `arb-land` | Landing reconciliation |
| `arb-math` | CPMM/CLMM/DLMM/DAMM-v2/LaunchLab quotes, economics, net-profit gate (`chronos.rs`), latency |
| `arb-metrics` | Metrics |
| `arb-risk` | Risk gate |
| `arb-sim` | Preflight simulation |
| `arb-store` | Persistence (sqlite/postgres/redis deps present) |
| `arb-tx` | Transaction planning + V0/V1 compilation, ALT integration tests |
| `arb-types` | Shared types |

Standalone (not in workspace): `webhook-service/` (axum webhook receiver:
Helius pump.fun, Bitquery, Solami handlers, dedup).

## Non-code assets

- `web/` — React control application (with `web/src`).
- `dashboard/` — dashboard assets.
- `config/` — `base.toml`, `development.toml`, `staging.toml`, `production.toml`,
  `execution.toml`, `protocols.toml`, `providers.toml`, `risk.toml`,
  `webhook.toml`, `config.example.toml`, `config.schema.json`.
- `.env.example` — committed; no real `.env`.
- `audit/` — python oracle (`audit/math_oracle/oracle.py`) + JSON test vectors
  (`cpmm_vectors.json`, `dlmm_single_bin_vectors.json`).
- `fixtures/` — test fixtures.
- `scripts/` — operational scripts.
- `reports/` — evidence artifacts from prior phases (phase15_2…16, production,
  v1_canary, v1_mainnet_probe, config_refactor, final_audit).

## Documentation map

- `README.md` — project overview, pipeline, status (shadow-mode mandatory).
- `docs/README.md`, `docs/PROJECT_STATE.md` — index + current snapshot.
- `docs/architecture/` — ARCHITECTURE, CODE_TREE, BUSINESS_LOGIC,
  EXECUTION_LIFECYCLE, PROTOCOL_COMPATIBILITY, WEB_APPLICATION.
- `docs/operations/` — DEPLOYMENT_RUNBOOK, OPERATIONS_RUNBOOK.
- `docs/economics/ECONOMICS_AND_SIZING.md` — cost model + borrow sizing.
- `docs/quality/` — TEST_EVIDENCE, SECURITY, PRODUCTION_READINESS_2026-11-13.
- `docs/adr/` — architecture decision records.
- `docs/audit/`, `docs/archive/` — historical audit campaigns (provenance only).

## CI

- GitHub Actions "Rust CI" — green on `main`.
- "Deploy to VPS" workflow exists (runs on `main` push).

## Build artifacts / generated

- `target/` (gitignored build cache) — not tracked.
- No committed lockfile drift: workspace `Cargo.lock` present.
