# RPC getMultipleAccounts Cardinality Audit — 2026-11-27

## Scope
Audited the account-to-pubkey association boundary in crates/arb-ingest/src/rpc_pool.rs.

## Defect
The previous implementation trusted the returned result.value array length. batch_fetch_accounts later associates returned account entries with requested pubkeys using positional zip. A provider response with fewer entries could therefore shift account data onto the wrong key; an oversized response could be silently truncated by downstream zip.

## Remediation
- Added a single response decoder that validates value.len() == requested_pubkeys.len().
- The high-capacity batch path falls back to bounded five-key requests when a large response is malformed.
- Every fallback response is cardinality-checked; malformed fallback results propagate an error.
- Slot attribution remains attached to the response that carried each account.

## Verification
- cargo test -p arb-ingest --lib rpc_pool::tests --locked
- Full crate tests, Clippy, formatting, and diff checks are recorded in the PR after completion.

## Safety boundary
The patch does not relax owner, slot, layout, PDA, or protocol validation. It only prevents malformed provider arrays from being positionally mis-associated with requested public keys.
