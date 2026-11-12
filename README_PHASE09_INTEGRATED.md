# Solana Arbitrage — Integrated Phase-09 Workspace

This is one continuous Rust workspace. Phase numbers are staged acceptance gates, not independent products.

## Start here

1. `docs/merge/BASELINE_AND_MERGE_MANIFEST_2026-11-07.md` — source precedence and merge record.
2. `docs/phases/PHASE_09A_CLMM_DLMM_MATH_AND_ADAPTERS.md` — CLMM/DLMM mathematical and adapter scope.
3. `docs/phases/PHASE_09A_2_YELLOWSTONE_STATE_INGESTION.md` — current native gRPC/account-state implementation.
4. `docs/math/CLMM_DLMM_MATH_AUDIT_2026-11-07.md` — code-level fixes and remaining risks.
5. `docs/phases/PHASE_09B_RISK_ECONOMICS_EXECUTION.md` — economics, sizing, routing, settlement.
6. `docs/phases/PHASE_09C_DATA_PROVIDERS_AND_EVENT_TRANSPORT.md` — agreed no-HTTP-RPC architecture, three generic WebSocket slots and Kafka.
7. `docs/research/PHASE_09A_2_ONLINE_RESEARCH_2026-11-07.md` — official protocol/SDK research links.
8. `docs/reference/remediation-v1/` — preserved reference contract, not production-wired.

## Important status

- Shadow-only. No Live/Canary execution is accepted by this patch.
- Solami Yellowstone gRPC is now the native account/slot stream; the old fake JSON-RPC-over-WebSocket Yellowstone implementation was replaced.
- Runtime HTTP JSON-RPC is disabled: `RpcPool::from_env()` returns an empty pool, and Supervisor no longer performs HTTP RPC polling, HTTP catalog discovery or RPC slot fallback. Legacy HTTP JSON-RPC code remains in the workspace for isolated tests and not-yet-migrated code; it is not activated by runtime configuration.
- Three provider-neutral WebSocket environment slots exist, but the generic WebSocket failover adapter is not implemented yet.
- Raw Yellowstone accounts are stored with slot/write-version ordering. They are not yet fully promoted into quote-ready pool/vault/TickArray/BinArray/Kamino state.
- Per-pool cache updates no longer clone the full market-state map. Strategy snapshots copy values once per cycle.
- CLMM quotes now preserve Orca's native fee-rate precision and require TickArray coverage; Raydium CLMM still needs a separate native parser/adapter and bitmap-extension traversal.
- DLMM reserve conversion rejects out-of-range values instead of clamping. Full Meteora BinArray and dynamic-fee parity remains to be completed.
- The previous Telegram/control-panel build is not present in the available full-workspace archives used for this merge; this package does not claim to include it.

## Validation

Run on a Rust-enabled host:

```bash
cargo fmt --all --check
cargo check --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

The packaging environment does not include Cargo/rustc, so these commands have not been executed here. Static archive validation is not a successful Rust build.

## Financial F1 update (2026-11-07)

See `docs/financial-engine/FINANCIAL_ENGINE_F1_IMPLEMENTATION_2026-11-07.md` and `docs/RELEASE_READINESS_AND_REFACTOR.md`. The signed economic evaluator now retains losses and authorizes profitability against conservative minimum output. The legacy `live-single-exec` binary target is quarantined. This is not a live-trading approval; build/test and the listed protocol/execution blockers remain.

## Pump.fun / PumpSwap adapter update (2026-11-07)

See `docs/pump-protocols/PUMP_FUN_PUMPSWAP_ADAPTERS_2026-11-07.md` for the scope and explicit limitations. `DexProtocol::PumpSwap` and a dedicated quote adapter are now present. Pump.fun standard bonding-curve fee assumption is 125 bps. Both Pump protocols remain quote-only; the current live instruction builders and coherent Yellowstone Pool+vault+fee-config join are not complete. The next implementation phase is `docs/phases/F2_PUMP_STATE_JOIN_AND_EXECUTION.md`.
