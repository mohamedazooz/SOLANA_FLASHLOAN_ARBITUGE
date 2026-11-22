# Rust HFT / Systems Best Practices

**S3 verification date**: 2026-11-22.

## Verified / current-practice facts

| Item | Finding | Source |
|---|---|---|
| Bounded channels | Bounded bounded-queue backpressure is the production default; unbounded producers hide overload until memory/latency blow up | [task-contention patterns](https://oleg.is/blog/tokio-task-contention-patterns), [tokio channel patterns 2026](https://toolsku.com/en/blog/rust-tokio-channel-patterns-2026) (S3) |
| Lock discipline | Short lock hold times; contention turns microsecond paths into queues; prefer message passing or sharded state | [task-contention patterns](https://oleg.is/blog/tokio-task-contention-patterns) (S3) |
| Runtime isolation | Separate runtimes (or pinned tasks) for latency-critical vs background work creates a scheduling boundary; `spawn_blocking` is not a universal fix (~50k blocking tasks/s showed degradation on a 32-core host in the cited measurement) | [dial9 principles for fast Tokio applications](https://dial9-rs.github.io/blog/principles-for-fast-tokio-applications) (S3, updated 2026-09) |
| Tick-to-trade method | Measure a labeled boundary end-to-end (receive→send), preallocate/arena to avoid page faults mid-path, zero-copy where alignment is controlled; throughput ≠ latency | [hfrtai tick-to-trade overview](https://hfrtai.com/knowledge/how_do_you_optimize_tick-to-trade_latency_for_high-frequency_trading_and_event-driven_systems_in_2026.php) (S3, secondary) |
| Overflow safety | Keep integer checked arithmetic in money paths (u64/u128), fixed-point only; no f64 in decisions | repo invariant + register (inherited) |

## Existing implementation mapping

- Bounded queues: previous P2-01 audit found no unbounded channel usages under `crates/` (OPEN_ISSUES P2-01 closed) — keep it that way; add an arch test if a decoder pipeline is introduced.
- Overflow: workspace release profile pins `overflow-checks = true` (Cargo.toml) — deliberate defense-in-depth; money paths use explicit `checked_*`.
- Toolchain: pinned 1.99.1 (rust-toolchain.toml), edition 2024, `let`-chains used — fmt/clippy reproducibility between local/CI/Docker documented in AGENTS.md.
- Profiling: criterion benches + probe infrastructure exist; `tokio-console`, `flamegraph`, `perf` not wired — acceptable until Phase 14 benchmarks require them.

## Recommendations (REC)

1. Keep the ingestion→quote→route pipeline on the shared runtime but confine telemetry/DB writes to a task boundary; if measured contention appears, split runtimes rather than adding locks.
2. Any new hot-path channel must be bounded with an explicit capacity comment explaining drop vs block policy.
3. Measure before optimizing: Phase 14 requires baseline numbers first (LATENCY_BENCHMARKS.md); no optimization PR without comparable before/after criteria results.

## Tests required

- Backpressure behavior test (producer outruns consumer ⇒ backpressure observed, no unbounded growth) for any new queue.
- Benchmark regression: criterion baselines for quote + build path in CI (non-blocking), graphed over time.
