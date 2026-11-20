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

- **Status**: Fixed in `fix/atomic-arb-mvp`; ordered route identity and best-quote retention are implemented in `supervisor/route_dedup.rs`. Regression tests cover direction, hop order, same pool set, and profit selection. The live profitability of any route remains unproven.

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

- **Status**: Graph-generated 2–6 hop paths now use bounded, multi-basin route-by-size search, including exact re-quotes and canonical net-profit evaluation; active bin/tick ceilings seed the search. The dedicated DLMM optimizer also refines the highest-profit basins first. Both report a best observed candidate, not a mathematically proven global optimum. Continue differential sizing tests against exhaustive small ranges and live snapshots before production use.

- **Files**: `crates/arb-engine/src/graph.rs`, `crates/arb-bot/src/supervisor/opportunity.rs`
- **Observed Behavior**: Coarse-to-fine integer search assumes that profit curves are quasi-concave or monotonic with respect to borrow size.
- **Root Cause**: In concentrated liquidity (Meteora DLMM bins, Orca Whirlpools), crossing tick boundaries or hitting empty bins introduces non-linear discontinuities and fee cliffs. A coarse sample can miss profitable narrow liquidity bins or overshoot into empty bins.
- **Required Fix**: Implement joint evaluation of $\text{Route} \times \text{Borrow Size}$ with boundary checks at active bin / tick liquidity limits. Always clamp maximum sizing by decoded live Kamino supply-vault balance.
- **Regression Test**: Multi-bin traversal test verifying that optimal size selection finds maximum net profit even when intermediate bins have zero liquidity.

---

### ISSUE-P0-03: Multi-Hop Route State Coherency and Stale Snapshot Skew

- **Status**: Existing runtime validation gathers all route dependency stamps together and enforces `max_slot_skew = 0`, with stale/future/mixed-provider/bank states rejected. The ingestion layer has a regression test for slot skew. This is verified in code; add an end-to-end route test if future dependency aggregation changes.

- **Files**: `crates/arb-engine/src/graph.rs`, `crates/arb-bot/src/supervisor/opportunity.rs`
- **Observed Behavior**: A candidate route with 2 or 3 hops can assemble pools whose snapshots were taken at divergent slots (e.g., Hop 1 at slot $S$, Hop 2 at slot $S-50$).
- **Root Cause**: While each individual pool satisfies `slot <= max_state_age_slots`, the inter-pool slot delta within a single candidate route was not strictly bounded. This creates phantom arbitrage opportunities from price divergence across time.
- **Required Fix**: Enforce route-level slot coherency: $\max_{h \in \text{hops}}(\text{slot}_h) - \min_{h \in \text{hops}}(\text{slot}_h) \le \Delta_{\text{max\_skew}}$ (default $\le 4$ slots).
- **Regression Test**: Reject opportunities where hop snapshots have divergent slot watermarks.

---

### ISSUE-P0-04: Implicit Simulation Error Mapping & Slippage Revert 0x1773

- **Status**: Simulation fail-open path fixed; `0x1773` root cause remains open pending deterministic replay with the original signed transaction and exact account snapshot. Slippage limits remain unchanged. See `reports/phase17_cross_dex/PHASE17_CORRECTION_2026-11-20.md`.

- **Files**: `crates/arb-sim/src/lib.rs`, `crates/arb-bot/src/supervisor/dispatch.rs`
- **Observed Behavior**: The independent Phase 17 audit records Meteora `ExceededAmountSlippageTolerance` (`0x1773`) during a non-broadcast simulation. That proves the attempted instruction exceeded its on-chain minimum under that simulated state; it does not identify whether quote drift, quote construction, pool state, or instruction data caused the mismatch.
- **Root Cause**:
  1. Simulation outcomes were represented as boolean `success` and string `error`, and the fallback path could accept incomplete responses.
  2. The precise source of the recorded slippage failure is not proven by the available artifacts.
- **Required Fix**:
  1. ✅ Introduce typed simulation statuses and fail closed on missing execution evidence, zero compute units, or stale context.
  2. ⏳ Reproduce error `0x1773` using the original signed transaction and exact pool/account snapshot, and record per-hop quote/minimum-output provenance.
  3. ✅ Keep current slippage limits unchanged until the failed assumption is identified and justified.
- **Regression Test**: Simulation harness verifying proper error categorization and bounded slippage behavior.
- **Forensic status (2026-11-20)**: The opt-in mainnet diagnostic previously substituted active bin `-6542` when the pool read failed and returned success when `simulateTransaction` reported an RPC error or an on-chain revert. It also hardcodes a `1_500_000` token-unit minimum for the first hop and repeats that amount as the second hop input. The harness now fails closed on missing pool state, RPC errors, missing simulation fields, zero compute units, or a non-null simulation error. This establishes that the old “pass” result was not evidence of successful execution. A same-bank quote and replay are still required to attribute the historical `0x1773` to price movement, fees, account state, or the hardcoded threshold; production slippage was not widened.

---

## 3. P1 Defects (High Priority)

### ISSUE-P1-01: Token-2022 Transfer Fee Double Counting Audit

- **Files**: `crates/arb-math/src/economic.rs`, `crates/arb-dex/`
- **Current Behavior**: Protocol adapters deduct token-specific transfer fees, while the economic engine also checks transfer fee allowances.
- **Required Fix**: Audit each DEX adapter's fee semantics (Raydium CPMM vs DLMM) to guarantee Token-2022 transfer fees are deducted exactly once.

---

### ISSUE-P1-02: Transport Ambiguity and Duplicate Submission Prevention

- **Files**: `crates/arb-bot/src/supervisor/recovery.rs`, `crates/arb-bot/src/supervisor/dispatch.rs`
- **Current Behavior**: On network submission timeout, transactions may be re-evaluated.
- **Required Fix**: Strictly enforce invariant: on timeout, reconcile transaction signature against RPC before taking any action. Never re-sign or re-dispatch duplicate transactions. Only mark expired when blockhash progression proves expiration beyond `last_valid_block_height`.

---

### ISSUE-P1-03: Realized PnL vs Quoted PnL Metric Separation

- **Files**: `crates/arb-metrics/src/lib.rs`, `crates/arb-store/src/lib.rs`
- **Current Behavior**: Some metrics aggregate estimated quote profit alongside realized profit.
- **Required Fix**: Strictly isolate quote estimates, simulation estimates, and confirmed realized balance deltas into disjoint database tables and metric vectors.

---

## 4. P2 Defects (Medium Priority)

### ISSUE-P2-01: Unbounded Ingest Memory Allocation During High-Vol Shocks

- **File**: `crates/arb-ingest/src/helius_ws.rs`
- **Current Behavior**: WebSocket message receiver uses unbounded tokio channel.
- **Required Fix**: Switch to bounded channels with backpressure drops and latency metrics.
