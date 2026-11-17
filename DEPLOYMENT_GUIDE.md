# DEPLOYMENT_GUIDE.md

**Repository:** SOLANA_FLASHLOAN_ARBITUGE · Branch: newrepair · Date: 2026-11-16

## Safety defaults (load-bearing)

- `ARB_LIVE=0`, `DRY_RUN=true`, shadow mode. Do not weaken without operator approval.
- Mainnet live trading disabled by default.
- Web API refuses to start without `WEB_ADMIN_PASSWORD`; binds localhost; TLS via reverse proxy.

## Setup

1. Clone repo; checkout `newrepair` (or merge target).
2. Install Rust 1.99.1 (per rust-toolchain.toml); build on Linux.
3. Copy `.env.example` → `.env`; fill `REPLACE_ME_*` with rotated keys.
4. Configure RPC/WS providers (Helius/Alchemy/Solami) with valid keys.
5. Set `ARB_MODE=simulate` initially.
6. Provide `config.toml` (copy from config.example if present) with validated thresholds.

## Verification before deploy

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace --all-targets`
- `cargo audit` / `cargo deny check`
- Run in simulate/shadow mode; confirm no live txs.

## Production promotion

Only after: full test suite green, dynamic egress capture clean, quote differential within documented tolerance, V1 size ladder verified, kill switch + circuit breakers tested. Live mainnet activation requires separate explicit authorization and dedicated keys.

## Monitoring

Track: opportunities detected/rejected, profitable/unprofitable simulations, tx built/submitted/landed/failed, bundles, net profit, execution cost, latency, RPC errors, Geyser lag, pool staleness, CU consumed. Never report estimated profit as realized.
