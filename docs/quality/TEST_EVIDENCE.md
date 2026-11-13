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

## Required next evidence run

Run `bash scripts/verify-linux.sh` on Linux with Rust 1.99.1 and cargo-deny installed.
Then append exact exit codes, compiler version, OS/container image, commit/archive hash,
fixture set, test counts, and log locations. Run the protocol differential and local
validator/fork suites separately; CI success alone does not prove on-chain parity.
