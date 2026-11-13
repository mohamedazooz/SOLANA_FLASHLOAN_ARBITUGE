# PHASE 2 — Architectural Forensic Audit

## Module dependency flow (verified from `use` graph)

```
arb-constants ──► arb-types ──► arb-math ──► arb-engine ──► arb-bot
                     │            │             │
                     ▼            ▼             ▼
                arb-ingest    arb-dex       arb-tx ◄── arb-land
                     │                          │
                     └────────► arb-sim ◄───────┘
arb-store, arb-metrics, arb-risk: leaf consumers
```

## Live pipeline (verified call chain, this session)

```
main.rs ─► Supervisor::run (run.rs)
  ├─ WS worker ............ helius_ws::start_stream ─► cache writes (3 fail-closed arms)
  ├─ Ingestion worker ..... 750ms ─► registry.fetch_live_pool_states ─► ingest.apply_pool_update
  ├─ Discovery refresh .... 60–90s ─► Orca catalog / Raydium v3 / DexScreener (DLMM only)
  ├─ Strategy tick ........ find_base_cycles_with_stats + find_cross_pool_dlmm_cycles
  │                        + find_multi_hop_cycles + capture_three_leg_dlmm
  ├─ Opportunity gates .... implausibility (ROI ≤ 1500 bps) → risk → economic
  └─ Dispatch ............. leader ─► LiteSVM sim ─► RPC sim ─► Jito bundle / SWQoS RPC
```

## Single sources of truth (established + verified)

| Concern | Source of truth | Duplicates found |
|---|---|---|
| Pool state | `MultiDexRegistry` → `PoolState` cache | none live (lib.rs:729 `apply_raw_account` DEAD, 0 callers) |
| Price/quote | `arb-math` (cpmm/clmm/dlmm) via `arb-engine::adapters` | none live |
| Fees | on-chain decode (`decode_*_fee_bps`) | catalog `fee_bps` fallback only where decode impossible (documented) |
| Orientation | on-chain bytes (DLMM token_x/y @88/@120; CLMM `clmm_onchain_mints`) | catalog trusted only when bytes unavailable (fail-open edge, P2, documented) |
| Tx construction | `arb-tx/src/primitives/builder.rs` | DEAD: `meteora_atomic_builder`, `kamino_flash`, `cross_dex_flash_arb` (tests-only), `dlmm_4leg_flash_arb` (tests-only) |
| Profit | chained `amount_out` + explicit cost stack (guarded.rs) | none (`fee_paid` reporting fixed in `152ad88`) |
| Execution status | dispatch.rs explicit state transitions | `jito_lander`/`observe_bundle_rtt` are wired (dispatch.rs:582/592); the clippy dead-code warning fires only under `--all-features` because `shadow-only` compiles the broadcast path out — by design |

## Cache writers (exhaustively enumerated)

run.rs:385/451 (registry fetch) · helius_ws.rs:756/807/842 (3 fail-closed WS cases) ·
pump_state.rs:684 (PumpFun only) · lib.rs:729 `apply_raw_account` (**DEAD — 0 callers, removal candidate**).
Graph rebuilt fresh each tick via `build_from_pools` — no stale graph reuse.

## Duplicated provider calls (fixed this cycle, evidence)

- Pre-`228c330`: ~430 sequential `getAccountInfo` fetches/cycle → 429 storms (soak4: 39,519 WARNs).
- Post: `batch_fetch_accounts` batch-100 `getMultipleAccounts` via shared RpcPool (soak16: 9 429s/3min, correct rotation).

## Circular dependencies

None. Cargo workspace builds as a DAG; `cargo check --workspace --all-targets --all-features` clean.

## Known architectural debts (not fixed — prioritized in backlog)

1. **P2** CLMM/Orca catalog orientation falls back to catalog when pool bytes are short
   (registry.rs canonical block `None =>` arm) — same silent class that bit DLMM, but
   requires a sub-213-byte Orca account to trigger (never observed live).
2. **P3** `pairs` degrades during rediscovery cycles (163→91) — decimals cache added,
   effect unverified by long soak.
3. **P4** 17 files > 1000 lines (list in BASELINE.md).
