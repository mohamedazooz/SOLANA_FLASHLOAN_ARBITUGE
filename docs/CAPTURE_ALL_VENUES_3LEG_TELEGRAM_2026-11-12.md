# Opportunity Capture: Cross-Basin, 3-Leg Meteora, All-Venue — 2026-11-12

**Status:** implemented, compiles, clippy clean. Live execution still fail-closed.

## What was wrong

The supervisor evaluated candidate classes in a **short-circuit cascade**:

```rust
let mut opps = graph.find_base_cycles(...);   // 2-hop + 3-hop, cross-DEX
opps.extend(cross_pool_dlmm);                 // Meteora 2-pool
if opps.is_empty() { opps = bellman_ford(); } // only when nothing else found
if opps.is_empty() { opps = multi_hop_dfs(); }// only when nothing else found
```

Two consequences:

1. **Longer cross-platform routes were invisible.** If any 2-hop cycle existed —
   which is almost every tick — Bellman-Ford and the 4–5 hop DFS never ran, so a
   more profitable 4-hop route across venues was never even priced.
2. **Meteora 3-leg (WSOL → A → B → WSOL) was never captured at all.** The pipeline
   `build_three_leg_meme_pipeline` existed and was tested, but no caller in the
   supervisor invoked it.

## What changed

### 1. Always-evaluate, then merge and rank

`crates/arb-bot/src/supervisor/run.rs` now evaluates **every** candidate class each
tick and merges them:

| Class | Source |
|---|---|
| 2-hop cross-DEX + 3-hop triangular | `graph.find_base_cycles` |
| Meteora 2-pool same-pair, both directions | `graph.find_cross_pool_dlmm_cycles` |
| Any-length negative cycles (all venues) | `BellmanFordEngine::find_negative_cycles` |
| 3–5 hop cross-venue DFS | `graph.find_multi_hop_cycles(.., 5)` |
| Meteora 3-leg WSOL→A→B→WSOL | `capture_three_leg_dlmm` |

Dedup is by **sorted pool-id set** (direction-agnostic), then the merged set is
sorted by `net_profit` descending so the richest route is priced and dispatched
first. Each class logs its own count, so a silent class is visible in the log.

### 2. `capture_three_leg_dlmm` (new)

`Supervisor::capture_three_leg_dlmm` bridges the engine pipeline to the standard
`ExecutionOpportunity` type:

- builds `ThreeLegMemeContextParams` (payer, WSOL-anchored, slippage, min profit,
  size ladder bounded by both `max_input` and the live risk-gate `max_sizing`);
- runs `build_three_leg_meme_pipeline`, which does chained-quote discovery and a
  3-leg-native integer size search;
- **fails closed** (empty vec, with a trace log) when there are fewer than 3
  eligible DLMM pools, no executable chain, any leg missing
  `execution_accounts`, or gross profit ≤ 0. No chain is ever fabricated;
- converts the 3 legs into `SwapHop`s carrying the pool's real
  `execution_accounts`, and emits a WSOL-anchored opportunity (`base_decimals = 9`)
  whose `net_profit` is re-derived later by the authoritative economic gate.

Config: `[strategy.dlmm_three_leg]` — `input_size`, `max_input`, `slippage_bps`,
`min_profit_lamports`, `size_ladder` (all defaulted).

### 3. Telegram per-route alerts on by default

`telegram.rs` previously **hard-muted** individual opportunity/rejection alerts
(`unwrap_or(true)` on `TELEGRAM_SILENCE_INDIVIDUAL_ALERTS`), so in a default
deployment no per-route message was ever delivered — only the minute/five-minute
digest. The default is now **off** (alerts delivered); set
`TELEGRAM_SILENCE_INDIVIDUAL_ALERTS=true` to restore digest-only behaviour.

### 4. V1 live-path repairs (carried over, re-applied after a tree reset)

- `TransactionVersion::as_rpc_version()` corrected to the official
  `solana-transaction` numbering (**0 = legacy, 1 = v0, 2 = v1**) and
  `rpc_max_supported_version_from_wire()` added.
- `simulateTransaction` (arb-sim + rpc_pool) now passes
  `maxSupportedTransactionVersion`, without which every v0/v1 pre-flight is
  rejected by the RPC.
- Per-transport V1 opt-in: `SOLANA_TX_V1_DIRECT_ENABLED` (SWQoS) and
  `SOLANA_TX_V1_JITO_ENABLED` (Jito bundle); `validate_jito_bundle` is
  version-aware (v1 ≤ 4096).

## Research (best practices applied)

| Source | Finding applied |
|---|---|
| SIMD-0385 (Activated) | v1 layout, 4096 B / 64 accts / 64 ix / 12 sigs, no ALTs, config-mask semantics |
| `solana-transaction-4.3.0` | RPC version numbering 0/1/2 — fixed the off-by-one |
| SIMD-0296 | Jito bundles already carry >1232 B; size gate must be version-aware |
| Jupiter Swap V2 docs | `/build` returns raw ix for custom tx; `/order`+`forJitoBundle` for managed landing; RTSE slippage — confirms "one route graph, many transports" is the industry pattern |
| Meteora DLMM docs | `lb_clmm` program `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`; rely on published IDL + SDK; program source not open — matches our pinned-IDL approach |

## Verification

- `cargo check -p arb-tx -p arb-sim -p arb-ingest -p arb-bot --all-targets` — 0 errors
- `cargo clippy ... -- -D warnings` — 0 warnings
- tests: arb-tx / arb-sim / arb-ingest suites

## Operator notes

- 3-leg capture only produces routes when ≥3 DLMM pools with complete
  `execution_accounts` are fresh in the tick. On a thin feed it silently yields
  nothing — that is by design (fail closed), and the trace log says why.
- Telegram: expect a large increase in per-route messages. Tune
  `min_rejected_alert_sol` / the digest if it is noisy.