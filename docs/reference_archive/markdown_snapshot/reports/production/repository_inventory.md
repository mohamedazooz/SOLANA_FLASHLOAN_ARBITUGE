# Repository Inventory & Crate Architecture

## 1. Workspace Crate Topology (14 Crates)

The repository is organized as a Cargo workspace with strict downward dependency flow:

```
[arb-types] [arb-constants]   <- Leaf crates, zero workspace dependencies
       \        /
       [arb-math]             <- Canonical quote math, Klend economics, integer arithmetic
       /   |    \
 [arb-dex] |     \
     |     |      \
 [arb-ingest]     |
     |     |      |
 [arb-engine]     |           <- Route discovery, supply-vault bounded sizing optimizer
     |     |      |
 [arb-risk]       |           <- Centralized 20-point risk gate, circuit breaker, kill switch
     |            |
 [arb-tx] <-------+           <- Instruction builders, V0/V1 compiler, Kamino flash loans
     |
 [arb-sim] [arb-land] [arb-store] [arb-metrics] [arb-config]
     \        |          |            /              /
      \-------+----------+-----------+--------------/
                         |
                     [arb-bot] <- Binary crate, Supervisor runtime loop, Web API, Telegram
```

### Detailed Crate Responsibilities

| Crate | Responsibility | Key Modules / Exports |
|---|---|---|
| `arb-types` | Core primitive types, TokenAmount, Mint, PoolId, Slot, Side | `token.rs`, `solana.rs`, `pool.rs` |
| `arb-constants` | Deployed protocol program IDs, token mints, sysvars | `programs.rs`, `mints.rs`, `tokens.rs` |
| `arb-math` | Exact DEX quote math (DLMM, CPMM, Whirlpools), Klend economics | `dlmm.rs`, `economic.rs`, `fixed_point.rs` |
| `arb-dex` | On-chain pool account decoders, Raydium, Orca, Meteora | `meteora.rs`, `raydium.rs`, `orca.rs`, `decoder.rs` |
| `arb-ingest` | Yellowstone gRPC client, multi-RPC supervision, slot tracker | `grpc.rs`, `supervisor.rs`, `freshness.rs` |
| `arb-engine` | Graph search, 2-hop/3-hop discovery, borrow sizing | `graph.rs`, `sizing.rs`, `pipeline.rs` |
| `arb-risk` | Capital protection, loss limits, circuit breaker, kill switch | `policy.rs`, `circuit_breaker.rs`, `kill_switch.rs` |
| `arb-tx` | V0/V1 compiler, Kamino flash borrow/repay, Jito tips | `compiler.rs`, `v1.rs`, `flash/kamino.rs`, `atomic.rs` |
| `arb-sim` | Preflight simulation, LiteSVM integration, local fork testing | `preflight.rs`, `simulator.rs` |
| `arb-land` | Jito Block Engine bundle submitter, tip floor oracle, status tracker | `jito.rs`, `tip_oracle.rs`, `tracker.rs` |
| `arb-store` | Persistence, SQLite trade storage, opportunity replay | `sqlite.rs`, `schema.rs` |
| `arb-metrics` | Prometheus metrics registry, telemetry counters, latency histograms | `registry.rs`, `recorder.rs` |
| `arb-config` | Strongly-typed layered TOML loader, validation, schema | `loader.rs`, `validation.rs`, `model.rs` |
| `arb-bot` | Main binary, supervisor runtime loop, Telegram bot, Web API | `main.rs`, `supervisor/`, `web_api.rs`, `telegram.rs` |

## 2. Binary Targets in `arb-bot`
1. `arb-bot` (`src/main.rs`): Main production runner.
2. `alchemy_webhook` (`src/bin/alchemy_webhook.rs`): Alchemy webhook receiver.
3. `discover` (`src/bin/discover.rs`): Pool discovery & indexing utility.
4. `latency_bench` (`src/bin/latency_bench.rs`): Benchmarks internal tick latency.
5. `live_scanner` (`src/bin/live_scanner.rs`): Live read-only pool state scanner.
6. `replay` (`src/bin/replay.rs`): Replays recorded market state updates.
7. `validate` (`src/bin/validate.rs`): CLI verification of configs & invariants.

## 3. Configuration Layer (`config/`)
- `base.toml`: Default global parameters.
- `development.toml`: Devnet & mock providers.
- `staging.toml`: Staging testbed settings.
- `production.toml`: Mainnet operational targets.
- `risk.toml`: Hard loss limits, tip ceilings, circuit breakers.
- `execution.toml`: Cu limits, priority fees, retry limits.
- `protocols.toml`: Meteora DLMM, Raydium, Orca pool filters.
- `providers.toml`: Helius RPC/WSS/gRPC endpoints & failovers.
- `webhook.toml`: Admin & Telegram webhook configuration.
- `config.schema.json`: JSON Schema for validation.
