//! ATA derivation, idempotent ATA creation, token-account close, sync-native
//! and system transfer helpers.

use super::*;

/// Derive Associated Token Address (ATA) deterministically for a wallet and mint.
pub fn get_associated_token_address(wallet: &Pubkey, mint: &Pubkey) -> Pubkey {
    let token_program = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
    let ata_program = Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM_ID).unwrap();
    let seeds = &[wallet.as_ref(), token_program.as_ref(), mint.as_ref()];
    let (ata, _) = Pubkey::find_program_address(seeds, &ata_program);
    ata
}

/// Derive Associated Token Address (ATA) deterministically for a wallet, mint, and token program (SPL Token or Token-2022).
pub fn get_associated_token_address_with_program_id(
    wallet: &Pubkey,
    mint: &Pubkey,
    token_program: &Pubkey,
) -> Pubkey {
    let ata_program = Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM_ID).unwrap();
    let seeds = &[wallet.as_ref(), token_program.as_ref(), mint.as_ref()];
    let (ata, _) = Pubkey::find_program_address(seeds, &ata_program);
    ata
}

/// Build idempotent ATA creation instruction (CreateIdempotent opcode 1).
/// Safely initializes intermediate token account if missing, does not fail if it already exists.
pub fn build_create_ata_idempotent(
    payer: Pubkey,
    wallet_owner: Pubkey,
    token_mint: Pubkey,
    associated_token_account: Pubkey,
) -> CompiledInstruction {
    let ata_program = Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM_ID).unwrap();
    let token_program = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
    let system_program = Pubkey::new_from_array(SYSTEM_PROGRAM_ID);

    CompiledInstruction {
        program_id: ata_program,
        accounts: vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(associated_token_account, false),
            AccountMeta::new_readonly(wallet_owner, false),
            AccountMeta::new_readonly(token_mint, false),
            AccountMeta::new_readonly(system_program, false),
            AccountMeta::new_readonly(token_program, false),
        ],
        data: vec![1], // CreateIdempotent opcode
    }
}

/// Build idempotent ATA creation instruction for a specific token program (SPL Token or Token-2022).
/// Data layout: [1] (CreateIdempotent opcode). Account order matches the ATA program.
pub fn build_create_ata_idempotent_for_program(
    payer: Pubkey,
    wallet_owner: Pubkey,
    token_mint: Pubkey,
    associated_token_account: Pubkey,
    token_program: Pubkey,
) -> CompiledInstruction {
    let ata_program = Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM_ID).unwrap();
    let system_program = Pubkey::new_from_array(SYSTEM_PROGRAM_ID);

    CompiledInstruction {
        program_id: ata_program,
        accounts: vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(associated_token_account, false),
            AccountMeta::new_readonly(wallet_owner, false),
            AccountMeta::new_readonly(token_mint, false),
            AccountMeta::new_readonly(system_program, false),
            AccountMeta::new_readonly(token_program, false),
        ],
        data: vec![1],
    }
}

/// Build SPL Token / Token-2022 `CloseAccount` instruction for a specific token program.
/// Data layout: [9] (CloseAccount opcode).
pub fn build_close_token_account_for_program(
    account_to_close: Pubkey,
    destination_rent_receiver: Pubkey,
    owner_authority: Pubkey,
    token_program: Pubkey,
) -> CompiledInstruction {
    CompiledInstruction {
        program_id: token_program,
        accounts: vec![
            AccountMeta::new(account_to_close, false),
            AccountMeta::new(destination_rent_receiver, false),
            AccountMeta::new_readonly(owner_authority, true),
        ],
        data: vec![9],
    }
}

/// Build SPL Token close_account instruction (opcode 9) to reclaim 100% of the rent deposit back to payer.
pub fn build_close_token_account(
    account_to_close: Pubkey,
    destination_rent_receiver: Pubkey,
    owner_authority: Pubkey,
) -> CompiledInstruction {
    let token_program = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();

    CompiledInstruction {
        program_id: token_program,
        accounts: vec![
            AccountMeta::new(account_to_close, false),
            AccountMeta::new(destination_rent_receiver, false),
            AccountMeta::new_readonly(owner_authority, true),
        ],
        data: vec![9], // CloseAccount opcode
    }
}

/// Build SPL Token sync_native instruction (opcode 17).
pub fn build_sync_native(account: Pubkey) -> CompiledInstruction {
    let token_program = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
    CompiledInstruction {
        program_id: token_program,
        accounts: vec![AccountMeta::new(account, false)],
        data: vec![17], // SyncNative opcode
    }
}

/// Build standard System Program transfer instruction.
pub fn build_system_transfer(from: Pubkey, to: Pubkey, lamports: u64) -> CompiledInstruction {
    let mut data = Vec::with_capacity(12);
    data.extend_from_slice(&2u32.to_le_bytes()); // SystemProgram::Transfer instruction index = 2
    data.extend_from_slice(&lamports.to_le_bytes());
    CompiledInstruction {
        program_id: Pubkey::new_from_array(SYSTEM_PROGRAM_ID),
        accounts: vec![AccountMeta::new(from, true), AccountMeta::new(to, false)],
        data,
    }
}
