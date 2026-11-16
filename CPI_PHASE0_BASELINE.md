# CPI_PHASE0_BASELINE.md: Solana Atomic CPI Swap Arbitrage Engine Baseline Audit

**Repository**: `https://github.com/mohamedazooz/SOLANA_FLASHLOAN_ARBITUGE`  
**Active Branch**: `feat/meteora-dlmm-swap2-atomic-cpi`  
**Base Commit (HEAD)**: `bf9d25cd9d4cebbc7727649a3eb57968eb822c7e`  
**Rust Toolchain**: `1.99.1` (Edition 2024, let-chains enabled)  
**Workspace**: `crates/*`, `programs/*`  

---

## 1. Executive Summary & Objective

This document establishes the authoritative engineering baseline for transitioning the arbitrage execution subsystem from top-level multi-instruction swaps into a **production-grade, atomic on-chain CPI execution model** anchored around:
1. **Kamino Klend Flash Loans** (`KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD`).
2. **Dedicated Arbitrage Executor Program** (`programs/arb-executor`).
3. **Meteora DLMM `swap2` CPI Integration** (`LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`).
4. **Three-Leg WSOL-Anchored Route**: $\text{WSOL} \to \text{Meme A} \to \text{Meme B} \to \text{WSOL}$.
5. **Exact Integer Accounting & Sequential Balance Isolation**: Zero pre-existing inventory subsidy, verified on-chain output delta checks, strictly non-overlapping single-hop slippage.

---

## 2. Invariant: Kamino Flash Loan Rejection of CPI Invocations

### 2.1 The Architectural Constraint
Kamino Lending (`klend`) explicitly rejects flash loan borrows and repays when executed through Cross-Program Invocation (CPI).

In Kamino's on-chain implementation (`handler_flash_borrow_reserve_liquidity.rs` and `handler_flash_repay_reserve_liquidity.rs`):
- Kamino inspects the Solana Instructions Sysvar (`sysvar::instructions`).
- It verifies that the caller of `flash_borrow_reserve_liquidity` and `flash_repay_reserve_liquidity` is an explicit, top-level transaction instruction invoked by the transaction signer.
- If called via CPI from an intermediary program, Kamino emits `InstructionNotAllowedInCPI` or fails sysvar instruction boundary verification.

### 2.2 Top-Level Transaction Structure
Consequently, the flash loan lifecycle **MUST NEVER** be placed inside an executor program. Instead, the transaction must follow a strictly ordered top-level layout:

```text
[Top-Level Solana Transaction]
├── Instruction 0: ComputeBudget::SetComputeUnitLimit
├── Instruction 1: ComputeBudget::SetComputeUnitPrice
├── Instruction 2: AssociatedTokenAccount::CreateIdempotent (WSOL, if not present)
├── Instruction 3: AssociatedTokenAccount::CreateIdempotent (Meme A, if not present)
├── Instruction 4: AssociatedTokenAccount::CreateIdempotent (Meme B, if not present)
├── Instruction 5: Kamino::flash_borrow_reserve_liquidity (borrow_instruction_index = 5)
│                  Liquidity transferred directly to user's WSOL ATA
├── Instruction 6: ArbExecutor::execute_three_leg (Top-Level Program Call)
│                  ├── CPI 1: Meteora DLMM swap2 (WSOL -> Meme A)
│                  │          User signer forwarded, delta A measured & verified
│                  ├── CPI 2: Meteora DLMM swap2 (Meme A -> Meme B)
│                  │          User signer forwarded, delta B measured & verified
│                  └── CPI 3: Meteora DLMM swap2 (Meme B -> WSOL)
│                             User signer forwarded, final WSOL delta verified >= repay + profit
├── Instruction 7: Kamino::flash_repay_reserve_liquidity (borrow_instruction_index = 5)
│                  Kamino debits borrow_amount (d_star) + flash_fee from user's WSOL ATA
└── Instruction 8: SystemProgram::Transfer (Jito Tip, where applicable)
```

---

## 3. Meteora DLMM Swap2 CPI Specification

### 3.1 IDL & Instruction Matrix
- **Program ID**: `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`
- **Instruction**: `swap2`
- **Discriminator**: `[65, 75, 63, 76, 235, 91, 91, 136]` (`sha256("global:swap2")[..8]`)
- **Arguments**:
  - `amount_in: u64` (Little-Endian)
  - `min_amount_out: u64` (Little-Endian)
  - `remaining_accounts_info: RemainingAccountsInfo` (Borsh-encoded vector of `RemainingAccountsSlice`)
- **Accounts Required (16 Fixed Accounts + Dynamic Bin Arrays)**:
  1. `lb_pair` [writable]
  2. `bin_array_bitmap_extension` [writable, optional -> sentinel if absent]
  3. `reserve_x` [writable]
  4. `reserve_y` [writable]
  5. `user_token_in` [writable]
  6. `user_token_out` [writable]
  7. `token_x_mint` [readonly]
  8. `token_y_mint` [readonly]
  9. `oracle` [writable]
  10. `host_fee_in` [writable, optional -> sentinel if absent]
  11. `user` [signer]
  12. `token_x_program` [readonly]
  13. `token_y_program` [readonly]
  14. `memo_program` [readonly: `MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr`]
  15. `event_authority` [readonly PDA: `["__event_authority"]`]
  16. `program` [readonly self-reference: `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`]
  17..: `bin_arrays` [writable, dynamic count in traversal order]

### 3.2 User Authority & CPI Signer Forwarding
Because the user's token account is debited by Meteora DLMM, `user` must sign the swap instruction.
In Solana CPI:
- The outer transaction is signed by the user (`payer`).
- When `ArbExecutor` executes `solana_cpi::invoke` (or `solana_program::program::invoke`), Solana runtime automatically preserves the signer privilege for `user`.
- No PDA delegation or intermediate token custody transfer to the executor is needed; all tokens remain securely in user-owned ATAs throughout execution.

---

## 4. Audit of Existing Codebase Flaws

### 4.1 Hop Input Contract Mismatch
Prior implementation across `three_leg_meme_pipeline.rs`, `run.rs`, and `builder.rs` suffered from inconsistent hop input semantics:
1. `three_leg_meme_pipeline.rs`: In `quote_chain`, leg 1 input was derived as `in2 = slippage_floor(out1)`.
2. `run.rs`: Assembled `hop_inputs = [in1, in2, in3, out3]`.
3. `builder.rs`: In `derive_swap_legs`, expected `hop_inputs` to contain *optimistic expected outputs*, and applied slippage a **second time**:
   $$\text{min\_out} = \text{in2} - (\text{in2} \times \text{slippage\_bps}) / 10000$$
   This double-discounted slippage, leading to severely distorted on-chain parameters.
4. **Remediation**: Establish a canonical `HopExecutionPlan` contract where:
   - For every hop $i \in \{0, 1, \dots, n-2\}$:
     $$\text{previous\_hop.min\_amount\_out} \ge \text{next\_hop.amount\_in}$$
   - Slippage is applied exactly once per hop boundary.

### 4.2 Priority Fee Double-Pricing Bug
1. `crates/arb-bot/src/supervisor/opportunity.rs`: Computed `cu_price` via `arb_tx::cap_cu_price_by_profit(...)`.
2. `crates/arb-tx/src/primitives/builder.rs`: Re-derived priority fee by calling `calculate_dynamic_cu_price(profit, priority_fee_micro_lamports)`, which added an unmodeled elevation:
   $$\text{elevation} = \min\left(\frac{\text{profit}}{25000}, 500000\right)$$
3. This resulted in transactions paying higher priority fees than accounted for in the profitability model, eroding net profits.
4. **Remediation**: The final compute budget price injected into the transaction MUST equal the price used in the authoritative profitability evaluation. Checked arithmetic with `div_ceil` must be used for lamport conversions.

### 4.3 Leg 3 WSOL Output Delta vs Initial Balance Defect
1. **Defect**: Early implementations calculated `final_wsol_delta = wsol_final - wsol_initial` and compared it against `leg3_min_amount_out` and `min_final_wsol_delta` (the repayment amount).
   Because `wsol_initial` held the flash loan principal which was debited in Leg 1, `wsol_final - wsol_initial` represents only the net profit (e.g., 0.05 SOL), whereas `leg3_min_amount_out` and `min_final_wsol_delta` represent the gross repayment scale (e.g., 10.04 SOL). This resulted in 100% false-positive runtime aborts.
2. **Remediation**:
   - Record `wsol_pre_leg3` immediately prior to Leg 3 swap execution.
   - Leg 3 received output is strictly `leg3_delta = wsol_final - wsol_pre_leg3`.
   - Ensure `leg3_delta >= leg3_min_amount_out` and `leg3_delta >= min_final_wsol_delta`.
   - Prevent pre-existing WSOL inventory subsidy by verifying `wsol_final >= wsol_baseline + min_final_wsol_delta`, where `wsol_baseline = wsol_initial - leg1_amount_in`.

### 4.4 CPI Optional Account Sentinel Privilege Handling
1. **Defect**: In Meteora DLMM `swap2`, absent optional accounts (`bin_array_bitmap_extension` and `host_fee_in`) use the DLMM program ID sentinel. Flagging an executable program account as writable in CPI triggers an immediate Solana runtime rejection (`An instruction attempted to borrow an account as writable that was not passed as writable`).
2. **Remediation**: The CPI builder dynamically inspects `is_writable` and sentinel address matching, emitting `AccountMeta::new_readonly` whenever the sentinel or an un-granted account is passed.

### 4.5 User Token Account Authority Verification
1. **Defect**: Validating only token data length allowed passing arbitrary or unverified token accounts.
2. **Remediation**: Added `validate_user_token_account` ensuring `account.owner` is SPL Token or Token-2022 and `data[32..64] == user.key`.

---

## 5. Security & Risk Gates Baseline

All existing fail-closed risk controls remain active and non-negotiable:
- `ARB_LIVE=0` / `DRY_RUN=true`
- Unverifiable flash liquidity fails closed (`kamino_available_liquidity = None` -> skip)
- Pre-existing intermediate inventory subsidization prevention (zero balance gate)
- Atomic transaction integrity validation (`verify_atomic_integrity`) before transaction signing
- MTU packet limits: 1,232 bytes (Legacy / V0 unversioned), 4,096 bytes (Solana Transaction V1)
