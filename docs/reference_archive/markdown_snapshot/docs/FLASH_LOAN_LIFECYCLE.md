# FLASH LOAN LIFECYCLE & KAMINO INTROSPECTION SPECIFICATION

## 1. Overview and Architecture

In the Solana runtime (SVM), Kamino Lending (`klend`) enforces atomic flash loan execution via instruction introspection over the `Sysvar1nstructions1111111111111111111111111` sysvar. 

A flash loan lifecycle consists of two mandatory anchors and intermediate trade legs executed within **a single Solana transaction**:
1. `klend::flash_borrow_reserve_liquidity` (Instruction Index $B$)
2. Intermediate token conversions / Arbitrage CPI legs
3. `klend::flash_repay_reserve_liquidity` (Instruction Index $R$, referencing $B$)

```
+----------------------------------------------------------------------------------------------------+
|                                    SOLANA TRANSACTION (ATOMIC)                                     |
|                                                                                                    |
|  [0] ComputeBudget::SetComputeUnitLimit(cu_limit)                                                   |
|  [1] ComputeBudget::SetComputeUnitPrice(cu_price)                                                   |
|  [2] AssociatedTokenAccount::CreateIdempotent(WSOL)                                                |
|  [3] AssociatedTokenAccount::CreateIdempotent(Intermediate Token A)                                |
|  [4] AssociatedTokenAccount::CreateIdempotent(Intermediate Token B)                                |
|  [5] klend::flash_borrow_reserve_liquidity (Amount = X WSOL)                                       |
|  [6] arb_executor::execute_three_leg (CPI Swaps: WSOL -> Token A -> Token B -> WSOL')               |
|  [7] klend::flash_repay_reserve_liquidity (Repay = X + Fee, Borrow_Instruction_Index = 5)           |
|  [8] SystemProgram::Transfer (Jito Builder Tip, Tail Placed)                                        |
+----------------------------------------------------------------------------------------------------+
```

---

## 2. Kamino Instruction Introspection & Sysvar Invariants

Kamino's `flash_repay_reserve_liquidity` does not track borrower state in account storage. Instead, it reads the current transaction's serialized instructions using the `Instructions` sysvar:

```rust
// Kamino Lending On-Chain Verification Logic (Conceptual):
let sysvar_account = &ctx.accounts.instruction_sysvar;
let borrow_ix = load_instruction_at_checked(borrow_instruction_index as usize, sysvar_account)?;

require_keys_eq!(borrow_ix.program_id, kamino_lending::ID, LendingError::InvalidProgramId);
require_eq!(borrow_ix.data[0..8], FLASH_BORROW_DISCRIMINATOR, LendingError::InvalidInstruction);
```

### Critical Introspection Rules:
1. **Top-Level Instruction Requirement**:
   Because `load_instruction_at_checked` reads top-level instructions from the transaction message, **`FlashBorrow` and `FlashRepay` MUST be top-level instructions**. If `FlashBorrow` is called via Cross-Program Invocation (CPI) from inside a custom smart contract, `SysvarInstructions` will not expose it as a top-level transaction instruction at the designated index, causing `FlashRepay` to fail with `InvalidFlashRepay` or `InvalidInstruction`.
2. **Borrow Index Stability**:
   The `borrow_instruction_index` field passed into `flash_repay_reserve_liquidity` must point exactly to the array offset of `flash_borrow_reserve_liquidity` in the top-level message. In the `arb-executor` architecture, `flash_borrow` is strictly placed at index `5`.
3. **Atomicity Guarantee**:
   If any instruction in the sequence fails (e.g. slippage exceeded in `arb-executor`, insufficient funds to repay principal + fee in `flash_repay`), the entire transaction aborts and reverts. No funds can leave the Kamino reserve unless repayment succeeds in the exact same transaction.

---

## 3. Repayment Mathematics and Precision

Kamino Lending charges a flash loan protocol fee defined by the reserve's `flash_loan_fee_sf` (expressed in scaled sub-fractions with $2^{60}$ representing 100%).

### Canonical Calculation (`arb-math/src/economic.rs`):
Given:
- $A$: Borrowed amount (in native lamports / smallest units)
- $F_{sf}$: Flash loan fee in sub-fractions ($2^{60} \approx 1.1529215 \times 10^{18}$)

$$\text{Fee Bits} = \max\left(A \times F_{sf}, 2^{60}\right)$$

Rounding ties away from zero:
$$\text{Fee} = \left\lfloor \frac{\text{Fee Bits} + 2^{59}}{2^{60}} \right\rfloor$$

$$\text{Total Repayment} = A + \text{Fee}$$

### Dust & Pre-Rounding Verification:
The system applies an on-chain guard against dust underflow:
$$\text{raw\_bits} < 2^{60} \implies \text{BorrowTooSmall}$$
This ensures that every flash borrow generates at least 1 smallest unit of protocol fee and prevents zero-fee exploit attempts.

---

## 4. Subsidization Protection & Balance Isolation

To guarantee that arbitrage transactions cannot be subsidized by pre-existing wallet balances or drained by malicious slippage:
1. **Pre-trade Balance Snapshotting**: `arb-executor` records the initial token balances of intermediate accounts $A$ and $B$, as well as the settlement token account (WSOL).
2. **Zero Residual Intermediate Dust**: At the end of `execute_three_leg`, intermediate balances are asserted to be $\le \text{Pre-trade Balance}$.
3. **Net Profit Invariant**:
   $$\Delta \text{WSOL} = \text{Post-trade Balance} - \text{Pre-trade Balance}$$
   $$\text{Assert}(\Delta \text{WSOL} \ge \text{Borrowed Amount} + \text{Flash Fee} + \text{Min Net Profit})$$
   If this inequality fails, the program immediately aborts with `ExecutionError::InsufficientRepaymentOrProfit`, triggering a complete transaction revert.
