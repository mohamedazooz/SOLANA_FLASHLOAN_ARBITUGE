# 01 — Engine Architecture (reconstructed, not trusted)

Source of truth: `audit_work/01_ARCHITECTURE_INVENTORY.md` (all claims carry `file:line`).
This document reconstructs the real architecture from source. No project-authored report was
accepted as evidence.

## 1. Module dependency graph (crate level)

```
arb-types     → arb-constants
arb-constants → (none)
arb-math      → arb-types
arb-dex       → arb-types, arb-math, arb-constants
arb-metrics   → (none)
arb-risk      → arb-types, arb-constants
arb-store     → arb-types
arb-land      → arb-types, arb-constants, arb-math
arb-tx        → arb-types, arb-constants
arb-ingest    → arb-types, arb-dex, arb-math, arb-constants, arb-metrics (+ unused optional `solami 0.1.58`)
arb-engine    → arb-constants, arb-types, arb-math, arb-dex, arb-ingest, arb-tx (dev: arb-sim)
arb-sim       → arb-types, arb-engine, arb-ingest, arb-tx (optional litesvm; dev: arb-constants, arb-math)
arb-execution → arb-types, arb-tx, arb-constants          ← ORPHAN (no dependent)
arb-mcp       → arb-types, arb-constants, arb-dex, arb-ingest ← ORPHAN (no dependent)
arb-bot       → all of the above except arb-mcp / arb-execution
```
Workspace root: `members = ["crates/*"]`, excludes `webhook-service`, `solami-probe`, `helius-probe`.
149 `.rs` files, ~2.96 MB, 78,558 lines (src+tests+benches).

## 2. Runtime data flow (ingestion → execution)

```
Yellowstone gRPC (arb-ingest/src/yellowstone.rs:79)  ─┐
Helius WS (arb-ingest/src/helius_ws.rs:381)          ├─► AccountSnapshotStore (lib.rs:101/124)
Registry poller (registry.rs:596 fetch_live_pool_states) ► MarketStateCache (lib.rs:92/547)
        │                                              │
        ▼                                              ▼
  RegisteredPool → PoolState (decode_* + compute_*_reserves; execution_accounts built registry.rs:988-1113)
        │
        ▼  (supervisor tick loop arb-bot/src/supervisor/run.rs:15)
  MarketGraph::build_from_pools (arb-engine/src/lib.rs:419)
    find_base_cycles (:565) → optimal_two_pool/three_pool (arb-math/src/cpmm.rs:244/307)
    find_cross_pool_dlmm_cycles (cross_pool_dlmm.rs:459; run.rs:526)
    BellmanFordEngine::find_negative_cycles (bellman_ford.rs:229; run.rs:587 — only if DFS empty)
    find_multi_hop_cycles (:969; run.rs:600 — only if still empty)
        ▼
  Gates (run.rs:615-1099): slot refresh → tip ceiling (t_max) → Chronos EV gate → RiskGate
        ▼
  Tx assembly: execution.rs:8 assemble_native_ixs → arb-tx lib.rs build_swap_hop_instruction (:2005)
    → assemble_flash_arbitrage (lib.rs:3061) — OR assemble_arbitrage (:2867)
    → Sign: Tier-1 RAM template patch (template.rs:269) → Tier-2 Redis (run.rs:1322) → Tier-3 build_signed_transaction (run.rs:1355; lib.rs:3471 → compile_message :3347, LEGACY)
        ▼
  dispatch_opportunity (dispatch.rs:44):
    LiteSVM sim :201 → RPC simulateTransaction :237 (fail-over at :278; fail-closed :328)
    → SlotClock::schedule :354 → validate_execution_readiness :369 (weak gate; the strong one is unused)
    → SWQoS send_raw_transaction :408  OR  Jito send_bundle_multi_region :420 (arb-land/src/lib.rs:319)
    → async landing tracker :501 → verify_landing (jito_service.rs:425)
```

SIMULATE/SHADOW/CANARY modes never send (dispatch.rs:78-125).

## 3. Execution engines and their real roles

| Engine | Code | Status in production |
|---|---|---|
| Native flash-loan engine | `assemble_flash_arbitrage` (arb-tx lib.rs:3061) | **The live path** (via execution.rs:25/42/93 → run.rs) |
| Native non-flash engine | `assemble_arbitrage` (:2867) | Reachable (non-flash routes) |
| Jupiter v6 route | `assemble_jupiter_flash_arbitrage` (:3228) + `jupiter.rs` HTTP client | **Disabled** (`route_via_jupiter = false`, run.rs:1110) |
| Split route | `assemble_split_flash_arbitrage` (:3284) | Disabled (`SplitAllocation = None`, run.rs:1009) |
| 3-leg meme pipeline | `three_leg_meme_pipeline.rs` + `compiler::compile_3leg_meme_dlmm_arb` (compiler.rs:372) | Tests only |
| Meteora atomic 2-swap | `meteora_atomic_builder.rs` | Used by arb-sim / tests only |
| `arb-execution` crate (v0/v1/merger) | arb-execution/src/* | **Orphan — zero dependents** |
| `AdaptiveTxCompiler`, `compile_v0_message`, `compiler.rs`, master-ALT provisioning | arb-tx | Tests only |
| `arb-mcp` MCP server | arb-mcp | Orphan CLI, unused by bot |

## 4. Call graph — route evaluation vs execution

Route evaluation (runtime): `run.rs` → `MarketGraph::{build_from_pools, find_base_cycles,
find_cross_pool_dlmm_cycles, find_multi_hop_cycles}` → `quote_pool` (arb-engine/src/lib.rs:68)
→ `quote_cpmm`/`quote_clmm`/`quote_dlmm_view` (arb-math) → `make_economic_opportunity` (:345)
→ `EconomicQuote` (economic.rs:274). Golden-section sizing (`golden_section_size`, :245) only on
the DFS-fallback; CPMM 2/3-hop uses closed forms; DLMM cross-pool uses `cross_pool_dlmm::optimize_size` (:235).

Execution: `run.rs` → `assemble_*` (arb-tx) → instruction builders per protocol
(`build_swap_hop_instruction` dispatch at lib.rs:2005) → legacy signer → `dispatch.rs` → sim →
send (Jito/SWQoS) → landing tracker. **No call from arb-bot to `compiler.rs`,
`AdaptiveTxCompiler`, `compile_v0_message`, `StaticMasterAlt`, `v0_compiler`, `v1_fallback`, or
`arb_execution` exists** (grep-verified).

## 5. Inventories (counts and locations)

### 5.1 Mathematical functions — arb-math (~120 pub fns)
- CPMM/sizing (cpmm.rs): `cpmm_out` :67, `cpmm_out_fee` :73, `quote_cpmm(_full)` :104/:117,
  `cpmm_price_impact_bps` :139, `isqrt_*`, `optimal_two/three/four_pool` :244/:307/:379,
  `clamp_at_marginal_ceiling_two_pool` :557, `marginal_profit_derivative*`
- CLMM (clmm.rs): `compute_swap_step(_checked)` :444/:592, `compute_swap(_with_ticks_per_array)` :869/:894,
  `quote_clmm(_with_ticks_per_array)` :995/:1020, tick↔price conversions, `parse_tick_array_account` :607
- DLMM (dlmm.rs): `get_price_from_id` :279, `pow_q64` :223, `compute_base_fee` :372,
  `compute_variable_fee` :395, `compute_total_fee_rate` :425, `consume_bin_liquidity` :649,
  `quote_dlmm_multi_bin` :805, `quote_dlmm_view` :904, `bin_array_index_for_bin` :574,
  `dlmm_active_bin_max_input` :303, `scale_dlmm_three_hop_input` :339
- Economic (economic.rs): `CostLedger` :89-123 (implemented, **unused at runtime**), `build_economic_quote` :274,
  `calculate_net_profit_raw` :399, `calculate_flash_fee_raw` :460, `base_raw_to_lamports` :244
- Root (lib.rs): `apply_transfer_fee_ceil` :69, `apply_transfer_fee` :80, `calculate_net_profit` :103, `pick_tip` :85
- Latency: `RttTracker`, `SlotClock`, Chronos (arb-risk integration)

### 5.2 Account resolvers (PDA derivations) — representative list
`derive_meteora_bin_array_pda` (arb-tx/lib.rs:1023), `derive_meteora_bin_array_bitmap_extension_pda` (:1043),
`derive_meteora_reserve_pda` (:1014), `derive_meteora_event_authority` (:1036), `bin_array_index_for_bin` (:1084),
`derive_orca_tick_array_pda` (:813), `derive_raydium_cpmm_{vault,authority,observation}_pda` (:1383/:1394/:1400),
`derive_raydium_clmm_{tick_array,observation,vault}_pda` (:1557/:1569/:1579), `derive_pump_fun_*_pda` (:1675-1692),
`get_associated_token_address` (:442, **SPL-only derivation**), `derive_ata` (master_alt.rs:725),
`derive_lending_market_authority` (kamino.rs:23), `derive_lookup_table_address` (alt.rs:25).
Ingest duplicates: `dlmm_bin_array_pdas` (registry.rs:1508), `derive_meteora_bin_array_pda` (:1842),
`bin_id_to_bin_array_index` (:1853), `raydium_clmm_tick_array_pdas` (:1476). **No cross-check test ties these together.**

### 5.3 Instruction builders — arb-tx/lib.rs
Swap dispatch `build_swap_hop_instruction` (:2005): PumpSwap v2 (:2006), PumpSwap legacy (:1903),
RaydiumV4 v2/V1-legacy/versioned (:732/:760/:667/:821), Orca Whirlpool 11-account (:947),
Meteora DLMM swap2 (`build_swap2_exact_in/out` meteora_dlmm.rs:364/:406), RaydiumCpmm (:1410),
RaydiumClmm (:1586), PumpFun (:1693), MeteoraDammV2 inline (:2447), LaunchLab inline (:2479).
Utility: `build_create_ata_idempotent(_for_program)` (:464/:490), `build_close_token_account(_for_program)` (:516/:534),
`build_sync_native` (:553), `build_system_transfer` (:563), `build_compute_unit_{limit,price}` (:215/:227),
`build_kamino_flash_{borrow,repay(_with_index)}` (:412/:422/:431), `build_jito_tip_instruction` / `build_marked_jito_tip_instruction` (:244/:292).

### 5.4 Transaction compilers
Runtime: `TxBuilder::build_signed_transaction` (lib.rs:3471) → `compile_message` (:3347, **legacy only**).
`PreWarmedTxTemplate` Tier-1/2 (template.rs:197/269). Test/standalone only: `compile_v0_message` (:3488),
`build_signed_v0_transaction` (:3656), `compile_3leg_meme_dlmm_arb` (compiler.rs:372), `AdaptiveTxCompiler`
(adaptive.rs:678), `dispatch_two_dex_arbitrage` (arb-execution/src/lib.rs:153), `compile_v1_fallback` (v1_fallback.rs:62),
`compile_v0/compile_v1` (arb-execution).

### 5.5 Wallet / token-account operations
Payer keypair from env/file (supervisor.rs:293-331); `payer` at run.rs:301. All swap ATAs are derived with the
**SPL Token program only** (lib.rs:442-448) — including Token-2022 mints (hot path). PumpSwap and the Meteora
standalone builder use token-program-aware derivation (lib.rs:1828-1857, meteora_atomic_builder.rs:177-182).
ATA create-idempotent, close, sync-native, WSOL wrap/unwrap — see §5.3.

### 5.6 Fallback / default / error paths (counts `unwrap_or(` / `unwrap_or_default`)
arb-ingest 91/52 · arb-tx 26/48 · arb-dex 15/18 · arb-bot 21/16 · arb-engine 15/6 · arb-sim 20/18.
Notable: Postgres→SQLite silent storage downgrade (supervisor.rs:243-289); RPC simulator URL fallback;
Tier-1→2→3 cascade; `FAST_PATH_*` / `BYPASS_RISK_GATE` / `BYPASS_CHRONOS_GATE` env escapes; `BYPASS_RISK_GATE`
bypasses everything except the kill switch (run.rs:1024-1038). See `01_ARCHITECTURE_INVENTORY.md` §5f.

## 6. Dead / test-only code (proof that tests ≠ production)
`guarded.rs`, `route_worker.rs`, `three_leg_meme_pipeline.rs`, `meteora_quote_engine.rs`, `meteora_size_optimizer.rs`
(replay bin + tests only), `meteora_pair_discovery.rs`, `meteora_fresh_state.rs`, `solana-message` differential tests,
`arb-execution/*`, `arb-mcp`, `arb-tx::compiler`, `AdaptiveTxCompiler`, master-ALT provisioning (`build_or_extend_master_alt_ixs`),
`estimate_minimal_cu`, `calculate_dynamic_cu_price` (tests-only; the live path uses `adaptive_cu_price`),
`calculate_fee_deduplicated_cu_price` (never called — bundle path pays both priority fee and tip),
`check_tip_lamports` (defined in arb-risk, never called), `simulateBundle` (never called).
**Consequence: the only code paths that can build a size-reduced (ALT-carrying) transaction are exercised
exclusively by tests; the code that runs in production cannot emit them.**

## 7. Missing tests (high-level; see per-section audits)
- Cross-implementation equality of `bin_array_index_for_bin` copies (arb-math vs arb-tx vs arb-dex vs arb-sim).
- On-wire ALT index resolution against a real table (only `#[ignore]`d test does it).
- `assemble_flash_arbitrage` end-to-end (never simulated; guaranteed-revert shapes undiscovered by CI).
- Flash-fee, transfer-fee, Token-2022 interplay (all fixtures sf=0 / bps=0).
- Multi-pool coherent-snapshot behavior of the *live* gate.
