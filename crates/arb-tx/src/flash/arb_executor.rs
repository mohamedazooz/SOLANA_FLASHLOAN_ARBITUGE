//! Dedicated builder and assembler for the on-chain Arbitrage Executor (`arb-executor`).
//!
//! Assembles a single atomic Solana transaction executing:
//! ```text
//! [0] ComputeBudget::SetComputeUnitLimit
//! [1] ComputeBudget::SetComputeUnitPrice
//! [2] ATA CreateIdempotent  (settlement account, flash mint = WSOL)
//! [3] ATA CreateIdempotent  (intermediate account A, meme mint A)
//! [4] ATA CreateIdempotent  (intermediate account B, meme mint B)
//! [5] klend::flash_borrow_reserve_liquidity      (borrow_instruction_index = 5)
//! [6] arb_executor::execute_three_leg            (CPI 1, 2, 3 with balance delta assertions)
//! [7] klend::flash_repay_reserve_liquidity       (borrow_instruction_index = 5)
//! [8] SystemProgram::Transfer                    (Jito tip, optional)
//! ```

use crate::AccountMeta;
use crate::atomic_integrity::{
    ExecutionMode, TransactionFormat, verify_atomic_integrity_for_format,
};
use crate::flash::dlmm_3leg_flash_arb::{
    FlashArb3LegError, FlashArb3LegParams, FlashThreeLegArbitrageTx,
};
use crate::meteora_dlmm::{DlmmSwapAccountSet, MEMO_PROGRAM_ID_STR, OPTIONAL_ACCOUNT_SENTINEL};
use crate::{
    AddressLookupTable, CompiledInstruction, KeypairSigner, TxBuilder, build_compute_unit_limit,
    build_compute_unit_price, build_create_ata_idempotent_for_program, build_kamino_flash_borrow,
    build_kamino_flash_repay_with_index, get_associated_token_address_with_program_id,
};
use arb_types::Lamports;
use solana_pubkey::Pubkey;
use std::str::FromStr;

/// Arbitrage Executor Program ID on Solana.
pub const ARB_EXECUTOR_PROGRAM_ID_STR: &str = "ArbExec1111111111111111111111111111111111111";

/// Discriminator for `execute_three_leg` (sha256("arb_executor:execute_three_leg")[..8]).
pub const EXECUTE_THREE_LEG_DISCRIMINATOR: [u8; 8] = [173, 221, 98, 44, 189, 21, 102, 184];

/// Build the `arb_executor::execute_three_leg` instruction.
pub fn build_arb_executor_three_leg_instruction(
    payer: Pubkey,
    settlement_account: Pubkey,
    intermediate_a: Pubkey,
    intermediate_b: Pubkey,
    dlmm_program_id: Pubkey,
    token_program: Pubkey,
    token_2022_program: Pubkey,
    memo_program: Pubkey,
    event_authority: Pubkey,
    leg_1_accounts: &DlmmSwapAccountSet,
    leg_2_accounts: &DlmmSwapAccountSet,
    leg_3_accounts: &DlmmSwapAccountSet,
    leg1_amount_in: u64,
    leg1_min_amount_out: u64,
    leg2_amount_in: u64,
    leg2_min_amount_out: u64,
    leg3_amount_in: u64,
    leg3_min_amount_out: u64,
    min_final_wsol_delta: u64,
) -> CompiledInstruction {
    let executor_program =
        Pubkey::from_str(ARB_EXECUTOR_PROGRAM_ID_STR).expect("valid arb executor program id");

    let leg1_bin_count = leg_1_accounts.bin_arrays.len() as u8;
    let leg2_bin_count = leg_2_accounts.bin_arrays.len() as u8;
    let leg3_bin_count = leg_3_accounts.bin_arrays.len() as u8;

    // 1. Data layout:
    // [0..8]   Discriminator
    // [8..16]  leg1_amount_in: u64 LE
    // [16..24] leg1_min_amount_out: u64 LE
    // [24..32] leg2_amount_in: u64 LE
    // [32..40] leg2_min_amount_out: u64 LE
    // [40..48] leg3_amount_in: u64 LE
    // [48..56] leg3_min_amount_out: u64 LE
    // [56..64] min_final_wsol_delta: u64 LE
    // [64..65] leg1_bin_array_count: u8
    // [65..66] leg2_bin_array_count: u8
    // [66..67] leg3_bin_array_count: u8
    let mut data = Vec::with_capacity(67);
    data.extend_from_slice(&EXECUTE_THREE_LEG_DISCRIMINATOR);
    data.extend_from_slice(&leg1_amount_in.to_le_bytes());
    data.extend_from_slice(&leg1_min_amount_out.to_le_bytes());
    data.extend_from_slice(&leg2_amount_in.to_le_bytes());
    data.extend_from_slice(&leg2_min_amount_out.to_le_bytes());
    data.extend_from_slice(&leg3_amount_in.to_le_bytes());
    data.extend_from_slice(&leg3_min_amount_out.to_le_bytes());
    data.extend_from_slice(&min_final_wsol_delta.to_le_bytes());
    data.push(leg1_bin_count);
    data.push(leg2_bin_count);
    data.push(leg3_bin_count);

    // 2. Account Metas:
    // Header accounts (0..9)
    let mut accounts = Vec::with_capacity(
        33 + leg1_bin_count as usize + leg2_bin_count as usize + leg3_bin_count as usize,
    );
    accounts.push(AccountMeta::new_readonly(payer, true)); // user signer
    accounts.push(AccountMeta::new(settlement_account, false)); // user_wsol_account
    accounts.push(AccountMeta::new(intermediate_a, false)); // user_meme_a_account
    accounts.push(AccountMeta::new(intermediate_b, false)); // user_meme_b_account
    accounts.push(AccountMeta::new_readonly(dlmm_program_id, false));
    accounts.push(AccountMeta::new_readonly(token_program, false));
    accounts.push(AccountMeta::new_readonly(token_2022_program, false));
    accounts.push(AccountMeta::new_readonly(memo_program, false));
    accounts.push(AccountMeta::new_readonly(event_authority, false));

    // Helper to push 8 pool accounts
    let sentinel = Pubkey::from_str(OPTIONAL_ACCOUNT_SENTINEL).expect("valid sentinel");
    let push_pool_accounts = |accs: &mut Vec<AccountMeta>, set: &DlmmSwapAccountSet| {
        accs.push(AccountMeta::new(set.lb_pair, false));
        let ext_meta = match set.bin_array_bitmap_extension {
            Some(pda) => AccountMeta::new(pda, false),
            None => AccountMeta::new_readonly(sentinel, false),
        };
        accs.push(ext_meta);
        accs.push(AccountMeta::new(set.reserve_x, false));
        accs.push(AccountMeta::new(set.reserve_y, false));
        accs.push(AccountMeta::new_readonly(set.token_x_mint, false));
        accs.push(AccountMeta::new_readonly(set.token_y_mint, false));
        accs.push(AccountMeta::new(set.oracle, false));
        let host_meta = match set.host_fee_in {
            Some(h) => AccountMeta::new(h, false),
            None => AccountMeta::new_readonly(sentinel, false),
        };
        accs.push(host_meta);
    };

    // Leg 1 pool accounts (9..17)
    push_pool_accounts(&mut accounts, leg_1_accounts);
    // Leg 2 pool accounts (17..25)
    push_pool_accounts(&mut accounts, leg_2_accounts);
    // Leg 3 pool accounts (25..33)
    push_pool_accounts(&mut accounts, leg_3_accounts);

    // Bin arrays in order
    for ba in &leg_1_accounts.bin_arrays {
        accounts.push(AccountMeta::new(*ba, false));
    }
    for ba in &leg_2_accounts.bin_arrays {
        accounts.push(AccountMeta::new(*ba, false));
    }
    for ba in &leg_3_accounts.bin_arrays {
        accounts.push(AccountMeta::new(*ba, false));
    }

    CompiledInstruction {
        program_id: executor_program,
        accounts,
        data,
    }
}

/// Assemble the complete atomic Solana transaction using the dedicated `ArbExecutor` CPI program.
pub fn assemble_flash_funded_three_pool_cpi(
    params: &FlashArb3LegParams,
    signer: &KeypairSigner,
    lookup_tables: &[AddressLookupTable],
) -> Result<FlashThreeLegArbitrageTx, FlashArb3LegError> {
    if params.payer != signer.pubkey() {
        return Err(FlashArb3LegError::PayerSignerMismatch {
            payer: params.payer.to_string(),
            signer: signer.pubkey().to_string(),
        });
    }

    // ── Invariant 1: three distinct pools ──
    if params.leg_1.lb_pair == params.leg_2.lb_pair
        || params.leg_1.lb_pair == params.leg_3.lb_pair
        || params.leg_2.lb_pair == params.leg_3.lb_pair
    {
        return Err(FlashArb3LegError::PoolsNotDistinct);
    }
    // ── Invariant 2: non-zero principal ──
    if params.borrow_amount == 0 {
        return Err(FlashArb3LegError::ZeroBorrowAmount);
    }
    // ── Invariant 3: repayment covers principal + fee ──
    let repay_amount = params.repay_amount();
    if repay_amount < params.borrow_amount.saturating_add(params.flash_fee) {
        return Err(FlashArb3LegError::InsufficientRepayment {
            borrow: params.borrow_amount,
            repay: repay_amount,
            fee: params.flash_fee,
        });
    }

    // ── Invariant 4: canonical hop input chaining ──
    if params.leg_1.min_amount_out < params.leg_2.amount_in {
        return Err(FlashArb3LegError::LegOutputBelowRequired {
            leg: 1,
            min_out: params.leg_1.min_amount_out,
            required: params.leg_2.amount_in,
        });
    }
    if params.leg_2.min_amount_out < params.leg_3.amount_in {
        return Err(FlashArb3LegError::LegOutputBelowRequired {
            leg: 2,
            min_out: params.leg_2.min_amount_out,
            required: params.leg_3.amount_in,
        });
    }
    if params.leg_3.min_amount_out < repay_amount {
        return Err(FlashArb3LegError::LegOutputBelowRequired {
            leg: 3,
            min_out: params.leg_3.min_amount_out,
            required: repay_amount,
        });
    }

    let payer = params.payer;
    let flash_mint = params.leg_1.token_in();
    let flash_program = params.leg_1.token_in_program();
    let mint_a = params.leg_1.token_out();
    let mint_a_program = params.leg_1.token_out_program();
    let mint_b = params.leg_2.token_out();
    let mint_b_program = params.leg_2.token_out_program();

    let settlement_account =
        get_associated_token_address_with_program_id(&payer, &flash_mint, &flash_program);
    let intermediate_account_a =
        get_associated_token_address_with_program_id(&payer, &mint_a, &mint_a_program);
    let intermediate_account_b =
        get_associated_token_address_with_program_id(&payer, &mint_b, &mint_b_program);

    let mut kamino = params.kamino.clone();
    kamino.user_destination_liquidity = settlement_account;

    // ── Instruction layout ──
    let mut ixs: Vec<CompiledInstruction> = Vec::with_capacity(9);
    ixs.push(build_compute_unit_limit(params.cu_limit));
    ixs.push(build_compute_unit_price(params.priority_fee_micro_lamports));
    ixs.push(build_create_ata_idempotent_for_program(
        payer,
        payer,
        flash_mint,
        settlement_account,
        flash_program,
    ));
    ixs.push(build_create_ata_idempotent_for_program(
        payer,
        payer,
        mint_a,
        intermediate_account_a,
        mint_a_program,
    ));
    ixs.push(build_create_ata_idempotent_for_program(
        payer,
        payer,
        mint_b,
        intermediate_account_b,
        mint_b_program,
    ));

    // [5] Kamino FlashBorrow
    let borrow_idx = ixs.len();
    ixs.push(build_kamino_flash_borrow(
        &kamino,
        payer,
        params.borrow_amount,
    ));

    // [6] ArbExecutor execute_three_leg
    let leg1_accs = params.leg_1.account_set(&payer);
    let leg2_accs = params.leg_2.account_set(&payer);
    let leg3_accs = params.leg_3.account_set(&payer);

    let dlmm_program = Pubkey::from_str(crate::meteora_dlmm::DLMM_PROGRAM_ID_STR).unwrap();
    let token_program = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap();
    let token_2022_program = Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID).unwrap();
    let memo_program = Pubkey::from_str(MEMO_PROGRAM_ID_STR).unwrap();
    let event_authority = crate::derive_meteora_event_authority();

    ixs.push(build_arb_executor_three_leg_instruction(
        payer,
        settlement_account,
        intermediate_account_a,
        intermediate_account_b,
        dlmm_program,
        token_program,
        token_2022_program,
        memo_program,
        event_authority,
        &leg1_accs,
        &leg2_accs,
        &leg3_accs,
        params.leg_1.amount_in,
        params.leg_1.min_amount_out,
        params.leg_2.amount_in,
        params.leg_2.min_amount_out,
        params.leg_3.amount_in,
        params.leg_3.min_amount_out,
        repay_amount,
    ));

    // [7] Kamino FlashRepay
    let repay_idx = ixs.len();
    ixs.push(build_kamino_flash_repay_with_index(
        &kamino,
        payer,
        params.borrow_amount,
        borrow_idx as u8,
    ));

    // [8] Jito tip transfer (if configured)
    if params.jito_tip_lamports > 0 {
        ixs.push(crate::build_jito_tip_instruction(
            payer,
            params.jito_tip_account,
            Lamports(params.jito_tip_lamports),
        ));
    }

    // ── Atomicity integrity verification ──
    let format = if params.v1_transport_supported {
        TransactionFormat::V1
    } else {
        TransactionFormat::Legacy
    };

    let integrity = verify_atomic_integrity_for_format(
        ExecutionMode::SingleTransaction,
        &ixs,
        Some(borrow_idx),
        Some(repay_idx),
        params.borrow_amount,
        repay_amount,
        params.flash_fee,
        Some(format),
    )
    .map_err(|e| FlashArb3LegError::AtomicityFailed(e.to_string()))?;

    if !integrity.is_atomic {
        return Err(FlashArb3LegError::AtomicityFailed(
            integrity
                .rejection_reason
                .unwrap_or_else(|| "atomic integrity rejected".to_string()),
        ));
    }

    // ── Choose format: V0 (with ALT) or V1 (4096 bytes) ──
    let wire_bytes = if params.v1_transport_supported {
        let priority_fee_lamports = ((params.priority_fee_micro_lamports as u128)
            .saturating_mul(params.cu_limit as u128)
            .saturating_add(999_999))
            / 1_000_000;
        let config = crate::adaptive::V1ConfigParams {
            priority_fee_lamports: Some(priority_fee_lamports as u64),
            compute_unit_limit: Some(params.cu_limit),
            loaded_accounts_data_size_limit: Some(1_048_576),
            heap_size: Some(256 * 1024),
        };
        let accounts_count = crate::adaptive::count_unique_accounts(signer.pubkey(), &ixs);
        let v1_tx = crate::adaptive::AdaptiveTxCompiler::build_signed_v1_transaction(
            signer,
            &ixs,
            params.recent_blockhash,
            config,
            crate::adaptive::FormatSelectionReason::LargePayloadV1 {
                accounts_count,
                wire_size: 0,
            },
        )
        .map(|mut tx| {
            let wire_size = tx.wire_bytes.len();
            tx.selection_reason = crate::adaptive::FormatSelectionReason::LargePayloadV1 {
                accounts_count,
                wire_size,
            };
            tx
        })
        .map_err(|e| FlashArb3LegError::AltCompilation(e.to_string()))?;
        v1_tx.wire_bytes
    } else {
        let message = TxBuilder::compile_v0_message(
            signer.pubkey(),
            &ixs,
            params.recent_blockhash,
            lookup_tables,
        )
        .map_err(|e| FlashArb3LegError::AltCompilation(e.to_string()))?;
        let signature = signer.sign(&message);
        let mut wire = Vec::with_capacity(1 + 64 + message.len());
        wire.push(1);
        wire.extend_from_slice(&signature);
        wire.extend_from_slice(&message);
        wire
    };

    let tx_size = wire_bytes.len();
    let limit = if params.v1_transport_supported {
        crate::SOLANA_PACKET_V1_MTU
    } else {
        crate::SOLANA_PACKET_MTU_LIMIT
    };
    if tx_size > limit {
        return Err(FlashArb3LegError::TransactionTooLarge {
            size: tx_size,
            limit,
        });
    }

    Ok(FlashThreeLegArbitrageTx {
        wire_bytes,
        tx_size,
        instructions: ixs,
        borrow_instruction_index: borrow_idx,
        repay_instruction_index: repay_idx,
        borrow_amount: params.borrow_amount,
        flash_fee: params.flash_fee,
        repay_amount,
        min_leg_1_out: params.leg_1.min_amount_out,
        min_leg_2_out: params.leg_2.min_amount_out,
        min_leg_3_out: params.leg_3.min_amount_out,
        settlement_account,
        intermediate_account_a,
        intermediate_account_b,
        pool_1: params.leg_1.lb_pair,
        pool_2: params.leg_2.lb_pair,
        pool_3: params.leg_3.lb_pair,
    })
}
