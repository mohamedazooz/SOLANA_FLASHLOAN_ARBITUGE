# End-to-End Execution Path & Language Boundary Map

## 1. Architectural Mandate & Critical Path Guarantee

**Hard Invariant**: The live, latency-critical arbitrage execution path is **100% Rust** (`edition = "2024"`, toolchain pinned to `1.99.1`).

* **Zero Python in Live Execution**: Python does not run in production. No Python interpreter, virtual environment, or script is invoked during market data ingestion, route discovery, sizing, profitability calculation, risk gating, transaction construction, signing, or submission.
* **Zero TypeScript/JavaScript in Live Execution**: TypeScript exists exclusively as client-side source code for the Web Dashboard SPA (`web/`), which is compiled at build time into static JavaScript/CSS bundles and served by Rust's built-in Axum web server. Node.js is not present in the runtime container.
* **Zero IPC on the Hot Path**: All trading subsystems (ingestion, graph pathfinding, quantitative math, risk gates, V1 instruction compilation, signing, simulation, and landing) execute within a **single native compiled process** (`arb-bot`). Inter-module communication occurs via zero-copy in-memory references (`&[u8]`), atomic integers, and lock-free / bounded channels (`tokio::sync`).

---

## 2. Stage-by-Stage Forensic Trace of the Execution Path

The production execution pipeline proceeds through 12 deterministic stages:

```
[1. Market Ingestion] 
       │ (WebSocket JSON-RPC -> Zero-Copy AccountSnapshotStore)
       ▼
[2. Pool State Validation] 
       │ (Binary Layout Decoding: bytemuck / raw offsets)
       ▼
[3. Route Discovery] 
       │ (MarketGraph, Bellman-Ford, CrossPoolDlmm, Meteora 3-Leg)
       ▼
[4. Borrow Sizing] 
       │ (Closed-Form Analytical Optima + Live Vault Depth Clamp)
       ▼
[5. Profitability Calculation] 
       │ (Exact Fixed-Point U68F60 / U256, Integer Fee Deductions)
       ▼
[6. Risk Validation] 
       │ (Economic Invariants, Account Dependencies, KillSwitch)
       ▼
[7. Transaction Construction] 
       │ (AdaptiveTxCompiler V1: 0x81 prefix, 64 accounts, ALTs-free)
       ▼
[8. Simulation] 
       │ (Async RPC simulateTransaction / LiteSVM local check)
       ▼
[9. Ed25519 Signing] 
       │ (In-memory KeypairSigner, 64-byte Ed25519 signature)
       ▼
[10. Submission] 
       │ (Jito Multi-Region Bundle Engine / SWQoS RPC)
       ▼
[11. Confirmation & Landing] 
       │ (Jito BundleStatus + RPC SignatureStatus Polling)
       ▼
[12. Settlement Reconciliation] 
       │ (Actual Balance Delta Extraction -> SQLite / Postgres)
```

---

### Detailed Stage Specifications

#### Stage 1: Market Data Ingestion
* **Source File & Function**:
  * `crates/arb-ingest/src/helius_ws.rs`: `HeliusWebSocketClient::start_stream` (line 576)
  * `crates/arb-ingest/src/lib.rs`: `AccountSnapshotStore::apply` (line 156), `AccountSnapshotStore::apply_batch` (line 203)
  * `crates/arb-bot/src/supervisor/run.rs`: `Supervisor::run` (lines 670–682, 1056–1060)
* **Programming Language**: Rust (100%)
* **Runtime / Process**: `arb-bot` native binary, Tokio multi-threaded async runtime worker task
* **Data Serialization Format**: JSON-RPC over WebSocket frames (`accountNotification`, `slotNotification`) decoded into native `AccountSnapshot` holding `Arc<Vec<u8>>`
* **Communication Mechanism**: TLS WebSocket connection over TCP (`tokio-tungstenite` / `reqwest`), mpsc channel to supervisor
* **Latency Contribution**: ~500 µs – 2 ms network ingress; < 25 µs local memory dispatch
* **Failure Handling**: Automatic reconnection with exponential backoff; stream generation counter incremented on reconnect to invalidate all in-flight quoting ticks; failover circuit breaker (`ProviderSupervisor`) switches to HTTP RPC fallback if WebSocket stalls > 300 slots.

#### Stage 2: Pool State Validation & Decoding
* **Source File & Function**:
  * `crates/arb-dex/src/decoders/meteora_dlmm.rs`: `MeteoraDlmmPool::decode` (line 32)
  * `crates/arb-dex/src/decoders/raydium_clmm.rs`: `RaydiumClmmPool::decode` (line 31)
  * `crates/arb-dex/src/decoders/raydium_cpmm.rs`: `RaydiumCpmmPool::decode` (line 33)
  * `crates/arb-dex/src/pump_swap.rs`: `PumpSwapPool::decode` (line 46)
  * `crates/arb-dex/src/orca_whirlpool.rs`: `OrcaWhirlpool` structure and `orca_tick_array_start_indexes` (lines 42–100)
  * `crates/arb-ingest/src/decoders.rs`: `decode_pool_info` (line 134), `resolve_fee_bps` (line 488)
* **Programming Language**: Rust (100%)
* **Runtime / Process**: `arb-bot` native binary
* **Data Serialization Format**: Direct binary deserialization from raw byte slices (`&[u8]`) using `bytemuck` zero-copy structures or exact manual byte-offset slicing
* **Communication Mechanism**: In-process synchronous function call
* **Latency Contribution**: ~5 – 20 µs per pool
* **Failure Handling**: Returns `Err(DexError::InvalidAccountData)`; malformed pool accounts are immediately discarded and never promoted to `AccountSnapshotStore`.

#### Stage 3: Route Discovery & Candidate Pathfinding
* **Source File & Function**:
  * `crates/arb-engine/src/cross_pool_dlmm.rs`: `MarketGraph::find_cross_pool_dlmm_cycles` (line 532)
  * `crates/arb-engine/src/bellman_ford.rs`: `BellmanFordEngine::find_negative_cycles` (line 230)
  * `crates/arb-engine/src/lib.rs`: `MarketGraph` struct (line 338), `MarketGraph::find_multi_hop_cycles` (line 1056)
  * `crates/arb-engine/src/meteora_pair_discovery.rs`: `MeteoraPairDiscovery::find_3leg_cycles` (line 440)
  * `crates/arb-bot/src/supervisor/strategy.rs`: `Supervisor::strategy_cycle` (lines 280–410)
* **Programming Language**: Rust (100%)
* **Runtime / Process**: `arb-bot` native binary
* **Data Serialization Format**: In-memory typed Rust structures (`Vec<ExecutionOpportunity>`, `SwapHop`)
* **Communication Mechanism**: In-process shared memory (`parking_lot::Mutex<MarketGraph>`)
* **Latency Contribution**: ~50 – 300 µs (parallelized across Rayon / Tokio cores)
* **Failure Handling**: If no valid cycle exists or state age > `max_state_age_slots`, returns empty opportunity list; tick terminates cleanly without dispatch.

#### Stage 4: Borrow Sizing Optimization
* **Source File & Function**:
  * `crates/arb-bot/src/supervisor/run.rs`: `Supervisor::kamino_available_flash_liquidity` (lines 61–72)
  * `crates/arb-bot/src/supervisor/strategy.rs`: `Supervisor::strategy_cycle` (lines 189–220)
  * `crates/arb-engine/src/flash_loan_sizing.rs`: `calculate_optimal_flash_loan` (line 167)
  * `crates/arb-engine/src/cross_pool_dlmm.rs`: `optimize_size` (line 305)
  * `crates/arb-engine/src/meteora_size_optimizer.rs`: `optimize_direction` (line 216), `find_best_opportunity` (line 413)
* **Programming Language**: Rust (100%)
* **Runtime / Process**: `arb-bot` native binary
* **Data Serialization Format**: Pure integer primitives (`u64`, `u128`)
* **Communication Mechanism**: In-process function execution
* **Latency Contribution**: ~10 – 50 µs
* **Failure Handling**: **Strict Fail-Closed Invariant**: Sizing is bounded by actual decoded WSOL token-balance of Kamino's supply vault (`KAMINO_WSOL_SUPPLY_VAULT`). If vault snapshot is stale (> 5000 slots) or missing, `kamino_available_flash_liquidity` returns `None`, setting `max_sizing = 0` and dropping the candidate immediately.

#### Stage 5: Profitability Calculation & Economic Accounting
* **Source File & Function**:
  * `crates/arb-math/src/economic.rs`: `calculate_net_profit_raw` (line 512), `calculate_flash_fee_raw` (line 655), `derive_rational_rate` (line 185)
  * `crates/arb-math/src/dlmm/quote_exact_in.rs`: `quote_dlmm_multi_bin` (line 14)
  * `crates/arb-math/src/dlmm/price_math.rs`: `get_price_from_id` (line 55)
  * `crates/arb-math/src/cpmm.rs`: `cpmm_out_upfront_fee_ceil` (line 41)
  * `crates/arb-math/src/clmm.rs`: `compute_swap_step` (line 779), `compute_swap_step_checked` (line 649)
* **Programming Language**: Rust (100%)
* **Runtime / Process**: `arb-bot` native binary
* **Data Serialization Format**: Pure integer fixed-point arithmetic (`u64`, `u128`, `U256`, and U68F60 for Kamino `flash_loan_fee_sf`). Zero floating-point (`f32`/`f64`) math in economic decisions.
* **Communication Mechanism**: In-process inline math
* **Latency Contribution**: ~5 – 15 µs
* **Failure Handling**: Checked integer arithmetic (`checked_add`, `checked_sub`, `checked_mul`, `checked_div`); overflow causes immediate return of `None` / `EconomicError`. Rejection logged if `net_profit < min_net_profit_abs`.

#### Stage 6: Risk Validation & Invariant Gating
* **Source File & Function**:
  * `crates/arb-bot/src/supervisor/run.rs`: `Supervisor::validate_economic_invariants` (lines 1166–1250)
  * `crates/arb-bot/src/supervisor/opportunity.rs`: `Supervisor::validate_route_account_dependencies` (lines 1633–1670)
  * `crates/arb-risk/src/lib.rs`: `RiskGate::check_trade_size` (line 525), `evaluate_chronos_ev` (line 352), `evaluate_chronos_gate` (line 455), `KillSwitch::is_blocked` (line 193)
* **Programming Language**: Rust (100%)
* **Runtime / Process**: `arb-bot` native binary
* **Data Serialization Format**: In-memory structural validation
* **Communication Mechanism**: In-process method calls
* **Latency Contribution**: ~2 – 10 µs
* **Failure Handling**: Aborts execution on:
  1. Input mint ≠ Output mint (non-closed cycle).
  2. Route state age > `max_state_age_slots`.
  3. Net profit < floor or ROI < threshold.
  4. Repayment funding deficiency (`expected_min_final_output < size_in + flash_fee`).
  5. Missing account dependencies or program ownership mismatch.
  6. Hardware / file KillSwitch active (`KILL_SWITCH`).

#### Stage 7: Transaction Construction & V1 Compilation
* **Source File & Function**:
  * `crates/arb-tx/src/transport/adaptive.rs`: `AdaptiveTxCompiler::compile_v1_message` (line 415)
  * `crates/arb-tx/src/flash/compiler.rs`: `try_compile_v1` (line 1008), `compile_3leg_meme_dlmm_arb` (line 363)
  * `crates/arb-tx/src/flash/kamino.rs`: `build_kamino_flash_borrow` (line 85), `build_kamino_flash_repay` (line 128)
  * `crates/arb-tx/src/primitives/instruction.rs`: `build_compute_unit_limit` (line 41), `build_compute_unit_price` (line 53)
  * `crates/arb-tx/src/primitives/atomic_integrity.rs`: `verify_atomic_integrity` (line 122)
* **Programming Language**: Rust (100%)
* **Runtime / Process**: `arb-bot` native binary
* **Data Serialization Format**: Solana Versioned Transaction V1 binary wire protocol:
  * 0x81 message discriminator byte
  * Compact-u16 account address table (64 inline accounts, zero ALTs required)
  * Explicit `ComputeBudgetInstruction` (CU limit + priority fee lamports)
  * Instructions sequence: `FlashBorrow` -> `Swap_1` -> ... -> `Swap_N` -> `ProfitAssertion` -> `FlashRepay`
* **Communication Mechanism**: In-process memory buffer compilation (`Vec<u8>`)
* **Latency Contribution**: ~15 – 45 µs
* **Failure Handling**: Rejects and aborts if serialized size > 4,096 bytes or inline accounts > 64.

#### Stage 8: Simulation (Pre-flight Verification)
* **Source File & Function**:
  * `crates/arb-sim/src/lib.rs`: `Simulator::simulate_async` (line 180), `Simulator::simulate` (line 190)
  * `crates/arb-bot/src/supervisor/dispatch.rs`: lines 488–618
* **Programming Language**: Rust (100%)
* **Runtime / Process**: `arb-bot` native binary
* **Data Serialization Format**: Base64 encoded wire transaction inside JSON-RPC `/simulateTransaction` body (`replaceRecentBlockhash: true`, `sigVerify: false`)
* **Communication Mechanism**: Async HTTP POST over TLS via `reqwest` (connection pooled)
* **Latency Contribution**: ~15 – 45 ms (network round-trip time)
* **Failure Handling**: Fail-closed: If simulation returns `err` or consumed CU > limit, the transaction is immediately rejected and recorded in rejections ledger. Never submitted to the network.

#### Stage 9: Cryptographic Ed25519 Signing
* **Source File & Function**:
  * `crates/arb-tx/src/primitives/signer.rs`: `KeypairSigner::sign` (line 106)
  * `crates/arb-bot/src/supervisor/opportunity.rs`: lines 1245–1255
* **Programming Language**: Rust (100%)
* **Runtime / Process**: `arb-bot` native binary
* **Data Serialization Format**: Raw 64-byte Ed25519 signature written directly into the wire transaction buffer at byte offset `1..65`
* **Communication Mechanism**: In-process CPU execution (curve25519-dalek SIMD optimized)
* **Latency Contribution**: ~18 – 30 µs
* **Failure Handling**: If keypair is missing (e.g. shadow/simulate mode), dummy signing is used for sizing checks; live mode panics at startup if keypair cannot be decoded.

#### Stage 10: Submission (Jito Bundle & SWQoS RPC)
* **Source File & Function**:
  * `crates/arb-land/src/lib.rs`: `send_bundle_multi_region` (line 525, line 719), `send_bundle_multi_region_authed` (line 726)
  * `crates/arb-bot/src/supervisor/dispatch.rs`: lines 765–795
* **Programming Language**: Rust (100%)
* **Runtime / Process**: `arb-bot` native binary
* **Data Serialization Format**: JSON-RPC payload containing base58/base64 bundle transaction arrays sent concurrently to multiple Jito regional endpoints (Amsterdam, Frankfurt, New York, Tokyo, Salt Lake City)
* **Communication Mechanism**: HTTP/2 Keep-Alive TLS POST over multi-region TCP sockets
* **Latency Contribution**: ~12 – 55 ms (WAN broadcast RTT)
* **Failure Handling**: Multi-region racing: earliest successful HTTP 200 response wins; durable dispatch intent recorded in `arb-store` BEFORE sending to prevent ambiguous state loss.

#### Stage 11: Confirmation & Landing Verification
* **Source File & Function**:
  * `crates/arb-land/src/lib.rs`: `get_bundle_statuses` (line 656), `get_bundle_statuses_detailed` (line 565)
  * `crates/arb-land/src/lifecycle.rs`: `evaluate_lifecycle_with_block_height` (line 136)
  * `crates/arb-ingest/src/rpc_pool.rs`: `RpcPool::poll_signature_with_details` (line 1447)
  * `crates/arb-bot/src/supervisor/dispatch.rs`: lines 893–965
* **Programming Language**: Rust (100%)
* **Runtime / Process**: `arb-bot` asynchronous background Tokio task
* **Data Serialization Format**: JSON-RPC bundle status and signature status parsing
* **Communication Mechanism**: Concurrent HTTP polling (350–400 ms intervals, up to 20 attempts)
* **Latency Contribution**: ~400 ms – 1.2 s (block production cadence)
* **Failure Handling**: Evaluates final status: `Landed`, `Failed`, or `Expired` (based on `lastValidBlockHeight`). Status is durably updated in `arb-store`.

#### Stage 12: Settlement Reconciliation & Accounting
* **Source File & Function**:
  * `crates/arb-bot/src/supervisor/dispatch.rs`: lines 1030–1120
  * `crates/arb-bot/src/supervisor/recovery.rs`: `Supervisor::reconcile_pending_dispatches` (line 78)
  * `crates/arb-store/src/lib.rs`: `Store::record_outcome` (line 95), `Store::resolve_dispatch_intent` (line 109); `SqliteStore` implementations (lines 267, 482), `PostgresStore` implementations (lines 957, 1072)
* **Programming Language**: Rust (100%)
* **Runtime / Process**: `arb-bot` native binary
* **Data Serialization Format**: SQL parameterized queries (`INSERT INTO outcomes`, `UPDATE dispatch_intents`)
* **Communication Mechanism**: In-process SQLite connection or TCP PostgreSQL connection
* **Latency Contribution**: ~1 – 5 ms (asynchronous database I/O)
* **Failure Handling**: Fail-closed ledger. If an outcome cannot be persisted, it is flagged in local emergency logs and broadcast via Telegram alert. Pending dispatches from previous crashes are reconciled upon bot restart before new opportunities are evaluated.

---

## 3. Summary of Latency Profiles Across Runtimes

| Pipeline Phase | Module / Crate | Runtime Engine | Typical Latency | Language |
| :--- | :--- | :--- | :--- | :--- |
| **Ingestion Decoding** | `arb-ingest` / `arb-dex` | Tokio native | 5 – 25 µs | **Rust** |
| **Route Pathfinding** | `arb-engine` | Native SIMD / multi-thread | 50 – 300 µs | **Rust** |
| **Optimal Sizing** | `arb-engine` / `arb-math` | Native ALU | 10 – 50 µs | **Rust** |
| **Exact Quoting** | `arb-math` | Fixed-point integer | 5 – 15 µs | **Rust** |
| **Risk & Invariant Gates** | `arb-risk` / `arb-bot` | Native branch checks | 2 – 10 µs | **Rust** |
| **V1 Tx Assembly** | `arb-tx` | Zero-alloc buffer write | 15 – 45 µs | **Rust** |
| **Ed25519 Signing** | `arb-tx` | dalek CPU instructions | 18 – 30 µs | **Rust** |
| **Local Path Latency** | **Subtotal (Ingest ➔ Tx)** | **Pure Rust Process** | **< 480 µs** | **Rust** |
| **RPC Simulation** | `arb-sim` | Network HTTP RPC | 15 – 45 ms | Rust client |
| **Jito Submission** | `arb-land` | Multi-region HTTP POST | 12 – 55 ms | Rust client |
| **Block Inclusion** | Solana Validators | SVM Consensus | 400 – 1200 ms | Solana Cluster |

**Conclusion**: The critical trading path from WebSocket state update to signed transaction is executed in **less than 500 microseconds** within a single Rust process. No external runtime or non-Rust script participates in any part of this sequence.
