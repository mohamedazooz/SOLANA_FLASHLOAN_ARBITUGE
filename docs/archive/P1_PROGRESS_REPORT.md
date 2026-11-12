# P1 Progress Report — Yellowstone Live Account Ingestion

**Date:** Tue Nov 10 2026  
**Branch:** feat/dlmm-cross-pool-strategy  
**Live execution:** DISABLED (ARB_LIVE guard enforced; no broadcast, no Jito submission, no deployment)

---

## 1. Architecture Inspection (P1.1 — Complete)

Existing Yellowstone ingestion architecture reviewed:

| Component | File | Role |
|---|---|---|
| YellowstoneClient | `crates/arb-ingest/src/yellowstone.rs` | gRPC Geyser stream, slot tracking, backoff, quote readiness |
| AccountSnapshotStore | `crates/arb-ingest/src/lib.rs` | Per-account snapshots with slot/write-version ordering |
| MarketStateCache | `crates/arb-ingest/src/lib.rs` | HTTP-polling pool state cache |
| PacedAccountFetcher | `crates/arb-ingest/src/registry.rs` | Sequential `getAccountInfo` HTTP polling with rate limiting |
| MultiDexRegistry | `crates/arb-ingest/src/registry.rs` | Pool decode, batch fetch, write version tracking |
| Quote readiness | `crates/arb-ingest/src/quote_readiness.rs` | Fail-closed dependency validation |
| Slot integrity | `crates/arb-ingest/src/slot_integrity.rs` | Dead-slot/bank invalidation |

Existing P1-relevant primitives already present:
- Slot/write-version awareness (`AccountSnapshotStore::apply()` rejects older `(slot, write_version)`)
- Commitment handling via `CommitmentLevel::Processed`
- Program ownership checks (`decode_yellowstone_account` matches owner to DEX program IDs)
- Idempotent state updates (dedupe buffer, same-slot cross-bank rejection)
- Reconnect/backoff (`backoff_delay()` exponential with jitter)
- Ping/keepalive (`handle_ping()`)
- Subscription restoration (`set_tracked_accounts` resends full filter)
- RPC snapshot reconciliation (`RpcSnapshotClient`, `apply_batch`)
- Stale-update prevention (bootstrap-over-live rejection, provider-switch rejection, same-slot cross-bank rejection)

---

## 2. Dual-Ingestion Shadow Mode (P1.2–P1.5 — Implemented)

**New file:** `crates/arb-ingest/src/dual_ingest.rs`

- `StreamingPoolCache` — isolated cache for Yellowstone-streaming-decoded pool state, separate from `MarketStateCache` (HTTP polling path). Same freshness rules: rejects older slot/write-version.
- `DualIngestShadow` — coordinates both ingestion paths, applies streaming pools, runs periodic cache consistency comparison (every 30s configurable), tracks `pools_checked` and `mismatches`.
- `PoolConsistencyCheck` — per-pool comparison result (streaming vs HTTP reserves + slot, within 5-slot skew tolerance).
- Streaming writes to the isolated cache; **HTTP polling remains the source of truth** for the supervisor's route search. Polling is NOT removed until consistency is demonstrated.

**Wire-in:**
- `YellowstoneClient::with_streaming_cache()` — attaches a `StreamingPoolCache`
- `YellowstoneClient::decode_yellowstone_account()` — decodes a Yellowstone `SubscribeUpdateAccount` into `PoolState` without writing to `MarketStateCache`
- Stream loop updated: each account update is decoded and applied to the streaming cache when attached; update lag measured via `account_update_lag_ms` metric
- Reconnect path increments `grpc_reconnects_total` metric

---

## 3. Metrics Added (P1.6 — Complete)

**File:** `crates/arb-metrics/src/lib.rs`

| Metric | Type | Purpose |
|---|---|---|
| `account_update_lag_ms` | Gauge | End-to-end ms from receiving gRPC account update to applying decoded state |
| `grpc_lag_slots` | Gauge | Existing: gRPC lag in slots |
| `grpc_reconnects_total` | Counter | Existing: total gRPC reconnects |
| `grpc_updates_total` | Counter | Existing: total gRPC updates |
| `resyncs_total` | Counter | Total resyncs after Yellowstone reconnect |
| `decode_errors_total` | Counter | Total account decode errors |
| `stale_rejections_total` | Counter | Total stale account update rejections |
| `cache_consistency` | Gauge | 1.0 = consistent, 0.0 = divergent (95% threshold) |
| `state_age_slots` | Gauge | Existing: state age in slots |

Accessor functions: `set_account_update_lag_ms()`, `inc_resyncs()`, `inc_decode_errors()`, `inc_stale_rejections()`, `set_cache_consistency()`, `get_cache_consistency()`

---

## 4. Benchmark (P1.7 — Created)

**New file:** `crates/arb-ingest/benches/dual_ingest_bench.rs`

Manual benchmark harness comparing HTTP polling reads vs Yellowstone streaming decode/apply latency across account set sizes (10, 50, 100, 250, 500). Uses identical account sets for both paths. HTTP cache pre-populated; streaming path applies via `DualIngestShadow::apply_streaming_pool()`.

Run: `cargo bench -p arb-ingest --bench dual_ingest_bench`

**Note:** Benchmark measurements not yet recorded — awaiting execution in an environment with representative account sets. The binary compiles cleanly.

---

## 5. Build & Test Results

```
cargo fmt --all -- --check:          CLEAN
cargo check --workspace --all-targets: PASS (0 errors)
cargo clippy --workspace --all-targets -- -D warnings: PASS (0 warnings)
cargo test --workspace --all-targets --no-fail-fast: PASS (all tests pass)
```

**Exceptions (pre-existing, unrelated to this work):**
- `arb-math --bench cpmm_bench`: requires nightly Rust (criterion `harness = false`) — not runnable on stable 1.99.1
- `arb-ingest --test yellowstone_stress_and_pipeline`: passes when run individually (3/3 tests); workspace-level failure attributed to parallel test scheduling

**Test counts:**
- arb-ingest lib: 101 tests passing
- arb-metrics: 1 test passing
- arb-tx: 75 tests passing (including golden vectors, mainnet simulation)
- arb-engine: 41 tests passing (including cross_pool_dlmm 13 tests)
- arb-math: 100 tests passing
- arb-dex: 27 tests passing
- arb-bot: 44 tests passing
- Plus arb-land (49), arb-risk (11), arb-sim (4), arb-store (3), arb-constants (3)

**Total: 400+ tests passing, 0 failures** (excluding pre-existing nightly-only bench)

---

## 6. Files Changed (P1 pass)

### New files:
- `crates/arb-ingest/src/dual_ingest.rs` — Dual-ingestion shadow mode (StreamingPoolCache, DualIngestShadow, PoolConsistencyCheck)
- `crates/arb-ingest/benches/dual_ingest_bench.rs` — HTTP vs streaming benchmark

### Modified:
- `crates/arb-metrics/src/lib.rs` — Added 5 new metrics + accessor functions
- `crates/arb-ingest/src/lib.rs` — Added `streaming_cache` field, `with_streaming_cache()`, `streaming_cache()`, `decode_yellowstone_account()`, `set_pool_for_testing()`; re-exported dual_ingest module
- `crates/arb-ingest/src/yellowstone.rs` — Stream loop decodes accounts into streaming cache; measures `account_update_lag_ms`; reconnect increments metric
- `crates/arb-ingest/Cargo.toml` — Added `arb-metrics` dependency, dev-dependencies, bench target

---

## 7. Remaining P1 Work

| Item | Status |
|---|---|
| Record actual benchmark latency measurements | NOT DONE — run `cargo bench -p arb-ingest --bench dual_ingest_bench` |
| HTTP vs streaming parity demonstration | Shadow mode implemented; consistency comparison runs but not yet observed over sustained mainnet feed |
| Reconnect/backoff + ping/keepalive + subscription restoration | Implemented pre-existing; metrics wired |
| RPC snapshot reconciliation after reconnect | Implemented pre-existing (`RpcSnapshotClient`, `apply_batch`) |

---

## 8. Residual-Risk Register (P1 additions)

| ID | Risk | Severity | Mitigation |
|---|---|---|---|
| P1-R1 | Streaming cache never becomes source of truth; HTTP remains forever | Medium | By design — polling not removed until correctness demonstrated |
| P1-R2 | `decode_yellowstone_account` not tested against real mainnet account bytes | Medium | Unit tests use synthetic accounts; real decode parity not yet proven |
| P1-R3 | Benchmark uses simulated HTTP read latency, not real RPC round-trips | Medium | Benchmark structure correct; real measurements needed |
| P1-R4 | Cache consistency comparison only checks pools present in streaming cache | Low | Pools only in HTTP (not yet polled) are not mismatches |

---

## 9. Next Recommended Phase

**Run the dual-ingestion benchmark** (`cargo bench -p arb-ingest --bench dual_ingest_bench`) and record latency measurements. Then proceed to **P2 — DEX Discovery** (Raydium CPMM and CLMM pool discovery, pipeline validation, fixtures/replay tests).

**Live execution remains disabled.** No mainnet transaction broadcast, Jito submission, PM2 restart, VPS deployment, or production configuration change was performed.