# REPOSITORY ARCHITECTURE REPORT
## SOLANA_FLASHLOAN_ARBITUGE — Modular Monorepo Architecture & Control Pipeline

**Date**: 2026-11-16  
**Auditor**: Principal Polyglot Software Architect & Systems Engineer  
**Repository**: `https://github.com/mohamedazooz/SOLANA_FLASHLOAN_ARBITUGE`  
**Toolchain**: Rust 1.99.1 (Edition 2024), Solana SVM Native Tools  

---

## 1. Monorepo Structural Hierarchy

The `SOLANA_FLASHLOAN_ARBITUGE` repository is structured as a zero-circular-dependency Rust workspace. Dependencies flow strictly downwards:

```
                                  +---------------------------+
                                  |         arb-bot           |  (Application Binary)
                                  +---------------------------+
                                                |
               +----------------+---------------+---------------+----------------+
               |                |               |               |                |
               v                v               v               v                v
      +----------------+ +-------------+ +-------------+ +-------------+ +-------------+
      |    arb-sim     | |  arb-land   | |  arb-store  | | arb-metrics | |   arb-tx    |
      +----------------+ +-------------+ +-------------+ +-------------+ +-------------+
               |                |               |               |                |
               +----------------+---------------+---------------+----------------+
                                                |
                                                v
                                       +----------------+
                                       |    arb-risk    |  (Risk Gate & Kill Switch)
                                       +----------------+
                                                |
                                                v
                                       +----------------+
                                       |   arb-engine   |  (Route Graph & Optimizer)
                                       +----------------+
                                                |
                                                v
                                       +----------------+
                                       |   arb-ingest   |  (Market Data & RpcPool)
                                       +----------------+
                                                |
                                                v
                                       +----------------+
                                       |    arb-dex     |  (DEX Protocol Decoders)
                                       +----------------+
                                                |
                                                v
                                       +----------------+
                                       |    arb-math    |  (Canonical Economics & Quoting)
                                       +----------------+
                                                |
                                                v
                               +--------------------------------+
                               |    arb-types / arb-constants   |  (Leaf Crates)
                               +--------------------------------+
```

---

## 2. Crate-by-Crate Technical Breakdown

### 1. `arb-types` & `arb-constants` (Leaf Layer)
- **Responsibility**: Foundational domain primitives, strongly typed currency units, and verified on-chain addresses.
- **Key Modules**:
  - `dimensional.rs`: Compile-time typed currency wrappers (`CheckedLamports`, `CheckedUsdc`, `ProfitAmount`) preventing invalid cross-mint arithmetic.
  - `constants`: Canonical program IDs (Meteora DLMM, Raydium AMM V4, Raydium CPMM, Orca Whirlpool, Kamino Lending), mints (WSOL, USDC, USDT), and system accounts. Zero duplicate address definitions.

### 2. `arb-math` (Canonical Economic & Quoting Layer)
- **Responsibility**: Authoritative single source of truth for all mathematical modeling, pool curve quoting, and net profitability calculations.
- **Key Modules**:
  - `economic.rs`: Canonical formula $NetProfit = Output - Repayment - Tip - PriorityFee - ContingencyBuffer$.
  - `kamino.rs`: U68F60 fixed-point fee evaluation with $2^{60}$ minimum fee floor.
  - `cpmm.rs`, `clmm.rs`, `dlmm.rs`: Exact protocol curve implementations matching on-chain smart contracts.

### 3. `arb-dex` (Protocol Adapter & Decoding Layer)
- **Responsibility**: Decoding live on-chain account data into high-performance in-memory representations.
- **Protocols Supported**:
  - Meteora DLMM (`decode_dlmm_state`, bin array navigation)
  - Raydium AMM V4 (`SwapBaseInV2` 8-account layout)
  - Raydium CPMM & CLMM
  - Orca Whirlpool (tick arrays, sqrtPriceX64)
  - PumpFun bonding curves

### 4. `arb-ingest` (Market Data & Stream Supervision)
- **Responsibility**: Ingestion of real-time account updates via Yellowstone gRPC and WebSocket connections.
- **Key Features**:
  - Dynamic `RpcPool` with multi-endpoint failover and 60-second cooldown on HTTP 429 rate limits.
  - `quote_readiness`: Evaluates bank slot generation, age, and state freshness to eliminate stale quote execution.

### 5. `arb-engine` (Route Graph Discovery & Sizing Optimizer)
- **Responsibility**: Topology graph construction, cycle detection (2-hop and 3-leg paths), and trade sizing optimization.
- **Key Algorithms**:
  - Golden-section / ternary search over candidate input sizes.
  - Sizing bounded by supply-vault liquidity (`available_amount`).

### 6. `arb-risk` (Risk Mitigation & Circuit Breakers)
- **Responsibility**: Pre-trade validation gate and circuit breaker engine.
- **Enforcements**:
  - Maximum trade size cap ($100k USD)
  - Max hourly / daily loss limits ($200 / $500 USD)
  - Breaker trip on 1,000 consecutive failure events
  - `ARB_MAX_IMPLAUSIBLE_ROI_BPS` (15%) guard

### 7. `arb-tx` (Transaction Synthesis & Hotpath Compiler)
- **Responsibility**: Wire serialization, instruction building, V0/V1 compiler, and sub-microsecond template patching.
- **Key Modules**:
  - `v1_builder`: SIMD-0385 compliant 4,096-byte transaction compiler.
  - `prewarmed_template`: In-place byte offset mutation for instant dispatch.
  - `atomic_integrity`: Cryptographic validation of borrow-swap-repay atomicity.

### 8. `programs/arb-executor` (On-Chain Smart Contract)
- **Responsibility**: Solana BPF program executing atomic 3-leg swaps via Meteora DLMM Swap2 CPI.
- **Guarantees**:
  - Program ID validation
  - Sequential balance delta checks
  - Zero pre-existing intermediate inventory subsidization

---

## 3. The Supervisor Execution Lifecycle

The bot runtime is orchestrated by the `Supervisor` actor model in `arb-bot/src/supervisor/`:

```
+-----------------------------------------------------------------------------------+
|                               Supervisor Run Loop                                 |
+-----------------------------------------------------------------------------------+
        |
        v
  [1. Ingest Ticks] --------> Read fresh slot & bank updates from RpcPool
        |
        v
  [2. Strategy Evaluation] -> Evaluate 2-hop cross-pool & 3-leg DLMM cycles
        |
        v
  [3. Sizing & Economics] --> Optimize input amount; enforce canonical net profit floor
        |
        v
  [4. Risk Gate] -----------> Check circuit breakers, loss limits, and ROI bounds
        |
        v
  [5. Assembly & Patching] -> Patch pre-warmed wire template (borrow, repay, tip, slot)
        |
        v
  [6. Non-Broadcast Sim] ---> Optional RPC preflight simulation against live bank state
        |
        v
  [7. Dispatch Engine] -----> Multi-region Jito bundle submission (or direct SWQoS)
        |
        v
  [8. Recovery Service] ----> Monitor transaction landing across 160-slot validity window
```

---

## 4. Architectural Invariants

1. **Strict Atomicity**: No flash loan transaction is split across multiple instructions or bundles.
2. **Deterministic Integer Math**: Absolute zero floating-point operations in authoritative financial decisions.
3. **Fail-Closed Operations**: When market data is stale, RPC fails, or profit is indeterminate, the engine refuses execution.
