# Priority Fees, Compute Units & Transaction V1 Audit

## 1. Executive Summary
This audit inspects Solana transaction fee economics, compute unit estimation across DEX protocols, and the distinction between Legacy/V0 Compute Budget instructions and Transaction V1 native message-level resource configuration.

---

## 2. Priority Fee Mathematics & Transport Semantics

### 2.1 Legacy & Version 0 Transactions (SIMD-0160 & SIMD-0296)
In Legacy and V0 transactions, the priority fee is specified via `ComputeBudgetInstruction::set_compute_unit_price(micro_lamports)`.
The Solana runtime computes the fee as:
$$\text{PriorityFeeLamports} = \left\lceil \frac{\text{MicroLamportsPerCU} \times \text{RequestedCULimit}}{1,000,000} \right\rceil$$

**Critical Invariants**:
1. The fee charged by the validator is based on the **requested CU limit**, NOT the actual CUs consumed during execution.
2. If an arbitrage bot sets a requested CU limit of 350,000 units and a price of 10,000 micro-lamports/CU, the validator charges:
   $$\left\lceil \frac{10,000 \times 350,000}{1,000,000} \right\rceil = 3,500\text{ lamports}$$
   even if the transaction only executed 120,000 CUs before completing.
3. Therefore, underestimating the requested CU limit causes transaction failure (`InstructionError::ComputeBudgetExceeded`), while over-requesting causes excessive priority fee payment.

### 2.2 Transaction V1 Wire Semantics (SIMD-0385)
Transaction V1 (`0x81` discriminator prefix, 4,096-byte MTU) replaces explicit `ComputeBudget` program instructions with native message-level configuration:
- In V1, bit `0x01` of the `TransactionConfigMask` corresponds to `CONFIG_MASK_PRIORITY_FEE`.
- When set, the configuration value in the message header is an **absolute total in lamports** (8-byte unsigned integer), NOT micro-lamports per CU.
- Verified against commit `cd42b2c`: `dlmm_4leg`, `cross_dex`, and `prewarmed_template` use `total_priority_fee_lamports(micro_lamports, cu_limit)` to convert quotes into the exact absolute lamports total before serializing the V1 message.
- Duplicate `ComputeBudget` instructions are strictly excluded from V1 instruction payloads.

---

## 3. Compute Unit Estimation & Runtime Ceilings

### 3.1 Protocol-Aware Hop Sizing
Rather than applying a crude flat guess per swap, `crates/arb-tx/src/primitives/cu.rs` defines exact protocol-specific execution costs:
- `RaydiumV4` / `RaydiumCpmm`: 35,000 CUs (simple constant product math)
- `PumpFun`: 42,000 CUs (bonding curve evaluation)
- `PumpSwap`: 48,000 CUs
- `MeteoraDammV2`: 65,000 CUs
- `OrcaWhirlpool` / `RaydiumClmm`: 85,000 CUs (tick array traversal)
- `MeteoraDlmm`: 120,000 CUs (active bin crossing and bin array shifts)
- Ephemeral ATA Creation / Closure: 20,000 CUs per temporary intermediate token account
- Kamino Flash Loan Borrow + Repay: 28,000 CUs

### 3.2 Dynamic CU Limit & Safety Buffer
- A 15% safety buffer is applied over measured base requirements:
  $$\text{CULimit} = \min\left(\text{BaseRequired} \times 1.15, \text{APPLICATION\_SAFETY\_CU\_LIMIT}\right)$$
- **Disambiguation**:
  - `SOLANA_RUNTIME_MAX_COMPUTE_UNITS = 1_400_000`: Solana network-wide consensus maximum per transaction.
  - `APPLICATION_SAFETY_CU_LIMIT = 350_000`: Sentinel application policy ceiling. Sizing an arbitrage route beyond 350,000 CUs indicates an inefficient multi-hop route whose priority fee and execution latency would be dominated by simpler 2-leg or 3-leg cycles.
