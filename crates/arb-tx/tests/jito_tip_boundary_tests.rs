//! Feature 19: Jito Leader-Aware Adaptive Tip Engine — Tier 2 Boundary Tests.
//!
//! Validates:
//! 1. AdaptiveBidder clamps 0-tip market floor to MIN_JITO_TIP_LAMPORTS (1,000 lamports).
//! 2. Max tip ceiling is strictly enforced even when market tips spike to extreme levels.
//! 3. Leader tracker at epoch boundary transition returns None for out-of-epoch slots (fails closed).
//! 4. Missing / non-Jito leader landability classification fails closed (prevents tip burn).
//! 5. Retained profit shortfall below hurdle causes bidder to refuse bidding.

use arb_land::bidder::{AdaptiveBidder, BidderConfig, MIN_JITO_TIP_LAMPORTS};
use arb_land::leader::{JitoValidatorSet, Landability, LeaderSchedule, landability};
use arb_land::ledger::LandingLedger;
use arb_types::{Lamports, RefuseReason, Slot, TipDecision, TipFloor};
use solana_pubkey::Pubkey;

fn make_tip_floor(p25: u64, p50: u64, p75: u64, p95: u64, p99: u64) -> TipFloor {
    TipFloor {
        observed_at_ms: 1_700_000_000_000,
        p25: Lamports(p25),
        p50: Lamports(p50),
        p75: Lamports(p75),
        p95: Lamports(p95),
        p99: Lamports(p99),
        ema50: Lamports(p50),
    }
}

fn test_bidder() -> AdaptiveBidder {
    AdaptiveBidder::new(BidderConfig {
        keep_share: 0.85,
        max_tip_share: 0.75,
        profit_floor: Lamports(2_000),
        ladder_growth: 1.6,
    })
    .expect("valid bidder config")
}

/// Boundary Case 1: Market reports 0 tip floor; bidder clamps tip bid to MIN_JITO_TIP_LAMPORTS.
#[test]
fn test_adaptive_bidder_zero_tip_floor_clamped_to_minimum() {
    let bidder = test_bidder();
    let ledger = LandingLedger::new();
    let zero_market = make_tip_floor(0, 0, 0, 0, 0);

    let plan = bidder.plan(Lamports(1_000_000), Some(zero_market), &ledger, None);

    assert!(
        plan.is_bid(),
        "Bidder must generate a bid for sufficient edge"
    );
    let TipDecision::Bid { tip, .. } = plan.decision else {
        panic!("Expected TipDecision::Bid");
    };

    assert!(
        tip.0 >= MIN_JITO_TIP_LAMPORTS,
        "Tip {} must be clamped to at least MIN_JITO_TIP_LAMPORTS ({})",
        tip.0,
        MIN_JITO_TIP_LAMPORTS
    );
}

/// Boundary Case 2: Market tip spikes to extreme values; bidder strictly caps tip at ceiling.
#[test]
fn test_adaptive_bidder_max_tip_cap_enforcement() {
    let bidder = test_bidder();
    let ledger = LandingLedger::new();
    let extreme_market =
        make_tip_floor(10_000_000, 25_000_000, 50_000_000, 100_000_000, 200_000_000);

    let edge = Lamports(10_000_000);
    let ceiling = bidder.ceiling(edge);

    let plan = bidder.plan(edge, Some(extreme_market), &ledger, None);

    // If a bid is produced, it must NEVER exceed ceiling
    if let TipDecision::Bid { tip, .. } = plan.decision {
        assert!(
            tip.0 <= ceiling.0,
            "Tip {} must never exceed ceiling {}",
            tip.0,
            ceiling.0
        );
    } else {
        // If all rungs exceeded ceiling, bidder correctly refused
        assert_eq!(
            plan.decision,
            TipDecision::Refuse {
                reason: RefuseReason::NoPositiveExpectedValue
            }
        );
    }
}

/// Boundary Case 3: Slot at epoch boundary transition.
#[test]
fn test_leader_tracker_slot_at_epoch_boundary_transition() {
    let leader_id = Pubkey::new_unique();
    let leaders = vec![leader_id.to_string()];
    let schedule = LeaderSchedule::from_slot_leaders_slice(&leaders, 0, 0, 0, 432_000)
        .expect("valid schedule");

    // Slot 0 is covered and returns the known leader
    assert_eq!(schedule.leader_at(Slot(0)), Some(leader_id));
    assert!(schedule.covers(Slot(0)));

    // Slot 432_000 is outside epoch 0 (which spans 0..432_000)
    assert!(!schedule.covers(Slot(432_000)));
    assert_eq!(
        schedule.leader_at(Slot(432_000)),
        None,
        "Slot at epoch boundary transition without loaded next epoch must return None (fail-closed)"
    );

    // Slot 432_001 is also strictly outside
    assert_eq!(schedule.leader_at(Slot(432_001)), None);
}

/// Boundary Case 4: Non-Jito leader fails closed (prevents tip burn on plain validators).
#[test]
fn test_missing_tip_account_fails_closed() {
    let non_jito_leader = Pubkey::new_unique();
    let leaders = vec![non_jito_leader.to_string()];
    let schedule =
        LeaderSchedule::from_slot_leaders_slice(&leaders, 0, 0, 0, 100).expect("valid schedule");

    // Empty Jito validator set (leader is not in the set)
    let jito_set = JitoValidatorSet::default();

    let status = landability(Some(&schedule), Some(&jito_set), Slot(0));
    assert_eq!(
        status,
        Landability::NonJito,
        "Unknown/non-Jito leader must be classified as NonJito to fail closed"
    );

    // Unknown slot outside schedule
    let status_unknown = landability(Some(&schedule), Some(&jito_set), Slot(101));
    assert_eq!(
        status_unknown,
        Landability::Unknown,
        "Uncovered slot must be classified as Unknown"
    );
}

/// Boundary Case 5: Tip exceeds retained profit hurdle; bidder refuses the bid.
#[test]
fn test_adaptive_bidder_tip_exceeds_retained_profit_refused() {
    // Bidder configured with profit_floor = 15_000
    let bidder = AdaptiveBidder::new(BidderConfig {
        keep_share: 0.85,
        max_tip_share: 0.75,
        profit_floor: Lamports(15_000),
        ladder_growth: 1.6,
    })
    .expect("valid config");

    let ledger = LandingLedger::new();

    // Edge 100_000. Hurdle is max(15_000, 0.85 * 100_000 = 85_000) = 85_000.
    // Retained profit = 100_000 - tip. Max allowable tip is 15_000.
    // Market lowest tip is 90_000.
    let market = make_tip_floor(90_000, 95_000, 100_000, 105_000, 110_000);

    let plan = bidder.plan(Lamports(100_000), Some(market), &ledger, None);

    assert!(
        !plan.is_bid(),
        "Bidder must refuse when market tip exceeds available retained profit margin"
    );
    assert_eq!(
        plan.decision,
        TipDecision::Refuse {
            reason: RefuseReason::NoPositiveExpectedValue
        }
    );
}
