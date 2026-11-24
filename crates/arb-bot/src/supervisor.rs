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
use arb_sim::{ForensicBundle, ForkSimulator, HopProvenance, Simulator};
use arb_store::{AttemptRecord, PostgresStore, SqliteStore, Store};
use arb_tx::{KaminoFlashParams, KeypairSigner, TxBuilder};
use arb_types::{
    Bps, DexProtocol, ExecutionOpportunity, Lamports, MintId, PoolState, RouteId, SignedLamports,
    Slot, SwapHop, TipDecision, TokenAmount,
};
use solana_pubkey::Pubkey;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

// Supervisor implementation is split by responsibility. The child modules extend the
// same type and can access its private state without introducing wrapper indirection.
pub mod dispatch;
pub mod execution;
pub mod opportunity;
pub mod recovery;
mod route_dedup;
pub mod run;
pub mod strategy;

use execution::max_state_age_slots;

/// Solana base transaction fee per signature, lamports (versioned & legacy).
/// Re-exported from canonical `arb_constants::SOLANA_BASE_FEE_LAMPORTS`.
pub const BASE_FEE_LAMPORTS: u64 = arb_constants::SOLANA_BASE_FEE_LAMPORTS;

type CachedBlockhash = ([u8; 32], Instant, Option<i64>);

/// Refreshed by a low-frequency background task; dispatch only reads atomics.
/// This keeps the SOL reserve guard out of the latency-sensitive HTTP-RPC path.
#[derive(Default)]
pub struct WalletBalanceCache {
    balance_lamports: AtomicU64,
    refreshed_at_ms: AtomicU64,
    healthy: AtomicBool,
}

impl WalletBalanceCache {
    pub fn update(&self, balance_lamports: u64) {
        self.balance_lamports
            .store(balance_lamports, Ordering::Release);
        self.refreshed_at_ms.store(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
                .unwrap_or(0),
            Ordering::Release,
        );
        self.healthy.store(true, Ordering::Release);
    }

    pub fn invalidate(&self) {
        self.healthy.store(false, Ordering::Release);
    }

    pub fn snapshot(&self, max_age: Duration) -> Option<u64> {
        if !self.healthy.load(Ordering::Acquire) {
            return None;
        }
        let refreshed_at_ms = self.refreshed_at_ms.load(Ordering::Acquire);
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        if refreshed_at_ms == 0
            || now_ms < refreshed_at_ms
            || now_ms - refreshed_at_ms > max_age.as_millis().min(u128::from(u64::MAX)) as u64
        {
            return None;
        }
        Some(self.balance_lamports.load(Ordering::Acquire))
    }
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
    /// Per-pool account dependency manifests backing the fail-closed quote
    /// dependency gate. Registered lazily per pool from the same manifest
    /// builder that drives subscriptions.
    dependency_manifests: parking_lot::Mutex<arb_ingest::DependencyManifestRegistry>,
    /// Recent landed compute-unit prices (µlamports/CU) for adaptive priority fee.
    /// Bounded window fed ONLY by confirmed landings (dispatch.rs) — the p75+25%
    /// escalation in `adaptive_cu_price` never sees unconfirmed prices.
    recent_landed_cu: Arc<parking_lot::Mutex<Vec<u64>>>,
    simulator: Simulator,
    store: Arc<dyn Store>,
    intel_persistence: Option<Arc<arb_store::AsyncIntelligencePersistence>>,
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
    blockhash_cache: parking_lot::Mutex<Option<CachedBlockhash>>,
    /// Tip paid per `(slot, route)`: a retry inside the same window may not
    /// jump past `ladder_growth × previous`, and a new window starts clean.
    previous_tips: parking_lot::Mutex<std::collections::HashMap<(u64, u32), Lamports>>,
    /// Measured CU consumption per route signature for actual CU tracking (Phase E).
    measured_cu: Arc<parking_lot::Mutex<std::collections::HashMap<String, u32>>>,
    /// Dedicated fallback snapshot store for RPC-refreshed Kamino reserve & WSOL mint
    /// accounts so live WS provider invariants in `AccountSnapshotStore` remain intact.
    kamino_fallback_store: arb_ingest::AccountSnapshotStore,
    /// Provider capability registry verifying transport methods and prerequisites (G1, G2).
    capability_registry: Arc<arb_ingest::ProviderCapabilityRegistry>,
    /// Isolated HTTP Transaction RPC adapter for blockhash, simulate, send, and status.
    tx_rpc: Arc<arb_ingest::TransactionRpcAdapter>,
    /// Parsed once at startup; dispatch never reads environment or calls RPC for this guard.
    min_sol_reserve_lamports: u64,
    wallet_balance_cache: Arc<WalletBalanceCache>,
    pub recent_landed: Arc<parking_lot::Mutex<Vec<crate::telegram::TradeSummary>>>,
    pub recent_dropped: Arc<parking_lot::Mutex<Vec<crate::telegram::DroppedTradeSummary>>>,
    pub recent_rejected: Arc<parking_lot::Mutex<Vec<crate::telegram::RejectedTradeSummary>>>,
    /// Unified control service shared with the web API and Telegram. `None`
    /// until main.rs attaches it (tests construct a supervisor without one).
    pub control: std::sync::OnceLock<Arc<crate::control_service::ControlService>>,
}

/// Resolve the configured store to a durable backend. Fail-closed: a
/// configured-but-unavailable store returns an error and startup aborts —
/// the supervisor never silently degrades to non-durable in-memory storage,
/// because dispatch-intent reconciliation (R12) requires durable persistence.
/// Precedence: `store_url` postgres → `database_url` postgres → explicit
/// `store_url` sqlite/custom → `database_url` sqlite → default `store_url`.
type OpenedStore = (
    Arc<dyn Store>,
    Option<Arc<arb_store::AsyncIntelligencePersistence>>,
);

fn open_store_and_intel(
    store_url: &str,
    database_url: Option<&str>,
) -> anyhow::Result<OpenedStore> {
    if store_url.starts_with("postgres") {
        let pg = Arc::new(
            PostgresStore::open(store_url)
                .map_err(|e| anyhow::anyhow!("PostgreSQL store unavailable (fail-closed): {e}"))?,
        );
        let intel_queue = Arc::new(arb_store::AsyncIntelligencePersistence::new(pg.clone()));
        return Ok((pg, Some(intel_queue)));
    }
    if let Some(pg_url) = database_url.filter(|u| u.starts_with("postgres")) {
        let pg = Arc::new(PostgresStore::open(pg_url).map_err(|e| {
            anyhow::anyhow!("PostgreSQL store from DATABASE_URL unavailable (fail-closed): {e}")
        })?);
        let intel_queue = Arc::new(arb_store::AsyncIntelligencePersistence::new(pg.clone()));
        return Ok((pg, Some(intel_queue)));
    }
    // An explicit, non-default store_url takes precedence over DATABASE_URL so
    // tests and custom configs isolate their storage and do not bleed into the
    // shared warm DB.
    let is_default_store = store_url.is_empty()
        || store_url == "sqlite://arb_sentinel.db"
        || store_url == "arb_sentinel.db";
    if !is_default_store {
        let s = Arc::new(
            SqliteStore::open(&sqlite_path_from_url(store_url))
                .map_err(|e| anyhow::anyhow!("SQLite store unavailable (fail-closed): {e}"))?,
        );
        let intel_queue = Arc::new(arb_store::AsyncIntelligencePersistence::new(s.clone()));
        return Ok((s, Some(intel_queue)));
    }
    if let Some(sqlite_url) = database_url.filter(|u| u.starts_with("sqlite://")) {
        let s = Arc::new(
            SqliteStore::open(&sqlite_path_from_url(sqlite_url)).map_err(|e| {
                anyhow::anyhow!("SQLite store from DATABASE_URL unavailable (fail-closed): {e}")
            })?,
        );
        let intel_queue = Arc::new(arb_store::AsyncIntelligencePersistence::new(s.clone()));
        return Ok((s, Some(intel_queue)));
    }
    let s = Arc::new(
        SqliteStore::open(&sqlite_path_from_url(store_url))
            .map_err(|e| anyhow::anyhow!("SQLite store unavailable (fail-closed): {e}"))?,
    );
    let intel_queue = Arc::new(arb_store::AsyncIntelligencePersistence::new(s.clone()));
    Ok((s, Some(intel_queue)))
}

#[allow(dead_code)]
fn open_store(store_url: &str, database_url: Option<&str>) -> anyhow::Result<Arc<dyn Store>> {
    open_store_and_intel(store_url, database_url).map(|(store, _)| store)
}

/// Convert a `sqlite://` URL to a filesystem path. `sqlite:///abs/path` yields
/// `/abs/path`; `sqlite://./rel` and `sqlite://rel` yield the relative path;
/// plain paths pass through. The URL was previously passed raw to
/// `Connection::open`, which silently failed on the literal filename and
/// degraded to in-memory storage.
fn sqlite_path_from_url(url: &str) -> String {
    match url.strip_prefix("sqlite://") {
        Some(rest) => {
            let no_leading_slashes = rest.trim_start_matches('/');
            if no_leading_slashes.starts_with("./") || no_leading_slashes.starts_with("../") {
                // `sqlite:///./rel` — scheme slashes followed by a dot-relative path
                no_leading_slashes.to_string()
            } else {
                rest.to_string()
            }
        }
        None => url.to_string(),
    }
}

/// Wallet material is intentionally unavailable in compile-time shadow builds.
/// This branch does not inspect environment variables or keypair files.
#[cfg(feature = "shadow-only")]
fn load_wallet_signer(_config: &Config) -> Option<KeypairSigner> {
    None
}

#[cfg(not(feature = "shadow-only"))]
fn load_wallet_signer(config: &Config) -> Option<KeypairSigner> {
    std::env::var("SOLANA_PRIVATE_KEY")
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
        })
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
        #[cfg(not(feature = "shadow-only"))]
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
        let recent_landed_cu = Arc::new(parking_lot::Mutex::new(Vec::<u64>::new()));

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
        // Fail-closed store selection (see `open_store`): a configured but
        // unavailable store aborts startup. Silent degradation to in-memory
        // storage would break dispatch-intent durability (R12) and PnL
        // accounting while appearing healthy.
        let (store, intel_persistence) = open_store_and_intel(
            &config.store.url,
            std::env::var("DATABASE_URL").ok().as_deref(),
        )
        .expect(
            "durable store unavailable; refusing to start (fail-closed: no in-memory fallback)",
        );
        info!("TRACE SUP 3: after store");

        let telegram = TelegramNotifier::from_env();
        let keypair = load_wallet_signer(&config);

        // Legacy JSON-RPC pool is disabled in runtime configuration; retained only for
        // unmigrated APIs and isolated tests. Market state comes from the WebSocket + RPC ingestion path. gRPC transport removed.
        // Endpoints merge TOML [rpc] url/failover_url (operator config) with the
        // env-sourced pool; placeholders and empty values are filtered.
        let is_usable_rpc = |u: &str| {
            let t = u.trim();
            !t.is_empty()
                && !t.contains("REPLACE_ME")
                && (t.starts_with("https://") || t.starts_with("http://"))
        };
        let mut config_rpc_urls = Vec::new();
        if is_usable_rpc(&config.rpc.url) {
            config_rpc_urls.push(config.rpc.url.trim().to_string());
        }
        if let Some(f) = config.rpc.failover_url.as_deref() {
            if is_usable_rpc(f) {
                config_rpc_urls.push(f.trim().to_string());
            }
        }
        let rpc_pool = arb_ingest::RpcPool::from_config_and_env(config_rpc_urls);

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
            swqos_fallback_hurdle_lamports: config.jito.swqos_fallback_hurdle_lamports,
            swqos_fallback_priority_fee_lamports: config.jito.swqos_fallback_priority_fee_lamports,
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

        let tx_rpc = Arc::new(arb_ingest::TransactionRpcAdapter::new(rpc_pool.clone()));
        let min_sol_reserve_lamports = std::env::var("MIN_SOL_RESERVE_LAMPORTS")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(config.wallet.min_sol_balance_lamports);

        Self {
            mode,
            config,
            ingest,
            graph: parking_lot::Mutex::new(graph),
            risk_gate,
            jito_lander,
            tip_strategy,
            registry,
            dependency_manifests: parking_lot::Mutex::new(
                arb_ingest::DependencyManifestRegistry::default(),
            ),
            recent_landed_cu,
            simulator,
            store,
            intel_persistence,
            telegram,
            keypair,
            last_known_live_sol_price: parking_lot::RwLock::new(0.0),
            rpc_pool,
            health_monitor,
            heartbeat,
            jito_service,
            slot_clock: parking_lot::Mutex::new(SlotClock::new()),
            rtt: parking_lot::Mutex::new((RttTracker::new(300.0), None)),
            blockhash_cache: parking_lot::Mutex::new(None),
            previous_tips: parking_lot::Mutex::new(std::collections::HashMap::new()),
            measured_cu: Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
            kamino_fallback_store: arb_ingest::AccountSnapshotStore::default(),
            capability_registry: Arc::new({
                let mut reg = arb_ingest::ProviderCapabilityRegistry::new();
                reg.register(arb_ingest::ProviderCapabilities::standard_json_rpc_ws(
                    "Helius",
                ));
                reg.register(arb_ingest::ProviderCapabilities::solami_blur());
                reg.register(arb_ingest::ProviderCapabilities::solami_mirage());
                reg
            }),
            tx_rpc,
            min_sol_reserve_lamports,
            wallet_balance_cache: Arc::new(WalletBalanceCache::default()),
            recent_landed: Arc::new(parking_lot::Mutex::new(Vec::new())),
            recent_dropped: Arc::new(parking_lot::Mutex::new(Vec::new())),
            recent_rejected: Arc::new(parking_lot::Mutex::new(Vec::new())),
            control: std::sync::OnceLock::new(),
        }
    }

    /// Isolated HTTP Transaction RPC adapter.
    pub fn tx_rpc(&self) -> &Arc<arb_ingest::TransactionRpcAdapter> {
        &self.tx_rpc
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn keypair(&self) -> Option<&KeypairSigner> {
        self.keypair.as_ref()
    }

    pub fn registry(&self) -> &Arc<parking_lot::RwLock<arb_ingest::MultiDexRegistry>> {
        &self.registry
    }

    pub fn store(&self) -> &Arc<dyn Store> {
        &self.store
    }

    pub fn graph(&self) -> &parking_lot::Mutex<MarketGraph> {
        &self.graph
    }

    pub fn risk_gate(&self) -> &Arc<RiskGate> {
        &self.risk_gate
    }

    pub fn jito_service(&self) -> &Arc<JitoService> {
        &self.jito_service
    }

    pub fn set_last_known_live_sol_price(&self, price: f64) {
        *self.last_known_live_sol_price.write() = price;
    }

    pub fn set_cached_blockhash(&self, bh: [u8; 32], lvbh: Option<i64>) {
        *self.blockhash_cache.lock() = Some((bh, Instant::now(), lvbh));
    }

    pub fn insert_supply_vault_liquidity(&self, slot: Slot, amount: u64) {
        let supply_vault = Pubkey::from_str(arb_constants::KAMINO_WSOL_SUPPLY_VAULT).unwrap();
        let spl = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap();
        let wsol = Pubkey::from_str(arb_tx::WSOL_MINT).unwrap();
        let mut data = vec![0u8; 165];
        data[..32].copy_from_slice(wsol.as_ref());
        data[32..64].copy_from_slice(supply_vault.as_ref());
        data[64..72].copy_from_slice(&amount.to_le_bytes());
        data[108] = 1;
        self.ingest
            .account_store()
            .apply(arb_ingest::AccountSnapshot {
                pubkey: supply_vault,
                owner: spl,
                slot: slot.0,
                write_version: arb_ingest::next_pool_write_version(),
                is_startup: false,
                provider_id: arb_ingest::ACCOUNT_WEBSOCKET_PROVIDER_ID,
                bank_id: Some(1),
                generation: self.ingest.stream_generation(),
                data: std::sync::Arc::new(data),
            });
    }

    pub fn insert_verified_economic_snapshots(&self, slot: Slot, flash_fee_sf: u64) {
        let klend = Pubkey::from_str(arb_constants::KAMINO_KLEND_PROGRAM_ID).unwrap();
        let token_program = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap();
        let wsol = Pubkey::from_str(arb_constants::WSOL_MINT).unwrap();
        let reserve = crate::verified_config::wsol_reserve();

        let mut reserve_data =
            vec![0u8; arb_tx::kamino::KAMINO_RESERVE_FLASH_LOAN_FEE_SF_OFFSET + 8];
        reserve_data[..8].copy_from_slice(&arb_tx::kamino::KAMINO_RESERVE_DISCRIMINATOR);
        let mint_start = arb_tx::kamino::KAMINO_RESERVE_LIQUIDITY_MINT_OFFSET;
        reserve_data[mint_start..mint_start + 32].copy_from_slice(wsol.as_ref());
        reserve_data[arb_tx::kamino::KAMINO_RESERVE_STATUS_OFFSET] =
            arb_tx::kamino::KAMINO_RESERVE_STATUS_ACTIVE;
        reserve_data[arb_tx::kamino::KAMINO_RESERVE_FLASH_LOAN_FEE_SF_OFFSET..][..8]
            .copy_from_slice(&flash_fee_sf.to_le_bytes());

        let mut mint_data = vec![0u8; 82];
        mint_data[44] = 9;
        mint_data[45] = 1;

        let snap_reserve = arb_ingest::AccountSnapshot {
            pubkey: reserve,
            owner: klend,
            slot: slot.0,
            write_version: arb_ingest::next_pool_write_version(),
            is_startup: false,
            provider_id: arb_ingest::ACCOUNT_WEBSOCKET_PROVIDER_ID,
            bank_id: Some(1),
            generation: self.ingest.stream_generation(),
            data: std::sync::Arc::new(reserve_data),
        };
        let snap_mint = arb_ingest::AccountSnapshot {
            pubkey: wsol,
            owner: token_program,
            slot: slot.0,
            write_version: arb_ingest::next_pool_write_version(),
            is_startup: false,
            provider_id: arb_ingest::ACCOUNT_WEBSOCKET_PROVIDER_ID,
            bank_id: Some(1),
            generation: self.ingest.stream_generation(),
            data: std::sync::Arc::new(mint_data),
        };
        self.ingest.account_store().apply(snap_reserve);
        self.ingest.account_store().apply(snap_mint);
    }

    /// Access the underlying risk kill-switch for administrative controls.
    pub fn kill_switch(&self) -> Arc<arb_risk::KillSwitch> {
        self.risk_gate.kill_switch()
    }

    /// Access the asynchronous intelligence persistence queue for zero-hot-path-cost telemetry.
    pub fn intel_persistence(&self) -> Option<Arc<arb_store::AsyncIntelligencePersistence>> {
        self.intel_persistence.clone()
    }

    pub fn ingest(&self) -> Arc<IngestClient> {
        Arc::clone(&self.ingest)
    }

    /// Access the provider capability registry to validate prerequisites (G1, G2).
    pub fn capability_registry(&self) -> Arc<arb_ingest::ProviderCapabilityRegistry> {
        Arc::clone(&self.capability_registry)
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
        self.compute_and_publish_ticker(current_slot);
        control.refresh_deps(crate::control_service::ControlDeps {
            slot: current_slot.0,
            pools_tracked: self.ingest.snapshot().pool_count(),
            stream_connected: self.ingest.stream_connected(),
            helius_connected: self.ingest.helius_connected(),
            route_coverage: self.route_coverage(),
            observability: crate::control_service::ObservabilitySnapshot {
                ws_subscriptions_active: arb_metrics::get_ws_subscriptions_active(),
                ws_reconnects_total: arb_metrics::get_ws_reconnects_total(),
                stale_rejections_total: arb_metrics::get_stale_rejections_total(),
                inventory_rejections_total: arb_metrics::get_inventory_rejections_total(),
                fork_invalidations_total: arb_metrics::get_fork_invalidations_total(),
                resyncs_total: arb_metrics::get_resyncs_total(),
                stream_gaps_total: arb_metrics::get_stream_slot_gaps_total(),
                process_rss_bytes: arb_metrics::get_process_rss_bytes(),
                process_cpu_percent: arb_metrics::get_process_cpu_percent(),
                decode_errors_total: arb_metrics::get_decode_errors_total(),
            },
            providers_health: serde_json::to_value(self.rpc_pool.health_snapshot()).ok(),
        });
    }

    /// Computes real-time token mid-prices across active on-chain pools in MarketStateCache
    /// and feeds the live calculated ticker to ControlService.
    pub(crate) fn compute_and_publish_ticker(
        &self,
        current_slot: Slot,
    ) -> Option<serde_json::Value> {
        let control = self.control()?;
        let market_snap = self.ingest.snapshot();
        let pools = market_snap.snapshot_pools();

        const WSOL_MINT_STR: &str = arb_constants::WSOL_MINT;
        const USDC_MINT_STR: &str = arb_constants::USDC_MINT;
        const USDT_MINT_STR: &str = arb_constants::USDT_MINT;
        const WIF_MINT_STR: &str = arb_constants::WIF_MINT;
        const POPCAT_MINT_STR: &str = arb_constants::POPCAT_MINT;
        const FARTCOIN_MINT_STR: &str = arb_constants::FARTCOIN_MINT;

        let mut sol_prices = Vec::new();
        let mut sol_venues = std::collections::HashSet::new();
        let mut wif_prices = Vec::new();
        let mut wif_venues = std::collections::HashSet::new();
        let mut popcat_prices = Vec::new();
        let mut popcat_venues = std::collections::HashSet::new();
        let mut fartcoin_prices = Vec::new();
        let mut fartcoin_venues = std::collections::HashSet::new();

        for p in &pools {
            if current_slot.0 > 0
                && current_slot.0.saturating_sub(p.last_update_slot)
                    > self.config.strategy.max_state_age_slots
            {
                continue;
            }

            // SOL/USD (WSOL <-> USDC or WSOL <-> USDT)
            if let Some((price, venue)) =
                strategy::extract_pool_mid_price(p, WSOL_MINT_STR, USDC_MINT_STR, 9, 6).or_else(
                    || strategy::extract_pool_mid_price(p, WSOL_MINT_STR, USDT_MINT_STR, 9, 6),
                )
            {
                sol_prices.push(price);
                sol_venues.insert(venue);
            }

            // WIF/SOL (WIF <-> WSOL)
            if let Some((price, venue)) =
                strategy::extract_pool_mid_price(p, WIF_MINT_STR, WSOL_MINT_STR, 6, 9)
            {
                wif_prices.push(price);
                wif_venues.insert(venue);
            }

            // POPCAT/SOL (POPCAT <-> WSOL)
            if let Some((price, venue)) =
                strategy::extract_pool_mid_price(p, POPCAT_MINT_STR, WSOL_MINT_STR, 9, 9)
            {
                popcat_prices.push(price);
                popcat_venues.insert(venue);
            }

            // FARTCOIN/SOL (FARTCOIN <-> WSOL)
            if let Some((price, venue)) =
                strategy::extract_pool_mid_price(p, FARTCOIN_MINT_STR, WSOL_MINT_STR, 6, 9)
            {
                fartcoin_prices.push(price);
                fartcoin_venues.insert(venue);
            }
        }

        let onchain_sol = if !sol_prices.is_empty() {
            Some(sol_prices.iter().sum::<f64>() / (sol_prices.len() as f64))
        } else {
            None
        };

        if let Some(price) = onchain_sol {
            *self.last_known_live_sol_price.write() = price;
            self.telegram.update_sol_price(price);
        } else {
            let mkt_price = self.telegram.sol_price();
            if mkt_price > 0.0 && mkt_price.is_finite() {
                *self.last_known_live_sol_price.write() = mkt_price;
            }
        }

        let live_sol = *self.last_known_live_sol_price.read();
        let (tg_sol, tg_wif_usd, tg_popcat_usd, tg_fartcoin_usd) =
            self.telegram.live_market_prices();
        let effective_sol = if live_sol > 0.0 { live_sol } else { tg_sol };

        let wif_price_sol = if !wif_prices.is_empty() {
            wif_prices.iter().sum::<f64>() / (wif_prices.len() as f64)
        } else if tg_wif_usd > 0.0 && effective_sol > 0.0 {
            tg_wif_usd / effective_sol
        } else {
            0.0
        };

        let popcat_price_sol = if !popcat_prices.is_empty() {
            popcat_prices.iter().sum::<f64>() / (popcat_prices.len() as f64)
        } else if tg_popcat_usd > 0.0 && effective_sol > 0.0 {
            tg_popcat_usd / effective_sol
        } else {
            0.0
        };

        let fartcoin_price_sol = if !fartcoin_prices.is_empty() {
            fartcoin_prices.iter().sum::<f64>() / (fartcoin_prices.len() as f64)
        } else if tg_fartcoin_usd > 0.0 && effective_sol > 0.0 {
            tg_fartcoin_usd / effective_sol
        } else {
            0.0
        };

        let sol_venues_str = if !sol_venues.is_empty() {
            sol_venues.into_iter().collect::<Vec<_>>().join(" • ")
        } else {
            "Orca • Meteora • Raydium".to_string()
        };
        let wif_venues_str = if !wif_venues.is_empty() {
            wif_venues.into_iter().collect::<Vec<_>>().join(" • ")
        } else {
            "Raydium • Orca • Meteora".to_string()
        };
        let popcat_venues_str = if !popcat_venues.is_empty() {
            popcat_venues.into_iter().collect::<Vec<_>>().join(" • ")
        } else {
            "Raydium • Meteora".to_string()
        };
        let fartcoin_venues_str = if !fartcoin_venues.is_empty() {
            fartcoin_venues.into_iter().collect::<Vec<_>>().join(" • ")
        } else {
            "PumpSwap • Raydium • Orca".to_string()
        };

        if effective_sol > 0.0 {
            let live_ticker = serde_json::json!({
                "sol_price_usd": effective_sol,
                "at_unix": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
                "tokens": {
                    "SOL": { "symbol": "SOL/USD", "price": effective_sol, "venues": sol_venues_str, "change_24h": "+0.0%" },
                    "WIF": { "symbol": "WIF/SOL", "price": wif_price_sol, "venues": wif_venues_str, "change_24h": "+0.0%" },
                    "POPCAT": { "symbol": "POPCAT/SOL", "price": popcat_price_sol, "venues": popcat_venues_str, "change_24h": "+0.0%" },
                    "FARTCOIN": { "symbol": "FARTCOIN/SOL", "price": fartcoin_price_sol, "venues": fartcoin_venues_str, "change_24h": "+0.0%" },
                    "USDC": { "symbol": "USDC/USD", "price": 1.000, "venues": "Kamino • Orca", "change_24h": "0.0%" }
                }
            });
            control.update_ticker(live_ticker.clone());
            Some(live_ticker)
        } else {
            None
        }
    }

    /// Evaluates cross-pool route spreads across all matching pools in MarketStateCache
    /// and streams them into ControlService so the live opportunity matrix updates on every slot.
    pub(crate) fn evaluate_and_record_cross_pool_routes(
        &self,
        current_slot: Slot,
        pools: &[PoolState],
        max_sizing: u64,
    ) {
        let Some(control) = self.control() else {
            return;
        };
        let mut recorded_pairs = std::collections::HashSet::new();
        let mut pairs_map: std::collections::HashMap<(Pubkey, Pubkey), Vec<&PoolState>> =
            std::collections::HashMap::new();
        for p in pools {
            let m1 = p.base_mint.0;
            let m2 = p.quote_mint.0;
            let pair_key = if m1 < m2 { (m1, m2) } else { (m2, m1) };
            pairs_map.entry(pair_key).or_default().push(p);
        }

        let test_size = if max_sizing > 0 {
            max_sizing.clamp(1_000_000_000, 100_000_000_000)
        } else {
            10_000_000_000 // 10 SOL default sample evaluation notional
        };

        let mut eval_count = 0u64;
        for ((m1, m2), pool_list) in pairs_map {
            if pool_list.len() < 2 {
                continue;
            }
            for i in 0..pool_list.len() {
                for j in (i + 1)..pool_list.len() {
                    eval_count += 2;
                    let p1 = pool_list[i];
                    let p2 = pool_list[j];
                    if p1.pool_id == p2.pool_id {
                        continue;
                    }

                    let pair_id = (p1.pool_id.0, p2.pool_id.0);
                    if recorded_pairs.contains(&pair_id) {
                        continue;
                    }
                    recorded_pairs.insert(pair_id);

                    let (token_sym, quote_sym) = if m1.to_string() == arb_constants::WSOL_MINT {
                        if m2.to_string() == arb_constants::USDC_MINT {
                            ("SOL", "USDC")
                        } else if m2.to_string() == arb_constants::WIF_MINT {
                            ("WIF", "SOL")
                        } else if m2.to_string() == arb_constants::POPCAT_MINT {
                            ("POPCAT", "SOL")
                        } else if m2.to_string() == arb_constants::FARTCOIN_MINT {
                            ("FARTCOIN", "SOL")
                        } else {
                            ("TOKEN", "SOL")
                        }
                    } else if m2.to_string() == arb_constants::WSOL_MINT {
                        if m1.to_string() == arb_constants::USDC_MINT {
                            ("SOL", "USDC")
                        } else if m1.to_string() == arb_constants::WIF_MINT {
                            ("WIF", "SOL")
                        } else if m1.to_string() == arb_constants::POPCAT_MINT {
                            ("POPCAT", "SOL")
                        } else if m1.to_string() == arb_constants::FARTCOIN_MINT {
                            ("FARTCOIN", "SOL")
                        } else {
                            ("TOKEN", "SOL")
                        }
                    } else {
                        ("TOKEN", "SOL")
                    };

                    let price_1 = strategy::extract_pool_mid_price(
                        p1,
                        &m1.to_string(),
                        &m2.to_string(),
                        p1.decimals_a,
                        p1.decimals_b,
                    )
                    .map(|(p, _)| p)
                    .unwrap_or(0.0);
                    let price_2 = strategy::extract_pool_mid_price(
                        p2,
                        &m1.to_string(),
                        &m2.to_string(),
                        p2.decimals_a,
                        p2.decimals_b,
                    )
                    .map(|(p, _)| p)
                    .unwrap_or(0.0);

                    if price_1 > 0.0 && price_2 > 0.0 {
                        let spread_pct = ((price_1 - price_2).abs() / price_1.min(price_2)) * 100.0;
                        let gross_diff = ((spread_pct / 100.0) * (test_size as f64)) as i64;
                        let out_lamports = test_size.saturating_add(gross_diff.max(0) as u64);
                        let net_diff = gross_diff.saturating_sub(5000);
                        let min_net = self.config.strategy.min_net_profit_abs as i64;
                        let is_eligible = net_diff >= min_net && net_diff > 0;

                        let (buy_pool, sell_pool) = if price_1 <= price_2 {
                            (p1, p2)
                        } else {
                            (p2, p1)
                        };

                        control.record_opportunity(crate::control_service::OpportunityRecord {
                            captured_at_unix: 0,
                            slot: current_slot.0,
                            route: format!(
                                "{} \u{2192} {} [{}/{}]",
                                buy_pool.protocol.as_str(),
                                sell_pool.protocol.as_str(),
                                token_sym,
                                quote_sym
                            ),
                            protocol: format!(
                                "{}-to-{}",
                                buy_pool.protocol.as_str(),
                                sell_pool.protocol.as_str()
                            )
                            .to_lowercase(),
                            borrow_lamports: test_size,
                            expected_output_lamports: out_lamports,
                            gross_profit_lamports: gross_diff,
                            net_profit_lamports: net_diff,
                            eligible: is_eligible,
                            hop_quotes: vec![
                                crate::control_service::HopQuoteRecord {
                                    protocol: buy_pool.protocol.as_str().to_string(),
                                    pool: buy_pool.pool_id.0.to_string(),
                                    amount_in: test_size,
                                    amount_out: test_size,
                                },
                                crate::control_service::HopQuoteRecord {
                                    protocol: sell_pool.protocol.as_str().to_string(),
                                    pool: sell_pool.pool_id.0.to_string(),
                                    amount_in: test_size,
                                    amount_out: out_lamports,
                                },
                            ],
                            reject_reason: if is_eligible {
                                None
                            } else {
                                Some("BELOW_MIN_PROFIT_HURDLE".to_string())
                            },
                        });
                    }
                }
            }
        }
        control.record_evaluations(eval_count.max(1));
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
                DexProtocol::MeteoraDlmm
                | DexProtocol::MeteoraDammV2
                | DexProtocol::MeteoraDbc
                | DexProtocol::MeteoraDlmmPro => meteora_count += 1,
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
        Self::record_landed_trade_task(&self.recent_landed, self.control().cloned(), trade);
    }

    pub(crate) fn record_landed_trade_task(
        recent_landed: &parking_lot::Mutex<Vec<crate::telegram::TradeSummary>>,
        control: Option<Arc<crate::control_service::ControlService>>,
        trade: crate::telegram::TradeSummary,
    ) {
        let mut guard = recent_landed.lock();
        if guard.len() >= 50 {
            guard.remove(0);
        }
        let route = trade.route.clone();
        let signature = trade.signature.clone();
        let slot = trade.slot;
        let profit_sol = trade.profit_sol;
        let sim_ok = trade.sim_ok;
        let tx_size_bytes = trade.tx_size_bytes;
        let cu_consumed = trade.cu_consumed;
        guard.push(trade);
        drop(guard);
        if let Some(control) = control {
            control.record_execution_with(
                "landed",
                &route,
                "trade confirmed landed",
                crate::control_service::ExecutionExtras {
                    signature: Some(signature),
                    slot: Some(slot),
                    net_profit_sol: Some(profit_sol),
                    sim_ok,
                    tx_size_bytes,
                    cu_consumed,
                },
            );
            control.record_execution_result(true);
        }
    }

    pub fn record_dropped_bundle(&self, dropped: crate::telegram::DroppedTradeSummary) {
        Self::record_dropped_bundle_task(&self.recent_dropped, self.control().cloned(), dropped);
    }

    pub(crate) fn record_dropped_bundle_task(
        recent_dropped: &parking_lot::Mutex<Vec<crate::telegram::DroppedTradeSummary>>,
        control: Option<Arc<crate::control_service::ControlService>>,
        dropped: crate::telegram::DroppedTradeSummary,
    ) {
        let mut guard = recent_dropped.lock();
        if guard.len() >= 50 {
            guard.remove(0);
        }
        let route = dropped.route.clone();
        let status = dropped.reason.clone();
        let bundle_id = dropped.bundle_id.clone();
        let slot = dropped.slot;
        let sim_ok = dropped.sim_ok;
        let tx_size_bytes = dropped.tx_size_bytes;
        let cu_consumed = dropped.cu_consumed;
        guard.push(dropped);
        drop(guard);
        if let Some(control) = control {
            control.record_execution_with(
                "dropped",
                &route,
                &status,
                crate::control_service::ExecutionExtras {
                    signature: (!bundle_id.is_empty()).then_some(bundle_id),
                    slot: (slot > 0).then_some(slot),
                    net_profit_sol: None,
                    sim_ok,
                    tx_size_bytes,
                    cu_consumed,
                },
            );
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
            control.record_rejection(&rejected);
        }
        let mut guard = self.recent_rejected.lock();
        if guard.len() >= 100 {
            guard.remove(0);
        }
        guard.push(rejected);
    }

    /// Daily aggregated operations report (checklist 637): durable today-PnL
    /// from the store + rejection-ring breakdown + stream-integrity counters.
    /// Written to `data/daily_report_<date>.json` and pushed to Telegram when
    /// configured (send_message no-ops without credentials).
    pub async fn emit_daily_report(&self, current_slot: u64) {
        let date = crate::daily_report::current_utc_date();
        let pnl = self.store.pnl_daily("today").unwrap_or_else(|e| {
            warn!("daily report: pnl_daily failed: {}", e);
            arb_store::PnlDaily {
                realized_net: SignedLamports::ZERO,
                simulated_net: SignedLamports::ZERO,
                attempts: 0,
                successful: 0,
            }
        });
        let dropped_recent = self.recent_dropped.lock().len() as u64;
        let rejections: Vec<crate::daily_report::RejectionReasoning> = {
            let guard = self.recent_rejected.lock();
            guard
                .iter()
                .map(|r| crate::daily_report::RejectionReasoning {
                    stage: r.stage.clone(),
                    reason: r.reason.clone(),
                })
                .collect()
        };
        let rejected_recent = rejections.len() as u64;
        let report = crate::daily_report::build_daily_report(
            date.clone(),
            current_slot,
            pnl,
            dropped_recent,
            rejected_recent,
            &rejections,
            arb_metrics::get_opportunities_total(),
            arb_metrics::get_stream_slot_gaps_total(),
        );
        let reports_dir = std::path::Path::new("data/reports");
        let _ = std::fs::create_dir_all(reports_dir);
        let path = reports_dir.join(format!("daily_report_{date}.json"));
        match std::fs::write(
            &path,
            serde_json::to_string_pretty(&report).unwrap_or_default(),
        ) {
            Ok(_) => info!("daily report written to {}", path.display()),
            Err(e) => warn!("daily report write failed ({}): {}", path.display(), e),
        }

        // 30-day retention pruning for daily report files
        let retention_days: u64 = std::env::var("DATA_RETENTION_DAYS")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|d| *d > 0)
            .unwrap_or(30);
        let cutoff = std::time::SystemTime::now()
            .checked_sub(Duration::from_secs(retention_days * 86_400))
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        if let Ok(entries) = std::fs::read_dir(reports_dir) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    if let Ok(modified) = meta.modified() {
                        if modified < cutoff {
                            let _ = std::fs::remove_file(entry.path());
                        }
                    }
                }
            }
        }
        self.telegram.send_message(&report.render_text()).await;
    }

    /// Periodic 5-minute digest with detailed lists of successful, dropped, and rejected trades,
    /// along with infrastructure and RPC rate-limit status.
    pub async fn emit_five_minute_digest(&self, current_slot: u64) {
        let digest_interval_secs = std::env::var("TELEGRAM_DIGEST_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|s| *s >= 5)
            .unwrap_or(300);
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
                DexProtocol::MeteoraDlmm
                | DexProtocol::MeteoraDammV2
                | DexProtocol::MeteoraDbc
                | DexProtocol::MeteoraDlmmPro => meteora_count += 1,
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
            interval_secs: digest_interval_secs,
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
        let mut config: Config = toml::from_str(config_str).expect("Valid example config");
        // Hermetic durable store: the example URL points at /app/data
        // (container topology). Point it at a temp file so the fail-closed
        // store selection succeeds without touching system paths.
        let db_path = std::env::temp_dir().join(format!(
            "arb_supervisor_init_{}.db",
            std::process::id() as u64
                + std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos() as u64
        ));
        config.store.url = format!("sqlite://{}", db_path.display());
        let prev_db_url = std::env::var("DATABASE_URL").ok();
        unsafe { std::env::remove_var("DATABASE_URL") };
        let supervisor = Supervisor::new(BotMode::Simulate, config);
        assert_eq!(supervisor.mode(), BotMode::Simulate);
        if let Some(prev) = prev_db_url {
            unsafe { std::env::set_var("DATABASE_URL", prev) };
        }
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn sqlite_path_from_url_resolves_absolute_relative_and_plain_paths() {
        assert_eq!(
            sqlite_path_from_url("sqlite:///app/data/shadow.db"),
            "/app/data/shadow.db"
        );
        assert_eq!(
            sqlite_path_from_url("sqlite:///./data/warm/sentinel_state.db"),
            "./data/warm/sentinel_state.db"
        );
        assert_eq!(
            sqlite_path_from_url("sqlite://./data/warm/sentinel_state.db"),
            "./data/warm/sentinel_state.db"
        );
        assert_eq!(
            sqlite_path_from_url("sqlite://data/shadow.db"),
            "data/shadow.db"
        );
        assert_eq!(
            sqlite_path_from_url("./data/warm/sentinel_state.db"),
            "./data/warm/sentinel_state.db"
        );
    }

    #[test]
    fn open_store_fails_closed_on_unreachable_postgres() {
        // Port 1: connection refused. Must error — never an in-memory store.
        let err = match open_store("postgresql://127.0.0.1:1/nope", None) {
            Ok(_) => panic!("unreachable postgres must fail closed"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("fail-closed"));
    }

    #[test]
    fn open_store_fails_closed_when_database_url_postgres_unreachable() {
        // DATABASE_URL takes precedence over a valid sqlite store_url and
        // must fail closed rather than silently falling back.
        let db_path = std::env::temp_dir().join(format!(
            "arb_store_prec_{}.db",
            std::process::id() as u64
                + std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos() as u64
        ));
        let store_url = format!("sqlite://{}", db_path.display());
        let err = match open_store(&store_url, Some("postgresql://127.0.0.1:1/nope")) {
            Ok(_) => panic!("unreachable DATABASE_URL postgres must fail closed"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("fail-closed"));
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn open_store_sqlite_url_is_durable_file_backed() {
        let db_path = std::env::temp_dir().join(format!(
            "arb_store_durable_{}.db",
            std::process::id() as u64
                + std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos() as u64
        ));
        let url = format!("sqlite://{}", db_path.display());
        let store = open_store(&url, None).expect("sqlite url must open a durable file store");
        store
            .record_attempt(&AttemptRecord {
                id: 1,
                slot: 1,
                route_id: "r".into(),
                expected_net: Lamports(1),
                tip_lamports: None,
                decision: "test".into(),
                ..Default::default()
            })
            .expect("write must succeed");
        drop(store);
        assert!(db_path.exists(), "store must be file-backed, not in-memory");
        // Data survives close/reopen: durable, not memory-resident.
        let reopened = open_store(&url, None).expect("reopen");
        assert_eq!(reopened.get_attempt(1).expect("read").expect("row").id, 1);
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn open_store_custom_sqlite_url_takes_precedence_over_database_url_sqlite() {
        let db_path_custom = std::env::temp_dir().join(format!(
            "arb_store_custom_{}.db",
            std::process::id() as u64
                + std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos() as u64
        ));
        let db_path_env = std::env::temp_dir().join(format!(
            "arb_store_env_{}.db",
            std::process::id() as u64
                + std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos() as u64
                + 1
        ));
        let custom_url = format!("sqlite://{}", db_path_custom.display());
        let env_url = format!("sqlite://{}", db_path_env.display());

        let store = open_store(&custom_url, Some(&env_url)).expect("must open custom sqlite store");
        store
            .record_attempt(&AttemptRecord {
                id: 42,
                slot: 100,
                route_id: "custom_isolation".into(),
                expected_net: Lamports(100),
                tip_lamports: None,
                decision: "test".into(),
                ..Default::default()
            })
            .expect("record attempt");
        drop(store);

        assert!(db_path_custom.exists(), "custom store file must exist");
        assert!(
            !db_path_env.exists(),
            "env store file must NOT be written when custom store_url is provided"
        );

        let _ = std::fs::remove_file(&db_path_custom);
        let _ = std::fs::remove_file(&db_path_env);
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

#[cfg(all(test, feature = "shadow-only"))]
mod shadow_only_signer_tests {
    use super::load_wallet_signer;

    #[test]
    fn shadow_only_build_never_loads_wallet_signer() {
        let config: crate::config::Config =
            toml::from_str(include_str!("../../../config/config.example.toml"))
                .expect("valid example config");
        assert!(load_wallet_signer(&config).is_none());
    }
}

#[cfg(test)]
mod wallet_balance_cache_tests {
    use super::WalletBalanceCache;
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    #[test]
    fn wallet_balance_cache_returns_only_fresh_healthy_snapshots() {
        let cache = WalletBalanceCache::default();
        assert_eq!(cache.snapshot(Duration::from_secs(15)), None);

        cache.update(75_000_000);
        assert_eq!(cache.snapshot(Duration::from_secs(15)), Some(75_000_000));

        cache.invalidate();
        assert_eq!(cache.snapshot(Duration::from_secs(15)), None);
    }

    #[test]
    fn wallet_balance_cache_rejects_stale_snapshots() {
        let cache = WalletBalanceCache::default();
        cache.update(75_000_000);
        cache.refreshed_at_ms.store(1, Ordering::Release);
        assert_eq!(cache.snapshot(Duration::from_secs(1)), None);
    }
}
