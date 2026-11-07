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

See [`docs/RELEASE_READINESS_AND_REFACTOR.md`](docs/RELEASE_READINESS_AND_REFACTOR.md)
and [`docs/PHASE12_11_12_12_PROTOCOL_DEPENDENCIES_AND_GUARDED_QUOTE_2026-11-07.md`](docs/PHASE12_11_12_12_PROTOCOL_DEPENDENCIES_AND_GUARDED_QUOTE_2026-11-07.md)
for documented capabilities and known limitations.

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

The workspace uses Rust edition 2024 and declares a minimum Rust version of
1.85. The Docker image builds the explicitly selected `arb-bot` and
`alchemy-webhook` binaries for Linux.

```powershell
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
```

These commands must pass on the target environment before a release is accepted.
For the current inspection snapshot and its actual command results, see
[`docs/audit/BASELINE.md`](docs/audit/BASELINE.md).

## Configuration and operation

Use `.env.example` for variable names and placeholders, then provide deployment
secrets out of band. Do not commit credentials or private keys. Review
`config.toml` and `docker-compose.yml` before running services. Compose defaults
the bot to shadow mode and binds its published service ports to loopback.

Do not enable live execution or submit a transaction without separate explicit
authorization after all required gates have been independently verified.
