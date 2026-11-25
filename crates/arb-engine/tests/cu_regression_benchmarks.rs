//! Item 531 — per-DEX compute-unit regression benchmarks.
//!
//! Purpose: turn the CU numbers the pipeline actually consumes into pinned
//! regression envelopes, so a silent estimate inflation (or a runaway CU model)
//! fails the build instead of degrading route ranking and wire capacity.
//!
//! Evidence separation (estimate ≠ measured):
//! * ESTIMATED — `cu_estimate()` ranking table (`crates/arb-engine/src/adapters.rs`)
//!   and the DLMM model `estimate_minimal_cu(true, hops) + CU_PER_BIN_ARRAY * bin_arrays`
//!   (`crates/arb-engine/src/dlmm_flash_arbitrage.rs`). These are conservative
//!   ranking/preflight numbers, not measurements.
//! * MEASURED — retained mainnet simulation evidence from the Phase 15.2 session:
//!   `reports/phase15_2/mainnet_simulation_evidence.json` (`compute_units_consumed:
//!   76954`, 3-leg USDC/WSOL/DLMM flash route) and
//!   `reports/phase15_2/PHASE15_2_FINAL_REPORT.md` (82,702 CU on the same-era route).
//!   Those two numbers anchor the measured envelope below; nothing in these tests
//!   fabricates a new measurement.

use arb_constants::MAX_COMPUTE_UNIT_LIMIT;
use arb_engine::adapters::cu_estimate;
use arb_engine::dlmm_flash_arbitrage::CU_PER_BIN_ARRAY;
use arb_tx::{calculate_dynamic_cu_limit, estimate_minimal_cu};
use arb_types::DexProtocol;

/// All protocol variants must stay covered — adding a DEX without a pinned
/// estimate is itself a regression (compile error via non-exhaustive match here).
const ALL_PROTOCOLS: [DexProtocol; 10] = [
    DexProtocol::RaydiumV4,
    DexProtocol::RaydiumCpmm,
    DexProtocol::RaydiumClmm,
    DexProtocol::OrcaWhirlpool,
    DexProtocol::MeteoraDlmm,
    DexProtocol::MeteoraDammV2,
    DexProtocol::MeteoraDbc,
    DexProtocol::RaydiumLaunchLab,
    DexProtocol::PumpFun,
    DexProtocol::PumpSwap,
];

/// Pinned per-DEX CU estimates, exactly as shipped in `adapters.rs`.
/// These values drive route ranking and the 200k wire-capacity rungs
/// (`reports/phase15_5/V1_SIZE_LADDER.json`); changing any of them must be a
/// conscious, reviewed edit — not a silent drift.
const PINNED_CU_ESTIMATES: [(DexProtocol, u64); 10] = [
    (DexProtocol::RaydiumV4, 65_000),
    (DexProtocol::RaydiumCpmm, 45_000),
    (DexProtocol::RaydiumClmm, 55_000),
    (DexProtocol::OrcaWhirlpool, 50_000),
    (DexProtocol::MeteoraDlmm, 60_000),
    (DexProtocol::MeteoraDammV2, 70_000),
    (DexProtocol::MeteoraDbc, 55_000),
    (DexProtocol::RaydiumLaunchLab, 60_000),
    (DexProtocol::PumpFun, 40_000),
    (DexProtocol::PumpSwap, 48_000),
];

/// Agave runtime hard ceiling the whole stack must respect.
/// Verified 2026-11-17 against anza-xyz/agave `program-runtime/src/execution_budget.rs`.
#[test]
fn max_compute_unit_limit_matches_agave_runtime_maximum() {
    assert_eq!(MAX_COMPUTE_UNIT_LIMIT, 1_400_000u32);
}

#[test]
fn cu_estimate_table_is_pinned_per_dex() {
    for (protocol, pinned) in PINNED_CU_ESTIMATES {
        assert_eq!(
            cu_estimate(protocol),
            pinned,
            "cu_estimate({:?}) drifted from the pinned ranking value; update the \
             pin consciously alongside route-ranking review",
            protocol
        );
    }
}

#[test]
fn every_dex_protocol_variant_has_a_pinned_estimate() {
    let pinned: Vec<DexProtocol> = PINNED_CU_ESTIMATES.iter().map(|(p, _)| *p).collect();
    for protocol in ALL_PROTOCOLS {
        assert!(
            pinned.contains(&protocol),
            "{:?} has no pinned cu_estimate entry",
            protocol
        );
        assert!(
            cu_estimate(protocol) > 0,
            "{:?} estimate must be positive",
            protocol
        );
    }
}

/// The per-hop estimates must never individually approach the runtime limit,
/// otherwise a 2–3 hop route would be born unrunnable.
#[test]
fn per_dex_estimates_stay_far_below_the_runtime_ceiling() {
    for protocol in ALL_PROTOCOLS {
        assert!(
            cu_estimate(protocol) <= u64::from(MAX_COMPUTE_UNIT_LIMIT) / 10,
            "{:?} estimate {} exceeds 10% of the runtime CU limit",
            protocol,
            cu_estimate(protocol)
        );
    }
}

/// DLMM 3-leg flash route CU model, reproduced exactly as written at
/// `dlmm_flash_arbitrage.rs:760` (estimate_minimal_cu(true, 2) with bin-array
/// plan cost). This is the ESTIMATED path; the MEASURED anchors are cited below.
#[test]
fn dlmm_flash_route_cu_model_pins_the_retained_route_shape() {
    // Minimal plan: 1 bin array per hop (plan floors at max(bins_crossed, 1)).
    let bins_min = 2usize;
    let model = |bin_arrays: usize| {
        estimate_minimal_cu(true, 2)
            .saturating_add(CU_PER_BIN_ARRAY.saturating_mul(bin_arrays as u32))
            .min(MAX_COMPUTE_UNIT_LIMIT)
    };

    // estimate_minimal_cu(true, 2) = 450 base + 28_000 flash + 2 * 35_000 hops.
    assert_eq!(estimate_minimal_cu(true, 2), 98_450);
    assert_eq!(model(bins_min), 128_450);
    assert_eq!(model(4), 158_450);
    assert_eq!(model(8), 218_450);
}

/// The model must remain conservative versus, but bounded by, the retained
/// mainnet evidence: measured 76,954–82,702 CU for the retained 3-leg
/// USDC/WSOL/DLMM flash route (`reports/phase15_2/mainnet_simulation_evidence.json`
/// and `PHASE15_2_FINAL_REPORT.md`). The estimate may over-cover (headroom is
/// the point) but must stay within 3x of the measured ceiling and always within
/// the runtime limit.
#[test]
fn dlmm_cu_model_stays_conservative_but_bounded_vs_retained_mainnet_evidence() {
    const MEASURED_MAINNET_CU_FLOOR: u64 = 69_954;
    const MEASURED_MAINNET_CU_CEILING: u64 = 82_702;

    let model = |bin_arrays: u32| {
        u64::from(
            estimate_minimal_cu(true, 2)
                .saturating_add(CU_PER_BIN_ARRAY.saturating_mul(bin_arrays))
                .min(MAX_COMPUTE_UNIT_LIMIT),
        )
    };

    let minimal = model(2);
    assert!(
        minimal >= MEASURED_MAINNET_CU_FLOOR,
        "model estimate {minimal} must cover the measured floor"
    );
    assert!(
        minimal <= MEASURED_MAINNET_CU_CEILING.saturating_mul(3),
        "model estimate {minimal} inflated beyond 3x the measured ceiling {MEASURED_MAINNET_CU_CEILING}"
    );
    assert!(minimal <= u64::from(MAX_COMPUTE_UNIT_LIMIT));

    // Full-planning worst case stays comfortably inside the runtime budget.
    let worst = model(8);
    assert!(worst <= u64::from(MAX_COMPUTE_UNIT_LIMIT));
    assert!(worst <= MEASURED_MAINNET_CU_CEILING.saturating_mul(3));
}

/// Mirror of the route aggregation (`adapters.rs` route quote:
/// `total_cu = total_cu.saturating_add(q.cu_estimate)`) plus the authoritative
/// clamp: whatever the hop mix, the dynamic limit never exits the Agave ceiling.
#[test]
fn route_cu_aggregation_and_dynamic_limit_clamp_to_runtime_ceiling() {
    // Heaviest possible hop mix within the 3-hop design space.
    let heaviest = cu_estimate(DexProtocol::MeteoraDammV2);
    for hops in 1usize..=3 {
        let route_total = heaviest
            .saturating_mul(hops as u64)
            .saturating_add(u64::from(estimate_minimal_cu(true, hops)));
        assert!(route_total <= u64::from(MAX_COMPUTE_UNIT_LIMIT));
        let clamped = calculate_dynamic_cu_limit(u32::try_from(route_total).unwrap());
        assert!(clamped <= MAX_COMPUTE_UNIT_LIMIT);
    }

    // Regression: >= 1,400,000 input clamps exactly to the runtime maximum
    // (1.3M would be 1,495,000 unclamped; values above u32::MAX saturate first).
    assert_eq!(
        calculate_dynamic_cu_limit(1_300_000),
        MAX_COMPUTE_UNIT_LIMIT
    );
    assert_eq!(
        calculate_dynamic_cu_limit(MAX_COMPUTE_UNIT_LIMIT),
        MAX_COMPUTE_UNIT_LIMIT
    );
    assert_eq!(calculate_dynamic_cu_limit(u32::MAX), MAX_COMPUTE_UNIT_LIMIT);
    // Below the clamp threshold the 15% buffer is preserved exactly.
    assert_eq!(calculate_dynamic_cu_limit(100_000), 115_000);
}
