> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Latency Audit — Per-Stage Instrumentation Inventory

> Scope: master directive §10 (Latency & Ping Optimization) and §17 (Execution
> Timeline). Every claim is classified: **[code]** verified by reading, **[test]**
> verified by running a test, **[unk]** not yet measured or implemented.
> Audited 2026-11-19 (P9 pass). Baseline commit: `f72afbc`.

---

## 1. Method

Latency must be measured per pipeline stage — not by RPC ping alone (§10.1).
The project's latency instrumentation lives in `arb-metrics` (Prometheus
histograms via `Histogram::with_opts`, `crates/arb-metrics/src/lib.rs:223-319`)
and is fed from supervisor hot-path call sites. A standalone profiler binary
(`crates/arb-bot/src/bin/latency_bench.rs`) measures decode + quote + build +
sign + relay-ingest on **real live accounts** against a 400 ms slot deadline.

## 2. Instrumented stages [code]

| Stage | Metric | Observe fn | Call site wired |
|---|---|---|---|
| State processing (per account update) | `state_processing_latency_us` | `observe_state_processing_latency_us` (`lib.rs:488`) | `supervisor/strategy.rs:68` |
| Exact quote calculation | `exact_quote_latency_us` | `observe_exact_quote_latency_us` (`lib.rs:496`) | `strategy.rs:226`, `supervisor/run.rs:268` |
| Route discovery | `route_discovery_latency_us` | `observe_route_discovery_latency_us` (`lib.rs:492`) | `strategy.rs:334` |
| Simulation | `simulation_latency_us` | `observe_simulation_latency_us` (`lib.rs:484`) | `dispatch.rs:391`, `dispatch.rs:450` |
| Ingest → quote freshness window | `t_ingest_to_quote_us` (in-record field, not histogram) | — | `strategy.rs:65` |
| Account-update lag (event arrival vs slot) | `account_update_lag_ms` (gauge) | — | `arb-metrics/lib.rs:35` |

Histogram bucket layout is defined via `with_opts` (`lib.rs:223-319`), so
p50/p90/p95/p99 are derivable from Prometheus quantiles without code changes.

## 3. Measured results [unk]

No committed evidence file contains p50/p90/p95/p99 runs from the live
supervisor yet. `latency_bench.rs` exists and was designed for this, but no
dedicated report of its output has been produced in this audit cycle.
**Status: instrumentation verified in code; absolute numbers NOT claimed.**
Action item (P9 backlog, see §6): commit a benchmark + live-run report to
`reports/latency/`.

## 4. Un-instrumented stages (gaps) [code]

The directive's §10 list contains stages with **no** histogram today:

1. WS/Yellowstone event arrival → decode start (transport-level RTT).
2. Decode time per protocol payload (currently folded into state-processing).
3. Instruction build / compile / sign.
4. Submission (Jito relay ingest is only in `latency_bench`, not in-metrics).
5. Bundle acceptance → landing → confirmation → finalization
   (landing status is tracked logically in the dispatch/land lifecycle, but no
   duration histogram is recorded per transition).

These gaps are **recorded, not yet fixed** — matched by P0 rows marked
`deferred→P9` in `full-pipeline-audit.md`. They do not affect correctness;
they affect observability only.

## 5. Leader-wait attribution [code]

Directive §17 requires separating local processing from network/leader wait.
Current state: landing-status transitions record timestamps as part of the
landing ledger (event history, not interval math); leader-wait is not isolated
into a metric. The in-record `t_ingest_to_quote_us` field on snapshots is the
correct pattern to extend.

## 6. Findings + required work

| ID | Finding | Severity | Status |
|---|---|---|---|
| LAT-1 | Build/sign/submit/landing stages lack duration histograms | observability | partially fixed @ this commit — assembly/signing (opportunity.rs) and submit-RTT/landing/E2E (dispatch.rs) now feed `arb-metrics` histograms `assembly_latency_us`, `signing_latency_us`, `submit_rtt_ms`, `landing_latency_ms`, `e2e_latency_ms` (regression test `latency_histograms_observe`); quote/risk histograms (strategy.rs) pre-existed |
| LAT-2 | No committed p50..p99 evidence report | observability | open — requires a live `-p arb-bot --bin latency_bench` run on Linux with real RPC/gRPC endpoints; numbers must not be fabricated |
| LAT-3 | Leader-wait vs local-processing attribution absent | observability | fixed @ this commit — `submit_rtt_ms` (broadcast round-trip) and `landing_latency_ms` (accept-to-confirmation) are recorded as separate network-phase series; local processing is the µs histograms. Leader-wait itself (slot scheduling) remains derivable only by landing_ms minus RTT — no dedicated collector |

**Test / verification method (for when these close):**
a regression test asserting each new `observe_*` fn produces a histogram with
≥1 observation after a synthetic call, plus a `reports/latency/*.json` file
generated by `latency_bench` copied verbatim from run output.

## 7. Evidence

- `crates/arb-metrics/src/lib.rs:35-38, 409, 424, 484-496` — metric names + observers.
- `crates/arb-bot/src/supervisor/strategy.rs:65,68,226,334` — call sites.
- `crates/arb-bot/src/supervisor/dispatch.rs:391,450` — simulation latency.
- `crates/arb-bot/src/supervisor/opportunity.rs:920,1135` — assembly + signing signal emission (LAT-1).
- `crates/arb-bot/src/supervisor/dispatch.rs:779,938-942` — submit RTT, landing, and E2E signal emission (LAT-1/LAT-3).
- `crates/arb-metrics/src/lib.rs` tests module — `latency_histograms_observe` regression (all 6 latency histogram families present after synthetic observations).
- `crates/arb-bot/src/bin/latency_bench.rs` — standalone real-data profiler.
- `docs/research/2026-solana-hft-best-practices.md` — provider RTT guidance (ping alone invalid).
