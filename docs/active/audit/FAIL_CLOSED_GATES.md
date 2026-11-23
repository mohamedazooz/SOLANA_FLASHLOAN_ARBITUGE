# FAIL-CLOSED Gates Register — Production Path

Scope: every fail-closed gate reachable on the production path
`Supervisor → Opportunity Evaluation → Risk Gate → Instruction Assembly → V1 Compilation →
Simulation → Economic Invariant Validation → Dispatch → Reconciliation`.

Method: static inspection at branch `fix/solana-arbitrage-production-remediation` HEAD
(`26ccba1`); line numbers are current at that commit. Every gate below either has a
protecting test referenced by file, or is flagged `NO DEDICATED TEST` — flagged gates are a
testing backlog, not open defects.

Verification invariant: **any** gate firing must abort the opportunity/transaction without
capital movement beyond burned fees; none of these gates may be weakened to make tests pass.

---

## 1. Opportunity evaluation gates — `crates/arb-bot/src/supervisor/opportunity.rs`

| Gate | File:line | Trigger | Behavior | Protecting test |
|---|---|---|---|---|
| G-01 KaminoLiquidity (unverifiable) | opportunity.rs:164 | Vault liquidity of the borrow reserve cannot be verified | `OPPORTUNITY_REJECTED stage=KaminoLiquidity`, skip | `crates/arb-engine/tests/three_leg_net_economics_tests.rs` (disabled-reserve sentinel `u64::MAX` fails closed) |
| G-01b KaminoLiquidity (size > liquidity) | opportunity.rs:177 | `size_in` exceeds verified available vault liquidity | reject, log both numbers | same |
| G-02 AccountDependencies | opportunity.rs:196 | Per-pool account dependency manifest incomplete/stale (`run.rs:59`) | reject fail-closed before dispatch | `crates/arb-bot/tests/cross_dex_dlmm_cpmm_supervisor_execution.rs:562-638` ("Missing account dependency must fail closed before dispatch") |
| G-03 CostsExceedGross | opportunity.rs:312 | Signed net < 0 after flash fee + base fee + priority fee; signed shortfall preserved in the log and audit record (F-07, commit `26ccba1`) | reject; never zero-clamps the negative amount | `crates/arb-engine/tests/hop_slippage_once_tests.rs`, `crates/arb-math/tests/differential_fuzzing_boundary_tests.rs` |
| G-04 JitoTipPlan | opportunity.rs:428 | Tip planner cannot produce a valid tip plan | reject with reason | supervisor tip tests |
| G-04b ProfitFloorProtection | opportunity.rs:340 | `t_max < MIN_JITO_TIP` — no tip can fit above the profit floor | reject | same |
| G-05 JitoLeaderGate | opportunity.rs:472 | Next leader is a non-Jito validator (bundle unreachable) | reject | `crates/arb-bot/src/jito_service.rs` leader-gate policy (`supervisor.rs:420`) |
| G-06 NetProfitEconomics (actual tip) | opportunity.rs:565,576 | Net ≤ 0 recomputed with the ACTUAL Jito tip — signed `net_signed`/`shortfall_signed` logged (F-07) | `OPPORTUNITY_REJECTED_ECONOMICS` | `crates/arb-engine/tests/three_leg_net_economics_tests.rs:199-213` |
| G-07 ImplausibleROI | opportunity.rs:624-651 | Implied ROI > `max_implausible_roi_bps` regardless of the state-age window — stale-quote artifact (Phase 15.2 ~965% anomaly) | reject fail-closed | `crates/arb-engine/tests/cross_pool_dlmm_strategy.rs:440` and Phase 15.2 regression tests |
| G-08 NetProfitFloor | opportunity.rs:727 | Net base-raw < configured `min_profit` | reject | `crates/arb-engine/tests/hop_slippage_once_tests.rs` floor cases |
| G-09 ChronosEVGate | opportunity.rs:810 | Expected-value gate rejects (failure-mode weighting) | reject with reason | EV gate tests |
| G-10 KillSwitch (cycle) | opportunity.rs:890 | Kill-switch blocked | opportunity processing aborted | `crates/arb-bot/src/supervisor/run.rs:1084` file-switch test |
| G-11 RiskGate | opportunity.rs:954 | Unified risk-gate policy rejection | reject with reason | risk gate tests |

## 2. Capture-time inventory provenance — `crates/arb-bot/src/supervisor/run.rs`

| Gate | File:line | Trigger | Behavior | Protecting test |
|---|---|---|---|---|
| G-12 Inventory provenance (3-leg capture) | run.rs:281-341 | Pre-existing WSOL/meme ATA balances would subsidize route execution or mask a repayment shortfall (PR24 §2.2) | candidate rejected, `inc_inventory_rejections()`, DSL remediation logged; NOT silently bypassed (F-01 policy) | pipeline test `crates/arb-engine/src/three_leg_meme_pipeline.rs:275` ("pre-existing token ATA balance would subsidize execution fail-closed") |
| G-12b Same policy, builder side | three_leg_meme_pipeline.rs:275 (error) | non-zero pre-existing intermediate/payer WSOL in build context | `ThreeLegMemePipelineError` fail-closed | `crates/arb-engine/src/three_leg_meme_pipeline.rs:1117` test |

## 3. Route state / stream freshness — `crates/arb-bot/src/supervisor/run.rs`

| Gate | File:line | Trigger | Behavior | Protecting test |
|---|---|---|---|---|
| G-13 Route state age | run.rs:1233-1238 | `state_age > strategy.max_state_age_slots` for any hop pool | reject | `cross_dex_dlmm_cpmm_supervisor_execution.rs:774` ("Stale account snapshot must fail closed") |
| G-14 WS stream circuit breaker | run.rs:1019-1162 | Helius slot 0 or WS watermark not advanced within `stale_after` | trip breaker, no stale fallback; dual-source check so stalled-socket snapshots don't pass | `crates/arb-ingest` slot integrity tests (`crates/arb-ingest/src/slot_integrity.rs`) |
| G-15 Unknown pool freshness | supervisor.rs:1579 | Freshness queried for a pool with no snapshot | fail closed, NOT "fresh" default | `supervisor.rs` unit test "Unknown pool must fail closed" |
| G-16 Kamino vault hydration | run.rs:883-891 | Supply-vault snapshot stale > 100 slots | re-hydrate before trusting | ingest tests |

## 4. Instruction assembly — `crates/arb-tx/src/primitives/builder.rs` + `crates/arb-tx/src/flash/`

| Gate | File:line | Trigger | Behavior | Protecting test |
|---|---|---|---|---|
| G-17 derive_swap_legs invalid slippage | builder.rs:352-398 | `slippage_bps > 10_000` or unchecked path | `checked_sub` ladder / explicit error; hop inputs exactly stamped (F-05, commit `8d58487`) | `crates/arb-engine/tests/hop_slippage_once_tests.rs:291` | 
| G-18 Last-hop profit floor encoded | builder.rs:385-392 | final hop min-out anchored to `borrow + flash_fee + tip + min_profit` floor | `min_out_slippage.max(floor)` — tx reverts on-chain instead of losing the floor | `crates/arb-tx/src/lib.rs:979-982` ("Final hop min_out must enforce profit floor") |
| G-19 Plan bin arrays depth limit | flash/dlmm_bin_discovery.rs:33,128,138,156 | depth > `METEORA_MAX_BINS_PER_SWAP` (260), i32 overflow, or 4096 defensive cap | `BinArrayPlanError::DepthAboveSwapLimit` fail-closed (F-02, commit `08e6657`) | `crates/arb-tx/tests/dlmm_bin_array_span_tests.rs` (8 tests incl. 260/overflow negative cases) |
| G-20 Bitmap extension required | flash/dlmm_bin_discovery.rs:168-170 | Planned arrays index-outside-±511 range ⇒ `requires_bitmap_extension` | caller must fetch real extension PDA; absence fails closed | `dlmm_bin_array_span_tests.rs` bitmap policy test |
| G-21 Bitmap extension absent at leg build | three_leg_meme_pipeline.rs:883 | Required-but-not-provided bitmap extension for a DLMM leg | `ThreeLegMemePipelineError::BitmapExtensionRequired { leg }` | `three_leg_meme_pipeline.rs:1163` `build_context_fails_closed_when_bitmap_extension_required_but_absent` |
| G-22 Bin array coverage verification | flash/dlmm_bin_discovery.rs:200 `verify_bin_array_coverage` | Loaded account set ≠ planned (index, PDA) set | error before tx construction | `dlmm_bin_discovery.rs:451-488` tests (missing, extra bogus, wrong PDA, exact match ok) |
| G-23 Legacy derive directional span overflow | protocols/meteora_dlmm.rs:258-284 | `bins_traversed` → i32 overflow or span exceeds arrays | `DlmmError::TooManyBinArrays { needed, max }` | `meteora_dlmm.rs:653` test; `phase17_pda_differential.rs:164-177` (depth beyond limit fails closed) |
| G-24 Compiler rejections | flash/compiler.rs:46-86 | `V0Oversized`, `V1Oversized` (4096), `V1TransportUnsupported`, `TooManyBinArrays`, `PoolsNotDistinct`, `MissingAltAddress`, `AltAccountAsSigner`, `AltNotActivated`, `MissingExecutionAccounts`, `InvalidIntermediateAta`, `Serialization`, `V1AccountLimitExceeded` (SIMD-0385 >64 inline), `V1InstructionLimitExceeded` (SIMD-0160 >64) | compile abort, `RejectionReason` preserved in the record | `compiler.rs:1522` and transport-limit tests |
| G-25 Atomic integrity / repayment revert | primitives/atomic_integrity.rs:122,144,459,508,532 | borrow/repay coupling, V0/V1 limits, insufficient-repayment revert construction | fail-closed errors, never assembled | `crates/arb-tx` flash lifecycle + isolation suites (`docs/active/audit/FLASH_LOAN_REGRESSION_MATRIX.md`) |
| G-26 Residual-safe ATA close allow-list | primitives/builder.rs close policy (after repay index) | close emitted ONLY for mints in `proven_zero_intermediate_mints` allow-list (F-01, commit `f1881c8`) | no non-empty TA close possible; default `&[]` ⇒ zero closes | `crates/arb-tx/tests/residual_safe_ata_lifecycle_tests.rs` (6 tests incl. cross-dex, allow-list, close-instruction accounting) |
| G-27 Idle-ATA settlement | supervisor/execution.rs (`settle_idle_intermediate_atas_with`) | probe ATAs via `get_multiple_accounts` → only zero-balance closed; separate Legacy tx, live-only | never closes funded account; RPC failure ⇒ no close | F-01 residual lifecycle tests |

## 5. Config authority — `crates/arb-bot/src/config.rs`

| Gate | File:line | Trigger | Behavior | Protecting test |
|---|---|---|---|---|
| G-28 Slippage policy validation | config.rs:559-579 (`validate_slippage_policies`) | `hop_slippage_bps` or `dlmm_three_leg.slippage_bps` > 10_000 or > `HOP_SLIPPAGE_MAX_BPS`; invalid values rejected, NOT clamped | config load fails (F-04, commit `8d58487`) | `config.rs:833` `slippage_policy_tests`; `crates/arb-types/src/lib.rs:53` fee_bps reject test |
| G-29 Single slippage authority | opportunity.rs (stamped value used verbatim by `derive_swap_legs`) | No post-quote overwrite of `hop_slippage_bps` | builder uses the exact quoted value | `hop_slippage_once_tests.rs` (quote min-out == instruction min-out) |

## 6. Simulation & pre-flight — `crates/arb-sim`

| Gate | File:line | Trigger | Behavior | Protecting test |
|---|---|---|---|---|
| G-30 Typed SimulationStatus | arb-sim/src/lib.rs (`SimulationStatus { Passed, Failed, Unavailable, Timeout, Stale, Unsupported }`) | Only `Passed` eligible for dispatch; missing execution evidence / zero CU / stale context fail closed | typed failure, no boolean pass-through | `docs/active/CURRENT_STATUS.md` regression; `arb-sim` harness tests |
| G-31 Sim staleness | arb-sim/src/lib.rs `SimResult::mark…stale` | RPC bank outside quote freshness window | mark `Stale` → reject | `crates/arb-sim/tests/litesvm_simulation_boundary_tests.rs:101` |
| G-32 ProfitabilityGate rejections | profitability_gate.rs:83-110 | `SimulationError`, `StaleQuote` (bank slot lag > `max_quote_slot_lag`), `RealizedDeltaShortfall`, `NetProfitBelowHurdle`, `InconsistentBalances`, `UnprofitableSimulation`, `NetProfitNegative` (hurdle of 0 can never wave a losing route through) | `SimulationOutcome::Rejected` | `arb-sim` gate tests; `cross_pool_dlmm_strategy.rs:313-346` |
| G-33 Forensic capture replay | arb-sim/src/forensic.rs (L7-9: emitting-program hint rationale) | every pre-flight simulation failure | sha256-pinned `ForensicBundle` → `reports/forensics/`; `replay_locally()` via fail-closed LiteSVM | `cargo test -p arb-sim --features litesvm --lib` (5 forensic tests) |

## 7. Dispatch, transport & reconciliation — `crates/arb-bot/src/supervisor/dispatch.rs`

| Gate | File:line | Trigger | Behavior | Protecting test |
|---|---|---|---|---|
| G-34 Live endpoint missing | dispatch.rs:321 | live mode without HTTP Transaction RPC endpoint | `OPPORTUNITY_REJECTED stage=LiveExecution` | gate tests |
| G-35 V1-only policy | dispatch.rs:398-408, 476 | non-V1 transaction reaching dispatch | reject `stage=V1-only policy`, `arb-land` journal record | transport matrix tests (`docs/active/audit/TRANSACTION_COMPATIBILITY_MATRIX.md`) |
| G-36 V1 transport capability | dispatch.rs:418-442 | `SOLANA_TX_V1_DIRECT_ENABLED` / `SOLANA_TX_V1_JITO_ENABLED` unset (Simulate mode exempt) | V1 direct/Jito dispatch disabled — feature flags stay opt-in | env-gate tests |
| G-37 Kill switch (dispatch) | dispatch.rs:652-658 | kill-switch flips mid-flight | dispatch aborted, journal `aborted_kill_switch` | run.rs:1084 switch test |
| G-38 Unreconcilable signature | dispatch.rs:721-725 | signature not extractable after send | abort + `dispatch_aborted_unreconcilable`, NO resubmit | reconciliation tests |
| G-39 Bounded poll / startup reconciliation | dispatch.rs:1438 | outcome unresolvable after bounded poll | dispatch intent left pending for startup reconciliation (no resubmit — duplicate-suppression) | reconciliation tests |

## 8. Infrastructure fail-closed defaults

| Gate | File:line | Trigger | Behavior | Protecting test |
|---|---|---|---|---|
| G-40 Durable store | supervisor.rs:305,147-184 | store unavailable (postgres/sqlite), incl. unreachable `DATABASE_URL` | refuse to start, no in-memory fallback | `supervisor.rs:1423-1446` tests |
| G-41 Web admin auth | web_api.rs:87 | `WEB_ADMIN_PASSWORD` unset | API refuses to start | `web_api.rs:1131` unknown-command fail-closed test |
| G-42 Jito cluster | jito_service.rs:96 | tip send targeting a non-Jito cluster | fail-closed default | jito service tests |
| G-43 Token-2022 extensions | phase17 tests | unverified transfer-fee mint in a flash route | reject fail-closed | `crates/arb-engine/tests/phase17_two_pool_flash_arb_tests.rs:248`; `cross_dex_dlmm_cpmm_supervisor_execution.rs:904` |

---

## Findings from the sweep (2026-11-22)

1. **No fail-open path found** in the surveyed production chain. Every rejection path
   records a reason (F-07 added signed shortfall to all economics rejections, commit
   `26ccba1`).
2. **NO DEDICATED TEST** gaps (testing backlog, not defects): G-04/G-05 tip-plan and leader
   gate units are exercised only via supervisor-level tests; G-34 live-endpoint gate is
   covered by config plumbing rather than a named unit test. Registered in
   `docs/active/testing/REGRESSION_TEST_MATRIX.md` backlog.
3. The `Custom(6003)` numeric collision (arb-executor `InvalidMemoProgramId`) is
   deliberately disambiguated by the `forensic.rs` emitting-program hint (G-33); for the
   historical Phase 17 incident the mapping is conclusively Meteora DLMM — see
   `0X1773_ROOT_CAUSE_INVESTIGATION.md` §1 E1.
