# Quant, Math & Route Quoting Verification Report
**Module Focus:** `crates/arb-math`, `crates/arb-dex`, `crates/arb-engine`  
**Auditor:** Quant Protocol Lead / Supervisor  

---

## 1. Integer-Only Settlement Math Verification

### Ground Truth in Code (`crates/arb-math/src/economic.rs`)
The codebase explicitly mandates:
> *"No floating-point arithmetic is used for the final executable amount or the profit guard."*

- **Monetary Units:** Strictly typed as `Lamports(u64)` and `TokenAmount(u64)`.
- **Large Intermediate Products:** Handled using `ruint::aliases::U256`, `U384`, and `U512` to eliminate intermediate multiplication overflows.
- **Rounding Direction:**
  - **Repayment / In-costs:** Always rounded **UP** ($\lceil \cdot \rceil$) to guarantee that obligations are never under-calculated.
  - **Outputs / Swap returns:** Always rounded **DOWN** ($\lfloor \cdot \rfloor$) to ensure that conservative, executable quantities reach on-chain limits.

---

## 2. Meteora DLMM Bin-Walking Algorithm (`crates/arb-math/src/dlmm.rs`)

### Comparison with Official Meteora DLMM SDK
Unlike traditional constant-product pools ($x \cdot y = k$), Meteora DLMM calculates price and liquidity across discrete price bins:
$$P_i = (1 + \text{bin\_step})^i$$

In `crates/arb-math/src/dlmm.rs::quote_dlmm_multi_bin`:
1. **Active Bin Traversal:** The engine iterates across bins starting from `pool.active_id`.
2. **Layer Consumption:**
   - Layer 1: Market Maker reserve liquidity (`bin.amount_x` or `bin.amount_y`).
   - Layer 2: Processed limit order liquidity.
   - Layer 3: Open limit order liquidity.
3. **Volatility Accumulator & Dynamic Fees:**
   - Computes variable fee rates dynamically using `update_volatility_accumulator(&mut v_params, &pool.parameters, current_bin_id)`.
   - Incorporates `base_fee_rate` + `variable_fee_rate`.
4. **Bin Crossing Limits:** Automatically fails closed if required adjacent `BinArray` accounts are not loaded in the snapshot, preventing transaction reverts on-chain.

---

## 3. Kamino Flash Loan Repayment Math (`crates/arb-math/src/economic.rs`)

### Fixed-Point Kamino Scaling Factor (U68F60)
Kamino Lending encodes reserve flash loan fees as a 128-bit unsigned integer with 60 fractional bits (`U68F60`).
In `calculate_flash_fee_raw(amount_lamports, fee_sf)`:
```rust
pub fn calculate_flash_fee_raw(amount_lamports: u64, fee_sf: u128) -> u64 {
    if fee_sf == 0 {
        return 1;
    }
    let fee = ((amount_lamports as u128 * fee_sf) >> 60) as u64;
    fee.max(1)
}
```
- **Kamino Standard Fee:** `11_529_215_046_068` (corresponds to $0.1\text{ bps}$ or $0.001\%$).
- **Minimum Fee:** Always clamped to at least $1\text{ lamport}$, matching Kamino's on-chain smart contract rule.

---

## 4. Pump.fun Graduation & Boost Migration Math (`pump_boost_arb.rs`)

The bonding curve parameters in `crates/arb-math/src/pump_boost_arb.rs` match live Solana mainnet contracts:
- **Target Lamports for Graduation:** $85,000,000,000\text{ lamports}$ ($85\text{ SOL}$).
- **Protocol Boost Allocation:** $\approx 17.58\text{ SOL}$ TWAP buyback injected during pool migration.
- **Initial Virtual Reserves:**
  $$V_{\text{SOL}} = 30\text{ SOL}, \quad V_{\text{TOK}} = 1,073,000,000\text{ tokens}$$
  $$K_{\text{curve}} = V_{\text{SOL}} \times V_{\text{TOK}} = 3.219 \times 10^{19}$$

---

## 5. Optimal Sizing Analysis

In `crates/arb-engine/src/flash_loan_sizing.rs`:
- **Kelly Criterion Integration:** Calculates optimal capital fraction $f^* = p - \frac{1-p}{R}$ with half-Kelly scaling ($0.5$).
- **Bounding Constraints:** Clamped strictly within:
  $$\text{min\_position\_sol} \le \text{Trade Size} \le \min(\text{max\_position\_sol}, \text{Available Kamino Vault Liquidity})$$
- Prevents borrowing more than what the lending vault holds, preventing instant `InsufficientLiquidity` aborts.
