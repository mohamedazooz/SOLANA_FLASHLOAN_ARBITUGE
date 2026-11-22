# MARKET DATA PIPELINE — Phase 7 (Mission §12)

**Status:** ACTIVE — verified against source on 2026-11-22.
**Rule:** this document describes only what exists in code. Classification labels
(`VERIFIED`, `PRESENT-UNWIRED`, `PLANNED`) distinguish evidence from intention.

---

## 1. Pipeline topology (as implemented)

```
                         ┌────────────────────────────────────────────────┐
                         │                ARB-INGEST (crate)              │
  Solana cluster ────────┤                                                │
   │                     │  helius_ws.rs      WS accountSubscribe stream  │
   ├── JSON-RPC WS ──────┤  rpc_pool.rs       multi-endpoint token-bucket │
   │   (Helius)          │  pumpdev_stream.rs dedicated pump.dev stream     │
   │                     │  transaction_rpc.rs tx status / reconcile      │
   └── HTTP JSON-RPC ────┤  fetcher.rs        paced sequential fetches    │
       (getMultiple-     │  hydration.rs      batched reserve hydration   │
        Accounts)        │  discovery.rs      pool discovery + catalog    │
                         └──────────────┬─────────────────────────────────┘
                                        │ PoolState { slot, write_version }
                         ┌──────────────▼─────────────────────────────────┐
                         │       COHERENCE / INTEGRITY LAYER              │
                         │  slot_integrity.rs  fork + gap tracking        │
                         │  provider_supervisor.rs  health / failover     │
                         │  dependency_manifest.rs  per-route accounts    │
                         │  quote_readiness.rs    fail-closed promote     │
                         │  dual_ingest.rs        WS-vs-poll shadow compare│
                         └──────────────┬─────────────────────────────────┘
                                        │ coherent snapshot set
                         ┌──────────────▼──────────────┐
                         │  arb-bot supervisor         │
                         │  opportunity gate ladder    │
                         │  strategy.rs exact quoting  │
                         └─────────────────────────────┘
```

## 2. Ingestion sources (evidence)

| Component | File | Status | Notes |
|---|---|---|---|
| Helius WS | `crates/arb-ingest/src/helius_ws.rs` (~900 lines) | **ACTIVE** | Per-endpoint backoff map (`ws_backoffs`), generation counter, stale-snapshot eviction on reconnect (`record_*` + generation bump, lines 537–560), reconnect cooldown 2.5–3.0 s (line 863), `arb_metrics::inc_ws_reconnects()` on every reconnect (lines 832/848) |
| HTTP polling | `crates/arb-ingest/src/rpc_pool.rs` (1,945 lines) | **ACTIVE — source of truth** | Multi-endpoint selection with per-endpoint backoff (lines 238–989), 429-specific backoff branches (lines 896–914, 974–989), token bucket (line 51) |
| Hydration | `crates/arb-ingest/src/hydration.rs` | **ACTIVE** | Batched `getMultipleAccounts` (batch-100) replaced per-account fan-out (~431 requests/cycle → ~DOZEN batched calls); immutable mint-decimal cache; returns only pools with both reserves non-zero |
| Pump.dev stream | `crates/arb-ingest/src/pumpdev_stream.rs` | **ACTIVE** (pump pipeline only) | Separate stream for bonding-curve pools |
| Yellowstone gRPC | `arb-ingest/Cargo.toml` optional dep `yellowstone-grpc-client 17.0.0`, feature `yellowstone` | **PRESENT-UNWIRED** | Client library compiled behind a feature; not wired into the supervisor's decision path. `require_bank_identity` in `quote_readiness.rs:14–21` is the designed promotion gate — only transports that supply bank ids (Yellowstone) may demand them |
| Dual ingest | `crates/arb-ingest/src/dual_ingest.rs` (373 lines) | **PRESENT — shadow only** | `StreamingPoolCache` isolated from `MarketStateCache`; **HTTP polling remains the source of truth**; comparison reports discrepancies; module doc explicitly says polling is NOT removed until correctness is demonstrated (lines 1–12) |
| Discovery | `crates/arb-ingest/src/discovery.rs` | **ACTIVE** | Raydium catalog + pump pairing, official program IDs (see DEX_SUPPORT_MATRIX) |

## 3. Freshness / correctness primitives (evidence)

| Primitive | Location | Semantics |
|---|---|---|
| `(slot, write_version)` ordering | `dual_ingest.rs:37–46`, `arb-types::PoolState` | Stale rejection when `(slot, wv) <= current` — lexicographic, no clock use |
| Fork / dead-slot tracking | `slot_integrity.rs` | Bounded (`MAX_TRACKED_SLOTS = 8_192`) parent map; `SLOT_DEAD` cascades invalidation to descendants → `SlotUpdateOutcome::Dead { invalidated_slots }`; bank-aware (`observe_with_bank`); gap counter counts only forward jumps past high-water mark, deduped via `counted_gap_ends` |
| Provider health | `provider_supervisor.rs` | Priority-sorted; failures ≥ threshold (3) → `OpenCircuit` + cooldown 2 s; half-open probe restore; `max_slot_lag = 8`, `stale_after = 3 s`; `select()` requires an **independent cluster slot source** — provider slots never compared across providers (module doc, line 1) |
| Quote promotion gate | `quote_readiness.rs::build_quote_dependency_set` | Fail-closed: `StreamDisconnected | NoSlotWatermark | EmptyDependencies | DuplicateDependency | MissingAccount | OwnerMismatch | StartupSnapshot | WrongProvider | WrongGeneration | MissingBankIdentity | DeadSlot | FutureSlot | StaleSlot | Incoherent(_)` — owned coherent set returned only when every dependency is live, same provider/generation |
| Dependency manifests | `dependency_manifest.rs` | Per-route required accounts + expected owners; validated by `validate_dependency_group` |

## 4. Event → decision data flow (stage ladder)

1. **Event arrival** — WS message → decode → `AccountSnapshot` into `AccountSnapshotStore`
   (provider_id + generation tagged; `strategy.rs:130` `state_start`).
2. **State processing** — slot watermark + coherence bookkeeping →
   `observe_state_processing_latency_us` (`strategy.rs:177`).
3. **Route discovery** — BF pre-screen over `MarketGraph` →
   `observe_route_discovery_latency_us` (`strategy.rs:402`), candidate `Slot(current+1)` target.
4. **Account-dependency gate (G3)** — `validate_route_account_dependencies`
   (`opportunity.rs:170–184`); rejection logged with stage=AccountDependencies.
5. **Exact integer re-quote** — pool-specific engines (never BF weights) →
   `observe_exact_quote_latency_us` (`strategy.rs:300–302`, `run.rs:279`).
6. **Risk / economics** — `t_risk_start` (`opportunity.rs:197`): flash-fee, dynamic CU
   (`estimate_route_cu` measured-CU override), capped `cu_price`, net-profit ladder (EI-gated).
7. **Assembly** — `t_assembly_start` (`opportunity.rs:963`) → `observe_assembly_latency_us`
   (line 1129).
8. **Signing** — `t_signing_start` (`opportunity.rs:1130`) → `observe_signing_latency_us`
   (line 1345).
9. **Simulation / dispatch** — `observe_simulation_latency_us` (`dispatch.rs:513,574`).
10. **Submit** — RTT recorded: `observe_submit_rtt_ms` (`dispatch.rs:880`).
11. **Landing + E2E** — `observe_landing_latency_ms` + `observe_e2e_latency_ms`
    (`dispatch.rs:1039,1048`).

All local elapsed measurement uses `std::time::Instant` (monotonic). Cross-process/source
timestamps are **not** used for any gating decision; slot + write-version are the only
cross-source ordering keys.

## 5. Failure conduct in the path

- **Reconnection**: endpoint rotation + per-endpoint backoff; every reconnect bumps the
  stream `generation` and evicts stale snapshots; half-open probe before restore.
- **Gap / fork detection**: dead-slot cascade invalidates dependent snapshots; gap
  counter is surfaced for observability (no silent continuation into the gate).
- **Disagreement**: dual-ingest comparison reports cache diffs; decisions still come from
  the polling cache (conservative).
- **Overflow**: gap accounting, failure counters and cooldowns use `saturating_*`.

## 6. Known gaps (registered — see docs/active/OPEN_ISSUES.md)

| Finding | Severity | Summary |
|---|---|---|
| HFT-F-2026-11-22-N1 | P3 | Yellowstone gRPC compiled behind feature but not wired into decision path; `dual_ingest` shadow harness exists but no sustained A/B run has produced a go/no-go decision. Default-source switch requires measured evidence. |
| HFT-F-2026-11-22-N2 | P3 | Stage ladder starts timing inside the supervisor (`t_ingest_to_quote_us` reported by the ingest worker); ingest→decode leg is not independently histogram-instrumented for the Helius WS path. |
| HFT-F-2026-11-22-N3 | P3 | Criterion benches exist only for `arb-ingest` (dual_ingest) and `arb-math` (cpmm); no ingestion-path decode/hydration hot-loop benches. |

No unsafe code, no unbounded queues, and no clock-based freshness decisions are used in
this pipeline (grep-audited 2026-11-22).
