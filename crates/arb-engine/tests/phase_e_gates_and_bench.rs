//! Phase E Integration Gates & 4,096-Candidate Benchmark (`arb-engine`):
//! - Verifies Multi-Mint (`WSOL` + `USDC`) anchor caps & isolated economic configs.
//! - Verifies `PumpSwap`, `RaydiumClmm`, and `MeteoraDammV2` graph indexing when hydrated.
//! - Verifies `estimate_route_unique_accounts_v1` prunes > 64 inline account routes.
//! - Benchmarks 4,096 candidate cycle evaluations in pure integer arithmetic cleanly.

use arb_dex::token2022::{TokenSemantics, decode_clock_epoch, parse_transfer_fee_for_epoch};
use arb_engine::{MarketGraph, estimate_route_unique_accounts_v1};
use arb_math::economic::EconomicConfig;
use arb_types::{
    ClmmState, ClmmTick, ClmmTickArray, DammV2State, DexProtocol, Lamports, MintId,
    PoolExecutionAccounts, PoolId, PoolKind, PoolState, Slot, TokenAmount, TokenProgramKind,
};
use solana_pubkey::Pubkey;
use std::collections::HashMap;
use std::time::Instant;

fn econ_wsol() -> EconomicConfig {
    EconomicConfig {
        flash_loan_fee_sf: 11_529_215_046_068,
        jito_tip_lamports: Lamports(10_000),
        base_decimals: 9,
        lamports_to_base_mint_rate: 1,
        lamports_to_base_rate_denom: 1,
        min_profit_raw: TokenAmount(10_000),
        hop_slippage_bps: 10,
        priority_fee_micro_lamports: 0,
    }
}

fn econ_usdc() -> EconomicConfig {
    EconomicConfig {
        flash_loan_fee_sf: 11_529_215_046_068,
        jito_tip_lamports: Lamports(10_000),
        base_decimals: 6,
        lamports_to_base_mint_rate: 150_000_000, // $150 SOL
        lamports_to_base_rate_denom: 1_000_000_000,
        min_profit_raw: TokenAmount(1_500),
        hop_slippage_bps: 10,
        priority_fee_micro_lamports: 0,
    }
}

#[test]
fn phase_c_and_a1_multimint_and_newly_enabled_dexes_enter_market_graph() {
    let wsol = MintId(arb_constants::wsol_mint());
    let usdc = MintId(arb_constants::usdc_mint());
    let meme = MintId(Pubkey::new_unique());

    let pumpswap_pool = PoolState {
        pool_id: PoolId(Pubkey::new_unique()),
        protocol: DexProtocol::PumpSwap,
        slot: Slot(500),
        write_version: 1,
        base_mint: meme,
        quote_mint: wsol,
        decimals_a: 6,
        decimals_b: 9,
        reserve_in: 500_000_000_000,
        reserve_out: 100_000_000_000,
        fee_numerator: 2_500,
        fee_denominator: 1_000_000,
        raydium_cpmm_fee: None,
        is_active: true,
        has_risky_token: false,
        kind: PoolKind::Cpmm,
        clmm: None,
        dlmm: None,
        pump_fun: None,
        damm_v2: None,
        transfer_fee_bps: 0,
        token_programs: [TokenProgramKind::SplToken; 2],
        token2022_fee_context: None,
        execution_accounts: Some(PoolExecutionAccounts::PumpSwap {
            global_config: Pubkey::new_unique(),
            pool_base_vault: Pubkey::new_unique(),
            pool_quote_vault: Pubkey::new_unique(),
            protocol_fee_recipient: Pubkey::new_unique(),
            protocol_fee_recipient_ata: Pubkey::default(),
            coin_creator_vault_authority: Pubkey::new_unique(),
            coin_creator_vault_ata: Pubkey::default(),
            fee_config: Pubkey::new_unique(),
            pool_version: arb_types::PumpSwapPoolVersion::V2,
        }),
        last_update_slot: 500,
        last_update_ms: 1_700_000_000_000,
    };

    let raydium_clmm_pool = PoolState {
        pool_id: PoolId(Pubkey::new_unique()),
        protocol: DexProtocol::RaydiumClmm,
        slot: Slot(500),
        write_version: 1,
        base_mint: wsol,
        quote_mint: usdc,
        decimals_a: 9,
        decimals_b: 6,
        reserve_in: 1_000_000_000_000,
        reserve_out: 150_000_000_000,
        fee_numerator: 2_500,
        fee_denominator: 1_000_000,
        raydium_cpmm_fee: None,
        is_active: true,
        has_risky_token: false,
        kind: PoolKind::Clmm,
        clmm: Some(ClmmState {
            sqrt_price: 1u128 << 64,
            liquidity: 10_000_000_000,
            tick_current: 0,
            tick_spacing: 60,
            fee_rate: 2_500,
            tick_arrays: vec![ClmmTickArray {
                start_tick_index: 0,
                account_pubkey: Some(Pubkey::new_unique().to_bytes()),
                account_slot: Some(500),
                ticks: vec![ClmmTick {
                    tick_index: 0,
                    initialized: true,
                    liquidity_net: 10_000_000_000,
                    limit_order_unfilled: 0,
                }],
            }],
        }),
        dlmm: None,
        pump_fun: None,
        damm_v2: None,
        transfer_fee_bps: 0,
        token_programs: [TokenProgramKind::SplToken; 2],
        token2022_fee_context: None,
        execution_accounts: Some(PoolExecutionAccounts::RaydiumClmm {
            amm_config: Pubkey::new_unique(),
            vault_0: Pubkey::new_unique(),
            vault_1: Pubkey::new_unique(),
            observation_state: Pubkey::new_unique(),
            tick_array_pdas: vec![Pubkey::new_unique(); 3],
            bitmap_extension: None,
            pool_mint_0: wsol.0,
        }),
        last_update_slot: 500,
        last_update_ms: 1_700_000_000_000,
    };

    let damm_v2_pool = PoolState {
        pool_id: PoolId(Pubkey::new_unique()),
        protocol: DexProtocol::MeteoraDammV2,
        slot: Slot(500),
        write_version: 1,
        base_mint: meme,
        quote_mint: usdc,
        decimals_a: 6,
        decimals_b: 6,
        reserve_in: 500_000_000_000,
        reserve_out: 500_000_000_000,
        fee_numerator: 2_500_000,
        fee_denominator: 1_000_000_000,
        raydium_cpmm_fee: None,
        is_active: true,
        has_risky_token: false,
        kind: PoolKind::Cpmm,
        clmm: None,
        dlmm: None,
        pump_fun: None,
        damm_v2: Some(DammV2State {
            clock_slot: 500,
            sqrt_price_x64: 1u128 << 64,
            liquidity: 10_000_000_000,
            sqrt_min_price_x64: 1u128 << 50,
            sqrt_max_price_x64: 1u128 << 78,
            cliff_fee_numerator: 2_500_000,
            ..Default::default()
        }),
        transfer_fee_bps: 0,
        token_programs: [TokenProgramKind::SplToken; 2],
        token2022_fee_context: None,
        execution_accounts: Some(PoolExecutionAccounts::MeteoraDammV2 {
            token_a_mint: meme.0,
            token_b_mint: usdc.0,
            authority: Pubkey::new_unique(),
            vault_a: Pubkey::new_unique(),
            vault_b: Pubkey::new_unique(),
            event_authority: Pubkey::new_unique(),
        }),
        last_update_slot: 500,
        last_update_ms: 1_700_000_000_000,
    };

    let mut graph = MarketGraph::new(vec![wsol, usdc]);
    graph.set_anchor_economic_config(wsol, econ_wsol());
    graph.set_anchor_economic_config(usdc, econ_usdc());

    let stats = graph.build_from_pools(&[pumpswap_pool, raydium_clmm_pool, damm_v2_pool]);
    assert_eq!(stats.pools_indexed, 3);
    assert_eq!(stats.excluded_raydium_clmm, 0);
    assert_eq!(stats.excluded_quote_only, 0);
    assert_eq!(stats.excluded_unsupported_protocol, 0);

    let mut caps = HashMap::new();
    caps.insert(wsol, 1_000_000_000);
    caps.insert(usdc, 100_000_000);
    let (_opps, scan) = graph.find_cycles_with_anchor_caps(Slot(500), &caps, 3, 64);
    assert_eq!(scan.base_anchors, 2);
}

#[test]
fn phase_b_route_cap_excludes_four_pool_v4_cycles_before_quoting() {
    let make_v4_pool = |id: u8, from: MintId, to: MintId| PoolState {
        pool_id: PoolId::new([id; 32]),
        protocol: DexProtocol::RaydiumV4,
        slot: Slot(500),
        write_version: 1,
        base_mint: from,
        quote_mint: to,
        decimals_a: 9,
        decimals_b: 6,
        reserve_in: 1_000_000_000_000,
        reserve_out: 1_000_000_000_000,
        fee_numerator: 25,
        fee_denominator: 10_000,
        raydium_cpmm_fee: None,
        is_active: true,
        has_risky_token: false,
        kind: PoolKind::Cpmm,
        clmm: None,
        dlmm: None,
        pump_fun: None,
        damm_v2: None,
        transfer_fee_bps: 0,
        token_programs: [TokenProgramKind::SplToken; 2],
        token2022_fee_context: None,
        execution_accounts: Some(PoolExecutionAccounts::RaydiumV4 {
            amm_authority: Pubkey::new_unique(),
            open_orders: Pubkey::new_unique(),
            target_orders: Pubkey::new_unique(),
            base_vault: Pubkey::new_unique(),
            quote_vault: Pubkey::new_unique(),
            serum_program: Pubkey::new_unique(),
            serum_market: Pubkey::new_unique(),
            serum_bids: Pubkey::new_unique(),
            serum_asks: Pubkey::new_unique(),
            serum_event_queue: Pubkey::new_unique(),
            serum_coin_vault: Pubkey::new_unique(),
            serum_pc_vault: Pubkey::new_unique(),
            serum_vault_signer: Pubkey::new_unique(),
            swap_version: arb_types::RaydiumV4SwapVersion::V2,
        }),
        last_update_slot: 500,
        last_update_ms: 1_700_000_000_000,
    };

    let m0 = MintId(arb_constants::wsol_mint());
    let m1 = MintId(Pubkey::new_unique());
    let m2 = MintId(Pubkey::new_unique());
    let m3 = MintId(Pubkey::new_unique());
    let mut graph = MarketGraph::new(vec![m0]).with_economic_config(econ_wsol());
    graph.build_from_pools(&[
        make_v4_pool(1, m0, m1),
        make_v4_pool(2, m1, m2),
        make_v4_pool(3, m2, m3),
        make_v4_pool(4, m3, m0),
    ]);

    let e0 = graph
        .edges_by_from
        .get(&m0)
        .unwrap()
        .iter()
        .find(|e| e.to_mint == m1)
        .unwrap();
    let e1 = graph
        .edges_by_from
        .get(&m1)
        .unwrap()
        .iter()
        .find(|e| e.to_mint == m2)
        .unwrap();
    let e2 = graph
        .edges_by_from
        .get(&m2)
        .unwrap()
        .iter()
        .find(|e| e.to_mint == m3)
        .unwrap();
    let e3 = graph
        .edges_by_from
        .get(&m3)
        .unwrap()
        .iter()
        .find(|e| e.to_mint == m0)
        .unwrap();
    assert!(
        estimate_route_unique_accounts_v1(&[e0, e1, e2, e3]) > 64,
        "4 distinct V4 hops must exceed V1 64-account inline cap"
    );

    let (opps, scan) = graph.find_cycles_with_stats(Slot(500), 1_000_000_000, 4, 64);
    assert!(
        opps.is_empty(),
        "four-pool routes must not become opportunities"
    );
    assert_eq!(
        scan.cycles_evaluated, 0,
        "four-pool routes must be pruned before quoting"
    );
    assert_eq!(
        scan.excluded_v1_account_limit, 0,
        "four-pool routes are rejected by the route cap first"
    );
}

#[test]
fn phase_d_token2022_clock_epoch_switches_from_older_to_newer_transfer_fee() {
    let mut clock = vec![0u8; 40];
    clock[16..24].copy_from_slice(&850u64.to_le_bytes());
    assert_eq!(decode_clock_epoch(&clock), Some(850));

    let mut mint_data = vec![0u8; 166 + 4 + 108];
    mint_data[44] = 9;
    mint_data[45] = 1;
    mint_data[165] = 1; // AccountType::Mint
    mint_data[166..168].copy_from_slice(&1u16.to_le_bytes()); // TransferFeeConfig
    mint_data[168..170].copy_from_slice(&108u16.to_le_bytes());
    let p = 170;
    // older @80..90: max=1_000_000, bps=100
    mint_data[p + 80..p + 88].copy_from_slice(&1_000_000u64.to_le_bytes());
    mint_data[p + 88..p + 90].copy_from_slice(&100u16.to_le_bytes());
    // newer effective epoch = 900 @90..98, max=500_000, bps=25
    mint_data[p + 90..p + 98].copy_from_slice(&900u64.to_le_bytes());
    mint_data[p + 98..p + 106].copy_from_slice(&500_000u64.to_le_bytes());
    mint_data[p + 106..p + 108].copy_from_slice(&25u16.to_le_bytes());

    let older = parse_transfer_fee_for_epoch(&mint_data, 850).expect("older fee active at 850");
    assert_eq!(older.transfer_fee_basis_points, 100);
    assert_eq!(older.maximum_fee, 1_000_000);

    let newer = parse_transfer_fee_for_epoch(&mint_data, 900).expect("newer fee active at 900");
    assert_eq!(newer.transfer_fee_basis_points, 25);
    assert_eq!(newer.maximum_fee, 500_000);

    let sem = TokenSemantics::parse_with_epoch(Pubkey::new_unique(), &mint_data, Some(900));
    assert!(sem.is_execution_eligible);
    assert_eq!(sem.transfer_fee_config.transfer_fee_basis_points, 25);

    let fail_without_clock =
        TokenSemantics::parse_with_epoch(Pubkey::new_unique(), &mint_data, None);
    assert!(!fail_without_clock.is_execution_eligible);
}

#[test]
fn phase_e_benchmark_4096_cycle_discovery_under_budget() {
    let anchor = MintId(arb_constants::wsol_mint());
    let mut pools = Vec::new();
    for i in 0..64u16 {
        let token = MintId(Pubkey::new_unique());
        let mut p1 = PoolState {
            pool_id: PoolId(Pubkey::new_unique()),
            protocol: DexProtocol::RaydiumV4,
            slot: Slot(500),
            write_version: 1,
            base_mint: anchor,
            quote_mint: token,
            decimals_a: 9,
            decimals_b: 6,
            reserve_in: 1_000_000_000_000,
            reserve_out: 1_000_000_000_000 + u128::from(i) * 1_000_000_000,
            fee_numerator: 25,
            fee_denominator: 10_000,
            raydium_cpmm_fee: None,
            is_active: true,
            has_risky_token: false,
            kind: PoolKind::Cpmm,
            clmm: None,
            dlmm: None,
            pump_fun: None,
            damm_v2: None,
            transfer_fee_bps: 0,
            token_programs: [TokenProgramKind::SplToken; 2],
            token2022_fee_context: None,
            execution_accounts: None,
            last_update_slot: 500,
            last_update_ms: 1_700_000_000_000,
        };
        let mut p2 = p1.clone();
        p2.pool_id = PoolId(Pubkey::new_unique());
        p2.reserve_in = 1_000_000_000_000;
        p2.reserve_out = 995_000_000_000;
        p1.base_mint = anchor;
        pools.push(p1);
        pools.push(p2);
    }
    let mut graph = MarketGraph::new(vec![anchor]).with_economic_config(econ_wsol());
    graph.build_from_pools(&pools);

    let start = Instant::now();
    let (_opps, scan) = graph.find_cycles_with_stats(Slot(500), 10_000_000_000, 2, 4_096);
    let elapsed_ms = start.elapsed().as_millis();
    assert!(scan.cycles_evaluated > 0);
    // Non-flaky upper bound covering debug CI containers; logs elapsed ms.
    eprintln!(
        "phase_e_benchmark_4096_cycle_discovery: cycles={} elapsed_ms={}",
        scan.cycles_evaluated, elapsed_ms
    );
    assert!(
        elapsed_ms < 5_000,
        "4096-candidate scan exceeded debug safety budget: {elapsed_ms}ms"
    );
}
