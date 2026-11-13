# Changelog

All notable changes to this project are documented here. For detailed test
evidence see [`docs/quality/TEST_EVIDENCE.md`](docs/quality/TEST_EVIDENCE.md)
and for the running remediation register see
[`docs/audit/REMEDIATION_PLAN.md`](docs/audit/REMEDIATION_PLAN.md).

## 2026-11-13 — Provider matrix + role-separated Solami WS key (PR #11 branch)

### Added

- **`SOLAMI_WS_API_KEY`** — role-separated Solami WebSocket stream key.
  `resolve_ws_key()` in `arb-ingest/src/solami_stream.rs` resolves the WS key
  as: explicit `SOLAMI_WS_API_KEY` → `api_key` embedded in `SOLAMI_WS_URL` →
  `SOLAMI_API_KEY` (legacy fallback). Previously query params were stripped
  from `SOLAMI_WS_URL`, silently discarding an operator-supplied WS key.
- **Provider matrix in `.env.example`**: Helius + Alchemy + Solami for RPC,
  WebSocket, and Yellowstone gRPC, with the exact auto-built URL forms the
  RPC pool and WS pool construct from each variable.
- 3 regression tests for key resolution; workspace campaign **961 passed /
  0 failed**.

### Verified live (2026-11-13)

- Solami RPC: `getHealth` 200 `ok`, live `getSlot` at 21–39 ms RTT.
- Solami WS and the operator Alchemy key are rejected by the providers
  (400/403/401) — documented in `docs/quality/Remaining_Risks.md`; Helius
  remains the only verified WS provider until valid keys are supplied.

## 2026-11-13 — R-I: store selection fail-closed (PR #11, branch `vibe/store-fail-closed-9d81d9`)

### Fixed

- **Silent production fallback to in-memory SQLite removed.**
  `crates/arb-bot/src/supervisor.rs` previously fell back to
  `SqliteStore::open_in_memory()` whenever the configured PostgreSQL store was
  unreachable or the configured SQLite file could not be opened. A production
  bot would then run with zero durable persistence — dispatch intents,
  recovery state, and PnL accounting all silently lost on restart. Store
  selection is now fail-closed: a configured-but-unavailable store refuses
  startup with an explicit panic message. In-memory SQLite remains available
  exclusively in tests.
- **`sqlite://` URLs now honored.** The supervisor passed store URLs raw to
  `Connection::open`, so a configured `sqlite:///path/data.db` never opened
  the intended file (the URL string itself was treated as a relative path).
  The docker-compose shadow deployment had zero durable persistence as a
  result. New `sqlite_path_from_url()` resolves absolute, relative (`./`,
  `../`), and plain paths correctly.
- Store precedence (documented and tested): config `store.url` postgres →
  `DATABASE_URL` postgres → `DATABASE_URL` `sqlite://` → config
  sqlite/plain path. Any configured-but-unavailable store fails closed.

### Evidence

- 4 new regression tests in `supervisor.rs` (path resolution, fail-closed on
  unreachable Postgres from both `store.url` and `DATABASE_URL`, durable
  file-backed SQLite write/reopen/read-back).
- Full campaign: **958 passed / 0 failed** (`cargo test --workspace
  --all-targets`), `cargo fmt` clean, `cargo clippy -D warnings` clean,
  `cargo audit` 0 vulnerabilities.
- Live smoke tests with the release binary: positive run created
  `shadow_smoke.db` + WAL (70 KB in 45 s, honest Helius-429 handling);
  negative run with an impossible store path exited 101 with an explicit
  fail-closed refusal. See `docs/quality/TEST_EVIDENCE.md`.

### Added (Phase-8 deliverable docs)

- `docs/quality/EXECUTIVE_AUDIT_REPORT.md`
- `docs/quality/CRITICAL_BUGS.md`
- `docs/quality/PERFORMANCE_BENCHMARKS.md`
- `docs/quality/Remaining_Risks.md`
- `docs/audit/FLASHLOAN_AUDIT.md`
- `docs/audit/DATABASE_AND_RECOVERY_AUDIT.md`
- Updated: `docs/quality/TEST_EVIDENCE.md`,
  `docs/audit/REMEDIATION_PLAN.md`, `docs/production/LIVE_READINESS_CHECKLIST.md`,
  `docs/PROJECT_STATE.md`.

## 2026-11-13 — PR #7 + PR #8 merged to main (`c4fe5fa`)

- **PR #7** — Unified Control Plane: ControlService as the single source of
  truth for Telegram + Web dashboard, authenticated Axum API, WebSocket
  streaming, emergency stop, runtime mode switching with audit logging.
- **PR #8** — Remediation R2/R3/R9/R10/R12: dead `arb-mcp` crate removed,
  canonical Kamino fee in `arb-math`, supervisor monolith split, `arb-tx`
  split into 31 domain modules, durable dispatch intents + startup
  reconciliation, web hardening, test isolation.
- **R-C** (structural blocker, commit `fed4f2b`): sqlx removed from
  `arb-store` (PostgreSQL via `tokio-postgres 0.7`), `rusqlite` 0.32 → 0.42
  (bundled SQLite 3.54.1 ≥ 3.50.4 fix), single `libsqlite3-sys` in
  `Cargo.lock`, `cargo audit` clean.

## Earlier campaigns

See `docs/quality/TEST_EVIDENCE.md` and `docs/audit/REMEDIATION_PLAN.md` for
the full history of remediation items (R2–R12), protocol verification, and
test campaigns.
