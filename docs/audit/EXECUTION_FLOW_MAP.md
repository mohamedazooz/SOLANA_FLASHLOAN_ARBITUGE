# Execution Flow Map — opportunity → settlement (verified 2026-11-13)

One canonical pipeline. Every stage lists its owning module and the verified
hand-off. Sources are file paths in this repository.

```
MARKET DATA            arb-ingest: IngestClient (WSS) + MultiDexRegistry
                       (fetch_live_pool_states via getMultipleAccounts)
  ↓ PoolState (slot, write_version, protocol, reserves, dlmm/clmm state)
STATE VALIDATION       arb-ingest: validate_execution_readiness (stream
                       generation + slot freshness, fail closed)
  ↓ fresh_pools: Vec<PoolState>
OPPORTUNITY DISCOVERY  arb-engine: MarketGraph::build_from_pools →
                       find_base_cycles (2/3-hop) + find_multi_hop_cycles
                       (DFS up to 6 hops, marginal-rate pruning 1.0010)
                       + MeteoraDlmmCrossPoolArbitrageStrategy (2-pool DLMM)
                       + capture_three_leg_dlmm (3-leg DLMM, liquidity-proofed)
  ↓ Vec<ExecutionOpportunity> (per-hop protocol + mints)
ECONOMIC VALIDATION    arb-math: build_economic_quote (single authoritative
                       accounting: flash fee, tip, min-profit floor)
  ↓ ranked by net_profit desc
RISK GATE              arb-bot supervisor/opportunity.rs:
                       RiskGate::evaluate_flash_loan / evaluate_full;
                       kill switch checked even under BYPASS_RISK_GATE
  ↓ eligible opportunities
TRANSACTION COMPILATION arb-tx: meteora_atomic_builder / dlmm_3leg_flash_arb
                       (borrow → swap×3 → repay) / compiler (adaptive V0/V1)
  ↓ compiled tx
V1 SERIALIZATION       arb-tx transport/adaptive: 0x81 discriminator,
                       4096-byte MTU, no ALT, explicit CU + loaded-data limits
  ↓ signed tx_bytes
SIMULATION             arb-sim: simulate_transaction (maxSupportedTransactionVersion)
  ↓ simulated
FIRE-TIME GATES        supervisor/dispatch.rs: kill switch + emergency latch →
                       stream readiness → size check on SIGNED bytes →
                       durable dispatch intent (fail-closed) → send
  ↓ signature persisted BEFORE send
DISPATCH               SWQoS direct RPC (base64) or Jito bundle (multi-region)
  ↓ bundle id / signature
LANDING TRACKER        arb-land: poll to terminal status
  ↓ LandStatus::{Landed, Dropped, Expired, Failed}
RECONCILIATION         supervisor/dispatch.rs resolves intent per outcome;
                       startup: supervisor/recovery.rs reconciles intents left
                       pending by a crash (never resubmits)
  ↓ resolved_status
ACCOUNTING             arb-store: record_outcome + resolve_dispatch_intent;
                       arb-metrics: net profit counters
  ↓
REPORTING              web_api (/api/status,/api/executions,/api/pnl,/api/routes)
                       + Telegram alerts + audit log
```

## Stage invariants (verified)

1. **No stale fallback**: empty fresh-pool set ⇒ tick skipped (strategy.rs).
2. **Fail-closed liquidity**: flash funding with unverifiable Kamino supply
   vault ⇒ 3-leg capture skipped (strategy.rs, this audit).
3. **One accounting source**: `arb-math::economic` is the only net-profit
   calculator; supervisor and graph consume it.
4. **No unpersisted dispatch**: intent persistence failure aborts the send
   (dispatch.rs, this audit).
5. **No blind retry**: reconciliation resolves by signature status; no code
   path resubmits an ambiguous transaction.
6. **Kill switch wins at fire time**: stop during simulation blocks the
   in-flight submission (dispatch.rs, this audit).

## Known non-pipeline behavior (documented, by design)

- The specialized 3-leg optimizer is DLMM-only; cross-protocol 3/4-pool cycles
  route through `find_multi_hop_cycles` (tested in
  `crates/arb-engine/tests/cross_dex_three_pool_live_loop.rs`).
- `PumpSwap`, `MeteoraDammV2`, `RaydiumLaunchLab` are quote-only in the adapter
  registry (no verified execution builder) — they never enter dispatch.
