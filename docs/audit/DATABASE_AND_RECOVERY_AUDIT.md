# Database and Recovery Audit

**Date:** 2026-11-13. Verified on post-merge main + R-I branch.

## Store architecture

- **SQLite** (`rusqlite 0.42`, bundled SQLite 3.54.1): default durable backend.
  WAL mode, busy_timeout 5000 ms, `synchronous = NORMAL`; schema migration +
  idempotent column evolution via `PRAGMA table_info` checks
  (`crates/arb-store/src/lib.rs`).
- **PostgreSQL** (`tokio-postgres 0.7`): production backend for
  `postgres://` URLs. `001_postgres_init.sql` executed at open
  (`batch_execute`), `BIGINT` for all Solana 64-bit values, TIMESTAMPTZ,
  indexes on slot/attempt/status. Single connection driven by a dedicated
  runtime; `NoTls` (fail-closed on TLS-requiring hosts — documented).
- **In-memory**: tests only. **Never** selected in production paths (R-I).

## Fail-closed store selection (R-I, verified)

`open_store(store_url, database_url)` in `arb-bot/src/supervisor.rs`:
precedence config-postgres → DATABASE_URL-postgres → DATABASE_URL-sqlite →
config-sqlite/plain. A configured-but-unavailable store **aborts startup**
with an explicit error. No silent in-memory degradation.

Live smoke evidence (release binary):
- Positive: `sqlite:///tmp/arb_smoke/shadow_smoke.db` → file + `-wal`/`-shm`
  created, 70 KB written in 45 s, clean startup logs.
- Negative: `sqlite:///proc/nope/impossible.db` → exit 101,
  `durable store unavailable; refusing to start (fail-closed: no in-memory fallback)`.

## Dispatch-intent durability (R12)

- Intent persisted **before** any send; signature extracted from signed bytes;
  `last_valid_block_height` captured from `getLatestBlockHash` at dispatch.
- Recovery proves expiration with block-height evidence **only** — never slot
  arithmetic (`recovery.rs`: `expiry_requires_block_height_evidence_beyond_last_valid_height`).
- Ambiguous outcomes (timeout, unknown signature, provider disagreement) stay
  `Unresolved` — never resubmitted blindly
  (`missing_or_one_sided_evidence_stays_unresolved`,
  `processed_is_not_landed_and_timeout_is_ambiguous`).
- Pending intents survive restart and resolve idempotently without
  resubmission (`pending_intents_survive_restart_and_resolve_without_resubmission`).

## Data integrity

- All Solana amounts stored as BIGINT with explicit `i64::try_from` guards
  (reject > BIGINT range instead of truncating).
- `u64 → i64` conversions guarded at every store boundary; financial paths use
  checked arithmetic (see `audit/FINANCIAL_LOGIC_AUDIT.md`).

## Remaining gaps

- PostgreSQL integration is exercised at the unit level with real SQL against
  SQLite; a live PostgreSQL instance test in CI is not available in this
  sandbox (connection-string path is exercised via fail-closed tests).
- Automated backup/restore drill is an operator task
  (`production/LIVE_READINESS_CHECKLIST.md`).
