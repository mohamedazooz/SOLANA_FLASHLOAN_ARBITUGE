//! Bot configuration loaded from TOML file.
use serde::Deserialize;
use std::fs;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub bot: BotConfig,
    pub rpc: RpcConfig,
    pub jito: JitoConfig,
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
    /// Per-hop min-out floor slippage (bps) applied to every swap leg when
    /// deriving `min_amount_out`. Overridden onto every opportunity before
    /// dispatch (single source of truth) and clamped to
    /// `HOP_SLIPPAGE_MAX_BPS` (1000) by the builders. `None` uses
    /// `DEFAULT_HOP_SLIPPAGE_BPS` (10).
    #[serde(default = "default_hop_slippage_bps")]
    pub hop_slippage_bps: u64,
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
    /// Maximum number of top active pools to select per DEX venue.
    /// `None` defaults to 33.
    #[serde(default)]
    pub max_per_venue: Option<usize>,
    /// How often the pool catalog is refreshed, in seconds. `None` uses 300.
    pub discovery_refresh_secs: Option<u64>,
    /// Minimum TVL (USD) for a discovered pool. `None` uses the default.
    pub discovery_min_tvl_usd: Option<f64>,
    /// MeteoraDlmmCrossPoolArbitrageStrategy tuning. `None` uses safe
    /// defaults (this strategy is always evaluated in the candidate
    /// cascade; tuning only bounds its CPU and size envelope).
    #[serde(default)]
    pub dlmm_cross_pool: Option<DlmmCrossPoolTuning>,
    /// Meteora 3-leg (WSOL -> A -> B -> WSOL) DLMM capture tuning. `None`
    /// uses safe defaults; the strategy is always evaluated in the
    /// candidate cascade and fails closed (no route) when it cannot
    /// resolve a fully executable chain.
    #[serde(default)]
    pub dlmm_three_leg: Option<DlmmThreeLegTuning>,
}

/// Tunable envelope for the cross-pool DLMM strategy. Every field carries
/// a safe default; a partial or absent `[strategy.dlmm_cross_pool]`
/// section is valid.
#[derive(Debug, Deserialize, Clone)]
pub struct DlmmCrossPoolTuning {
    /// Maximum distinct pool *pairs* evaluated per scan. Default 256.
    #[serde(default = "dcp_default_max_pairs")]
    pub max_pairs: usize,
    /// Minimum candidate input size in base-mint raw units. Default 100k.
    #[serde(default = "dcp_default_min_input")]
    pub min_input: u64,
    /// Hard candidate input ceiling in base-mint raw units. Default: no
    /// additional cap beyond the risk gate's `max_sizing`.
    #[serde(default = "dcp_default_max_input")]
    pub max_input: u64,
    /// Coarse candidate samples (ladder + linear fill). Default 10.
    #[serde(default = "dcp_default_coarse")]
    pub coarse_samples: usize,
    /// Local refinement iterations. Default 8.
    #[serde(default = "dcp_default_refine")]
    pub refine_samples: usize,
    /// Minimum gross cycle spread (bps). Default 10 (epsilon).
    #[serde(default = "dcp_default_spread")]
    pub min_spread_bps: u64,
    /// Dust guard on candidate input size (base-mint raw units). Default 1e6.
    #[serde(default = "dcp_default_min_liq")]
    pub min_liquidity_bin_raw: u128,
}

impl Default for DlmmCrossPoolTuning {
    fn default() -> Self {
        Self {
            max_pairs: dcp_default_max_pairs(),
            min_input: dcp_default_min_input(),
            max_input: dcp_default_max_input(),
            coarse_samples: dcp_default_coarse(),
            refine_samples: dcp_default_refine(),
            min_spread_bps: dcp_default_spread(),
            min_liquidity_bin_raw: dcp_default_min_liq(),
        }
    }
}

fn dcp_default_max_pairs() -> usize {
    256
}
fn dcp_default_min_input() -> u64 {
    100_000
}
fn dcp_default_max_input() -> u64 {
    u64::MAX
}
fn dcp_default_coarse() -> usize {
    10
}
fn dcp_default_refine() -> usize {
    8
}
fn dcp_default_spread() -> u64 {
    10
}
fn dcp_default_min_liq() -> u128 {
    1_000_000
}

/// Tunable envelope for the Meteora 3-leg DLMM capture strategy. Every
/// field carries a safe default; a partial or absent
/// `[strategy.dlmm_three_leg]` section is valid.
#[derive(Debug, Deserialize, Clone)]
pub struct DlmmThreeLegTuning {
    /// Discovery / primary input size in WSOL lamports. Default 0.1 SOL.
    #[serde(default = "three_leg_default_input")]
    pub input_size: u64,
    /// Hard candidate input ceiling in WSOL lamports (further bounded by
    /// the live risk-gate `max_sizing`). Default 2 SOL.
    #[serde(default = "three_leg_default_max_input")]
    pub max_input: u64,
    /// Per-leg slippage applied when deriving min-out floors (bps). Default 100.
    #[serde(default = "three_leg_default_slippage")]
    pub slippage_bps: u64,
    /// Hard minimal net profit (lamports) for the pipeline's internal gate.
    #[serde(default = "three_leg_default_min_profit")]
    pub min_profit_lamports: u64,
    /// Number of candidate sizes the 3-leg size optimizer evaluates. Default 4.
    #[serde(default = "three_leg_default_sizes")]
    pub size_ladder: usize,
}

impl Default for DlmmThreeLegTuning {
    fn default() -> Self {
        Self {
            input_size: three_leg_default_input(),
            max_input: three_leg_default_max_input(),
            slippage_bps: three_leg_default_slippage(),
            min_profit_lamports: three_leg_default_min_profit(),
            size_ladder: three_leg_default_sizes(),
        }
    }
}

fn three_leg_default_input() -> u64 {
    100_000_000
}
fn three_leg_default_max_input() -> u64 {
    2_000_000_000
}
fn default_hop_slippage_bps() -> u64 {
    arb_constants::DEFAULT_HOP_SLIPPAGE_BPS
}
fn three_leg_default_slippage() -> u64 {
    100
}
fn three_leg_default_min_profit() -> u64 {
    30_000
}
fn three_leg_default_sizes() -> usize {
    4
}

/// Effective net-profit floor in WSOL lamports: the TOML floor raised by the
/// optional USD-denominated override `MIN_NET_PROFIT_USD` converted at
/// `SOL_USD_PRICE`. Floating point is confined to this config-boundary
/// conversion; the returned floor is integer lamports and every downstream
/// decision uses it as-is.
pub fn min_net_profit_floor_lamports(toml_floor: u64) -> u64 {
    let parse_env = |key: &str| -> Option<f64> {
        std::env::var(key)
            .ok()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|v| v.is_finite())
    };
    let Some(min_usd) = parse_env("MIN_NET_PROFIT_USD").filter(|v| *v >= 0.0) else {
        return toml_floor;
    };
    let Some(sol_price) = parse_env("SOL_USD_PRICE").filter(|v| *v > 0.0) else {
        return toml_floor;
    };
    let lamports = (min_usd / sol_price * 1e9).round();
    if !lamports.is_finite() || lamports < 0.0 || lamports >= u64::MAX as f64 {
        return toml_floor;
    }
    toml_floor.max(lamports as u64)
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
    let contents =
        fs::read_to_string(path).with_context(|| format!("Failed to read config file '{path}'"))?;
    let mut config: Config = toml::from_str(&contents).with_context(|| fmt_parse_error(path))?;

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
    if config.chronos.decay_lambda_per_sec < 0.1 || config.chronos.decay_lambda_per_sec > 100.0 {
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

#[cfg(test)]
mod min_profit_floor_tests {
    use super::min_net_profit_floor_lamports;

    // One sequential test: the env is process-global and cargo runs test
    // cases in parallel threads, so per-case env manipulation would race.
    #[test]
    fn min_net_profit_floor_matrix() {
        let baseline = 30_000u64;

        assert_eq!(min_net_profit_floor_lamports(baseline), baseline);

        unsafe { std::env::set_var("MIN_NET_PROFIT_USD", "0.05") };
        assert_eq!(min_net_profit_floor_lamports(baseline), baseline);

        unsafe { std::env::set_var("SOL_USD_PRICE", "200.0") };
        assert_eq!(min_net_profit_floor_lamports(baseline), 250_000);
        assert_eq!(min_net_profit_floor_lamports(1_000_000), 1_000_000);

        unsafe { std::env::set_var("MIN_NET_PROFIT_USD", "-1") };
        assert_eq!(min_net_profit_floor_lamports(baseline), baseline);
        unsafe { std::env::set_var("MIN_NET_PROFIT_USD", "abc") };
        assert_eq!(min_net_profit_floor_lamports(baseline), baseline);

        unsafe { std::env::remove_var("MIN_NET_PROFIT_USD") };
        unsafe { std::env::remove_var("SOL_USD_PRICE") };
        assert_eq!(min_net_profit_floor_lamports(baseline), baseline);
    }
}
