# Economic Model & Unit Conversion Forensic Audit

## 1. Executive Summary & Defect Investigation

### 1.1 The Critical $0.20 USD vs 50,000 Lamports Defect
In earlier iterations of the codebase, a comment in `crates/arb-bot/src/supervisor/opportunity.rs` stated:
```rust
// Strict Decoupled Profit Protection Invariant:
// P_net = G - C_fees - T_Jito >= $0.20 USD
// T_max = G - C_fees - $0.20 - B
let min_net_floor = self.config.strategy.min_net_profit_abs; // 50,000
```
This conflated a **$0.20 USD** profit target with **50,000 lamports**.
Under actual market conditions:
- If SOL = $200 USD:
  $$50,000\text{ lamports} = \frac{50,000}{1,000,000,000} \times \$200 = 0.00005 \times \$200 = \$0.01\text{ USD}$$
- A true **$0.20 USD** profit floor at SOL = $200 requires:
  $$\frac{\$0.20}{\$200} \times 10^9 = 0.001\text{ SOL} = 1,000,000\text{ lamports}$$
Treating 50,000 lamports as equivalent to $0.20 was a **20x error** in economic scale. At $0.01 profit, high-frequency transactions could easily be rendered unprofitable by minor off-quote slippage, sub-optimal execution timing, or transient price volatility.

### 1.2 Remediated Conversion Strategy
1. **Dynamic Lamport Floor Conversion (`min_net_profit_floor_lamports`)**:
   - `crates/arb-bot/src/config.rs` provides `min_net_profit_floor_lamports(toml_floor)`, which dynamically reads `MIN_NET_PROFIT_USD` and `SOL_USD_PRICE` from the environment:
     $$\text{effective\_floor} = \max\left(\text{toml\_floor}, \left\lfloor \frac{\text{MIN\_NET\_PROFIT\_USD}}{\text{SOL\_USD\_PRICE}} \times 10^9 \right\rceil\right)$$
   - Previously, this function was bypassed when `arb_config::loader::load` succeeded. In our remediation, `crates/arb-bot/src/config.rs:424` now explicitly routes through `min_net_profit_floor_lamports(arb.strategy.min_net_profit_abs.0)`, guaranteeing that USD overrides immediately elevate the absolute floor to true dollar parity.
2. **Strict Dimensional Typing**:
   - Cross-currency arithmetic is guarded at compile and runtime using `TokenAmount`, `ProfitAmount`, `Lamports`, and `UsdMicros`.
   - Conversions between SOL lamports and other mints (e.g. USDC, 6 decimals) must provide a rational exchange rate `rate_num / rate_denom` and pass a slot-staleness check (`current_slot - price_slot <= max_age_slots`).
   - If price data is missing or stale, the system **fails closed** (`DimensionalError::StalePrice` / `DimensionalError::MissingConversion`).

---

## 2. Authoritative Cost & Profit Calculation

The canonical economic quote model is defined centrally in `crates/arb-math/src/economic.rs` and consumed across discovery, candidate ranking, route sizing, simulation, bidding, and execution.

### 2.1 The Mandatory Profitability Invariant
Before any transaction can be compiled or broadcast, it must satisfy:
$$\text{FINAL\_NET\_PROFIT} \ge \text{CONFIGURED\_MIN\_NET\_PROFIT}$$

Where:
$$\text{FINAL\_NET\_PROFIT} = \text{REALIZED\_MIN\_OUTPUT} - \text{BORROWED\_PRINCIPAL} - \text{FLASH\_LOAN\_FEE} - \text{BASE\_TX\_FEE} - \text{PRIORITY\_FEE} - \text{JITO\_TIP} - \text{OTHER\_VERIFIED\_COSTS}$$

### 2.2 Component Breakdown
1. **Flash Loan Fee (`Kamino` U68F60 fixed-point)**:
   - On-chain reserve fee parameter `sf` has 60 fractional bits (`fixed::types::U68F60`).
   - Exact formula:
     $$\text{fee} = \left\lfloor \frac{\text{borrow} \times \text{sf} + 2^{59}}{2^{60}} \right\rfloor$$
     with a strict 1-unit minimum floor. Overflow or uncomputable fees immediately fail closed.
2. **Base Transaction Fee**:
   - 5,000 lamports per signature (`DEFAULT_BASE_FEE_LAMPORTS`), charged unconditionally by Solana runtime.
3. **Priority Fee**:
   - For V0: $\lceil \text{cu\_price\_microlamports} \times \text{requested\_cu\_limit} / 10^6 \rceil$.
   - For V1: Exact absolute lamport value encoded in the transaction message header.
4. **Jito Tip**:
   - Explicitly planned by the Adaptive Bidder based on real market floor, hurdle, and ceiling.
   - Never an arbitrary percentage of raw edge.
5. **No Double Counting**:
   - `CostLedger` tracks `included_in_quote` and `deducted_already` for every cost item (DEX swap fees, transfer fees, account creation rent). If an AMM swap quote already factors in a 25 bps liquidity provider fee, it is not deducted a second time from net profit.
