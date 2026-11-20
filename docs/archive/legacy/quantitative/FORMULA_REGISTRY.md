> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# QUANTITATIVE REGISTRY — FORMULA REGISTRY

This registry records every mathematical formula across the Sentinel arbitrage system, detailing integer representation, rounding rules, overflow guarantees, and test coverage.

---

## 1. Constant-Product AMM (CPMM) — Exact Input

- **Formula Name:** `cpmm_out_fee` / `cpmm_quote_exact_in`
- **Mathematical Expression:**
  $$\Delta x_{\text{net}} = \left\lfloor \frac{\Delta x \times (D - F)}{D} \right\rfloor$$
  $$\Delta y = \left\lfloor \frac{R_y \times \Delta x_{\text{net}}}{R_x + \Delta x_{\text{net}}} \right\rfloor$$
- **Code Location:** `crates/arb-math/src/cpmm.rs:25`
- **Input Variables:** $\Delta x$ (amount in), $R_x$ (reserve in), $R_y$ (reserve out), $F$ (fee numerator), $D$ (fee denominator, e.g. 10,000 or 1,000,000)
- **Output Variables:** $\Delta y$ (amount out)
- **Units:** Atomic token units (base/quote decimals)
- **Decimal Precision:** Exact integer arithmetic (no decimals)
- **Integer Representation:** `u64` / `u128` / `ruint::aliases::U256`
- **Rounding Method:** Truncation toward zero (floor division, conservative for trader)
- **Overflow Behavior:** Uses checked arithmetic (`checked_mul`, `checked_add`); returns `None` on overflow
- **Fee Treatment:** Fee is deducted from input amount before computing output
- **Source of Formula:** Uniswap V2 / Raydium CPMM canonical equation
- **Test Coverage:** `tests/fuzzing_economic_decimals.rs`, `tests/cpmm_real_bytes_fixture.rs`
- **Known Limitations:** Inaccurate for concentrated liquidity or multi-bin curves

---

## 2. Constant-Product AMM (CPMM) — Exact Output

- **Formula Name:** `cpmm_in_exact_out`
- **Mathematical Expression:**
  $$\Delta x_{\text{raw}} = \left\lceil \frac{R_x \times \Delta y}{R_y - \Delta y} \right\rceil$$
  $$\Delta x = \left\lceil \frac{\Delta x_{\text{raw}} \times D}{D - F} \right\rceil$$
- **Code Location:** `crates/arb-math/src/cpmm.rs:95`
- **Input Variables:** $\Delta y$ (desired amount out), $R_x$ (reserve in), $R_y$ (reserve out), $F$ (fee), $D$ (denominator)
- **Output Variables:** $\Delta x$ (required amount in)
- **Units:** Atomic token units
- **Decimal Precision:** Exact integer arithmetic
- **Integer Representation:** `u128` / `U256`
- **Rounding Method:** Ceiling division (`(num + denom - 1) / denom`) ensuring sufficient input
- **Overflow Behavior:** Checked operations; returns `None` if $\Delta y \ge R_y$
- **Fee Treatment:** Grosses up required input by fee rate
- **Source of Formula:** Uniswap V2 exactOutput specification
- **Test Coverage:** `crates/arb-math/src/cpmm.rs:tests`
- **Known Limitations:** Cannot supply output greater than or equal to pool reserve

---

## 3. Concentrated Liquidity (CLMM) — Step Output

- **Formula Name:** `clmm_compute_swap_step`
- **Mathematical Expression:**
  $$\Delta \sqrt{P} = \sqrt{P}_{\text{target}} - \sqrt{P}$$
  $$\Delta y = \lfloor L \times (\sqrt{P}_{\text{upper}} - \sqrt{P}_{\text{lower}}) \rfloor$$
  $$\Delta x = \left\lfloor \frac{L \times 2^{64} \times (\sqrt{P}_{\text{upper}} - \sqrt{P}_{\text{lower}})}{\sqrt{P}_{\text{upper}} \times \sqrt{P}_{\text{lower}}} \right\rfloor$$
- **Code Location:** `crates/arb-math/src/clmm.rs:180`
- **Input Variables:** Current $\sqrt{P}$, Target $\sqrt{P}$, Active Liquidity $L$, Fee rate
- **Output Variables:** $\Delta x$ consumed, $\Delta y$ produced, Next $\sqrt{P}$
- **Units:** Q64.64 fixed point for $\sqrt{P}$, atomic token units for amounts
- **Decimal Precision:** 64-bit fractional precision
- **Integer Representation:** `u128` / `U256` / `U384`
- **Rounding Method:** Truncate output amounts; round up input required
- **Overflow Behavior:** `ruint::aliases::U256` intermediate products
- **Fee Treatment:** Fee deducted from input amount at each tick step
- **Source of Formula:** Uniswap V3 Whitepaper / Orca Whirlpool core
- **Test Coverage:** `crates/arb-math/tests/differential.rs`, `p0_golden_vectors.rs`
- **Known Limitations:** Requires pre-loaded tick arrays; cannot extrapolate beyond loaded ticks

---

## 4. Discrete Liquidity (DLMM) — Bin Price & Output

- **Formula Name:** `get_price_from_id` & `quote_dlmm_bin_step`
- **Mathematical Expression:**
  $$P(\text{bin\_id}) = \left(1 + \frac{\text{bin\_step}}{10000}\right)^{\text{bin\_id}} \times 2^{64}$$
  $$\Delta y = \left\lfloor \Delta x_{\text{net}} \times P(\text{bin\_id}) \times 2^{-64} \right\rfloor$$
- **Code Location:** `crates/arb-math/src/dlmm.rs:88`
- **Input Variables:** `bin_id` ($i32$), `bin_step` ($u16$), `amount_in` ($u64$), bin reserves
- **Output Variables:** `amount_out` ($u64$), consumed input, remaining input
- **Units:** Q64.64 price, atomic token units
- **Decimal Precision:** Exact Q64.64 fixed-point integer
- **Integer Representation:** `ruint::aliases::U256` / `U512`
- **Rounding Method:** Floor division
- **Overflow Behavior:** Checked math with bin boundary assertion
- **Fee Treatment:** Base fee + dynamic volatility fee
- **Source of Formula:** Trader Joe DLMM Whitepaper / Meteora DLMM SDK
- **Test Coverage:** `tests/dlmm_bin_array_boundary_tests.rs`, `tests/meteora_cross_pool_vertical_slice.rs`
- **Known Limitations:** Bin arrays must be loaded in advance

---

## 5. Kamino Flash Loan Fee

- **Formula Name:** `calculate_flash_fee_raw`
- **Mathematical Expression:**
  $$\text{fee} = \max\left(1, \left\lfloor \frac{\text{amount} \times sf}{2^{60}} \right\rfloor\right)$$
- **Code Location:** `crates/arb-math/src/economic.rs:188`
- **Input Variables:** `amount` ($u64$), `sf` ($u128$, Kamino fee scaling factor in Q68.60)
- **Output Variables:** `fee` ($u64$)
- **Units:** Lamports or atomic token units
- **Decimal Precision:** 60-bit fractional precision
- **Integer Representation:** `u128`
- **Rounding Method:** Floor division with 1-lamport floor
- **Overflow Behavior:** `(amount as u128 * sf) >> 60`, impossible to overflow `u128`
- **Fee Treatment:** Represents protocol borrow fee added to repayment obligation
- **Source of Formula:** Kamino Lending (`klend`) smart contract
- **Test Coverage:** `crates/arb-tx/tests/v1_property_fuzz.rs:kamino_fee_differential`
- **Known Limitations:** Assumes non-zero fee factor; $sf=0$ yields minimum 1 lamport

---

## 6. Optimal Trade Sizing (Two-Pool Closed Form)

- **Formula Name:** `optimal_trade_size_two_pool`
- **Mathematical Expression:**
  $$x^* = \left\lfloor \frac{\sqrt{R_{x1} R_{y1} R_{x2} R_{y2} (1 - f_1)(1 - f_2)} - R_{x1} R_{y2}}{R_{y2} + R_{y1}(1 - f_1)} \right\rfloor$$
- **Code Location:** `crates/arb-math/src/cpmm.rs:320`
- **Input Variables:** Reserves $R_{x1}, R_{y1}, R_{x2}, R_{y2}$, fees $f_1, f_2$
- **Output Variables:** $x^*$ (optimal borrow amount in lamports)
- **Units:** Lamports
- **Decimal Precision:** Integer square root
- **Integer Representation:** `ruint::aliases::U256` / `U512`
- **Rounding Method:** Floor division; returns 0 if numerator $\le 0$
- **Overflow Behavior:** `isqrt` uses checked integer square root algorithm
- **Fee Treatment:** Full two-leg fee factored into derivative roots
- **Source of Formula:** Convex optimization of cross-pool AMM arbitrage
- **Test Coverage:** `crates/arb-math/tests/worked_example.rs`
- **Known Limitations:** Valid for two constant-product pools; multi-bin or CLMM pools use ladder search

---

## 7. Canonical Net Profit Evaluation

- **Formula Name:** `calculate_net_profit`
- **Mathematical Expression:**
  $$\text{NetProfit} = \text{AmountOut}_{\text{final}} - \text{Borrowed} - \text{FlashLoanFee} - \text{TxFee} - \text{Tip}$$
- **Code Location:** `crates/arb-math/src/economic.rs:45`
- **Input Variables:** Final output, borrowed amount, flash loan fee, CU priority fee, Jito tip
- **Output Variables:** `NetProfit` ($i64$)
- **Units:** Signed lamports ($i64$)
- **Decimal Precision:** Exact lamports
- **Integer Representation:** `i64` / `u64`
- **Rounding Method:** Exact subtraction
- **Overflow Behavior:** Checked integer arithmetic; negative result represents loss
- **Fee Treatment:** Net of ALL mandatory costs
- **Source of Formula:** Canonical quantitative accounting rule
- **Test Coverage:** `tests/three_leg_net_economics_tests.rs`, `crates/arb-risk/src/lib.rs`
- **Known Limitations:** None; single source of truth across all crates
