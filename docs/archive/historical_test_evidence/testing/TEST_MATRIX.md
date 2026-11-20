> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# TESTING — TEST STRATEGY & TEST MATRIX

## 1. Test Strategy Overview

The system enforces a multi-tier testing strategy ensuring end-to-end correctness, mathematical precision, security boundaries, and resilience under adversarial conditions:

1. **Unit Tests:** Deterministic verification of formulas, PDA derivations, binary decoders, and data structures.
2. **Property-Based Tests (proptest):** Mathematical invariant testing across infinite random values (checked overflow, fee monotonicity, token conservation).
3. **Boundary & Fuzz Tests:** Evaluation of integer boundaries, extreme tick ranges, bin-array crossings, and oversized transaction messages.
4. **Differential Tests:** Cross-validation of Rust quoter calculations against official Python and on-chain SDK fixtures.
5. **Integration Tests:** End-to-end simulations of multi-hop flashloan pipelines, bundle landing handlers, and quota exhaustion failovers.

---

## 2. Test Execution Matrix

Executed on 2026-11-19 via `cargo test --workspace` on Windows / MSVC Rust 1.100.0:

| Crate / Target | Test Suite | Tests Passed | Tests Failed | Key Verified Invariants |
|---|---|---|---|---|
| `arb-types` | Unit & Currency Tests | 23 | 0 | Typed amount safety, cross-mint arithmetic rejection, lamports conversion |
| `arb-constants` | Unit Tests | 1 | 0 | Well-known mints, program IDs, sysvar addresses |
| `arb-config` | Config File & Gate Tests | 6 | 0 | Deserialization, min profit hurdles, live freshness gates |
| `arb-dex` | Decoders & Fee Tests | 45 | 0 | Discriminator checks, tick array layouts, bin array indices |
| `arb-math` | AMM Math & Boundary Tests | 199 | 0 | CPMM/CLMM/DLMM quotes, bin boundary traversal, convex sizing, integer sqrt |
| `arb-store` | Storage & Cache Tests | 18 | 0 | Lock-free reads, monotonic updates, SQLite persistence |
| `arb-risk` | Pre-Trade Gate Tests | 14 | 0 | Notional caps, profit floors, price impact bounds |
| `arb-tx` | Builder & Property Fuzz Tests | 115 | 0 | Kamino borrow/repay atomicity, V0/V1 headers, prewarmed templates |
| `arb-ingest` | Ingestion & Concurrency Tests | 159 | 0 | Slot watermarks, dead slot eviction, WebSocket failover, deduplication |
| `arb-sim` | Simulation & Fork Tests | 34 | 0 | Value conservation, CU consumption tracking, simulation errors |
| `arb-land` | Tip & Bundle Tests | 68 | 0 | Dynamic tip ladders, regional Jito dispatch, bundle rejection handling |
| `arb-metrics` | Telemetry & Latency Tests | 2 | 0 | Metric registry, latency histograms |
| `arb-engine` | Graph & Pipeline Tests | 71 | 0 | 2/3/4-hop detection, Bellman-Ford negative cycles, sizing optimization |
| `arb-bot` | Supervisor & Web API Tests | 79 | 0 | Admin authentication, kill switch, recovery ledger, telemetry streams |
| **TOTAL** | **Full Workspace** | **834+** | **0** | **100% Pass Rate Across All Member Crates** |

---

## 3. Key Property & Invariant Tests

### 3.1 Value Conservation
- **File:** `tests/dimensional_currency_tests.rs`, `tests/meteora_fresh_pipeline_test.rs`
- **Invariant:** In any multi-hop arbitrage route, sum of inputs and intermediate balances must be strictly accounted for; no phantom tokens or un-repaid lamports can exist.

### 3.2 Non-Negative Profit Rule
- **File:** `crates/arb-risk/src/lib.rs`, `crates/arb-math/src/economic.rs`
- **Invariant:** A route where gross edge $\le$ sum of all transaction fees, DEX fees, flashloan fees, and tips is strictly rejected. Net profit cannot be fabricated.

### 3.3 Atomic Flash-Loan Repayment Guarantee
- **File:** `crates/arb-tx/tests/v1_property_fuzz.rs`, `crates/arb-tx/src/flash/kamino.rs`
- **Invariant:** Transaction instructions are ordered such that Kamino borrow is instruction index $k$ and Kamino repay is instruction index $N$ in the same atomic transaction. The runtime aborts the entire bundle if repay is unsatisfied.

### 3.4 Dead Slot Eviction
- **File:** `crates/arb-ingest/tests/slot_integrity.rs`
- **Invariant:** Any slot notified as dead or unconfirmed immediately invalidates and evicts all account updates associated with that slot from `MarketStateCache`.
