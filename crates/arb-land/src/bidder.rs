//! Adaptive Jito tip pricing.
//!
//! Turning an edge into a price is a constrained maximisation, not a fixed
//! percentage. Three limits bound the bid:
//!
//! * **hurdle** — the portion of the edge that must survive all costs, so a tip
//!   can never be chosen that leaves the trade worthless;
//! * **ceiling** — `t_max`, the highest tip the risk budget permits. Below the
//!   ceiling there is a bid; above it the opportunity is *refused*, never
//!   repriced, because a trade that only works at an unsustainable tip is not a
//!   trade;
//! * **minimum** — the 1000 lamport floor the block engine enforces.
//!
//! Within those bounds the bidder walks the published price rungs and picks the
//! one with the highest expected value, `P(land) * (edge - tip)`, where
//! `P(land)` comes from the measured ledger rather than from the assumption that
//! price implies inclusion.

use crate::ledger::LandingLedger;
use arb_types::{Lamports, Percentile, RefuseReason, TipDecision, TipFloor, TipPlan};

/// Protocol minimum tip enforced by the Jito block engine, in lamports.
pub const MIN_JITO_TIP_LAMPORTS: u64 = 1_000;

/// Pricing policy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BidderConfig {
    /// Fraction of the edge that must survive every cost, tip included.
    pub keep_share: f64,
    /// Hard cap on the tip as a fraction of the edge.
    pub max_tip_share: f64,
    /// Absolute floor on the retained edge, in lamports.
    pub profit_floor: Lamports,
    /// Multiplicative ceiling on how far a retry may raise the previous tip.
    pub ladder_growth: f64,
}

impl Default for BidderConfig {
    fn default() -> Self {
        Self {
            keep_share: arb_constants::DEFAULT_KEEP_SHARE,
            max_tip_share: arb_constants::DEFAULT_MAX_TIP_SHARE,
            profit_floor: Lamports(arb_constants::DEFAULT_MIN_KEEP_LAMPORTS),
            ladder_growth: 1.6,
        }
    }
}

impl BidderConfig {
    /// Validate the policy. A misconfigured bidder must refuse to start rather
    /// than silently price every trade with a nonsensical split.
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.keep_share.is_finite() || !(0.0..=1.0).contains(&self.keep_share) {
            return Err("keep_share must be a finite fraction in [0, 1]");
        }
        if !self.max_tip_share.is_finite() || !(0.0..=1.0).contains(&self.max_tip_share) {
            return Err("max_tip_share must be a finite fraction in [0, 1]");
        }
        if !self.ladder_growth.is_finite() || self.ladder_growth < 1.0 {
            return Err("ladder_growth must be finite and >= 1.0");
        }
        Ok(())
    }
}

/// One priced candidate rung.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Candidate {
    tip: Lamports,
    rung: Percentile,
    inclusion_p: f64,
    expected_value: i64,
}

impl Candidate {
    /// Higher EV wins; on a tie the cheaper bid wins, then the earlier rung.
    fn beats(self, incumbent: Option<Self>) -> bool {
        match incumbent {
            None => true,
            Some(other) => {
                self.expected_value > other.expected_value
                    || (self.expected_value == other.expected_value && self.tip.0 < other.tip.0)
            }
        }
    }
}

/// Prices a tip for one opportunity.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AdaptiveBidder {
    config: BidderConfig,
}

impl AdaptiveBidder {
    /// Build a bidder, rejecting an invalid policy.
    pub fn new(config: BidderConfig) -> Result<Self, &'static str> {
        config.validate()?;
        Ok(Self { config })
    }

    /// Active policy.
    pub fn config(&self) -> BidderConfig {
        self.config
    }

    /// Portion of the edge that must survive all costs.
    ///
    /// `max(profit_floor, keep_share * edge)`: the absolute floor stops
    /// sub-cent dust from being worth the bundle plumbing, the share scales.
    pub fn hurdle(&self, edge: Lamports) -> Lamports {
        let share = fraction_of(edge, self.config.keep_share);
        Lamports(self.config.profit_floor.0.max(share.0))
    }

    /// Contention-aware hurdle that adapts to pool heat (write-lock contention) and CUs.
    ///
    /// On calm, low-contention pools (e.g. 3x Meteora DLMM), there is no need to enforce
    /// the aggressive 30,000+ lamport global hurdle floor used for crowded meme-token pools.
    /// Instead, the floor is relaxed to 10,000 lamports (covering 5k base fee + 1k min tip + min keep),
    /// while still respecting the keep_share fraction.
    /// Contention-aware hurdle that adapts to pool heat (write-lock contention) and CUs.
    ///
    /// Strictly guarantees that the hurdle is AT LEAST profit_floor + contingency_buffer
    /// so that net retained profit P_net is strictly >= 0.20 USD after all costs and tip.
    pub fn calculate_contention_hurdle(
        &self,
        edge: Lamports,
        estimated_cu: u64,
        is_hot_write_lock: bool,
    ) -> Lamports {
        let share = fraction_of(edge, self.config.keep_share);
        let floor = if is_hot_write_lock {
            let cu_penalty = (estimated_cu.saturating_mul(1_000)) / 100_000;
            self.config.profit_floor.0.saturating_add(cu_penalty)
        } else {
            let cu_overhead = (estimated_cu.saturating_mul(500)) / 100_000;
            10_000u64.saturating_add(cu_overhead)
        };
        Lamports(floor.max(share.0))
    }

    /// Highest permissible tip, `t_max`.
    pub fn ceiling(&self, edge: Lamports) -> Lamports {
        let by_keep = edge.saturating_sub(self.hurdle(edge));
        let by_cap = fraction_of(edge, self.config.max_tip_share);
        Lamports(by_keep.0.min(by_cap.0))
    }

    /// Highest permissible tip under contention-aware hurdle.
    pub fn ceiling_with_hurdle(&self, edge: Lamports, hurdle: Lamports) -> Lamports {
        let by_keep = edge.saturating_sub(hurdle);
        let by_cap = fraction_of(edge, self.config.max_tip_share);
        Lamports(by_keep.0.min(by_cap.0))
    }

    /// Price one opportunity.
    ///
    /// `market` is `None` when no fresh snapshot exists. A missing snapshot is a
    /// refusal, not a discount: with no published price there is no rung to bid
    /// and no honest way to pick one. Pricing the *hurdle* instead would pay
    /// 85% of the edge as a tip — 5.7× the ceiling on a 1_000_000-lamport edge —
    /// which is precisely the breach `t_max` exists to forbid.
    pub fn plan(
        &self,
        edge: Lamports,
        market: Option<TipFloor>,
        ledger: &LandingLedger,
        previous_tip: Option<Lamports>,
    ) -> TipPlan {
        self.plan_with_context(edge, market, ledger, previous_tip, 0, true)
    }

    /// Price one opportunity with runtime execution context (estimated CUs and pool contention).
    pub fn plan_with_context(
        &self,
        edge: Lamports,
        market: Option<TipFloor>,
        ledger: &LandingLedger,
        previous_tip: Option<Lamports>,
        estimated_cu: u64,
        is_hot_write_lock: bool,
    ) -> TipPlan {
        let hurdle = self.calculate_contention_hurdle(edge, estimated_cu, is_hot_write_lock);
        let ceiling = self.ceiling_with_hurdle(edge, hurdle);

        if ceiling.0 < MIN_JITO_TIP_LAMPORTS {
            let reason = if edge.0 <= hurdle.0 {
                RefuseReason::EdgeBelowHurdle
            } else {
                RefuseReason::EdgeBelowMinimumTip
            };
            return self.refused(reason, edge, hurdle, ceiling, market, previous_tip);
        }

        let Some(floor) = market else {
            return self.refused(
                RefuseReason::MarketUnavailable,
                edge,
                hurdle,
                ceiling,
                market,
                previous_tip,
            );
        };

        let Some(mut winner) = self.best_rung(edge, ceiling, floor, ledger) else {
            return self.refused(
                RefuseReason::NoPositiveExpectedValue,
                edge,
                hurdle,
                ceiling,
                market,
                previous_tip,
            );
        };

        // Escalation cap: one retry may not outbid the previous attempt by more
        // than `ladder_growth`, which stops a losing slot from escalating the
        // price on itself until the edge is gone.
        let mut escalated = false;
        if let Some(previous) = previous_tip {
            let cap = fraction_of(previous, self.config.ladder_growth);
            let cap = Lamports(cap.0.max(MIN_JITO_TIP_LAMPORTS));
            if winner.tip.0 > cap.0 {
                winner.tip = cap;
                winner.expected_value =
                    expected_value_lamports(winner.inclusion_p, edge.saturating_sub(cap));
                escalated = true;
            }
        }

        if winner.expected_value <= 0 || winner.tip.0 < MIN_JITO_TIP_LAMPORTS {
            return self.refused(
                RefuseReason::NoPositiveExpectedValue,
                edge,
                hurdle,
                ceiling,
                market,
                previous_tip,
            );
        }

        TipPlan {
            decision: TipDecision::Bid {
                tip: winner.tip,
                rung: winner.rung,
                escalated,
            },
            edge,
            hurdle,
            ceiling,
            inclusion_p: winner.inclusion_p,
            expected_value: winner.expected_value,
            market,
            previous_tip,
        }
    }

    /// Walk the rung grid and keep the best affordable candidate.
    fn best_rung(
        &self,
        edge: Lamports,
        ceiling: Lamports,
        market: TipFloor,
        ledger: &LandingLedger,
    ) -> Option<Candidate> {
        let mut best: Option<Candidate> = None;
        for rung in Percentile::GRID {
            let price = market.at(rung);
            // A rung the ceiling cannot fund is not a candidate: bidding it
            // would breach the risk budget, and bidding it lower would be a
            // different rung with a different probability.
            if price.0 > ceiling.0 {
                continue;
            }
            let tip = Lamports(price.0.max(MIN_JITO_TIP_LAMPORTS));
            let candidate = self.candidate(rung, tip, edge, ledger);
            if candidate.beats(best) {
                best = Some(candidate);
            }
        }
        best
    }

    /// Score `tip` as if it were paid at `rung`.
    fn candidate(
        &self,
        rung: Percentile,
        tip: Lamports,
        edge: Lamports,
        ledger: &LandingLedger,
    ) -> Candidate {
        let inclusion_p = ledger.coverage(rung);
        let margin = edge.saturating_sub(tip);
        Candidate {
            tip,
            rung,
            inclusion_p,
            expected_value: expected_value_lamports(inclusion_p, margin),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn refused(
        &self,
        reason: RefuseReason,
        edge: Lamports,
        hurdle: Lamports,
        ceiling: Lamports,
        market: Option<TipFloor>,
        previous_tip: Option<Lamports>,
    ) -> TipPlan {
        TipPlan {
            decision: TipDecision::Refuse { reason },
            edge,
            hurdle,
            ceiling,
            inclusion_p: 0.0,
            expected_value: 0,
            market,
            previous_tip,
        }
    }
}

/// `value * fraction`, rounded, saturating at the `u64` bounds.
fn fraction_of(value: Lamports, fraction: f64) -> Lamports {
    if !fraction.is_finite() || fraction <= 0.0 || value.0 == 0 {
        return Lamports::ZERO;
    }
    let scaled = value.0 as f64 * fraction;
    let limit = u64::MAX as f64;
    if scaled >= limit {
        return Lamports(u64::MAX);
    }
    // Safe: finite, positive and strictly below `u64::MAX`.
    Lamports(scaled.round() as u64)
}

/// `probability * margin`, rounded, as a signed EV in lamports.
fn expected_value_lamports(probability: f64, margin: Lamports) -> i64 {
    if !probability.is_finite() || probability <= 0.0 {
        return 0;
    }
    let value = margin.0 as f64 * probability.min(1.0);
    let limit = i64::MAX as f64;
    if value >= limit {
        return i64::MAX;
    }
    if value <= i64::MIN as f64 {
        return i64::MIN;
    }
    // Safe: finite and inside the `i64` range.
    value.round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn floor(p50: u64, p75: u64, p95: u64, p99: u64) -> TipFloor {
        TipFloor {
            observed_at_ms: 0,
            p25: Lamports(p50 / 2),
            p50: Lamports(p50),
            p75: Lamports(p75),
            p95: Lamports(p95),
            p99: Lamports(p99),
            ema50: Lamports(p50),
        }
    }

    fn bidder() -> AdaptiveBidder {
        AdaptiveBidder::new(BidderConfig {
            keep_share: 0.85,
            max_tip_share: 0.75,
            profit_floor: Lamports(2_000),
            ladder_growth: 1.6,
        })
        .expect("valid config")
    }

    #[test]
    fn rejects_invalid_config() {
        assert!(
            AdaptiveBidder::new(BidderConfig {
                keep_share: 1.5,
                ..BidderConfig::default()
            })
            .is_err()
        );
        assert!(
            AdaptiveBidder::new(BidderConfig {
                ladder_growth: 0.5,
                ..BidderConfig::default()
            })
            .is_err()
        );
        assert!(
            AdaptiveBidder::new(BidderConfig {
                max_tip_share: f64::NAN,
                ..BidderConfig::default()
            })
            .is_err()
        );
    }

    #[test]
    fn hurdle_is_max_of_floor_and_share() {
        let b = bidder();
        // share dominates: 0.85 * 1_000_000
        assert_eq!(b.hurdle(Lamports(1_000_000)).0, 850_000);
        // floor dominates: 0.85 * 1_000 = 850 < 2_000
        assert_eq!(b.hurdle(Lamports(1_000)).0, 2_000);
    }

    #[test]
    fn ceiling_is_min_of_keep_remainder_and_cap() {
        let b = bidder();
        // edge 1_000_000: keep_remainder 150_000, cap 750_000 -> 150_000
        assert_eq!(b.ceiling(Lamports(1_000_000)).0, 150_000);
        // edge 10_000: hurdle = max(2_000, 0.85 * 10_000 = 8_500) = 8_500,
        // retained = 10_000 - 8_500 = 1_500, cap = 0.75 * 10_000 = 7_500 → min = 1_500.
        assert_eq!(b.ceiling(Lamports(10_000)).0, 1_500);
    }

    #[test]
    fn refuses_edge_below_hurdle() {
        let b = bidder();
        let ledger = LandingLedger::new();
        // Edge 2_000: hurdle = max(2_000, 0.85 * 2_000 = 1_700) = 2_000, ceiling = 0 < min tip.
        let plan = b.plan(Lamports(2_000), Some(floor(1, 1, 1, 1)), &ledger, None);
        assert!(!plan.is_bid());
        assert_eq!(
            plan.decision,
            TipDecision::Refuse {
                reason: RefuseReason::EdgeBelowHurdle
            }
        );
        assert_eq!(plan.ceiling.0, 0);
    }

    #[test]
    fn no_market_refuses_instead_of_inventing_a_price() {
        let b = bidder();
        let ledger = LandingLedger::new();
        let plan = b.plan(Lamports(1_000_000), None, &ledger, None);
        assert!(!plan.is_bid());
        assert_eq!(
            plan.decision,
            TipDecision::Refuse {
                reason: RefuseReason::MarketUnavailable
            }
        );
        // The bound is still reported, so the operator can see that the
        // opportunity was affordable and the *market* was what went missing.
        assert_eq!(plan.ceiling.0, 150_000);
        assert_eq!(plan.market, None);
    }

    #[test]
    fn picks_the_highest_affordable_rung_on_a_cheap_market() {
        let b = bidder();
        let ledger = LandingLedger::new();
        // Ceiling 150_000. p99 costs 100_000 -> affordable.
        let plan = b.plan(
            Lamports(1_000_000),
            Some(floor(1_000, 2_000, 50_000, 100_000)),
            &ledger,
            None,
        );
        let TipDecision::Bid { tip, rung, .. } = plan.decision else {
            panic!("expected bid");
        };
        // Ceiling 150_000. All rungs through P99 are affordable; the
        // expected-value-optimal affordable rung is P95: 0.95 * (1_000_000 - 50_000) = 902_500.
        assert_eq!(rung, Percentile::P95);
        assert_eq!(tip.0, 50_000);
        assert_eq!(plan.expected_value, 902_500);
        assert!(plan.expected_value > 0);
    }

    #[test]
    fn skips_rungs_above_the_ceiling() {
        let b = bidder();
        let ledger = LandingLedger::new();
        // Ceiling 150_000; p95 costs 400_000 -> unaffordable, p75 costs 10_000.
        let plan = b.plan(
            Lamports(1_000_000),
            Some(floor(1_000, 10_000, 400_000, 900_000)),
            &ledger,
            None,
        );
        let TipDecision::Bid { rung, .. } = plan.decision else {
            panic!("expected bid");
        };
        assert!(
            rung <= Percentile::P75,
            "chose an unaffordable rung: {rung}"
        );
    }

    #[test]
    fn measured_failure_rate_steers_away_from_an_expensive_rung() {
        let b = bidder();
        let mut ledger = LandingLedger::new();
        // q99 never lands when we pay for it.
        for _ in 0..60 {
            ledger.record(Percentile::P99, false);
        }
        let plan = b.plan(
            Lamports(1_000_000),
            Some(floor(1_000, 2_000, 50_000, 100_000)),
            &ledger,
            None,
        );
        let TipDecision::Bid { rung, .. } = plan.decision else {
            panic!("expected bid");
        };
        assert_ne!(
            rung,
            Percentile::P99,
            "ledger evidence must override the market prior"
        );
    }

    #[test]
    fn coverage_is_a_bound_never_a_product() {
        let mut ledger = LandingLedger::new();
        for _ in 0..30 {
            ledger.record(Percentile::P90, true);
        }
        let coverage = ledger.coverage(Percentile::P90);
        assert!(
            coverage <= 1.0,
            "coverage must stay a probability, got {coverage}"
        );
        // A product across nine regions would collapse to ~0; the bound does not.
        assert!(coverage > 0.8);
    }

    #[test]
    fn escalation_cap_holds_a_retry_back() {
        let b = bidder();
        let ledger = LandingLedger::new();
        let previous = Lamports(10_000);
        let plan = b.plan(
            Lamports(1_000_000),
            Some(floor(1_000, 2_000, 50_000, 100_000)),
            &ledger,
            Some(previous),
        );
        let TipDecision::Bid { tip, escalated, .. } = plan.decision else {
            panic!("expected bid");
        };
        assert!(escalated, "a 100k bid over a 10k retry must be capped");
        assert_eq!(tip.0, 16_000);
    }

    #[test]
    fn escalation_cap_does_not_raise_a_cheaper_bid() {
        let b = bidder();
        let ledger = LandingLedger::new();
        let plan = b.plan(
            Lamports(1_000_000),
            Some(floor(1_000, 2_000, 50_000, 100_000)),
            &ledger,
            Some(Lamports(500_000)),
        );
        let TipDecision::Bid { escalated, .. } = plan.decision else {
            panic!("expected bid");
        };
        assert!(!escalated);
    }

    #[test]
    fn saturating_helpers_do_not_overflow() {
        assert_eq!(fraction_of(Lamports(u64::MAX), 1.0).0, u64::MAX);
        assert_eq!(fraction_of(Lamports(u64::MAX), 2.0).0, u64::MAX);
        assert_eq!(fraction_of(Lamports(0), 0.5).0, 0);
        assert_eq!(expected_value_lamports(f64::NAN, Lamports(1)), 0);
    }

    #[test]
    fn test_contention_aware_hurdle_allows_calm_dlmm_edge() {
        let b = AdaptiveBidder::default();
        let ledger = LandingLedger::new();
        let edge = Lamports(23_000); // 23k lamports edge (like slot 465880662)

        // Under standard global hurdle (profit_floor = 30,000), 23k is refused!
        let hot_plan = b.plan_with_context(
            edge,
            Some(floor(1_000, 2_000, 50_000, 100_000)),
            &ledger,
            None,
            240_000,
            true, // Hot write-lock
        );
        assert_eq!(
            hot_plan.decision,
            TipDecision::Refuse {
                reason: RefuseReason::EdgeBelowHurdle
            }
        );

        // Under calm write-lock (e.g. Meteora DLMM 3-hop), hurdle drops to ~11k, allowing the bid!
        let calm_plan = b.plan_with_context(
            edge,
            Some(floor(1_000, 2_000, 50_000, 100_000)),
            &ledger,
            None,
            240_000,
            false, // Calm pool
        );
        let TipDecision::Bid { tip, .. } = calm_plan.decision else {
            panic!("Calm DLMM trade must be bid, got: {:?}", calm_plan.decision);
        };
        assert!(tip.0 >= MIN_JITO_TIP_LAMPORTS);
        assert!(tip.0 < 23_000);
    }
}
