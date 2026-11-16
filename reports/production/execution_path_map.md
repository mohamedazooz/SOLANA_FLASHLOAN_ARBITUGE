# Execution Path Map

## 1. End-to-End Data Flow

This map traces every transition from market data observation through trade execution and settlement. Every stage documents: Input, Output, Data Owner, Freshness Requirement, Failure Behavior, Retry Behavior, Observability, Invariants, and Tests.

### 1.1 Yellowstone gRPC Ingestion → Account Decoding

| Stage | Input | Output | Data Owner | Freshness | Failure | Retry | Observability | Invariants | Tests |
|-------|-------|--------|------------|-----------|---------|-------|--------------|------------|-------|
| **Yellowstone gRPC Subscription** | gSSE subscription message | Stream of account update packets | Ingest layer | Slot of first update; `write_version` for ordering | Connection drop → resubscribe; generation change → rederive filters | Auto-resubscribe on EOF; max 3 generation changes per session | `messages_received`, `provider_lag`, `subscription_ready` | All accounts carry `owner` field; `write_version` monotonically increases per bank | Yellowstone integration tests with mock gSSE server |
| **Account Decoding** | Raw account data bytes | `PoolSnapshot` / `ClmmState` / `DlmmState` / `PumpFunState` | Ingest layer | `slot` field must be ≥ last seen slot; `write_version` must be ≥ last seen | Deserialization error → log + skip account; malformed data → skip | Best-effort: skip and continue; no backpressure on gRPC | `account_decodes_total`, `decode_errors`, `stale_slot_rejected` | Deserialization fixtures from real mainnet snapshots |
| **Pool Registry** | Decoded pool snapshots | In-memory pool map keyed by `pool_id` | Ingest layer | State age ≤ `max_state_age_slots` (configurable, default 2) | Stale snapshot → overwrite if newer; same slot lower write_version → drop | None (push-based from ingest) | `pool_freshness_agged`, `registry_size`, `total_pools_tracked` | Registry accepts only newer snapshots (slot, write_version tuple comparison) |
| **State Integrity** | Pool map | Validated pool map + integrity report | Ingest layer | All pools must have valid `fee_denominator != 0` and known protocol | Pool with invalid fee → marked unusable; pool missing required fields → dropped | Re-ingest on next gRPC update cycle | `stale_pools_purged`, `invalid_pools_rejected` | Every pool in registry has `is_fee_valid() == true` or is removed |

### 1.2 Provider Supervisor → Quote Engine

| Stage | Input | Output | Data Owner | Freshness | Failure | Retry | Observability | Invariants | Tests |
|-------|-------|--------|------------|-----------|---------|-------|--------------|------------|-------|
| **Provider Supervisor** | Provider health pings | Primary + fallback provider selection | Supervisor | `max_slot_lag` (default 8); `stale_after` (default 3s) | Provider fail → switch to fallback; all failures → open circuit | 3 failures → Open Circuit + 2s cooldown; then test fallback | `provider_health_status`, `provider_lag`, `provider_switches` | Primary must have `slot_lag ≤ max_slot_lag`; fallback must be distinct infrastructure | Provider health checks with simulated lag injection |
| **Quote Engine** | Pool map + route request | `SizedOpportunity` with profit calculation | Strategy engine | State age ≤ `max_state_age_slots` (default 2 slots) | Missing pool → route rejected; insufficient liquidity → route rejected | Re-quote on next data refresh | `quote_latency`, `quote_errors`, `routes_discovered`, `routes_rejected` | `net_profit ≥ min_net_profit_abs` (50,000 lamports) AND `net_profit ≥ min_net_profit_bps * amount_in` (8 bps) | Differential tests against Meteora SDK and Kamino reference |
| **Route Discovery** | Quote engine output | `ArbRoute` with hops | Strategy engine | Route must use pools with `slot` within max_state_age_slots | No valid route found → return None; repeated failures → circuit breaker | Route cache TTL (default 15s) | `routes_found`, `routes_by_protocol`, `route_depth_distribution` | Mint continuity: hop_i.output_mint == hop_{i+1}.input_mint; triangular cycle: final output mint == input mint |

### 1.3 Profitability Evaluation → Risk Engine

| Stage | Input | Output | Data Owner | Freshness | Failure | Retry | Observability | Invariants | Tests |
|-------|-------|--------|------------|-----------|---------|-------|--------------|------------|-------|
| **Profitability Evaluation** | `SizedOpportunity` | `ExecutionOpportunity` with risk gate result | Strategy engine | All snapshot slots within `max_state_age_slots` | Negative net profit → opportunity rejected; price impact > max → rejected | Re-evaluate on next data refresh | `profitability_evaluated`, `profitability_passed`, `profitability_rejected` | `net_profit_usd > 0` after ALL cost deductions; no double-counted fees | Independent re-calculation using second implementation |
| **Risk Engine** | `ExecutionOpportunity` | `RiskGateOutput: pass/fail + reason` | Risk config | All checks use current state; no stale data | Any check fails → gate denied; final check before dispatch | Kill switch → immediate deny; circuit breaker → cooldown | `risk_checks_passed`, `risk_denied_reasons`, `circuit_breaker_state` | Kill switch marker → all execution blocked; circuit breaker opens at 8 consecutive failures, closes at 3 successes in HalfOpen | Regression tests for every risk check scenario |

### 1.4 Execution Authorization → Transaction Builder

| Stage | Input | Output | Data Owner | Freshness | Failure | Retry | Observability | Invariants | Tests |
|-------|-------|--------|------------|-----------|---------|-------|--------------|------------|-------|
| **Execution Authorization** | `RiskGateOutput.pass` + `ExecutionOpportunity` | Signed transaction ready for simulation | Supervisor | Blockhash freshness ≤ 1 slot lastValidBlockHeight | Mode check: `mode == Live` gated by `ARB_LIVE`; safety floor: balance ≥ 50,000,000 lamports | Re-authorize if blockhash stale or mode changes | `execution_authorized`, `authorizations_by_mode`, `safety_floor_checks` | `balance ≥ 0.05 SOL` (50M lamports); blockhash `lastValidBlockHeight` ≥ current slot; no duplicate execution | Mode gate tests with shadow/simulate/live configurations |
| **Transaction Builder** | Authorized transaction | Fully built transaction with all instructions | Supervisor | Compute unit limit reflects actual route; all account PDAs verified | Missing account → build fails; invalid instruction → bail with error | Rebuild on next authorization cycle | `tx_built`, `tx_build_errors`, `instruction_counts_by_protocol` | All instruction accounts have correct writable/signer flags; program IDs match protocols; discriminators match expected values | Golden serialization fixtures; differential test against Agave implementation |
| **Simulation** | Built transaction | Simulation result (Ok/Failed) | Supervisor | Must use fresh blockhash; LiteSVM or RPC simulateTransaction | Simulation failed → re-authorize with fresh blockhash; max 3 attempts | LiteSVM local + RPC simulation; RPC error → fail closed | `simulations_passed`, `simulations_failed`, `simulation_latency` | Simulation result must be reproducible across runs with identical input | LiteSVM integration tests; RPC simulateTransaction idempotency tests |
| **Jito Submission** | Simulated transaction | Bundle ID from block engine | Jito service | Bundle submission within tip floor window | Submission error → retry with exponential backoff; max 5 attempts | Bundle deduplication: same tx → skip resubmission | `bundles_submitted`, `bundles_landed`, `bundles_failed`, `tip_bid_lamports` | Bundle ID must appear in `getInflightBundleStatuses` within 5 minutes | Jito block engine integration tests; bundle landing rate measurement |

### 1.5 Landing Tracking → Realized PnL

| Stage | Input | Output | Data Owner | Freshness | Failure | Retry | Observability | Invariants | Tests |
|-------|-------|--------|------------|-----------|---------|-------|--------------|------------|-------|
| **Landing Tracking** | Bundle ID from submission | `InflightStatus` (Pending/Landed/Failed/Invalid) | arb-land service | Status must be resolved within 5 minutes (block engine window) | Polling timeout → mark as Expired/Unknown | 20 attempts × 400ms poll interval (dispatch.rs) | `inflights_tracked`, `landing_success_rate`, `bundle_status_by_rung` | `Landed.slot` must be ≥ submission slot; `Failed` → some fees burned, no profit; `Invalid` → outside 5min window | Landing ledger entry with slot, status, evidence (true/false/none) |
| **Realized PnL** | Landed bundle + input capital | `UsdMicros` realized profit/loss | Ledger | Must record at confirmation; finalize after settlement | Reorg → adjust PnL; finalization after `finalized` block height | Auto-finalize after N confirmations (configurable) | `realized_pnl_total`, `win_rate`, `average_profit`, `average_loss` | `realized_net_profit_usd = gross_profit_usd - total_costs_usd`; never positive on failed landings | PnL reconciliation between simulated and realized; daily/hr loss limit checks |

## 2. Missing Validation Between Stages

### 2.1 Identified Gaps

1. **Quote → Route Continuity**: No explicit validation that `hop_inputs[i]` matches the expected output of `hop_i-1`. The route discovery builds routes independently, and the quote engine computes each hop in isolation.

2. **Risk → Execution Freshness**: Risk engine checks use `state_age_slots` from `SizedOpportunity`, but the execution authorization checks blockhash freshness independently. There is no shared freshness state between these stages.

3. **Simulation → Landing Expectation**: Simulation uses LiteSVM or RPC `simulateTransaction`, but landing tracking depends on block engine `getInflightBundleStatuses`. There is no explicit mapping from simulation result to expected landing probability.

4. **Cost Breakdown Double-Counting**: The `CostBreakdown` struct has individual fields (dex_fees, flash_fee, base_fee, priority_fee, tip, rent, expected_failure). Some of these may overlap with costs already accounted for in the route's `min_amount_out` calculation.

5. **Provider Generation Invariants**: After a Yellowstone generation change, all account filters must be re-derived. There is no explicit invariant checking that the provider supervisor detects generation changes and triggers a full re-filter derivation before resuming quote generation.

6. **Tip Budget → Net Profit Gate**: The tip oracle prices bids based on expected net profit, but the risk engine's `check_min_profit` checks `net ≥ min_abs (50,000 lamports) AND ≥ min_bps (8 bps of amount_in)`. These two checks use different bases and may conflict.

7. **Split Routing Capital Allocation**: When `maybe_split_allocation` splits a route into parallel pools, there is no invariant ensuring that the combined net profit of the split exceeds the original unsplit route's net profit after accounting for additional tip/priority fee costs.

8. **Blockhash → Simulation Gap**: The transaction builder uses a blockhash with `lastValidBlockHeight`, but simulation may use a different blockhash or LiteSVM state. There is no invariant ensuring simulation results correspond to the transaction that will be submitted.

9. **Kamino Borrow → Repay Index Consistency**: Kamino flash loan builder uses `borrow_instruction_index` defaulting to 2, but this must exactly match the index assigned during `build_kamino_flash_borrow`. There is no compile-time assertion linking these two.

10. **Meteora Bin Array Version Skew**: The `METEORA_BINS_PER_BIN_ARRAY=70` constant in `arb-constants` must match the on-chain `BINS_PER_ARRAY` in the deployed Meteora program. A mismatch silently targets wrong BinArrays.

## 3. Execution Path Summary

```
Yellowstone gRPC
  → Account Decoding → Pool Registry → State Integrity
  → Provider Supervisor (health, lag, failover)
  → Quote Engine (pool snapshots → quotes)
  → Route Discovery (graph construction → valid routes)
  → Profitability Evaluation (net profit calc + risk gate)
  → Risk Engine (circuit breaker, kill switch, loss limits)
  → Execution Authorization (mode gate, safety floor, blockhash)
  → Transaction Builder (instruction packing + PDA derivation)
  → Simulation (LiteSVM or RPC simulateTransaction)
  → Jito Submission (bundle + tip)
  → Landing Tracking (getInflightBundleStatuses poll)
  → Realized PnL (confirmation → settlement → finalization)
```