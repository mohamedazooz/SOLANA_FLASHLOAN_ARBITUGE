//! Comprehensive Integration and Unit Test Suite for Milestone M2.
//! Verifies:
//! 1. Compute Budget Realignment: 260,000 CU (2-hop) and 320,000 CU (3-hop).
//! 2. Atomic 7-Instruction (2-hop) and 8-Instruction (3-hop) bundle assembly.
//! 3. Kamino Flash Loan Instructions Overhaul:
//!    - 16-byte Borrow ([135, 231, 52, 167, 7, 52, 212, 193] + amount: u64).
//!    - 17-byte Repay ([185, 117, 0, 203, 96, 245, 180, 186] + amount: u64 + borrow_idx: 2).
//!    - Canonical 12-account matrix and PDA derivation (`lma` seeds).
//! 4. PreWarmedTxTemplate sub-microsecond in-place byte mutation & signature validity.
//! 5. Solana MTU compliance (<= 1232 bytes).

use arb_tx::kamino::{
    FLASH_BORROW_DISCRIMINATOR, FLASH_REPAY_DISCRIMINATOR, KAMINO_KLEND_PROGRAM_ID,
    KAMINO_MAIN_MARKET, KAMINO_MAIN_MARKET_AUTHORITY, KAMINO_WSOL_FEE_VAULT, KAMINO_WSOL_RESERVE,
    KAMINO_WSOL_SUPPLY_VAULT, derive_lending_market_authority,
};
use arb_tx::template::PreWarmedTxTemplate;
use arb_tx::{
    COMPUTE_BUDGET_LIMIT_2HOP, COMPUTE_BUDGET_LIMIT_3HOP, JITO_TIP_ACCOUNTS, KaminoBundleParams,
    KaminoFlashParams, KeypairSigner, ORCA_SOL_USDC_POOL, SOLANA_PACKET_MTU_LIMIT,
    SYSVAR_INSTRUCTIONS_ID, TOKEN_PROGRAM_ID, TxBuilder, TxError, WSOL_MINT, get_cu_limit_for_hops,
    get_next_jito_tip_account,
};
use arb_types::{
    Bps, DexProtocol, ExecutionOpportunity, Lamports, MintId, PoolId, RouteId, Slot, SwapHop,
    TokenAmount,
};
use ed25519_dalek::{Signature, Verifier};
use solana_pubkey::Pubkey;
use std::str::FromStr;
use std::time::Instant;

/// Exact U68F60 encoding of a 5 bps flash-loan rate — the on-chain unit of klend's
/// `ReserveConfig.fees.flash_loan_fee_sf`.
///
/// `0.0005 * 2^60 = 576_460_752_303_423.488`, truncated to `576_460_752_303_423`,
/// which prices a 10 SOL borrow at exactly 5,000,000 lamports. The previous fixture
/// `9_223_372_036_854_775` is `0.0005 * 2^64` (Q64.64): read as U68F60 it means 0.8 %,
/// a 16x overcharge that made the profit-floor assertions in this file wrong.
const KAMINO_FEE_SF_5_BPS: u64 = 576_460_752_303_423;

fn create_test_signer(seed_byte: u8) -> KeypairSigner {
    let mut seed = [seed_byte; 32];
    seed[0] = seed_byte;
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&seed);
    let pubkey_bytes = signing_key.verifying_key().to_bytes();
    let mut keypair_bytes = [0u8; 64];
    keypair_bytes[..32].copy_from_slice(&seed);
    keypair_bytes[32..].copy_from_slice(&pubkey_bytes);
    KeypairSigner::from_bytes(&keypair_bytes).unwrap()
}

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

fn create_2hop_opportunity(wsol: Pubkey, usdc: Pubkey, size_lamports: u64) -> ExecutionOpportunity {
    ExecutionOpportunity {
        route_id: RouteId(201),
        slot: Slot(1000),
        size_in: TokenAmount(size_lamports),
        expected_out: TokenAmount(size_lamports + 50_000_000),
        hop_inputs: vec![
            size_lamports,
            size_lamports + 25_000_000,
            size_lamports + 50_000_000,
        ],
        gross_profit: Lamports(50_000_000),
        net_profit: Lamports(45_000_000),
        base_decimals: 9,
        price_impact_bps: Bps(5),
        hops: vec![
            SwapHop {
                pool_id: PoolId::new([1; 32]),
                protocol: DexProtocol::RaydiumV4,
                mint_in: MintId(wsol),
                mint_out: MintId(usdc),
                fee_bps: 25,
                active_bin_id: None,
                token_programs: [arb_types::TokenProgramKind::SplToken; 2],
                trade_intent: Default::default(),
                execution_accounts: Some(mock_raydium_v4_exec_accounts()),
            },
            SwapHop {
                pool_id: PoolId(Pubkey::from_str(ORCA_SOL_USDC_POOL).unwrap()),
                protocol: DexProtocol::OrcaWhirlpool,
                mint_in: MintId(usdc),
                mint_out: MintId(wsol),
                fee_bps: 5,
                active_bin_id: None,
                token_programs: [arb_types::TokenProgramKind::SplToken; 2],
                trade_intent: Default::default(),
                execution_accounts: None,
            },
        ],
        expected_min_final_output: 0,
    }
}

fn create_3hop_opportunity(
    wsol: Pubkey,
    usdc: Pubkey,
    bonk: Pubkey,
    size_lamports: u64,
) -> ExecutionOpportunity {
    ExecutionOpportunity {
        route_id: RouteId(301),
        slot: Slot(1001),
        size_in: TokenAmount(size_lamports),
        expected_out: TokenAmount(size_lamports + 80_000_000),
        hop_inputs: vec![
            size_lamports,
            size_lamports + 20_000_000,
            size_lamports + 50_000_000,
            size_lamports + 80_000_000,
        ],
        gross_profit: Lamports(80_000_000),
        net_profit: Lamports(70_000_000),
        base_decimals: 9,
        price_impact_bps: Bps(8),
        hops: vec![
            SwapHop {
                pool_id: PoolId::new([1; 32]),
                protocol: DexProtocol::RaydiumV4,
                mint_in: MintId(wsol),
                mint_out: MintId(usdc),
                fee_bps: 25,
                active_bin_id: None,
                token_programs: [arb_types::TokenProgramKind::SplToken; 2],
                trade_intent: Default::default(),
                execution_accounts: Some(mock_raydium_v4_exec_accounts()),
            },
            SwapHop {
                pool_id: PoolId(Pubkey::from_str(ORCA_SOL_USDC_POOL).unwrap()),
                protocol: DexProtocol::OrcaWhirlpool,
                mint_in: MintId(usdc),
                mint_out: MintId(bonk),
                fee_bps: 5,
                active_bin_id: None,
                token_programs: [arb_types::TokenProgramKind::SplToken; 2],
                trade_intent: Default::default(),
                execution_accounts: None,
            },
            SwapHop {
                pool_id: PoolId::new([3; 32]),
                protocol: DexProtocol::RaydiumV4,
                mint_in: MintId(bonk),
                mint_out: MintId(wsol),
                fee_bps: 25,
                active_bin_id: None,
                token_programs: [arb_types::TokenProgramKind::SplToken; 2],
                trade_intent: Default::default(),
                execution_accounts: Some(mock_raydium_v4_exec_accounts()),
            },
        ],
        expected_min_final_output: 0,
    }
}

#[test]
fn test_2hop_bundle_exact_7_instructions_and_260k_cu() {
    let signer = create_test_signer(1);
    let payer = signer.pubkey();
    let tip_acc = Pubkey::from_str(JITO_TIP_ACCOUNTS[0]).unwrap();
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    let usdc = Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
    let kamino_params = KaminoFlashParams::default();
    let opp = create_2hop_opportunity(wsol, usdc, 10_000_000_000); // 10 SOL
    let alt = TxBuilder::create_default_mainnet_alt();
    let recent_blockhash = [0x55; 32];

    let params = KaminoBundleParams {
        payer,
        signer: &signer,
        tip_account: tip_acc,
        tip_lamports: Lamports(50_000),
        priority_fee_micro_lamports: 1_000,
        opportunity: &opp,
        kamino_params: &kamino_params,
        recent_blockhash,
        lookup_tables: &[alt],
        flash_loan_fee_sf: 0,
    };

    // 1. Verify assemble_atomic_bundle outputs EXACTLY 7 instructions
    let ixs = TxBuilder::assemble_atomic_bundle(
        payer,
        tip_acc,
        Lamports(50_000),
        1_000,
        &opp,
        &kamino_params,
        KAMINO_FEE_SF_5_BPS, // fee_sf: exact U68F60 encoding of 5 bps
        TokenAmount(30_000), // min_profit_raw in base-mint (WSOL) units
    )
    .unwrap();

    assert_eq!(
        ixs.len(),
        7,
        "2-hop bundle must contain strictly 7 instructions"
    );

    // [0] SetComputeUnitLimit: 260_000 CU
    assert_eq!(get_cu_limit_for_hops(2), COMPUTE_BUDGET_LIMIT_2HOP);
    assert_eq!(ixs[0].data[0], 2); // SetComputeUnitLimit tag
    assert_eq!(&ixs[0].data[1..5], &COMPUTE_BUDGET_LIMIT_2HOP.to_le_bytes());
    assert_eq!(&ixs[0].data[1..5], &[0xA0, 0xF7, 0x03, 0x00]); // 260,000 = 0x0003F7A0

    // [1] SetComputeUnitPrice
    assert_eq!(ixs[1].data[0], 3); // SetComputeUnitPrice tag
    assert_eq!(&ixs[1].data[1..9], &1_000u64.to_le_bytes());

    // [2] KaminoFlashBorrow: Index = 2
    assert_eq!(
        ixs[2].program_id,
        Pubkey::from_str(KAMINO_KLEND_PROGRAM_ID).unwrap()
    );
    assert_eq!(ixs[2].data.len(), 16);
    assert_eq!(&ixs[2].data[0..8], &FLASH_BORROW_DISCRIMINATOR);
    assert_eq!(&ixs[2].data[8..16], &10_000_000_000u64.to_le_bytes());
    assert_eq!(
        ixs[2].accounts.len(),
        12,
        "Borrow must have canonical 12 accounts"
    );
    assert_eq!(ixs[2].accounts[0].pubkey, payer); // User authority signer
    assert!(ixs[2].accounts[0].is_signer);
    assert_eq!(
        ixs[2].accounts[1].pubkey,
        Pubkey::from_str(KAMINO_MAIN_MARKET_AUTHORITY).unwrap()
    );
    assert_eq!(
        ixs[2].accounts[2].pubkey,
        Pubkey::from_str(KAMINO_MAIN_MARKET).unwrap()
    );
    assert_eq!(
        ixs[2].accounts[10].pubkey,
        Pubkey::from_str(SYSVAR_INSTRUCTIONS_ID).unwrap()
    );

    // [3] Swap Leg 1
    assert_eq!(
        ixs[3].program_id,
        Pubkey::from_str("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8").unwrap()
    );

    // [4] Swap Leg 2
    assert_eq!(
        ixs[4].program_id,
        Pubkey::from_str("whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc").unwrap()
    );

    // [5] KaminoFlashRepay: Index = 5, pointing to borrow_instruction_index = 2
    assert_eq!(
        ixs[5].program_id,
        Pubkey::from_str(KAMINO_KLEND_PROGRAM_ID).unwrap()
    );
    assert_eq!(
        ixs[5].data.len(),
        17,
        "Repay data must be strictly 17 bytes"
    );
    assert_eq!(&ixs[5].data[0..8], &FLASH_REPAY_DISCRIMINATOR);
    // Repay `amount` == borrow principal; the origination fee is collected
    // separately, on top, from the same source ATA at repay time.
    assert_eq!(&ixs[5].data[8..16], &10_000_000_000u64.to_le_bytes());
    assert_eq!(
        ixs[5].data[16], 2,
        "borrow_instruction_index must be strictly 2"
    );
    assert_eq!(
        ixs[5].accounts.len(),
        12,
        "Repay must have canonical 12 accounts"
    );
    assert_eq!(
        ixs[5].accounts[10].pubkey,
        Pubkey::from_str(SYSVAR_INSTRUCTIONS_ID).unwrap()
    );

    // [6] Jito Tip Transfer
    assert_eq!(ixs[6].program_id, Pubkey::new_from_array([0; 32])); // System Program
    assert_eq!(&ixs[6].data[0..4], &2u32.to_le_bytes());
    assert_eq!(&ixs[6].data[4..12], &50_000u64.to_le_bytes());
    assert_eq!(ixs[6].accounts[0].pubkey, payer);
    assert_eq!(ixs[6].accounts[1].pubkey, tip_acc);

    // 2. Build signed VersionedTransaction and assert wire format & MTU compliance
    let v0_tx = TxBuilder::build_kamino_bundle(&params).unwrap();
    assert!(
        v0_tx.wire_bytes.len() <= SOLANA_PACKET_MTU_LIMIT,
        "Transaction must fit within 1232 bytes MTU"
    );
    assert_eq!(v0_tx.signatures.len(), 1);

    // Cryptographic signature check
    let sig = Signature::from_bytes(&v0_tx.signatures[0]);
    assert!(
        signer.verifying_key().verify(&v0_tx.message, &sig).is_ok(),
        "Ed25519 signature must verify over message"
    );
}

#[test]
fn test_3hop_bundle_exact_8_instructions_and_320k_cu() {
    let signer = create_test_signer(2);
    let payer = signer.pubkey();
    let tip_acc = Pubkey::from_str(JITO_TIP_ACCOUNTS[1]).unwrap();
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    let usdc = Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
    let bonk = Pubkey::from_str("DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263").unwrap();
    let kamino_params = KaminoFlashParams::default();
    let opp = create_3hop_opportunity(wsol, usdc, bonk, 25_000_000_000); // 25 SOL
    let alt = TxBuilder::create_default_mainnet_alt();
    let recent_blockhash = [0x77; 32];

    let params = KaminoBundleParams {
        payer,
        signer: &signer,
        tip_account: tip_acc,
        tip_lamports: Lamports(100_000),
        priority_fee_micro_lamports: 2_000,
        opportunity: &opp,
        kamino_params: &kamino_params,
        recent_blockhash,
        lookup_tables: &[alt],
        flash_loan_fee_sf: 0,
    };

    let ixs = TxBuilder::assemble_atomic_bundle(
        payer,
        tip_acc,
        Lamports(100_000),
        2_000,
        &opp,
        &kamino_params,
        KAMINO_FEE_SF_5_BPS, // fee_sf: exact U68F60 encoding of 5 bps
        TokenAmount(30_000), // min_profit_raw in base-mint (WSOL) units
    )
    .unwrap();

    assert_eq!(
        ixs.len(),
        8,
        "3-hop bundle must contain strictly 8 instructions"
    );

    // [0] SetComputeUnitLimit: 320_000 CU
    assert_eq!(ixs[0].data[0], 2);
    assert_eq!(&ixs[0].data[1..5], &COMPUTE_BUDGET_LIMIT_3HOP.to_le_bytes());
    assert_eq!(&ixs[0].data[1..5], &[0x00, 0xE2, 0x04, 0x00]); // 320,000 = 0x0004E200

    // [2] KaminoFlashBorrow at index 2
    assert_eq!(
        ixs[2].program_id,
        Pubkey::from_str(KAMINO_KLEND_PROGRAM_ID).unwrap()
    );

    // [3, 4, 5] Swap Hops 1, 2, 3
    assert_eq!(
        ixs[3].program_id,
        Pubkey::from_str("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8").unwrap()
    );
    assert_eq!(
        ixs[4].program_id,
        Pubkey::from_str("whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc").unwrap()
    );
    assert_eq!(
        ixs[5].program_id,
        Pubkey::from_str("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8").unwrap()
    );

    // [6] KaminoFlashRepay with borrow_instruction_index = 2
    assert_eq!(
        ixs[6].program_id,
        Pubkey::from_str(KAMINO_KLEND_PROGRAM_ID).unwrap()
    );
    assert_eq!(ixs[6].data.len(), 17);
    assert_eq!(
        ixs[6].data[16], 2,
        "borrow_instruction_index in 3-hop must still be 2"
    );

    // [7] Jito Tip Transfer
    assert_eq!(ixs[7].program_id, Pubkey::new_from_array([0; 32]));

    // Build and verify signed transaction
    let v0_tx = TxBuilder::build_kamino_bundle(&params).unwrap();
    assert!(v0_tx.wire_bytes.len() <= SOLANA_PACKET_MTU_LIMIT);
}

#[test]
fn test_kamino_12_account_metas_and_pda_derivation() {
    let market = Pubkey::from_str(KAMINO_MAIN_MARKET).unwrap();
    let klend = Pubkey::from_str(KAMINO_KLEND_PROGRAM_ID).unwrap();
    let (auth, bump) = derive_lending_market_authority(&market, &klend);

    assert_eq!(
        auth,
        Pubkey::from_str(KAMINO_MAIN_MARKET_AUTHORITY).unwrap()
    );
    assert_eq!(bump, 248);

    let user = Pubkey::new_from_array([33; 32]);
    let params = KaminoFlashParams::for_user(user);
    let accounts = params.to_flash_accounts(user);

    assert_eq!(accounts.user_authority, user);
    assert_eq!(accounts.lending_market_authority, auth);
    assert_eq!(accounts.lending_market, market);
    assert_eq!(
        accounts.reserve,
        Pubkey::from_str(KAMINO_WSOL_RESERVE).unwrap()
    );
    assert_eq!(
        accounts.reserve_supply_vault,
        Pubkey::from_str(KAMINO_WSOL_SUPPLY_VAULT).unwrap()
    );
    assert_eq!(
        accounts.reserve_fee_vault,
        Pubkey::from_str(KAMINO_WSOL_FEE_VAULT).unwrap()
    );
    assert_eq!(
        accounts.sysvar_instructions,
        Pubkey::from_str(SYSVAR_INSTRUCTIONS_ID).unwrap()
    );
    assert_eq!(
        accounts.token_program,
        Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap()
    );
}

#[test]
fn test_prewarmed_template_patch_and_verify() {
    let signer = create_test_signer(42);
    let payer = signer.pubkey();
    let tip_acc = Pubkey::from_str(JITO_TIP_ACCOUNTS[2]).unwrap();
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    let usdc = Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
    let kamino_params = KaminoFlashParams::default();
    let opp = create_2hop_opportunity(wsol, usdc, 10_000_000_000);
    let alt = TxBuilder::create_default_mainnet_alt();
    let initial_blockhash = [0xAA; 32];
    let initial_tip = 50_000u64;

    let bundle_params = KaminoBundleParams {
        payer,
        signer: &signer,
        tip_account: tip_acc,
        tip_lamports: Lamports(initial_tip),
        priority_fee_micro_lamports: 1_000,
        opportunity: &opp,
        kamino_params: &kamino_params,
        recent_blockhash: initial_blockhash,
        lookup_tables: &[alt],
        flash_loan_fee_sf: 0,
    };

    // Pre-warm the transaction template ahead of time
    let mut template =
        PreWarmedTxTemplate::from_bundle_params(&bundle_params, KAMINO_FEE_SF_5_BPS).unwrap();

    // Verify initial wire size fits within MTU limit
    assert!(template.raw_bytes.len() <= SOLANA_PACKET_MTU_LIMIT);

    let borrow_offset = template.borrow_amount_offset;
    let repay_offset = template.repay_amount_offset;
    let tip_offset = template.tip_amount_offset;
    let bh_offset = template.blockhash_offset;
    let msg_offset = template.message_offset;

    // Live trigger simulation: New opportunity with d* = 15 SOL, tip = 80,000 lamports, new blockhash
    let new_d_star = 15_000_000_000u64;
    let new_tip = 80_000u64;
    let new_blockhash = [0x99; 32];

    let start = Instant::now();
    let wire_tx = template.patch_and_sign(new_d_star, new_tip, new_blockhash, &signer);
    let elapsed = start.elapsed();

    // Verify byte contents have mutated in-place
    let borrow_bytes: [u8; 8] = wire_tx[borrow_offset..borrow_offset + 8]
        .try_into()
        .unwrap();
    assert_eq!(u64::from_le_bytes(borrow_bytes), new_d_star);

    let repay_bytes: [u8; 8] = wire_tx[repay_offset..repay_offset + 8].try_into().unwrap();
    // klend's repay argument is the borrowed principal only; the origination fee is
    // collected separately, on top, at repay time, so it must never be added here.
    assert_eq!(u64::from_le_bytes(repay_bytes), new_d_star);
    assert_eq!(
        arb_tx::kamino::calculate_kamino_flash_fee(new_d_star, KAMINO_FEE_SF_5_BPS),
        7_500_000,
        "the stored fee_sf must still price 5 bps on 15 SOL for ATA funding"
    );

    let tip_bytes: [u8; 8] = wire_tx[tip_offset..tip_offset + 8].try_into().unwrap();
    assert_eq!(u64::from_le_bytes(tip_bytes), new_tip);

    let bh_bytes: [u8; 32] = wire_tx[bh_offset..bh_offset + 32].try_into().unwrap();
    assert_eq!(bh_bytes, new_blockhash);

    // Cryptographic verification of newly generated Dalek signature
    let sig_bytes: [u8; 64] = wire_tx[1..65].try_into().unwrap();
    let sig = Signature::from_bytes(&sig_bytes);
    let message_bytes = &wire_tx[msg_offset..];
    assert!(
        signer.verifying_key().verify(message_bytes, &sig).is_ok(),
        "In-place signature must cryptographically verify"
    );

    println!(
        "PreWarmedTxTemplate patch_and_sign executed in: {:?}",
        elapsed
    );
}

#[test]
fn test_jito_tip_accounts_round_robin() {
    assert_eq!(JITO_TIP_ACCOUNTS.len(), 8);
    for acc in JITO_TIP_ACCOUNTS {
        assert!(Pubkey::from_str(acc).is_ok());
    }

    let acc1 = get_next_jito_tip_account();
    let acc2 = get_next_jito_tip_account();
    assert_ne!(
        acc1, acc2,
        "Subsequent calls should cycle through tip accounts"
    );
}

#[test]
fn test_zero_borrow_amount_rejected() {
    let signer = create_test_signer(3);
    let payer = signer.pubkey();
    let tip_acc = Pubkey::from_str(JITO_TIP_ACCOUNTS[0]).unwrap();
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    let usdc = Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
    let kamino_params = KaminoFlashParams::default();
    let zero_opp = create_2hop_opportunity(wsol, usdc, 0);

    let err = TxBuilder::assemble_atomic_bundle(
        payer,
        tip_acc,
        Lamports(50_000),
        1_000,
        &zero_opp,
        &kamino_params,
        KAMINO_FEE_SF_5_BPS, // fee_sf: exact U68F60 encoding of 5 bps
        TokenAmount(30_000), // min_profit_raw in base-mint (WSOL) units
    )
    .unwrap_err();

    match err {
        TxError::ZeroBorrowAmount => (),
        _ => panic!("Expected ZeroBorrowAmount error, got: {:?}", err),
    }
}

#[test]
fn test_invalid_hop_count_rejected() {
    let signer = create_test_signer(4);
    let payer = signer.pubkey();
    let tip_acc = Pubkey::from_str(JITO_TIP_ACCOUNTS[0]).unwrap();
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    let usdc = Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
    let kamino_params = KaminoFlashParams::default();

    // 1-hop opportunity (invalid)
    let one_hop_opp = ExecutionOpportunity {
        route_id: RouteId(101),
        slot: Slot(100),
        size_in: TokenAmount(1_000_000),
        expected_out: TokenAmount(1_050_000),
        hop_inputs: vec![],
        gross_profit: Lamports(50_000),
        net_profit: Lamports(45_000),
        base_decimals: 9,
        price_impact_bps: Bps(1),
        hops: vec![SwapHop {
            pool_id: PoolId::new([1; 32]),
            protocol: DexProtocol::RaydiumV4,
            mint_in: MintId(wsol),
            mint_out: MintId(usdc),
            fee_bps: 25,
            active_bin_id: None,
            token_programs: [arb_types::TokenProgramKind::SplToken; 2],
            trade_intent: Default::default(),
            execution_accounts: None,
        }],

        expected_min_final_output: 0,
    };

    let err = TxBuilder::assemble_atomic_bundle(
        payer,
        tip_acc,
        Lamports(50_000),
        1_000,
        &one_hop_opp,
        &kamino_params,
        KAMINO_FEE_SF_5_BPS, // fee_sf: exact U68F60 encoding of 5 bps
        TokenAmount(30_000), // min_profit_raw in base-mint (WSOL) units
    )
    .unwrap_err();

    match err {
        TxError::InvalidHopCount(1) => (),
        _ => panic!("Expected InvalidHopCount(1), got: {:?}", err),
    }
}

#[test]
fn test_validate_jito_bundle_is_version_aware() {
    let signer = KeypairSigner::new_random();
    let blockhash = [7u8; 32];
    let empty_ixs: Vec<arb_tx::CompiledInstruction> = Vec::new();

    // A small legacy transaction is always accepted.
    let legacy = TxBuilder::build_signed_transaction(&signer, &empty_ixs, blockhash).unwrap();
    assert!(arb_tx::validate_jito_bundle(std::slice::from_ref(&legacy)).is_ok());

    // A V1 transaction that exceeds the legacy 1,232-byte MTU must now pass
    // the Jito size gate (SIMD-0296 / SIMD-0385: v1 may reach 4,096 bytes),
    // while a legacy envelope of the same size must still be rejected.
    let v1 = arb_tx::AdaptiveTxCompiler::build_signed_v1_transaction(
        &signer,
        &empty_ixs,
        blockhash,
        arb_tx::V1ConfigParams {
            compute_unit_limit: Some(200_000),
            loaded_accounts_data_size_limit: Some(65_536),
            ..Default::default()
        },
        arb_tx::FormatSelectionReason::ExplicitRequested(arb_tx::TransactionVersion::V1),
    )
    .unwrap();
    assert_eq!(v1.wire_bytes[0], 0x81);
    // Message: 1 (version) + 3 (header) + 4 (mask) + 32 (blockhash) + 1 + 1
    //          + 32 (1 address) + 8 (CU 4 + loaded-data 4; no priority fee)
    //          + 64 (trailing signature) = 146 bytes.
    assert_eq!(v1.wire_size(), 146);
    assert!(arb_tx::validate_jito_bundle(std::slice::from_ref(&v1.wire_bytes)).is_ok());

    // Pad the legacy envelope past 1,232 bytes: version detection sees a
    // legacy/unknown envelope and holds it to the tighter bound.
    let mut oversized_legacy = legacy;
    oversized_legacy.resize(2000, 0);
    let err = arb_tx::validate_jito_bundle(&[oversized_legacy]).unwrap_err();
    match err {
        TxError::TransactionTooLarge { size, limit } => {
            assert_eq!(size, 2000);
            assert_eq!(limit, SOLANA_PACKET_MTU_LIMIT);
        }
        other => panic!("Expected TransactionTooLarge, got: {other:?}"),
    }
}

#[test]
fn test_rpc_max_supported_version_from_wire() {
    let signer = KeypairSigner::new_random();
    let blockhash = [7u8; 32];
    let empty_ixs: Vec<arb_tx::CompiledInstruction> = Vec::new();

    let legacy = TxBuilder::build_signed_transaction(&signer, &empty_ixs, blockhash).unwrap();
    assert_eq!(
        arb_tx::rpc_max_supported_version_from_wire(&legacy).unwrap(),
        None,
        "legacy transactions need no RPC version hint"
    );

    let v0 = arb_tx::AdaptiveTxCompiler::build_signed_v0_transaction(
        &signer,
        &empty_ixs,
        blockhash,
        &[],
        arb_tx::FormatSelectionReason::ExplicitRequested(arb_tx::TransactionVersion::V0),
    )
    .unwrap();
    assert_eq!(v0.wire_bytes[65], 0x80);
    assert_eq!(
        arb_tx::rpc_max_supported_version_from_wire(&v0.wire_bytes).unwrap(),
        Some(0),
        "v0 maps to RPC version 0"
    );

    let v1 = arb_tx::AdaptiveTxCompiler::build_signed_v1_transaction(
        &signer,
        &empty_ixs,
        blockhash,
        arb_tx::V1ConfigParams {
            compute_unit_limit: Some(200_000),
            loaded_accounts_data_size_limit: Some(65_536),
            ..Default::default()
        },
        arb_tx::FormatSelectionReason::ExplicitRequested(arb_tx::TransactionVersion::V1),
    )
    .unwrap();
    assert_eq!(v1.wire_bytes[0], 0x81);
    assert_eq!(
        arb_tx::rpc_max_supported_version_from_wire(&v1.wire_bytes).unwrap(),
        Some(1),
        "v1 (SIMD-0385) maps to RPC version 1"
    );

    // Malformed wire bytes fail closed.
    assert!(arb_tx::rpc_max_supported_version_from_wire(&[]).is_err());
    assert!(arb_tx::rpc_max_supported_version_from_wire(&[0x01, 0x02]).is_err());
    assert!(arb_tx::rpc_max_supported_version_from_wire(&[0xFF, 0x00, 0x00]).is_err());
}
