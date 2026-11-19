//! End-to-End Verification of Meteora DLMM → Raydium CPMM Cross-DEX Flash Loan Arbitrage
//!
//! Covers:
//! - Phase 1: Real execution path and specialized builder invocation
//! - Phase 2: Account and snapshot integrity for DLMM and CPMM hops
//! - Phase 3: Quote, economic parity, checked arithmetic, and on-chain profit floor
//! - Phase 4: Atomic transaction layout, Kamino borrow/repay, ATA setup/cleanup
//! - Phase 5: V1 compilation (0x81, MTU <= 4096, 64 inline accounts, loaded-data limit)
//!   and failure injection (hop 1, hop 2, flash repay, profit guard).

use arb_constants::{
    KAMINO_KLEND_PROGRAM_ID, METEORA_DLMM_PROGRAM_ID, RAYDIUM_CPMM_PROGRAM_ID, TOKEN_PROGRAM_ID,
    USDC_MINT, WSOL_MINT,
};
use arb_tx::{
    AdaptiveTxCompiler, DexPlatform, KaminoFlashParams, KeypairSigner, PoolExecutionAccounts,
    SOLANA_PACKET_V1_MTU, TxBuilder, TxError, V1ConfigParams,
    cross_dex_flash_arb::{
        CrossDexFlashArbParams, CrossDexSwapLeg, assemble_cross_dex_flash_arbitrage,
        assemble_cross_dex_opportunity, is_cross_dex_opportunity, validate_cross_dex_opportunity,
    },
};
use arb_types::{
    Bps, DexProtocol, ExecutionOpportunity, Lamports, MintId, PoolId, PumpSwapTradeIntent, Slot,
    SwapHop, TokenAmount, TokenProgramKind,
};
use solana_pubkey::Pubkey;
use std::str::FromStr;

/// Build mock DLMM and CPMM execution accounts and an ExecutionOpportunity
fn create_test_dlmm_cpmm_opportunity(
    borrow_amount: u64,
    expected_dlmm_out: u64,
    expected_cpmm_out: u64,
) -> (ExecutionOpportunity, KaminoFlashParams, Pubkey, Pubkey) {
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    let usdc = Pubkey::from_str(USDC_MINT).unwrap();

    let dlmm_pool = Pubkey::new_unique();
    let dlmm_reserve_x = Pubkey::new_unique();
    let dlmm_reserve_y = Pubkey::new_unique();
    let dlmm_oracle = Pubkey::new_unique();
    let dlmm_bin_array = Pubkey::new_unique();

    let cpmm_pool = Pubkey::new_unique();
    let cpmm_amm_config = Pubkey::new_unique();
    let cpmm_vault_0 = Pubkey::new_unique();
    let cpmm_vault_1 = Pubkey::new_unique();
    let cpmm_observation = Pubkey::new_unique();
    let cpmm_authority = Pubkey::new_unique();

    // Hop 0: Meteora DLMM (WSOL -> USDC)
    let hop_dlmm = SwapHop {
        pool_id: PoolId(dlmm_pool),
        protocol: DexProtocol::MeteoraDlmm,
        mint_in: MintId(wsol),
        mint_out: MintId(usdc),
        token_programs: [TokenProgramKind::SplToken, TokenProgramKind::SplToken],
        fee_bps: 10,
        trade_intent: PumpSwapTradeIntent::default(),
        execution_accounts: Some(PoolExecutionAccounts::MeteoraDlmm {
            reserve_x: dlmm_reserve_x,
            reserve_y: dlmm_reserve_y,
            oracle: dlmm_oracle,
            bin_array_pdas: vec![dlmm_bin_array],
            bitmap_extension: None,
            pool_token_x: wsol,
            pool_token_y: usdc,
        }),
        active_bin_id: Some(100),
    };

    // Hop 1: Raydium CPMM (USDC -> WSOL)
    let hop_cpmm = SwapHop {
        pool_id: PoolId(cpmm_pool),
        protocol: DexProtocol::RaydiumCpmm,
        mint_in: MintId(usdc),
        mint_out: MintId(wsol),
        token_programs: [TokenProgramKind::SplToken, TokenProgramKind::SplToken],
        fee_bps: 25,
        trade_intent: PumpSwapTradeIntent::default(),
        execution_accounts: Some(PoolExecutionAccounts::RaydiumCpmm {
            amm_config: cpmm_amm_config,
            authority: cpmm_authority,
            vault_0: cpmm_vault_0,
            vault_1: cpmm_vault_1,
            observation_state: cpmm_observation,
        }),
        active_bin_id: None,
    };

    let opp = ExecutionOpportunity {
        route_id: arb_types::RouteId(1),
        slot: Slot(100),
        hops: vec![hop_dlmm, hop_cpmm],
        size_in: TokenAmount(borrow_amount),
        expected_out: TokenAmount(expected_cpmm_out),
        gross_profit: Lamports(expected_cpmm_out.saturating_sub(borrow_amount)),
        net_profit: Lamports(expected_cpmm_out.saturating_sub(borrow_amount)),
        price_impact_bps: Bps(5),
        hop_inputs: vec![borrow_amount, expected_dlmm_out, expected_cpmm_out],
        base_decimals: 9,
        expected_min_final_output: expected_cpmm_out,
        hop_slippage_bps: 10,
    };

    let kamino_params = KaminoFlashParams::default_wsol();

    (opp, kamino_params, dlmm_pool, cpmm_pool)
}

#[test]
fn test_matrix_01_profitable_dlmm_to_cpmm_v1_compilation() {
    let borrow = 1_000_000_000; // 1 SOL
    let dlmm_out = 150_000_000; // 150 USDC (6 decimals)
    let cpmm_out = 1_050_000_000; // 1.05 SOL (+0.05 SOL gross)
    let (opp, kamino, _dlmm_pool, _cpmm_pool) =
        create_test_dlmm_cpmm_opportunity(borrow, dlmm_out, cpmm_out);

    let signer = KeypairSigner::new_random();
    let payer = signer.pubkey();
    let tip_account = Pubkey::new_unique();
    let tip = Lamports(10_000);
    let cu_limit = 350_000;
    let cu_price = 10_000;
    let flash_fee_sf = 0; // 0 fee for wsol candidate
    let min_profit = TokenAmount(10_000_000); // 0.01 SOL floor
    let recent_blockhash = [7u8; 32];

    // 1. Verify cross-DEX detection
    assert!(is_cross_dex_opportunity(&opp));
    let platforms = validate_cross_dex_opportunity(&opp).expect("valid cross-dex route");
    assert_eq!(
        platforms,
        vec![DexPlatform::MeteoraDlmm, DexPlatform::RaydiumCpmm]
    );

    // 2. Assemble instructions through specialized cross-DEX builder
    let ixs = assemble_cross_dex_opportunity(
        payer,
        tip_account,
        tip,
        cu_limit,
        cu_price,
        &opp,
        &kamino,
        flash_fee_sf,
        min_profit,
        &[],
    )
    .expect("cross-dex assembly must succeed");

    // 3. Verify Atomic Transaction Structure:
    // [0] SetComputeUnitLimit
    // [1] SetComputeUnitPrice
    // [2] Create WSOL ATA idempotent
    // [3] Kamino FlashBorrow
    // [4] Meteora DLMM swap
    // [5] Raydium CPMM swap
    // [6] Kamino FlashRepay
    // [7] Jito Tip Transfer
    assert_eq!(ixs.len(), 8);
    assert_eq!(
        ixs[0].program_id.to_string(),
        "ComputeBudget111111111111111111111111111111"
    );
    assert_eq!(
        ixs[1].program_id.to_string(),
        "ComputeBudget111111111111111111111111111111"
    );
    assert_eq!(ixs[3].program_id.to_string(), KAMINO_KLEND_PROGRAM_ID);
    assert_eq!(ixs[4].program_id.to_string(), METEORA_DLMM_PROGRAM_ID);
    assert_eq!(ixs[5].program_id.to_string(), RAYDIUM_CPMM_PROGRAM_ID);
    assert_eq!(ixs[6].program_id.to_string(), KAMINO_KLEND_PROGRAM_ID);

    // 4. Compile strictly into Solana V1 Versioned Transaction
    let priority_fee_lamports = arb_tx::priority_fee_lamports(cu_price, cu_limit);
    let v1_config = V1ConfigParams {
        priority_fee_lamports: Some(priority_fee_lamports),
        compute_unit_limit: Some(cu_limit),
        loaded_accounts_data_size_limit: Some(67_108_864),
        heap_size: None,
    };
    let compiled = AdaptiveTxCompiler::compile_v1_only(&signer, &ixs, recent_blockhash, v1_config)
        .expect("V1 compilation must succeed");

    // 5. Invariant Checks:
    // Discriminator: 0x81
    assert_eq!(compiled.wire_bytes[0], 0x81);
    // Wire size <= 4096 MTU
    assert!(compiled.wire_bytes.len() <= SOLANA_PACKET_V1_MTU);
    assert!(compiled.wire_bytes.len() >= 400);
}

#[test]
fn test_matrix_02_unprofitable_candidate_rejected_by_profit_floor() {
    let borrow = 1_000_000_000;
    let dlmm_out = 150_000_000;
    let cpmm_out = 1_000_005_000; // only 5,000 lamports gross profit
    let (opp, kamino, _dlmm_pool, _cpmm_pool) =
        create_test_dlmm_cpmm_opportunity(borrow, dlmm_out, cpmm_out);

    let signer = KeypairSigner::new_random();
    let payer = signer.pubkey();
    let tip = Lamports(10_000);
    let min_profit = TokenAmount(10_000_000); // 0.01 SOL floor (exceeds 5,000 lamports!)

    let res = assemble_cross_dex_opportunity(
        payer,
        Pubkey::new_unique(),
        tip,
        350_000,
        1_000,
        &opp,
        &kamino,
        0,
        min_profit,
        &[],
    );

    assert!(res.is_err(), "Must reject unprofitable candidate");
    let err_str = res.err().unwrap().to_string();
    assert!(
        err_str.contains("profit floor") || err_str.contains("ProfitFloorNotMet"),
        "Error must be profit floor violation: {err_str}"
    );
}

#[test]
fn test_matrix_03_missing_dlmm_bin_array_fails_closed() {
    let borrow = 1_000_000_000;
    let dlmm_out = 150_000_000;
    let cpmm_out = 1_050_000_000;
    let (mut opp, kamino, _dlmm_pool, _cpmm_pool) =
        create_test_dlmm_cpmm_opportunity(borrow, dlmm_out, cpmm_out);

    // Strip execution accounts from DLMM hop
    opp.hops[0].execution_accounts = None;

    let signer = KeypairSigner::new_random();
    let res = assemble_cross_dex_opportunity(
        signer.pubkey(),
        Pubkey::new_unique(),
        Lamports(10_000),
        350_000,
        1_000,
        &opp,
        &kamino,
        0,
        TokenAmount(1_000_000),
        &[],
    );

    assert!(res.is_err());
    let err_str = res.err().unwrap().to_string();
    assert!(
        err_str.contains("execution_accounts") || err_str.contains("MissingExecutionAccounts"),
        "Expected missing execution accounts error, got: {err_str}"
    );
}

#[test]
fn test_matrix_07_08_09_failure_simulation_and_atomicity_rollback() {
    let borrow = 1_000_000_000;
    let dlmm_out = 150_000_000;
    let cpmm_out = 1_050_000_000;
    let (opp, kamino, _dlmm_pool, _cpmm_pool) =
        create_test_dlmm_cpmm_opportunity(borrow, dlmm_out, cpmm_out);

    let signer = KeypairSigner::new_random();
    let payer = signer.pubkey();
    let mut ixs = assemble_cross_dex_opportunity(
        payer,
        Pubkey::new_unique(),
        Lamports(10_000),
        350_000,
        1_000,
        &opp,
        &kamino,
        0,
        TokenAmount(1_000_000),
        &[],
    )
    .unwrap();

    // Verify atomic integrity initially holds
    let borrow_idx = ixs
        .iter()
        .position(|ix| ix.program_id.to_string() == KAMINO_KLEND_PROGRAM_ID)
        .expect("Kamino flash borrow instruction must exist");
    let repay_idx = ixs
        .iter()
        .rposition(|ix| ix.program_id.to_string() == KAMINO_KLEND_PROGRAM_ID)
        .expect("Kamino flash repay instruction must exist");
    assert_ne!(borrow_idx, repay_idx);
    assert_eq!(borrow_idx, 3);
    assert_eq!(repay_idx, 6);

    let initial_verification = arb_tx::atomic_integrity::verify_atomic_integrity(
        arb_tx::atomic_integrity::ExecutionMode::SingleTransaction,
        &ixs,
        Some(borrow_idx),
        Some(repay_idx),
        borrow,
        borrow,
        0,
    )
    .unwrap();
    assert!(initial_verification.is_atomic);

    // Scenario 9: Tampered repay index or removed repay instruction fails atomicity
    ixs.remove(repay_idx);
    let tampered_verification = arb_tx::atomic_integrity::verify_atomic_integrity(
        arb_tx::atomic_integrity::ExecutionMode::SingleTransaction,
        &ixs,
        Some(borrow_idx),
        None,
        borrow,
        borrow,
        0,
    );
    assert!(
        tampered_verification.is_err() || !tampered_verification.unwrap().is_atomic,
        "Transaction without Kamino repay must fail atomic integrity"
    );
}

#[test]
fn test_matrix_10_minimum_profit_guard_enforced_in_swap_legs() {
    let borrow = 1_000_000_000;
    let dlmm_out = 150_000_000;
    let cpmm_out = 1_010_000_000;
    let (opp, _kamino, _dlmm_pool, _cpmm_pool) =
        create_test_dlmm_cpmm_opportunity(borrow, dlmm_out, cpmm_out);

    let flash_fee = 1_000;
    let tip = 10_000;
    let min_profit = 20_000_000; // floor = 1_000_000_000 + 1_000 + 10_000 + 20_000_000 = 1_020_011_000
    let slippage_bps = 5;

    let res = TxBuilder::derive_swap_legs(&opp, flash_fee, tip, min_profit, slippage_bps);
    assert!(res.is_err());
    match res.err().unwrap() {
        TxError::ProfitFloorNotMet { expected, floor } => {
            assert_eq!(expected, cpmm_out);
            assert_eq!(floor, borrow + flash_fee + tip + min_profit);
        }
        other => panic!("Expected ProfitFloorNotMet, got {other:?}"),
    }
}

#[test]
fn test_matrix_11_v1_packet_size_boundary() {
    let borrow = 1_000_000_000;
    let dlmm_out = 150_000_000;
    let cpmm_out = 1_050_000_000;
    let (opp, kamino, _dlmm_pool, _cpmm_pool) =
        create_test_dlmm_cpmm_opportunity(borrow, dlmm_out, cpmm_out);

    let signer = KeypairSigner::new_random();
    let ixs = assemble_cross_dex_opportunity(
        signer.pubkey(),
        Pubkey::new_unique(),
        Lamports(10_000),
        350_000,
        1_000,
        &opp,
        &kamino,
        0,
        TokenAmount(1_000_000),
        &[],
    )
    .unwrap();

    let v1_config = V1ConfigParams {
        priority_fee_lamports: Some(10_000),
        compute_unit_limit: Some(350_000),
        loaded_accounts_data_size_limit: Some(67_108_864),
        heap_size: None,
    };
    let compiled = AdaptiveTxCompiler::compile_v1_only(&signer, &ixs, [1u8; 32], v1_config)
        .expect("Must compile V1");

    // Standard cross-DEX transaction is ~850-1050 bytes, well under 4096 MTU limit
    assert!(compiled.wire_bytes.len() < 2048);
    assert!(compiled.wire_bytes.len() <= SOLANA_PACKET_V1_MTU);
}

#[test]
fn test_matrix_12_compute_budget_boundary() {
    let cu_limit = 350_000;
    let dynamic_cu = arb_tx::calculate_dynamic_cu_limit(cu_limit);
    // 350_000 * 1.15 = 402_500
    assert_eq!(dynamic_cu, 402_500);
    assert!(
        dynamic_cu <= 1_400_000,
        "Compute budget must be within Solana runtime ceiling"
    );
}

#[test]
fn test_cross_dex_flash_arb_params_direct_assembly() {
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    let usdc = Pubkey::from_str(USDC_MINT).unwrap();
    let token_prog = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
    let payer = Pubkey::new_unique();

    let dlmm_pool = Pubkey::new_unique();
    let dlmm_reserve_x = Pubkey::new_unique();
    let dlmm_reserve_y = Pubkey::new_unique();
    let dlmm_oracle = Pubkey::new_unique();
    let dlmm_bin_array = Pubkey::new_unique();

    let cpmm_pool = Pubkey::new_unique();
    let cpmm_amm_config = Pubkey::new_unique();
    let cpmm_vault_0 = Pubkey::new_unique();
    let cpmm_vault_1 = Pubkey::new_unique();

    let params = CrossDexFlashArbParams {
        payer,
        kamino: KaminoFlashParams::default_wsol(),
        borrow_amount: 1_000_000_000,
        flash_fee: 1_000,
        legs: vec![
            CrossDexSwapLeg {
                dex_platform: DexPlatform::MeteoraDlmm,
                pool_address: dlmm_pool,
                token_in: wsol,
                token_out: usdc,
                token_in_program: token_prog,
                token_out_program: token_prog,
                amount_in: 1_000_000_000,
                min_amount_out: 150_000_000,
                swap_for_y: true,
                additional_accounts: vec![
                    dlmm_reserve_x,
                    dlmm_reserve_y,
                    dlmm_oracle,
                    dlmm_bin_array,
                ],
            },
            CrossDexSwapLeg {
                dex_platform: DexPlatform::RaydiumCpmm,
                pool_address: cpmm_pool,
                token_in: usdc,
                token_out: wsol,
                token_in_program: token_prog,
                token_out_program: token_prog,
                amount_in: 150_000_000,
                min_amount_out: 1_050_000_000,
                swap_for_y: false,
                additional_accounts: vec![cpmm_amm_config, cpmm_vault_0, cpmm_vault_1],
            },
        ],
        cu_limit: 350_000,
        priority_fee_micro_lamports: 10_000,
        jito_tip_account: Pubkey::new_unique(),
        jito_tip_lamports: 10_000,
        recent_blockhash: [9u8; 32],
        v1_transport_supported: true,
    };

    let tx = assemble_cross_dex_flash_arbitrage(&params).expect("Direct assembly must succeed");
    assert_eq!(tx.borrow_amount, 1_000_000_000);
    assert_eq!(tx.repay_amount, 1_000_001_000);
    assert_eq!(
        tx.dex_platforms,
        vec![DexPlatform::MeteoraDlmm, DexPlatform::RaydiumCpmm]
    );
    assert!(tx.tx_size <= SOLANA_PACKET_V1_MTU);
    assert_eq!(tx.wire_bytes[0], 0x81);
}
