> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# MAINNET EXECUTION READINESS REPORT
**Lead Architect:** Principal Solana HFT Architect, Senior Rust Engineer & DevOps Supervisor  
**Branch:** `fix/mainnet-transport-landing`  
**Date:** 2026-11-15  
**Final Status:** READY FOR OPERATOR-AUTHORIZED SHADOW SOAK & CONTROLLED BROADCAST  

---

## 1. Executive Summary & Acceptance Checklist

| Requirement | Acceptance Criteria | Audit Status | Evidence |
|---|---|---|---|
| **Transport Compatibility** | Coherent V1/V0 policy; explicit capability checks for Direct RPC, SWQoS, and Jito | ✅ **PASSED** | `crates/arb-tx/src/transport/selection.rs` unit tests (8/8) |
| **V1/V0 Release Policy** | No contradictory env vars; proper wire MTUs (1,232 vs 4,096 bytes) | ✅ **PASSED** | `dispatch.rs` + `.env.example` reconciled |
| **Landing State Machine** | 10 states; never expire purely on polling timeout; lastValidBlockHeight proof | ✅ **PASSED** | `crates/arb-land/src/lifecycle.rs` unit tests (5/5) |
| **3-Pool Atomic Execution** | Kamino borrow $\rightarrow$ 3x DLMM swaps $\rightarrow$ Kamino repay in ONE tx | ✅ **PASSED** | `three_pool_flash_arb_tests.rs` (17/17) |
| **Differential Quotes** | Local math vs Meteora SDK differential verification | ✅ **PASSED** | `meteora_sdk_differential_tests.rs` (5/5) & `differential.rs` (50/50) |
| **Mainnet RPC Simulation** | Non-broadcast live RPC simulation with real program logs | ✅ **PASSED** | `mainnet_non_broadcast_simulation.rs` (2/2) |
| **Static Analysis & Linting** | `cargo fmt --check` clean, `cargo clippy -D warnings` clean | ✅ **PASSED** | 0 warnings, 0 errors across workspace |

---

## 2. Root Cause & Architecture Remediation Summary

1. **Transaction Transport Selection Layer**:
   - Replaced fragmented environment variable reads with the unified `TransportCoordinator` (`crates/arb-tx/src/transport/selection.rs`).
   - Enabled Jito bundle V1 support with explicit capability checks, solving the contradictory restriction where V1 was mandatory but Jito was blocked.
2. **Landing Lifecycle & Expiration Logic**:
   - Upgraded `LandStatus` to the complete 10-state lifecycle in `crates/arb-land/src/lifecycle.rs`.
   - Prevented false `Expired` reports: an unconfirmed transaction remains `UNKNOWN` while `current_block_height <= last_valid_block_height`.
   - Added `getTransaction` with `maxSupportedTransactionVersion: 1` to extract on-chain metadata.
3. **Flash Loan Invariant Verification**:
   - Tested atomic 3-pool DLMM execution verifying `borrow_instruction_index` tracking and non-reverting repay calls.
   - Enforced canonical U68F60 Kamino fee accounting and strict integer net profit guards.

---

## 3. Deployment Instructions

1. **Deploy to VPS Host**:
   ```bash
   git fetch origin
   git checkout fix/mainnet-transport-landing
   cargo build --release -p arb-bot
   ```
2. **Environment Configuration**:
   - Copy `.env.example` to `.env` (never commit `.env`).
   - Configure RPC URLs (`SOLANA_RPC_PRIMARY`, `SOLANA_RPC_URL`).
   - Keep safety defaults:
     ```bash
     ARB_MODE=simulate
     ARB_LIVE=0
     DRY_RUN=true
     SOLANA_TX_V1_DIRECT_ENABLED=false
     SOLANA_TX_V1_JITO_ENABLED=false
     ```
3. **Execute 24-Hour Shadow Soak**:
   - Run in `simulate` mode with live WebSocket/RPC feeds.
   - Monitor Telegram telemetry and confirm zero unexpected panics or state drift.

---

## 4. Controlled Mainnet Broadcast Runbook (Post-Soak & Human Authorization Only)

1. Ensure written operator authorization.
2. Fund dedicated test wallet with max spending limit (e.g. 0.05 SOL).
3. Set in `.env`:
   ```bash
   ARB_MODE=live
   ARB_LIVE=1
   SOLANA_TX_V1_DIRECT_ENABLED=true
   SOLANA_TX_V1_JITO_ENABLED=true
   ```
4. Start bot under systemd / supervisor.
5. In case of unexpected behavior, invoke the emergency stop:
   ```bash
   curl -X POST http://127.0.0.1:8080/emergency-stop -H "Authorization: Bearer <SENTINEL_ADMIN_KEY>"
   ```

---

## 5. Rollback Instructions

If unexpected execution anomalies occur:
1. Immediately trigger the emergency stop latch via Telegram (`/kill`) or HTTP admin API.
2. Revert to simulate mode:
   ```bash
   sed -i 's/ARB_LIVE=1/ARB_LIVE=0/g' .env
   sed -i 's/ARB_MODE=live/ARB_MODE=simulate/g' .env
   systemctl restart solana-arb-bot
   ```
3. If necessary, checkout previous stable tag/commit on `main`.
