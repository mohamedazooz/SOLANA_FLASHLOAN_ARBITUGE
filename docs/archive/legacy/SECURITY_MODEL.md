> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# SECURITY MODEL & ATTACK SURFACE ANALYSIS

## 1. Threat Matrix & Defense-in-Depth

| Threat Vector | Attack Scenario | On-Chain / Off-Chain Defense |
|---|---|---|
| **Inventory Subsidization** | Malicious route steals tokens from pre-existing wallet ATAs to fabricate fake "profit" | `arb-executor` records pre-trade intermediate token balances. Enforces final balance $\le$ initial balance. |
| **Slippage Exploitation / MEV Frontrunning** | Searcher sandwich or toxic arbitrageur crosses bins before our tx | Strict `min_amount_out` enforced per leg. Overall WSOL delta strictly asserted before repaying flash loan. |
| **Flash Loan Under-repayment** | Dust calculation round down allows borrower to return less than required | Formula compares raw bit fractions with $2^{60}$. Revert occurs if repay balance < borrow + fee. |
| **Flash Loan CPI Replay / Spoofing** | Attacker invokes flash repay from a fake instruction index or via nested CPI | Introspection via `SysvarInstructions` validates program ID and top-level instruction index strictly. |
| **State Desynchronization (Stale Data)** | Route calculated on stale slot resulting in reverted tx and wasted priority fees | Off-chain quoter checks slot freshness ($\le 1,500$ slots). Fails closed on any disconnection. |
| **Transaction Size / CU Overflow** | 3-leg DLMM exceeds 1,232 bytes (V0) or max loaded account size (64 MiB) | Uses Address Lookup Tables (ALTs) or V1 envelope. Sets `loaded_accounts_data_size_limit` to 64 MiB. |

---

## 2. On-Chain Invariants (`arb-executor`)

1. **Signer Authority**: Only the payer/signer can invoke `execute_three_leg`.
2. **Account Whitelist & Alignment**:
   - `settlement_account` must match payer's associated token account for WSOL.
   - Intermediate accounts $A$ and $B$ must match payer's associated token accounts for intermediate mints.
3. **Strict Solvency Condition**:
   $$\text{Final WSOL Balance} - \text{Initial WSOL Balance} \ge \text{min\_final\_wsol\_delta}$$
   Where:
   $$\text{min\_final\_wsol\_delta} \ge \text{Borrowed Amount} + \text{Flash Fee} + \text{Min Net Profit}$$

---

## 3. Operational Safety Defaults

- `ARB_LIVE=0`: The bot runs in dry-run / simulation mode by default.
- `DRY_RUN=true`: Transactions are simulated locally against Bankrun/RPC and not broadcast to the gossip network.
- `Shadow Mode`: Dispatches simulated bundles to measure latency and quote accuracy without capital risk.
