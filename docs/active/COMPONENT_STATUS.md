# Component Verification Matrix

This matrix documents the verification maturity of every functional component across the arbitrage pipeline as of baseline commit `d090123`.

Maturity Levels:
1. **Implemented**: Code exists and compiles in the workspace.
2. **Wired**: Connected into the active runtime dataflow (`crates/arb-bot/src/supervisor/`).
3. **Tested**: Covered by automated unit, property, or integration tests in CI.
4. **Simulated**: Proven via non-broadcast Solana RPC `simulateTransaction`.
5. **Shadow-Tested**: Run in live shadow mode on a VPS without broadcasting transactions.
6. **Production-Verified**: Executed and confirmed on Solana mainnet with real balance deltas.

---

## 1. Ingestion & Market State Layer (`arb-ingest`)

| Component | Implemented | Wired | Tested | Simulated | Shadow-Tested | Production-Verified | Evidence / Notes |
|---|:---:|:---:|:---:|:---:|:---:|:---:|---|
| **Helius WebSocket Client** | ✅ | ✅ | ✅ | N/A | ✅ | ❌ | Streams account updates and slot advancing events. |
| **RPCFast WebSocket Client** | ✅ | ✅ | ✅ | N/A | ✅ | ❌ | Secondary WebSocket endpoint wired. |
| **AccountStore** | ✅ | ✅ | ✅ | N/A | ✅ | ❌ | In-memory concurrent account snapshot store with generation counters. |
| **MarketStateCache** | ✅ | ✅ | ✅ | N/A | ✅ | ❌ | Pool state cache with freshness timestamps and dead-slot tracking. |
| **State Integrity Validator** | ✅ | ✅ | ✅ | N/A | ✅ | ❌ | Verifies account owners, data lengths, and slot coherency. |
| **ProviderSupervisor** | ✅ | ✅ | ✅ | N/A | ✅ | ❌ | Health-monitoring failover circuit breaker across WS and RPC fallback. |
| **HTTP Catalog Discovery** | ✅ | ✅ | ✅ | N/A | ✅ | ❌ | Config-gated (`http_discovery_enabled`), discovers active multi-DEX pools. |
| **Yellowstone gRPC Client** | ⚠️ | ❌ | ❌ | ❌ | ❌ | ❌ | Dependency in tree; feature gated; client integration pending Phase 4. |

---

## 2. Quoting & Protocol Math Layer (`arb-math`, `arb-dex`)

| Component | Implemented | Wired | Tested | Simulated | Shadow-Tested | Production-Verified | Evidence / Notes |
|---|:---:|:---:|:---:|:---:|:---:|:---:|---|
| **Canonical Economic Engine** | ✅ | ✅ | ✅ | N/A | ✅ | ❌ | Single source of truth for fee, cost, and net profit floor. |
| **Kamino Klend Math** | ✅ | ✅ | ✅ | N/A | ✅ | ❌ | Official U68F60 formula with pre-rounding tie break. |
| **Meteora DLMM Math** | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ | Active bin pricing, bin array traversal, swap2 exact quote math. |
| **Raydium CPMM Math** | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ | Constant product reserve math ($x \cdot y = k$) with integer rounding. |
| **Raydium CLMM Math** | ✅ | ✅ | ✅ | ⚠️ | ❌ | ❌ | Sqrt-price Q64.64 tick traversal and dynamic fee upper bound (PR #89). |
| **Orca Whirlpool Math** | ✅ | ✅ | ✅ | ⚠️ | ❌ | ❌ | Tick array stepping and fee rate math implemented. |
| **Pump.fun / PumpSwap Math** | ✅ | ✅ | ✅ | ⚠️ | ❌ | ❌ | Bonding curve virtual reserve calculation and fee structure. |

---

## 3. Graph Discovery & Sizing Layer (`arb-engine`)

| Component | Implemented | Wired | Tested | Simulated | Shadow-Tested | Production-Verified | Evidence / Notes |
|---|:---:|:---:|:---:|:---:|:---:|:---:|---|
| **MarketGraph Builder** | ✅ | ✅ | ✅ | N/A | ✅ | ❌ | Directed graph indexed by input mint; enforces max graph age. |
| **DLMM Cross-Pool Strategy** | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ | Two-pool direct arbitrage discovery across distinct DLMM pools. |
| **Bellman-Ford Engine** | ✅ | ✅ | ✅ | N/A | ✅ | ❌ | Negative-log cycle detection for multi-venue pathfinding. |
| **Multi-Hop DFS Engine** | ✅ | ✅ | ✅ | N/A | ✅ | ❌ | Bounded depth-first search for 3..5 hop cyclic arbitrage. |
| **Integer Sizing Optimizer** | ✅ | ✅ | ✅ | N/A | ⚠️ | ❌ | Coarse-to-fine integer search; needs non-monotonic hardening (P0). |
| **Kamino Vault Sizing Cap** | ✅ | ✅ | ✅ | N/A | ✅ | ❌ | Clamps maximum trade size to live decoded supply-vault liquidity. |

---

## 4. Transaction Assembly & Wire Layer (`arb-tx`)

| Component | Implemented | Wired | Tested | Simulated | Shadow-Tested | Production-Verified | Evidence / Notes |
|---|:---:|:---:|:---:|:---:|:---:|:---:|---|
| **Kamino Flash Loan Builder** | ✅ | ✅ | ✅ | ✅ | ⚠️ | ❌ | Generates atomic `FlashBorrow` and `FlashRepay` instructions. |
| **Cross-DEX Flash Arb Assembler** | ✅ | ✅ | ✅ | ✅ | ⚠️ | ❌ | Coordinates multi-hop instructions with Kamino flash loan. |
| **Solana V1 Compiler** | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ | Compiles 0x81 wire format with inline accounts and 4,096-byte limit. |
| **Compute Budget Builder** | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ | Sets CU limit and micro-lamport priority fee. |
| **Loaded Data Limit Builder** | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ | Sets 16 MiB loaded accounts data size limit. |
| **Jito Tip Instruction Builder** | ✅ | ✅ | ✅ | ✅ | ⚠️ | ❌ | Injects tip transfer to randomly selected Jito tip account. |

---

## 5. Simulation & Risk Layer (`arb-sim`, `arb-risk`)

| Component | Implemented | Wired | Tested | Simulated | Shadow-Tested | Production-Verified | Evidence / Notes |
|---|:---:|:---:|:---:|:---:|:---:|:---:|---|
| **RPC Preflight Simulator** | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ | Async non-broadcast simulation via `simulateTransaction`. |
| **LiteSVM Simulator** | ✅ | ✅ | ✅ | ⚠️ | ❌ | ❌ | Optional in-memory test simulator; skipped for V1 transactions. |
| **RiskGate** | ✅ | ✅ | ✅ | N/A | ✅ | ❌ | Max trade size, daily loss limit, consecutive failure tracking. |
| **KillSwitch** | ✅ | ✅ | ✅ | N/A | ✅ | ❌ | Local file watch (`KILL_SWITCH`) and authenticated `/pause` API. |
| **GUARD-01 Pre-Dispatch** | ✅ | ✅ | ✅ | N/A | ✅ | ❌ | Revalidates stream generation and pool freshness before dispatch. |

---

## 6. Landing & Settlement Layer (`arb-land`, `arb-bot`)

| Component | Implemented | Wired | Tested | Simulated | Shadow-Tested | Production-Verified | Evidence / Notes |
|---|:---:|:---:|:---:|:---:|:---:|:---:|---|
| **JitoLander (Bundle Dispatch)** | ✅ | ✅ | ✅ | ⚠️ | ⚠️ | ❌ | Block engine JSON-RPC client with tip verification. |
| **Direct RPC Transport** | ✅ | ✅ | ✅ | ✅ | ⚠️ | ❌ | Direct `sendTransaction` submission path. |
| **Dynamic Tip Oracle** | ✅ | ✅ | ✅ | N/A | ✅ | ❌ | Tracks percentile tip fee recommendations. |
| **Recovery & Reconciliation** | ✅ | ✅ | ⚠️ | ❌ | ❌ | ❌ | Reconciles ambiguous dispatches against slot/blockhash progress. |
| **Realized PnL Derived Balance** | ✅ | ✅ | ⚠️ | ❌ | ❌ | ❌ | Compares pre/post balances; requires production mainnet execution. |
