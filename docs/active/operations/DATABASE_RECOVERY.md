# Database Recovery & Operational Safety Runbook

Status: ACTIVE · Last evidence date: 2026-11-23 · Sources: `crates/arb-store/*`, `crates/arb-bot/src/supervisor/recovery.rs` (REPOSITORY_VERIFIED)

## Recovery invariants (enforced in code)

1. **Dispatch intent before send** (`dispatch.rs`): the signed transaction's signature, provider,
   dispatch slot and blockhash expiry (`last_valid_block_height`) are durably recorded BEFORE
   `sendTransaction`/`sendBundle`. A crash inside an ambiguous timeout leaves a pending intent.
2. **Reconcile, never blind-retry** (`recovery.rs`): startup reconciliation iterates
   `pending_dispatch_intents` and, for each, checks `getSignatureStatuses` first; expiration is
   claimed ONLY when the current block height strictly exceeds the recorded
   `last_valid_block_height` (never via slot arithmetic). A potentially-landed transaction is NOT
   resubmitted (duplicate risk — the same money could be spent twice by two identical swaps).
3. **Unpersistable attempt ⇒ no dispatch** (`opportunity.rs`, PIPE-11): if `record_attempt` fails,
   the opportunity is refused. Attempt ids are `slot * 1_000_000 + monotonic seq`, plain INSERT;
   collisions fail loudly instead of overwriting evidence.
4. **Provenance discipline** (`outcomes`): realized PnL only ever sums rows with
   `provenance='actual'`; legacy/unknown rows are backfilled to `estimated` at open time and are
   never eligible for realized accounting.

## SQLite (default local store)

- WAL mode + `busy_timeout=5000` + `synchronous=NORMAL` enforced at open (`SqliteStore::open`).
- WAL leaves `*.db-wal` / `*.db-shm` next to the DB; recovery after a process crash:
  reopening the same path replays/checkpoints the WAL automatically. Do NOT delete `-wal`/`-shm`
  while a process may hold the file.
- Backup (service stopped): `sqlite3 arb.db ".backup /backups/arb-$(date +%F).db"`.
- Live backup without stopping: use VACUUM INTO from the SQLite CLI (WAL allows concurrent readers).

## PostgreSQL (production shared store)

- Connection from `DATABASE_URL` (+ optional explicit store URL override; misconfigured mixed
  configs fail closed, see `supervisor::tests::open_store_custom_sqlite_url_takes_precedence` and
  the fail-closed unit test).
- Schema is idempotent on open (`001_postgres_init.sql`, all columns
  `ADD COLUMN IF NOT EXISTS`) — restarting against an older schema upgrades it in place without
  data changes.
- Backups: `pg_dump -Fc $DATABASE_URL > /backups/arb-$(date +%F).dump`. Test restores quarterly:
  `pg_restore -d <staging_db> /backups/….dump` and validate `attempts`/`outcomes` counts against
  the ops dashboard.
- Retention: no automatic destructive pruning in PostgresStore (deliberate — financial evidence
  retention is an operator decision covering both legal and audit windows). Suggested ops job:
  `DELETE FROM fills WHERE attempt_id IN (SELECT id FROM attempts WHERE created_at < now() - interval '180 days')`
  then children/parents in the order `fills → outcomes → dispatch_intents → attempts`
  (same FK-safe order as the SQLite pruner).
- Concurrency: the runtime uses a pooled client; all writes are single-row INSERT/UPDATE keyed by
  `attempt_id` — no cross-row transactions are required for correctness of individual records. Do
  not run two bots against one DB with the same wallet: attempt-id uniqueness keys on slot+process
  seq and duplicate pairs would double-dispatch.

## Crash / restart procedure

1. Stop the service (`systemctl stop arb-bot` or SIGTERM — graceful shutdown drains in-flight tasks).
2. Start it again; reconciliation runs `pending_dispatch_intents` automatically.
3. Verify: `SELECT signature, provider, bundle_id, resolved_status FROM dispatch_intents WHERE status='pending'`
   — after reconciliation there should be no rows left as `pending` with a resolved value of
   `null` (each must be marked on-chain-landed / failed / expired).
4. Check `SELECT COUNT(*) FROM dispatch_intents WHERE status='pending'` → 0 expected.
5. PnL integrity check after any restart:
   `SELECT provenance, COUNT(*), SUM(realized_net) FROM outcomes WHERE status='landed' GROUP BY provenance;`

## Failure modes and correct operator behavior

| Symptom | Meaning | Action |
|---|---|---|
| `attempt persistence failed — refusing to dispatch` | DB write failure; attempt never sent | Fix DB connectivity; NOT a fund risk. |
| `dispatch aborted: durable intent persistence failed` | Pre-send intent INSERT failed; tx NOT sent | Fix DB; do not hand-resend. |
| pending intents present after restart | Reconciliation not yet run or RPC unreachable | Check RPC liveness; never manually reset `status`. |
| `realized_net` negative on a `landed`+`actual` row | On-chain loss (deducted slippage/fees exceeded edge) | Normal risk recording. Review route profitability; do NOT adjust realized figures. |
| WAL file growing unbounded | Long-running txn/checkpoint stall | Check long-lived readers; `PRAGMA wal_checkpoint(TRUNCATE);` |

## Related

- `docs/active/architecture/DATA_MODEL.md` — schema and units contract.