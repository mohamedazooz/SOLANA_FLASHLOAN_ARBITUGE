# Solana Flash Loan Arbitrage — Active Engineering Documentation

Welcome to the active, authoritative documentation suite for the Solana Flash Loan Arbitrage Bot.

> **Operational Principle**: The repository’s current Rust code (`crates/`) and reproducible test evidence are the sole sources of truth. Documentation, previous claims, and TODOs are not proof that a feature works.

---

## 1. Documentation Map

Every agent, engineer, and maintainer operating in this repository must consult the following active documents:

| Document | Purpose |
|---|---|
| [`MASTER_ROADMAP.md`](../../docs/active/MASTER_ROADMAP.md) | **Single Authoritative Roadmap**: Sequential phases from Phase 0 baseline through Phase 19 production readiness. |
| [`CURRENT_STATUS.md`](../../docs/active/CURRENT_STATUS.md) | **Current Verified Baseline**: Exact commit, compiler, passing tests, and verified vs pending capabilities. |
| [`CURRENT_ARCHITECTURE.md`](../../docs/active/CURRENT_ARCHITECTURE.md) | **Authoritative System Architecture**: Crate breakdown, dataflow, concurrency model, and memory layout. |
| [`OPEN_ISSUES.md`](../../docs/active/OPEN_ISSUES.md) | **Forensic Defect Register**: Prioritized defect register (P0, P1, P2) covering correctness, sizing, and simulation. |
| [`DECISIONS.md`](../../docs/active/DECISIONS.md) | **Engineering Decisions Register**: Single sources of truth, ADR references, integer economics, and V1 wire rules. |
| [`MASTER_TASK_REGISTER.md`](../../docs/active/MASTER_TASK_REGISTER.md) | **Master Task Register**: Granular task tracking using strict handoff schemas across all phases. |
| [`TEST_STRATEGY.md`](../../docs/active/TEST_STRATEGY.md) | **Comprehensive Testing Strategy**: Unit, property, differential, fuzz, integration, and simulation suites. |
| [`PRODUCTION_READINESS.md`](../../docs/active/PRODUCTION_READINESS.md) | **Production Acceptance Matrix**: Formal verification gates, security audit, and live-arming requirements. |
| [`BASELINE_AUDIT.md`](../../docs/active/BASELINE_AUDIT.md) | **Phase 0 Baseline Audit**: Complete workspace dependency, environment variable, and compiler audit. |
| [`COMPONENT_STATUS.md`](../../docs/active/COMPONENT_STATUS.md) | **Component Verification Matrix**: Implemented vs wired vs tested vs simulated vs production-verified. |
| [`CALL_GRAPH.md`](../../docs/active/CALL_GRAPH.md) | **Runtime Call Graph**: Detailed execution trace from startup through ingestion, quoting, sizing, and dispatch. |
| [`DOCUMENTATION_INVENTORY.md`](../../docs/active/DOCUMENTATION_INVENTORY.md) | **Repository Documentation Inventory**: Comprehensive classification of all active and archived files. |
| [`research/`](../../docs/active/research/README.md) | **Phase 1 Research Dossier**: Verified online/open-source best-practice findings per domain (Solana core, Jito, Yellowstone, DEX, flash loans, HFT, security). |
| [`audit/FLASH_LOAN_FORENSIC_AUDIT.md`](../../docs/active/audit/FLASH_LOAN_FORENSIC_AUDIT.md) | **Phase 2 Flash Loan Forensic Audit**: 19-step lifecycle verification against the official Kamino clone, subsidy-exposure coverage matrix, findings FL-F-2026-11-22-N1/N2, ISSUE-P0-04 status. |
| [`audit/FLASH_LOAN_REGRESSION_MATRIX.md`](../../docs/active/audit/FLASH_LOAN_REGRESSION_MATRIX.md) | **Phase 2 Regression Matrix**: FL-01…FL-22 invariants × guard location × test × evidence. |
| [`audit/DEX_SUPPORT_MATRIX.md`](../../docs/active/audit/DEX_SUPPORT_MATRIX.md) | **Phase 3 DEX Support Matrix**: per-protocol A–H pillar classification, verified program IDs, dormant-approximation findings. |
| [`audit/QUOTE_PARITY_REPORT.md`](../../docs/active/audit/QUOTE_PARITY_REPORT.md) | **Phase 3 Quote Parity**: differential-vs-official-SDK evidence per venue, honest parity gaps. |
| [`audit/ARBITRAGE_ENGINE_AUDIT.md`](../../docs/active/audit/ARBITRAGE_ENGINE_AUDIT.md) | **Phase 4 Arbitrage Engine Audit**: BF-discovery-only policy, integer re-derivation ladder, executability gates. |
| [`audit/ECONOMIC_INVARIANTS.md`](../../docs/active/audit/ECONOMIC_INVARIANTS.md) | **Phase 4 Economic Invariants**: EI-01…EI-12 with enforcement points and tests. |
| [`audit/TRANSACTION_COMPILER_AUDIT.md`](../../docs/active/audit/TRANSACTION_COMPILER_AUDIT.md) | **Phase 5 Compiler Audit**: SIMD-0385 layout re-verified, CB strip + fail-closed index rewrite, V0/V1 gates. |
| [`audit/TRANSACTION_COMPATIBILITY_MATRIX.md`](../../docs/active/audit/TRANSACTION_COMPATIBILITY_MATRIX.md) | **Phase 5 Compatibility Matrix**: format × transport × runtime, no-downgrade rules. |

| [`security/ONCHAIN_PROGRAM_SECURITY_AUDIT.md`](../../docs/active/security/ONCHAIN_PROGRAM_SECURITY_AUDIT.md) | **Phase 6 On-Chain Program Security**: delta-based profit guards, signer/program/pool/tip validation, no-unsafe/no-panic audit. |
| [`security/PROGRAM_DEPLOYMENT_VERIFICATION.md`](../../docs/active/security/PROGRAM_DEPLOYMENT_VERIFICATION.md) | **Phase 6 Deployment Verification**: NOT VERIFIED verdict + prerequisites before live use. |
| [`architecture/MARKET_DATA_PIPELINE.md`](../../docs/active/architecture/MARKET_DATA_PIPELINE.md) | **Phase 7 Market Data Pipeline**: ingestion topology, coherence/integrity primitives, 11-rung latency ladder with wired call sites. |
| [`performance/HFT_ARCHITECTURE.md`](../../docs/active/performance/HFT_ARCHITECTURE.md) | **Phase 7 HFT Architecture**: hot-path concurrency audit, batching wins, evidence-first optimization policy (no unsafe, no speculative changes). |
| [`performance/LATENCY_BENCHMARKS.md`](../../docs/active/performance/LATENCY_BENCHMARKS.md) | **Phase 7 Latency Benchmarks**: stage-ladder metrics × sources, bench inventory, captured test evidence, honest NOT-EXECUTED gaps. |

---

## 2. Agent Operating System

All autonomous agents and human developers must follow the specialized role definitions and handoff protocols in [`docs/agents/`](../../docs/agents/):
- [`docs/agents/README.md`](../../docs/agents/README.md) — System overview
- [`docs/agents/AGENT_RULES.md`](../../docs/agents/AGENT_RULES.md) — Immutable operational rules
- [`docs/agents/SUPERVISOR.md`](../../docs/agents/SUPERVISOR.md) — Architecture consistency & verification
- [`docs/agents/QUANT_AGENT.md`](../../docs/agents/QUANT_AGENT.md) — Quoting math, fees, and sizing
- [`docs/agents/INGESTION_AGENT.md`](../../docs/agents/INGESTION_AGENT.md) — WebSocket feeds & state integrity
- [`docs/agents/EXECUTION_AGENT.md`](../../docs/agents/EXECUTION_AGENT.md) — Flash loans, V1 compiler, and landing
- [`docs/agents/HANDOFF_PROTOCOL.md`](../../docs/agents/HANDOFF_PROTOCOL.md) — Mandatory structured handoff schema

---

## 3. Core Repository Invariants

1. **Safety Defaults**: `ARB_LIVE=0`, `DRY_RUN=true`, shadow mode. Live execution requires explicit operator configuration.
2. **Never Touch `main` Directly**: Work strictly on dedicated feature branches and submit pull requests.
3. **No Stale Fallbacks**: Fail closed when data feeds fail or lag; never serve cached market data as fresh.
4. **Zero Floating-Point in Authoritative Decisions**: Use integer/fixed-point arithmetic (`u64`, `u128`, `ruint::Uint<256, 4>`) for token amounts and financial calculations.
5. **Atomic Flash Loans**: Borrow, all swaps, and repayment must execute in a single atomic transaction. No bundle splitting.
