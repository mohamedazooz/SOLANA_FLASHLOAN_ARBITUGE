# Forensic Language and Runtime Inventory

## 1. Executive Summary & Repository Statistics

This document provides a complete forensic inventory of all programming languages, runtimes, build tools, dependency manifests, generated artifacts, and execution boundaries in the `SOLANA_FLASHLOAN_ARBITUGE` repository.

Total repository files (excluding `.git`, `target/`, and `node_modules/`): **1,565 files** (including 10 uncommitted compiled Python bytecode files in `__pycache__/`).

### 1.1 Language & File Extension Breakdown

| Language / Format | Primary Extensions | File Count | Primary Functional Role |
| :--- | :--- | :--- | :--- |
| **Markdown / Docs** | `.md` | 943 | Architecture, audit records, specifications, runbooks |
| **Rust** | `.rs` | 353 | Core trading bot, DEX quoting, math, risk, V1 tx compiler, on-chain program |
| **JSON / JSONL** | `.json`, `.jsonl` | 73 | Config schemas, test fixtures, audit findings, report data |
| **TOML** | `.toml` | 38 | Cargo workspace, package manifests, bot runtime configurations |
| **Python** | `.py` | **31** | Mathematical oracle, offline diagnostics, RPC probes, forensic inspect scripts |
| **TypeScript / TSX** | `.ts`, `.tsx` | **18** | Web monitoring dashboard (React + Vite) |
| **Shell & Batch** | `.sh`, `.bat` | **17** | Linux build validation, systemd deployment, CI bootstrap, dashboard launcher |
| **Plain Text / Logs** | `.txt`, `.log` | 16 | Diagnostic captures, soak logs, license, public keys |
| **Python Bytecode** | `.pyc` | **10** | Uncommitted compiled bytecode files in `__pycache__/` (forensic risk) |
| **Systemd Units** | `.service`, `.timer`, `.logrotate` | 8 | Production system daemon configs and watchdog timers |
| **Binary Assets** | `.bin`, `.db`, `.db-wal`, `.db-shm` | 8 | Raw mainnet transaction captures, local SQLite runtime db |
| **CSV** | `.csv` | 6 | Historical pool benchmark snapshots |
| **YAML** | `.yml` | 5 | GitHub Actions CI/CD workflows, Docker Compose |
| **JavaScript** | `.js` | **5** | Differential test harness (`scripts/differential-harness/`), Tailwind/PostCSS config, Vite build asset |
| **SQL** | `.sql` | 4 | Database schema migrations (`crates/arb-store/migrations/`) |
| **Proptest State** | `.proptest-regressions` | 4 | Saved fuzzing regression states |
| **HTML** | `.html` | **3** | Monolithic fallback (`dashboard/index.html`), Vite root (`web/index.html`), built bundle (`web/dist/index.html`) |
| **CSS** | `.css` | **2** | Tailwind source (`web/src/styles.css`), built bundle (`web/dist/assets/index-BKx_7nHs.css`) |
| **Environment Templates** | `.example` | 2 | `.env.example`, `.env.production.example` |
| **Git & Tooling Meta** | `.gitignore`, `.dockerignore`, etc. | 8 | Version control and container ignore rules |
| **No Extension / Licenses** | `LICENSE`, etc. | 12 | Top-level metadata and container descriptors |
| **Total** | — | **1,565** | — |

---

## 2. Functional Ownership Classification

Every file in the repository belongs to one of six mutually exclusive architectural classifications:

### 2.1 Production Source Code (Authoritative Runtime)
Code that compiles into binaries executed in production environments (`arb-bot`, `arb-executor`, and Web UI assets served to operators).

* **Rust Workspace (14 crates in `crates/` + 1 program in `programs/`)**:
  * `crates/arb-types`: Fundamental domain types (`Slot`, `Lamports`, `PoolId`, `MintId`, `RouteId`, `ExecutionOpportunity`, `SwapHop`, `DexProtocol`).
  * `crates/arb-constants`: Protocol addresses, program IDs, mint IDs, Kamino market/reserve constants, Jito tip accounts.
  * `crates/arb-math`: Core quantitative algorithms: exact CPMM, CLMM sqrt-price and tick-crossing, DLMM single-bin and multi-bin traversal, DAMM v2, LaunchLab, Kamino U68F60 flash fee calculation, dimensional currency conversions (`base_raw_to_lamports_checked`), latency tracking, and Chronos exponential decay.
  * `crates/arb-dex`: Protocol binary state decoders and quote engines: Meteora (DLMM, DAMM v2, DBC), Raydium (AMM v4, CPMM, CLMM, LaunchLab), Orca Whirlpools, Pump.fun bonding curves, and PumpSwap.
  * `crates/arb-engine`: Route discovery, Graph construction, 2-pool DLMM cross-venue scanning, 3-leg cyclic routing, Bellman-Ford negative-cycle detection, multi-hop DFS, and optimal borrow sizing.
  * `crates/arb-ingest`: Market data ingestion engine, Multi-Provider WebSocket supervision (`HeliusWebSocketClient`, public RPC fallback), AccountSnapshotStore snapshot management, and provider failover circuit breakers.
  * `crates/arb-risk`: Latency gates, order velocity limits, exposure limits, dynamic drawdown limits, and hardware file-based KillSwitch.
  * `crates/arb-tx`: Solana V1 binary transaction compiler (`AdaptiveTxCompiler`, `try_compile_v1`, 0x81 prefix, 4,096-byte envelope, 64 inline accounts, loaded-data-limits), instruction builders (Kamino flash borrow/repay, Raydium, Orca, Meteora, ComputeBudget), and Ed25519 cryptographic signing (`KeypairSigner`).
  * `crates/arb-sim`: Pre-flight RPC simulation (`simulateTransaction` with `replaceRecentBlockhash`), local LiteSVM engine (optional acceleration).
  * `crates/arb-land`: Jito bundle multi-region dispatcher (`send_bundle_multi_region`), SWQoS direct submission, and async landing tracker (`get_bundle_statuses`, `evaluate_lifecycle_with_block_height`).
  * `crates/arb-store`: High-throughput persistent ledger for attempt records, dispatch intents, and post-trade settlement balance verification (SQLite / PostgreSQL).
  * `crates/arb-metrics`: Prometheus metrics HTTP exporter (`/metrics` on port 9100).
  * `crates/arb-config`: Multi-tier TOML configuration loader, environment variable resolution, and strict freshness validation.
  * `crates/arb-bot`: Unified production binary (`src/main.rs`), Supervisor loop (`run.rs`, `strategy.rs`, `opportunity.rs`, `dispatch.rs`, `recovery.rs`), ControlService (`control_service.rs`), Telegram bot & alerting (`telegram.rs`, `telegram_control.rs`), and authenticated Web API server (`web_api.rs`).
  * `programs/arb-executor`: On-chain pinocchio/no_std atomic arbitrage execution program (zero CU overhead, instruction introspection profit guard).
* **TypeScript / React (`web/src/`) (17 files)**:
  * Operator Web Dashboard client: `main.tsx`, `App.tsx`, `api.ts`, and 14 dashboard view components in `web/src/components/` (`EgyptianGuide.tsx`, `Executions.tsx`, `Login.tsx`, `Logs.tsx`, `Opportunities.tsx`, `Overview.tsx`, `Pools.tsx`, `Providers.tsx`, `Rejections.tsx`, `RiskControl.tsx`, `RouteMonitor.tsx`, `Sidebar.tsx`, `StrategyConfig.tsx`, `TelegramPanel.tsx`).
  * Runs purely client-side in the operator's browser; compiled to static files in `web/dist/` during Docker build and served by the Rust `arb-bot` Axum web server.
* **HTML / CSS (2 files)**:
  * `web/index.html`: Vite single-page application entry point.
  * `web/src/styles.css`: Tailwind and terminal CSS stylesheets.
* **Production Deployment Shell Scripts (4 files)**:
  * `deploy/production/deploy.sh`: Systemd production deployment orchestrator.
  * `deploy/production/rollback.sh`: Emergency systemd rollback script.
  * `scripts/arb_bot_failure_alert.sh`: Production watchdog failure alerting script.
  * `scripts/arb_bot_watchdog.sh`: Systemd process supervisor health watchdog.

### 2.2 Test-Only Source Code
Code executed solely during CI/test runs to prove correctness, parity, and security invariants.

* **Rust Integration Tests (`crates/*/tests/`)**:
  * 18 dedicated integration test targets in `crates/arb-math/tests/`, `crates/arb-dex/tests/`, `crates/arb-bot/tests/`, `crates/arb-tx/tests/`, `crates/arb-engine/tests/`, and `crates/arb-config/tests/`.
* **Rust Differential & Parity Tests**:
  * `crates/arb-math/tests/python_oracle_differential.rs`: Differential testing comparing Rust math against the Python mathematical oracle.
  * `crates/arb-math/tests/orca_sdk_parity.rs`: Orca Whirlpool SDK bit-exact parity tests.
  * `crates/arb-math/tests/raydium_clmm_swap_math_parity_tests.rs`: Raydium CLMM bit-exact parity tests against official test vectors.
* **Rust Fuzzing Suite (`fuzz/`)**:
  * `fuzz/fuzz_targets/cpmm_quote.rs`, `net_profit.rs`, `transfer_fee.rs`, `dlmm_traversal.rs` (Cargo-fuzz / libFuzzer targets).
* **Python Mathematical Oracle & Test Forensics (9 files)**:
  * `audit/math_oracle/oracle.py`: Canonical Python mathematical oracle generating test vectors for CPMM and DLMM swap math.
  * `artifacts/forensics/compare_tick_math.py`: Whirlpool tick math differential verification.
  * `artifacts/forensics/dump_orca_tickarray.py`: Orca tick array binary dump tool.
  * `artifacts/forensics/inspect_orca_accounts.py`: Orca account layout inspector.
  * `artifacts/forensics/probe_decimals.py`: Mint decimal offset verification tool.
  * `artifacts/forensics/probe_decimals2.py`: Mint decimal boundary test tool.
  * `artifacts/forensics/probe_raydium_v4_status.py`: Raydium AMM v4 status flag inspector.
  * `artifacts/forensics/validate_orca_tickarray_layout.py`: Tick array binary offset validator.
  * `artifacts/forensics/verify_kamino_reserve_offsets.py`: Kamino lending reserve layout byte offset validator.
* **JavaScript Differential Harness (2 files)**:
  * `scripts/differential-harness/run_mainnet_snapshot_differential.js`: Offline test script comparing Rust DLMM quotes against Meteora's official TypeScript SDK (`@meteora-ag/dlmm`).
  * `scripts/differential-harness/capture_mainnet_snapshot.js`: Mainnet pool state snapshot extractor for differential testing.

### 2.3 Development Tooling & Scripts
Standalone utilities, CI validation scripts, and diagnostic probes.

* **Python Development & Diagnostic Scripts (15 files)**:
  * `scripts/archive_docs.py`: Documentation archival tool.
  * `scripts/audit_codebase.py`: Codebase scanning and line count auditor.
  * `scripts/calculate_arbitrage_opportunities.py`: Offline route economics analysis tool.
  * `scripts/decode_jwt.py`: JWT inspection and token decoding utility.
  * `scripts/helius_opportunity_probe.py`: Live Helius WebSocket opportunity diagnostic probe.
  * `scripts/probe_rpc_wss_v1_sim.py`: Offline RPC/WSS endpoint probing tool.
  * `scripts/read_only_helius_jito_probe.py`: Read-only connectivity check for Helius and Jito endpoints.
  * `scripts/scan_live_cross_pool.py`: Live cross-pool opportunity scanner.
  * `scripts/solscan_tool.py`: Solscan API CLI query utility.
  * `scripts/test_keys.py`: API key validation diagnostic script.
  * `scripts/test_mcp.py`: MCP endpoint connection testing script.
  * `scripts/test_mcp_call.py`: MCP tool invocation test script.
  * `scripts/test_pro_endpoints.py`: Solscan Pro API test script.
  * `scripts/test_public_tools.py`: Solscan public API test script.
  * `.agents/skills/oxylabs-web-api/scripts/web_api.py`: Oxylabs search/scrape skill helper script.
* **Development Shell & Batch Scripts (4 files)**:
  * `scripts/verify-linux.sh`: Canonical Linux build/format/clippy/test validation script.
  * `scripts/deploy_vps.sh`: VPS bootstrapping and setup script.
  * `scripts/generate_secrets.sh`: Cryptographic secret generator for `.env`.
  * `scripts/update_server_env.sh`: Remote environment variable updater.
* **Build Tooling & Configuration (3 files)**:
  * `web/vite.config.ts`: Vite bundling configuration.
  * `web/postcss.config.js`: PostCSS configuration for Tailwind.
  * `web/tailwind.config.js`: Tailwind CSS theme configuration.

### 2.4 Generated Artifacts & Bundles
Artifacts produced by builds or tests, strictly non-authoritative.

* `web/dist/index.html`: Compiled production HTML shell.
* `web/dist/assets/index-CZBVW879.js`: Bundled and minified React JavaScript asset.
* `web/dist/assets/index-BKx_7nHs.css`: Bundled and minified Tailwind CSS asset.
* `audit/test_vectors/cpmm_vectors.json`: Test vectors emitted by `oracle.py`.
* `audit/test_vectors/dlmm_single_bin_vectors.json`: Test vectors emitted by `oracle.py`.
* `fixtures/meteora_mainnet_sdk_quotes.json`: Generated SDK comparison vectors.
* `reports/`: Audit summaries, soak test captures, and simulation results.

### 2.5 Documentation & Examples
* `docs/`: 900+ documentation files (architecture, security, active roadmap, handoff protocols).
* `AGENTS.md`, `GEMINI.md`, `CLAUDE.md`: Operational constitution and directives.
* `crates/*/examples/`: Developer examples (`wallet_probe.rs`, `v1_canary_broadcast.rs`, `full_path_rehearsal.rs`).
* Archived Python scripts (3 files):
  * `docs/archive/old_audits/audit_extract/audit_work/gen_bug_register.py`
  * `docs/archive/old_audits/audit_work/gen_bug_register.py`
  * `docs/archive/old_audits/external_audit/math_oracle/oracle.py`

### 2.6 Unused, Orphaned, or Redundant Code

1. **`dashboard/index.html` (2,949 lines)**:
   * Monolithic legacy Cyberpunk HTML/CSS/JS dashboard. Superseded by the React SPA in `web/`, but still retained as a fallback and compiled into `arb-bot` via `include_str!` in `crates/arb-bot/src/web_api.rs:514`.
2. **Unused npm Dependencies (`web/package.json`)**:
   * `@pump-fun/pump-sdk` (`^4.3.1`) and `@pump-fun/pump-swap-sdk` (`^2.2.0`). Declared in `dependencies`, but zero files in `web/src` import them. They inflate `node_modules` with heavy libraries (`@solana/web3.js`, `bn.js`, RPC websockets).
3. **Standalone `webhook-service` Crate (`webhook-service/`)**:
   * Isolated crate outside the Cargo workspace with its own `Cargo.lock` and `target/` directory. Uses outdated Axum 0.7 and thiserror 1.0 (conflicting with workspace Axum 0.8.9 and thiserror 2). Not included in Dockerfile or CI build pipeline.
4. **`_backup_pre_unification/` (7 files)**:
   * Gitignored backup directory containing outdated copies of Python calculation scripts and Windows batch files:
     * `calculate_arbitrage_opportunities.py`
     * `decode_jwt.py`
     * `scan_live_cross_pool.py`
     * `test_keys.py`
     * `askpass.bat`
     * `start_all.bat`
     * `start_dashboard.bat`
5. **Redundant Root Shell & Batch Scripts (6 files)**:
   * `run-shadow.sh`, `stop-shadow.sh`, `verification_script.sh`, `verify-shadow-package.sh`, `verify_fix.sh`, `start_dashboard.bat` contain redundant and overlapping commands superseded by `scripts/verify-linux.sh`.

---

## 3. Language Ownership Matrix

| Language | Authoritative Functional Role | Non-Authoritative / Auxiliary Role | Forbidden Contexts |
| :--- | :--- | :--- | :--- |
| **Rust** | **Core Trading System**: Market ingestion, state decoding, route graph, math calculations, risk validation, V1 tx compilation, Ed25519 signing, Jito landing, SQLite/Postgres persistence, Prometheus metrics, Admin REST/WS API, Telegram Bot, on-chain execution program. | Unit, integration, and fuzz testing. | None. Rust owns 100% of the live path. |
| **Python** | **Mathematical Oracle & Modeling**: Offline proof derivation, test vector generation (`oracle.py`), backtesting, and diagnostic RPC probing. | Ad-hoc forensic scripts (`artifacts/forensics/`). | **STRICTLY FORBIDDEN** from participating in live market scanning, quoting, decision making, or transaction execution. |
| **TypeScript** | **Operator User Interface**: React + Vite frontend dashboard (`web/`). Compiled to static assets at build time. | None. | **FORBIDDEN** on execution path. Moving backend API or Telegram bot to Node/TS is rejected. |
| **JavaScript** | **Differential Test Harness**: Running official protocol SDKs (`@meteora-ag/dlmm`) to capture snapshot test vectors. | Build tool configuration (`tailwind.config.js`, `postcss.config.js`), generated browser bundle (`web/dist/assets/*.js`). | **FORBIDDEN** on runtime execution path. |
| **HTML / CSS** | **Frontend Presentation**: Dashboard UI layouts and styles (`web/src/styles.css`, `web/index.html`). | Legacy fallback UI (`dashboard/index.html`), built assets (`web/dist/`). | No dynamic server-side rendering logic. |
| **Shell / Batch** | **Operations & Automation**: CI check scripts (`scripts/verify-linux.sh`), systemd service setup, deployment, secret generation, and local Windows startup (`start_dashboard.bat`). | None. | No trading logic or runtime transaction handling. |
| **SQL** | **Persistence Schemas**: Database table creation and index migrations for SQLite/Postgres in `crates/arb-store/migrations/`. | None. | No business logic in stored procedures or triggers. |
