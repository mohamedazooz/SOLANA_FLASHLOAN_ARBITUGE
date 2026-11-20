> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# QUANTITATIVE MODEL — PROFIT CALCULATION & PNL RECONCILIATION

**Author:** Quantitative Financial Auditor  
**Date:** 2026-11-19  
**Status:** CANONICAL & VERIFIED  

---

## 1. The Three Tiers of Profit (No Fabricated PnL)

To eliminate misleading reporting, the Sentinel system maintains three strictly separated tiers of profit:

```text
+-------------------+      Simulation      +-------------------+      On-Chain Land     +-------------------+
|   EXPECTED PnL    |  ----------------->  |   SIMULATED PnL   |  ------------------->  |   REALIZED PnL    |
| (Quoting Engine)  |                      |  (Pre-flight RPC) |                        | (Post-Tx Balance) |
+-------------------+                      +-------------------+                        +-------------------+
```

1. **EXPECTED PnL:**
   Calculated by `arb-engine` and `arb-math` from local WebSocket account snapshots. Used solely as an initial filter to decide whether to build a transaction.
2. **SIMULATED PnL:**
   Observed from RPC `simulateTransaction` pre-flight execution logs. Evaluates the actual runtime state of the current slot.
3. **REALIZED PnL:**
   The ONLY authoritative financial measure. Derived from confirmed on-chain post-transaction bank balances minus pre-transaction balances.

**Rule:** Estimated or simulated profits must NEVER be recorded as earned or realized profits in telemetry, dashboards, or reports.

---

## 2. Canonical Realized PnL Formula

For a transaction landed at signature `sig` and slot `S`:

$$\Delta \text{SOL} = \text{Balance}_{\text{post}}(\text{Wallet}) - \text{Balance}_{\text{pre}}(\text{Wallet})$$
$$\text{RealizedNetProfit} = \Delta \text{SOL} - \text{PaidPriorityFee} - \text{PaidJitoTip}$$

Where:
- Flash loan principal and flash loan fees are already accounted for inside $\Delta \text{SOL}$ (because repayment was enforced atomically within the transaction).
- Swap outputs and intermediate fees are fully settled.
- Priority fees and tips transferred from the wallet are subtracted.

---

## 3. Dimensional Currency Safety

Arbitrage cycles must start and end in the **same currency asset** (WSOL or USDC).
- **Cross-Mint Profit Aggregation Prohibited:**
  The system strictly forbids adding a raw USDC balance change ($6\text{ decimals}$) to a SOL balance change ($9\text{ decimals}$) or claiming profit by assuming a fictional exchange rate.
- **Checked Decimal Scaling:**
  Where display conversion to USD is required for operator visibility:
  $$\text{Value}_{\text{USD}} = \frac{\text{Lamports} \times \text{VerifiedSolUsdPrice}}{10^9}$$
  If a verified live oracle price is unavailable, display conversions fail closed or display raw lamports.

---

## 4. Reconciliation Difference ($\delta_{\text{recon}}$)

After every confirmed trade:
$$\delta_{\text{recon}} = \text{RealizedNetProfit} - \text{ExpectedNetProfit}$$

- **Positive $\delta_{\text{recon}}$:** Favorable execution slippage (e.g. favorable user trade occurred right before our bundle in the block).
- **Negative $\delta_{\text{recon}}$:** Adverse slippage or unexpected compute unit pricing.
- **Slippage Threshold Alert:** If $|\delta_{\text{recon}}| > 25,000\text{ lamports}$, an operational alert is dispatched to investigate model divergence.
