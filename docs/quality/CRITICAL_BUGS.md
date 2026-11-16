# Critical Bugs Register

Every entry: ID, severity, location, root cause, fix, regression test, status.

## Fixed

### R-I — Silent production fallback to in-memory SQLite — **CRITICAL — FIXED**
- **Location:** `crates/arb-bot/src/supervisor.rs` (`Supervisor::new`, store selection).
- **Reproduction:** configure `store.url = "sqlite:///app/data/shadow.db"` on a
  host without `/app/data`; observe startup succeed with **no** database file
  and non-durable dispatch intents. Also: unreachable PostgreSQL → silent
  in-memory store.
- **Root cause (two-layer):** (1) explicit fallback to
  `SqliteStore::open_in_memory()` on any store error; (2) `sqlite://` URLs
  passed raw to `Connection::open` (no prefix stripping) so the configured
  file never opened — including the docker-compose shadow deployment.
- **Fix:** fail-closed `open_store(store_url, database_url)` (startup aborts
  with explicit error) + `sqlite_path_from_url` resolution for
  `sqlite:///abs`, `sqlite:///./rel`, `sqlite://rel`, plain paths.
- **Regression tests:** `open_store_fails_closed_on_unreachable_postgres`,
  `open_store_fails_closed_when_database_url_postgres_unreachable`,
  `open_store_sqlite_url_is_durable_file_backed` (write→close→reopen→read),
  `sqlite_path_from_url_resolves_absolute_relative_and_plain_paths`.
- **Live evidence:** positive smoke (file + WAL created, 45 s run) and
  negative smoke (unopenable path → exit 101, explicit fail-closed message).
- **Remaining risk:** none identified; Postgres `NoTls` limitation documented
  in `postgres_store.rs` (fail-closed on TLS-requiring hosts).

### R-C — libsqlite3-sys RUSTSEC-2026-0459 — **MEDIUM — FIXED**
- Two SQLite stacks (`links = "sqlite3"`) could not coexist; sqlx pinned the
  vulnerable one. Fixed by migrating PostgresStore to tokio-postgres + rusqlite
  0.42 (bundled SQLite 3.54.1). See `audit/REMEDIATION_PLAN.md`.

### R-D — protobuf RUSTSEC-2024-0437 — **MEDIUM — FIXED**
- prometheus 0.13 → 0.14 (protobuf 3.7.2).

### R-E — Legacy `?token=` WS auth fallback — **LOW — FIXED**
- Single-use ticket only; e2e test asserts the ticket flow.

### R-G — Audit log not fsync'd — **LOW — FIXED**
- `ControlService::audit()` fsyncs every entry.

### klend repay mismatch — **CRITICAL — FIXED (PR #6)**
- `flash_repay` must carry the borrow instruction index and repay exactly the
  principal (`flash_borrow_check_matching_repay`); enforced in
  `arb-tx/src/flash/dlmm_3leg_flash_arb.rs` with tests.

## Open

### R-A — ≥24 h shadow soak — **HIGH — BLOCKED (environmental)**
- Requires a long-lived host; sandbox cannot run multi-day processes.
- Runbook: `production/SHADOW_MODE_RUNBOOK.md`.

### R-B — Live V1 transport canary — **HIGH — BLOCKED (policy)**
- Requires explicit operator authorization; `ARB_LIVE=0` fail-closed.

### R-F — 5 unmaintained transitive deps — **LOW — MONITORED**
- ansi_term, bincode 1.x, derivative, libsecp256k1, paste. No upstream fixes;
  no runtime exposure identified.

### R-H — Persistent mainnet-fork validator run — **INFO — BLOCKED (infra)**
- Snapshot-based mainnet simulation exists
  (`arb-sim/fork_sim.rs`, `mainnet_non_broadcast_simulation` tests); a live
  forked-validator run needs infrastructure beyond this sandbox.
