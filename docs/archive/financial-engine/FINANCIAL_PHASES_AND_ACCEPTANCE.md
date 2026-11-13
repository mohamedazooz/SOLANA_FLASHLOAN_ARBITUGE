# Financial Core Remediation — Phases and Acceptance

## F0 — Audit / contract freeze (current)

Deliverables: source audit, single formula contract, protocol source register, golden test plan. No production formulas are declared fixed in this phase.

Acceptance:
- all known profit/cost calculations inventoried
- all amounts assigned units and mint denomination
- no live path can bypass RiskGate
- no live path uses guessed size/fee/tip

## F1 — Signed, denomination-safe core

Files: `arb-types`, `arb-math/economic.rs`, `arb-store`, `arb-risk`.

Implement:
- strongly typed mint amounts and signed base PnL
- separate floor/ceiling rational conversions
- reject zero denominators
- expected and min-out PnL
- cost ledger cannot mix units
- checked sums and duplicate cost IDs
- versioned serialized accounting schema

Acceptance:
- golden vectors, property tests, fuzz tests
- negative PnL retained exactly
- no duplicate cost subtraction
- all current call sites migrated or compilation fails by design

## F2 — One evaluation through the whole pipeline

Files: `arb-engine`, `arb-bot/supervisor`, `arb-risk`, `arb-tx`, `arb-land`, `arb-store`.

Implement:
- immutable `EconomicEvaluation` shared by all stages
- same `min_out` and profit floor used in RiskGate and transaction builder
- reject when no authoritative USD valuation for max notional
- no `unwrap_or` sizing/fee/tip fallback
- execution result state machine and event ledger

Acceptance:
- unit test proves same evaluation hash reaches RiskGate and TxBuilder
- negative / stale / mixed-mint routes are rejected
- no signing after RiskGate failure

## F3 — Meme-coin venue economics

Files: `arb-types`, `arb-dex`, `arb-ingest`, `arb-engine`, `arb-tx`.

Implement:
- Pump bonding curve decoder + exact buy/sell quote and current fee state
- PumpSwap dedicated adapter (canonical and non-canonical pools)
- Pump FeeConfig / creator / protocol / LP / holder rewards state
- Pump v2/v3 instruction semantics and fee-accounting behavior
- graduation state machine and synthetic migration boundary
- cross-venue route quotes Pump ↔ PumpSwap ↔ Raydium ↔ Orca ↔ Meteora

Acceptance:
- differential tests against Pump Rust/TypeScript SDK and current IDLs
- quote output and fee components match on known fixtures
- migration crossing test proves no artificial free edge

## F4 — Optimal sizing by curve family

- CPMM: closed-form optimum only where assumptions hold (two constant-product pools, fee model fixed and compatible, no transfer fees or other discontinuities); integer verify around optimum.
- CLMM: piecewise optimization across initialized tick boundaries and liquidity changes.
- DLMM: piecewise/discrete optimization across bins and volatility fee changes.
- Pump bonding curve: virtual reserves with real-reserve caps and graduation/synthetic-migration behavior.
- PumpSwap: fee tier and reserve-dependent CPMM quote.

Acceptance:
- evaluate all candidate sizes with exact integer quote functions
- search boundaries around every tick/bin/fee tier
- optimize net minimum PnL after flash/network/priority/tip costs
- no use of CPMM derivative as a CLMM/DLMM/Pump universal optimizer

## F5 — Chronos probability and EV

Separate:
- state-freshness hazard
- route/leader landing probability
- probability of landed success vs landed program failure
- probability of not landing / expiry

Acceptance:
- probabilities calibrated from timestamped observations and confidence intervals
- separate SWQoS and Jito distributions
- no default 30% treated as measured fact
- EV uses disjoint outcome states and actual outcome-specific cost models

## F6 — Jito, flash loan and realized accounting

Implement:
- borrow + all swaps + repay + min-profit assertion in one transaction
- Jito bundle submission/status by bundle ID
- signature status by transaction signature
- transaction metadata: fee, error, CU consumed, pre/post lamports, pre/post token balances, inner instructions
- exact tip-account transfer verification
- dedup by signature and bundle ID
- actual signed PnL reconciliation

Acceptance:
- BundleAccepted != Landed
- Processed != Finalized
- failed landed tx fee accounted
- no landed success recorded without metadata reconciliation
- expected-vs-realized PnL variance report
