//! Meteora DLMM Swap2 CPI invocation builder.

use crate::error::ExecutorError;
use crate::validation::METEORA_DLMM_PROGRAM_ID;
use solana_account_info::AccountInfo;
use solana_cpi::invoke;
use solana_instruction::{AccountMeta, Instruction};
use solana_program_error::ProgramResult;

/// Official Anchor discriminator for `swap2` = sha256("global:swap2")[..8].
pub const METEORA_SWAP2_DISCRIMINATOR: [u8; 8] = [65, 75, 63, 76, 235, 91, 91, 136];

/// Fixed account references for one Meteora DLMM swap2 leg.
pub struct DlmmSwapAccounts<'a> {
    pub lb_pair: &'a AccountInfo<'a>,
    pub bin_array_bitmap_extension: &'a AccountInfo<'a>,
    pub reserve_x: &'a AccountInfo<'a>,
    pub reserve_y: &'a AccountInfo<'a>,
    pub user_token_in: &'a AccountInfo<'a>,
    pub user_token_out: &'a AccountInfo<'a>,
    pub token_x_mint: &'a AccountInfo<'a>,
    pub token_y_mint: &'a AccountInfo<'a>,
    pub oracle: &'a AccountInfo<'a>,
    pub host_fee_in: &'a AccountInfo<'a>,
    pub user: &'a AccountInfo<'a>,
    pub token_x_program: &'a AccountInfo<'a>,
    pub token_y_program: &'a AccountInfo<'a>,
    pub memo_program: &'a AccountInfo<'a>,
    pub event_authority: &'a AccountInfo<'a>,
    pub dlmm_program: &'a AccountInfo<'a>,
    pub bin_arrays: &'a [AccountInfo<'a>],
}

/// Execute a Meteora DLMM `swap2` instruction via Cross-Program Invocation (CPI).
pub fn invoke_meteora_swap2<'a>(
    accounts: &DlmmSwapAccounts<'a>,
    amount_in: u64,
    min_amount_out: u64,
) -> ProgramResult {
    if accounts.bin_arrays.is_empty() {
        return Err(ExecutorError::InvalidBinArray.into());
    }

    // 1. Build instruction data (28 bytes)
    // - 8 bytes discriminator
    // - 8 bytes amount_in (LE)
    // - 8 bytes min_amount_out (LE)
    // - 4 bytes RemainingAccountsInfo slices length = 0 (empty slices for standard swaps)
    let mut data = Vec::with_capacity(28);
    data.extend_from_slice(&METEORA_SWAP2_DISCRIMINATOR);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&min_amount_out.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes()); // remaining_accounts_info.slices = vec![]

    // 2. Build AccountMetas matching the official 16 fixed accounts + dynamic bin arrays
    let mut metas = Vec::with_capacity(16 + accounts.bin_arrays.len());
    metas.push(AccountMeta::new(*accounts.lb_pair.key, false));

    // When the optional bitmap extension is unconfigured, the official builder encodes the
    // DLMM program ID sentinel as readonly. Passing an executable program account or an un-granted
    // account as writable causes an immediate runtime privilege rejection.
    let bitmap_ext_meta = if *accounts.bin_array_bitmap_extension.key == METEORA_DLMM_PROGRAM_ID
        || !accounts.bin_array_bitmap_extension.is_writable
    {
        AccountMeta::new_readonly(*accounts.bin_array_bitmap_extension.key, false)
    } else {
        AccountMeta::new(*accounts.bin_array_bitmap_extension.key, false)
    };
    metas.push(bitmap_ext_meta);

    metas.push(AccountMeta::new(*accounts.reserve_x.key, false));
    metas.push(AccountMeta::new(*accounts.reserve_y.key, false));
    metas.push(AccountMeta::new(*accounts.user_token_in.key, false));
    metas.push(AccountMeta::new(*accounts.user_token_out.key, false));
    metas.push(AccountMeta::new_readonly(*accounts.token_x_mint.key, false));
    metas.push(AccountMeta::new_readonly(*accounts.token_y_mint.key, false));
    metas.push(AccountMeta::new(*accounts.oracle.key, false));

    // When host fee is unconfigured, sentinel is encoded as readonly
    let host_fee_meta = if *accounts.host_fee_in.key == METEORA_DLMM_PROGRAM_ID
        || !accounts.host_fee_in.is_writable
    {
        AccountMeta::new_readonly(*accounts.host_fee_in.key, false)
    } else {
        AccountMeta::new(*accounts.host_fee_in.key, false)
    };
    metas.push(host_fee_meta);

    metas.push(AccountMeta::new_readonly(*accounts.user.key, true)); // user signer forwarded
    metas.push(AccountMeta::new_readonly(
        *accounts.token_x_program.key,
        false,
    ));
    metas.push(AccountMeta::new_readonly(
        *accounts.token_y_program.key,
        false,
    ));
    metas.push(AccountMeta::new_readonly(*accounts.memo_program.key, false));
    metas.push(AccountMeta::new_readonly(
        *accounts.event_authority.key,
        false,
    ));
    metas.push(AccountMeta::new_readonly(METEORA_DLMM_PROGRAM_ID, false));

    for ba in accounts.bin_arrays {
        metas.push(AccountMeta::new(*ba.key, false));
    }

    // 3. Assemble AccountInfos for CPI invocation
    let mut account_infos = Vec::with_capacity(16 + accounts.bin_arrays.len());
    account_infos.push((*accounts.lb_pair).clone());
    account_infos.push((*accounts.bin_array_bitmap_extension).clone());
    account_infos.push((*accounts.reserve_x).clone());
    account_infos.push((*accounts.reserve_y).clone());
    account_infos.push((*accounts.user_token_in).clone());
    account_infos.push((*accounts.user_token_out).clone());
    account_infos.push((*accounts.token_x_mint).clone());
    account_infos.push((*accounts.token_y_mint).clone());
    account_infos.push((*accounts.oracle).clone());
    account_infos.push((*accounts.host_fee_in).clone());
    account_infos.push((*accounts.user).clone());
    account_infos.push((*accounts.token_x_program).clone());
    account_infos.push((*accounts.token_y_program).clone());
    account_infos.push((*accounts.memo_program).clone());
    account_infos.push((*accounts.event_authority).clone());
    account_infos.push((*accounts.dlmm_program).clone());

    for ba in accounts.bin_arrays {
        account_infos.push(ba.clone());
    }

    let instruction = Instruction {
        program_id: *accounts.dlmm_program.key,
        accounts: metas,
        data,
    };

    invoke(&instruction, &account_infos)
}
