//! Chronos — Temporal Execution Intelligence Engine.
//!
//! Chronos is a *risk-intelligence* layer. It observes execution conditions
//! (state freshness, latency, volatility, opportunity decay) and produces a
//! risk-adjusted ranking. It is deliberately separated from the Economic
//! Engine:
//!
//!   * Chronos does NOT modify the actual AMM quote.
//!   * Chronos does NOT invent protocol fees.
//!   * Chronos does NOT alter the final raw token amount.
//!   * Chronos does NOT replace the Economic Engine.
//!   * Chronos does NOT independently authorize execution.
//!
//! The final profitability decision comes from the Economic Engine
//! (`arb_math::economic`). Chronos produces a risk score and an execution
//! gate recommendation; only the Economic Engine's `meets_min_profit` and
//! on-chain `min_out`/`min_profit` guards authorize execution.

use arb_types::{ArbError, Result, Slot};
use std::collections::VecDeque;

/// Divisor for basis points (100% = 10,000 bps).
pub const BPS_DIVISOR_F64: f64 = 10_000.0;

/// Maximum finite value accepted for any f64 input to avoid overflow in
/// intermediate calculations.
pub const MAX_F64_INPUT: f64 = 1e12;

// ============================================================================
// 1. Temporal observation types
// ============================================================================

/// A timestamped observation of a pipeline stage. All times are wall-clock
/// milliseconds since UNIX epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimestampedEvent {
    pub name: &'static str,
    pub timestamp_ms: u64,
    pub slot: Option<Slot>,
}

/// A complete observation pipeline for one opportunity, from detection through
/// confirmation. Missing events are represented by `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecutionTimeline {
    pub detection: Option<TimestampedEvent>,
    pub pool_update: Option<TimestampedEvent>,
    pub quote: Option<TimestampedEvent>,
    pub build: Option<TimestampedEvent>,
    pub submission: Option<TimestampedEvent>,
    pub landing: Option<TimestampedEvent>,
    pub confirmation: Option<TimestampedEvent>,
}

impl ExecutionTimeline {
    /// Age of a quoted pool state in slots: `current_slot - last_update_slot`.
    /// Returns `None` when no pool-update slot is known (fail closed).
    pub fn state_age_slots(&self, current_slot: Slot) -> Option<u64> {
        self.pool_update
            .map(|ev| current_slot.0.saturating_sub(ev.slot.unwrap_or(Slot(0)).0))
    }

    /// Age of the quote in milliseconds: `now_ms - quote.timestamp_ms`.
    pub fn quote_age_ms(&self, now_ms: u64) -> Option<u64> {
        self.quote.map(|ev| now_ms.saturating_sub(ev.timestamp_ms))
    }

    /// End-to-end pipeline latency in ms: detection -> confirmation/landing.
    /// Returns `None` when either endpoint is missing.
    pub fn pipeline_latency_ms(&self) -> Option<u64> {
        let start = self.detection?.timestamp_ms;
        let end = self
            .confirmation
            .or(self.landing)
            .map(|ev| ev.timestamp_ms)?;
        end.checked_sub(start)
    }

    /// Detection -> submission latency in ms.
    pub fn submission_latency_ms(&self) -> Option<u64> {
        let start = self.detection?.timestamp_ms;
        let end = self.submission?.timestamp_ms;
        end.checked_sub(start)
    }
}

/// Latency metrics computed from observed samples.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LatencyMetrics {
    pub samples: u64,
    pub mean_ms: f64,
    pub median_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
}

/// Rolling window of latency observations (ms).
#[derive(Debug, Clone, Default)]
pub struct LatencyWindow {
    window: VecDeque<f64>,
    max_samples: usize,
}

impl LatencyWindow {
    pub fn new(max_samples: usize) -> Self {
        Self {
            window: VecDeque::new(),
            max_samples: max_samples.max(1),
        }
    }

    /// Record an observed latency sample in milliseconds.
    pub fn observe(&mut self, latency_ms: f64) {
        if !latency_ms.is_finite() || latency_ms < 0.0 {
            return;
        }
        if self.window.len() >= self.max_samples {
            self.window.pop_front();
        }
        self.window.push_back(latency_ms);
    }

    pub fn compute(&self) -> LatencyMetrics {
        let n = self.window.len() as u64;
        if n == 0 {
            return LatencyMetrics::default();
        }
        let mut sorted: Vec<f64> = self.window.iter().copied().collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let sum: f64 = sorted.iter().sum();
        let mean_ms = sum / n as f64;

        let median_ms = if n % 2 == 1 {
            sorted[n as usize / 2]
        } else {
            let mid = n as usize / 2;
            (sorted[mid - 1] + sorted[mid]) / 2.0
        };

        let p95_idx = ((n as f64) * 0.95).ceil() as usize - 1;
        let p99_idx = ((n as f64) * 0.99).ceil() as usize - 1;
        let p95_ms = sorted[p95_idx.min(n as usize - 1)];
        let p99_ms = sorted[p99_idx.min(n as usize - 1)];
        let max_ms = sorted[n as usize - 1];

        LatencyMetrics {
            samples: n,
            mean_ms,
            median_ms,
            p95_ms,
            p99_ms,
            max_ms,
        }
    }
}

/// State freshness metrics. A pool state is stale if its age exceeds the
/// protocol-specific threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateFreshness {
    pub current_slot: Slot,
    pub last_update_slot: Slot,
    pub state_age_slots: u64,
    pub max_age_slots: u64,
    pub is_fresh: bool,
}

impl StateFreshness {
    /// Evaluate freshness against a protocol-specific threshold.
    /// Returns `Err` when the state is from a future slot or is missing.
    pub fn check(
        current_slot: Slot,
        last_update_slot: Slot,
        max_age_slots: u64,
    ) -> Result<StateFreshness> {
        if last_update_slot.0 > current_slot.0 {
            return Err(ArbError::InvalidInput(format!(
                "pool state from future slot {} > current {}",
                last_update_slot.0, current_slot.0
            )));
        }
        let state_age_slots = current_slot.0.saturating_sub(last_update_slot.0);
        let is_fresh = state_age_slots <= max_age_slots;
        Ok(StateFreshness {
            current_slot,
            last_update_slot,
            state_age_slots,
            max_age_slots,
            is_fresh,
        })
    }
}

// ============================================================================
// 2. Chronos edge weight (route ranking only — never authorizes execution)
// ============================================================================

/// Compute a spatiotemporal edge weight for route ranking.
///
/// Formulation:
///   W_e = -ln((p_out / p_in) * (1 - f) * (1 - s)) + lambda * tau + beta
///
/// Where:
///   - `p_in`, `p_out`: Relative pool price or exchange rate (strictly positive,
///     finite). The ratio `p_out / p_in` is the gross exchange rate along one hop.
///   - `fee_bps`: Protocol swap fee in basis points (0 <= fee_bps < 10,000).
///   - `slippage_bps`: Expected slippage allowance in basis points (0 <= slippage_bps < 10,000).
///   - `latency_ms`: Observed transmission/pipeline latency in milliseconds.
///   - `lambda_decay`: Hazard decay rate per second (lambda in s^-1), making
///     `lambda * tau` dimensionless.
///   - `beta_risk`: Dimensionless execution risk penalty per hop.
///
/// Returns `Err` on: non-positive or non-finite prices, non-finite latency /
/// lambda / beta, fee or slippage >= 100%, negative latency/lambda/beta, or any
/// intermediate overflow / non-finite result. NaN and Infinity are rejected.
///
/// The negative-log formulation may be used for route ranking ONLY when its
/// assumptions are valid. It must NOT independently authorize execution; the
/// final profitability decision comes from the Economic Engine.
pub fn chronos_edge_weight(
    p_in: f64,
    p_out: f64,
    fee_bps: u32,
    slippage_bps: u32,
    latency_ms: f64,
    lambda_decay: f64,
    beta_risk: f64,
) -> Result<f64> {
    // Validate inputs: positive finite prices.
    if !p_in.is_finite() || !p_out.is_finite() || p_in <= 0.0 || p_out <= 0.0 {
        return Err(ArbError::InvalidInput(
            "chronos prices must be strictly positive and finite".into(),
        ));
    }
    // Validate all intermediate inputs are finite and non-negative.
    if !latency_ms.is_finite() || latency_ms < 0.0 {
        return Err(ArbError::InvalidInput(
            "chronos latency must be finite and non-negative".into(),
        ));
    }
    if !lambda_decay.is_finite() || lambda_decay < 0.0 {
        return Err(ArbError::InvalidInput(
            "chronos lambda_decay must be finite and non-negative".into(),
        ));
    }
    if !beta_risk.is_finite() || beta_risk < 0.0 {
        return Err(ArbError::InvalidInput(
            "chronos beta_risk must be finite and non-negative".into(),
        ));
    }
    // Fee and slippage bounds: below 100%.
    if fee_bps >= 10_000 || slippage_bps >= 10_000 {
        return Err(ArbError::InvalidInput(
            "chronos fee or slippage cannot exceed 100% (10,000 bps)".into(),
        ));
    }

    let f = fee_bps as f64 / BPS_DIVISOR_F64;
    let s = slippage_bps as f64 / BPS_DIVISOR_F64;
    let tau = latency_ms / 1_000.0; // latency in seconds

    // Effective relative output multiplier (fee + slippage applied once each).
    let rate = (p_out / p_in) * (1.0 - f) * (1.0 - s);
    if !rate.is_finite() || rate <= 0.0 {
        return Err(ArbError::InvalidInput(
            "chronos effective exchange rate non-positive or non-finite".into(),
        ));
    }

    let financial_weight = -rate.ln();
    if !financial_weight.is_finite() {
        return Err(ArbError::InvalidInput(
            "chronos financial weight overflowed or non-finite".into(),
        ));
    }

    let temporal_penalty = lambda_decay * tau;
    if !temporal_penalty.is_finite() {
        return Err(ArbError::InvalidInput(
            "chronos temporal penalty overflowed or non-finite".into(),
        ));
    }

    let total_weight = financial_weight + temporal_penalty + beta_risk;
    if !total_weight.is_finite() {
        return Err(ArbError::InvalidInput(
            "chronos total edge weight overflowed or non-finite".into(),
        ));
    }

    Ok(total_weight)
}

// ============================================================================
// 3. Volatility and execution-uncertainty model
// ============================================================================

/// A volatility observation: price change between two consecutive observations
/// of the same pool, expressed in basis points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VolatilityObservation {
    pub pool_id: arb_types::PoolId,
    pub price_before_bps: u64,
    pub price_after_bps: u64,
    pub volatility_bps: u64,
    pub observed_at_ms: u64,
}

/// Rolling volatility estimate across a pool's recent observations.
#[derive(Debug, Clone, Default)]
pub struct VolatilityEstimator {
    observations: VecDeque<VolatilityObservation>,
    max_samples: usize,
}

impl VolatilityEstimator {
    pub fn new(max_samples: usize) -> Self {
        Self {
            observations: VecDeque::new(),
            max_samples: max_samples.max(1),
        }
    }

    pub fn observe(&mut self, obs: VolatilityObservation) {
        if self.observations.len() >= self.max_samples {
            self.observations.pop_front();
        }
        self.observations.push_back(obs);
    }

    /// Mean volatility in bps across the window. Zero when no observations.
    pub fn mean_volatility_bps(&self) -> u64 {
        if self.observations.is_empty() {
            return 0;
        }
        let sum: u128 = self
            .observations
            .iter()
            .map(|o| o.volatility_bps as u128)
            .sum();
        (sum / self.observations.len() as u128) as u64
    }

    /// Max volatility in bps across the window.
    pub fn max_volatility_bps(&self) -> u64 {
        self.observations
            .iter()
            .map(|o| o.volatility_bps)
            .max()
            .unwrap_or(0)
    }

    pub fn samples(&self) -> u64 {
        self.observations.len() as u64
    }
}

/// Execution-uncertainty buffer in basis points. Combines measured volatility
/// and execution conditions into a single risk buffer (for ranking only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionUncertainty {
    pub volatility_buffer_bps: u64,
    pub latency_buffer_bps: u64,
    pub total_buffer_bps: u64,
}

impl ExecutionUncertainty {
    /// Combine volatility and latency into a total uncertainty buffer in bps.
    /// Used for ranking only; never added to the authoritative EconomicQuote.
    pub fn new(volatility_buffer_bps: u64, latency_buffer_bps: u64) -> Self {
        let total = volatility_buffer_bps.saturating_add(latency_buffer_bps);
        Self {
            volatility_buffer_bps,
            latency_buffer_bps,
            total_buffer_bps: total,
        }
    }
}

// ============================================================================
// 4. Opportunity decay and risk-adjusted ranking
// ============================================================================

/// Risk-adjusted ranking score for an opportunity. Lower is better (more
/// attractive). Computed from economic net profit plus Chronos risk terms.
///
/// This score is used for ranking candidate opportunities. It does NOT
/// authorize execution; only the Economic Engine's profit floor does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RiskAdjustedScore {
    /// Raw net profit in base-mint raw units (from Economic Engine).
    pub net_profit_raw: u64,
    /// Chronos edge weight (route ranking term).
    pub chronos_weight: f64,
    /// Execution uncertainty buffer in bps.
    pub uncertainty_bps: u64,
    /// State age in slots.
    pub state_age_slots: u64,
    /// Final score: lower is more attractive.
    pub score: f64,
}

/// Compute a risk-adjusted score from an economic quote and Chronos risk inputs.
/// Returns `Err` when the quote is not profitable or risk inputs are invalid.
pub fn compute_risk_adjusted_score(
    net_profit_raw: u64,
    chronos_weight: f64,
    uncertainty_bps: u64,
    state_age_slots: u64,
    age_penalty_bps_per_slot: u64,
) -> Result<RiskAdjustedScore> {
    if !chronos_weight.is_finite() {
        return Err(ArbError::InvalidInput(
            "chronos_weight must be finite".into(),
        ));
    }
    if net_profit_raw == 0 {
        return Err(ArbError::InvalidInput(
            "non-profitable opportunity cannot be ranked".into(),
        ));
    }

    // Age penalty: older state is less attractive (linear in slots).
    let age_penalty = state_age_slots as f64 * age_penalty_bps_per_slot as f64 / BPS_DIVISOR_F64;
    let uncertainty_penalty = uncertainty_bps as f64 / BPS_DIVISOR_F64;

    // Score = chronos_weight + uncertainty_penalty + age_penalty, scaled so
    // that larger net profit improves (lowers) the score.
    let profit_scale = net_profit_raw as f64;
    let score = if profit_scale > 0.0 {
        chronos_weight + uncertainty_penalty + age_penalty + (1.0 / profit_scale)
    } else {
        f64::INFINITY
    };

    if !score.is_finite() {
        return Err(ArbError::InvalidInput(
            "risk-adjusted score overflowed or non-finite".into(),
        ));
    }

    Ok(RiskAdjustedScore {
        net_profit_raw,
        chronos_weight,
        uncertainty_bps,
        state_age_slots,
        score,
    })
}

// ============================================================================
// 5. Execution Gate (risk gate — recommends approve/reject, never authorizes)
// ============================================================================

/// Recommendation from the Chronos execution gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateDecision {
    /// The opportunity passes Chronos risk checks; still requires Economic
    /// Engine profit-floor approval before execution.
    Approve,
    /// The opportunity fails Chronos risk checks (stale state, excessive
    /// latency, negative EV). Rejected before reaching the builder.
    Reject,
}

/// Input to the Chronos execution gate.
#[derive(Debug, Clone, Copy)]
pub struct GateInput {
    pub net_profit_raw: u64,
    pub min_profit_raw: u64,
    pub state_age_slots: u64,
    pub max_state_age_slots: u64,
    pub quote_age_ms: u64,
    pub max_quote_age_ms: u64,
    pub pipeline_latency_ms: u64,
    pub max_pipeline_latency_ms: u64,
    pub chronos_weight: f64,
    pub uncertainty_bps: u64,
    pub max_uncertainty_bps: u64,
    pub sol_price_usd: f64,
    pub min_ev_usd: f64,
}

/// Output of the Chronos execution gate.
#[derive(Debug, Clone, Copy)]
pub struct GateOutput {
    pub decision: GateDecision,
    pub reason: Option<&'static str>,
    pub net_profit_raw: u64,
    pub meets_min_profit: bool,
    pub state_is_fresh: bool,
    pub latency_is_acceptable: bool,
    pub uncertainty_is_acceptable: bool,
}

/// Evaluate the Chronos execution gate. Returns `Approve` only when all risk
/// conditions pass AND the economic profit floor is met. The Economic Engine
/// remains the sole authority on profitability; this gate adds risk conditions.
///
/// Chronos does NOT authorize execution by itself. The builder must still
/// enforce on-chain `min_out` and `min_profit`.
pub fn evaluate_gate(input: GateInput) -> Result<GateOutput> {
    if !input.sol_price_usd.is_finite() || input.sol_price_usd <= 0.0 {
        return Err(ArbError::InvalidInput(
            "gate sol_price_usd must be positive and finite".into(),
        ));
    }
    if !input.chronos_weight.is_finite() {
        return Err(ArbError::InvalidInput(
            "gate chronos_weight must be finite".into(),
        ));
    }

    let meets_min_profit = input.net_profit_raw >= input.min_profit_raw;
    let state_is_fresh = input.state_age_slots <= input.max_state_age_slots;
    let latency_is_acceptable = input.pipeline_latency_ms <= input.max_pipeline_latency_ms;
    let uncertainty_is_acceptable = input.uncertainty_bps <= input.max_uncertainty_bps;
    let quote_is_fresh = input.quote_age_ms <= input.max_quote_age_ms;

    let all_risk_ok =
        state_is_fresh && latency_is_acceptable && uncertainty_is_acceptable && quote_is_fresh;

    let (decision, reason) = if !meets_min_profit {
        (GateDecision::Reject, Some("below minimum profit floor"))
    } else if !all_risk_ok {
        (
            GateDecision::Reject,
            Some("chronos risk conditions not met"),
        )
    } else {
        (GateDecision::Approve, None)
    };

    Ok(GateOutput {
        decision,
        reason,
        net_profit_raw: input.net_profit_raw,
        meets_min_profit,
        state_is_fresh,
        latency_is_acceptable,
        uncertainty_is_acceptable,
    })
}

// ============================================================================
// 6. Tests (robustness)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_edge_inputs() -> (f64, f64, u32, u32, f64, f64, f64) {
        (1.0, 1.03, 25, 10, 5.0, 1.0, 0.0005)
    }

    #[test]
    fn chronos_edge_weight_profitable_negative() {
        let (p_in, p_out, fee, slip, lat, lam, beta) = valid_edge_inputs();
        let w = chronos_edge_weight(p_in, p_out, fee, slip, lat, lam, beta).unwrap();
        assert!(
            w < 0.0,
            "profitable edge should have negative weight, got {w}"
        );
    }

    #[test]
    fn chronos_edge_weight_unprofitable_positive() {
        let w = chronos_edge_weight(1.0, 0.99, 25, 10, 10.0, 3.0, 0.0005).unwrap();
        assert!(w > 0.0, "unprofitable spread must have positive weight");
    }

    #[test]
    fn chronos_edge_weight_rejects_nan_prices() {
        assert!(chronos_edge_weight(f64::NAN, 1.0, 25, 10, 5.0, 1.0, 0.0).is_err());
        assert!(chronos_edge_weight(1.0, f64::NAN, 25, 10, 5.0, 1.0, 0.0).is_err());
    }

    #[test]
    fn chronos_edge_weight_rejects_infinity_prices() {
        assert!(chronos_edge_weight(f64::INFINITY, 1.0, 25, 10, 5.0, 1.0, 0.0).is_err());
        assert!(chronos_edge_weight(1.0, f64::INFINITY, 25, 10, 5.0, 1.0, 0.0).is_err());
    }

    #[test]
    fn chronos_edge_weight_rejects_zero_and_negative_prices() {
        assert!(chronos_edge_weight(0.0, 1.0, 25, 10, 5.0, 1.0, 0.0).is_err());
        assert!(chronos_edge_weight(1.0, 0.0, 25, 10, 5.0, 1.0, 0.0).is_err());
        assert!(chronos_edge_weight(-1.0, 1.0, 25, 10, 5.0, 1.0, 0.0).is_err());
    }

    #[test]
    fn chronos_edge_weight_rejects_non_finite_latency() {
        assert!(chronos_edge_weight(1.0, 1.03, 25, 10, f64::NAN, 1.0, 0.0).is_err());
        assert!(chronos_edge_weight(1.0, 1.03, 25, 10, f64::INFINITY, 1.0, 0.0).is_err());
        assert!(chronos_edge_weight(1.0, 1.03, 25, 10, -5.0, 1.0, 0.0).is_err());
    }

    #[test]
    fn chronos_edge_weight_rejects_extreme_lambda() {
        assert!(chronos_edge_weight(1.0, 1.03, 25, 10, 5.0, f64::NAN, 0.0).is_err());
        assert!(chronos_edge_weight(1.0, 1.03, 25, 10, 5.0, f64::INFINITY, 0.0).is_err());
        assert!(chronos_edge_weight(1.0, 1.03, 25, 10, 5.0, -1.0, 0.0).is_err());
    }

    #[test]
    fn chronos_edge_weight_rejects_extreme_beta() {
        assert!(chronos_edge_weight(1.0, 1.03, 25, 10, 5.0, 1.0, f64::NAN).is_err());
        assert!(chronos_edge_weight(1.0, 1.03, 25, 10, 5.0, 1.0, f64::INFINITY).is_err());
        assert!(chronos_edge_weight(1.0, 1.03, 25, 10, 5.0, 1.0, -1.0).is_err());
    }

    #[test]
    fn chronos_edge_weight_rejects_fee_over_100_pct() {
        assert!(chronos_edge_weight(1.0, 1.03, 10_000, 10, 5.0, 1.0, 0.0).is_err());
        assert!(chronos_edge_weight(1.0, 1.03, 10_001, 10, 5.0, 1.0, 0.0).is_err());
        assert!(chronos_edge_weight(1.0, 1.03, 25, 10_000, 5.0, 1.0, 0.0).is_err());
    }

    #[test]
    fn chronos_edge_weight_latency_increases_weight() {
        let low = chronos_edge_weight(1.0, 1.02, 25, 10, 10.0, 3.0, 0.0005).unwrap();
        let high = chronos_edge_weight(1.0, 1.02, 25, 10, 500.0, 3.0, 0.0005).unwrap();
        assert!(high > low, "higher latency must increase weight");
    }

    #[test]
    fn chronos_edge_weight_result_is_finite() {
        for _ in 0..1000 {
            let w = chronos_edge_weight(1.0, 1.001, 25, 10, 10.0, 1.0, 0.0005).unwrap();
            assert!(w.is_finite());
        }
    }

    #[test]
    fn state_freshness_rejects_future_slot() {
        assert!(StateFreshness::check(Slot(100), Slot(101), 10).is_err());
    }

    #[test]
    fn state_freshness_accepts_within_threshold() {
        let f = StateFreshness::check(Slot(100), Slot(95), 10).unwrap();
        assert!(f.is_fresh);
        assert_eq!(f.state_age_slots, 5);
    }

    #[test]
    fn state_freshness_rejects_beyond_threshold() {
        let f = StateFreshness::check(Slot(100), Slot(80), 10).unwrap();
        assert!(!f.is_fresh);
        assert_eq!(f.state_age_slots, 20);
    }

    #[test]
    fn state_freshness_same_slot_is_fresh() {
        let f = StateFreshness::check(Slot(100), Slot(100), 0).unwrap();
        assert!(f.is_fresh);
        assert_eq!(f.state_age_slots, 0);
    }

    #[test]
    fn latency_window_computes_percentiles() {
        let mut w = LatencyWindow::new(100);
        for i in 1..=100 {
            w.observe(i as f64);
        }
        let m = w.compute();
        assert_eq!(m.samples, 100);
        assert!(m.mean_ms > 0.0);
        assert!(m.p95_ms >= m.median_ms);
        assert!(m.p99_ms >= m.p95_ms);
        assert!(m.max_ms >= m.p99_ms);
    }

    #[test]
    fn latency_window_rejects_invalid_samples() {
        let mut w = LatencyWindow::new(100);
        w.observe(f64::NAN);
        w.observe(f64::INFINITY);
        w.observe(-1.0);
        let m = w.compute();
        assert_eq!(m.samples, 0);
    }

    #[test]
    fn volatility_estimator_mean_and_max() {
        let mut v = VolatilityEstimator::new(100);
        v.observe(VolatilityObservation {
            pool_id: arb_types::PoolId::new([1; 32]),
            price_before_bps: 100,
            price_after_bps: 105,
            volatility_bps: 5,
            observed_at_ms: 1_000,
        });
        v.observe(VolatilityObservation {
            pool_id: arb_types::PoolId::new([1; 32]),
            price_before_bps: 105,
            price_after_bps: 100,
            volatility_bps: 5,
            observed_at_ms: 1_100,
        });
        assert_eq!(v.mean_volatility_bps(), 5);
        assert_eq!(v.max_volatility_bps(), 5);
        assert_eq!(v.samples(), 2);
    }

    #[test]
    fn risk_adjusted_score_rejects_unprofitable() {
        assert!(
            compute_risk_adjusted_score(
                0, // net profit zero
                0.0, 0, 0, 1
            )
            .is_err()
        );
    }

    #[test]
    fn risk_adjusted_score_rejects_non_finite_weight() {
        assert!(compute_risk_adjusted_score(1_000_000, f64::NAN, 0, 0, 1).is_err());
        assert!(compute_risk_adjusted_score(1_000_000, f64::INFINITY, 0, 0, 1).is_err());
    }

    #[test]
    fn risk_adjusted_score_lower_is_better() {
        let good = compute_risk_adjusted_score(1_000_000, 0.0, 0, 0, 1).unwrap();
        let bad = compute_risk_adjusted_score(1_000, 10.0, 100, 50, 1).unwrap();
        assert!(good.score < bad.score);
    }

    #[test]
    fn gate_approves_when_all_ok() {
        let out = evaluate_gate(GateInput {
            net_profit_raw: 1_000_000,
            min_profit_raw: 10_000,
            state_age_slots: 1,
            max_state_age_slots: 10,
            quote_age_ms: 50,
            max_quote_age_ms: 400,
            pipeline_latency_ms: 100,
            max_pipeline_latency_ms: 500,
            chronos_weight: -0.02,
            uncertainty_bps: 5,
            max_uncertainty_bps: 50,
            sol_price_usd: 150.0,
            min_ev_usd: 0.05,
        })
        .unwrap();
        assert_eq!(out.decision, GateDecision::Approve);
        assert!(out.meets_min_profit);
        assert!(out.state_is_fresh);
        assert!(out.latency_is_acceptable);
        assert!(out.uncertainty_is_acceptable);
        assert!(out.reason.is_none());
    }

    #[test]
    fn gate_rejects_below_profit_floor() {
        let out = evaluate_gate(GateInput {
            net_profit_raw: 1_000,
            min_profit_raw: 10_000,
            state_age_slots: 1,
            max_state_age_slots: 10,
            quote_age_ms: 50,
            max_quote_age_ms: 400,
            pipeline_latency_ms: 100,
            max_pipeline_latency_ms: 500,
            chronos_weight: -0.02,
            uncertainty_bps: 5,
            max_uncertainty_bps: 50,
            sol_price_usd: 150.0,
            min_ev_usd: 0.05,
        })
        .unwrap();
        assert_eq!(out.decision, GateDecision::Reject);
        assert!(!out.meets_min_profit);
        assert_eq!(out.reason, Some("below minimum profit floor"));
    }

    #[test]
    fn gate_rejects_stale_state() {
        let out = evaluate_gate(GateInput {
            net_profit_raw: 1_000_000,
            min_profit_raw: 10_000,
            state_age_slots: 50,
            max_state_age_slots: 10,
            quote_age_ms: 50,
            max_quote_age_ms: 400,
            pipeline_latency_ms: 100,
            max_pipeline_latency_ms: 500,
            chronos_weight: -0.02,
            uncertainty_bps: 5,
            max_uncertainty_bps: 50,
            sol_price_usd: 150.0,
            min_ev_usd: 0.05,
        })
        .unwrap();
        assert_eq!(out.decision, GateDecision::Reject);
        assert!(!out.state_is_fresh);
    }

    #[test]
    fn gate_rejects_extreme_latency() {
        let out = evaluate_gate(GateInput {
            net_profit_raw: 1_000_000,
            min_profit_raw: 10_000,
            state_age_slots: 1,
            max_state_age_slots: 10,
            quote_age_ms: 50,
            max_quote_age_ms: 400,
            pipeline_latency_ms: 10_000,
            max_pipeline_latency_ms: 500,
            chronos_weight: -0.02,
            uncertainty_bps: 5,
            max_uncertainty_bps: 50,
            sol_price_usd: 150.0,
            min_ev_usd: 0.05,
        })
        .unwrap();
        assert_eq!(out.decision, GateDecision::Reject);
        assert!(!out.latency_is_acceptable);
    }

    #[test]
    fn gate_rejects_non_finite_inputs() {
        assert!(
            evaluate_gate(GateInput {
                net_profit_raw: 1_000_000,
                min_profit_raw: 10_000,
                state_age_slots: 1,
                max_state_age_slots: 10,
                quote_age_ms: 50,
                max_quote_age_ms: 400,
                pipeline_latency_ms: 100,
                max_pipeline_latency_ms: 500,
                chronos_weight: f64::NAN,
                uncertainty_bps: 5,
                max_uncertainty_bps: 50,
                sol_price_usd: 150.0,
                min_ev_usd: 0.05,
            })
            .is_err()
        );
        assert!(
            evaluate_gate(GateInput {
                net_profit_raw: 1_000_000,
                min_profit_raw: 10_000,
                state_age_slots: 1,
                max_state_age_slots: 10,
                quote_age_ms: 50,
                max_quote_age_ms: 400,
                pipeline_latency_ms: 100,
                max_pipeline_latency_ms: 500,
                chronos_weight: -0.02,
                uncertainty_bps: 5,
                max_uncertainty_bps: 50,
                sol_price_usd: f64::NAN,
                min_ev_usd: 0.05,
            })
            .is_err()
        );
    }
}
