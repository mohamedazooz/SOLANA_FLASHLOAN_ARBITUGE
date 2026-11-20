> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Dynamic Pricing

**Repository:** Solana Arbitrage Bot  
**Status:** Architecture defined (Phase 4 pending implementation)

Market-dependent parameters must **not** remain static. They are observed from
live data with freshness and validity checks. Each source has an independent
freshness policy — there is no universal timeout.

---

## Data Contract

Every market-dependent execution decision verifies a `MarketObservation`:

```text
MarketObservation {
    source,
    observed_at,
    received_at,
    slot,
    value,
    confidence,
    freshness,
    generation
}
```

Checks performed before use:
- Source validity
- Data freshness (per-source policy)
- Slot consistency
- Provider health
- Generation consistency
- Valid numeric values

---

## Per-Source Freshness Policies

| Source | Example policy | Stale handling |
|--------|---------------|----------------|
| Pool snapshots | `max_state_age_slots` (default 2 slots) | Fail closed; skip tick |
| SOL price | Live from SOL-USDC pools; `None` if unavailable | Fail closed; no guessed constant |
| Jito tips | `tip_floor_max_age_ms` (default 5000 ms) | Treat as absent; never reuse stale |
| Priority fees | Recent fee-market observations | Percentile CU pricing; bounded escalation |
| Blockhashes | 3s cache TTL; `None` on fetch failure | Fail closed; never zero blockhash |
| Landing statistics | Observed send/landed outcomes; track sample size | Insufficient history ⇒ not reliable |

---

## Specific Parameters

### Jito Tip Floor
- Integrate official Tip Floor REST API (`https://bundles.jito.wtf/api/v1/bundles/tip_floor`).
- WebSocket updates (`tip_stream`) supplement REST polling.
- Percentile-based bidding (p50/p75/p90/p99).
- EMA and freshness tracking.
- Validate timestamps and response values; reject stale data.
- Configurable min/max safety bounds.
- **Never confuse minimum safety floor with current market price.**

### Priority Fees
- Use recent fee-market observations.
- Consider relevant writable accounts.
- Percentile-based CU pricing.
- Bound escalation and total expenditure.
- Prevent integer overflow.

### Compute Units
- Use transaction simulation.
- Maintain historical CU consumption by protocol/route.
- Adaptive safety margin.
- Respect Solana runtime limits.
- Handle legacy, V0, V1 transaction formats separately.
- Do not apply legacy Compute Budget instructions to V1 transactions.

### DEX Fees
- Read verified on-chain values.
- Respect protocol-specific fee formulas.
- Never substitute a generic fee when data is missing.
- Invalid fee data ⇒ typed error (never silent 100% cap).

### Liquidity
- Calculate real executable depth (reserves, bins, ticks, account coverage).
- Reject incomplete route coverage.

### Price Impact
- Calculate using actual route execution.
- No arbitrary constant multipliers.

### SOL/USD
- Use a verified live price source.
- Track source, timestamp, confidence.
- Reject unavailable or stale prices.

### Landing Probability
- Use observed send/landed outcomes.
- Track sample size and confidence.
- Avoid using insufficient history as reliable probability.

## DLMM 92% Safety Factor (Open Question)

`arb-math/src/dlmm.rs:305` applies a static 92% factor. This must be compared
against official Meteora SDK quote results; if unjustified, remove the
approximation and reject incomplete quotes instead.
