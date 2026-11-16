# Phase 0 — Project Baseline Report

## 0.1 Git State

- **Current branch**: `main`
- **HEAD commit**: `541cdec` — "docs: Add phase status and baseline audit documentation"
- **Parent commit**: `c42c387` — "feat: complete solana flashloan arbitrage workspace phase 12"
- **Origin remote**: `https://github.com/<org>/phase1210.git` (fetch + push)
- **Origin remote (feature branch)**: `https://github.com/<org>/phase1210.git` (fetch only)
- **Git remotes**: 2 configured (origin main, origin feature/ci-bootstrap)

## 0.2 Uncommitted Modifications

The working directory has extensive modifications across the repository. Key modified files include:

### Root-level modifications:
- `.env.example` — modified
- `Cargo.toml` — modified (workspace root)
- `Dockerfile` — modified
- `PROMPET.MD` — modified
- `README.md` — modified
- `config.toml` — modified
- `rust-toolchain.toml` — modified
- `SAFETY_AND_HARDCODED_AUDIT.md` — modified
- `deny.toml` — modified

### Crate modifications (all modified):
- `crates/arb-bot/` — Cargo.toml, main.rs, supervisor.rs, dispatch.rs, execution.rs, run.rs, verified_config.rs, telegram.rs, bin/validate.rs, bin/replay.rs, bin/live_scanner.rs, bin/latency_bench.rs, bin/discover.rs, bin/alchemy_webhook.rs, jito_service.rs
- `crates/arb-constants/` — Cargo.toml, src/lib.rs
- `crates/arb-dex/` — Cargo.toml, src/lib.rs, orca_whirlpool.rs, pump_fun.rs, pump_swap.rs, traits.rs
- `crates/arb-engine/` — Cargo.toml, src/adapters.rs
- `crates/arb-ingest/` — Cargo.toml, lib.rs, yellowstone.rs, registry.rs, quote_readiness.rs, rpc_reconcile.rs, slot_integrity.rs, bitquery.rs, pump_state.rs, pump_state_tests.rs, dexscreener.rs, oxylabs.rs, rpc_pool.rs, dependency_manifest.rs, solami_stream.rs, pumpdev_stream.rs, pumpfun_discovery.rs, gmgn.rs, bitquery_ws_live.rs
- `crates/arb-land/` — Cargo.toml, src/lib.rs, ledger.rs, leader.rs, bidder.rs
- `crates/arb-math/` — Cargo.toml, src/economic.rs, src/lib.rs, src/dlmm.rs, src/clmm.rs, src/cpmm.rs, src/chronos.rs, latency.rs, tests/
- `crates/arb-tx/` — Cargo.toml, src/kamino.rs, alt.rs, template.rs, jupiter.rs, lib.rs, tests/
- `crates/arb-risk/` — Cargo.toml, src/lib.rs
- `crates/arb-sim/` — Cargo.toml, src/litesvm_sim.rs
- `crates/arb-metrics/` — Cargo.toml, src/lib.rs
- `crates/arb-store/` — Cargo.toml, src/lib.rs, src/redis_cache.rs, src/postgres_store.rs
- `crates/arb-mcp/` — Cargo.toml, src/lib.rs, src/main.rs, src/tools.rs

## 0.3 Untracked Files

New directories and files present in the workspace:
- `.agents/`
- `.github/`
- `config/` — new configuration directory
- `crates/arb-ingest/src/oxylabs.rs` — new file
- `crates/arb-ingest/tests/oxylabs_mock.rs` — new test file
- `docs/ARCHITECTURE.md` — new doc
- `docs/ECONOMICS_AND_SIZING.md` — new doc
- `docs/EXECUTION_LIFECYCLE.md` — new doc
- `docs/OPERATIONS_RUNBOOK.md` — new doc
- `docs/PROTOCOL_COMPATIBILITY.md` — new doc
- `docs/RELEASE_READINESS.md` — new doc
- `docs/SECURITY.md` — new doc
- `docs/audit/DATA_INTEGRITY_AUDIT.md` — new doc
- `docs/audit/DECISIONS_AND_CONFLICTS.md` — new doc
- `docs/audit/EXECUTION_AUDIT.md` — new doc
- `docs/audit/LOGIC_AND_MATH_AUDIT.md` — new doc
- `docs/audit/SECURITY_AUDIT.md` — new doc
- `phase1210.rar` — archive
- `reports/` — new directory
- `scripts/` — new directory
- `skills-lock.json` — new file

## 0.4 Worktrees

No worktrees currently configured.

## 0.5 Stashes

No stashes present.

## 0.6 Tags

No tags present in the repository.

## 0.7 Build Status

- `cargo fmt --all -- --check`: Had formatting diffs (resolved by running `cargo fmt --all`)
- `cargo check --workspace --all-targets`: Compiles successfully with 1 warning (unused `mut` variable in `arb-ingest/src/oxylabs.rs:351`)
- `cargo clippy --workspace --all-targets -- -D warnings`: FAILS due to the same unused `mut` variable warning
- `cargo test --workspace`: Passes (only doc-tests executed; no integration tests ran due to environment limitations)

## 0.8 Repository Inventory Summary

The workspace contains 23 crates organized into the following domains:

| Crate | Purpose |
|-------|---------|
| `arb-constants` | Centralized on-chain addresses, program IDs, and protocol constants |
| `arb-types` | Core domain types (Lamports, TokenAmount, Bps, Slot, PoolKind, PoolSnapshot, CostBreakdown, SizedOpportunity, etc.) |
| `arb-math` | Mathematical operations: CPMM, CLMM, DLMM, CPMM optimal sizing, chronos risk, economic calculations |
| `arb-engine` | Route discovery, graph algorithms (Bellman-Ford), guarded routing, split routing |
| `arb-dex` | Protocol-specific adapters: Raydium V4, CPMM, CLMM, Orca Whirlpool, Meteora DLMM, Pump.fun, PumpSwap |
| `arb-ingest` | Yellowstone gRPC ingestion, provider supervision, account decoding, discovery, registry |
| `arb-tx` | Transaction assembly: Kamino flash loans, Jupiter quotes, Meteora DLMM swaps, Raydium CPMM/CLMM/Pump.fun instruction builders |
| `arb-land` | Bundle landing tracking, tip oracle, bidder logic, ledger management |
| `arb-bot` | Supervisor orchestrator: health monitoring, risk gating, dispatch, execution state machine |
| `arb-risk` | Risk engine: circuit breaker, kill switch, loss limits, velocity checks |
| `arb-metrics` | Metrics exposition and telemetry |
| `arb-store` | Persistence layer: Redis cache, PostgreSQL store |
| `arb-sim` | LiteSVM simulation environment |
| `arb-mcp` | CoinMarketCap MCP integration |

Additional files: `webhook-service/`, `audit.toml`, `gm.md`, `gp.md`, `re.md`, `PROMPET.MD`

## 0.9 Known Issues From Audit

1. **USDC reserve address mismatch**: `verified_config.rs` hardcodes `D6q6wuQSrifJKUvxCoXepEAbW39zWCuYAZBpqUg6xK25` while `arb-constants` has `D6q6wuQSrifJKZYpR1M8R4YawnLDtDsMmWM1NbBmgJ59`. Need verification of which is correct.

2. **RaydiumV4Accounts pool_coin_vault typo**: `arb-types/src/lib.rs` has `DQyrAcCrDXYZujhgwgCRiaACVKAfdA3kh2v7iqWw5jz` while `arb-constants` has `DQyrAcCrDXQ7NeoqGgDCZwBvWDcYmFCjSb9JtteuvPpz`. Possible typo (DXYZ vs DXQ7).

3. **Legacy binaries contain stale hardcoded pool constants**: `live_scanner.rs`, `live_single_exec.rs`, `latency_bench.rs` still contain `RAYDIUM_SOL_USDC_POOL` and overlap with supervisor. Quarantined but still compile.

4. **Auxiliary binaries bypass main mode gate**: `live-scanner`, `validate`, `discover`, `replay` are not governed by `--mode live` / `ARB_LIVE` gate.

5. **Token-2022 transfer fee**: `adapters.rs` rejects any pool with `transfer_fee_bps != 0` or `has_risky_token` until verified per-mint TransferFeeConfig quote. Intentional fail-closed, but means Token-2022 tokens cannot be traded.

6. **PumpFun/PumpSwap quote-only**: Cannot reach transaction builder until verified. Intentional.

7. **CORS permissive in webhook-service**: `CorsLayer::permissive()` allows any origin — security concern for production.

8. **No HSM/secrets manager**: Keypair is file/env input; file permissions (0600) are the only control.

9. **Admin token shared**: Bearer token, no per-operator identity, no audit trail of who paused/resumed.

10. **Dependency advisories unverified**: `cargo audit` blocked (RustSec DB fetch failed). Vulnerability status unknown.

11. **No release artifact signature**: No cosign/SBOM produced.

12. **Credentials not rotated**: Repository cannot prove keys exposed outside git are revoked.

13. **Kamino offsets verified on 2026-11-04**: Must re-verify if program upgraded (offsets may change).

14. **Meteora safety factor 92%**: Hardcoded to avoid crossing suboptimal bins; may be overly conservative.

15. **Fee cap 10% (MAX_FEE_RATE)**: Hardcoded; may not reflect all DEX fee schedules.

16. **ComputeUnits MAX = 1,400,000**: Solana cap; hardcoded.

17. **Tip oracle DEFAULT_MAX_AGE_MS**: Value not visible in reviewed code.

18. **TIP_FLOOR_URL**: Value not visible in reviewed code.

19. **webhook-service config load**: Falls back to `config.toml` in CWD; panics if missing. No graceful degradation.

20. **`arb-types` PoolState.fee_bps**: Caps at 10,000 (100%) via `unwrap_or(10_000)` — silently caps invalid fees instead of rejecting.