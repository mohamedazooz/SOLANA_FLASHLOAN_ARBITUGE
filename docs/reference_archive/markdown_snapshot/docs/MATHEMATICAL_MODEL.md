# MATHEMATICAL MODEL & QUANTITATIVE DERIVATIONS

## 1. Zero Floating-Point Rule

All financial calculations, sizing algorithms, routing decisions, fee deductions, and profit assertions are strictly computed using **unsigned fixed-point integer arithmetic** (`u64`, `u128`, `U256`). Floating-point arithmetic (`f32`, `f64`) is prohibited on all execution paths due to IEEE-754 precision loss, indeterminism, and dust rounding exploits.

---

## 2. Multi-Hop Constant-Product & Concentrated Liquidity Routing

For a 3-hop cyclic arbitrage cycle starting and ending with WSOL:
$$\text{WSOL} \xrightarrow{\text{Pool } 1} \text{Token } A \xrightarrow{\text{Pool } 2} \text{Token } B \xrightarrow{\text{Pool } 3} \text{WSOL}$$

Let $\Delta x$ be the flash-borrowed amount of WSOL.
Each pool $i \in \{1, 2, 3\}$ has a state-dependent transfer function:
$$\Delta y_1 = f_1(\Delta x, S_1)$$
$$\Delta y_2 = f_2(\Delta y_1, S_2)$$
$$\Delta x' = f_3(\Delta y_2, S_3)$$

Where $S_i$ represents pool $i$'s reserves, active bins, dynamic fee rate $\gamma_i = (1 - \text{fee}_i)$, and liquidity distribution.

### In concentrated liquidity (DLMM bins):
For each bin $b$ with price $P_b$ and available liquidity $L_b$:
$$\Delta y = \min\left(L_b, \Delta x \cdot P_b \cdot \gamma_i\right)$$
$$\Delta x_{\text{consumed}} = \frac{\Delta y}{P_b \cdot \gamma_i}$$
If $\Delta x > \Delta x_{\text{consumed}}$, the remaining $\Delta x$ crosses into bin $b \pm 1$.

---

## 3. Net Profit Analytical Formulation

The net arbitrage profit $\Pi(\Delta x)$ in base token (WSOL lamports) is given by:

$$\Pi(\Delta x) = \Delta x' - \Delta x - \text{Fee}_{\text{flash}}(\Delta x) - \text{Cost}_{\text{network}}$$

Where:
1. **Flash Loan Fee**:
   $$\text{Fee}_{\text{flash}}(\Delta x) = \left\lfloor \frac{\max(\Delta x \cdot F_{sf}, 2^{60}) + 2^{59}}{2^{60}} \right\rfloor$$
2. **Network Costs**:
   $$\text{Cost}_{\text{network}} = \text{Fee}_{\text{base}} + \left\lceil \frac{\text{CU}_{\text{limit}} \times \text{CU}_{\text{price}}}{10^6} \right\rceil + \text{Tip}_{\text{Jito}}$$
   Where:
   - $\text{Fee}_{\text{base}} = 5,000 \text{ lamports}$ (per signature)
   - Priority fee is rounded up using ceiling division to prevent under-tipping validators:
     $$\text{Priority Fee Lamports} = \left\lfloor \frac{\text{CU}_{\text{limit}} \times \text{CU}_{\text{price}} + 999,999}{1,000,000} \right\rfloor$$
   - $\text{Tip}_{\text{Jito}} \ge \text{Dynamic Floor}$ (only paid if transaction lands in a bundle).

---

## 4. Optimal Sizing Optimization ($\Delta x^*$)

To find the trade size $\Delta x^*$ that maximizes $\Pi(\Delta x)$:
$$\frac{d\Pi}{d\Delta x} = 0 \iff \frac{d\Delta x'}{d\Delta x} = 1 + \frac{d\text{Fee}_{\text{flash}}}{d\Delta x}$$

In discrete bin/orderbook models, $\Pi(\Delta x)$ is piecewise concave. We solve for $\Delta x^*$ using a bounded ternary search / golden-section search over integer space:
$$\Delta x \in [x_{\min}, x_{\max}]$$
Where $x_{\max} = \min(\text{VaultAvailableLiquidity}, \text{Pool1Depth}, \text{RiskMaxLimit})$.

Each iteration evaluates integer quotes $f(\Delta x)$ without floating-point approximations. The optimal size $\Delta x^*$ must strictly satisfy:
$$\Pi(\Delta x^*) \ge \text{MIN\_NET\_PROFIT\_RAW}$$
