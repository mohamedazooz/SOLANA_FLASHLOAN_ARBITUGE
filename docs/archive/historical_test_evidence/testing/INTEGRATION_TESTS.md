> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# End-to-End Pipeline Integration Tests

This document catalogs and details the integration test suites verifying interactions across multiple crates in the canonical execution pipeline. Integration tests exercise the full journey from market ingestion and state validation through route quote optimization, transaction compilation, and bundle landing.

---

## 1. Integration Test Catalog

The repository maintains specialized integration test suites across core crates:

| Test Suite File | Tested Crate | Pipeline Stage | Primary Verification Goal |
|---|---|---|---|
| [three_leg_meme_pipeline_tests.rs](file:///c:/Users/azoozz/Downloads/my%20arbitrge/SOLANA_FLASHLOAN_ARBITUGE/crates/arb-engine/tests/three_leg_meme_pipeline_tests.rs) | `arb-engine` | Discovery $\to$ Quote $\to$ Sizing $\to$ Compiler $\to$ Gate | Full 3-leg cycle (`WSOL -> MEME_A -> MEME_B -> WSOL`) using discrete DLMM active bin math and V0 Static Master ALT. |
| [three_pool_flash_arb_tests.rs](file:///c:/Users/azoozz/Downloads/my%20arbitrge/SOLANA_FLASHLOAN_ARBITUGE/crates/arb-tx/tests/three_pool_flash_arb_tests.rs) | `arb-tx` | Flash Loan $\to$ Multi-DEX $\to$ Repay | Kamino flash borrow + 3 swaps + Kamino flash repay atomicity; V0 and V1 SIMD-0385 wire format compliance. |
| [guard_01_execution_tests.rs](file:///c:/Users/azoozz/Downloads/my%20arbitrge/SOLANA_FLASHLOAN_ARBITUGE/crates/arb-engine/tests/guard_01_execution_tests.rs) | `arb-engine` | StateIntegrity $\to$ Pre-Dispatch Gate | Fail-closed eviction: prevents route dispatch when underlying pool slot is stale, mutated, or missing dependencies. |
| [intermediate_balance_isolation_tests.rs](file:///c:/Users/azoozz/Downloads/my%20arbitrge/SOLANA_FLASHLOAN_ARBITUGE/crates/arb-tx/tests/intermediate_balance_isolation_tests.rs) | `arb-tx` | Hop Execution $\to$ Token ATA Security | Verifies intermediate token accounts are strictly isolated; guarantees no leakage of unswapped balances. |
| [meteora_fresh_pipeline_test.rs](file:///c:/Users/azoozz/Downloads/my%20arbitrge/SOLANA_FLASHLOAN_ARBITUGE/crates/arb-sim/tests/meteora_fresh_pipeline_test.rs) | `arb-sim` | Simulation $\to$ Compute Unit Audit | Simulates DLMM bin traversals against on-chain bytecode; verifies CU budget bounds under 400,000 CUs. |
| [bundle_rejection_tests.rs](file:///c:/Users/azoozz/Downloads/my%20arbitrge/SOLANA_FLASHLOAN_ARBITUGE/crates/arb-land/tests/bundle_rejection_tests.rs) | `arb-land` | Jito Dispatch $\to$ Status Tracking | Mock HTTP block engine rejection (HTTP 400, 429, 503); verifies fail-closed error propagation without silent drops. |
| [mainnet_alt_integration_tests.rs](file:///c:/Users/azoozz/Downloads/my%20arbitrge/SOLANA_FLASHLOAN_ARBITUGE/crates/arb-tx/tests/mainnet_alt_integration_tests.rs) | `arb-tx` | Address Lookup Tables | Resolves account indices against on-chain static ALTs; verifies table compression down to 1-byte indexes. |

---

## 2. Deep Dive: Key Pipeline Test Scenarios

### 2.1 Three-Leg Meme Arbitrage Pipeline (`three_leg_meme_pipeline_tests.rs`)
- **Route Topology**: `WSOL -> Meme Token A -> Meme Token B -> WSOL`
- **Execution Flow**:
  1. `MeteoraDlmmPoolPairDiscovery` discovers eligible triangular graph cycles.
  2. `MeteoraDlmmCrossPoolQuoteEngine` computes exact discrete bin swap quotes across active bins.
  3. `ThreeLegMemeSizeSearchResult` conducts multi-rung search for optimal capital allocation $x^*$.
  4. `StaticMasterAlt` compresses all 24 account metas into 1-byte lookup table indices.
  5. `compile_3leg_meme_dlmm_arb` emits a signed VersionedTransaction $\le 1232$ bytes.
  6. `simulate_profitability_gate` verifies net profit after all fee deductions $\ge \text{MIN\_PROFIT\_THRESHOLD}$.

### 2.2 Kamino Atomic Flash Loan Construction (`three_pool_flash_arb_tests.rs`)
- **Instruction Ordering Matrix**:
  $$\text{Index 0: ComputeBudget::set_compute_unit_limit}$$
  $$\text{Index 1: ComputeBudget::set_compute_unit_price}$$
  $$\text{Index 2: KaminoFlashLoan::FlashBorrowReserveLiquidity}(A_{\text{borrow}})$$
  $$\text{Index 3: DEX Swap Hop 1 } (A_{\text{borrow}} \to B_1)$$
  $$\text{Index 4: DEX Swap Hop 2 } (B_1 \to B_2)$$
  $$\text{Index 5: DEX Swap Hop 3 } (B_2 \to A_{\text{final}})$$
  $$\text{Index 6: KaminoFlashLoan::FlashRepayReserveLiquidity}(A_{\text{borrow}} + \text{fee})$$
  $$\text{Index 7: JitoTipPayment::transfer}(\text{tip\_lamports})$$
- **Verification Guarantee**: The runtime strictly rejects any transaction where the repay instruction index does not match the borrow reserve, or where $A_{\text{final}} < A_{\text{borrow}} + \text{fee}$.

### 2.3 StateGuard-01 Fail-Closed Invalidation (`guard_01_execution_tests.rs`)
- **Simulated Adversarial Conditions**:
  - Pool update arrived with slot $S_k$, but dependent token account update arrived with slot $S_{k-3}$ (Stale dependency).
  - Pool reserve modified in cache between opportunity generation and transaction dispatch (Dirty read).
  - Provider WebSocket sent heartbeat disconnect signal.
- **Observed Behavior**: `revalidate_before_dispatch` immediately returns `Err(GuardedQuoteError::DependencyStale)` and halts execution, preventing wasted transaction priority fees.

---

## 3. Running Integration Tests

To run all pipeline integration suites:

```bash
# Run all integration tests across engine, tx, sim, and land crates
cargo test --package arb-engine --test three_leg_meme_pipeline_tests
cargo test --package arb-tx --test three_pool_flash_arb_tests
cargo test --package arb-engine --test guard_01_execution_tests
cargo test --package arb-land --test bundle_rejection_tests
```

All integration suites pass deterministically without network access.
