//! Feature 5: On-Chain Multi-Leg Route Execution (`arb-executor`) — Tier 2 Boundary Tests.
//!
//! Validates hop count boundaries (0, 1, 4 rejection; 2, 3 allowed),
//! zero intermediate output rejection, and on-chain sequential solvency constraints
//! (`leg1_min_out >= leg2_in`, `leg2_min_out >= leg3_in`, `leg3_min_out >= min_final_wsol_delta`).

use arb_constants::WSOL_MINT;
use arb_executor::error::ExecutorError;
use arb_executor::instructions::execute_three_leg::{
    ExecuteThreeLegArgs, process_execute_three_leg,
};
use arb_tx::{KaminoFlashParams, TxBuilder, TxError};
use arb_types::primitives::{Bps, Lamports, MintId, PoolId, RouteId, Slot, TokenAmount};
use arb_types::{DexProtocol, ExecutionOpportunity, SwapHop};
use solana_pubkey::Pubkey;
use std::str::FromStr;

fn mock_raydium_v4_exec_accounts() -> arb_types::PoolExecutionAccounts {
    let m = arb_tx::RaydiumV4Accounts::mainnet_sol_usdc();
    arb_types::PoolExecutionAccounts::RaydiumV4 {
        amm_authority: m.amm_authority,
        open_orders: m.amm_open_orders,
        target_orders: m.amm_target_orders,
        base_vault: m.pool_coin_vault,
        quote_vault: m.pool_pc_vault,
        serum_program: m.serum_program,
        serum_market: m.serum_market,
        serum_bids: m.serum_bids,
        serum_asks: m.serum_asks,
        serum_event_queue: m.serum_event_queue,
        serum_coin_vault: m.serum_coin_vault,
        serum_pc_vault: m.serum_pc_vault,
        serum_vault_signer: m.serum_vault_signer,
        swap_version: arb_types::RaydiumV4SwapVersion::V2,
    }
}

fn create_n_hop_opportunity(hops_count: usize, size_lamports: u64) -> ExecutionOpportunity {
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    let usdc = Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
    let bonk = Pubkey::from_str("DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263").unwrap();
    let ray = Pubkey::from_str("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R").unwrap();
    let tokens = [wsol, usdc, bonk, ray, wsol];

    let mut hops = Vec::with_capacity(hops_count);
    for i in 0..hops_count {
        let mint_out = if i + 1 == hops_count {
            tokens[0]
        } else {
            tokens[(i + 1) % tokens.len()]
        };
        hops.push(SwapHop {
            pool_id: PoolId::new([(i + 1) as u8; 32]),
            protocol: DexProtocol::RaydiumV4,
            mint_in: MintId(tokens[i % tokens.len()]),
            mint_out: MintId(mint_out),
            fee_bps: 25,
            active_bin_id: None,
            token_programs: [arb_types::TokenProgramKind::SplToken; 2],
            trade_intent: Default::default(),
            execution_accounts: Some(mock_raydium_v4_exec_accounts()),
        });
    }

    ExecutionOpportunity {
        hop_slippage_bps: arb_types::DEFAULT_HOP_SLIPPAGE_BPS,
        route_id: RouteId(777),
        slot: Slot(2000),
        size_in: TokenAmount(size_lamports),
        expected_out: TokenAmount(size_lamports + 50_000_000),
        hop_inputs: {
            let mut v = Vec::with_capacity(hops_count + 1);
            v.push(size_lamports);
            for i in 1..=hops_count {
                v.push(size_lamports + (50_000_000u64 * i as u64) / hops_count as u64);
            }
            v
        },
        gross_profit: Lamports(50_000_000),
        net_profit: Lamports(45_000_000),
        base_decimals: 9,
        price_impact_bps: Bps(5),
        hops,
        expected_min_final_output: 0,
    }
}

/// Boundary Case 1: Hop count = 0 is rejected with InvalidHopCount(0).
#[test]
fn test_boundary_f5_hop_count_zero_rejection() {
    let opp = create_n_hop_opportunity(0, 1_000_000_000);
    let res = TxBuilder::assemble_atomic_bundle(
        Pubkey::new_unique(),
        Pubkey::new_unique(),
        Lamports(50_000),
        1_000,
        &opp,
        &KaminoFlashParams::default(),
        11_529_215_046_068,
        TokenAmount(30_000),
    );
    match res {
        Err(TxError::InvalidHopCount(0)) => (),
        other => panic!("expected InvalidHopCount(0), got {:?}", other),
    }
}

/// Boundary Case 2: Hop count = 1 (single-leg cycle) is rejected with InvalidHopCount(1).
#[test]
fn test_boundary_f5_hop_count_one_rejection() {
    let opp = create_n_hop_opportunity(1, 1_000_000_000);
    let res = TxBuilder::assemble_atomic_bundle(
        Pubkey::new_unique(),
        Pubkey::new_unique(),
        Lamports(50_000),
        1_000,
        &opp,
        &KaminoFlashParams::default(),
        11_529_215_046_068,
        TokenAmount(30_000),
    );
    match res {
        Err(TxError::InvalidHopCount(1)) => (),
        other => panic!("expected InvalidHopCount(1), got {:?}", other),
    }
}

/// Boundary Case 3: Hop count = 4 (exceeding max 3 hops for standard bundle) is rejected with InvalidHopCount(4).
#[test]
fn test_boundary_f5_hop_count_four_boundary() {
    let opp = create_n_hop_opportunity(4, 1_000_000_000);
    let res = TxBuilder::assemble_atomic_bundle(
        Pubkey::new_unique(),
        Pubkey::new_unique(),
        Lamports(50_000),
        1_000,
        &opp,
        &KaminoFlashParams::default(),
        11_529_215_046_068,
        TokenAmount(30_000),
    );
    match res {
        Err(TxError::InvalidHopCount(4)) => (),
        other => panic!("expected InvalidHopCount(4), got {:?}", other),
    }
}

/// Boundary Case 4: Zero intermediate hop output is rejected in hop limit calculation.
#[test]
fn test_boundary_f5_zero_intermediate_output_rejection() {
    let mut opp = create_n_hop_opportunity(2, 1_000_000_000);
    // Force hop 0 to quote 0 output
    opp.hop_inputs[1] = 0;

    let res = TxBuilder::derive_swap_legs(&opp, 11_529_215_046_068, 10_000, 30_000, 25);
    match res {
        Err(TxError::InvalidInput(msg)) => {
            assert!(
                msg.contains("hop 0 quotes zero output"),
                "unexpected msg: {msg}"
            );
        }
        other => panic!(
            "expected InvalidInput('hop 0 quotes zero output'), got {:?}",
            other
        ),
    }
}

/// Boundary Case 5: On-chain CPI process_execute_three_leg solvency violation boundary checks.
#[test]
fn test_boundary_f5_cpi_intermediate_underfunded_boundary() {
    // Sub-case A: leg1_min_amount_out < leg2_amount_in fails with Leg2InputExceedsLeg1Output
    let args_leg2_fail = ExecuteThreeLegArgs {
        leg1_amount_in: 1_000_000_000,
        leg1_min_amount_out: 100_000_000,
        leg2_amount_in: 100_000_001, // 1 lamport higher
        leg2_min_amount_out: 50_000_000,
        leg3_amount_in: 50_000_000,
        leg3_min_amount_out: 1_050_000_000,
        min_final_wsol_delta: 10_000_000,
        leg1_bin_array_count: 1,
        leg2_bin_array_count: 1,
        leg3_bin_array_count: 1,
    };
    assert_eq!(
        process_execute_three_leg(&[], args_leg2_fail),
        Err(ExecutorError::Leg2InputExceedsLeg1Output.into())
    );

    // Sub-case B: leg2_min_amount_out < leg3_amount_in fails with Leg3InputExceedsLeg2Output
    let args_leg3_fail = ExecuteThreeLegArgs {
        leg1_amount_in: 1_000_000_000,
        leg1_min_amount_out: 100_000_000,
        leg2_amount_in: 100_000_000,
        leg2_min_amount_out: 50_000_000,
        leg3_amount_in: 50_000_001, // 1 lamport higher
        leg3_min_amount_out: 1_050_000_000,
        min_final_wsol_delta: 10_000_000,
        leg1_bin_array_count: 1,
        leg2_bin_array_count: 1,
        leg3_bin_array_count: 1,
    };
    assert_eq!(
        process_execute_three_leg(&[], args_leg3_fail),
        Err(ExecutorError::Leg3InputExceedsLeg2Output.into())
    );

    // Sub-case C: leg3_min_amount_out < min_final_wsol_delta fails with FinalWsolDeltaBelowMinimum
    let args_final_fail = ExecuteThreeLegArgs {
        leg1_amount_in: 1_000_000_000,
        leg1_min_amount_out: 100_000_000,
        leg2_amount_in: 100_000_000,
        leg2_min_amount_out: 50_000_000,
        leg3_amount_in: 50_000_000,
        leg3_min_amount_out: 9_999_999,
        min_final_wsol_delta: 10_000_000, // min delta not met
        leg1_bin_array_count: 1,
        leg2_bin_array_count: 1,
        leg3_bin_array_count: 1,
    };
    assert_eq!(
        process_execute_three_leg(&[], args_final_fail),
        Err(ExecutorError::FinalWsolDeltaBelowMinimum.into())
    );
}

/// Boundary Case 6: `build_arb_executor_route_instruction` packing and discriminators.
#[test]
fn test_build_arb_executor_route_instruction_packing() {
    use arb_tx::{
        ARB_EXECUTOR_PROGRAM_ID_STR, AccountMeta, DISCRIMINATOR_EXECUTE_ROUTE, RouteHopParams,
        build_arb_executor_route_instruction,
    };
    use std::str::FromStr;

    let payer = Pubkey::new_unique();
    let settlement = Pubkey::new_unique();
    let intermediate = Pubkey::new_unique();

    let hop0 = RouteHopParams {
        protocol: 1, // RaydiumCPMM
        flags: 0,
        amount_in: 1_000_000_000,
        min_amount_out: 100_000_000,
        hop_accounts: vec![
            AccountMeta::new(Pubkey::new_unique(), false),
            AccountMeta::new(Pubkey::new_unique(), false),
        ],
    };
    let hop1 = RouteHopParams {
        protocol: 0, // MeteoraDLMM
        flags: 0,
        amount_in: 100_000_000,
        min_amount_out: 1_050_000_000,
        hop_accounts: vec![
            AccountMeta::new(Pubkey::new_unique(), false),
            AccountMeta::new(Pubkey::new_unique(), false),
            AccountMeta::new(Pubkey::new_unique(), false),
        ],
    };

    let ix = build_arb_executor_route_instruction(
        payer,
        settlement,
        &[intermediate],
        &[hop0, hop1],
        50_000_000,
    );

    assert_eq!(
        ix.program_id,
        Pubkey::from_str(ARB_EXECUTOR_PROGRAM_ID_STR).unwrap()
    );
    // Header accounts: payer (signer, writable) + settlement (writable) + intermediate (writable) = 3
    // Hop accounts: 2 + 3 = 5
    // Total accounts = 8
    assert_eq!(ix.accounts.len(), 8);
    assert_eq!(ix.accounts[0].pubkey, payer);
    assert!(ix.accounts[0].is_signer);
    assert!(ix.accounts[0].is_writable);
    assert_eq!(ix.accounts[1].pubkey, settlement);
    assert!(!ix.accounts[1].is_signer);
    assert!(ix.accounts[1].is_writable);
    assert_eq!(ix.accounts[2].pubkey, intermediate);

    // Data verification
    assert_eq!(ix.data[0], DISCRIMINATOR_EXECUTE_ROUTE);
    assert_eq!(ix.data[1], 2); // 2 hops
    let min_final_delta = u64::from_le_bytes(ix.data[9..17].try_into().unwrap());
    assert_eq!(min_final_delta, 50_000_000);
}

/// Boundary Case 7: Full `assemble_flash_funded_route_cpi` generates atomic bundle with profit guard.
#[test]
fn test_assemble_flash_funded_route_cpi_generates_atomic_bundle() {
    use arb_tx::{
        AccountMeta, FlashRouteCpiParams, KaminoFlashParams, RouteHopParams,
        assemble_flash_funded_route_cpi,
    };

    let payer = Pubkey::new_unique();
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    let token_a = Pubkey::new_unique();
    let spl_token = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap();

    let hop0 = RouteHopParams {
        protocol: 1, // RaydiumCPMM
        flags: 0,
        amount_in: 1_000_000_000,
        min_amount_out: 100_000_000,
        hop_accounts: vec![
            AccountMeta::new(Pubkey::new_unique(), false),
            AccountMeta::new(Pubkey::new_unique(), false),
        ],
    };
    let hop1 = RouteHopParams {
        protocol: 0, // MeteoraDLMM
        flags: 0,
        amount_in: 100_000_000,
        min_amount_out: 1_050_000_000,
        hop_accounts: vec![
            AccountMeta::new(Pubkey::new_unique(), false),
            AccountMeta::new(Pubkey::new_unique(), false),
        ],
    };

    let params = FlashRouteCpiParams {
        payer,
        kamino: KaminoFlashParams::default(),
        borrow_amount: 1_000_000_000,
        flash_fee: 100_000,
        settlement_mint: wsol,
        settlement_program: spl_token,
        intermediate_tokens: vec![(token_a, spl_token)],
        hops: vec![hop0, hop1],
        min_final_delta: 50_000_000,
        min_net_profit_lamports: 10_000_000,
        jito_tip_account: Pubkey::new_unique(),
        jito_tip_lamports: 50_000,
        cu_limit: 300_000,
        priority_fee_micro_lamports: 1_000,
    };

    let ixs = assemble_flash_funded_route_cpi(&params).expect("assembly succeeds");

    // Must have profit guard instruction right before repay
    let executor_prog = Pubkey::from_str(arb_tx::ARB_EXECUTOR_PROGRAM_ID_STR).unwrap();
    let guard_count = ixs
        .iter()
        .filter(|ix| {
            ix.program_id == executor_prog && ix.data[0] == arb_tx::DISCRIMINATOR_PROFIT_GUARD
        })
        .count();
    assert_eq!(guard_count, 1, "exactly one profit guard must be present");
}
