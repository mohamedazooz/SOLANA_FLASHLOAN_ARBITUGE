# Master Task Register

This register is the single authoritative ledger for engineering task tracking, execution status, and verification evidence across all roadmap phases.

Allowed Statuses: `NOT_STARTED` | `IN_PROGRESS` | `BLOCKED` | `PARTIALLY_COMPLETED` | `COMPLETED` | `VERIFIED` | `REJECTED`

---

## Phase 0: Baseline & Forensic Audit (VERIFIED)

```markdown
TASK_ID: TASK-P0-001
PHASE: Phase 0 — Baseline & Forensic Audit
PRIORITY: P0
STATUS: VERIFIED
OBJECTIVE: Inspect Git repository state, default branch, latest commit, PR status, CI workflows, and working tree cleanliness.
FILES: .git, Cargo.lock, .github/workflows/ci.yml
FUNCTIONS: N/A
CURRENT_BEHAVIOR: Working tree contained uncommitted working changes on top of merged PR #90.
ROOT_CAUSE: Previous development session was halted with dirty working directory before clean documentation reset.
REQUIRED_CHANGE: Verify commit d090123 as origin/main baseline; back up working changes to backup branch; create clean feature branch.
TEST_PLAN: Run `git status`, `git log`, `gh pr list`, and `gh run list`.
ACCEPTANCE_CRITERIA: Baseline commit recorded, working tree clean, CI passing on main, backup branch secured.
ACTUAL_RESULT: origin/main at d090123 (PR #90 merged); CI run 44705864527 passed; backup branch `backup/pre-existing-phase17-work` created; clean branch `feat/master-roadmap-documentation-reset` checked out.
COMMIT: d090123
PR: #90
REMAINING_RISKS: None.
NEXT_TASK: TASK-P0-002
```

```markdown
TASK_ID: TASK-P0-002
PHASE: Phase 0 — Baseline & Forensic Audit
PRIORITY: P0
STATUS: VERIFIED
OBJECTIVE: Verify full workspace compilation, crate dependencies, toolchain version, and compiler flags without behavior changes.
FILES: Cargo.toml, rust-toolchain.toml, crates/*/Cargo.toml
FUNCTIONS: N/A
CURRENT_BEHAVIOR: Unverified compiler behavior on fresh environment invocation.
ROOT_CAUSE: Need verified verification of rustc 1.99.1 and Cargo workspace before architectural work.
REQUIRED_CHANGE: Execute `cargo check --workspace` and audit all 14 crates + on-chain program.
TEST_PLAN: Run `rustc --version`, `cargo --version`, `cargo check --workspace`.
ACCEPTANCE_CRITERIA: Clean workspace compilation with 0 compiler errors.
ACTUAL_RESULT: Compilation succeeded in 23.59s across all 14 crates and 1 on-chain program on rustc 1.99.1 (edition 2024).
COMMIT: d090123
PR: #90
REMAINING_RISKS: None.
NEXT_TASK: TASK-P0-003
```

```markdown
TASK_ID: TASK-P0-003
PHASE: Phase 0 — Baseline & Forensic Audit
PRIORITY: P0
STATUS: VERIFIED
OBJECTIVE: Inventory all environment variables across the workspace and classify into Required, Optional, Unused/Obsolete, and Secret.
FILES: crates/**/*.rs
FUNCTIONS: std::env::var call sites
CURRENT_BEHAVIOR: Dozens of disparate environment variable reads scattered across crates without formal taxonomy.
ROOT_CAUSE: Organic evolution of features, canaries, and diagnostic flags over multiple development phases.
REQUIRED_CHANGE: Perform static AST/regex audit across all Rust files and publish exhaustive taxonomy in BASELINE_AUDIT.md.
TEST_PLAN: Run `scripts/audit_workspace.py` environment variable scanner.
ACCEPTANCE_CRITERIA: Every `std::env::var` usage identified and categorized with security implications documented.
ACTUAL_RESULT: 95 unique environment variables inventoried, categorized, and recorded in BASELINE_AUDIT.md.
COMMIT: d090123
PR: #90
REMAINING_RISKS: None.
NEXT_TASK: TASK-P0-004
```

```markdown
TASK_ID: TASK-P0-004
PHASE: Phase 0 — Baseline & Forensic Audit
PRIORITY: P0
STATUS: VERIFIED
OBJECTIVE: Produce Phase 0 deliverables: BASELINE_AUDIT.md, CURRENT_ARCHITECTURE.md, COMPONENT_STATUS.md, CALL_GRAPH.md, and OPEN_ISSUES.md.
FILES: docs/active/BASELINE_AUDIT.md, docs/active/CURRENT_ARCHITECTURE.md, docs/active/COMPONENT_STATUS.md, docs/active/CALL_GRAPH.md, docs/active/OPEN_ISSUES.md
FUNCTIONS: N/A
CURRENT_BEHAVIOR: Fragmented, conflicting documentation across 15+ subdirectories.
ROOT_CAUSE: Accumulation of legacy agent reports and uncurated audit files.
REQUIRED_CHANGE: Authoritative, bare-metal documentation produced and grounded directly in Rust source code.
TEST_PLAN: Review all generated deliverables for code accuracy and link integrity.
ACCEPTANCE_CRITERIA: All Phase 0 deliverables completed and verified against Rust AST.
ACTUAL_RESULT: All deliverables produced in `docs/active/` with complete cross-references.
COMMIT: b87f7b9
PR: #91
REMAINING_RISKS: None.
NEXT_TASK: TASK-P1-001
```

---

## Phase 1: Documentation & Agent Reset (COMPLETED / IN_PROGRESS)

```markdown
TASK_ID: TASK-P1-001
PHASE: Phase 1 — Documentation & Agent Reset
PRIORITY: P0
STATUS: COMPLETED
OBJECTIVE: Inventory all existing markdown and documentation files in the repository and archive them without deleting history.
FILES: docs/**, reports/**, root markdown files
FUNCTIONS: scripts/reorganize_docs.py
CURRENT_BEHAVIOR: Over 315 documentation files scattered across root, `docs/`, `audit/`, `audit_work/`, `audit_extract/`.
ROOT_CAUSE: Uncurated audit sprawl from multiple independent agent sessions.
REQUIRED_CHANGE: Safely archive historical documents into `docs/archive/` (`legacy/`, `old_roadmaps/`, `old_audits/`, `old_reports/`, `old_architecture/`, `old_experiments/`, `historical_test_evidence/`) with standardized archive banners.
TEST_PLAN: Run `python scripts/reorganize_docs.py` and verify `DOCUMENTATION_INVENTORY.md`.
ACCEPTANCE_CRITERIA: 100% of historical documents preserved with archive notices; zero deletions.
ACTUAL_RESULT: 315 items relocated to `docs/archive/` subdirectories with banners applied; complete catalog produced in `docs/active/DOCUMENTATION_INVENTORY.md`.
COMMIT: b87f7b9
PR: #91
REMAINING_RISKS: None.
NEXT_TASK: TASK-P1-002
```

```markdown
TASK_ID: TASK-P1-002
PHASE: Phase 1 — Documentation & Agent Reset
PRIORITY: P0
STATUS: COMPLETED
OBJECTIVE: Author the single authoritative active documentation suite in `docs/active/` and agent instructions in `docs/agents/`.
FILES: docs/active/*, docs/agents/*, AGENTS.md
FUNCTIONS: N/A
CURRENT_BEHAVIOR: Inconsistent instructions and outdated agent guidance.
ROOT_CAUSE: Conflicting guidelines between legacy files and new architecture standards.
REQUIRED_CHANGE: Create clean, cohesive active suite (`README.md`, `MASTER_ROADMAP.md`, `CURRENT_ARCHITECTURE.md`, `CURRENT_STATUS.md`, `OPEN_ISSUES.md`, `DECISIONS.md`, `MASTER_TASK_REGISTER.md`, `TEST_STRATEGY.md`, `PRODUCTION_READINESS.md`) and agent system (`AGENT_RULES.md`, `SUPERVISOR.md`, `QUANT_AGENT.md`, `INGESTION_AGENT.md`, `EXECUTION_AGENT.md`, `HANDOFF_PROTOCOL.md`).
TEST_PLAN: Verify Markdown formatting, link resolution, and invariant compliance.
ACCEPTANCE_CRITERIA: Complete active suite in place; root AGENTS.md updated.
ACTUAL_RESULT: All active documents and agent instruction files generated and verified.
COMMIT: b87f7b9
PR: #91
REMAINING_RISKS: None.
NEXT_TASK: TASK-P1-003
```

```markdown
TASK_ID: TASK-P1-003
PHASE: Phase 1 — Documentation & Agent Reset
PRIORITY: P0
STATUS: VERIFIED
OBJECTIVE: Open the documentation-only pull request and await review/acceptance before touching trading logic.
FILES: docs/**, reports/**, evidence/**, AGENTS.md
FUNCTIONS: N/A
CURRENT_BEHAVIOR: N/A (closed).
ROOT_CAUSE: Step 6/7 of master engineering roadmap requires isolated documentation PR.
REQUIRED_CHANGE: Commit documentation suite, push branch, open PR via GitHub CLI, and present summary to operator.
TEST_PLAN: `git status`, `cargo check --workspace`, `git diff --stat`, `gh pr create`.
ACCEPTANCE_CRITERIA: Pull request opened containing strictly documentation, zero code/trading logic modifications.
ACTUAL_RESULT: PR #91 merged 2026-11-20T14:29:23Z into `main`; content was documentation-only. Follow-up: 99 broken `file:///c:/Users/azoozz/...` links in active docs repaired to repo-relative links and verified resolving (2026-11-20).
COMMIT: b87f7b9
PR: #91
REMAINING_RISKS: None.
NEXT_TASK: TASK-P2-001
```

---

## Phase 2: Critical Correctness Defects (VERIFIED — PRs #93–#98 merged, gate recorded in #99)

```markdown
TASK_ID: TASK-P2-001
PHASE: Phase 2 — Critical Correctness Defects
PRIORITY: P0
STATUS: NOT_STARTED
OBJECTIVE: Fix route deduplication defect in `crates/arb-bot/src/supervisor/strategy.rs` where sorting pool IDs discards route directionality and hop sequence.
FILES: crates/arb-bot/src/supervisor/strategy.rs, crates/arb-types/src/lib.rs
FUNCTIONS: Supervisor::strategy_cycle, ExecutionOpportunity::route_signature
CURRENT_BEHAVIOR: Pool IDs are sorted into `Vec<Pubkey>`, stripping out direction, causing $A \rightarrow B \rightarrow A$ and $B \rightarrow A \rightarrow B$ to collide.
ROOT_CAUSE: `ids.sort_unstable()` used as deduplication key in `seen_pool_sets`.
REQUIRED_CHANGE: Replace sorted pool list with directional `RouteSignature` incorporating ordered hops, input/output mints, swap direction, and protocol IDs.
TEST_PLAN: Add unit test verifying that inverted cycles on identical pools both survive deduplication.
ACCEPTANCE_CRITERIA: Deterministic deduplication preserving full directed route identity with passing regression test.
ACTUAL_RESULT: Implemented in `supervisor/route_dedup.rs` with ordered route identity, keep-best duplicate selection, and direction/hop-order regression tests. Live route profitability remains unproven.
COMMIT: PR #93 head (merged into `main` 2026-11-20)
PR: #93
REMAINING_RISKS: Potential increase in duplicate candidates if signature hashing is flawed.
NEXT_TASK: TASK-P2-002
```

```markdown
TASK_ID: TASK-P2-002
PHASE: Phase 2 — Critical Correctness Defects
PRIORITY: P0
STATUS: NOT_STARTED
OBJECTIVE: Eliminate monotonic profit assumptions in borrow sizing optimizer and enforce joint Route x Sizing evaluation.
FILES: crates/arb-engine/src/graph.rs, crates/arb-engine/src/sizing.rs
FUNCTIONS: MarketGraph::find_cross_pool_dlmm_cycles, optimize_borrow_size
CURRENT_BEHAVIOR: Sizing uses coarse sampling that can overshoot liquidity boundaries in DLMM bins.
ROOT_CAUSE: Assumption that net profit increases monotonically up to an inflection point.
REQUIRED_CHANGE: Implement bounded integer optimization with discrete boundary checks at tick/bin liquidity limits.
TEST_PLAN: Unit test with non-monotonic fee and liquidity profile.
ACCEPTANCE_CRITERIA: Sizing selects maximum profit point without overshooting available bin liquidity.
ACTUAL_RESULT: DLMM local-peak basin ranking (PR #94) plus bounded joint route-and-size search with per-hop re-quotes and bin/tick boundaries (PR #95). Returns best observed candidate within sample budget, not a proven global optimum. Orca active-tick fixture corrected (PR #97). Live Kamino vault-cap decoding and production differential soak remain required.
COMMIT: PR #94/#95/#97 heads (merged into `main` 2026-11-20)
PR: #94, #95, #97
REMAINING_RISKS: Search compute overhead on large graphs.
NEXT_TASK: TASK-P2-003
```

```markdown
TASK_ID: TASK-P2-003
PHASE: Phase 2 — Critical Correctness Defects
PRIORITY: P0
STATUS: NOT_STARTED
OBJECTIVE: Implement route-level slot coherency check to prevent phantom arbitrage across skewed pool snapshots.
FILES: crates/arb-engine/src/graph.rs, crates/arb-bot/src/supervisor/opportunity.rs
FUNCTIONS: Supervisor::validate_route_account_dependencies
CURRENT_BEHAVIOR: Individual pools pass state age check, but relative skew between hops in a route is unconstrained.
ROOT_CAUSE: Missing delta check between maximum and minimum snapshot slots among route hops.
REQUIRED_CHANGE: Reject candidates where `max(hop.slot) - min(hop.slot) > max_snapshot_skew_slots` (default 4 slots).
TEST_PLAN: Regression test providing skewed pool states and verifying fail-closed rejection.
ACCEPTANCE_CRITERIA: Skewed multi-hop routes rejected before transaction assembly.
ACTUAL_RESULT: No code change required: runtime dependency validation already passes `max_slot_skew = 0` for all route pools (stricter than the 4-slot ceiling) and `dependency_group_checks_provenance_and_slot_skew` rejects divergent watermarks.
COMMIT: N/A (existing implementation verified)
PR: N/A
REMAINING_RISKS: None.
NEXT_TASK: TASK-P2-004
```

```markdown
TASK_ID: TASK-P2-004
PHASE: Phase 2 — Critical Correctness Defects
PRIORITY: P0
STATUS: NOT_STARTED
OBJECTIVE: Classify simulation outcomes into explicit enum states and investigate root causes of slippage revert 0x1773.
FILES: crates/arb-sim/src/lib.rs, crates/arb-bot/src/supervisor/dispatch.rs
FUNCTIONS: Simulator::simulate_async, Supervisor::dispatch_opportunity
CURRENT_BEHAVIOR: Simulation returns boolean `success` and generic error string. Revert 0x1773 occasionally seen.
ROOT_CAUSE: Lack of structured simulation outcome enum; slippage parameters misaligned with live quote execution price.
REQUIRED_CHANGE: Introduce `SimulationOutcome` (`PASSED`, `FAILED`, `UNAVAILABLE`, `TIMEOUT`, `STALE`, `UNSUPPORTED`). Reproduce 0x1773 with live pool balance state.
TEST_PLAN: Mainnet non-broadcast simulation regression test.
ACCEPTANCE_CRITERIA: Explicit simulation state mapping; 0x1773 root cause identified and fixed without widening slippage.
ACTUAL_RESULT: Typed `SimulationOutcome` states with fail-closed dispatch preflight landed (PR #93); fake-bin fallback and simulation-error pass-through removed (PR #96); realized/estimated PnL provenance separated (PR #98). `0x1773` root-cause reproduction with original signed transaction and exact snapshot remains OPEN and blocks live execution (see `OPEN_ISSUES.md` ISSUE-P0-04).
COMMIT: PR #93/#96/#98 heads (merged into `main` 2026-11-20)
PR: #93, #96, #98
REMAINING_RISKS: 0x1773 root cause unreproduced; live execution blocked.
NEXT_TASK: TASK-P3-001
```

---

## Phase 3: WebSocket Ingestion & State Integrity (EXIT GATE VERIFIED 2026-11-20)

```markdown
TASK_ID: TASK-P3-001
PHASE: Phase 3 — WebSocket Ingestion & State Integrity
PRIORITY: P0
STATUS: VERIFIED
OBJECTIVE: Verify Phase 3 exit gate: automated reconnection, missing-account, stale-state, duplicate-update, and provider-failure tests pass; invalid/incomplete state cannot reach quote/execution path.
FILES: crates/arb-ingest/src/helius_ws.rs, crates/arb-ingest/src/slot_integrity.rs, crates/arb-ingest/src/quote_readiness.rs, crates/arb-ingest/src/provider_supervisor.rs, crates/arb-ingest/src/dependency_manifest.rs
FUNCTIONS: SlotIntegrity::observe_with_bank, SnapshotReconciler::record_batch, QuoteReadiness execution guard, ProviderSupervisor::failover
CURRENT_BEHAVIOR: N/A (verification task; no behavior change).
ROOT_CAUSE: N/A.
REQUIRED_CHANGE: Execute the arb-ingest test suite on the verified baseline and map each gate criterion to concrete passing tests.
TEST_PLAN: `cargo test -p arb-ingest` on commit a4524ba baseline.
ACCEPTANCE_CRITERIA: Reconnection, missing-account, stale-state, duplicate-update, and provider-failure tests all pass with zero failures.
ACTUAL_RESULT: 2026-11-20 — `cargo test -p arb-ingest`: 162 passed, 0 failed (145 unit + 3 concurrency + 2 CPMM fixture + 5 Oxylabs mock + 1 pool fetch + 3 quota-exhaustion + 3 RPC telemetry). Gate mapping recorded in `CURRENT_STATUS.md` §3. CI `Rust CI` green on `main` (Run ID 44802369313).
COMMIT: a4524ba (verification ran on this baseline)
PR: [Phase 3 gate evidence PR]
REMAINING_RISKS: In-process tests only; Phase 5 comparative WS-vs-gRPC benchmark and live shadow soak remain open (Phase 18 gate).
NEXT_TASK: TASK-P4-001
```

```markdown
TASK_ID: TASK-P3-002
PHASE: Phase 3 — WebSocket Ingestion and State Integrity
PRIORITY: P0
STATUS: COMPLETED (lifecycle verification delivered in TASK-P3-003)
OBJECTIVE: Unify WebSocket/RPC provider identities and make WebSocket disconnect/reconnect invalidate stale quote dependencies.
FILES: crates/arb-ingest/src/lib.rs, crates/arb-ingest/src/helius_ws.rs, crates/arb-ingest/src/rpc_reconcile.rs, crates/arb-bot/src/supervisor/run.rs, crates/arb-bot/src/supervisor/execution.rs, crates/arb-bot/src/supervisor.rs, crates/arb-bot/src/verified_config.rs, crates/arb-bot/src/bin/validate.rs
FUNCTIONS: AccountSnapshotStore::apply, AccountSnapshotStore::apply_batch, IngestClient::stream_connected, IngestClient::mark_helius_connected, IngestClient::mark_helius_disconnected, HeliusWsClient::seed_initial_accounts, Supervisor::run
CURRENT_BEHAVIOR: The run-loop health gate used reversed provider IDs. Its readiness flag combined two connection booleans, the run-loop accepted any healthy provider, and HTTP account seeding advanced the WebSocket slot watermark. Reconnects reused the previous stream generation; older snapshots could also block updates from a lagging endpoint.
ROOT_CAUSE: Provider identities and health mappings were independently hard-coded; RPC bootstrap and WebSocket liveness shared a watermark; readiness did not require the account-data provider; and reconnects did not reset account snapshots.
REQUIRED_CHANGE: Centralize provider IDs; require the healthy account WebSocket to authorize trading; isolate RPC clock/bootstrap observations and WS watermarks; reset per-session slot baselines; advance generation and clear old account snapshots on reconnect while preserving decoded pool state; let live stream data supersede bootstrap snapshots regardless of slot ordering.
TEST_PLAN: Run the focused `three_leg_net_economics_tests` regression suite and `cargo clippy -p arb-ingest --lib -- -D warnings`; require full Rust CI to pass on the replacement PR.
ACCEPTANCE_CRITERIA: RPC cannot authorize trading without a healthy account WebSocket; disconnected and prior-generation data fail readiness; live updates replace bootstrap snapshots; focused tests and full CI pass.
ACTUAL_RESULT: Implemented provider identity constants and a fail-closed WS-only health gate; RPC bootstrap no longer advances WS liveness; disconnect resets WS watermarks; reconnect resets the run-loop slot baseline, advances generation, and clears old account snapshots while preserving pool cache; live data can replace bootstrap state independent of slot ordering. Fixed the `i64::MIN` fee-sentinel route selection exposed by CI and replaced deprecated `AtomicU64::fetch_update`. Focused suite: 9 passed; targeted ingest Clippy passed; full Rust CI is running on replacement PR #104.
COMMIT: 99e5389
PR: #104 (supersedes #102)
REMAINING_RISKS: Live WS-server integration for the full reconnect handshake (dial → upgrade → drop → redial) is exercised only in-process via `reset_connection_state` and the epoch pair; no networked CI run of the real dial loop exists yet.

NEXT_TASK: Phase 4 (TASK-P4-001).
```

```markdown
TASK_ID: TASK-P3-003
PHASE: Phase 3 — WebSocket Ingestion and State Integrity
PRIORITY: P0
STATUS: VERIFIED
OBJECTIVE: Verify reconnection, backoff, and resubscription lifecycle with focused regression tests (Phase 3 task 3).
FILES: crates/arb-ingest/src/helius_ws.rs
FUNCTIONS: HeliusWsClient::record_connect_backoff, HeliusWsClient::select_active_endpoint_index, HeliusWsClient::reset_connection_state (new, verbatim extraction from start_stream), IngestClient::mark_helius_connected, IngestClient::mark_helius_disconnected, AccountSnapshotStore::apply
CURRENT_BEHAVIOR: The reconnect-reset block was inline inside `start_stream` (untestable without a live WS server); no focused regression tests covered backoff classification, endpoint selection under backoff, and the epoch boundary semantics.
ROOT_CAUSE: N/A (verification + behavior-preserving extraction task).
REQUIRED_CHANGE: Extract the per-connection reset verbatim into `reset_connection_state` and add focused regression tests for (a) provider-error backoff classification (429/4xx/transient), (b) endpoint selection skipping non-expired backoffs, (c) reconnect reset clearing subscription/flag/watermark state while preserving the decoded pool cache, (d) epoch generation advancing only across a disconnect boundary (idempotency while connected), (e) bootstrap-vs-live snapshot supersede semantics at the epoch boundary.
TEST_PLAN: `cargo test -p arb-ingest`; `cargo test -p arb-engine --test three_leg_net_economics_tests`; `cargo clippy -p arb-ingest --all-targets -- -D warnings`; `cargo fmt --all -- --check`.
ACCEPTANCE_CRITERIA: All five focused tests pass; no existing test regresses; clippy clean.
ACTUAL_RESULT: 2026-11-21 — `cargo test -p arb-ingest`: 150 passed, 0 failed (was 145; +5 new: record_connect_backoff_classifies_provider_error_classes, select_active_endpoint_skips_non_expired_backoffs, reconnect_reset_clears_subscription_state_and_flags_preserving_pool_cache, reconnect_epoch_advances_only_across_disconnect_boundary, account_store_accepts_live_and_refuses_prev_epoch_bootstrap). Focused three-leg suite: 9 passed. Clippy `-D warnings` clean, fmt clean.
REMAINING_RISKS: Real-network reconnect handshake (TLS dial, upgrade, mid-stream drop, redial) is not covered by in-process tests; gate while live-WS infrastructure is unavailable.
NEXT_TASK: Phase 4 (TASK-P4-001).
```

```markdown
TASK_ID: TASK-M2-001
PHASE: Mission §7 — Complete Flash Loan Forensic Audit (Phase 2)
PRIORITY: P0
STATUS: VERIFIED
OBJECTIVE: Verify the complete 19-step flash-loan lifecycle against the official Kamino implementation, audit every assembler for subsidy exposure (no pre-existing balance can disguise a loss), and produce the forensic audit + regression matrix without touching slippage.
FILES: crates/arb-tx/src/flash/kamino.rs, crates/arb-tx/src/flash/kamino_flash.rs, crates/arb-tx/src/flash/arb_executor.rs, crates/arb-tx/src/flash/dlmm_flash_arb.rs, crates/arb-tx/src/primitives/builder.rs, crates/arb-tx/src/primitives/atomic_integrity.rs, crates/arb-bot/src/supervisor/opportunity.rs, programs/arb-executor/src/instructions/execute_three_leg.rs, docs/active/audit/FLASH_LOAN_FORENSIC_AUDIT.md, docs/active/audit/FLASH_LOAN_REGRESSION_MATRIX.md
FUNCTIONS: build_kamino_flash_borrow, build_kamino_flash_repay_with_index, derive_swap_legs, assemble_split_flash_arbitrage, assemble_flash_funded_two_pool, compile_v1_message, verify_atomic_integrity_for_format, execute_three_leg (on-chain)
CURRENT_BEHAVIOR: Split-route return hop min_out bound only by quote-relative slippage; DLMM two-pool engine floor excludes tip/min_profit; 0x1773 (ISSUE-P0-04) root cause still unreproduced.
ROOT_CAUSE: FL-F-2026-11-22-N1: split-route return hop lacked the final-hop economic floor that `derive_swap_legs` enforces on all other assemblers (path double-gated dead: feature off by default + `split = None` hardcoded at opportunity.rs:756).
REQUIRED_CHANGE: `split_return_min_out` guard binding `max(slippage_floor, borrow+fee+tip+min_profit)` to the return hop; document per-path floor coverage; track DLMM engine wire-up requirement (N2) without touching reachable code semantics.
TEST_PLAN: `cargo test --workspace`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo clippy -p arb-bot --features split-routing --all-targets -- -D warnings`; new `split_return_min_out_tests`.
ACCEPTANCE_CRITERIA: Official-clone differential documented; every reachable path floored; findings registered; suite green.
ACTUAL_RESULT: 2026-11-22 — 19-step lifecycle verified vs official Kamino clone (accounts, writability, optional referrer placeholder, discriminators re-hashed, fee semantics, flash_repay_checks); subsidy matrix completed; N1 FIXED (+4 tests, split feature), N2 registered P3 (path unreachable). `cargo test --workspace`: 1,626 passed / 0 failed / 6 ignored (+4 over baseline). Clippy clean both feature sets; fmt clean. LiteSVM e2e not run this session (SBF blobs absent; SKIP classed honestly). ISSUE-P0-04 remains OPEN pending a real captured failure.
COMMIT: (this PR)
PR: (this PR)
REMAINING_RISKS: 0x1773 unresolved (live execution blocked, fail-closed); Token-2022 unsupported path unchanged; DLMM two-pool engine must not be wired up before FL-F-2026-11-22-N2 correction.
NEXT_TASK: Mission §8 Phase 3 — DEX support matrix completion.
```

```markdown
TASK_ID: TASK-M3-001
PHASE: Mission §8 — DEX and Quote Engine Audit (Phase 3)
PRIORITY: P1
STATUS: VERIFIED
OBJECTIVE: Classify all 11 DexProtocol variants across pillars A–H (discovery, decoding, quote, sizing, building, CPI, simulation, mainnet verification) with verified program IDs and honest unsupported labels.
FILES: docs/active/audit/DEX_SUPPORT_MATRIX.md, docs/active/audit/QUOTE_PARITY_REPORT.md, docs/active/OPEN_ISSUES.md, docs/active/README.md
FUNCTIONS: decode_pool_info, QuoteEngine dispatch (engines/mod.rs), build_swap_hop_instruction, per-protocol differential test suites
CURRENT_BEHAVIOR: 7 protocols carry the full loop; DAMM v2 / DBC / DLMM Pro / LaunchLab quote via constant-product approximations but are fail-closed for discovery (unreachable); Token-2022 excluded by policy; Jupiter is transport-only.
ROOT_CAUSE: N/A (audit + registration task).
REQUIRED_CHANGE: Documentation + register entries only; no runtime behavior changed.
TEST_PLAN: cargo test -p arb-dex; cargo test -p arb-math; program-ID verification against official sources.
ACCEPTANCE_CRITERIA: Matrix complete for all 11 protocols; parity evidence cited per venue; gaps classified with IDs.
ACTUAL_RESULT: 2026-11-22 — matrix delivered; program IDs re-verified (Meteora docs / Bitquery / Raydium catalog); arb-dex 127 passed / 0 failed; arb-math 342 passed / 0 failed; findings DEX-F-2026-11-22-N1 (P3, dormant CP approximations) and N2 (P3 telemetry) registered.
PR: (this PR)
REMAINING_RISKS: Pump.fun/PumpSwap mainnet snapshots not committed (synthetic fixtures only); four dormant approximations must never be enabled without the registered prerequisites.
NEXT_TASK: Mission §9 Phase 4 — Arbitrage engine & quantitative correctness.
```

```markdown
TASK_ID: TASK-M4-001
PHASE: Mission §9 — Arbitrage Engine & Quantitative Correctness (Phase 4)
PRIORITY: P1
STATUS: VERIFIED
OBJECTIVE: Audit Bellman-Ford usage, route graph/dedup, exact re-quoting & sizing, the net-profit consistency ladder across scanner/builder/sim/risk/executor/PnL, and produce the engine audit + economic invariants docs.
FILES: docs/active/audit/ARBITRAGE_ENGINE_AUDIT.md, docs/active/audit/ECONOMIC_INVARIANTS.md, docs/active/OPEN_ISSUES.md, docs/active/README.md
FUNCTIONS: evaluate_cycle (bellman_ford.rs:404), compute_cycle_profit, calculate_net_profit_raw (economic.rs:552), route_dedup, liquidity-capped sizing
CURRENT_BEHAVIOR: BF f64 weights pre-screen only; integer re-derivation for pricing/sizing/gating; best-observed sizing (not global optimum); realized PnL is balance-delta-only.
ROOT_CAUSE: N/A (audit task).
REQUIRED_CHANGE: Documentation + register entries only.
TEST_PLAN: cargo test -p arb-engine; cargo test -p arb-risk.
ACCEPTANCE_CRITERIA: Full consistency ladder documented with evidence; limitations explicitly stated; suite green.
ACTUAL_RESULT: 2026-11-22 — arb-engine 190 passed / 0 failed; arb-risk 22 passed / 0 failed; findings ARB-F-2026-11-22-N1/N2/N3 (all P3) registered; EI-01..EI-12 invariants tabulated with enforcement/test evidence.
PR: (this PR)
REMAINING_RISKS: Live profitability unproven (no live tx permitted); 0x1773 still blocks live; DLMM optimizer wire-up gated on FL-N2.
NEXT_TASK: Mission §10 Phase 5 — Transaction compiler audit.
```

```markdown
TASK_ID: TASK-M5-001
PHASE: Mission §10 — Solana Transaction Compiler (Phase 5)
PRIORITY: P1
STATUS: VERIFIED
OBJECTIVE: Audit the V0/V1 compilation pipeline (serialization, remapping, sanitation limits, transport selection, no-downgrade rule) against the current SIMD-0385 spec.
FILES: docs/active/audit/TRANSACTION_COMPILER_AUDIT.md, docs/active/audit/TRANSACTION_COMPATIBILITY_MATRIX.md, docs/active/OPEN_ISSUES.md, docs/active/README.md
FUNCTIONS: compile_v1_message (adaptive.rs:415), compile_v1_only, compile_v0_message, select_transport, V1ConfigParams::compute_mask_and_values
CURRENT_BEHAVIOR: V1 spec-complete; env-gated submission defaults OFF; V0 production default.
ROOT_CAUSE: N/A (audit task).
REQUIRED_CHANGE: Documentation only.
TEST_PLAN: cargo test -p arb-tx (434 tests incl. property fuzz + boundary ladders).
ACCEPTANCE_CRITERIA: Full SIMD-0385 layout table + compatibility matrix + no-downgrade rule documented with test evidence.
ACTUAL_RESULT: 2026-11-22 — 434 passed / 0 failed; TXC-F-2026-11-22-N1 (P3) registered (V1 toggles pending live landing evidence).
PR: (this PR)
REMAINING_RISKS: Live on-cluster V1 observation outstanding; V0 remains the default path.
NEXT_TASK: Mission §11 Phase 6 — On-chain program security.
```

```markdown
TASK_ID: TASK-M7-001
PHASE: Mission §12 — Market Data & HFT Infrastructure (Phase 7)
PRIORITY: P1
STATUS: VERIFIED
OBJECTIVE: Audit the market-data pipeline and HFT hot path (sources, coherence gates, latency ladder, concurrency primitives); document honestly without speculative optimization.
FILES: docs/active/architecture/MARKET_DATA_PIPELINE.md, docs/active/performance/HFT_ARCHITECTURE.md, docs/active/performance/LATENCY_BENCHMARKS.md, docs/active/OPEN_ISSUES.md, docs/active/README.md
FUNCTIONS: build_quote_dependency_set (quote_readiness.rs), SlotIntegrity::observe_with_bank (slot_integrity.rs), ProviderSupervisor::select (provider_supervisor.rs), StreamingPoolCache::apply (dual_ingest.rs), observe_* latency fns (arb-metrics 612-646), MultiDexRegistry::fetch_live_pool_states (hydration.rs)
CURRENT_BEHAVIOR: HTTP polling is the decision source (conservative); dual-ingest shadow harness present; 9 of 10 ladder rungs have wired histograms; USB idle — no sub-slot loop introduced.
ROOT_CAUSE: N/A (audit task).
REQUIRED_CHANGE: Documentation only.
TEST_PLAN: cargo test -p arb-ingest -p arb-metrics.
ACCEPTANCE_CRITERIA: Pipeline map + latency ladder table + bench inventory + honest gaps, all source-verified.
ACTUAL_RESULT: 2026-11-22 — arb-ingest 177/0, arb-metrics 11/0; HFT-F-2026-11-22-N1/N2/N3 (P3) registered; benches NOT executed this session (recorded honestly).
PR: (this PR)
REMAINING_RISKS: Yellowstone A/B outstanding; ingest-decode histogram missing; bench coverage limited.
NEXT_TASK: Mission §13 Phase 8 — Jito landing audit.
```
