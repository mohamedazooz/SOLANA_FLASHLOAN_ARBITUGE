//! PumpSwap V2 exact-input CPI invocation builder.
//!
//! The former generic `global:swap` instruction was not a PumpSwap trade
//! interface. Current PumpSwap uses `buy_exact_quote_in_v2` for quote-to-base
//! exact-input trades and `sell_v2` for base-to-quote exact-input trades. This
//! CPI deliberately supports only that fixed 17-account V2 interface. Pump.fun
//! bonding-curve instructions have a different account contract and are not
//! accepted here.

use crate::error::ExecutorError;
use crate::validation::{
    PUMP_FEES_FEE_CONFIG_ADDRESS, PUMP_SWAP_EVENT_AUTHORITY, PUMP_SWAP_PROGRAM_ID,
    SPL_TOKEN_PROGRAM_ID, TOKEN_2022_PROGRAM_ID, validate_user_token_account_for_mint,
};
use arb_constants::{
    PUMP_FEES_FEE_CONFIG_DISCRIMINATOR, PUMP_FEES_PROGRAM_ID,
    PUMP_SWAP_BUY_EXACT_QUOTE_IN_V2_DISCRIMINATOR, PUMP_SWAP_GLOBAL_CONFIG,
    PUMP_SWAP_GLOBAL_CONFIG_DISCRIMINATOR, PUMP_SWAP_POOL_DISCRIMINATOR,
    PUMP_SWAP_SELL_V2_DISCRIMINATOR, SYSTEM_PROGRAM_ID,
};
use solana_account_info::AccountInfo;
use solana_address::Address;
use solana_cpi::invoke;
use solana_instruction::{AccountMeta, Instruction};
use solana_program_error::ProgramResult;

/// PumpSwap V2's fixed account matrix, including the program account at index 16.
const PUMP_SWAP_V2_ACCOUNT_COUNT: usize = 17;

/// Build the exact-input PumpSwap V2 instruction data.
///
/// `is_buy` means quote-token input and base-token output; `false` means
/// base-token input and quote-token output. The argument order is identical
/// (`amount_in`, `min_amount_out`) for these two exact-input instructions.
fn build_pump_swap_v2_data(amount_in: u64, min_amount_out: u64, is_buy: bool) -> Vec<u8> {
    let discriminator = if is_buy {
        PUMP_SWAP_BUY_EXACT_QUOTE_IN_V2_DISCRIMINATOR
    } else {
        PUMP_SWAP_SELL_V2_DISCRIMINATOR
    };

    let mut data = Vec::with_capacity(24);
    data.extend_from_slice(&discriminator);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&min_amount_out.to_le_bytes());
    data
}

/// Execute PumpSwap V2 `buy_exact_quote_in_v2` or `sell_v2` via CPI.
///
/// Accounts must follow the official 17-account matrix:
/// `[pool, user, global_config, base_mint, quote_mint, user_base_ata,
/// user_quote_ata, pool_base_vault, pool_quote_vault, base_token_program,
/// quote_token_program, system_program, user_volume_accumulator, fee_config,
/// buyback_fee_recipient_quote_ata, event_authority, pump_amm_program]`.
pub fn invoke_pump_swap<'a>(
    accounts: &'a [AccountInfo<'a>],
    amount_in: u64,
    min_amount_out: u64,
    is_buy: bool,
) -> ProgramResult {
    if accounts.len() != PUMP_SWAP_V2_ACCOUNT_COUNT || amount_in == 0 || min_amount_out == 0 {
        return Err(ExecutorError::InvalidInstructionData.into());
    }

    let program_info = &accounts[16];
    if *program_info.key != PUMP_SWAP_PROGRAM_ID {
        return Err(ExecutorError::UnauthorizedProgramId.into());
    }

    // Pin the accounts whose identities are stable and published by Pump.
    if *accounts[2].key != Address::from_str_const(PUMP_SWAP_GLOBAL_CONFIG)
        || *accounts[11].key != Address::from(SYSTEM_PROGRAM_ID)
        || *accounts[0].owner != PUMP_SWAP_PROGRAM_ID
        || *accounts[2].owner != PUMP_SWAP_PROGRAM_ID
        || *accounts[15].key != PUMP_SWAP_EVENT_AUTHORITY
        || !program_info.executable
        || program_info.is_writable
        || program_info.is_signer
        || !accounts[11].executable
    {
        return Err(ExecutorError::UnauthorizedProgramId.into());
    }

    // Verify the global configuration account type, not only its well-known key.
    let global_config_data = accounts[2].try_borrow_data()?;
    if global_config_data.get(..PUMP_SWAP_GLOBAL_CONFIG_DISCRIMINATOR.len())
        != Some(PUMP_SWAP_GLOBAL_CONFIG_DISCRIMINATOR.as_slice())
    {
        return Err(ExecutorError::InvalidInstructionData.into());
    }
    drop(global_config_data);

    // A Pool account has the canonical Anchor discriminator; checking it here
    // prevents the route executor from treating an arbitrary account as a pool.
    let pool_data = accounts[0].try_borrow_data()?;
    if pool_data.len() < PUMP_SWAP_POOL_DISCRIMINATOR.len()
        || pool_data[..PUMP_SWAP_POOL_DISCRIMINATOR.len()] != PUMP_SWAP_POOL_DISCRIMINATOR
    {
        return Err(ExecutorError::InvalidInstructionData.into());
    }
    drop(pool_data);

    if !accounts[1].is_signer {
        return Err(ExecutorError::MissingRequiredSigner.into());
    }

    // The two mint accounts determine their canonical token programs. Do not
    // permit a caller to substitute a different or non-executable program at
    // either CPI slot.
    let valid_token_program =
        |program: &Address| *program == SPL_TOKEN_PROGRAM_ID || *program == TOKEN_2022_PROGRAM_ID;
    if accounts[9].key != accounts[3].owner
        || accounts[10].key != accounts[4].owner
        || !valid_token_program(accounts[9].key)
        || !valid_token_program(accounts[10].key)
        || !accounts[9].executable
        || !accounts[10].executable
    {
        return Err(ExecutorError::MismatchedTokenProgram.into());
    }

    validate_user_token_account_for_mint(&accounts[5], accounts[1].key, &accounts[3])?;
    validate_user_token_account_for_mint(&accounts[6], accounts[1].key, &accounts[4])?;

    // Match the official V2 signer/writable matrix exactly. Passing more
    // privilege than the callee needs can create unexpected writable aliases
    // and account-lock contention; missing privilege would only fail later in
    // the CPI. Only `user` is a signer.
    let expected_writable = [
        true, true, false, false, false, true, true, true, true, false, false, false, true, false,
        true, false,
    ];
    let expected_signer = [
        false, true, false, false, false, false, false, false, false, false, false, false, false,
        false, false, false,
    ];
    for index in 0..16 {
        if accounts[index].is_writable != expected_writable[index]
            || accounts[index].is_signer != expected_signer[index]
        {
            return Err(ExecutorError::InvalidInstructionData.into());
        }
    }

    let fee_program = Address::from_str_const(PUMP_FEES_PROGRAM_ID);
    if *accounts[13].key != PUMP_FEES_FEE_CONFIG_ADDRESS || *accounts[13].owner != fee_program {
        return Err(ExecutorError::UnauthorizedProgramId.into());
    }
    let fee_config_data = accounts[13].try_borrow_data()?;
    if fee_config_data.get(..PUMP_FEES_FEE_CONFIG_DISCRIMINATOR.len())
        != Some(PUMP_FEES_FEE_CONFIG_DISCRIMINATOR.as_slice())
    {
        return Err(ExecutorError::InvalidInstructionData.into());
    }
    drop(fee_config_data);

    // The buyback recipient is a quote-mint token account. The callee still
    // validates that its authority is in GlobalConfig.buyback_fee_recipients;
    // this local preflight rejects malformed, frozen, or wrong-mint accounts.
    if *accounts[14].owner != *accounts[10].key {
        return Err(ExecutorError::InvalidTokenAccount.into());
    }
    let buyback_data = accounts[14].try_borrow_data()?;
    if buyback_data.len() < 165
        || &buyback_data[0..32] != accounts[4].key.as_ref()
        || buyback_data[108] != 1
    {
        return Err(ExecutorError::InvalidTokenAccount.into());
    }
    drop(buyback_data);

    let data = build_pump_swap_v2_data(amount_in, min_amount_out, is_buy);
    let metas: Vec<AccountMeta> = accounts[..16]
        .iter()
        .map(|account| {
            if account.is_writable {
                AccountMeta::new(*account.key, account.is_signer)
            } else {
                AccountMeta::new_readonly(*account.key, account.is_signer)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buy_encodes_exact_quote_in_v2_with_little_endian_amounts() {
        let data = build_pump_swap_v2_data(123_456, 120_000, true);
        assert_eq!(
            &data[..8],
            PUMP_SWAP_BUY_EXACT_QUOTE_IN_V2_DISCRIMINATOR.as_slice()
        );
        assert_eq!(u64::from_le_bytes(data[8..16].try_into().unwrap()), 123_456);
        assert_eq!(
            u64::from_le_bytes(data[16..24].try_into().unwrap()),
            120_000
        );
        assert_eq!(data.len(), 24);
    }

    #[test]
    fn sell_encodes_sell_v2_with_little_endian_amounts() {
        let data = build_pump_swap_v2_data(123_456, 120_000, false);
        assert_eq!(&data[..8], PUMP_SWAP_SELL_V2_DISCRIMINATOR.as_slice());
        assert_eq!(u64::from_le_bytes(data[8..16].try_into().unwrap()), 123_456);
        assert_eq!(
            u64::from_le_bytes(data[16..24].try_into().unwrap()),
            120_000
        );
        assert_eq!(data.len(), 24);
    }
}
