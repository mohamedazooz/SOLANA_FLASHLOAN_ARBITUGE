# Repository Baseline

Captured: 2026-11-07 17:12 (+02:00), using GitHub API and the local Windows
workspace. This is an evidence snapshot, not a Phase 15 acceptance report.
Secrets and credential values are intentionally not reproduced here.

## Source of truth and workspace state

| Item | Observed value |
|---|---|
| Repository | `https://github.com/mohamedazooz/SOLANA_FLASHLOAN_ARBITUGE` |
| GitHub default branch | `main` |
| Latest GitHub `main` SHA | `c42c38777b23080a67630d4ca730d5df0a7913f1` |
| Latest `main` commit date | 2026-11-07 12:48:37 UTC |
| Local checked-out branch | `feature/ci-bootstrap` |
| Local HEAD | `8e1b3ea5225179503ae724c6805ae73cbbcc8a54` |
| Local HEAD subject/date | `ci: add GitHub Actions workflow for Rust CI`, 2026-11-07 15:02:28 +02:00 |
| Remote refs | `origin/main` = latest `main`; `origin/feature/ci-bootstrap` = local HEAD |
| Pull request | #1, open; CI bootstrap branch |
| Tags / submodules | No tags returned; no configured submodules observed |
| Worktree | Dirty: 23 tracked files modified and two untracked files (`PROMPET.MD`, `re.md`) at inspection time |

The local `main` ref matches GitHub `main`, but the active branch is a separate
feature branch and the working tree already contains substantial user changes.
No checkout, reset, merge, or fetch that would rewrite the worktree was done.
The locally modified files are:

`crates/arb-bot/src/supervisor.rs`, `crates/arb-bot/src/verified_config.rs`,
`crates/arb-dex/src/pump_swap.rs`, `crates/arb-engine/src/adapters.rs`,
`crates/arb-engine/src/guarded.rs`, `crates/arb-engine/src/lib.rs`,
`crates/arb-ingest/src/dependency_manifest.rs`, `crates/arb-ingest/src/lib.rs`,
`crates/arb-ingest/src/provider_supervisor.rs`,
`crates/arb-ingest/src/pump_state.rs`,
`crates/arb-ingest/src/pump_state_tests.rs`,
`crates/arb-ingest/src/quote_readiness.rs`, `crates/arb-ingest/src/rpc_pool.rs`,
`crates/arb-ingest/src/rpc_reconcile.rs`,
`crates/arb-ingest/src/slot_integrity.rs`,
`crates/arb-ingest/src/yellowstone.rs`, `crates/arb-math/src/clmm.rs`,
`crates/arb-math/src/economic.rs`, `crates/arb-mcp/src/tools.rs`,
`crates/arb-risk/src/lib.rs`, `crates/arb-store/src/lib.rs`,
`crates/arb-store/src/postgres_store.rs`, and `crates/arb-tx/src/kamino.rs`.

The repository root's `README.md` was inaccurate about crate names and live
capabilities; it has been corrected in this change. A missing `can_edit_creator_fee`
field assignment in the locally modified PumpSwap decoder was also fixed and
covered by its existing layout test. Neither change alters the GitHub `main`
commit or the other pre-existing working-tree changes.

## Toolchain and workspace inventory

- Host: Windows, `x86_64-pc-windows-msvc`.
- `rust-toolchain.toml`: `stable`, minimal profile, `rustfmt` and `clippy`.
- Selected local versions: `rustc 1.99.0`, `cargo 1.99.0`; Rust 1.85.0 is also
  installed locally.
- Workspace: Cargo resolver 2, Rust edition 2024, declared `rust-version = 1.85`.
- `cargo metadata --no-deps --format-version 1`: 14 workspace packages. The
  separately maintained `webhook-service` is explicitly excluded.
- The only declared opt-in package feature found in the initial manifest scan is
  `arb-sim/litesvm`; its default feature set is empty. It is not enabled by
  default.
- Dockerfile builder base is `rust:1.85-slim-bookworm`; because the repository
  toolchain file requests `stable`, the container synchronizes the stable
  channel rather than pinning an exact compiler release.
- Locked dependency versions include `yellowstone-grpc-client 16.0.1`,
  `yellowstone-grpc-proto 14.0.1`; `solana-pubkey` resolves to 2.4.0 from the
  workspace's 2.2-compatible requirement. No Anchor dependency was found in the
  workspace manifests.

Workspace packages and intended responsibility:

| Crate | Observed responsibility |
|---|---|
| `arb-bot` | Runtime, configuration, supervisor, admin/Telegram surfaces, binaries |
| `arb-constants` | Shared program constants |
| `arb-types` | Shared domain types |
| `arb-dex` | DEX adapters and token/account decoding helpers |
| `arb-math` | AMM, CLMM, DLMM, and economics math |
| `arb-engine` | Candidate routes, graph/search, sizing, quote interfaces |
| `arb-ingest` | Yellowstone and other ingest code, account/provider state |
| `arb-land` | Sender/landing and fee-floor integrations |
| `arb-metrics` | Metrics |
| `arb-risk` | Risk gates |
| `arb-sim` | Simulation interfaces |
| `arb-store` | SQLite/Postgres persistence |
| `arb-tx` | Transaction and instruction builders |
| `arb-mcp` | Separate MCP library and binary |

`arb-bot` explicitly declares `arb-bot`, `alchemy-webhook`, `live-scanner`,
`validate`, `discover`, `latency-bench`, and `replay` binaries. The existence of
these targets does not establish that they share the same production guards.

## Runtime topology and phase status (initial inspection)

The documented intended flow is Yellowstone gRPC and configured provider inputs
to account/state ingestion, dependency resolution, protocol adapters and route
search, then economics/risk, transaction construction, simulation, sender
integration, persistence, and metrics/alerts. These are crate and document
responsibilities; this baseline has **not** proven every edge is wired in one
end-to-end production path.

The existing release-readiness documentation and README state that the bot is
shadow-oriented and not live-ready. They describe the following important
limitations; revalidation against current runtime wiring is still required:

- Yellowstone account ingestion exists, but account promotion and protocol
  decoder parity are incomplete.
- HTTP JSON-RPC runtime fallback is documented as disabled; configured
  generic WebSocket provider failover is not complete.
- Guarded quote and dependency-manifest interfaces exist, but older quote APIs
  and executors still require universal enforcement proof.
- Exact Raydium CLMM / Meteora DLMM traversal, Pump migration/fee semantics,
  and differential protocol fixtures remain acceptance work.
- Local LiteSVM is optional and disabled by default.
- Durable Jito bundle/signature/finality reconciliation, persistent kill switch,
  operator authorization, risk budgets, and a connected truthful dashboard have
  not been shown to pass acceptance tests.
- No live transaction was submitted as part of this inspection.

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
| `cargo audit`, `cargo deny` | Pending installation attempt; checks have not yet been run |
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
