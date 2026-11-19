# Performance Benchmarks

**Environment:** sandbox, release profile (`opt-level=3`), real mainnet bytes,
live cluster blockhash, Helius RPC, Jito block-engine probes.
**Tool:** `cargo run --release -p arb-bot --bin latency-bench`.
**Date:** 2026-11-13.

## Dev vs release (same machine, same bench)

| Operation | dev | release | Speedup |
|---|---|---|---|
| Raydium v4 layout decode | 114 ns | **4 ns** | ~28x |
| Orca Whirlpool decode | 141 ns | **2 ns** | ~70x |
| `optimal_two_pool` sizing | 7,295 ns | **257 ns** | ~28x |
| Ed25519 sign + wire | 117.5 µs | **13.8 µs** | ~8.5x |

## Total critical path (release, per arbitrage movement)

| Stage | Latency | Dominated by |
|---|---|---|
| WS/RPC ingest event | ~50 ms | network |
| Binary state deserialization | 2.0 µs | local (negligible) |
| Sizing / marginal split | 1.0 µs | local (negligible) |
| Tx build + real signing | 15.0 µs | local (negligible) |
| Jito block-engine ingestion | ~999 ms | network RTT |
| **Total slot-to-land** | **~1,049 ms** | network |

Local compute is < 20 µs of the critical path; the path is network-bound.
Server placement near Frankfurt/Amsterdam shaves ~650 ms of Jito RTT.

## Provider probes (same run)

- Jito block-engine: Amsterdam 41 ms, Frankfurt 60 ms, NY 282 ms, Tokyo 747 ms.
- Helius RPC RTT (earlier campaign): avg 18.4 ms (min 12.4 / max 29.9).

## Criterion re-measurement (sandbox, 2026-11-16)

Tool: `cargo bench` (release profile, criterion 0.5). Commands and raw output
recorded in `docs/IMPLEMENTATION_PROGRESS.md`. This is a different (weaker)
sandbox than the 2026-11-13 campaign above — numbers are for regression
tracking, not absolute comparison across machines.

| Benchmark | Mean |
|---|---|
| `cpmm/cpmm_out` (quote) | 13.914 ns [13.792, 14.074] |
| `cpmm/optimal_two_pool` (borrow sizing) | 233.13 ns [231.84, 234.66] |

Dual-ingest harness (`cargo bench -p arb-ingest --bench dual_ingest_bench
--features extra-ingest`, in-process mock, no network):

| Accounts | HTTP (ms) | Stream (ms) |
|---|---|---|
| 10 | 0.001 | 0.013 |
| 100 | 0.009 | 0.049 |
| 500 | 0.076 | 0.571 |

Caveat: this harness measures the in-process ingest/write path only, not
network RTT; HTTP-polling remains source-of-truth until streaming consistency
is proven (unchanged conclusion from the harness itself).

## Operational resilience observation (shadow smoke, 2026-11-13)

Under Helius HTTP 429 (rate limit), the stack exhibited: provider cooldown +
rotation, honest `rpc=FAIL` health reporting (no fabricated success), and
continued operation of non-RPC-dependent subsystems. No stale-data fallback
was used.

## Method notes

- Benchmarks measure the release binary; dev numbers are misleading for HFT
  claims (8x–70x slower) and are shown only for comparison.
- No performance claim is based on code inspection alone.
