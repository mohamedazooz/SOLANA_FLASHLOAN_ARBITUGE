# Phase 09A.2 — Online research and implementation references

Research checked before this implementation on 2026-11-07.

## Yellowstone gRPC

- Official Yellowstone Rust example: https://github.com/rpcpool/yellowstone-grpc/tree/master/examples/rust
- Official Yellowstone repository: https://github.com/rpcpool/yellowstone-grpc
- `yellowstone-grpc-client` API: https://docs.rs/yellowstone-grpc-client/latest/yellowstone_grpc_client/struct.GeyserGrpcClient.html
- Builder API and `x_token`, keepalive, reconnect configuration: https://docs.rs/yellowstone-grpc-client/latest/yellowstone_grpc_client/struct.GeyserGrpcBuilder.html
- Proto account filter fields: https://docs.rs/yellowstone-grpc-proto/latest/yellowstone_grpc_proto/geyser/struct.SubscribeRequestFilterAccounts.html
- Proto slot filter fields: https://docs.rs/yellowstone-grpc-proto/latest/yellowstone_grpc_proto/geyser/struct.SubscribeRequestFilterSlots.html
- Version pairing: Yellowstone changelog records `yellowstone-grpc-client 16.0.1`; matching proto release is `yellowstone-grpc-proto 14.0.1`.

Implementation implications:
- Yellowstone is a bidirectional gRPC subscription, not Solana JSON-RPC over WebSocket.
- Subscription requests replace the current filter set; every dynamic update must resend the full desired set.
- Account updates carry account bytes, owner, slot and write-version. The consumer must preserve ordering and deduplicate/reconcile replay.
- Plain subscriptions do not automatically reconnect. The current code reconnects explicitly; replay-window and fork-aware recovery are not yet accepted.

## CLMM

- Raydium CLMM math: https://github.com/raydium-io/raydium-docs-v1/blob/main/algorithms/clmm-math.mdx
- Raydium on-chain swap implementation: https://github.com/raydium-io/raydium-clmm/blob/master/programs/amm/src/instructions/swap.rs
- Orca Whirlpool token math: https://github.com/orca-so/whirlpools/blob/main/programs/whirlpool/src/math/token_math.rs

Implementation implications:
- CLMM price is represented by Q64.64 sqrt price; tick traversal must cross initialized ticks and apply signed liquidity deltas.
- Raydium and Orca account layouts, TickArray derivation and execution accounts are not interchangeable.
- Required input rounds upward and output rounds downward according to the protocol's integer arithmetic.

## DLMM

- Meteora DLMM SDK reference: https://github.com/MeteoraAg/docs/blob/main/developer-guides/dlmm/typescript-sdk/reference.mdx
- Meteora DLMM SDK source: https://github.com/MeteoraAg/dlmm-sdk

Implementation implications:
- Quote results depend on ordered BinArrays, active bin, base and variable fee parameters, protocol fee share and volatility state.
- SDK exposes consumed input, output, fee, protocol fee, price impact, end price and required BinArray public keys. These are suitable differential-test fields.
- Missing bin coverage must reject the quote rather than be treated as an empty bin.
