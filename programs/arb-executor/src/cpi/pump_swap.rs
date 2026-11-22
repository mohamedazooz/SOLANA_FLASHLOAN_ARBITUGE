//! PumpSwap / Pump.fun Swap CPI invocation builder.

use crate::error::ExecutorError;
use crate::validation::{PUMP_FUN_PROGRAM_ID, PUMP_SWAP_PROGRAM_ID};
use solana_account_info::AccountInfo;
use solana_cpi::invoke;
use solana_instruction::{AccountMeta, Instruction};
use solana_program_error::ProgramResult;

/// Pump.fun buy discriminator: `[102, 6, 61, 18, 1, 218, 235, 234]`
pub const PUMP_FUN_BUY_DISCRIMINATOR: [u8; 8] = [102, 6, 61, 18, 1, 218, 235, 234];
/// Pump.fun sell discriminator: `[51, 230, 133, 164, 1, 127, 131, 173]`
pub const PUMP_FUN_SELL_DISCRIMINATOR: [u8; 8] = [51, 230, 133, 164, 1, 127, 131, 173];
/// PumpSwap V2 swap discriminator: `[248, 198, 158, 145, 225, 117, 134, 201]`
pub const PUMP_SWAP_SWAP_DISCRIMINATOR: [u8; 8] = [248, 198, 158, 145, 225, 117, 134, 201];

/// Execute PumpSwap or Pump.fun swap via CPI.
///
/// Accounts provided to this call must end with the program account (PumpSwap or Pump.fun).
/// Accounts preceding the program account are mapped to `AccountMeta`s according to their
/// `is_writable` and `is_signer` flags and forwarded in the CPI instruction.
pub fn invoke_pump_swap<'a>(
    accounts: &'a [AccountInfo<'a>],
    amount_in: u64,
    min_amount_out: u64,
    is_buy: bool,
) -> ProgramResult {
    if accounts.len() < 2 {
        return Err(ExecutorError::NotEnoughAccountInfos.into());
    }

    let program_info = accounts
        .last()
        .ok_or(ExecutorError::NotEnoughAccountInfos)?;

    if *program_info.key != PUMP_SWAP_PROGRAM_ID && *program_info.key != PUMP_FUN_PROGRAM_ID {
        return Err(ExecutorError::UnauthorizedProgramId.into());
    }

    let mut data = Vec::with_capacity(24);
    if *program_info.key == PUMP_FUN_PROGRAM_ID {
        if is_buy {
            data.extend_from_slice(&PUMP_FUN_BUY_DISCRIMINATOR);
            data.extend_from_slice(&amount_in.to_le_bytes()); // amount
            data.extend_from_slice(&min_amount_out.to_le_bytes()); // max_sol_cost
        } else {
            data.extend_from_slice(&PUMP_FUN_SELL_DISCRIMINATOR);
            data.extend_from_slice(&amount_in.to_le_bytes()); // amount
            data.extend_from_slice(&min_amount_out.to_le_bytes()); // min_sol_output
        }
    } else {
        data.extend_from_slice(&PUMP_SWAP_SWAP_DISCRIMINATOR);
        data.extend_from_slice(&amount_in.to_le_bytes());
        data.extend_from_slice(&min_amount_out.to_le_bytes());
    }

    let metas: Vec<AccountMeta> = accounts[..accounts.len() - 1]
        .iter()
        .map(|acc| {
            if acc.is_writable {
                AccountMeta::new(*acc.key, acc.is_signer)
            } else {
                AccountMeta::new_readonly(*acc.key, acc.is_signer)
            }
        })
        .collect();

    let instruction = Instruction {
        program_id: *program_info.key,
        accounts: metas,
        data,
    };

    invoke(&instruction, accounts)
}
