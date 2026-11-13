# Code Tree — Annotated

Every workspace member, module, and its single responsibility. Line counts are
indicative. Rule: one responsibility per file; split on responsibility, not on
line count.

```
SOLANA_FLASHLOAN_ARBITUGE/
├── AGENTS.md                      agent/maintainer guide: conventions, invariants, traps
├── README.md                      project overview + doc index pointer
├── Cargo.toml                     workspace (members = crates/*), shared lints
├── rust-toolchain.toml            pinned 1.99.1, edition 2024
├── rustfmt.toml / deny.toml / audit.toml
├── .env.example                   every env var, placeholder values (real .env is gitignored)
├── config.toml                    deployment config (gitignored)
├── transaction-profile.toml       CU/fee profile used by jito_service + config
├── Dockerfile / docker-compose.yml / docker-compose.shadow.yml
├── run-shadow.sh / stop-shadow.sh / verify-shadow-package.sh
│
├── crates/
│   ├── arb-types/                 LEAF: domain types, no workspace deps
│   │   └── src/
│   │       ├── lib.rs             TokenAmount, MintId, PoolId, RouteId, SwapHop, Slot…
│   │       ├── currency.rs        mint-aware money types
│   │       └── dimensional.rs     dimensional accounting (unit-safety)
│   ├── arb-constants/             LEAF: single source of truth for program IDs,
│   │                              seeds, discriminators, pool addresses, CU tables
│   ├── arb-math/                  LEAF: all arithmetic (no I/O)
│   │   └── src/
│   │       ├── dlmm.rs            DLMM bin math (prices, amounts, fees)
│   │       ├── clmm.rs / cpmm.rs / damm_v2.rs / launchlab.rs
│   │       ├── economic.rs        CANONICAL economic calculator + canonical Kamino
│   │       │                      klend flash fee (KaminoFeeError, checked/unchecked)
│   │       ├── economic_integration.rs  UnifiedEconomicCalculator / CostLedger wiring
│   │       ├── chronos.rs         time/slot math
│   │       └── latency.rs         latency measurement helpers
│   ├── arb-dex/                   protocol adapters: decode + quote per DEX
│   │   └── src/                   traits.rs (adapter interface), meteora_dlmm,
│   │                              meteora_damm_v2, raydium_v4/cpmm/clmm/launchlab,
│   │                              orca_whirlpool, pump_fun, pump_swap, dlmm_validation
│   ├── arb-ingest/                market data + state freshness
│   │   └── src/
│   │       ├── solami_stream.rs   primary WS provider
│   │       ├── helius_ws.rs       Helius WS provider
│   │       ├── bitquery.rs / dexscreener.rs / gmgn.rs / pumpdev_stream.rs
│   │       ├── provider_supervisor.rs  rotation, health, failover policy
│   │       ├── rpc_pool.rs / rpc_reconcile.rs  HTTP RPC + account reconciliation
│   │       ├── registry.rs / discovery.rs / pumpfun_discovery.rs
│   │       ├── dual_ingest.rs     gRPC + WS dual feed
│   │       ├── dependency_manifest.rs  per-account slot/version tracking
│   │       ├── quote_readiness.rs freshness/readiness gate
│   │       └── slot_integrity.rs  slot ordering, fork detection
│   ├── arb-engine/                discovery + sizing
│   │   └── src/
│   │       ├── route_graph.rs     unified pool-to-token graph, cycle enumeration
│   │       ├── route_worker.rs    graph worker
│   │       ├── bellman_ford.rs    negative-cycle detection (arbitrage signal)
│   │       ├── adapters.rs        pool-state → adapter quoting
│   │       ├── meteora_quote_engine.rs / meteora_size_optimizer.rs
│   │       ├── flash_loan_sizing.rs    liquidity-bounded borrow sizing
│   │       ├── dlmm_flash_arbitrage.rs / cross_pool_dlmm.rs
│   │       ├── three_leg_meme_pipeline.rs
│   │       ├── meteora_pair_discovery.rs / meteora_fresh_state.rs
│   │       └── guarded.rs         guarded-quote interface
│   ├── arb-risk/                  risk gate + kill switch (single file, 918 L)
│   ├── arb-tx/                    instruction builders + compiler (see groups below)
│   │   └── src/
│   │       ├── lib.rs             public API: curated re-exports + unit tests
│   │       ├── primitives/        core tx primitives
│   │       │   ├── instruction.rs AccountMeta, CompiledInstruction, compute-budget
│   │       │   │                  instructions, Jito tip builders, wire helpers
│   │       │   ├── cu.rs          CU estimation, dynamic priority fee, profit cap
│   │       │   ├── ata.rs         ATA derivation, idempotent create, close, sync-native
│   │       │   ├── signer.rs      KeypairSigner, AddressLookupTable handle,
│   │       │   │                  KaminoBundleParams, VersionedTransaction
│   │       │   ├── builder.rs     TxBuilder: assembly → signing → serialization
│   │       │   ├── error.rs       TxError
│   │       │   └── atomic_integrity.rs  borrow/swap/repay structure verification
│   │       ├── protocols/         per-DEX instruction builders
│   │       │   ├── meteora_accounts.rs  DLMM account matrix, PDA derivation, swap
│   │       │   ├── meteora_dlmm.rs      swap2 exact-in/out, directional bin arrays
│   │       │   ├── raydium_v4.rs / raydium_cpmm.rs / raydium_clmm.rs
│   │       │   ├── orca.rs        Whirlpool builder + tick-array PDA
│   │       │   ├── pump_fun.rs / pump_swap.rs
│   │       │   ├── swap_hop.rs    unified per-hop dispatch across protocols
│   │       │   ├── pump_fun_raydium.rs  legacy combo builder
│   │       │   └── jupiter.rs     Jupiter client (off-chain route fallback)
│   │       ├── flash/             flash-loan strategy assemblers
│   │       │   ├── kamino.rs      klend borrow/repay builders, reserve decoding,
│   │       │   │                  fee computation (delegates math to arb-math)
│   │       │   ├── kamino_flash.rs  KaminoFlashParams + thin borrow/repay wrappers
│   │       │   ├── dlmm_bin_discovery.rs  bin-array planning + coverage verification
│   │       │   ├── dlmm_flash_arb.rs        2-pool flash arb assembler
│   │       │   ├── dlmm_3leg_flash_arb.rs   3-pool flash arb assembler
│   │       │   ├── dlmm_4leg_flash_arb.rs   4-pool flash arb assembler
│   │       │   ├── cross_dex_flash_arb.rs   cross-DEX assembler
│   │       │   ├── meteora_atomic_builder.rs 2-pool Meteora atomic V1 builder
│   │       │   └── compiler.rs    3-leg meme arb compiler (V0/V1, byte budgets)
│   │       └── transport/         format/transport infrastructure
│   │           ├── adaptive.rs    V0/V1 capability detector + adaptive compiler
│   │           ├── alt.rs         ALT management (V0 only)
│   │           ├── master_alt.rs  static master ALT
│   │           └── template.rs    pre-warmed tx templates
│   ├── arb-sim/                   simulation (optional litesvm feature, off by default)
│   ├── arb-land/                  landing: bidder, leader, ledger, tip_oracle
│   ├── arb-store/                 persistence
│   │   ├── src/lib.rs             Store trait (attempts, dispatch_intents, …),
│   │   │                          SqliteStore (WAL), MemoryStore
│   │   ├── src/postgres_store.rs  Postgres store
│   │   └── migrations/            001_init.sql (SQLite), 001_postgres_init.sql
│   ├── arb-metrics/               telemetry counters
│   └── arb-bot/                   THE BINARY
│       └── src/
│           ├── main.rs            wiring: config → ingest → engine → supervisor
│           ├── config.rs          env + config.toml parsing, validation, floors
│           ├── verified_config.rs validated config snapshot
│           ├── supervisor.rs      Supervisor struct + shared state
│           ├── supervisor/run.rs         event loop: stream generation, slots,
│           │                             per-tick strategy_cycle call
│           ├── supervisor/strategy.rs    per-tick cycle: verified inputs, snapshot,
│           │                             graph sizing, candidate capture, ranking
│           ├── supervisor/opportunity.rs gate ladder → assembly → V1 compile →
│           │                             dispatch (the ONLY execution path)
│           ├── supervisor/dispatch.rs    mode gate, signing, SWQoS/Jito send,
│           │                             landing tracker, dispatch-intent recording
│           ├── supervisor/recovery.rs    startup reconciliation of pending
│           │                             dispatch intents (never blind-resubmits)
│           ├── supervisor/execution.rs   execution plumbing
│           ├── control_service.rs        ONE control service (Telegram + web)
│           ├── web_api.rs         Axum REST + WS API, auth, sessions, rate limit,
│           │                      security headers, 64 KB body limit
│           ├── telegram.rs / telegram_control.rs
│           ├── jito_service.rs    tip accounts, bundle params
│           ├── health_monitor.rs / heartbeat.rs / errors.rs
│           └── bin/               alchemy_webhook, discover, latency_bench,
│                                  live_scanner, replay, validate
│
├── web/                           React + TypeScript + Vite control app
│   └── src/
│       ├── App.tsx                sections A–I, WS with backoff+jitter reconnect
│       ├── api.ts                 REST client
│       ├── components/            Overview, Opportunities, Pools, Executions,
│       │                          StrategyConfig, Providers, RiskControl, Logs,
│       │                          TelegramPanel, Login
│       └── styles.css
│
├── docs/                          see docs/README.md (architecture/ operations/
│                                  economics/ reference/ quality/ adr/ archive/)
├── reports/                       machine-generated test evidence (JSON manifests)
├── fixtures/                      mainnet account snapshots for differential tests
├── scripts/                       verify-linux.sh and helpers
└── .github/workflows/rust-ci.yml  fmt + check + clippy -D warnings + tests + web build
```

## arb-tx public API stability

All historical `arb_tx::<module>::Item` paths still resolve: each group
(`flash`, `primitives`, `protocols`, `transport`) re-exports its modules, and
`lib.rs` glob-re-exports the groups. `flash/mod.rs` uses a CURATED list for
`kamino` (not a glob) because `kamino.rs` and `kamino_flash.rs` both define
`build_kamino_flash_borrow/repay` — see AGENTS.md "Known traps".
