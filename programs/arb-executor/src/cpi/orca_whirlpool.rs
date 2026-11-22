//! Orca Whirlpool Swap CPI invocation builder.

use crate::error::ExecutorError;
use crate::validation::ORCA_WHIRLPOOL_PROGRAM_ID;
use solana_account_info::AccountInfo;
use solana_cpi::invoke;
use solana_instruction::{AccountMeta, Instruction};
use solana_program_error::ProgramResult;

/// Official Anchor discriminator for Orca Whirlpool `swap`
pub const ORCA_WHIRLPOOL_SWAP_DISCRIMINATOR: [u8; 8] = [248, 198, 158, 145, 225, 117, 135, 200];

/// Execute Orca Whirlpool `swap` via CPI.
///
/// Expected accounts (11 swap accounts + 1 program account = 12 accounts):
/// - [0] TokenProgram (readonly)
/// - [1] UserAuthority (signer, readonly)
/// - [2] Whirlpool (writable)
/// - [3] TokenOwnerAccountA (writable)
/// - [4] TokenVaultA (writable)
/// - [5] TokenOwnerAccountB (writable)
/// - [6] TokenVaultB (writable)
/// - [7] TickArray0 (writable)
/// - [8] TickArray1 (writable)
/// - [9] TickArray2 (writable)
/// - [10] Oracle (readonly or writable if adaptive fee)
/// - [11] Orca Whirlpool Program (readonly)
pub fn invoke_orca_whirlpool_swap<'a>(
    accounts: &'a [AccountInfo<'a>],
    amount_in: u64,
    min_amount_out: u64,
    sqrt_price_limit: u128,
    a_to_b: bool,
) -> ProgramResult {
    if accounts.len() < 12 {
        return Err(ExecutorError::NotEnoughAccountInfos.into());
    }

    let program_info = &accounts[11];
    if *program_info.key != ORCA_WHIRLPOOL_PROGRAM_ID {
        return Err(ExecutorError::UnauthorizedProgramId.into());
    }

    let mut data = Vec::with_capacity(34);
    data.extend_from_slice(&ORCA_WHIRLPOOL_SWAP_DISCRIMINATOR);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&min_amount_out.to_le_bytes());
    data.extend_from_slice(&sqrt_price_limit.to_le_bytes());
    data.push(1); // amount_specified_is_input = true
    data.push(a_to_b as u8);

    let oracle_meta = if accounts[10].is_writable {
        AccountMeta::new(*accounts[10].key, false)
    } else {
        AccountMeta::new_readonly(*accounts[10].key, false)
    };

    let metas = vec![
        AccountMeta::new_readonly(*accounts[0].key, false),
        AccountMeta::new_readonly(*accounts[1].key, accounts[1].is_signer),
        AccountMeta::new(*accounts[2].key, false),
        AccountMeta::new(*accounts[3].key, false),
        AccountMeta::new(*accounts[4].key, false),
        AccountMeta::new(*accounts[5].key, false),
        AccountMeta::new(*accounts[6].key, false),
        AccountMeta::new(*accounts[7].key, false),
        AccountMeta::new(*accounts[8].key, false),
        AccountMeta::new(*accounts[9].key, false),
        oracle_meta,
    ];

    let instruction = Instruction {
        program_id: *program_info.key,
        accounts: metas,
        data,
    };

    invoke(&instruction, &accounts[..12])
}
