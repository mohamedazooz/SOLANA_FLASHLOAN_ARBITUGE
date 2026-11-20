> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# QUANTITATIVE MODEL — OPTIMAL TRADE SIZING

**Author:** Quantitative Financial Auditor & Solana HFT Engineer  
**Date:** 2026-11-19  
**Status:** CANONICAL & VERIFIED  

---

## 1. Problem Formulation

In multi-pool arbitrage, trade profit is a concave function of input size $x$:
- Small $x$: Negligible price impact, but fixed transaction fees (priority fee, tip, base fee) dominate, resulting in negative net profit.
- Large $x$: Fixed fees are diluted, but non-linear price impact across the DEX pools destroys the price spread, causing the marginal return to drop below zero.
- **Optimal Size $x^*$:** The point where marginal gross revenue equals marginal slippage cost plus marginal borrow fee.

```text
Net Profit
    ^
    |             * Peak Profit (x*)
    |           *   *
    |          *     *
    |         *       *
0 --|--------*---------*----------> Trade Size x
    |       *           *
    |      *             * (Slippage Destruction)
    |     * (Fixed Fee
    |        Dominance)
```

---

## 2. Analytical Closed-Form Solution (Two-Pool CPMM)

For a two-hop cycle between Pool 1 (Buy Token $Y$ with $X$) and Pool 2 (Sell Token $Y$ for $X$):
Let:
- Pool 1: Input reserve $R_{x1}$, Output reserve $R_{y1}$, Fee factor $\gamma_1 = 1 - f_1$
- Pool 2: Input reserve $R_{y2}$, Output reserve $R_{x2}$, Fee factor $\gamma_2 = 1 - f_2$

The total output $y$ from Pool 1 for input $x$ is:
$$y(x) = \frac{R_{y1} \gamma_1 x}{R_{x1} + \gamma_1 x}$$

Feeding $y(x)$ into Pool 2 yields output $x_{\text{out}}$:
$$x_{\text{out}}(x) = \frac{R_{x2} \gamma_2 y(x)}{R_{y2} + \gamma_2 y(x)}$$

Gross profit is:
$$G(x) = x_{\text{out}}(x) - x$$

Setting the derivative $\frac{dG}{dx} = 0$ and solving yields the exact optimal input $x^*$:
$$x^* = \left\lfloor \frac{\sqrt{R_{x1} R_{y1} R_{x2} R_{y2} \gamma_1 \gamma_2} - R_{x1} R_{y2}}{R_{y2} + \gamma_1 R_{y1}} \right\rfloor$$

### Invariant Checks:
1. If $\sqrt{R_{x1} R_{y1} R_{x2} R_{y2} \gamma_1 \gamma_2} \le R_{x1} R_{y2}$, the spread is insufficient to overcome fees; $x^* = 0$ (no arbitrage exists).
2. Computed in checked integer arithmetic using `isqrt` from `arb-math`.

---

## 3. Numerical Ladder Optimization (CLMM & DLMM Multi-Hop)

Because Concentrated Liquidity (CLMM) and Discretized Liquidity (DLMM) have **piece-wise discontinuous liquidity profiles** (tick boundaries and discrete price bins), closed-form derivatives do not exist across bin boundaries.

The engine employs a **multi-rung golden search ladder**:
1. **Search Interval:** Evaluates sizing candidates across a geometric progression:
   $$\{0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 20.0, 35.0, 50.0\}\text{ SOL}$$
2. **Exact Quoting:** Each rung is simulated through the exact multi-bin quoter (`quote_chain_inputs`).
3. **Local Search:** Once the peak interval $[x_a, x_b]$ is identified, a ternary search refines the optimum to within 50,000 lamports.

---

## 4. Safety Gates & Clamping Constraints

The computed optimal size $x^*$ must satisfy all pre-trade constraints:

1. **Borrow Ceiling:**
   $$x^* = \min(x^*, \text{MAX\_BORROW\_LAMPORTS})$$
2. **Available Pool Liquidity:**
   $x^*$ must not deplete more than $15\%$ of any pool's active reserves.
3. **Price Impact Threshold:**
   If implied price impact exceeds $\text{MAX\_PRICE\_IMPACT\_BPS}$ (default 150 bps / 1.5%), the trade size is clamped down.
4. **Minimum Net Profit Gate:**
   If $\text{NetProfit}(x^*) < \text{MIN\_NET\_PROFIT\_LAMPORTS}$, the opportunity is rejected without transaction submission.
