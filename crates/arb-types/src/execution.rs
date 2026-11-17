//! Jito tip market model, posteriors, bid decisions and inflight status.

use super::primitives::*;
use serde::{Deserialize, Serialize};
use std::fmt;

// Jito tip market — quantiles, posteriors, bid decisions
// ============================================================================

/// Z-score for a one-sided 95% confidence bound (`Phi^-1(0.95)`).
pub const Z_95: f64 = 1.644_853_626_951_472_2;

/// Rung of the landed-tip distribution published by the Jito tip floor.
///
/// The rung is the *treatment* the bidder learns from: bidding at `P90` implies
/// roughly 90% inclusion, and the observed landing rate at that rung corrects
/// that assumption. It is deliberately not a per-block-engine concept — Jito
/// resolves all regions as a single auction, so a landing cannot be attributed
/// to one region and per-region credit assignment would be fabricated data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Percentile {
    P50,
    P65,
    P75,
    P85,
    P90,
    P95,
    P99,
}

impl Percentile {
    /// Number of rungs in the grid.
    pub const COUNT: usize = 7;

    /// The rung grid, cheapest first.
    pub const GRID: [Self; Self::COUNT] = [
        Self::P50,
        Self::P65,
        Self::P75,
        Self::P85,
        Self::P90,
        Self::P95,
        Self::P99,
    ];

    /// Stable storage index, `0..COUNT`.
    pub const fn index(self) -> usize {
        match self {
            Self::P50 => 0,
            Self::P65 => 1,
            Self::P75 => 2,
            Self::P85 => 3,
            Self::P90 => 4,
            Self::P95 => 5,
            Self::P99 => 6,
        }
    }

    /// Stable, human-readable label (`"q50"` .. `"q99"`).
    pub const fn label(self) -> &'static str {
        match self {
            Self::P50 => "q50",
            Self::P65 => "q65",
            Self::P75 => "q75",
            Self::P85 => "q85",
            Self::P90 => "q90",
            Self::P95 => "q95",
            Self::P99 => "q99",
        }
    }

    /// Market-implied inclusion probability of a bid priced at this rung.
    ///
    /// Used as the prior when the ledger holds no observation for the rung: a
    /// bid at the 95th percentile of *landed* tips is assumed to be included
    /// about 95% of the time.
    pub fn probability(self) -> f64 {
        match self {
            Self::P50 => 0.50,
            Self::P65 => 0.65,
            Self::P75 => 0.75,
            Self::P85 => 0.85,
            Self::P90 => 0.90,
            Self::P95 => 0.95,
            Self::P99 => 0.99,
        }
    }
}

impl fmt::Display for Percentile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Immutable snapshot of the Jito landed-tip distribution, in lamports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TipFloor {
    /// Wall-clock time the snapshot was produced, milliseconds since UNIX epoch.
    pub observed_at_ms: u64,
    pub p25: Lamports,
    pub p50: Lamports,
    pub p75: Lamports,
    pub p95: Lamports,
    pub p99: Lamports,
    /// Exponential moving average of the 50th percentile.
    pub ema50: Lamports,
}

impl TipFloor {
    /// Price a rung. Rungs the endpoint does not publish (`P65`, `P85`, `P90`)
    /// are linearly interpolated between their bracketing published rungs.
    pub fn at(&self, rung: Percentile) -> Lamports {
        match rung {
            Percentile::P50 => self.p50,
            Percentile::P65 => Self::lerp(self.p50, self.p75, 3, 5),
            Percentile::P75 => self.p75,
            Percentile::P85 => Self::lerp(self.p75, self.p95, 1, 2),
            Percentile::P90 => Self::lerp(self.p75, self.p95, 3, 4),
            Percentile::P95 => self.p95,
            Percentile::P99 => self.p99,
        }
    }

    /// Integer linear interpolation, `a + (b - a) * num / den`.
    fn lerp(a: Lamports, b: Lamports, num: u64, den: u64) -> Lamports {
        let den = u128::from(den.max(1));
        let a_wide = u128::from(a.0);
        let b_wide = u128::from(b.0);
        let delta = b_wide.saturating_sub(a_wide);
        let out = a_wide.saturating_add(delta.saturating_mul(u128::from(num)) / den);
        Lamports(u64::try_from(out).unwrap_or(u64::MAX))
    }

    /// Age of the snapshot in milliseconds, saturating at zero for clock skew.
    pub fn age_ms(&self, now_ms: u64) -> u64 {
        now_ms.saturating_sub(self.observed_at_ms)
    }

    /// True when the snapshot is young enough to price a bid from.
    pub fn is_fresh(&self, now_ms: u64, max_age_ms: u64) -> bool {
        self.age_ms(now_ms) <= max_age_ms
    }
}

/// Beta posterior over a landing probability.
///
/// `alpha` counts landings, `beta` counts misses, with a `Beta(1, 1)` uniform
/// prior so an unobserved rung reports the prior mean (0.5) rather than 0.0.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Posterior {
    pub alpha: f64,
    pub beta: f64,
}

impl Posterior {
    /// Uniform prior, `Beta(1, 1)`.
    pub const PRIOR: Self = Self {
        alpha: 1.0,
        beta: 1.0,
    };

    /// Fold one observation into the posterior.
    pub fn record(&mut self, landed: bool) {
        if landed {
            self.alpha += 1.0;
        } else {
            self.beta += 1.0;
        }
    }

    /// Number of observations folded in, excluding the prior.
    pub fn observations(&self) -> f64 {
        (self.alpha + self.beta - 2.0).max(0.0)
    }

    /// Posterior mean.
    pub fn mean(&self) -> f64 {
        let total = self.alpha + self.beta;
        if total <= 0.0 {
            return 0.5;
        }
        (self.alpha / total).clamp(0.0, 1.0)
    }

    /// Wilson score lower bound on the success probability.
    ///
    /// A raw mean over a handful of samples is optimistic; the lower bound
    /// refuses to price an unproven rung as if it were certain.
    pub fn wilson_lower(&self, z: f64) -> f64 {
        let n = self.alpha + self.beta;
        if n <= 0.0 || !z.is_finite() || z <= 0.0 {
            return 0.0;
        }
        let p = (self.alpha / n).clamp(0.0, 1.0);
        let z_sq = z * z;
        let denom = 1.0 + z_sq / n;
        let centre = p + z_sq / (2.0 * n);
        let variance = p * (1.0 - p) / n + z_sq / (4.0 * n * n);
        let spread = z * variance.max(0.0).sqrt();
        ((centre - spread) / denom).clamp(0.0, 1.0)
    }
}

impl Default for Posterior {
    fn default() -> Self {
        Self::PRIOR
    }
}

/// Why a bid was declined. A refused opportunity is not repriced — it is dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefuseReason {
    /// The edge does not clear the profit hurdle, so no tip can satisfy it.
    EdgeBelowHurdle,
    /// The ceiling cannot fund the protocol minimum tip of 1000 lamports.
    EdgeBelowMinimumTip,
    /// No fresh tip-floor snapshot, so there is no published price to bid against.
    MarketUnavailable,
    /// Every rung priced at or above the hurdle produced a non-positive EV.
    NoPositiveExpectedValue,
    /// The target slot's leader does not run Jito-Solana, so no auction exists.
    LeaderNotJito,
}

impl fmt::Display for RefuseReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EdgeBelowHurdle => f.write_str("edge below profit hurdle"),
            Self::EdgeBelowMinimumTip => f.write_str("edge cannot fund the minimum Jito tip"),
            Self::MarketUnavailable => f.write_str("no fresh tip-floor snapshot"),
            Self::NoPositiveExpectedValue => {
                f.write_str("no rung produced positive expected value")
            }
            Self::LeaderNotJito => f.write_str("leader does not run Jito-Solana"),
        }
    }
}

/// The bid itself, or the reason there is none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TipDecision {
    Bid {
        tip: Lamports,
        rung: Percentile,
        /// True when the escalation cap reduced an otherwise larger bid.
        escalated: bool,
    },
    Refuse {
        reason: RefuseReason,
    },
}

impl TipDecision {
    /// The tip to actually pay, if any.
    pub fn tip(self) -> Option<Lamports> {
        match self {
            Self::Bid { tip, .. } => Some(tip),
            Self::Refuse { .. } => None,
        }
    }

    /// True when a bid may be built.
    pub fn is_bid(self) -> bool {
        matches!(self, Self::Bid { .. })
    }
}

/// Full trace of one pricing decision, kept for audit and telemetry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TipPlan {
    pub decision: TipDecision,
    /// Net profit before the tip is deducted.
    pub edge: Lamports,
    /// Portion of the edge that must survive all costs.
    pub hurdle: Lamports,
    /// Hard price ceiling `t_max`.
    pub ceiling: Lamports,
    /// Land probability used for the winning rung.
    pub inclusion_p: f64,
    /// `inclusion_p * (edge - tip)`, in lamports. Negative when the tip outstrips the edge.
    pub expected_value: i64,
    /// Market snapshot the plan was priced against, if one was fresh.
    pub market: Option<TipFloor>,
    /// Tip paid on the previous attempt in the same window, if any.
    pub previous_tip: Option<Lamports>,
}

impl TipPlan {
    /// True when the plan produced a payable bid.
    pub fn is_bid(&self) -> bool {
        self.decision.is_bid()
    }
}

/// Status of a submitted bundle reported by `getInflightBundleStatuses`.
///
/// The endpoint only looks back five minutes, so `Invalid` means "no longer in
/// the block engine's records" — never "failed to land".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InflightStatus {
    /// Bundle id unknown to the block engine (outside the 5 minute window).
    Invalid,
    /// Not failed, not landed, not invalid.
    Pending,
    /// Every region that received the bundle marked it failed.
    Failed,
    /// Landed on chain.
    Landed { slot: u64 },
}

impl InflightStatus {
    /// True when polling may stop.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Landed { .. } | Self::Failed | Self::Invalid)
    }

    /// Evidence to fold into the landing ledger.
    ///
    /// `Some(true)` landed, `Some(false)` failed, `None` when the status carries
    /// no evidence about the bid — which covers both `Pending` and `Invalid`.
    pub fn evidence(self) -> Option<bool> {
        match self {
            Self::Landed { .. } => Some(true),
            Self::Failed => Some(false),
            Self::Pending | Self::Invalid => None,
        }
    }
}
