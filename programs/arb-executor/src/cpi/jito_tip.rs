//! Jito tip transfer CPI invocation.
//!
//! The tip is a plain System-Program transfer from the transaction signer to
//! a canonical Jito tip account, executed only after the three-leg arbitrage
//! body has fully validated. It is the last on-chain action of
//! `execute_three_leg_with_tip`, matching the Jito convention that the tip
//! must be the final instruction/transfer in a bundle's last transaction.

use crate::validation::{SYSTEM_PROGRAM_ID, validate_jito_tip_account};
use solana_account_info::AccountInfo;
use solana_cpi::invoke;
use solana_instruction::{AccountMeta, Instruction};
use solana_program_error::ProgramResult;

/// System Program `Transfer` enum variant tag (u32 LE) followed by u64 lamports.
pub const SYSTEM_TRANSFER_IX_TAG: u32 = 2;

/// Transfer `tip_lamports` from the signing user to a validated Jito tip
/// account via System Program CPI. Zero-lamport tips are a no-op.
pub fn invoke_jito_tip<'a>(
    user: &AccountInfo<'a>,
    tip_account: &AccountInfo<'a>,
    tip_lamports: u64,
) -> ProgramResult {
    if tip_lamports == 0 {
        return Ok(());
    }

    validate_jito_tip_account(tip_account)?;

    let data = {
        let mut d = Vec::with_capacity(12);
        d.extend_from_slice(&SYSTEM_TRANSFER_IX_TAG.to_le_bytes());
        d.extend_from_slice(&tip_lamports.to_le_bytes());
        d
    };

    let instruction = Instruction {
        program_id: SYSTEM_PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(*user.key, true),
            AccountMeta::new(*tip_account.key, false),
        ],
        data,
    };

    invoke(&instruction, &[user.clone(), tip_account.clone()])
}
