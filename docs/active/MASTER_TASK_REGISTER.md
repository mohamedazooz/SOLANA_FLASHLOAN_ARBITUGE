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
STATUS: PARTIALLY_COMPLETED
OBJECTIVE: Unify WebSocket/RPC provider identities and make WebSocket disconnect/reconnect invalidate stale quote dependencies.
FILES: crates/arb-ingest/src/lib.rs, crates/arb-ingest/src/helius_ws.rs, crates/arb-ingest/src/rpc_reconcile.rs, crates/arb-bot/src/supervisor/run.rs, crates/arb-bot/src/supervisor/execution.rs, crates/arb-bot/src/supervisor.rs, crates/arb-bot/src/verified_config.rs, crates/arb-bot/src/bin/validate.rs
FUNCTIONS: AccountSnapshotStore::apply, AccountSnapshotStore::apply_batch, IngestClient::stream_connected, IngestClient::mark_helius_connected, IngestClient::mark_helius_disconnected, HeliusWsClient::seed_initial_accounts, Supervisor::run
CURRENT_BEHAVIOR: The run-loop health gate used reversed provider IDs. Its readiness flag combined two connection booleans, the run-loop accepted any healthy provider, and HTTP account seeding advanced the WebSocket slot watermark. Reconnects reused the previous stream generation; older snapshots could also block updates from a lagging endpoint.
ROOT_CAUSE: Provider identities and health mappings were independently hard-coded; RPC bootstrap and WebSocket liveness shared a watermark; readiness did not require the account-data provider; and reconnects did not reset account snapshots.
REQUIRED_CHANGE: Centralize provider IDs; require the healthy account WebSocket to authorize trading; isolate RPC clock/bootstrap observations; advance generation and clear old account snapshots on reconnect while preserving decoded pool state; let live stream data supersede bootstrap snapshots regardless of slot ordering.
TEST_PLAN: Add regression tests for provider identity, RPC-only health rejection, disconnect readiness, reconnect generation invalidation, and live-over-bootstrap replacement; run focused arb-ingest and arb-bot suites.
ACCEPTANCE_CRITERIA: RPC cannot authorize trading without a healthy account WebSocket; disconnected and prior-generation data fail readiness; live updates replace bootstrap snapshots; tests pass.
ACTUAL_RESULT: Implemented provider identity constants and fail-closed WS-only health gate; RPC bootstrap no longer advances WS liveness; reconnect resets WS watermarks and account snapshots while preserving pool cache; live data can replace bootstrap state. Formatting and `git diff --check` completed; no build or tests were run.
COMMIT: follow-up commits
PR: https://github.com/mohamedazooz/SOLANA_FLASHLOAN_ARBITUGE/pull/102
REMAINING_RISKS: Reconnection, provider-selection, and snapshot replacement behavior still needs focused regression tests and a compile run. Routes require fresh snapshots for every dependency before becoming eligible again.
NEXT_TASK: Phase 3 task 3 — verify reconnection, backoff, and resubscription lifecycle with focused regression tests.
```
