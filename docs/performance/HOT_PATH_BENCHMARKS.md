# HOT_PATH_BENCHMARKS — Measured Hot-Path Performance

**Baseline commit**: `25ffe606f255d4ae5f0ea52e550df96afee55ef4`
**Measurement date**: 2026-11-21
**Toolchain**: rustc 1.99.1 (pinned), criterion 0.5.1
**Host**: sandboxed Linux x86_64, shared/multi-tenant CPU — treat numbers as
order-of-magnitude evidence for the *shape* of each stage, not as absolute
certification-grade latencies.

`cargo bench -p arb-math` — run to completion, exit code 0:

| Benchmark | Time (criterion median, 100 samples) |
|---|---|
| `cpmm/cpmm_out` (canonical CPMM quote, U256 checked math) | **16.8 ns** [16.674 – 16.982] |
| `cpmm/optimal_two_pool` (two-hop optimal sizing, U384 un-factored radicand search) | **205 ns** [204.01 – 206.57] |

Outliers: 1/100 (cpmm_out), 7/100 (optimal_two_pool) — normal scheduler noise
on a shared host.

## What each stage implies

- Single-pool CPMM quote ≈ 17 ns → the quote stage is never the bottleneck; a
  full 2–4 hop route evaluation costs well under 1 µs of pure math.
- Two-pool optimal sizing ≈ 205 ns → the integer sizing optimizer (coarse grid
  + refinement across a 441-bit search space) is likewise not latency-leading.

## Stage-level latency budget (architecture view)

Stages measured *offline* above are marked ✅. Stages whose latency is
dominated by network I/O (provider RTT, RPC simulate, Jito dispatch) are
tracked at runtime by `arb-math/src/latency.rs` (`RttTracker`, log-normal EWMA
model, 95% fire-time quantiles) and the `arb-ingest` dual-ingest bench
(`crates/arb-ingest/benches/dual_ingest_bench.rs`); live-network numbers are
**BLOCKED** in this sandbox (no egress) and are not fabricated here.

| Stage | Measured here | Note |
|---|---|---|
| Pool ingest → decode | ❌ BLOCKED (live) | runtime-metric based |
| Quote (per pool, CPMM) | ✅ 16.8 ns | criterion |
| Sizing (2-pool optimal) | ✅ 205 ns | criterion |
| Tx build + V1 serialize + sign | ❌ (covered by CU-budget regression tests, not wall-clock here) | CPU-bound, µs-class |
| Simulation (RPC) | ❌ BLOCKED (live) | network-bound |
| Jito dispatch / landing | ❌ BLOCKED (live) | network-bound |

## Before/after status

**No performance-relevant code change was made in this mission** (the audit
found the canonical implementations correct), so before == after at this
commit; the numbers above establish the measured baseline for future
optimization phases. Corroborating in-suite evidence from the workspace run:
`crates/arb-engine/tests/cu_regression_benchmarks.rs` (CU budgets pinned green)
and the dual-ingest bench compile target.

## Standing optimization rules (from AGENTS.md / DECISIONS.md)

- Optimize only after correctness; never trade correctness for ns.
- Hot path performs essential work only; reporting/persistence/analytics are
  moved off the latency path by construction (supervisor → dedicated metrics
  and store tasks).
- No `unsafe` without a measured need + documented invariants (workspace
  currently compiles with `unsafe_code = "allow"` at attribute level, but the
  decision-path crates use no `unsafe` blocks).
