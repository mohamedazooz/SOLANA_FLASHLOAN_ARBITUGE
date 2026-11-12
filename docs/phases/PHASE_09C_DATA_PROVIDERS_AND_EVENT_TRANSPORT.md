# Phase 09C — provider-neutral WebSockets, Solami Yellowstone gRPC and Kafka

## Agreed transport architecture

**HTTP JSON-RPC is not part of the Solana market-data architecture.** The runtime provider factory is disabled and returns no HTTP RPC endpoints. Market-state ingestion is based on Solami Yellowstone gRPC, with three configurable provider-neutral Solana PubSub WebSocket slots for auxiliary/secondary streams.

### Yellowstone gRPC

- `SOLAMI_GRPC_ENDPOINT`
- `SOLAMI_API_KEY` (x-token)

Solami Yellowstone is the authoritative raw account/slot stream. Pool state is not quote-ready until protocol-specific decoders validate owner, discriminator, PDA, slot coherence, and all required dependent accounts.

### Three generic WebSocket slots

Expose exactly three provider-neutral endpoint slots in `.env.example`:

- `SOLANA_WS_1_URL`
- `SOLANA_WS_2_URL`
- `SOLANA_WS_3_URL`

The operator may assign Solami, Helius, DRPC, Chainstack or another compatible provider to any slot. Provider-specific authentication and subscription adapters remain separate: not every provider uses the same WebSocket protocol, auth header, query parameter or event schema.

The runtime failover adapter is not yet implemented. The required behavior is strict order 1 -> 2 -> 3, reconnect/resubscribe after failure, and feed unavailable after all three fail. It must never silently substitute stale cache or HTTP RPC.

## Important protocol distinction

A WebSocket URL is not a generic replacement for Yellowstone gRPC. Standard Solana JSON-RPC WebSocket subscriptions (`accountSubscribe`, `programSubscribe`, `logsSubscribe`, etc.), Yellowstone Geyser gRPC, and vendor-specific decoded trade streams are different transports/contracts. They require explicit adapters and schema validation. Do not send Yellowstone gRPC through a WebSocket client or treat decoded trade notifications as authoritative pool account state.

## Kafka

Kafka is an event transport/buffer, not an RPC endpoint. Define versioned event envelopes with:

- network, provider, source sequence, slot, write version, commitment and fork identity;
- account/pubkey or pool ID, owner program, raw payload hash, decoder version and decode status;
- idempotency key and ingestion timestamp.

Partition by stable account/pool identity where ordering is required. Consumers must be idempotent and detect gaps/reordering. Replay must reproduce the same market-state transitions. Kafka messages are not automatically authoritative; only verified account updates can promote quote-critical state.

## Acceptance

- Provider failover tests with deterministic order and no stale-cache substitution.
- WebSocket reconnect/heartbeat/subscription restore tests for each protocol adapter.
- Duplicate, out-of-order, gap, reconnect and fork/replay tests.
- Kafka schema compatibility and idempotent consumer tests.
- Metrics for provider latency, reconnects, feed age, dropped updates and queue lag with bounded-cardinality labels.
