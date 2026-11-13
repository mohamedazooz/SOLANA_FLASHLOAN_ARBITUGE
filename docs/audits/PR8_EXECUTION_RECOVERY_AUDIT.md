# PR #8 Execution & Recovery Audit (2026-11-13)

Scope: the execution pipeline and durable-dispatch/recovery subsystem
delivered by PR #8 (`feat/remediation-r2-r12`, head `e2c965f`), plus the
blockhash-evidence remediation implemented in this integration round.
Terms: **Verified** = inspected AND exercised by a test that ran;
**Partially verified** = inspected, partially exercised; **Not verified** =
inspected only or blocked.

## 1. Pipeline integrity — VERIFIED

One canonical pipeline (Market Data → discovery → strategy → quote →
RiskGate → compile → simulate → sign → persist intent → dispatch → track →
reconcile → account). Stage-by-stage map with invariants:
[../audit/EXECUTION_FLOW_MAP.md](../audit/EXECUTION_FLOW_MAP.md).
Single-source guarantees (fee, net profit, protocol names, wire limits,
control commands) verified in
[../audit/DEPENDENCY_AND_CALL_GRAPH.md](../audit/DEPENDENCY_AND_CALL_GRAPH.md).
No duplicated transaction builders (grep sweep re-run this round: the 3-leg
and 4-leg flash builders remain the only two, used by the supervisor; no
parallel builder exists).

## 2. Recovery — remediated this round (was the top integration finding)

**Finding I1 (HIGH, fixed)**: `classify_reconciliation` proved blockhash
expiration with fixed slot arithmetic
(`current_slot >= dispatch_slot + 160`). A slot threshold is not proof that
a specific blockhash expired (MAX_LOCKOUT_HISTORY is ~150 slots but the
actual validity is per-blockhash, returned by `getLatestBlockhash` as
`lastValidBlockHeight`).

**Fix (evidence-based expiration)**:
- `supervisor/execution.rs` — `fetch_recent_blockhash` now captures
  `lastValidBlockHeight` from the `getLatestBlockhash` response and caches
  it alongside the hash.
- `supervisor/opportunity.rs` → `DispatchContext` — the evidence travels
  with the opportunity into dispatch.
- `supervisor/dispatch.rs` — the dispatch intent **persists**
  `last_valid_block_height` before the send.
- `arb-store` — `DispatchIntentRecord` + SQLite/Postgres schemas gain the
  column; **safe schema evolution** for pre-existing databases (SQLite:
  `PRAGMA table_info` check + `ALTER TABLE`; Postgres:
  `ADD COLUMN IF NOT EXISTS`).
- `supervisor/recovery.rs` — `classify_reconciliation(poll,
  block_height_evidence)`: `expired_unconfirmed` **only** when the live
  `getBlockHeight` (new `RpcPool::get_block_height`, rpc_pool.rs) is
  strictly greater than the persisted `lastValidBlockHeight`. Missing or
  one-sided evidence stays `unresolved_after_poll` — slot arithmetic
  removed entirely.

**Deterministic tests (all ran, all pass)** — recovery.rs + arb-store:
- `landed_and_failed_statuses_resolve_to_onchain_outcome`
- `processed_is_not_landed_and_timeout_is_ambiguous`
- `expiry_requires_block_height_evidence_beyond_last_valid_height`
  (strictly-greater proof; equal/one-before stay unresolved)
- `missing_or_one_sided_evidence_stays_unresolved`
- `pending_intents_survive_restart_and_resolve_without_resubmission`
- `sqlite_store_persists_and_roundtrips_last_valid_block_height`
- `schema_evolution_adds_last_valid_block_height_to_legacy_db`
  (creates a **legacy-schema** DB, opens it with the new store, verifies
  the column was added and works — the "existing database upgrade" test)

Directive Part 4 scenario mapping: successful landing → landed test;
on-chain failure → failed test; RPC timeout → ambiguous test; unknown
signature / delayed visibility / provider outage → poll returns None →
`unresolved_after_poll` (ambiguous test); expired blockhash →
block-height-evidence test; restart during pending dispatch →
restart-survival test; duplicate recovery attempt → resolve is
single-transition (`WHERE status='pending'`, second call is a no-op —
verified in the restart test); conflicting provider responses →
`getSignatureStatuses` is the single arbiter; a conflicting outcome cannot
override a resolved intent. **Not verifiable in sandbox**: live provider
failover during reconciliation (needs multi-endpoint chaos harness) —
documented, not claimed.

## 3. Durable dispatch & database — VERIFIED

- **Schema exists and is applied**: `dispatch_intents` created by
  `001_init.sql:38` (SQLite, executed in `SqliteStore::open`) and
  `001_postgres_init.sql:50` (Postgres, executed via
  `sqlx::raw_sql(POSTGRES_INIT_SQL)` in `PostgresStore::open`) — not just
  SQL files on disk; the startup path runs them (verified by the fresh-DB
  and legacy-upgrade tests above).
- **Fail-closed persistence** (PR #8 F1): signature extraction failure or
  store write failure aborts dispatch
  (`dispatch_aborted_unreconcilable` / `dispatch_aborted_unpersisted`).
  Persistence-unavailable ⇒ no dispatch — there is **no in-memory
  fallback** for dispatch intents (grep sweep: `record_dispatch_intent`
  has exactly one call site, on the durable store).
- **Idempotency**: SQLite `INSERT OR REPLACE`, Postgres
  `ON CONFLICT (attempt_id) DO UPDATE` — a re-recorded intent for the same
  attempt cannot create a second row.
- **Postgres connection pool**: max_connections(5) — bounded.
- **Partially verified / blocked**: live Postgres round-trip requires a
  running PostgreSQL server (not available in sandbox); the PG code path
  is compile-verified and mirrors the tested SQLite path.

## 4. Financial correctness — VERIFIED (carried + re-confirmed)

- Kamino fee: canonical RAW-bits formula in `arb-math`
  (`calculate_flash_fee_raw`), round-half-up, disabled sentinel —
  [../audit/FINANCIAL_LOGIC_AUDIT.md](../audit/FINANCIAL_LOGIC_AUDIT.md).
- Net profit: integer raw units; repayment gate
  `expected_out >= size_in + flash_fee`; borrow candidates above verified
  supply-vault liquidity dropped (fail-closed, PR #8 F3).
- Property tests: fee rounding + CPMM dust invariant
  (`arb-math/tests/differential.rs`, 50 props, regression seed committed).

## 5. Transaction / V1 — VERIFIED (carried)

Wire-size gate on signed bytes vs 4096, no downgrade, transport flag
required, `MissingLoadedAccountsDataLimit` enforced,
`maxSupportedTransactionVersion` plumbed — full evidence:
[../audit/TRANSACTION_FORMAT_AUDIT.md](../audit/TRANSACTION_FORMAT_AUDIT.md).

## 6. Performance / Rust quality — PARTIALLY VERIFIED

- clippy `-D warnings` clean; supervisor decomposed into
  strategy/opportunity/recovery/dispatch modules (no circular deps —
  module tree is a strict hierarchy under `supervisor::`); all files in
  the execution path < 1,000 lines.
- Benchmarks (run 2026-11-13): decode 2–4 ns, route optimization 289 ns,
  signing 14.7 µs, RPC RTT ~18 ms — hot path is network-bound.
- **Not verified**: long-run memory growth / task-leak behavior under
  multi-day soak (blocked; soak pending).
