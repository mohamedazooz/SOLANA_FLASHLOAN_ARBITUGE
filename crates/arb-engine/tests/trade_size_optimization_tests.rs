//! Integration & Optimization Test for Section 2:
//! Real DLMM Trade Size Optimization across both directions, multi-basin coarse-to-fine search,
//! and complete accounting of Hop 1 output, Hop 2 output, fees, network costs, net profit, CUs,
//! and required Bin Arrays.

use arb_engine::{
    DlmmArbitrageDirection, DlmmOptimizerConfig, MeteoraDlmmPoolPairDiscovery,
    MeteoraDlmmTradeSizeOptimizer,
};
use arb_types::{
    DexProtocol, DlmmBin, DlmmFeeParams, DlmmState, MintId, PoolExecutionAccounts, PoolId,
    PoolKind, PoolState, Slot, TokenProgramKind,
};
use solana_pubkey::Pubkey;

fn make_test_pool(
    pool_bytes_id: u8,
    active_bin_id: i32,
    bin_step: u16,
    base_factor: u16,
    liquidity_x: u64,
    liquidity_y: u64,
) -> PoolState {
    let pool_pk = Pubkey::new_from_array([pool_bytes_id; 32]);
    let reserve_x = Pubkey::new_from_array([10 + pool_bytes_id; 32]);
    let reserve_y = Pubkey::new_from_array([20 + pool_bytes_id; 32]);
    let mint_x = MintId(Pubkey::new_from_array([100; 32])); // Base Token (e.g. WSOL, 9 dec)
    let mint_y = MintId(Pubkey::new_from_array([200; 32])); // Quote Token (e.g. USDC, 6 dec)

    // Build 5 continuous bins around active_bin_id to create realistic multi-bin liquidity
    let mut bins = Vec::new();
    for offset in -2..=2 {
        bins.push(DlmmBin {
            bin_id: active_bin_id + offset,
            amount_x: liquidity_x as u128,
            amount_y: liquidity_y as u128,
        });
    }

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
    };

    PoolState {
        pool_id: PoolId(pool_pk),
        protocol: DexProtocol::MeteoraDlmm,
        slot: Slot(467_700_000),
        write_version: 1,
        base_mint: mint_x,
        quote_mint: mint_y,
        decimals_a: 9,
        decimals_b: 6,
        reserve_in: (liquidity_x as u128) * 5,
        reserve_out: (liquidity_y as u128) * 5,
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
        last_update_slot: 467_700_000,
        last_update_ms: 1_700_000_000_000,
    }
}

#[test]
fn test_trade_size_optimization_both_directions_and_recording() {
    // Pool 1: active bin 0 (lower price for base token)
    let pool1 = make_test_pool(1, 0, 10, 10, 500_000_000, 500_000_000);
    // Pool 2: active bin 120 (higher price for base token)
    let pool2 = make_test_pool(2, 120, 10, 10, 500_000_000, 500_000_000);

    let discovery = MeteoraDlmmPoolPairDiscovery::discover_pairs(&[pool1, pool2]);
    assert_eq!(discovery.distinct_eligible_pool_pairs, 1);
    let pair = &discovery.eligible_pairs[0];

    // Range: 10,000 raw USDC (0.01 USDC) up to 20,000,000 raw USDC (20 USDC)
    let cfg = DlmmOptimizerConfig {
        min_input: 10_000,
        max_input: 20_000_000,
        coarse_samples: 16,
        max_candidate_basins: 4,
        refine_steps_per_basin: 8,
        estimated_network_cost_lamports: 9_000, // 9,000 lamports = 1,620 raw USDC @ $180/SOL
        sol_usd_price: 180,
        quote_decimals: 6,
        is_quote_sol: false,
    };

    println!("\n=========================================================================");
    println!("             METEORA TRADE SIZE OPTIMIZATION TEST SUITE                  ");
    println!("=========================================================================");

    // Optimize Direction A
    let opt_a = MeteoraDlmmTradeSizeOptimizer::optimize_direction(
        pair,
        DlmmArbitrageDirection::DirectionA,
        &cfg,
    );
    assert!(opt_a.is_some(), "Direction A optimization must succeed");
    let res_a = opt_a.unwrap();

    println!("\n--- DIRECTION A EVALUATION (Pool 1 -> Pool 2) ---");
    println!(
        "Evaluated {} distinct sizes",
        res_a.tested_evaluations.len()
    );
    assert!(!res_a.tested_evaluations.is_empty());

    // Print a sample of evaluated sizes
    for (i, eval) in res_a.tested_evaluations.iter().enumerate() {
        if i % 3 == 0 || eval.trade_size == res_a.optimal_amount_in {
            println!(
                "  Size={:>8} | Hop1={:>8} | Hop2={:>8} | Fees={:>5} | NetCost={:>5} | NetProfit={:>8} ({:>4} bps) | CU={:>6} | BinArrays={}",
                eval.trade_size,
                eval.hop1_out,
                eval.hop2_out,
                eval.total_fees_raw,
                eval.network_costs_quote_raw,
                eval.net_profit_quote_raw,
                eval.net_profit_bps,
                eval.compute_units,
                eval.total_bin_arrays,
            );
        }
    }
    println!(
        "Optimal Size A: in={}, gross={}, net={}, spread={} bps, net_bps={}, is_profitable={}",
        res_a.optimal_amount_in,
        res_a.expected_gross_profit,
        res_a.expected_net_profit,
        res_a.spread_bps,
        res_a.net_profit_bps,
        res_a.is_profitable
    );

    // Optimize Direction B
    let opt_b = MeteoraDlmmTradeSizeOptimizer::optimize_direction(
        pair,
        DlmmArbitrageDirection::DirectionB,
        &cfg,
    );
    assert!(opt_b.is_some(), "Direction B optimization must succeed");
    let res_b = opt_b.unwrap();

    println!("\n--- DIRECTION B EVALUATION (Pool 2 -> Pool 1) ---");
    println!(
        "Evaluated {} distinct sizes",
        res_b.tested_evaluations.len()
    );
    assert!(!res_b.tested_evaluations.is_empty());
    for (i, eval) in res_b.tested_evaluations.iter().enumerate() {
        if i % 3 == 0 || eval.trade_size == res_b.optimal_amount_in {
            println!(
                "  Size={:>8} | Hop1={:>8} | Hop2={:>8} | Fees={:>5} | NetCost={:>5} | NetProfit={:>8} ({:>4} bps) | CU={:>6} | BinArrays={}",
                eval.trade_size,
                eval.hop1_out,
                eval.hop2_out,
                eval.total_fees_raw,
                eval.network_costs_quote_raw,
                eval.net_profit_quote_raw,
                eval.net_profit_bps,
                eval.compute_units,
                eval.total_bin_arrays,
            );
        }
    }
    println!(
        "Optimal Size B: in={}, gross={}, net={}, spread={} bps, net_bps={}, is_profitable={}",
        res_b.optimal_amount_in,
        res_b.expected_gross_profit,
        res_b.expected_net_profit,
        res_b.spread_bps,
        res_b.net_profit_bps,
        res_b.is_profitable
    );

    // Global best opportunity selection
    let best = MeteoraDlmmTradeSizeOptimizer::find_best_opportunity(pair, &cfg).unwrap();
    println!("\n--- GLOBAL BEST FEASIBLE OPPORTUNITY ---");
    println!("Best Direction:       {:?}", best.direction);
    println!("Optimal Amount In:    {} raw units", best.optimal_amount_in);
    println!("Hop 1 Output:         {} raw units", best.expected_hop1_out);
    println!(
        "Hop 2 Output:         {} raw units",
        best.expected_final_out
    );
    println!(
        "Expected Gross Edge:  {} raw units",
        best.expected_gross_profit
    );
    println!(
        "Expected Net Profit:  {} raw units",
        best.expected_net_profit
    );
    println!("Net Profit Bps:       {} bps", best.net_profit_bps);
    println!("Compute Units:        {}", best.estimated_cu);
    println!("Required Bin Arrays:  {}", best.required_bin_arrays);
    println!("Is Net Profitable:    {}", best.is_profitable);

    // Assert that the best opportunity is the maximum of Direction A and Direction B
    assert_eq!(
        best.expected_net_profit,
        res_a.expected_net_profit.max(res_b.expected_net_profit)
    );
}

#[test]
fn test_export_phase15_2_optimizer_statistics() {
    let pool1 = make_test_pool(1, 0, 10, 10, 500_000_000, 500_000_000);
    let pool2 = make_test_pool(2, 120, 10, 10, 500_000_000, 500_000_000);
    let discovery = MeteoraDlmmPoolPairDiscovery::discover_pairs(&[pool1, pool2]);
    let pair = &discovery.eligible_pairs[0];
    let cfg = DlmmOptimizerConfig {
        min_input: 10_000,
        max_input: 20_000_000,
        coarse_samples: 16,
        max_candidate_basins: 4,
        refine_steps_per_basin: 8,
        estimated_network_cost_lamports: 9_000,
        sol_usd_price: 180,
        quote_decimals: 6,
        is_quote_sol: false,
    };

    let a = MeteoraDlmmTradeSizeOptimizer::optimize_direction(
        pair,
        DlmmArbitrageDirection::DirectionA,
        &cfg,
    )
    .expect("Direction A must produce a result");
    let b = MeteoraDlmmTradeSizeOptimizer::optimize_direction(
        pair,
        DlmmArbitrageDirection::DirectionB,
        &cfg,
    )
    .expect("Direction B must produce a result");

    let out_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("reports")
        .join("phase15_2");
    std::fs::create_dir_all(&out_dir).expect("create report dir");
    let body = serde_json::json!({
        "generated_at_utc": "2026-11-11T00:00:00Z",
        "fixture": "deterministic_two_pool_dlmm_optimizer",
        "config": cfg,
        "unimodality_assumption": false,
        "result_kind": "best_observed_candidate",
        "global_optimum_claimed": false,
        "direction_a": a,
        "direction_b": b,
    });
    let path = out_dir.join("optimizer_search_statistics.json");
    std::fs::write(&path, serde_json::to_string_pretty(&body).unwrap())
        .expect("write optimizer statistics");
    println!("wrote {}", path.display());
    assert!(path.exists());
}
