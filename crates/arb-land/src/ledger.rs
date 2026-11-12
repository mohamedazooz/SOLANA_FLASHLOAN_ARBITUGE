//! Empirical landing-rate ledger.
//!
//! The bidder's core assumption is that bidding at the `q`-th percentile of the
//! landed-tip distribution is included about `q` of the time. That is a market
//! prior, not a measurement. This ledger replaces it with the realised landing
//! rate observed at each rung.
//!
//! The learning unit is the **price rung**, not the block-engine region. Jito
//! resolves every region as a single auction and reports one landing per bundle
//! id, so a landing cannot be attributed to the region that "won" it. Recording
//! per-region posteroors would manufacture attribution that does not exist and
//! would then be multiplied across nine regions into a fictional joint
//! probability.
//!
//! Coverage is therefore a single lower bound per rung: never a product.

use arb_types::{Percentile, Posterior, Z_95};
use serde::{Deserialize, Serialize};

/// Schema version of the serialized ledger.
///
/// A snapshot written by an older schema is discarded rather than reinterpreted,
/// because a silently mis-read posterior is a mispriced bid.
pub const LEDGER_SCHEMA: u32 = 1;

fn schema_version() -> u32 {
    LEDGER_SCHEMA
}

/// Per-rung Beta posteriors over the landing probability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LandingLedger {
    #[serde(default = "schema_version")]
    schema: u32,
    /// One posterior per rung, indexed by [`Percentile::index`].
    #[serde(default)]
    rungs: [Posterior; Percentile::COUNT],
    /// Total landed observations.
    landings: u64,
    /// Total observations that produced evidence (landed or failed).
    attempts: u64,
}

impl LandingLedger {
    /// A ledger with uniform priors and no observations.
    pub fn new() -> Self {
        Self {
            schema: LEDGER_SCHEMA,
            rungs: [Posterior::PRIOR; Percentile::COUNT],
            landings: 0,
            attempts: 0,
        }
    }

    /// Schema version carried by this instance.
    pub fn schema(&self) -> u32 {
        self.schema
    }

    /// Posterior for one rung. Unknown rungs report the uniform prior.
    pub fn posterior(&self, rung: Percentile) -> Posterior {
        self.rungs
            .get(rung.index())
            .copied()
            .unwrap_or(Posterior::PRIOR)
    }

    /// Observations recorded against one rung, excluding the prior.
    pub fn observations(&self, rung: Percentile) -> f64 {
        self.posterior(rung).observations()
    }

    /// Fold one landing observation into a rung.
    pub fn record(&mut self, rung: Percentile, landed: bool) {
        if let Some(posterior) = self.rungs.get_mut(rung.index()) {
            posterior.record(landed);
        }
        self.attempts = self.attempts.saturating_add(1);
        if landed {
            self.landings = self.landings.saturating_add(1);
        }
    }

    /// Land probability to price a bid at `rung` with.
    ///
    /// Measured rungs use the Wilson lower bound, so a rung with three lucky
    /// landings is not treated as certain. An unmeasured rung falls back to the
    /// rung's own market-implied probability, which is the only defensible
    /// number before evidence exists.
    pub fn coverage(&self, rung: Percentile) -> f64 {
        self.coverage_with(rung, Z_95)
    }

    /// [`Self::coverage`] with an explicit confidence z-score.
    pub fn coverage_with(&self, rung: Percentile, z: f64) -> f64 {
        let posterior = self.posterior(rung);
        if posterior.observations() <= 0.0 {
            rung.probability()
        } else {
            posterior.wilson_lower(z)
        }
    }

    /// Posterior mean for one rung, for reporting.
    pub fn mean(&self, rung: Percentile) -> f64 {
        self.posterior(rung).mean()
    }

    /// Realised landing rate across every rung, `0.5` before any observation.
    pub fn landing_rate(&self) -> f64 {
        if self.attempts == 0 {
            return 0.5;
        }
        (self.landings as f64 / self.attempts as f64).clamp(0.0, 1.0)
    }

    /// Total observations that produced evidence.
    pub fn attempts(&self) -> u64 {
        self.attempts
    }

    /// Total landed observations.
    pub fn landings(&self) -> u64 {
        self.landings
    }

    /// True when no rung has been measured yet.
    pub fn is_empty(&self) -> bool {
        self.attempts == 0
    }

    /// Discard every observation and return to the priors.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Serialize for persistence across restarts.
    ///
    /// Learning that dies with the process is not learning: the posteriors are
    /// the only accumulated asset in this system.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Restore a snapshot, rejecting a schema this build does not understand.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let ledger: Self = serde_json::from_str(json)?;
        if ledger.schema == LEDGER_SCHEMA {
            Ok(ledger)
        } else {
            Ok(Self::new())
        }
    }
}

impl Default for LandingLedger {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unmeasured_rung_reports_market_probability() {
        let ledger = LandingLedger::new();
        assert!(ledger.is_empty());
        assert!((ledger.coverage(Percentile::P50) - 0.50).abs() < 1e-12);
        assert!((ledger.coverage(Percentile::P99) - 0.99).abs() < 1e-12);
    }

    #[test]
    fn measured_rung_uses_lower_bound_not_mean() {
        let mut ledger = LandingLedger::new();
        for _ in 0..3 {
            ledger.record(Percentile::P90, true);
        }
        let mean = ledger.mean(Percentile::P90);
        let coverage = ledger.coverage(Percentile::P90);
        assert!(
            coverage < mean,
            "lower bound {coverage} must trail mean {mean}"
        );
        assert!(coverage > 0.0);
    }

    #[test]
    fn repeated_failures_drive_coverage_down() {
        let mut ledger = LandingLedger::new();
        for _ in 0..40 {
            ledger.record(Percentile::P99, false);
        }
        let coverage = ledger.coverage(Percentile::P99);
        assert!(coverage < 0.10, "coverage should collapse, got {coverage}");
        assert!(ledger.landing_rate() < 0.05);
    }

    #[test]
    fn coverage_converges_to_realised_rate() {
        let mut ledger = LandingLedger::new();
        // 60% realised landing rate at this rung.
        for i in 0..200 {
            ledger.record(Percentile::P75, i % 10 < 6);
        }
        let coverage = ledger.coverage(Percentile::P75);
        assert!(
            (0.50..0.62).contains(&coverage),
            "lower bound should sit just under 0.60, got {coverage}"
        );
    }

    #[test]
    fn rungs_are_independent() {
        let mut ledger = LandingLedger::new();
        for _ in 0..10 {
            ledger.record(Percentile::P50, true);
        }
        assert!(ledger.observations(Percentile::P50) > 0.0);
        assert_eq!(ledger.observations(Percentile::P95), 0.0);
        assert!((ledger.coverage(Percentile::P95) - 0.95).abs() < 1e-12);
    }

    #[test]
    fn snapshot_roundtrips() {
        let mut ledger = LandingLedger::new();
        for _ in 0..7 {
            ledger.record(Percentile::P85, true);
        }
        for _ in 0..3 {
            ledger.record(Percentile::P85, false);
        }
        let json = ledger.to_json().expect("serialize");
        let restored = LandingLedger::from_json(&json).expect("deserialize");
        assert_eq!(restored, ledger);
        assert_eq!(restored.attempts(), 10);
        assert_eq!(restored.landings(), 7);
    }

    #[test]
    fn unknown_schema_is_discarded() {
        let json = r#"{"schema":999,"landings":5,"attempts":5}"#;
        let restored = LandingLedger::from_json(json).expect("deserialize");
        assert_eq!(restored.schema(), LEDGER_SCHEMA);
        assert!(restored.is_empty());
    }
}
