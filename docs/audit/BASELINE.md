# PHASE 1 — Repository Baseline

**Audit branch:** `vibe/forensic-audit-3adf0b` · **Base commit (main):** `f82c837` (Merge PR #17)
**Date:** 2026-11-13 · **Auditor:** independent forensic pass, evidence-mandatory rules.

## Git state at audit start

| Item | Value |
|---|---|
| Branch | `main` (fresh, up to date with origin) |
| Head | `f82c837` — Merge pull request #17 |
| Working tree | clean (audit branch created from it) |
| Recent merges | #17 (WS coverage + DLMM/CLMM orientation + collect_fee_mode), #16, #15, #14, #13 |

## Workspace inventory (measured, not estimated)

13 crates, edition 2024, `rust-version = "1.99"` (validated on 1.99.1 via `rust-toolchain.toml`).

| Crate | src LOC | Role |
|---|---|---|
| arb-bot | 12,219 | Supervisor, strategy, dispatch, control service, Telegram |
| arb-ingest | 13,237 | Registry, discovery, WS, RPC pool, DLMM/CLMM decoding |
| arb-tx | 17,008 | Flash loan, primitives, transport (V0+ALT, V1), Jito, SWQoS |
| arb-engine | 8,600 | Adapters, quoting, cycle finders, flash arbitrage engines |
| arb-math | 7,529 | CPMM/CLMM/DLMM fixed-point math |
| arb-dex | 4,913 | DEX protocol bindings |
| arb-sim | 2,641 | LiteSVM / RPC simulation |
| arb-types | 2,678 | Shared state types |
| arb-land | 2,545 | Jito landing / tip strategy |
| arb-store | 1,381 | Persistence |
| arb-risk | 918 | Risk gates |
| arb-constants | 465 | Program IDs, seeds, anchors |
| arb-metrics | 308 | Counters |
| **Total (src)** | **74,442** | |

### Files > 1000 lines (17 — maintainability P4 finding)

registry.rs (2362), rpc_pool.rs (1595), arb-types/lib.rs (1521), arb-math/dlmm.rs (1515),
arb-tx/flash/compiler.rs (1490), arb-engine/lib.rs (1437), transport/master_alt.rs (1417),
arb-math/clmm.rs (1242), discovery.rs (1227), helius_ws.rs (1193), arb-ingest/lib.rs (1187),
supervisor/opportunity.rs (1177), dlmm_flash_arbitrage.rs (1144), adapters.rs (1125),
transport/adaptive.rs (1114), primitives/builder.rs (1012), arb-tx/lib.rs (1010).

## Entry points

- `arb-bot` bin `arb-bot` (main supervisor) and bin `alchemy-webhook`.
- Mode gating: `ARB_MODE` / `ARB_LIVE` / `DRY_RUN` / `ARB_CANARY`; live requires explicit
  truthy `ARB_LIVE=1` (main.rs:349 bails otherwise; control_service.rs:535 double-refuses).
- `shadow-only` cargo feature: compile-time containment that removes the broadcast path
  entirely (verified: `cfg(feature = "shadow-only")` blocks in dispatch.rs:77/93/162).

## CI

`.github/workflows/rust-ci.yml`: fmt check, check, test, `clippy --workspace --all-targets -- -D warnings`.
Web app build workflow (React + Vite) — both green on PR #17.

## Test suites

839 test results collected this audit: **837 passed, 2 failed** — both are
live-mainnet integration tests requiring a funded `SOLANA_PRIVATE_KEY`
(mainnet_non_broadcast_simulation ×2, `InvalidAccountForFee` = unfunded payer).
Classified **BLOCKED (environmental)**, not regressions. No production key may be
used in automated tests (audit rule 16).

## External integrations verified present in code (not assumed)

<<<<<<< HEAD
Accordingly, Phase 12.1–12.12 are **documented but not re-certified here**;
12.13–12.16 and Phase 13–15 are **not accepted as complete** based on this
baseline. Files, APIs, and documentation alone are not runtime evidence.

## Environment-variable references

This is the initial literal-reference inventory, not yet a full owner/default/
requiredness audit. It intentionally records names only. Dynamic names may also
be supplied through config fields such as `primary_token_env`.

- Runtime/configuration: `ARB_CONFIG`, `ARB_MODE`, `ARB_CANARY`, `ARB_LIVE`,
  `ARB_EVENT_LOG`, `ARB_TEST_EXIT`, `MAX_TRADE_SIZE_USD`, and `CHRONOS_*`.
- Data and stores: `SOLAMI_GRPC_ENDPOINT`, `SOLAMI_API_KEY`, `SOLAMI_WS_URL`,
  `SOLANA_RPC_URL`, `SOLANA_MAINNET_RPC_FALLBACK`, `SOLAMI_RPC_URL`,
  `DATABASE_URL`, `REDIS_URL`, `DATA_DIR`, `YELLOWSTONE_ENTRY_PARENT_UPDATES`.
- Execution and simulation: `SOLANA_PRIVATE_KEY`, `COLD_WALLET_ADDRESS`,
  `PROFIT_RECEIVER_ADDRESS`, `ENABLE_PREFLIGHT_SIMULATION`,
  `ARB_SIM_TIMEOUT_MS`, `JUPITER_API_KEY`, `JUP_API_KEY`, `KILL_SWITCH_PATH`.
- Integrations and alerts: `BITQUERY_CLIENT_ID`, `BITQUERY_CLIENT_SECRET`,
  `BITQUERY_ACCESS_TOKEN`, `BITQUERY_OAUTH_URL`, `BITQUERY_GRAPHQL_URL`,
  `TELEGRAM_BOT_TOKEN`, `TELEGRAM_CHAT_ID`, `TELEGRAM_MIN_ALERT_USD`,
  `TELEGRAM_REJECTED_ALERT_MIN_SOL`, `SOL_USD_PRICE`,
  `ALCHEMY_WEBHOOK_LISTEN`, and `ALCHEMY_WEBHOOK_SIGNING_KEY`.
- Compose interpolation also references `BOT_MODE`, `POSTGRES_USER`,
  `POSTGRES_PASSWORD`, `POSTGRES_DB`, and `RUST_LOG`.

Requiredness, safe defaults, validation, and exact runtime owners still need
verification. Do not put secret values in this document.

## Baseline commands and evidence

| Command | Result on this snapshot |
|---|---|
| `rustc --version` | Exit 0: `rustc 1.99.0` |
| `cargo --version` | Exit 0: `cargo 1.99.0` |
| `rustup show` | Exit 0; stable MSVC selected from `rust-toolchain.toml` |
| `cargo metadata --no-deps --format-version 1` | Exit 0; 14 workspace packages |
| `cargo fmt --all -- --check` | Exit 1; formatter reported existing differences, including `crates/arb-bot/src/main.rs` |
| `cargo check --workspace --all-targets` | Exit 101 after the decoder correction; compilation remains blocked by the Windows-incompatible `yellowstone-grpc-client 16.0.1` import of Tokio's Unix-only `UnixStream` |
| `cargo test --workspace --all-targets` | Exit 101 after the decoder correction; dependency compile fails on `tokio::net::UnixStream` in `yellowstone-grpc-client 16.0.1` for the Windows target |
| `cargo test -p arb-dex` | Exit 0 after the decoder correction; 18 unit tests passed |
| `cargo test -p arb-math -p arb-dex` | Exit 0 after fail-closed fixture updates; 148 tests passed across the selected targets |
| `cargo clippy -p arb-math -p arb-dex --all-targets -- -D warnings` | Exit 0 after fixing the strict-lint failures in the changed math/DEX code |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | Exit 101; compilation stops at the same Windows-incompatible `yellowstone-grpc-client` import |
| `cargo audit` | `cargo-audit 0.22.2` installed; both fetch attempts failed because the connection reset while fetching RustSec advisory DB. `--no-fetch` confirms no usable cached DB is present, so vulnerability status is unknown. |
| `cargo deny check` | `cargo-deny 0.20.2` installed. Initial run exposed removed `licenses.unlicensed` config key; removed that obsolete key from `deny.toml` to use current fail-closed defaults. Rerun: advisories, bans, and sources passed; licenses failed because all 14 workspace crates omit license expressions. No license was guessed or assigned. |
| `docker compose config -q` | Exit 0 |
| `docker build -t solana-arb-baseline:local .` | Exit 1; Docker could not resolve `static.rust-lang.org` while the floating `stable` toolchain attempted to download Rust 1.99.1 |
| Linux offline check via Docker | Exit 101 before compilation; the mounted Cargo cache has no `axum` index/package metadata |
| `gitleaks git` | Exit 1 with eight repeated generic-key detections across six commits; all correspond to two test-only URL-builder fixture lines in `solami_stream.rs`, not runtime credentials |
| `gitleaks dir crates` | Exit 1 with the same two test-only fixture detections |
| Gitleaks on `.env.example`, `config.toml`, `Dockerfile`, `docker-compose.yml`, `PROMPET.MD`, `re.md` | Exit 0; no findings |
| `git diff --check` | Exit 0 |

The Windows workspace blocker is in the published dependency source: its
`lib.rs` unconditionally imports Tokio's Unix-only `UnixStream`. The Linux
container build was blocked before Rust compilation by the moving toolchain's
network download; the offline fallback was blocked by an uncached dependency.
Neither establishes a successful Linux workspace build or substitutes for the
full acceptance suite.

## Initial findings and next gates

1. **Build portability:** workspace checks cannot compile on this Windows host
   with the locked Yellowstone client version. Resolve through supported
   dependency/platform configuration, or formally declare Linux-only support
   and validate the Linux target; do not patch Cargo's registry cache.
2. **Formatting:** formatting check fails on existing files. Avoid mass-formatting
   user-modified files without reviewing the resulting diff.
3. **Decoder regression:** the PumpSwap account decoder validated the
   `can_edit_creator_fee` byte but discarded it, leaving the public field
   uninitialized. The current local patch wires the decoded value and asserts
   present/absent layout behavior.
4. **Strict min-out expectations:** several economics/CLMM/DLMM fixtures were
   inconsistent with conservative min-out gating, explicit tick-array
   requirements, and checked fixed-point errors. Fixtures were aligned to those
   fail-closed contracts; the selected math/DEX test targets now pass.
5. **Secret scan:** the available-history findings are test-only false positives.
   No confirmed credential was detected in the scanned code/config paths; the
   history depth and release artifacts still need separate review.
6. **Acceptance evidence:** no full workspace test, completed all-features
   Clippy result, dependency audit, shadow soak, replay/fault injection,
   live-path rejection matrix, or realized PnL evidence is established by this
   baseline.
7. **Dependency policy:** current `cargo-deny` passes advisories, bans, and
   sources, but the license check fails for all 14 workspace packages because
   their manifests do not declare a license expression. Do not invent a project
   license; choose and document it before adding SPDX metadata. `cargo audit`
   remains unverified until the RustSec advisory database can be fetched.

## Re-validation pass (same day, later snapshot)

The sections above describe the earlier `feature/ci-bootstrap` worktree. The
repository has since moved: those changes were committed as `541cdec`, pushed to
`main`, and `main` is now the checked-out baseline. Re-checked on 2026-11-07.

| Item | Observed value |
|---|---|
| Repository | `https://github.com/mohamedazooz/SOLANA_FLASHLOAN_ARBITUGE` |
| Branch / HEAD | `main` = `541cdecdc2118190e012453f8767d4d37e6fba91` = `origin/main` |
| Older branch | `feature/ci-bootstrap` = `8e1b3ea` (CI commit); **not merged into `main`** |
| Commit history | `7677cbe` → `c42c387` → `541cdec` (+ agent checkpoint refs) |
| Worktree | Dirty: `deny.toml`, `docs/audit/BASELINE.md`, `docs/audit/FINDINGS_REGISTER.md` only; no source files modified |
| `git fetch --all --tags` | Exit 0; no tags |
| Toolchain | `rustc 1.99.0`, `cargo 1.99.0`, host `x86_64-pc-windows-msvc` |
| Source size | 85 `.rs` files / 1.61 MB under `crates/`; 409 `#[test]`/`#[tokio::test]` functions (static count, not executed) |
| CI on `main` | **Absent**: `.github/workflows/rust-ci.yml` exists only on `8e1b3ea` |

Baseline commands on this snapshot:

| Command | Result on this snapshot |
|---|---|
| `cargo fmt --all -- --check` | Exit 1; existing formatting drift (first shown: `crates/arb-bot/src/main.rs:350`, `:361`) |
| `cargo check --workspace --all-targets` (Windows) | Exit 101; unchanged `tokio::net::UnixStream` import in `yellowstone-grpc-client 16.0.1` |
| Docker Linux check (`rust:1.99.1-slim-bookworm`, mounted repo) | First attempt BLOCKED: Docker DNS could not resolve `index.crates.io`. Re-run with `--network host` resolves DNS; result recorded in the CI/evidence column of `FINDINGS_REGISTER.md` when it completes. |
| `docker compose config -q` | Exit 0 (previously recorded; not re-run with source changes) |

This pass did **not** run the workspace test suite, Clippy, `cargo audit`,
`cargo deny`, or any runtime/shadow session. Nothing in this section is a
release or Phase-15 acceptance claim, and no live transaction was submitted.
=======
| Integration | Where | Verified |
|---|---|---|
| Meteora DLMM | arb-ingest (decoders), arb-math (bin math), arb-dex | probe + oracle, this audit |
| Orca Whirlpools | arb-ingest (decoders @101/@133/@181/@213), arb-math (orca_whirlpools_core 2.1.1) | live probe slot 468876781 |
| Raydium V4/CPMM/CLMM | arb-ingest, arb-tx, arb-math | probe (CLMM) + discriminators |
| Kamino flash loan | arb-tx/src/flash/kamino.rs | discriminators verified earlier session |
| Jito | arb-land, arb-bot dispatch | submission path only; live landing UNPROVEN |
| WS (publicnode/Helius) | arb-ingest/helius_ws.rs | soak16: 3-min clean run |
| Yellowstone gRPC | IngestClient plumbing | present; provider exposure NOT VERIFIED |
| Telegram | arb-bot/telegram.rs | present, auth-gated |
>>>>>>> origin/main
