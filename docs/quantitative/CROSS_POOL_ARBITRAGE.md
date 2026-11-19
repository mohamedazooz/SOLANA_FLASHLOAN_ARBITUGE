# QUANTITATIVE AUDIT — CROSS-POOL ARBITRAGE SCANNER

**Author:** Quantitative Financial Auditor & Solana HFT Engineer  
**Date:** 2026-11-19  
**Status:** AUDITED & ARCHITECTURALLY ISOLATED  

---

## 1. Executive Summary & Role Separation

The cross-pool scanner scripts introduced in PR #81 (`scripts/scan_live_cross_pool.py`, `scripts/calculate_arbitrage_opportunities.py`) are **independent research and discovery tools**.

In accordance with Section 11 of the Master Directive:
- **Research Role:** Low-cost off-chain discovery of active token pairs, general liquidity distributions, and historical pricing spreads across public DEX APIs (DexScreener, GMGN).
- **Production Boundary:** Public REST APIs (such as DexScreener) **MUST NOT** be used as authoritative executable quotes for live HFT trading.
- **Strict Isolation:** The production HFT pipeline uses WebSocket-ingested on-chain account data exclusively.

---

## 2. Forensic Audit of the Research Scanner

### 2.1 Token Orientation & Mint Grouping
- **Risk Identified:** Older cross-pool scanners frequently group trading pairs by symbol (e.g. `PEPE`, `DOGE`). On Solana, multiple unrelated or malicious tokens share identical ticker symbols.
- **Remediation:** All market grouping must be performed by canonical **Token Mint Address** (`MintId`), never token symbols alone.
- **Orientation Invariant:** When comparing Pool A ($M_1 / M_2$) and Pool B ($M_2 / M_1$), prices must be inverted before spread calculation:
  $$P_{B}(M_1 \text{ in terms of } M_2) = \frac{1}{P_B(M_2 \text{ in terms of } M_1)}$$

### 2.2 Liquidity Heuristics vs. Execution Sizing
- In `scan_live_cross_pool.py`, a 1% liquidity rule was used as an exploratory filter:
  $$\text{TradeSize} = 0.01 \times \min(\text{Liquidity}_A, \text{Liquidity}_B)$$
- While acceptable as a rough research heuristic, this is **not** optimal execution sizing.
- Real execution sizing must evaluate the exact non-linear price impact curve ($x^*$) balancing margin against slippage and fixed fees.

### 2.3 Stale Price & Latency Limitations of Public APIs
- DexScreener and public DEX APIs aggregate trades with 2 to 15 seconds of latency.
- In Solana high-frequency trading (400ms slot times), an opportunity observed on a public dashboard has already been captured or closed by MEV searchers 5 to 30 slots earlier.
- Therefore, scanner output is strictly historical evidence, not an actionable live quote.

---

## 3. Comparative Matrix: Scanner vs. Production HFT Engine

| Dimension | Research Scanner (`scan_live_cross_pool.py`) | Production Execution Engine (`arb-engine`) |
|---|---|---|
| **Data Source** | DexScreener HTTP REST API | Solami / Helius WebSocket account streams |
| **Update Latency** | 2,000 – 15,000 ms | 5 – 50 ms (intra-slot) |
| **Pricing Model** | Display mid-market price | Exact tick/bin array integer reserve equations |
| **Sizing Logic** | 1% of total liquidity | Convex derivative root ($x^*$) |
| **Cost Modeling** | Estimated flat fee | Exact Kamino loan fee + DEX fee + CU price + Jito tip |
| **Execution Path** | None (Reporting only) | Atomic flash loan transaction compilation & Jito bundle send |
| **Safety Invariant** | Informational | Fail-closed pre-trade risk gates |
