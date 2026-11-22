# Yellowstone / Geyser gRPC Research

**S3 verification date**: 2026-11-22.

## Verified facts

| Item | Finding | Source (S3 unless noted) |
|---|---|---|
| Project protocol | gRPC Geyser interface (`geyser.proto`), maintained in [rpcpool/yellowstone-grpc](https://github.com/rpcpool/yellowstone-grpc); streams slots, accounts, transactions, blocks, blocks meta, entries; commitments processed / confirmed / finalized | repo README (fetched 2026-11-22) |
| Rust client releases | `yellowstone-grpc-client 12.1.0` (2026-02-24) in changelog; lib.rs docs show dedicated `dedup` + `reconnect` modules | [CHANGELOG.md](https://github.com/rpcpool/yellowstone-grpc/blob/master/CHANGELOG.md), [docs.rs lib.rs](https://docs.rs/yellowstone-grpc-client/latest/src/yellowstone_grpc_client/lib.rs.html) (S3) |
| Auto-reconnect | Rust client ≥ 13.1 has built-in auto-reconnect: on recoverable error it re-subscribes with `from_slot` (few slots before checkpoint) and **deduplicates replayed updates** via per-slot seen-message tracking | [rpcedge production guide](https://rpcedge.com/blog/production-yellowstone-grpc-reconnect-replay) 2026-07-22, docs.rs (S3) |
| Replay retention | `from_slot` replay works only if the server retained those slots (`subscribeReplayInfo`); endpoints differ | [solanatracker tutorial](https://www.solanatracker.io/resources/yellowstone-grpc-setup), [shyft docs](https://docs.shyft.to/solana-yellowstone-grpc/docs/getting-started/replaying-slots-with-grpc) (S3) |
| Node SDK | `@triton-one/yellowstone-grpc` 9.0.0 (published ~2026-11) | npm (S3) |
| Pre-execution data | "deshred pre-execution transactions" available (leading-edge latency) | yellowstone-grpc README (fetched 2026-11-22) |
| Fork awareness | plugin emits slot status transitions + bank invalidation; blocks reconstructed from BlockMeta (validators' blocks may have zero entries — known upstream issue) | yellowstone-grpc README (fetched 2026-11-22) |

## Comparison vs current Helius WS ingestion

| Dimension | Helius WS (current, `arb-ingest`) | Yellowstone gRPC (feature-gated) |
|---|---|---|
| Wire | JSON-RPC over WebSocket | HTTP/2 protobuf, gzip/zstd |
| Commitment model | notification-level | explicit per-subscription `CommitmentLevel` + slot status stream |
| Reconnect | repo-implemented backoff/rotation/generation tracking (tested) | client ≥13.1 built-in + repo must still gate on slot continuity |
| Cost | bundled with RPC plans | separate integration (per-provider pricing/quotas) |

## Existing implementation

`crates/arb-ingest/Cargo.toml:41` — `yellowstone-grpc-client = "17.0.0"` behind the `yellowstone` feature; client integration pending (Phase 4 in regression roadmap; CURRENT_STATUS marks it Not Wired).

## Recommendation (REC)

The WS stage-exit gate for ingestion was verified with 162 passing tests; **do not replace the WS path without a measured A/B benchmark** (Phase 5 in the master roadmap). When wiring Yellowstone:

1. Normalize gRPC updates into the same decoded-pool event interface — no parallel state machine.
2. Preserve slot-integrity gates (`slot_integrity.rs`): process only updates with parent-consistent slot chains; treat provider generation changes as disconnects (already implemented for WS).
3. On reconnect, apply overlap dedup before side effects; fail closed when replay retention is not confirmed.
4. Measure both paths event→decode→quote before any default-provider switch.

## Tests required

- Update normalization parity: same pool account payload through WS decode and gRPC decode produce identical decoded state (property or fixture test).
- Reconnect gap test: dropped stream + replay from `lastSlot − k` ⇒ no duplicate application, no stale selection.
- Fail-closed test: gRPC provider that cannot confirm retention ⇒ route rejected, WS remains authoritative.
