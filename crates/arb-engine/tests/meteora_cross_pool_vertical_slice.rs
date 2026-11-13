//! Meteora DLMM Cross-Pool Arbitrage Vertical Slice Integration Test.
//!
//! Validates:
//! 1. `MeteoraDlmmPoolPairDiscovery`
//! 2. `MeteoraDlmmCrossPoolQuoteEngine`
//! 3. `MeteoraDlmmTradeSizeOptimizer`

use arb_engine::{
    DlmmOptimizerConfig, MeteoraDlmmCrossPoolQuoteEngine, MeteoraDlmmPoolPairDiscovery,
    MeteoraDlmmTradeSizeOptimizer,
};
use arb_types::{
    DexProtocol, DlmmBin, DlmmFeeParams, DlmmState, MintId, PoolExecutionAccounts, PoolId,
    PoolKind, PoolState, Slot, TokenProgramKind,
};
use solana_pubkey::Pubkey;

fn make_test_dlmm_pool(
    pool_bytes_id: u8,
    active_bin_id: i32,
    bin_step: u16,
    base_factor: u16,
    mint_x_byte: u8,
    mint_y_byte: u8,
    liquidity_x: u64,
    liquidity_y: u64,
) -> PoolState {
    let pool_pk = Pubkey::new_from_array([pool_bytes_id; 32]);
    let reserve_x = Pubkey::new_from_array([10 + pool_bytes_id; 32]);
    let reserve_y = Pubkey::new_from_array([20 + pool_bytes_id; 32]);
    let mint_x = MintId(Pubkey::new_from_array([mint_x_byte; 32]));
    let mint_y = MintId(Pubkey::new_from_array([mint_y_byte; 32]));

    let bins = vec![
        DlmmBin {
            bin_id: active_bin_id - 1,
            amount_x: (liquidity_x as u128) * 2,
            amount_y: 0,
        },
        DlmmBin {
            bin_id: active_bin_id,
            amount_x: liquidity_x as u128,
            amount_y: liquidity_y as u128,
        },
        DlmmBin {
            bin_id: active_bin_id + 1,
            amount_x: 0,
            amount_y: (liquidity_y as u128) * 2,
        },
    ];

    let fee_params = DlmmFeeParams {
        base_factor,
        filter_period: 30,
        decay_period: 600,
        reduction_factor: 5000,
        variable_fee_control: 40000,
        max_volatility_accumulator: 350000,
        protocol_share: 1000,
        base_fee_power_factor: 0,
        function_type: 0,
        collect_fee_mode: 0,
    };

    let dlmm_state = DlmmState {
        active_bin_id,
        status: 0,
        bin_step_bps: bin_step,
        bins,
        params: Some(fee_params),
        volatility_accumulator: 0,
        volatility_reference: 0,
        index_reference: active_bin_id,
        last_update_timestamp: 1_700_000_000,
    };

    let exec = PoolExecutionAccounts::MeteoraDlmm {
        reserve_x,
        reserve_y,
        oracle: pool_pk,
        bin_array_pdas: vec![Pubkey::new_from_array([30 + pool_bytes_id; 32])],
        bitmap_extension: None,
        pool_token_x: mint_x.0,
        pool_token_y: mint_y.0,
    };

    PoolState {
        pool_id: PoolId(pool_pk),
        protocol: DexProtocol::MeteoraDlmm,
        slot: Slot(467_000_000),
        write_version: 1,
        base_mint: mint_x,
        quote_mint: mint_y,
        decimals_a: 9,
        decimals_b: 6,
        reserve_in: (liquidity_x as u128) * 3,
        reserve_out: (liquidity_y as u128) * 3,
        fee_numerator: ((bin_step as u64) * (base_factor as u64) / 100).max(1),
        fee_denominator: 10_000,
        is_active: true,
        has_risky_token: false,
        kind: PoolKind::Dlmm,
        clmm: None,
        dlmm: Some(dlmm_state),
        pump_fun: None,
        transfer_fee_bps: 0,
        token_programs: [TokenProgramKind::SplToken, TokenProgramKind::SplToken],
        execution_accounts: Some(exec),
        last_update_slot: 467_000_000,
        last_update_ms: 1_700_000_000_000,
    }
}

#[test]
fn test_meteora_pair_discovery_canonicalization_and_deduplication() {
    let pool1 = make_test_dlmm_pool(1, 0, 10, 100, 1, 2, 50_000_000_000, 50_000_000_000);
    // Reversed token ordering: base=2, quote=1
    let pool2 = make_test_dlmm_pool(2, 50, 10, 100, 2, 1, 50_000_000_000, 50_000_000_000);

    let pools = vec![pool1, pool2];
    let report = MeteoraDlmmPoolPairDiscovery::discover_pairs(&pools);

    assert_eq!(report.total_meteora_pools_discovered, 2);
    assert_eq!(report.active_and_valid_pools, 2);
    assert_eq!(
        report.unique_token_pairs, 1,
        "Should canonicalize into 1 unique token pair"
    );
    assert_eq!(
        report.distinct_eligible_pool_pairs, 1,
        "Should find exactly 1 eligible pool pair"
    );

    let pair = &report.eligible_pairs[0];
    assert_ne!(
        pair.pool_a, pair.pool_b,
        "Pool A and Pool B must be distinct"
    );
}

#[test]
fn test_meteora_quoting_both_directions_identifies_profitable_edge() {
    // Create an intentional price gap between two pools on the same pair (1, 2)
    // Pool 1 active bin 0: price = 1.0
    let pool1 = make_test_dlmm_pool(1, 0, 10, 10, 1, 2, 100_000_000_000, 100_000_000_000);
    // Pool 2 active bin 200: higher price for token X in terms of token Y
    let pool2 = make_test_dlmm_pool(2, 200, 10, 10, 1, 2, 100_000_000_000, 100_000_000_000);

    let pools = vec![pool1, pool2];
    let report = MeteoraDlmmPoolPairDiscovery::discover_pairs(&pools);
    assert_eq!(report.distinct_eligible_pool_pairs, 1);
    let pair = &report.eligible_pairs[0];

    // Quote Direction A and Direction B at input size 10_000_000
    let quote_a = MeteoraDlmmCrossPoolQuoteEngine::quote_direction_a(pair, 10_000_000);
    let quote_b = MeteoraDlmmCrossPoolQuoteEngine::quote_direction_b(pair, 10_000_000);

    assert!(quote_a.is_some(), "Direction A quote must succeed");
    assert!(quote_b.is_some(), "Direction B quote must succeed");

    let qa = quote_a.unwrap();
    let qb = quote_b.unwrap();

    println!(
        "Quote Direction A: gross_profit={}, spread_bps={}",
        qa.gross_profit, qa.spread_bps
    );
    println!(
        "Quote Direction B: gross_profit={}, spread_bps={}",
        qb.gross_profit, qb.spread_bps
    );

    // One direction must capture the price gap and be distinctly more profitable than the other
    assert!(qa.gross_profit != qb.gross_profit);
    assert!(qa.gross_profit > 0 || qb.gross_profit > 0);
}

#[test]
fn test_meteora_trade_size_optimizer_finds_positive_net_profit() {
    let pool1 = make_test_dlmm_pool(1, 0, 10, 10, 1, 2, 100_000_000_000, 100_000_000_000);
    let pool2 = make_test_dlmm_pool(2, 150, 10, 10, 1, 2, 100_000_000_000, 100_000_000_000);

    let report = MeteoraDlmmPoolPairDiscovery::discover_pairs(&[pool1, pool2]);
    let pair = &report.eligible_pairs[0];

    let cfg = DlmmOptimizerConfig {
        min_input: 1_000_000,
        max_input: 50_000_000,
        coarse_samples: 10,
        max_candidate_basins: 4,
        refine_steps_per_basin: 8,
        estimated_network_cost_lamports: 15_000,
        sol_usd_price: 180,
        quote_decimals: 6,
        is_quote_sol: false,
    };

    let opt_res = MeteoraDlmmTradeSizeOptimizer::find_best_opportunity(pair, &cfg);
    assert!(opt_res.is_some(), "Optimizer should find candidate");
    let res = opt_res.unwrap();

    assert!(res.optimal_amount_in >= cfg.min_input);
    assert!(res.optimal_amount_in <= cfg.max_input);
    assert!(res.expected_gross_profit > 0);
    assert!(res.expected_net_profit > 0);
    println!(
        "Optimal Size: in={}, hop1_out={}, final_out={}, net_profit={}",
        res.optimal_amount_in,
        res.expected_hop1_out,
        res.expected_final_out,
        res.expected_net_profit
    );
}
