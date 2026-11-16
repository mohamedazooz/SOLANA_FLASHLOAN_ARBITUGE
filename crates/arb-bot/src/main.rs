//! Entry point for the Solana Arbitrage Bot.
//!
//! M0 — Foundation & Bootstrap: CLI configuration, logging, safety checks.

use anyhow::{Context, Result};
use arb_config::{loader as arb_config_loader, AppConfig as ArbAppConfig};
use std::fs::OpenOptions;
use std::sync::{Arc, Mutex};
use tracing::{info, warn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, fmt};

pub mod errors;
pub mod health_monitor;
pub mod jito_service;
pub mod supervisor;
pub mod telegram;
pub mod verified_config;

/// Supported operational modes for the bot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BotMode {
    Shadow,
    Simulate,
    Live,
    Canary,
}

impl std::str::FromStr for BotMode {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "shadow" => Ok(BotMode::Shadow),
            "simulate" => Ok(BotMode::Simulate),
            "live" => Ok(BotMode::Live),
            "canary" => Ok(BotMode::Canary),
            other => anyhow::bail!(
                "Unknown bot mode: '{other}'. Allowed modes: shadow, simulate, live, canary"
            ),
        }
    }
}

impl std::fmt::Display for BotMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BotMode::Shadow => write!(f, "shadow"),
            BotMode::Simulate => write!(f, "simulate"),
            BotMode::Live => write!(f, "live"),
            BotMode::Canary => write!(f, "canary"),
        }
    }
}

/// LIVE execution is opt-in only when `ARB_LIVE` is explicitly truthy.
///
/// Presence alone must never authorize live trading: `.env.example` ships
/// `ARB_LIVE=0`, and a truthiness bug here would enable signing/broadcasting.
pub(crate) fn live_execution_enabled() -> bool {
    is_truthy_flag(std::env::var("ARB_LIVE").ok().as_deref())
}

fn is_truthy_flag(value: Option<&str>) -> bool {
    matches!(
        value.map(str::trim),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("on")
    )
}

struct Args {
    config_path: String,
    mode: BotMode,
}

fn parse_args() -> Result<Args> {
    let mut args = std::env::args().skip(1);
    let mut config_path = std::env::var("ARB_CONFIG").unwrap_or_default();
    let mut cli_mode: Option<String> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!(
                    "Solana Arbitrage Bot\n\nUsage: arb-bot [OPTIONS]\n\nOptions:\n  -c, --config <PATH>  Path to configuration TOML [default: auto-detected]\n  -m, --mode <MODE>    Bot mode: shadow | simulate | live [default: shadow]\n  -h, --help           Print help"
                );
                std::process::exit(0);
            }
            "--config" | "-c" => {
                config_path = args.next().context("Missing argument after --config")?;
            }
            "--mode" | "-m" => {
                cli_mode = Some(args.next().context("Missing argument after --mode")?);
            }
            other => anyhow::bail!("Unknown argument: '{other}'"),
        }
    }

    if config_path.is_empty() {
        if std::path::Path::new("config.toml").exists() {
            config_path = "config.toml".to_string();
        } else if std::path::Path::new("config/config.toml").exists() {
            config_path = "config/config.toml".to_string();
        } else if std::path::Path::new("config/config.example.toml").exists() {
            config_path = "config/config.example.toml".to_string();
        } else {
            config_path = "config.toml".to_string();
        }
    }

    // Determine mode precedence:
    // 1. Explicit CLI argument `--mode`
    // 2. Environment variable `ARB_MODE` / `ARB_CANARY`
    // 3. `bot.mode` configured in the target config.toml file
    // 4. Safe fallback: "shadow"
    let mode_str = if let Some(m) = cli_mode {
        m
    } else if std::env::var("ARB_CANARY").is_ok() {
        "canary".to_string()
    } else if let Ok(m) = std::env::var("ARB_MODE") {
        m
    } else if let Ok(contents) = std::fs::read_to_string(&config_path) {
        #[derive(serde::Deserialize)]
        struct PartialConfig {
            bot: Option<PartialBot>,
        }
        #[derive(serde::Deserialize)]
        struct PartialBot {
            mode: Option<String>,
        }
        toml::from_str::<PartialConfig>(&contents)
            .ok()
            .and_then(|c| c.bot)
            .and_then(|b| b.mode)
            .unwrap_or_else(|| "shadow".to_string())
    } else {
        "shadow".to_string()
    };

    let mode: BotMode = mode_str.parse()?;
    Ok(Args { config_path, mode })
}

/// Bot configuration loaded from TOML file.
pub mod config {
    use serde::Deserialize;
    use std::fs;

    #[derive(Debug, Deserialize, Clone)]
    pub struct Config {
        pub bot: BotConfig,
        pub grpc: GrpcConfig,
        pub rpc: RpcConfig,
        pub jito: JitoConfig,
        pub jupiter: JupiterConfig,
        pub wallet: WalletConfig,
        pub strategy: StrategyConfig,
        pub risk: RiskConfig,
        pub store: StoreConfig,
        pub metrics: MetricsConfig,
        pub admin: AdminConfig,
        #[serde(default)]
        pub chronos: ChronosConfig,
    }

    #[derive(Debug, Deserialize, Clone)]
    pub struct ChronosConfig {
        #[serde(default = "default_true")]
        pub enabled: bool,
        #[serde(default = "default_lambda")]
        pub decay_lambda_per_sec: f64,
        #[serde(default = "default_max_age_ms")]
        pub max_age_ms: u64,
        #[serde(default = "default_min_ev_usd")]
        pub min_ev_usd: f64,
        #[serde(default = "default_failed_cost")]
        pub failed_attempt_cost_lamports: u64,
        #[serde(default = "default_beta_risk")]
        pub beta_risk_factor: f64,
    }

    fn default_true() -> bool {
        true
    }
    fn default_lambda() -> f64 {
        3.0
    }
    fn default_max_age_ms() -> u64 {
        400
    }
    fn default_min_ev_usd() -> f64 {
        0.05
    }
    fn default_failed_cost() -> u64 {
        10_000
    }
    fn default_beta_risk() -> f64 {
        0.0005
    }
    fn default_keep_share() -> f64 {
        0.85
    }
    fn default_ladder_growth() -> f64 {
        1.6
    }
    fn default_tip_floor_max_age_ms() -> u64 {
        arb_land::DEFAULT_MAX_AGE_MS
    }
    fn default_tip_floor_poll_ms() -> u64 {
        1_000
    }
    fn default_leader_gate_enabled() -> bool {
        true
    }
    fn default_leader_refresh_secs() -> u64 {
        60
    }

    impl Default for ChronosConfig {
        fn default() -> Self {
            Self {
                enabled: true,
                decay_lambda_per_sec: 3.0,
                max_age_ms: 400,
                min_ev_usd: 0.05,
                failed_attempt_cost_lamports: 10_000,
                beta_risk_factor: 0.0005,
            }
        }
    }

    #[derive(Debug, Deserialize, Clone)]
    pub struct GrpcConfig {
        pub primary_url: String,
        pub primary_token_env: String,
        pub failover_url: Option<String>,
        pub failover_token_env: Option<String>,
        pub commitment: String,
        pub ping_reply: bool,
    }

    #[derive(Debug, Deserialize, Clone)]
    pub struct RpcConfig {
        pub url: String,
        pub commitment: String,
    }

    #[derive(Debug, Deserialize, Clone)]
    pub struct JitoConfig {
        pub regions: Vec<String>,
        pub tip_floor_percentile: String,
        pub tip_share_bps: u64,
        pub max_tip_lamports: u64,
        pub max_tip_share_bps: u64,
        pub min_keep_lamports: u64,

        // --- Adaptive tip pricing -------------------------------------------
        // Every field below defaults, so an existing `config.toml` keeps
        // parsing unchanged and the operator opts in by setting a value.
        /// Fraction of the edge that must survive every cost, tip included.
        #[serde(default = "default_keep_share")]
        pub keep_share: f64,
        /// Multiplicative cap on how far a retry may raise the previous tip.
        #[serde(default = "default_ladder_growth")]
        pub ladder_growth: f64,
        /// Age past which a tip-floor snapshot is treated as absent.
        #[serde(default = "default_tip_floor_max_age_ms")]
        pub tip_floor_max_age_ms: u64,
        /// Tip-floor poll interval; clamped to the endpoint's 1 rps per-IP limit.
        #[serde(default = "default_tip_floor_poll_ms")]
        pub tip_floor_poll_ms: u64,
        /// Refuse to tip when the slot's leader is not known to run Jito-Solana.
        #[serde(default = "default_leader_gate_enabled")]
        pub leader_gate_enabled: bool,
        /// Validator registry endpoint. `None` uses the public kobe deployment.
        #[serde(default)]
        pub kobe_url: Option<String>,
        /// Where to persist the landing ledger. `None` keeps it in memory only.
        #[serde(default)]
        pub ledger_path: Option<String>,
        /// Interval between leader-schedule refreshes, in seconds.
        #[serde(default = "default_leader_refresh_secs")]
        pub leader_refresh_secs: u64,
    }

    #[derive(Debug, Deserialize, Clone)]
    pub struct JupiterConfig {
        pub base_url: String,
        pub api_key_env: String,
        pub timeout_ms: u64,
    }

    #[derive(Debug, Deserialize, Clone)]
    pub struct WalletConfig {
        pub keypair_path: String,
        pub cold_wallet_pubkey: Option<String>,
        pub min_sol_balance_lamports: u64,
    }

    #[derive(Debug, Deserialize, Clone)]
    pub struct StrategyConfig {
        pub base_tokens: Vec<String>,
        pub min_net_profit_abs: u64,
        pub min_net_profit_bps: u64,
        pub max_price_impact_bps: u64,
        pub max_state_age_slots: u64,
        /// Maximum slot age for a pool to enter the *discovery* graph. `None`
        /// means "same policy as quote freshness" (`max_state_age_slots`).
        /// This is an explicit policy, never an arbitrary multiplier.
        #[serde(default)]
        pub graph_max_age_slots: Option<u64>,
        pub use_flash_loan: bool,
        /// Hard cap on dynamically discovered pools (each costs account reads
        /// every tick). `None` uses the discovery default.
        pub max_pools: Option<usize>,
        /// How often the pool catalog is refreshed, in seconds. `None` uses 300.
        pub discovery_refresh_secs: Option<u64>,
        /// Minimum TVL (USD) for a discovered pool. `None` uses the default.
        pub discovery_min_tvl_usd: Option<f64>,
    }

    fn default_max_orders_per_hour() -> usize {
        10
    }

    fn default_cooldown_slots() -> u64 {
        75
    }

    #[derive(Debug, Deserialize, Clone)]
    pub struct RiskConfig {
        pub max_trade_size_usd: f64,
        pub max_daily_loss_usd: f64,
        pub max_hourly_loss_usd: f64,
        pub max_consecutive_failures: u64,
        pub min_landing_rate: f64,
        pub max_attempts_per_slot: u32,
        pub max_inflight_bundles: u32,
        pub max_tip_spend_ratio: f64,
        #[serde(default = "default_max_orders_per_hour")]
        pub max_orders_per_hour: usize,
        #[serde(default = "default_cooldown_slots")]
        pub cooldown_slots: u64,
    }

    #[derive(Debug, Deserialize, Clone)]
    pub struct StoreConfig {
        pub url: String,
    }

    #[derive(Debug, Deserialize, Clone)]
    pub struct MetricsConfig {
        pub listen: String,
    }

    #[derive(Debug, Deserialize, Clone)]
    pub struct AdminConfig {
        pub listen: String,
        pub token_env: String,
    }

    #[derive(Debug, Deserialize, Clone)]
    pub struct BotConfig {
        pub mode: String,
        #[serde(default)]
        pub canary: bool,
    }

    pub fn load(path: &str) -> anyhow::Result<Config> {
        use anyhow::Context;
        let contents = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file '{path}'"))?;
        let mut config: Config =
            toml::from_str(&contents).with_context(|| fmt_parse_error(path))?;

        // The deployment environment is the source of truth for the notional cap.
        if let Ok(raw) = std::env::var("MAX_TRADE_SIZE_USD") {
            let value: f64 = raw.parse().context("MAX_TRADE_SIZE_USD must be numeric")?;
            if !value.is_finite() || value <= 0.0 {
                anyhow::bail!("MAX_TRADE_SIZE_USD must be positive and finite");
            }
            config.risk.max_trade_size_usd = value;
        }
        if let Ok(raw) = std::env::var("CHRONOS_DECAY_LAMBDA_PER_SEC") {
            let value: f64 = raw
                .parse()
                .context("CHRONOS_DECAY_LAMBDA_PER_SEC must be numeric")?;
            if !value.is_finite() || value < 0.0 {
                anyhow::bail!("CHRONOS_DECAY_LAMBDA_PER_SEC must be finite and non-negative");
            }
            config.chronos.decay_lambda_per_sec = value;
        }
        if let Ok(raw) = std::env::var("CHRONOS_MAX_AGE_MS") {
            config.chronos.max_age_ms = raw
                .parse()
                .context("CHRONOS_MAX_AGE_MS must be an integer")?;
        }
        if let Ok(raw) = std::env::var("CHRONOS_MIN_EV_USD") {
            let value: f64 = raw.parse().context("CHRONOS_MIN_EV_USD must be numeric")?;
            if !value.is_finite() || value < 0.0 {
                anyhow::bail!("CHRONOS_MIN_EV_USD must be finite and non-negative");
            }
            config.chronos.min_ev_usd = value;
        }

        if config.risk.max_tip_spend_ratio > 1.0 {
            anyhow::bail!("max_tip_spend_ratio must be <= 1.0");
        }

        // —— ChronosConfig validation ——
        if config.chronos.max_age_ms < 100 || config.chronos.max_age_ms > 5000 {
            anyhow::bail!(
                "chronos.max_age_ms must be between 100 and 5000 ms, got {}",
                config.chronos.max_age_ms
            );
        }
        if config.chronos.decay_lambda_per_sec < 0.1 || config.chronos.decay_lambda_per_sec > 100.0
        {
            anyhow::bail!(
                "chronos.decay_lambda_per_sec must be between 0.1 and 100.0, got {}",
                config.chronos.decay_lambda_per_sec
            );
        }
        if config.chronos.min_ev_usd < 0.0 {
            anyhow::bail!(
                "chronos.min_ev_usd must be >= 0.0, got {}",
                config.chronos.min_ev_usd
            );
        }

        if config.strategy.use_flash_loan {
            use tracing::info;
            info!("⚡ Flash Loan enabled — zero-risk atomic flash loan execution active");
        }

        Ok(config)
    }

    fn fmt_parse_error(path: &str) -> anyhow::Error {
        anyhow::anyhow!("Failed to parse TOML configuration from '{path}'")
    }

    /// Convert the strongly-typed [`arb_config::AppConfig`] into the legacy
    /// flat [`config::Config`] consumed by [`Supervisor::new`].
    ///
    /// This is the Phase 3 adaptation layer: `arb-config` is now the single
    /// validated loading authority, and the legacy struct is a thin adapter
    /// kept so the supervisor's internal types and behavior are unchanged.
    /// All validation has already run in [`arb_config::loader::load`].
    pub(super) fn from_arb_config(arb: ArbAppConfig) -> config::Config {
        let usd = |micros: arb_types::UsdMicros| micros.0 as f64 / 1_000_000.0;
        config::Config {
            bot: config::BotConfig {
                mode: arb.mode.mode.to_string(),
                canary: matches!(arb.mode.mode, arb_config::Mode::Canary),
            },
            grpc: config::GrpcConfig {
                primary_url: arb.provider.primary_url,
                primary_token_env: arb.provider.primary_token_env,
                failover_url: arb.provider.failover_url,
                failover_token_env: arb.provider.failover_token_env,
                commitment: arb.provider.commitment,
                ping_reply: arb.provider.ping_reply,
            },
            rpc: config::RpcConfig {
                url: arb.provider.primary_url.clone(),
                commitment: arb.provider.commitment.clone(),
            },
            jito: config::JitoConfig {
                regions: arb.jito.regions,
                tip_floor_percentile: arb.jito.tip_floor_percentile,
                tip_share_bps: arb.jito.tip_share_bps.0,
                max_tip_lamports: arb.jito.max_tip_lamports.0,
                max_tip_share_bps: arb.jito.max_tip_share_bps.0,
                min_keep_lamports: arb.jito.min_keep_lamports.0,
                keep_share: arb.jito.keep_share,
                ladder_growth: arb.jito.ladder_growth,
                tip_floor_max_age_ms: arb.jito.tip_floor_max_age_ms.0,
                tip_floor_poll_ms: arb.jito.tip_floor_poll_ms.0,
                leader_gate_enabled: arb.jito.leader_gate_enabled,
                kobe_url: arb.jito.kobe_url,
                ledger_path: arb.jito.ledger_path,
                leader_refresh_secs: arb.jito.leader_refresh_secs.0,
            },
            jupiter: config::JupiterConfig {
                base_url: arb.jupiter.base_url,
                api_key_env: arb.jupiter.api_key_env,
                timeout_ms: arb.jupiter.timeout_ms.0,
            },
            wallet: config::WalletConfig {
                keypair_path: arb.wallet.keypair_path,
                cold_wallet_pubkey: arb.wallet.cold_wallet_pubkey,
                min_sol_balance_lamports: arb.wallet.min_sol_balance_lamports.0,
            },
            strategy: config::StrategyConfig {
                base_tokens: arb.strategy.base_tokens,
                min_net_profit_abs: arb.strategy.min_net_profit_abs.0,
                min_net_profit_bps: arb.strategy.min_net_profit_bps.0,
                max_price_impact_bps: arb.strategy.max_price_impact_bps.0,
                max_state_age_slots: arb.strategy.max_state_age_slots.0,
                graph_max_age_slots: arb
                    .strategy
                    .graph_max_age_slots
                    .map(|s| s.0),
                use_flash_loan: arb.strategy.use_flash_loan,
                max_pools: arb.strategy.max_pools,
                discovery_refresh_secs: arb
                    .strategy
                    .discovery_refresh_secs
                    .map(|s| s.0),
                discovery_min_tvl_usd: arb
                    .strategy
                    .discovery_min_tvl_usd
                    .map(usd),
            },
            risk: config::RiskConfig {
                max_trade_size_usd: usd(arb.risk.max_trade_size_usd),
                max_daily_loss_usd: usd(arb.risk.max_daily_loss_usd),
                max_hourly_loss_usd: usd(arb.risk.max_hourly_loss_usd),
                max_consecutive_failures: arb.risk.max_consecutive_failures,
                min_landing_rate: arb.risk.min_landing_rate,
                max_attempts_per_slot: arb.risk.max_attempts_per_slot,
                max_inflight_bundles: arb.risk.max_inflight_bundles as u32,
                max_tip_spend_ratio: arb.risk.max_tip_spend_ratio,
                max_orders_per_hour: arb.risk.max_orders_per_hour,
                cooldown_slots: arb.risk.cooldown_slots.0,
            },
            store: config::StoreConfig {
                url: arb.store.url,
            },
            metrics: config::MetricsConfig {
                listen: arb.metrics.listen.to_string(),
            },
            admin: config::AdminConfig {
                listen: arb.admin.listen.to_string(),
                token_env: arb.admin.token_env,
            },
            chronos: config::ChronosConfig {
                enabled: arb.chronos.enabled,
                decay_lambda_per_sec: arb.chronos.decay_lambda_per_sec,
                max_age_ms: arb.chronos.max_age_ms.0,
                min_ev_usd: usd(arb.chronos.min_ev_usd),
                failed_attempt_cost_lamports: arb
                    .chronos
                    .failed_attempt_cost_lamports
                    .0,
                beta_risk_factor: arb.chronos.beta_risk_factor,
            },
        }
    }
}

fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("FATAL PANIC: {info}");
        default_hook(info);
    }));
}

#[derive(Clone)]
struct FileMakeWriter {
    file: Arc<Mutex<std::fs::File>>,
}

impl std::io::Write for FileMakeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut f = self
            .file
            .lock()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        f.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let mut f = self
            .file
            .lock()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        f.flush()
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for FileMakeWriter {
    type Writer = Self;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn init_event_logger() {
    let log_file_path =
        std::env::var("ARB_EVENT_LOG").unwrap_or_else(|_| "logs/arb_events.log".to_string());

    if let Some(parent) = std::path::Path::new(&log_file_path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let default_filter = "info,arb_bot=debug,arb_engine=debug,arb_ingest=info,arb_tx=debug,arb_land=debug,arb_risk=debug,arb_sim=debug";
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));

    let stdout_layer = fmt::layer().with_target(true).with_ansi(true);

    match OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file_path)
    {
        Ok(file) => {
            let file_writer = FileMakeWriter {
                file: Arc::new(Mutex::new(file)),
            };
            let file_layer = fmt::layer()
                .with_target(true)
                .with_ansi(false)
                .with_writer(file_writer);

            tracing_subscriber::registry()
                .with(env_filter)
                .with(stdout_layer)
                .with(file_layer)
                .init();
            tracing::info!(
                "Detailed event logging active: console + file '{}'",
                log_file_path
            );
        }
        Err(e) => {
            tracing_subscriber::registry()
                .with(env_filter)
                .with(stdout_layer)
                .init();
            tracing::warn!(
                "Failed to open log file '{}': {}. Logging to stdout only.",
                log_file_path,
                e
            );
        }
    }
}

/// True when a secret is absent or obviously a placeholder. Placeholders must
/// never authorize an admin action (SEC-02 / Gate E).
fn is_placeholder_secret(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "" | "changeme" | "change-me" | "placeholder" | "your_token_here" | "token"
    )
}

/// Constant-time comparison for admin bearer tokens. Length is not secret, but
/// the byte comparison must not short-circuit on the first mismatch.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Accepts `Authorization: Bearer <token>` or `X-Admin-Token: <token>`.
fn admin_authorized(headers: &axum::http::HeaderMap, expected: &str) -> bool {
    let provided = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .or_else(|| headers.get("x-admin-token").and_then(|v| v.to_str().ok()));
    provided.is_some_and(|p| constant_time_eq(p.as_bytes(), expected.as_bytes()))
}

#[tokio::main]
async fn main() -> Result<()> {
    install_panic_hook();
    if dotenvy::dotenv().is_ok() {
        // .env loaded
    }
    let args = parse_args()?;

    init_event_logger();

    info!("Starting arb-bot in mode: {}", args.mode);

    let config = config::load(&args.config_path)
        .with_context(|| format!("Failed to load configuration from '{}'", args.config_path))?;

    info!("Configuration loaded — gRPC: {}", config.grpc.primary_url);
    info!("Max trade size: {} USD", config.risk.max_trade_size_usd);
    info!(
        "Min net profit: {} lamports ({} bps)",
        config.strategy.min_net_profit_abs, config.strategy.min_net_profit_bps
    );

    if args.mode == BotMode::Live {
        if !live_execution_enabled() {
            anyhow::bail!("Mode live requires ARB_LIVE=1 (explicit truthy value).");
        }
        warn!("LIVE execution mode active on real network funds!");
    } else {
        info!("Running in safe mode (dry-run / shadow). No live transactions will be submitted.");
    }

    // Start Prometheus text-exporter metrics server
    let metrics_server = arb_metrics::MetricsServer::new(&config.metrics.listen);
    let metrics_handle = metrics_server.start();
    info!(
        "📈 Prometheus Metrics Server running on http://{}/metrics",
        config.metrics.listen
    );

    info!("TRACE MAIN 1: before Supervisor::new");
    let admin_listen = config.admin.listen.clone();
    let admin_token_env = config.admin.token_env.clone();
    let supervisor = supervisor::Supervisor::new(args.mode, config);
    let admin_kill_switch = supervisor.kill_switch();

    // Operator authentication is mandatory for every mutating admin route.
    // A missing or placeholder token disables the admin listener entirely
    // (fail closed) instead of exposing /pause and /resume unauthenticated.
    match std::env::var(&admin_token_env) {
        Ok(token) if !token.trim().is_empty() && !is_placeholder_secret(&token) => {
            // Start Admin HTTP Server on configured port (e.g. 127.0.0.1:9101)
            tokio::spawn(async move {
                let ks_pause = Arc::clone(&admin_kill_switch);
                let ks_resume = Arc::clone(&admin_kill_switch);
                let ks_status = Arc::clone(&admin_kill_switch);
                // Each route closure owns its own copy; the token value itself is
                // never logged or moved twice.
                let pause_token = token.clone();
                let resume_token = token;

                let app = axum::Router::new()
                    .route("/health", axum::routing::get(|| async { "OK\n" }))
                    .route(
                        "/status",
                        axum::routing::get(move || {
                            let ks = Arc::clone(&ks_status);
                            async move {
                                if ks.is_blocked() {
                                    "PAUSED\n"
                                } else {
                                    "RUNNING\n"
                                }
                            }
                        }),
                    )
                    .route(
                        "/pause",
                        axum::routing::post(move |headers: axum::http::HeaderMap| {
                            let ks = Arc::clone(&ks_pause);
                            let expected = pause_token.clone();
                            async move {
                                if !admin_authorized(&headers, &expected) {
                                    warn!("admin: rejected unauthenticated /pause request");
                                    return (
                                        axum::http::StatusCode::UNAUTHORIZED,
                                        "unauthorized\n",
                                    );
                                }
                                ks.activate();
                                info!("admin: kill switch activated via authenticated /pause");
                                (
                                    axum::http::StatusCode::OK,
                                    "Bot trading paused via admin API\n",
                                )
                            }
                        }),
                    )
                    .route(
                        "/resume",
                        axum::routing::post(move |headers: axum::http::HeaderMap| {
                            let ks = Arc::clone(&ks_resume);
                            let expected = resume_token.clone();
                            async move {
                                if !admin_authorized(&headers, &expected) {
                                    warn!("admin: rejected unauthenticated /resume request");
                                    return (
                                        axum::http::StatusCode::UNAUTHORIZED,
                                        "unauthorized\n",
                                    );
                                }
                                ks.deactivate();
                                info!("admin: kill switch cleared via authenticated /resume");
                                (
                                    axum::http::StatusCode::OK,
                                    "Bot trading resumed via admin API\n",
                                )
                            }
                        }),
                    );

                match tokio::net::TcpListener::bind(&admin_listen).await {
                    Ok(listener) => {
                        info!(
                            "🛠️ Admin HTTP Server running on http://{admin_listen} (token required)"
                        );
                        let _ = axum::serve(listener, app).await;
                    }
                    Err(e) => {
                        warn!("Could not bind Admin HTTP Server on {admin_listen}: {e}");
                    }
                }
            });
        }
        _ => {
            warn!(
                "Admin HTTP API disabled: env var '{}' is missing or a placeholder; /pause and /resume stay unavailable (fail closed)",
                admin_token_env
            );
        }
    }

    info!("TRACE MAIN 2: after Supervisor::new, before run");
    supervisor.run().await?;
    info!("TRACE MAIN 3: after run");

    metrics_handle.shutdown().await;

    info!("arb-bot exited cleanly.");
    Ok(())
}

#[cfg(test)]
mod flag_tests {
    use super::{admin_authorized, constant_time_eq, is_placeholder_secret, is_truthy_flag};

    #[test]
    fn arb_live_requires_an_explicit_truthy_value() {
        assert!(is_truthy_flag(Some("1")));
        assert!(is_truthy_flag(Some("true")));
        assert!(is_truthy_flag(Some(" TRUE ")));
        // Presence or falsy values must never authorize live execution.
        assert!(!is_truthy_flag(Some("0")));
        assert!(!is_truthy_flag(Some("false")));
        assert!(!is_truthy_flag(Some("")));
        assert!(!is_truthy_flag(None));
    }

    #[test]
    fn placeholder_secrets_never_authorize_admin() {
        for placeholder in ["", "  ", "changeme", "CHANGE-ME", "placeholder", "token"] {
            assert!(
                is_placeholder_secret(placeholder),
                "expected {placeholder:?} to be treated as a placeholder"
            );
        }
        assert!(!is_placeholder_secret("s3cret-operator-token"));
    }

    #[test]
    fn constant_time_eq_matches_exact_bytes_only() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        // Length mismatch is a mismatch, never a panic.
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"", b"x"));
    }

    #[test]
    fn admin_authorized_accepts_bearer_and_x_admin_token() {
        let mut headers = axum::http::HeaderMap::new();
        assert!(!admin_authorized(&headers, "secret-token"));

        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Bearer secret-token"),
        );
        assert!(admin_authorized(&headers, "secret-token"));
        assert!(!admin_authorized(&headers, "other-token"));

        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "x-admin-token",
            axum::http::HeaderValue::from_static("secret-token"),
        );
        assert!(admin_authorized(&headers, "secret-token"));
        assert!(!admin_authorized(&headers, "other-token"));

        // A non-Bearer Authorization header must not be accepted.
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderValue::from_static("Basic secret-token"),
        );
        assert!(!admin_authorized(&headers, "secret-token"));
    }
}
