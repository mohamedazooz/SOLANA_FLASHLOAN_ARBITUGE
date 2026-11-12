# Decisions and Conflicts

This log records decisions made during the remediation, the evidence behind
them, and conflicts that a future reviewer must not silently "resolve" in the
opposite direction.

## D-01 — Work on `main` @ `541cdec`, not the unmerged CI branch

**Conflict:** an earlier CI workflow exists only on `feature/ci-bootstrap`
(`8e1b3ea`), which is not merged into `main`.

**Decision:** do not merge or cherry-pick the feature branch wholesale; add a
fresh `.github/workflows/ci.yml` on `main` aligned with the validated toolchain.
Merging the old branch could overwrite user changes and would reintroduce the
moving-`stable` toolchain.

**Evidence:** `git log --oneline --all`; `origin/main` = `541cdec`;
`feature/ci-bootstrap` = `8e1b3ea`.

## D-02 — Propagate the existing license, do not invent one

**Conflict:** all 14 workspace crates omitted `license`, but
`[workspace.package]` already declared `license = "MIT OR Apache-2.0"`.

**Decision:** add `license.workspace = true` to each crate and
`license = "MIT OR Apache-2.0"` to `webhook-service`. This propagates the
project's pre-existing declaration; it does not choose a new license on the
owner's behalf.

**Residual:** the owner should confirm the license is intentional before release.

## D-03 — `rust-version` must not claim 1.85

**Conflict:** the workspace declared `rust-version = "1.85"`, but the code uses
edition 2024 let-chains (`if let … && …`), which require Rust 1.88+ and were
validated on 1.99.1.

**Decision:** set `rust-version = "1.99"` (validated major/minor) and pin the
toolchain to `1.99.1` in `rust-toolchain.toml`. Claiming 1.85 was provably false
and would let a user attempt an unsupported compiler.

**Residual:** the true MSRV was not bisected; 1.99 is the validated floor.

## D-04 — Docker is the reference build environment

**Conflict:** Windows cannot compile the workspace because
`yellowstone-grpc-client 16.0.1` imports Tokio's Unix-only `UnixStream`. The
local machine also cannot download a missing Rust 1.99.1 toolchain reliably.

**Decision:** Linux `rust:1.99.1-slim-bookworm` is the reference; `scripts/verify-linux.sh`
is the single verification entrypoint. Windows remains unsupported for the full
workspace. Do not patch the Cargo registry cache or vendor a modified dependency.

**Residual:** network to crates.io is flaky; evidence is only valid for a run
that completed.

## D-05 — Disable split routing instead of shipping it unverified

**Conflict:** the split branch reused the first hop's mints/fee and a
slippage-only return hop. Fixing it requires per-leg quotes, decimals, fees, and
aggregate min-out equivalence.

**Decision:** set `split = None`; the branch is unreachable. `SPLIT-01` is
mitigated by disabling, not "fixed" as a capability. The dead code must not be
re-enabled without the full re-quote design and tests.

**Residual:** split capability is unavailable (a feature gap, not a safety gap).

## D-06 — Jupiter remains disabled

**Conflict:** a Jupiter execution path exists.

**Decision:** `route_via_jupiter = false` stays. A Jupiter quote is not proof
that the locally sized route has equivalent fees, account metas, min-out, or
flash-loan repayment semantics. Enabling it requires the same evidence as a
native protocol.

## D-07 — Pump.fun and PumpSwap fail closed

**Conflict:** quote adapters and old tests assumed Pump.fun instructions could be
built.

**Decision:** the builders return `UnsupportedProtocol` until the account matrix,
fee mode, and program version are verified. Tests now assert the failure. This is
safer than emitting a best-effort instruction matrix.

## D-08 — `ARB_LIVE` is truthiness, not presence

**Conflict:** `.env.example` ships `ARB_LIVE=0`, but the old check was
`var(...).is_ok()`.

**Decision:** only `1/true/TRUE/yes/on` authorize. The same helper is used at
startup and dispatch. `ARB_LIVE=0`/`false`/empty rejects.

## D-09 — Admin authentication is mandatory, otherwise the listener is off

**Conflict:** `/pause` and `/resume` were unauthenticated, and
`admin.token_env` was unused.

**Decision:** require `Authorization: Bearer` or `X-Admin-Token` with a
constant-time compare. If the configured env var is missing/empty/placeholder,
disable the entire admin listener (fail closed) and log it. `/health` and
`/status` remain read-only.

**Residual:** a shared bearer token is not per-operator identity; replace it
before multi-operator use.

## D-10 — Kill-switch file name

**Conflict:** the baseline notes mention `KILL_SWITCH_PATH`, while the code and
`.env.example` now use `KILL_SWITCH_FILE`.

**Decision:** `KILL_SWITCH_FILE` is canonical; default `data/kill_switch`. Update
deployment env; do not support two names silently.

## D-11 — Canary is simulated, not realized

**Conflict:** canary incremented fills/realized PnL.

**Decision:** canary writes only `simulated_net_profit_total` and a `canary`
outcome with a `Canary-Simulated-` label. A fill requires landing evidence.

## D-12 — Graph freshness is an explicit policy

**Conflict:** `max_state_age_slots * 4 .max(8)` was arbitrary.

**Decision:** add `strategy.graph_max_age_slots` (`Option<u64>`), defaulting to
`max_state_age_slots`. No arbitrary multiplier. Execution still revalidates at
`max_state_age_slots`.

## D-13 — Canonical config path

**Conflict:** `Dockerfile` copied `/app/config` and referenced
`/app/config/config.toml`, but no `config/` directory existed; the root
`config.toml` was the real file.

**Decision:** `config/config.example.toml` is the tracked reference;
`config/config.toml` is gitignored and deployment-provided. The image copies the
example as `/app/config/config.toml` so a missing mount fails with a clear
configuration error, never a missing-file crash. Compose mounts `./config`
read-only.

## D-14 — Formatting is a final, separate change

**Conflict:** mass-formatting early would obscure audit diffs.

**Decision:** apply `cargo fmt --all` after the substantive fixes; keep the check
in CI. `FMT-01` closes only after the check exits 0.

## D-15 — Keep the pre-existing `deny.toml` edit

**Conflict:** `deny.toml` was already modified before this session (obsolete
`licenses.unlicensed` key removed).

**Decision:** keep it; it matches current `cargo-deny` schema and fail-closed
defaults. Do not revert user work.

## D-16 - A fail-closed guard must be modelled by its tests, not bypassed

**Conflict:** three tests written before the hardening assumed the old permissive
behaviour: a CLMM fixture with no tick arrays, a re-created bank without an
identity, and a disconnect that was expected to surface as a watermark error.

**Decision:** fix the tests to exercise the guard as designed:
`TEST-01` supplies real tick-array coverage, `TEST-02` supplies bank identities
for the dead and re-created bank, and `TEST-03` makes `quote_dependencies`
report `StreamDisconnected` before `NoSlotWatermark`. Do **not** weaken a
production guard to make a stale test pass; if a guard is wrong, change it
deliberately and document it.

**Residual:** these are still unit-level tests. Forced-disconnect and
fork-recreation E2E fault injection remains open (`12.15`/`12.16`).

## D-17 - All-targets Clippy is part of the gate

**Conflict:** `cargo check --all-targets` compiled, but `cargo clippy
--workspace --all-targets -- -D warnings` surfaced test-only errors:
`items_after_test_module` in `arb-constants`, `unused_mut` in a `bitquery`
test, and ten `needless_borrow` sites in `supervisor/dispatch.rs`.

**Decision:** treat all-targets Clippy with `-D warnings` as a required gate and
fix the sites (`LINT-02`). Test-only code is not exempt: it is compiled and run
by CI, and lint drift hides real breakage. No crate-level lint suppressions were
added.

## Open conflicts for the owner

1. Confirm the license choice.
2. Confirm whether Windows support is required; if yes, replace or feature-gate
   the Yellowstone client.
3. Decide whether split routing and Jupiter are in scope for v1; both are
   disabled today.
4. Provide a real dashboard target (Grafana/Prometheus or custom) before Gate F
   can pass.
5. Approve a credential-rotation window; it is an operator action, not a code
   change.
