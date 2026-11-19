# QUANTITATIVE MODEL — COMPREHENSIVE FEE MODEL

**Author:** Quantitative Financial Auditor  
**Date:** 2026-11-19  
**Status:** CANONICAL & VERIFIED  

---

## 1. Unified Fee Decomposition

In the Sentinel execution pipeline, every potential trade must satisfy:
$$\text{NetProfit} = \text{GrossEdge} - \sum \text{All Mandatory Execution Costs}$$

Where total mandatory execution costs include:
$$\sum \text{Costs} = \text{Fee}_{\text{FlashLoan}} + \sum_{i=1}^{N} \text{Fee}_{\text{DEX}, i} + \sum \text{Fee}_{\text{Token2022}} + \text{Cost}_{\text{PriorityFee}} + \text{Cost}_{\text{JitoTip}} + \text{Cost}_{\text{BaseTx}}$$

---

## 2. Protocol-by-Protocol Fee Schedules

### 2.1 Kamino Flash Loan Borrow Fee
- **Scaling Factor:** $sf = 11,529,215,046,068$ (in Q68.60 format)
- **Rate:** $\approx 0.1\text{ bps (0.001\%)}$
- **Formula:**
  $$\text{Fee}_{\text{Kamino}} = \max\left(1, \left\lfloor \frac{\text{BorrowedLamports} \times sf}{2^{60}} \right\rfloor\right)$$
- **Example:** For a 10 SOL borrow ($10^{10}$ lamports), fee is $100,000\text{ lamports (0.0001 SOL)}$.

### 2.2 Raydium Fees
- **Raydium CPMM (CP-Swap):**
  - Standard tier: 25 bps (0.25%).
  - Configurable per `AmmConfig` account on-chain. Read dynamically from pool account.
- **Raydium CLMM:**
  - Fee tiers: 1 bps, 4 bps, 25 bps, 100 bps.
  - Read from `ClmmConfig` account on-chain.
- **Raydium AMM V4:**
  - Standard trade fee: 25 bps (22 bps to LPs, 3 bps to protocol).

### 2.3 Orca Whirlpool Fees
- Configured in hundredths of a basis point ($1/1,000,000$).
- Standard tiers: 100 (1 bps), 200 (2 bps), 400 (4 bps), 3000 (30 bps), 10000 (100 bps).
- Read dynamically from the `fee_rate` field of the Whirlpool account.

### 2.4 Meteora DLMM Fees
- **Base Fee:** Derived from `bin_step` and base factor.
- **Variable Dynamic Fee:** Scaled by `volatility_accumulator` during periods of rapid price movement.
- **Total Fee:** $\text{fee\_bps} = \text{base\_fee\_bps} + \text{variable\_fee\_bps}$, capped at $\text{max\_fee\_bps}$ (up to 10% on extreme meme coins).

### 2.5 Pump.fun & PumpSwap Fees
- **Pump.fun Bonding Curve:** 100 bps (1%) flat fee deducted from SOL leg.
- **PumpSwap AMM:**
  $$\text{fee\_bps} = \text{protocol\_fee\_bps} + \text{creator\_fee\_bps}$$
  Typically 25 bps protocol fee + creator fee (0 to 100 bps).

### 2.6 Token-2022 Transfer Fees
- If a route touches a Token-2022 mint with the `TransferFeeConfig` extension, transfer fees are withheld at each transfer hop:
  $$\text{fee} = \min\left(\text{max\_fee}, \left\lceil \frac{\text{amount} \times \text{fee\_bps}}{10000} \right\rceil\right)$$
- If transfer fees apply to intermediate tokens, they are factored into the net quoter before route selection.

---

## 3. Solana Network Execution Costs

### 3.1 Base Transaction Fee
- 5,000 lamports ($0.000005\text{ SOL}$) per signature (typically 1 signature for the trader wallet).

### 3.2 Compute Budget Priority Fee
- **Compute Unit Limit:** Estimated per route complexity:
  - 2-hop CPMM: $\approx 180,000\text{ CU}$
  - 3-hop DLMM/CLMM: $\approx 350,000\text{ CU}$
  - Flashloan borrow + 3 hops + repay: $\approx 450,000\text{ CU}$
- **Micro-lamport Price:** Fetched from RPC `getPriorityFeeEstimate` or configured floor.
- **Cost:**
  $$\text{PriorityFeeCost} = \left\lfloor \frac{\text{CU\_Limit} \times \text{ComputeUnitPriceMicroLamports}}{1,000,000} \right\rfloor$$

### 3.3 Jito MEV Tip
- **Floor:** 10,000 lamports.
- **Dynamic Tip:** P50/P75 percentile from `TipOracle`.
- **Profit Cap:** Hard cap at $50\%$ of calculated gross edge:
  $$\text{JitoTip} \le 0.50 \times (\text{GrossEdge} - \text{BaseFees})$$
- Under no circumstances is a tip permitted to turn a profitable trade into a realized loss.
