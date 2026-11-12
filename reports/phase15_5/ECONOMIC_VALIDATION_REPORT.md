# Economic Validation Report

## Summary

This report validates the economic calculations in the arb-execution crate.

## Fee Components

### 1. Jito Tips
- **Recipient:** Jito tip accounts (8 verified accounts)
- **Mechanism:** Block engine auction
- **Range:** 10,000+ lamports (floor)
- **Economics:** Paid to validators for bundle inclusion priority

### 2. Priority Fees (V0)
- **Recipient:** Block leader
- **Mechanism:** SWQoS (Stake-Weighted Quality of Service)
- **Unit:** Micro-lamports per Compute Unit
- **Formula:** `cu_price * cu_limit / 1e6`

### 3. Priority Fees (V1)
- **Recipient:** Block leader
- **Mechanism:** SWQoS
- **Unit:** Absolute total in lamports
- **Formula:** `priority_fee_lamports` (direct field in V1 config)

### 4. Network Transaction Fees
- **Base Fee:** 5,000 lamports (standard)
- **Paid by:** Fee payer
- **Mechanism:** Solana base fee

### 5. Swap Fees
- **DEX-specific:** Basis points per swap
- **Raydium V4:** ~25 bps (0.25%)
- **Meteora DLMM:** Variable (base factor + volatility fee)
- **Orca Whirlpool:** ~30 bps (0.30%)

### 6. Flash Loan Fees
- **Kamino:** ~0.001% (1 bp) of borrowed amount
- **Formula:** `borrow_amount * fee_sf / 1e9`

### 7. Rent and Account Creation Costs
- **ATA Creation:** ~0.00203928 SOL (rent-exempt minimum)
- **Reclaimed via:** CloseAccount instruction
- **Net Cost:** 0 if successfully closed

## Economic Validation

### V0 Path Economics
- Jito tip: Paid via SystemProgram::transfer to Jito tip account
- Priority fee: Paid via ComputeBudget::set_compute_unit_price
- Total cost: tip + (cu_price * cu_limit / 1e6) + base_fee + swap_fees + flash_fee

### V1 Path Economics
- Priority fee: Paid via V1 config (absolute lamports)
- Jito tip: Converted to additional priority fee
- Total cost: (base_cu_price + extra) * cu_limit / 1e6 + base_fee + swap_fees + flash_fee

### Key Distinction
Jito tips and priority fees are **distinct economic mechanisms**:
- Jito tips: Auction-based, go to block engine, guarantee bundle inclusion
- Priority fees: SWQoS-based, go to leader, probabilistic inclusion

### Double-Counting Check
- V0: Tip + Priority fee are separate (no double counting) ✅
- V1: Tip converted to priority fee (single payment) ✅

### Omission Check
- Base fee: Included ✅
- Swap fees: Included (in quote) ✅
- Flash loan fee: Included ✅
- Rent: Included (reclaimed) ✅
- Compute cost: Included (CU limit) ✅

## Verdict

**PASS** — Economic calculations are correct with proper separation of Jito tips and priority fees.