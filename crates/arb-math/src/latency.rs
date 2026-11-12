//! Round-trip-time tracking and slot-window fire-time planning.
//!
//! A bundle is only useful if it reaches a block engine before the leader
//! closes the slot. Both quantities are unknown a priori: the wire latency to
//! each block engine drifts, and the build/sign path has a fixed cost. This
//! module estimates the first from observed samples and turns it into an
//! absolute fire time for the second.
//!
//! Latency is modelled as log-normal, which is the right shape for a positive,
//! right-skewed quantity: the mean of `ln(rtt)` and its variance are tracked as
//! exponentially weighted moving averages, and a quantile is recovered with
//! `exp(mu + z * sigma)`.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Z-score for a one-sided 95% confidence bound (`Phi^-1(0.95)`).
pub const Z_95: f64 = 1.644_853_626_951_472_2;

/// Nominal slot duration on Solana mainnet, in milliseconds.
pub const SLOT_MS: f64 = 400.0;

/// Hard cap on the pre-fire safety margin: one slot.
///
/// Firing earlier than a slot ahead of the window means acting on state that
/// will be stale by the time the bundle is evaluated, so the margin is bounded.
pub const MAX_MARGIN_MS: u64 = 400;

/// Margin used before any round trip has been observed: half a slot.
pub const COLD_MARGIN_MS: u64 = 200;

/// Exponentially weighted tracker over observed round-trip times.
///
/// The blend of a time-based and a sample-based weight means the tracker both
/// adapts to genuine drift (time weight) and converges quickly from a cold start
/// (burst weight), without a separate warm-up phase.
#[derive(Debug, Clone)]
pub struct RttTracker {
    halflife_s: f64,
    mean_log: f64,
    var_log: f64,
    mean_ms: f64,
    mean_sq_ms: f64,
    samples: u64,
    last_ms: Option<f64>,
}

impl RttTracker {
    /// `halflife_s` is the drift half-life: after this many seconds without
    /// samples, half of the historical weight is replaced by new observations.
    pub fn new(halflife_s: f64) -> Self {
        Self {
            halflife_s: if halflife_s.is_finite() && halflife_s > 0.0 {
                halflife_s
            } else {
                1.0
            },
            mean_log: 0.0,
            var_log: 0.0,
            mean_ms: 0.0,
            mean_sq_ms: 0.0,
            samples: 0,
            last_ms: None,
        }
    }

    /// Fold one round-trip sample into the estimate.
    ///
    /// `dt_s` is the wall-clock gap since the previous sample. Non-finite or
    /// non-positive samples are rejected rather than poisoning the estimate.
    /// Returns `true` when the sample was accepted.
    pub fn observe(&mut self, rtt_ms: f64, dt_s: f64) -> bool {
        if !rtt_ms.is_finite() || rtt_ms <= 0.0 {
            return false;
        }
        // Outlier guard: a single extreme sample must not dominate the
        // log-normal estimate. Winsorise the observation at 3x the current p95
        // before folding it in, so the estimator stays robust to one-off spikes
        // while still following genuine sustained drift.
        let rtt_ms = if self.samples > 0 {
            rtt_ms.min(self.p95_ms() * 3.0).max(1.0)
        } else {
            rtt_ms
        };
        let log = rtt_ms.ln();
        if self.samples == 0 {
            self.mean_log = log;
            self.var_log = 0.0;
            self.mean_ms = rtt_ms;
            self.mean_sq_ms = rtt_ms * rtt_ms;
        } else {
            let burst = 1.0 / (self.samples.saturating_add(1) as f64);
            let time_weight = if dt_s.is_finite() && dt_s > 0.0 {
                1.0 - (-dt_s / self.halflife_s).exp()
            } else {
                0.0
            };
            let alpha = time_weight.max(burst).clamp(0.0, 1.0);

            let dev_log = log - self.mean_log;
            self.mean_log += alpha * dev_log;
            self.var_log += alpha * (dev_log * dev_log - self.var_log);

            let dev_ms = rtt_ms - self.mean_ms;
            self.mean_ms += alpha * dev_ms;
            self.mean_sq_ms += alpha * (rtt_ms * rtt_ms - self.mean_sq_ms);
        }
        self.samples = self.samples.saturating_add(1);
        self.last_ms = Some(rtt_ms);
        true
    }

    /// Number of accepted samples.
    pub fn samples(&self) -> u64 {
        self.samples
    }

    /// Most recent accepted sample, if any.
    pub fn last_ms(&self) -> Option<f64> {
        self.last_ms
    }

    /// Standard deviation of `ln(rtt)`.
    pub fn sigma_log(&self) -> f64 {
        self.var_log.max(0.0).sqrt()
    }

    /// Median round-trip time in milliseconds.
    pub fn p50_ms(&self) -> f64 {
        self.mean_log.exp()
    }

    /// 95th percentile round-trip time in milliseconds.
    pub fn p95_ms(&self) -> f64 {
        (self.mean_log + Z_95 * self.sigma_log()).exp()
    }

    /// Standard deviation of the raw round-trip time, in milliseconds.
    ///
    /// Tracked separately from `sigma_log` because the two live in different
    /// units: `sigma_log` shapes the log-normal quantile, while this one sizes
    /// the additive safety margin.
    pub fn sigma_ms(&self) -> f64 {
        let variance = self.mean_sq_ms - self.mean_ms * self.mean_ms;
        if variance.is_finite() && variance > 0.0 {
            variance.sqrt()
        } else {
            0.0
        }
    }

    /// Lead time to subtract from a slot-window start: `p95 + 3σ`, in
    /// milliseconds, clamped to one slot.
    ///
    /// Zero before the first sample. An unmeasured lead is not a lead, so a
    /// caller must treat zero as "no measurement" rather than "no margin".
    pub fn lead_ms(&self) -> f64 {
        if self.samples == 0 {
            return 0.0;
        }
        (self.p95_ms() + 3.0 * self.sigma_ms()).clamp(1.0, MAX_MARGIN_MS as f64)
    }
}

impl Default for RttTracker {
    fn default() -> Self {
        Self::new(1.0)
    }
}

/// Absolute time to fire a bundle for one slot window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FirePlan {
    /// Wall-clock milliseconds at which the bundle must be submitted.
    pub fire_at_ms: u64,
    /// Total lead time subtracted from the window start.
    pub margin_ms: u64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub sigma_ms: f64,
    pub samples: u64,
}

/// Plan when to fire a bundle for a slot whose window opens at `window_start_ms`.
///
/// `build_budget_ms` is the measured cost of assembling and signing the bundle;
/// it is subtracted on top of the latency margin because both costs are paid
/// serially before the bundle reaches the wire.
pub fn plan_fire(window_start_ms: u64, build_budget_ms: u64, tracker: &RttTracker) -> FirePlan {
    let p50_ms = tracker.p50_ms();
    let p95_ms = tracker.p95_ms();
    let sigma_ms = tracker.sigma_ms();

    let margin_ms = if tracker.samples() == 0 {
        COLD_MARGIN_MS
    } else {
        ms_from_f64(tracker.lead_ms())
    };

    FirePlan {
        fire_at_ms: window_start_ms.saturating_sub(margin_ms.saturating_add(build_budget_ms)),
        margin_ms,
        p50_ms,
        p95_ms,
        sigma_ms,
        samples: tracker.samples(),
    }
}

/// Maximum number of `(slot, time)` pairs kept by [`SlotClock`].
pub const MAX_SLOT_PAIRS: usize = 128;

/// One predicted fire time, in the clock's own monotonic frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SlotFire {
    /// Monotonic nanoseconds at which the bundle must be on the wire.
    pub fire_at_ns: u64,
    /// Predicted start of the slot window.
    pub window_start_ns: u64,
    /// Lead time subtracted from the window start.
    pub lead_ms: u64,
    /// RMS residual of the slot-to-time fit, in nanoseconds.
    pub fit_rms_ns: u64,
    /// Pairs behind the fit.
    pub fit_samples: usize,
}

/// Maps slot numbers onto the local monotonic clock.
///
/// Slot boundaries are published by the network, but the local clock has its
/// own drift, so the mapping is a least-squares fit over observed
/// `(slot, Instant)` pairs rather than an assumed 400 ms cadence. The residual
/// is reported with every schedule: it is the honest uncertainty on the window
/// start, and a caller can decide not to trust a bad fit.
#[derive(Debug, Clone)]
pub struct SlotClock {
    base: Instant,
    pairs: VecDeque<(u64, u64)>,
}

impl SlotClock {
    /// A clock with no observations, anchored at the current instant.
    pub fn new() -> Self {
        Self {
            base: Instant::now(),
            pairs: VecDeque::with_capacity(MAX_SLOT_PAIRS),
        }
    }

    /// Record the local time at which `slot` was observed.
    pub fn observe_slot(&mut self, slot: u64) {
        self.observe_slot_at(slot, Instant::now());
    }

    /// [`Self::observe_slot`] with an explicit reading, for tests and for a
    /// caller that observed the slot before reaching the clock.
    pub fn observe_slot_at(&mut self, slot: u64, now: Instant) {
        if self.pairs.len() == MAX_SLOT_PAIRS {
            self.pairs.pop_front();
        }
        self.pairs.push_back((slot, nanos_since(now, self.base)));
    }

    /// Current reading of the clock's monotonic frame.
    pub fn now_ns(&self) -> u64 {
        nanos_since(Instant::now(), self.base)
    }

    /// Number of pairs behind the fit.
    pub fn samples(&self) -> usize {
        self.pairs.len()
    }

    /// Predicted monotonic start of `slot`, or `None` before the fit exists.
    ///
    /// Three distinct pairs are required: two points fit any line exactly and
    /// report zero error while saying nothing about the drift. The estimate is
    /// anchored on the earliest observation, not on the mean, so it is not
    /// biased late by the polling lag — see [`SlotClock::fit`].
    pub fn predict_window_start_ns(&self, slot: u64) -> Option<u64> {
        let (intercept, slope) = self.fit()?;
        let start = intercept + slope * slot as f64;
        if !start.is_finite() || start < 0.0 {
            return None;
        }
        if start >= u64::MAX as f64 {
            return Some(u64::MAX);
        }
        Some(start.round() as u64)
    }

    /// RMS residual of the fit, in nanoseconds.
    pub fn fit_error_ns(&self) -> Option<u64> {
        let (intercept, slope) = self.fit()?;
        let n = self.pairs.len() as f64;
        let sum: f64 = self
            .pairs
            .iter()
            .map(|(slot, ns)| {
                let predicted = intercept + slope * *slot as f64;
                let residual = *ns as f64 - predicted;
                residual * residual
            })
            .sum();
        Some(ns_from_f64((sum / n).sqrt()))
    }

    /// Absolute fire time for `slot`, or `None` when it cannot be derived.
    ///
    /// Refuses while either half of the estimate is missing: no fit means no
    /// window start, and no round-trip sample means no lead to subtract. A
    /// deadline invented from an uncalibrated clock is worse than sending
    /// immediately.
    pub fn schedule(
        &self,
        slot: u64,
        tracker: &RttTracker,
        build_budget_ms: u64,
    ) -> Option<SlotFire> {
        let window_start_ns = self.predict_window_start_ns(slot)?;
        let lead_ms = tracker.lead_ms();
        if !lead_ms.is_finite() || lead_ms <= 0.0 {
            return None;
        }
        let lead_ns = ms_from_f64(lead_ms).saturating_mul(1_000_000);
        let build_ns = build_budget_ms.saturating_mul(1_000_000);
        Some(SlotFire {
            fire_at_ns: window_start_ns
                .saturating_sub(lead_ns)
                .saturating_sub(build_ns),
            window_start_ns,
            lead_ms: ms_from_f64(lead_ms),
            fit_rms_ns: self.fit_error_ns().unwrap_or(0),
            fit_samples: self.pairs.len(),
        })
    }

    /// Time remaining until `fire_at_ns`; zero once the deadline has passed.
    pub fn wait_duration(&self, fire_at_ns: u64) -> Duration {
        Duration::from_nanos(fire_at_ns.saturating_sub(self.now_ns()))
    }

    /// Least-squares fit of `(slot -> monotonic ns)`, anchored on the earliest
    /// observation.
    ///
    /// `None` while the pairs are fewer than three, all on one slot, or imply a
    /// non-advancing clock — a negative slope is not a clock.
    ///
    /// A slot is never observed at the instant it begins, only after it, so the
    /// plain least-squares intercept inherits the *mean* polling lag (about
    /// half a slot for a 300 ms poll against 400 ms slots) and every predicted
    /// boundary lands that much late. Shifting the line down by the smallest
    /// residual re-anchors it on the closest observation to a real boundary.
    ///
    /// this assumes the polling phase spreads over the window; a
    /// feed that timestamps the boundary itself (geyser `slotsUpdates`) would
    /// make both the assumption and this correction unnecessary. The shift is
    /// clamped to one slot and may only move the deadline *earlier*, so a
    /// corrupt sample can cost a slot of earliness, never a late bundle.
    fn fit(&self) -> Option<(f64, f64)> {
        let n = self.pairs.len();
        if n < 3 {
            return None;
        }
        let count = n as f64;
        let mean_x = self.pairs.iter().map(|(slot, _)| *slot as f64).sum::<f64>() / count;
        let mean_y = self.pairs.iter().map(|(_, ns)| *ns as f64).sum::<f64>() / count;

        let mut denominator = 0.0;
        let mut numerator = 0.0;
        for (slot, ns) in &self.pairs {
            let dx = *slot as f64 - mean_x;
            denominator += dx * dx;
            numerator += dx * (*ns as f64 - mean_y);
        }
        if denominator <= 0.0 {
            return None;
        }
        let slope = numerator / denominator;
        if !slope.is_finite() || slope <= 0.0 {
            return None;
        }
        let intercept = mean_y - slope * mean_x;
        let earliest = self
            .pairs
            .iter()
            .map(|(slot, ns)| *ns as f64 - (intercept + slope * *slot as f64))
            .fold(f64::INFINITY, f64::min);
        let shift = earliest.clamp(-(SLOT_MS * 1_000_000.0), 0.0);
        Some((intercept + shift, slope))
    }
}

impl Default for SlotClock {
    fn default() -> Self {
        Self::new()
    }
}

/// Nanoseconds between `now` and `base`, saturating at the clock's bounds.
fn nanos_since(now: Instant, base: Instant) -> u64 {
    let nanos = now.saturating_duration_since(base).as_nanos();
    u64::try_from(nanos).unwrap_or(u64::MAX)
}

/// Convert a non-negative float duration in nanoseconds to whole nanoseconds.
fn ns_from_f64(value: f64) -> u64 {
    if !value.is_finite() || value <= 0.0 {
        return 0;
    }
    let limit = u64::MAX as f64;
    if value >= limit {
        return u64::MAX;
    }
    // Safe: `value` is finite, positive and below `u64::MAX`.
    value.round() as u64
}

/// Convert a non-negative float duration to whole milliseconds, saturating.
fn ms_from_f64(value: f64) -> u64 {
    if !value.is_finite() || value <= 0.0 {
        return 0;
    }
    let limit = u64::MAX as f64;
    if value >= limit {
        return u64::MAX;
    }
    // Safe: `value` is finite, positive and below `u64::MAX`.
    value.round() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn warm(samples: u64) -> RttTracker {
        let mut tracker = RttTracker::new(1.0);
        for _ in 0..samples {
            assert!(tracker.observe(120.0, 0.25));
        }
        tracker
    }

    #[test]
    fn rejects_unusable_samples() {
        let mut tracker = RttTracker::new(1.0);
        assert!(!tracker.observe(0.0, 0.1));
        assert!(!tracker.observe(-5.0, 0.1));
        assert!(!tracker.observe(f64::NAN, 0.1));
        assert!(!tracker.observe(f64::INFINITY, 0.1));
        assert_eq!(tracker.samples(), 0);
    }

    #[test]
    fn first_sample_sets_expectation_exactly() {
        let mut tracker = RttTracker::new(1.0);
        assert!(tracker.observe(150.0, 0.0));
        assert_eq!(tracker.samples(), 1);
        assert!((tracker.p50_ms() - 150.0).abs() < 1e-9);
        assert!(tracker.sigma_log().abs() < 1e-9);
        assert!(tracker.sigma_ms().abs() < 1e-9);
    }

    #[test]
    fn single_outlier_does_not_dominate_p95() {
        let mut tracker = warm(20);
        assert!(tracker.observe(9_000.0, 0.25));
        assert!(
            tracker.p95_ms() < 1_500.0,
            "p95 inflated by one outlier: {}",
            tracker.p95_ms()
        );
    }

    #[test]
    fn sustained_drift_is_followed() {
        let mut tracker = warm(10);
        let before = tracker.p50_ms();
        for _ in 0..60 {
            assert!(tracker.observe(400.0, 2.0));
        }
        assert!(
            tracker.p50_ms() > before * 2.0,
            "tracker failed to follow drift: {} -> {}",
            before,
            tracker.p50_ms()
        );
    }

    #[test]
    fn cold_start_uses_half_slot_margin() {
        let tracker = RttTracker::new(1.0);
        let plan = plan_fire(10_000, 30, &tracker);
        assert_eq!(plan.margin_ms, COLD_MARGIN_MS);
        assert_eq!(plan.fire_at_ms, 10_000 - COLD_MARGIN_MS - 30);
    }

    #[test]
    fn margin_is_capped_at_one_slot() {
        let mut tracker = RttTracker::new(1.0);
        for _ in 0..40 {
            assert!(tracker.observe(3_000.0, 1.0));
        }
        let plan = plan_fire(1_000_000, 0, &tracker);
        assert_eq!(plan.margin_ms, MAX_MARGIN_MS);
    }

    #[test]
    fn fire_time_never_underflows() {
        let tracker = warm(5);
        let plan = plan_fire(0, 10_000, &tracker);
        assert_eq!(plan.fire_at_ms, 0);
    }

    fn slot_clock_at(base: Instant) -> SlotClock {
        SlotClock {
            base,
            pairs: VecDeque::new(),
        }
    }

    /// A perfectly regular 400 ms train, observed 20 times.
    fn regular_clock(t0: Instant) -> SlotClock {
        let mut clock = slot_clock_at(t0);
        for i in 0..20u64 {
            clock.observe_slot_at(
                300_000_000 + i,
                t0 + Duration::from_millis(i * SLOT_MS as u64),
            );
        }
        clock
    }

    #[test]
    fn slot_clock_refuses_before_it_can_fit() {
        let t0 = Instant::now();
        let mut clock = slot_clock_at(t0);
        clock.observe_slot_at(10, t0);
        clock.observe_slot_at(11, t0 + Duration::from_millis(400));

        assert_eq!(clock.samples(), 2);
        assert!(
            clock.predict_window_start_ns(12).is_none(),
            "two points fit any line exactly and prove nothing"
        );
        assert!(clock.schedule(12, &warm(5), 2).is_none());
    }

    #[test]
    fn slot_clock_predicts_a_regular_train_exactly() {
        let t0 = Instant::now();
        let clock = regular_clock(t0);
        let expected = t0 + Duration::from_millis(40 * SLOT_MS as u64);
        let predicted = clock
            .predict_window_start_ns(300_000_040)
            .expect("regular train must fit");

        let expected_ns = expected.duration_since(t0).as_nanos() as i128;
        let delta = (predicted as i128 - expected_ns).abs();
        assert!(delta < 10_000, "window start off by {delta} ns");
        assert!(clock.fit_error_ns().unwrap_or(u64::MAX) < 10_000);
    }

    #[test]
    fn the_fit_anchors_on_the_earliest_observation_not_the_mean() {
        let t0 = Instant::now();
        let mut clock = slot_clock_at(t0);
        // A regular 400 ms train, each polled 20..380 ms after it began. The
        // lags are symmetric about the middle slot, so the mean lag is exactly
        // 200 ms and the cadence stays exactly 400 ms: a line through the mean
        // is 180 ms late, while the earliest sample is only 20 ms late.
        const LAGS_MS: [u64; 13] = [140, 260, 80, 320, 20, 380, 200, 380, 20, 320, 80, 260, 140];
        for (i, lag_ms) in LAGS_MS.iter().enumerate() {
            clock.observe_slot_at(
                300_000_000 + i as u64,
                t0 + Duration::from_millis(i as u64 * SLOT_MS as u64 + lag_ms),
            );
        }

        let predicted = clock
            .predict_window_start_ns(300_000_012)
            .expect("polled train must fit");
        // Truth: slot 300_000_012 began at t0 + 12 * 400 ms.
        let truth_ns: i128 = 12 * SLOT_MS as i128 * 1_000_000;
        let delta_ms = (predicted as i128 - truth_ns).abs() as f64 / 1e6;
        assert!(
            delta_ms < 30.0,
            "boundary off by {delta_ms} ms; a mean-anchored fit would be ~180 ms late"
        );
    }

    #[test]
    fn schedule_subtracts_lead_and_build_from_the_window_start() {
        let t0 = Instant::now();
        let clock = regular_clock(t0);
        let tracker = warm(50); // constant 120 ms -> zero sigma -> lead 120 ms

        let fire = clock
            .schedule(300_000_040, &tracker, 5)
            .expect("measured clock must schedule");

        assert!(
            (fire.lead_ms as f64 - 120.0).abs() < 1e-9,
            "{}",
            fire.lead_ms
        );
        let start = clock.predict_window_start_ns(300_000_040).expect("start");
        let expected = start - 125_000_000;
        assert_eq!(fire.window_start_ns, start);
        assert_eq!(fire.fire_at_ns, expected);
        assert_eq!(fire.fit_samples, 20);
    }

    #[test]
    fn schedule_refuses_without_a_round_trip_measurement() {
        let t0 = Instant::now();
        let clock = regular_clock(t0);
        let cold = RttTracker::new(1.0);
        assert_eq!(cold.lead_ms(), 0.0);
        assert!(
            clock.schedule(300_000_040, &cold, 5).is_none(),
            "no measurement means no deadline, not a guessed one"
        );
    }

    #[test]
    fn duplicate_slots_and_backwards_time_are_not_a_clock() {
        let t0 = Instant::now();

        let mut duplicate = slot_clock_at(t0);
        for i in 0..10u64 {
            duplicate.observe_slot_at(7, t0 + Duration::from_millis(i * 400));
        }
        assert!(duplicate.predict_window_start_ns(50).is_none());

        let mut backwards = slot_clock_at(t0);
        for i in 0..10u64 {
            backwards.observe_slot_at(300_000_000 + i, t0 + Duration::from_millis(4_000 - i * 400));
        }
        assert!(
            backwards.predict_window_start_ns(300_000_020).is_none(),
            "a negative slope is not a clock"
        );
    }

    #[test]
    fn wait_duration_never_underflows() {
        let t0 = Instant::now();
        let clock = regular_clock(t0);
        assert_eq!(clock.wait_duration(0), Duration::ZERO);

        let wait = clock.wait_duration(clock.now_ns() + 1_000_000_000);
        assert!(
            wait >= Duration::from_millis(900) && wait <= Duration::from_millis(1_000),
            "unexpected wait: {wait:?}"
        );
    }

    #[test]
    fn lead_is_p95_plus_three_sigma_and_capped() {
        let mut tracker = RttTracker::new(1.0);
        for _ in 0..40 {
            assert!(tracker.observe(3_000.0, 1.0));
        }
        assert_eq!(tracker.lead_ms(), MAX_MARGIN_MS as f64);

        let mut jittery = RttTracker::new(1.0);
        for rtt in [100.0, 110.0, 120.0, 130.0, 140.0].iter().cycle().take(100) {
            assert!(jittery.observe(*rtt, 0.5));
        }
        let lead = jittery.lead_ms();
        assert!(
            lead > jittery.p95_ms(),
            "lead {lead} must exceed the bare p95 {} once sigma > 0",
            jittery.p95_ms()
        );
        assert!(lead <= MAX_MARGIN_MS as f64);
    }
}
