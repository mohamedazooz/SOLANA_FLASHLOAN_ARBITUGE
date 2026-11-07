# Phase 12.1 / 12.2 — Fork invalidation and snapshot reconciliation

## Online protocol references checked (2026-11-07)

- Yellowstone `geyser.proto`: `SubscribeUpdateSlot` carries `slot`, optional `parent`, `status`, optional `dead_error`, and (in current schema) optional `bank_id`. Account updates carry `slot`, `is_startup`, optional `bank_id`; account info carries `write_version`.
  - https://github.com/rpcpool/yellowstone-grpc/blob/master/yellowstone-grpc-proto/proto/geyser.proto
  - https://docs.rs/yellowstone-grpc-proto/latest/yellowstone_grpc_proto/geyser/struct.SubscribeUpdateSlot.html
- Yellowstone slot statuses include PROCESSED=0, CONFIRMED=1, FINALIZED=2, COMPLETED=4, CREATED_BANK=5, DEAD=6.
  - https://github.com/rpcpool/yellowstone-grpc/blob/master/examples/golang/proto/geyser.pb.go
- Solana `getMultipleAccounts` returns one response context slot for up to 100 account addresses; `minContextSlot` can enforce a minimum evaluation slot.
  - https://solana.com/docs/rpc/http/getmultipleaccounts

## Implemented

### 12.1
- Added bounded `SlotIntegrity` parent graph (8,192 slots), finalized/high-water tracking, and dead-slot propagation to descendants whose parent links are known.
- Yellowstone slot updates now pass parent and status to the integrity tracker.
- On `SLOT_DEAD`, account snapshots, decoded pool cache entries, and dedupe entries at invalidated slots are removed.
- Pool updates and account updates for known-dead slots are rejected.
- Unknown slot statuses are reported as unknown; they are not interpreted as success.

### 12.2
- Added `SnapshotReconciler` generation token and required-account set.
- A generation is ready only after all required keys are recorded at one common RPC context slot not older than the requested target slot.
- Stale generations, old context slots, and duplicate account results are rejected.

## Important limitations / not claimed complete

- This is source-local fork invalidation for the currently wired Yellowstone stream. It is not a cross-provider canonical fork consensus mechanism.
- Descendant invalidation can only traverse parent edges observed and retained in the bounded graph. Unknown ancestry is not guessed.
- `SnapshotReconciler` is a tested primitive; the current runtime does not yet call `getMultipleAccounts` and feed its result into this reconciler. Network bootstrap, batch splitting (100 keys), account absence handling, owner checks, and atomic publication remain Phase 12.3 integration work.
- The current `AccountSnapshot` does not persist bank_id/provider identity. The Phase 11 provenance primitives remain separate; end-to-end bank-aware account storage still needs integration.
- `cargo fmt`, `cargo check`, `cargo test`, and `clippy` were not run in this environment because the Rust toolchain is unavailable.
