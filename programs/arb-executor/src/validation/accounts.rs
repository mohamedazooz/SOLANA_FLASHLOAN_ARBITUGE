//! Account and balance validation utilities for Arbitrage Executor.

use crate::error::ExecutorError;
use solana_account_info::AccountInfo;
use solana_address::Address;
use solana_program_error::ProgramError;

/// Known protocol constants.
pub const METEORA_DLMM_PROGRAM_ID: Address =
    Address::from_str_const("LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo");

pub const SPL_TOKEN_PROGRAM_ID: Address =
    Address::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");

pub const TOKEN_2022_PROGRAM_ID: Address =
    Address::from_str_const("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");

pub const MEMO_PROGRAM_ID: Address =
    Address::from_str_const("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");

pub const METEORA_EVENT_AUTHORITY: Address =
    Address::from_str_const("D1ZN9Wj1fRSUQfCjhvnu1hqDMT7hzjzBBpi12nVniYD6");

/// Read the current SPL Token or Token-2022 balance from an `AccountInfo`.
///
/// In both SPL Token and Token-2022, the base layout stores the `u64` balance
/// at bytes 64..72 as a little-endian unsigned integer.
pub fn read_token_account_balance(account: &AccountInfo) -> Result<u64, ProgramError> {
    let owner = account.owner;
    if *owner != SPL_TOKEN_PROGRAM_ID && *owner != TOKEN_2022_PROGRAM_ID {
        return Err(ExecutorError::InvalidTokenAccount.into());
    }

    let data = account.try_borrow_data()?;
    if data.len() < 72 {
        return Err(ExecutorError::InvalidTokenAccount.into());
    }

    let balance_bytes: [u8; 8] = data[64..72]
        .try_into()
        .map_err(|_| ExecutorError::InvalidTokenAccount)?;
    Ok(u64::from_le_bytes(balance_bytes))
}

/// Validate protocol programs, authority PDAs, and signer privileges.
pub fn validate_protocol_programs(
    dlmm_program: &AccountInfo,
    memo_program: &AccountInfo,
    event_authority: &AccountInfo,
    user: &AccountInfo,
) -> Result<(), ProgramError> {
    if *dlmm_program.key != METEORA_DLMM_PROGRAM_ID {
        return Err(ExecutorError::InvalidMeteoraProgramId.into());
    }

    if *memo_program.key != MEMO_PROGRAM_ID {
        return Err(ExecutorError::InvalidMemoProgramId.into());
    }

    if *event_authority.key != METEORA_EVENT_AUTHORITY {
        return Err(ExecutorError::InvalidEventAuthority.into());
    }

    if !user.is_signer {
        return Err(ExecutorError::MissingRequiredSigner.into());
    }

    Ok(())
}

/// Assert that the three pools in the arbitrage route are strictly distinct.
pub fn validate_distinct_pools(
    p1: &Address,
    p2: &Address,
    p3: &Address,
) -> Result<(), ProgramError> {
    if p1 == p2 || p2 == p3 || p1 == p3 {
        return Err(ExecutorError::PoolsNotDistinct.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_distinct_pools() {
        let p1 = Address::from_str_const("11111111111111111111111111111111");
        let p2 = Address::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
        let p3 = Address::from_str_const("LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo");

        assert!(validate_distinct_pools(&p1, &p2, &p3).is_ok());
        assert_eq!(
            validate_distinct_pools(&p1, &p1, &p3),
            Err(ExecutorError::PoolsNotDistinct.into())
        );
        assert_eq!(
            validate_distinct_pools(&p1, &p2, &p2),
            Err(ExecutorError::PoolsNotDistinct.into())
        );
        assert_eq!(
            validate_distinct_pools(&p1, &p2, &p1),
            Err(ExecutorError::PoolsNotDistinct.into())
        );
    }
}
