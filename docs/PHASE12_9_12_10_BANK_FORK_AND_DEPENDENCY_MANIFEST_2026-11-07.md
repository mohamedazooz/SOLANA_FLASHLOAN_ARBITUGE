# Phase 12.9 + 12.10 — Bank-aware Fork Handling & Protocol Dependency Manifests

Date: 2026-11-07
Scope: `arb-ingest` source layer. This patch builds on Phase 12.7 + 12.8.

## Phase 12.9 — Bank-aware fork invalidation

### Why slot-only identity is insufficient

Yellowstone `yellowstone-grpc-proto 14.0.1` exposes `bank_id` on account and slot updates and exposes `SubscribeUpdateEntryUpdateParent`. The proto's `EntryUpdateParent` comment says it invalidates entries for `cleared_bank_id`, including entries that arrive after the update; the replacement bank is identified by a subsequent `SLOT_CREATED_BANK` update. The entry filter must set `include_update_parent=true` to receive this update. That filter also emits entries; there is no entry-parent-only selector in the current proto. Because this project has a low incremental data budget, the setting is opt-in and defaults off.

A slot may have multiple candidate banks. Solana's Alpenglow streaming guidance explicitly says to buffer by `(slot, bank_id)` rather than slot alone. A consumer that keys solely by slot can merge competing candidate state.

### Changes

- `SlotIntegrity` now tracks invalidated banks by `(slot, bank_id)` in addition to dead slots.
- `YellowstoneClient::record_slot_update_with_bank` consumes `bank_id` from slot updates.
- Yellowstone can request entry-parent reconciliation updates via `YELLOWSTONE_ENTRY_PARENT_UPDATES=true`. It is **disabled by default** because the protocol entry filter emits entry updates as well as parent updates; the bot does not otherwise consume entries, so enabling it can materially increase bandwidth.
- `UpdateOneof::EntryUpdateParent` calls `record_entry_update_parent(slot, cleared_bank_id)`.
- `AccountSnapshotStore` has bounded tombstones for invalidated slots and banks. Both single and batch writes reject tombstoned identities.
- Because current `PoolState` does not carry `bank_id`, a cleared-bank event conservatively removes all cached pools and dedupe entries at that slot rather than risk retaining a competing candidate.
- Tombstone read locks are held through publication. This closes the race where an invalidation could occur after the check but before account insertion. Lock order is tombstones then account map for writers and invalidators.
- Quote readiness requires a non-empty `bank_id` for live account snapshots and rejects identities whose `(slot, bank_id)` has been invalidated. This deliberately fails closed if a processed account update cannot be associated with a candidate bank.

### Important protocol nuance

The Yellowstone proto formally promises invalidation of **entries** for `EntryUpdateParent`; it does not state that every account update for that bank is automatically invalid. This implementation applies a conservative local policy: it also invalidates account snapshots tagged with the cleared `(slot, bank_id)` to prevent stale candidate state from entering quotes. This policy is stricter than the wire-level guarantee and must be revisited if the provider documents different account-update semantics.

`SLOT_DEAD` still invalidates the slot and known descendants using the bounded parent graph. `SLOT_CREATED_BANK` reopens only that slot; descendants of the dead branch remain invalid until fresh parent relationships and updates are observed.

## Phase 12.10 — Protocol dependency manifests

New module: `crates/arb-ingest/src/dependency_manifest.rs`.

A `PoolDependencyManifest` records the exact accounts needed for one pool, including:

- pool state
- vault A / vault B
- mint A / mint B
- token program
- fee/config accounts
- oracle / observation
- CLMM tick arrays
- DLMM bin arrays / bitmap extension
- other adapter-specific accounts

Each dependency carries an explicit expected owner and a `required_for_quote` flag. The module does not guess PDAs, owners, tick arrays, bin arrays, or fee accounts. Protocol adapters must derive/resolve these using verified program logic and active swap direction/range.

`PoolDependencyManifest::from_registered_pool` seeds the invariant pool/vault/mint accounts from a verified `RegisteredPool`, but requires explicit pool owner, token-account owner and mint owner. It intentionally does not infer SPL Token vs Token-2022.

`DependencyManifestRegistry::route_requirements` merges route dependencies, deduplicates shared accounts, checks owner conflicts, sorts keys deterministically, and rejects route snapshots above 100 accounts. It does **not** split one quote dependency set into multiple RPC calls because separate `getMultipleAccounts` calls do not guarantee one identical context slot.

`YellowstoneClient::sync_manifest_subscriptions` wires manifest addresses into the explicit account-address filter. `quote_dependencies_for_route` derives required keys/owners from the same manifest and passes them through the existing Quote Readiness Gate.

### Not yet claimed

- This does not automatically derive every Raydium CLMM tick array or Meteora DLMM bin array. The adapter must populate dynamic dependencies.
- This does not yet replace every legacy quote builder call site. A repository-wide call-site audit and compile/test pass are required before claiming no bypass path exists.
- It does not provide cross-provider canonical bank consensus. Provider switching remains a generation reset and full state rebuild.
- It does not make `processed` commitment irreversible. For execution-sensitive policy, commitment selection and finality latency must remain explicit.

## Tests added

- Bank-scoped invalidation distinguishes two bank IDs within one slot.
- Live quote snapshots without a bank identity are rejected.
- Cleared-bank account snapshots are removed and late writes rejected by store tombstones.
- Manifest route merging deduplicates shared dependencies and remains deterministic.
- Conflicting owners for one key fail closed.
- Routes above 100 required accounts fail closed rather than being split.

## Validation status

- Static source inspection and whitespace checks performed.
- ZIP integrity and patch dry-run performed after packaging.
- Rust toolchain (`cargo`, `rustc`, `rustfmt`) is unavailable in this execution environment. `cargo fmt`, `cargo check`, `cargo test`, and `clippy` were not run. Tests are added but not compiler-verified here.
- The checked-in `Cargo.lock` appears not to include the `yellowstone-grpc-client` / `yellowstone-grpc-proto` dependencies declared in `crates/arb-ingest/Cargo.toml`; do not use `cargo --locked` until the lockfile is regenerated and reviewed in a Rust-enabled environment.

## Upstream references

- Yellowstone gRPC proto 14.0.1: https://docs.rs/crate/yellowstone-grpc-proto/14.0.1/source/proto/geyser.proto
- Yellowstone upstream proto: https://github.com/rpcpool/yellowstone-grpc/blob/master/yellowstone-grpc-proto/proto/geyser.proto
- Solana Alpenglow streaming guidance: https://solana.com/vi/upgrades/alpenglow
- Solana Bank runtime source: https://github.com/solana-labs/solana/blob/master/runtime/src/bank.rs
