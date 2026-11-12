# Phase 12.7 + 12.8 — Final Execution Guard & Adversarial Readiness Tests

Date: 2026-11-07
Base: Phase 12.5 + 12.6

## Phase 12.7 — Final execution-boundary guard

### Change

The strategy loop now pins `expected_stream_generation` when it accepts a new slot. That generation is carried in `DispatchContext` and checked again immediately before the transaction is sent through either the direct SWQoS RPC path or Jito.

`arb_ingest::validate_execution_readiness` fails closed when:

- Yellowstone is disconnected;
- the stream generation changed after the opportunity was produced;
- no observed-slot watermark exists;
- the source watermark is behind the opportunity slot;
- the opportunity is older than `max_state_age_slots` at fire time.

The check runs after preflight simulation and after the optional slot-window wait, immediately before network submission. This closes a real time-of-check/time-of-use window: a transaction can be simulated successfully, then the stream can disconnect or reconnect while the dispatcher waits for its send window.

### Deliberate scope boundary

This guard is **not** a replacement for account-level quote dependency validation. `build_quote_dependency_set` remains a separate primitive. The current `ExecutionOpportunity`/`SwapHop` types do not carry a protocol-complete account dependency manifest, and the current graph does not expose one to the dispatcher. Consequently this phase does not claim that pool state, vaults, CLMM tick arrays, DLMM bin arrays, mints, fee/config PDAs and Token-2022 extension accounts are all revalidated at dispatch time.

Do not describe this as complete end-to-end quote-state validation until a per-pool dependency manifest is produced by each protocol decoder and passed through quote construction to dispatch.

## Phase 12.8 — Adversarial regression tests

Added unit tests for:

- disconnect between quote and dispatch;
- generation change after quote construction;
- source watermark older than quote slot;
- quote exceeding the configured maximum slot age;
- valid quote in the same generation within the freshness bound.

The existing Phase 12.1–12.6 tests continue to cover slot-dead descendant invalidation, snapshot completeness, owner checks, startup rejection, provider/generation consistency, bank mismatch and atomic batch publication.

## Important protocol audit finding — bank-aware fork handling remains incomplete

The project pins `yellowstone-grpc-proto = 14.0.1`. Its schema exposes `bank_id` on account and slot updates and includes the `EntryUpdateParent` update. The account path preserves `account_update.bank_id`, but the slot handler currently calls `record_slot_update(slot, parent, status)` without passing `slot_update.bank_id`, and the update loop does not handle `EntryUpdateParent`.

The current `SlotIntegrity` is slot/parent based, not `(slot, bank_id)` based. This is conservative in some paths (a dead slot invalidates all cached data for that slot and known descendants), but it cannot precisely retain one viable bank while discarding another bank at the same slot. It also does not process explicit parent-update invalidations.

This matters especially for processed commitment. Yellowstone's current protocol documentation describes bank identity and parent-update markers; Solana's Alpenglow guidance recommends buffering by `(slot, bank_id)` and discarding the bank that does not reach Confirmed. Processed updates can arrive without a complete per-bank ordering guarantee. Sources:

- Yellowstone Geyser proto: https://github.com/rpcpool/yellowstone-grpc/blob/master/yellowstone-grpc-proto/proto/geyser.proto
- Solana Alpenglow / Yellowstone bank reconstruction guidance: https://solana.com/vi/upgrades/alpenglow
- Solana accountSubscribe commitment options: https://solana.com/docs/rpc/websocket/accountsubscribe

**Operational conclusion:** this phase improves the final dispatch boundary but does not certify processed-commitment fork correctness. Phase 12.9 must implement bank-aware slot state, `EntryUpdateParent` invalidation, and bank-specific account-store eviction before treating processed data as fully coherent for live execution.

## Verification status

- Source edits: completed.
- ZIP integrity: checked after packaging.
- Patch dry-run against Phase 12.5 + 12.6: checked after packaging.
- `git diff --check`: checked after packaging.
- Rust compilation/tests/clippy: not run; the build environment has no `cargo`/Rust toolchain. Tests are added but not runtime-verified.

## Changed files

- `crates/arb-ingest/src/quote_readiness.rs`
- `crates/arb-ingest/src/lib.rs`
- `crates/arb-bot/src/supervisor/run.rs`
- `crates/arb-bot/src/supervisor/dispatch.rs`
