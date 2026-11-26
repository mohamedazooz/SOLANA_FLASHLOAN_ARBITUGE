# End-to-End Runtime Call Graph

This document traces the complete execution flow through the codebase, from process startup in `crates/arb-bot/src/main.rs` through ingestion, candidate discovery, economic evaluation, transaction compilation, simulation, dispatch, and settlement reconciliation.

---

## 1. Startup & Initialization Call Flow

```
main() [crates/arb-bot/src/main.rs]
├── install_panic_hook()
├── dotenvy::dotenv()
├── parse_args() -> Args { config_path, mode }
├── init_event_logger() -> FileMakeWriter + spawn_log_retention()
├── config::load(&config_path) -> Config
├── arb_metrics::MetricsServer::new(&config.metrics.listen).start()
├── supervisor::Supervisor::with_mode_atomic(mode, config)
│   ├── IngestManager::new()
│   ├── MultiDexRegistry::new()
│   ├── RpcPool::new()
│   ├── MarketGraph::new()
│   ├── RiskGate::new()
│   ├── Simulator::new()
│   └── JitoService::new()
├── spawn(admin_http_server) [/health, /status, /pause, /resume]
├── attach_control(ControlService)
├── spawn(web_api::serve) [React UI + authenticated REST/WS API]
├── spawn(telegram_control::run_listener)
└── supervisor.run().await
```

---

## 2. Ingestion & Market Data Plane

```
Supervisor::run() [crates/arb-bot/src/supervisor/run.rs]
├── capability_registry.validate_execution_prerequisites()
├── reconcile_pending_dispatches().await
├── lander.verify_tip_accounts()
├── spawn(heartbeat_writer) [atomic 1s liveness refresh]
├── spawn(jito_service.run())
├── discover_pools() [if http_discovery_enabled]
├── spawn(maintenance_sampler) [CPU/RSS via /proc]
├── spawn(store_pruner) [retention cleanup]
├── HeliusWsClient::from_env().start()
│   ├── connect_and_stream()
│   ├── parse_account_notification() -> AccountSnapshot
│   ├── account_store.apply(snapshot)
│   └── slot_notify.notify_waiters()
├── pre_hydrate_startup_accounts() [Kamino WSOL supply vault & payer ATA]
└── loop { tick }
    ├── heartbeat.observe_tick(slot)
    ├── publish_control_state()
    ├── health_monitor.sweep() [every 100 ticks]
    ├── emit_five_minute_digest() [every 60s]
    ├── kill_switch.check_file() -> break if active
    ├── check_slot_advancement() [WS slot -> Helius slot -> RPC fallback]
    ├── provider_supervisor.select() -> skip if all providers unhealthy
    └── strategy_cycle().await
```

---

## 3. Per-Tick Strategy & Opportunity Discovery

```
Supervisor::strategy_cycle() [crates/arb-bot/src/supervisor/strategy.rs]
├── ingest.refresh_pump_swap_states()
├── build_verified_inputs(current_slot).await -> VerifiedQuoteInputs
│   ├── Kamino fee parameter decoding (U68F60)
│   └── WSOL mint decimals validation
├── ingest.snapshot() -> MarketStateSnapshot
├── compute_and_publish_ticker()
├── calculate_max_sizing()
│   ├── read live_sol_price()
│   └── clamp by kamino_available_flash_liquidity(current_slot)
├── filter fresh_pools (slot delta <= max_graph_age_slots)
├── graph.build_from_pools(&fresh_pools)
+-- Canonical Route Discovery:
|   +-- graph.find_cycles_with_stats() [bounded simple-cycle scan, 2..4 production hops]
|   +-- graph.find_cross_pool_dlmm_cycles() [venue-specific DLMM seed generation]
|   |   +-- graph.evaluate_directed_route() [canonical exact re-quote, sizing and economics]
|   +-- capture_three_leg_dlmm() [specialized chained-bin candidate generation]
|   |   +-- verify Kamino WSOL supply-vault liquidity (fail closed)
|   |   +-- verify payer WSOL + both intermediate ATA balances are zero
|   |   +-- graph.evaluate_directed_route() [canonical exact re-quote, sizing and economics]
|   +-- route_dedup::dedupe_best_by_route() [ordered route identity; retain best net]
|   +-- rank_opportunities_cross_mint()
+-- process_opportunities().await
```

---

## 4. Opportunity Processing Ladder & Risk Gates

```
Supervisor::process_opportunities() [crates/arb-bot/src/supervisor/opportunity.rs]
├── for opp in opportunities:
│   ├── Invariant Check: opp.size_in <= kamino_available_flash_liquidity
│   ├── Gate G3: validate_route_account_dependencies()
│   ├── Economic Floor Check:
│   │   ├── calculate_kamino_flash_fee(opp.size_in, flash_fee_sf)
│   │   ├── priority_fee_lamports(cu_price, cu_limit)
│   │   └── base_fee + priority_fee + flash_fee + tip
│   ├── RiskGate::evaluate_opportunity()
│   │   ├── check max_trade_size_usd
│   │   ├── check daily loss limit
│   │   └── check consecutive failure circuit breaker
│   ├── Implausible Profit Gate: reject if ROI > MAX_IMPLAUSIBLE_ROI_BPS
│   ├── validate_economic_invariants()
│   │   ├── check closed cycle (mint_in == mint_out)
│   │   ├── check state age
│   │   ├── check positive net profit
│   │   ├── check repayment funding: expected_out >= size_in + fee + costs
│   │   └── check tip < net_profit
│   ├── TxBuilder::build_cross_dex_flash_arbitrage()
│   │   ├── Kamino FlashBorrow instruction
│   │   ├── Hop swap instructions (Raydium / Meteora / Orca)
│   │   ├── On-chain Profit Assertion guard
│   │   └── Kamino FlashRepay instruction
│   ├── AdaptiveTxCompiler::compile_v1_only()
│   │   ├── 0x81 wire format discriminator
│   │   ├── 4,096 byte length assertion
│   │   ├── 64 inline account limit assertion
│   │   ├── SetComputeUnitLimit & SetComputeUnitPrice
│   │   └── SetLoadedAccountsDataSizeLimit (16 MiB)
│   └── dispatch_opportunity().await
```

---

## 5. Simulation, Dispatch & Reconciliation

```
Supervisor::dispatch_opportunity() [crates/arb-bot/src/supervisor/dispatch.rs]
├── GUARD-01: Revalidate stream generation & pool freshness
├── parse_version_from_wire(tx_bytes) -> assert V1
├── select_authoritative_transport()
├── Mandatory Preflight Simulation:
│   ├── simulator.simulate_async(tx_bytes).await
│   │   └── RPC simulateTransaction(sigVerify: false, replaceRecentBlockhash: true)
│   ├── if Simulation FAILED -> record_rejection and return
│   └── if Simulation PASSED -> extract preflight_cu
├── Persist DispatchIntent in arb-store
├── Branch by BotMode:
│   ├── Simulate / PaperTrade -> record simulated settlement and return
│   └── Live ->
│       ├── Keypair::sign_transaction()
│       ├── JitoLander::send_bundle() OR RpcPool::send_transaction()
│       └── spawn(track_inflight_settlement)
│           ├── poll signature statuses
│           ├── on timeout -> recovery::reconcile_intent()
│           ├── fetch confirmed transaction meta
│           ├── extract pre_balances and post_balances
│           ├── derive exact realized PnL
│           └── store finalized outcome in database & Prometheus
```
