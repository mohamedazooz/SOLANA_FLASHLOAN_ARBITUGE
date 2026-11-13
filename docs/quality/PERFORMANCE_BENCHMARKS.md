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

## Operational resilience observation (shadow smoke, 2026-11-13)

Under Helius HTTP 429 (rate limit), the stack exhibited: provider cooldown +
rotation, honest `rpc=FAIL` health reporting (no fabricated success), and
continued operation of non-RPC-dependent subsystems. No stale-data fallback
was used.

## Method notes

- Benchmarks measure the release binary; dev numbers are misleading for HFT
  claims (8x–70x slower) and are shown only for comparison.
- No performance claim is based on code inspection alone.
