# Protocol Parity & Arbitrage Remediation Rules

**Status:** Binding instructions for all agents and maintainers
**Effective:** 2026-11-27
**Applies to:** arb-dex, arb-math, arb-engine, arb-ingest, arb-tx, arb-sim, arb-land, arb-bot, and programs/arb-executor.

These rules supplement (and do not override) root AGENTS.md, docs/agents/AGENT_RULES.md, docs/active/DECISIONS.md, and role-specific instructions. In case of conflict, preserve the stricter safety invariant and escalate the conflict rather than guessing.

## 1. Evidence-first protocol work

1. Current repository code and reproducible tests are the source of truth for local behavior.
2. For protocol behavior, use official program source, official SDK/interface crates, official docs, and pinned on-chain fixtures. Third-party bots are comparative references only.
3. Before implementing a protocol change, record the exact upstream URL, crate version/commit, program version where known, and the relevant instruction/account schema.
4. README statements, code comments, enum variants, mocks, and a builder function do not prove a live integration.
5. Classify every capability precisely: Implemented, Wired, Tested, Simulated, Shadow-Tested, or Production-Verified. Never promote maturity without evidence.

## 2. Canonical architecture — no parallel paths

1. Use arb-math/src/economic.rs as the only authoritative economic/fee/profit calculator.
2. Use the existing market-state and route graph, canonical flash-loan assembler, and Supervisor execution entry point. Extend them; do not add competing production paths.
3. Trace reachability before fixing legacy code. If a path is disconnected, keep it disconnected and either deprecate it safely or add explicit tests; do not wire it into production as a shortcut.
4. A DEX route is executable only when decode, quote, account resolution, instruction building, risk gates, and simulation all agree.
5. Do not weaken existing quote-only, unpriceable, or unexecutable safety gates to increase apparent route coverage.

## 3. Exact economics and fee accounting

1. Authoritative amounts and decisions use integer/fixed-point arithmetic only. No f32/f64 in quote, sizing, routing, fee, or profit decisions.
2. Document every rounding direction and overflow bound. Use checked arithmetic; arithmetic uncertainty is a rejection, never a zero fee or a profitable fallback.
3. Count each cost exactly once. If a DEX quote already embeds LP/protocol/creator/transfer fees, do not subtract those same fees again in the PnL ledger.
4. The canonical net-profit floor must include principal repayment, flash-loan fee, network/base fee, priority fee, Jito tip, and configured minimum net profit, with mint/decimal conversion handled explicitly.
5. Never convert a fee-calculation error to zero. Never use saturating arithmetic where it can turn an underflow/overflow or loss into an apparent profit.
6. Distinguish transaction fee, refundable account rent/locked capital, protocol fees, and realized balance deltas. Refundable rent is not automatically a per-trade expense.

## 4. Account state and protocol parity

1. Validate account owner, discriminator, data length/version, mint, authority, status, and protocol-specific bounds before parsing or quoting.
2. All accounts required for a quote and transaction must be complete and coherent at a compatible slot/context. Missing bitmap extensions, tick arrays, bin arrays, config accounts, oracle state, or remaining accounts means fail closed.
3. Never assume a fixed number of tick arrays or remaining accounts is sufficient for every swap. Derive the exact account set from the official protocol algorithm/SDK and test boundary crossings.
4. Model dynamic/adaptive fees from authoritative state. Do not substitute static fee constants or guessed defaults.
5. Token-2022 support is extension-specific. Parse epoch-aware transfer fees and required transfer-hook accounts; reject unsupported or unverified extensions.
6. Add differential tests against the official SDK using identical raw account bytes, slot/time, direction, input amount, and extension state. Compare raw output, fee components, rounding, and required account metas.
7. Keep unsupported or incomplete routes explicitly gated; telemetry discovery must not imply quote or execution support.

## 5. Flash-loan and transaction invariants

1. Borrow, all swap legs, profit assertion/min-output protection, and repayment remain in one atomic Solana transaction.
2. Preserve the existing canonical Kamino integration and exact repayment formula; do not introduce a second fee implementation.
3. Never claim Marginfi, Solend, or another lender is integrated based only on an enum, mock, or research note.
4. Never split a flash loan across transactions or Jito bundle members.
5. Do not strip safety instructions, silently downgrade transaction versions, or mutate account metas to fit transport constraints.
6. V1-to-Jito stays disabled until a dated, reproducible end-to-end ingress canary proves acceptance. Use only a transport/version combination whose limits have been verified.
7. Keep ARB_LIVE=0, DRY_RUN=true, and shadow mode. No autonomous live broadcast or safety-gate change without explicit operator authorization.

## 6. Fix workflow — mandatory

For each defect:

1. Re-check the current branch, HEAD, worktree, and existing uncommitted changes. Never reset, stash, checkout, or overwrite operator changes without explicit approval.
2. Reproduce the defect with a focused test or trace the active call path before editing.
3. Write the root cause and acceptance criteria in the issue/task register.
4. Make the smallest cohesive fix in the canonical path.
5. Add regression tests, property tests, and official-SDK differential fixtures as applicable.
6. Run targeted tests first, then crate/workspace checks relevant to the changed code. Record exact commands and counts.
7. Inspect git diff for unrelated edits, formatting churn, accidental fixture churn, and secret exposure.
8. Update OPEN_ISSUES.md, CURRENT_STATUS.md, and task/handoff records only with verified evidence.
9. Do not claim simulation, landing, or realized PnL unless the corresponding evidence exists.
10. Keep changes on a feature branch. Do not push, open a PR, merge, or change live settings unless the operator asks for that step.

## 7. Priority order for the current remediation pass

Follow the order in docs/active/research/REFERENCE_IMPLEMENTATION_COMPARISON_2026-11-27.md:

1. Kamino Reserve decoder differential verification.
2. Raydium CLMM complete account dependency resolution.
3. PumpSwap V2 remaining-account correctness.
4. Orca adaptive-fee correctness or explicit rejection.
5. Meteora DAMM V2 quote/instruction parity while preserving quote-only gate.
6. Live simulation matrix and Jito V1 compatibility evidence.
7. Legacy economics cleanup only after reachability analysis.

Do not work on several overlapping protocol files concurrently unless ownership and interfaces are explicitly partitioned.


## 8. Dependency license gate

Never add or vendor an official SDK dependency whose license is outside the repository's deny.toml allowlist. Do not weaken or bypass cargo-deny to obtain a reference parser. If the official SDK is license-incompatible, use official public source as a schema reference plus pinned public-chain fixtures and independent tests; record that this is not an SDK differential test. Any allowlist change requires explicit legal/license review.
