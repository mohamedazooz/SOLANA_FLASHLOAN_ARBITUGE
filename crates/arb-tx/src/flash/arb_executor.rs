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
    validate_flash_funded_three_pool_params,
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
pub const ARB_EXECUTOR_PROGRAM_ID_STR: &str = arb_constants::ARB_EXECUTOR_PROGRAM_ID;

/// 1-byte instruction discriminator for `execute_swap`.
pub const DISCRIMINATOR_EXECUTE_SWAP: u8 = 0;
/// 1-byte instruction discriminator for `execute_route`.
pub const DISCRIMINATOR_EXECUTE_ROUTE: u8 = 1;
/// 1-byte instruction discriminator for `profit_guard`.
pub const DISCRIMINATOR_PROFIT_GUARD: u8 = 2;

/// 8-byte instruction discriminator for `profit_guard`.
pub const PROFIT_GUARD_DISCRIMINATOR: [u8; 8] = [2, 0, 0, 0, 0, 0, 0, 0];
/// 8-byte instruction discriminator for `execute_route`.
pub const EXECUTE_ROUTE_DISCRIMINATOR: [u8; 8] = [1, 0, 0, 0, 0, 0, 0, 0];

/// Discriminator for `execute_three_leg` (sha256("arb_executor:execute_three_leg")[..8]).
pub const EXECUTE_THREE_LEG_DISCRIMINATOR: [u8; 8] = [173, 221, 98, 44, 189, 21, 102, 184];

/// Discriminator for `execute_three_leg_with_tip`
/// (sha256("arb_executor:execute_three_leg_with_tip")[..8]).
pub const EXECUTE_THREE_LEG_WITH_TIP_DISCRIMINATOR: [u8; 8] = [124, 34, 60, 60, 78, 94, 133, 177];

/// Build the standalone `profit_guard` instruction for the inactive arb-executor program.
///
/// The current program ID is a vanity placeholder and has no verified Mainnet deployment.
/// Do not append this instruction to a production transaction until the executor is
/// deployed and its on-chain program identity and binary are verified.
/// When deployed, the instruction verifies `token_account >= min_balance_after`.
pub fn build_profit_guard_instruction(
    token_account: Pubkey,
    min_balance_after: u64,
) -> CompiledInstruction {
    let executor_program =
        Pubkey::from_str(ARB_EXECUTOR_PROGRAM_ID_STR).expect("valid arb executor program id");

    let mut data = Vec::with_capacity(9);
    data.push(DISCRIMINATOR_PROFIT_GUARD);
    data.extend_from_slice(&min_balance_after.to_le_bytes());

    CompiledInstruction {
        program_id: executor_program,
        accounts: vec![AccountMeta::new_readonly(token_account, false)],
        data,
    }
}

/// Parameters for a single hop within a multi-leg route instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteHopParams {
    pub protocol: u8,
    pub flags: u8,
    pub amount_in: u64,
    pub min_amount_out: u64,
    pub hop_accounts: Vec<AccountMeta>,
}

/// Build the `arb_executor::execute_route` instruction for arbitrary 2-leg and 3-leg paths.
pub fn build_arb_executor_route_instruction(
    payer: Pubkey,
    settlement_account: Pubkey,
    intermediate_accounts: &[Pubkey],
    hops: &[RouteHopParams],
    min_final_delta: u64,
) -> CompiledInstruction {
    let executor_program =
        Pubkey::from_str(ARB_EXECUTOR_PROGRAM_ID_STR).expect("valid arb executor program id");

    let hop_count = hops.len() as u8;
    let mut data = Vec::with_capacity(89);
    data.push(DISCRIMINATOR_EXECUTE_ROUTE);
    data.push(hop_count);
    data.extend_from_slice(&[0u8; 7]); // padding
    data.extend_from_slice(&min_final_delta.to_le_bytes());

    for i in 0..3 {
        if let Some(hop) = hops.get(i) {
            data.push(hop.protocol);
            data.push(hop.flags);
            data.push(hop.hop_accounts.len() as u8);
            data.extend_from_slice(&[0u8; 5]); // padding
            data.extend_from_slice(&hop.amount_in.to_le_bytes());
            data.extend_from_slice(&hop.min_amount_out.to_le_bytes());
        } else {
            data.extend_from_slice(&[0u8; 24]);
        }
    }

    let total_accounts =
        2 + intermediate_accounts.len() + hops.iter().map(|h| h.hop_accounts.len()).sum::<usize>();
    let mut metas = Vec::with_capacity(total_accounts);
    metas.push(AccountMeta::new(payer, true));
    metas.push(AccountMeta::new(settlement_account, false));
    for intermediate in intermediate_accounts {
        metas.push(AccountMeta::new(*intermediate, false));
    }
    for hop in hops {
        metas.extend(hop.hop_accounts.iter().cloned());
    }

    CompiledInstruction {
        program_id: executor_program,
        accounts: metas,
        data,
    }
}

/// Parameters for multi-leg flash-funded route CPI execution.
#[derive(Debug, Clone)]
pub struct FlashRouteCpiParams {
    pub payer: Pubkey,
    pub kamino: crate::KaminoFlashParams,
    pub borrow_amount: u64,
    pub flash_fee: u64,
    pub settlement_mint: Pubkey,
    pub settlement_program: Pubkey,
    pub intermediate_tokens: Vec<(Pubkey, Pubkey)>, // (mint, program)
    pub hops: Vec<RouteHopParams>,
    pub min_final_delta: u64,
    pub min_net_profit_lamports: u64,
    pub jito_tip_account: Pubkey,
    pub jito_tip_lamports: u64,
    pub cu_limit: u32,
    pub priority_fee_micro_lamports: u64,
}

/// Assembles a complete atomic multi-leg flash-funded route CPI transaction.
pub fn assemble_flash_funded_route_cpi(
    params: &FlashRouteCpiParams,
) -> Result<Vec<CompiledInstruction>, FlashArb3LegError> {
    if params.hops.len() < 2 || params.hops.len() > 3 {
        return Err(FlashArb3LegError::AtomicityFailed(
            "hop count must be 2 or 3".into(),
        ));
    }

    let mut ixs = Vec::new();
    let effective_cu = crate::calculate_dynamic_cu_limit(params.cu_limit);
    ixs.push(build_compute_unit_limit(effective_cu));
    ixs.push(build_compute_unit_price(params.priority_fee_micro_lamports));

    let settlement_ata = get_associated_token_address_with_program_id(
        &params.payer,
        &params.settlement_mint,
        &params.settlement_program,
    );

    // Ephemeral ATA for settlement token
    ixs.push(build_create_ata_idempotent_for_program(
        params.payer,
        params.payer,
        params.settlement_mint,
        settlement_ata,
        params.settlement_program,
    ));

    // Ephemeral ATAs for intermediate tokens
    let mut intermediate_atas = Vec::with_capacity(params.intermediate_tokens.len());
    for (mint, program) in &params.intermediate_tokens {
        let ata = get_associated_token_address_with_program_id(&params.payer, mint, program);
        intermediate_atas.push(ata);
        ixs.push(build_create_ata_idempotent_for_program(
            params.payer,
            params.payer,
            *mint,
            ata,
            *program,
        ));
    }

    // Kamino FlashBorrow
    let borrow_idx = ixs.len();
    let mut kamino = params.kamino.clone();
    if kamino.user_destination_liquidity == Pubkey::default()
        || kamino.user_destination_liquidity == params.settlement_mint
    {
        kamino.user_destination_liquidity = settlement_ata;
    }
    ixs.push(build_kamino_flash_borrow(
        &kamino,
        params.payer,
        params.borrow_amount,
    ));

    // Route instruction
    ixs.push(build_arb_executor_route_instruction(
        params.payer,
        settlement_ata,
        &intermediate_atas,
        &params.hops,
        params.min_final_delta,
    ));

    // Standalone on-chain profit guard assertion before FlashRepay
    let min_balance_after = params
        .borrow_amount
        .checked_add(params.flash_fee)
        .and_then(|f| f.checked_add(params.min_net_profit_lamports))
        .ok_or_else(|| FlashArb3LegError::AtomicityFailed("min_balance_after overflow".into()))?;
    ixs.push(build_profit_guard_instruction(
        settlement_ata,
        min_balance_after,
    ));

    // Kamino FlashRepay
    let repay_idx = ixs.len();
    ixs.push(build_kamino_flash_repay_with_index(
        &kamino,
        params.payer,
        params.borrow_amount,
        borrow_idx as u8,
    ));

    // Close intermediate ATAs
    for ((_, program), ata) in params
        .intermediate_tokens
        .iter()
        .zip(intermediate_atas.iter())
    {
        ixs.push(crate::build_close_token_account_for_program(
            *ata,
            params.payer,
            params.payer,
            *program,
        ));
    }

    // Jito tip
    if params.jito_tip_lamports > 0 {
        ixs.push(crate::build_jito_tip_instruction(
            params.payer,
            params.jito_tip_account,
            Lamports(params.jito_tip_lamports),
        ));
    }

    // Verify atomicity
    let repay_amount = params.borrow_amount.checked_add(params.flash_fee).ok_or(
        FlashArb3LegError::RepaymentOverflow {
            borrow: params.borrow_amount,
            fee: params.flash_fee,
        },
    )?;
    let integrity = verify_atomic_integrity_for_format(
        ExecutionMode::SingleTransaction,
        &ixs,
        Some(borrow_idx),
        Some(repay_idx),
        params.borrow_amount,
        repay_amount,
        params.flash_fee,
        Some(TransactionFormat::V1),
    )
    .map_err(|e| FlashArb3LegError::AtomicityFailed(e.to_string()))?;

    if !integrity.is_atomic {
        return Err(FlashArb3LegError::AtomicityFailed(
            integrity
                .rejection_reason
                .unwrap_or_else(|| "atomic verification failed".to_string()),
        ));
    }

    Ok(ixs)
}

/// Estimated deepest CPI chain generated by the executor layout above:
/// payer top-level -> `arb_executor::execute_three_leg` (frame 2) -> DEX program
/// swap (frame 3) -> SPL token transfer CPI (frame 4). Verified against the
/// Agave runtime budget (`arb_constants::MAX_INSTRUCTION_STACK_DEPTH = 5`,
/// SIMD-0268 = 9); any future nesting deeper than this must fail closed at
/// compile time instead of silently burning a doomed transaction.
pub const EXECUTOR_CPI_STACK_DEPTH_ESTIMATE: usize = 4;
const _: () = assert!(
    EXECUTOR_CPI_STACK_DEPTH_ESTIMATE < arb_constants::MAX_INSTRUCTION_STACK_DEPTH,
    "executor CPI chain exceeds the Agave MAX_INSTRUCTION_STACK_DEPTH budget"
);
const _: () = assert!(
    EXECUTOR_CPI_STACK_DEPTH_ESTIMATE < arb_constants::MAX_INSTRUCTION_STACK_DEPTH_SIMD_0268
);

/// Runtime re-check of the executor stack-depth budget against the Grover-stable
/// Agave default (5 frames). Kept as an explicit function so the guard is invoked
/// (and testable) on the real assembly path, not just at compile time.
pub fn validate_executor_cpi_stack_depth() -> Result<(), FlashArb3LegError> {
    if EXECUTOR_CPI_STACK_DEPTH_ESTIMATE >= arb_constants::MAX_INSTRUCTION_STACK_DEPTH {
        return Err(FlashArb3LegError::CpiStackDepthExceeded {
            depth: EXECUTOR_CPI_STACK_DEPTH_ESTIMATE,
            limit: arb_constants::MAX_INSTRUCTION_STACK_DEPTH,
        });
    }
    Ok(())
}

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

/// Build the `arb_executor::execute_three_leg_with_tip` instruction.
///
/// The base `execute_three_leg` wire format is untouched: this variant swaps
/// the discriminator, appends `tip_bps: u64` (LE) to the argument bytes, and
/// appends one extra writable `jito_tip_account` at the end of the account
/// list (index `33 + leg_1..leg_3 bin array counts`), exactly matching the
/// on-chain `execute_three_leg_with_tip` processor.
pub fn build_arb_executor_three_leg_with_tip_instruction(
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
    tip_bps: u64,
    jito_tip_account: Pubkey,
) -> CompiledInstruction {
    let mut instruction = build_arb_executor_three_leg_instruction(
        payer,
        settlement_account,
        intermediate_a,
        intermediate_b,
        dlmm_program_id,
        token_program,
        token_2022_program,
        memo_program,
        event_authority,
        leg_1_accounts,
        leg_2_accounts,
        leg_3_accounts,
        leg1_amount_in,
        leg1_min_amount_out,
        leg2_amount_in,
        leg2_min_amount_out,
        leg3_amount_in,
        leg3_min_amount_out,
        min_final_wsol_delta,
    );

    instruction.data[0..8].copy_from_slice(&EXECUTE_THREE_LEG_WITH_TIP_DISCRIMINATOR);
    instruction.data.extend_from_slice(&tip_bps.to_le_bytes());
    instruction
        .accounts
        .push(AccountMeta::new(jito_tip_account, false));
    instruction
}

/// Assemble strictly the instructions vector for the 3-leg flash-funded CPI route.
///
/// Order:
/// `[0]` SetComputeUnitLimit
/// `[1]` SetComputeUnitPrice
/// `[2..4]` Ephemeral ATA CreateIdempotent (settlement, meme A, meme B)
/// `[5]` Kamino FlashBorrow (top-level)
/// `[6]` ArbExecutor execute_three_leg (genuine CPI for Hop 1, 2, 3)
/// `[7]` Kamino FlashRepay (top-level, borrow_instruction_index = 5)
/// `[8]` Jito Tip (optional)
pub fn assemble_flash_funded_three_pool_cpi_instructions(
    params: &FlashArb3LegParams,
) -> Result<(Vec<CompiledInstruction>, usize, usize), FlashArb3LegError> {
    let repay_amount = validate_flash_funded_three_pool_params(params)?;
    // CPI has an additional stack-depth invariant beyond shared route checks.
    validate_executor_cpi_stack_depth()?;

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

    Ok((ixs, borrow_idx, repay_idx))
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

    let (ixs, borrow_idx, repay_idx) = assemble_flash_funded_three_pool_cpi_instructions(params)?;
    let repay_amount = params.repay_amount()?;
    let flash_mint = params.leg_1.token_in();
    let flash_program = params.leg_1.token_in_program();
    let mint_a = params.leg_1.token_out();
    let mint_a_program = params.leg_1.token_out_program();
    let mint_b = params.leg_2.token_out();
    let mint_b_program = params.leg_2.token_out_program();

    let settlement_account =
        get_associated_token_address_with_program_id(&params.payer, &flash_mint, &flash_program);
    let intermediate_account_a =
        get_associated_token_address_with_program_id(&params.payer, &mint_a, &mint_a_program);
    let intermediate_account_b =
        get_associated_token_address_with_program_id(&params.payer, &mint_b, &mint_b_program);

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
        // PIPE-10: canonical ceil conversion, fail-closed on overflow
        // (previously a silent saturating cast could mint a wrong total fee).
        let priority_fee_lamports = crate::adaptive::total_priority_fee_lamports(
            params.priority_fee_micro_lamports,
            u64::from(params.cu_limit),
        )
        .ok_or_else(|| {
            FlashArb3LegError::AltCompilation("V1 priority fee exceeds u64 lamports".to_string())
        })?;
        let config = crate::adaptive::V1ConfigParams {
            priority_fee_lamports: Some(priority_fee_lamports),
            compute_unit_limit: Some(params.cu_limit),
            loaded_accounts_data_size_limit: Some(
                crate::adaptive::V1_MAX_LOADED_ACCOUNTS_DATA_SIZE_LIMIT,
            ),
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

#[cfg(test)]
mod cpi_stack_depth_tests {
    use super::*;

    #[test]
    fn executor_stack_depth_budget_is_satisfied() {
        validate_executor_cpi_stack_depth()
            .expect("executor layout must fit the Agave 5-frame budget");
        assert!(EXECUTOR_CPI_STACK_DEPTH_ESTIMATE < arb_constants::MAX_INSTRUCTION_STACK_DEPTH);
        assert!(
            EXECUTOR_CPI_STACK_DEPTH_ESTIMATE
                < arb_constants::MAX_INSTRUCTION_STACK_DEPTH_SIMD_0268
        );
    }
}

#[cfg(test)]
mod wire_parity_tests {
    //! Cross-layer parity: the off-chain assembled instruction bytes must
    //! decode byte-exactly into the on-chain `arb-executor` argument structs.
    use super::*;
    use arb_executor::{ExecuteThreeLegArgs, ExecuteThreeLegWithTipArgs};
    use borsh::BorshDeserialize;

    fn key(id: u8, variant: u8) -> Pubkey {
        let mut bytes = [0u8; 32];
        bytes[0] = id;
        bytes[1] = variant;
        Pubkey::from(bytes)
    }

    fn fixture_leg(id: u8, bin_count: usize) -> DlmmSwapAccountSet {
        DlmmSwapAccountSet {
            lb_pair: key(id, 1),
            bin_array_bitmap_extension: None,
            reserve_x: key(id, 2),
            reserve_y: key(id, 3),
            user_token_in: key(id, 4),
            user_token_out: key(id, 5),
            token_x_mint: key(id, 6),
            token_y_mint: key(id, 7),
            oracle: key(id, 8),
            host_fee_in: None,
            user: key(9, id),
            token_x_program: key(id, 10),
            token_y_program: key(id, 11),
            memo_program: key(id, 12),
            event_authority: key(id, 13),
            bin_arrays: (0..bin_count).map(|i| key(id, 20 + i as u8)).collect(),
        }
    }

    fn fixture_accounts() -> Pubkey {
        key(42, 42)
    }

    #[test]
    fn base_builder_bytes_decode_into_on_chain_args() {
        let (l1, l2, l3) = (fixture_leg(1, 2), fixture_leg(2, 3), fixture_leg(3, 2));
        let ix = build_arb_executor_three_leg_instruction(
            fixture_accounts(),
            key(0, 1),
            key(0, 2),
            key(0, 3),
            key(0, 4),
            key(0, 5),
            key(0, 6),
            key(0, 7),
            key(0, 8),
            &l1,
            &l2,
            &l3,
            1_000_000_000,
            25_000_000,
            25_000_000,
            50_000_000_000,
            50_000_000_000,
            1_050_000_000,
            1_010_000_000,
        );

        assert_eq!(ix.data[0..8], EXECUTE_THREE_LEG_DISCRIMINATOR);
        let args = ExecuteThreeLegArgs::try_from_slice(&ix.data[8..])
            .expect("base variant args must decode on-chain");
        assert_eq!(args.leg1_amount_in, 1_000_000_000);
        assert_eq!(args.min_final_wsol_delta, 1_010_000_000);
        assert_eq!(
            args.leg1_bin_array_count, 2,
            "bin-array counts must round-trip through the wire"
        );
        assert_eq!(ix.accounts.len(), 33 + 2 + 3 + 2);
    }

    #[test]
    fn with_tip_builder_bytes_decode_into_on_chain_args() {
        let (l1, l2, l3) = (fixture_leg(1, 1), fixture_leg(2, 1), fixture_leg(3, 1));
        let tip_account = Pubkey::from_str("ADaUMid9yfUytqMBgopwjb2DTLSokTSzL1zt6iGPaS49")
            .expect("canonical jito tip account");
        let ix = build_arb_executor_three_leg_with_tip_instruction(
            fixture_accounts(),
            key(0, 1),
            key(0, 2),
            key(0, 3),
            key(0, 4),
            key(0, 5),
            key(0, 6),
            key(0, 7),
            key(0, 8),
            &l1,
            &l2,
            &l3,
            1_000_000_000,
            25_000_000,
            25_000_000,
            50_000_000_000,
            50_000_000_000,
            1_050_000_000,
            1_010_000_000,
            7_000,
            tip_account,
        );

        assert_eq!(ix.data[0..8], EXECUTE_THREE_LEG_WITH_TIP_DISCRIMINATOR);
        assert_eq!(ix.data.len(), 8 + 67, "7 u64 + 3 u8 + tip_bps u64 payload");
        let args = ExecuteThreeLegWithTipArgs::try_from_slice(&ix.data[8..])
            .expect("tip variant args must decode on-chain");
        assert_eq!(args.tip_bps, 7_000);
        assert_eq!(args.leg1_amount_in, 1_000_000_000);
        assert_eq!(args.leg2_bin_array_count, 1);

        // Tip account is the final, writable, non-signer account;
        // the body accounts are untouched relative to the base variant.
        let last = ix.accounts.last().expect("tip account present");
        assert_eq!(last.pubkey, tip_account);
        assert!(last.is_writable);
        assert!(!last.is_signer);
        assert_eq!(ix.accounts.len(), 33 + 1 + 1 + 1 + 1);
    }

    #[test]
    fn tip_derivative_boundary_sweep_invariants() {
        // Off-chain mirror of the on-chain sizing contract: any candidate
        // (edge, bps) the assembler could ever choose must produce a tip that
        // never exceeds the edge and never exceeds the retained floor needs.
        for &edge in &[0u64, 1, 999, 10_000, 1_000_000_000, u64::MAX] {
            for &bps in &[0u64, 1, 7_000, 8_500, 10_000] {
                let tip = tip_amount_for_bps(edge, bps).expect("valid domain always sizes");
                assert!(tip <= edge, "tip must never exceed the realized edge");
                let expected = (edge as u128 * bps as u128 / 10_000) as u64;
                assert_eq!(
                    tip, expected,
                    "floor semantics: tip = floor(edge * bps / 10_000) in checked u128"
                );
            }
        }
    }

    fn tip_amount_for_bps(edge: u64, bps: u64) -> Result<u64, arb_executor::ExecutorError> {
        arb_executor::tip_amount_for_bps(edge, bps)
    }
}
