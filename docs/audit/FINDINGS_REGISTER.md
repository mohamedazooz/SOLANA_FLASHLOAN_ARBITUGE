# Initial Findings Register

This is the initial register from the inspection in
[`BASELINE.md`](BASELINE.md), not a claim that every historical finding in
`PROMPET.MD` has been rechecked. Source changes are uncommitted in the existing
`feature/ci-bootstrap` worktree. No unrelated user changes were reverted.

| ID | Severity | Status | Evidence / root cause | Fix or remaining gate |
|---|---|---|---|---|
| BUILD-01 | Medium | OPEN | Windows `cargo check` and workspace tests fail in `yellowstone-grpc-client 16.0.1`: its library unconditionally imports Tokio's Unix-only `UnixStream`. | Verify supported target policy and dependency/platform support. Do not patch the Cargo registry cache. |
| BUILD-02 | Medium | BLOCKED | Docker build reached rustup but failed downloading the moving `stable` compiler because Docker DNS could not resolve `static.rust-lang.org`; offline Linux check could not resolve uncached `axum`. | Pin a reproducible container toolchain and rerun with a working dependency cache/network. |
| DEX-01 | Medium | FIXED | `PumpSwapPoolHeader::decode` validated `can_edit_creator_fee` but discarded it, leaving the struct field uninitialized. | Field is now populated and present/absent layout assertions pass in `pump_swap::tests::decodes_pool_account_layout_and_older_short_layout`. |
| LINT-01 | Low | FIXED | Token-2022 risk scanner had a loop that always returned in its first iteration, triggering `clippy::never_loop`; it rejects every non-zero extension trailer. | Simplified to a fail-closed non-zero-trailer check; existing malformed/extension tests pass and targeted strict Clippy passes. |
| ECON-01 | Medium | FIXED | Economic tests expected profitability from expected output even when conservative minimum output did not clear the configured floor; `is_profitable` correctly uses conservative minimum PnL. | Updated fixtures/assertions for min-out costs and floor; signed extreme losses remain represented. |
| CLMM-01 | Medium | FIXED | CLMM tests relied on synthetic tick-boundary fallback or successful quotes with no required tick-array coverage. | Tests now assert missing coverage fails closed and pass explicit tick-array fixtures for route-ordering checks. |
| DLMM-01 | Medium | FIXED | Extreme-bin test expected an unrepresentable Q64.64 price to succeed, conflicting with checked overflow/underflow rejection. | Test now expects typed `MathOverflow` at those parameters and `BinIdOutOfRange` outside protocol bounds. |
| SEC-01 | Medium | NOT_REPRODUCED | Gitleaks reported eight history hits and two current-source hits in the same `#[cfg(test)]` URL-builder fixture (`solami_stream.rs`); inspection confirmed test-only context. Gitleaks found no hits in `.env.example`, `config.toml`, `Dockerfile`, `docker-compose.yml`, `PROMPET.MD`, or `re.md`. | No credential rotation is indicated by these findings. The history scan was limited to the six available commits; continue secret scanning on the complete intended history and release artifacts. |

The existing historical issue list in `PROMPET.MD` remains largely
**NOT REVALIDATED**. In particular, decoder parity, economics across every
executor, universal guarded-quote enforcement, simulation policy, Jito
reconciliation, operator controls, dashboard truthfulness, and shadow/fault
acceptance remain open.
