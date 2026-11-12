# arb-* Workspace — Architecture Inventory (READ-ONLY audit)

Scope: all 15 crates under `D:\work_new\crates\*`. No code changes, no builds.
Totals: 149 `.rs` files, ~2.96 MB, 78,558 lines (src + tests + benches). Line counts
per crate: arb-bot 17 files/9,269 · arb-constants 1/463 · arb-dex 12/4,248 · arb-engine 18/8,956
· arb-execution 5/2,331 · arb-ingest 27/14,066 · arb-land 5/2,545 · arb-math 18/9,273
· arb-mcp 3/345 · arb-metrics 1/342 · arb-risk 1/914 · arb-sim 10/3,632 · arb-store 3/1,046
· arb-tx 24/18,130 · arb-types 4/2,998.

## 1. Crate → crate dependency graph (from each `Cargo.toml` `[dependencies]`)

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
arb-ingest    → arb-types, arb-dex, arb-math, arb-constants, arb-metrics (+ optional `solami 0.1.58`, never used)
arb-engine    → arb-constants, arb-types, arb-math, arb-dex, arb-ingest, arb-tx (dev-dep: arb-sim)
arb-sim       → arb-types, arb-engine, arb-ingest, arb-tx (optional litesvm) (dev-deps: arb-constants, arb-math)
arb-execution → arb-types, arb-tx, arb-constants
arb-mcp       → arb-types, arb-constants, arb-dex, arb-ingest
arb-bot       → arb-types, arb-math, arb-dex, arb-engine, arb-ingest, arb-tx, arb-land,
                arb-risk, arb-constants, arb-store, arb-metrics, arb-sim   (NOT arb-mcp, NOT arb-execution)
```
External/optional gaps: `solami` is declared optional in arb-ingest but unused (`solami::` has zero
references). `arb-execution` and `arb-mcp` are referenced by no workspace `Cargo.toml` — both are
standalone/orphan crates. Workspace root excludes `webhook-service`, `solami-probe`, `helius-probe`.

## 2. Module-level map (src only)

| Crate | Modules (file) | Notes |
|---|---|---|
| arb-types (`src/lib.rs:3-11`) | `currency`, `dimensional`; root re-exports `currency::*` | Newtypes Lamports/TokenAmount/Bps/Slot/ComputeUnits (`lib.rs:17-127`), `PoolId`/`MintId`/`PoolKind`/`PoolState`/`PoolSnapshot`/`SizedOpportunity`, `PoolExecutionAccounts` (`lib.rs:360-439`), `SwapHop` (`lib.rs:1035`), `ExecutionOpportunity` (`lib.rs:1098`), Tip market types `Percentile`/`TipFloor`/`Posterior`/`TipDecision`/`TipPlan`/`InflightStatus` (`lib.rs:1131-1440`), `ArbError` (`lib.rs:1447`) |
| arb-constants | single `lib.rs` (463 lines) | Program IDs, tip accounts, discriminators, SOL/USDC/Kamino/Meteora mainnet addresses, `DEFAULT_HOP_SLIPPAGE_BPS=10` (`lib.rs:352`), `MAX_COMPUTE_UNIT_LIMIT=350_000` (`lib.rs:363`), `SOLANA_PACKET_MTU_LIMIT=1232` (`lib.rs:367`) |
| arb-math (`src/lib.rs:13-21`) | `chronos`, `clmm`, `cpmm`, `damm_v2`, `dlmm`, `economic`, `latency`, `launchlab`, `pump_fun_raydium`; root adds `apply_transfer_fee*` (`lib.rs:69,80`), `pick_tip` (`lib.rs:85`), `calculate_net_profit` (`lib.rs:103`), cfg(test) invariant checks (`lib.rs:124,137`) | |
| arb-dex | `traits` (AmmPool), `raydium_v4`, `raydium_cpmm`, `raydium_clmm`, `raydium_launchlab`, `orca_whirlpool`, `meteora_dlmm`, `meteora_damm_v2`, `pump_fun`, `pump_swap`; root decoders `decode_transfer_fee_config`/`calculate_transfer_fee`/`active_transfer_fee`/`token2022_has_risky_extension`/`decode_amm_pool`/`program_id_to_protocol` (`lib.rs:42-241`) | No instruction builders — decode/quote only |
| arb-engine (`src/lib.rs:5-41`) | `route_worker`, `adapters`, `guarded`, `bellman_ford`, `cross_pool_dlmm`, `meteora_pair_discovery`, `meteora_quote_engine`, `meteora_fresh_state`, `meteora_size_optimizer`, `three_leg_meme_pipeline` | Zero I/O/async (header `src/lib.rs:2`) |
| arb-tx | `alt`, `master_alt`, `meteora_dlmm`, `compiler`, `adaptive`, `meteora_atomic_builder`, `error`, `jupiter`, `kamino`, `pump_fun_raydium`, `template`; 4,686-line root `lib.rs` | Instruction/tx builders, wallet, templates |
| arb-ingest | 27 files: `lib` (YellowstoneClient, MarketStateCache, AccountSnapshotStore), `yellowstone`, `helius_ws`, `solami_stream`, `dual_ingest`, `helius_ws`, `registry`, `discovery`, `rpc_pool`, `rpc_reconcile`, `slot_integrity`, `provider_supervisor`, `dependency_manifest`, `quote_readiness`, `pump_state`, `pumpfun_discovery`, `pumpdev_stream`, `bitquery`, `dexscreener`, `gmgn`, `pump_state_tests` | Mix of live producers + unused probes |
| arb-land | `bidder`, `leader`, `ledger`, `tip_oracle`, `lib` (JitoLander, TipStrategy, LandStatus, bundle auction) | |
| arb-risk | single `lib.rs` | `CircuitBreaker`, `KillSwitch`, `RiskGate`, `ChronosEv*` |
| arb-sim | `fork_sim`, `litesvm_sim`, `lib` (`Simulator`, `shadow_check`), `meteora_fresh_pipeline`, `meteora_simulation_harness`, `opportunity_recorder`, `profitability_gate` | |
| arb-store | `lib` (`SqliteStore`, `Store` trait, `AttemptRecord`), `postgres_store`, `redis_cache` | |
| arb-metrics | single `lib.rs` | Prometheus counters, global atomics |
| arb-execution | `lib` (`DispatcherConfig`, `dispatch_two_dex_arbitrage`), `merger`, `sizing`, `v0_compiler`, `v1_fallback` | Standalone, unused by other crates |
| arb-mcp | `lib` (`pub mod tools`), `main` (stdio JSON-RPC), `tools` | MCP server binary, unused by bot |
| arb-bot | `main`, `supervisor` + `supervisor/{run,dispatch,execution}`, `jito_service`, `verified_config`, `health_monitor`, `heartbeat`, `telegram`, `errors`; 6 bins (`bin/alchemy_webhook.rs`, `discover.rs`, `latency_bench.rs`, `live_scanner.rs`, `replay.rs`, `validate.rs`) | Runtime binary `arb-bot` |

## 3. Data-flow diagram (ingest → engine → tx → execution), runtime path

```
HELIUS LASERSTREAM (yellowstone geyser gRPC, x-token)      HELIUS_WS (mainnet WS JSON-RPC)
  arb-ingest/src/yellowstone.rs:79 start_stream              arb-ingest/src/helius_ws.rs:381 start_stream
        │  SubscribeUpdateAccount                                    │  accountNotification
        ▼                                                          ▼
  arb-ingest/src/lib.rs:743 decode_and_apply_yellowstone_account / :850 decode_yellowstone_account
        │                                                          │
        ▼                                                          ▼
  AccountSnapshotStore (lib.rs:101, apply :124)  ◄──►  MarketStateCache (lib.rs:92, apply_pool_update :547)
        ▲                                                          ▲   Snapshot consumers: snapshot_pools (:309)
        │                                                          │
  arb-ingest/src/registry.rs:596 fetch_live_pool_states ──► RegisteredPool ──► apply_pool_update
  (spawned "registry_worker" by supervisor supervisor/run.rs:208-247; seeded once at :157)
        ▲
  PoolDiscovery::discover (discovery.rs) ← DEXscreener/GMGN/pumpfun scrapers (dexscreener.rs, gmgn.rs,
  pumpfun_discovery.rs — used for seeding/watch set, not hot path)

  MultiDexRegistry (registry.rs:447 mainnet_default) — decodes chain accounts into PoolState:
    decode_pool_info :1217, decode_clmm_state :1727, decode_dlmm_state :1782, resolve_fee_bps :1890,
    resolve_mints_decimals :1394, PoolExecutionAccounts built at :988-1113 (RaydiumCpmm/Clmm/Orca/
    MeteoraDlmm/RaydiumV4), PUMP_* at pump_state.rs (decode_pump_swap_global_config etc.)

        │  RPC fallback pollers (rpc_pool.rs:43 new / :62 from_env; batch-100 getMultipleAccounts)
        ▼
  Supervisor tick loop (arb-bot/src/supervisor/run.rs:15 run()):
    3. build_verified_inputs (execution.rs:155) → VerifiedQuoteInputs (verified_config.rs:44 build_verified_inputs_from_snapshots :171)
    2. pools = self.ingest.snapshot().snapshot_pools() (run.rs:445); fresh filter :494
    3. MarketGraph::build_from_pools (engine lib.rs:419); economic_config injected :430
       find_base_cycles (lib.rs:565)         ← primary 2-/3-hop CPMM/CLMM/DLMM search
       find_cross_pool_dlmm_cycles (engine cross_pool_dlmm.rs:459; cfg from run.rs:526)
       BellmanFordEngine::find_negative_cycles (bellman_ford.rs:229; run.rs:587, only if opps empty)
       find_multi_hop_cycles (lib.rs:969, run.rs:600, only if still empty)
        │
        ▼  Vec<ExecutionOpportunity>
    4. per-opp gates (run.rs:615-1099): refresh slot→target_slot :618 → jito_service.tip_account_for_slot :619
       → per-route cost basis :624-679 → t_max :686 → jito_service.plan_with_context :723
       → real_net_base_raw = arb_math::economic::calculate_net_profit_raw :805 → min_net_base_raw :853
       → Chronos EV gate :957 (arb_risk::evaluate_chronos_gate :974) → max_state_age_slots (execution.rs:292)
       → flash-loan/RiskGate evaluate_flash_loan/evaluate_full (:1047/:1058) or BYPASS_RISK_GATE :1033
       → economic invariants validate_economic_invariants :1583/:1663
        │
        ▼
    5. Tx assembly: assemble_native_ixs (execution.rs:8) → TxBuilder::assemble_flash_arbitrage
       (arb-tx lib.rs:3061) or assemble_arbitrage (:2867) — or split path (split=None, run.rs:1009)
       or Jupiter path (route_via_jupiter=false, run.rs:1110)
    6. Sign/patch: blockhash fetch :1277; ram_templates::patch_and_sign_with_swaps :1306 (tier-1);
       Redis CachedTemplate fallback :1322; TxBuilder::build_signed_transaction :1355 (tier-3);
       PreWarmedTxTemplate::from_precompiled_bytes :1327/:1367
    7. dispatch_opportunity (dispatch.rs:44) — Live mode only:
       balance check :133, MTU :170, simulate_local (arb-sim) :201, RPC simulate_async :237,
       RpcPool::simulate_transaction fallback :278, SlotClock::schedule :354, fire-time stream-readiness
       guard arb_ingest::validate_execution_readiness :369, then either SWQoS direct
       rpc_pool.send_raw_transaction :408 or Jito bundle jito_lander.send_bundle_multi_region :420
       (arb-land lib.rs:319) → spawn landing tracker :501 → verify_landing (jito_service.rs:425)
       → arb-land LandStatus (arb-land/src/lib.rs:61) → record_outcome/metrics/telegram

SIMULATE/SHADOW/CANARY modes: no send; arb_metrics::add_simulated_net_profit (dispatch.rs:87/106).
```

## 4. Call graph — route evaluation & execution path

### 4a. arb-engine::lib route evaluation calls into arb-math / arb-dex / arb-ingest
- `quote_pool` (arb-engine/src/lib.rs:68) — CPMM arm → `arb_math::cpmm_out` (:77); CLMM arm →
  `arb_math::clmm::quote_clmm` (:112) + `arb_math::clmm::TickArrayFacade` (:98);
  DLMM arm → `arb_math::dlmm::quote_dlmm_view` (:177); no fallback on CLMM/DLMM (:127-129, :184-186).
- `cycle_profit_at` (:190) → `quote_route` (adapters.rs:250) → `quote_hop` (adapters.rs:186)
  → `quote_pump_fun_hop`→`arb_dex::PumpFunBondingCurve::quote_with_fee` (adapters.rs:326/335),
  `quote_cpmm_hop` (adapters.rs:407), `quote_clmm_hop`→`arb_math::clmm::compute_swap` (adapters.rs:484),
  `quote_dlmm_hop`→`arb_math::dlmm::quote_dlmm_multi_bin` (adapters.rs:581). `cu_estimate`/`accounts_required` (adapters.rs:77/92).
- `golden_section_size` (:245) → `cycle_profit_at`.
- `find_base_cycles` (:565) → `optimal_two_pool`/`optimal_three_pool` (arb-math cpmm.rs:244/307),
  `quote_pool`, `cpmm_price_impact_bps` (arb-math cpmm.rs:139), `edge_max_input_capacity` (:488,
  calls `arb_math::clmm::clmm_active_tick_max_input` :538, `arb_math::dlmm::dlmm_active_bin_max_input` :552),
  `arb_math::dlmm::scale_dlmm_three_hop_input` (:797), `make_economic_opportunity` (:345, calls
  `arb_math::economic::{calculate_flash_fee_raw, build_economic_quote, base_raw_to_lamports}` :358/365/391).
- `find_multi_hop_cycles` (:969) → `dfs_multi_hop` (:1003) → `evaluate_arbitrage_cycle` (:1070)
  → `quote_pool`, `golden_section_size`, `cycle_profit_at`, `arb_math::cpmm::clamp_at_marginal_ceiling_two_pool` (:1131).
- `build_from_pools` (:419), `edge_max_input_capacity` (:488) — pure over `PoolState`.
- `BellmanFordEngine::find_negative_cycles` (bellman_ford.rs:229) → `optimal_two/three/four_pool`,
  `clamp_at_marginal_ceiling_two_pool`, `arb_math::economic::*` (bellman_ford.rs:25-32). Builds
  `ArbitrageCycle` and emits `ExecutionOpportunity` (:453).
- `CrossPoolScan::find_cross_pool_dlmm_cycles` (cross_pool_dlmm.rs:459) → `arb_math::cpmm_price_impact_bps`
  (cross_pool_dlmm.rs:25) + `quote_pool`; requires `execution_accounts` on both pools (:393).
- NOT in runtime path: `route_worker` (RouteArchitectWorker, bins only), `guarded::quote_route_guarded`
  (guarded.rs:54, no arb-bot callers), `three_leg_meme_pipeline` (arb-tx compiler used only by
  `engine/tests/three_leg_meme_pipeline_tests.rs`), `meteora_quote_engine`, `meteora_size_optimizer`
  (used by `replay.rs` bin), `meteora_pair_discovery`, `meteora_fresh_state`.

### 4b. Runtime path: arb-bot calls into engine/tx/jito
arb-bot `supervisor/run.rs` →
  engine: `MarketGraph::{build_from_pools, find_base_cycles, find_multi_hop_cycles}` (:511/:512/:600),
          `graph.find_cross_pool_dlmm_cycles` (:535), `arb_engine::BellmanFordEngine::from_pools` (:587),
          `arb_engine::DlmmCrossPoolConfig` (:526), `arb_engine::SplitAllocation` (always `None`, :1009);
          bins use `find_base_cycles` (replay.rs:307, validate.rs:247).
  arb-tx: `estimate_route_cu` (:646), `calculate_dynamic_cu_limit` (:654), `cap_cu_price_by_profit` (:656),
          `build_swap_hop_instruction` (:1135/:1174), `TxBuilder::assemble_split_flash_arbitrage` (:1184),
          `build_swap_hop_instruction` per hop, `PreWarmedTxTemplate::from_precompiled_bytes` (:1327/:1367),
          `TxBuilder::build_signed_transaction` (:1355), `TxBuilder::serialize_instructions` (:1385),
          `TxBuilder::{assemble_flash_arbitrage, assemble_arbitrage, assemble_jupiter_flash_arbitrage}` via execution.rs:25/42/93,
          `estimate_minimal_cu` (tx lib.rs:88, unused at runtime).
  arb-tx instruction builders reached from `build_swap_hop_instruction` (tx lib.rs:1980, match at :2005):
    PumpSwap (:2006, → v2 `build_pump_swap_instruction_v2` :1803 or legacy `build_pump_swap_instruction` :1903),
    RaydiumV4 (:2099 → `build_raydium_v4_swap_instruction` :732 → v2 :821 / versioned :760 / v1_legacy :667),
    OrcaWhirlpool (:2181 → `build_orca_whirlpool_swap_instruction_11` :947),
    MeteoraDlmm (:2236 → `build_meteora_dlmm_swap_instruction` :1165 / `_with_bin_arrays` :1731),
    RaydiumCpmm (:2293 → `build_raydium_cpmm_swap_instruction` :1410),
    RaydiumClmm (:2336 → `build_raydium_clmm_swap_instruction` :1586),
    PumpFun (:2399 → `build_pump_fun_swap_instruction` :1693), MeteoraDammV2 (:2410, inline instr :2447-2472),
    RaydiumLaunchLab (:2479, inline).
  arb-tx wallet/token ops: `KeypairSigner` load (supervisor.rs:293), `build_create_ata_idempotent` (tx lib.rs:464),
    `build_close_token_account` (:534), `build_sync_native` (:553), `build_system_transfer` (:563),
    `get_associated_token_address` (:442), `build_marked_jito_tip_instruction` (assemble_flash_arbitrage :3215),
    template `patch_and_sign_with_swaps` (template.rs:269).
  arb-bot jito_service: `plan_with_context` (run.rs:723), `tip_account_for_slot` (:619),
    `leader_for` (dispatch.rs:72), `bids_for` (dispatch.rs:351), `mark/verify_landing` (:425),
    `record` (dispatch.rs:585/:638/:669), `run` (tx spawned in run.rs:53-55).
  arb-risk: `RiskGate::{evaluate_flash_loan, evaluate_full, evaluate_chronos_gate, record_order, record_bundle_sent,
    record_bundle_landed, on_failure, on_success, record_loss, maybe_reset_periodic, kill_switch}` — lines as
    listed in §7. arb-land: `JitoLander::send_bundle_multi_region` (dispatch.rs:420), `LandStatus` (dispatch.rs:514-665),
    `arb_land::get_random_tip_account` (execution.rs:95), `arb_land::MIN_JITO_TIP_LAMPORTS` (run.rs:690),
    `TipStrategy::adaptive_cu_price` (run.rs:655), `arb_land::DEFAULT_MAX_AGE_MS` (main.rs:213).
  arb-math direct from run.rs: `arb_math::economic::{calculate_flash_fee_raw :625/:1712, calculate_net_profit_raw :805}`,
    `arb_math::{SLOT_MS, RttTracker, SlotClock}` (:14), `arb_math::ChronosEv*` via arb-risk.

### 4c. arb-engine → arb-tx compiler
Only via `arb-engine/src/three_leg_meme_pipeline.rs:27-29` (imports `arb_tx::meteora_dlmm::{...}` and
`ThreeLegMemeArbitrageContext`). The actual `arb_tx::compile_3leg_meme_dlmm_arb` (compiler.rs:372) is invoked
by tests only (`crates/arb-engine/tests/three_leg_meme_pipeline_tests.rs:345`). No runtime call
from the supervisor to the compiler module; the supervisor emits `Vec<CompiledInstruction>` and signs directly.
`AdaptiveTxCompiler` (adaptive.rs:678 `compile_adaptive`), `lib.rs:3488 compile_v0_message`, `:3656 build_signed_v0_transaction`,
`CreateSLTP...` v1 path — used only by tests and the unused arb-execution crate. arb-execution's
`dispatch_two_dex_arbitrage` (arb-execution/src/lib.rs:153) and `v1_fallback::compile_v1_fallback` (v1_fallback.rs:62) are unreachable from the bot.

## 5. Inventory by requested category

### 5a. Mathematical functions (`pub fn` in arb-math, grouped)
- CPMM/sizing (cpmm.rs): `cpmm_out_upfront_fee_ceil` :41, `cpmm_out` :67, `cpmm_out_fee` :73, `quote_cpmm` :104,
  `quote_cpmm_full` :117, `cpmm_price_impact_bps` :139, `cpmm_fee_paid`/`_with_denom` :173/:183,
  `isqrt_u384/u512/u768` :189/:204/:219, `optimal_two_pool` :244, `optimal_three_pool` :307, `optimal_four_pool` :379,
  `is_past_marginal_degradation_ceiling_two_pool` :455, `marginal_profit_derivative_cpmm` :503,
  `marginal_profit_derivative` :537, `clamp_at_marginal_ceiling_two_pool` :557.
- CLMM (clmm.rs): `tick_index_to_sqrt_price_x64` :110, `sqrt_price_x64_to_tick_index` :195,
  `get_amount_delta_a/b` :224/:260, `get_delta_x/y` :289/:376, `clmm_max_*`/`clmm_active_tick_max_input` :304-353,
  `get_next_sqrt_price_from_a_input/b_input` :389/:412, `compute_swap_step(_checked)` :444/:592,
  `parse_tick_array_account` :607, `parse_raydium_clmm_tick_array_account` :679,
  `find_next_tick_boundary(_with_size)` :744/:760, `update_liquidity_on_cross` :829, `compute_swap(_with_ticks_per_array)` :869/:894,
  `quote_clmm(_with_ticks_per_array)` :995/:1020, `clmm_max_input_in_tick` :336, `get_start_tick_index(_with_size)` :592/:597.
- DLMM (dlmm.rs): `pow_q64` :223, `get_price_from_id` :279, `dlmm_active_bin_max_input` :303, `scale_dlmm_three_hop_input` :339,
  `compute_base_fee/variable_fee/total_fee_rate` :372/:395/:425, `compute_fee_to_add` :439, `included_fee_amount` :464,
  `compute_fee_from_amount/protocol_fee` :488/:505, `update_references_with_decay`/`update_volatility_accumulator` :516/:539,
  `advance_active_id` :558, `bin_array_index_for_bin` :574, `consume_bin_liquidity` :649,
  `quote_dlmm_multi_bin` :805, `quote_dlmm_view` :904, `quote_dlmm_exact_out` :922, `dlmm_fee_on_input` :103.
- DAMM v2 (damm_v2.rs): `quote_damm_v2_exact_in(_with_fee_mode)` :75/:99, `damm_v2_virtual_reserves` :189, `DammV2FeeParams::effective_fee_numerator` :34.
- LaunchLab (launchlab.rs): `quote_launchlab_buy_exact_in(_ppm)` :14/:82, `quote_launchlab_sell_exact_in(_ppm)` :48/:118.
- Economic (economic.rs): `CostLedger::add/total/is_profitable` :100/:115/:191, `base_raw_to_lamports` :244,
  `build_economic_quote` :274, `calculate_net_profit_raw` :399, `calculate_flash_fee_raw` :460, `ECONOMIC_CALCULATION_VERSION`.
- Chronos (chronos.rs): `state_age_slots/quote_age_ms/pipeline_latency_ms` :58/:64/:70, `LatencyMetrics`/`LatencyWindow`,
  `VolatilityEstimator::*`, `chronos_edge_weight` :222, `compute_risk_adjusted_score` :407, `evaluate_gate` :504.
- Latency (latency.rs): `RttTracker::*` (:51-157), `plan_fire` :189, `SlotClock::new/observe_slot(_at)/now_ns/predict_window_start_ns/schedule/wait_duration` :243-339.
- pump_fun_raydium.rs: `calculate_optimal_trade(_integer)` :118/:132, `pump_fun_buy_cost` :54, `pump_fun_marginal_price` :44,
  `raydium_sell_output` :68, `verify_marginal_decreasing` :90, constants at `DEFAULT_FLASH_LOAN_FEE_BPS` etc. (`lib.rs:37-41`).
- Root (lib.rs): `apply_transfer_fee_ceil` :69, `apply_transfer_fee` :80, `pick_tip` :85, `calculate_net_profit` :103.

### 5b. Account resolvers (PDA derivations / chain-decode)
arb-tx: `derive_orca_tick_array_pda` (lib.rs:813), `derive_meteora_reserve_pda` :1016, `derive_meteora_bin_array_pda` :1026,
  `derive_meteora_event_authority` :1036, `derive_meteora_bin_array_bitmap_extension_pda` :1045, `derive_meteora_oracle_pda` :1074,
  `bin_array_index_for_bin` :1084, `derive_raydium_cpmm_vault_pda` :1383, `derive_raydium_cpmm_authority` :1394,
  `derive_raydium_cpmm_observation_pda` :1400, `derive_raydium_clmm_tick_array_pda` :1557, `derive_raydium_clmm_observation_pda` :1569,
  `derive_raydium_clmm_vault_pda` :1579, `derive_pump_fun_global_pda` :1675, `derive_pump_fun_bonding_curve_pda` :1681,
  `derive_pump_fun_event_authority` :1687, `derive_directional_bin_arrays` (meteora_dlmm.rs:239),
  `derive_bin_array_pda` (meteora_dlmm.rs:209), `derive_bin_array_bitmap_extension_pda` (meteora_dlmm.rs:217),
  `derive_lending_market_authority` (kamino.rs:23), `derive_lookup_table_address` (alt.rs:25), `derive_ata` (master_alt.rs:726),
  `meteora_bin_array_indices` (lib.rs:1059), `build_or_extend_master_alt_ixs` (master_alt.rs:748).
arb-ingest registry.rs: `fetch_live_pool_states` :596 (decodes pool accounts → PoolState with execution_accounts :988-1113),
  `resolve_mints_decimals` :1394, `raydium_clmm_tick_array_pdas` :1476, `derive_meteora_bin_array_pda` :1842,
  `bin_id_to_bin_array_index` :1853, `decode_pool_info` :1217, `decode_clmm_state`/`decode_raydium_clmm_state` :1727/:1752,
  `decode_dlmm_state` :1782, `decode_bin_array` :1859, `resolve_fee_bps` :1890, `compute_clmm_virtual_reserves` :1944,
  `compute_dlmm_reserves` :1986, `clmm_tick_array_covers_tick` :1959.
arb-ingest dependency_manifest.rs: `resolve_protocol_dependencies` :246 (manifest resolver, not wired to runtime fetch).
arb-dex: `decode_amm_pool`/`decode_transfer_fee_config`/`active_transfer_fee`/`token2022_has_risky_extension` (dex lib.rs:42-241);
  per-pool `from_account` constructors (`RaydiumCpmmPool::from_account`, `MeteoraDlmmPool::from_account`, `RaydiumClmmPool::from_account`, `pump_fun::*`).

### 5c. Instruction builders
arb-tx root (see §4b for the swap builders) plus: `build_compute_unit_limit` :215, `build_compute_unit_price` :227,
  `build_jito_tip_instruction` :244, `attach_jitodontfront` :274, `build_marked_jito_tip_instruction` :292, `ensure_bundle_mev_protection` :305,
  `build_create_ata_idempotent(_for_program)` :464/:490, `build_close_token_account(_for_program)` :516/:534,
  `build_sync_native` :553, `build_system_transfer` :563, `derive_swap_legs` :3002, `assemble_atomic_bundle` :2739,
  `build_kamino_bundle` :2820, `assemble_arbitrage` :2867, `assemble_flash_arbitrage` :3061,
  `assemble_jupiter_flash_arbitrage` :3228, `assemble_split_flash_arbitrage` :3284,
  `build_kamino_flash_borrow` :412 / `build_kamino_flash_repay(_with_index)` :422/:431, kamino.rs :85/:124.
arb-tx pump_fun_raydium.rs: `PumpFunRaydiumArbitrageBuilder::build_instructions` :164, `build_and_submit` :361.
arb-tx meteora_atomic_builder.rs: `build_atomic_two_swap(_with_traversal)` :89/:128, `extract_dlmm_accounts` :53.
arb-tx compiler.rs: `compile_3leg_meme_dlmm_arb` :372, `build_leg_swap_instructions` :483, `partition_accounts` :639.
arb-tx meteora_dlmm.rs: `build_swap2_exact_in(_with_remaining_accounts)` :364/:374, `build_swap2_exact_out(_with_remaining_accounts)` :406/:415.
arb-land: no builders (bundle auction only). arb-dex: no builders. arb-engine: no builders.

### 5d. Transaction compilers
Runtime: `TxBuilder::build_signed_transaction` (tx lib.rs:3472, legacy message via `compile_message` :3347 — always built, see supervisor/run.rs:1354),
`build_signed_v0_transaction` (:3656, test/CL only), `TxBuilder::compile_message` (:3347),
`PreWarmedTxTemplate::{from_precompiled_bytes:197, patch_and_sign_with_swaps:269, locate_offsets:80}` (template.rs).
Standalone/test-only: arb-execution `dispatch_two_dex_arbitrage` (arb-execution/src/lib.rs:153), `v0_compiler` (v0_compiler.rs:152),
`v1_fallback::compile_v1_fallback` (v1_fallback.rs:62), `merger.rs` (:313/:392); arb-tx `compiler::compile_3leg_meme_dlmm_arb`,
`adaptive::{compile_adaptive :678, compile_v1_message :383, build_signed_v1_transaction :558, build_signed_legacy_transaction :591, build_signed_v0_transaction :623}`,
`create_default_mainnet_alt` (:3679), `sign_versioned_transaction` (:3711), `compile_v0_message` (:3488),
`validate_jito_bundle` (:325), `encode_compact_u16` (:3332).

### 5e. Wallet / token-account operations
arb-tx: `KeypairSigner` (lib.rs:2556; `from_base58` :2571, `from_seed` :2598, `new_random` :2609,
  `from_bytes` :2614, `from_file` :2630, `pubkey` :2649, `sign` :2654, `verifying_key` :2660);
  `AddressLookupTable` (:2667), `VersionedTransaction` (:2694), `TxBuilder` (:2714), `extract_transaction_signature` (:318),
  ATA ops listed in §5c/§4b, `KaminoFlashParams` (:345), `KaminoFlashAccounts`/`to_flash_accounts` (:366/:379),
  `decode_reserve_flash_fee_sf` (kamino.rs:221).
arb-bot: keypair load from `SOLANA_PRIVATE_KEY`/file `solana_keypair.json` (supervisor.rs:293-331);
  `payer` derived at run.rs:301; MTU guard dispatch.rs:170; `recent_landed_cu` metrics.
arb-engine / arb-math: none (pure compute). arb-land: none. arb-ingest: read-only RPC only.

### 5f. Fallback / default / error paths (counts of `unwrap_or(` / `unwrap_or_default` / `fallback` / `default`, src only)
arb-ingest 91/52/21/145 · arb-tx 26/48/7/119 · arb-dex 15/18/2/74 · arb-bot 21/16/12/107 ·
arb-engine 15/6/2/35 · arb-math 24/0/1/23 · arb-sim 20/18/0/39 · arb-land 11/6/0/25 · arb-tx is included above ·
arb-store 6/4/0/12 · arb-types 3/0/2/82 · arb-risk 2/0/0/26 · arb-execution 1/2/8/5 · arb-mcp 4/4/0/3 · arb-metrics 0 · arb-constants 0.
Notable, cited:
- `Pubkey::from_str(WSOL_MINT).unwrap()` at tx lib.rs:3083-3084, `:1331` (after `.map_err(Serialization)`), run.rs:120-126, :1014-1015 (`unwrap_or_default` on parse); all are compile-time constants.
- Fail-closed guards (not defaults): zero blockhash → skip (run.rs:1280); flash fee `.unwrap_or(u64::MAX)` (run.rs:629);
  CLMM/DLMM quote returns 0 (engine lib.rs:128/:185); `cpmm_out` → 0 (lib.rs:77); `quote_route`/`quote_hop` `Err(InvalidInput)` on
  empty reserves/missing state (adapters.rs:188-209, :412, :452-465, :530-540); `fee_bps()` → `None` on zero denom (arb-types lib.rs:928-936).
- Explicit defaults: `RaydiumV4SwapVersion::V2` (types lib.rs:296-300), `PumpSwapPoolVersion::V2` :323,
  `PoolKind::Cpmm` :512, `#[serde(default)]` on `StrategyConfig::dlmm_cross_pool`/`graph_max_age_slots`/`max_per_venue` (main.rs:317-336),
  `ChronosConfig::default` (main.rs:225), `JitoConfig` tip-floor/leader-gate defaults (main.rs:266-292),
  `toml ... .unwrap_or_else(|| "shadow")` mode fallback (main.rs:116/141-145), `.env.example`/ARB_LIVE truthy-only live gate (main.rs:61, :670).
- Config fallbacks: Postgres→SQLite on connect failure (supervisor.rs:243-289, silently downgrades storage);
  keypair env→file→none (supervisor.rs:293-331); Yellowstone_endpoint helius→solami→empty with warn (supervisor.rs:197-231);
  `FAST_ENDPOINT`/`SOLANA_RPC_URL`/`HELIUS_RPC_URL` simulator URL fallback (supervisor.rs:337-343);
  Tier-1 RAM template → Tier-2 Redis CachedTemplate → Tier-3 full compile (run.rs:1302-1383);
  pre-flight RPC failure → RpcPool::simulate_transaction fallback (dispatch.rs:274-335, fail closed at :328-333);
  `ENABLE_PREFLIGHT_SIMULATION`/`FAST_PATH_*`/`BYPASS_RISK_GATE` env escapes (dispatch.rs:218-226/:340; run.rs:1513, :1024);
  stale SOL/USDC pool price → 0.0 → `live_sol_price()` None (supervisor.rs:446-447, :501);
  `lamports_to_base_raw_checked` error → fail closed (run.rs:853-888); `SOL_PRICE_FALLBACK` env (arb-ingest lib.rs:941 region);
  `SOLANA_WS_FALLBACK_*` / `SOLANA_RPC_FALLBACK_*` (rpc_pool.rs:84-85, :68-69); Shyft fallback (rpc_pool.rs:400);
  DexScreener trending fallback to stale snapshot (dexscreener.rs:173-208); tip_oracle TIP_FLOOR_URL (tip_oracle.rs:26) with no fresh snapshot → MarketUnavailable (types lib.rs:1329-1335);
  `TipFloor::at` interpolation (types lib.rs:1224-1244); `Posterior::PRIOR` Beta(1,1) (types lib.rs:1269);
  Vite: `Adaptive format SelectionFormat::v0 fallback v1` (adaptive.rs:200-207); `V1 fallback` (arb-execution v1_fallback.rs:61).
- Errors: `arb-types ArbError` (lib.rs:1447), `TxError` (tx error.rs:7), `ArbBot::errors::ArbError` (bot errors.rs:5),
  `DimensionalArithmeticError` (currency.rs:26).

## 6. Runtime path vs dead / test-only code

USED at runtime (bot main → Supervisor::run → dispatch):
- arb-bot: `main.rs` (BotMode, admin/dashboard servers), `supervisor.rs`, `supervisor/{run,dispatch,execution}.rs`,
  `jito_service.rs`, `health_monitor.rs`, `heartbeat.rs`, `telegram.rs`, `verified_config.rs`, `errors.rs`.
- arb-engine: `MarketGraph` (find_base_cycles / find_multi_hop_cycles / find_cross_pool_dlmm_cycles / build_from_pools),
  `BellmanFordEngine::find_negative_cycles` (only when DFS finds nothing, run.rs:586), `quote_pool`/`cycle_profit_at`/`golden_section_size`
  (internal), `adapters::quote_route/quote_hop`, `DlmmCrossPoolConfig`. NOT: `guarded`, `route_worker` (bins only),
  `three_leg_meme_pipeline`, `meteora_quote_engine`, `meteora_size_optimizer` (replay bin), `meteora_pair_discovery`,
  `meteora_fresh_state`, `cross_pool_dlmm` only via find_cross_pool_dlmm_cycles.
- arb-math: `cpmm` (all sizing), `clmm`/`dlmm` (quotes), `economic`, `latency::{RttTracker, SlotClock}`, `calculate_net_profit_raw`,
  `calculate_flash_fee_raw`. NOT at runtime: `chronos` module fns (used via arb-risk), `pump_fun_raydium` sizing (test/bench), `latency::plan_fire`,
  `damm_v2::quote_damm_v2_exact_in` (test/bench), `launchlab::*` (test only).
- arb-tx: `TxBuilder::build_signed_transaction`, `serialize_instructions`, `build_swap_hop_instruction`,
  `assemble_flash_arbitrage` (flash path), `estimate_route_cu`/`calculate_dynamic_cu_limit`/`cap_cu_price_by_profit`,
  ATA/close/sync/transfer builders, `KeypairSigner`, `PreWarmedTxTemplate` (patch path), `estimate_minimal_cu` unused,
  `build_signed_v0_transaction`, `AdaptiveTxCompiler`, `compiler::compile_3leg_meme_dlmm_arb`, `meteora_atomic_builder`,
  `pump_fun_raydium` — NOT at runtime. `build_signed_legacy/v0`/`V1` are test/standalone only.
- arb-ingest: `YellowstoneClient`, `MarketStateCache`, `AccountSnapshotStore`, `HeliusWsClient`, `MultiDexRegistry`,
  `PoolDiscovery`, `RpcPool`, `SlotIntegrity`, `quote_readiness::validate_execution_readiness`, `dependency_manifest` types
  (via lib re-exports), `pump_state` decode fns. NOT at runtime: `solami_stream` (never spawned), `dual_ingest` (bench/tests),
  `bitquery`, `pumpdev_stream`, `gmgn` (bins only), `dexscreener` (discovery fallback only), `pumpfun_discovery`, `provider_supervisor`
  (default-constructed only), `rpc_reconcile`, `dependency_manifest::resolve_protocol_dependencies` (not called by fetch path).
- arb-land: `JitoLander::send_bundle_multi_region`, `LandStatus`, `TipStrategy`, `Min_JITO_TIP_LAMPORTS`,
  `get_random_tip_account` (jupiter path only). `Bidder`/`LeaderSchedule`/`TipOracle` used via JitoService (`plan_with_context`).
- arb-risk: `RiskGate`, `KillSwitch`, `CircuitBreaker` (all via run.rs gates). arb-sim: `Simulator::{simulate_async, simulate_local}`,
  `ForkSimulator::from_env` (opt-in ENABLE_PREFLIGHT_SIMULATION). `profitability_gate`, `meteora_fresh_pipeline`,
  `meteora_simulation_harness`, `opportunity_recorder` — NOT at runtime.
- arb-store: `PostgresStore`/`SqliteStore`/`RedisStore` via `dyn Store` (supervisor.rs:419). arb-metrics: all atomics. arb-mcp, arb-execution: orphan crates, no workspace callers.
- Bins in arb-bot (`alchemy_webhook`, `discover`, `latency_bench`, `live_scanner`, `replay`, `validate`) are standalone and never linked into the `arb-bot` binary.

## 7. Public-surface counts & notes
- arb-math: ~120 `pub fn` across 9 modules (list in §5a). arb-tx: ~180 `pub fn` (§5c/5d). arb-engine: 60+ public items (§4a).
- Instruction builders per protocol: Raydium v4 (3 variants), CPMM, CLMM, Whirlpool (11-acct), Meteora DLMM (swap + swap2), Meteora DAMM v2 (inline), LaunchLab (inline), PumpFun (curve + PumpFunRaydium builder), PumpSwap (v1/v2 + exact-quote-in-v2).
- `arb-types` `RaydiumV4SwapVersion::V2` is the default; V1 legacy is opt-in (types lib.rs:285-309). `PumpSwapPoolVersion::V2` default; sell+ExactOutBase is rejected by type design (types lib.rs:347-355).
- Two `Cargo.lock`-referenced workspaces: root + `webhook-service/solami-probe/helius-probe` (excluded).
- `.mcp.json` in repo root contains a live-looking Helius API key (not reproduced here); outside crate scope, flagged for the parent session.
- `solami` optional dependency in arb-ingest has zero call sites; `abi-decoded` Yellowstone path is the only gRPC ingest wired, and it requires `GRPC_ENABLED=1` + endpoint or it runs in "Pure WebSocket" mode (run.rs:99-114).
- No build/test executed; no fmt/lint run (per instructions).
