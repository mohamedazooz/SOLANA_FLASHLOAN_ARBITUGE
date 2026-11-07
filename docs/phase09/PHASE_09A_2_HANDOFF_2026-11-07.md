# Phase 09A.2 handoff — 2026-11-07

## Delivered in this patch

- Native Yellowstone gRPC client replaces the fake JSON-RPC-over-WebSocket implementation in `arb-ingest/src/yellowstone.rs`.
- Solami gRPC endpoint and actual token value are read from environment; the previous code passed the literal environment-variable name as the token.
- Runtime HTTP JSON-RPC provider factory is disabled (`RpcPool::from_env()` returns no endpoints).
- Supervisor no longer performs HTTP catalog/trending discovery, HTTP-RPC pool polling, or RPC slot fallback.
- Strategy cycles skip when the Yellowstone slot watermark does not advance.
- Raw account snapshot store applies one account at a time and rejects older slot/write-version pairs.
- MarketStateCache applies one pool update at a time without cloning the full map.
- Stale pool fallback in graph construction was removed.
- Orca tick traversal now requires current TickArray coverage and advances to array edges; absent adjacent arrays return a typed coverage error.
- TickArray PDA generation is caller-sized rather than fixed to three arrays.
- Tick parser rejects malformed initialized flags and malformed tick fields.
- DLMM reserve conversion rejects values outside u64 rather than clamping.

## Not delivered / not claimed

- No generic WebSocket 1/2/3 failover runtime yet; only the environment contract exists.
- No fork-aware Yellowstone state rollback/replacement yet.
- No automatic pool -> vault -> TickArray/BinArray dependency planner yet.
- No Raydium CLMM native account adapter yet.
- No Meteora BinArray account decoder or full dynamic-fee parity yet.
- No quote-state promotion from raw Yellowstone account store yet.
- No Cargo build/test/clippy or Docker build was possible in this environment.

## Runtime behavior now

The system is intentionally fail-closed. If Solami Yellowstone gRPC is not configured, the stream does not provide a slot watermark and the strategy loop does not proceed. Even when connected, raw account records are not considered quote-ready until the next decoder/promotion stage is completed. Verified Kamino inputs currently reject because they have not yet been promoted from Yellowstone account snapshots.

## Commands to run on a Rust-enabled host

```bash
cargo fmt --all --check
cargo check --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Do not enable Live/Canary. This patch is a Shadow-only ingestion foundation.

## Build-system caveat

The workspace `Cargo.lock` could not be regenerated in this environment because Cargo/rustc are unavailable. The manifest now declares Yellowstone client/proto dependencies, so the first Rust-enabled verification run must update the lockfile (`cargo check --workspace --all-targets`) and then commit the regenerated `Cargo.lock`. Do not use `--locked` against this handoff until that update is done.
