//! Three-pool flash-funded net-economics tests.
//!
//! Validates the directive's economic matrices for the 3-leg Meteora DLMM
//! pipeline:
//!
//! * the size-search objective is NET profit (gross - Kamino flash fee -
//!   network cost), never gross output
//! * candidates above the actual available Kamino flash liquidity are rejected
//! * a gross-positive / net-negative route fails closed
//! * a network cost larger than the gross edge fails closed
//! * the flash fee is the exact klend Exclusive value (no truncation, no
//!   double count)

mod common;

use arb_engine::{
    FlashLoanSizingConfig, build_three_leg_meme_pipeline, calculate_kamino_flash_fee,
    calculate_optimal_flash_loan, net_profit_lamports, simulate_profitability_gate,
};
use common::{meme_pool_fixture, params as base_params};

const FLASH_LOAN_FEE_SF: u64 = 11_529_215_046_068; // 1e-5 U68F60

#[test]
fn net_profit_deducts_flash_fee_and_network_cost() {
    let pools = meme_pool_fixture();
    let mut p = base_params(100_000_000);
    p.flash_loan_fee_sf = FLASH_LOAN_FEE_SF;
    p.network_cost_lamports = 25_000;

    let result = build_three_leg_meme_pipeline(&pools, &p).unwrap();
    let gross = result.gross_profit_lamports;
    let fee = result.flash_fee_lamports;
    assert_eq!(
        result.net_profit_lamports,
        gross - fee as i64 - 25_000,
        "net must be gross - flash fee - network cost"
    );
    assert_eq!(
        fee,
        calculate_kamino_flash_fee(result.size_search.best_input, FLASH_LOAN_FEE_SF)
    );
}

#[test]
fn size_search_ranks_by_net_not_gross() {
    let pools = meme_pool_fixture();
    let mut p = base_params(100_000_000);
    p.flash_loan_fee_sf = FLASH_LOAN_FEE_SF;
    p.network_cost_lamports = 1_000;
    let result = build_three_leg_meme_pipeline(&pools, &p).unwrap();

    // The reported best must be the argmax of the net column.
    let best = result
        .size_search
        .tested_sizes
        .iter()
        .max_by_key(|(_, net)| *net)
        .unwrap()
        .0;
    assert_eq!(best, result.size_search.best_input);
    assert_eq!(
        result.net_profit_lamports,
        result.size_search.best_net_profit_lamports
    );
}

#[test]
fn gross_positive_net_negative_fails_closed() {
    let pools = meme_pool_fixture();
    let mut p = base_params(100_000_000);
    p.flash_loan_fee_sf = FLASH_LOAN_FEE_SF;
    // Network cost larger than the gross edge: gross > 0 but net < 0.
    p.network_cost_lamports = 1_000_000_000;
    let err = build_three_leg_meme_pipeline(&pools, &p).unwrap_err();
    assert!(
        err.to_string().contains("net profit"),
        "expected net-profit-floor rejection, got: {err}"
    );
}

#[test]
fn flash_fee_exceeding_gross_profit_fails_closed() {
    let pools = meme_pool_fixture();
    let mut p = base_params(100_000_000);
    // Disabled-reserve sentinel (u64::MAX) must fail closed, never trade.
    p.flash_loan_fee_sf = u64::MAX;
    let err = build_three_leg_meme_pipeline(&pools, &p).unwrap_err();
    assert!(
        err.to_string().contains("net profit"),
        "expected net-profit-floor rejection, got: {err}"
    );
}

#[test]
fn candidates_above_available_flash_liquidity_rejected() {
    let pools = meme_pool_fixture();
    let mut p = base_params(100_000_000);
    p.flash_loan_fee_sf = FLASH_LOAN_FEE_SF;
    // Every ladder entry exceeds the actual vault balance.
    p.kamino_available_liquidity = Some(1_000);
    let err = build_three_leg_meme_pipeline(&pools, &p).unwrap_err();
    assert!(
        err.to_string().contains("available Kamino flash liquidity"),
        "expected insufficient-flash-liquidity rejection, got: {err}"
    );
}

#[test]
fn borrow_ceiling_clamps_to_available_liquidity() {
    let pools = meme_pool_fixture();
    let mut p = base_params(100_000_000);
    p.flash_loan_fee_sf = FLASH_LOAN_FEE_SF;
    // Ladder: [10M, 50M, 100M]; vault holds 60M -> 100M must be dropped.
    p.kamino_available_liquidity = Some(60_000_000);
    let result = build_three_leg_meme_pipeline(&pools, &p).unwrap();
    assert!(
        result.size_search.best_input <= 60_000_000,
        "borrow {} exceeded available liquidity",
        result.size_search.best_input
    );
    assert!(
        result
            .size_search
            .tested_sizes
            .iter()
            .all(|(size, _)| *size <= 60_000_000),
        "tested sizes must respect the vault ceiling"
    );
}

#[test]
fn profitability_gate_uses_net_profit() {
    let pools = meme_pool_fixture();
    let mut p = base_params(100_000_000);
    p.flash_loan_fee_sf = FLASH_LOAN_FEE_SF;
    p.network_cost_lamports = 0;
    let result = build_three_leg_meme_pipeline(&pools, &p).unwrap();

    // The gate must clear at the net floor and fail one lamport above net.
    let net = simulate_profitability_gate(&result, p.min_profit_lamports).unwrap();
    assert_eq!(net, result.net_profit_lamports);
    let too_high = (result.net_profit_lamports as u64) + 1;
    assert!(simulate_profitability_gate(&result, too_high).is_err());
}

#[test]
fn sizing_helper_charges_exact_klend_fee() {
    let pools = meme_pool_fixture();
    let cfg = FlashLoanSizingConfig::default();
    let result = calculate_optimal_flash_loan(
        &pools[0],
        &pools[1],
        &pools[2],
        FLASH_LOAN_FEE_SF,
        &cfg,
        5_000,
        0,
        0,
    );
    let expected_fee = calculate_kamino_flash_fee(result.borrow_amount, FLASH_LOAN_FEE_SF);
    assert_eq!(result.flash_fee, expected_fee);
    assert!(result.borrow_amount > 0);
    // The chain must actually quote through real mints: the final WSOL output
    // and its slippage floor must be non-zero for the profitable fixture.
    assert!(
        result.expected_final_output > 0,
        "final WSOL output must be quoted"
    );
    assert!(
        result.leg_min_outputs[2] > 0,
        "conservative floor must be quoted"
    );
    // Net accounting identity on the conservative floor: profit = floor_out
    // - borrow - exact klend fee - network cost (dex fees are already embedded
    // in the quoted outputs and must NOT be subtracted again).
    assert_eq!(
        result.expected_profit,
        net_profit_lamports(
            result.leg_min_outputs[2],
            result.borrow_amount,
            FLASH_LOAN_FEE_SF,
            result.network_costs,
        )
    );
    assert!(
        result.expected_profit > 0,
        "profitable fixture must size to positive net"
    );
}

#[test]
fn route_fails_closed_when_repayment_not_guaranteed() {
    let pools = meme_pool_fixture();
    let mut p = base_params(100_000_000);
    p.flash_loan_fee_sf = FLASH_LOAN_FEE_SF;
    p.network_cost_lamports = 0;
    // 50% per-leg slippage haircut collapses the guaranteed final floor far
    // below principal + fee: the pipeline must fail closed instead of emitting
    // a route that cannot atomically repay the flash loan.
    p.slippage_bps = 5_000;
    let err = build_three_leg_meme_pipeline(&pools, &p).unwrap_err();
    assert!(matches!(
        err,
        arb_engine::ThreeLegMemePipelineError::RepaymentNotGuaranteed { .. }
    ));
}
