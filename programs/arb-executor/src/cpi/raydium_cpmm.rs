//! Raydium CPMM SwapBaseInput CPI invocation builder.

use crate::error::ExecutorError;
use crate::validation::RAYDIUM_CPMM_PROGRAM_ID;
use solana_account_info::AccountInfo;
use solana_cpi::invoke;
use solana_instruction::{AccountMeta, Instruction};
use solana_program_error::ProgramResult;

/// Official Anchor discriminator for `swap_base_input`
pub const RAYDIUM_CPMM_SWAP_BASE_INPUT_DISCRIMINATOR: [u8; 8] =
    [143, 190, 90, 218, 196, 30, 51, 222];

/// Execute Raydium CPMM `swap_base_input` via CPI.
///
/// Expected accounts (13 swap accounts + 1 program account = 14 accounts):
/// - [0] Payer (signer, writable)
/// - [1] Authority (readonly)
/// - [2] AmmConfig (readonly)
/// - [3] PoolState (writable)
/// - [4] InputTokenAccount (writable)
/// - [5] OutputTokenAccount (writable)
/// - [6] InputVault (writable)
/// - [7] OutputVault (writable)
/// - [8] InputTokenProgram (readonly)
/// - [9] OutputTokenProgram (readonly)
/// - [10] InputTokenMint (readonly)
/// - [11] OutputTokenMint (readonly)
/// - [12] ObservationState (writable)
/// - [13] Raydium CPMM Program (readonly)
pub fn invoke_raydium_cpmm_swap<'a>(
    accounts: &'a [AccountInfo<'a>],
    amount_in: u64,
    min_amount_out: u64,
) -> ProgramResult {
    if accounts.len() < 14 {
        return Err(ExecutorError::NotEnoughAccountInfos.into());
    }

    let program_info = &accounts[13];
    if *program_info.key != RAYDIUM_CPMM_PROGRAM_ID {
        return Err(ExecutorError::UnauthorizedProgramId.into());
    }

    let mut data = Vec::with_capacity(24);
    data.extend_from_slice(&RAYDIUM_CPMM_SWAP_BASE_INPUT_DISCRIMINATOR);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&min_amount_out.to_le_bytes());

    let metas = vec![
        AccountMeta::new(*accounts[0].key, accounts[0].is_signer),
        AccountMeta::new_readonly(*accounts[1].key, false),
        AccountMeta::new_readonly(*accounts[2].key, false),
        AccountMeta::new(*accounts[3].key, false),
        AccountMeta::new(*accounts[4].key, false),
        AccountMeta::new(*accounts[5].key, false),
        AccountMeta::new(*accounts[6].key, false),
        AccountMeta::new(*accounts[7].key, false),
        AccountMeta::new_readonly(*accounts[8].key, false),
        AccountMeta::new_readonly(*accounts[9].key, false),
        AccountMeta::new_readonly(*accounts[10].key, false),
        AccountMeta::new_readonly(*accounts[11].key, false),
        AccountMeta::new(*accounts[12].key, false),
    ];

    let instruction = Instruction {
        program_id: *program_info.key,
        accounts: metas,
        data,
    };

    invoke(&instruction, &accounts[..14])
}
