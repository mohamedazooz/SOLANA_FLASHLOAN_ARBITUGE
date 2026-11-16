# Dependency Inventory

**Repository:** Solana Arbitrage Bot  
**Audit date:** Mon Nov 16 2026  
**Toolchain:** Rust 1.99.1 (pinned in rust-toolchain.toml), edition 2024, resolver 2  
**Scope:** Workspace `crates/*` (14 crates) + standalone `webhook-service`

---

## 1. Workspace-Level Dependencies (root Cargo.toml)

These are shared versions pinned at the workspace level and inherited by crates via `{ workspace = true }`.

| Crate | Version | Purpose | Security notes |
|-------|---------|---------|----------------|
| axum | 0.8.9 | HTTP web framework (admin API, metrics) | — |
| tokio | 1 | Async runtime (rt-multi-thread, macros, sync, time, signal, io-util) | — |
| tokio-util | 0.7 | Tokio utilities | — |
| futures | 0.3 | Async primitives | — |
| hex | 0.4.3 | Hex encoding | — |
| hmac | 0.13.0 | HMAC | — |
| sha2 | 0.11.1 | SHA-2 family | — |
| serde | 1 | Serialization framework | — |
| serde_json | 1 | JSON serialization | — |
| toml | 0.8 | TOML parsing (config) | — |
| thiserror | 2 | Error types | — |
| anyhow | 1 | Error handling | — |
| tracing | 0.1 | Structured logging | — |
| tracing-subscriber | 0.3 (json, env-filter) | Tracing subscribers | — |
| reqwest | 0.12 (rustls-tls, json, http2) | HTTP client | Uses Rustls (not OpenSSL) — good |
| parking_lot | 0.12 | Fast locks | — |
| arc-swap | 1 | Atomic pointer swap | — |
| proptest | 1 | Property-based testing | dev dependency |
| criterion | 0.5 | Benchmarking | dev dependency |
| ruint | 1.12 | Fixed-width unsigned integers | — |
| solana-pubkey | 2.2 | Solana Pubkey type | — |

**Workspace lints:** `unsafe_code = "allow"` and many clippy lints deliberately allowed (see root Cargo.toml). This is permissive — clippy is not enforcing strictness, which is a supply-chain/code-quality concern but not a direct vulnerability.

---

## 2. Per-Crate Dependencies

### arb-bot (main supervisor + 7 binaries)
Workspace deps +:
| Crate | Version | Purpose |
|-------|---------|---------|
| dotenvy | 0.15 | .env loading |
| base64 | 0.22 | Base64 encoding |
| bs58 | 0.5 | Base58 encoding (Solana addresses) |
| All 12 workspace path crates (arb-*) | — | Internal crates |

Binaries: `arb-bot` (main), `alchemy-webhook`, `live-scanner`, `validate`, `discover`, `latency-bench`, `replay`.

### arb-math (economic, CLMM, DLMM, CPMM, Chronos math)
Workspace deps +:
| Crate | Version | Purpose |
|-------|---------|---------|
| ruint | workspace | Fixed-width unsigned integers |
| solana-pubkey | workspace | Pubkey |
| thiserror | workspace | Errors |
| proptest | workspace (dev) | Property tests |
| criterion | workspace (dev) | Benchmarks |

### arb-ingest (Yellowstone/Geyser ingestion, registry, slot integrity)
Workspace deps +:
| Crate | Version | Purpose |
|-------|---------|---------|
| async-trait | 0.1 | Async traits |
| tokio-tungstenite | 0.24 (rustls-tls-webpki-roots, connect) | WebSocket client |
| yellowstone-grpc-client | 16.0.1 | Solana Geyser gRPC client |
| yellowstone-grpc-proto | 14.0.1 | Geyser protobufs |
| base64 | 0.22 | Base64 |
| dotenvy | 0.15 | .env loading |
| ruint | workspace | Fixed-width ints |
| futures | workspace | Async primitives |

### arb-tx (transaction building, Kamino flash-loan, CU estimation)
Workspace deps +:
| Crate | Version | Purpose |
|-------|---------|---------|
| bs58 | 0.5 | Base58 |
| base64 | 0.22 | Base64 |
| ed25519-dalek | 2.1 (fast, std) | Ed25519 signing |
| solana-pubkey | workspace (curve25519) | Pubkey |

### arb-dex (DEX state deserialization, unified quoting)
Workspace deps +:
| Crate | Version | Purpose |
|-------|---------|---------|
| ruint | workspace | Fixed-width ints |
| solana-pubkey | workspace (curve25519) | Pubkey |

### arb-land (Jito lander, tip oracle, bidder, ledger, leader)
Workspace deps +:
| Crate | Version | Purpose |
|-------|---------|---------|
| tokio-tungstenite | 0.24 (connect, url, rustls-tls-webpki-roots) | WebSocket (tip stream) |
| url | 2.5 | URL parsing |
| base64 | 0.22 (alloc) | Base64 |

### arb-risk (RiskGate, CircuitBreaker, KillSwitch, Chronos)
Workspace deps +:
| Crate | Version | Purpose |
|-------|---------|---------|
| parking_lot | workspace | Locks |
| tracing | workspace | Logging |

### arb-engine (route graph, Bellman-Ford, adapters)
Workspace deps +:
| Crate | Version | Purpose |
|-------|---------|---------|
| base64 | 0.22 | Base64 |
| serde_json | workspace | JSON |

### arb-types (core types)
Workspace deps +:
| Crate | Version | Purpose |
|-------|---------|---------|
| thiserror | workspace | Errors |
| serde | workspace | Serialization |
| solana-pubkey | workspace (serde, bytemuck) | Pubkey |

### arb-constants (protocol constants)
| Crate | Version | Purpose |
|-------|---------|---------|
| solana-pubkey | workspace | Pubkey |

### arb-store (persistence)
Workspace deps +:
| Crate | Version | Purpose |
|-------|---------|---------|
| rusqlite | 0.32 (bundled) | SQLite |
| redis | 1.8.1 | Redis client |
| sqlx | 0.8 (runtime-tokio, tls-rustls, postgres, sqlite, macros, migrate) | Async SQL |
| base64 | 0.22 | Base64 |

### arb-metrics (Prometheus metrics)
Workspace deps +:
| Crate | Version | Purpose |
|-------|---------|---------|
| prometheus | 0.13 | Prometheus client |
| once_cell | 1 | Once-cell initialization |

### arb-sim (simulation)
Workspace deps +:
| Crate | Version | Purpose |
|-------|---------|---------|
| bincode | 1.3 | Binary serialization |
| solana-transaction | 4.3 (serde, optional) | Solana transactions |
| litesvm | 0.18.2 (optional) | Local SVM simulation |

### arb-mcp (MCP server)
Workspace deps +:
| Crate | Version | Purpose |
|-------|---------|---------|
| (internal path crates only) | — | arb-types, arb-constants, arb-dex, arb-ingest |

### webhook-service (standalone, edition 2021, NOT in workspace)
| Crate | Version | Purpose | Security notes |
|-------|---------|---------|----------------|
| tokio | 1.0 (full) | Async runtime | Full features — larger attack surface than workspace's minimal features |
| axum | 0.7 (ws) | HTTP + WebSocket | Older major version than workspace (0.8.9) — version drift |
| serde | 1.0 (derive) | Serialization | — |
| serde_json | 1.0 (arbitrary_precision) | JSON | — |
| toml | 0.8 | TOML config | — |
| dashmap | 6.0 | Concurrent map | — |
| redis | 0.25 (tokio-comp, aio, connection-manager) | Redis | — |
| futures-util | 0.3 | Async utilities | — |
| tracing | 0.1 | Logging | — |
| tracing-subscriber | 0.3 (env-filter, fmt) | Logging | — |
| async-trait | 0.1 | Async traits | — |
| anyhow | 1.0 | Errors | — |
| thiserror | 1.0 | Errors | — |
| tower | 0.4 (util) | Service utilities | — |
| tower-http | 0.5 (trace, cors) | CORS + tracing | CORS layer used here (SEC-01) |
| reqwest | 0.11 (json) | HTTP client | Older than workspace's 0.12 — version drift |
| dotenvy | 0.15 | .env loading | — |

**webhook-service release profile:** `panic = "abort"`, `overflow-checks = true`, `lto = "fat"`. `panic = "abort"` means unwinding is disabled — acceptable for a service but differs from workspace crates.

---

## 3. Duplicate / Version-Drift Detection

| Dependency | Workspace version | webhook-service version | Drift? |
|------------|-------------------|-------------------------|--------|
| tokio | 1 (minimal features) | 1.0 (full) | Feature drift (full vs minimal) |
| axum | 0.8.9 | 0.7 | **Major version drift** |
| serde | 1 | 1.0 | OK |
| serde_json | 1 | 1.0 | OK |
| toml | 0.8 | 0.8 | OK |
| tracing | 0.1 | 0.1 | OK |
| tracing-subscriber | 0.3 | 0.3 | OK |
| anyhow | 1 | 1.0 | OK |
| thiserror | 2 | 1.0 | **Major version drift** |
| reqwest | 0.12 | 0.11 | **Minor version drift** |
| redis | 1.8.1 (arb-store) | 0.25 (webhook-service) | **Major version drift** |
| tower-http | — | 0.5 | Only in webhook-service |

**Risk:** Version drift between the workspace crates and webhook-service means security patches may apply to one but not the other. `axum` 0.7 vs 0.8.9 and `thiserror` 1 vs 2 are the most notable drifts.

---

## 4. Security-Critical Dependencies

| Crate | Why security-critical | Version | Verification needed |
|-------|----------------------|---------|-------------------|
| solana-pubkey | Address/key handling | 2.2 | Verify no key material leakage in Debug |
| ed25519-dalek | Transaction signing | 2.1 | Verify constant-time ops, no side channels |
| reqwest | HTTP client (RPC, Jito, Jupiter) | 0.12 / 0.11 | Verify TLS config, no credential logging |
| tokio-tungstenite | WebSocket (tip stream, Geyser) | 0.24 | Verify TLS root config |
| tokio | Async runtime | 1 | — |
| axum | HTTP framework | 0.8.9 / 0.7 | CORS configuration (SEC-01) |
| tower-http | CORS layer | 0.5 | `CorsLayer::permissive()` — SEC-01 |
| redis | Dedup / caching | 1.8.1 / 0.25 | Redis ACL / TLS |
| rusqlite / sqlx | Persistence | 0.32 / 0.8 | SQL injection via parameterized queries only |
| litesvm | Local simulation | 0.18.2 | Simulation fidelity vs mainnet |

---

## 5. Supply Chain Checks Required

| Check | Tool | Status | Notes |
|-------|------|--------|-------|
| cargo audit | RustSec advisory DB | NOT VERIFIED | DB fetch failed — report as NOT VERIFIED, do not claim a clean audit |
| cargo deny | deny.toml present | Present | Run `cargo deny check` and report |
| Dependency license checks | cargo deny | To be run | MIT OR Apache-2.0 workspace license |
| Duplicate dependency detection | cargo deny / cargo tree | To be run | Version drift noted above |
| Dependency source restrictions | — | Not configured | No cargo source replacement configured |
| Secret scanning | grep / secret scanner | To be run | `.env` must never be committed |
| SBOM generation | — | Not present | Not configured |
| Build provenance | — | Not present | Not configured |
| Signed release artifacts | cosign / similar | Not present (C-14/C-27) | Not configured |

**Rule:** If RustSec or another vulnerability database is unavailable, report the check as NOT VERIFIED. Do not report a successful security audit when the underlying database was inaccessible.

---

## 6. Dependency Risk Summary

| Risk level | Count | Items |
|------------|-------|-------|
| High | 1 | `cargo audit` NOT VERIFIED (RustSec DB unavailable) |
| Medium | 4 | Version drift: axum (0.8.9 vs 0.7), thiserror (2 vs 1), reqwest (0.12 vs 0.11), redis (1.8.1 vs 0.25) |
| Medium | 1 | `unsafe_code = "allow"` workspace lint (permissive) |
| Medium | 1 | `panic = "abort"` in webhook-service release profile |
| Low | — | All other dependencies are standard, well-maintained crates |

---

## 7. No New Dependencies (Rule 11)

Per Rule 11 ("Never introduce unnecessary dependencies or overengineering"), the refactor must not add new external crates unless absolutely necessary. Configuration parsing will use the existing `toml` + `serde` stack already in the workspace. Secret provider abstraction will use environment variables (std library) with a trait interface — no new secret-manager crate unless the operator explicitly provisions one.
