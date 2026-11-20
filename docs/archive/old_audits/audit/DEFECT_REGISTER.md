> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Defect Register — Complete Audit

> **Source:** Cross-referenced from [CRITICAL_BUGS.md](../quality/CRITICAL_BUGS.md), [PHASE_12_REPOSITORY_AUDIT.md](../quality/PHASE_12_REPOSITORY_AUDIT.md), [PHASE_15_FINAL_REPORT.md](../quality/PHASE_15_FINAL_REPORT.md), and direct code inspection.
> **Severity Scale:** CRITICAL (fund loss / soundness) > HIGH (wrong behavior reachable in production config) > MEDIUM (misleading docs / dead surface) > LOW (cosmetic / future maintenance).
> **Status:** FIXED / OPEN / BLOCKED / MONITORED.

---

## Fixed Defects

### F-1 — Profitability gate could pass a structural loss when hurdle = 0
- **ID:** F-1
- **Severity:** CRITICAL
- **Location:** `crates/arb-sim/src/profitability_gate.rs` (net-profit step)
- **Root Cause:** `net_profit = gross_edge.saturating_sub(total_costs)` maps a guaranteed loss (`gross_edge < total_costs`) to `0`; with `strict_profit_hurdle == 0` the check `0 >= 0` passes the gate.
- **Fix:** Explicit `NetProfitNegative { gross_edge, total_costs }` rejection before hurdle comparison; subtraction is now provably non-clamping.
- **Regression Tests:** `test_profitability_gate_rejects_structural_loss_even_with_zero_hurdle`, `test_profitability_gate_zero_hurdle_passes_exact_breakeven`
- **Status:** ✅ FIXED (verified in audit)

### F-2 — 69 dead/phantom environment keys in `.env.example`
- **ID:** F-2
- **Severity:** HIGH
- **Location:** `.env.example` (128 keys → 59 after cleanup)
- **Root Cause:** Template accreted keys from abandoned architectures (Sentinel admin/WS, HuggingFace/S3, Solami Beam/gRPC/webhook, webhook domain, Jupiter-adjacent price feeds) and providers never part of execution path: **1inch, OKX, Binance, Arbitrum/EVM, Flashbots, Titan, Beaver**. Also `ENABLE_ARBITRUM_FLASHLOANS=true` — unused key defaulting to `true`, and `HOST=0.0.0.0` for a localhost-bind control plane.
- **Fix:** Every key grep-verified against all source files with exact-quoted-string matching. Removed only with **zero** consumers.
- **Regression Tests:** None applicable (configuration hygiene)
- **Status:** ✅ FIXED

### F-3 — `DRY_RUN` is not a real gate
- **ID:** F-3
- **Severity:** MEDIUM
- **Location:** `.env.example` (was `DRY_RUN=false`); sole consumer is `crates/arb-bot/src/heartbeat.rs:245`
- **Root Cause:** Template implied a dry-run switch that does not gate dispatch. Actual gates: `ARB_LIVE` (`main.rs:76-79`), `LIVE`/`EXECUTE_LIVE` (`main.rs:140-142`), `ARB_MODE` (CLI > env > config > `shadow`).
- **Fix:** Template now ships `DRY_RUN=true` with explicit comment that it is heartbeat-reported only; recorded in KNOWN_LIMITATIONS.
- **Status:** ✅ FIXED (doc-side)

### F-4 — `SOLANA_TX_V1_JITO_ENABLED` documented but unwired
- **ID:** F-4
- **Severity:** MEDIUM
- **Location:** `.env.example` V1 block claimed a Jito-ingress V1 toggle; exact-string grep shows **no** consumer.
- **Root Cause:** Only `SOLANA_TX_V1_DIRECT_ENABLED` is read (`supervisor/dispatch.rs:248`, `supervisor/opportunity.rs:766`).
- **Fix:** Key removed; comment corrected; limitation recorded in KNOWN_LIMITATIONS.
- **Status:** ✅ FIXED (doc-side)

### F-5 — Duplicated `MIN_NET_PROFIT_USD` with conflicting values
- **ID:** F-5
- **Severity:** HIGH
- **Location:** `.env.example` had `MIN_NET_PROFIT_USD=0.05` and later `MIN_NET_PROFIT_USD=0.10`
- **Root Cause:** Two sources of truth for one economic parameter; whichever line the operator's dotenv loader keeps last silently doubles the profit floor.
- **Fix:** Duplicate removed; single canonical entry (0.05) remains.
- **Status:** ✅ FIXED

---

### R-I — Silent production fallback to in-memory SQLite
- **ID:** R-I
- **Severity:** CRITICAL
- **Location:** `crates/arb-bot/src/supervisor.rs` (`Supervisor::new`, store selection)
- **Reproduction:** Configure `store.url = "sqlite:///app/data/shadow.db"` on host without `/app/data`; observe startup succeed with **no** database file and non-durable dispatch intents. Also: unreachable PostgreSQL → silent in-memory store.
- **Root Cause (two-layer):** (1) Explicit fallback to `SqliteStore::open_in_memory()` on any store error; (2) `sqlite://` URLs passed raw to `Connection::open` (no prefix stripping) so configured file never opened.
- **Fix:** Fail-closed `open_store(store_url, database_url)` (startup aborts with explicit error) + `sqlite_path_from_url` resolution for `sqlite:///abs`, `sqlite:///./rel`, `sqlite://rel`, plain paths.
- **Regression Tests:** `open_store_fails_closed_on_unreachable_postgres`, `open_store_fails_closed_when_database_url_postgres_unreachable`, `open_store_sqlite_url_is_durable_file_backed` (write→close→reopen→read), `sqlite_path_from_url_resolves_absolute_relative_and_plain_paths`.
- **Live Evidence:** Positive smoke (file + WAL created, 45s run) and negative smoke (unopenable path → exit 101, explicit fail-closed message).
- **Status:** ✅ FIXED

### R-C — libsqlite3-sys RUSTSEC-2026-0459
- **ID:** R-C
- **Severity:** MEDIUM
- **Root Cause:** Two SQLite stacks (`links = "sqlite3"`) could not coexist; sqlx pinned the vulnerable one.
- **Fix:** Migrated PostgresStore to tokio-postgres + rusqlite 0.42 (bundled SQLite 3.54.1).
- **Status:** ✅ FIXED

### R-D — protobuf RUSTSEC-2024-0437
- **ID:** R-D
- **Severity:** MEDIUM
- **Fix:** prometheus 0.13 → 0.14 (protobuf 3.7.2).
- **Status:** ✅ FIXED

### R-E — Legacy `?token=` WS auth fallback
- **ID:** R-E
- **Severity:** LOW
- **Fix:** Single-use ticket only; e2e test asserts the ticket flow.
- **Status:** ✅ FIXED

### R-G — Audit log not fsync'd
- **ID:** R-G
- **Severity:** LOW
- **Fix:** `ControlService::audit()` fsyncs every entry.
- **Status:** ✅ FIXED

### klend repay mismatch
- **ID:** klend-repay
- **Severity:** CRITICAL
- **Location:** `crates/arb-tx/src/flash/dlmm_3leg_flash_arb.rs`
- **Root Cause:** `flash_repay` must carry the borrow instruction index and repay exactly the principal (`flash_borrow_check_matching_repay`).
- **Fix:** Enforced in builder with tests.
- **Status:** ✅ FIXED (PR #6)

---

## Open Defects

### R-A — ≥24h shadow soak not executed
- **ID:** R-A
- **Severity:** HIGH
- **Status:** BLOCKED (environmental)
- **Blocker:** Requires a long-lived host; sandbox cannot run multi-day processes.
- **Runbook:** `docs/production/SHADOW_MODE_RUNBOOK.md`
- **Next Action:** Deploy to VPS with monitoring; run shadow mode for 24h minimum.

### R-B — Live V1 transport canary not executed
- **ID:** R-B
- **Severity:** HIGH
- **Status:** BLOCKED (policy)
- **Blocker:** Requires explicit operator authorization; `ARB_LIVE=0` fail-closed.
- **Next Action:** Operator review of `docs/quality/PRODUCTION_READINESS_2026-11-13.md` → explicit live authorization decision.

### R-F — 5 unmaintained transitive dependencies
- **ID:** R-F
- **Severity:** LOW
- **Status:** MONITORED
- **Dependencies:** `ansi_term`, `bincode 1.x`, `derivative`, `libsecp256k1`, `paste`
- **Status:** No upstream fixes; no runtime exposure identified. Monitor for CVE.

### R-H — Persistent mainnet-fork validator run
- **ID:** R-H
- **Severity:** INFO
- **Status:** BLOCKED (infra)
- **Note:** Snapshot-based mainnet simulation exists (`arb-sim/fork_sim.rs`, `mainnet_non_broadcast_simulation` tests); a live forked-validator run needs infrastructure beyond this sandbox.

---

## Summary

| Category | Count |
|----------|-------|
| CRITICAL (Fixed) | 4 |
| HIGH (Fixed) | 2 |
| MEDIUM (Fixed) | 3 |
| LOW (Fixed) | 2 |
| **Total Fixed** | **11** |
| HIGH (Open/Blocked) | 2 |
| LOW (Monitored) | 1 |
| INFO (Blocked) | 1 |
| **Total Open** | **4** |

---

## Verification Commands

```bash
# Re-run all tests (should show 0 failures)
cargo test --workspace --all-targets

# Run specific regression tests
cargo test -p arb-sim profitability_gate
cargo test -p arb-bot open_store
cargo test -p arb-tx kamino_flash

# Verify clippy clean
cargo clippy --workspace --all-targets -- -D warnings

# Verify fmt clean
cargo fmt --all -- --check
```