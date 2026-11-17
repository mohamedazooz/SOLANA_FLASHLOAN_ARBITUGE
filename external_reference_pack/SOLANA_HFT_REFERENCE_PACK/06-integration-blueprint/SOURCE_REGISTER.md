# Source Register

وقت الفحص: 2026-11-16. الفروع المذكورة متغيرة؛ على الوكيل تثبيت commit SHA قبل أي مقارنة.

| Project | Canonical repository | Primary source files | License/status notes |
|---|---|---|---|
| Cetipoo On-chain Arbitrage | https://github.com/Cetipoo/solana-onchain-arbitrage-bot | `src/route.rs`, `src/v10.rs`, `src/transaction.rs`, `lib/executor-v10-abi/src/cost.rs` | README warns reference/demo, not production-ready; MIT |
| Lorenz | https://github.com/lorenz-protocol/lorenz | `crates/lorenz-amm/src/lib.rs`, `crates/lorenz-graph/src/lib.rs`, `crates/lorenz-dex/src/decoder.rs`, `crates/lorenz-stream/src/geyser.rs`, `crates/lorenz-agent/src/risk.rs`, `docs/INVARIANTS.md` | Experimental; live Geyser seam is explicitly NotImplemented; MIT |
| FnZero Sol Trade SDK | https://github.com/0xfnzero/sol-trade-sdk | `docs/ARCHITECTURE.md`, `docs/LOW_LATENCY_BOTS.md`, `docs/GAS_FEE_STRATEGY.md`, `examples/pool_quote_tools.rs` | MIT per repository search; verify current release |
| FnZero Parser SDK | https://github.com/0xfnzero/sol-parser-sdk | `docs/ACCOUNT_SUBSCRIPTIONS.md`, `docs/grpc-vs-rpc.md`, `examples/grpc_raw_cache.rs`, `src/grpc/client.rs`, `src/grpc/config.rs` | MIT per repository search |
| FnZero Streamer | https://github.com/0xfnzero/solana-streamer | `src/streaming/yellowstone_grpc.rs`, `src/streaming/grpc/connection.rs`, `src/streaming/grpc/subscription.rs`, `src/streaming/common/order_buffer.rs` | MIT per repository search |
| Meteora DLMM SDK | https://github.com/MeteoraAg/dlmm-sdk | `commons/src/quote.rs`, `commons/src/math/price_math.rs`, `commons/src/pda.rs`, `commons/src/token_2022.rs`, `idls/dlmm.json`, `commons/tests/integration/test_swap.rs` | Verify LICENSE and current deployed program/IDL before reuse |

## Important corrections to the earlier shortlist
- `ShadowArb ArbitrageBot` was not included because a canonical source repository could not be verified in the follow-up source inspection.
- `Cetipoo/solana-mev-bot` was inspected but exposed only a README-sized repository tree in this check; therefore the pack uses the verifiable `solana-onchain-arbitrage-bot` reference repository.
