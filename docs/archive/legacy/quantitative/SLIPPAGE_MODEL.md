> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# QUANTITATIVE MODEL — SLIPPAGE & CONSERVATIVE BOUNDS

**Author:** Quantitative Financial Auditor  
**Date:** 2026-11-19  
**Status:** CANONICAL & VERIFIED  

---

## 1. Multi-Hop Slippage Compounding

In multi-hop atomic flash loans, slippage must be bounded conservatively at each intermediate swap hop to prevent the transaction from reverting or producing negative settlement profit:

Let a route consist of $N$ hops ($H_1, H_2, \dots, H_N$):
- Hop 1: Input $x_0 \to \text{Expected Out } y_1$
- Hop 2: Input $y_1 \to \text{Expected Out } y_2$
- Hop $N$: Input $y_{N-1} \to \text{Expected Out } x_{\text{final}}$

### 1.1 Hop Minimum Out Protection (`min_out`)
At each hop $i$, an explicit on-chain minimum output constraint is encoded in the instruction:
$$\text{min\_out}_i = \left\lfloor y_i \times \frac{10000 - \text{slippage\_bps}_i}{10000} \right\rfloor$$

### 1.2 Intermediate Balance Clamping
- The input to Hop $i+1$ is strictly clamped to $\text{min\_out}_i$.
- If Hop $i$ experiences maximum permissible slippage and delivers exactly $\text{min\_out}_i$, Hop $i+1$ has sufficient funds to execute without failing.
- The final hop's $\text{min\_out}_N$ must satisfy:
  $$\text{min\_out}_N \ge \text{BorrowedAmount} + \text{FlashLoanFee} + \text{PriorityFee} + \text{Tip} + \text{TargetMinProfit}$$
If this inequality is not satisfied under maximum adverse slippage, the transaction is **rejected pre-flight**.

---

## 2. Distinction: Price Impact vs. Slippage

| Metric | Cause | When Applied? | Deterministic? |
|---|---|---|---|
| **Price Impact** | Trade size moving down the pool's liquidity curve | Calculated during quoting | **Deterministic:** Known exactly from order size and pool reserves |
| **Slippage** | State changes between snapshot observation and transaction landing (e.g. concurrent user trades) | Bounded by `min_out` | **Stochastic:** Protected by on-chain assertion |

---

## 3. Directional Tick & Bin Boundary Traversal

In concentrated (CLMM) and discretized (DLMM) liquidity, price impact accelerates rapidly when crossing bin or tick boundaries:
1. **Thin Liquidity Arrays:** If adjacent tick arrays or bin arrays are uninitialized, price impact jumps to 100% (liquidity gap).
2. **Boundary Gate:** The quoting engine checks that sufficient initialized bins/ticks exist in the trade direction to cover the entire simulated trade size.
3. If the trade would exhaust all loaded bin arrays, it fails closed with `ERR_BIN_ARRAY_EXHAUSTED`.
