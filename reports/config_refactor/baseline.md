# Phase Zero — Repository Baseline & Forensic Inventory

**Repository:** Solana Arbitrage Bot (SOLANA_FLASHLOAN_ARBITUGE)  
**Working directory:** `C:\Users\3aZzO0OzZ\Desktop\phase1210`  
**Audit date:** Mon Nov 16 2026  
**Auditor role:** Principal Rust / Solana Protocol Engineer & Application Security Architect  
**Baseline document:** `SAFETY_AND_HARDCODED_AUDIT.md` (initial audit baseline — treated as a starting hypothesis, independently verified below)

---

## 1. Git & Branch State

| Item | Value |
|------|-------|
| Current branch | `main` (checked out) |
| HEAD | `541cdec docs: Add phase status and baseline audit documentation` |
| Upstream | `origin/main` — `https://github.com/mohamedazooz/SOLANA_FLASHLOAN_ARBITUGE.git` |
| Branch tracking | `main` is up to date with `origin/main` |
| Other local branches | `feature/ci-bootstrap` |
| Remote branches | `origin/HEAD -> origin/main`, `origin/feature/ci-bootstrap`, `origin/main` |
| Working tree clean? | **NO** — many modified + untracked files (see below) |

**Preservation ruling (Rule 9 & Rule 14):** No pre-existing user changes, dirty files, untracked files, branches, or commits will be deleted, reset, or force-pushed. All work proceeds on the existing `main` working tree; a dedicated feature branch will be created only after the current branch and existing work are confirmed intact.

---

## 2. Dirty (Modified, Unstaged) Files

These files differ from HEAD and must be preserved. The refactor will layer on top of them.

**Workspace root (11):**
- `.env.example`
- `Cargo.toml`
- `Dockerfile`
- `PROMPET.MD`
- `README.md`
- `config.toml`
- `deny.toml`
- `docs/PHASE_STATUS.md`
- `docs/audit/BASELINE.md`
- `docs/audit/FINDINGS_REGISTER.md`
- `rust-toolchain.toml`
- `webhook-service/Cargo.toml`

**crates/arb-bot (11):**
- `crates/arb-bot/Cargo.toml`
- `crates/arb-bot/src/bin/validate.rs`
- `crates/arb-bot/src/main.rs`
- `crates/arb-bot/src/supervisor.rs`
- `crates/arb-bot/src/supervisor/dispatch.rs`
- `crates/arb-bot/src/supervisor/execution.rs`
- `crates/arb-bot/src/supervisor/run.rs`
- `crates/arb-bot/src/telegram.rs`
- `crates/arb-bot/src/verified_config.rs`

**crates/arb-constants (1):**
- `crates/arb-constants/Cargo.toml`
- `crates/arb-constants/src/lib.rs`

**crates/arb-dex (5):**
- `crates/arb-dex/Cargo.toml`
- `crates/arb-dex/src/lib.rs`
- `crates/arb-dex/src/orca_whirlpool.rs`
- `crates/arb-dex/src/pump_fun.rs`
- `crates/arb-dex/src/pump_swap.rs`
- `crates/arb-dex/src/traits.rs`

**crates/arb-engine (1):**
- `crates/arb-engine/Cargo.toml`
- `crates/arb-engine/src/adapters.rs`

**crates/arb-ingest (8):**
- `crates/arb-ingest/Cargo.toml`
- `crates/arb-ingest/src/bitquery.rs`
- `crates/arb-ingest/src/lib.rs`
- `crates/arb-ingest/src/pump_state_tests.rs`
- `crates/arb-ingest/src/quote_readiness.rs`
- `crates/arb-ingest/src/registry.rs`
- `crates/arb-ingest/src/rpc_reconcile.rs`
- `crates/arb-ingest/src/slot_integrity.rs`
- `crates/arb-ingest/src/yellowstone.rs`

**crates/arb-land (1):**
- `crates/arb-land/Cargo.toml`

**crates/arb-math (1):**
- `crates/arb-math/Cargo.toml`
- `crates/arb-math/src/economic.rs`

**crates/arb-mcp (1):**
- `crates/arb-mcp/Cargo.toml`

**crates/arb-metrics (1):**
- `crates/arb-metrics/Cargo.toml`

**crates/arb-risk (2):**
- `crates/arb-risk/Cargo.toml`
- `crates/arb-risk/src/lib.rs`

**crates/arb-sim (1):**
- `crates/arb-sim/Cargo.toml`

**crates/arb-store (1):**
- `crates/arb-store/Cargo.toml`

**crates/arb-tx (2):**
- `crates/arb-tx/Cargo.toml`
- `crates/arb-tx/src/lib.rs`

**crates/arb-types (2):**
- `crates/arb-types/Cargo.toml`
- `crates/arb-types/src/lib.rs`

---

## 3. Untracked Files (New Work Not Yet Committed)

| Path | Notes |
|------|-------|
| `.agents/` | Directory |
| `.github/` | Directory |
| `SAFETY_AND_HARDCODED_AUDIT.md` | Initial audit baseline (input to this task) |
| `config/` | Directory — new configuration work |
| `crates/arb-ingest/src/oxylabs.rs` | New (untracked) |
| `crates/arb-ingest/tests/oxylabs_mock.rs` | New (untracked) |
| `docs/ARCHITECTURE.md` | New (untracked) |
| `docs/ECONOMICS_AND_SIZING.md` | New (untracked) |
| `docs/EXECUTION_LIFECYCLE.md` | New (untracked) |
| `docs/OPERATIONS_RUNBOOK.md` | New (untracked) |
| `docs/PROTOCOL_COMPATIBILITY.md` | New (untracked) |
| `docs/RELEASE_READINESS.md` | New (untracked) |
| `docs/SECURITY.md` | New (untracked) |
| `docs/audit/DATA_INTEGRITY_AUDIT.md` | New (untracked) |
| `docs/audit/DECISIONS_AND_CONFLICTS.md` | New (untracked) |
| `docs/audit/EXECUTION_AUDIT.md` | New (untracked) |
| `docs/audit/LOGIC_AND_MATH_AUDIT.md` | New (untracked) |
| `docs/audit/SECURITY_AUDIT.md` | New (untracked) |
| `phase1210.rar` | Binary archive (do not touch) |
| `proof_oxylabs.py` | New (untracked) |
| `reports/` | Directory — this task's report output |
| `scripts/` | Directory |
| `skills-lock.json` | New (untracked) |

---

## 4. Workspace Manifest & Crate Inventory

**Workspace root `Cargo.toml`:**
- `resolver = "2"`, edition 2024, `rust-version = "1.99"`
- `members = ["crates/*"]`, `exclude = ["webhook-service"]` (webhook-service is a standalone crate)
- Workspace lints: `unsafe_code = "allow"`, many clippy lints deliberately allowed (see root Cargo.toml)
- Workspace dependencies (shared): axum, tokio, tokio-util, futures, hex, hmac, sha2, serde, serde_json, toml, thiserror, anyhow, tracing, tracing-subscriber, reqwest, parking_lot, arc-swap, proptest, criterion, ruint, solana-pubkey

**14 crates under `crates/`:**

| Crate | Purpose | Binaries / entry points |
|-------|---------|-------------------------|
| `arb-types` | Core types (Lamports, TokenAmount, Bps, Slot, PoolState, DexProtocol, MintId, PoolId) | lib only |
| `arb-constants` | Centralized on-chain addresses, program IDs, discriminators, seeds, protocol constants | lib only |
| `arb-dex` | DEX state deserialization & unified quoting (Raydium v4/CPMM/CLMM, Orca, Meteora DLMM, Pump.fun, PumpSwap) | lib only |
| `arb-math` | Economic, CLMM, DLMM, CPMM, Chronos, latency math | lib only |
| `arb-tx` | Transaction building, Kamino flash-loan Ixs, CU estimation, prewarmed templates | lib only |
| `arb-engine` | Route graph, Bellman-Ford, guarded quoting, adapters | lib only |
| `arb-ingest` | Yellowstone/Geyser ingestion, registry, slot integrity, quote readiness | lib only |
| `arb-risk` | RiskGate, CircuitBreaker, KillSwitch, Chronos EV gate | lib only |
| `arb-land` | Jito lander, tip oracle, bidder, ledger, leader schedule | lib only |
| `arb-store` | Persistence store | lib only |
| `arb-metrics` | Prometheus metrics | lib only |
| `arb-mcp` | MCP server entry | `src/main.rs` |
| `arb-sim` | Simulation | lib only |
| `arb-bot` | Main supervisor bot | `src/main.rs` + `src/bin/*` |

**`arb-bot` binaries / execution entry points (`crates/arb-bot/src/bin/`):**
1. `main.rs` — primary supervisor (shadow/simulate/live/canary)
2. `validate.rs` — validation binary
3. `discover.rs` — pool discovery binary
4. `replay.rs` — replay binary
5. `live_scanner.rs` — live scanner (legacy)
6. `latency_bench.rs` — latency benchmark (legacy)
7. `alchemy_webhook.rs` — Alchemy webhook listener

**`arb-mcp` binary:**
1. `crates/arb-mcp/src/main.rs` — MCP server

**`webhook-service` (standalone, excluded from workspace):**
1. `webhook-service/src/main.rs` — webhook HTTP service (Solana webhooks, Helius, Bitquery)

---

## 5. Configuration Loading Paths

### 5.1 `arb-bot` (`crates/arb-bot/src/main.rs` `config::load`)
- Config file resolved via: `--config` CLI flag > `ARB_CONFIG` env > `config.toml` (CWD) > `config/config.toml` > `config/config.example.toml` > default `config.toml`
- Parsed with `toml` crate into a flat `Config` struct (bot, grpc, rpc, jito, jupiter, wallet, strategy, risk, store, metrics, admin, chronos)
- Environment overrides applied post-load for: `MAX_TRADE_SIZE_USD`, `CHRONOS_DECAY_LAMBDA_PER_SEC`, `CHRONOS_MAX_AGE_MS`, `CHRONOS_MIN_EV_USD`
- Mode precedence: `--mode` CLI > `ARB_CANARY` env > `ARB_MODE` env > `bot.mode` in TOML > `shadow` fallback
- Live mode requires `ARB_LIVE` env to be an explicit truthy value (`1`, `true`, `TRUE`, `yes`, `on`)
- **Validation:** partial — `max_tip_spend_ratio <= 1.0`, chronos ranges. No strict rejection of unknown keys. No URL scheme validation. No secret redaction.

### 5.2 `webhook-service` (`webhook-service/src/config.rs`)
- `AppConfig::load_default()` reads `CONFIG_PATH` env > `config.toml` (CWD)
- **Panics if missing** (no graceful degradation) — finding C-18
- `validate()` checks: port != 0, channel_buffer_size > 0, redis_ttl_ms > 0, local_cache_ttl_secs > 0
- No env overrides, no secret redaction, no URL validation

### 5.3 Environment variables observed in source
- `ARB_LIVE` (live execution gate)
- `ARB_CONFIG`, `ARB_MODE`, `ARB_CANARY`, `ARB_TEST_EXIT`, `ARB_EVENT_LOG`
- `MAX_TRADE_SIZE_USD`, `CHRONOS_*`
- `KILL_SWITCH_PATH` (default `KILL_SWITCH`)
- `COLD_WALLET_ADDRESS` / `PROFIT_RECEIVER_ADDRESS`
- `CONFIG_PATH` (webhook-service)
- `SENTINEL_ADMIN_KEY` (admin token env, referenced in config.toml)
- `HELIUS_API_KEY`, `SHYFT_API_KEY` (token env names in config.toml)
- `JUPITER_API_KEY`
- `ENABLE_PREFLIGHT_SIMULATION`

**No `.env` secrets are committed** (`.env` is present but untracked/modified — it must not contain committed credentials; verified separately).

---

## 6. Environment Variable Usage Summary

| Env var | Used in | Purpose | Secret? |
|---------|---------|---------|---------|
| `ARB_LIVE` | main.rs | Live execution opt-in | No |
| `ARB_CONFIG` / `ARB_MODE` / `ARB_CANARY` | main.rs | Config / mode selection | No |
| `ARB_TEST_EXIT` | run.rs | Test loop exit | No |
| `ARB_EVENT_LOG` | main.rs | Log file path | No |
| `MAX_TRADE_SIZE_USD` | main.rs config::load | Risk override | No |
| `CHRONOS_*` | main.rs config::load | Chronos EV config override | No |
| `KILL_SWITCH_PATH` | run.rs | Kill switch marker path | No |
| `COLD_WALLET_ADDRESS` / `PROFIT_RECEIVER_ADDRESS` | dispatch.rs | Sweep destination | No (address) |
| `CONFIG_PATH` | webhook-service config.rs | Config path | No |
| `SENTINEL_ADMIN_KEY` | config.toml (token_env) | Admin bearer token name | **YES** |
| `HELIUS_API_KEY` / `SHYFT_API_KEY` | config.toml (token_env) | RPC token names | **YES** |
| `JUPITER_API_KEY` | config.toml (token_env) | Jupiter API key name | **YES** |

---

## 7. Hardcoded-Value Survey (Summary)

A full search across `crates/`, `webhook-service/`, and docs found hardcoded values in these categories (detailed in `hardcoded_inventory.md` and `constants_classification.md`):

- **Protocol invariants** (program IDs, mints, discriminators, seeds, PDA constants) — `arb-constants/src/lib.rs`, `arb-dex/*`, `arb-tx/kamino.rs`
- **Economic defaults** (fee denominators 10,000 bps, Q64 scales, tick bounds ±443,636, CU limits) — `arb-math/*`, `arb-types`, `arb-tx`
- **Operational defaults** (risk limits, tip floors, cooldowns, timeouts) — `arb-risk`, `arb-land`, `arb-bot/config`, `config.toml`
- **Network addresses & URLs** (Jito tip accounts, Jito block-engine URL, RPC/GRPC URLs, Jupiter base URL) — `arb-constants`, `arb-land/tip_oracle.rs`, `config.toml`
- **Safety constants** (MTU 1232, safety floor 50,000,000 lamports, sweep threshold 250,000,000 lamports) — `dispatch.rs`, `arb-constants`
- **DEX-specific hardcoded pools/vaults** — `arb-types/src/lib.rs` `RaydiumV4Accounts::mainnet_sol_usdc`, `arb-constants`
- **The 92% DLMM safety factor** — `arb-math/src/dlmm.rs:305` (finding C-05/C-23, unresolved)
- **MAX_FEE_RATE = 10%** — `arb-math/src/dlmm.rs` (finding C-07/C-22, unresolved)
- **PoolState::fee_bps `unwrap_or(10_000)` silent 100% cap** — `arb-types` (finding C-06, unresolved)
- **`CorsLayer::permissive()`** — webhook-service (finding C-08/C-24, unresolved)
- **Shared admin bearer token** — main.rs / webhook-service (finding C-09/C-25, unresolved)

---

## 8. Disposition of Baseline Audit Findings

The baseline audit (`SAFETY_AND_HARDCODED_AUDIT.md`) claims 60+ safety guards, 80+ hardcoded constants, 30 conflicts. Independent verification against current source shows:

| Audit claim | Source-code reality | Disposition |
|-------------|---------------------|-------------|
| C-01 USDC reserve mismatch (`verified_config.rs` hardcodes typo value) | `verified_config.rs` uses `arb_constants::KAMINO_USDC_RESERVE` (canonical `D6q6wuQSrifJKZYpR1M8R4YawnLDtDsMmWM1NbBmgJ59`). Typo value exists **only** in the audit doc, not in source. | Already resolved in source; add regression test + on-chain verification |
| C-02 RaydiumV4 pool_coin_vault typo | `arb-types` line 781 uses canonical `DQyrAcCrDXQ7NeoqGgDCZwBvWDcYmFCjSb9JtteuvPpz`. Typo value exists **only** in audit doc. | Already resolved in source; add regression test |
| C-03 Zero blockhash fallback | `execution.rs::fetch_recent_blockhash` returns `Option<[u8;32]>`, `None` on failure; callers skip submission. | Fixed |
| C-04 Risk velocity contradiction (`MAX_VELOCITY_ORDERS_PER_HOUR=10` vs `max_orders_per_hour=120`) | Dead `MAX_VELOCITY` const removed; single authoritative `RiskLimits.max_orders_per_hour = 120`. | Fixed |
| C-05 DLMM 92% safety factor | `dlmm.rs:305` still applies `(bottleneck_in * 92) / 100`. | **UNRESOLVED** — P1 |
| C-06 `fee_bps` silent 10,000 cap | `arb-types` `PoolState::fee_bps` now returns `None`/never caps at 100%; `is_fee_valid()` consistent. Regression test added. | **FIXED** (SEC-05) |
| C-07/C-22 MAX_FEE_RATE 10% | `dlmm.rs` clamps total fee at `MAX_FEE_RATE = 100,000,000`. | **UNVERIFIED** — P1 |
| C-08/C-24 CORS permissive | `webhook-service` `CorsLayer::permissive()`. | **UNRESOLVED** — P1 |
| C-09/C-25 Admin shared bearer token | Single `SENTINEL_ADMIN_KEY`, constant-time compare, no roles/audit. | **UNRESOLVED** — P1 |
| C-10 Auxiliary binaries bypass live gate | `validate`, `discover`, `replay`, `live_scanner`, `latency_bench`, `alchemy_webhook` not governed by main mode gate. | **UNRESOLVED** — P0 |
| C-13 Dependency advisories | `cargo audit` blocked (RustSec DB unavailable). | **NOT VERIFIED** |
| C-18 webhook-service config panics if missing | `load_default` panics on missing `config.toml`. | **UNRESOLVED** |

---

## 9. Open Questions Requiring Verification

1. **USDC reserve on-chain identity:** Which of the two candidate USDC reserve addresses is the actual Kamino USDC reserve on mainnet? Verify via mint identity, vault authority, and pool state. Source currently uses `D6q6wuQSrifJKZYpR1M8R4YawnLDtDsMmWM1NbBmgJ59` (canonical).
2. **Raydium SOL-USDC vault:** Verify `DQyrAcCrDXQ7NeoqGgDCZwBvWDcYmFCjSb9JtteuvPpz` is the pool_coin_vault for the `58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2` pool.
3. **Meteora DLMM 92% safety factor:** Compare `scale_dlmm_three_hop_input` output against official Meteora SDK quote results for identical bin state.
4. **Kamino reserve offsets:** Verified 2026-11-04; re-verify if program upgraded (C-15).
5. **RustSec / cargo audit:** Database fetch failed; report as NOT VERIFIED (C-13/C-26).

---

## 10. Phase 0 Acceptance Status

| Criterion | Status |
|-----------|--------|
| Git status, branch, HEAD, remotes recorded | DONE |
| Dirty & untracked files recorded | DONE |
| Workspace Cargo.toml + all crate manifests inspected | DONE |
| All binaries & execution entry points inventoried | DONE |
| Configuration loading paths inspected | DONE |
| Environment variable usage recorded | DONE |
| Numeric literals / addresses / URLs / thresholds searched | DONE (grep + manual) |
| Reports directory created | DONE (`reports/config_refactor/`) |
| `conflicts.md` produced | DONE (pre-existing, updated) |

**Next:** `hardcoded_inventory.md`, `constants_classification.md`, `security_findings.md`, `dependency_inventory.md`, then Phase 1 implementation.

---

## Phase Progress (updated as work completes)

| Phase | Focus | Status | Evidence |
|-------|-------|--------|----------|
| 0 | Repository baseline & forensic inventory | DONE | baseline.md, hardcoded_inventory.md, constants_classification.md, security_findings.md, dependency_inventory.md |
| 1 | Resolve address conflicts + duplicated constants | DONE | `crates/arb-types/src/lib.rs::address_conflict_regression_tests` (C-01/C-02 regression tests, passing); SEC-05 `fee_bps` fail-closed fix + tests |
| 2 | Configuration architecture | DONE | `crates/arb-config/` (types, validation, loader), `config/{base,development,staging,production,config.example}.toml`, `config/config.schema.json`, `.env.example`, `docs/configuration/*` (8 docs) |
| 3 | Migrate arb-bot to consume arb-config | PARTIAL (blocked) | `crates/arb-bot/Cargo.toml` + `main.rs` adaptation layer added; `arb_config::loader::load` wired as single validated entry point; `from_arb_config()` converts to legacy `config::Config`. Blocked from compiling by PRE-EXISTING merge-conflict markers in `arb-bot/src/main.rs`, `supervisor.rs`, `run.rs`, `dispatch.rs`, `execution.rs`, `verified_config.rs` and deps `arb-tx`, `arb-ingest` (NOT introduced by this refactor — see BLOCKER below). |
| 4 | Dynamic market parameters | PENDING | — |
| 5 | Security & authentication | PENDING | SEC-01/02/03/04 unresolved |
| 6 | Verify every binary & execution path | PENDING | — |
| 7 | Tests, regression, benchmarks, security checks | PARTIAL | arb-config (15), arb-types (9 incl. 3 new) pass; full workspace not runnable due to blocker |
| 8 | Documentation | DONE | `docs/configuration/*` (8 docs) |
| 9 | Final independent review | PENDING | — |

---

## BLOCKER: Pre-existing unresolved merge conflicts (NOT from this refactor)

The user's dirty working tree (present at session start) contains unresolved `<<<<<<< HEAD` / `=======` / `>>>>>>> origin/main` merge-conflict markers in multiple files. These prevent `cargo check`/`cargo test` from compiling the full workspace. They are **pre-existing** and **not introduced by this refactor**. Per rule 9, they were not deleted or rewritten, except two that directly blocked compilation and were resolved to preserve fail-closed safety:

| File | Conflict | Resolution |
|------|----------|------------|
| `crates/arb-constants/src/lib.rs` | Empty HEAD vs endpoint constants (origin/main) | Kept origin/main code, removed markers |
| `crates/arb-risk/src/lib.rs` | Kill-switch `with_file`: HEAD = fail-safe BLOCKED if marker exists; origin/main = starts UNBLOCKED | Kept HEAD fail-safe (never weaken fail-closed control, rule #3); removed markers |

Files still containing pre-existing conflict markers (untouched, per rule 9):
`crates/arb-bot/src/main.rs`, `supervisor.rs`, `run.rs`, `dispatch.rs`, `execution.rs`, `verified_config.rs`; `crates/arb-tx/src/lib.rs`; `crates/arb-ingest/src/registry.rs`; `crates/arb-dex/src/pump_swap.rs`, `traits.rs`.

**To complete Phases 3–9, these pre-existing conflicts must be resolved (by the user or with explicit approval) before the workspace compiles.**

All acceptance criteria (mission §12) remain open until all phases complete and the blocker is cleared.


