# Phase 11 — Webhook Service Audit

**Status:** AUDITED (2026-11-22) · **Mode of evidence:** source inspection + independently executed test suite (`webhook-service/` owns its own `Cargo.lock`, lives **outside** the main workspace).

---

## 1. Purpose And Classification

The webhook service is an **edge gatekeeper for SaaS-origin events** (Bitquery, Helius pump.fun notifications, Solami). It is **deliberately decoupled** from the main arbitrage workspace: separate `Cargo.toml` + `Cargo.lock` (`webhook-service/Cargo.lock`), no dependency on workspace crates.

**Classification: PRESENT-UNWIRED.** Nothing in `crates/arb-bot` HTTP-calls or otherwise consumes webhook-service output. Its only realistic near-term value to the trading system is **pump.fun pool/opportunity discovery** (program-event feed). It must not be presented as part of the live market-data hot path — it is not (see MARKET_DATA_PIPELINE.md, Helius WS + hydration is the real ingest source).

## 2. Component Map

| Component | File:loc | Role |
|---|---|---|
| Atomics | `webhook-service/src/lib.rs:58` `constant_time_eq` | constant-time secret comparison |
| Token auth | `webhook-service/src/lib.rs:70` `verify_webhook_token` | bearer-style check, rejects `None` (fail-closed) |
| HMAC auth | `webhook-service/src/lib.rs:87` `verify_hmac_signature` | SHA-256 tag over raw body, constant-time compare (GitHub / Standard-Webhooks convention) |
| Replay guard | `webhook-service/src/lib.rs:111` `event_age_allowed(max_age, ts, now)` | stale-event rejection |
| Dedup | `webhook-service/src/dedup.rs:1-32` | two-tier: in-proc `DashMap` (local_ttl) + distributed Redis `SET NX PX` (redis_ttl_ms), `Option<ConnectionManager>` |
| Rate limit | `webhook-service/src/rate_limit.rs:26` `RateLimiter` | fixed-window per-client key (ConnectInfo / X-Forwarded-For), `MAX_TRACKED_KEYS=10_000` prune bound |
| Body cap | `webhook-service/src/config.rs:41,59` `max_body_bytes` (defaulted) | OWASP DoS control |
| Router | `webhook-service/src/router.rs:11` | program-ID / account → monitored-pool mapping |
| Handlers | `helius_pumpfun_handler.rs:46`, `bitquery_handler.rs`, `solami_handler.rs` | per-provider decode + auth + dedup + replay ordering |
| Engine | `webhook-service/src/engine.rs` | pipeline driving the above |

## 3. Threat-Scenario Coverage (tested)

Suite target: `cargo test` in `webhook-service/` — **28 passed / 0 failed** (11 lib unit + bitquery 3 + security_hardening 6 + solami 4 + webhook 4).

| Scenario | Guard | Test evidence |
|---|---|---|
| Invalid signature | `verify_hmac_signature` constant-time | security_hardening |
| Replay (stale event) | `event_age_allowed` | security_hardening |
| Duplicate delivery | two-tier dedup (local + Redis) | security_hardening |
| Malformed payload | per-handler decode errors, fail-closed | per-handler suites |
| Oversized payload | `max_body_bytes` cap | lib/config tests |
| Rate-limit exhaustion | fixed-window limiter incl. `max_per_window == 0` disable + prune bound | unit (rate_limit) |
| Unauthorized access attempt | logged rejection path | `helius_pumpfun_handler.rs:70` |
| Secret comparison timing | `constant_time_eq` (no early-exit leak) | lib unit |

Not exercised at integration level (honest gaps): Redis outage behavior of the two-tier dedup (local tier covers, but no explicit connectivity-loss test), provider retry semantics under sustained provider outage, multi-replica secret-rotation window.

## 4. Findings

### WEB-F-2026-11-22-N1: CI coverage gap for webhook-service (P2)
- **Location/cause:** `.github/workflows/rust-ci.yml` ran only `cargo-deny` against `webhook-service/Cargo.toml`; build, tests, fmt, and clippy for the service were **never executed by CI**, despite 28 tests existing locally.
- **Impact:** regressions could merge silently; test suite rot undetected.
- **Correction (this PR):** dedicated `webhook` CI job: fmt check → build → test → clippy `-D warnings`, pinned to the repo's `rust-toolchain.toml` toolchain (1.99.1) with scoped cache (`workspaces: webhook-service`).
- **Acceptance:** job green on this PR.

### WEB-F-2026-11-22-N2: per-instance in-memory rate limiter (P3)
- **Location/cause:** `webhook-service/src/rate_limit.rs:26` — counters are process-local `DashMap`. Behind >1 replica (or connection-spread across workers), effective limit multiplies by instance count.
- **Impact:** limit exhaustion protection weakens under horizontal scale. Single VPS blip today.
- **Recommended:** move to Redis-backed sliding window if/when replica count >1. Not hot path; no urgency.

### WEB-F-2026-11-22-N3: service unwired from main bot (P3, by design)
- Documented PRESENT-UNWIRED status (Section 1). If pump.fun discovery is to become an ingestion source, it must go through the same `slot_integrity` / `(slot, write_version)` coherence contract as the primary ingest — direct DB/API coupling would violate the freshness contract.

## 5. Verification Performed On This Branch

| Command | Result |
|---|---|
| `cargo fmt --check` (webhook-service) | EXIT=0 |
| `cargo test` (webhook-service) | **28 passed / 0 failed** |
| New CI job syntax | `python -c "import yaml; yaml.safe_load(...)"` OK |

Main workspace untouched: no crate changes, no behavior changes in this phase; changes are docs + CI workflow only.
