# PR #7 + PR #8 Integration Report (2026-11-13)

Full evidence in: [PR7_PR8_INTEGRATION_BASELINE.md](PR7_PR8_INTEGRATION_BASELINE.md) ·
[PR7_CONTROL_PLANE_AUDIT.md](PR7_CONTROL_PLANE_AUDIT.md) ·
[PR8_EXECUTION_RECOVERY_AUDIT.md](PR8_EXECUTION_RECOVERY_AUDIT.md) ·
[../audit/](../audit/) (7 documents from the previous round).

## Verdicts

| Component | Verdict |
|---|---|
| **A. PR #7 (Control Plane)** | **VERIFIED** — one control service, safe mode transitions, fail-closed auth, audited; all 5 findings fixed (C1–C4 fixed, C5 runbook) |
| **B. PR #8 (Execution/Recovery)** | **VERIFIED** — fail-closed dispatch intents, evidence-based recovery (I1 fixed this round), V1 gates, financial single-sources |
| **C. Integration** | **VERIFIED** — PR #8 ⊇ PR #7 (single divergent commit superseded); dry-run merge clean; both CI green; combined suite 954/0 |
| **D. Branch relationship** | PR #8 branched from PR #7 at `ded8e8c`; merge order **PR #7 → PR #8**; PR #9 absent from both (ancestor-check + file sweep) |

## Findings (severity-ranked)

| # | Severity | Finding | Fix | Evidence |
|---|---|---|---|---|
| I1 | HIGH | Blockhash expiry proven by fixed slot arithmetic (160) instead of `lastValidBlockHeight` | Evidence-based: persist `lastValidBlockHeight` at dispatch; recovery proves expiry with live `getBlockHeight` strictly beyond it; slot arithmetic removed | 5 recovery tests + 2 store tests pass |
| C1 | HIGH | PR #7 session token in WS URLs | PR #8 single-use tickets; fallback removed | ticket tests + e2e upgrade test |
| C2 | HIGH | Kill switch not enforced at fire time | fire-time gate in dispatch | arb-bot suite |
| F1 | HIGH | Intent persistence best-effort | fail-closed abort outcomes | arb-bot suite |
| C3/F4 | MEDIUM | Spoofable XFF | right-most entry | web tests |
| F3 | MEDIUM | Unverifiable flash liquidity proceeded | capture skipped | workspace suite |
| C4 | LOW | Audit log not fsync'd | `sync_data()` per entry | control_service audit |
| F6 | LOW | Wrong CPMM dust property | corrected invariant + seed | 50/50 props |

## Files changed this integration round

- `crates/arb-store/src/lib.rs` — `last_valid_block_height` on
  `DispatchIntentRecord`, SQLite insert/select, schema evolution
  (`evolve_schema`), legacy-upgrade test.
- `crates/arb-store/src/postgres_store.rs` — column in insert/select +
  `ADD COLUMN IF NOT EXISTS` evolution.
- `crates/arb-store/migrations/001_init.sql`, `001_postgres_init.sql` —
  column in CREATE TABLE.
- `crates/arb-ingest/src/rpc_pool.rs` — `get_block_height()`.
- `crates/arb-bot/src/supervisor/execution.rs` —
  `fetch_recent_blockhash` returns (hash, lastValidBlockHeight).
- `crates/arb-bot/src/supervisor.rs` — cache type carries evidence.
- `crates/arb-bot/src/supervisor/opportunity.rs` — evidence captured
  (live path) and passed via DispatchContext.
- `crates/arb-bot/src/supervisor/dispatch.rs` — DispatchContext field +
  persisted in the intent.
- `crates/arb-bot/src/supervisor/recovery.rs` — evidence-based
  classification; slot arithmetic removed; 6 tests.
- `crates/arb-bot/src/web_api.rs` — `?token=` fallback removed
  (ticket-only WS auth); e2e test uses the ticket flow.
- `web/src/api.ts` — stale `?token=` doc comment corrected.
- `docs/audits/*` (this round's documents), `docs/architecture/WEB_APPLICATION.md`.

## Tests actually executed (this round)

- `cargo fmt --all -- --check` — clean.
- `cargo check --workspace --all-targets` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — **0**.
- `env -u WEB_ADMIN_PASSWORD cargo test -q --workspace --all-targets` —
  **954 passed / 0 failed** (CI-equivalent; +3 vs 951: block-height
  evidence tests + legacy-DB evolution test, minus 1 rewritten).
- `web`: `tsc --noEmit` + `npm run build` — clean.

## Readiness classification

| Area | Class |
|---|---|
| I. Database (SQLite path) | **VERIFIED** (fresh + upgrade + restart + fail-closed); Postgres path **PARTIALLY VERIFIED** (compile-verified mirror; no live PG server in sandbox) |
| J. Control Plane | **VERIFIED** (unit + e2e HTTP/WS); browser visual pass **NOT VERIFIED** (no browser in sandbox) |
| K. Recovery | **VERIFIED** (deterministic tests); live failover chaos **NOT VERIFIED** |
| L. Kamino flash-loan | **VERIFIED** (canonical fee + repayment gate + property tests); mainnet program differential **PARTIALLY VERIFIED** (fixtures harness pending — PR #9 territory, out of scope) |
| M. Transaction/V1 | **VERIFIED** (wire gates + sim plumbing); live transport **NOT VERIFIED** (policy-blocked) |
| N. Three-pool execution | **VERIFIED** (builder + cross-DEX connectivity + per-pool fee tests); on-chain landing **NOT VERIFIED** |
| O. Shadow-mode readiness | **READY** (runbook below); ≥24 h soak **NOT RUN** |

## Remaining live blockers

1. ≥24 h shadow soak with predicted-vs-observed report (R-A).
2. Authorized canary for live V1 transport (R-B).
3. libsqlite3-sys RUSTSEC-2026-0459 — structurally blocked (documented R-C).
4. HTTPS/TLS termination at the reverse proxy (deployment).

## Final recommendations

- **READY FOR MERGE** — merge PR #7 first, then PR #8 (human merge; token
  cannot merge). Nothing in this round blocks merge; the code is strictly
  safer than main.
- **READY FOR SHADOW** — after merge, execute SHADOW_MODE_RUNBOOK.md.
- **NOT READY FOR LIVE** — pending the soak, the authorized canary, and
  the live-gate checklist (LIVE_READINESS_CHECKLIST.md).
