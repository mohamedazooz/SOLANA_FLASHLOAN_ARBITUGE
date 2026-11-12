//! Risk management: multi-layer limits, circuit breaker, kill switch.

use arb_types::{Lamports, Slot, TokenAmount};
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
                        true
                    }
                    _ => false,
                }
            }
            BreakerState::HalfOpen => true,
        }
    }
    pub fn record_success(&self) {
        let mut state = self.state.lock();
        let mut failures = self.consecutive_failures.lock();
        let mut successes = self.consecutive_successes.lock();
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
    pub fn record_failure(&self, current_slot: Slot) {
        let mut failures = self.consecutive_failures.lock();
        *failures += 1;
        if *failures >= self.max_consecutive_failures {
            *self.state.lock() = BreakerState::Open;
            *self.last_trip_slot.lock() = Some(current_slot);
            *self.consecutive_successes.lock() = 0;
            warn!("circuit breaker opened after {} failures", *failures);
        }
    }
    pub fn trip(&self, current_slot: Slot, reason: &str) {
        *self.state.lock() = BreakerState::Open;
        *self.last_trip_slot.lock() = Some(current_slot);
        *self.consecutive_successes.lock() = 0;
        warn!("circuit breaker tripped: {}", reason);
    }
    pub fn reset(&self) {
        *self.state.lock() = BreakerState::Closed;
        *self.consecutive_failures.lock() = 0;
        *self.consecutive_successes.lock() = 0;
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
}

impl KillSwitch {
    pub fn new() -> Self {
        Self {
            enabled: Mutex::new(false),
            file_path: Mutex::new(None),
        }
    }

    /// Durable kill switch: the marker file is the source of truth across
    /// restarts. Starts UNBLOCKED by default; operator must explicitly pause
    /// via `/pause` endpoint or `KILL_SWITCH_DISABLED=1` to bypass entirely.
    pub fn with_file(path: impl Into<String>) -> Self {
        let path = path.into();
        // Start unblocked regardless of marker file presence.
        let blocked = false;
        if std::path::Path::new(&path).exists() {
            warn!(path = %path, "kill-switch marker exists at startup; ignoring (start unblocked)");
        }
        Self {
            enabled: Mutex::new(blocked),
            file_path: Mutex::new(Some(path)),
        }
    }

    pub fn activate(&self) {
        // Lock ordering: file_path then enabled, consistent with is_blocked().
        let file_path = self.file_path.lock().clone();
        let mut enabled = self.enabled.lock();
        *enabled = true;
        if let Some(path) = file_path {
            if let Some(parent) = std::path::Path::new(&path).parent()
                && !parent.as_os_str().is_empty()
                && let Err(e) = std::fs::create_dir_all(parent)
            {
                error!(path = %path, error = %e, "failed to create kill-switch marker directory");
            }
            if let Err(e) = std::fs::write(&path, b"paused\n") {
                // Fail safe: the in-memory flag stays set, and the operator is
                // told durability failed so a restart cannot silently re-arm.
                error!(path = %path, error = %e, "failed to persist kill-switch marker");
            }
        }
        error!("kill switch activated");
    }

    pub fn deactivate(&self) {
        let file_path = self.file_path.lock().clone();
        let mut enabled = self.enabled.lock();
        if let Some(path) = file_path
            && let Err(e) = std::fs::remove_file(&path)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            // Keep the in-memory state cleared but warn: the marker survives,
            // so a restart will re-block (fail safe).
            error!(path = %path, error = %e, "failed to remove kill-switch marker; restart will re-block");
        }
        *enabled = false;
        warn!("kill switch deactivated by operator");
    }

    pub fn is_blocked(&self) -> bool {
        // Env override: KILL_SWITCH_DISABLED=1 bypasses kill switch entirely.
        if std::env::var("KILL_SWITCH_DISABLED").ok().as_deref() == Some("1") {
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
            self.activate();
            true
        } else {
            false
        }
    }
    pub fn activate_from_signal(&self) {
        self.activate();
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

pub const MAX_VELOCITY_ORDERS_PER_HOUR: usize = 10;

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
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChronosEvInput {
    /// Latency since quote ingestion in milliseconds (tau = age_ms / 1000.0)
    pub age_ms: u64,
    /// Maximum allowable age before dropping quote (e.g. 400ms = 1 slot)
    pub max_age_ms: u64,
    /// Hazard decay rate per second (lambda in s^-1, default 3.0)
    pub decay_lambda_per_sec: f64,
    /// Gross profit in lamports
    pub gross_profit_lamports: u64,
    /// Kamino flash loan fee in lamports (e.g. 5 bps)
    pub flash_fee_lamports: u64,
    /// Base network signature fee (default 5,000 lamports)
    pub base_network_fee_lamports: u64,
    /// Priority compute fee in lamports (CU_price * CU_limit / 10^6)
    pub priority_fee_lamports: u64,
    /// Jito tip in lamports (only deducted on landing success in private bundles)
    pub jito_tip_lamports: u64,
    /// Additional failed attempt cost in lamports (opportunity cost, simulation, dropped bundle overhead)
    pub failed_attempt_cost_lamports: u64,
    /// Conversion rate: USD per SOL
    pub sol_price_usd: f64,
    /// Minimum EV in USD required to approve execution (e.g. $0.05)
    pub min_ev_usd: f64,
}

/// Result of Chronos EV Gate evaluation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChronosEvResult {
    /// Survival probability: P_survival = exp(-lambda * tau)
    pub survival_probability: f64,
    /// Net profit on success in lamports (Gross - flash - base - priority - tip)
    pub net_profit_success_lamports: i64,
    /// Net profit on failure in lamports (-base - priority - failed_cost)
    pub net_profit_failure_lamports: i64,
    /// Expected Value in lamports: EV = P * Pi_success + (1 - P) * Pi_failure
    pub expected_value_lamports: f64,
    /// Expected Value converted to USD
    pub expected_value_usd: f64,
    /// Approval flag: age <= max_age && Pi_success > 0 && EV >= min_ev_usd
    pub is_eligible: bool,
}

/// Evaluates survival probability and rigorous Expected Value (EV):
///   EV = P_success * Pi_success + (1 - P_success) * Pi_failure
pub fn evaluate_chronos_ev(input: ChronosEvInput) -> Result<ChronosEvResult, String> {
    if input.max_age_ms == 0 {
        return Err("max_age_ms must be > 0".to_string());
    }
    if !input.decay_lambda_per_sec.is_finite() || input.decay_lambda_per_sec < 0.0 {
        return Err("decay_lambda must be non-negative and finite".to_string());
    }
    if !input.sol_price_usd.is_finite() || input.sol_price_usd <= 0.0 {
        return Err("sol_price_usd must be positive and finite".to_string());
    }

    let tau_seconds = input.age_ms as f64 / 1_000.0;
    let survival_probability = (-input.decay_lambda_per_sec * tau_seconds).exp();

    // Pi_success: All costs deducted (including Jito tip and flash fee)
    let total_success_costs = input
        .flash_fee_lamports
        .saturating_add(input.base_network_fee_lamports)
        .saturating_add(input.priority_fee_lamports)
        .saturating_add(input.jito_tip_lamports);

    let net_profit_success_lamports = (input.gross_profit_lamports as i128
        - total_success_costs as i128)
        .clamp(i64::MIN as i128, i64::MAX as i128) as i64;

    // Pi_failure: On failure in Jito private bundle, tip is NOT deducted, but network fee and fail overhead are paid
    let total_failure_costs = input
        .base_network_fee_lamports
        .saturating_add(input.priority_fee_lamports)
        .saturating_add(input.failed_attempt_cost_lamports);

    let net_profit_failure_lamports =
        (-(total_failure_costs as i128)).clamp(i64::MIN as i128, i64::MAX as i128) as i64;

    // EV = P * Pi_success + (1 - P) * Pi_failure
    let expected_value_lamports = (survival_probability * net_profit_success_lamports as f64)
        + ((1.0 - survival_probability) * net_profit_failure_lamports as f64);

    let expected_value_usd = (expected_value_lamports / 1_000_000_000.0) * input.sol_price_usd;

    let is_eligible = input.age_ms <= input.max_age_ms
        && net_profit_success_lamports > 0
        && expected_value_usd >= input.min_ev_usd;

    Ok(ChronosEvResult {
        survival_probability,
        net_profit_success_lamports,
        net_profit_failure_lamports,
        expected_value_lamports,
        expected_value_usd,
        is_eligible,
    })
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
            return Err(format!(
                "chronos EV gate rejected: age={}ms (max {}), EV=${:.4} (min ${:.4}), P_surv={:.2}%",
                input.age_ms,
                input.max_age_ms,
                result.expected_value_usd,
                input.min_ev_usd,
                result.survival_probability * 100.0
            ));
        }
        Ok(result)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn evaluate_full(
        &self,
        opportunity_net: Lamports,
        amount_in: TokenAmount,
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
        self.check_min_profit(opportunity_net, amount_in)?;
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
        amount_in: TokenAmount,
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
        self.check_min_profit(opportunity_net, amount_in)?;
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
        amount_in: TokenAmount,
    ) -> Result<(), String> {
        if opportunity_net.as_u64() < self.limits.min_net_profit_abs {
            return Err(format!(
                "net profit {} < min_abs {}",
                opportunity_net.as_u64(),
                self.limits.min_net_profit_abs
            ));
        }
        let min_bps_amount =
            amount_in.as_u64() as u128 * self.limits.min_net_profit_bps as u128 / 10_000;
        if (opportunity_net.as_u64() as u128) < min_bps_amount {
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
        let ks = KillSwitch::new();
        assert!(!ks.is_blocked());
        ks.activate();
        assert!(ks.is_blocked());
        ks.deactivate();
        assert!(!ks.is_blocked());
    }
    #[test]
    fn kill_switch_marker_survives_restart() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("arb-risk-ks-{}-{nanos}", std::process::id()));
        let path = dir.join("kill_switch");
        let path_str = path.to_string_lossy().to_string();

        let ks = KillSwitch::with_file(&path_str);
        assert!(!ks.is_blocked(), "fresh path must start unblocked");
        ks.activate();
        assert!(ks.is_blocked());
        assert!(path.exists(), "activation must persist a marker file");
        drop(ks);

        let restarted = KillSwitch::with_file(&path_str);
        assert!(
            restarted.is_blocked(),
            "kill switch must remain blocked after a restart"
        );
        restarted.deactivate();
        assert!(!restarted.is_blocked());
        assert!(!path.exists(), "deactivation must remove the marker");
        let _ = std::fs::remove_dir_all(&dir);
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
        let ks = KillSwitch::new();
        let start = std::time::Instant::now();
        ks.activate();
        assert!(ks.is_blocked());
        assert!(start.elapsed().as_millis() < 1000);
    }

    #[test]
    fn test_chronos_ev_gate_profitable() {
        let input = ChronosEvInput {
            age_ms: 20, // 20ms fresh quote
            max_age_ms: 400,
            decay_lambda_per_sec: 3.0,
            gross_profit_lamports: 10_000_000, // 0.01 SOL ($1.50 at $150/SOL)
            flash_fee_lamports: 50_000,
            base_network_fee_lamports: 5_000,
            priority_fee_lamports: 10_000,
            jito_tip_lamports: 5_000_000, // 50% tip
            failed_attempt_cost_lamports: 10_000,
            sol_price_usd: 150.0,
            min_ev_usd: 0.05,
        };

        let res = evaluate_chronos_ev(input).expect("ev evaluation success");
        assert!(res.is_eligible);
        assert!(res.survival_probability > 0.90);
        assert!(res.net_profit_success_lamports > 4_000_000);
        assert!(res.expected_value_usd > 0.50);
    }

    #[test]
    fn test_chronos_ev_gate_stale_rejection() {
        let input = ChronosEvInput {
            age_ms: 450, // 450ms > max 400ms
            max_age_ms: 400,
            decay_lambda_per_sec: 3.0,
            gross_profit_lamports: 10_000_000,
            flash_fee_lamports: 50_000,
            base_network_fee_lamports: 5_000,
            priority_fee_lamports: 10_000,
            jito_tip_lamports: 5_000_000,
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
            gross_profit_lamports: 50_000, // Very small profit
            flash_fee_lamports: 5_000,
            base_network_fee_lamports: 5_000,
            priority_fee_lamports: 10_000,
            jito_tip_lamports: 25_000,
            failed_attempt_cost_lamports: 20_000,
            sol_price_usd: 150.0,
            min_ev_usd: 0.01,
        };

        let res = evaluate_chronos_ev(input).expect("ev calculation");
        assert!(!res.is_eligible, "Low/Negative EV must be rejected");
        assert!(res.expected_value_lamports < 0.0);
    }
}
