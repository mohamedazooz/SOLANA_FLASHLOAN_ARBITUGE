//! `execute_three_leg` instruction implementation.

use borsh::{BorshDeserialize, BorshSerialize};
use solana_account_info::AccountInfo;
use solana_program_error::ProgramResult;

use crate::cpi::{DlmmSwapAccounts, invoke_meteora_swap2};
use crate::error::ExecutorError;
use crate::validation::{
    TOKEN_2022_PROGRAM_ID, read_token_account_balance, validate_distinct_pools,
    validate_protocol_programs, validate_user_token_account,
};

/// Parameters for the `execute_three_leg` instruction.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct ExecuteThreeLegArgs {
    /// Exact amount in for Leg 1 (borrowed WSOL).
    pub leg1_amount_in: u64,
    /// Minimum acceptable output of Meme A from Leg 1.
    pub leg1_min_amount_out: u64,
    /// Exact amount in for Leg 2 (Meme A). Must satisfy leg1_min_amount_out >= leg2_amount_in.
    pub leg2_amount_in: u64,
    /// Minimum acceptable output of Meme B from Leg 2.
    pub leg2_min_amount_out: u64,
    /// Exact amount in for Leg 3 (Meme B). Must satisfy leg2_min_amount_out >= leg3_amount_in.
    pub leg3_amount_in: u64,
    /// Minimum acceptable output of WSOL from Leg 3.
    pub leg3_min_amount_out: u64,
    /// Minimum final WSOL net increase required across the 3 legs to satisfy repayment + net profit.
    pub min_final_wsol_delta: u64,
    /// Number of bin arrays passed for Leg 1.
    pub leg1_bin_array_count: u8,
    /// Number of bin arrays passed for Leg 2.
    pub leg2_bin_array_count: u8,
    /// Number of bin arrays passed for Leg 3.
    pub leg3_bin_array_count: u8,
}

/// Helper to select the token program matching the mint's owner.
fn resolve_token_program<'a>(
    mint: &'a AccountInfo<'a>,
    spl_token_prog: &'a AccountInfo<'a>,
    token_2022_prog: &'a AccountInfo<'a>,
) -> &'a AccountInfo<'a> {
    if *mint.owner == TOKEN_2022_PROGRAM_ID {
        token_2022_prog
    } else {
        spl_token_prog
    }
}

/// Execute the 3-leg DLMM CPI arbitrage route with on-chain sequential balance validation.
pub fn process_execute_three_leg<'a>(
    accounts: &'a [AccountInfo<'a>],
    args: ExecuteThreeLegArgs,
) -> ProgramResult {
    // 1. Validate invariant: previous hop guaranteed min output must cover next hop amount in
    if args.leg1_min_amount_out < args.leg2_amount_in {
        return Err(ExecutorError::Leg2InputExceedsLeg1Output.into());
    }
    if args.leg2_min_amount_out < args.leg3_amount_in {
        return Err(ExecutorError::Leg3InputExceedsLeg2Output.into());
    }
    if args.leg3_min_amount_out < args.min_final_wsol_delta {
        return Err(ExecutorError::FinalWsolDeltaBelowMinimum.into());
    }

    let c1 = args.leg1_bin_array_count as usize;
    let c2 = args.leg2_bin_array_count as usize;
    let c3 = args.leg3_bin_array_count as usize;

    let required_accounts = 33 + c1 + c2 + c3;
    if accounts.len() < required_accounts {
        return Err(ExecutorError::NotEnoughAccountInfos.into());
    }

    // Unpack Header Accounts
    let user = &accounts[0];
    let user_wsol_account = &accounts[1];
    let user_meme_a_account = &accounts[2];
    let user_meme_b_account = &accounts[3];
    let dlmm_program = &accounts[4];
    let token_program = &accounts[5];
    let token_2022_program = &accounts[6];
    let memo_program = &accounts[7];
    let event_authority = &accounts[8];

    validate_protocol_programs(dlmm_program, memo_program, event_authority, user)?;

    // Pool Accounts: Leg 1 (indices 9..17)
    let leg1_lb_pair = &accounts[9];
    let leg1_bitmap_ext = &accounts[10];
    let leg1_reserve_x = &accounts[11];
    let leg1_reserve_y = &accounts[12];
    let leg1_token_x_mint = &accounts[13];
    let leg1_token_y_mint = &accounts[14];
    let leg1_oracle = &accounts[15];
    let leg1_host_fee_in = &accounts[16];

    // Pool Accounts: Leg 2 (indices 17..25)
    let leg2_lb_pair = &accounts[17];
    let leg2_bitmap_ext = &accounts[18];
    let leg2_reserve_x = &accounts[19];
    let leg2_reserve_y = &accounts[20];
    let leg2_token_x_mint = &accounts[21];
    let leg2_token_y_mint = &accounts[22];
    let leg2_oracle = &accounts[23];
    let leg2_host_fee_in = &accounts[24];

    // Pool Accounts: Leg 3 (indices 25..33)
    let leg3_lb_pair = &accounts[25];
    let leg3_bitmap_ext = &accounts[26];
    let leg3_reserve_x = &accounts[27];
    let leg3_reserve_y = &accounts[28];
    let leg3_token_x_mint = &accounts[29];
    let leg3_token_y_mint = &accounts[30];
    let leg3_oracle = &accounts[31];
    let leg3_host_fee_in = &accounts[32];

    validate_distinct_pools(leg1_lb_pair.key, leg2_lb_pair.key, leg3_lb_pair.key)?;

    // Slice Bin Arrays directly from accounts slice
    let ba_offset1 = 33;
    let ba_offset2 = ba_offset1 + c1;
    let ba_offset3 = ba_offset2 + c2;
    let ba_offset_end = ba_offset3 + c3;

    let leg1_bin_arrays = &accounts[ba_offset1..ba_offset2];
    let leg2_bin_arrays = &accounts[ba_offset2..ba_offset3];
    let leg3_bin_arrays = &accounts[ba_offset3..ba_offset_end];

    // Baseline Token Balances & Ownership Validation
    let wsol_initial = validate_user_token_account(user_wsol_account, user.key, None)?;
    let meme_a_initial = validate_user_token_account(user_meme_a_account, user.key, None)?;
    let meme_b_initial = validate_user_token_account(user_meme_b_account, user.key, None)?;

    // Initial WSOL must cover Leg 1 input
    let wsol_baseline = wsol_initial
        .checked_sub(args.leg1_amount_in)
        .ok_or(ExecutorError::CalculationOverflow)?;

    // ==========================================
    // LEG 1: WSOL -> Meme A
    // ==========================================
    let leg1_token_x_program =
        resolve_token_program(leg1_token_x_mint, token_program, token_2022_program);
    let leg1_token_y_program =
        resolve_token_program(leg1_token_y_mint, token_program, token_2022_program);

    let leg1_swap_accounts = DlmmSwapAccounts {
        lb_pair: leg1_lb_pair,
        bin_array_bitmap_extension: leg1_bitmap_ext,
        reserve_x: leg1_reserve_x,
        reserve_y: leg1_reserve_y,
        user_token_in: user_wsol_account,
        user_token_out: user_meme_a_account,
        token_x_mint: leg1_token_x_mint,
        token_y_mint: leg1_token_y_mint,
        oracle: leg1_oracle,
        host_fee_in: leg1_host_fee_in,
        user,
        token_x_program: leg1_token_x_program,
        token_y_program: leg1_token_y_program,
        memo_program,
        event_authority,
        dlmm_program,
        bin_arrays: leg1_bin_arrays,
    };

    invoke_meteora_swap2(
        &leg1_swap_accounts,
        args.leg1_amount_in,
        args.leg1_min_amount_out,
    )?;

    // Validate Leg 1 Sequential Delta
    let meme_a_post1 = read_token_account_balance(user_meme_a_account)?;
    let delta_a1 = meme_a_post1
        .checked_sub(meme_a_initial)
        .ok_or(ExecutorError::CalculationOverflow)?;

    if delta_a1 < args.leg1_min_amount_out {
        return Err(ExecutorError::Leg1OutputBelowMinimum.into());
    }
    if delta_a1 < args.leg2_amount_in {
        return Err(ExecutorError::Leg2InputExceedsLeg1Output.into());
    }

    // ==========================================
    // LEG 2: Meme A -> Meme B
    // ==========================================
    let leg2_token_x_program =
        resolve_token_program(leg2_token_x_mint, token_program, token_2022_program);
    let leg2_token_y_program =
        resolve_token_program(leg2_token_y_mint, token_program, token_2022_program);

    let leg2_swap_accounts = DlmmSwapAccounts {
        lb_pair: leg2_lb_pair,
        bin_array_bitmap_extension: leg2_bitmap_ext,
        reserve_x: leg2_reserve_x,
        reserve_y: leg2_reserve_y,
        user_token_in: user_meme_a_account,
        user_token_out: user_meme_b_account,
        token_x_mint: leg2_token_x_mint,
        token_y_mint: leg2_token_y_mint,
        oracle: leg2_oracle,
        host_fee_in: leg2_host_fee_in,
        user,
        token_x_program: leg2_token_x_program,
        token_y_program: leg2_token_y_program,
        memo_program,
        event_authority,
        dlmm_program,
        bin_arrays: leg2_bin_arrays,
    };

    invoke_meteora_swap2(
        &leg2_swap_accounts,
        args.leg2_amount_in,
        args.leg2_min_amount_out,
    )?;

    // Validate Leg 2 Sequential Delta & Meme A Inventory Isolation
    let meme_b_post2 = read_token_account_balance(user_meme_b_account)?;
    let delta_b2 = meme_b_post2
        .checked_sub(meme_b_initial)
        .ok_or(ExecutorError::CalculationOverflow)?;

    if delta_b2 < args.leg2_min_amount_out {
        return Err(ExecutorError::Leg2OutputBelowMinimum.into());
    }
    if delta_b2 < args.leg3_amount_in {
        return Err(ExecutorError::Leg3InputExceedsLeg2Output.into());
    }

    let meme_a_post2 = read_token_account_balance(user_meme_a_account)?;
    if meme_a_post2 < meme_a_initial {
        return Err(ExecutorError::IntermediateInventorySubsidized.into());
    }

    // ==========================================
    // LEG 3: Meme B -> WSOL
    // ==========================================
    let leg3_token_x_program =
        resolve_token_program(leg3_token_x_mint, token_program, token_2022_program);
    let leg3_token_y_program =
        resolve_token_program(leg3_token_y_mint, token_program, token_2022_program);

    let leg3_swap_accounts = DlmmSwapAccounts {
        lb_pair: leg3_lb_pair,
        bin_array_bitmap_extension: leg3_bitmap_ext,
        reserve_x: leg3_reserve_x,
        reserve_y: leg3_reserve_y,
        user_token_in: user_meme_b_account,
        user_token_out: user_wsol_account,
        token_x_mint: leg3_token_x_mint,
        token_y_mint: leg3_token_y_mint,
        oracle: leg3_oracle,
        host_fee_in: leg3_host_fee_in,
        user,
        token_x_program: leg3_token_x_program,
        token_y_program: leg3_token_y_program,
        memo_program,
        event_authority,
        dlmm_program,
        bin_arrays: leg3_bin_arrays,
    };

    let wsol_pre_leg3 = read_token_account_balance(user_wsol_account)?;

    invoke_meteora_swap2(
        &leg3_swap_accounts,
        args.leg3_amount_in,
        args.leg3_min_amount_out,
    )?;

    // Validate Leg 3 WSOL Output Delta & Inventory Isolation
    let wsol_final = read_token_account_balance(user_wsol_account)?;
    let meme_b_final = read_token_account_balance(user_meme_b_account)?;

    verify_sequential_balance_transitions(
        wsol_initial,
        wsol_baseline,
        wsol_pre_leg3,
        wsol_final,
        meme_b_initial,
        meme_b_final,
        &args,
    )?;

    Ok(())
}

/// Validate Leg 3 WSOL output, ensure repayment threshold is reached, and prevent
/// subsidization from pre-existing WSOL or intermediate tokens.
pub fn verify_sequential_balance_transitions(
    _wsol_initial: u64,
    wsol_baseline: u64,
    wsol_pre_leg3: u64,
    wsol_final: u64,
    meme_b_initial: u64,
    meme_b_final: u64,
    args: &ExecuteThreeLegArgs,
) -> Result<(), ExecutorError> {
    // Actual WSOL produced by Leg 3
    let leg3_delta = wsol_final
        .checked_sub(wsol_pre_leg3)
        .ok_or(ExecutorError::CalculationOverflow)?;

    if leg3_delta < args.leg3_min_amount_out {
        return Err(ExecutorError::Leg3OutputBelowMinimum);
    }
    if leg3_delta < args.min_final_wsol_delta {
        return Err(ExecutorError::FinalWsolDeltaBelowMinimum);
    }

    // Intermediate token B inventory isolation
    if meme_b_final < meme_b_initial {
        return Err(ExecutorError::IntermediateInventorySubsidized);
    }

    // Pre-existing WSOL inventory isolation:
    // The final WSOL balance must cover the pre-borrow baseline plus the required repayment
    let required_final_wsol = wsol_baseline
        .checked_add(args.min_final_wsol_delta)
        .ok_or(ExecutorError::CalculationOverflow)?;
    if wsol_final < required_final_wsol {
        return Err(ExecutorError::FinalWsolDeltaBelowMinimum);
    }

    Ok(())
}
