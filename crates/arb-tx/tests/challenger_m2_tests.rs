//! Empirical Adversarial Stress Test Suite for Milestone M2 (Kamino Flash Loan & TxBuilder).
//! Authored by Challenger M2 (`teamwork_challenger_m2_1`).
//!
//! Objectives:
//! 1. Introspection & Indexing: Verify borrow_instruction_index == 2 in 7-ix and 8-ix bundles.
//! 2. AccountMetas Ordering: Test all 12 accounts against Kamino Anchor IDL requirements.
//! 3. CU Budget Boundaries: Verify 260k (2-hop) and 320k (3-hop) enforcement across edge cases.
//! 4. Kamino On-Chain Invariant Oracle: Emulate Kamino's on-chain sysvar introspection checks.
//! 5. Pre-warmed Template Fuzzing & MTU Verification.

use arb_tx::kamino::{
    FLASH_BORROW_DISCRIMINATOR, FLASH_REPAY_DISCRIMINATOR, KAMINO_KLEND_PROGRAM_ID,
    KAMINO_MAIN_MARKET, KAMINO_WSOL_FEE_VAULT, KAMINO_WSOL_RESERVE, KAMINO_WSOL_SUPPLY_VAULT,
    derive_lending_market_authority,
};
use arb_tx::template::PreWarmedTxTemplate;
use arb_tx::{
    COMPUTE_BUDGET_LIMIT_2HOP, COMPUTE_BUDGET_LIMIT_3HOP, JITO_TIP_ACCOUNTS, KaminoBundleParams,
    KaminoFlashAccounts, KaminoFlashParams, KeypairSigner, MAX_COMPUTE_UNIT_LIMIT,
    SOLANA_PACKET_MTU_LIMIT, SYSVAR_INSTRUCTIONS_ID, TOKEN_PROGRAM_ID, TxBuilder, TxError,
    WSOL_MINT, get_cu_limit_for_hops,
};
use arb_types::{
    Bps, DexProtocol, ExecutionOpportunity, Lamports, MintId, PoolId, RouteId, Slot, SwapHop,
    TokenAmount,
};
use ed25519_dalek::{Signature, Verifier};
use solana_pubkey::Pubkey;
use std::str::FromStr;
use std::time::Instant;

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

fn create_n_hop_opportunity(hops_count: usize, size_lamports: u64) -> ExecutionOpportunity {
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    let usdc = Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
    let bonk = Pubkey::from_str("DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263").unwrap();
    let ray = Pubkey::from_str("4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R").unwrap();

    let tokens = [wsol, usdc, bonk, ray, wsol];
    let mut hops = Vec::with_capacity(hops_count);

    for i in 0..hops_count {
        let mint_out = if i + 1 == hops_count {
            tokens[0] // close the route back to the borrowed mint
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

// ---------------------------------------------------------------------------
// TEST 1: INSTRUCTION INDEXING & TOP-LEVEL INTROSPECTION ALIGNMENT
// ---------------------------------------------------------------------------
#[test]
fn test_adversarial_instruction_indexing_2hop_and_3hop() {
    let signer = create_test_signer(10);
    let payer = signer.pubkey();
    let tip_acc = Pubkey::from_str(JITO_TIP_ACCOUNTS[0]).unwrap();
    let kamino_params = KaminoFlashParams::default();

    // 1.1: 2-Hop Bundle
    let opp_2hop = create_n_hop_opportunity(2, 5_000_000_000);
    let ixs_2hop = TxBuilder::assemble_atomic_bundle(
        payer,
        tip_acc,
        Lamports(50_000),
        1_000,
        &opp_2hop,
        &kamino_params,
        576_460_752_303_423, // fee_sf: 5 bps in U68F60 (test fixture)
        TokenAmount(30_000), // min_profit_raw in base-mint (WSOL) units
    )
    .expect("2-hop bundle assembly must succeed");

    assert_eq!(
        ixs_2hop.len(),
        7,
        "2-hop bundle must have exactly 7 instructions"
    );

    // Borrow MUST be strictly at index 2
    let borrow_ix = &ixs_2hop[2];
    assert_eq!(
        borrow_ix.program_id,
        Pubkey::from_str(KAMINO_KLEND_PROGRAM_ID).unwrap()
    );
    assert_eq!(&borrow_ix.data[..8], &FLASH_BORROW_DISCRIMINATOR);

    // Repay MUST be strictly at index 5 and specify borrow_instruction_index = 2
    let repay_ix = &ixs_2hop[5];
    assert_eq!(
        repay_ix.program_id,
        Pubkey::from_str(KAMINO_KLEND_PROGRAM_ID).unwrap()
    );
    assert_eq!(&repay_ix.data[..8], &FLASH_REPAY_DISCRIMINATOR);
    assert_eq!(
        repay_ix.data[16], 2,
        "Repay borrow_instruction_index must strictly be 2"
    );

    // 1.2: 3-Hop Bundle
    let opp_3hop = create_n_hop_opportunity(3, 10_000_000_000);
    let ixs_3hop = TxBuilder::assemble_atomic_bundle(
        payer,
        tip_acc,
        Lamports(100_000),
        2_500,
        &opp_3hop,
        &kamino_params,
        576_460_752_303_423, // fee_sf: 5 bps in U68F60 (test fixture)
        TokenAmount(30_000), // min_profit_raw in base-mint (WSOL) units
    )
    .expect("3-hop bundle assembly must succeed");

    assert_eq!(
        ixs_3hop.len(),
        8,
        "3-hop bundle must have exactly 8 instructions"
    );

    // In 3-hop, Borrow is still strictly at index 2
    let borrow_ix_3hop = &ixs_3hop[2];
    assert_eq!(
        borrow_ix_3hop.program_id,
        Pubkey::from_str(KAMINO_KLEND_PROGRAM_ID).unwrap()
    );
    assert_eq!(&borrow_ix_3hop.data[..8], &FLASH_BORROW_DISCRIMINATOR);

    // Repay is at index 6, but its borrow_instruction_index MUST STILL be 2!
    let repay_ix_3hop = &ixs_3hop[6];
    assert_eq!(
        repay_ix_3hop.program_id,
        Pubkey::from_str(KAMINO_KLEND_PROGRAM_ID).unwrap()
    );
    assert_eq!(&repay_ix_3hop.data[..8], &FLASH_REPAY_DISCRIMINATOR);
    assert_eq!(
        repay_ix_3hop.data[16], 2,
        "3-hop Repay borrow_instruction_index must remain 2"
    );
}

// ---------------------------------------------------------------------------
// TEST 2: ZERO TIP AND PRIORITY FEE VARIATIONS ON INDEXING
// ---------------------------------------------------------------------------
#[test]
fn test_instruction_indexing_with_zero_priority_fee_and_zero_tip() {
    let signer = create_test_signer(11);
    let payer = signer.pubkey();
    let tip_acc = Pubkey::from_str(JITO_TIP_ACCOUNTS[1]).unwrap();
    let kamino_params = KaminoFlashParams::default();
    let opp = create_n_hop_opportunity(2, 2_000_000_000);

    // Case A: Zero priority fee (bundle-only deduplication)
    let ixs_zero_pf = TxBuilder::assemble_atomic_bundle(
        payer,
        tip_acc,
        Lamports(50_000),
        0, // 0 priority fee
        &opp,
        &kamino_params,
        576_460_752_303_423, // fee_sf: 5 bps in U68F60 (test fixture)
        TokenAmount(30_000), // min_profit_raw in base-mint (WSOL) units
    )
    .expect("assembly with 0 priority fee should succeed");

    assert_eq!(ixs_zero_pf.len(), 7);
    assert_eq!(ixs_zero_pf[1].data[0], 3); // SetComputeUnitPrice instruction present
    assert_eq!(&ixs_zero_pf[1].data[1..9], &0u64.to_le_bytes());
    // Crucial: Borrow index is STILL 2 because SetComputeUnitPrice is not omitted
    assert_eq!(&ixs_zero_pf[2].data[..8], &FLASH_BORROW_DISCRIMINATOR);
    assert_eq!(ixs_zero_pf[5].data[16], 2);

    // Case B: Zero Tip Amount
    let ixs_zero_tip = TxBuilder::assemble_atomic_bundle(
        payer,
        tip_acc,
        Lamports(0), // 0 tip
        1000,
        &opp,
        &kamino_params,
        576_460_752_303_423, // fee_sf: 5 bps in U68F60 (test fixture)
        TokenAmount(30_000), // min_profit_raw in base-mint (WSOL) units
    )
    .expect("assembly with 0 tip should succeed");

    // Tip instruction is omitted when tip == 0
    assert_eq!(
        ixs_zero_tip.len(),
        6,
        "Without tip, bundle has 6 instructions"
    );
    // Borrow is still index 2, repay is index 5 pointing to 2
    assert_eq!(&ixs_zero_tip[2].data[..8], &FLASH_BORROW_DISCRIMINATOR);
    assert_eq!(ixs_zero_tip[5].data[16], 2);
}

// ---------------------------------------------------------------------------
// TEST 3: ACCOUNTMETAS ORDERING & ANCHOR IDL VERIFICATION
// ---------------------------------------------------------------------------
#[test]
fn test_adversarial_account_metas_matrix() {
    let user = Pubkey::new_from_array([55; 32]);
    let ata = Pubkey::new_from_array([66; 32]);
    let accounts = KaminoFlashAccounts::new_wsol_main_market(user, ata);
    let borrow_ix = arb_tx::kamino::build_kamino_flash_borrow(&accounts, 1_000_000_000);
    let repay_ix = arb_tx::kamino::build_kamino_flash_repay(&accounts, 1_000_000_000, 2);

    // 1. Account count MUST be exactly 12
    assert_eq!(borrow_ix.accounts.len(), 12);
    assert_eq!(repay_ix.accounts.len(), 12);

    // 2. Kamino on-chain requirement: Borrow accounts pubkeys MUST MATCH Repay accounts pubkeys exactly
    for i in 0..12 {
        assert_eq!(
            borrow_ix.accounts[i].pubkey, repay_ix.accounts[i].pubkey,
            "Pubkey mismatch between borrow and repay at index {i}"
        );
    }

    // 3. Verify specific account roles and flags against Anchor IDL:
    // [0] userTransferAuthority: Signer, Readonly
    assert_eq!(borrow_ix.accounts[0].pubkey, user);
    assert!(borrow_ix.accounts[0].is_signer);
    assert!(!borrow_ix.accounts[0].is_writable);
    assert!(repay_ix.accounts[0].is_signer);
    assert!(!repay_ix.accounts[0].is_writable);

    // [1] lendingMarketAuthority: Non-signer, Readonly (PDA)
    let market = Pubkey::from_str(KAMINO_MAIN_MARKET).unwrap();
    let klend = Pubkey::from_str(KAMINO_KLEND_PROGRAM_ID).unwrap();
    let (expected_lma, bump) = derive_lending_market_authority(&market, &klend);
    assert_eq!(bump, 248);
    assert_eq!(borrow_ix.accounts[1].pubkey, expected_lma);
    assert!(!borrow_ix.accounts[1].is_signer);
    assert!(!borrow_ix.accounts[1].is_writable);

    // [2] lendingMarket: Non-signer, Readonly
    assert_eq!(borrow_ix.accounts[2].pubkey, market);
    assert!(!borrow_ix.accounts[2].is_signer);
    assert!(!borrow_ix.accounts[2].is_writable);

    // [3] reserve: Non-signer, Writable (mut)
    assert_eq!(
        borrow_ix.accounts[3].pubkey,
        Pubkey::from_str(KAMINO_WSOL_RESERVE).unwrap()
    );
    assert!(!borrow_ix.accounts[3].is_signer);
    assert!(borrow_ix.accounts[3].is_writable);
    assert!(repay_ix.accounts[3].is_writable);

    // [4] reserveLiquidityMint: Non-signer, Readonly
    assert_eq!(
        borrow_ix.accounts[4].pubkey,
        Pubkey::from_str(WSOL_MINT).unwrap()
    );
    assert!(!borrow_ix.accounts[4].is_signer);
    assert!(!borrow_ix.accounts[4].is_writable);

    // [5] reserveSupplyVault: Non-signer, Writable (mut)
    assert_eq!(
        borrow_ix.accounts[5].pubkey,
        Pubkey::from_str(KAMINO_WSOL_SUPPLY_VAULT).unwrap()
    );
    assert!(!borrow_ix.accounts[5].is_signer);
    assert!(borrow_ix.accounts[5].is_writable);
    assert!(repay_ix.accounts[5].is_writable);

    // [6] userLiquidityAta: Non-signer, Writable (mut)
    assert_eq!(borrow_ix.accounts[6].pubkey, ata);
    assert!(!borrow_ix.accounts[6].is_signer);
    assert!(borrow_ix.accounts[6].is_writable);
    assert!(repay_ix.accounts[6].is_writable);

    // [7] reserveFeeVault: Non-signer, Writable (mut)
    assert_eq!(
        borrow_ix.accounts[7].pubkey,
        Pubkey::from_str(KAMINO_WSOL_FEE_VAULT).unwrap()
    );
    assert!(!borrow_ix.accounts[7].is_signer);
    assert!(borrow_ix.accounts[7].is_writable);
    assert!(repay_ix.accounts[7].is_writable);

    // [8] referrerTokenState: Non-signer, Readonly (sentinel = klend program id when None)
    assert_eq!(borrow_ix.accounts[8].pubkey, klend);
    assert!(!borrow_ix.accounts[8].is_signer);
    assert!(!borrow_ix.accounts[8].is_writable);

    // [9] referrerAccount: Non-signer, Readonly (sentinel = klend program id when None)
    assert_eq!(borrow_ix.accounts[9].pubkey, klend);
    assert!(!borrow_ix.accounts[9].is_signer);
    assert!(!borrow_ix.accounts[9].is_writable);

    // [10] sysvarInstructions: Non-signer, Readonly
    assert_eq!(
        borrow_ix.accounts[10].pubkey,
        Pubkey::from_str(SYSVAR_INSTRUCTIONS_ID).unwrap()
    );
    assert!(!borrow_ix.accounts[10].is_signer);
    assert!(!borrow_ix.accounts[10].is_writable);

    // [11] tokenProgram: Non-signer, Readonly
    assert_eq!(
        borrow_ix.accounts[11].pubkey,
        Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap()
    );
    assert!(!borrow_ix.accounts[11].is_signer);
    assert!(!borrow_ix.accounts[11].is_writable);
}

// ---------------------------------------------------------------------------
// TEST 4: COMPUTE UNIT BUDGET BOUNDARIES & MAXIMUM LIMIT ENFORCEMENT
// ---------------------------------------------------------------------------
#[test]
fn test_adversarial_cu_budget_enforcement() {
    // 4.1 Strict hop mapping
    assert_eq!(get_cu_limit_for_hops(2), 260_000);
    assert_eq!(get_cu_limit_for_hops(3), 320_000);
    assert_eq!(COMPUTE_BUDGET_LIMIT_2HOP, 260_000);
    assert_eq!(COMPUTE_BUDGET_LIMIT_3HOP, 320_000);

    // 4.2 MAX_COMPUTE_UNIT_LIMIT must NOT clamp 320,000
    assert!(
        MAX_COMPUTE_UNIT_LIMIT >= 320_000,
        "MAX_COMPUTE_UNIT_LIMIT ({MAX_COMPUTE_UNIT_LIMIT}) must be >= 320,000 to prevent clamping"
    );

    // 4.3 assemble_atomic_bundle strictly sets the instruction data
    let signer = create_test_signer(12);
    let payer = signer.pubkey();
    let tip_acc = Pubkey::from_str(JITO_TIP_ACCOUNTS[0]).unwrap();
    let kamino_params = KaminoFlashParams::default();

    // 2-Hop
    let opp_2hop = create_n_hop_opportunity(2, 1_000_000_000);
    let ixs_2hop = TxBuilder::assemble_atomic_bundle(
        payer,
        tip_acc,
        Lamports(50_000),
        1_000,
        &opp_2hop,
        &kamino_params,
        576_460_752_303_423, // fee_sf: 5 bps in U68F60 (test fixture)
        TokenAmount(30_000), // min_profit_raw in base-mint (WSOL) units
    )
    .unwrap();
    let cu_bytes_2hop: [u8; 4] = ixs_2hop[0].data[1..5].try_into().unwrap();
    assert_eq!(u32::from_le_bytes(cu_bytes_2hop), 260_000);

    // 3-Hop
    let opp_3hop = create_n_hop_opportunity(3, 1_000_000_000);
    let ixs_3hop = TxBuilder::assemble_atomic_bundle(
        payer,
        tip_acc,
        Lamports(50_000),
        1_000,
        &opp_3hop,
        &kamino_params,
        576_460_752_303_423, // fee_sf: 5 bps in U68F60 (test fixture)
        TokenAmount(30_000), // min_profit_raw in base-mint (WSOL) units
    )
    .unwrap();
    let cu_bytes_3hop: [u8; 4] = ixs_3hop[0].data[1..5].try_into().unwrap();
    assert_eq!(u32::from_le_bytes(cu_bytes_3hop), 320_000);

    // 4.4 Boundary rejections: 0, 1, 4, 10 hops
    for invalid_hops in [0, 1, 4, 5, 10] {
        let opp_invalid = create_n_hop_opportunity(invalid_hops, 1_000_000_000);
        let err = TxBuilder::assemble_atomic_bundle(
            payer,
            tip_acc,
            Lamports(50_000),
            1_000,
            &opp_invalid,
            &kamino_params,
            576_460_752_303_423, // fee_sf: 5 bps in U68F60 (test fixture)
            TokenAmount(30_000), // min_profit_raw in base-mint (WSOL) units
        )
        .unwrap_err();

        match err {
            TxError::InvalidHopCount(h) => assert_eq!(h, invalid_hops),
            _ => panic!("Expected InvalidHopCount({invalid_hops}), got {err:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// TEST 5: KAMINO ON-CHAIN SYSVAR INTROSPECTION ORACLE EMULATION
// ---------------------------------------------------------------------------
/// Emulates the exact logic from Kamino Lend's `flash_ixs.rs`:
/// - `flash_borrow_checks_internal`
/// - `flash_repay_checks`
/// - `flash_borrow_check_matching_repay`
fn kamino_onchain_introspection_oracle(
    instructions: &[arb_tx::CompiledInstruction],
) -> Result<(), &'static str> {
    let klend_program_id = Pubkey::from_str(KAMINO_KLEND_PROGRAM_ID).unwrap();

    // 1. Find flash borrow
    let mut borrow_info: Option<(usize, u64, &arb_tx::CompiledInstruction)> = None;
    for (idx, ix) in instructions.iter().enumerate() {
        if ix.program_id == klend_program_id && ix.data.starts_with(&FLASH_BORROW_DISCRIMINATOR) {
            if borrow_info.is_some() {
                return Err("Multiple flash borrows not allowed");
            }
            if ix.data.len() != 16 {
                return Err("Flash borrow data length != 16");
            }
            let amount = u64::from_le_bytes(ix.data[8..16].try_into().unwrap());
            borrow_info = Some((idx, amount, ix));
        }
    }

    let (borrow_idx, borrow_amount, borrow_ix) =
        borrow_info.ok_or("No flash borrow found in transaction")?;

    // 2. Find matching flash repay
    let mut repay_info: Option<(usize, u64, u8, &arb_tx::CompiledInstruction)> = None;
    for (idx, ix) in instructions.iter().enumerate() {
        if ix.program_id == klend_program_id && ix.data.starts_with(&FLASH_REPAY_DISCRIMINATOR) {
            if repay_info.is_some() {
                return Err("Multiple flash repays not allowed");
            }
            if ix.data.len() != 17 {
                return Err("Flash repay data length != 17");
            }
            let amount = u64::from_le_bytes(ix.data[8..16].try_into().unwrap());
            let borrow_instruction_index = ix.data[16];
            repay_info = Some((idx, amount, borrow_instruction_index, ix));
        }
    }

    let (repay_idx, repay_amount, borrow_instruction_index, repay_ix) =
        repay_info.ok_or("No flash repay found in transaction")?;

    // 3. Repay must appear after borrow
    if repay_idx <= borrow_idx {
        return Err("Flash repay must appear after flash borrow");
    }

    // 4. Repay borrow_instruction_index MUST match actual borrow index
    if borrow_instruction_index as usize != borrow_idx {
        return Err("borrow_instruction_index does not match actual borrow instruction index");
    }

    // 5. Repay liquidity_amount MUST be >= borrow liquidity_amount (fee included)
    if repay_amount < borrow_amount {
        return Err("Repay liquidity amount is less than borrow amount");
    }

    // 6. Number of accounts must match
    if borrow_ix.accounts.len() != repay_ix.accounts.len() {
        return Err("Account count mismatch between borrow and repay");
    }

    // 7. Every account pubkey must match in order
    for (b_acc, r_acc) in borrow_ix.accounts.iter().zip(repay_ix.accounts.iter()) {
        if b_acc.pubkey != r_acc.pubkey {
            return Err("Account pubkey mismatch between borrow and repay");
        }
    }

    // 8. Reserve account at index 3 must match
    if borrow_ix.accounts[3].pubkey != repay_ix.accounts[3].pubkey {
        return Err("Invalid reserve account on flash repay");
    }

    Ok(())
}

#[test]
fn test_oracle_verification_of_atomic_bundles() {
    let signer = create_test_signer(20);
    let payer = signer.pubkey();
    let tip_acc = Pubkey::from_str(JITO_TIP_ACCOUNTS[0]).unwrap();
    let kamino_params = KaminoFlashParams::default();

    // Test 2-Hop bundle against Kamino Oracle
    let opp_2hop = create_n_hop_opportunity(2, 10_000_000_000);
    let ixs_2hop = TxBuilder::assemble_atomic_bundle(
        payer,
        tip_acc,
        Lamports(50_000),
        1_000,
        &opp_2hop,
        &kamino_params,
        576_460_752_303_423, // fee_sf: 5 bps in U68F60 (test fixture)
        TokenAmount(30_000), // min_profit_raw in base-mint (WSOL) units
    )
    .unwrap();

    assert!(
        kamino_onchain_introspection_oracle(&ixs_2hop).is_ok(),
        "2-Hop bundle MUST pass Kamino on-chain oracle verification"
    );

    // Test 3-Hop bundle against Kamino Oracle
    let opp_3hop = create_n_hop_opportunity(3, 20_000_000_000);
    let ixs_3hop = TxBuilder::assemble_atomic_bundle(
        payer,
        tip_acc,
        Lamports(50_000),
        1_000,
        &opp_3hop,
        &kamino_params,
        576_460_752_303_423, // fee_sf: 5 bps in U68F60 (test fixture)
        TokenAmount(30_000), // min_profit_raw in base-mint (WSOL) units
    )
    .unwrap();

    assert!(
        kamino_onchain_introspection_oracle(&ixs_3hop).is_ok(),
        "3-Hop bundle MUST pass Kamino on-chain oracle verification"
    );

    // Adversarial Mutation A: Tamper with borrow_instruction_index (e.g. set to 1)
    let mut tampered_ixs = ixs_2hop.clone();
    tampered_ixs[5].data[16] = 1; // Corrupt index
    assert_eq!(
        kamino_onchain_introspection_oracle(&tampered_ixs),
        Err("borrow_instruction_index does not match actual borrow instruction index")
    );

    // Adversarial Mutation B: Prepend a dummy instruction (shifting borrow to index 3)
    let mut shifted_ixs = vec![ixs_2hop[0].clone()];
    shifted_ixs.extend(ixs_2hop.clone());
    // Borrow is now at index 3, but repay still specifies index 2
    assert_eq!(
        kamino_onchain_introspection_oracle(&shifted_ixs),
        Err("borrow_instruction_index does not match actual borrow instruction index")
    );

    // Adversarial Mutation C: Mismatch account pubkey at index 6 (user ATA)
    let mut mismatched_accounts = ixs_2hop.clone();
    mismatched_accounts[5].accounts[6].pubkey = Pubkey::new_from_array([99; 32]);
    assert_eq!(
        kamino_onchain_introspection_oracle(&mismatched_accounts),
        Err("Account pubkey mismatch between borrow and repay")
    );
}

// ---------------------------------------------------------------------------
// TEST 6: PRE-WARMED TEMPLATE FUZZING & DALEK CRYPTOGRAPHIC VERIFICATION
// ---------------------------------------------------------------------------
#[test]
fn test_prewarmed_template_fuzzing_and_cryptographic_soundness() {
    let signer = create_test_signer(77);
    let payer = signer.pubkey();
    let tip_acc = Pubkey::from_str(JITO_TIP_ACCOUNTS[3]).unwrap();
    let kamino_params = KaminoFlashParams::default();
    let opp = create_n_hop_opportunity(2, 5_000_000_000);
    let alt = TxBuilder::create_default_mainnet_alt();

    let bundle_params = KaminoBundleParams {
        payer,
        signer: &signer,
        tip_account: tip_acc,
        tip_lamports: Lamports(50_000),
        priority_fee_micro_lamports: 1_000,
        opportunity: &opp,
        kamino_params: &kamino_params,
        recent_blockhash: [0x11; 32],
        lookup_tables: &[alt],
        flash_loan_fee_sf: 0,
    };

    let mut template =
        PreWarmedTxTemplate::from_bundle_params(&bundle_params, 576_460_752_303_423).unwrap();
    assert!(template.raw_bytes.len() <= SOLANA_PACKET_MTU_LIMIT);

    let borrow_offset = template.borrow_amount_offset;
    let tip_offset = template.tip_amount_offset;
    let bh_offset = template.blockhash_offset;
    let msg_offset = template.message_offset;

    // Stress test with 5 diverse mutations (boundary values, max u64, small values)
    let test_vectors: Vec<(u64, u64, u8)> = vec![
        (1, 10_000, 0x01),
        (50_000, 30_000, 0x02),
        (10_000_000_000, 100_000, 0x03),
        (100_000_000_000, 1_000_000, 0x04),
        (u64::MAX - 100_000, 500_000, 0x05),
    ];

    for (borrow_val, tip_val, bh_byte) in test_vectors {
        let new_bh = [bh_byte; 32];
        let wire = template.patch_and_sign(borrow_val, tip_val, new_bh, &signer);

        // Assert wire size fits MTU
        assert!(
            wire.len() <= SOLANA_PACKET_MTU_LIMIT,
            "Wire length {} exceeds MTU limit {}",
            wire.len(),
            SOLANA_PACKET_MTU_LIMIT
        );

        // Verify borrow amount mutated
        let read_borrow =
            u64::from_le_bytes(wire[borrow_offset..borrow_offset + 8].try_into().unwrap());
        assert_eq!(read_borrow, borrow_val);

        // Verify tip mutated
        let read_tip = u64::from_le_bytes(wire[tip_offset..tip_offset + 8].try_into().unwrap());
        assert_eq!(read_tip, tip_val);

        // Verify blockhash mutated
        let read_bh = &wire[bh_offset..bh_offset + 32];
        assert_eq!(read_bh, &new_bh);

        // Verify Dalek signature cryptographically validates over message
        let sig_bytes: [u8; 64] = wire[1..65].try_into().unwrap();
        let sig = Signature::from_bytes(&sig_bytes);
        let msg = &wire[msg_offset..];
        assert!(
            signer.verifying_key().verify(msg, &sig).is_ok(),
            "Signature must cryptographically verify over mutated message"
        );
    }
}

// ---------------------------------------------------------------------------
// TEST 7: SUB-MICROSECOND HOT-PATH BENCHMARK (EMPIRICAL MEASUREMENT)
// ---------------------------------------------------------------------------
#[test]
fn test_prewarmed_template_hotpath_sub_microsecond_benchmark() {
    let signer = create_test_signer(99);
    let payer = signer.pubkey();
    let tip_acc = Pubkey::from_str(JITO_TIP_ACCOUNTS[4]).unwrap();
    let kamino_params = KaminoFlashParams::default();
    let opp = create_n_hop_opportunity(2, 8_000_000_000);
    let alt = TxBuilder::create_default_mainnet_alt();

    let bundle_params = KaminoBundleParams {
        payer,
        signer: &signer,
        tip_account: tip_acc,
        tip_lamports: Lamports(50_000),
        priority_fee_micro_lamports: 1_000,
        opportunity: &opp,
        kamino_params: &kamino_params,
        recent_blockhash: [0xEE; 32],
        lookup_tables: &[alt],
        flash_loan_fee_sf: 0,
    };

    let mut template =
        PreWarmedTxTemplate::from_bundle_params(&bundle_params, 576_460_752_303_423).unwrap();

    // Warm up CPU cache
    for i in 0..100 {
        template.patch_and_sign(1_000_000 + i, 50_000, [i as u8; 32], &signer);
    }

    // Benchmark 1,000 in-place patch_and_sign iterations
    let iterations = 1000;
    let start = Instant::now();
    for i in 0..iterations {
        template.patch_and_sign(
            10_000_000_000 + i as u64,
            60_000 + i as u64,
            [(i % 256) as u8; 32],
            &signer,
        );
    }
    let elapsed = start.elapsed();
    let per_op = elapsed / iterations as u32;

    println!(
        "\n[BENCHMARK] 1,000 iterations of patch_and_sign took {elapsed:?} (avg per op: {per_op:?})"
    );
}
