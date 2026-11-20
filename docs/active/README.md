# Solana Flash Loan Arbitrage — Active Engineering Documentation

Welcome to the active, authoritative documentation suite for the Solana Flash Loan Arbitrage Bot.

> **Operational Principle**: The repository’s current Rust code (`crates/`) and reproducible test evidence are the sole sources of truth. Documentation, previous claims, and TODOs are not proof that a feature works.

---

## 1. Documentation Map

Every agent, engineer, and maintainer operating in this repository must consult the following active documents:

| Document | Purpose |
|---|---|
| [`MASTER_ROADMAP.md`](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md) | **Single Authoritative Roadmap**: Sequential phases from Phase 0 baseline through Phase 19 production readiness. |
| [`CURRENT_STATUS.md`](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md) | **Current Verified Baseline**: Exact commit, compiler, passing tests, and verified vs pending capabilities. |
| [`CURRENT_ARCHITECTURE.md`](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_ARCHITECTURE.md) | **Authoritative System Architecture**: Crate breakdown, dataflow, concurrency model, and memory layout. |
| [`OPEN_ISSUES.md`](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/OPEN_ISSUES.md) | **Forensic Defect Register**: Prioritized defect register (P0, P1, P2) covering correctness, sizing, and simulation. |
| [`DECISIONS.md`](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/DECISIONS.md) | **Engineering Decisions Register**: Single sources of truth, ADR references, integer economics, and V1 wire rules. |
| [`MASTER_TASK_REGISTER.md`](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_TASK_REGISTER.md) | **Master Task Register**: Granular task tracking using strict handoff schemas across all phases. |
| [`TEST_STRATEGY.md`](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/TEST_STRATEGY.md) | **Comprehensive Testing Strategy**: Unit, property, differential, fuzz, integration, and simulation suites. |
| [`PRODUCTION_READINESS.md`](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/PRODUCTION_READINESS.md) | **Production Acceptance Matrix**: Formal verification gates, security audit, and live-arming requirements. |
| [`BASELINE_AUDIT.md`](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/BASELINE_AUDIT.md) | **Phase 0 Baseline Audit**: Complete workspace dependency, environment variable, and compiler audit. |
| [`COMPONENT_STATUS.md`](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/COMPONENT_STATUS.md) | **Component Verification Matrix**: Implemented vs wired vs tested vs simulated vs production-verified. |
| [`CALL_GRAPH.md`](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CALL_GRAPH.md) | **Runtime Call Graph**: Detailed execution trace from startup through ingestion, quoting, sizing, and dispatch. |
| [`DOCUMENTATION_INVENTORY.md`](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/DOCUMENTATION_INVENTORY.md) | **Repository Documentation Inventory**: Comprehensive classification of all active and archived files. |

---

## 2. Agent Operating System

All autonomous agents and human developers must follow the specialized role definitions and handoff protocols in [`docs/agents/`](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/agents/):
- [`docs/agents/README.md`](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/agents/README.md) — System overview
- [`docs/agents/AGENT_RULES.md`](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/agents/AGENT_RULES.md) — Immutable operational rules
- [`docs/agents/SUPERVISOR.md`](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/agents/SUPERVISOR.md) — Architecture consistency & verification
- [`docs/agents/QUANT_AGENT.md`](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/agents/QUANT_AGENT.md) — Quoting math, fees, and sizing
- [`docs/agents/INGESTION_AGENT.md`](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/agents/INGESTION_AGENT.md) — WebSocket feeds & state integrity
- [`docs/agents/EXECUTION_AGENT.md`](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/agents/EXECUTION_AGENT.md) — Flash loans, V1 compiler, and landing
- [`docs/agents/HANDOFF_PROTOCOL.md`](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/agents/HANDOFF_PROTOCOL.md) — Mandatory structured handoff schema

---

## 3. Core Repository Invariants

1. **Safety Defaults**: `ARB_LIVE=0`, `DRY_RUN=true`, shadow mode. Live execution requires explicit operator configuration.
2. **Never Touch `main` Directly**: Work strictly on dedicated feature branches and submit pull requests.
3. **No Stale Fallbacks**: Fail closed when data feeds fail or lag; never serve cached market data as fresh.
4. **Zero Floating-Point in Authoritative Decisions**: Use integer/fixed-point arithmetic (`u64`, `u128`, `ruint::Uint<256, 4>`) for token amounts and financial calculations.
5. **Atomic Flash Loans**: Borrow, all swaps, and repayment must execute in a single atomic transaction. No bundle splitting.
