# Shadow-only release candidate — 2026-11-12

## What this package guarantees by construction

- `arb-bot` is built with the `shadow-only` Cargo feature.
- The binary accepts only `--mode shadow`; other modes fail at startup.
- The shadow-only build does not load `.env`, `SOLANA_PRIVATE_KEY`, or a keypair file.
- Supervisor signer loading is compiled out for this build.
- A second guard at the dispatch boundary rejects `BotMode::Live` before any transport call.
- The Docker image is built with `--features shadow-only`; the shadow compose file mounts no wallet/keypair and does not inject a `.env` file.
- No transaction is broadcast by this artifact.

## What this does NOT claim

This is a contained shadow release candidate, **not a mathematically certified trading release**. The source archive's own audit register still identifies unresolved correctness/evidence items in ALT compilation, DLMM bin-array account selection, cross-pool optimizer assumptions, fee/unit reconciliation, data freshness and per-venue instruction parity. Some later source files appear to contain partial mitigations, but without a Rust toolchain in the build environment and a complete regression run, these cannot be signed off as fixed.

The current shadow branch can discover/quote opportunities and record simulated PnL. It does not, by itself, prove that every quote corresponds to a transaction that passes `simulateTransaction`; the source has a documented gap in mandatory non-broadcast RPC simulation on the hot path. Treat its PnL as *indicative only* until simulation integration and a seven-day soak are completed.

## Build and run

```bash
cp .env.shadow.example .env.shadow.local
# Fill only read-only market-data/RPC credentials; do not add any wallet/private key.
./run-shadow.sh
```

Alternatively: `docker compose -f docker-compose.shadow.yml up -d --build`.

Metrics are bound to `127.0.0.1:9100`; use an SSH tunnel to inspect `/metrics`.

## Verification limitation

The source workspace pins Rust 1.99.1. The packaging environment used to prepare this ZIP has no `rustc`, `cargo`, `rustfmt`, or Docker executable, so no compile/test/clippy result is asserted. Run on the target VPS before considering the package operational:

```bash
rustup toolchain install 1.99.1 --component rustfmt --component clippy
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release --locked -p arb-bot --bin arb-bot --features shadow-only
```

Then verify the running container has no signer material, has zero broadcast attempts, and sustains a seven-day data/slot freshness soak.

## Security action required before VPS use

The input archive contained a hard-coded SSH password in legacy deployment automation. Those scripts and the credential were removed from this ZIP. Treat that VPS password as exposed: change it immediately, revoke any associated SSH password access where practical, and use SSH keys with host-key verification. Also rotate any provider/API credentials that were ever stored in the removed `.env`/MCP/keypair artifacts.
