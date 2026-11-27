//! Standalone on-chain Atomic Profit Guard for Arbitrage Executor (`arb-executor`).
//!
//! Enforces that the target settlement token account holds at least
//! `min_balance_after = borrowed_principal + flash_fee + min_net_profit`.
//!
//! Placed strictly before `Kamino FlashRepay`:
//! FlashBorrow -> Swap 1 -> ... -> Swap N -> ProfitGuard -> FlashRepay
//!
//! If `current_balance < min_balance_after` by even 1 lamport, this instruction
//! immediately returns `ExecutorError::InsufficientProfit` (or `FinalWsolDeltaBelowMinimum`),
//! aborting and reverting the entire transaction. This mathematically prevents
//! wallet balance bleed where pre-existing WSOL would be seized to subsidize
//! failing or unprofitable arbitrage routes.

use crate::error::ExecutorError;
use crate::validation::read_token_account_balance;
use solana_account_info::AccountInfo;
use solana_program_error::ProgramResult;

/// POD representation of `ProfitGuardArgs`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ProfitGuardArgs {
    /// Minimum required SPL/Token-2022 balance after all swaps.
    pub min_balance_after: u64,
}

/// Process the standalone on-chain profit guard instruction.
///
/// Accounts:
/// - `[0]` `token_account`: Settlement token account (readonly).
///
/// Instruction Data:
/// - `min_balance_after: u64` (8 bytes LE).
pub fn process_profit_guard<'a>(
    accounts: &'a [AccountInfo<'a>],
    instruction_data: &[u8],
) -> ProgramResult {
    if accounts.is_empty() {
        return Err(ExecutorError::NotEnoughAccountInfos.into());
    }
    if accounts.len() != 1 {
        return Err(ExecutorError::InvalidInstructionData.into());
    }

    if instruction_data.len() != 8 {
        return Err(ExecutorError::InvalidInstructionData.into());
    }

    let min_balance_after = u64::from_le_bytes(
        instruction_data[..8]
            .try_into()
            .map_err(|_| ExecutorError::InvalidInstructionData)?,
    );

    let token_account = &accounts[0];
    let current_balance = read_token_account_balance(token_account)?;

    if current_balance < min_balance_after {
        return Err(ExecutorError::InsufficientProfit.into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation::SPL_TOKEN_PROGRAM_ID;
    use solana_address::Address;
    use solana_program_error::ProgramError;

    fn create_mock_token_account<'a>(
        key: &'a Address,
        lamports: &'a mut u64,
        data: &'a mut [u8],
        owner: &'a Address,
        balance: u64,
    ) -> AccountInfo<'a> {
        data[64..72].copy_from_slice(&balance.to_le_bytes());
        data[108] = 1; // Token account must be initialized and not frozen.
        AccountInfo::new(key, false, false, lamports, data, owner, false)
    }

    #[test]
    fn test_profit_guard_exact_breakeven_success() {
        let key = Address::from([1u8; 32]);
        let mut lamports = 1_000_000;
        let mut data = vec![0u8; 165];
        let owner = SPL_TOKEN_PROGRAM_ID;
        let acc = create_mock_token_account(&key, &mut lamports, &mut data, &owner, 10_000_000);

        let args = 10_000_000u64.to_le_bytes();
        let accounts = [acc];
        assert!(process_profit_guard(&accounts, &args).is_ok());
    }

    #[test]
    fn test_profit_guard_surplus_success() {
        let key = Address::from([1u8; 32]);
        let mut lamports = 1_000_000;
        let mut data = vec![0u8; 165];
        let owner = SPL_TOKEN_PROGRAM_ID;
        let acc = create_mock_token_account(&key, &mut lamports, &mut data, &owner, 10_000_001);

        let args = 10_000_000u64.to_le_bytes();
        let accounts = [acc];
        assert!(process_profit_guard(&accounts, &args).is_ok());
    }

    #[test]
    fn test_profit_guard_one_lamport_shortfall_rejection() {
        let key = Address::from([1u8; 32]);
        let mut lamports = 1_000_000;
        let mut data = vec![0u8; 165];
        let owner = SPL_TOKEN_PROGRAM_ID;
        let acc = create_mock_token_account(&key, &mut lamports, &mut data, &owner, 9_999_999);

        let args = 10_000_000u64.to_le_bytes();
        let accounts = [acc];
        let res = process_profit_guard(&accounts, &args);
        assert_eq!(
            res,
            Err(ProgramError::Custom(
                ExecutorError::InsufficientProfit as u32
            ))
        );
    }

    #[test]
    fn test_profit_guard_zero_balance_rejection() {
        let key = Address::from([1u8; 32]);
        let mut lamports = 1_000_000;
        let mut data = vec![0u8; 165];
        let owner = SPL_TOKEN_PROGRAM_ID;
        let acc = create_mock_token_account(&key, &mut lamports, &mut data, &owner, 0);

        let args = 1u64.to_le_bytes();
        let accounts = [acc];
        let res = process_profit_guard(&accounts, &args);
        assert_eq!(
            res,
            Err(ProgramError::Custom(
                ExecutorError::InsufficientProfit as u32
            ))
        );
    }

    #[test]
    fn rejects_trailing_profit_guard_data() {
        let key = Address::from([1u8; 32]);
        let mut lamports = 1_000_000;
        let mut data = vec![0u8; 165];
        let owner = SPL_TOKEN_PROGRAM_ID;
        let acc = create_mock_token_account(&key, &mut lamports, &mut data, &owner, 10_000_000);
        let accounts = [acc];
        let mut args = 10_000_000u64.to_le_bytes().to_vec();
        args.push(0);
        assert_eq!(
            process_profit_guard(&accounts, &args),
            Err(ExecutorError::InvalidInstructionData.into())
        );
    }
}
