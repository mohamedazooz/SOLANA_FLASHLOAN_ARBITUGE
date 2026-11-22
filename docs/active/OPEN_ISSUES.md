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
- **Status (2026-11-20)**: Implemented in PR #93 with ordered route identity and keep-best duplicate selection. PR #93 CI is green; `main` remains blocked until review and merge.

---

### ISSUE-P0-02: Non-Monotonic Profit Assumption in Borrow Sizing

- **Status**: Graph-generated 2–6 hop paths now use bounded, multi-basin route-by-size search, including exact re-quotes and canonical net-profit evaluation; active bin/tick ceilings seed the search. The dedicated DLMM optimizer also refines the highest-profit basins first. Both report a best observed candidate, not a mathematically proven global optimum. Continue differential sizing tests against exhaustive small ranges and live snapshots before production use.

- **Files**: `crates/arb-engine/src/graph.rs`, `crates/arb-bot/src/supervisor/opportunity.rs`
- **Observed Behavior**: Coarse-to-fine integer search assumes that profit curves are quasi-concave or monotonic with respect to borrow size.
- **Root Cause**: In concentrated liquidity (Meteora DLMM bins, Orca Whirlpools), crossing tick boundaries or hitting empty bins introduces non-linear discontinuities and fee cliffs. A coarse sample can miss profitable narrow liquidity bins or overshoot into empty bins.
- **Required Fix**: Implement joint evaluation of $\text{Route} \times \text{Borrow Size}$ with boundary checks at active bin / tick liquidity limits. Always clamp maximum sizing by decoded live Kamino supply-vault balance.
- **Regression Test**: Multi-bin traversal test verifying that optimal size selection finds maximum net profit even when intermediate bins have zero liquidity.
- **Status (2026-11-20)**: DLMM local-peak ranking is in PR #94; bounded joint route-and-size search with per-hop re-quotes and bin/tick input boundaries is in PR #95. It returns the best observed candidate within its sample budget, not a proven global optimum. PR #95's CI exposed an Orca fixture exactly on a zero-capacity tick boundary; PR #97 corrects that fixture and has green CI. Live Kamino vault-cap decoding and production differential soak remain required.

---

### ISSUE-P0-03: Multi-Hop Route State Coherency and Stale Snapshot Skew

- **Status**: Existing runtime validation gathers all route dependency stamps together and enforces `max_slot_skew = 0`, with stale/future/mixed-provider/bank states rejected. The ingestion layer has a regression test for slot skew. This is verified in code; add an end-to-end route test if future dependency aggregation changes.

- **Files**: `crates/arb-engine/src/graph.rs`, `crates/arb-bot/src/supervisor/opportunity.rs`
- **Observed Behavior**: A candidate route with 2 or 3 hops can assemble pools whose snapshots were taken at divergent slots (e.g., Hop 1 at slot $S$, Hop 2 at slot $S-50$).
- **Root Cause**: While each individual pool satisfies `slot <= max_state_age_slots`, the inter-pool slot delta within a single candidate route was not strictly bounded. This creates phantom arbitrage opportunities from price divergence across time.
- **Required Fix**: Enforce route-level slot coherency: $\max_{h \in \text{hops}}(\text{slot}_h) - \min_{h \in \text{hops}}(\text{slot}_h) \le \Delta_{\text{max\_skew}}$ (default $\le 4$ slots).
- **Regression Test**: Reject opportunities where hop snapshots have divergent slot watermarks.
- **Status (2026-11-20)**: Runtime dependency validation passes `max_slot_skew = 0` for all route pools, stricter than the four-slot ceiling, and the ingest dependency-group test rejects divergent watermarks. No additional code change is pending.

---

### ISSUE-P0-04: Implicit Simulation Error Mapping & Slippage Revert 0x1773

- **Status**: Simulation fail-open path fixed; deterministic-replay capture plumbing delivered (see Required Fix 2, 2026-11-21); `0x1773` root cause remains open pending actual replay of a captured failure — no capture existed for the historical event. Slippage limits remain unchanged. See `reports/phase17_cross_dex/PHASE17_CORRECTION_2026-11-20.md`.

- **Files**: `crates/arb-sim/src/lib.rs`, `crates/arb-bot/src/supervisor/dispatch.rs`
- **Observed Behavior**: The independent Phase 17 audit records Meteora `ExceededAmountSlippageTolerance` (`0x1773`) during a non-broadcast simulation. That proves the attempted instruction exceeded its on-chain minimum under that simulated state; it does not identify whether quote drift, quote construction, pool state, or instruction data caused the mismatch.
- **Root Cause**:
  1. Simulation outcomes were represented as boolean `success` and string `error`, and the fallback path could accept incomplete responses.
  2. The precise source of the recorded slippage failure is not proven by the available artifacts.
- **Required Fix**:
  1. ✅ Introduce typed simulation statuses and fail closed on missing execution evidence, zero compute units, or stale context.
  2. ✅ (capture plumbing, 2026-11-21) Deterministic-replay forensic capture is implemented and wired: `crates/arb-sim/src/forensic.rs` (`ForensicBundle`: sha256-pinned signed wire bytes + per-hop quote/minimum-output provenance + bounded logs + emitting-program hint + context slot) records every pre-flight simulation failure (`crates/arb-bot/src/supervisor/opportunity.rs`) into `reports/forensics/` (override `FORENSIC_CAPTURE_DIR`), and `replay_locally()` reruns the captured wire through the fail-closed LiteSVM harness (milestone-A, `crates/arb-sim/tests/litesvm_arb_executor_e2e.rs`). NOTE: the HISTORICAL `0x1773` incident itself is still not reproduced — no forensic bundle/saved wire existed when it occurred, so the numeric `Custom(6003)` ambiguity (arb-executor `InvalidMemoProgramId` vs Meteora `ExceededAmountSlippageTolerance`) remains unresolved for that event; every future failure WILL produce a replayable bundle. Live execution remains blocked until a live-consistent same-snapshot replay of a real failure is recorded.
  3. ✅ Keep current slippage limits unchanged until the failed assumption is identified and justified.
- **Regression Test**: Simulation harness verifying proper error categorization and bounded slippage behavior.
- **Status (2026-11-20)**: Typed outcomes and fail-closed dispatch preflight are in PR #93; the Phase 17 diagnostic's fake-bin fallback and simulation-error pass-through are removed in PR #96 (green CI). The opt-in mainnet diagnostic previously substituted active bin `-6542` when the pool read failed and returned success when `simulateTransaction` reported an RPC error or an on-chain revert; it now fails closed. Historical evidence shows the harness could report a revert as a pass while using a hardcoded first-hop minimum; it does not identify the exact on-chain price, fee, or state cause. Same-snapshot quote and replay remain outstanding, so this issue still blocks live execution.

### FL-F-2026-11-22-N1: Split-Route Return Hop Missing Economic Floor (Fixed)

- **Severity**: P2 at time of audit — the path was double-gated dead (`split-routing` feature off by default AND `split = None` hardcoded at `opportunity.rs:756` with a "disabled until proven" notice).
- **File**: `crates/arb-bot/src/supervisor/opportunity.rs` (split-route return hop).
- **Observed Behavior (pre-fix)**: return hop `min_out` was quote-relative slippage only; had split routing ever been re-enabled, price drift within the slippage band could let the Kamino repay succeed out of a pre-existing wallet balance.
- **Correction (this phase)**: `split_return_min_out(total_out, slip, borrow, fee, tip, min_profit) = max(slippage_floor, economic_floor)` is bound to the return hop; integer-only saturating math; gate parity with `TxBuilder::derive_swap_legs`.
- **Regression Tests**: `supervisor::opportunity::split_return_min_out_tests` (4 tests, `--features split-routing`): economic floor binds / slippage binds / zero slippage / u64 saturation.
- **Status**: FIXED in branch `vibe/flashloan-forensic-8ade4e`.

### FL-F-2026-11-22-N2: DLMM Two-Pool Engine Lacks On-Chain Retained-Profit Floor (P3)

- **File**: `crates/arb-engine/src/dlmm_flash_arbitrage.rs` + `crates/arb-tx/src/flash/dlmm_flash_arb.rs`.
- **Observed Behavior**: `leg_b.min_amount_out = repay (principal+fee)` — repayment sufficiency guaranteed (no subsidized repayment), but retained profit and tip are enforced only at quote time (`ProfitBelowFloor`), not as an on-chain min-out. Path is NOT reachable from `arb-bot` (tests/examples only).
- **Required Fix**: when the two-pool engine is wired into supervision, raise `leg_b.min_amount_out` to `max(repay, borrow+fee+tip+min_profit)` and include the tip; then add the FL-15-style regression test to this assembler.
- **Status**: OPEN (P3); wire-up must not proceed without this.

### DEX-F-2026-11-22-N1: DAMM v2 / DBC / DLMM Pro / LaunchLab Quote via CP Approximation (P3)

- **File**: `crates/arb-dex/src/engines/mod.rs:94–118`.
- **Observed Behavior**: those four protocols quote through a generic constant-product fallback with hard-coded fees (25/25/10/25 bps) — not official protocol math. They are fail-closed for discovery (`crates/arb-ingest/src/decoders.rs:274` returns `None`), so no live edge can reference them today; instruction builders and hydration accounts exist but are unreachable in the live loop.
- **Required Fix Before ANY Enablement**: exact per-protocol math (DAMM v2 configurable-fee CP + position ranges; DBC 16-point piecewise curve; DLMM Pro continuous ticks; LaunchLab curve) + dynamic fee model + discovery + mainnet snapshot differential tests.
- **Status**: OPEN (P3). See `docs/active/audit/DEX_SUPPORT_MATRIX.md`.

### DEX-F-2026-11-22-N2: Raydium CLMM Discovery Fallback Fee Is a Structural Constant (P3, telemetry)

- **File**: `crates/arb-ingest/src/decoders.rs:262` (`decode_pool_info` CLMM arm).
- **Observed Behavior**: discovery-time struct carries `fee_bps: 25` placeholder. Operative fee is correct: catalog `feeRate` (`discovery.rs:517`) + millionths→bps conversion with fail-closed exclusion of `fee_on`/`dynamic_fee_info` pools (`decoders.rs:692–701`). Risk only if a CLMM pool reaches the graph without hydration — log/telemetry improvement.

### ARB-F-2026-11-22-N1..N3 (P3, audit notations)

- N1: DLMM two-pool engine unreachable + quote-time-only floor (cross-ref FL-F-2026-11-22-N2).
- N2: BF f64 `-ln` weights are pre-screen only; integer re-derivation mandatory for pricing/sizing/gating (`bellman_ford.rs:404`) — keep the boundary.
- N3: Sizing returns best-observed candidate (budgeted multi-basin search), never claimed as global optimum — accepted limitation.

### TXC-F-2026-11-22-N1: V1 Transport Toggles Pending Live Landing Observation (P3)

- **File**: `crates/arb-tx/src/transport/selection.rs` (env gates `SOLANA_TX_V1_DIRECT_ENABLED` / `SOLANA_TX_V1_JITO_ENABLED`, default OFF).
- **Behavior**: V1 compile is spec-complete and fuzz/differential-tested, but enabling submission requires an on-cluster observation of a V1 land on Agave ≥4.2.2 — not available in the sandbox. Gate is correct and fail-closed; do not toggle before evidence.

---

### HFT-F-2026-11-22-N1: Yellowstone gRPC Compile-Ready but Not Decision-Wired (P3)

- **File**: `crates/arb-ingest/Cargo.toml`, `crates/arb-ingest/src/dual_ingest.rs`, `crates/arb-ingest/src/quote_readiness.rs`
- **Severity**: P3
- **Status**: Open.
- **Evidence**: `yellowstone-grpc-client 17.0.0` is an optional dependency behind feature `yellowstone`; dual-ingest shadow harness (`dual_ingest.rs`) writes to an isolated `StreamingPoolCache` while HTTP polling (`MarketStateCache`) remains the trading source of truth; `QuoteReadinessContext.require_bank_identity` is the designed promotion gate for bank-id-bearing transports. Greedy client library exists; no wire-in or sustained A/B run.
- **Risk if ignored**: none today (conservative direction). Risk only if streaming source were switched without measured A/B: silent data-path regression.
- **Required action**: Run sustained dual-ingest A/B; promote streaming as decision source only with measured evidence; then wire `require_bank_identity=true` end-to-end.
- **Accepted criteria**: recorded A/B report + parity run + no regression in landing latency before any switch.

### HFT-F-2026-11-22-N2: No Ingest-Side Decode Histogram (P3)

- **File**: `crates/arb-ingest/src/helius_ws.rs`, `crates/arb-metrics/src/lib.rs`
- **Severity**: P3
- **Status**: Open.
- **Evidence**: Stage ladder begins histogramming at supervisor stages; ingest→decode leg reaches the ladder only as inherited `t_ingest_to_quote_us` (not a registered histogram in `arb-metrics`).
- **Required action**: Add `INGEST_DECODE_LATENCY_US` histogram recorded at the WS apply path.
- **Accepted criteria**: metric registered + recorded + boundary test.

### HFT-F-2026-11-22-N3: Criterion Bench Coverage Limited (P3)

- **File**: `crates/arb-ingest/benches/`, `crates/arb-math/benches/`
- **Severity**: P3
- **Status**: Open.
- **Evidence**: Only `dual_ingest_bench.rs` and `cpmm_bench.rs` exist; hydration batch fan, route discovery and exact-quote hot loops have no benches.
- **Required action**: Add benches before any optimization; baseline-first policy is binding.

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
