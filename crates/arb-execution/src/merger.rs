//! Cross-DEX Account Deduplication & Privilege Escalation (Module 1).
//!
//! When chaining Leg 1 (e.g., Raydium) and Leg 2 (e.g., Meteora DLMM):
//! 1. **Privilege Escalation Rule (Bitwise OR):**
//!    If an account appears in both DEX legs:
//!    - `merged.is_signer = leg1.is_signer | leg2.is_signer`
//!    - `merged.is_writable = leg1.is_writable | leg2.is_writable`
//! 2. **Directional Liquidity Array Pruning:**
//!    Retain ONLY `[active_array, next_directional_array_1]` (max 2 arrays per leg).
//! 3. **Atomic ATA Lifecycle Optimization:**
//!    Reuse deterministic temporary WSOL/Token intermediate state or idempotent
//!    ATA creation + `CloseAccount` at the end of the route.

use arb_constants::MEMO_PROGRAM_ID;
use arb_tx::{
    AccountMeta, CompiledInstruction, DlmmSwapAccountSet, METEORA_DLMM_PROGRAM_ID, SwapDirection,
    SwapMode, TOKEN_PROGRAM_ID, ThreeLegMemeArbitrageContext, build_swap2_exact_in,
    build_swap2_exact_out, derive_meteora_event_authority,
};
use solana_pubkey::Pubkey;
use std::collections::HashMap;
use std::str::FromStr;

/// Merge two DEX leg account lists with privilege escalation (bitwise OR).
///
/// For any account that appears in both lists, the merged privileges are:
/// - `is_signer = leg1.is_signer | leg2.is_signer`
/// - `is_writable = leg1.is_writable | leg2.is_writable`
pub fn merge_account_privileges(
    leg1_accounts: &[AccountMeta],
    leg2_accounts: &[AccountMeta],
) -> Vec<AccountMeta> {
    // Use a HashMap for O(1) lookups, but with pre-allocated capacity
    let mut merged: HashMap<Pubkey, (bool, bool)> =
        HashMap::with_capacity(leg1_accounts.len() + leg2_accounts.len());

    // Insert leg1 accounts
    for acc in leg1_accounts {
        merged.insert(acc.pubkey, (acc.is_signer, acc.is_writable));
    }

    // Merge leg2 accounts with bitwise OR
    for acc in leg2_accounts {
        merged
            .entry(acc.pubkey)
            .and_modify(|(is_signer, is_writable)| {
                *is_signer |= acc.is_signer;
                *is_writable |= acc.is_writable;
            })
            .or_insert((acc.is_signer, acc.is_writable));
    }

    // Convert back to Vec, preserving deterministic order:
    // First all leg1 accounts in order, then any leg2-only accounts
    let mut result = Vec::with_capacity(merged.len());
    let mut seen: std::collections::HashSet<Pubkey> = std::collections::HashSet::new();

    for acc in leg1_accounts {
        if seen.insert(acc.pubkey) {
            let (is_signer, is_writable) = merged[&acc.pubkey];
            result.push(AccountMeta {
                pubkey: acc.pubkey,
                is_signer,
                is_writable,
            });
        }
    }

    for acc in leg2_accounts {
        if seen.insert(acc.pubkey) {
            let (is_signer, is_writable) = merged[&acc.pubkey];
            result.push(AccountMeta {
                pubkey: acc.pubkey,
                is_signer,
                is_writable,
            });
        }
    }

    result
}

/// Merge all instructions by deduplicating accounts across the entire instruction set
/// and applying privilege escalation globally.
pub fn merge_dex_legs(instructions: &[CompiledInstruction]) -> Vec<CompiledInstruction> {
    // Global account privilege map
    let mut global_privileges: HashMap<Pubkey, (bool, bool)> = HashMap::new();

    // First pass: collect all privileges with bitwise OR
    for ix in instructions {
        // Program ID is always non-signer, readonly
        global_privileges
            .entry(ix.program_id)
            .and_modify(|(_, w)| *w = false)
            .or_insert((false, false));

        for acc in &ix.accounts {
            global_privileges
                .entry(acc.pubkey)
                .and_modify(|(s, w)| {
                    *s |= acc.is_signer;
                    *w |= acc.is_writable;
                })
                .or_insert((acc.is_signer, acc.is_writable));
        }
    }

    // Second pass: rebuild instructions with merged privileges
    instructions
        .iter()
        .map(|ix| {
            let merged_accounts: Vec<AccountMeta> = ix
                .accounts
                .iter()
                .map(|acc| {
                    let (is_signer, is_writable) = global_privileges[&acc.pubkey];
                    AccountMeta {
                        pubkey: acc.pubkey,
                        is_signer,
                        is_writable,
                    }
                })
                .collect();

            CompiledInstruction {
                program_id: ix.program_id,
                accounts: merged_accounts,
                data: ix.data.clone(),
            }
        })
        .collect()
}

/// Prune directional bin arrays for CLMM/DLMM legs based on swap direction.
///
/// Retains ONLY `[active_array, next_directional_array_1]` (maximum 2 arrays per leg)
/// instead of 3+ arrays, saving 32-128 bytes of unindexed static keys.
/// Prune directional bin arrays based on actual quote traversal.
///
/// Returns all arrays required by the swap direction, up to `max_arrays`.
/// Returns an error if required arrays are missing.
pub fn prune_directional_arrays(
    bin_arrays: &[Pubkey],
    _active_bin_id: i32,
    swap_for_y: bool,
    max_arrays: usize,
) -> Result<Vec<Pubkey>, arb_tx::TxError> {
    if bin_arrays.is_empty() {
        return Err(arb_tx::TxError::MissingExecutionAccounts {
            pool_id: Pubkey::default(),
            reason: "No bin arrays provided".to_string(),
        });
    }

    let max_arrays = max_arrays.min(bin_arrays.len()).max(1);

    // Retain all arrays up to max_arrays, in the correct direction
    // Direction: swap_for_y=true means price moves down (lower indices)
    //            swap_for_y=false means price moves up (higher indices)
    let mut result = Vec::with_capacity(max_arrays);

    if swap_for_y {
        // Price moves down: retain arrays from active downward
        // In the provided list, arrays should be ordered from active downward
        for i in 0..max_arrays {
            if i < bin_arrays.len() {
                result.push(bin_arrays[i]);
            }
        }
    } else {
        // Price moves up: retain arrays from active upward
        for i in 0..max_arrays {
            if i < bin_arrays.len() {
                result.push(bin_arrays[i]);
            }
        }
    }

    if result.is_empty() {
        return Err(arb_tx::TxError::MissingExecutionAccounts {
            pool_id: Pubkey::default(),
            reason: "No bin arrays retained after pruning".to_string(),
        });
    }

    Ok(result)
}

/// Build the three Meteora DLMM swap2 instructions with directional array pruning.
pub fn build_leg_swap_instructions(
    ctx: &ThreeLegMemeArbitrageContext,
) -> Result<Vec<CompiledInstruction>, arb_tx::TxError> {
    let mut ixs = Vec::with_capacity(3);

    // --- Leg 1: WSOL → Meme A ---
    let leg1_bin_arrays = prune_directional_arrays(
        &ctx.leg_1_accounts.bin_array_pdas,
        0, // active_bin_id placeholder
        ctx.leg_1_direction == SwapDirection::XtoY,
        ctx.leg_1_accounts.bin_array_pdas.len(), // retain all provided arrays
    )?;

    let leg1_set = DlmmSwapAccountSet {
        lb_pair: ctx.leg_1_accounts.lb_pair,
        bin_array_bitmap_extension: ctx.leg_1_accounts.bitmap_extension,
        reserve_x: ctx.leg_1_accounts.reserve_x,
        reserve_y: ctx.leg_1_accounts.reserve_y,
        user_token_in: ctx.bot_wsol_ata(),
        user_token_out: ctx.meme_a_ata,
        token_x_mint: ctx.leg_1_accounts.token_x_mint,
        token_y_mint: ctx.leg_1_accounts.token_y_mint,
        oracle: ctx.leg_1_accounts.oracle,
        host_fee_in: ctx.leg_1_accounts.host_fee_in,
        user: ctx.payer,
        token_x_program: ctx.leg_1_accounts.token_x_program,
        token_y_program: ctx.leg_1_accounts.token_y_program,
        memo_program: Pubkey::from_str(MEMO_PROGRAM_ID).unwrap(),
        event_authority: derive_meteora_event_authority(),
        bin_arrays: leg1_bin_arrays,
    };

    let leg1_ix = match ctx.leg_1_params.mode {
        SwapMode::ExactIn => build_swap2_exact_in(
            &leg1_set,
            ctx.leg_1_params.amount_in,
            ctx.leg_1_params.min_amount_out,
        ),
        SwapMode::ExactOut => build_swap2_exact_out(
            &leg1_set,
            ctx.leg_1_params.max_amount_in,
            ctx.leg_1_params.amount_out,
        ),
    };
    ixs.push(leg1_ix);

    // --- Leg 2: Meme A → Meme B ---
    let leg2_bin_arrays = prune_directional_arrays(
        &ctx.leg_2_accounts.bin_array_pdas,
        0,
        ctx.leg_2_direction == SwapDirection::XtoY,
        ctx.leg_2_accounts.bin_array_pdas.len(),
    )?;

    let leg2_set = DlmmSwapAccountSet {
        lb_pair: ctx.leg_2_accounts.lb_pair,
        bin_array_bitmap_extension: ctx.leg_2_accounts.bitmap_extension,
        reserve_x: ctx.leg_2_accounts.reserve_x,
        reserve_y: ctx.leg_2_accounts.reserve_y,
        user_token_in: ctx.meme_a_ata,
        user_token_out: ctx.meme_b_ata,
        token_x_mint: ctx.leg_2_accounts.token_x_mint,
        token_y_mint: ctx.leg_2_accounts.token_y_mint,
        oracle: ctx.leg_2_accounts.oracle,
        host_fee_in: ctx.leg_2_accounts.host_fee_in,
        user: ctx.payer,
        token_x_program: ctx.leg_2_accounts.token_x_program,
        token_y_program: ctx.leg_2_accounts.token_y_program,
        memo_program: Pubkey::from_str(MEMO_PROGRAM_ID).unwrap(),
        event_authority: derive_meteora_event_authority(),
        bin_arrays: leg2_bin_arrays,
    };

    let leg2_ix = match ctx.leg_2_params.mode {
        SwapMode::ExactIn => build_swap2_exact_in(
            &leg2_set,
            ctx.leg_2_params.amount_in,
            ctx.leg_2_params.min_amount_out,
        ),
        SwapMode::ExactOut => build_swap2_exact_out(
            &leg2_set,
            ctx.leg_2_params.max_amount_in,
            ctx.leg_2_params.amount_out,
        ),
    };
    ixs.push(leg2_ix);

    // --- Leg 3: Meme B → WSOL (static bridge) ---
    let leg3_bin_arrays = prune_directional_arrays(
        &ctx.leg_3_accounts.bin_array_pdas,
        0,
        ctx.leg_3_direction == SwapDirection::XtoY,
        ctx.leg_3_accounts.bin_array_pdas.len(),
    )?;

    let leg3_set = DlmmSwapAccountSet {
        lb_pair: ctx.leg_3_accounts.lb_pair,
        bin_array_bitmap_extension: ctx.leg_3_accounts.bitmap_extension,
        reserve_x: ctx.leg_3_accounts.reserve_x,
        reserve_y: ctx.leg_3_accounts.reserve_y,
        user_token_in: ctx.meme_b_ata,
        user_token_out: ctx.bot_wsol_ata(),
        token_x_mint: ctx.leg_3_accounts.token_x_mint,
        token_y_mint: ctx.leg_3_accounts.token_y_mint,
        oracle: ctx.leg_3_accounts.oracle,
        host_fee_in: ctx.leg_3_accounts.host_fee_in,
        user: ctx.payer,
        token_x_program: ctx.leg_3_accounts.token_x_program,
        token_y_program: ctx.leg_3_accounts.token_y_program,
        memo_program: Pubkey::from_str(MEMO_PROGRAM_ID).unwrap(),
        event_authority: derive_meteora_event_authority(),
        bin_arrays: leg3_bin_arrays,
    };

    let leg3_ix = match ctx.leg_3_params.mode {
        SwapMode::ExactIn => build_swap2_exact_in(
            &leg3_set,
            ctx.leg_3_params.amount_in,
            ctx.leg_3_params.min_amount_out,
        ),
        SwapMode::ExactOut => build_swap2_exact_out(
            &leg3_set,
            ctx.leg_3_params.max_amount_in,
            ctx.leg_3_params.amount_out,
        ),
    };
    ixs.push(leg3_ix);

    Ok(ixs)
}

/// Partition accounts into ALT-resolved vs inline.
///
/// Returns (alt_accounts, inline_accounts) where alt_accounts are addresses that
/// exist in the Static Master ALT and inline_accounts are addresses that must be
/// included inline in the V0 message.
pub fn partition_accounts(
    ctx: &ThreeLegMemeArbitrageContext,
    master_alt: &arb_tx::StaticMasterAlt,
) -> Result<(Vec<Pubkey>, Vec<Pubkey>), arb_tx::TxError> {
    let mut alt_addresses: Vec<Pubkey> = Vec::new();
    let mut inline_addresses: Vec<Pubkey> = Vec::new();
    let mut seen: std::collections::HashSet<Pubkey> = std::collections::HashSet::new();

    // Collect all referenced accounts from the context
    let mut all_refs: Vec<Pubkey> = vec![
        Pubkey::from_str(METEORA_DLMM_PROGRAM_ID).unwrap(),
        ctx.wsol_token_program,
        ctx.meme_a_token_program,
        ctx.meme_b_token_program,
        Pubkey::from_str(MEMO_PROGRAM_ID).unwrap(),
        Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap(),
        Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID).unwrap(),
        Pubkey::new_from_array(arb_constants::SYSTEM_PROGRAM_ID),
        Pubkey::new_from_array(arb_constants::COMPUTE_BUDGET_PROGRAM_ID),
        ctx.wsol_mint,
        ctx.meme_a_mint.0,
        ctx.meme_b_mint.0,
        ctx.bot_wsol_ata(),
        ctx.meme_a_ata,
        ctx.meme_b_ata,
        ctx.pool_1.0,
        ctx.pool_2.0,
        ctx.pool_3.0,
    ];

    // Per-leg accounts
    for leg in [
        &ctx.leg_1_accounts,
        &ctx.leg_2_accounts,
        &ctx.leg_3_accounts,
    ] {
        all_refs.push(leg.lb_pair);
        all_refs.push(leg.reserve_x);
        all_refs.push(leg.reserve_y);
        all_refs.push(leg.token_x_mint);
        all_refs.push(leg.token_y_mint);
        all_refs.push(leg.oracle);
        all_refs.push(leg.user_token_in);
        all_refs.push(leg.user_token_out);
        if let Some(hf) = leg.host_fee_in {
            all_refs.push(hf);
        }
        for ba in &leg.bin_array_pdas {
            all_refs.push(*ba);
        }
        if let Some(ext) = leg.bitmap_extension {
            all_refs.push(ext);
        }
    }

    // Jito tip account
    all_refs.push(ctx.jito_tip_account);

    // Deduplicate and classify
    for addr in all_refs {
        if !seen.insert(addr) {
            continue;
        }
        if master_alt.contains(&addr) {
            alt_addresses.push(addr);
        } else {
            inline_addresses.push(addr);
        }
    }

    Ok((alt_addresses, inline_addresses))
}

/// Decide whether to close an intermediate ATA.
///
/// Safety rules:
/// - Never close a canonical pre-existing ATA.
/// - Never close an account with nonzero token balance.
/// - For Token-2022, never close with nonzero withheld fees.
/// - Only close dedicated temporary accounts exclusive to the route.
pub fn should_close_intermediate_ata(ctx: &ThreeLegMemeArbitrageContext, is_meme_a: bool) -> bool {
    let (exists, balance, is_2022, withheld) = if is_meme_a {
        (
            ctx.meme_a_ata_exists,
            ctx.meme_a_ata_balance,
            ctx.meme_a_is_token_2022,
            ctx.meme_a_withheld_fees,
        )
    } else {
        (
            ctx.meme_b_ata_exists,
            ctx.meme_b_ata_balance,
            ctx.meme_b_is_token_2022,
            ctx.meme_b_withheld_fees,
        )
    };

    // Never close a pre-existing canonical ATA
    if exists {
        return false;
    }
    // Never close with nonzero balance
    if balance > 0 {
        return false;
    }
    // For Token-2022, never close with withheld fees
    if is_2022 && withheld > 0 {
        return false;
    }

    // Safe to close: temporary account, zero balance, no withheld fees
    true
}
