# Phase 12.5 + 12.6 — Stream Lifecycle & Quote Readiness

Date: 2026-11-07
Scope: `arb-ingest` source-state safety; no transaction submission changes.

## Executive result

Implemented two fail-closed primitives and connected the stream lifecycle to the existing Yellowstone client:

- **12.5 — Stream generation lifecycle:** each connection attempt starts a new monotonically increasing generation, clears stale account/pool state, and only marks the stream connected after the initial subscription is accepted. Disconnect clears state and makes the source unavailable.
- **12.6 — Quote readiness gate:** a dependency group is returned only when every required account exists, has the expected owner, is a live (not startup) update, belongs to the selected provider and current stream generation, is not on a known-dead slot, is not future/stale, and passes provider/bank/slot coherence checks.

The gate obtains the dependency snapshots through one `AccountSnapshotStore::get_many` read lock. It returns owned immutable copies; it does not expose a partially assembled mutable map. The store also refuses same-slot live updates with conflicting bank IDs, refuses live provider switches without a generation reset, and never lets an HTTP startup snapshot overwrite live stream state.

## 12.5 implementation details

`AccountSnapshot` now carries:

- `provider_id`: source identity (`ProviderId(1)` for the configured Yellowstone stream; `ProviderId(0)` for HTTP bootstrap records).
- `bank_id`: optional Yellowstone bank identity. HTTP `getMultipleAccounts` cannot supply it, so bootstrap records use `None`.
- `generation`: stream epoch in which the account update was received.

`YellowstoneClient` now exposes:

- `begin_stream_generation()` — increments generation and clears account snapshots, pool cache, slot watermark and dedupe state before connecting.
- `mark_stream_connected()` — called only after gRPC connection and initial subscription succeed.
- `mark_stream_disconnected()` — clears source-derived state and closes readiness.
- `stream_generation()`, `stream_connected()`, and `provider_id()` — read-only lifecycle metadata.

The real `start_stream` loop invokes these transitions on connection attempts, successful subscription, disconnect, cancellation and missing credentials. There is deliberately no HTTP market-data fallback.

### Fork-repair edge case

Yellowstone's `SLOT_DEAD` has no bank ID in the slot message. The current slot-level invalidator therefore conservatively invalidates known descendants. Yellowstone may subsequently report `SLOT_CREATED_BANK` for a repaired/re-created bank at the same slot. The tracker now re-opens that slot only; descendants of the dead bank remain invalid until fresh state is observed. This is conservative and avoids accidentally reviving old descendant state.

The proto also defines `SLOT_FIRST_SHRED_RECEIVED = 3`; this is now recognized as a normal slot lifecycle event rather than an unknown status.

## 12.6 readiness contract

`build_quote_dependency_set(...)` and `YellowstoneClient::quote_dependencies(...)` reject:

| Condition | Gate result |
|---|---|
| Stream disconnected | `StreamDisconnected` |
| No observed slot watermark | `NoSlotWatermark` |
| Caller requests a future slot | `FutureSlot` |
| Empty / duplicate dependency list | `EmptyDependencies` / `DuplicateDependency` |
| Missing expected owner or account | `MissingExpectedOwner` / `MissingAccount` |
| Owner mismatch | `OwnerMismatch` |
| Yellowstone startup snapshot | `StartupSnapshot` |
| Different provider or generation | `WrongProvider` / `WrongGeneration` |
| Known dead fork | `DeadSlot` |
| Future or too-old slot | `FutureSlot` / `StaleSlot` |
| Mixed bank, provider or excessive slot skew | `Incoherent(...)` |

For tightly coupled pool-state/vault dependencies, pass `max_slot_skew = 0`. A different bank ID is expected across different slots; the coherence validator rejects conflicting bank IDs only for accounts stamped at the same slot, while the explicit slot-skew limit governs cross-slot config dependencies. If a protocol's immutable or slowly changing config accounts legitimately have a wider skew, that must be an explicit per-dependency policy; do not globally relax the pool/vault gate.

The method uses the latest slot watermark observed by the Yellowstone stream as the freshness reference; the caller-provided target slot may not be ahead of that watermark. It must be called immediately before quote construction and rechecked immediately before transaction construction/submission. A readiness result is a point-in-time validation, not a guarantee that state cannot change in the next slot.

## Best-practice constraints retained

1. HTTP `getMultipleAccounts` remains bootstrap/reconciliation only, never the market-data hot path.
2. RPC bootstrap snapshots remain `is_startup=true`, `ProviderId(0)`, `bank_id=None`; they cannot pass the live quote gate.
3. Provider-local write versions are not compared across providers. The account store rejects a live provider switch until state is explicitly cleared in a new source generation; the quote gate independently rejects mixed-provider dependency groups.
4. For one account at one slot, conflicting live bank IDs are rejected rather than resolved by write-version ordering. `SLOT_DEAD` invalidation clears the affected slot before a replacement bank is admitted.
5. HTTP bootstrap batches must be homogeneous in slot, provider, bank, generation and startup status.
6. A stream reconnect invalidates prior source state; a reconnect does not inherit readiness from the previous generation.
7. No automatic cross-provider failover is claimed. `ProviderSupervisor` remains policy primitives; a future multi-provider runtime must keep each provider's state, bank IDs and write versions isolated.
8. No profitability, inclusion, or execution guarantee is implied by quote readiness.

## Research and protocol references

- Yellowstone Geyser proto: slot status enum, `SubscribeUpdateSlot` (`slot`, `parent`, `status`, optional `bank_id`) and `SubscribeUpdateAccount` (`slot`, `is_startup`, optional `bank_id`): https://github.com/rpcpool/yellowstone-grpc/blob/master/yellowstone-grpc-proto/proto/geyser.proto
- Yellowstone Rust proto type fields: https://docs.rs/yellowstone-grpc-proto/latest/yellowstone_grpc_proto/geyser/struct.SubscribeUpdateSlot.html
- Yellowstone changelog (2026-10-16): ordering of Processed/Confirmed/Finalized with CreatedBank/Dead matters; a dead bank can be repaired and slot status ordering can otherwise discard the replacement or mislabel a fork: https://github.com/rpcpool/yellowstone-grpc/blob/master/CHANGELOG.md
- Solana commitment semantics: `processed` is newest but can roll back; `confirmed` and `finalized` provide stronger commitment: https://solana.com/docs/rpc
- Solana `getMultipleAccounts` returns one context slot for one request and accepts up to 100 keys: https://solana.com/docs/rpc/http/getmultipleaccounts

## Tests added

- Stream generation clears prior state and disconnect closes readiness.
- Same-slot cross-bank updates, live cross-provider writes and bootstrap-over-live replacement are rejected.
- Live current-generation dependencies pass the readiness gate.
- Startup, disconnected, dead-slot, stale-slot, mixed-bank and mixed-generation data are rejected.
- Re-created bank re-opens its slot but does not revive old descendants.

## Validation status

- Source-level delimiter/field-construction audit performed.
- Rust toolchain is unavailable in this execution environment; `cargo fmt`, `cargo check`, `cargo test`, and `cargo clippy` were **not run**. The new tests are authored but not compiler-verified here.
- Runtime integration is complete only through Yellowstone lifecycle and the public quote-readiness API. The strategy/quote executor still needs to call `quote_dependencies` for its exact dependency set and fail closed on any error. Until that call site is wired and compiler/tests pass, do not enable live execution based solely on this patch.

## Files changed

- `crates/arb-ingest/src/lib.rs`
- `crates/arb-ingest/src/yellowstone.rs`
- `crates/arb-ingest/src/slot_integrity.rs`
- `crates/arb-ingest/src/rpc_reconcile.rs`
- `crates/arb-ingest/src/quote_readiness.rs` (new)
- `crates/arb-ingest/src/pump_state_tests.rs`
- `crates/arb-bot/src/verified_config.rs`
