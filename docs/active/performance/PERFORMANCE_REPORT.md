# Performance Report — Phase 14 Baseline

**Rule honored:** these are *measured* numbers, not claims. Every entry lists its exact command. Sandbox caveat: the benchmark host is a shared sandbox VM; compute numbers bound the relative behavior of the hot path, while network RTTs are production-representative.

## Measurement environment

- Sandbox VM, shared CPU; `--release` profiles with Cargo bench defaults (opt-level 3).
- Date: 2026-11-23. Commit: phase/14 branch baseline (origin/main `ca3d49d`).

## 1. Hot-path microbenchmarks (Criterion, 100 samples)

| Operation | Time | Command |
|---|---|---|
| `cpmm_out` (one swap hop) | **16.97 ns** (16.89–17.06) | `cargo bench -p arb-math` |
| `optimal_two_pool` (closed-form optimal borrow x*) | **194.15 ns** (193.61–194.75) | `cargo bench -p arb-math` |

Reading: the entire economic decision core for a 2-pool route costs under 0.5 µs. No `f64` in these paths; pure u128 integer math. Engine arithmetic is not the bottleneck — network RTT dominates.

## 2. Ingest fan-out (dual-ingest shadow benchmark, synthetic 32-byte pool states)

| Accounts | HTTP polling (ms) | Streaming (ms) | Command |
|---|---|---|---|
| 10 | 0.002 | 0.012 | `cargo bench -p arb-ingest --bench dual_ingest_bench --features extra-ingest` |
| 50 | 0.004 | 0.042 | 〃 |
| 100 | 0.008 | 0.051 | 〃 |
| 250 | 0.024 | 0.266 | 〃 |
| 500 | 0.068 | 0.572 | 〃 |

Both paths ingest identical account sets with equivalent write semantics. Streaming's shadow-sim overhead is per-message dispatch, not decode; consistent with the standing decision that HTTP `getMultipleAccounts` polling remains the source of truth until stream consistency is proven at runtime (TXC-N1).

## 3. Live mainnet pre-flight (read-only, no broadcast)

`cargo run -q -p arb-bot --bin latency-bench --release` with operator env, 2026-11-23:

| Segment | Measured | Notes |
|---|---|---|
| Helius RPC RTT (burst avg) | **39.0 ms** (30.7–45.7) | |
| Solami RPC RTT | **31.8 ms** avg (6.9–81.1) | high variance |
| Raydium v4 account decode (752 B real bytes) | **4 ns**/op | |
| Orca Whirlpool decode (653 B real bytes) | **3 ns**/op | |
| `optimal_two_pool` on real reserves | **236 ns**/op | matches criterion |
| Ed25519 sign + wire build (real keypair, live blockhash) | **13.73 µs**/op | |
| Jito block-engine RTT: Amsterdam | **51.9 ms** | best region |
| Jito: Frankfurt | **57.6 ms** | |
| Jito: NY | **275.9 ms** | |
| Jito: Tokyo | **719.3 ms** | worst |

## 4. Slot-budget conclusion (400 ms block time)

Deterministic local work (decode → quote → sizing → build → sign) ≈ **15 µs total** — 0.004% of slot. Critical-path variance is ~100% network:

1. Ingest event transmission: ~30–50 ms (RTT-bound).
2. Local compute: ≤ 15 µs.
3. Jito ingestion: 52 ms (Amsterdam) → deploy-region decision: **Frankfurt/Amsterdam VPS colocation is the single highest-value latency lever** (Tokyo costs > 15× Amsterdam).

## 5. Profiling status

- Criterion baselines: rerunnable, `target/criterion` stores history for trend diffs.
- Flamegraph/perf: unavailable in sandbox (no perf-event permission) — recorded as not run, not claimed.
- Tokio console / Prometheus: the bot exports Prometheus metrics at `127.0.0.1:9100/metrics` (Docker HEALTHCHECK depends on it); Grafana dashboards are deployment-time work (see `DEPLOYMENT_ARCHITECTURE.md`).

## 6. "Never claim without a comparable benchmark"

Any optimization PR must rerun the exact commands above and show both numbers side-by-side. No optimization work was performed in this phase; this is the baseline the mission requires before touching code (mission §19, "Define baseline measurements before optimization").
