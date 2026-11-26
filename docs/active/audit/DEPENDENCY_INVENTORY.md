# Dependency Inventory — Baseline (`b9b516c`)

**Audit Date**: 2026-11-26  
**Source**: Root `Cargo.toml`, per-crate `Cargo.toml`, `cargo tree --depth 1 -e normal --workspace`, `webhook-service/Cargo.toml`, `web/package.json`

---

## 1. Workspace Toolchain & Core Pins

- **Rust toolchain**: pinned `1.99.1`, `edition = "2024"`, `overflow-checks = true` in `[profile.release]`.
- **Workspace crates**: 15 members in `crates/*` + `programs/arb-executor` (`fuzz` excluded from workspace).
- **Removed dead dependencies (PR #167, verified absent)**: `yellowstone-grpc-client`, `solami`, dead `arb-ingest/yellowstone`, `arb-land/jito`, `arb-dex/orca-sdk` features removed; lockfile pruned by ~3,050 lines.

## 2. Key External Crates (Production & Dev)

| Dependency | Version / Feature Notes | Consuming Crates |
|---|---|---|
| `solana-pubkey` | Workspace pin (with `serde`, `bytemuck`, `curve25519` as needed) | All crates |
| `ruint` | Workspace pin (`U256` / fixed-point integer math) | `arb-types`, `arb-math`, `arb-dex` |
| `tokio` | Workspace pin (`rt-multi-thread`, `macros`, `signal`, `sync`, `time`) | `arb-ingest`, `arb-tx`, `arb-land`, `arb-store`, `arb-metrics`, `arb-bot` |
| `reqwest` | Workspace pin (`http2`, `json`, `rustls-tls`) | `arb-tx`, `arb-land`, `arb-bot` |
| `tokio-tungstenite` | `0.24` (`rustls-tls-webpki-roots`) | `arb-ingest`, `arb-land` |
| `ed25519-dalek` | `2.1` (`fast`, `std`) | `arb-tx` |
| `axum` | `0.8` (`ws`) | `arb-bot`, `webhook-service` |
| `rusqlite` | `0.42` (`bundled`, SQLite WAL) | `arb-store` |
| `tokio-postgres` / `redis` | `0.7` / `1.8.1` | `arb-store` |
| `prometheus` | `0.14` | `arb-metrics`, `webhook-service` |
| `orca_whirlpools_core` | `=2.1.1` (`dev-dependency` only, oracle parity tests) | `arb-math` |
| `proptest` / `criterion` | Dev / benchmark dependencies | `arb-math`, `arb-dex`, `arb-tx` |
| `litesvm` (optional) | Feature-gated inside `arb-sim` (`--features arb-sim/litesvm`) | `arb-sim` |

## 3. Frontend (`web/package.json`) & Webhook (`webhook-service`)

- `web/`: React / TypeScript / Vite `v6.4.6` (`npm run build` + `tsc --noEmit` clean).
- `webhook-service/`: Standalone Rust crate (`Cargo.toml` independent of root workspace) using Axum, HMAC/SHA256, Prometheus metrics.

## 4. Audit Gaps (Environment-Blocked)

- `cargo-audit` and `cargo-deny` are NOT installed on this Windows host (must run in Linux CI `.github/workflows/rust-ci.yml`, where `security` job executes `cargo-deny check`).
- Anchor / Agave SBF compiler tools (`cargo build-sbf` / `anchor`) are absent on this host, so `programs/arb-executor` is verified under host `cargo test` (`no-entrypoint` / unit harness), not SBF bytecode generation.
