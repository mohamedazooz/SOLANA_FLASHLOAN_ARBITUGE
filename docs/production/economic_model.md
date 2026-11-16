# Authoritative Economic Calculation Model

## 1. Single Source of Truth
The canonical economic calculations reside exclusively in `crates/arb-math/src/economic.rs` (Calculation Version 2). No other crate, adapter, or binary is permitted to evaluate net profit independently.

## 2. Mathematical Definition of Net Profit

Authoritative execution decisions require conservative calculation based on minimum guaranteed output (`minimum_final_amount_raw`):

$$\text{NetProfit}_{\text{raw}} = \text{Out}_{\text{min}} - \text{Principal} - \text{Fee}_{\text{flash}} - \sum \text{Fee}_{\text{DEX}} - \sum \text{Fee}_{\text{transfer}} - \text{Cost}_{\text{lamports\_in\_base}}$$

Where:
- $\text{Out}_{\text{min}}$: Output after maximum permissible slippage across all legs.
- $\text{Principal}$: Raw borrowed atomic token units.
- $\text{Fee}_{\text{flash}}$: Kamino flash loan fee derived from exact on-chain reserve config.
- $\text{Cost}_{\text{lamports\_in\_base}} = \left\lceil \frac{(\text{Fee}_{\text{base}} + \text{Fee}_{\text{priority}} + \text{Tip}_{\text{Jito}}) \times \text{Rate}_{\text{num}}}{\text{Rate}_{\text{denom}}} \right\rceil$.

## 3. Kamino Flash Loan Fee Invariant

The flash loan fee is computed from the on-chain `U68F60` fraction (`flash_loan_fee_sf`):
- Scaled factor has 60 fractional bits ($2^{60} \approx 1.1529 \times 10^{18}$).
- Raw unscaled fee bits:
  $$\text{fee\_bits} = \text{borrow\_amount} \times \text{flash\_loan\_fee\_sf}$$
- Floor requirement: Minimum fee is 1 atomic unit ($2^{60}$ fractional bits).
- Guard: If $\text{borrow\_amount} \times \text{flash\_loan\_fee\_sf} < 2^{60}$, the borrow is rejected with `ArbError::BorrowTooSmall`. The comparison is strictly performed pre-rounding on raw bits.
- Rounding: Half ties-away (`(fee_bits + (1 << 59)) >> 60`).

## 4. Cost Ledger & Double-Count Prevention
Every cost item is recorded in `CostLedger` with:
- `CostId`: Unique sequential identifier.
- `CostScope`: `PerHop`, `PerLoan`, `PerTransaction`, `PerBundle`, or `PerAccount`.
- `CostType`: `AmmSwapFee`, `TransferFee`, `FlashLoanPremium`, `BaseSignatureFee`, `PriorityFee`, `JitoTip`, `RentDeposit`, etc.
- Invariants: `included_in_quote` and `deducted_already` boolean guards ensure zero double-deduction.

## 5. Floating-Point Invariant
Floating-point types (`f32`, `f64`) are **strictly prohibited** in all authoritative financial guards, sizing decisions, and execution gates. All calculations utilize `u128`, `i128`, or `U256` checked integer math.
