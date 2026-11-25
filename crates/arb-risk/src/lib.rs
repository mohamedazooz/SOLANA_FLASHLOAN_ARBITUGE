//! Risk management: multi-layer limits, circuit breaker, kill switch.

use arb_types::{Lamports, Slot};
use parking_lot::Mutex;
use std::sync::Arc;
use tracing::{error, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BreakerState {
    #[default]
    Closed,
    Open,
    HalfOpen,
}

pub struct CircuitBreaker {
    state: Mutex<BreakerState>,
    consecutive_failures: Mutex<u64>,
    consecutive_successes: Mutex<u64>,
    /// In-flight probe marker while HalfOpen (slot of the admitted probe).
    /// Half-open must admit exactly one probe at a time; otherwise a recovering
    /// breaker floods the provider with unbounded probe traffic (all requests
    /// pass while HalfOpen). A probe older than one cooldown is treated as lost
    /// so an abandoned probe cannot block recovery forever.
    half_open_probe: Mutex<Option<Slot>>,
    max_consecutive_failures: u64,
    success_threshold: u64,
    cooldown_slots: u64,
    last_trip_slot: Mutex<Option<Slot>>,
}

impl CircuitBreaker {
    pub fn new(max_consecutive_failures: u64, cooldown_slots: u64) -> Self {
        Self::with_success_threshold(max_consecutive_failures, cooldown_slots, 3)
    }
    pub fn with_success_threshold(
        max_consecutive_failures: u64,
        cooldown_slots: u64,
        success_threshold: u64,
    ) -> Self {
        Self {
            state: Mutex::new(BreakerState::Closed),
            consecutive_failures: Mutex::new(0),
            consecutive_successes: Mutex::new(0),
            half_open_probe: Mutex::new(None),
            max_consecutive_failures,
            success_threshold: success_threshold.max(1),
            cooldown_slots,
            last_trip_slot: Mutex::new(None),
        }
    }
    pub fn is_allowed(&self, current_slot: Slot) -> bool {
        let mut state = self.state.lock();
        match *state {
            BreakerState::Closed => true,
            BreakerState::Open => {
                let last = *self.last_trip_slot.lock();
                match last {
                    Some(trip) if current_slot.0.saturating_sub(trip.0) >= self.cooldown_slots => {
                        *state = BreakerState::HalfOpen;
                        *self.consecutive_successes.lock() = 0;
                        *self.half_open_probe.lock() = Some(current_slot);
                        true
                    }
                    _ => false,
                }
            }
            BreakerState::HalfOpen => {
                let mut probe = self.half_open_probe.lock();
                let stale = matches!(*probe, Some(s) if current_slot.0.saturating_sub(s.0) >= self.cooldown_slots);
                if probe.is_none() || stale {
                    *probe = Some(current_slot);
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Marks a half-open probe as finished (success path): frees the single
    /// half-open probe slot and applies the success-threshold recovery logic.
    pub fn record_success(&self) {
        let mut state = self.state.lock();
        let mut failures = self.consecutive_failures.lock();
        let mut successes = self.consecutive_successes.lock();
        *self.half_open_probe.lock() = None;
        *failures = 0;
        match *state {
            BreakerState::HalfOpen => {
                *successes += 1;
                if *successes >= self.success_threshold {
                    *state = BreakerState::Closed;
                    *successes = 0;
                }
            }
            _ => {
                *state = BreakerState::Closed;
                *successes = 0;
            }
        }
    }

    /// Marks a probe/result as failed. While HalfOpen, a single failure MUST
    /// re-trip the breaker immediately (standard circuit-breaker semantics);
    /// previously the breaker stayed half-open until the failure counter
    /// re-reached its max, letting full traffic through after one probe error
    /// (RISK-001).
    pub fn record_failure(&self, current_slot: Slot) {
        let mut state = self.state.lock();
        let mut failures = self.consecutive_failures.lock();
        if *state == BreakerState::HalfOpen {
            *state = BreakerState::Open;
            *self.last_trip_slot.lock() = Some(current_slot);
            *self.consecutive_successes.lock() = 0;
            *self.half_open_probe.lock() = None;
            *failures = 0;
            warn!("circuit breaker re-opened: half-open probe failed");
            return;
        }
        *failures += 1;
        if *failures >= self.max_consecutive_failures {
            *state = BreakerState::Open;
            *self.last_trip_slot.lock() = Some(current_slot);
            *self.consecutive_successes.lock() = 0;
            *self.half_open_probe.lock() = None;
            warn!("circuit breaker opened after {} failures", *failures);
        }
    }

    pub fn trip(&self, current_slot: Slot, reason: &str) {
        *self.state.lock() = BreakerState::Open;
        *self.last_trip_slot.lock() = Some(current_slot);
        *self.consecutive_successes.lock() = 0;
        *self.half_open_probe.lock() = None;
        warn!("circuit breaker tripped: {}", reason);
    }

    pub fn reset(&self) {
        *self.state.lock() = BreakerState::Closed;
        *self.consecutive_failures.lock() = 0;
        *self.consecutive_successes.lock() = 0;
        *self.half_open_probe.lock() = None;
        *self.last_trip_slot.lock() = None;
    }
    pub fn state(&self) -> BreakerState {
        *self.state.lock()
    }
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new(8, 10)
    }
}

pub struct KillSwitch {
    enabled: Mutex<bool>,
    file_path: Mutex<Option<String>>,
    persistence_error: Mutex<Option<String>>,
}

/// True when the mainnet master switch `ARB_LIVE` is enabled. While false,
/// debug-only env bypasses (e.g. `KILL_SWITCH_DISABLED`) are permitted; while
/// true, safety gates may never be dismissed from the environment.
fn live_master_switch_enabled() -> bool {
    std::env::var("ARB_LIVE").ok().is_some_and(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

impl KillSwitch {
    pub fn new() -> Self {
        Self {
            enabled: Mutex::new(false),
            file_path: Mutex::new(None),
            persistence_error: Mutex::new(None),
        }
    }

    /// Durable kill switch. An existing marker is loaded as BLOCKED so a
    /// restart cannot silently re-arm execution.
    pub fn with_file(path: impl Into<String>) -> Self {
        let path = path.into();
        let blocked = std::path::Path::new(&path).exists();
        if blocked {
            warn!(path = %path, "kill-switch marker found at startup; restoring blocked state");
        }
        Self {
            enabled: Mutex::new(blocked),
            file_path: Mutex::new(Some(path)),
            persistence_error: Mutex::new(None),
        }
    }

    /// Engage the in-memory block first, then persist it. Returns true only
    /// when the durable marker is confirmed written. False means this instance
    /// has no persistence path; Err means persistence failed. In every case
    /// the in-memory block remains engaged.
    pub fn activate(&self) -> std::io::Result<bool> {
        // Lock ordering: file_path, enabled, persistence_error.
        let file_path = self.file_path.lock().clone();
        let mut enabled = self.enabled.lock();
        *enabled = true;

        let Some(path) = file_path else {
            *self.persistence_error.lock() =
                Some("kill-switch persistence path is not configured".to_string());
            error!("kill switch activated in memory only; no persistence path configured");
            return Ok(false);
        };

        if let Some(parent) = std::path::Path::new(&path).parent()
            && !parent.as_os_str().is_empty()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            *self.persistence_error.lock() = Some(error.to_string());
            error!(path = %path, error = %error, "failed to create kill-switch marker directory");
            return Err(error);
        }

        let persist_result = (|| -> std::io::Result<()> {
            let mut marker = std::fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&path)?;
            std::io::Write::write_all(&mut marker, b"paused\n")?;
            marker.sync_all()
        })();
        if let Err(error) = persist_result {
            *self.persistence_error.lock() = Some(error.to_string());
            error!(path = %path, error = %error, "failed to persist kill-switch marker");
            return Err(error);
        }

        *self.persistence_error.lock() = None;
        error!("kill switch activated and durably persisted");
        Ok(true)
    }

    /// Clear the block only after the durable marker has been removed. A
    /// filesystem failure leaves the in-memory block engaged and is returned.
    pub fn deactivate(&self) -> std::io::Result<()> {
        let file_path = self.file_path.lock().clone();
        let mut enabled = self.enabled.lock();
        if let Some(path) = file_path
            && let Err(error) = std::fs::remove_file(&path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            *self.persistence_error.lock() = Some(error.to_string());
            *enabled = true;
            error!(path = %path, error = %error, "failed to remove kill-switch marker; remaining blocked");
            return Err(error);
        }

        *enabled = false;
        *self.persistence_error.lock() = None;
        warn!("kill switch deactivated by operator");
        Ok(())
    }

    /// True only when a durable marker currently exists on disk.
    pub fn is_durably_blocked(&self) -> bool {
        self.file_path
            .lock()
            .as_ref()
            .is_some_and(|path| std::path::Path::new(path).is_file())
    }

    /// Last persistence error, or None when no error is currently recorded.
    pub fn persistence_error(&self) -> Option<String> {
        self.persistence_error.lock().clone()
    }

    pub fn is_blocked(&self) -> bool {
        // Env override: KILL_SWITCH_DISABLED=1 bypasses the kill switch, but
        // ONLY while the mainnet master switch ARB_LIVE is not enabled. In
        // live mode the kill switch is non-dismissible from the environment —
        // an explicit operator action (/resume) is the only way to unblock.
        static BYPASS_LOG: std::sync::Once = std::sync::Once::new();
        if !live_master_switch_enabled()
            && std::env::var("KILL_SWITCH_DISABLED")
                .ok()
                .is_some_and(|v| v.trim() == "1")
        {
            BYPASS_LOG.call_once(|| {
                warn!("KILL_SWITCH_DISABLED=1 active: kill switch bypassed (non-live mode only)");
            });
            return false;
        }
        if self
            .file_path
            .lock()
            .as_ref()
            .is_some_and(|path| std::path::Path::new(path).exists())
        {
            *self.enabled.lock() = true;
        }
        *self.enabled.lock()
    }
    pub fn check_file(&self, path: &str) -> bool {
        if std::path::Path::new(path).exists() {
            if let Err(error) = self.activate() {
                error!(path = %path, error = %error, "kill switch blocked in memory but persistence failed");
            }
            true
        } else {
            false
        }
    }
    pub fn activate_from_signal(&self) {
        if let Err(error) = self.activate() {
            error!(error = %error, "signal activated in-memory kill switch but persistence failed");
        }
    }
}

impl Default for KillSwitch {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct RiskLimits {
    pub max_trade_size_usd: f64,
    pub min_net_profit_abs: u64,
    pub min_net_profit_bps: u64,
    pub max_price_impact_bps: u64,
    pub max_state_age_slots: u64,
    pub max_tip_share_bps: u64,
    pub max_tip_lamports: u64,
    pub max_daily_loss_usd: f64,
    pub max_hourly_loss_usd: f64,
    // Phase E: new enforcement configs
    pub min_landing_rate: f64,
    pub max_inflight_bundles: usize,
    pub max_tip_spend_ratio: f64,
    pub max_orders_per_hour: usize,
}

impl Default for RiskLimits {
    fn default() -> Self {
        Self {
            max_trade_size_usd: 25.0,
            min_net_profit_abs: arb_constants::DEFAULT_MIN_PROFIT_LAMPORTS,
            min_net_profit_bps: 8,
            max_price_impact_bps: 40,
            max_state_age_slots: 2,
            max_tip_share_bps: arb_constants::DEFAULT_JITO_MAX_TIP_SHARE_BPS,
            max_tip_lamports: 2_000_000,
            max_daily_loss_usd: 5.0,
            max_hourly_loss_usd: 2.0,
            // Phase E defaults
            min_landing_rate: 0.02,
            max_inflight_bundles: 2,
            max_tip_spend_ratio: 0.7,
            max_orders_per_hour: 120,
        }
    }
}

/// Configuration for Chronos Spatiotemporal Expected Value (EV) Gate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChronosRiskConfig {
    pub enabled: bool,
    /// Hazard decay rate per second (lambda in s^-1, default 3.0)
    pub decay_lambda_per_sec: f64,
    /// Maximum allowable age in milliseconds before auto-dropping (default 400ms = 1 slot)
    pub max_age_ms: u64,
    /// Minimum expected value in USD required for execution approval (default $0.05)
    pub min_ev_usd: f64,
    /// Estimated failed transaction cost overhead in lamports
    pub failed_attempt_cost_lamports: u64,
    /// Dimensionless risk penalty factor per hop
    pub beta_risk_factor: f64,
}

impl Default for ChronosRiskConfig {
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

/// Chronos Expected Value (EV) evaluation input parameters.
///
/// `net_profit_success_lamports` is already the canonical, fully-costed success
/// PnL produced by `arb-math`. Do not pass gross PnL and re-deduct flash/DEX/tip
/// costs here: doing so creates a second economic calculator and can flip a
/// positive canonical result negative through duplicate costs or unit rounding.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChronosEvInput {
    /// Latency since quote ingestion in milliseconds.
    pub age_ms: u64,
    /// Maximum allowable quote age in milliseconds (100..=5000).
    pub max_age_ms: u64,
    /// Hazard decay rate per second. Converted once to conservative micro-units.
    pub decay_lambda_per_sec: f64,
    /// Canonical net PnL after flash fee, base fee, priority fee and actual tip.
    pub net_profit_success_lamports: i128,
    /// Network fee paid when a transaction is attempted but the bundle fails.
    pub base_network_fee_lamports: u64,
    /// Priority fee paid when a transaction is attempted but the bundle fails.
    pub priority_fee_lamports: u64,
    /// Additional verified failed-attempt overhead in lamports.
    pub failed_attempt_cost_lamports: u64,
    /// Fresh USD per SOL reference, used only for the EV display/threshold conversion.
    pub sol_price_usd: f64,
    /// Minimum expected value in USD required for approval.
    pub min_ev_usd: f64,
}

/// Chronos result. Floating-point fields are display-only projections; the
/// approval decision uses the exact integer/fixed-point fields below.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChronosEvResult {
    /// Display-only survival probability in [0, 1].
    pub survival_probability: f64,
    /// Exact Q32 survival probability (1.0 == 2^32).
    pub survival_probability_q32: u64,
    /// Canonical net PnL on success, in lamports.
    pub net_profit_success_lamports: i128,
    /// PnL on failure, in lamports (network + priority + failed-attempt costs).
    pub net_profit_failure_lamports: i128,
    /// Display-only projection of `expected_value_lamports_exact`.
    pub expected_value_lamports: f64,
    /// Exact expected value in integer lamports, rounded down conservatively.
    pub expected_value_lamports_exact: i128,
    /// Display-only projection of `expected_value_usd_micros`.
    pub expected_value_usd: f64,
    /// Exact expected value in USD micros, rounded down conservatively.
    pub expected_value_usd_micros: i128,
    /// Approval flag computed exclusively from integer/fixed-point values.
    pub is_eligible: bool,
}

const CHRONOS_Q32_ONE: u64 = 1_u64 << 32;
const CHRONOS_EXP_TERMS: u128 = 256;
const CHRONOS_LAMBDA_SCALE: f64 = 1_000_000.0;
const CHRONOS_MAX_LAMBDA_MICROS_PER_SEC: u64 = 1_000_000_000_000; // 1e6 s^-1
const USD_MICROS_PER_USD: f64 = 1_000_000.0;
const LAMPORTS_PER_SOL_I128: i128 = 1_000_000_000;

/// Convert a finite non-negative configuration float to a scaled integer.
/// Ceiling is used for decay/minimum thresholds; floor is used for SOL price,
/// so the EV gate cannot become more permissive due to decimal conversion.
fn chronos_scaled_u64(value: f64, scale: f64, round_up: bool, name: &str) -> Result<u64, String> {
    if !value.is_finite() || value < 0.0 {
        return Err(format!("{name} must be finite and non-negative"));
    }
    let scaled = value * scale;
    if !scaled.is_finite() || scaled >= u64::MAX as f64 {
        return Err(format!("{name} is outside the supported fixed-point range"));
    }
    let rounded = if round_up {
        scaled.ceil()
    } else {
        scaled.floor()
    };
    Ok(rounded as u64)
}

#[inline]
fn chronos_mul_q32(lhs: u64, rhs: u64) -> u64 {
    ((u128::from(lhs) * u128::from(rhs)) >> 32) as u64
}

/// Conservative lower bound for exp(-lambda * age).
///
/// For x >= 0, (1 - x/n)^n <= exp(-x) when x/n <= 1. We evaluate the
/// 256th power in Q32 with floor-rounded products, preserving the lower bound.
/// This is deterministic, allocation-free and needs only eight multiplications.
fn chronos_survival_q32(
    age_ms: u64,
    max_age_ms: u64,
    lambda_micros_per_sec: u64,
) -> Result<u64, String> {
    if age_ms > max_age_ms {
        return Ok(0);
    }
    if lambda_micros_per_sec == 0 || age_ms == 0 {
        return Ok(CHRONOS_Q32_ONE);
    }

    // x(Q32) = lambda(micro/s) * age(ms) * 2^32 / 1e9.
    // Ceiling prevents an optimistic underestimate of decay.
    let denominator = 1_000_000_000_u128;
    let numerator = u128::from(lambda_micros_per_sec)
        .checked_mul(u128::from(age_ms))
        .and_then(|v| v.checked_mul(u128::from(CHRONOS_Q32_ONE)))
        .ok_or_else(|| "Chronos decay exponent overflow".to_string())?;
    let x_q32 = numerator
        .checked_add(denominator - 1)
        .ok_or_else(|| "Chronos decay exponent rounding overflow".to_string())?
        / denominator;

    // At x >= 32, exp(-x) is below Q32 resolution; zero is conservative.
    if x_q32 >= 32_u128 * u128::from(CHRONOS_Q32_ONE) {
        return Ok(0);
    }

    let reduction = x_q32
        .checked_add(CHRONOS_EXP_TERMS - 1)
        .ok_or_else(|| "Chronos range-reduction overflow".to_string())?
        / CHRONOS_EXP_TERMS;
    let base = u64::try_from(u128::from(CHRONOS_Q32_ONE) - reduction)
        .map_err(|_| "Chronos range-reduction underflow".to_string())?;

    let mut exponent = CHRONOS_EXP_TERMS;
    let mut factor = base;
    let mut result = CHRONOS_Q32_ONE;
    while exponent > 0 {
        if exponent & 1 == 1 {
            result = chronos_mul_q32(result, factor);
        }
        exponent >>= 1;
        if exponent > 0 {
            factor = chronos_mul_q32(factor, factor);
        }
    }
    Ok(result.min(CHRONOS_Q32_ONE))
}

fn format_signed_usd_micros(value: i128) -> String {
    let magnitude = value.unsigned_abs();
    let sign = if value < 0 { "-" } else { "" };
    format!(
        "{sign}{}.{:06}",
        magnitude / 1_000_000,
        magnitude % 1_000_000
    )
}

/// Evaluates EV using the canonical fully-costed success PnL:
/// EV = P(survival) * net_success + (1-P(survival)) * net_failure.
/// All approval arithmetic is fixed-point/integer; floats are used only to
/// ingest legacy TOML numeric fields and to produce human-readable projections.
pub fn evaluate_chronos_ev(input: ChronosEvInput) -> Result<ChronosEvResult, String> {
    if input.max_age_ms == 0 || input.max_age_ms > 5_000 {
        return Err("max_age_ms must be in 1..=5000".to_string());
    }
    if input.net_profit_success_lamports <= 0 {
        return Err("canonical net success PnL must be positive".to_string());
    }
    if !input.sol_price_usd.is_finite() || input.sol_price_usd <= 0.0 {
        return Err("sol_price_usd must be positive and finite".to_string());
    }

    let lambda_micros = chronos_scaled_u64(
        input.decay_lambda_per_sec,
        CHRONOS_LAMBDA_SCALE,
        true,
        "decay_lambda_per_sec",
    )?;
    if lambda_micros > CHRONOS_MAX_LAMBDA_MICROS_PER_SEC {
        return Err("decay_lambda_per_sec exceeds the supported maximum".to_string());
    }
    let sol_price_micros = chronos_scaled_u64(
        input.sol_price_usd,
        USD_MICROS_PER_USD,
        false,
        "sol_price_usd",
    )?;
    if sol_price_micros == 0 {
        return Err("sol_price_usd is below the supported micro-USD precision".to_string());
    }
    let min_ev_micros =
        chronos_scaled_u64(input.min_ev_usd, USD_MICROS_PER_USD, true, "min_ev_usd")?;

    let failure_costs = input
        .base_network_fee_lamports
        .checked_add(input.priority_fee_lamports)
        .and_then(|v| v.checked_add(input.failed_attempt_cost_lamports))
        .ok_or_else(|| "Chronos failure-cost sum overflow".to_string())?;
    let net_profit_failure_lamports = -i128::from(failure_costs);
    let survival_probability_q32 =
        chronos_survival_q32(input.age_ms, input.max_age_ms, lambda_micros)?;

    let probability = i128::from(survival_probability_q32);
    let one_minus_probability = i128::from(CHRONOS_Q32_ONE) - probability;
    let weighted_success = input
        .net_profit_success_lamports
        .checked_mul(probability)
        .ok_or_else(|| "Chronos success EV multiplication overflow".to_string())?;
    let weighted_failure = net_profit_failure_lamports
        .checked_mul(one_minus_probability)
        .ok_or_else(|| "Chronos failure EV multiplication overflow".to_string())?;
    let expected_value_lamports_exact = weighted_success
        .checked_add(weighted_failure)
        .ok_or_else(|| "Chronos EV accumulation overflow".to_string())?
        .div_euclid(i128::from(CHRONOS_Q32_ONE));

    // USD micros = lamports * USD-micros-per-SOL / 1e9. Euclidean division
    // rounds negative values down and positive values down, never overstating EV.
    let expected_value_usd_micros = expected_value_lamports_exact
        .checked_mul(i128::from(sol_price_micros))
        .ok_or_else(|| "Chronos USD conversion overflow".to_string())?
        .div_euclid(LAMPORTS_PER_SOL_I128);

    let is_eligible = input.age_ms <= input.max_age_ms
        && input.net_profit_success_lamports > 0
        && expected_value_usd_micros >= i128::from(min_ev_micros);

    Ok(ChronosEvResult {
        survival_probability: survival_probability_q32 as f64 / CHRONOS_Q32_ONE as f64,
        survival_probability_q32,
        net_profit_success_lamports: input.net_profit_success_lamports,
        net_profit_failure_lamports,
        expected_value_lamports: expected_value_lamports_exact as f64,
        expected_value_lamports_exact,
        expected_value_usd: expected_value_usd_micros as f64 / USD_MICROS_PER_USD,
        expected_value_usd_micros,
        is_eligible,
    })
}
/// Compute the conservative minimum net-profit hurdle in Lamports.
/// Input notional and PnL must already be converted to the same SOL/Lamport unit.
pub fn minimum_net_profit_hurdle_lamports(
    amount_in_lamports: Lamports,
    absolute_floor: Lamports,
    min_profit_bps: u64,
) -> Lamports {
    let numerator = u128::from(amount_in_lamports.as_u64()) * u128::from(min_profit_bps);
    let bps_floor = numerator / 10_000 + u128::from(numerator % 10_000 != 0);
    Lamports(
        absolute_floor
            .as_u64()
            .max(u64::try_from(bps_floor).unwrap_or(u64::MAX)),
    )
}
pub struct RiskGate {
    limits: RiskLimits,
    breaker: Arc<CircuitBreaker>,
    kill_switch: Arc<KillSwitch>,
    chronos_config: ChronosRiskConfig,
    daily_loss_usd: Mutex<f64>,
    hourly_loss_usd: Mutex<f64>,
    recent_order_timestamps: Mutex<Vec<std::time::Instant>>,
    // Phase E: inflight bundle tracking for landing rate enforcement
    inflight_bundles: Mutex<std::collections::HashMap<u64, u64>>, // slot -> count
    total_bundles_sent: Mutex<u64>,
    total_bundles_landed: Mutex<u64>,
    last_loss_reset: Mutex<std::time::Instant>,
}

impl RiskGate {
    pub fn new(
        limits: RiskLimits,
        breaker: Arc<CircuitBreaker>,
        kill_switch: Arc<KillSwitch>,
    ) -> Self {
        Self::with_chronos(limits, breaker, kill_switch, ChronosRiskConfig::default())
    }

    pub fn with_chronos(
        limits: RiskLimits,
        breaker: Arc<CircuitBreaker>,
        kill_switch: Arc<KillSwitch>,
        chronos_config: ChronosRiskConfig,
    ) -> Self {
        Self {
            limits,
            breaker,
            kill_switch,
            chronos_config,
            daily_loss_usd: Mutex::new(0.0),
            hourly_loss_usd: Mutex::new(0.0),
            recent_order_timestamps: Mutex::new(Vec::new()),
            inflight_bundles: Mutex::new(std::collections::HashMap::new()),
            total_bundles_sent: Mutex::new(0),
            total_bundles_landed: Mutex::new(0),
            last_loss_reset: Mutex::new(std::time::Instant::now()),
        }
    }

    pub fn chronos_config(&self) -> ChronosRiskConfig {
        self.chronos_config
    }

    pub fn evaluate_chronos_gate(&self, input: ChronosEvInput) -> Result<ChronosEvResult, String> {
        // No profit-dependent age relaxation: stale quotes remain stale regardless
        // of their apparent gross edge.
        let result = evaluate_chronos_ev(input)?;
        if self.chronos_config.enabled && !result.is_eligible {
            let survival_hundredths = (u128::from(result.survival_probability_q32) * 10_000)
                / u128::from(CHRONOS_Q32_ONE);
            return Err(format!(
                "chronos EV gate rejected: age={}ms (max {}), EV=${} (min ${:.4}), P_surv={}.{:02}%",
                input.age_ms,
                input.max_age_ms,
                format_signed_usd_micros(result.expected_value_usd_micros),
                input.min_ev_usd,
                survival_hundredths / 100,
                survival_hundredths % 100
            ));
        }
        Ok(result)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn evaluate_full(
        &self,
        opportunity_net: Lamports,
        amount_in_lamports: Lamports,
        current_slot: Slot,
        state_age_slots: u64,
        trade_size_usd: f64,
        tip_lamports: u64,
        tip_share_bps: u64,
        price_impact_bps: u64,
    ) -> Result<(), String> {
        if self.kill_switch.is_blocked() {
            return Err("kill switch active".to_string());
        }
        if !self.breaker.is_allowed(current_slot) {
            return Err("circuit breaker open".to_string());
        }
        self.check_trade_size(trade_size_usd)?;
        self.check_velocity()?;
        self.check_tip_share(tip_share_bps)?;
        self.check_tip_lamports(tip_lamports)?;
        self.check_price_impact(price_impact_bps)?;
        self.check_state_age(state_age_slots)?;
        self.check_min_profit(opportunity_net, amount_in_lamports)?;
        self.check_losses()?;
        // Funding mode must not weaken the common execution risk policy.
        self.check_max_inflight_bundles(current_slot)?;
        self.check_tip_spend_ratio(tip_lamports, opportunity_net)?;
        self.check_min_landing_rate()?;
        Ok(())
    }
    pub fn check_velocity(&self) -> Result<(), String> {
        let now = std::time::Instant::now();
        let one_hour = std::time::Duration::from_secs(3600);
        let mut orders = self.recent_order_timestamps.lock();
        orders.retain(|t| now.duration_since(*t) < one_hour);
        let max_vel = self.limits.max_orders_per_hour;
        if orders.len() >= max_vel {
            return Err(format!(
                "order velocity {} >= max {}/hr",
                orders.len(),
                max_vel
            ));
        }
        Ok(())
    }

    pub fn record_order(&self) {
        self.recent_order_timestamps
            .lock()
            .push(std::time::Instant::now());
    }

    pub fn check_trade_size(&self, size_usd: f64) -> Result<(), String> {
        let limit = self.limits.max_trade_size_usd;
        if !size_usd.is_finite() || size_usd <= 0.0 {
            return Err(format!(
                "trade size must be positive and finite, got {size_usd}"
            ));
        }
        if !limit.is_finite() || limit <= 0.0 {
            return Err(format!("max_trade_size_usd is invalid: {limit}"));
        }
        if size_usd > limit {
            return Err(format!("trade size {size_usd} > max {limit}"));
        }
        Ok(())
    }

    /// Full risk evaluation for flash-loan routes, including the configured USD notional cap.
    #[allow(clippy::too_many_arguments)]
    pub fn evaluate_flash_loan(
        &self,
        opportunity_net: Lamports,
        amount_in_lamports: Lamports,
        current_slot: Slot,
        state_age_slots: u64,
        trade_size_usd: f64,
        tip_lamports: u64,
        tip_share_bps: u64,
        price_impact_bps: u64,
    ) -> Result<(), String> {
        if self.kill_switch.is_blocked() {
            return Err("kill switch active".to_string());
        }
        self.check_trade_size(trade_size_usd)?;
        if !self.breaker.is_allowed(current_slot) {
            return Err("circuit breaker open".to_string());
        }
        self.check_velocity()?;
        self.check_tip_share(tip_share_bps)?;
        self.check_tip_lamports(tip_lamports)?;
        self.check_price_impact(price_impact_bps)?;
        self.check_state_age(state_age_slots)?;
        self.check_min_profit(opportunity_net, amount_in_lamports)?;
        self.check_losses()?;

        // Phase E: new enforcement checks
        self.check_max_inflight_bundles(current_slot)?;
        self.check_tip_spend_ratio(tip_lamports, opportunity_net)?;
        self.check_min_landing_rate()?;

        Ok(())
    }

    pub fn check_tip_share(&self, tip_share_bps: u64) -> Result<(), String> {
        if tip_share_bps > self.limits.max_tip_share_bps {
            return Err(format!(
                "tip share {} > max {}",
                tip_share_bps, self.limits.max_tip_share_bps
            ));
        }
        Ok(())
    }
    pub fn check_tip_lamports(&self, tip_lamports: u64) -> Result<(), String> {
        if tip_lamports > self.limits.max_tip_lamports {
            return Err(format!(
                "tip {} > max {}",
                tip_lamports, self.limits.max_tip_lamports
            ));
        }
        Ok(())
    }
    pub fn check_price_impact(&self, price_impact_bps: u64) -> Result<(), String> {
        if price_impact_bps > self.limits.max_price_impact_bps {
            return Err(format!(
                "price impact {} > max {}",
                price_impact_bps, self.limits.max_price_impact_bps
            ));
        }
        Ok(())
    }
    pub fn check_state_age(&self, state_age_slots: u64) -> Result<(), String> {
        if state_age_slots > self.limits.max_state_age_slots {
            return Err(format!(
                "state age {} > max {}",
                state_age_slots, self.limits.max_state_age_slots
            ));
        }
        Ok(())
    }
    pub fn check_min_profit(
        &self,
        opportunity_net: Lamports,
        amount_in_lamports: Lamports,
    ) -> Result<(), String> {
        if opportunity_net.as_u64() < self.limits.min_net_profit_abs {
            return Err(format!(
                "net profit {} < min_abs {}",
                opportunity_net.as_u64(),
                self.limits.min_net_profit_abs
            ));
        }
        let min_bps_amount = minimum_net_profit_hurdle_lamports(
            amount_in_lamports,
            Lamports(0),
            self.limits.min_net_profit_bps,
        )
        .as_u64();
        if opportunity_net.as_u64() < min_bps_amount {
            return Err(format!("net profit {} < min_bps", opportunity_net.as_u64()));
        }
        Ok(())
    }
    pub fn check_losses(&self) -> Result<(), String> {
        let daily = *self.daily_loss_usd.lock();
        let hourly = *self.hourly_loss_usd.lock();
        if daily > self.limits.max_daily_loss_usd {
            return Err(format!(
                "daily loss {} > max {}",
                daily, self.limits.max_daily_loss_usd
            ));
        }
        if hourly > self.limits.max_hourly_loss_usd {
            return Err(format!(
                "hourly loss {} > max {}",
                hourly, self.limits.max_hourly_loss_usd
            ));
        }
        Ok(())
    }

    /// Check that the number of inflight bundles for this slot doesn't exceed the limit.
    fn check_max_inflight_bundles(&self, current_slot: Slot) -> Result<(), String> {
        let mut inflight = self.inflight_bundles.lock();
        // Clean up old slots (keep current and previous slot)
        inflight.retain(|&slot, _| slot + 1 >= current_slot.0);
        let count = inflight.get(&current_slot.0).copied().unwrap_or(0);
        let max_bundles = self.limits.max_inflight_bundles as u64;
        if count >= max_bundles {
            return Err(format!(
                "inflight bundles {} >= max {} for slot {}",
                count, max_bundles, current_slot.0
            ));
        }
        Ok(())
    }

    /// Check that tip spend ratio doesn't exceed the configured limit.
    /// tip_spend_ratio = tip_lamports / (tip_lamports + net_profit)
    fn check_tip_spend_ratio(
        &self,
        tip_lamports: u64,
        opportunity_net: Lamports,
    ) -> Result<(), String> {
        let net = opportunity_net.as_u64();
        if net == 0 {
            return Ok(()); // No profit to measure ratio against
        }
        let total = tip_lamports.saturating_add(net);
        let ratio = tip_lamports as f64 / total as f64;
        if ratio > self.limits.max_tip_spend_ratio {
            return Err(format!(
                "tip spend ratio {:.2} > max {:.2}",
                ratio, self.limits.max_tip_spend_ratio
            ));
        }
        Ok(())
    }

    /// Check that the historical landing rate meets the minimum threshold.
    fn check_min_landing_rate(&self) -> Result<(), String> {
        let total_sent = *self.total_bundles_sent.lock();
        let total_landed = *self.total_bundles_landed.lock();
        if total_sent < 20 || self.limits.min_landing_rate <= 0.0 {
            return Ok(()); // Require at least 20 historical attempts before enforcing statistical landing rate floor
        }
        let rate = total_landed as f64 / total_sent as f64;
        if rate < self.limits.min_landing_rate {
            return Err(format!(
                "landing rate {:.2}% < min {:.2}% (landed {} / sent {})",
                rate * 100.0,
                self.limits.min_landing_rate * 100.0,
                total_landed,
                total_sent
            ));
        }
        Ok(())
    }
    pub fn record_loss(&self, loss_usd: f64) {
        *self.daily_loss_usd.lock() += loss_usd;
        *self.hourly_loss_usd.lock() += loss_usd;
    }

    /// Record a bundle sent for landing rate tracking.
    pub fn record_bundle_sent(&self, current_slot: Slot) {
        *self.total_bundles_sent.lock() += 1;
        let mut inflight = self.inflight_bundles.lock();
        *inflight.entry(current_slot.0).or_insert(0) += 1;
    }

    /// Record a bundle landing result for landing rate tracking.
    pub fn record_bundle_landed(&self, current_slot: Slot, landed: bool) {
        if landed {
            *self.total_bundles_landed.lock() += 1;
        }
        let mut inflight = self.inflight_bundles.lock();
        if let Some(count) = inflight.get_mut(&current_slot.0) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                inflight.remove(&current_slot.0);
            }
        }
    }
    pub fn daily_loss_usd(&self) -> f64 {
        *self.daily_loss_usd.lock()
    }
    pub fn hourly_loss_usd(&self) -> f64 {
        *self.hourly_loss_usd.lock()
    }
    pub fn reset_losses(&self) {
        *self.daily_loss_usd.lock() = 0.0;
        *self.hourly_loss_usd.lock() = 0.0;
        *self.last_loss_reset.lock() = std::time::Instant::now();
    }
    /// Periodic loss window reset (hourly and daily windows).
    pub fn maybe_reset_periodic(&self) {
        let mut last_reset = self.last_loss_reset.lock();
        let now = std::time::Instant::now();
        if now.duration_since(*last_reset) >= std::time::Duration::from_secs(3600) {
            *self.hourly_loss_usd.lock() = 0.0;
            if now.duration_since(*last_reset) >= std::time::Duration::from_secs(86400) {
                *self.daily_loss_usd.lock() = 0.0;
                *last_reset = now;
            }
        }
    }
    pub fn on_failure(&self, current_slot: Slot) {
        self.breaker.record_failure(current_slot);
    }
    pub fn is_breaker_open(&self, current_slot: Slot) -> bool {
        !self.breaker.is_allowed(current_slot)
    }

    pub fn on_success(&self) {
        self.breaker.record_success();
    }
    pub fn breaker(&self) -> Arc<CircuitBreaker> {
        self.breaker.clone()
    }
    pub fn kill_switch(&self) -> Arc<KillSwitch> {
        self.kill_switch.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn breaker_half_open_success_closes() {
        let cb = CircuitBreaker::with_success_threshold(1, 1, 2);
        cb.record_failure(Slot(1));
        assert_eq!(cb.state(), BreakerState::Open);
        assert!(cb.is_allowed(Slot(2)));
        assert_eq!(cb.state(), BreakerState::HalfOpen);
        cb.record_success();
        assert_eq!(cb.state(), BreakerState::HalfOpen);
        cb.record_success();
        assert_eq!(cb.state(), BreakerState::Closed);
    }
    #[test]
    fn breaker_fail_closed() {
        let cb = CircuitBreaker::new(1, 100);
        cb.record_failure(Slot(1));
        assert!(!cb.is_allowed(Slot(2)));
    }
    #[test]
    fn kill_switch_blocks() {
        let _env_lock = ENV_LOCK.lock();
        let ks = KillSwitch::new();
        assert!(!ks.is_blocked());
        ks.activate().unwrap();
        assert!(ks.is_blocked());
        ks.deactivate().unwrap();
        assert!(!ks.is_blocked());
    }
    // Env-touching tests serialize behind ENV_LOCK because env::set_var is
    // process-global and racy under cargo's parallel threads (edition 2024
    // makes it unsafe; is_blocked reads KILL_SWITCH_DISABLED per call).
    static ENV_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
    #[test]
    fn kill_switch_env_bypass_is_live_gated() {
        let _env_lock = ENV_LOCK.lock();
        // SAFETY: guarded by ENV_LOCK; no other activity in this binary can
        // read or write these vars while held.
        unsafe { std::env::remove_var("KILL_SWITCH_DISABLED") };
        let ks = KillSwitch::new();
        ks.activate().unwrap();

        // Bypass honored while ARB_LIVE is not enabled.
        unsafe { std::env::set_var("KILL_SWITCH_DISABLED", "1") };
        assert!(!ks.is_blocked(), "bypass must apply while not live");

        // ARB_LIVE=1 makes the kill switch non-dismissible.
        unsafe { std::env::set_var("ARB_LIVE", "1") };
        assert!(ks.is_blocked(), "live mode must enforce the kill switch");
        unsafe { std::env::set_var("ARB_LIVE", "true") };
        assert!(ks.is_blocked());
        unsafe { std::env::set_var("ARB_LIVE", "0") };
        assert!(
            !ks.is_blocked(),
            "bypass applies again once ARB_LIVE is off"
        );

        // Non-"1" values of KILL_SWITCH_DISABLED never bypass.
        unsafe { std::env::set_var("KILL_SWITCH_DISABLED", "true") };
        assert!(ks.is_blocked(), "exact '1' is required");
        unsafe { std::env::set_var("KILL_SWITCH_DISABLED", " 1 ") };
        assert!(!ks.is_blocked(), "value is trimmed");

        unsafe {
            std::env::remove_var("KILL_SWITCH_DISABLED");
            std::env::remove_var("ARB_LIVE");
        }
        assert!(ks.is_blocked(), "without the bypass the switch enforces");
        ks.deactivate().unwrap();
        assert!(!ks.is_blocked());
    }
    #[test]
    fn kill_switch_without_persistence_path_is_explicitly_memory_only() {
        // This test shares process environment with the live-gated bypass test.
        // Serialize access and clear bypass flags so parallel execution cannot
        // make an active in-memory kill switch appear unblocked.
        let _env_lock = ENV_LOCK.lock();
        unsafe {
            std::env::remove_var("KILL_SWITCH_DISABLED");
            std::env::remove_var("ARB_LIVE");
        }
        let ks = KillSwitch::new();
        assert!(!ks.activate().unwrap());
        assert!(ks.is_blocked());
        assert!(!ks.is_durably_blocked());
        assert!(ks.persistence_error().is_some());
        unsafe {
            std::env::remove_var("KILL_SWITCH_DISABLED");
            std::env::remove_var("ARB_LIVE");
        }
    }

    #[test]
    fn kill_switch_write_failure_keeps_memory_block_engaged() {
        let _env_lock = ENV_LOCK.lock();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("arb-risk-ks-fail-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A directory at the marker path forces the marker write to fail.
        let marker_path = dir.join("marker");
        std::fs::create_dir_all(&marker_path).unwrap();
        let ks = KillSwitch::with_file(marker_path.to_string_lossy());
        assert!(ks.activate().is_err());
        assert!(
            ks.is_blocked(),
            "failed persistence must never clear in-memory block"
        );
        assert!(ks.persistence_error().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kill_switch_remove_failure_keeps_memory_block_engaged() {
        let _env_lock = ENV_LOCK.lock();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("arb-risk-ks-remove-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("marker");
        let ks = KillSwitch::with_file(marker.to_string_lossy());
        assert!(ks.activate().unwrap());
        std::fs::remove_file(&marker).unwrap();
        std::fs::create_dir(&marker).unwrap();
        assert!(ks.deactivate().is_err());
        assert!(
            ks.is_blocked(),
            "failed marker removal must keep the in-memory block"
        );
        assert!(ks.persistence_error().is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn kill_switch_marker_survives_restart() {
        let _env_lock = ENV_LOCK.lock();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("arb-risk-ks-{}-{nanos}", std::process::id()));
        let path = dir.join("kill_switch");
        let path_str = path.to_string_lossy().to_string();

        let ks = KillSwitch::with_file(&path_str);
        assert!(!ks.is_blocked(), "fresh path must start unblocked");
        ks.activate().unwrap();
        assert!(ks.is_blocked());
        assert!(path.exists(), "activation must persist a marker file");
        drop(ks);

        let restarted = KillSwitch::with_file(&path_str);
        assert!(
            restarted.is_blocked(),
            "kill switch must remain blocked after a restart"
        );
        restarted.deactivate().unwrap();
        assert!(!restarted.is_blocked());
        assert!(!path.exists(), "deactivation must remove the marker");
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn inflight_bundle_limit_rejects_third_concurrent_send_and_recovers_on_landing() {
        let rg = RiskGate::new(
            RiskLimits::default(),
            Arc::new(CircuitBreaker::default()),
            Arc::new(KillSwitch::default()),
        );
        rg.record_bundle_sent(Slot(100));
        rg.record_bundle_sent(Slot(100));
        assert!(
            rg.check_max_inflight_bundles(Slot(100)).is_err(),
            "third concurrent inflight bundle must be rejected (default max 2)"
        );
        rg.record_bundle_landed(Slot(100), true);
        assert!(
            rg.check_max_inflight_bundles(Slot(100)).is_ok(),
            "landing must free inflight capacity"
        );
        assert!(
            rg.check_max_inflight_bundles(Slot(102)).is_ok(),
            "stale slots must be cleaned so a later slot has fresh capacity"
        );
    }

    #[test]
    fn inflight_bundle_limit_honors_configured_maximum() {
        let limits = RiskLimits {
            max_inflight_bundles: 1,
            ..RiskLimits::default()
        };
        let rg = RiskGate::new(
            limits,
            Arc::new(CircuitBreaker::default()),
            Arc::new(KillSwitch::default()),
        );
        rg.record_bundle_sent(Slot(50));
        assert!(rg.check_max_inflight_bundles(Slot(50)).is_err());
    }

    #[test]
    fn risk_gate_loss_tracking() {
        let rg = RiskGate::new(
            RiskLimits::default(),
            Arc::new(CircuitBreaker::default()),
            Arc::new(KillSwitch::default()),
        );
        rg.record_loss(3.0);
        assert!((rg.daily_loss_usd() - 3.0).abs() < f64::EPSILON);
    }
    #[test]
    fn risk_gate_checks() {
        let rg = RiskGate::new(
            RiskLimits::default(),
            Arc::new(CircuitBreaker::default()),
            Arc::new(KillSwitch::default()),
        );
        assert!(rg.check_trade_size(10.0).is_ok());
        assert!(rg.check_trade_size(100.0).is_err());
        assert!(rg.check_tip_share(8000).is_ok()); // 80% is within 70-85% dynamic cap
        assert!(rg.check_tip_share(9000).is_err()); // 90% exceeds 85% cap
        assert!(rg.check_price_impact(50).is_err());
        assert!(rg.check_state_age(5).is_err());
    }
    #[test]
    fn panic_trips_breaker() {
        let cb = Arc::new(CircuitBreaker::new(1, 1));
        let hook = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            panic!("boom");
        }));
        assert!(hook.is_err());
        cb.trip(Slot(1), "panic");
        assert_eq!(cb.state(), BreakerState::Open);
    }
    #[test]
    fn kill_switch_under_second() {
        let _env_lock = ENV_LOCK.lock();
        let ks = KillSwitch::new();
        let start = std::time::Instant::now();
        ks.activate().unwrap();
        assert!(ks.is_blocked());
        assert!(start.elapsed().as_millis() < 1000);
    }

    #[test]
    fn test_chronos_ev_gate_profitable() {
        let input = ChronosEvInput {
            age_ms: 20, // 20ms fresh quote
            max_age_ms: 400,
            decay_lambda_per_sec: 3.0,
            net_profit_success_lamports: 4_935_000, // canonical net after all landed costs
            base_network_fee_lamports: 5_000,
            priority_fee_lamports: 10_000,
            failed_attempt_cost_lamports: 10_000,
            sol_price_usd: 150.0,
            min_ev_usd: 0.05,
        };

        let res = evaluate_chronos_ev(input).expect("ev evaluation success");
        assert!(res.is_eligible);
        assert!(res.survival_probability > 0.90);
        assert!(res.net_profit_success_lamports > 4_000_000);
        assert!(res.expected_value_usd > 0.50);
        assert_eq!(res.net_profit_success_lamports, 4_935_000);
        assert_eq!(res.net_profit_failure_lamports, -25_000);
        assert!(res.expected_value_usd_micros > 500_000);
    }

    #[test]
    fn test_chronos_ev_gate_stale_rejection() {
        let input = ChronosEvInput {
            age_ms: 450, // 450ms > max 400ms
            max_age_ms: 400,
            decay_lambda_per_sec: 3.0,
            net_profit_success_lamports: 4_935_000, // canonical net after all landed costs
            base_network_fee_lamports: 5_000,
            priority_fee_lamports: 10_000,
            failed_attempt_cost_lamports: 10_000,
            sol_price_usd: 150.0,
            min_ev_usd: 0.05,
        };

        let res = evaluate_chronos_ev(input).expect("ev calculation");
        assert!(
            !res.is_eligible,
            "Stale quote exceeding max_age_ms must be ineligible"
        );
    }

    #[test]
    fn test_chronos_ev_gate_low_ev_rejection() {
        // High decay and latency making EV negative after failed attempt cost
        let input = ChronosEvInput {
            age_ms: 350,
            max_age_ms: 400,
            decay_lambda_per_sec: 10.0, // Aggressive decay e^(-3.5) ~= 0.03 (3% survival)
            net_profit_success_lamports: 5_000, // Already net of all success costs
            base_network_fee_lamports: 5_000,
            priority_fee_lamports: 10_000,
            failed_attempt_cost_lamports: 20_000,
            sol_price_usd: 150.0,
            min_ev_usd: 0.01,
        };

        let res = evaluate_chronos_ev(input).expect("ev calculation");
        assert!(!res.is_eligible, "Low/Negative EV must be rejected");
        assert!(res.expected_value_lamports < 0.0);
    }
    #[test]
    fn chronos_fixed_point_survival_is_monotone_and_conservative() {
        let fresh = chronos_survival_q32(20, 400, 3_000_000).expect("fresh survival");
        let older = chronos_survival_q32(40, 400, 3_000_000).expect("older survival");
        assert!(fresh > older);
        let probability = fresh as f64 / CHRONOS_Q32_ONE as f64;
        assert!(probability <= (-0.06_f64).exp());
        assert!(probability > 0.9417);
        assert_eq!(
            chronos_survival_q32(0, 400, 3_000_000).unwrap(),
            CHRONOS_Q32_ONE
        );
        assert_eq!(chronos_survival_q32(20, 400, 0).unwrap(), CHRONOS_Q32_ONE);
        assert_eq!(chronos_survival_q32(401, 400, 3_000_000).unwrap(), 0);
    }

    #[test]
    fn chronos_failure_cost_overflow_fails_closed() {
        let input = ChronosEvInput {
            age_ms: 20,
            max_age_ms: 400,
            decay_lambda_per_sec: 3.0,
            net_profit_success_lamports: 1_000_000,
            base_network_fee_lamports: u64::MAX,
            priority_fee_lamports: 1,
            failed_attempt_cost_lamports: 0,
            sol_price_usd: 150.0,
            min_ev_usd: 0.01,
        };
        assert!(evaluate_chronos_ev(input).is_err());
    }
    #[test]
    fn breaker_half_open_admits_single_probe_and_retrips_on_failure() {
        let cb = CircuitBreaker::with_success_threshold(1, 2, 1);
        cb.record_failure(Slot(1));
        assert_eq!(cb.state(), BreakerState::Open);
        assert!(!cb.is_allowed(Slot(2)));
        assert!(cb.is_allowed(Slot(3)));
        assert_eq!(cb.state(), BreakerState::HalfOpen);
        assert!(!cb.is_allowed(Slot(4)));
        cb.record_failure(Slot(4));
        assert_eq!(cb.state(), BreakerState::Open);
        assert!(!cb.is_allowed(Slot(5)));
        assert!(cb.is_allowed(Slot(6)));
    }

    #[test]
    fn breaker_half_open_stale_probe_is_released() {
        let cb = CircuitBreaker::with_success_threshold(1, 2, 1);
        cb.record_failure(Slot(1));
        assert!(cb.is_allowed(Slot(3)));
        assert!(!cb.is_allowed(Slot(4)));
        assert!(!cb.is_allowed(Slot(4)));
        assert!(cb.is_allowed(Slot(5)));
        cb.record_success();
        assert_eq!(cb.state(), BreakerState::Closed);
    }

    #[test]
    fn breaker_half_open_success_frees_probe_slot() {
        let cb = CircuitBreaker::with_success_threshold(2, 2, 2);
        cb.record_failure(Slot(1));
        cb.record_failure(Slot(1));
        assert!(cb.is_allowed(Slot(3)));
        cb.record_success();
        assert_eq!(cb.state(), BreakerState::HalfOpen);
        assert!(cb.is_allowed(Slot(4)));
        cb.record_success();
        assert_eq!(cb.state(), BreakerState::Closed);
    }
}
