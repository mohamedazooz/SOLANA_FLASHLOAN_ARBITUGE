# Repository Inventory — Final Audit (PHASE A)

Baseline: branch `vibe/final-audit-3adf0b`, commit `f176e27` (includes PR #21 + shadow-mode removal, PR #22 content).
Workspace: 13 crates, ~96,871 lines of Rust (src only), edition 2024, rust-toolchain 1.99.1.

## Crate inventory

| Crate | src LoC | Responsibility | Production callers | Status |
|---|---|---|---|---|
| `arb-bot` | 12,209 | Supervisor loop, opportunity ranking, dispatch, control (Telegram/web), config | binary `arb-bot` | PRODUCTION — entry point |
| `arb-tx` | 17,071 | TxBuilder (legacy/V0+ALT/V1), Kamino flash borrow/repay builders, Meteora swap2 builder, bin-array discovery, transport (adaptive/Jito) | arb-bot, arb-engine | PRODUCTION |
| `arb-ingest` | 13,792 | RPC pool, WS subscriptions, pool discovery, registry, LbPair/BinArray decoders, dependency manifest | arb-bot | PRODUCTION |
| `arb-engine` | 8,770 | Bellman-Ford route graph, DLMM flash-arb evaluator, 3-leg meme pipeline, size optimizer, quote engine | arb-bot | PRODUCTION |
| `arb-math` | 7,529 | Authoritative economics (`economic.rs::calculate_net_profit_raw`), DLMM bin math, Kamino fee (U68F60), oracle differentials | all crates | PRODUCTION (single source of truth) |
| `arb-dex` | 4,917 | Protocol adapters: meteora_dlmm, orca_whirlpool, raydium_clmm, validation | arb-engine, arb-ingest | PRODUCTION |
| `arb-land` | 2,545 | TipStrategy (adaptive p75+25%), JitoLander (dormant), bundle telemetry | arb-bot | PARTIALLY DORMANT (`JitoLander` behind `#[allow(dead_code)]`, reserved) |
| `arb-sim` | 2,704 | LiteSVM simulation, Meteora fresh pipeline, fork simulator | tests + live pipeline probes | PRODUCTION (pre-dispatch simulation) |
| `arb-types` | 2,727 | Core types (PoolId, MintId, Lamports, Slot, BotMode) | all | PRODUCTION |
| `arb-store` | 1,381 | Account store / cache with freshness | arb-ingest | PRODUCTION |
| `arb-risk` | 918 | Risk gates | arb-bot | PRODUCTION |
| `arb-constants` | 471 | Protocol constants (program IDs, discriminators, limits) | all | PRODUCTION |
| `arb-metrics` | 308 | Metrics | arb-bot | PRODUCTION |

## Production call graph (opportunity → execution)

```
Supervisor::run (supervisor/run.rs)
 ├─ capture_three_leg_dlmm (run.rs:79)          [3-leg DLMM capture]
 │   └─ arb_engine::build_three_leg_meme_pipeline (run.rs:163)
 │       ├─ chained DLMM quotes (meteora_quote_engine)
 │       ├─ coarse→fine size search (meteora_size_optimizer)
 │       └─ net profit via arb_math::economic (single source of truth)
 ├─ economic gate (supervisor/opportunity.rs)   [re-derives net with real tip]
 ├─ risk gates (arb-risk)
 └─ dispatch (supervisor/dispatch.rs)
     ├─ Simulate mode → arb-sim LiteSVM / fork simulation, zero broadcast
     ├─ Live mode → TxBuilder compile (V0+ALT → V1 when supported) → lander
     └─ Canary mode → gated live path
```

## Duplicates found (audited, justified)

* `bin_array_index_for_bin` exists in 3 crates (`arb-tx`, `arb-dex`, `arb-math`) — all are `div_euclid(70)`, all pinned by differential tests (`phase17_pda_differential.rs` asserts byte-identical results across all three). Documented in SINGLE_SOURCE_OF_TRUTH.md.
* `calculate_kamino_flash_fee` re-exports in `arb-tx` delegate to `arb_math::economic` — no duplicate logic.

## Dead / dormant code (deliberately reserved, annotated)

* `arb-land::JitoLander`, RTT tracker, `observe_bundle_rtt` — `#[allow(dead_code)]`, reserved for bundle-landing integration.
* `dependency_manifest.rs::resolve_protocol_dependencies` — validated module, currently test-only callers; production wiring pending (documented).
* `arb-tx/examples/*`, `arb-ingest/examples/*` — probes, not production.

## Test coverage

994 tests pass / 0 fail across the workspace (`cargo test --workspace --all-targets`), including
oracle differentials (50), property/fuzz suites (12+), PDA differentials, V1 header differentials,
two-pool and three-pool flash-arb integration tests.
