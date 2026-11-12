//! End-to-End Bot Supervisor orchestrating Ingest, Engine, Risk, Tx, Sim, Lander, Store & Alerts.

use crate::config::Config;
use crate::jito_service::{ExecutionRoute, JitoService};
use crate::telegram::TelegramNotifier;
use crate::verified_config::VerifiedQuoteInputs;
use crate::{BotMode, health_monitor::HealthMonitor};
use anyhow::Result;
use arb_engine::MarketGraph;
use arb_ingest::YellowstoneClient;
use arb_land::{BidderConfig, JitoLander, TipStrategy};
use arb_math::economic::calculate_net_profit_raw;
use arb_math::{RttTracker, SlotClock};
use arb_risk::{CircuitBreaker, KillSwitch, RiskGate, RiskLimits};
use arb_sim::Simulator;
use arb_store::{AttemptRecord, PostgresStore, SqliteStore, Store};
use arb_tx::{JupiterClient, KaminoFlashParams, KeypairSigner, TxBuilder};
use arb_types::{
    DexProtocol, ExecutionOpportunity, InflightStatus, Lamports, MintId, PoolState, Slot, SwapHop,
    TipDecision, TokenAmount,
};
use base64::prelude::*;
use solana_pubkey::Pubkey;
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
mod run;

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
    mode: BotMode,
    config: Config,
    ingest: Arc<YellowstoneClient>,
    graph: parking_lot::Mutex<MarketGraph>,
    risk_gate: Arc<RiskGate>,
    jito_lander: JitoLander,
    tip_strategy: TipStrategy,
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
    /// Shared short-TTL Redis / in-memory cache for sub-ms dedup of pools & quotes.
    redis_cache: Arc<arb_store::RedisStore>,
    /// Jupiter v6 client for live swap-instruction resolution of exotic / trending
    /// (e.g. Meteora DLMM) routes — byte-exact, fully account-resolved, live.
    jupiter: arb_tx::JupiterClient,
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
}

impl Supervisor {
    pub fn new(mode: BotMode, config: Config) -> Self {
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
        let kill_switch = Arc::new(KillSwitch::new());
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
        // HTTP JSON-RPC is disabled by architecture. Simulation remains unavailable
        // until a provider-native gRPC/WebSocket simulation path is implemented.
        let simulator = Simulator::default();

        let base_mints: Vec<MintId> = config
            .strategy
            .base_tokens
            .iter()
            .filter_map(|t| Pubkey::from_str(t).ok().map(MintId))
            .collect();

        let graph = MarketGraph::new(base_mints);
        // Yellowstone requires a Yellowstone-compatible gRPC endpoint. Do not pass
        // an HTTP RPC URL here; absent configuration leaves ingestion unavailable.
        let ingest_url = std::env::var("SOLAMI_GRPC_ENDPOINT")
            .ok()
            .filter(|value| !value.trim().is_empty() && !value.starts_with("REPLACE_ME"))
            .unwrap_or_default();
        let ingest_token = std::env::var("SOLAMI_API_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty() && !value.starts_with("REPLACE_ME"))
            .unwrap_or_default();
        if ingest_url.is_empty() || ingest_token.is_empty() {
            warn!(
                "Solami Yellowstone gRPC is not configured; market ingestion will remain unavailable"
            );
        }
        let ingest = Arc::new(YellowstoneClient::new(ingest_url, ingest_token));

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
        let keypair = std::env::var("SOLANA_PRIVATE_KEY")
            .ok()
            .and_then(|pk| match KeypairSigner::from_base58(&pk) {
                Ok(signer) => {
                    info!(
                        "🔑 In-Memory Keypair loaded from SOLANA_PRIVATE_KEY: {}",
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
                                "🔑 In-Memory Keypair loaded from {kp_path}: {}",
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
        // unmigrated APIs and isolated tests. Market state comes from Yellowstone gRPC.
        let rpc_pool = arb_ingest::RpcPool::from_env();

        // Server health monitor: watches Yellowstone gRPC, disk, Telegram. Logs the exact
        // reason for any outage and alerts — never blocks trading.
        let data_dir =
            PathBuf::from(std::env::var("DATA_DIR").unwrap_or_else(|_| "data".to_string()));
        let health_monitor = HealthMonitor::new(Arc::clone(&ingest), data_dir, telegram.clone());

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

        Self {
            mode,
            config,
            ingest,
            graph: parking_lot::Mutex::new(graph),
            risk_gate,
            jito_lander,
            tip_strategy,
            recent_landed_cu,
            simulator,
            store,
            telegram,
            keypair,
            last_known_live_sol_price: parking_lot::RwLock::new(0.0),
            rpc_pool,
            health_monitor,
            redis_cache: Arc::new(arb_store::RedisStore::new(
                std::env::var("REDIS_URL").ok(),
                10,
            )),
            jupiter: JupiterClient::new(),
            jito_service,
            slot_clock: parking_lot::Mutex::new(SlotClock::new()),
            rtt: parking_lot::Mutex::new((RttTracker::new(300.0), None)),
            blockhash_cache: parking_lot::Mutex::new(None),
            previous_tips: parking_lot::Mutex::new(std::collections::HashMap::new()),
            measured_cu: Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
            ram_templates: parking_lot::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Access the underlying risk kill-switch for administrative controls.
    pub fn kill_switch(&self) -> Arc<arb_risk::KillSwitch> {
        self.risk_gate.kill_switch()
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
                DexProtocol::RaydiumV4 | DexProtocol::RaydiumCpmm | DexProtocol::RaydiumClmm => {
                    raydium_count += 1
                }
                DexProtocol::MeteoraDlmm => meteora_count += 1,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_supervisor_initialization() {
        let config_str = include_str!("../../../config/config.example.toml");
        let config: Config = toml::from_str(config_str).expect("Valid example config");
        let supervisor = Supervisor::new(BotMode::Shadow, config);
        assert_eq!(supervisor.mode, BotMode::Shadow);
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
