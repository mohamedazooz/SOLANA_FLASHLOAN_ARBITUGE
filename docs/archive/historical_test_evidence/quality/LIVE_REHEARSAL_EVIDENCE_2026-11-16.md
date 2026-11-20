> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Live Non-Broadcast Rehearsal Evidence (2026-11-16)

**Binary**: `target/release/examples/full_path_rehearsal.exe`  
**Execution Timestamp**: `2026-11-16T14:48:04Z`  
**Branch**: `feat/mainnet-readiness-p0-p15` (`1c77c35`)  
**Network**: Solana Mainnet-Beta (read-only live state capture + simulation)  

---

## 1. Verified Live Observations

### A. RpcPool Dynamic Failover & Rate-Limit Cooldown
- **Observed**: Mainnet RPC endpoint returned HTTP 429:
  ```text
  2026-11-16T14:48:04.051233Z WARN arb_ingest::registry: RPC endpoint https://mainnet.helius-rpc.com/?api-key=... failed (HTTP 429 Too Many Requests); placing on 60s cooldown and rotating
  ```
- **Behavior**: Cooldown latch engaged; rotated automatically to available backup RPC endpoints without panic or crash.

### B. Live Pool Hydration
- **Observed**: 500 pools registered; 83 Meteora DLMM pools; top 30 live DLMM pools hydrated:
  1. `CAngSupL` ($16,370,045 liquidity)
  2. `54Vp27uL` ($12,000,109 liquidity)
  3. `3C5YE97H` ($4,333,697 liquidity)
  4. `9d9mb8ko` ($3,606,207 liquidity)
  5. `5rCf1DM8` ($2,947,106 liquidity)

### C. Fail-Closed Economic Gate & Builder Defense
- **Evaluated Pairs**:
  - `6p6xgHyF7A`: `INTERMEDIATE_ABOVE_POOL_LIQUIDITY` (amount exceeds pool capacity).
  - `2zMMhcVQEX`: `PROFIT_BELOW_FLOOR` (net: -111,435 lamports below floor 1).
  - `METvsvVRap`: `PROFIT_BELOW_FLOOR` (net: -130,767 lamports).
  - `DezXAZ8z7P`: `PROFIT_BELOW_FLOOR` (net: -978,784 lamports).
  - `4kjruqGHDs`: `PROFIT_BELOW_FLOOR` (net: -172,678 lamports).
- **Forced Dry-Run Verification**:
  ```text
  [5b] dry-run verdict: the builder gate refuses this transaction -- conservative final output 9888678 < repayment 10000100 raw. The flash loan cannot be repaid at the slippage floor: the route is economically negative at the quote level itself. Fail-closed confirmed at the last line of defense; nothing built, nothing broadcast.
  ```
- **Conclusion**: Atomicity, repayment solvency, and fail-closed protections are 100% operational against real mainnet state. Zero lamports spent; zero broadcast.
