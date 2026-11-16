# Repository Inventory Report

## 1.1 Cargo Workspace Configuration

- **Workspace resolver**: `resolver = "2"`
- **Workspace members**: `["crates/*"]` — all crates under `crates/` directory
- **Workspace exclude**: `["webhook-service"]` — webhook service is separate
- **Edition**: `2024`
- **Rust version**: `1.99` (require 1.88+ for edition 2024 let-chains)
- **License**: `MIT OR Apache-2.0`
- **Publish**: `false` (internal workspace)
- **Default features allowed**: `unsafe_code = "allow"`, `missing_debug_implementations = "allow"`, etc.

### Workspace Dependencies
Key workspace-level dependencies:
- `axum = "0.8.9"` — HTTP framework
- `tokio = { version = "1", default-features = false, features = ["rt-multi-thread", "macros", "sync", "time", "signal", "io-util"] }` — async runtime
- `tokio-util = "0.7"` — tokio utilities
- `futures = "0.3"` — async fundamentals
- `proptest = "1"` — property-based testing
- `criterion = "0.5"` — benchmarking
- `ruint = "1.12"` — big integer arithmetic
- `solana-pubkey = "2.2"` — Solana pubkey math

### Profile Configuration
- **Release**: `opt-level = 3`, `lto = "thin"`, `codegen-units = 16`, `debug = false`
- **Bench**: inherits release profile

## 1.2 Crate Inventory

The workspace contains 23 crates. Each crate's purpose and structure:

### `arb-constants` — Centralized Constants
- **Purpose**: Single source of truth for all on-chain addresses, program IDs, mint addresses, and protocol constants
- **Key contents**: Token programs, mint addresses (WSOL, USDC, SOL), Jito tip accounts, DEX program IDs, Meteora DLMM constants (BINS_PER_BIN_ARRAY=70, BIN_STEP=4), Kamino program ID, flash loan discriminators, compute budget limits
- **Source**: `src/lib.rs` — 354 lines of constants and helper functions
- **Cargo.toml**: Basic crate configuration

### `arb-types` — Core Domain Types
- **Purpose**: Newtypes and canonical primitives for Solana arbitrage
- **Key types**:
  - `Lamports` — SOL unit (lamports), with checked arithmetic
  - `TokenAmount` — Token amount in smallest atomic units
  - `Bps` — Basis points (1 bp = 0.01%, 10000 = 100%)
  - `Slot` — Solana slot number
  - `ComputeUnits` — Compute unit tracking (MAX = 1,400,000)
  - `PoolKind` — Cpmm, Clmm, Dlmm
  - `PoolSnapshot` — Canonical pool state with freshness
  - `PlannedHop` — Single leg of a planned route
  - `CostBreakdown` — Itemised cost of executing one opportunity
  - `SizedOpportunity` — Sized, priced, costed arbitrage opportunity
  - `ExecutionOpportunity` — Detected and sized arbitrage opportunity
  - `DlmmFeeParams` — Meteora DLMM fee parameters
  - `DlmmState` — Discrete-bin state
  - `DlmmBin` — Single bin in DLMM ladder
  - `PumpFunState` — Pump.fun bonding curve state
  - `PoolState` — Canonical pool state across all DEX protocols
  - `RaydiumV4Accounts` — 18-account matrix for Raydium v4
  - `SwapHop` — Swap step in arbitrage route
  - `SwapQuote` — Quotation result for a swap
  - `QuoteAmount` — Unified quoted amount
  - `ArbRoute` — Arbitrage route definition
  - `RefuseReason`, `TipDecision`, `TipPlan` — Jito tip market
  - `TipFloor`, `Posterior` — Tip floor and beta posterior
  - `InflightStatus` — Jito bundle status
  - `ArbError` — Unified error types

- **Source**: `src/lib.rs` — 1264+ lines
- **Cargo.toml**: Contains proptest, ruint, serde dependencies

### `arb-math` — Mathematical Operations
- **Purpose**: Mathematical operations for arbitrage pricing and risk
- **Submodules**:
  - `cpmm` — Constant product market maker math
  - `clmm` — Concentrated liquidity math (Orca Whirlpool, Raydium CLMM)
  - `dlmm` — Meteora DLMM discrete bin ladder
  - `cpmm` — CPMM math with U384/U512 optimal sizing
  - `economic` — Economic calculations: cost ledger, flash loan fees, net profit
  - `chronos` — Risk gate and timeline metrics
  - `latency` — Fire timing and slot clock

- **Key exports**: `build_economic_quote`, `calculate_net_profit_raw`, `calculate_flash_fee_raw`, `optimal_two_pool`, `optimal_three_pool`, `compute_swap`, `quote_dlmm`, `advance_active_id`, `pow_q64`, etc.

- **Source**: `src/lib.rs`, `src/cpmm.rs`, `src/clmm.rs`, `src/dlmm.rs`, `src/cppmm.rs`, `src/economic.rs`, `src/chronos.rs`, `src/latency.rs`

### `arb-engine` — Route Discovery and Graph Algorithms
- **Purpose**: Route discovery, graph algorithms, guarded routing, split routing
- **Key components**:
  - `bellman_ford.rs` — Shortest path / arbitrage detection
  - `adapters.rs` — Protocol execution capability checks
  - `guarded.rs` — State gating and revalidation
  - `route_worker.rs` — Route worker loop and split routing

- **Key functions**: `estimate_minimal_cu`, `calculate_dynamic_cu_limit`, `calculate_dynamic_cu_price`, `per_hop_cu`, `estimate_route_cu`, `cap_cu_price_by_profit`

- **Source**: `src/lib.rs`, `src/bellman_ford.rs`, `src/adapters.rs`, `src/guarded.rs`, `src/route_worker.rs`

### `arb-dex` — Protocol-Specific Adapters
- **Purpose**: Protocol-specific swap adapters and trait definitions
- **Key files**:
  - `traits.rs` — Shared trait definitions for DEX adapters
  - `raydium_v4.rs` — Raydium AMM v4 adapter
  - `orca_whirlpool.rs` — Orca Whirlpool adapter
  - `pump_fun.rs` — Pump.fun bonding curve adapter
  - `pump_swap.rs` — PumpSwap adapter
  - `lib.rs` — Module declarations

### `arb-ingest` — Yellowstone gRPC Ingestion
- **Purpose**: Yellowstone gRPC ingestion, provider supervision, account decoding
- **Key files**:
  - `lib.rs` — Main ingest module
  - `yellowstone.rs` — Yellowstone gRPC client
  - `registry.rs` — Pool registry and discovery
  - `quote_readiness.rs` — 12 fail-closed quote readiness checks
  - `slot_integrity.rs` — Slot integrity tracking
  - `provider_supervisor.rs` — Provider health monitoring and failover
  - `rpc_reconcile.rs` — RPC reconciliation
  - `rpc_pool.rs` — RPC connection pool
  - `oxylabs.rs` — Oxylabs web API integration
  - `bitquery.rs` — Bitquery data integration
  - `gmgn.rs` — Gmgn data integration
  - `pump_state.rs` — Pump.fun state tracking
  - `pump_state_tests.rs` — Pump.fun state tests
  - `pumpfun_discovery.rs` — Pump.fun discovery
  - `pumpdev_stream.rs` — Pump.dev stream
  - `solami_stream.rs` — Solami stream
  - `dexscreener.rs` — DexScreener integration
  - `bitquery_ws_live.rs` — Bitquery WebSocket live
  - `oxylabs_mock.rs` — Oxylabs mock for testing

### `arb-tx` — Transaction Assembly
- **Purpose**: Transaction building, instruction packing, atomic bundle builder
- **Key modules**:
  - `kamino.rs` — Kamino flash loan builder (12-account matrix)
  - `jupiter.rs` — Jupiter quote client
  - `template.rs` — Pre-warmed transaction template
  - `alt.rs` — Alt manager
  - `error.rs` — Transaction error types
  - `lib.rs` — Main transaction module

- **Key functions**:
  - `build_kamino_flash_borrow`, `build_kamino_flash_repay`
  - `build_meteora_dlmm_swap_instruction`
  - `build_raydium_v4_swap_instruction_18`
  - `build_raydium_cpmm_swap_instruction`
  - `build_raydium_clmm_swap_instruction`
  - `build_orca_whirlpool_swap_instruction_11`
  - `build_pump_fun_buy_sell_instruction`
  - `estimate_minimal_cu`, `estimate_route_cu`, `per_hop_cu`
  - `calculate_dynamic_cu_limit`, `calculate_dynamic_cu_price`
  - `cap_cu_price_by_profit`
  - `get_next_jito_tip_account`, `build_jito_tip_instruction`
  - `attach_jitodontfront`, `build_marked_jito_tip_instruction`
  - `derive_meteora_reserve_pda`, `derive_meteora_bin_array_pda`
  - `derive_meteora_event_authority`, `derive_meteora_oracle_pda`
  - `bin_array_index_for_bin`

### `arb-land` — Bundle Landing and Tip Oracle
- **Purpose**: Bundle landing tracking, tip oracle, bidder logic
- **Key files**:
  - `lib.rs` — Main land module
  - `tip_oracle.rs` — Tip floor pricing and bid decisions
  - `ledger.rs` — Landing ledger with persistence
  - `leader.rs` — Leader tracking and auction awareness

- **Key types**: `TipFloor`, `Percentile`, `Posterior`, `TipDecision`, `RefuseReason`, `InflightStatus`
- **Key functions**: `pick_tip`, `calculate_net_profit`, `TipFloor::at`, `TipFloor::age_ms`, `TipFloor::is_fresh`

### `arb-bot` — Supervisor Orchestrator
- **Purpose**: Health monitoring, risk gating, dispatch, execution state machine
- **Key files**:
  - `main.rs` — Entry point
  - `supervisor.rs` — Top-level supervisor
  - `verified_config.rs` — Fail-closed verified inputs
  - `health_monitor.rs` — Health monitoring
  - `errors.rs` — Error definitions
  - `supervisor/dispatch.rs` — Execution guards
  - `supervisor/execution.rs` — Assembly guards
  - `supervisor/run.rs` — Runner loop

### `arb-risk` — Risk Engine
- **Purpose**: Circuit breaker, kill switch, loss limits, velocity checks
- **Key file**: `src/lib.rs` — Risk limits and circuit breaker state

### `arb-metrics` — Metrics Exposition
- **Purpose**: Metrics exposition for Prometheus-compatible monitoring

### `arb-store` — Persistence Layer
- **Purpose**: Redis cache and PostgreSQL store
- **Key files**: `lib.rs`, `redis_cache.rs`, `postgres_store.rs`

### `arb-sim` — Simulation Environment
- **Purpose**: LiteSVM deterministic simulation
- **Key file**: `src/litesvm_sim.rs`

### `arb-mcp` — CoinMarketCap MCP
- **Purpose**: CoinMarketCap Market Data Protocol integration

## 1.3 Build Artifacts

- **Target directory**: `target/` (root workspace)
- **Build profile**: Release-opt by default
- **Incremental**: Enabled (per-crate incremental compilation)
- **Dependencies**: `Cargo.lock` at root locks all workspace versions

## 1.4 Test Infrastructure

- **Unit tests**: Distributed across crates, cover math, protocol adapters
- **Integration tests**: Located in crate `tests/` directories
- **Benchmarks**: `criterion`-based, defined in `arb-math/benches/`
- **Property-based tests**: `proptest` used in `arb-types` and `arb-math`
- **Test targets**: workspace `--all-targets` includes both `lib` and `test` targets

## 1.5 Configuration Files

- `config.toml` — Main bot configuration (mode, grpc, rpc, jupiter, wallet, strategy, risk, store, metrics, admin, chronos)
- `.env` — Environment credentials (API keys, wallet paths)
- `.env.example` — Example environment variables (modified)
- `config/` — New configuration directory with base.toml, development.toml, etc.
- `audit.toml` — Safety audit configuration
- `deny.toml` — Dependency review configuration
- `rust-toolchain.toml` — Rust 1.99 specification

## 1.6 Scripts and Binaries

- `crates/arb-bot/src/bin/` — 6 binaries: validate, replay, live_scanner, latency_bench, discover, alchemy_webhook
- `scripts/` — New scripts directory (empty at inventory time)
- `docker-compose.yml` — Docker composition
- `Dockerfile` — Container definition

## 1.7 Documentation

Extensive documentation across `docs/`:
- Phase status and baseline docs
- Audit findings and decisions
- Protocol compatibility guides
- Execution lifecycle documentation
- Operations runbook
- Security model
- Release readiness
- Economics and sizing docs

Plus newly created Phase 0 reports in `reports/production/`.

## 1.8 Key Cross-Cutting Concerns

1. **Fail-closed design**: Every safety gate defaults to rejecting; never assumes success
2. **Single source of truth**: `arb-constants` for all on-chain addresses and program IDs
3. **Exact integer arithmetic**: No floating point in raw token accounting (`ruint`, `ruint` aliases)
4. **Token-2022 safety**: Reject pools with risky token extensions until verified per-mint
5. **Provider redundancy**: Primary/fallback RPC with Yellowstone gRPC ingestion
6. **Jito integration**: Tip floor bidding with `jitodontfront` sandwich protection
7. **Kamino flash loans**: 12-account matrix with on-chain fee reading (NOT hardcoded)
8. **Meteora DLMM**: Versioned bin array traversal with `METEORA_BINS_PER_BIN_ARRAY=70`
9. **Compute unit budgeting**: Protocol-specific per-hop CU estimates with 15% safety buffer
10. **Structured tracing**: JSON-based structured logging throughout