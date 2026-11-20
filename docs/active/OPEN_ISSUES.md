# Forensic Defect Register & Open Issues

This register tracks all identified engineering defects, correctness gaps, and architectural vulnerabilities across the codebase. Issues are strictly prioritized and must be resolved before proceeding through phase exit gates.

---

## 1. Issue Priority Definitions

- **P0 — Critical / Blocker**: Causes false opportunities, inverted routes, incorrect borrow sizing, underfunded repayments, loss-making trades, or unsafe transactions. Blocks live execution.
- **P1 — High**: Latency degradation, incomplete state tracking, potential edge-case reverts, or unverified fee edge cases.
- **P2 — Medium**: Observability gaps, suboptimal data structures, or code cohesion refactoring.

---

## 2. P0 Defects (Phase 2 Correctness Priority)

### ISSUE-P0-01: Route Deduplication Discards Direction and Hop Sequence

- **File**: `crates/arb-bot/src/supervisor/strategy.rs` (Lines 388–400)
- **Observed Behavior**:
  ```rust
  let mut seen_pool_sets: HashSet<Vec<Pubkey>> = Default::default();
  for o in &merged {
      let mut ids: Vec<Pubkey> = o.hops.iter().map(|h| h.pool_id.0).collect();
      ids.sort_unstable();
      seen_pool_sets.insert(ids);
  }
  ```
- **Root Cause**: Sorting pool public keys alone strips out the directed sequence of hops, intermediate tokens, and swap direction. If an opportunity evaluates $A \rightarrow B \rightarrow A$ and another evaluates $B \rightarrow A \rightarrow B$ over the same pool set, the second route is erroneously discarded as a duplicate.
- **Required Fix**: Implement a comprehensive `RouteSignature` preserving ordered hops, pool addresses, input mints, output mints, swap directions, and protocol IDs.
- **Regression Test**: Test verifying that forward and reverse cycles over identical pool sets are both preserved and evaluated independently.

---

### ISSUE-P0-02: Non-Monotonic Profit Assumption in Borrow Sizing

- **Files**: `crates/arb-engine/src/graph.rs`, `crates/arb-bot/src/supervisor/opportunity.rs`
- **Observed Behavior**: Coarse-to-fine integer search assumes that profit curves are quasi-concave or monotonic with respect to borrow size.
- **Root Cause**: In concentrated liquidity (Meteora DLMM bins, Orca Whirlpools), crossing tick boundaries or hitting empty bins introduces non-linear discontinuities and fee cliffs. A coarse sample can miss profitable narrow liquidity bins or overshoot into empty bins.
- **Required Fix**: Implement joint evaluation of $\text{Route} \times \text{Borrow Size}$ with boundary checks at active bin / tick liquidity limits. Always clamp maximum sizing by decoded live Kamino supply-vault balance.
- **Regression Test**: Multi-bin traversal test verifying that optimal size selection finds maximum net profit even when intermediate bins have zero liquidity.

---

### ISSUE-P0-03: Multi-Hop Route State Coherency and Stale Snapshot Skew

- **Files**: `crates/arb-engine/src/graph.rs`, `crates/arb-bot/src/supervisor/opportunity.rs`
- **Observed Behavior**: A candidate route with 2 or 3 hops can assemble pools whose snapshots were taken at divergent slots (e.g., Hop 1 at slot $S$, Hop 2 at slot $S-50$).
- **Root Cause**: While each individual pool satisfies `slot <= max_state_age_slots`, the inter-pool slot delta within a single candidate route was not strictly bounded. This creates phantom arbitrage opportunities from price divergence across time.
- **Required Fix**: Enforce route-level slot coherency: $\max_{h \in \text{hops}}(\text{slot}_h) - \min_{h \in \text{hops}}(\text{slot}_h) \le \Delta_{\text{max\_skew}}$ (default $\le 4$ slots).
- **Regression Test**: Reject opportunities where hop snapshots have divergent slot watermarks.

---

### ISSUE-P0-04: Implicit Simulation Error Mapping & Slippage Revert 0x1773

- **Files**: `crates/arb-sim/src/lib.rs`, `crates/arb-bot/src/supervisor/dispatch.rs`
- **Observed Behavior**: Simulation errors have occasionally triggered Anchor error `0x1773` (ConstraintRaw / SlippageExceeded) on multi-hop executions.
- **Root Cause**:
  1. Simulation outcomes are represented as boolean `success` and string `error` rather than an explicit enum (`PASSED`, `FAILED`, `UNAVAILABLE`, `TIMEOUT`, `STALE`, `UNSUPPORTED`).
  2. Fixed slippage allowances without dynamic quote drift adjustment cause reverts on fast-moving pools.
- **Required Fix**:
  1. Introduce strongly typed `SimulationOutcome` enum with complete log capture.
  2. Perform deep forensic reproduction of error `0x1773` to determine exact pool balance drift during simulation.
  3. Never silently widen slippage to mask execution errors.
- **Regression Test**: Simulation harness verifying proper error categorization and bounded slippage behavior.

---

## 3. P1 Defects (High Priority)

### ISSUE-P1-01: Token-2022 Transfer Fee Double Counting Audit

- **Files**: `crates/arb-math/src/economic.rs`, `crates/arb-dex/`
- **Current Behavior**: Quote engines return net output after Token-2022 transfer fees and expose the fee breakdown separately; the router chains the net output and treats the breakdown as telemetry. Production discovery excludes non-base SPL mint owners, and the graph/adapters reject nonzero legacy transfer-fee estimates. Token-2022 is therefore unsupported by the live pilot path.
- **Status**: Closed for the pilot by fail-closed exclusion; Token-2022 remains unsupported until epoch-aware mint data is wired from ingestion through route quoting and transaction construction. Do not enable it based only on the standalone quote helpers.

---

### ISSUE-P1-02: Transport Ambiguity and Duplicate Submission Prevention

- **Files**: `crates/arb-bot/src/supervisor/recovery.rs`, `crates/arb-bot/src/supervisor/dispatch.rs`
- **Current Behavior**: On network submission timeout, transactions may be re-evaluated.
- **Required Fix**: Strictly enforce invariant: on timeout, reconcile transaction signature against RPC before taking any action. Never re-sign or re-dispatch duplicate transactions. Only mark expired when blockhash progression proves expiration beyond `last_valid_block_height`.
- **Status**: Implemented in `dispatch.rs` and `recovery.rs`: the signed transaction signature and last-valid block height are persisted before send; transport errors poll that signature, unresolved intents stay pending, and recovery never resubmits. Expiration requires block-height evidence. Runtime soak remains a release gate.

---

### ISSUE-P1-03: Realized PnL vs Quoted PnL Metric Separation

- **Files**: `crates/arb-metrics/src/lib.rs`, `crates/arb-store/src/lib.rs`
- **Current Behavior**: SQLite stores outcome provenance and the daily PnL query already excludes estimated landed rows from realized PnL. However, estimated landed rows were omitted from the estimate bucket, and the `net_profit_total` metric also received estimates when balance metadata was unavailable. It was a Prometheus counter, which cannot represent signed realized losses.
- **Status**: Fixed in source: signed realized PnL is a gauge fed only by on-chain balance deltas; estimates for landed transactions have their own gauge and are included in the estimated PnL bucket. Database rows remain in the shared outcomes table with enforced provenance filters; table-level separation is not required for correct aggregation.

---

## 4. P2 Defects (Medium Priority)

### ISSUE-P2-01: Unbounded Ingest Memory Allocation During High-Vol Shocks

- **File**: `crates/arb-ingest/src/helius_ws.rs`
- **Current Behavior**: The referenced `helius_ws.rs` no longer contains an unbounded Tokio channel (repository search found no `unbounded_channel` or `UnboundedReceiver` usage under `crates/`). This register entry is stale and does not describe the current implementation.
- **Status**: Closed as not present in current source. Continue to monitor bounded ingestion queues if a channel-based decoder pipeline is introduced.
