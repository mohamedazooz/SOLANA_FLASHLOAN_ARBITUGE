# Data Model — Persistent Storage

Status: ACTIVE · Last evidence date: 2026-11-23 · Sources: `crates/arb-store/src/lib.rs`, `crates/arb-store/src/postgres_store.rs`, `crates/arb-store/migrations/*` (REPOSITORY_VERIFIED)

## Backends

| Store | File | Use |
|---|---|---|
| `SqliteStore` | `arb-store/src/lib.rs` | Default durable local store (WAL pragmas, `busy_timeout=5000`, `synchronous=NORMAL`). |
| `PostgresStore` | `arb-store/src/postgres_store.rs` | Production/shared store (BIGINT for all Solana 64-bit values). |
| `MemoryStore` | `arb-store/src/lib.rs` (tests only) | Test double; not for production. |
| `RedisStore` / `ShortTtlCache` | `arb-store/src/redis_cache.rs` | Short-TTL caching + two-tier webhook dedup; never authoritative for money. |

Selection: `open_store()` (arb-bot) — `oracle://`/`postgresql:///postgres://` URL selects PostgreSQL; anything else
falls back to SQLite file-backed (`sqlite://` or bare path). Mixed-config fail-closed test exists
(`supervisor::tests::open_store_mixed_config_fails_closed`).

## Tables (financial reconstruction contract)

Every URL-documented column exists in BOTH SQLite and PostgreSQL migrations and both Rust stores.

### `attempts` — one row per evaluated-with-intent-to-execute opportunity
| Column | Meaning | Unit | Producer |
|---|---|---|---|
| `id` | attempt id = `slot * 1_000_000 + process-monotonic seq` (PIPE-11; plain INSERT, collision fails closed) | — | `opportunity.rs:~1379` |
| `slot` | evaluation slot | — | runtime |
| `route_id` | `route-{RouteId}` | — | runtime |
| `expected_net` | estimated net after all costs (provenance-tracked downstream) | lamports | `base_raw_to_lamports_checked` fail-closed conversion |
| `tip_lamports` | Jito tip | lamports | runtime |
| `decision` | bot mode at record time | — | runtime |
| `borrow_amount` | flash-loan principal | base-mint raw atomic units | runtime (`opp.size_in`) |
| `flash_fee_raw` | flash-loan fee (provider sf applied) | base-mint raw | runtime; `u64::MAX` (uncomputable) recorded as NULL, never as a poison value |
| `priority_fee_lamports` | priority fee | lamports | `arb_tx::priority_fee_lamports` |
| `expected_output_raw` | quote-time expected route output | base-mint raw | runtime |
| `pool_ids` | ordered pool pubkeys, comma-separated | — | runtime |

Unit discipline (mission §5 ECON): `*_raw` columns are BASE-MINT raw atomic units, never lamports;
`*_lamports` columns are lamports. NULL means unknown — economics aggregation must not treat
NULL as zero cost.

### `outcomes` — one row per terminal status of an attempt
| Column | Meaning |
|---|---|
| `status` | `sent`, `landed`, `failed`, `expired`, `canary`, `paper_trade`, `dispatch_aborted_*` … |
| `realized_net` | signed; only meaningful per `provenance` |
| `provenance` | `actual` (decoded on-chain balance deltas) or `estimated` (pre-dispatch/simulation). Legacy NULL rows are backfilled to `estimated` — never promoted to `actual` retroactively (MATH-01/PIPE-01). |
| `actual_output_raw` | on-chain-decoded route output, base-mint raw (NULL = no balance evidence) |
| `failure_reason` | machine-readable reason for errored outcomes (NULL for successes) |

Realized PnL aggregation (`pnl_daily`) sums `realized_net` ONLY where `status='landed' AND
provenance='actual'`; estimated landed rows are kept in the separate `simulated_net` bucket.

### `fills` — per-token balance deltas (evidence chain for realized PnL)
`attempt_id`, `token` (mint base58), `amount` (raw atomic units of that mint — deliberately not
lamports).

### `dispatch_intents` — pre-send crash-recovery record (R12)
`attempt_id` (PK), `signature` (base58, extracted from signed bytes), `provider` (`swqos`|`jito`),
`dispatch_slot`, `last_valid_block_height` (from getLatestBlockHash; recovery proves expiration with
block height — never slot arithmetic), `status` (`pending`→`resolved`), `resolved_status`,
`resolved_at`, `bundle_id` (provider acceptance id — set after transport confirm via
`set_dispatch_bundle_id`; SWQoS acceptance ids also recorded).

Design invariant: the intent is written BEFORE the send so a crash during an ambiguous timeout
leaves a signature-keyed trace; reconciliation checks `getSignatureStatuses` first and never blindly
resubmits (fixed regression test: `supervisor::recovery`).

### Other
`token_prices`, `circuit_breaker_events` (PostgreSQL), plus protocol-intelligence tables
(`002_protocol_intelligence_sqlite.sql`).

## Schema evolution contract

- Fresh installs: `INIT_SQL` → `PROTOCOL_INTELLIGENCE_INIT_SQL` → `evolve_schema()`.
- Legacy DBs: `evolve_schema()` idempotently adds missing columns via `PRAGMA table_info` probing:
  `dispatch_intents.last_valid_block_height`, `dispatch_intents.bundle_id`,
  `outcomes.provenance` (+backfill), `outcomes.actual_output_raw`, `outcomes.failure_reason`,
  `attempts.borrow_amount`, `attempts.flash_fee_raw`, `attempts.priority_fee_lamports`,
  `attempts.expected_output_raw`, `attempts.pool_ids`.
- Ordering invariant: indexes referencing evolved columns (e.g. `idx_outcomes_pnl` on `provenance`)
  are created in `evolve_schema` AFTER the column-add step. Creating them in INIT_SQL failed the
  whole migration chain for legacy DBs that predate the column — fixed and regression-tested
  (`schema_evolution_adds_economic_columns_to_legacy_db`).
- PostgreSQL: same columns appended with `ALTER TABLE ... ADD COLUMN IF NOT EXISTS` in
  `001_postgres_init.sql` (idempotent on every open).

## Retention

`prune_older_than_days` (SQLite): deletes newest-first-safe `fills → outcomes → dispatch_intents →
attempts` rows older than `days` (regression-tested). PostgresStore relies on ops-side retention
(run VACUUM/retention per DATABASE_RECOVERY.md; no automatic destructive delete — operator task).

## Test evidence (2026-11-23)

`cargo test -p arb-store --release --all-targets` — 15/15 lib tests including:
attempt economic-fields roundtrip, outcome detail persistence + provenance-guarded PnL,
bundle-id roundtrip through pending intents, legacy-schema evolution for all new columns.
