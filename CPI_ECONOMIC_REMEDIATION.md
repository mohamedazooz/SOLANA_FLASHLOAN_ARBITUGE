# CPI_ECONOMIC_REMEDIATION.md: Mathematical & Economic Remediation Specification

**Repository**: `https://github.com/mohamedazooz/SOLANA_FLASHLOAN_ARBITUGE`  
**Active Branch**: `feat/meteora-dlmm-swap2-atomic-cpi`  

---

## 1. Exact Profitability & Repayment Invariants

All accounting is strictly integer-based with explicit checked arithmetic (`checked_add`, `checked_sub`, `checked_mul`). Floating point (`f64`) arithmetic is strictly prohibited from authoritative decisions.

### 1.1 Gross & Net Profit Formulations
Given:
- $B$: Borrow amount (WSOL lamports)
- $W_{\text{out}}$: Final WSOL output produced by the 3-leg cycle
- $\text{Fee}_{\text{Kamino}}$: Flash loan fee deducted by Kamino klend
- $\text{Cost}_{\text{Base}}$: Solana base transaction fee (5,000 lamports)
- $\text{Cost}_{\text{Priority}}$: Micro-lamports priority fee converted via ceiling division
- $\text{Cost}_{\text{Tip}}$: Jito bundle tip lamports (dynamic or verified floor)
- $\text{Cost}_{\text{Rent}}$: Non-recoverable rent for newly initialized ATAs (if any)
- $\text{Buffer}_{\text{Contingency}}$: Risk contingency reserve

$$\text{GrossProfit} = W_{\text{out}} - B$$

$$\text{TotalCosts} = \text{Fee}_{\text{Kamino}} + \text{Cost}_{\text{Base}} + \text{Cost}_{\text{Priority}} + \text{Cost}_{\text{Tip}} + \text{Cost}_{\text{Rent}} + \text{Buffer}_{\text{Contingency}}$$

$$\text{NetProfit} = \text{GrossProfit} - \text{TotalCosts}$$

### 1.2 Fail-Closed Solvency Guard
An opportunity is dispatched **if and only if**:
$$\text{NetProfit} \ge \text{MIN\_NET\_PROFIT\_LAMPORTS}$$
If at any point during integer subtraction an underflow occurs ($\text{TotalCosts} > \text{GrossProfit}$), the evaluation rejects the opportunity immediately (`fail-closed`).

---

## 2. Hop Input Contract Remediation

### 2.1 The Critical Defect in Previous Implementations
In multi-hop decentralized routing, if Hop 1 expects an optimistic output $O_1$ and sets Hop 2 input $I_2 = O_1$, any slippage incurred during Hop 1 execution results in Hop 1 actual output $O'_1 < I_2$.
If the user account contains 0 pre-existing tokens, Hop 2 fails with **Insufficient Funds**.
If the user account contained pre-existing tokens from previous operations, Hop 2 executes by consuming the user's unallocated tokens, **subsidizing an unprofitable trade and hiding a fatal execution deficit**.

### 2.2 Canonical `HopExecutionPlan` Specification
We enforce the following invariant across `three_leg_meme_pipeline.rs`, `run.rs`, `builder.rs`, and on-chain `arb-executor`:

For a 3-leg route:
1. **Leg 1**:
   - $\text{amount\_in}^{(1)} = B$
   - $\text{min\_amount\_out}^{(1)} = \lfloor O_1 \times (10000 - s) / 10000 \rfloor$
2. **Leg 2**:
   - $\text{amount\_in}^{(2)} \le \text{min\_amount\_out}^{(1)}$ (Canonical: $\text{amount\_in}^{(2)} = \text{min\_amount\_out}^{(1)}$)
   - $\text{min\_amount\_out}^{(2)} = \lfloor O_2 \times (10000 - s) / 10000 \rfloor$
3. **Leg 3**:
   - $\text{amount\_in}^{(3)} \le \text{min\_amount\_out}^{(2)}$ (Canonical: $\text{amount\_in}^{(3)} = \text{min\_amount\_out}^{(2)}$)
   - $\text{min\_amount\_out}^{(3)} = \max\left(\lfloor O_3 \times (10000 - s) / 10000 \rfloor, B + \text{Fee}_{\text{Kamino}} + \text{TotalCosts} + \text{MinNetProfit}\right)$

### 2.3 Single Slippage Enforcement
Slippage tolerance ($s$, in basis points) is applied **exactly once** at the quote boundary when deriving $\text{min\_amount\_out}$. Downstream instruction builders must **never** re-apply slippage to an already discounted minimum output.

---

## 3. Priority Fee Calculation Alignment

### 3.1 Defect Elimination
- **Defect**: `crates/arb-bot/src/supervisor/opportunity.rs` calculated compute unit price using profit-capped dynamic bounds (`arb_tx::cap_cu_price_by_profit`), while `crates/arb-tx/src/primitives/builder.rs` separately re-computed a dynamic elevation using `calculate_dynamic_cu_price`, adding unbudgeted micro-lamports.
- **Remediation**: The instruction builder receives the authoritative evaluated compute unit price directly, ensuring:
  $$\text{Price}_{\text{Instruction}} \equiv \text{Price}_{\text{EconomicModel}}$$

### 3.2 Exact Lamport Conversion
$$\text{Cost}_{\text{Priority}} = \left\lceil \frac{\text{cu\_price} \times \text{cu\_limit}}{1{,}000{,}000} \right\rceil = \text{checked\_mul}(\text{cu\_price}, \text{cu\_limit}).\text{div\_ceil}(1{,}000{,}000)$$
Ceiling division guarantees non-refundable network fees are never underestimated.

---

## 4. On-Chain Sequential Balance Delta Verification

Inside `programs/arb-executor`:
1. **Leg 1**:
   $$\Delta A_1 = \text{meme\_a\_post1} - \text{meme\_a\_initial} \ge \text{leg1\_min\_amount\_out} \ge \text{leg2\_amount\_in}$$
2. **Leg 2**:
   $$\Delta B_2 = \text{meme\_b\_post2} - \text{meme\_b\_initial} \ge \text{leg2\_min\_amount\_out} \ge \text{leg3\_amount\_in}$$
   $$\text{meme\_a\_post2} \ge \text{meme\_a\_initial} \quad (\text{zero intermediate subsidy})$$
3. **Leg 3**:
   $$\Delta W_3 = \text{wsol\_final} - \text{wsol\_pre\_leg3} \ge \text{leg3\_min\_amount\_out} \ge \text{min\_final\_wsol\_delta}$$
   $$\text{meme\_b\_final} \ge \text{meme\_b\_initial} \quad (\text{zero intermediate subsidy})$$
4. **Pre-Existing WSOL Inventory Protection**:
   Let $\text{wsol\_baseline} = \text{wsol\_initial} - \text{leg1\_amount\_in}$.
   $$\text{wsol\_final} \ge \text{wsol\_baseline} + \text{min\_final\_wsol\_delta}$$
   Guarantees the trader's pre-existing WSOL balance is never drained to cover a repayment deficit.
