# Solana Atomic Arbitrage Engine

A Rust workspace for discovering, quoting, and evaluating Solana DEX arbitrage
routes. It includes market-data ingestion, protocol adapters, integer quote math,
risk checks, transaction construction, simulation interfaces, Jito-related
execution components, persistence, and metrics.

## Status

This repository is **not verified as production- or live-trading-ready**. The
runtime defaults to shadow mode, and the presence of a builder, sender, quote, or
configuration option is not proof that the complete route is safe or executable.
Protocol decoder parity, full dependency/fork reconciliation, all-in economic
validation, universal execution guards, and durable confirmation reconciliation
remain acceptance requirements. Do not treat quotes or simulations as realized
profit, and do not enable live execution based on this README.

Start with:

- [`docs/RELEASE_READINESS.md`](docs/RELEASE_READINESS.md) — Gate A–F acceptance
  sheet and the honest overall verdict.
- [`docs/PHASE_STATUS.md`](docs/PHASE_STATUS.md) — phase-by-phase status.
- [`docs/audit/FINDINGS_REGISTER.md`](docs/audit/FINDINGS_REGISTER.md) — every
  finding with status and evidence.
- [`docs/OPERATIONS_RUNBOOK.md`](docs/OPERATIONS_RUNBOOK.md) — shadow-mode
  operation, pause/resume, and incident response.
- [`docs/SECURITY.md`](docs/SECURITY.md) — secret inventory and rotation.

Historical documents (`docs/RELEASE_READINESS_AND_REFACTOR.md`,
`docs/PHASE12_11_12_12_…`) are retained for provenance; where they disagree with
the files above, the files above are current.

## Workspace crates

- `arb-bot`: bot runtime, configuration, supervisor, Telegram integration, and
  operational binaries.
- `arb-constants`, `arb-types`: shared program IDs, domain types, and errors.
- `arb-math`: AMM/CLMM/DLMM and economic arithmetic.
- `arb-dex`: DEX quote adapters and token-account decoding helpers.
- `arb-engine`: route graph/search, sizing, quote evaluation, and guarded quote
  interfaces.
- `arb-ingest`: Yellowstone gRPC ingestion, provider/account state, and protocol
  dependency/readiness logic.
- `arb-risk`: risk and execution decision gates.
- `arb-sim`: simulation interfaces; the optional `litesvm` feature is disabled
  by default.
- `arb-tx`, `arb-land`: instruction/transaction construction and landing/sender
  integrations.
- `arb-store`, `arb-metrics`: persistence and telemetry.
- `arb-mcp`: standalone MCP server/tooling.

These are workspace boundaries, not a claim that every adapter or crate is
wired into a production execution path. The current runtime and phase evidence
are recorded in the project documentation.

## Build and validation

The workspace uses Rust edition 2024 and is pinned to Rust **1.99.1** by
`rust-toolchain.toml`; the workspace declares `rust-version = "1.99"`. Earlier
declarations of 1.85 were incorrect (edition-2024 let-chains require newer
compilers).

**Linux is the reference build target.** The full workspace does not compile on
Windows because `yellowstone-grpc-client 16.0.1` imports Tokio's Unix-only
`UnixStream`. Use Linux, WSL2, or Docker.

```bash
# one-shot Linux verification (fmt, check, clippy, tests, optional deny)
bash scripts/verify-linux.sh

# or explicitly, on Linux/macOS
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --lib --bins --tests --no-fail-fast --locked
```

The Docker image builds the `arb-bot` and `alchemy-webhook` binaries. Actual
command results for this snapshot are recorded in
[`docs/TEST_EVIDENCE.md`](docs/TEST_EVIDENCE.md); a command is only "passing" for
the environment in which its log was captured.

## Configuration and operation

Use `.env.example` for variable names and placeholders, then provide deployment
secrets out of band. Do not commit credentials or private keys.

- `config/config.example.toml` is the tracked reference configuration.
- `config/config.toml` is deployment-provided and gitignored; the Docker image
  copies the example as a safe default so a missing mount fails with a clear
  configuration error.
- `KILL_SWITCH_FILE` (default `data/kill_switch`) must live on persistent
  storage; the marker is the durable pause state.
- Admin `/pause` and `/resume` require `Authorization: Bearer …` or
  `X-Admin-Token: …`; a missing or placeholder token disables the admin listener.

Compose defaults the bot to shadow mode and binds its published service ports to
loopback. Review `docker-compose.yml` before running services.

Do not enable live execution or submit a transaction without separate explicit
authorization after all required gates in
[`docs/RELEASE_READINESS.md`](docs/RELEASE_READINESS.md) have been independently
verified.

## Unified route integration status (2026-11-11)

See [`FINAL_ROUTE_INTEGRATION_STATUS.md`](FINAL_ROUTE_INTEGRATION_STATUS.md) for the consolidated venue graph and [`docs/SUPERVISOR_ROUTE_REPAIR_2026-11-11.md`](docs/SUPERVISOR_ROUTE_REPAIR_2026-11-11.md) for the supervisor, per-protocol quote, and Solana V1 repairs. The graph is WSOL-anchored to match the configured Kamino flash-loan reserve. The Shadow release profile is V1-only: no Legacy/V0 compilation, no ALTs, 4,096-byte wire ceiling, 64 inline accounts, and explicit CU/loaded-account limits. Meteora DLMM BinArray PDAs must be derived for the exact swap traversal and included inline; routes that exceed the inline account budget are rejected. V1 RPC requests use `maxSupportedTransactionVersion: 1`. Jito bundle transport is excluded from this profile. The source has not been built or tested in this environment; live execution remains disabled.


## Unified multi-venue routing (2026-11-11)

See [`docs/UNIFIED_MULTI_VENUE_ROUTE_ADAPTERS_2026-11-11.md`](docs/UNIFIED_MULTI_VENUE_ROUTE_ADAPTERS_2026-11-11.md) for the unified route graph, per-protocol adapter matrix, strict mint-continuity rules, current Pump/Raydium/Meteora/Solana updates, and remaining acceptance gates. Route discovery and quote support do not imply live execution acceptance; shadow mode remains mandatory.
