# Solana Arbitrage — Baseline and Merge Manifest

**Date:** 2026-11-07  
**Purpose:** One continuous workspace; phases are implementation gates, not separate products.

## Source precedence used

1. `solana-arbitrage-phase09-remediation-2026-11-07.zip` is the selected full-workspace baseline because it contains the complete workspace plus the later Phase-09 changes.
2. `solana-arb-bot.zip` was diffed against that baseline. The Phase-09 archive is a superset with targeted changes in config, RPC, supervisor, ingestion, landing, risk and webhook tests; the earlier files were not allowed to overwrite those newer files.
3. `solana-arb-remediation-v1(1).zip` is a reference contract, not a drop-in Rust crate. Its architecture, audit, integration plan, validation notes and proof-pipeline source are preserved under `docs/reference/remediation-v1/`. It is deliberately not wired into production because its types and API have not been reconciled with this workspace's `PoolState`, `ExecutionOpportunity`, `RiskGate`, transaction builder and store.

## Workspace preserved

- Root Rust workspace and lockfile.
- All crates under `crates/*`.
- Separate `webhook-service/` workspace and tests.
- Existing configs, scripts, migrations, benches, integration tests and adversarial tests.
- User-provided audit/context files `gp.md`, `gm.md`, `rep.md`.

## Important merge finding

The available full-workspace archive does **not** contain the earlier Telegram/control-panel build described in a previous handoff. No Telegram control-plane source files were present in the supplied full-workspace ZIPs at merge time. This package therefore does not claim to have merged that unavailable implementation. It preserves the existing Telegram notification code and documents this missing artifact rather than fabricating a merge.

## Change policy

- No production module is deleted solely because a static scan labels it dead.
- No DEX adapter is promoted to executable status without protocol-native account decoding, quote parity, instruction/account parity and simulation evidence.
- No live mode or transaction broadcast is enabled by this merge.
- All failed or incomplete quote paths must reject; they must not manufacture a zero-price, default-fee or stale-state quote.
- Phase acceptance requires Rust build, unit/integration/property tests, replay fixtures and independent quote comparison. Static checks alone do not constitute acceptance.

## Changes made in this integrated baseline

- Preserved the full Phase-09 workspace as the root source of truth.
- Added the remediation-v1 package as reference documentation/source, not as a competing crate.
- CLMM math now rejects zero-progress, incomplete, or iteration-capped swaps instead of returning partial output as if complete.
- CLMM step compatibility function now returns a typed error rather than fabricating an output/price on math failure.
- Orca adapter no longer uses a fixed eight-tick heuristic or fabricated one-percent price fallback. It requires decoded tick arrays and quotes through the multi-tick math path.
- Orca adapter passes the native fee-rate integer through without a lossy bps round-trip and exposes decoded arrays through `ClmmState`.
- DLMM Q64.64 exponentiation now propagates representational underflow/overflow rather than substituting zero, one or `u128::MAX` as a price.
- Engine CLMM/DLMM input and bin-reserve conversions now reject values outside `u64` instead of clamping/truncating them.

## Not claimed

This is not yet a fully protocol-verified CLMM/DLMM implementation. Raydium CLMM has no independent native adapter/parser in the current workspace. Orca and Meteora account traversal currently fetch bounded neighboring arrays, not a proven dynamic walk for arbitrary input sizes. Those are explicit Phase-09A blockers.

## Follow-up delta — Phase 09A.2 (2026-11-07)

The continuous workspace received a further source-level patch after the initial merge:

- Replaced the old pseudo-Yellowstone JSON-RPC WebSocket reader with the official Yellowstone gRPC client (`yellowstone-grpc-client 16.0.1` + `yellowstone-grpc-proto 14.0.1`).
- Solami gRPC endpoint and actual x-token value are now read directly; the prior code incorrectly passed the literal env-var name as the token and allowed Helius endpoint fallback.
- Runtime `RpcPool::from_env()` is empty; no HTTP JSON-RPC endpoints are read from `.env.example` or activated in Supervisor.
- Removed Supervisor HTTP-catalog/GMGN/DexScreener discovery, RPC-based pool refresh and RPC slot fallback. A missing/stalled Yellowstone slot watermark skips the strategy cycle.
- Health monitor now checks the Yellowstone slot watermark rather than `getSlot` over HTTP RPC.
- Added raw account snapshot store ordered by `(slot, write_version)` and per-pool update-only state mutation.
- Removed stale pool fallback in MarketGraph construction.
- CLMM state now preserves Orca's native `fee_rate`; quote math no longer derives it from truncated `fee_bps`.
- Orca TickArray PDA planning is caller-sized; CLMM traversal requires array coverage and rejects missing arrays instead of inventing spacing-only coverage.
- DLMM reserve conversion rejects values outside the `u64` domain instead of clamping.

This delta is not a complete provider migration: three generic WebSocket slots are configuration-only, fork-aware Yellowstone reconciliation is pending, raw accounts are not yet promoted into all quote-ready DEX states, and legacy HTTP JSON-RPC code remains for isolated tests/unmigrated paths. No Rust build/test/clippy was available in the packaging environment.
