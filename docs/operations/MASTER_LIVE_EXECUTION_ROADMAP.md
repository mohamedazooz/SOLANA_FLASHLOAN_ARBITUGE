# MASTER LIVE EXECUTION ROADMAP & REVERSE ENGINEERING AUDIT
## Solana High-Frequency MEV Arbitrage Engine
**Version**: 1.0.0-PROD-READINESS  
**Date**: 2026-11-09  
**Target Environment**: Solana Devnet / Mainnet-Beta  
**Author**: Principal Polyglot Software Architect & Senior Reverse Engineer  
**Cross-Referenced Documents**: `audit2.md`, `audit3.md`, `audit4.md`, `docs/COMPREHENSIVE_REMOVAL_PLAN.md`, `docs/adr/0001-0012`

---

## 🎯 Executive Summary & Diagnostic Truth

Through deep reverse engineering of `crates/arb-tx`, `crates/arb-ingest`, `crates/arb-sim`, `crates/arb-risk`, and `crates/arb-bot`, we have established that **simply removing safety locks (`ARB_LIVE=1`, `KillSwitch`) will NOT execute profitable live transactions**. In fact, executing the current codebase live will result in **100% on-chain reverts and wasted transaction fees** due to critical structural bugs in account resolution and transaction assembly.

This Roadmap provides the definitive, battle-tested plan to fix every fatal defect, eliminate friction locks, optimize execution latency down to sub-10ms, and ensure transactions actually land inside Solana blocks.

```
                              THE MEV EXECUTION PIPELINE
                              
[ Yellowstone Geyser ] ──► [ Ingest / State Cache ] ──► [ Graph / Strategy ]
                                                               │
                                                               ▼
[ Jito Block Engine ] ◄── [ Pre-Dispatch Reval ] ◄── [ Tx Assembly & ALTs ]
         │
         ▼
[ Solana Block Landing ]
```

---

## 🔬 Matrix of Critical Audit Findings & Resolutions

Cross-referenced against `audit2.md`, `audit3.md`, and `audit4.md`:

| # | Severity | File & Location | Defect / Vulnerability Description | Live Consequence | Resolution Plan |
|---|---|---|---|---|---|
| **F-01** | **P0 - Blocker** | `crates/arb-tx/src/lib.rs:1442-1455` | **Raydium V4 Accounts Hardcoded to SOL-USDC**: Overrides only `amm_pool`. All 17 other accounts (Serum market, OpenOrders, Bids, Asks, Vaults) are hardcoded. | **100% On-chain Revert** on any non-SOL-USDC pair. | Read live vault and OpenBook accounts from `arb-ingest::MarketStateCache`, OR route via Jupiter v6 `/swap-instructions`. |
| **F-02** | **P0 - Blocker** | `crates/arb-tx/src/lib.rs:1464-1484` | **Orca Whirlpool Bogus PDA Derivation**: Derives vaults using non-existent seed `[b"token_vault_a", pool]` and hardcodes `tick_spacing=64`, `tick_current=0`. | **100% On-chain Revert** due to invalid accounts and wrong tick arrays. | Read `token_vault_a/b` directly from decoded Whirlpool account layout; compute dynamic tick array PDAs. |
| **F-03** | **P0 - Blocker** | `crates/arb-tx/src/lib.rs:1510-1533` | **Meteora DLMM Bogus PDA Derivation**: Derives reserves using `[b"reserve_x", pool]` (reserves are stored in `LbPair` body, not derived) and hardcodes bin step. | **100% On-chain Revert**. | Parse `LbPair` struct for reserve mints and vaults; decode live active bin arrays. |
| **F-04** | **P0 - Blocker** | `crates/arb-tx/src/lib.rs:1584-1599` | **Raydium CLMM Config Mismatch**: Sets `amm_config` to Raydium CPMM config (`D4A2...`), hardcodes `tick_spacing=60`. | **100% On-chain Revert**. | Ingest proper CLMM `AmmConfig` PDA and dynamic tick arrays. |
| **F-05** | **P0 - Blocker** | `crates/arb-tx/src/template.rs:166-200` | **PreWarmedTxTemplate Omits Swap Amounts**: Patches borrow, repay, tip, blockhash, but never updates `amount_in` and `min_out` in instruction data. | **Slippage / Revert**: Swap executes with stale amounts or causes Kamino repayment insolvency. | Patch inner instruction data byte buffers with calculated `amount_in` and `min_out` during hot-path execution. |
| **F-06** | **P0 - Blocker** | `crates/arb-tx/src/pump_fun_raydium.rs:530-553` | **Legacy MTU 1293 Bytes Overflow**: Assembles legacy tx without ALTs. 18 Raydium + 12 Pump accounts = 1293 bytes (> 1232 limit). | **100% Dropped by Validators**. | Convert `PumpFunRaydiumArbitrageBuilder` to v0 Versioned Transaction with Address Lookup Tables (ALTs). |
| **F-07** | **P1 - Critical** | `crates/arb-tx/src/lib.rs:1903` | **Kamino Flash Fee Hardcoded to 0**: In `build_kamino_bundle`, `flash_loan_fee_sf` is set to 0. | **Kamino Repay Failure**: `InsufficientRepayAmount` error reverts transaction. | Decode `flash_loan_fee_sf` from Kamino Reserve state on-chain via Q64.64 math. |
| **F-08** | **P1 - Critical** | `crates/arb-sim/src/litesvm_sim.rs:45-59` | **LiteSVM Empty Sandbox**: `LiteSVM::new()` called without loading BPF ELF binaries or accounts. | **Instant Crash / Failure** with `ProgramAccountNotFound`. | Bypass via `FAST_PATH_SKIP_RPC_SIM=true` or properly inject ELF binaries and Geyser state. |
| **F-09** | **P1 - High** | `crates/arb-bot/src/supervisor/dispatch.rs:371-401` | **Final Stream-Readiness Fail-Closed**: Aborts dispatch if slot advanced or stream generation toggled during simulation. | **Lost Alpha**: Valid opportunities rejected due to minor network jitter. | Weaken to `warning-only` (ADR-0006) while relying on Kamino on-chain min-out backstop. |
| **F-10** | **P1 - High** | `crates/arb-bot/src/main.rs:601` | **ARB_LIVE Master Policy Gate**: Requires `ARB_LIVE=1` explicitly or bails at startup. | **Process Exit**: Cannot run live mode without env variable. | Remove check (ADR-0001); make `--mode live` first-class. |

---

## 🗺️ The 5-Phase Master Execution Architecture

```mermaid
flowchart TD
    subgraph Phase 0: Immediate Config
        A1["Set .env Flags"] --> A2["Bypass Risk Gate & RPC Sim"]
        A2 --> A3["Test Devnet Connectivity"]
    end

    subgraph Phase 1: Policy Lock Removal
        B1["Remove ARB_LIVE Startup Gate"] --> B2["Remove Dispatch Re-check"]
        B2 --> B3["Start KillSwitch Unblocked"]
    end

    subgraph Phase 2: Transaction Assembly P0 Fixes
        C1["Fix PreWarmedTxTemplate Amount Patching"] --> C2["Convert Legacy Tx to v0 with ALTs"]
        C2 --> C3["Fix Kamino Q64.64 Fee Ingestion"]
        C3 --> C4["Enable Jupiter v6 Dynamic Account Bridge"]
    end

    subgraph Phase 3: Fast-Path Sub-10ms Pipeline
        D1["Weaken Stream-Readiness to Warning-Only"] --> D2["Zero SOL Balance Blocking Elimination"]
        D2 --> D3["Direct Jito / SWQoS Dispatch"]
    end

    subgraph Phase 4: Long-Term Protocol Construction
        E1["Wire GUARD-01 Guarded Quote"] --> E2["Native PumpSwap v2 Adapter"]
        E2 --> E3["Durable State Machine 14.5"]
    end

    Phase 0 --> Phase 1 --> Phase 2 --> Phase 3 --> Phase 4
```

---

## 📋 Comprehensive Granular TODO List

### Phase 0: Instant Config-Only Smoothness (Zero Code Changes — 5 Minutes)
- [ ] **0.1 Setup Environment Flags in `.env`**:
  ```bash
  ARB_LIVE=1
  ARB_MODE=live
  BYPASS_RISK_GATE=1
  FAST_PATH_SKIP_RPC_SIM=true
  FAST_PATH_IMMEDIATE_DISPATCH=true
  MIN_SOL_RESERVE_LAMPORTS=0
  ENABLE_PREFLIGHT_SIMULATION=false
  ```
- [ ] **0.2 Clear Stale Markers**:
  - Remove `data/kill_switch` or any marker file if present.
- [ ] **0.3 Devnet Sanity Run**:
  - Run `cargo check --workspace` to ensure build environment is clean.

---

### Phase 1: Policy Lock Removal (Safe-ish Unlocking — ADR-0001 & ADR-0002)
- [ ] **1.1 Remove `ARB_LIVE` Startup Gate in `crates/arb-bot/src/main.rs`**:
  - Line 60-69: Modify `live_execution_enabled()` to return `true`.
  - Line 599-602: Remove `if !live_execution_enabled() { anyhow::bail!(...); }`.
- [ ] **1.2 Remove `ARB_LIVE` Redundant Gate in `crates/arb-bot/src/supervisor/dispatch.rs`**:
  - Line 126-128: Simplify `BotMode::Live => { ... }` removing the wrapper check.
- [ ] **1.3 Update Default Config in `config/config.example.toml`**:
  - Change `mode = "shadow"` to `mode = "live"`.
- [ ] **1.4 Modify KillSwitch Auto-Block in `crates/arb-risk/src/lib.rs`**:
  - Line 131-141: In `KillSwitch::with_file`, initialize `enabled: Mutex::new(false)`.
  - Line 179-189: In `is_blocked()`, add check for `KILL_SWITCH_DISABLED=1`.
- [ ] **1.5 Verification**:
  - Run `cargo test -p arb-bot -p arb-risk` to verify test suite passes.

---

### Phase 2: Fixing P0 Transaction Builders & Account Resolution (The Core Fixes)
- [ ] **2.1 Fix `PreWarmedTxTemplate` Swap Amount Patching (`crates/arb-tx/src/template.rs`)**:
  - Lines 166-200: In `patch_and_sign`, locate the swap instruction offsets.
  - Patch `amount_in` (u64 LE) and `min_amount_out` (u64 LE) into the instruction data buffer so hot-path trades execute with real amounts.
- [ ] **2.2 Convert `PumpFunRaydiumArbitrageBuilder` to v0 with ALTs (`crates/arb-tx/src/pump_fun_raydium.rs`)**:
  - Replace `TxBuilder::build_signed_transaction` (legacy) with `TxBuilder::compile_v0_message` using Address Lookup Tables.
  - Verify compiled transaction size is `<= 1232` bytes.
- [ ] **2.3 Fix Kamino Flash Loan Fee in `crates/arb-tx/src/lib.rs`**:
  - Line 1903: Stop passing `flash_loan_fee_sf = 0`.
  - Connect `decode_reserve_flash_fee_sf` from `crates/arb-tx/src/kamino.rs` to fetch live on-chain fee parameters.
- [ ] **2.4 Enable Jupiter v6 REST Client as Dynamic Account Resolver**:
  - In `crates/arb-bot/src/supervisor/run.rs:915`, change `let route_via_jupiter = false;` to evaluate true when a multi-DEX route involves non-canonical pools (Raydium V4 non-SOL-USDC, Orca, Meteora).
  - Use `jupiter-swap-api-client` `/swap-instructions` to resolve all 20+ accounts dynamically, bypassing broken manual PDA derivations.
- [ ] **2.5 Fix Native Pump.fun Builder WSOL Unwrapping / Rent Reclaim**:
  - Verify `crates/arb-tx/src/lib.rs:1396-1431` correctly appends CloseAccount instruction for ephemeral WSOL ATAs to return rent (0.00203928 SOL) to payer.

---

### Phase 3: Fast-Path Latency & Block Landing Optimization
- [ ] **3.1 Weaken Final Stream-Readiness Guard (`crates/arb-bot/src/supervisor/dispatch.rs`)**:
  - Lines 371-401: Convert `return;` on readiness failure to `warn!(...); arb_metrics::inc_stream_readiness_warnings();`.
  - Allow the transaction to dispatch, trusting the on-chain Kamino repayment check to protect principal.
- [ ] **3.2 Eliminate Synchronous `getBalance` RPC Call**:
  - Verify lines 129-169 in `dispatch.rs` are completely skipped when `MIN_SOL_RESERVE_LAMPORTS=0`.
- [ ] **3.3 Jito Dynamic Tip Oracle Integration (`crates/arb-land/src/tip_oracle.rs`)**:
  - Ensure WebSocket connection to `wss://bundles.jito.wtf/api/v1/bundles/tip_stream` is active.
  - Dynamically set bundle tip to 75th percentile (`P75`) of recent landed bundles:
    $$\text{Tip} = \max(\text{Tip}_{\text{floor}}, \min(\text{Edge} \times 0.70, \text{Jito}_{P75}))$$
- [ ] **3.4 Leader Schedule Optimization (`crates/arb-land/src/leader.rs`)**:
  - Check target slot leader against Jito validator registry.
  - If Leader is Jito: Dispatch bundle to multi-region block engines (Amsterdam, Frankfurt, NY).
  - If Leader is Non-Jito: Dispatch via SWQoS (Staked RPC) with compute budget priority fees.

---

### Phase 4: Long-Term Protocol Construction (Grade D — ADR-0007 to ADR-0012)
- [ ] **4.1 PumpSwap AMM Adapter (`crates/arb-tx/src/lib.rs:1441`)**:
  - Implement Anchor instruction builder for `pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA`.
  - Map the 14 required accounts (`global_config`, `pool`, `pool_authority`, `base_vault`, `quote_vault`, `fee_vault`).
- [ ] **4.2 Wire GUARD-01 Universal Guarded Quote Pipeline**:
  - Replace direct quote path in `crates/arb-bot/src/supervisor/run.rs` with `quote_route_guarded`.
  - Assert identical SHA-256 quote hash across RiskGate, TxBuilder, and Pre-Dispatch.
- [ ] **4.3 Durable State Machine & Crash Reconciliation (14.5)**:
  - Implement SQLite WAL state journal tracking bundle statuses (`Submitted -> Inflight -> Landed -> Reconciled`).
  - Add recovery loop on bot boot to check in-flight signatures before opening trading.
- [ ] **4.4 In-Process LiteSVM Fork Simulation (14.1 / 14.3)**:
  - Load Kamino, Raydium, and Pump.fun ELF binaries into `LiteSVM`.
  - Stream live account data via Yellowstone Geyser into `svm.set_account()` for sub-millisecond local simulation.

---

## 🛡️ Protocol Invariants That Must NEVER Be Removed

```rust
// NEVER REMOVE OR BYPASS THESE UNDER ANY CIRCUMSTANCES:
1. const SOLANA_PACKET_MTU_LIMIT: usize = 1232;  // Dropped by validators if exceeded
2. if blockhash == [0u8; 32] { return; }         // 100% rejected by runtime
3. Kamino Atomic Repay Invariant                  // borrow -> swap -> repay in SAME tx
4. Minimum Out Profit Floor                       // min_out >= borrow + fee + tip + profit
```

---

## 🚀 Execution Commands Runbook

### Step 1: Verification Build
```bash
cargo check --workspace --all-targets
cargo test -p arb-tx -p arb-bot
```

### Step 2: Running in Canary / Devnet Mode
```bash
cargo run --bin arb-bot -- --mode canary --config config/config.toml
```

### Step 3: Running Live with Fast-Path
```bash
# Ensure .env has FAST_PATH_SKIP_RPC_SIM=true and proper keypair
cargo run --release --bin arb-bot -- --mode live --config config/config.toml
```
