# Dependency and Call Graph — workspace audit (2026-11-13)

## Crate layering (verified via Cargo.toml + `cargo tree`)

```
arb-constants   (leaf: program ids, mints, MTU constants)
arb-types       (leaf: PoolState, DexProtocol, EconomicConfig re-exports,
                 dimensional math)
arb-math        (leaf: cpmm/clmm/dlmm curves, economic accounting, sizing)
arb-risk        (leaf: RiskGate, KillSwitch, CircuitBreaker)
arb-store       (Store trait; SqliteStore, MemoryStore, PostgresStore, Redis)
arb-metrics     (leaf: prometheus counters)
arb-tx          (depends: constants/types/math — transaction primitives,
                 protocol builders, flash-loan compilers, transport)
arb-dex         (on-chain layout decoders per protocol)
arb-ingest      (WSS/RPC ingestion, registry, RpcPool)
arb-engine      (graph, adapters, quote engines, sizing pipelines)
arb-land        (Jito lander, landing tracker)
arb-sim         (RPC simulation)
arb-bot         (supervisor, config, control, web, telegram — the binary)
```

No cycles. `arb-math` and `arb-types` are leaves; every financial calculation
funnels downward.

## Single-source guarantees (verified)

| Concern | Authoritative owner | Delegators |
|---|---|---|
| Kamino flash fee | `arb-math::economic::calculate_flash_fee_raw` | `arb-tx::kamino` (re-exports + property test enforces agreement) |
| Net profit | `arb-math::economic::build_economic_quote` / `calculate_net_profit_raw` | arb-engine graph, arb-bot supervisor |
| CPMM/CLMM/DLMM curves | `arb-math::{cpmm,clmm,dlmm}` | arb-engine adapters, arb-tx builders |
| Protocol names | `arb_types::DexProtocol::as_str` | route monitor, logs |
| Wire-format limits | `arb-constants` + `arb-tx::SOLANA_PACKET_V1_MTU` | dispatch size gate, Jito bundle validator |
| Control commands | `arb-bot::control_service::execute_command` | web API + Telegram (one grammar) |

## Duplicate-logic sweep (verified clean)

- One quote engine per protocol family (adapter registry in arb-engine);
  protocol builders in arb-tx consume the same curve math.
- One discovery pipeline (MarketGraph) + two protocol-specialized capture
  paths that convert into the SAME `ExecutionOpportunity` type.
- One dispatch entry point (`supervisor/dispatch.rs`); no alternative send path
  exists (V1-only profile refuses Jito fallback for oversized tx).
- Dead `arb-mcp` crate removed (R2); dead Jupiter assembler removed (PR #7).

## Residual coupling worth watching

- `arb-bot::supervisor::dispatch` is the largest hot-path module (838 lines);
  within the ~1,000-line guideline but the next split candidate if it grows.
- `webhook-service/` and audit scratch dirs were removed in the reorg commit;
  `Cargo.toml` exclude list cleaned accordingly.
