# Current Verified Status

**Authoritative Git Baseline**: Commit `1ff841a` on `main` (merge of PR #164, 2026-11-25)
**Current Branch**: `fix/deep-quote-accounting-hardening-20261125` (pushed to `origin`; fee-audit implementation commit `db3f26b`; not merged)
**CI Status**: Vercel status check succeeded for fee-audit commit `db3f26b`; no GitHub Actions workflow run is attached to the branch push.
**Compilation Baseline**: Latest full local workspace run: 1,805 passed / 0 failed / 7 ignored; `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets -- -D warnings` both pass.

## Local Hardening in Progress — 2026-11-25

- Canonical success PnL from `arb-math` is now shared by Chronos, RiskGate and telemetry; Chronos no longer re-deducts landed costs.
- Chronos EV approval uses conservative Q32 survival probability and integer lamport/USD-micro arithmetic; floats remain display/config-boundary projections only.
- RiskGate converts input notional to Lamports with ceiling and compares it to net Lamports; the BPS floor rounds upward through one shared `arb-risk` helper also used by the validator.
- TOML config structs reject unknown fields; `load_strict` enforces its explicit top-level allowlist. The read-only validator now sizes probes from the live USD price and `.env` SOL/USD/portfolio caps, uses configured slippage and applies the same absolute/BPS net hurdle.
- ROI environment overrides can only tighten the configured phantom-quote ceiling; zero/invalid SOL reserve overrides cannot disable the reserve guard.
- Canonical env names are `MAX_PRIORITY_FEE_SHARE_BPS`, `MAX_INFLIGHT_BUNDLES`, and `MAX_CONSECUTIVE_FAILURES`; legacy aliases remain supported. Unused gas-budget keys were removed from the local ignored `.env`.
- `.env` and `config/config.toml` remain local/ignored; only `.env.example` is tracked. No secret values are committed.
- Live execution remains disabled. Historical `ISSUE-P0-04` and empirical landing/fee calibration remain open blockers.
- Read-only mainnet validation (2026-11-25 19:17Z, slot 473871973): 5/9 registered pools priced (55%); 10 graph edges; 0 cycles; probe cap 0.020000 SOL ($1.4082); configured minimum net hurdle 10,000 lamports / 1 bps. Best tested route gross edge was -250 lamports and net was -19,921 lamports at 0.001 SOL after flash fee, Jito p75 benchmark (14,661), and 5,000-lamport base signature fee. No transactions submitted.
- Whirlpool-vs-Jupiter venue-restricted quote at 0.020000 SOL differed by 0.0480% (4.80 bps), within the 10 bps slippage ceiling; exact pool/account alignment and same-slot comparison remain an investigation item.

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
## Transaction Fee Audit & Profit Calibration — 2026-11-25

- **Profit gate**: `.env` and all active TOML profiles now use `MIN_NET_PROFIT_LAMPORTS=10000`, `MIN_NET_PROFIT_BPS=1`, and `CHRONOS_MIN_EV_USD=0.01`. These are strategy thresholds, so explicit ENV values replace TOML values; they are not silently raised by `max()`.
- **Priority fees**: base share 200 bps (2%); local ENV caps the maximum share at 500 bps (5%). The diagnostic validator now estimates route-specific CU from the two DEX protocols plus flash-loan overhead, adds a ceiling-rounded 15% fallback margin, and computes `ceil(cu_price_micro_lamports * requested_cu_limit / 1_000_000)`. Production uses measured route CU with 10% headroom and keeps the wider 15% margin for unmeasured routes.
- **Priority-fee market data gap**: `adaptive_cu_price` currently receives an empty recent-fee sample list, so its CU-price input remains the 100,000 micro-lamports/CU fallback; the 5% profit-share cap prevents unbounded spend, but a background `getRecentPrioritizationFees` sampler is still needed for market-responsive bidding. Do not describe the current CU price as live-fee-oracle output.
- **Base transaction fee**: 5,000 lamports per signature. The current flash-arbitrage builder path uses the payer as its only transaction signer; signature precompiles are not used. Revisit the count if a future route adds another signer.
- **Jito**: protocol minimum is 1,000 lamports, but this is not a competitive bid recommendation. Current bidding targets 5% of available edge, caps at 10%, retains at least 90% of edge, caps the tip at 125,000 lamports, and requires at least 10,000 lamports retained. `max_tip_spend_ratio=0.10`. The validator reports the live p75 tip floor (14,661 lamports in this sample) as a market-cost benchmark, not a guaranteed fixed charge.
- **DEX fee accounting**: `amount_out_raw` is post-pool-fee and post-output-transfer-fee; `fees_breakdown` is diagnostic and must not be deducted again from that amount. Orca Whirlpool and Raydium CLMM fee breakdowns now use the exact `fee_paid` returned by the protocol-specific per-step swap traversal instead of an aggregate floor estimate. Raydium CPMM pools with enabled creator fees remain fail-closed until the generic quote path can represent that fee direction.
- **Validator configuration**: it now loads the legacy `config/base.toml` schema through `arb_bot::config::load` and fails on config errors instead of silently falling back to hardcoded 100,000 lamports / 50 bps. Its priority-fee estimate is route-sized and profit-capped.
- **Safety**: live execution remains disabled. Read-only validation submitted no transactions. The latest live graph still has no profitable cycles; lowering the hurdle does not make negative gross edges executable.

References: [Solana fee structure](https://solana.com/docs/core/fees/fee-structure), [Solana compute budget](https://solana.com/docs/core/fees/compute-budget), [Jito low-latency transaction and bundle docs](https://docs.jito.wtf/lowlatencytxnsend/), [Orca Whirlpool swap math](https://github.com/orca-so/whirlpools/blob/main/programs/whirlpool/src/math/swap_math.rs), [Raydium CPMM source](https://github.com/raydium-io/raydium-cp-swap).
