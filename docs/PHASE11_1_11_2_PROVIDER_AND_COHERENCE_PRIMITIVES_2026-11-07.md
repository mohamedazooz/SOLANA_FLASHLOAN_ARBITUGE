# Phase 11.1–11.2 — Provider supervision and dependency coherence

## Research basis

The Yellowstone Geyser protocol exposes account `slot`, `write_version`, optional `bank_id`, and slot statuses including `SLOT_DEAD`. `write_version` is an ordering value from the producing validator/provider and must not be treated as a global sequence across independent providers. See the upstream `geyser.proto`: https://github.com/rpcpool/yellowstone-grpc/blob/master/yellowstone-grpc-proto/proto/geyser.proto.

Solana JSON-RPC account notifications include a context slot; that is a slot watermark, not proof that independently received accounts share the same bank. See https://solana.com/docs/rpc/websocket/accountsubscribe.

## Implemented

- Added a deterministic provider-health state machine: Healthy / Degraded / OpenCircuit / Disabled.
- Added failure threshold, cooldown, half-open recovery, freshness and slot-lag checks.
- Added dependency provenance validation: one provider per dependency group, optional same-bank requirement, no future slots, bounded slot skew, and startup snapshot rejection.
- Added unit tests for failover, stale/lagging source rejection, provider mixing, bank/slot coherence.

## Explicit integration boundary

These are policy primitives, not a live multi-Yellowstone connection manager. The current `YellowstoneClient` still owns one endpoint/stream. Do not claim automatic stream failover until a runtime supervisor owns multiple clients, subscription replay, snapshot bootstrap, and promotion barriers.

Likewise, `bank_id` is provider/stream context and should only be compared within a compatible source context. If absent, the validator falls back to provider identity plus slot-skew policy; that is not cryptographic fork proof. `SLOT_DEAD` handling and descendant invalidation remain a separate StateIntegrity phase.

## Safety rules

- Never compare `write_version` between providers.
- On source switch, invalidate quote-ready state until required accounts are rehydrated from the new source.
- Pool header and vault dependencies should use `max_slot_skew = 0` unless protocol semantics prove a wider window safe.
- Slow-changing mint/config accounts may use a separate, explicitly configured age/skew policy.
- Startup snapshots are bootstrap material, not execution-grade deltas.

## Verification status

Rust toolchain is unavailable in the current execution environment. Tests are authored but `cargo fmt`, `cargo check`, and `cargo test` have not been run. These changes are not production-verified.
