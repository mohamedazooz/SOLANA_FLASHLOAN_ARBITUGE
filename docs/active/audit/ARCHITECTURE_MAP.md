# Architecture Map — Verified Code Topology (`b9b516c`)

**Baseline**: `origin/main` @ `b9b516c`

---

## 1. Acyclic Crate Dependency DAG (Verified via `cargo tree --depth 1`)

```mermaid
graph TD
    bot["arb-bot (daemon, supervisor, web_api, telegram, control_service)"]
    engine["arb-engine (MarketGraph, route sizing, cycle discovery)"]
    sim["arb-sim (RPC simulation validator, optional LiteSVM)"]
    ingest["arb-ingest (Helius WS, snapshot hydration, slot integrity)"]
    tx["arb-tx (swap builders, Kamino flash assemblers, V0/V1 compiler)"]
    land["arb-land (Jito Block Engine, SWQoS/RPC, tip/CU bidding)"]
    dex["arb-dex (decoders, Token-2022 checks, quote engines)"]
    risk["arb-risk (RiskGate, KillSwitch, BPS floors)"]
    store["arb-store (SQLite WAL, Postgres, Redis)"]
    config["arb-config (TOML profiles, freshness validation)"]
    metrics["arb-metrics (Prometheus exporter)"]
    math["arb-math (canonical economics, CPMM/CLMM/DLMM/DAMM v2 math)"]
    types["arb-types (domain types, PoolState, SwapHop, ExecutionOpportunity)"]
    constants["arb-constants (program IDs, discriminators, mints, tips)"]
    idl["arb-idl-gen (standalone IDL generator)"]
    exec["programs/arb-executor (INACTIVE undeployed native program)"]

    bot --> config & constants & dex & engine & ingest & land & math & metrics & risk & sim & store & tx & types
    engine --> constants & dex & ingest & math & tx & types
    sim --> constants & engine & ingest & tx & types
    ingest --> constants & dex & math & types
    tx --> constants & math & types
    land --> constants & math & types
    dex --> constants & math & types
    risk --> constants & types
    config --> constants & types
    store --> types
    math --> constants & types
    types --> constants
```

---

## 2. Production Execution Path (Verified End-to-End)

1. **Market Data Ingestion (`arb-ingest`)**:
   - `HeliusWsClient` + `DualIngestShadow` stream account/slot updates, hydrated via `hydration.rs` and `pump_state.rs` into coherent `PoolState` snapshots with slot and owner verification.
2. **Route Discovery & Sizing (`arb-engine`)**:
   - `MarketGraph::find_cycles_with_stats` (`route_graph.rs`) and DLMM cross-pool/3-leg strategies (`adapters.rs`, `cross_pool_dlmm.rs`, `dlmm_3leg_flash_arb` support) build directed multi-hop cycles (`MAX_ROUTE_CANDIDATES_PER_TICK = 4096`) priced via `arb-dex` / `arb-math` integer math.
3. **Canonical Economics & Risk Gating (`arb-math`, `arb-risk`)**:
   - `arb-math::economic` is the single source of truth for Kamino flash loan fees (`fee_bits = max(amount * sf, 2^60)`, ties-away rounding) and net profit calculation after base fee, priority fee, and Jito tip.
   - `RiskGate` + `KillSwitch` enforce durable emergency-stop and minimum lamport / BPS profit hurdles.
4. **Transaction Assembly & Compilation (`arb-tx`)**:
   - `Supervisor` (`crates/arb-bot/src/supervisor/execution.rs`) routes opportunities ONLY to:
     - `cross_dex_flash_arb::assemble_cross_dex_opportunity`
     - `TxBuilder::assemble_flash_arbitrage`
     - Non-flash `TxBuilder::assemble_arbitrage`
   - `AdaptiveTxCompiler` compiles **SIMD-0385 Transaction V1** (`0x81`, inline accounts <= 64, wire <= 4,096 B, strips `ComputeBudget` instructions and rewrites Kamino `borrow_instruction_index`) or V0 ALT transactions without silent downgrade.
5. **Preflight Simulation & Dispatch Reconciliation (`arb-sim`, `arb-land`, `arb-store`, `arb-bot`)**:
   - `BotMode::Simulate` (default fail-safe) submits base64 wire to `simulateTransaction` with `maxSupportedTransactionVersion: 1` where V1 applies.
   - `DispatchIntentRecord` persists the extracted signature **before** send in `dispatch.rs`; `recovery.rs` reconciles ambiguous timeouts by on-chain signature status and `lastValidBlockHeight` progression.

---

## 3. Standalone & Inactive Components

- **Standalone**: `webhook-service/` (isolated `Cargo.toml`, listens for webhooks, HMAC validation) and `web/` (Vite React dashboard served/authenticated via `arb-bot/src/web_api.rs` and `control_service.rs`).
- **Inactive**: `programs/arb-executor` (not deployed on mainnet; placeholder `ArbExec1...` key; native CPI alternative not wired into `Supervisor`), `compile_3leg_meme_dlmm_arb`, `build_flash_funded_three_pool_transaction`.
