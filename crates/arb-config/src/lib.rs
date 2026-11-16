//! Strongly-typed configuration system for the Solana Arbitrage Bot.
//!
//! Design goals (from the config refactor mission):
//! - One authoritative loading architecture: compiled protocol invariants → base
//!   TOML → environment-specific TOML → environment variables → CLI overrides.
//! - Operator configuration can never override protocol invariants or mandatory
//!   safety limits.
//! - Every field has an explicit typed unit, validation, documented meaning,
//!   safe operational range, and optional environment override.
//! - Invalid values produce actionable errors that identify the config key
//!   without revealing secrets. NaN, Infinity, negative/overflowing values are
//!   rejected; nothing is silently clamped.
//! - Secrets are never stored in plaintext in config files; secret env names are
//!   declared here but values are resolved through a secret provider.
//!
//! Newtypes (units) are imported from `arb-types` where they already exist
//! (Lamports, TokenAmount, Bps, Slot, ComputeUnits, UsdMicros, Milliseconds,
//! Seconds) and reused here so units are never ambiguous.

use arb_types::{Bps, Lamports, Milliseconds, Seconds, Slot, UsdMicros};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use thiserror::Error;

pub mod loader;
pub mod validation;

/// Top-level application configuration.
///
/// Loaded via [`loader::load`], which applies the precedence:
///   1. compiled protocol invariants (arb-constants)
///   2. base.toml
///   3. environment-specific TOML (development/staging/production)
///   4. environment variables
///   5. CLI overrides
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Deployment mode: shadow | simulate | canary | live.
    /// `live` additionally requires `execution.live_enabled` to be true and all
    /// mandatory security controls to pass validation.
    pub mode: ModeConfig,
    /// RPC / gRPC provider endpoints and credentials.
    pub provider: ProviderConfig,
    /// Jito block-engine / tip configuration.
    pub jito: JitoConfig,
    /// Jupiter swap API configuration.
    pub jupiter: JupiterConfig,
    /// Wallet / keypair configuration.
    pub wallet: WalletConfig,
    /// Strategy / routing parameters.
    pub strategy: StrategyConfig,
    /// Risk limits (the authoritative safety policy).
    pub risk: RiskConfig,
    /// Chronos expected-value gate configuration.
    pub chronos: ChronosConfig,
    /// Persistence store configuration.
    pub store: StoreConfig,
    /// Metrics server configuration.
    pub metrics: ObservabilityConfig,
    /// Administrative API configuration.
    pub admin: AdminConfig,
    /// Security controls (secrets, CORS, rate limiting).
    pub security: SecurityConfig,
}

/// Bot execution mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    /// Shadow mode: no transactions are built or sent.
    Shadow,
    /// Simulate mode: transactions are built but not sent; profit is tracked
    /// under simulated metrics.
    Simulate,
    /// Canary mode: simulated execution only, never broadcast.
    Canary,
    /// Live mode: transactions may be broadcast. Opt-in only; requires
    /// `execution.live_enabled == true` and all security controls.
    Live,
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mode::Shadow => write!(f, "shadow"),
            Mode::Simulate => write!(f, "simulate"),
            Mode::Canary => write!(f, "canary"),
            Mode::Live => write!(f, "live"),
        }
    }
}

/// Wrapper carrying the mode plus a flag that must be explicitly set to allow
/// live execution (`ARB_LIVE` truthy). Presence of `Mode::Live` alone is
/// insufficient to authorize trading.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeConfig {
    pub mode: Mode,
    /// Explicit opt-in for live execution. Default false; must be set to true
    /// (via `ARB_LIVE=1`) before any transaction can be broadcast.
    #[serde(default)]
    pub live_enabled: bool,
}

/// RPC / gRPC provider configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Primary RPC/Geyser endpoint URL (approved schemes: http, https).
    pub primary_url: String,
    /// Environment variable name holding the primary provider auth token.
    /// The token value itself is never stored in this file.
    pub primary_token_env: String,
    /// Optional failover endpoint URL.
    #[serde(default)]
    pub failover_url: Option<String>,
    /// Environment variable name holding the failover provider auth token.
    #[serde(default)]
    pub failover_token_env: Option<String>,
    /// Commitment level: processed | confirmed | finalized.
    pub commitment: String,
    /// Whether to respond to pings.
    #[serde(default = "default_true")]
    pub ping_reply: bool,
}

/// Jito block-engine and tip configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JitoConfig {
    /// Jito block-engine regions used for bundle submission.
    pub regions: Vec<String>,
    /// Tip-floor percentile used for adaptive bidding (p50 | p75 | p90 | p99).
    pub tip_floor_percentile: String,
    /// Tip share applied to a route's gross edge, in basis points.
    pub tip_share_bps: Bps,
    /// Maximum Jito tip in lamports.
    pub max_tip_lamports: Lamports,
    /// Maximum tip share as a fraction of edge, in basis points.
    pub max_tip_share_bps: Bps,
    /// Minimum lamports the wallet must keep after paying the tip.
    pub min_keep_lamports: Lamports,
    /// Fraction of the edge that must survive every cost, tip included.
    #[serde(default = "default_keep_share")]
    pub keep_share: f64,
    /// Multiplicative cap on how far a retry may raise the previous tip.
    #[serde(default = "default_ladder_growth")]
    pub ladder_growth: f64,
    /// Age past which a tip-floor snapshot is treated as absent (milliseconds).
    #[serde(default = "default_tip_floor_max_age_ms")]
    pub tip_floor_max_age_ms: Milliseconds,
    /// Tip-floor poll interval (milliseconds); clamped to the endpoint's
    /// 1-request-per-second per-IP limit.
    #[serde(default = "default_tip_floor_poll_ms")]
    pub tip_floor_poll_ms: Milliseconds,
    /// Refuse to tip when the slot's leader is not known to run Jito-Solana.
    #[serde(default = "default_leader_gate_enabled")]
    pub leader_gate_enabled: bool,
    /// Optional validator registry endpoint (kobe). None → public deployment.
    #[serde(default)]
    pub kobe_url: Option<String>,
    /// Where to persist the landing ledger. None → in memory only.
    #[serde(default)]
    pub ledger_path: Option<String>,
    /// Interval between leader-schedule refreshes (seconds).
    #[serde(default = "default_leader_refresh_secs")]
    pub leader_refresh_secs: Seconds,
    /// Minimum edge in lamports required to qualify for SWQoS direct execution fallback.
    #[serde(default = "default_swqos_fallback_hurdle_lamports")]
    pub swqos_fallback_hurdle_lamports: Lamports,
    /// Priority fee in lamports used for SWQoS direct execution fallback.
    #[serde(default = "default_swqos_fallback_priority_fee_lamports")]
    pub swqos_fallback_priority_fee_lamports: Lamports,
}

/// Jupiter swap API configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JupiterConfig {
    /// Base URL for the Jupiter v2/v6 swap API (approved scheme: https).
    pub base_url: String,
    /// Environment variable name holding the Jupiter API key.
    pub api_key_env: String,
    /// Request timeout (milliseconds).
    pub timeout_ms: Milliseconds,
}

/// Wallet / keypair configuration.
///
/// The private key material itself is never stored here; only the path to a
/// key file (local development only) or a secret provider reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletConfig {
    /// Path to the operator keypair file (local development only).
    pub keypair_path: String,
    /// Optional cold wallet public key for profit sweeps.
    #[serde(default)]
    pub cold_wallet_pubkey: Option<String>,
    /// Minimum SOL balance reserve (lamports) enforced before live execution.
    pub min_sol_balance_lamports: Lamports,
}

/// Strategy / routing parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyConfig {
    /// Base token mints monitored for arbitrage opportunities.
    pub base_tokens: Vec<String>,
    /// Minimum net profit in lamports required to execute.
    pub min_net_profit_abs: Lamports,
    /// Minimum net profit in basis points of amount_in.
    pub min_net_profit_bps: Bps,
    /// Maximum price impact in basis points.
    pub max_price_impact_bps: Bps,
    /// Maximum pool state age in slots for fresh quotes.
    pub max_state_age_slots: Slot,
    /// Maximum slot age for a pool to enter the discovery graph. None → same
    /// policy as `max_state_age_slots`. Never an arbitrary multiplier.
    #[serde(default)]
    pub graph_max_age_slots: Option<Slot>,
    /// Whether flash loans are used for execution.
    #[serde(default = "default_true")]
    pub use_flash_loan: bool,
    /// Hard cap on dynamically discovered pools.
    #[serde(default)]
    pub max_pools: Option<usize>,
    /// Pool catalog refresh interval (seconds).
    #[serde(default)]
    pub discovery_refresh_secs: Option<Seconds>,
    /// Minimum TVL (USD) for a discovered pool.
    #[serde(default)]
    pub discovery_min_tvl_usd: Option<UsdMicros>,
    /// Maximum hops per arbitrage route. Every extra hop stacks another DEX
    /// trade fee plus another failure point, so routes longer than 4 hops are
    /// economically dominated (cumulative fees exceed realistic cycle
    /// spreads). Validated to 2..=4; default 3.
    #[serde(default = "default_max_route_hops")]
    pub max_route_hops: usize,
    /// HTTP-based pool catalog discovery (Orca/Raydium/Meteora/Dexscreener/
    /// Pump.fun REST APIs). Defaults to **false** (disabled): pool candidates
    /// must arrive through the configured WebSocket account-update stream and
    /// pass account validation. Operators may re-enable explicitly.
    #[serde(default)]
    pub http_discovery_enabled: bool,
    /// Slippage contingency buffer in lamports subtracted from net edge before
    /// computing maximum allowable tip to protect the profit floor against slippage.
    #[serde(default = "default_contingency_buffer_lamports")]
    pub contingency_buffer_lamports: Lamports,
    /// Baseline priority fee share of gross profit during calm market conditions (bps).
    #[serde(default = "default_priority_fee_base_share_bps")]
    pub priority_fee_base_share_bps: Bps,
    /// Maximum share of gross profit allocated to priority fee under peak competition (bps).
    #[serde(default = "default_priority_fee_max_share_bps")]
    pub priority_fee_max_share_bps: Bps,
}

fn default_max_route_hops() -> usize {
    3
}

/// Authoritative risk limits. These are the single source of truth for safety
/// thresholds; no lower-priority config source may weaken them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskConfig {
    /// Maximum trade size in USD.
    pub max_trade_size_usd: UsdMicros,
    /// Maximum daily loss in USD.
    pub max_daily_loss_usd: UsdMicros,
    /// Maximum hourly loss in USD (must not exceed daily).
    pub max_hourly_loss_usd: UsdMicros,
    /// Maximum consecutive failures before the circuit breaker opens.
    pub max_consecutive_failures: u64,
    /// Minimum landing rate (landed/sent) required to continue.
    pub min_landing_rate: f64,
    /// Maximum attempts per slot.
    pub max_attempts_per_slot: u32,
    /// Maximum inflight bundles per slot.
    pub max_inflight_bundles: usize,
    /// Maximum tip spend ratio (tip / (tip + net)).
    pub max_tip_spend_ratio: f64,
    /// Maximum orders per hour (velocity limit).
    pub max_orders_per_hour: usize,
    /// Circuit-breaker cooldown window (slots).
    pub cooldown_slots: Slot,
    /// Maximum allowable ROI ceiling in basis points (phantom quote filter).
    #[serde(default = "default_max_implausible_roi_bps")]
    pub max_implausible_roi_bps: Bps,
}

/// Chronos expected-value gate configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChronosConfig {
    /// Whether the EV gate is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Hazard decay rate per second (lambda).
    pub decay_lambda_per_sec: f64,
    /// Maximum allowable quote age (milliseconds).
    pub max_age_ms: Milliseconds,
    /// Minimum expected value in USD required for execution approval.
    pub min_ev_usd: UsdMicros,
    /// Estimated failed transaction cost overhead (lamports).
    pub failed_attempt_cost_lamports: Lamports,
    /// Dimensionless risk penalty factor per hop.
    pub beta_risk_factor: f64,
}

/// Persistence store configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreConfig {
    /// Store URL (sqlite:// or postgres://).
    pub url: String,
}

/// Observability / metrics configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservabilityConfig {
    /// Prometheus metrics listen address (host:port).
    pub listen: SocketAddr,
}

/// Administrative API configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminConfig {
    /// Admin HTTP listen address (host:port).
    pub listen: SocketAddr,
    /// Environment variable name holding the admin bearer token.
    pub token_env: String,
}

/// Security controls: CORS, rate limiting, secrets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    /// Explicit production CORS allowlist. Empty = deny all cross-origin.
    /// `CorsLayer::permissive()` is never allowed.
    #[serde(default)]
    pub cors_allow_origins: Vec<String>,
    /// Maximum HTTP request body size in bytes.
    #[serde(default = "default_max_request_body")]
    pub max_request_body_bytes: usize,
    /// Whether webhook signature validation is required.
    #[serde(default = "default_true")]
    pub webhook_signature_required: bool,
}

// ---------------------------------------------------------------------------
// Default helper functions (serde `default` targets)
// ---------------------------------------------------------------------------

fn default_true() -> bool {
    true
}
fn default_keep_share() -> f64 {
    0.85
}
fn default_ladder_growth() -> f64 {
    1.6
}
fn default_tip_floor_max_age_ms() -> Milliseconds {
    Milliseconds(5_000)
}
fn default_tip_floor_poll_ms() -> Milliseconds {
    Milliseconds(1_000)
}
fn default_leader_gate_enabled() -> bool {
    true
}
fn default_leader_refresh_secs() -> Seconds {
    Seconds(60)
}
fn default_swqos_fallback_hurdle_lamports() -> Lamports {
    Lamports(8_000)
}
fn default_swqos_fallback_priority_fee_lamports() -> Lamports {
    Lamports(2_000)
}
fn default_contingency_buffer_lamports() -> Lamports {
    Lamports(10_000)
}
fn default_priority_fee_base_share_bps() -> Bps {
    Bps(arb_constants::DEFAULT_PRIORITY_FEE_BASE_SHARE_BPS as u16)
}
fn default_priority_fee_max_share_bps() -> Bps {
    Bps(arb_constants::DEFAULT_PRIORITY_FEE_MAX_SHARE_BPS as u16)
}
fn default_max_implausible_roi_bps() -> Bps {
    Bps(1_500)
}
fn default_max_request_body() -> usize {
    1_048_576 // 1 MiB
}

impl Default for ModeConfig {
    fn default() -> Self {
        Self {
            mode: Mode::Shadow,
            live_enabled: false,
        }
    }
}

impl AppConfig {
    /// Build a safe default configuration (shadow mode, all safety limits on).
    pub fn default_safe() -> Self {
        Self {
            mode: ModeConfig::default(),
            provider: ProviderConfig {
                primary_url: String::new(),
                primary_token_env: "HELIUS_API_KEY".to_string(),
                failover_url: None,
                failover_token_env: None,
                commitment: "processed".to_string(),
                ping_reply: true,
            },
            jito: JitoConfig {
                regions: vec![
                    "frankfurt".to_string(),
                    "amsterdam".to_string(),
                    "ny".to_string(),
                ],
                tip_floor_percentile: "p50".to_string(),
                tip_share_bps: Bps(5500),
                max_tip_lamports: Lamports(2_000_000),
                max_tip_share_bps: Bps(7000),
                min_keep_lamports: Lamports(10_000),
                keep_share: 0.60,
                ladder_growth: 1.6,
                tip_floor_max_age_ms: Milliseconds(5_000),
                tip_floor_poll_ms: Milliseconds(1_000),
                leader_gate_enabled: true,
                kobe_url: None,
                ledger_path: Some("data/adaptive_ledger.json".to_string()),
                leader_refresh_secs: Seconds(60),
                swqos_fallback_hurdle_lamports: Lamports(8_000),
                swqos_fallback_priority_fee_lamports: Lamports(2_000),
            },
            jupiter: JupiterConfig {
                base_url: "https://api.jup.ag/swap/v2".to_string(),
                api_key_env: "JUPITER_API_KEY".to_string(),
                timeout_ms: Milliseconds(400),
            },
            wallet: WalletConfig {
                keypair_path: "solana_keypair.json".to_string(),
                cold_wallet_pubkey: None,
                min_sol_balance_lamports: Lamports(0),
            },
            strategy: StrategyConfig {
                base_tokens: vec![
                    "So11111111111111111111111111111111111111112".to_string(), // WSOL
                    "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".to_string(), // USDC
                ],
                min_net_profit_abs: Lamports(50_000),
                min_net_profit_bps: Bps(5),
                max_price_impact_bps: Bps(40),
                max_state_age_slots: Slot(2),
                graph_max_age_slots: None,
                use_flash_loan: true,
                max_pools: Some(1000),
                discovery_refresh_secs: Some(Seconds(15)),
                discovery_min_tvl_usd: Some(UsdMicros(25_000_000_000)), // $25,000
                max_route_hops: 3,
                http_discovery_enabled: false,
                contingency_buffer_lamports: Lamports(10_000),
                priority_fee_base_share_bps: Bps(
                    arb_constants::DEFAULT_PRIORITY_FEE_BASE_SHARE_BPS as u16,
                ),
                priority_fee_max_share_bps: Bps(
                    arb_constants::DEFAULT_PRIORITY_FEE_MAX_SHARE_BPS as u16
                ),
            },
            risk: RiskConfig {
                max_trade_size_usd: UsdMicros(10_000_000_000), // $10,000
                max_daily_loss_usd: UsdMicros(5_000_000),      // $5.00
                max_hourly_loss_usd: UsdMicros(2_000_000),     // $2.00
                max_consecutive_failures: 8,
                min_landing_rate: 0.02,
                max_attempts_per_slot: 3,
                max_inflight_bundles: 2,
                max_tip_spend_ratio: 0.7,
                max_orders_per_hour: 120,
                cooldown_slots: Slot(75),
                max_implausible_roi_bps: Bps(1_500),
            },
            chronos: ChronosConfig {
                enabled: true,
                decay_lambda_per_sec: 3.0,
                max_age_ms: Milliseconds(400),
                min_ev_usd: UsdMicros(50_000_000), // $0.05
                failed_attempt_cost_lamports: Lamports(10_000),
                beta_risk_factor: 0.0005,
            },
            store: StoreConfig {
                url: "sqlite://arb_sentinel.db".to_string(),
            },
            metrics: ObservabilityConfig {
                listen: "127.0.0.1:9100"
                    .parse()
                    .expect("valid default metrics addr"),
            },
            admin: AdminConfig {
                listen: "127.0.0.1:9101".parse().expect("valid default admin addr"),
                token_env: "SENTINEL_ADMIN_KEY".to_string(),
            },
            security: SecurityConfig {
                cors_allow_origins: Vec::new(),
                max_request_body_bytes: 1_048_576,
                webhook_signature_required: true,
            },
        }
    }
}

/// Errors produced during configuration loading and validation.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config file '{0}': {1}")]
    IoError(String, #[source] std::io::Error),
    #[error("failed to parse TOML from '{0}': {1}")]
    TomlParse(String, #[source] toml::de::Error),
    #[error("validation error: {0}")]
    Validation(String),
    #[error("missing required environment variable: {0}")]
    MissingEnv(String),
    #[error("invalid value for environment variable '{0}': {1}")]
    InvalidEnv(String, String),
    #[error("unknown configuration key: {0}")]
    UnknownKey(String),
    #[error("secret placeholder detected for {0}")]
    PlaceholderSecret(String),
}

/// Result type for configuration operations.
pub type ConfigResult<T> = std::result::Result<T, ConfigError>;
