# FLASH LOAN & 3-POOL DLMM ARBITRAGE AUDIT REPORT
**Author:** Arbitrage & Flash Loan Specialist (Agent 3) & Principal Polyglot Architect  
**Branch:** `fix/mainnet-transport-landing`  
**Date:** 2026-11-15  
**Audit Scope:** Kamino Klend Flash Loans, 3-Pool Meteora DLMM Atomic Routing, Dynamic Fees, Sizing & Satiation Invariants  

---

## 1. Executive Summary

This audit validates the complete atomic arbitrage execution path:
$$\text{Kamino Borrow} \longrightarrow \text{DLMM Pool 1} \longrightarrow \text{DLMM Pool 2} \longrightarrow \text{DLMM Pool 3} \longrightarrow \text{Kamino Repay}$$

All instructions are packaged into **one single atomic transaction** compiled as Solana Transaction V1 (or V0 with ALT), strictly enforcing:
$$\text{final\_balance} \ge \text{borrowed\_amount} + \text{flash\_loan\_fee} + \text{all\_execution\_costs} + \text{minimum\_required\_profit}$$

---

## 2. Protocol Details & Instruction Ordering

The atomic route assembles the following sequence in exact order:
1. `ComputeBudget::SetComputeUnitLimit`: Dynamic CU bound.
2. `ComputeBudget::SetComputeUnitPrice`: Dynamic priority fee derived from gross profit.
3. `ATA::CreateIdempotent` (WSOL settlement ATA).
4. `ATA::CreateIdempotent` (Intermediate token A ATA).
5. `ATA::CreateIdempotent` (Intermediate token B ATA).
6. `klend::flash_borrow_reserve_liquidity`:
   - Program ID: `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD`
   - Discriminator: `[135, 231, 52, 167, 7, 52, 212, 193]`
   - Borrow instruction index: dynamically tracked at `borrow_idx` (typically index 5).
7. `lb_clmm::swap2` (Pool 1: WSOL $\rightarrow$ Meme A):
   - Program ID: `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`
   - Active bin array traversal + optional bitmap extension.
8. `lb_clmm::swap2` (Pool 2: Meme A $\rightarrow$ Meme B).
9. `lb_clmm::swap2` (Pool 3: Meme B $\rightarrow$ WSOL).
10. `klend::flash_repay_reserve_liquidity`:
   - Program ID: `KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD`
   - Discriminator: `[185, 117, 0, 203, 96, 245, 180, 186]`
   - Repay amount: principal amount.
   - Borrow instruction index: verified matching `borrow_idx`.
11. `SystemProgram::Transfer` (optional marked Jito tip instruction when using Jito bundle).

---

## 3. Financial & Economic Invariant Verification

- **Kamino Flash Loan Fee**:
  - Exact U68F60 fixed-point arithmetic (`arb-math/src/economic.rs`).
  - Fee formula:
    $$\text{fee} = \max\left(\frac{\text{amount} \times \text{fee\_sf}}{2^{60}}, 1\right)$$
  - Mainnet WSOL reserve fee scale factor: `fee_sf = 11_529_215_046_068` ($10^{-5}$ or 0.1 bps).
- **Zero Gross-as-Net**:
  - Gross revenue is strictly intermediate.
  - Net profit deductions include: Flash loan fee + AMM swap fees + base transaction fee + priority fee + Jito tip + token transfer fees.
  - Integer math only — zero floating-point arithmetic in authoritative risk gates.

---

## 4. Test Evidence Summary

1. **3-Pool Atomic Assembly & Ordering**:
   - `cargo test -p arb-tx --test three_pool_flash_arb_tests`: 17 passed, 0 failed.
   - Tested: duplicate pool rejection, non-closing route rejection, mint chain mismatch, insufficient repayment, borrow/repay index alignment.
2. **Differential Quote Verification**:
   - `cargo test -p arb-engine --test meteora_sdk_differential_tests`: 5 passed, 0 failed.
   - `cargo test -p arb-engine --test meteora_mainnet_sdk_differential`: 3 passed, 0 failed.
   - `cargo test -p arb-math --test differential`: 50 passed, 0 failed.
3. **Transaction Size Ladder**:
   - `cargo test -p arb-tx --test v1_size_ladder_tests`: 1 passed, 0 failed.
   - Real 3-pool DLMM routes fit comfortably between 1,574 and 2,465 serialized bytes, well within the V1 4,096-byte limit.

---

## 5. Status Classification
- **Kamino Borrow/Repay Wiring**: **CONFIRMED** (tested and verified with exact discriminators).
- **Meteora DLMM 3-Pool Swap Routing**: **CONFIRMED** (tested with multi-bin arrays and exact-in quotes).
- **Economic Invariant Enforcement**: **CONFIRMED** (canonical `arb-math` integer calculations).
