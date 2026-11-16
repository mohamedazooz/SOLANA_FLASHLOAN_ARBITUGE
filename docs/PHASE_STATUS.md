# Phase Status

Status in this file reflects source/docs inspection, the targeted remediation in
the current working tree, and the Linux verification run recorded in
[`TEST_EVIDENCE.md`](TEST_EVIDENCE.md). It is not a release approval. A phase is
not complete just because an API, type, or document exists.

Baseline: `main` = `541cdec` plus uncommitted remediation. No live transaction
was submitted.

## Phase 12 — Ingestion, dependencies, and guarded quoting

| Phase | Status | Evidence / remaining acceptance work |
|---|---|---|
| 12.1–12.10 | NOT REVALIDATED | Prior phase documentation exists; full fork, rollback, gap, freshness, and dispatch acceptance was not rerun here. Source-level guards are documented in [`audit/DATA_INTEGRITY_AUDIT.md`](audit/DATA_INTEGRITY_AUDIT.md). |
| 12.11 | PARTIAL | Dependency manifests, role/owner checks, and `quote_dependencies_for_route` readiness exist. Protocol-specific canonical decoder parity is unproven (`12.13`). |
| 12.12 | PARTIAL | Guarded quote/revalidation interfaces exist in `arb-engine::guarded`, but the supervisor still uses the direct quote path (`GUARD-01` OPEN). |
| 12.13 | OPEN | No differential fixtures against official SDK/on-chain quotes for any enabled protocol. |
| 12.14 | OPEN | Universal guarded-quote/hash enforcement through every risk, builder, and pre-dispatch path is unproven (`GUARD-01`). |
| 12.15–12.16 | OPEN | No complete snapshot-to-reconciliation E2E replay and no forced-disconnect fault injection with an in-flight quote. Slot/dead-bank/recreation units exist; they are not an E2E replay. |

## Phase 13 — Economics and protocol coverage

| Phase | Status | Evidence / remaining acceptance work |
|---|---|---|
| 13.1 | PARTIAL | `TokenAmount` raw units, checked arithmetic, explicit decimals, and `EconomicQuote` signed fields exist. A workspace-wide proof that no decision path defaults decimals/fees is missing. |
| 13.2 | OPEN | Exact quotes and official parity for all enabled protocol versions are not established. See [`PROTOCOL_COMPATIBILITY.md`](PROTOCOL_COMPATIBILITY.md). |
| 13.3 | OPEN | Pump.fun/PumpSwap execution adapters are intentionally disabled (fail closed). Cross-venue executable routing is not accepted. |
| 13.4 | OPEN | Split routing is disabled (`split = None`); the capability and aggregate min-out equivalence are not implemented (`SPLIT-01` mitigated, not fixed). |
| 13.5 | PARTIAL | `EconomicQuote` carries expected and conservative (min-out) signed net values; all route/risk/builder callers are not proven to consume one ledger. Dual USD conversion remains an open concern. |
| 13.6 | OPEN | No discrete optimizer/brute-force acceptance evidence for the sizing search. |
| 13.7 | OPEN | No verified auction/tip policy and budget acceptance evidence against live Jito conditions. |

## Phase 14 — Atomic execution

| Phase | Status | Evidence / remaining acceptance work |
|---|---|---|
| 14.1 | OPEN | Exact Kamino lender instruction/repayment semantics are implemented from state, but not proven by fork simulation of the full borrow→swap→repay route. |
| 14.2 | PARTIAL | Builders exist for Raydium v4/CPMM/CLMM, Orca, Meteora; Pump.fun/PumpSwap fail closed. Every account meta, ALT, packet, and CU path is not fork-verified. |
| 14.3 | OPEN | Same-transaction repayment is atomic by construction, but an enforceable on-chain all-cost profit guard is not accepted. |
| 14.4 | PARTIAL | RPC simulation is mandatory in Live and fail-closed; coherent-bank parity between simulated and bundle bank is not demonstrated. |
| 14.5 | OPEN | No durable accepted→landed state machine and no idempotent ambiguous-timeout reconciliation. A crash after send can leave an untracked accepted bundle. |

## Phase 15 — Operations and release

| Phase | Status | Evidence / remaining acceptance work |
|---|---|---|
| 15.1 | PARTIAL | Shadow is the default mode; canary now records simulated metrics only. A real-feed shadow soak with truthful outcomes is absent. |
| 15.2 | OPEN | Persisted operator approval, per-operator authorization, budgets, monitoring, and release gates are not accepted. Kill switch and admin auth are implemented but not drilled. |
| 15.3 | OPEN | Persistent risk controls exist (kill-switch marker test); crash/restart execution-state transition tests are missing. |

## Remediation closed in this pass (source-level)

These are recorded in [`audit/FINDINGS_REGISTER.md`](audit/FINDINGS_REGISTER.md);
they do not change the phase verdicts above.

| ID | Status |
|---|---|
| `ADMIN-01` | FIXED — admin routes require a bearer token; placeholder token disables the listener. |
| `KILL-01` | FIXED — kill switch persists to a marker file and survives restart. |
| `LIVE-03` | FIXED — `ARB_LIVE` requires an explicit truthy value. |
| `CANARY-01` | FIXED — canary records simulated metrics/outcome only. |
| `GRAPH-01` | FIXED — explicit `strategy.graph_max_age_slots`. |
| `SPLIT-01` | FIXED (disabled) — split routing unreachable. |
| `FMT-01` | FIXED — `cargo fmt --all -- --check` clean. |
| `BUILD-03` | FIXED — Dockerfile/config path resolved. |
| `BUILD-04` | FIXED — Linux compile errors at `541cdec` fixed at the root cause. |
| `CI-01` | FIXED (workflow added); execution on GitHub not yet observed. |
| `DEP-01` | FIXED (license metadata); advisory scan still blocked. |
| `GUARD-01` | **OPEN** — highest-priority execution gap. |

**Overall:** not production-ready and not Phase 15 complete. Keep live execution
disabled. No live transaction was submitted during this work.
