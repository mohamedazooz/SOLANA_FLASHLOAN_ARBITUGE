//! Raydium CLMM Swap CPI invocation builder.

use crate::error::ExecutorError;
use crate::validation::RAYDIUM_CLMM_PROGRAM_ID;
use solana_account_info::AccountInfo;
use solana_cpi::invoke;
use solana_instruction::{AccountMeta, Instruction};
use solana_program_error::ProgramResult;

/// Official Anchor discriminator for Raydium CLMM `swap`
pub const RAYDIUM_CLMM_SWAP_DISCRIMINATOR: [u8; 8] = [248, 198, 158, 145, 225, 117, 134, 201];

/// Execute Raydium CLMM `swap` via CPI.
///
/// Expected accounts (12 swap accounts + 1 program account = 13 accounts):
/// - [0] Payer (signer, writable)
/// - [1] AmmConfig (readonly)
/// - [2] PoolState (writable)
/// - [3] InputTokenAccount (writable)
/// - [4] OutputTokenAccount (writable)
/// - [5] InputVault (writable)
/// - [6] OutputVault (writable)
/// - [7] ObservationState (writable)
/// - [8] TokenProgram (readonly)
/// - [9] TickArray0 (writable)
/// - [10] TickArray1 (writable)
/// - [11] TickArray2 (writable)
/// - [12] Raydium CLMM Program (readonly)
pub fn invoke_raydium_clmm_swap<'a>(
    accounts: &'a [AccountInfo<'a>],
    amount_in: u64,
    min_amount_out: u64,
    sqrt_price_limit_x64: u128,
    is_base_input: bool,
) -> ProgramResult {
    if accounts.len() < 13 {
        return Err(ExecutorError::NotEnoughAccountInfos.into());
    }

    let program_info = &accounts[12];
    if *program_info.key != RAYDIUM_CLMM_PROGRAM_ID {
        return Err(ExecutorError::UnauthorizedProgramId.into());
    }

    let mut data = Vec::with_capacity(41);
    data.extend_from_slice(&RAYDIUM_CLMM_SWAP_DISCRIMINATOR);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&min_amount_out.to_le_bytes());
    data.extend_from_slice(&sqrt_price_limit_x64.to_le_bytes());
    data.push(is_base_input as u8);

    let metas = vec![
        AccountMeta::new(*accounts[0].key, accounts[0].is_signer),
        AccountMeta::new_readonly(*accounts[1].key, false),
        AccountMeta::new(*accounts[2].key, false),
        AccountMeta::new(*accounts[3].key, false),
        AccountMeta::new(*accounts[4].key, false),
        AccountMeta::new(*accounts[5].key, false), // InputVault
        AccountMeta::new(*accounts[6].key, false), // OutputVault
        AccountMeta::new(*accounts[7].key, false),
        AccountMeta::new_readonly(*accounts[8].key, false),
        AccountMeta::new(*accounts[9].key, false),
        AccountMeta::new(*accounts[10].key, false),
        AccountMeta::new(*accounts[11].key, false),
    ];

    let instruction = Instruction {
        program_id: *program_info.key,
        accounts: metas,
        data,
    };

    invoke(&instruction, &accounts[..13])
}
