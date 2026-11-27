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
OBJECTIVE: Classify all 10 active DexProtocol variants across pillars A–H (discovery, decoding, quote, sizing, building, CPI, simulation, mainnet verification) with verified program IDs and honest unsupported labels.
FILES: crates/arb-ingest/src/decoders.rs, crates/arb-ingest/src/hydration.rs, crates/arb-types/src/pool.rs, crates/arb-dex/src/quote_engine.rs, crates/arb-engine/src/adapters.rs, docs/active/audit/DEX_SUPPORT_MATRIX.md, docs/active/audit/QUOTE_PARITY_REPORT.md, docs/active/OPEN_ISSUES.md
FUNCTIONS: decode_pool_info, QuoteEngine dispatch (engines/mod.rs), build_swap_hop_instruction, per-protocol differential test suites
CURRENT_BEHAVIOR: 7 protocols carry the full loop; DAMM v2, DBC and LaunchLab remain fail-closed; Meteora DLMM Pro was removed from the active enum/runtime; Token-2022 excluded by policy; Jupiter is transport-only.
ROOT_CAUSE: N/A (audit + registration task).
REQUIRED_CHANGE: Keep support matrix accurate, retire incomplete adapters from the active enum, and harden supported quote/accounting paths without enabling unverified venues.
TEST_PLAN: cargo test --workspace --all-targets; cargo clippy --workspace --all-targets -- -D warnings; official Raydium CP-Swap SDK differential remains an open acceptance gate.
ACCEPTANCE_CRITERIA: Matrix complete for all 10 active protocols; retired protocols excluded from runtime count; parity evidence cited per venue; gaps classified with IDs.
ACTUAL_RESULT: 2026-11-25 - 10 active protocols; DLMM Pro retired; Raydium CPMM exact fee ratio, accrued-fee reserves, owner/mint checks and slot-coherent gating added; full official-SDK quote differential remains open.
PR: (this PR)
REMAINING_RISKS: Pump.fun/PumpSwap mainnet snapshots not committed (synthetic fixtures only); DAMM v2/DBC/LaunchLab remain unsupported; Raydium CPMM creator-fee-enabled pools are skipped until fee-side semantics are implemented.
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
TASK_ID: TASK-M6-001
PHASE: Mission §11 — On-Chain Program Security (Phase 6)
PRIORITY: P0
STATUS: VERIFIED
OBJECTIVE: Audit every arb-executor instruction (signer, PDA/state, program identity, pools, mints, token accounts, tips, arithmetic, reentrancy, replay) and verify deployment status honestly.
FILES: programs/arb-executor/src/instructions/*.rs, programs/arb-executor/src/validation/*.rs, docs/active/security/ONCHAIN_PROGRAM_SECURITY_AUDIT.md, docs/active/security/PROGRAM_DEPLOYMENT_VERIFICATION.md, docs/active/OPEN_ISSUES.md
FUNCTIONS: process_execute_three_leg, resolve_route_mint_accounts, validate_protocol_programs, validate_user_token_account_for_mint, validate_jito_tip_account, tip_amount_for_bps, process_profit_guard
CURRENT_BEHAVIOR: native (non-Anchor) stateless entrypoint; delta-based profit guards; hard-coded trusted program IDs; vanity placeholder deploy ID.
ROOT_CAUSE: N/A (audit task).
REQUIRED_CHANGE: Documentation + register entries only.
TEST_PLAN: cargo test -p arb-executor.
ACCEPTANCE_CRITERIA: Full security property table; deployment verdict honest (NOT VERIFIED); findings registered.
ACTUAL_RESULT: 2026-11-22 — 36 passed / 0 failed; no unsafe/unwrap in prod paths; findings EXEC-F-2026-11-22-N1 (P2 placeholder ID) and N2 (P3 SBF build absent) registered. Program is native, not Anchor (mission wording corrected).
PR: #129
REMAINING_RISKS: Deployment identity unverified — on-chain executor cannot be trusted for live routing until EXEC-N1 items are done.
NEXT_TASK: Mission §12 Phase 7 — Market data & HFT infrastructure.
```

```markdown
TASK_ID: TASK-M7-001
PHASE: Mission §12 — Market Data & HFT Infrastructure (Phase 7)
PRIORITY: P1
STATUS: VERIFIED
OBJECTIVE: Audit the market-data pipeline and HFT hot path (sources, coherence gates, latency ladder, concurrency primitives); document honestly without speculative optimization.
FILES: docs/active/architecture/MARKET_DATA_PIPELINE.md, docs/active/performance/HFT_ARCHITECTURE.md, docs/active/performance/LATENCY_BENCHMARKS.md, docs/active/OPEN_ISSUES.md, docs/active/README.md
FUNCTIONS: build_quote_dependency_set (quote_readiness.rs), SlotIntegrity::observe_with_bank (slot_integrity.rs), ProviderSupervisor::select (provider_supervisor.rs), StreamingPoolCache::apply (dual_ingest.rs), observe_* latency fns (arb-metrics 612-646), MultiDexRegistry::fetch_live_pool_states (hydration.rs)
CURRENT_BEHAVIOR: HTTP polling is the decision source (conservative); dual-ingest shadow harness present; 9 of 10 ladder rungs have wired histograms; no sub-slot loop introduced without evidence.
ROOT_CAUSE: N/A (audit task).
REQUIRED_CHANGE: Documentation only.
TEST_PLAN: cargo test -p arb-ingest -p arb-metrics.
ACCEPTANCE_CRITERIA: Pipeline map + latency ladder table + bench inventory + honest gaps, all source-verified.
ACTUAL_RESULT: 2026-11-22 — arb-ingest 177/0, arb-metrics 11/0; HFT-F-2026-11-22-N1/N2/N3 (P3) registered; benches NOT executed this session (recorded honestly).
PR: (this PR)
REMAINING_RISKS: Yellowstone A/B outstanding; ingest-decode histogram missing; bench coverage limited.
NEXT_TASK: Mission §13 Phase 8 — Jito landing audit.

```

```markdown
TASK_ID: TASK-M8-001
PHASE: Mission §13 — Jito & Landing Engine (Phase 8)
PRIORITY: P1
STATUS: VERIFIED
OBJECTIVE: Audit bundle submission, tip pricing, tip validation, lifecycle state machine, landing ledger, retry/reconciliation semantics.
FILES: docs/active/audit/JITO_LANDING_AUDIT.md, docs/active/performance/JITO_BENCHMARKS.md, docs/active/OPEN_ISSUES.md, docs/active/README.md
FUNCTIONS: send_bundle/send_bundle_multi_region (arb-land lib.rs:458/525), AdaptiveBidder (bidder.rs), LandingLedger (ledger.rs), TransactionLifecycle (lifecycle.rs), TipOracle (tip_oracle.rs), validate_jito_tip_account (programs/arb-executor validation)
CURRENT_BEHAVIOR: EV-maximizing bidder with refuse-above-ceiling rule; stale tip floor = no bid; proven-expiry-only retries; no per-region landing attribution (correct by design); tip account drift CI-guarded.
ROOT_CAUSE: N/A (audit task).
REQUIRED_CHANGE: Documentation only.
TEST_PLAN: cargo test -p arb-land.
ACCEPTANCE_CRITERIA: Semantic separation acceptance≠landing≠confirmation≠finalization documented with test evidence; no blind retry verified.
ACTUAL_RESULT: 2026-11-22 — arb-land 68/0; JTO-F-2026-11-22-N1/N2/N3 (P3) registered; no soak data yet (posteriors at prior).
PR: #131
REMAINING_RISKS: No measured landing data until shadow soak; bundle-sim policy unsettled.
NEXT_TASK: Mission §14 Phase 9 — Risk & configuration hardening.
```

```markdown
TASK_ID: TASK-M9-001
PHASE: Mission §14 — Risk & Configuration Hardening (Phase 9)
PRIORITY: P1
STATUS: VERIFIED
OBJECTIVE: Audit configuration precedence, canonical path selection, standalone testing config, secret-safe logging, config provenance, validation, risk limits and kill-switch controls.
FILES: crates/arb-bot/src/main.rs, crates/arb-bot/src/config.rs, config/config.example.toml, config/testing.toml, docs/active/security/CONFIGURATION_SECURITY.md, docs/active/operations/RISK_CONTROL_POLICY.md, docs/active/OPEN_ISSUES.md
FUNCTIONS: arb_config::validation::validate (validation.rs:19), check_no_placeholder_secret_env_names (validation.rs:414), RiskGate (arb-risk lib.rs:454), KillSwitch (lib.rs:115), apply_env_overrides (loader.rs:48-70), fire-time gate (dispatch.rs:649-665)
CURRENT_BEHAVIOR: one standalone TOML selected by --config/ARB_CONFIG/canonical config/config.toml; legacy [bot]/[rpc] schema or alternate [mode]/[provider] AppConfig schema; allow-listed env overrides; no implicit profile merge; runtime live dispatch requires ARB_LIVE; startup path log + append-only SHA-256 provenance; RPC URL credentials redacted.
ROOT_CAUSE: N/A (audit task).
REQUIRED_CHANGE: Enforce the canonical config path; remove tracked legacy root config; add loopback testing config; append non-secret config provenance; redact provider URLs in logs; align tests/docs with actual loader behavior.
TEST_PLAN: cargo test -p arb-bot --lib config_file_tests; cargo test -p arb-bot --bin arb-bot canonical_config_path_wins_and_legacy_root_is_never_selected; cargo test -p arb-bot --bin arb-bot config_audit_is_append_only_and_contains_no_config_values; cargo test --workspace --all-targets; cargo clippy --workspace --all-targets -- -D warnings.
ACCEPTANCE_CRITERIA: only config/config.toml is the implicit default; root config is not tracked/selected; testing config parses and uses loopback resources; audit rows contain hash/path/mode but no secrets; RPC URLs are not logged; docs accurately state no profile merging.
ACTUAL_RESULT: 2026-11-25 - removed tracked root config.toml; default resolver accepts only config/config.toml; added path regression test, standalone loopback testing.toml, selected-path startup log, append-only SHA-256 provenance log, secret-safe RPC logging, updated config tests and security docs. CFN-F N1/N2 resolved locally; N3 partially addressed (hash/path/mode provenance, semantic diff open).
PR: (this PR)
REMAINING_RISKS: legacy bot Config and alternate arb-config AppConfig remain separate schemas; config files are standalone (no automatic base/profile merge); audit log records file provenance, not semantic field-level diffs; BAM re-evaluation pending.
NEXT_TASK: Mission §15 Phase 10 — Backend, frontend and API.
```

```markdown
TASK_ID: TASK-M10-001
PHASE: Mission §15 — Backend, Frontend and API (Phase 10)
PRIORITY: P1
STATUS: VERIFIED
OBJECTIVE: Audit web_api.rs, control_service.rs, Telegram control and the React dashboard; document the API surface and auth contract; verify security contract tests.
FILES: docs/active/audit/BACKEND_SECURITY_AUDIT.md, docs/active/audit/FRONTEND_AUDIT.md, docs/active/architecture/API_REFERENCE.md, docs/active/OPEN_ISSUES.md, docs/active/README.md
FUNCTIONS: WebState::new (web_api.rs:79-106), login/authorize (154-206), control_post (320-354), execute_command (control_service.rs:855-876), live CONFIRM-LIVE gate (755-775), ws tickets (111-137), api.ts token handling
CURRENT_BEHAVIOR: Bearer-token auth (no cookies → no CSRF), placeholder-password rejection, per-IP rate limits, single-use WS tickets, 64 KiB body cap, OWASP headers, live double-gated; frontend is projection+control only.
ROOT_CAUSE: N/A (audit task).
REQUIRED_CHANGE: Documentation only.
TEST_PLAN: node --test web/security-contract.test.mjs; cargo test -p arb-bot (web auth unit tests).
ACCEPTANCE_CRITERIA: Full API table, auth internals, high-risk action gates, frontend classification, findings registered.
ACTUAL_RESULT: 2026-11-22 — security contract 3/0 pass; BCK-F-2026-11-22-N1/N2/N3 + FRT-F-2026-11-22-N1/N2 (P3) registered; no auth bypass found.
PR: (this PR)
REMAINING_RISKS: sha256 login hashing before any non-loopback exposure (N3); IP-trust caveat.
NEXT_TASK: Mission §16 Phase 11 — Webhook service.
```

```markdown
TASK_ID: TASK-M11-001
PHASE: Mission §16 — Webhook Service (Phase 11)
PRIORITY: P1
STATUS: VERIFIED
OBJECTIVE: Independently test webhook-service outside the workspace; document auth/dedup/replay/rate-limit guards; close the CI coverage gap (build, test, clippy, dependency audit) required by the mission spec §16.
FILES: docs/active/audit/WEBHOOK_SERVICE_AUDIT.md, .github/workflows/rust-ci.yml, docs/active/OPEN_ISSUES.md, docs/active/README.md
FUNCTIONS: constant_time_eq (lib.rs:58), verify_webhook_token (lib.rs:70), verify_hmac_signature (lib.rs:87), event_age_allowed (lib.rs:111), Deduplicator two-tier (dedup.rs:1-32), RateLimiter (rate_limit.rs:26), max_body_bytes (config.rs:41,59), handle_helius_pumpfun_webhook (helius_pumpfun_handler.rs:46)
CURRENT_BEHAVIOR: 28/28 tests pass standalone; auth fail-closed via constant-time compare (token + HMAC); two-tier dedup (DashMap local_ttl + Redis SET NX PX); fixed-window rate limit; OWASP body cap; PRESENT-UNWIRED from main bot (discovery-only value).
ROOT_CAUSE: N1 — CI ran cargo-deny only for webhook-service; no build/test/clippy gate existed.
REQUIRED_CHANGE: Dedicated webhook-service CI job; audit doc; findings WEB-F-2026-11-22-N1 (P2, fixed in this PR), N2/N3 (P3, documented).
TEST_PLAN: cd webhook-service && cargo fmt --check && cargo build --all-targets && cargo test && cargo clippy --all-targets -- -D warnings; YAML parse check of rust-ci.yml.
ACCEPTANCE_CRITERIA: CI job present and green; scenario coverage table documented; findings registered.
ACTUAL_RESULT: 2026-11-22 — 28/0 pass fresh on branch; fmt EXIT=0; CI job syntax validated; WEB-F-N1 fixed by this PR, WEB-F-N2/N3 (P3) registered.
PR: (this PR)
REMAINING_RISKS: Redis-outage dedup behavior untested at integration level; in-memory rate limiter per-instance (N2).
NEXT_TASK: Mission §17 Phase 12 — Database & storage (DATA_MODEL.md + DATABASE_RECOVERY.md).
```

```markdown
TASK_ID: TASK-M12-001
PHASE: Mission §17 — Database & Storage (Phase 12)
PRIORITY: P1
STATUS: COMPLETED
OBJECTIVE: Audit the persistence layer against the mission §17 contract; extend execution records
so every attempt carries enough data to reconstruct the economic result; register and close the
gap findings.
FILES: crates/arb-store/migrations/001_init.sql, crates/arb-store/migrations/001_postgres_init.sql,
crates/arb-store/src/lib.rs, crates/arb-store/src/postgres_store.rs,
crates/arb-bot/src/supervisor/opportunity.rs, crates/arb-bot/src/supervisor/dispatch.rs,
crates/arb-bot/src/supervisor/recovery.rs, docs/active/architecture/DATA_MODEL.md,
docs/active/operations/DATABASE_RECOVERY.md
FUNCTIONS: Store::record_attempt, Store::record_outcome_detail (new), Store::set_dispatch_bundle_id (new), SqliteStore::evolve_schema, postg_store::row_to_attempt, opportunity record site
CURRENT_BEHAVIOR (pre-fix): attempts stored only (id, slot, route_id, expected_net, tip, decision);
outcomes stored no actual_output/failure_reason; dispatch_intents had no bundle id. Economic
reconstruction (borrow, flash fee, priority fee, expected output, pool ids) was impossible from
the durable store.
ROOT_CAUSE: Schema evolved feature-by-feature without a reconstruction contract.
REQUIRED_CHANGE: add attempts.{borrow_amount,flash_fee_raw,priority_fee_lamports,expected_output_raw,pool_ids},
outcomes.{actual_output_raw,failure_reason}, dispatch_intents.bundle_id to both SQLite and
PostgreSQL schemas with idempotent evolution; populate from the production record sites; keep
NULL = unknown (never assume zero costs).
TEST_PLAN: cargo test -p arb-store -p arb-bot --all-targets.
ACCEPTANCE_CRITERIA: roundtrip of all economic fields on backends covered by tests; outcome
detail persistence + provenance-guarded PnL intact; bundle id captured after provider acceptance
(pre-send intent stays signature-keyed); legacy databases open and evolve without failure.
ACTUAL_RESULT: 2026-11-23 — 15/15 arb-store tests including 4 new (economic roundtrip,
outcome detail + PnL guard, bundle-id roundtrip, legacy evolution). FOUND + FIXED real legacy-DB
bug in the same phase: INIT_SQL created idx_outcomes_pnl on outcomes(status, provenance, …)
BEFORE evolve_schema added `provenance`, so any database predating provenance failed the entire
open() migration chain; index creation moved into evolve_schema after the column-add step and is
covered by schema_evolution_adds_economic_columns_to_legacy_db.
REMAINING_RISKS: PostgresStore evolution not end-to-end tested in CI (no PG service in sandbox);
outcomes.actual_output_raw/failure_reason are recorded but not yet fed by all dispatch
reconciliation paths (wired to record_outcome_detail call sites is a later pass).
PR: (this PR)
NEXT_TASK: Mission §18 Phase 13 — CI/CD, Docker & VPS.
```

```markdown
TASK_ID: TASK-M13-001
PHASE: Mission §18 — CI/CD, Docker & VPS (Phase 13)
PRIORITY: P1
STATUS: COMPLETED
OBJECTIVE: Audit all deployment workflows; pin deployable commit; add health-check + automatic rollback; remove duplicate compose; pin third-party Actions to SHAs; add weekly advisory re-scan; write DEPLOYMENT_ARCHITECTURE/VPS_RUNBOOK/ROLLBACK_PROCEDURE.
FILES: .github/workflows/deploy-vps.yml, .github/workflows/rust-ci.yml, docker-compose.shadow.yml (deleted), docs/active/operations/{DEPLOYMENT_ARCHITECTURE,VPS_RUNBOOK,ROLLBACK_PROCEDURE}.md, docs/active/MASTER_TASK_REGISTER.md
FUNCTIONS: deploy job remote script (deploy-vps.yml), rollback() closure, merge-base ancestry gate, schedule cron (rust-ci.yml:15), job if-guards
CURRENT_BEHAVIOR (before): deploy-vps shipped `git reset --hard origin/main` — deployed whatever main pointed at at run time (race with concurrent merge), no health gate, no rollback; docker-compose.shadow.yml byte-identical to docker-compose.yml; third-party actions tag-pinned only; advisories only re-checked on pushes.
ROOT_CAUSE: DEP-F-2026-11-23-N1 (P1): deploy SHA-race; DEP-F-2026-11-23-N2 (P1): no post-deploy health/rollback; DEP-F-2026-11-23-N3 (P3): duplicate compose; DEP-F-2026-11-23-N4 (P2): mutable action tags; DEP-F-2026-11-23-N5 (P2): no periodic advisory re-scan.
REQUIRED_CHANGE: pinned github.sha deploy with ancestry verification; rollback() closure invoked on build/restart/health failure; `environment: production` approval hook; SHA pins for dtolnay/rust-toolchain, Swatinem/rust-cache, EmbarkStudios/cargo-deny-action, gitleaks/gitleaks-action, actions/checkout, actions/setup-node; weekly cron security-only re-scan (heavy jobs if-guarded); duplicate compose removed; 3 ops docs written.
TEST_PLAN: YAML parse of both workflows; local yaml safety check; (live deploy runs are operator-dispatch only by mission §4.5).
ACCEPTANCE_CRITERIA: Deploy can never build an unintended commit; failure auto-rolls back; all action refs immutable SHAs; advisories re-scanned weekly without pushes; no duplicate compose files; ops docs present.
ACTUAL_RESULT: 2026-11-23 — both workflows YAML-valid; pins resolved from GitHub API and annotated; OK.
PR: (this PR)
REMAINING_RISKS: `production` GitHub Environment must be created by operator to activate the approval gate; SBOM artifact not yet generated (P3); StrictHostKeyChecking=accept-new is TOFU (documented, P3); live rollback path untested on the actual VPS (requires operator dispatch).
NEXT_TASK: Mission §19 Phase 14 — Performance & reliability testing.

TASK_ID: TASK-M14-001
PHASE: Mission §19 — Performance & reliability testing (Phase 14)
PRIORITY: P1
STATUS: COMPLETED
OBJECTIVE: Reproducible benchmark baseline (criterion + dual-ingest + live pre-flight) and reliability evidence report; no optimization claimed without comparable numbers.
FILES: docs/active/performance/PERFORMANCE_REPORT.md, docs/active/performance/RELIABILITY_REPORT.md, docs/active/MASTER_TASK_REGISTER.md
FUNCTIONS: cpmm_out (arb-math benches 16.97 ns), optimal_two_pool (194.15 ns criterion / 236 ns on live reserves), latency-bench movements 1–5, DualIngestShadow bench
CURRENT_BEHAVIOR: local deterministic work (decode→quote→size→build→sign) ≈ 15 µs; network dominates (Helius 39 ms RTT; Jito Amsterdam 51.9 ms best vs Tokyo 719 ms).
ROOT_CAUSE: N/A (measurement task). Perf/flamegraph not permitted in sandbox — recorded.
REQUIRED_CHANGE: docs only; perf baseline recorded pre-optimization per mission §19.
TEST_PLAN: cargo bench -p arb-math; cargo bench -p arb-ingest --bench dual_ingest_bench --features extra-ingest; 10 reliability suites executed (--release) 41/41 pass.
ACCEPTANCE_CRITERIA: baseline recorded with exact commands; every pass claim backed by a fresh run; live RTL math parity via latency-bench (read-only, no broadcast).
ACTUAL_RESULT: 2026-11-23 — bench + assertions listed in RELIABILITY_REPORT all pass; slot-budget conclusion: Frankfurt/Amsterdam colocation is the top latency lever.
PR: (this PR)
REMAINING_RISKS: live chaos testing + 24 h shadow soak remain operator actions; perf_event unavailable in sandbox.
NEXT_TASK: Mission §20 Phase 16 — Final documentation rebuild + root README.

TASK_ID: TASK-M16-001
PHASE: Mission §20/§15–16 — Final documentation rebuild + root README (Phases 15–16)
PRIORITY: P1
STATUS: COMPLETED
OBJECTIVE: Complete the ACTIVE status/operations tree (PROJECT_STATUS, PRODUCTION_READINESS, KNOWN_ISSUES, SHADOW_MODE_RUNBOOK, INCIDENT_RESPONSE) and rebuild the root README to the 30-section mission spec with only verified claims.
FILES: docs/active/status/{PROJECT_STATUS,PRODUCTION_READINESS,KNOWN_ISSUES}.md, docs/active/operations/{SHADOW_MODE_RUNBOOK,INCIDENT_RESPONSE}.md, README.md, docs/active/MASTER_TASK_REGISTER.md
FUNCTIONS: N/A (documentation + verification-of-links task; every referenced path existence-checked).
CURRENT_BEHAVIOR (before): status/ contained only INACTIVE_FEATURES; no readiness verdict, no incident response, README predated phases 13–14 evidence and carried 3 broken doc links.
ROOT_CAUSE: N/A.
REQUIRED_CHANGE: docs only; verdict SIMULATION READY / SHADOW REQUIRED pending 24h operator soak recorded explicitly; README restructured 1–30 with link-existence validation.
TEST_PLAN: file-existence check of all referenced docs; content cross-check against register entries and fresh phase-14 measurements.
ACCEPTANCE_CRITERIA: no empty placeholder docs; ACTIVE/INACTIVE/ARCHIVE intact; every claim labeled to evidence; README distinguishes Quote vs Execution / Shadow vs Live / simulated vs mainnet-verified.
ACTUAL_RESULT: 2026-11-23 — all 5 new docs written from register evidence; README rebuilt (30 sections), 3 stale links corrected to real files after existence check.
PR: (this PR)
REMAINING_RISKS: 24h live shadow soak + production-environment approval gate remain operator actions (recorded in PRODUCTION_READINESS.md).
NEXT_TASK: Mission §22 Phase 17 — shadow soak authorization request (operator).
```

```markdown
TASK_ID: TASK-DEX-STATE-20261127-001
PHASE: Phase 7 — Exact Quote Math & Differential/Boundary Tests
PRIORITY: P2
STATUS: COMPLETED
BASE_COMMIT: 4dc530a
BRANCH: fix/protocol-parity-hardening-20261127-v2
FILES_CHANGED:
  - crates/arb-dex/src/engines/raydium_cpmm.rs
  - crates/arb-dex/src/engines/raydium_v4.rs
  - crates/arb-dex/tests/raydium_cpmm_boundary_tests.rs
  - crates/arb-dex/tests/raydium_v4_boundary_tests.rs
  - docs/active/OPEN_ISSUES.md
  - docs/active/CURRENT_STATUS.md
  - docs/active/MASTER_TASK_REGISTER.md
FUNCTIONS_CHANGED:
  - RaydiumCpmmQuoteEngine::quote_exact_out
  - RaydiumAmmV4QuoteEngine::quote_exact_out
OBSERVED_BEHAVIOR: Both Raydium exact-in adapters rejected inactive PoolState snapshots, but the corresponding exact-out adapters could still calculate a quote for the same inactive pool.
ROOT_CAUSE: The exact-out implementations validated fee state and quote inputs but omitted the shared is_active guard already present in exact-in.
IMPLEMENTATION: Added fail-closed PoolDisabled checks to both exact-out adapters before reserve/quote math; expanded each existing disabled-pool regression test to assert rejection for exact-in and exact-out; documented the finding and evidence.
TESTS_EXECUTED:
  - cargo fmt --all -- --check
  - cargo test -p arb-dex --test raydium_cpmm_boundary_tests
  - cargo test -p arb-dex --test raydium_v4_boundary_tests
  - cargo clippy -p arb-dex --all-targets -- -D warnings
  - cargo test -p arb-dex --all-targets
  - git diff --check
TEST_RESULTS: Formatting and diff checks passed. CPMM boundary tests: 8 passed; V4 boundary tests: 5 passed; complete arb-dex all-targets suite: 168 passed, 0 failed. Clippy passed with -D warnings.
CI_STATUS: PENDING (PR workflow)
KNOWN_LIMITATIONS: This only enforces the already-decoded PoolState.is_active flag. It does not change upstream status decoding, add new exact-out support, or enable any previously gated venue.
OPEN_QUESTIONS: None for this narrow fix.
REMAINING_RISKS: Workspace-wide CI remains pending. No RPC simulation, live transaction, landing, or realized PnL was part of this task.
NEXT_ACTION: Review PR CI and keep merge as an operator action.
```


```markdown
TASK_ID: TASK-ROUTE-CONTRACT-20261127-001
PHASE: Phase 8 ? Route Discovery & Joint Sizing / Phase 10 ? Flash Loan Integration
PRIORITY: P0
STATUS: COMPLETED
BASE_COMMIT: 3319b7eaf8d7c82229ae315a5d501b4d278af3a1
BRANCH: fix/flashloan-2-3-pool-route-gates-20261127
PR: #203 (open; all required checks passed)
OBJECTIVE: Enforce the operator's active execution contract: mandatory atomic Kamino flash loan and only closed 2- or 3-pool cycles, without expanding DEX coverage or changing the operator's existing checkout.
FILES_CHANGED:
  - config/base.toml
  - config/config.example.toml
  - config/config.schema.json
  - config/execution.toml
  - crates/arb-bot/src/config.rs
  - crates/arb-bot/src/supervisor/execution.rs
  - crates/arb-bot/src/supervisor/opportunity.rs
  - crates/arb-config/src/lib.rs
  - crates/arb-config/src/validation.rs
  - crates/arb-engine/src/lib.rs
  - crates/arb-engine/tests/cross_dex_three_pool_live_loop.rs
  - crates/arb-engine/tests/phase_e_gates_and_bench.rs
  - crates/arb-tx/src/flash/cross_dex_flash_arb.rs
  - crates/arb-tx/tests/cross_dex_dlmm_cpmm_v1_atomic_tests.rs
  - docs/active/DECISIONS.md
  - docs/active/CURRENT_STATUS.md
  - docs/active/OPEN_ISSUES.md
  - docs/active/MASTER_TASK_REGISTER.md
FUNCTIONS_CHANGED:
  - validate_strategy
  - MarketGraph::find_cycles_with_stats
  - MarketGraph::find_cycles_with_anchor_caps
  - evaluate_directed_route
  - Supervisor::process_opportunities
  - Supervisor::assemble_native_ixs_for_anchor
  - is_cross_dex_opportunity
  - validate_cross_dex_opportunity
OBSERVED_BEHAVIOR: Active route scanning/evaluation accepted up to six hops, config validation accepted up to four, cross-DEX validation accepted four, and the Supervisor/assembler retained a non-flash fallback.
ROOT_CAUSE: Independent route-depth and funding-mode gates had drifted across the engine, config, cross-DEX validator, and Supervisor.
IMPLEMENTATION: The active MarketGraph scanner/evaluator is capped at 2..=3; the Supervisor rejects routes outside 2..=3, non-closing/discontinuous cycles, and repeated pools; config validation rejects max_route_hops outside 2..=3 and rejects use_flash_loan=false; the native assembler no longer falls back to non-flash assembly; cross-DEX classification and validation enforce the same 2..=3 bound; config TOML/schema docs were aligned. `max_pools` remains the separate discovery-universe cap. Disconnected legacy graph utilities and the exported four-leg builder were not wired into the active Supervisor.
TESTS_EXECUTED:
  - cargo +1.99.1 fmt --all -- --check
  - cargo +1.99.1 test -p arb-config -p arb-engine -p arb-tx --locked
  - cargo +1.99.1 check -p arb-bot --locked
  - cargo +1.99.1 clippy -p arb-config -p arb-engine -p arb-tx -p arb-bot --all-targets --locked -- -D warnings
  - cargo +1.99.1 test -p arb-bot --all-targets --locked
  - cargo +1.99.1 test -p arb-tx --test cross_dex_dlmm_cpmm_v1_atomic_tests cross_dex_validator_rejects_four_pool_routes --locked
  - git diff --check
TEST_RESULTS: `arb-config`/`arb-engine`/`arb-tx` test suites passed; `arb-bot --all-targets` passed (142 tests across library, binaries, and integration targets); focused cross-DEX four-pool regression passed (1/1); `arb-bot` check passed; targeted Clippy passed with -D warnings; format, JSON schema parse, and `git diff --check` passed.
CI_STATUS: PASS (PR #203; Rust CI, webhook CI, cargo-deny, gitleaks, Web Build, and Vercel all passed)
KNOWN_LIMITATIONS: Verification host is Windows with Rust 1.99.1; Linux CI remains required. No RPC simulation, shadow soak, live transaction, landing, or realized PnL was performed. The legacy four-leg builder remains disconnected from the active Supervisor.
OPEN_QUESTIONS: None for this scoped route/funding contract.
REMAINING_RISKS: GitHub CI and Linux build must pass before merge; no live execution authorization or profitability claim is implied.
NEXT_ACTION: PR #203 is open with all required checks passing; human review and merge remain operator actions.
```markdown
TASK_ID: TASK-ECON-2026-11-27-001
PHASE: Phase 7 — Protocol Mathematics and Quote Correctness / Phase 9 — Canonical Economic Engine
PRIORITY: P1
STATUS: VERIFIED
OBJECTIVE: Revalidate CPMM price impact, protocol swap-fee rounding, Kamino flash-loan premium, Solana base/priority fees, Jito tip, and canonical net-profit accounting against official sources and active call paths.
FILES: crates/arb-dex/src/engines/raydium_cpmm.rs, reports/ECONOMIC_COST_AUDIT_2026-11-27.md, docs/active/CURRENT_STATUS.md, docs/active/OPEN_ISSUES.md
FUNCTIONS: RaydiumCpmmQuoteEngine::quote_exact_in, RaydiumCpmmQuoteEngine::quote_exact_out, cpmm_price_impact_bps, calculate_kamino_flash_fee, AdaptiveBidder::plan_with_context, Supervisor economic opportunity evaluation
CURRENT_BEHAVIOR: CPMM input creator fees use the official combined-rate ceiling and proportional split; curve impact excludes fees; flash/base/priority/Jito costs are separately represented.
ROOT_CAUSE: No production formula defect was confirmed in the canonical path. The review found a documentation ambiguity around CPMM creator fee rounding that warranted a source-pinned regression test.
REQUIRED_CHANGE: Preserve the verified formulas; add regression tests for CPMM fee split, exact-out inverse, round-trip, and fee-exclusive price impact; document the economic audit and legacy-path reachability.
TEST_PLAN: cargo fmt --all -- --check; cargo test -p arb-math -p arb-dex; cargo clippy -p arb-math -p arb-dex --all-targets -- -D warnings.
ACCEPTANCE_CRITERIA: Official formula parity pinned by tests; all targeted math/DEX tests pass; no fee double-counting; no `.env` or live-execution changes; all work remains on a new feature branch rebased on current main.
ACTUAL_RESULT: Two Raydium CPMM official-fee-rounding parity tests pass; All 1,258 tests across `arb-math`, `arb-dex`, `arb-tx`, `arb-land`, and `arb-bot` passed (0 failures; 5 live-RPC tests ignored); `cargo fmt --all -- --check` and Clippy for `arb-math`/`arb-dex` passed. Clippy for `arb-tx`/`arb-land`/`arb-bot` did not complete because Cargo waited on a build-directory lock. No production formula rewrite was applied because the existing code matches the official source.
BRANCH: fix/economic-accounting-audit-20261127`r`nBASE_COMMIT: 6cbc9b6108c27a984a97b2a41265ed707b0c21ce`r`nCOMMIT: 3132e09 (rebased feature commit).
PR: #204 (open; https://github.com/mohamedazooz/SOLANA_FLASHLOAN_ARBITUGE/pull/204).
REMAINING_RISKS: No RPC simulation, Jito ingress canary, mainnet landing, or realized PnL evidence was produced. Legacy Bellman-Ford helper remains disconnected and requires a separate deprecation decision.
NEXT_TASK: Review PR #204 CI and human review; update branch if needed; do not merge automatically.
```
