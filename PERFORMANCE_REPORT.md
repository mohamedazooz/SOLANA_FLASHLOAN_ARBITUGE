# PERFORMANCE_REPORT.md

**Repository:** SOLANA_FLASHLOAN_ARBITUGE · Branch: newrepair · Date: 2026-11-16

## Optimizations present (per commits)

- HFT release profile: opt-level 3, LTO fat, codegen-units 1, panic abort, overflow-checks true.
- TCP_NODELAY socket tuning.
- Zero-alloc hot path (per commit `perf:`).
- Bounded channels, backpressure, event coalescing (ingest).
- RAM template cache (Tier-1 pre-sign <5µs) + Redis fallback (Tier-2) + full compile (Tier-3).
- WS_MAX_ACCOUNT_SUBS (512 default) with batch-probe fail-safe.

## Measurements

- Latency/CU/memory/CPU/throughput: **not measured this session** (no cargo bench, no runtime).
- Prior evidence: `reports/phase15_2/` (timestamps change; treat as historical).

## Verification gaps

- No baseline benchmark established in this session.
- Optimization claims not re-verified.
- Recommend Criterion benchmarks for quote math + tx compile + ingest throughput.

## Status

Performance work present; measurement evidence pending re-run.
