# HFT ARCHITECTURE — Phase 7 (Mission §12)

**Status:** ACTIVE — source-verified 2026-11-22. Evidence-based only; no speculative
optimization has been applied. Rule honored: **HFT changes require measured evidence;
no unsafe Rust without demonstrated need + invariants + tests.**

---

## 1. Current hot-path architecture

Single multi-threaded Tokio runtime (`#[tokio::main]`, default multi-thread workers,
`crates/arb-bot/src/main.rs:374`). Supervisor loop advances as a stage ladder with
monotonic `Instant` timers at every stage boundary (see MARKET_DATA_PIPELINE §4).

### Concurrency primitives in the hot path

| Where | Primitive | Why correct + fast | Evidence |
|---|---|---|---|
| Caches (`MarketStateCache`, `StreamingPoolCache`) | `parking_lot::RwLock` | Contended reads are short clone-from-map ops; no std lock overhead, no poisoning | `dual_ingest.rs:20–22`, `hydration.rs` |
| Metrics | `OnceLock` static atomics + `register_histogram` | Lock-free increments; histograms only touched at stage boundaries | `arb-metrics/src/lib.rs:240+`, observe fns 612–646 |
| Measured CU map | `parking_lot::Mutex<HashMap<...>>` | Tiny maps, short critical sections | `opportunity.rs:236` |
| Ingest → decisions | Slot+write-version ordered snapshots, no cross-thread clock comparisons | Deterministic ordering without wall-clock skew | `dual_ingest.rs:37–46` |
| RPC fan-out | Token-bucket + per-endpoint backoff maps (`RwLock<HashMap<usize, Instant>>`) | No thundering-herd requests, 429-aware rotation | `rpc_pool.rs:51, 238, 896–914` |

### Batching (measured structural win already shipped)

`hydration.rs` replaced the per-account `PacedAccountFetcher` fan-out (~163 pool +
~68 vault + ~200 derived accounts **fetched one-by-one every 750 ms** — 300–400 HTTP
requests/cycle) with batch-100 `getMultipleAccounts` through the shared `RpcPool`
(`hydration.rs:33–52`). This is the largest documented latency/CU-of-network win in the
ingest path and its comment carries the measured history.

## 2. Instrumentation (wired, verified call sites)

Every observability function in `arb-metrics::observe_*` (lib.rs:612–646) is called from
production code — verified by grep across `arb-bot`:

| Stage | Metric | Call site |
|---|---|---|
| state processing | `STATE_PROCESSING_LATENCY_US` | `strategy.rs:177` |
| route discovery | `ROUTE_DISCOVERY_LATENCY_US` | `strategy.rs:402` |
| exact quote | `EXACT_QUOTE_LATENCY_US` | `strategy.rs:302`, `run.rs:279` |
| assembly | `ASSEMBLY_LATENCY_US` | `opportunity.rs:1129` |
| signing | `SIGNING_LATENCY_US` | `opportunity.rs:1345` |
| simulation | `SIMULATION_LATENCY_US` | `dispatch.rs:513,574` |
| submit RTT | `SUBMIT_RTT_MS` | `dispatch.rs:880` |
| landing | `LANDING_LATENCY_MS` | `dispatch.rs:1039` |
| end-to-end | `E2E_LATENCY_MS` | `dispatch.rs:1048` |

Missing rung: **ingest-side event→decode histogram** for the Helius WS path (first stage
timer is inherited via `t_ingest_to_quote_us`). Registered as HFT-F-2026-11-22-N2.

## 3. Optimization policy (binding)

1. **Evidence first.** Any optimization must be preceded by a criterion baseline and
   followed by a comparable run (`cargo bench -p arb-ingest --bench dual_ingest_bench`,
   `cargo bench -p arb-math --bench cpmm_bench`).
2. **No unsafe.** Zero `unsafe` outside test modules across `arb-ingest`/`arb-bot`
   (grep-audited). CI `clippy -D warnings` enforces warning cleanliness.
3. **No wall-clock gating.** Freshness uses `(slot, write_version)` + provider health
   timers only; wall clocks never decide trade eligibility.
4. **Conservative subsidy of quotes** is prohibited; the exact-quote ladder (BF
   pre-screen → integer pool-specific quote) is the mandated path (`runs/evaluate_cycle`,
   `strategy.rs` exact re-quote).
5. **Sub-slot hooks:** the design supports receiving updates faster than the 400 ms slot
   cadence (WS event loop is async and decoupled), but no sub-slot tick loop is
   introduced without a demonstrated need + benchmark difference. Not implemented, on
   purpose. No `spin`/`exceeded-slippage` unsafe hot paths added.

## 4. Bottleneck analysis (structural, honest)

| Bottleneck | Severity | Analysis |
|---|---|---|
| HTTP polling as decision source | Dominant | Poll cadence (750 ms hydration cycle) bounds stale-data window; latency floor is one full poll cycle. Mitigation is dual-ingest + Yellowstone promotion, gated on measured evidence (HFT-F-...-N1). |
| Clone-heavy snapshots | Minor | `snapshot()` returns owned `Vec<PoolState>` (clone per call). Acceptable at current pool cardinality (hundreds); would need `Arc<Path>`-style hot-pool caching only after a benchmark shows it matters. |
| `Instant`-based provider freshness | None | Correct-by-design; monotonic; no cross-provider slot comparison. |
| Tokio runtime defaults | None today | No measured contention; no runtime tuning applied without evidence. |

## 5. Verification commands (this session)

```
cargo test -p arb-ingest -p arb-metrics        # see LATENCY_BENCHMARKS §3 for counts
cargo bench -p arb-ingest --bench dual_ingest_bench   # A/B harness present
cargo clippy --workspace --all-targets -- -D warnings
```

## 6. Findings → docs/active/OPEN_ISSUES.md

- HFT-F-2026-11-22-N1 (P3): Yellowstone compile-ready, not decision-wired; A/B evidence
  required before any default switch.
- HFT-F-2026-11-22-N2 (P3): no dedicated ingest→decode histogram.
- HFT-F-2026-11-22-N3 (P3): bench coverage limited to 2 crates.
