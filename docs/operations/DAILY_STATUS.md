# DAILY_STATUS — SOLANA_FLASHLOAN_ARBITUGE

## Date and repository state

* **Date:** 2026-11-17 (session ongoing)
* **Branch:** `vibe/ws-provider-audit-0fcd5e` → PR #70
* **Commit:** batch-17 commit (this push) — fix(economics): wire recent_landed_cu to confirmed landings + live/hardcode pricing exhaustive audit — 454/495
* **Previous commits:** `642a548` (batch 5), `3576f13` (batch 4), `e5a56c8` (batch 3), `924bdaa` (batch 2)
* **Current phase:** Phase 14 (Observability) + Phase 15 (Controlled Production Readiness) — remaining checklist items
* **Verified readiness level:** LEVEL 2 — TEST VERIFIED (1207 tests passed, 0 failed). Not LEVEL 3+: 24h shadow soak BLOCKED, dual-provider WSS BLOCKED on free plans.

## In progress right now (not yet committed)

Nothing in flight — batches 1–4 complete, committed and pushed.

## What changed (this session, in order)

### Batch 1 — commit `337a458` (Domain 18 dashboard wiring + infra)

| Item | What changed | Files |
|---|---|---|
| /assets static route | Vite JS/CSS were 404 (router served only index.html). Added `assets()` handler with path-traversal rejection (`..`, `\`, unknown ext, canonicalize must stay inside `web/dist`) + 3 unit tests | `crates/arb-bot/src/web_api.rs` |
| start_dashboard.bat | Ready Windows launcher: checks cargo, skips if server healthy on 127.0.0.1:8080, builds release if binary missing, generates/prints random `WEB_ADMIN_PASSWORD` (respects .env), starts arb-bot, polls /health up to 120s, opens browser | `start_dashboard.bat` (new) |
| RPC quota-utilization table | Providers.tsx rewritten: per-provider OK/429/Errors/Quota/Cooldown/p50–p95 table from `rpc_pool.health_snapshot()` + Active WS subscriptions card | `web/src/components/Providers.tsx`, `control_service.rs` (`providers_health` in ControlDeps + `subscriptions_active` in providers()) |
| Observability stats | New `ObservabilitySnapshot` (6 counters) filled in `publish_control_state` from arb-metrics getters; 6 Stat cards in Overview | `control_service.rs`, `supervisor.rs`, `Overview.tsx`, `api.ts` |
| Gross profit column | `OpportunityRecord.gross_profit_lamports` = expected_output − borrow, computed in `record_opportunity()`; column in Opportunities.tsx | `control_service.rs`, `supervisor/strategy.rs`, `Opportunities.tsx` |
| Fork-invalidation counter | `FORK_INVALIDATIONS_TOTAL` + `inc_fork_invalidations(n)`; SLOT_DEAD path counts invalidated slots+banks; entry-update parent path counts removed snapshots | `arb-metrics/src/lib.rs`, `arb-ingest/src/lib.rs` |

### Batch 2 — commit `924bdaa` (execution provenance + stream gaps)

| Item | What changed | Files |
|---|---|---|
| Execution sim/size/CU provenance (checklist 630–632) | `ExecutionExtras` + `ExecutionRecord` gained `sim_ok: Option<bool>`, `tx_size_bytes: Option<u64>`, `cu_consumed: Option<u64>` (omitted from JSON when absent — old clients compatible). Preflight sim CU captured in dispatch (primary + RpcPool fallback paths); on-chain `computeUnitsConsumed` from landed meta overrides preflight estimate. Flows through `TradeSummary`/`DroppedTradeSummary` → `/api/executions`. Executions.tsx: Sim (badge ok/fail) / Tx Size (B) / CU Consumed columns | `control_service.rs`, `telegram.rs`, `supervisor.rs`, `supervisor/dispatch.rs`, `api.ts`, `Executions.tsx` |
| send_failed visibility | Bundle dispatch transport error now records an execution record (`send_failed`) + `record_execution_result(false)` so every execution attempt is explainable on the dashboard | `supervisor/dispatch.rs` |
| Stream slot-gap counter (checklist 620) | `SlotIntegrity` gained `gaps_detected` + bounded `counted_gap_ends` dedup. Gap = slots never observed between consecutive slot notifications, walking the known parent chain to narrow the window; only forward jumps past the high-water mark count; late/reordered notifications never inflate. Exported via `stream_slot_gaps_total` (+delta watermark `reported_slot_gaps` in IngestClient). Displayed in Overview as "Stream slot gaps" + `ObservabilitySnapshot.stream_gaps_total` | `slot_integrity.rs`, `arb-ingest/src/lib.rs`, `arb-metrics/src/lib.rs`, `control_service.rs`, `supervisor.rs`, `api.ts`, `Overview.tsx` |
| Gap tests (5 new) | consecutive→0; parent-chain jump counts missing once (dedup across processed/confirmed/finalized); forward jump without parent uses high-water mark; late/reordered notifications never inflate; first observed slot never counts | `slot_integrity.rs` tests |

### Batch 3 — commit `e5a56c8` (observability + retention)

| Item | What changed | Files |
|---|---|---|
| 575 Simulation Latency | `SIMULATION_LATENCY_US` Prometheus Histogram (buckets 1k–5M µs) + `observe_simulation_latency_us`; measured around `simulate_async` (primary) and `RpcPool::simulate_transaction` (fallback) in dispatch; debug log with measured µs; exported via /metrics | `arb-metrics/src/lib.rs`, `supervisor/dispatch.rs` |
| 634 Structured JSON logs | `LOG_FORMAT=json` gate in `init_event_logger()`: both stdout and file layers switch to `.json().flatten_event(true)`; default text format unchanged; Err-fallback branch covered | `main.rs` |
| 638 30-day retention | `Store::prune_older_than_days(u32)` (default no-op) + SqliteStore impl deleting fills→outcomes→dispatch_intents→attempts by `created_at < datetime('now', ?1)` (bound param, no string concat); periodic caller every 6h in run.rs via `DATA_RETENTION_DAYS` (default 30); test `retention_prune_deletes_old_rows_and_keeps_recent` (40-day-old backdated rows deleted, recent kept, second call = 0) | `arb-store/src/lib.rs`, `supervisor/run.rs` |
| 584/585 CPU/RSS metrics | `process_cpu_percent` Gauge + `process_rss_bytes` IntGauge; sampler task every 30s reads `/proc/self/stat` utime+stime (cpu % over interval via tick delta, CLK_TCK=100) and `/proc/self/status` VmRSS (Linux `cfg`-gated; no-ops elsewhere); surfaced through `ObservabilitySnapshot` → Overview cards "RSS (MB)" and "CPU %" | `arb-metrics/src/lib.rs`, `supervisor/run.rs`, `control_service.rs`, `supervisor.rs`, `api.ts`, `Overview.tsx` |

### Batch 4 — checklist 563, 541, 624 (same session)

| Item | What changed | Files |
|---|---|---|
| 563 Jito Rejection test | Evidence updated: the block-engine rejection branch is already covered by 6 mock-engine contract tests (HTTP 400 carries status+reason as Err; 200-without-result is not landing evidence; acceptance returns bundle id; empty/oversized local rejection; unreachable = network error) | `crates/arb-land/tests/bundle_rejection_tests.rs` (existing, evidence corrected) |
| 541 Failure Injection | Evidence updated: network/provider failure injection exists via mock servers — quota exhaustion (429/Retry-After/cooldown), mock block engine rejection, failover circuit breaker | existing tests, evidence corrected |
| 624 Per-hop exact quotes | `OpportunityRecord.hop_quotes: Vec<HopQuoteRecord>` (protocol/pool/amount_in/amount_out) derived from `hop_inputs`+`hops` at the meteora-dlmm-3leg record site; omitted from JSON when the pipeline doesn't supply per-hop amounts; "Per-hop quotes" column in Opportunities.tsx with full-pool tooltip | `control_service.rs`, `supervisor/strategy.rs`, `api.ts`, `Opportunities.tsx` |

**Checklist after batch 4: 436 / 495 — 59 remaining.**

### Batch 5 — checklist 542, 222, 504 (same session)

| Item | What changed | Files |
|---|---|---|
| 222 Provider recovery cycle | 2 new tests on `HeliusWsClient` endpoint election: 429 backoff → rotation to healthy endpoints → cooldown expiry → return to the recovered endpoint while still-limited ones stay excluded; and all-endpoints-backoff elects the earliest-expiring cooldown + updates `current_idx` | `crates/arb-ingest/src/helius_ws.rs` (tests) |
| 542 Concurrency/lock contention | New test suite with observable invariants (not just no-panic): 8 threads hammer `SlotIntegrity` (bounded wall clock, finalized ≤ highest, gap accounting still counts after contention); concurrent `rotate_endpoint`/`track_account` on `HeliusWsClient` (valid indices, no stall); 12 concurrent calls against 3 mock 429 endpoints → all fail closed, and after cooldown a further 3 calls issue **zero** new HTTP requests (counted at the mock origins) | `crates/arb-ingest/tests/concurrency_tests.rs` (new) |
| 504 Re-dispatch decision | Design decision documented as D-18 in DECISIONS_AND_CONFLICTS.md: no automatic same-slot re-dispatch of a dropped Jito bundle (edge decay, duplicate-landing risk, tip ladder covers escalation, reconciliation gate requires on-chain evidence); current dispatch behaviour already matches the contract | `docs/audit/DECISIONS_AND_CONFLICTS.md` |

### Batch 6 — checklist 637 + Domain 16 reviews 578/580/581/586 (same session)

| Item | What changed | Files |
|---|---|---|
| 637 Daily aggregated report | New pure module `daily_report.rs`: UTC-date civil-from-days (no dependency), aggregation of durable today-PnL (`Store::pnl_daily("today")`), rejection-ring top-5 reason ranking, stream-gap/opportunity counters, landing-rate bps with clamp; rendered text + JSON file `data/daily_report_<date>.json`. Runtime: emitted at the first strategy-loop visit after the UTC date flips (date check every 10 min) in run.rs; Telegram via `send_message` (no-ops unconfigured). 3 unit tests incl. leap-year/rollover boundaries and an off-by-one the leap assertion caught against an independent Python check | `crates/arb-bot/src/daily_report.rs` (new), `lib.rs`, `supervisor.rs` (`emit_daily_report`), `supervisor/run.rs` |
| Domain 16 review | New `docs/audit/PERFORMANCE_HOT_PATH_REVIEW.md` — line-level review of the hot path: 578 clones (26 census: ~20 rejection-path-only, 1 per-candidate, 1 cached → no per-pool-tick clone on the success path), 580 lock contention (parking_lot, no await inside, fixed slot_clock→rtt order — corrects the earlier std-Mutex evidence), 581 landing-tracker bounded lifetime (20×400 ms), 586 no unbounded_channel anywhere + Telegram bounded-1000 drop policy. 579/582/583 sharpened to PARTIAL pending measurement runs (no fake optimization) | `MASTER_ENGINEERING_CHECKLIST.md`, `docs/audit/PERFORMANCE_HOT_PATH_REVIEW.md` (new) |

**Checklist after batch 6: 443 / 495 — 52 remaining.**

### Batch 7 — checklist 647 + 637 landed previously (same session)

| Item | What changed | Files |
|---|---|---|
| 647 TOML/ENV defaults sweep | New integration test pinning the shared economic defaults across every shipped config layer: 6 critical `strategy` keys (`min_net_profit_abs=50_000`, `min_net_profit_bps=5`, `max_price_impact_bps=40`, `max_state_age_slots=2`, `contingency_buffer_lamports=10_000`, `hop_slippage_bps=10`) must never drift in any of base/example/development/production/staging (missing keys tolerated only in overlays); `graph_max_age_slots >= max_state_age_slots` coherence per file; and the in-code fallback is read from the real `AppConfig::default_safe().strategy` (not copied constants) and asserted equal to base.toml | `crates/arb-config/tests/toml_env_consistency_tests.rs` (new) |

**Checklist after batch 7: 444 / 495 — 51 remaining.**

### Batch 8 — checklist 609 (same session)

| Item | What changed | Files |
|---|---|---|
| 609 Upgradeable-program risks | New risk register: impact matrix of program upgrades on decode/fee/dispatch, per-program register for every execution-path program ID with expected upgrade-authority behavior and risk levels, special ArbExecutor case (source builds locally but **no retained deployment evidence → UNVERIFIED** — executor-dependent paths stay in shadow mode), standing in-code mitigations, and an operational re-verification duty (upgrade-authority + deployment-slot diff check) binding a change to a re-run of the differential suite | `docs/audit/PROGRAM_UPGRADEABILITY_RISKS.md` (new) |

**Checklist after batch 8: 445 / 495 — 50 remaining.**

### Batch 9 — checklist 656 (same session)

| Item | What changed | Files |
|---|---|---|
| 656 Magic constants | Three Kamino freshness bounds extracted from the hot path into named, documented workspace constants: `KAMINO_ZAP_FRESHNESS_TOLERANCE_SLOTS` (1000, was inline `<= 1000` twice in flash-liquidity fallback matching), `KAMINO_RPC_SNAPSHOT_MAX_AGE_SLOTS` (1500, was the raw RPC-snapshot substitution bound), `KAMINO_SUPPLY_VAULT_MAX_AGE_SLOTS` (5000, was a function-local `const` documented ~30 min). No behavior change — same values, single source of truth | `crates/arb-constants/src/lib.rs`, `supervisor/execution.rs`, `supervisor/run.rs` |

**Checklist after batch 9: 446 / 495 — 49 remaining.**

### Batch 10 — checklist 653 (same session)

| Item | What changed | Files |
|---|---|---|
| 653 Dead code | 12 of 17 `allow(dead_code)` sites resolved by deleting the dead item itself (JitoLander.max_retries field; bitquery OAuth unused Deserialize fields; `decode_token_amount` moved into the tests module — only its test called it; `decode_account_data` in route_worker; `RouteArchitectWorker` disconnected instance side `rpc_pool`/`new`/`start_worker_loop`/`stop` — already-disabled loop, no callers; unused arb-tx test helper) + 5 stale annotations removed where a real caller exists (daily_report.rs×5, supervisor jito_lander/simulator, registry test fn). Remaining 3 are documented: `tests/common/mod.rs` test-harness-wide allow + 2 inline-justified in dispatch.rs | `arb-tx/src/lib.rs`, `arb-land/src/lib.rs`, `arb-ingest/src/bitquery.rs`, `arb-ingest/src/registry.rs`, `arb-engine/src/route_worker.rs`, `arb-bot/src/supervisor.rs`, `arb-bot/src/daily_report.rs` |

**Checklist after batch 10: 447 / 495 — 48 remaining.**

### Batch 11 — checklist 570, 571, 572 (same session)

| Item | What changed | Files |
|---|---|---|
| 570 State latency | New Prometheus histogram `state_processing_latency_us` measured in `strategy_cycle`: from cycle entry (PumpSwap promotion boundary) to pools-snapshot ready | `arb-metrics/src/lib.rs`, `supervisor/strategy.rs` |
| 571 Discovery latency | New histogram `route_discovery_latency_us`: section-2 start (graph build) through all discovery classes (base cycles, DLMM cross-pool, Bellman-Ford, multi-hop, 3-leg, boost) until candidates merged | same |
| 572 Exact-quote latency | New histogram `exact_quote_latency_us` measured around the two exact-quote engines: `find_cross_pool_dlmm_cycles` (size search) and `build_three_leg_meme_pipeline` (chained quotes) — per invocation | same + `supervisor/run.rs` |

**Checklist after batch 11: 450 / 495 — 45 remaining.**

### Batch 12 — checklist 654 (same session)

| Item | What changed | Files |
|---|---|---|
| 654 Duplicate RPC calls | New inventory audit of every runtime RPC call site: startup-only / freshness-gated (vault hydration >100 slots) / batched+deduped (ATA probes) / cached (blockhash 3 s) / conditional fail-closed recovery (Kamino raw fallback) / unified client (`RpcPool` + process-wide `shared_dispatch_client`). Verdict: no duplicate-call patterns remain on the live per-tick path; `write_version: 1` on HTTP-sourced snapshots documented as an ordering token with real slot/owner provenance | `docs/audit/RPC_CALL_AUDIT.md` (new) |

**Checklist after batch 12: 451 / 495 — 44 remaining.**

### Batch 13 — checklist 651 progress (1/7 files) (same session)

| Item | What changed | Files |
|---|---|---|
| 651 — arb-types/lib.rs split (1678 → 102-line lib) | Module split by responsibility: `primitives.rs` (Lamports..MintId, UsdMicros), `dex.rs` (DexProtocol, TokenProgramKind, Raydium/PumpSwap versions/intents, PoolExecutionAccounts), `pool.rs` (PoolKind/Curve/Snapshot, PlannedHop, CostBreakdown, SizedOpportunity, CLMM/DLMM/PumpFun state, SwapQuote, ArbRoute, HopExecutionPlan), `execution.rs` (tip market, posteriors, TipDecision/TipPlan, InflightStatus), `error.rs` (Result + ArbError). `lib.rs` keeps `pub use submodule::*;` glob re-exports so every historical `arb_types::X` path is preserved with zero downstream churn; submodules use explicit `use super::<sibling>::*;` (proven: circular glob `use super::*` does not resolve). Fixed 6× E0425 (`error.rs` missing primitives import) + pruned 7 unused-import warnings. Full gate: fmt, clippy `-D warnings` clean, `cargo test --workspace --all-targets` = **1195 passed / 0 failed**. **Item stays `[ ]` PARTIAL** — 6 files remain >1000 lines | `crates/arb-types/src/{lib,primitives,dex,pool,execution,error}.rs` |

**Checklist after batch 13: 451 / 495 — 44 remaining** (651 stays `[ ]` PARTIAL; no counter change without complete evidence).

### Batch 14 — checklist 651 progress (2/7 files) (same session)

| Item | What changed | Files |
|---|---|---|
| 651 — arb-ingest/registry.rs split (2861 → 4 files) | `fetcher.rs` (300): `PacedAccountFetcher` + shared `pub(crate) static CONSECUTIVE_RPC_FAILURES` (moved out of decoders where it sat unused). `decoders.rs` (1532): all pool/vault/fee/state decoders, PDA derivation, fetch helpers + both test modules. `registry.rs` (203): `RegisteredPool` + `MultiDexRegistry` (registration/mainnet_default) + `pub use crate::{decoders::*, fetcher::PacedAccountFetcher};` re-exports. `hydration.rs` (861): `batch_fetch_accounts` + `fetch_live_pool_states` + `dlmm_mint_decimals` in a second `impl MultiDexRegistry` block; registry struct fields widened to `pub(crate)`. Zero downstream path churn (lib.rs re-export list unchanged). Full gate: fmt, clippy `-D warnings` clean, workspace tests **1195/0**. **651 stays `[ ]` PARTIAL — 5 files remain** | `crates/arb-ingest/src/{registry,fetcher,decoders,hydration}.rs` |

**Checklist after batch 14: 451 / 495 — 44 remaining.**

### Batch 15 — checklist 528 + 529 (runtime stack/data guards) (same session)

| Item | What changed | Files |
|---|---|---|
| 528 Runtime Stack Requirements | External research: Agave `execution_budget.rs` (anza-xyz/agave master, fetched raw) gives authoritative `MAX_INSTRUCTION_STACK_DEPTH=5` (SIMD-0268: 9) and `MAX_COMPUTE_UNIT_LIMIT=1_400_000` (matches current CU config). Added `arb_constants::{MAX_INSTRUCTION_STACK_DEPTH, MAX_INSTRUCTION_STACK_DEPTH_SIMD_0268}` with source+date; executor CPI chain (payer→arb-executor→DEX→SPL token = 4 frames) enforced by compile-time `const assert!` + `validate_executor_cpi_stack_depth()` wired as Invariant 0 into `assemble_flash_funded_three_pool_cpi_instructions`; new error variant `FlashArb3LegError::CpiStackDepthExceeded`. Tests: `executor_stack_depth_budget_is_satisfied` | `arb-constants/src/lib.rs`, `arb-tx/src/flash/arb_executor.rs`, `dlmm_3leg_flash_arb.rs` |
| 529 Account Data Limits | Same Agave source: `MAX_LOADED_ACCOUNTS_DATA_SIZE_BYTES=64MiB`; added constant. New `apply_runtime_limit_guards` in fork_sim wired into the sim pipeline before dispatch: fail-closed (success=false + explicit error) when `loadedAccountsDataSize > 64MiB`, and when size==0 && inner==0 with non-empty logs (unreproducible provenance); never rescues an already-failing sim. 3 tests added. Stale evidence path `opportunity.rs:1337` corrected to fork_sim.rs | `arb-constants/src/lib.rs`, `arb-sim/src/fork_sim.rs` |

**Checklist after batch 15: 453 / 495 — 42 remaining.**

### Batch 16 — checklist 531 (per-DEX CU regression benchmarks) (same session)

| Item | What changed | Files |
|---|---|---|
| 531 CU regression benchmarks | New integration test `cu_regression_benchmarks.rs` (7 tests): **(A)** `cu_estimate()` ranking table pinned literally for all 11 `DexProtocol` variants — any silent estimate drift fails the build; explicit exhaustive-variant coverage test; per-DEX <= 10% of runtime ceiling. **(B)** DLMM 3-leg flash CU model reproduced and pinned literally (`estimate_minimal_cu(true,2) + CU_PER_BIN_ARRAY×bins` = 450+28,000+70,000+30,000 = 128,450 at the minimal 2-bin plan; 4/8-bin pins too) and envelope-checked against the **retained mainnet measured evidence** 76,954–82,702 CU (`reports/phase15_2/mainnet_simulation_evidence.json`, `PHASE15_2_FINAL_REPORT.md`): estimate must cover the measured floor, stay <= 3x the measured ceiling, and <= 1,400,000 CU. **(C)** Route aggregation (`saturating_add` mirror of adapters.rs) with the heaviest hop mix (1–3 hops × DAMM v2) never exits `MAX_COMPUTE_UNIT_LIMIT` (1,400,000, Agave `execution_budget.rs`, verified 2026-11-17), and `calculate_dynamic_cu_limit` clamps exactly at the ceiling for 1.3M / 1.4M / u32::MAX while preserving the 15% buffer below the threshold. ESTIMATED vs MEASURED explicitly separated per evidence rules. Gates: fmt clean, clippy `-D warnings` clean, workspace **1206 tests passed / 0 failed** | `crates/arb-engine/tests/cu_regression_benchmarks.rs` (new), `MASTER_ENGINEERING_CHECKLIST.md` |

**Checklist after batch 16: 454 / 495 — 41 remaining.**

### Batch 17 — profit/capture/per-hop/ATA/hardcode pricing audit + recent_landed_cu repair (same session)

| Item | What changed | Files |
|---|---|---|
| **VERIFIED_BUG (disconnected)**: `Supervisor.recent_landed_cu` | The field consumed by `adaptive_cu_price` (p75+25% dynamic CU price) was never populated anywhere — `recent_landed_cu: Vec::new()` fed every call ⇒ priority price always fell back to the 100,000 µlam/CU constant. Fixed: field is now `Arc<Mutex<Vec<u64>>>`; on `LandStatus::Landed` the encoded `cu_price` (what actually cleared the auction for a confirmed bundle) is pushed via `arb_land::record_landed_cu_price` into a bounded 64-sample window (`RECENT_LANDED_CU_WINDOW`, oldest dropped); consumers snapshot `lock().clone()` (`opportunity.rs`, `run.rs`). The governing ceiling remains `cap_cu_price_by_profit` (profit-share cap) so the escalation can never bid beyond the approved edge share. Test: `landed_cu_price_window_is_bounded_and_order_sensitive` | `arb-land/src/lib.rs`, `supervisor.rs`, `supervisor/dispatch.rs`, `supervisor/opportunity.rs`, `supervisor/run.rs` |
| Exhaustive pricing/hardcode enumeration (user request) | `docs/audit/FINANCIAL_LOGIC_AUDIT.md` §7–9: §7.1 live-priced values table (flash fee from reserve, per-DEX fees from protocol accounts, Token-2022 transfer fees + tripwire, Jito tip via TipOracle 5s freshness fail-closed, competition factor, live SOL price fail-closed, flash liquidity); §7.2 justified chain/runtime constants (5,000 base fee, 1.4M CU limit, Agave depths, Jito tip accounts, ATA PDA); §7.3 ranking-model constants (regression-pinned by 531, superseded by measured_cu); §7.4 non-production hardcoded values (180/150 SOL price, 15k lamports — replay/optimizer/isolated pipelines only); §7.5 ATA accounting (idempotent create + in-tx close rent refund + preexisting-ATA fail-safe probe + sync_native); §8 full capture chain (11 stages from live update to dispatch with per-hop cost attribution); §9 open-source best-practice comparison (BoltTx, RPCFast, solbotrank, devrels, yavorovych: live-derived priority, edge-share cap, failure costs in EV, Jito only for atomicity, depth-based sizing) — all matched or exceeded | `docs/audit/FINANCIAL_LOGIC_AUDIT.md` |
| Checklist item 493 evidence | Annotated with the disconnection finding + repair | `MASTER_ENGINEERING_CHECKLIST.md` |

**Checklist after batch 17: 454 / 495 — 41 remaining (item 493 evidence updated; no new items).**
Gates: fmt clean, clippy `-D warnings` clean, workspace **1207 passed / 0 failed**.

### Checklist ledger

* Marked `[x]` this session: 618, 619, 621, 622, 626, 620, 630, 631, 632, 575, 584, 585, 634, 638, 563, 541, 624, 542, 222, 504, 637, 578, 580, 581, 586, 647, 609, 656, 653, 570, 571, 572, 654.
* In progress: 651 (2/7 files split).
* Marked `[x]` batch 16: 531.
* Batch 17: repaired disconnection behind item 493 (recent_landed_cu); exhaustive pricing audit in FINANCIAL_LOGIC_AUDIT.md §7–9.

## Problems discovered (this session)

| Severity | Issue | Root cause | Status |
|---|---|---|---|
| P2 | Vite dashboard JS/CSS 404 | `/api` router had no `/assets` route | **FIXED/TESTED** (`337a458`) |
| P3 | Gap-detection semantics | First implementation counted raw slot jumps; parent-chain evidence requires walking ancestors so an authoritative parent link narrows the unobserved window | **FIXED** — test `late_and_reordered_notifications_never_inflate_gaps` caught it (expected 38, not 39) before push |
| S-1 (carried) | User's real API keys + funded keypair were pasted in chat earlier | — | Rotation still advised |

## Remaining work

* **P1:** None open.
* **P2:** None open. — all three P2 items (542, 222, 504) completed in batch 5.
* **P3:** Domain 16 remainder now PARTIAL-measurement-only (579, 582, 583); Domain 11/19: 570–572 latency split, 521 periodic CU comparison, 528/529/531, TOML/ENV defaults sweep, files >1000 lines, dead code, magic constants; 321 layout version, 375 stream discovery, 609 upgradeability record.
* **P4:** Docs consolidation.
* **BLOCKED (external):** 30-min data-plane session, dual-provider WSS, quota compliance proof, live Jito bundle submission, 24-hour shadow soak (run-shadow.sh ready), Solami Mirage protobuf parser (G5, provider quota).

## Acceptance gate

* **Passed:** unit/integration suites (1192), clippy -D warnings, fmt, web build, checklist evidence updates.
* **Failed:** none.
* **Not tested:** differential quote parity for CLMM tick-crossing/multi-bin (no oracle coverage — documented PARTIAL).
* **Blocked:** 24h shadow soak, dual-provider WSS proof, live Jito bundle, quota-compliance proof, Solami Mirage parser (free-plan limits).

## Next single priority

Batch 7 (Domain 19/11 remaining actionable P3s): file-splitting review for the 7 files >1000 lines (651) starting with the biggest (registry.rs 2861), TOML/ENV default-consistency sweep (647), magic-constants extraction (656). All offline-actionable; after them only externally BLOCKED evidence and measurement-gated items remain.

## Capital safety

* **Live signing:** disabled by default; requires explicit `LIVE_EXECUTION_ENABLED=true` + non-placeholder keypair (fail-closed gate in main.rs).
* **Live broadcasting:** only in Live mode after the gate; not enabled in this session.
* **Financial transactions sent this session:** none (all work offline: code, tests, docs).
* **Current mode:** Read-only development + shadow-mode evidence from the 2026-11-17 rehearsal (one authorized earlier live flash-loan tx landed finalized: sig `5w9k6SeA…gX`, slot 470365241).
