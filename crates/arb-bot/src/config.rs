//! Bot configuration loaded from TOML file.
use serde::Deserialize;
use std::fs;

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
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
    #[serde(default)]
    pub jupiter: JupiterConfig,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
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
    0.01
}
fn default_failed_cost() -> u64 {
    10_000
}
fn default_beta_risk() -> f64 {
    0.0005
}
fn default_keep_share() -> f64 {
    0.90
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
fn default_swqos_fallback_hurdle_lamports() -> u64 {
    8_000
}
fn default_swqos_fallback_priority_fee_lamports() -> u64 {
    2_000
}
fn default_contingency_buffer_lamports() -> u64 {
    10_000
}
fn default_priority_fee_base_share_bps() -> u64 {
    arb_constants::DEFAULT_PRIORITY_FEE_BASE_SHARE_BPS
}
fn default_priority_fee_max_share_bps() -> u64 {
    arb_constants::DEFAULT_PRIORITY_FEE_MAX_SHARE_BPS
}
fn default_max_implausible_roi_bps() -> u64 {
    1_500
}
/// Resolve the wallet SOL reserve. Zero/malformed overrides are rejected so
/// an environment typo cannot silently disable the reserve guard.
pub fn effective_min_sol_reserve_lamports(
    configured: u64,
    env_override: Option<&str>,
) -> anyhow::Result<u64> {
    if configured == 0 {
        anyhow::bail!("wallet.min_sol_balance_lamports must be positive");
    }
    let Some(raw) = env_override else {
        return Ok(configured);
    };
    let value = raw.trim().parse::<u64>().map_err(|error| {
        anyhow::anyhow!("MIN_SOL_RESERVE_LAMPORTS must be a positive integer: {error}")
    })?;
    if value == 0 {
        anyhow::bail!(
            "MIN_SOL_RESERVE_LAMPORTS=0 is not allowed; reserve guard cannot be disabled"
        );
    }
    Ok(value)
}
/// Apply the optional environment ROI ceiling without allowing it to weaken
/// the configured phantom-quote guard. Invalid or zero overrides are errors.
pub fn effective_max_implausible_roi_bps(
    configured: u64,
    env_override: Option<&str>,
) -> anyhow::Result<u64> {
    if configured == 0 {
        anyhow::bail!("configured max_implausible_roi_bps must be positive");
    }
    let Some(raw) = env_override else {
        return Ok(configured);
    };
    let value = raw.trim().parse::<u64>().map_err(|error| {
        anyhow::anyhow!("ARB_MAX_IMPLAUSIBLE_ROI_BPS must be a positive integer: {error}")
    })?;
    if value == 0 {
        anyhow::bail!("ARB_MAX_IMPLAUSIBLE_ROI_BPS must be positive");
    }
    Ok(configured.min(value))
}
fn default_jupiter_base_url() -> String {
    "https://api.jup.ag/swap/v2".to_string()
}
fn default_jupiter_api_key_env() -> String {
    "JUPITER_API_KEY".to_string()
}
fn default_jupiter_timeout_ms() -> u64 {
    400
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct JupiterConfig {
    #[serde(default = "default_jupiter_base_url")]
    pub base_url: String,
    #[serde(default = "default_jupiter_api_key_env")]
    pub api_key_env: String,
    #[serde(default = "default_jupiter_timeout_ms")]
    pub timeout_ms: u64,
}

impl Default for JupiterConfig {
    fn default() -> Self {
        Self {
            base_url: default_jupiter_base_url(),
            api_key_env: default_jupiter_api_key_env(),
            timeout_ms: default_jupiter_timeout_ms(),
        }
    }
}

impl Default for ChronosConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            decay_lambda_per_sec: 3.0,
            max_age_ms: 400,
            min_ev_usd: 0.01,
            failed_attempt_cost_lamports: 10_000,
            beta_risk_factor: 0.0005,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct RpcConfig {
    pub url: String,
    #[serde(default)]
    pub failover_url: Option<String>,
    pub commitment: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
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
    /// Minimum edge in lamports required to qualify for SWQoS direct submission fallback.
    #[serde(default = "default_swqos_fallback_hurdle_lamports")]
    pub swqos_fallback_hurdle_lamports: u64,
    /// Priority fee in lamports used for SWQoS direct submission fallback.
    #[serde(default = "default_swqos_fallback_priority_fee_lamports")]
    pub swqos_fallback_priority_fee_lamports: u64,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct WalletConfig {
    pub keypair_path: String,
    pub cold_wallet_pubkey: Option<String>,
    #[serde(default)]
    pub profit_wallet: Option<String>,
    pub min_sol_balance_lamports: u64,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct StrategyConfig {
    /// Configured monitoring/watchlist mints. The active flash-loan graph is
    /// currently anchored to the WSOL Kamino reserve; this list does not create
    /// additional independent flash-loan anchors.
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
    /// Mandatory for all executable arbitrage; config validation rejects false.
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
    /// Maximum pools (swap legs) per closed flash-loan route (2..=3, default 3).
    /// The pool discovery universe is bounded separately by `max_pools`.
    #[serde(default = "default_max_route_hops")]
    pub max_route_hops: usize,
    /// HTTP-based pool catalog discovery (REST APIs). Defaults to **false**
    /// (disabled): pool candidates must arrive via the WebSocket
    /// account-update stream. Operators may re-enable explicitly.
    #[serde(default)]
    pub http_discovery_enabled: bool,
    /// Slippage contingency buffer in lamports subtracted from net edge before
    /// computing maximum allowable tip to protect the profit floor against slippage.
    #[serde(default = "default_contingency_buffer_lamports")]
    pub contingency_buffer_lamports: u64,
    /// Baseline priority fee share of gross profit during calm market conditions (bps).
    #[serde(default = "default_priority_fee_base_share_bps")]
    pub priority_fee_base_share_bps: u64,
    /// Maximum share of gross profit allocated to priority fee under peak competition (bps).
    #[serde(default = "default_priority_fee_max_share_bps")]
    pub priority_fee_max_share_bps: u64,
}

fn default_max_route_hops() -> usize {
    3
}

/// Tunable envelope for the cross-pool DLMM strategy. Every field carries
/// a safe default; a partial or absent `[strategy.dlmm_cross_pool]`
/// section is valid.
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
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
    /// `u64`, not `u128`: the `toml` crate cannot deserialize into `u128`, so
    /// a `u128` field here made every shipped `config.toml` with this key
    /// fail to parse at startup (see deploy run 43741572901).
    #[serde(default = "dcp_default_min_liq")]
    pub min_liquidity_bin_raw: u64,
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
    1
}
fn dcp_default_min_liq() -> u64 {
    1_000_000
}

/// Tunable envelope for the Meteora 3-leg DLMM capture strategy. Every
/// field carries a safe default; a partial or absent
/// `[strategy.dlmm_three_leg]` section is valid.
#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
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
    arb_constants::DEFAULT_HOP_SLIPPAGE_BPS
}
fn three_leg_default_min_profit() -> u64 {
    arb_constants::DEFAULT_MIN_PROFIT_LAMPORTS
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
#[serde(deny_unknown_fields)]
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
    /// Maximum allowable ROI ceiling in basis points (phantom quote filter).
    #[serde(default = "default_max_implausible_roi_bps")]
    pub max_implausible_roi_bps: u64,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct StoreConfig {
    pub url: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct MetricsConfig {
    pub listen: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct AdminConfig {
    pub listen: String,
    pub token_env: String,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct BotConfig {
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub canary: bool,
    #[serde(default)]
    pub live_enabled: Option<bool>,
}

pub fn from_arb_config(arb: arb_config::AppConfig) -> Config {
    let usd = |micros: arb_types::UsdMicros| micros.0 as f64 / 1_000_000.0;
    Config {
        bot: BotConfig {
            mode: Some(format!("{:?}", arb.mode.mode).to_ascii_lowercase()),
            canary: matches!(arb.mode.mode, arb_config::Mode::Canary),
            live_enabled: Some(arb.mode.live_enabled),
        },
        rpc: RpcConfig {
            url: arb.provider.primary_url.clone(),
            failover_url: arb.provider.failover_url.clone(),
            commitment: arb.provider.commitment.clone(),
        },
        jito: JitoConfig {
            regions: arb.jito.regions,
            tip_floor_percentile: arb.jito.tip_floor_percentile,
            tip_share_bps: arb.jito.tip_share_bps.0 as u64,
            max_tip_lamports: arb.jito.max_tip_lamports.0,
            max_tip_share_bps: arb.jito.max_tip_share_bps.0 as u64,
            min_keep_lamports: arb.jito.min_keep_lamports.0,
            keep_share: arb.jito.keep_share,
            ladder_growth: arb.jito.ladder_growth,
            tip_floor_max_age_ms: arb.jito.tip_floor_max_age_ms.0,
            tip_floor_poll_ms: arb.jito.tip_floor_poll_ms.0,
            leader_gate_enabled: arb.jito.leader_gate_enabled,
            kobe_url: arb.jito.kobe_url,
            ledger_path: arb.jito.ledger_path,
            leader_refresh_secs: arb.jito.leader_refresh_secs.0,
            swqos_fallback_hurdle_lamports: arb.jito.swqos_fallback_hurdle_lamports.0,
            swqos_fallback_priority_fee_lamports: arb.jito.swqos_fallback_priority_fee_lamports.0,
        },
        wallet: WalletConfig {
            keypair_path: arb.wallet.keypair_path,
            cold_wallet_pubkey: arb.wallet.cold_wallet_pubkey,
            profit_wallet: None,
            min_sol_balance_lamports: arb.wallet.min_sol_balance_lamports.0,
        },
        strategy: StrategyConfig {
            base_tokens: arb.strategy.base_tokens,
            min_net_profit_abs: min_net_profit_floor_lamports(arb.strategy.min_net_profit_abs.0),
            min_net_profit_bps: arb.strategy.min_net_profit_bps.0 as u64,
            hop_slippage_bps: u64::from(arb.strategy.hop_slippage_bps.0),
            max_price_impact_bps: arb.strategy.max_price_impact_bps.0 as u64,
            max_state_age_slots: arb.strategy.max_state_age_slots.0,
            graph_max_age_slots: arb.strategy.graph_max_age_slots.map(|s| s.0),
            use_flash_loan: arb.strategy.use_flash_loan,
            max_pools: arb.strategy.max_pools,
            max_per_venue: None,
            discovery_refresh_secs: arb.strategy.discovery_refresh_secs.map(|s| s.0),
            discovery_min_tvl_usd: arb.strategy.discovery_min_tvl_usd.map(usd),
            dlmm_cross_pool: None,
            dlmm_three_leg: None,
            max_route_hops: arb.strategy.max_route_hops,
            http_discovery_enabled: arb.strategy.http_discovery_enabled,
            contingency_buffer_lamports: arb.strategy.contingency_buffer_lamports.0,
            priority_fee_base_share_bps: arb.strategy.priority_fee_base_share_bps.0 as u64,
            priority_fee_max_share_bps: arb.strategy.priority_fee_max_share_bps.0 as u64,
        },
        risk: RiskConfig {
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
            max_implausible_roi_bps: arb.risk.max_implausible_roi_bps.0 as u64,
        },
        store: StoreConfig { url: arb.store.url },
        metrics: MetricsConfig {
            listen: arb.metrics.listen.to_string(),
        },
        admin: AdminConfig {
            listen: arb.admin.listen.to_string(),
            token_env: arb.admin.token_env,
        },
        chronos: ChronosConfig {
            enabled: arb.chronos.enabled,
            decay_lambda_per_sec: arb.chronos.decay_lambda_per_sec,
            max_age_ms: arb.chronos.max_age_ms.0,
            min_ev_usd: usd(arb.chronos.min_ev_usd),
            failed_attempt_cost_lamports: arb.chronos.failed_attempt_cost_lamports.0,
            beta_risk_factor: arb.chronos.beta_risk_factor,
        },
        jupiter: JupiterConfig {
            base_url: arb.jupiter.base_url,
            api_key_env: arb.jupiter.api_key_env,
            timeout_ms: arb.jupiter.timeout_ms.0,
        },
    }
}

/// Slippage policy validation (F-04): reject invalid configurations instead
/// of silently clamping in multiple locations. The builder still clamps what
/// it computes via `effective_hop_slippage_bps` as the last line of defence,
/// but a config beyond the documented ceiling is an operator error and must
/// fail at startup: `HOP_SLIPPAGE_MAX_BPS` (1_000 = 10%) is the safety
/// ceiling; 10_000 would make min-outs meaningless.
fn validate_slippage_policies(config: &Config) -> anyhow::Result<()> {
    if !arb_constants::is_valid_slippage_bps(config.strategy.hop_slippage_bps) {
        anyhow::bail!(
            "strategy.hop_slippage_bps = {} must be <= 10_000",
            config.strategy.hop_slippage_bps
        );
    }
    if config.strategy.hop_slippage_bps > arb_constants::HOP_SLIPPAGE_MAX_BPS {
        anyhow::bail!(
            "strategy.hop_slippage_bps = {} exceeds the HOP_SLIPPAGE_MAX_BPS ceiling of {}",
            config.strategy.hop_slippage_bps,
            arb_constants::HOP_SLIPPAGE_MAX_BPS
        );
    }
    if let Some(three_leg) = &config.strategy.dlmm_three_leg
        && (!arb_constants::is_valid_slippage_bps(three_leg.slippage_bps)
            || three_leg.slippage_bps > arb_constants::HOP_SLIPPAGE_MAX_BPS)
    {
        anyhow::bail!(
            "strategy.dlmm_three_leg.slippage_bps = {} exceeds the ceiling of {}",
            three_leg.slippage_bps,
            arb_constants::HOP_SLIPPAGE_MAX_BPS
        );
    }
    Ok(())
}

pub fn load(path: &str) -> anyhow::Result<Config> {
    use anyhow::Context;
    let contents =
        fs::read_to_string(path).with_context(|| format!("Failed to read config file '{path}'"))?;
    let root: toml::Value = toml::from_str(&contents).with_context(|| fmt_parse_error(path))?;

    // Select the schema explicitly. Never swallow AppConfig validation errors
    // and silently fall back to the legacy BotConfig parser.
    let is_app_config = root.get("mode").is_some() || root.get("provider").is_some();
    if is_app_config {
        let arb = arb_config::loader::load(path)
            .with_context(|| format!("Invalid AppConfig schema in '{path}'"))?;
        let config = from_arb_config(arb);
        validate_live_freshness(&config, crate::live_execution_enabled())?;
        return Ok(config);
    }

    let mut config: Config = toml::from_str(&contents).with_context(|| fmt_parse_error(path))?;

    // Profit wallet override from environment
    if let Ok(raw) = std::env::var("PROFIT_WALLET").or_else(|_| std::env::var("COLD_WALLET")) {
        let trimmed = raw.trim();
        if !trimmed.is_empty() && !trimmed.contains("REPLACE_ME") {
            config.wallet.profit_wallet = Some(trimmed.to_string());
            config.wallet.cold_wallet_pubkey = Some(trimmed.to_string());
        }
    }

    // Trade sizing override (MAX_TRADE_SIZE or MAX_TRADE_SIZE_USD)
    if let Ok(raw) =
        std::env::var("MAX_TRADE_SIZE").or_else(|_| std::env::var("MAX_TRADE_SIZE_USD"))
    {
        if let Ok(value) = raw.trim().parse::<f64>() {
            if value.is_finite() && value > 0.0 {
                config.risk.max_trade_size_usd = config.risk.max_trade_size_usd.min(value);
            }
        }
    }

    // Daily loss override
    if let Ok(raw) =
        std::env::var("MAX_DAILY_LOSS").or_else(|_| std::env::var("MAX_DAILY_LOSS_USD"))
    {
        if let Ok(value) = raw.trim().parse::<f64>() {
            if value.is_finite() && value > 0.0 {
                config.risk.max_daily_loss_usd = config.risk.max_daily_loss_usd.min(value);
            }
        }
    }

    // Concurrent trades / in-flight limit
    if let Ok(raw) =
        std::env::var("MAX_INFLIGHT_BUNDLES").or_else(|_| std::env::var("MAX_CONCURRENT_TRADES"))
    {
        if let Ok(value) = raw.trim().parse::<u32>() {
            if value > 0 {
                config.risk.max_inflight_bundles = config.risk.max_inflight_bundles.min(value);
            }
        }
    }

    // Min net profit override
    if let Ok(raw) =
        std::env::var("MIN_NET_PROFIT").or_else(|_| std::env::var("MIN_NET_PROFIT_LAMPORTS"))
    {
        if let Ok(value) = raw.trim().parse::<u64>() {
            config.strategy.min_net_profit_abs = value;
        }
    }
    // Apply the same optional USD floor in both AppConfig and legacy BotConfig
    // paths; it only takes effect when SOL_USD_PRICE is explicitly supplied.
    config.strategy.min_net_profit_abs =
        min_net_profit_floor_lamports(config.strategy.min_net_profit_abs);

    // Slippage BPS override
    if let Ok(raw) = std::env::var("MAX_SLIPPAGE_BPS") {
        if let Ok(value) = raw.trim().parse::<u64>() {
            config.strategy.hop_slippage_bps = config.strategy.hop_slippage_bps.min(value);
        }
    }

    if let Ok(raw) = std::env::var("JITO_MIN_KEEP_LAMPORTS") {
        let value: u64 = raw
            .trim()
            .parse()
            .context("JITO_MIN_KEEP_LAMPORTS must be an integer")?;
        if value == 0 {
            anyhow::bail!("JITO_MIN_KEEP_LAMPORTS must be positive");
        }
        config.jito.min_keep_lamports = value;
    }
    if let Ok(raw) = std::env::var("JITO_MAX_TIP_SHARE_BPS") {
        let value: u64 = raw
            .trim()
            .parse()
            .context("JITO_MAX_TIP_SHARE_BPS must be an integer")?;
        if value == 0 || value > 10_000 {
            anyhow::bail!("JITO_MAX_TIP_SHARE_BPS must be in 1..=10000");
        }
        config.jito.max_tip_share_bps = config.jito.max_tip_share_bps.min(value);
        if config.jito.tip_share_bps > config.jito.max_tip_share_bps {
            anyhow::bail!("JITO_MAX_TIP_SHARE_BPS cannot be lower than JITO_TIP_SHARE_BPS");
        }
    } // Priority fee share override (legacy MAX_PRIORITY_FEE alias retained)
    if let Ok(raw) =
        std::env::var("MAX_PRIORITY_FEE_SHARE_BPS").or_else(|_| std::env::var("MAX_PRIORITY_FEE"))
    {
        if let Ok(value) = raw.trim().parse::<u64>() {
            config.strategy.priority_fee_max_share_bps =
                config.strategy.priority_fee_max_share_bps.min(value);
        }
    }

    // Explicit flash-borrow cap intersects the general USD notional limit.
    if let Ok(raw) = std::env::var("MAX_FLASH_BORROW_USD") {
        let value: f64 = raw
            .trim()
            .parse()
            .context("MAX_FLASH_BORROW_USD must be numeric")?;
        if !value.is_finite() || value <= 0.0 {
            anyhow::bail!("MAX_FLASH_BORROW_USD must be positive and finite");
        }
        config.risk.max_trade_size_usd = config.risk.max_trade_size_usd.min(value);
    }
    if let Ok(raw) = std::env::var("MAX_DAILY_LOSS_USD") {
        let value: f64 = raw
            .trim()
            .parse()
            .context("MAX_DAILY_LOSS_USD must be numeric")?;
        if !value.is_finite() || value <= 0.0 {
            anyhow::bail!("MAX_DAILY_LOSS_USD must be positive and finite");
        }
        config.risk.max_daily_loss_usd = config.risk.max_daily_loss_usd.min(value);
    }
    if let Ok(raw) = std::env::var("MAX_HOURLY_LOSS_USD") {
        let value: f64 = raw
            .trim()
            .parse()
            .context("MAX_HOURLY_LOSS_USD must be numeric")?;
        if !value.is_finite() || value <= 0.0 {
            anyhow::bail!("MAX_HOURLY_LOSS_USD must be positive and finite");
        }
        config.risk.max_hourly_loss_usd = config.risk.max_hourly_loss_usd.min(value);
    }
    if let Ok(raw) = std::env::var("MIN_NET_PROFIT_BPS") {
        let value: u64 = raw
            .trim()
            .parse()
            .context("MIN_NET_PROFIT_BPS must be an integer")?;
        config.strategy.min_net_profit_bps = value;
    }
    if let Ok(raw) = std::env::var("MAX_PRICE_IMPACT_BPS") {
        let value: u64 = raw
            .trim()
            .parse()
            .context("MAX_PRICE_IMPACT_BPS must be an integer")?;
        config.strategy.max_price_impact_bps = config.strategy.max_price_impact_bps.min(value);
    }
    if let Ok(raw) = std::env::var("CHRONOS_ENABLED") {
        let enabled = match raw.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" => false,
            _ => anyhow::bail!("CHRONOS_ENABLED must be true or false"),
        };
        config.chronos.enabled &= enabled;
    }
    // Max transaction failures
    if let Ok(raw) = std::env::var("MAX_CONSECUTIVE_FAILURES")
        .or_else(|_| std::env::var("MAX_TRANSACTION_FAILURES"))
    {
        if let Ok(value) = raw.trim().parse::<u64>() {
            if value > 0 {
                config.risk.max_consecutive_failures =
                    config.risk.max_consecutive_failures.min(value);
            }
        }
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
        config.chronos.max_age_ms = config.chronos.max_age_ms.min(
            raw.parse()
                .context("CHRONOS_MAX_AGE_MS must be an integer")?,
        );
    }
    if let Ok(raw) = std::env::var("CHRONOS_MIN_EV_USD") {
        let value: f64 = raw.parse().context("CHRONOS_MIN_EV_USD must be numeric")?;
        if !value.is_finite() || value < 0.0 {
            anyhow::bail!("CHRONOS_MIN_EV_USD must be finite and non-negative");
        }
        config.chronos.min_ev_usd = config.chronos.min_ev_usd.max(value);
    }

    if config.risk.max_tip_spend_ratio > 1.0 {
        anyhow::bail!("max_tip_spend_ratio must be <= 1.0");
    }

    validate_slippage_policies(&config)?;

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
        info!(
            "⚡ Flash Loan enabled — atomic borrow/repay path configured; risk gates still apply"
        );
    }

    validate_live_freshness(&config, crate::live_execution_enabled())?;

    Ok(config)
}

/// LIVE-mode freshness ceiling, in slots. The value is the project's own
/// recorded production discipline (`config/base.toml` and
/// `config/production.toml`: `max_state_age_slots = 2`), not an invented
/// number. Simulation tolerates the looser root-`config.toml` HTTP-RPC
/// polling drift; live execution never does.
pub const LIVE_MAX_STATE_AGE_SLOTS: u64 = 2;

/// Fail-closed startup gate (PIPE-08): live mode refuses configurations that
/// carry development freshness tolerances. Opportunities whose pool states
/// lag beyond this ceiling are racy for atomic execution — the whole point of
/// atomicity is that quotes and payment land in the same epoch of state.
/// `live_mode` is passed explicitly (main.rs resolves the BotMode) so the
/// check never races the `ARB_LIVE` env in tests.
pub fn validate_live_freshness(config: &Config, live_mode: bool) -> anyhow::Result<()> {
    if !live_mode {
        return Ok(());
    }
    if config.strategy.max_state_age_slots > LIVE_MAX_STATE_AGE_SLOTS {
        anyhow::bail!(
            "live mode requires strategy.max_state_age_slots <= {LIVE_MAX_STATE_AGE_SLOTS}, \
             got {} — HTTP-RPC polling tolerance must not reach live execution; \
             use config/production.toml freshness discipline or a streaming feed",
            config.strategy.max_state_age_slots
        );
    }
    Ok(())
}

fn fmt_parse_error(path: &str) -> anyhow::Error {
    anyhow::anyhow!("Failed to parse TOML configuration from '{path}'")
}

#[cfg(test)]
mod config_file_tests {
    use super::{Config, LIVE_MAX_STATE_AGE_SLOTS, validate_live_freshness};

    /// Regression: `min_liquidity_bin_raw` was declared `u128`, which the
    /// `toml` crate cannot deserialize. Every tracked full bot config must
    /// deserialize before it can be selected at startup.
    #[test]
    fn default_slippage_uses_the_shared_protocol_constant() {
        assert_eq!(
            super::default_hop_slippage_bps(),
            arb_constants::DEFAULT_HOP_SLIPPAGE_BPS
        );
        assert_eq!(
            super::three_leg_default_slippage(),
            arb_constants::DEFAULT_HOP_SLIPPAGE_BPS
        );
    }

    #[test]
    fn tracked_config_files_deserialize() {
        for path in [
            "../../config/config.example.toml",
            "../../config/testing.toml",
            "../../config/development.toml",
            "../../config/staging.toml",
            "../../config/production.toml",
        ] {
            let full = concat!(env!("CARGO_MANIFEST_DIR"), "/").to_string() + path;
            let contents = std::fs::read_to_string(&full)
                .unwrap_or_else(|e| panic!("{full} must exist and be readable: {e}"));
            let parsed: Config = toml::from_str(&contents)
                .unwrap_or_else(|e| panic!("{full} must deserialize into Config: {e}"));
            let local = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config/config.toml");
            if std::path::Path::new(local).is_file() {
                let loaded =
                    super::load(local).expect("local runtime config must deserialize and validate");
                assert!(loaded.risk.max_trade_size_usd > 0.0);
            }
            let _ = parsed;
        }
    }

    #[test]
    fn testing_config_is_loopback_and_uses_isolated_resources() {
        let full = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config/testing.toml");
        let parsed = super::load(full).expect("testing config loads and validates");
        assert_eq!(parsed.rpc.url, "http://127.0.0.1:8899");
        assert_eq!(parsed.store.url, "sqlite://arb_sentinel_test.db");
        assert_eq!(parsed.metrics.listen, "127.0.0.1:19100");
        assert_eq!(parsed.admin.listen, "127.0.0.1:19101");
        assert!(!parsed.bot.canary);
    }

    /// PIPE-08: start from the tracked production-shaped example (2 slots),
    /// then prove the live gate refuses the dev tolerance (300) at startup.
    #[test]
    fn live_freshness_gate() {
        let full = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config.example.toml");
        let contents =
            std::fs::read_to_string(full).expect("config.example.toml must exist and be readable");
        let mut cfg: Config =
            toml::from_str(&contents).expect("config.example.toml must deserialize");
        assert_eq!(cfg.strategy.max_state_age_slots, 2);
        assert!(validate_live_freshness(&cfg, true).is_ok());

        cfg.strategy.max_state_age_slots = 3;
        assert!(validate_live_freshness(&cfg, true).is_err());
        assert_eq!(cfg.strategy.max_state_age_slots, 3);

        cfg.strategy.max_state_age_slots = 300;
        assert!(validate_live_freshness(&cfg, true).is_err());
        // The same dev tolerance is legitimate outside live mode.
        assert!(validate_live_freshness(&cfg, false).is_ok());
        assert_eq!(LIVE_MAX_STATE_AGE_SLOTS, 2);
    }
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

        unsafe { std::env::set_var("MIN_NET_PROFIT_USD", "0.05") };
        unsafe { std::env::set_var("SOL_USD_PRICE", "200") };
        let base_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../config/base.toml");
        let loaded = super::load(base_path).expect("legacy config should load");
        assert_eq!(loaded.strategy.min_net_profit_abs, 250_000);
        unsafe { std::env::remove_var("MIN_NET_PROFIT_USD") };
        unsafe { std::env::remove_var("SOL_USD_PRICE") };
        assert_eq!(min_net_profit_floor_lamports(baseline), baseline);
    }
}

#[cfg(test)]
mod slippage_policy_tests {
    use super::{Config, validate_slippage_policies};

    fn tracked_config() -> Config {
        let full = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../config/config.example.toml"
        );
        let contents = std::fs::read_to_string(full).expect("tracked config example readable");
        toml::from_str(&contents).expect("tracked config example deserializes")
    }

    /// F-04: the shipped production values (hop 10 bps, 3-leg 15 bps) are both
    /// intentional, distinct-per-strategy parameters and both pass load-time
    /// validation. Distinct values are NOT drift: the bug was the runtime
    /// overwrite of the authored 3-leg value, not the existence of two.
    #[test]
    fn shipped_values_pass_validation() {
        let cfg = tracked_config();
        assert!(cfg.strategy.hop_slippage_bps > 0);
        assert!(cfg.strategy.hop_slippage_bps <= 1_000);
        validate_slippage_policies(&cfg).expect("shipped config must satisfy the slippage policy");
    }

    /// Matching values also pass (single-value setups).
    #[test]
    fn matching_values_pass_validation() {
        let mut cfg = tracked_config();
        cfg.strategy.hop_slippage_bps = 50;
        if let Some(three) = &mut cfg.strategy.dlmm_three_leg {
            three.slippage_bps = 50;
        }
        validate_slippage_policies(&cfg).expect("matching values must pass");
    }

    /// Above `HOP_SLIPPAGE_MAX_BPS` (1_000) => operator error => bail.
    #[test]
    fn values_above_ceiling_are_rejected() {
        let mut cfg = tracked_config();
        cfg.strategy.hop_slippage_bps = 1_001;
        assert!(validate_slippage_policies(&cfg).is_err());

        let mut cfg = tracked_config();
        let three = cfg
            .strategy
            .dlmm_three_leg
            .get_or_insert_with(super::DlmmThreeLegTuning::default);
        three.slippage_bps = 5_000;
        assert!(validate_slippage_policies(&cfg).is_err());
    }

    /// Above the protocol maximum (10_000) is doubly invalid.
    #[test]
    fn values_above_protocol_maximum_are_rejected() {
        let mut cfg = tracked_config();
        cfg.strategy.hop_slippage_bps = 10_001;
        assert!(validate_slippage_policies(&cfg).is_err());

        // 10_000 is protocol-valid but above the strategy ceiling.
        let mut cfg = tracked_config();
        cfg.strategy.hop_slippage_bps = 10_000;
        assert!(validate_slippage_policies(&cfg).is_err());
    }

    /// Zero slippage is valid input (0 bps => min-outs equal expected outs).
    #[test]
    fn zero_slippage_is_valid() {
        let mut cfg = tracked_config();
        cfg.strategy.hop_slippage_bps = 0;
        if let Some(three) = &mut cfg.strategy.dlmm_three_leg {
            three.slippage_bps = 0;
        }
        validate_slippage_policies(&cfg).expect("0 bps is a valid policy");
    }
}

#[cfg(test)]
#[cfg(test)]
mod reserve_limit_tests {
    use super::effective_min_sol_reserve_lamports;

    #[test]
    fn reserve_override_cannot_disable_wallet_reserve() {
        assert_eq!(
            effective_min_sol_reserve_lamports(50_000_000, None).unwrap(),
            50_000_000
        );
        assert_eq!(
            effective_min_sol_reserve_lamports(50_000_000, Some("10000000")).unwrap(),
            10_000_000
        );
        assert!(effective_min_sol_reserve_lamports(50_000_000, Some("0")).is_err());
        assert!(effective_min_sol_reserve_lamports(50_000_000, Some("bad")).is_err());
    }
}
#[cfg(test)]
mod roi_limit_tests {
    use super::effective_max_implausible_roi_bps;

    #[test]
    fn roi_env_override_can_only_tighten_the_configured_ceiling() {
        assert_eq!(
            effective_max_implausible_roi_bps(1_500, None).unwrap(),
            1_500
        );
        assert_eq!(
            effective_max_implausible_roi_bps(1_500, Some("100000")).unwrap(),
            1_500
        );
        assert_eq!(
            effective_max_implausible_roi_bps(1_500, Some("750")).unwrap(),
            750
        );
        assert!(effective_max_implausible_roi_bps(1_500, Some("0")).is_err());
        assert!(effective_max_implausible_roi_bps(1_500, Some("invalid")).is_err());
    }
}
