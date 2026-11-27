# Binding Agent Operational Rules & Directives

These rules are permanent, mandatory, and binding on every autonomous agent and engineer operating in this repository.

---

## 1. Core Operating Principles

1. **The Code and Tests are Truth**: The repository’s current Rust code (`crates/`) and reproducible test evidence are the sole sources of truth. Documentation, previous reports, TODOs, and agent claims are not proof that a feature works.
2. **Never Rewrite from Scratch**: Do not rewrite the codebase or crates from scratch unless a documented technical case with empirical evidence proves it is necessary.
3. **Preserve History (Zero Deletion)**: Never delete historical documentation, reports, test evidence, Git history, branches, or commits as part of any cleanup.
4. **Distinguish Capability Maturity**: Clearly distinguish between:
   - `Implemented` (code exists)
   - `Wired` (connected into runtime dataflow)
   - `Tested` (verified by CI test)
   - `Simulated` (proven via RPC `simulateTransaction`)
   - `Shadow-Tested` (executed on VPS without broadcasting)
   - `Production-Verified` (settled on mainnet with positive realized PnL)
5. **No Stale-Cache Fallback**: Fail closed immediately if an authoritative market data feed fails or disconnects. Never serve cached state as fresh.
6. **Single Canonical Implementation**: Exactly one economic calculator (`arb-math/src/economic.rs`), one market graph, one flash loan builder, and one supervisor entry point. Parallel implementations are prohibited.
7. **Pure Integer / Fixed-Point Arithmetic**: Never use floating-point numbers (`f32`/`f64`) in authoritative economic, routing, sizing, or profit assertion decisions.
8. **Never Equate Estimated Profit with Realized Profit**: Realized profit is measured exclusively by confirmed post-execution balance changes in the reference asset.
9. **Zero Autonomous Live Execution**: Never broadcast a live transaction unless the human operator has explicitly configured `ARB_LIVE=1` and `ARB_MODE=live`.
10. **Zero Hallucination of SDKs and Protocols**: Search live documentation, GitHub, `docs.rs`, and on-chain program schemas before writing protocol integration code. Record exact URLs, commit hashes, or program versions.
11. **Maintainable Modular Architecture**: Keep modules focused, cohesive, and maintainable. Prefer files below ~500 lines unless strong domain cohesion requires otherwise.
12. **Zero Code Truncation**: Never deliver lazy stubs, incomplete blocks (`// TODO`, `// rest of code...`), or truncated implementations. Deliver complete, compile-ready code with exhaustive error handling.

---

## 2. Git and Branching Workflow

1. **Never touch `main` directly**: All development occurs on dedicated feature branches (`feat/...`, `fix/...`, `test/...`).
2. **Clean Baseline Check**: Verify the working tree is clean or securely back up working changes to a dedicated backup branch before switching branches.
3. **Verified PR Creation**: Open PRs with complete problem description, root cause, implementation details, regression tests, and verification evidence.
4. **Manual Merge Requirement**: Agents must not merge PRs automatically or push directly to `main`. Merging is an operator action in GitHub.


## 3. Protocol Parity Addendum — 2026-11-27

The binding rules in [`PROTOCOL_PARITY_RULES.md`](PROTOCOL_PARITY_RULES.md) apply to all protocol, economics, route, and transaction work. The reference comparison dossier is [`../active/research/REFERENCE_IMPLEMENTATION_COMPARISON_2026-11-27.md`](../active/research/REFERENCE_IMPLEMENTATION_COMPARISON_2026-11-27.md). Agents MUST read both before implementation. Do not weaken route gates, infer production support from mocks/builders, or treat third-party README claims as independent proof. Revalidate all inherited findings against the current branch and preserve pre-existing local changes.
