//! Feature 3: Kamino Repayment Instruction & Sysvar Index Matching — Tier 2 Boundary Tests.
//!
//! Validates sysvar instruction introspection, borrow_instruction_index matching,
//! privilege constraints (sysvar cannot be signer), and payload length boundary conditions.

use arb_constants::{FLASH_BORROW_DISCRIMINATOR, FLASH_REPAY_DISCRIMINATOR};
use arb_tx::{
    ExecutionMode, KAMINO_KLEND_PROGRAM_ID, KaminoFlashAccounts,
    flash::kamino::{build_kamino_flash_borrow, build_kamino_flash_repay},
    primitives::instruction::CompiledInstruction,
    verify_atomic_integrity,
};
use solana_pubkey::Pubkey;
use std::str::FromStr;

/// Oracle that verifies Sysvar1nstructions-based introspection logic as enforced by klend on-chain.
fn introspect_kamino_repay_instruction(
    instructions: &[CompiledInstruction],
    repay_idx: usize,
) -> Result<(), &'static str> {
    if repay_idx >= instructions.len() {
        return Err("repay index out of bounds");
    }
    let repay_ix = &instructions[repay_idx];
    if repay_ix.data.len() < 17 {
        return Err("repay payload too short (need 17 bytes)");
    }
    if repay_ix.data[..8] != FLASH_REPAY_DISCRIMINATOR {
        return Err("invalid repay discriminator");
    }

    let borrow_idx = repay_ix.data[16] as usize;
    if borrow_idx >= instructions.len() {
        return Err("borrow_instruction_index out of bounds");
    }
    if borrow_idx >= repay_idx {
        return Err("borrow_instruction_index must strictly precede repay");
    }

    let borrow_ix = &instructions[borrow_idx];
    if borrow_ix.data.len() < 16 {
        return Err("targeted borrow payload too short");
    }
    if borrow_ix.data[..8] != FLASH_BORROW_DISCRIMINATOR {
        return Err("targeted instruction is not a flash borrow");
    }

    let borrow_amount = u64::from_le_bytes(borrow_ix.data[8..16].try_into().unwrap());
    let repay_principal = u64::from_le_bytes(repay_ix.data[8..16].try_into().unwrap());
    if repay_principal != borrow_amount {
        return Err("repay principal does not match borrow amount");
    }

    Ok(())
}

fn create_sample_borrow_and_repay(
    user_authority: Pubkey,
    borrow_amount: u64,
    borrow_idx: u8,
) -> (CompiledInstruction, CompiledInstruction) {
    let klend = Pubkey::from_str(KAMINO_KLEND_PROGRAM_ID).unwrap();
    let accounts = KaminoFlashAccounts {
        user_authority,
        lending_market: Pubkey::new_unique(),
        lending_market_authority: Pubkey::new_unique(),
        reserve: Pubkey::new_unique(),
        reserve_liquidity_mint: Pubkey::new_unique(),
        reserve_supply_vault: Pubkey::new_unique(),
        user_liquidity_ata: Pubkey::new_unique(),
        reserve_fee_vault: Pubkey::new_unique(),
        referrer_token_state: None,
        referrer_account: None,
        sysvar_instructions: Pubkey::from_str_const(arb_constants::SYSVAR_INSTRUCTIONS_ID),
        token_program: Pubkey::from_str_const(arb_constants::TOKEN_PROGRAM_ID),
        klend_program_id: klend,
    };

    let borrow = build_kamino_flash_borrow(&accounts, borrow_amount);
    let repay = build_kamino_flash_repay(&accounts, borrow_amount, borrow_idx);
    (borrow, repay)
}

/// Boundary Case 1: Repay borrow_instruction_index points to itself.
#[test]
fn test_boundary_f3_repay_borrow_index_points_to_self() {
    assert_eq!(
        FLASH_REPAY_DISCRIMINATOR,
        [185, 117, 0, 203, 96, 245, 180, 186]
    );
    let user = Pubkey::new_unique();
    let (borrow, mut repay) = create_sample_borrow_and_repay(user, 1_000_000_000, 2);
    // Tamper repay to point to its own index (e.g. index 3)
    repay.data[16] = 3;

    let dummy_cu = CompiledInstruction {
        program_id: Pubkey::new_from_array(arb_constants::COMPUTE_BUDGET_PROGRAM_ID),
        accounts: vec![],
        data: vec![0u8; 16],
    };
    let dummy_swap = CompiledInstruction {
        program_id: Pubkey::new_unique(),
        accounts: vec![],
        data: vec![0u8; 10],
    };

    let ixs = vec![dummy_cu, dummy_swap, borrow, repay];
    // Repay is at index 3, pointing to index 3
    assert_eq!(
        introspect_kamino_repay_instruction(&ixs, 3),
        Err("borrow_instruction_index must strictly precede repay")
    );
}

/// Boundary Case 2: Repay borrow_instruction_index out of bounds (pointing past end of bundle).
#[test]
fn test_boundary_f3_repay_borrow_index_out_of_bounds() {
    let user = Pubkey::new_unique();
    let (borrow, mut repay) = create_sample_borrow_and_repay(user, 1_000_000_000, 255);
    // Point index to 255
    repay.data[16] = 255;

    let ixs = vec![borrow, repay];
    assert_eq!(
        introspect_kamino_repay_instruction(&ixs, 1),
        Err("borrow_instruction_index out of bounds")
    );
}

/// Boundary Case 3: Repay borrow_instruction_index points to a non-borrow instruction.
#[test]
fn test_boundary_f3_repay_borrow_index_points_to_non_borrow_ix() {
    let user = Pubkey::new_unique();
    let (borrow, mut repay) = create_sample_borrow_and_repay(user, 1_000_000_000, 0);
    // Set index to 0, which is ComputeBudget
    repay.data[16] = 0;

    let dummy_cu = CompiledInstruction {
        program_id: Pubkey::new_from_array(arb_constants::COMPUTE_BUDGET_PROGRAM_ID),
        accounts: vec![],
        data: vec![0u8; 16],
    };

    let ixs = vec![dummy_cu, borrow, repay];
    assert_eq!(
        introspect_kamino_repay_instruction(&ixs, 2),
        Err("targeted instruction is not a flash borrow")
    );
}

/// Boundary Case 4: Sysvar instructions account marked is_signer = true triggers privilege rejection.
#[test]
fn test_boundary_f3_sysvar_privilege_violations() {
    let user = Pubkey::new_unique();
    let (mut borrow, repay) = create_sample_borrow_and_repay(user, 1_000_000_000, 0);

    // Corrupt sysvar_instructions (account index 10) to have is_signer = true
    borrow.accounts[10].is_signer = true;

    let ixs = vec![borrow, repay];
    let verification = verify_atomic_integrity(
        ExecutionMode::SingleTransaction,
        &ixs,
        Some(0),
        Some(1),
        1_000_000_000,
        1_000_100_000,
        100_000,
    )
    .unwrap();

    assert!(
        !verification.is_atomic,
        "sysvar with signer privilege must be rejected"
    );
    let reason = verification.rejection_reason.unwrap();
    assert!(
        reason.contains("incorrectly marked as signer"),
        "expected signer violation reason, got: {reason}"
    );
}

/// Boundary Case 5: Repay payload truncation boundaries (16 bytes, 8 bytes).
#[test]
fn test_boundary_f3_repay_payload_length_boundaries() {
    let user = Pubkey::new_unique();
    let (borrow, mut repay) = create_sample_borrow_and_repay(user, 1_000_000_000, 0);

    // Truncate repay to 16 bytes (missing trailing borrow index)
    repay.data.truncate(16);
    let ixs = vec![borrow.clone(), repay.clone()];
    assert_eq!(
        introspect_kamino_repay_instruction(&ixs, 1),
        Err("repay payload too short (need 17 bytes)")
    );

    // Truncate repay to 8 bytes (discriminator only)
    repay.data.truncate(8);
    let ixs2 = vec![borrow, repay];
    assert_eq!(
        introspect_kamino_repay_instruction(&ixs2, 1),
        Err("repay payload too short (need 17 bytes)")
    );
}

/// Boundary Case 6: Repay principal mismatch boundary (repay principal != borrow amount).
#[test]
fn test_boundary_f3_repay_principal_mismatch_boundary() {
    let user = Pubkey::new_unique();
    let (borrow, mut repay) = create_sample_borrow_and_repay(user, 1_000_000_000, 0);

    // Set repay principal to borrow_amount + 1 lamport
    let wrong_principal = 1_000_000_001u64;
    repay.data[8..16].copy_from_slice(&wrong_principal.to_le_bytes());

    let ixs = vec![borrow, repay];
    assert_eq!(
        introspect_kamino_repay_instruction(&ixs, 1),
        Err("repay principal does not match borrow amount")
    );
}
