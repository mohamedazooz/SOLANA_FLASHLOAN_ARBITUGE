# Current Verified Status

**Authoritative Git Baseline**: Commit `a4524ba` on `main` (merge of PR #99)
**CI Status**: GitHub Actions workflow `Rust CI` passing on `main` (Run ID: 44802369313, 2026-11-20T19:27Z)
**Compilation Baseline**: `cargo check --workspace` 0 errors; `cargo test -p arb-ingest` 162 passed / 0 failed (local run 2026-11-20, rustc 1.99.1)

---

## 1. Capability Verification Matrix

Every critical capability in the codebase is explicitly classified across six standardized maturity stages:
1. **Implemented**: Code exists in `crates/`.
2. **Wired**: Connected into the active runtime loop (`crates/arb-bot/src/supervisor/`).
3. **Tested**: Verified by reproducible unit, property, or integration tests in CI.
4. **Simulated**: Verified via non-broadcast live Solana JSON-RPC `simulateTransaction`.
5. **Shadow-Tested**: Executed on a live VPS in observation/simulate mode without broadcasting trades.
6. **Production-Verified**: Landed and settled on Solana mainnet with confirmed signature and positive realized PnL.

| Capability / Subsystem | Implemented | Wired | Tested | Simulated | Shadow-Tested | Production-Verified | Notes / Gaps |
|---|:---:|:---:|:---:|:---:|:---:|:---:|---|
| **Kamino Flash Loan (Borrow/Repay)** | ✅ | ✅ | ✅ | ✅ | ⚠️ | ❌ | Tested via unit and simulation tests. Realized settlement requires live execution. |
| **Meteora DLMM (`swap2`)** | ✅ | ✅ | ✅ | ✅ | ⚠️ | ❌ | DLMM swap2 instruction builder & bin traversal verified against mainnet state. |
| **Raydium CPMM (`swap_base_input`)** | ✅ | ✅ | ✅ | ✅ | ⚠️ | ❌ | CPMM swap builder verified in 2-pool atomic execution tests. |
| **Raydium AMM v4** | ✅ | ✅ | ✅ | ⚠️ | ⚠️ | ❌ | Basic constant product quote and builder wired. |
| **Raydium CLMM** | ✅ | ✅ | ✅ | ⚠️ | ❌ | ❌ | Dynamic fee & limit order traversal guards implemented (PR #89). |
| **Orca Whirlpool** | ✅ | ✅ | ✅ | ⚠️ | ❌ | ❌ | Whirlpool swap builder implemented. |
| **Pump.fun / PumpSwap** | ✅ | ✅ | ✅ | ⚠️ | ❌ | ❌ | Bonding curve and PumpSwap adapter wired; promotion guards active. |
| **WebSocket Ingestion (Helius/RPCFast)** | ✅ | ✅ | ✅ | N/A | ✅ | ❌ | Live account and slot streaming active. |
| **Yellowstone gRPC Client** | ⚠️ | ❌ | ❌ | ❌ | ❌ | ❌ | Dependency exists; feature gated; client integration pending Phase 4. |
| **Market Graph & Cycle Search** | ✅ | ✅ | ✅ | N/A | ✅ | ❌ | Direct DLMM, Bellman-Ford, DFS multi-hop wired. Ordered route dedup now has regression coverage; live route profitability remains unproven. |
| **Integer Sizing Optimizer** | ✅ | ✅ | ✅ | N/A | ⚠️ | ❌ | Coarse-to-fine search wired. Needs joint evaluation with route topology. |
| **Economic Engine (`arb-math`)** | ✅ | ✅ | ✅ | N/A | ✅ | ❌ | Single source of truth for Kamino fee, net profit floor. Token-2022 review pending. |
| **V1 Transaction Compiler** | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ | 0x81 wire format, 4,096-byte limit, 64 accounts, 16 MiB loaded data limit enforced. |
| **RPC Preflight Simulator** | ✅ | ✅ | ✅ | ⚠️ | ⚠️ | ❌ | Typed outcomes and fail-closed response/context checks implemented and unit-tested. Historical `0x1773` remains unreproduced; no live readiness claim. |
| **Jito Bundle Transport** | ✅ | ✅ | ✅ | ⚠️ | ⚠️ | ❌ | Block engine client wired. V1 bundle support requires upstream verification. |
| **Settlement PnL Reconciliation** | ✅ | ✅ | ⚠️ | ❌ | ❌ | ❌ | State machine wired in `dispatch.rs` and `recovery.rs`. Live delta verification pending. |

---

## 2. Active Working State and Branches

- **Default Branch**: `main` (commit `a4524ba`). Working tree clean.
- **Documentation-reset PR (merged)**:
  - PR #91: `docs: master engineering roadmap, active documentation suite, agent OS, and archive reorganization` (Merged 2026-11-20T14:29:23Z). This closed the documentation-only first-PR gate; code changes are now permitted in separate PRs.
- **Phase 2 correctness PRs (merged)**:
  - PR #93: `fix: route identity and fail-closed simulation preflight` (Merged 2026-11-20T18:43:06Z).
  - PR #94: `fix: prioritize high-profit DLMM sizing basins` (Merged 2026-11-20T18:43:23Z).
  - PR #95: `feat: jointly size graph routes across liquidity boundaries` (Merged 2026-11-20T19:13:41Z).
  - PR #96: `fix: fail closed on cross-DEX simulation errors` (Merged 2026-11-20T19:19:36Z).
  - PR #97: `test: keep Orca fixture inside active tick` (Merged 2026-11-20T19:00:38Z).
  - PR #98: `fix: separate realized and estimated profit accounting` (Merged 2026-11-20T19:21:17Z).
  - PR #99: `docs: record phase two correctness gate status` (Merged 2026-11-20T19:27:54Z).
  - PR #100: `docs(architecture): complete polyglot architecture and dependency audit` (Merged 2026-11-20T18:14:00Z).

---

## 3. Phase Gate Status (2026-11-20)

- **Phase 0 (Baseline & Forensic Audit)**: VERIFIED — deliverables in `docs/active/` (BASELINE_AUDIT, CURRENT_ARCHITECTURE, COMPONENT_STATUS, CALL_GRAPH, OPEN_ISSUES).
- **Phase 1 (Documentation & Agent Reset)**: CLOSED — PR #91 merged; archive preserved under `docs/archive/` with banners; broken Windows-local `file:///c:/Users/azoozz/...` links in active docs and `AGENTS.md` repaired to repo-relative links (99 links verified resolving).
- **Phase 2 (Critical Correctness Defects)**: CLOSED — PRs #93–#98 merged with regression tests; gate status recorded in PR #99. Outstanding: ISSUE-P0-04 root-cause reproduction of `0x1773` remains open and still blocks live execution; all other P0s fixed or proven already implemented (see `OPEN_ISSUES.md`).
- **Phase 3 (WebSocket Ingestion & State Integrity)**: EXIT-GATE VERIFIED 2026-11-20 — `cargo test -p arb-ingest` passes: 145 unit + 3 concurrency + 2 CPMM fixture + 5 Oxylabs mock + 1 pool fetch + 3 quota-exhaustion + 3 RPC telemetry = **162 passed, 0 failed**. Gate coverage confirmed in code:
  - Reconnection/backoff/rotation: `provider_recovery_cycle_backoff_rotation_then_return_after_expiry`, `recovery_cycle_all_endpoints_backoff_elects_earliest_expiry` (`helius_ws.rs`).
  - Missing-required-account fail-closed: `incomplete_hydration_missing_account_rejects_the_route` (`quote_readiness.rs`), `SnapshotReconciler::MissingAccount` (`slot_integrity.rs`).
  - Stale/duplicate/bank-tombstone state: `rejects_startup_disconnected_stale_and_dead_data`, `rejects_stale_and_mixed_bank_dependencies`, `rejects_bank_invalidated_by_entry_update_parent`, `stale_or_lagging_provider_is_not_selected`.
  - Provider failure/failover: `quota_exhaustion_tests` (429 fail-closed + healthy-endpoint failover), `failover_skips_open_circuit_and_recovers_after_success`.
  - Invalid/incomplete state cannot reach quote/exec path: `execution_guard_rejects_disconnect_generation_change_and_stale_quote`, `wrong_owner_on_tracked_account_rejects_the_route`, `empty_or_unowned_dependency_set_fails_closed`.
  - Note: these are in-process test results on the current snapshot; live provider behavior was previously shadow-tested (see capability matrix) but the Phase 5-style comparative benchmark has not been run.

---

## 4. Immediate Next Focus (Phase 4)

1. **Phase 4 (Optional Yellowstone gRPC Ingestion)**: research the official Yellowstone/Geyser protocol and a concrete managed provider (auth, filters, quotas, pricing) before implementing the feature-gated client; WebSocket path must remain independently operable.
2. **Live-execution blockers (do not clear without evidence)**: ISSUE-P0-04 `0x1773` same-snapshot quote/replay reproduction; Phase 5 comparative benchmark before any "gRPC is faster" claim; Phase 18 soak before any readiness claim.
