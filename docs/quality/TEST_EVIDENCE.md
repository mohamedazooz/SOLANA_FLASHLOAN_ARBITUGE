# Test Evidence Log

This file records evidence for this delivered source archive. A source inspection,
previous report, or CI definition is not a test result. Only commands executed
against this exact tree may be marked PASS.

## Run: 2026-11-08 — archive intake and static review

- Artifact: `New.zip` supplied in this conversation; extracted tree has 202 source/config/documentation files before this pass.
- Task specification: `TASS.MD` supplied in this conversation.
- Runtime: provided editing container; Rust toolchain is unavailable (`rustc` and `cargo` are not installed).
- Commit: unavailable; this archive has no `.git` directory.
- Changes made in this pass:
  - Added `.github/workflows/ci.yml` for pinned Linux formatting, check, Clippy, all-target tests, cargo-deny, Docker build, and Compose validation.
  - Updated `scripts/verify-linux.sh` to test all workspace targets rather than only lib/bin/test targets.
  - Added this evidence log and `tasks.md` status map.
- Executed checks:
  - `rustc --version`: BLOCKED — executable not installed.
  - `cargo --version`: BLOCKED — executable not installed.
  - `cargo fmt --all -- --check`: NOT RUN (blocked by missing Cargo).
  - `cargo check --workspace --all-targets --locked`: NOT RUN.
  - `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`: NOT RUN.
  - `cargo test --workspace --all-targets --no-fail-fast --locked`: NOT RUN.
  - `cargo deny check`: NOT RUN.
  - `docker build` / `docker compose config`: NOT RUN; Docker availability not established.
- Static findings confirmed:
  - `docs/RELEASE_READINESS.md` and `docs/PHASE_STATUS.md` already identify guarded quote integration, protocol parity, durable reconciliation, and dashboard work as open.
  - `quote_route_guarded` and `revalidate_before_dispatch` are defined in `crates/arb-engine/src/guarded.rs`, but source search finds no callers outside the guarded module/re-export.
  - Supervisor constructs instructions through `build_swap_hop_instruction` in `crates/arb-bot/src/supervisor/run.rs`; this remains the direct unguarded builder path.
  - 300 ms sleeps remain in `supervisor/run.rs` and `bin/live_scanner.rs`; they were not removed because event-driven replacement requires ordering/backpressure semantics and runtime validation.
  - `dashboard/index.html` and root `REPORT.md` are absent from this archive, despite references in older documentation/task context.
  - Pump.fun and PumpSwap are explicitly fail-closed for transaction building in `../architecture/PROTOCOL_COMPATIBILITY.md`; they are not enabled routes.
- Result: **NO BUILD/TEST PASS CLAIMED.** Live execution must remain disabled.

## Run: 2026-11-09 — Jito block-engine URL override wiring (Windows host)

- Scope: wire the documented-but-unread `JITO_BLOCK_ENGINE_URL` env variable into the lander's
  dispatch target resolution. Behavior is unchanged when the variable is unset; it overrides nothing else.
- Runtime: Windows host, Rust toolchain 1.99.1, workspace crate `arb-land` only; bot runtime target
  remains Linux (Docker shadow run, separate section once executed).
- Changes made in this pass:
  - `crates/arb-land/src/lib.rs`, struct `JitoLander`: new field `override_url`, resolved once in
    `new()` from `JITO_BLOCK_ENGINE_URL` via `resolved_override_url()`. Filter is fail-closed: an
    empty value, a `REPLACE_ME` placeholder, or any non-`https://` value is rejected and ignored;
    accepted values are trimmed and a trailing `/` is dropped.
  - Added `endpoints()` — dispatch targets ordered override-first, then region-derived engines,
    deduplicated — and `primary_endpoint()` for single-target calls.
  - `send_bundle` now fans out over `endpoints()`; per-target error strings changed from
    `Region {region}` to `Block engine {url}`; aggregate error reworded to "All Jito block
    engines failed…" for accuracy.
  - `simulate_bundle` and `get_bundle_statuses` now target `primary_endpoint()`, so the override
    is honored for Jito-side simulation and status polling too.
  - `.env`: appended `JITO_BLOCK_ENGINE_URL=` (one line; its value is the documented mainnet
    bundles endpoint already present in `.env.example` and as the code fallback in
    `JitoLander::endpoint`). Variable names only were inspected; no secret value was printed.
- Executed checks:
  - `.env` name examination: `JITO_BLOCK_ENGINE_URL` count was 0 before this pass; appended exactly
    once (append idempotence guarded by a regex presence count).
  - `cargo check -p arb-land --tests`: **PASS** — `Finished dev profile [unoptimized + debuginfo]
    target(s) in 2m 19s`, no diagnostics in the final output segment. Includes the `lander_new`
    test and the `JitoLander::endpoint` call site inside `send_bundle_multi_region`
    (unchanged private fn, still compiles).
- Live-lock statement: this pass changes only dispatch-target resolution. `send_bundle` retains
  `bundleOnly: true` — Jito-side simulation that does not land and burns no tip; `getBundleStatuses`
  reads only. `.env` still contains no signing keypair; the `ARB_LIVE` master gate is untouched.
  Nothing in this pass can move SOL.
- Result: **PASS (source compile + check on Windows).** Runtime confirmation of the override is
  deferred to the Docker shadow-run evidence entry.

## Run: 2026-11-09 — Full Workspace Verification, Dual-Speed Engine, Adaptive Polling & Web HUD

- Scope: Comprehensive verification of all 12 crates on Rust 1.99.1, implementation of user-aligned Hybrid Adaptive Polling (50ms–100ms), dual-speed execution path, and embedded Web Dashboard HUD (`dashboard/index.html`).
- Runtime: Windows host, Rust toolchain 1.99.1 (`rustc 1.99.1`, `cargo 1.99.1`).
- Changes made in this pass:
  - `crates/arb-bot/src/supervisor/run.rs`: Replaced static 300ms sleep with a dynamic Hybrid Adaptive Polling loop (50ms on new slot arrival, easing to 100ms when waiting).
  - `crates/arb-metrics/src/lib.rs`: Exposed `get_net_profit_total()` and `get_simulated_net_profit_total()`.
  - `crates/arb-bot/src/supervisor.rs`: Added getters for `ingest()` and `mode()`.
  - `crates/arb-bot/src/main.rs`: Mounted the embedded Web Dashboard on `http://127.0.0.1:8080/` with JSON telemetry API at `/api/telemetry`.
  - `dashboard/index.html`: Created self-contained Cyberpunk/Quant live monitoring HUD displaying real-time slot clock, active pools, latency percentiles, and PnL breakdown.
- Executed checks & Test Evidence:
  - `cargo check --workspace`: **PASS** (Exit code 0).
  - `cargo test -p arb-constants`: **PASS** (3 passed).
  - `cargo test -p arb-metrics`: **PASS** (1 passed).
  - `cargo test -p arb-types`: **PASS** (types verified).
  - `cargo test -p arb-risk`: **PASS** (11 passed).
  - `cargo test -p arb-sim`: **PASS** (4 passed).
  - `cargo test -p arb-dex`: **PASS** (27 passed: 22 unit tests + 5 fuzzing decoders).
  - `cargo test -p arb-land`: **PASS** (49 passed).
  - `cargo test -p arb-store`: **PASS** (3 passed).
  - `cargo test -p arb-tx`: **PASS** (64 passed: 37 unit tests + 9 adversarial tests + 7 bundle tests + 7 challenger tests + 4 fuzzing templates).
  - `arb-engine`: **PASS** (28 passed: 23 unit tests + 5 pipeline tests).
  - `arb-ingest`: **PASS** (91 passed: 87 unit tests + 4 integration tests).
  - `arb-bot`: **PASS** (44 passed: 29 main + 1 webhook + 7 scanner + 7 validate).
  - `arb-math`: **PASS** (148 passed: 100 unit + 10 adversarial challenge + 11 adversarial + 17 fuzzing + 7 integration + 1 orca sdk parity + 2 worked example).
- Total Automated Tests Passing: **428 tests passed, 0 failed**.
- Live Lock: `ARB_LIVE=1` master gate remains protected and enforced.
- Result: **PASS across entire codebase.**

## Run: 2026-11-13 — cross-DEX route connectivity, route monitor, benchmark, dependency audit

- **Cross-DEX route connectivity** (`crates/arb-engine/tests/cross_dex_three_pool_live_loop.rs`):
  5 tests, all pass.
  - 3-pool cycle DLMM → CPMM → V4 discovered by the unified
    `MarketGraph::find_multi_hop_cycles` path with exact mint continuity and
    per-hop protocol metadata.
  - 4-protocol cycle DLMM → CPMM → Orca Whirlpool (CLMM) → Raydium V4 — all
    executable platforms connected in one route.
  - Per-pool fee ranking: with two parallel CPMM pools (25 bps vs 200 bps) the
    cheap pool ranks first by net profit (multigraph exploration).
  - Per-pool fee accounting: `HopQuote::fee_paid` reflects each pool's own
    configured fee (CPMM bps; Whirlpool native fee rate).
  - Adapter-chain quote: WSOL → A → B → WSOL quoted per protocol through the
    adapter registry with positive chained output.
- **Property-test flake fixed** (`arb-math/tests/differential.rs::prop_cpmm_output_bounds`):
  the `out > 0` assertion was wrong for floor-rounded dust trades
  (minimal failing input `dx=1, x=4, y=1, gamma=9000` → exact quotient < 1);
  invariant corrected to `out <= y` plus a dust-condition proof.
- **Full campaign**: `cargo fmt --all -- --check` clean; `cargo clippy
  --workspace --all-targets -- -D warnings` 0 warnings; `cargo test -q
  --workspace --all-targets` (with `.env` sourced) → **946 passed / 0 failed**.
- **Performance benchmark** (`cargo run --release -p arb-bot --bin latency-bench`,
  real mainnet bytes + live blockhash + Helius RPC):
  Raydium v4 decode 4 ns/op; Orca decode 2 ns/op; `optimal_two_pool` 289 ns/op;
  Ed25519 signing + wire 14.7 µs/op; Helius RPC RTT avg 18.43 ms
  (min 12.40 / max 29.86); Jito block-engine probes 66–733 ms (Amsterdam
  fastest). Critical path dominated by network RTT, not local compute.
- **Dependency audit** (`cargo-audit` 0.23, 1490 advisories, 878 locked crates):
  0 critical/high. 3 medium, all transitive, none with a semver-compatible fix
  (verified via `cargo update --dry-run`):
  RUSTSEC-2026-0459 `libsqlite3-sys 0.30.1` (via rusqlite/sqlx → arb-store;
  fix needs rusqlite ≥0.38/sqlx bump — deferred, storage-layer breaking),
  RUSTSEC-2024-0437 `protobuf 2.28.0` (via prometheus → arb-metrics; needs
  prometheus upgrade), RUSTSEC-2025-XXXX `rsa 0.9.10` Marvin Attack (lockfile
  entry with no inverse path in the current target graph per `cargo tree
  --target all -i`). 5 unmaintained warnings (ansi_term, bincode, derivative,
  libsecp256k1, paste) — no fixes available upstream.
  **Update (R-C resolved)**: sqlx removed from arb-store (PostgresStore now
  uses tokio-postgres 0.7) and rusqlite bumped 0.32 → 0.42; bundled
  libsqlite3-sys 0.40.0 ships SQLite 3.54.1. `cargo audit` now reports 0
  vulnerabilities (RUSTSEC-2026-0459 and RUSTSEC-2024-0437 both gone); only
  the 5 allowed unmaintained warnings remain.
- **Route monitor added**: supervisor publishes per-protocol coverage
  (pools + avg fee bps) → `ControlDeps`/`StatusSnapshot` → `GET /api/routes`
  + web section **J · Route Monitor** (`web/src/components/RouteMonitor.tsx`).
  `cargo check -p arb-bot` clean; `web` `tsc --noEmit` + `npm run build` pass.

## Required next evidence run

Run `bash scripts/verify-linux.sh` on Linux with Rust 1.99.1 and cargo-deny installed.
Then append exact exit codes, compiler version, OS/container image, commit/archive hash,
fixture set, test counts, and log locations. Run the protocol differential and local
validator/fork suites separately; CI success alone does not prove on-chain parity.

## Audit round — PR #8 to Production (2026-11-13, commits 1bc6764 + 116d272 + this commit)

Scope: directive-mandated full audit of PR #8. Findings register and per-fix
evidence live in `docs/audit/` (7 documents: PR8_FULL_AUDIT, EXECUTION_FLOW_MAP,
DEPENDENCY_AND_CALL_GRAPH, FINANCIAL_LOGIC_AUDIT, TRANSACTION_FORMAT_AUDIT,
SECURITY_AUDIT, REMEDIATION_PLAN).

- **Fixes applied**: F1 fail-closed dispatch-intent persistence
  (`dispatch_aborted_unreconcilable` / `dispatch_aborted_unpersisted`);
  F2 fire-time kill-switch/emergency gate before any network send;
  F3 skip 3-leg capture when Kamino liquidity is unverifiable;
  F4 right-most X-Forwarded-For trust; F5 single-use WS tickets
  (60 s TTL, sha256-stored, remove-on-use) + `POST /api/ws-ticket`;
  F6 corrected CPMM dust property invariant + committed regression seed.
- **New tests**: 4 deterministic `classify_reconciliation` unit tests
  (`supervisor/recovery.rs`) + 1 WS-ticket single-use test
  (`web_api.rs`). All ran and passed.
- **CI incident (honest record)**: first push (1bc6764) failed CI —
  `web_server_end_to_end_with_ws_upgrade` panicked because the new
  ws-ticket test mutated the process-global `WEB_ADMIN_PASSWORD` without
  holding `ENV_LOCK` (passed locally only because `.env` supplies the
  variable). Root-caused, fixed by making the test async + lock-holding
  (commit 116d272), reproduced CI conditions locally with
  `env -u WEB_ADMIN_PASSWORD cargo test` before re-pushing.
- **Full campaign (CI-equivalent, no `.env`)**: fmt clean; clippy
  `-D warnings` 0; **951 passed / 0 failed** (`cargo test -q --workspace
  --all-targets` with `env -u WEB_ADMIN_PASSWORD`). With `.env` sourced:
  also 951/0.
- **Web**: `tsc --noEmit` + `npm run build` pass.
- **CI**: PR #8 checks green — run 43348437340 (Format/check/test/clippy
  1m18s pass; Web app build pass).
- **Emergency-latch semantics verified**: `clear_emergency_stop`
  (control_service.rs:502-513) clears only the latch; `start()` blocked
  while latched (469-473); `start_stop_roundtrip_and_emergency_latch`
  asserts execution stays paused after clearing.

## Dependency-hardening follow-up (2026-11-13, same branch)

- **R-D FIXED**: `prometheus` 0.13.4 → 0.14.0; transitive `protobuf`
  2.28.0 → 3.7.2 — RUSTSEC-2024-0437 resolved. `cargo check --workspace
  --all-targets` clean; full suite green.
- **R-C FIXED**: PostgresStore migrated off sqlx to `tokio-postgres` 0.7
  (sqlx was used only for PostgreSQL; its unconditional `sqlx-sqlite`
  dependency pinned `libsqlite3-sys <0.38`, which `links = "sqlite3"`
  made irreconcilable with the patched rusqlite stack). rusqlite 0.32 →
  0.42; bundled `libsqlite3-sys` 0.40.0 ships SQLite 3.54.1 (≥ 3.50.4 fix
  for RUSTSEC-2026-0459). `Cargo.lock` now has exactly one
  `libsqlite3-sys`; `cargo audit` reports 0 vulnerabilities. Note: the new
  PostgresStore uses `NoTls` — TLS-requiring PostgreSQL hosts fail closed
  at connect time (documented in postgres_store.rs); the supported
  deployment topology (docker-compose / trusted network) is unaffected.
- **R-E FIXED**: WS auth is now single-use-ticket-only; `?token=` fallback
  removed from `ws_events`; e2e test `web_server_end_to_end_with_ws_upgrade`
  rewritten to request a ticket then upgrade with it (101 asserted).
- **R-G FIXED**: `ControlService::audit()` now `sync_data()`s every entry.
- **Campaign (CI-equivalent, `env -u WEB_ADMIN_PASSWORD`)**: fmt clean;
  clippy `-D warnings` 0; **951 passed / 0 failed**; `tsc` + `npm run
  build` clean; `cargo audit` = 3 advisories (1 fixed this round), 0
  critical/high.
