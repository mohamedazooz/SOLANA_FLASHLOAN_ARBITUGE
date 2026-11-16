# Initial Findings Register

This is the initial register from the inspection in
[`BASELINE.md`](BASELINE.md), not a claim that every historical finding in
`PROMPET.MD` has been rechecked. Source changes are uncommitted on top of
`main` = `541cdec`. No unrelated user changes were reverted.

| ID | Severity | Status | Evidence / root cause | Fix or remaining gate |
|---|---|---|---|---|
| BUILD-01 | Medium | OPEN | Windows `cargo check` and workspace tests fail in `yellowstone-grpc-client 16.0.1`: its library unconditionally imports Tokio's Unix-only `UnixStream`. | Verify supported target policy and dependency/platform support. Do not patch the Cargo registry cache. |
| BUILD-02 | Medium | BLOCKED | Docker build reached rustup but failed downloading the moving `stable` compiler because Docker DNS could not resolve `static.rust-lang.org`; offline Linux check could not resolve uncached `axum`. | Pin a reproducible container toolchain and rerun with a working dependency cache/network. |
| DEP-01 | Medium | PARTIAL / BLOCKED | The workspace now declares `MIT OR Apache-2.0` consistently across all 14 crates (plus `webhook-service`), so the manifest license gap that failed `cargo-deny`'s license check is closed. `cargo audit` still cannot fetch the RustSec advisory DB, and `cargo-deny` is not installed in the verification container used for this pass. | Re-run `cargo deny check advisories bans sources licenses` and `cargo audit` in an environment with network access to the advisory DB and record the result in `TEST_EVIDENCE.md`. Choosing the license expression is the project owner's decision; it is documented, not invented here. |
| DEX-01 | Medium | FIXED | `PumpSwapPoolHeader::decode` validated `can_edit_creator_fee` but discarded it, leaving the struct field uninitialized. | Field is now populated and present/absent layout assertions pass in `pump_swap::tests::decodes_pool_account_layout_and_older_short_layout`. |
| LINT-01 | Low | FIXED | Token-2022 risk scanner had a loop that always returned in its first iteration, triggering `clippy::never_loop`; it rejects every non-zero extension trailer. | Simplified to a fail-closed non-zero-trailer check; existing malformed/extension tests pass and targeted strict Clippy passes. |
| ECON-01 | Medium | FIXED | Economic tests expected profitability from expected output even when conservative minimum output did not clear the configured floor; `is_profitable` correctly uses conservative minimum PnL. | Updated fixtures/assertions for min-out costs and floor; signed extreme losses remain represented. |
| CLMM-01 | Medium | FIXED | CLMM tests relied on synthetic tick-boundary fallback or successful quotes with no required tick-array coverage. | Tests now assert missing coverage fails closed and pass explicit tick-array fixtures for route-ordering checks. |
| DLMM-01 | Medium | FIXED | Extreme-bin test expected an unrepresentable Q64.64 price to succeed, conflicting with checked overflow/underflow rejection. | Test now expects typed `MathOverflow` at those parameters and `BinIdOutOfRange` outside protocol bounds. |
| SEC-01 | Medium | NOT_REPRODUCED | Gitleaks reported eight history hits and two current-source hits in the same `#[cfg(test)]` URL-builder fixture (`solami_stream.rs`); inspection confirmed test-only context. Gitleaks found no hits in `.env.example`, `config.toml`, `Dockerfile`, `docker-compose.yml`, `PROMPET.MD`, or `re.md`. | No credential rotation is indicated by these findings. The history scan was limited to the six available commits; continue secret scanning on the complete intended history and release artifacts. |

### Re-validation pass — source inspection at `541cdec`

These entries were verified by reading the code at the re-validated HEAD. They
are source findings, not runtime proof; none was exercised against a cluster.

| ID | Severity | Status | Evidence / root cause | Fix or remaining gate |
|---|---|---|---|---|
| ADMIN-01 | High | FIXED | `crates/arb-bot/src/main.rs` served `/pause` and `/resume` with no authentication; `AdminConfig.token_env` was parsed but never read. | Both mutating routes now require `Authorization: Bearer` or `X-Admin-Token` (constant-time compare). Missing/empty/placeholder token disables the whole admin listener (fail closed) and logs it. Unit tests cover placeholder rejection, constant-time comparison, and both accepted header forms. No full HTTP integration test yet. |
| KILL-01 | High | FIXED | `KillSwitch` state was an in-memory `Mutex<bool>` (`crates/arb-risk/src/lib.rs`); a pause was lost on restart. | `KillSwitch::with_file` persists `data/kill_switch` (or `KILL_SWITCH_FILE`), starts blocked if the marker exists, removes it on resume, and logs persistence failures. Regression test `kill_switch_marker_survives_restart`. |
| GUARD-01 | High | OPEN | `GuardedRouteQuote`, `quote_route_guarded`, `revalidate_before_dispatch` still have no callers outside `arb-engine` and its re-export. | Remains the highest-priority execution gap: make the guarded quote the only builder/dispatch entry and assert one quote hash at risk, builder, and pre-dispatch. |
| SPLIT-01 | High | FIXED (disabled) | The split branch reused `first_hop` mints/fee for every leg and a slippage-only return hop. | Split routing is now hard-disabled (`split = None`), so the branch is unreachable. This removes the risk; split capability is not implemented and must not be re-enabled without per-leg re-quoting and aggregate min-out tests. |
| GRAPH-01 | Medium | FIXED | `max_graph_age_slots = max_state_age_slots * 4 .max(8)` had no justification. | Added explicit `strategy.graph_max_age_slots: Option<u64>` (defaults to the quote-freshness policy) in `main.rs`, `run.rs`, and both config files. Execution still revalidates at `max_state_age_slots`. |
| CANARY-01 | Medium | FIXED | `BotMode::Canary` incremented fills and realized PnL although no transaction was signed or sent. | Canary now uses `add_simulated_net_profit`, records a `canary` outcome, and labels alerts `Canary-Simulated-…`. |
| CI-01 | Medium | FIXED | `main` (`541cdec`) had no `.github/`; CI existed only on the unmerged branch. | Added `.github/workflows/ci.yml` with fmt, check (all targets), clippy `-D warnings`, tests, cargo-deny, gitleaks, and a Docker build/compose validation job. Workflow execution on GitHub is not yet observed. |
| FMT-01 | Low | FIXED | `cargo fmt --all -- --check` exited 1. | Ran `cargo fmt --all`; `cargo fmt --all -- --check` now exits 0 and passed again inside the Linux container. |
| BUILD-03 | High | FIXED | `Dockerfile` copied `/app/config` and referenced `/app/config/config.toml`, but no `config/` directory existed; compose mounted `./config`. | Added `config/config.example.toml`; the image pins `rust:1.99.1-slim-bookworm`, copies the example as a safe default config, runs non-root, and compose mounts the real config read-only. Docker build result is tracked in `TEST_EVIDENCE.md`. |
| BUILD-04 | High | FIXED | The source at `541cdec` did **not** compile on Linux: an invalid `Option<PumpFunState>` tuple pattern (`arb-ingest/src/registry.rs`), untyped `try_into` slices (`yellowstone.rs`), missing test imports, a nonexistent `ExecutionOpportunity::base_mint` (`run.rs`), and an E0521 borrow from `route_label.clone()` (`dispatch.rs`). A later all-targets run also exposed two test-only type mismatches in `arb-tx` (matching an `anyhow::Error` as `TxError`). | Each was fixed at the root cause. The final `cargo check --workspace --all-targets`, Clippy, and test results are recorded in `TEST_EVIDENCE.md`; only the run recorded there counts. Windows remains blocked by the dependency, not by these errors. |
| TEST-01 | Medium | FIXED | `arb-engine` lib test `adapters::tests::cpmm_then_clmm_quotes` failed with `CLMM quote failed: missing tick-array coverage at start tick 0`. The shared `clmm_pool` fixture declared `tick_arrays: vec![]`, which the exact CLMM traversal correctly rejects; the test was stale relative to the fail-closed math. | The fixture now supplies the tick arrays immediately above and below the active tick, so the routing test exercises real traversal instead of synthetically bypassing coverage checks. `clmm_tick_crossing` still overrides the fixture with its own initialized ticks. |
| TEST-02 | Medium | FIXED | `arb-ingest` lib test `slot_integrity::tests::recreated_bank_reopens_slot_but_not_old_descendants` failed `assertion failed: !g.is_dead(40)`. The test called `observe(..., SLOT_CREATED_BANK)` without a bank identity, while `SlotIntegrity` deliberately refuses to reopen a dead slot without one (also covered by `created_bank_without_identity_does_not_reopen_dead_slot`). | The test now uses `observe_with_bank` with explicit bank IDs for the dead and re-created banks, preserving the invariant that a re-created bank reopens only its own bank identity and never old descendants. |
| TEST-03 | Low | FIXED | `arb-ingest` lib test `quote_dependency_gate_requires_live_current_generation_data` expected `StreamDisconnected` after `mark_stream_disconnected()`, but `quote_dependencies` checked the cleared slot watermark first and returned `NoSlotWatermark`. | The connection check now precedes the watermark check; a disconnected stream reports `StreamDisconnected` (the actionable root cause) while every path still fails closed. Slot/generation/staleness checks are unchanged. |
| LINT-02 | Low | FIXED | `cargo clippy --workspace --all-targets -- -D warnings` (not covered by the earlier lib-only lint run) failed on test-only code: `items_after_test_module` in `arb-constants`, `unused_mut` in the `bitquery` WS URL test, and ten `needless_borrow` sites in `supervisor/dispatch.rs` where `&str` parameters were passed as `&&str`. | Moved the `PUMP_FUN_GRADUATION_LAMPORTS` constant above the test module, removed the redundant `mut`, and passed the `&str` fields directly. The workspace Clippy run now exits 0 with `-D warnings`; no lint suppressions were added. |

Re-verified as corrected since `re.md` (source-level; differential/runtime proof
still required):

| ID | Status | Evidence at `541cdec` |
|---|---|---|
| FIN-01 (Kamino fee) | FIXED (source) | Fee decoded from `ReserveConfig.fees.flash_loan_fee_sf` with ceiling rounding and fail-closed checks (`arb-tx/src/kamino.rs:161-249`); no bps hardcode remains on this path. |
| FIN-05 (fee default) | FIXED (source) | `PoolState::fee_bps` returns `Option<u16>` and `None` on a zero denominator (`arb-types/src/lib.rs:719-727`); no 25-bps default. |
| M-07 (DLMM phantom price) | FIXED (source) | `pow_q64` returns `Result` and rejects unrepresentable Q64.64 prices (`arb-math/src/dlmm.rs:176`, tests `:843-845`). |
| L-05 (stale-pool fallback) | FIXED (source) | The graph is built from `fresh_pools` only; no fallback to stale pools (`supervisor/run.rs:194-203`). |


The existing historical issue list in `PROMPET.MD` remains largely
**NOT REVALIDATED**. In particular, decoder parity, economics across every
executor, universal guarded-quote enforcement, simulation policy, Jito
reconciliation, operator controls, dashboard truthfulness, and shadow/fault
acceptance remain open.
