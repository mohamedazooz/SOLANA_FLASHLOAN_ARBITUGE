//! Comprehensive 3-Pool Routing and Kamino Flash Loan Optimization Tests.

use arb_dex::token2022::TransferFeeConfig;
use arb_engine::three_pool_router::{
    BASE_NETWORK_FEE_LAMPORTS, DEFAULT_MIN_NET_PROFIT_LAMPORTS, KaminoBorrowOptimizer,
    RouteHopSpec, SOLANA_PACKET_MTU_BYTES, ThreePoolRouteEngine,
};
use arb_types::{DexProtocol, MintId, PoolId, PoolKind, PoolState, Slot, TokenProgramKind};
use solana_pubkey::Pubkey;

fn make_cpmm_pool(
    pool_id: Pubkey,
    mint_in: Pubkey,
    mint_out: Pubkey,
    reserve_in: u64,
    reserve_out: u64,
    trade_fee_rate: u64,
) -> PoolState {
    PoolState {
        pool_id: PoolId::from(pool_id),
        protocol: DexProtocol::RaydiumCpmm,
        slot: Slot(100),
        write_version: 1,
        base_mint: MintId::from(mint_in),
        quote_mint: MintId::from(mint_out),
        decimals_a: 9,
        decimals_b: 6,
        reserve_in: reserve_in as u128,
        reserve_out: reserve_out as u128,
        fee_numerator: trade_fee_rate,
        fee_denominator: 1_000_000,
        is_active: true,
        has_risky_token: false,
        kind: PoolKind::Cpmm,
        clmm: None,
        dlmm: None,
        pump_fun: None,
        transfer_fee_bps: 0,
        token_programs: [TokenProgramKind::SplToken, TokenProgramKind::SplToken],
        execution_accounts: None,
        last_update_slot: 100,
        last_update_ms: 1000,
    }
}

#[test]
fn test_three_pool_route_sequential_chaining() {
    let engine = ThreePoolRouteEngine::new();

    let mint_sol = arb_constants::wsol_mint();
    let mint_usdc = Pubkey::new_unique();
    let mint_bonk = Pubkey::new_unique();

    // Pool 1: SOL -> USDC (CPMM)
    // 100 SOL (100e9 lamports), 15,000 USDC (15,000e6)
    let pool_1 = make_cpmm_pool(
        Pubkey::new_unique(),
        mint_sol,
        mint_usdc,
        100_000_000_000,
        15_000_000_000,
        2500, // 25 bps (0.25%)
    );

    // Pool 2: USDC -> BONK (CPMM)
    // 50,000 USDC (50,000e6), 2,000,000,000 BONK
    let pool_2 = make_cpmm_pool(
        Pubkey::new_unique(),
        mint_usdc,
        mint_bonk,
        50_000_000_000,
        2_000_000_000_000,
        2500,
    );

    // Pool 3: BONK -> SOL (CPMM)
    // Favorable rate: 2,000,000,000 BONK, 120 SOL
    let pool_3 = make_cpmm_pool(
        Pubkey::new_unique(),
        mint_bonk,
        mint_sol,
        2_000_000_000_000,
        120_000_000_000,
        2500,
    );

    let hops = vec![
        RouteHopSpec {
            pool_state: pool_1,
            input_mint: MintId::from(mint_sol),
            output_mint: MintId::from(mint_usdc),
            input_transfer_fee: None,
            output_transfer_fee: None,
        },
        RouteHopSpec {
            pool_state: pool_2,
            input_mint: MintId::from(mint_usdc),
            output_mint: MintId::from(mint_bonk),
            input_transfer_fee: None,
            output_transfer_fee: None,
        },
        RouteHopSpec {
            pool_state: pool_3,
            input_mint: MintId::from(mint_bonk),
            output_mint: MintId::from(mint_sol),
            input_transfer_fee: None,
            output_transfer_fee: None,
        },
    ];

    let borrow_amount = 1_000_000_000u64; // 1 SOL
    let flash_fee_bps = 5; // 5 bps (0.05%)
    let priority_fee = 10_000;
    let tip_share_bps = 5_000; // 50%

    let quote = engine
        .simulate_route(
            &hops,
            borrow_amount,
            flash_fee_bps,
            priority_fee,
            tip_share_bps,
        )
        .expect("Simulation must succeed");

    assert_eq!(quote.hops.len(), 3);
    assert_eq!(quote.principal_borrowed, borrow_amount);

    // Verify exact sequential chaining: output of hop i feeds hop i+1
    assert_eq!(quote.hops[0].amount_out_raw, quote.hops[1].amount_in_raw);
    assert_eq!(quote.hops[1].amount_out_raw, quote.hops[2].amount_in_raw);
    assert_eq!(quote.hops[2].amount_out_raw, quote.gross_output);

    // Verify Kamino flash loan fee: ceil(1e9 * 5 / 10000) = 500,000 lamports
    assert_eq!(quote.flash_loan_fee, 500_000);
    assert_eq!(quote.total_repay_required, borrow_amount + 500_000);
    assert_eq!(
        quote.gross_profit,
        quote.gross_output as i64 - quote.total_repay_required as i64
    );
    assert_eq!(
        quote.total_costs,
        quote.network_fee + quote.priority_fee + quote.jito_tip
    );
    assert_eq!(
        quote.net_profit_lamports,
        quote.gross_profit - quote.total_costs as i64
    );

    // Verify network fee & size constraints
    assert_eq!(quote.network_fee, BASE_NETWORK_FEE_LAMPORTS);
    assert!(quote.estimated_tx_size_bytes <= SOLANA_PACKET_MTU_BYTES);

    // Compute units for 3 CPMM hops: 450 + 28,000 + 3*35,000 = 133,450
    assert_eq!(quote.estimated_compute_units, 133_450);
}

#[test]
fn test_three_pool_route_token2022_transfer_fees() {
    let engine = ThreePoolRouteEngine::new();

    let mint_sol = arb_constants::wsol_mint();
    let mint_token2022 = Pubkey::new_unique();
    let mint_c = Pubkey::new_unique();

    let pool_1 = make_cpmm_pool(
        Pubkey::new_unique(),
        mint_sol,
        mint_token2022,
        100_000_000_000,
        100_000_000_000,
        0,
    );

    let pool_2 = make_cpmm_pool(
        Pubkey::new_unique(),
        mint_token2022,
        mint_c,
        100_000_000_000,
        100_000_000_000,
        0,
    );

    let pool_3 = make_cpmm_pool(
        Pubkey::new_unique(),
        mint_c,
        mint_sol,
        100_000_000_000,
        100_000_000_000,
        0,
    );

    // 100 bps transfer fee on token2022
    let fee_cfg = TransferFeeConfig {
        transfer_fee_basis_points: 100, // 1%
        maximum_fee: 1_000_000_000,
    };

    let hops = vec![
        RouteHopSpec {
            pool_state: pool_1,
            input_mint: MintId::from(mint_sol),
            output_mint: MintId::from(mint_token2022),
            input_transfer_fee: None,
            output_transfer_fee: Some(fee_cfg),
        },
        RouteHopSpec {
            pool_state: pool_2,
            input_mint: MintId::from(mint_token2022),
            output_mint: MintId::from(mint_c),
            input_transfer_fee: Some(fee_cfg),
            output_transfer_fee: None,
        },
        RouteHopSpec {
            pool_state: pool_3,
            input_mint: MintId::from(mint_c),
            output_mint: MintId::from(mint_sol),
            input_transfer_fee: None,
            output_transfer_fee: None,
        },
    ];

    let quote = engine
        .simulate_route(&hops, 1_000_000_000, 0, 5_000, 5_000)
        .expect("Quote simulation must succeed");

    // Check that transfer fees were accounted for
    assert!(
        quote
            .transfer_fees_by_hop
            .iter()
            .any(|fee| { fee.input_transfer_fee_raw > 0 || fee.output_transfer_fee_raw > 0 })
    );
    assert!(quote.hops[0].transfer_fee_breakdown.output_transfer_fee > 0);
    assert!(quote.hops[1].transfer_fee_breakdown.input_transfer_fee > 0);
}

#[test]
fn test_kamino_borrow_optimizer_finds_profitable_size() {
    let optimizer = KaminoBorrowOptimizer::new();

    let mint_sol = arb_constants::wsol_mint();
    let mint_usdc = Pubkey::new_unique();
    let mint_bonk = Pubkey::new_unique();

    // Pool 1: SOL -> USDC
    let pool_1 = make_cpmm_pool(
        Pubkey::new_unique(),
        mint_sol,
        mint_usdc,
        1_000_000_000_000, // 1000 SOL
        150_000_000_000,   // 150,000 USDC
        2500,
    );

    // Pool 2: USDC -> BONK
    let pool_2 = make_cpmm_pool(
        Pubkey::new_unique(),
        mint_usdc,
        mint_bonk,
        150_000_000_000, // 150,000 USDC
        150_000_000_000, // 150,000 BONK
        2500,
    );

    // Pool 3: BONK -> SOL (arbitrage profit: 100k BONK backed by 1200 SOL)
    let pool_3 = make_cpmm_pool(
        Pubkey::new_unique(),
        mint_bonk,
        mint_sol,
        100_000_000_000,   // 100,000 BONK
        1_200_000_000_000, // 1200 SOL
        2500,
    );

    let hops = vec![
        RouteHopSpec {
            pool_state: pool_1,
            input_mint: MintId::from(mint_sol),
            output_mint: MintId::from(mint_usdc),
            input_transfer_fee: None,
            output_transfer_fee: None,
        },
        RouteHopSpec {
            pool_state: pool_2,
            input_mint: MintId::from(mint_usdc),
            output_mint: MintId::from(mint_bonk),
            input_transfer_fee: None,
            output_transfer_fee: None,
        },
        RouteHopSpec {
            pool_state: pool_3,
            input_mint: MintId::from(mint_bonk),
            output_mint: MintId::from(mint_sol),
            input_transfer_fee: None,
            output_transfer_fee: None,
        },
    ];

    let kamino_liquidity = 50_000_000_000u64; // 50 SOL available
    let opt_quote = optimizer
        .optimize_borrow_size(&hops, kamino_liquidity, 5, 10_000, 5_000)
        .expect("Optimizer should find a profitable borrow size");

    assert!(opt_quote.is_profitable);
    assert!(opt_quote.net_profit_lamports >= DEFAULT_MIN_NET_PROFIT_LAMPORTS as i64);
    assert!(opt_quote.principal_borrowed >= 10_000_000); // >= 0.01 SOL
    assert!(opt_quote.principal_borrowed <= kamino_liquidity);
    assert!(opt_quote.gross_output > opt_quote.total_repay_required);
}

#[test]
fn test_three_pool_route_fail_closed_validation() {
    let engine = ThreePoolRouteEngine::new();

    let mint_a = Pubkey::new_unique();
    let mint_b = Pubkey::new_unique();
    let mint_c = Pubkey::new_unique();
    let mint_d = Pubkey::new_unique();

    let pool_1 = make_cpmm_pool(Pubkey::new_unique(), mint_a, mint_b, 100_000, 100_000, 0);
    let pool_2 = make_cpmm_pool(Pubkey::new_unique(), mint_b, mint_c, 100_000, 100_000, 0);
    let pool_broken = make_cpmm_pool(Pubkey::new_unique(), mint_d, mint_a, 100_000, 100_000, 0);

    // Case 1: Broken intermediate mint continuity (hop 2 out = mint_c, hop 3 in = mint_d)
    let broken_hops = vec![
        RouteHopSpec {
            pool_state: pool_1.clone(),
            input_mint: MintId::from(mint_a),
            output_mint: MintId::from(mint_b),
            input_transfer_fee: None,
            output_transfer_fee: None,
        },
        RouteHopSpec {
            pool_state: pool_2.clone(),
            input_mint: MintId::from(mint_b),
            output_mint: MintId::from(mint_c),
            input_transfer_fee: None,
            output_transfer_fee: None,
        },
        RouteHopSpec {
            pool_state: pool_broken,
            input_mint: MintId::from(mint_d),
            output_mint: MintId::from(mint_a),
            input_transfer_fee: None,
            output_transfer_fee: None,
        },
    ];

    let res = engine.simulate_route(&broken_hops, 1000, 0, 0, 0);
    assert!(res.is_err());

    // Case 2: Non-cyclical route (final hop output != initial borrow)
    let non_cyclical_hops = vec![
        RouteHopSpec {
            pool_state: pool_1,
            input_mint: MintId::from(mint_a),
            output_mint: MintId::from(mint_b),
            input_transfer_fee: None,
            output_transfer_fee: None,
        },
        RouteHopSpec {
            pool_state: pool_2,
            input_mint: MintId::from(mint_b),
            output_mint: MintId::from(mint_c),
            input_transfer_fee: None,
            output_transfer_fee: None,
        },
    ];
    let res2 = engine.simulate_route(&non_cyclical_hops, 1000, 0, 0, 0);
    assert!(res2.is_err());
}
