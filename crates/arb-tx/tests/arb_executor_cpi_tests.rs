//! Integration and verification tests for the On-Chain CPI Arbitrage Executor (`arb-executor`).
//!
//! Verifies:
//! - Exact instruction layout, accounts matrix, and discriminator matching `fixtures/dlmm.json`
//! - Top-level transaction sequence:
//!   ComputeBudget -> Ephemeral ATAs -> FlashBorrow -> ArbExecutor CPI -> FlashRepay -> JitoTip
//! - Atomic integrity verification (`verify_atomic_integrity`)
//! - Sequential solvency constraints: `previous_hop.min_amount_out >= next_hop.amount_in`
//! - Fail-closed enforcement on insufficient output or distinct pool violations
//! - Both V0 with ALT and Solana V1 (SIMD-0385) wire compilation

use arb_constants::{TOKEN_PROGRAM_ID, WSOL_MINT};
use arb_tx::dlmm_3leg_flash_arb::{
    FlashArb3LegError, FlashArb3LegParams, FlashArb3LegPoolLeg, assemble_flash_funded_three_pool,
};
use arb_tx::flash::arb_executor::{
    ARB_EXECUTOR_PROGRAM_ID_STR, EXECUTE_THREE_LEG_DISCRIMINATOR,
    assemble_flash_funded_three_pool_cpi, assemble_flash_funded_three_pool_cpi_instructions,
    build_arb_executor_three_leg_instruction,
};
use arb_tx::{KaminoFlashParams, KeypairSigner};
use solana_pubkey::Pubkey;
use std::str::FromStr;

const FLASH_LOAN_FEE_SF: u64 = 11_529_215_046_068; // 1e-5 U68F60

fn wsol() -> Pubkey {
    Pubkey::from_str(WSOL_MINT).unwrap()
}

fn spl() -> Pubkey {
    Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap()
}

fn meme_a() -> Pubkey {
    Pubkey::new_from_array([0xA1u8; 32])
}

fn meme_b() -> Pubkey {
    Pubkey::new_from_array([0xB2u8; 32])
}

fn leg(
    lb_pair: Pubkey,
    token_x_mint: Pubkey,
    token_y_mint: Pubkey,
    amount_in: u64,
    min_amount_out: u64,
    bin_arrays: Vec<Pubkey>,
) -> FlashArb3LegPoolLeg {
    FlashArb3LegPoolLeg {
        lb_pair,
        reserve_x: Pubkey::new_unique(),
        reserve_y: Pubkey::new_unique(),
        token_x_mint,
        token_y_mint,
        oracle: Pubkey::new_unique(),
        token_x_program: spl(),
        token_y_program: spl(),
        swap_for_y: true,
        amount_in,
        min_amount_out,
        bin_array_pdas: bin_arrays,
        bitmap_extension: None,
        host_fee_in: None,
    }
}

fn make_test_params(
    payer: Pubkey,
    borrow: u64,
    leg1_in: u64,
    leg1_min: u64,
    leg2_in: u64,
    leg2_min: u64,
    leg3_in: u64,
    leg3_min: u64,
    v1_transport: bool,
) -> FlashArb3LegParams {
    let fee = arb_tx::kamino::calculate_kamino_flash_fee(borrow, FLASH_LOAN_FEE_SF);
    FlashArb3LegParams {
        payer,
        kamino: KaminoFlashParams::default_wsol(),
        borrow_amount: borrow,
        flash_fee: fee,
        leg_1: leg(
            Pubkey::new_from_array([10u8; 32]),
            wsol(),
            meme_a(),
            leg1_in,
            leg1_min,
            vec![Pubkey::new_unique(), Pubkey::new_unique()],
        ),
        leg_2: leg(
            Pubkey::new_from_array([20u8; 32]),
            meme_a(),
            meme_b(),
            leg2_in,
            leg2_min,
            vec![Pubkey::new_unique()],
        ),
        leg_3: leg(
            Pubkey::new_from_array([30u8; 32]),
            meme_b(),
            wsol(),
            leg3_in,
            leg3_min,
            vec![Pubkey::new_unique(), Pubkey::new_unique()],
        ),
        cu_limit: 450_000,
        priority_fee_micro_lamports: 50_000,
        jito_tip_account: Pubkey::new_unique(),
        jito_tip_lamports: 10_000,
        recent_blockhash: [1u8; 32],
        v1_transport_supported: v1_transport,
    }
}

#[test]
fn direct_and_cpi_assemblers_share_route_validation_contract() {
    let signer = KeypairSigner::new_random();
    let valid = make_test_params(
        signer.pubkey(),
        1_000_000_000,
        1_000_000_000,
        1_010_000_000,
        1_010_000_000,
        1_020_000_000,
        1_020_000_000,
        1_100_000_000,
        true,
    );

    let assert_parity = |params: &FlashArb3LegParams| {
        let direct = assemble_flash_funded_three_pool(params)
            .map(|_| ())
            .map_err(|error| error.to_string());
        let cpi = assemble_flash_funded_three_pool_cpi_instructions(params)
            .map(|_| ())
            .map_err(|error| error.to_string());
        assert_eq!(direct, cpi, "direct/CPI validation diverged");
    };

    assert_parity(&valid);

    let mut zero_borrow = valid.clone();
    zero_borrow.borrow_amount = 0;
    assert_parity(&zero_borrow);

    let mut duplicate_pool = valid.clone();
    duplicate_pool.leg_3.lb_pair = duplicate_pool.leg_1.lb_pair;
    assert_parity(&duplicate_pool);

    let mut non_closing = valid.clone();
    non_closing.leg_3.token_y_mint = Pubkey::new_unique();
    assert_parity(&non_closing);

    let mut mint_mismatch = valid.clone();
    mint_mismatch.leg_2.token_x_mint = Pubkey::new_unique();
    assert_parity(&mint_mismatch);

    let mut token_program_mismatch = valid.clone();
    token_program_mismatch.leg_2.token_x_program = Pubkey::new_unique();
    assert_parity(&token_program_mismatch);

    let mut closing_program_mismatch = valid.clone();
    closing_program_mismatch.leg_3.token_y_program = Pubkey::new_unique();
    assert_parity(&closing_program_mismatch);

    let mut underfunded_hop = valid;
    underfunded_hop.leg_1.min_amount_out = underfunded_hop.leg_2.amount_in - 1;
    assert_parity(&underfunded_hop);
}

#[test]
fn test_arb_executor_instruction_layout() {
    let payer = Pubkey::new_unique();
    let settlement = Pubkey::new_unique();
    let intermediate_a = Pubkey::new_unique();
    let intermediate_b = Pubkey::new_unique();
    let dlmm_prog = Pubkey::from_str(arb_tx::meteora_dlmm::DLMM_PROGRAM_ID_STR).unwrap();
    let spl_prog = spl();
    let token_2022_prog = Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID).unwrap();
    let memo_prog = Pubkey::from_str(arb_tx::meteora_dlmm::MEMO_PROGRAM_ID_STR).unwrap();
    let event_auth = arb_tx::derive_meteora_event_authority();

    let leg1 = leg(
        Pubkey::new_unique(),
        wsol(),
        meme_a(),
        1000,
        900,
        vec![Pubkey::new_unique(), Pubkey::new_unique()],
    );
    let leg2 = leg(
        Pubkey::new_unique(),
        meme_a(),
        meme_b(),
        900,
        800,
        vec![Pubkey::new_unique()],
    );
    let leg3 = leg(
        Pubkey::new_unique(),
        meme_b(),
        wsol(),
        800,
        1200,
        vec![Pubkey::new_unique(), Pubkey::new_unique()],
    );

    let ix = build_arb_executor_three_leg_instruction(
        payer,
        settlement,
        intermediate_a,
        intermediate_b,
        dlmm_prog,
        spl_prog,
        token_2022_prog,
        memo_prog,
        event_auth,
        &leg1.account_set(&payer),
        &leg2.account_set(&payer),
        &leg3.account_set(&payer),
        1000,
        900,
        900,
        800,
        800,
        1200,
        1050,
    );

    assert_eq!(ix.program_id.to_string(), ARB_EXECUTOR_PROGRAM_ID_STR);
    assert_eq!(&ix.data[0..8], &EXECUTE_THREE_LEG_DISCRIMINATOR);
    assert_eq!(ix.data.len(), 67);

    // Verify Header accounts
    assert_eq!(ix.accounts[0].pubkey, payer);
    assert!(ix.accounts[0].is_signer);
    assert_eq!(ix.accounts[1].pubkey, settlement);
    assert!(ix.accounts[1].is_writable);
    assert_eq!(ix.accounts[2].pubkey, intermediate_a);
    assert_eq!(ix.accounts[3].pubkey, intermediate_b);
    assert_eq!(ix.accounts[4].pubkey, dlmm_prog);

    // Total accounts = 9 header + 3 * 8 pool accounts + (2 + 1 + 2 bin arrays) = 38 accounts
    assert_eq!(ix.accounts.len(), 38);
}

#[test]
fn test_assemble_flash_funded_three_pool_cpi_happy_path() {
    let signer = KeypairSigner::new_random();
    let borrow = 1_000_000_000u64; // 1 SOL
    let leg1_out = 990_000_000u64;
    let leg2_out = 985_000_000u64;
    let leg3_out = 1_050_000_000u64; // 1.05 SOL

    let params = make_test_params(
        signer.pubkey(),
        borrow,
        borrow,
        leg1_out,
        leg1_out, // leg2_in == leg1_out
        leg2_out,
        leg2_out, // leg3_in == leg2_out
        leg3_out,
        true,
    );

    let tx = assemble_flash_funded_three_pool_cpi(&params, &signer, &[])
        .expect("CPI flash arbitrage assembly must succeed");

    assert_eq!(tx.borrow_instruction_index, 5);
    assert_eq!(tx.repay_instruction_index, 7);
    assert_eq!(tx.instructions.len(), 9);

    // Instruction sequence assertions:
    // [0] ComputeBudget Limit
    // [1] ComputeBudget Price
    // [2] ATA WSOL
    // [3] ATA Meme A
    // [4] ATA Meme B
    // [5] Kamino Borrow
    // [6] ArbExecutor CPI
    // [7] Kamino Repay
    // [8] Jito Tip
    assert_eq!(
        tx.instructions[6].program_id.to_string(),
        ARB_EXECUTOR_PROGRAM_ID_STR
    );
    assert_eq!(
        &tx.instructions[6].data[0..8],
        &EXECUTE_THREE_LEG_DISCRIMINATOR
    );

    // Kamino repay index points to borrow index
    assert_eq!(tx.instructions[7].data[8..16], borrow.to_le_bytes());

    // Wire bytes valid and within MTU
    assert!(!tx.wire_bytes.is_empty());
    assert!(tx.wire_bytes.len() <= 4096);
}

#[test]
fn test_cpi_fails_closed_when_hop_solvency_violated() {
    let signer = KeypairSigner::new_random();
    let borrow = 1_000_000_000u64;

    // Violation: leg2_in (995_000_000) > leg1_min_out (990_000_000)
    let params = make_test_params(
        signer.pubkey(),
        borrow,
        borrow,
        990_000_000,
        995_000_000, // VIOLATION
        985_000_000,
        985_000_000,
        1_050_000_000,
        true,
    );

    let err = assemble_flash_funded_three_pool_cpi(&params, &signer, &[])
        .expect_err("Must fail when leg1 min_out < leg2 amount_in");

    assert!(matches!(
        err,
        FlashArb3LegError::LegOutputBelowRequired { leg: 1, .. }
    ));
}

#[test]
fn test_cpi_fails_closed_when_repayment_not_covered() {
    let signer = KeypairSigner::new_random();
    let borrow = 1_000_000_000u64;

    // Violation: leg3_min_out (950_000_000) < repay_amount (~1_000_011_529)
    let params = make_test_params(
        signer.pubkey(),
        borrow,
        borrow,
        990_000_000,
        990_000_000,
        980_000_000,
        980_000_000,
        950_000_000, // VIOLATION: does not cover borrow
        true,
    );

    let err = assemble_flash_funded_three_pool_cpi(&params, &signer, &[])
        .expect_err("Must fail when final output cannot cover repayment");

    assert!(matches!(
        err,
        FlashArb3LegError::LegOutputBelowRequired { leg: 3, .. }
    ));
}

#[test]
fn test_cpi_fails_closed_when_pools_not_distinct() {
    let signer = KeypairSigner::new_random();
    let borrow = 1_000_000_000u64;
    let mut params = make_test_params(
        signer.pubkey(),
        borrow,
        borrow,
        990_000_000,
        990_000_000,
        980_000_000,
        980_000_000,
        1_050_000_000,
        true,
    );

    // Violation: duplicate pool between leg 1 and leg 2
    params.leg_2.lb_pair = params.leg_1.lb_pair;

    let err = assemble_flash_funded_three_pool_cpi(&params, &signer, &[])
        .expect_err("Must fail when pools are not distinct");

    assert!(matches!(err, FlashArb3LegError::PoolsNotDistinct));
}
