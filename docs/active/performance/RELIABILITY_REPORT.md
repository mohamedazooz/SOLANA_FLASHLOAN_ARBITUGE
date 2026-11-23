# Reliability Report — Phase 14

**Scope:** fault-injection and boundary suites, each executed on this branch with `--release`. Every "pass" below is a command run in this workspace during this phase, not a historical claim.

## Executed suites (release, this branch, 2026-11-23)

| # | Suite | Objective | Result |
|---|---|---|---|
| 1 | `cargo test -p arb-ingest --test concurrency_tests --release` | concurrent slot-integrity observers: no stall, invariants hold under contention | **3/3 pass** |
| 2 | `cargo test -p arb-ingest --test geyser_ingestion_boundary_tests --release` | stream boundary: stale/bank-recreation/write-version ordering | **5/5 pass** |
| 3 | `cargo test -p arb-ingest --test provider_supervision_boundary_tests --release` | provider supervisor failure isolation and status truthfulness | **5/5 pass** |
| 4 | `cargo test -p arb-ingest --test quota_exhaustion_tests --release` | HTTP 429 quota exhaustion fails closed with cooldown; failover to healthy endpoint in the same dispatch | **3/3 pass** |
| 5 | `cargo test -p arb-ingest --test rpc_telemetry_tests --release` | telemetry reflects actual probed provider state (no phantom "active") | **3/3 pass** |
| 6 | `cargo test -p arb-ingest mirage --release` | WS: connection drop→backoff reconnect, extended disconnect→RPC fallback, corrupt protobuf/truncated binary frames fail closed, clean close handling | **5/5 pass** (unit) |
| 7 | `cargo test -p arb-tx --test compute_budget_exhaustion_tests --release` | CU-limit exhaustion envelopes cannot pass silently | **5/5 pass** |
| 8 | `cargo test -p arb-tx --test intermediate_balance_isolation_tests --release` | no pre-existing wallet/ATA balance can disguise a failed leg | **8/8 pass** |
| 9 | `cargo test -p arb-tx --test v1_size_ladder_tests --release` | V1 4096-byte ladder incl. acceptance ceiling at the MTU boundary | **1/1 pass** |
| 10 | `cargo test -p arb-bot --test shadow_soak_boundary_tests --release` | shadow soak boundary conditions | **5/5 pass** |

## What is proven, by class

- **Provider failure:** RPC 429 storms fail closed with cooldown (no quota-firehose), failover within the same dispatch to healthy endpoints. Verified by suites 3, 4.
- **Transport failure:** WS drop triggers exponential backoff; extended outage degrades to HTTP poll fallback; malformed frames are rejected, not guessed. Verified by suite 6 + boundary tests of slot generation (suite 2).
- **Data integrity:** slot/write-version ordering rejects stale, cross-bank, and out-of-order updates; dead slots invalidate related caches. Verified by suites 1, 2.
- **Economic safety under stress:** compute-budget exhaustion and intermediate-balance isolation suites prove adversarial envelopes cannot fake profitability. Verified by suites 7, 8.

## Gaps (explicit, not hidden)

- No live network-failure soak (Yahoo-style chaos: node reboot mid-dispatch, Jito outage drain) — sandbox cannot host a real validator; the shadow-soak boundary suites are the in-repo proxy. A 24 h live shadow soak remains a §17 Phase-22/17 operator action.
- Database-outage behavior: SQLite is local-durable; WAL corruption recovery is documented in `DATABASE_RECOVERY.md` but not chaos-tested against disk-level failure.

## Verdict

`RELIABILITY: unit/integration-proven, chaos-not-proven`. Local deterministic reliability is established by 41 passing fault-injection assertions above; live chaos and 24 h soak are explicitly outstanding before SHADOW READY can upgrade to PRODUCTION.
