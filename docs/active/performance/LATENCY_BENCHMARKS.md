# LATENCY BENCHMARKS — Phase 7 (Mission §12)

**Status:** ACTIVE — 2026-11-22. Honesty rules: nothing below is claimed as measured on
mainnet. Local `Instant` timing was used to structure the ladder; absolute numbers on a
shared VPS are environment-specific. Sustained A/B between HTTP-polling and streaming
sources has **not** been executed yet (HFT-F-2026-11-22-N1) — the harness exists
(`dual_ingest_bench.rs`), the decision does not.

---

## 1. Latency stage ladder (event → landing)

| # | Stage | Rule | Boundary metric (`arb-metrics`) | Producer site |
|---|---|---|---|---|
| 1 | event → account snapshot | (slot, write_version) ordering only | *(not separately histogrammed for WS)* | `helius_ws.rs` apply path; timer enters supervisor as `t_ingest_to_quote_us` |
| 2 | state processing | slot watermark + coherence | `STATE_PROCESSING_LATENCY_US` | `strategy.rs:177` |
| 3 | route discovery (BF pre-screen) | f64 `-ln` prefilter only | `ROUTE_DISCOVERY_LATENCY_US` | `strategy.rs:402` |
| 4 | exact integer re-quote | pool-specific engines | `EXACT_QUOTE_LATENCY_US` | `strategy.rs:302`, `run.rs:279` |
| 5 | risk + economics | integer CostLedger | (inside assembly pre-stage) | `opportunity.rs:197` |
| 6 | assembly | V0/V1 compile | `ASSEMBLY_LATENCY_US` | `opportunity.rs:1129` |
| 7 | signing | monotonic timer | `SIGNING_LATENCY_US` | `opportunity.rs:1345` |
| 8 | simulation | dispatcher | `SIMULATION_LATENCY_US` | `dispatch.rs:513,574` |
| 9 | submit | RPC RTT | `SUBMIT_RTT_MS` | `dispatch.rs:880` |
| 10 | landing | reconciliation by signature/bundle | `LANDING_LATENCY_MS` | `dispatch.rs:1039` |
| 11 | end-to-end | ingest→land | `E2E_LATENCY_MS` | `dispatch.rs:1048` |

Rule: all local elapsed values use monotonic `Instant::now()`; wall-clock and source
timestamps are never used to gate trading decisions. Cross-source ordering uses
`(slot, write_version)` only.

## 2. Benchmark harnesses present

| Bench | Crate | What it measures |
|---|---|---|
| `benches/dual_ingest_bench.rs` | `arb-ingest` (criterion 0.5) | HTTP `getMultipleAccounts` polling path vs streaming ingest decode-and-cache, equal account sets, both written to their isolated caches |
| `benches/cpmm_bench.rs` | `arb-math` (criterion 0.5) | CPMM quote core |

**Not run this session** (points to `target/` bench artifacts and wall time not committed
here). Command: `cargo bench -p arb-ingest --bench dual_ingest_bench`. This is recorded
honestly as NOT EXECUTED in this report; no performance claim is derived from it.

## 3. Test evidence captured this session (2026-11-22)

| Suite | Result |
|---|---|
| `cargo test -p arb-ingest` | **177 passed / 0 failed / 0 ignored** (150 unit + 27 integration incl. geyser boundary 5, provider supervision boundary 5, concurrency 3, quota 3, rpc telemetry 3) |
| `cargo test -p arb-metrics` | **11 passed / 0 failed / 0 ignored** (2 unit + latency_boundary 5 + latency_telemetry 4) |

Boundary tests relevant to latency/freshness invariants:
- `geyser_ingestion_boundary_tests.rs` (5) — bank-identity requiring transports.
- `provider_supervision_boundary_tests.rs` (5) — health/failover transitions.
- `latency_boundary_tests.rs` (5) + `latency_telemetry_tests.rs` (4) — stage-ladder
  recording correctness.
- `quota_exhaustion_tests.rs` (3) — 429 classification → backoff.

## 4. Environment-difference caveats

- Shared VPS vs co-located runner dominate absolute numbers; only relative A/B on the
  same host is meaningful for optimization decisions.
- `LANDING_LATENCY_MS` depends on Jito bundle acceptance vs landing semantics (see
  `JITO_LANDING_RESEARCH.md` / Phase 8); acceptance is not landing.
- Slot-length jitter: ladder metrics measured against wall time while decision freshness
  uses slot arithmetic; both are reported, never conflated.

## 5. Open performance follow-ups

| Finding | Severity | Action |
|---|---|---|
| HFT-F-2026-11-22-N1 | P3 | Run sustained dual-ingest A/B; only then consider flipping decision source |
| HFT-F-2026-11-22-N2 | P3 | Add `INGEST_DECODE_LATENCY_US` histogram at the WS apply path |
| HFT-F-2026-11-22-N3 | P3 | Extend bench coverage: hydration batching, discovery, exact-quote hot loops |
