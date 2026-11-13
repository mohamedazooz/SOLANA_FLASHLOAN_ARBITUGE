//! End-to-End Bot Supervisor orchestrating Ingest, Engine, Risk, Tx, Sim, Lander, Store & Alerts.

use crate::config::Config;
use crate::jito_service::{ExecutionRoute, JitoService};
use crate::telegram::TelegramNotifier;
use crate::verified_config::VerifiedQuoteInputs;
use crate::{BotMode, health_monitor::HealthMonitor};
use anyhow::Result;
use arb_engine::MarketGraph;
use arb_ingest::IngestClient;
use arb_land::{BidderConfig, JitoLander, TipStrategy};
use arb_math::economic::calculate_net_profit_raw;
use arb_math::{RttTracker, SlotClock};
use arb_risk::{CircuitBreaker, KillSwitch, RiskGate, RiskLimits};
use arb_sim::{ForkSimulator, Simulator};
use arb_store::{AttemptRecord, PostgresStore, SqliteStore, Store};
use arb_tx::{KaminoFlashParams, KeypairSigner, TxBuilder};
use arb_types::{
    Bps, DexProtocol, ExecutionOpportunity, InflightStatus, Lamports, MintId, PoolState, RouteId,
    Slot, SwapHop, TipDecision, TokenAmount,
};
use base64::prelude::*;
use solana_pubkey::Pubkey;
use std::collections::HashSet;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

// Supervisor implementation is split by responsibility. The child modules extend the
// same type and can access its private state without introducing wrapper indirection.
mod dispatch;
mod execution;
mod opportunity;
mod recovery;
mod run;
mod strategy;

use execution::max_state_age_slots;

/// Solana base transaction fee per signature, lamports (versioned & legacy).
/// Part of the single per-tx cost list used by both the Jito edge estimate and
/// the go/no-go gate (TX-02: one cost basis, never two).
const BASE_FEE_LAMPORTS: u64 = 5_000;

/// Maximum share of the gross profit the priority fee may consume (20%). Bounds
/// a runaway priority bid so it never eats the edge it was meant to capture.
const PRIORITY_FEE_MAX_SHARE_BPS: u64 = 2_000;

/// A pre-warmed signed transaction template persisted in Redis.
///
/// Holds the wire bytes plus the blockhash and tip they were built with, so
/// `PreWarmedTxTemplate::from_precompiled_bytes` can re-locate the mutable
/// offsets (borrow amount, repay amount, tip, blockhash) and patch them in place
/// at fire time — skipping the full instruction rebuild on the critical path.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CachedTemplate {
    wire: Vec<u8>,
    blockhash: Vec<u8>,
    tip: u64,
}

pub struct Supervisor {
    mode: Arc<std::sync::atomic::AtomicU8>,
    config: Config,
    ingest: Arc<IngestClient>,
    graph: parking_lot::Mutex<MarketGraph>,
    risk_gate: Arc<RiskGate>,
    jito_lander: JitoLander,
    tip_strategy: TipStrategy,
    registry: Arc<parking_lot::RwLock<arb_ingest::MultiDexRegistry>>,
    /// Recent landed compute-unit prices for adaptive priority fee.
    recent_landed_cu: Vec<u64>,
    #[allow(dead_code)]
    simulator: Simulator,
    store: Arc<dyn Store>,
    telegram: TelegramNotifier,
    keypair: Option<KeypairSigner>,
    last_known_live_sol_price: parking_lot::RwLock<f64>,
    rpc_pool: arb_ingest::RpcPool,
    /// Periodic health sweep: RPC, disk, Telegram. Logs the exact reason for
    /// any outage and alerts via Telegram — never blocks the hot path.
    health_monitor: HealthMonitor,
    /// Atomic, externally verifiable process liveness record. Written from the
    /// main loop so a wedged loop is visible as a stale `heartbeat_seq`.
    heartbeat: Arc<crate::heartbeat::Heartbeat>,
    /// Shared short-TTL Redis / in-memory cache for sub-ms dedup of pools & quotes.
    redis_cache: Arc<arb_store::RedisStore>,
    /// Adaptive Jito tip pricing + leader gating + landing evidence.
    jito_service: Arc<JitoService>,
    /// Maps slots onto the local monotonic clock so a bundle leaves at
    /// `window_start - p95 - 3σ` instead of whenever the tick reaches the
    /// broadcast path.
    slot_clock: parking_lot::Mutex<SlotClock>,
    /// Measured round-trip time to the block engine, plus the instant of the
    /// previous sample (the EWMA's time weight needs the gap).
    rtt: parking_lot::Mutex<(RttTracker, Option<Instant>)>,
    /// Short-TTL recent-blockhash cache. A blockhash is valid for ~60-90s, so
    /// reusing one for a few seconds removes an RPC round-trip from the send
    /// path without risking an expired hash.
    blockhash_cache: parking_lot::Mutex<Option<([u8; 32], Instant)>>,
    /// Tip paid per `(slot, route)`: a retry inside the same window may not
    /// jump past `ladder_growth × previous`, and a new window starts clean.
    previous_tips: parking_lot::Mutex<std::collections::HashMap<(u64, u32), Lamports>>,
    /// Measured CU consumption per route signature for actual CU tracking (Phase E).
    measured_cu: Arc<parking_lot::Mutex<std::collections::HashMap<String, u32>>>,
    /// Tier-1 in-memory pre-warmed transaction templates for zero-copy in-place signing
    ram_templates:
        parking_lot::Mutex<std::collections::HashMap<String, arb_tx::PreWarmedTxTemplate>>,
    /// Dedicated fallback snapshot store for RPC-refreshed Kamino reserve & WSOL mint
    /// accounts so live WS provider invariants in `AccountSnapshotStore` remain intact.
    kamino_fallback_store: arb_ingest::AccountSnapshotStore,
    pub recent_landed: Arc<parking_lot::Mutex<Vec<crate::telegram::TradeSummary>>>,
    pub recent_dropped: Arc<parking_lot::Mutex<Vec<crate::telegram::DroppedTradeSummary>>>,
    pub recent_rejected: Arc<parking_lot::Mutex<Vec<crate::telegram::RejectedTradeSummary>>>,
    /// Unified control service shared with the web API and Telegram. `None`
    /// until main.rs attaches it (tests construct a supervisor without one).
    pub control: std::sync::OnceLock<Arc<crate::control_service::ControlService>>,
}

impl Supervisor {
    pub fn new(mode: BotMode, config: Config) -> Self {
        Self::with_mode_atomic(
            Arc::new(std::sync::atomic::AtomicU8::new(mode as u8)),
            config,
        )
    }

    /// Constructor sharing an externally-owned mode atomic — the unified
    /// control service writes to the same atomic to switch modes at runtime.
    pub fn with_mode_atomic(mode: Arc<std::sync::atomic::AtomicU8>, config: Config) -> Self {
        info!("TRACE SUP 1: start");
        dotenvy::dotenv().ok();
        let limits = RiskLimits {
            max_trade_size_usd: config.risk.max_trade_size_usd,
            min_net_profit_abs: config.strategy.min_net_profit_abs,
            min_net_profit_bps: config.strategy.min_net_profit_bps,
            max_price_impact_bps: config.strategy.max_price_impact_bps,
            max_state_age_slots: config.strategy.max_state_age_slots,
            max_tip_share_bps: config.jito.max_tip_share_bps,
            max_tip_lamports: config.jito.max_tip_lamports,
            max_daily_loss_usd: config.risk.max_daily_loss_usd,
            max_hourly_loss_usd: config.risk.max_hourly_loss_usd,
            // Phase E: new enforcement configs
            min_landing_rate: config.risk.min_landing_rate,
            max_inflight_bundles: config.risk.max_inflight_bundles as usize,
            max_tip_spend_ratio: config.risk.max_tip_spend_ratio,
            max_orders_per_hour: config.risk.max_orders_per_hour,
        };

        let max_consecutive_failures = config.risk.max_consecutive_failures;
        let cooldown_slots = config.risk.cooldown_slots;

        let breaker = Arc::new(CircuitBreaker::new(
            max_consecutive_failures,
            cooldown_slots,
        ));
        // Durable kill switch: the marker file survives restarts, and an
        // existing marker forces the supervisor to start BLOCKED (fail safe).
        let kill_switch_path =
            std::env::var("KILL_SWITCH_FILE").unwrap_or_else(|_| "data/kill_switch".to_string());
        let kill_switch = Arc::new(KillSwitch::with_file(kill_switch_path));
        let chronos_risk = arb_risk::ChronosRiskConfig {
            enabled: config.chronos.enabled,
            decay_lambda_per_sec: config.chronos.decay_lambda_per_sec,
            max_age_ms: config.chronos.max_age_ms,
            min_ev_usd: config.chronos.min_ev_usd,
            failed_attempt_cost_lamports: config.chronos.failed_attempt_cost_lamports,
            beta_risk_factor: config.chronos.beta_risk_factor,
        };
        let risk_gate = Arc::new(RiskGate::with_chronos(
            limits,
            breaker,
            kill_switch,
            chronos_risk,
        ));

        let tip_strategy = TipStrategy::new(
            config.jito.tip_share_bps,
            arb_constants::DEFAULT_JITO_TIP_FLOOR_LAMPORTS,
            config.jito.max_tip_lamports,
            config.jito.min_keep_lamports,
        );

        // No synthetic CU seed: adaptive CU pricing learns only from measured observations.
        let recent_landed_cu = Vec::<u64>::new();

        let jito_lander = JitoLander::new(config.jito.regions.clone());

        // The configured Kamino flash-loan reserve is WSOL. Every candidate
        // cycle must therefore start and finish in WSOL so the borrow, route
        // accounting, fee units and same-transaction repayment share one mint
        // and 9-decimal raw-unit domain. Other configured tokens remain valid
        // intermediate route vertices; they are not independent loan anchors.
        let wsol_mint =
            Pubkey::from_str(arb_tx::WSOL_MINT).expect("canonical WSOL mint constant must parse");
        let graph = MarketGraph::new(vec![MintId(wsol_mint)]);
        // Ingestion is WebSocket + RPC only. gRPC transport removed.
        let ingest = Arc::new(IngestClient::new());

        info!("TRACE SUP 2: before store");
        let store: Arc<dyn Store> = if config.store.url.starts_with("postgres")
            || config.store.url.starts_with("postgresql")
        {
            match PostgresStore::open(&config.store.url) {
                Ok(pg) => {
                    // Never log the DSN: it carries the database password.
                    info!("Connected to PostgreSQL store");
                    Arc::new(pg)
                }
                Err(e) => {
                    warn!(
                        "Failed to connect to PostgreSQL ({}): falling back to SQLite",
                        e
                    );
                    Arc::new(SqliteStore::open_in_memory().unwrap())
                }
            }
        } else if let Ok(pg_url) = std::env::var("DATABASE_URL") {
            if pg_url.starts_with("postgres") || pg_url.starts_with("postgresql") {
                match PostgresStore::open(&pg_url) {
                    Ok(pg) => {
                        info!("Connected to PostgreSQL store via DATABASE_URL");
                        Arc::new(pg)
                    }
                    Err(e) => {
                        warn!(
                            "Failed to connect to PostgreSQL via DATABASE_URL ({}): falling back to SQLite",
                            e
                        );
                        Arc::new(
                            SqliteStore::open(&config.store.url)
                                .unwrap_or_else(|_| SqliteStore::open_in_memory().unwrap()),
                        )
                    }
                }
            } else {
                Arc::new(
                    SqliteStore::open(&config.store.url)
                        .unwrap_or_else(|_| SqliteStore::open_in_memory().unwrap()),
                )
            }
        } else {
            Arc::new(
                SqliteStore::open(&config.store.url)
                    .unwrap_or_else(|_| SqliteStore::open_in_memory().unwrap()),
            )
        };
        info!("TRACE SUP 3: after store");

        let telegram = TelegramNotifier::from_env();
        #[cfg(feature = "shadow-only")]
        let keypair: Option<KeypairSigner> = {
            info!("shadow-only build: signer disabled; no wallet material will be loaded");
            None
        };

        #[cfg(not(feature = "shadow-only"))]
        let keypair = std::env::var("SOLANA_PRIVATE_KEY")
            .ok()
            .and_then(|pk| match KeypairSigner::from_base58(&pk) {
                Ok(signer) => {
                    info!(
                        "In-memory keypair loaded from SOLANA_PRIVATE_KEY: {}",
                        signer.pubkey()
                    );
                    Some(signer)
                }
                Err(e) => {
                    warn!("Failed to parse SOLANA_PRIVATE_KEY: {e}");
                    None
                }
            })
            .or_else(|| {
                let kp_path = if !config.wallet.keypair_path.is_empty() {
                    config.wallet.keypair_path.as_str()
                } else {
                    "solana_keypair.json"
                };
                if std::path::Path::new(kp_path).exists() {
                    match KeypairSigner::from_file(kp_path) {
                        Ok(signer) => {
                            info!(
                                "In-memory keypair loaded from {kp_path}: {}",
                                signer.pubkey()
                            );
                            Some(signer)
                        }
                        Err(e) => {
                            warn!("Failed to load {kp_path}: {e}");
                            None
                        }
                    }
                } else {
                    None
                }
            });

        // Legacy JSON-RPC pool is disabled in runtime configuration; retained only for
        // unmigrated APIs and isolated tests. Market state comes from the WebSocket + RPC ingestion path. gRPC transport removed.
        let rpc_pool = arb_ingest::RpcPool::from_env();

        let sim_url = rpc_pool
            .endpoints()
            .first()
            .cloned()
            .or_else(|| std::env::var("SOLANA_RPC_URL").ok())
            .or_else(|| std::env::var("FAST_ENDPOINT").ok())
            .or_else(|| std::env::var("HELIUS_RPC_URL").ok());
        let simulator = match sim_url {
            Some(url) => Simulator::with_rpc_url(url),
            None => Simulator::new(),
        };

        let registry = Arc::new(parking_lot::RwLock::new(
            arb_ingest::MultiDexRegistry::mainnet_default(rpc_pool.clone()),
        ));

        // Server health monitor: watches WebSocket/RPC ingestion, disk, Telegram. Logs the exact
        // reason for any outage and alerts — never blocks trading.
        let data_dir =
            PathBuf::from(std::env::var("DATA_DIR").unwrap_or_else(|_| "data".to_string()));
        let health_monitor = HealthMonitor::new(
            Arc::clone(&ingest),
            data_dir,
            telegram.clone(),
            rpc_pool.clone(),
        );

        // Jito transport remains isolated from the Solana market-data plane.
        // No RPC market-state reads are performed by this service in this phase.
        let jito_service_config = crate::jito_service::JitoServiceConfig {
            regions: config.jito.regions.clone(),
            bidder: BidderConfig {
                keep_share: config.jito.keep_share,
                max_tip_share: (config.jito.max_tip_share_bps as f64) / 10_000.0,
                profit_floor: Lamports(config.jito.min_keep_lamports),
                ladder_growth: config.jito.ladder_growth,
            },
            tip_floor_max_age_ms: config.jito.tip_floor_max_age_ms,
            tip_floor_poll_ms: config.jito.tip_floor_poll_ms,
            kobe_url: config
                .jito
                .kobe_url
                .clone()
                .unwrap_or_else(|| crate::jito_service::KOBE_VALIDATORS_URL.to_string()),
            ledger_path: config
                .jito
                .ledger_path
                .clone()
                .map(std::path::PathBuf::from),
            // `leader_gate_enabled` is a fail-closed policy: when enabled, the
            // bot refuses to bid on unknown leaders rather than silently paying
            // a tip into a non-Jito slot. The service config still exposes the
            // explicit override for an allowlist/legacy opt-in mode.
            tip_on_unknown_leader: !config.jito.leader_gate_enabled,
        };
        let jito_service =
            Arc::new(JitoService::new(jito_service_config).expect("valid jito service config"));
        info!("TRACE SUP 4: after jito_service");

        let heartbeat = Arc::new(crate::heartbeat::Heartbeat::new(
            crate::heartbeat::Heartbeat::path_from_env(),
            BotMode::from_u8(mode.load(std::sync::atomic::Ordering::Acquire)).to_string(),
        ));
        info!(
            "Heartbeat writer active: pid={} path={}",
            heartbeat.pid(),
            heartbeat.path().display()
        );

        Self {
            mode,
            config,
            ingest,
            graph: parking_lot::Mutex::new(graph),
            risk_gate,
            jito_lander,
            tip_strategy,
            registry,
            recent_landed_cu,
            simulator,
            store,
            telegram,
            keypair,
            last_known_live_sol_price: parking_lot::RwLock::new(0.0),
            rpc_pool,
            health_monitor,
            heartbeat,
            redis_cache: Arc::new(arb_store::RedisStore::new(
                std::env::var("REDIS_URL").ok(),
                10,
            )),
            jito_service,
            slot_clock: parking_lot::Mutex::new(SlotClock::new()),
            rtt: parking_lot::Mutex::new((RttTracker::new(300.0), None)),
            blockhash_cache: parking_lot::Mutex::new(None),
            previous_tips: parking_lot::Mutex::new(std::collections::HashMap::new()),
            measured_cu: Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
            ram_templates: parking_lot::Mutex::new(std::collections::HashMap::new()),
            kamino_fallback_store: arb_ingest::AccountSnapshotStore::default(),
            recent_landed: Arc::new(parking_lot::Mutex::new(Vec::new())),
            recent_dropped: Arc::new(parking_lot::Mutex::new(Vec::new())),
            recent_rejected: Arc::new(parking_lot::Mutex::new(Vec::new())),
            control: std::sync::OnceLock::new(),
        }
    }

    /// Access the underlying risk kill-switch for administrative controls.
    pub fn kill_switch(&self) -> Arc<arb_risk::KillSwitch> {
        self.risk_gate.kill_switch()
    }

    pub fn ingest(&self) -> Arc<IngestClient> {
        Arc::clone(&self.ingest)
    }

    /// Shared heartbeat writer, so the main loop can record liveness and gates
    /// can record rejections.
    pub fn heartbeat(&self) -> Arc<crate::heartbeat::Heartbeat> {
        Arc::clone(&self.heartbeat)
    }

    /// Effective execution mode. Runtime-switchable through the shared atomic
    /// so the unified control service (web + Telegram) can transition
    /// shadow/paper/live without a restart.
    pub fn mode(&self) -> BotMode {
        BotMode::from_u8(self.mode.load(std::sync::atomic::Ordering::Acquire))
    }

    pub fn set_mode(&self, mode: BotMode) {
        self.mode
            .store(mode as u8, std::sync::atomic::Ordering::Release);
    }

    /// Attaches the shared control service (called once from main before run).
    pub fn attach_control(&self, control: Arc<crate::control_service::ControlService>) {
        let _ = self.control.set(control);
    }

    fn control(&self) -> Option<&Arc<crate::control_service::ControlService>> {
        self.control.get()
    }

    /// Publishes the current live-state view to the unified control service.
    /// Cheap (small struct copy) and called once per loop iteration.
    fn publish_control_state(&self, current_slot: Slot) {
        let Some(control) = self.control() else {
            return;
        };
        control.refresh_deps(crate::control_service::ControlDeps {
            slot: current_slot.0,
            pools_tracked: self.ingest.snapshot().pool_count(),
            stream_connected: self.ingest.stream_connected(),
            helius_connected: self.ingest.helius_connected(),
            route_coverage: self.route_coverage(),
        });
    }

    /// Per-protocol route coverage for the route monitor: one cheap pass over
    /// the registry (protocol + fee_bps only, no PoolState clones).
    fn route_coverage(&self) -> Vec<crate::control_service::ProtocolCoverage> {
        use std::collections::BTreeMap;
        let registry = self.registry.read();
        let mut acc: BTreeMap<&'static str, (usize, u64)> = BTreeMap::new();
        for p in registry.pools() {
            let entry = acc.entry(p.protocol.as_str()).or_default();
            entry.0 += 1;
            entry.1 += u64::from(p.fee_bps);
        }
        acc.into_iter()
            .map(
                |(protocol, (pools, fee_sum))| crate::control_service::ProtocolCoverage {
                    protocol: protocol.to_string(),
                    pools,
                    avg_fee_bps: if pools > 0 {
                        fee_sum as f64 / pools as f64
                    } else {
                        0.0
                    },
                },
            )
            .collect()
    }

    /// Fold one accepted block-engine round trip into the latency model.
    ///
    /// Only accepted submissions are measured: a timeout is the failure mode
    /// being guarded against, not evidence about the normal path.
    fn observe_bundle_rtt(&self, elapsed: std::time::Duration) {
        let mut guard = self.rtt.lock();
        let dt_s = guard
            .1
            .map(|last| last.elapsed().as_secs_f64())
            .unwrap_or(0.0);
        if guard.0.observe(elapsed.as_secs_f64() * 1_000.0, dt_s) {
            guard.1 = Some(Instant::now());
        }
    }

    /// Save detailed market snapshot to disk and optionally send Telegram digest card.
    pub async fn emit_market_snapshot(&self, current_slot: u64, send_telegram: bool) {
        let pools = self.ingest.snapshot().snapshot_pools();
        let total_pools = pools.len();
        let mut orca_count = 0;
        let mut raydium_count = 0;
        let mut meteora_count = 0;
        let mut pump_count = 0;
        let mut pumpswap_count = 0;

        for p in &pools {
            match p.protocol {
                DexProtocol::OrcaWhirlpool => orca_count += 1,
                DexProtocol::RaydiumV4
                | DexProtocol::RaydiumCpmm
                | DexProtocol::RaydiumClmm
                | DexProtocol::RaydiumLaunchLab => raydium_count += 1,
                DexProtocol::MeteoraDlmm | DexProtocol::MeteoraDammV2 => meteora_count += 1,
                DexProtocol::PumpFun => pump_count += 1,
                DexProtocol::PumpSwap => pumpswap_count += 1,
            }
        }

        let base_tokens_count = self.graph.lock().base_anchors.len();
        let sol_price = self.live_sol_price().unwrap_or(0.0);
        let opportunities_total = arb_metrics::get_opportunities_total();

        let snapshot = serde_json::json!({
            "timestamp": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            "slot": current_slot,
            "total_pools": total_pools,
            "dex_distribution": {
                "orca_whirlpool": orca_count,
                "raydium": raydium_count,
                "meteora_dlmm": meteora_count,
                "pump_fun": pump_count,
                "pump_swap": pumpswap_count,
            },
            "base_tokens_count": base_tokens_count,
            "sol_price_usd": sol_price,
            "opportunities_total": opportunities_total,
        });

        let _ = std::fs::create_dir_all("data");
        let _ = std::fs::write(
            "data/market_snapshot.json",
            serde_json::to_string_pretty(&snapshot).unwrap_or_default(),
        );

        if send_telegram {
            self.telegram
                .alert_market_snapshot_digest(
                    total_pools,
                    orca_count,
                    raydium_count,
                    meteora_count,
                    pump_count,
                    base_tokens_count,
                    sol_price,
                    opportunities_total,
                )
                .await;
        }
    }

    pub fn record_landed_trade(&self, trade: crate::telegram::TradeSummary) {
        let mut guard = self.recent_landed.lock();
        if guard.len() >= 50 {
            guard.remove(0);
        }
        let route = trade.route.clone();
        guard.push(trade);
        drop(guard);
        if let Some(control) = self.control() {
            control.record_execution("landed", &route, "trade confirmed landed");
            control.record_execution_result(true);
        }
    }

    pub fn record_dropped_bundle(&self, dropped: crate::telegram::DroppedTradeSummary) {
        let mut guard = self.recent_dropped.lock();
        if guard.len() >= 50 {
            guard.remove(0);
        }
        let route = dropped.route.clone();
        let status = dropped.reason.clone();
        guard.push(dropped);
        drop(guard);
        if let Some(control) = self.control() {
            control.record_execution("dropped", &route, &status);
            control.record_execution_result(false);
        }
    }

    pub fn record_rejected_opportunity(&self, rejected: crate::telegram::RejectedTradeSummary) {
        self.heartbeat
            .observe_rejection(rejected.slot, &rejected.stage, &rejected.reason);
        if let Some(control) = self.control() {
            control.record_log(
                "warn",
                &rejected.stage,
                format!("{}: {}", rejected.route, rejected.reason),
            );
        }
        let mut guard = self.recent_rejected.lock();
        if guard.len() >= 100 {
            guard.remove(0);
        }
        guard.push(rejected);
    }

    /// Periodic 5-minute digest with detailed lists of successful, dropped, and rejected trades,
    /// along with infrastructure and RPC rate-limit status.
    pub async fn emit_five_minute_digest(&self, current_slot: u64) {
        let pools = self.ingest.snapshot().snapshot_pools();
        let total_pools = pools.len();
        let mut orca_count = 0;
        let mut raydium_count = 0;
        let mut meteora_count = 0;
        let mut pump_count = 0;

        for p in &pools {
            match p.protocol {
                DexProtocol::OrcaWhirlpool => orca_count += 1,
                DexProtocol::RaydiumV4
                | DexProtocol::RaydiumCpmm
                | DexProtocol::RaydiumClmm
                | DexProtocol::RaydiumLaunchLab => raydium_count += 1,
                DexProtocol::MeteoraDlmm | DexProtocol::MeteoraDammV2 => meteora_count += 1,
                DexProtocol::PumpFun | DexProtocol::PumpSwap => pump_count += 1,
            }
        }

        let sol_price = self.live_sol_price().unwrap_or(0.0);
        let opportunities_total = arb_metrics::get_opportunities_total();

        let landed = {
            let mut guard = self.recent_landed.lock();
            std::mem::take(&mut *guard)
        };
        let dropped = {
            let mut guard = self.recent_dropped.lock();
            std::mem::take(&mut *guard)
        };
        let rejected = {
            let mut guard = self.recent_rejected.lock();
            std::mem::take(&mut *guard)
        };

        let rpc_status = if self.rpc_pool.endpoints_count() > 0 {
            format!("{} Active (Balanced)", self.rpc_pool.endpoints_count())
        } else {
            "No endpoints".to_string()
        };

        let ws_status = if self.ingest.helius_connected() {
            "Connected (Mainnet WebSocket)".to_string()
        } else {
            "Reconnecting...".to_string()
        };

        // gRPC transport removed: ingestion is WebSocket + RPC only.
        let ingest_status = "WebSocket + RPC (gRPC disabled)".to_string();

        let jito_status = if self.jito_service.leader_for(Slot(current_slot)).is_some() {
            "Online (Leader Synced / Tip Active)".to_string()
        } else {
            "Online (Tip Stream Active)".to_string()
        };

        let stats = crate::telegram::DigestStats {
            slot: current_slot,
            sol_price,
            total_pools,
            orca_count,
            raydium_count,
            meteora_count,
            pump_count,
            cycles_evaluated: opportunities_total,
            rpc_status,
            ws_status,
            ingest_status,
            jito_status,
            landed_trades: landed,
            dropped_trades: dropped,
            rejected_trades: rejected,
        };

        self.telegram.alert_five_minute_digest(&stats).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_supervisor_initialization() {
        let config_str = include_str!("../../../config/config.example.toml");
        let config: Config = toml::from_str(config_str).expect("Valid example config");
        let supervisor = Supervisor::new(BotMode::Shadow, config);
        assert_eq!(supervisor.mode(), BotMode::Shadow);
    }

    fn pool_at(id: u8, last_update_slot: u64) -> PoolState {
        PoolState {
            pool_id: arb_types::PoolId::new([id; 32]),
            protocol: DexProtocol::RaydiumV4,
            slot: Slot(last_update_slot),
            write_version: 1,
            base_mint: MintId(Pubkey::new_from_array([7; 32])),
            quote_mint: MintId(Pubkey::new_from_array([8; 32])),
            decimals_a: 9,
            decimals_b: 6,
            reserve_in: 1,
            reserve_out: 1,
            fee_numerator: 25,
            fee_denominator: 10_000,
            is_active: true,
            has_risky_token: false,
            kind: arb_types::PoolKind::Cpmm,
            clmm: None,
            dlmm: None,
            pump_fun: None,
            transfer_fee_bps: 0,
            token_programs: Default::default(),
            execution_accounts: None,
            last_update_slot,
            last_update_ms: 0,
        }
    }

    fn hop_for(id: u8) -> SwapHop {
        SwapHop {
            pool_id: arb_types::PoolId::new([id; 32]),
            protocol: DexProtocol::RaydiumV4,
            mint_in: MintId(Pubkey::new_from_array([7; 32])),
            mint_out: MintId(Pubkey::new_from_array([8; 32])),
            fee_bps: 25,
            active_bin_id: None,
            token_programs: Default::default(),
            trade_intent: Default::default(),
            execution_accounts: None,
        }
    }

    #[test]
    fn state_age_is_max_over_hops_and_fails_closed() {
        let pools = vec![pool_at(1, 100), pool_at(2, 97)];

        // Whole-route freshness = the stalest hop: max(0, 3) = 3.
        assert_eq!(
            max_state_age_slots(&[hop_for(1), hop_for(2)], &pools, Slot(100)),
            3
        );

        // Unknown pool must fail closed, not fall back to "fresh".
        assert_eq!(
            max_state_age_slots(&[hop_for(1), hop_for(99)], &pools, Slot(100)),
            u64::MAX
        );

        // Never-stamped pool reports its full age, never 0.
        assert_eq!(
            max_state_age_slots(&[hop_for(3)], &[pool_at(3, 0)], Slot(100)),
            100
        );
    }
}
