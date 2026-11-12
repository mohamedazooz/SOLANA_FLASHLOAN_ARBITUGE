# Phase 12.3–12.4 — RPC bootstrap and atomic snapshot publication

Date: 2026-11-07
Scope: bounded HTTP reconciliation for account dependency groups; atomic visibility in the account snapshot store.

## What changed

1. Added `arb_ingest::rpc_reconcile::RpcSnapshotClient`.
   - Uses Solana JSON-RPC `getMultipleAccounts` with `encoding=base64`, configured commitment, and `minContextSlot`.
   - Enforces the protocol maximum of 100 pubkeys per call. A quote dependency group larger than 100 is rejected, not silently split.
   - Preserves positional correspondence between requested keys and returned values; `null` is a hard failure for a required account.
   - Validates response context slot, response length, base64 encoding, and each account's expected owner.
   - Does not publish partial results. All validation completes before publication.
2. Added `AccountSnapshotStore::apply_batch`.
   - Requires a non-empty batch, unique pubkeys, one common context slot, and every record newer than its current stored record.
   - Checks the complete batch under one write lock, then inserts it under that same lock. Readers cannot observe half of a dependency group.
3. Bootstrap records are marked `is_startup=true` and use `write_version=0`.
   - They are reconciliation seeds, not live Geyser writes.
   - Quote eligibility must remain blocked until live, non-startup stream updates refresh the required dependency group.
4. Added unit tests for invalid RPC config, response parsing/null preservation, and all-or-nothing batch publication.

## Correctness decisions

- Solana documents a maximum of 100 addresses per `getMultipleAccounts` request and returns one `context.slot` for the response. Therefore one executable pool dependency group must fit in one request if it needs a single coherent RPC context. Splitting a tightly coupled group into multiple RPC requests does **not** create a guaranteed common snapshot, even if every request uses `minContextSlot`.
- `minContextSlot` is a lower bound, not a request for an exact slot. The returned `context.slot` is recorded and used as the batch's context.
- RPC `getMultipleAccounts` does not return Yellowstone `write_version` or `bank_id`. The implementation intentionally does not fabricate either field. Provider/bank provenance remains available only for stream-derived records until a separate provenance model is integrated.
- The HTTP client is for bootstrap/reconciliation only. It must not be called per quote, per pool update, or on the market-data hot path.
- Owner validation must be supplied by the caller from trusted protocol metadata. The client does not infer expected owners from untrusted account data.

## Operational integration status

This phase adds a callable primitive, not automatic startup wiring. The runtime still needs a lifecycle owner to:

1. Build a complete dependency set from the pool registry (pool state, vaults, mints, fee/config PDAs, and CLMM tick/bin dependencies where applicable).
2. Reject groups above 100 or deliberately decompose them only when each independent sub-group can be validated separately; never claim cross-group atomicity.
3. Call `fetch_and_publish` after stream gaps/reconnects or on explicit bootstrap, with a generation that increases on each attempt.
4. Keep quoting disabled for a dependency group until live stream updates replace its startup snapshots and the provider/slot/bank coherence validator accepts the group.

No runtime automatic RPC polling was introduced. This is intentional to protect the existing data budget and avoid HTTP in the hot path.

## Tests and validation

- Added Rust unit tests; not executed in this environment because the Rust toolchain (`cargo`) is unavailable.
- No `cargo fmt`, `cargo check`, `cargo test`, or `clippy` result is claimed.
- Static checks performed after editing: diff whitespace check, brace/parenthesis balance, ZIP integrity, and patch dry-run against the Phase 12.1–12.2 source archive.

## References

- Solana official `getMultipleAccounts`: https://solana.com/docs/rpc/http/getmultipleaccounts — maximum 100 pubkeys, `minContextSlot`, response context slot and positional account values.
- Solana Arabic RPC documentation: https://solana.com/ar/docs/rpc/http/getmultipleaccounts — null account semantics and account fields.
- Solana RPC JSON structures: https://solana.com/docs/rpc/json-structures — base64 account data tuple and owner representation.

## Remaining blockers before production enablement

- Wire bootstrap into reconnect/startup orchestration with explicit generation and timeout/retry budget.
- Add runtime quote-readiness gating that requires every required account to be non-startup and coherent; merely storing bootstrap data must never enable execution.
- Integrate bank/provider provenance for stream updates and define how RPC bootstrap provenance is represented without pretending it is a Yellowstone bank.
- Add integration tests using a local/mock JSON-RPC server and run full Rust CI in a toolchain-enabled environment.
