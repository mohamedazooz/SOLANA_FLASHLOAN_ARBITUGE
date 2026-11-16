# PHASE 11 — Performance Report

## Measured (real runs, this cycle)

| Metric | Value | Source |
|---|---|---|
| 429 rate (3-min soak, publicnode) | 9 events, 3 rotations, zero impact | soak16 log |
| Graph edges (steady) | 95–96 | soak15/16 |
| DLMM pairs (steady) | 15–21 (0 only in first 8s warmup) | soak16 |
| Rejections | honest EDGE_BELOW_HURDLE only | soak16 |
| RPC fetches/cycle | batched (batch-100) vs ~430 sequential before | commit `228c330` |
| 429-WARNs/cycle (before → after batching) | 39,519 → 6–9 | soak4 vs soak15/16 |
| Test suite wall time (release, ingest) | ~3.3s incl. live fetch test | this audit |

## Latency percentiles (P50/P95/P99)

- **NOT MEASURED with Criterion in this pass** — `cpmm_bench` exists (harness = false);
  full benchmark suite run + flamegraph + tokio-console = **NOT DONE** (backlog P2).
- Observed operational latencies (soak logs): strategy tick cadence 750ms; discovery
  refresh 60–90s; WS update → cache write sub-millisecond (write-version guard path).

## Optimization history (measured before/after)

| Change | Before | After |
|---|---|---|
| Batched getMultipleAccounts | 429 storms, pairs=0 | 6–9 429s/3min, pairs=15–21 |
| Fail-closed DLMM reserves | 4,276 phantom rejections (p50 ROI 153.5x) | 0 phantom, honest rejections |
| Canonical orientation (DLMM+CLMM) | 156x phantom quotes possible | 0 divergence WARNs live |

## Rule honored

No correctness or state-integrity trade was made for latency; the batching change
reduced network load *and* improved data honesty.
