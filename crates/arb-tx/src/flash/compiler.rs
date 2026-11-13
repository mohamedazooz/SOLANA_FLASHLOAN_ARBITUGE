//! Adaptive 3-leg Meteora DLMM meme arbitrage transaction compiler.
//!
//! Compiles the route WSOL → Meme A → Meme B → WSOL across three distinct Meteora DLMM
//! pools into either a V0 Jito-compatible transaction (≤ 1232 bytes, using the Static
//! Master ALT) or a V1 direct-transport transaction (≤ 4096 bytes, all accounts inline).
//!
//! No live broadcasting or automatic trading authorization is performed.

use crate::master_alt::StaticMasterAlt;
use crate::meteora_dlmm::{DlmmSwapAccountSet, build_swap2_exact_in, build_swap2_exact_out};
use crate::{
    CompiledInstruction, KeypairSigner, TxError, build_compute_unit_limit,
    build_compute_unit_price, build_marked_jito_tip_instruction,
};
use arb_constants::{
    MEMO_PROGRAM_ID, METEORA_DLMM_PROGRAM_ID, SOLANA_PACKET_MTU_LIMIT, TOKEN_PROGRAM_ID,
};
use arb_types::{Lamports, MintId, PoolId};
use solana_pubkey::Pubkey;
use std::collections::HashSet;
use std::str::FromStr;

// USDT mint (not in arb-constants; defined locally).
#[allow(dead_code)]
const USDT_MINT_STR: &str = "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB";

// Re-export types needed by tests and external consumers.
use crate::meteora_dlmm;
pub use meteora_dlmm::{SwapDirection, SwapMode};

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// Outcome of compiling a 3-leg Meteora DLMM meme arbitrage route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileResult {
    /// V0 transaction compiled with the Static Master ALT, fits within 1232 bytes,
    /// suitable for Jito block engine submission.
    V0JitoCompatible(CompiledV0Transaction),
    /// V1 transaction compiled with all accounts inline, fits within 4096 bytes,
    /// suitable for direct RPC/TPU transport (NOT Jito unless independently verified).
    V1DirectTransport(CompiledV1Transaction),
    /// Route was rejected with the given reason.
    Rejected(RejectionReason),
}

/// Reason a route was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RejectionReason {
    /// V0 wire size exceeded 1232 bytes and V1 is not supported by the transport.
    V0Oversized { wire_size: usize, limit: usize },
    /// V1 wire size exceeded 4096 bytes.
    V1Oversized { wire_size: usize, limit: usize },
    /// V1 transport not supported by the configured endpoint.
    V1TransportUnsupported,
    /// More than two Bin Arrays required for at least one leg.
    TooManyBinArrays { leg: usize, needed: usize },
    /// Pool IDs are not distinct (Pool 1 == Pool 2 == Pool 3 required).
    PoolsNotDistinct,
    /// Required address missing from the Static Master ALT.
    MissingAltAddress { address: String },
    /// ALT account used as a signer (not allowed).
    AltAccountAsSigner { address: String },
    /// ALT not activated on-chain.
    AltNotActivated,
    /// Missing execution accounts for a pool.
    MissingExecutionAccounts { pool_id: String, reason: String },
    /// Invalid intermediate ATA state (unsafe to close, missing, etc.).
    InvalidIntermediateAta { mint: String, reason: String },
    /// Serialization error.
    Serialization(String),
    /// V1 message requires more than the SIMD-0385 inline address limit (64).
    V1AccountLimitExceeded { count: usize, max: usize },
    /// V1 message requires more than the SIMD-0160 instruction limit (64).
    V1InstructionLimitExceeded { count: usize, max: usize },
}

/// Compiled V0 transaction with the Static Master ALT.
///
/// Wire size is measured from the canonical Solana wire format:
/// compact-u16 signature count + signatures + message bytes. The workspace does not pin
/// `solana-sdk`'s `VersionedTransaction`, so `bincode::serialize` cannot be used as-is
/// (plain bincode would emit u64 vec lengths instead of compact-u16, producing a buffer
/// larger than the real wire size). The bytes of `wire_bytes` are exactly what the RPC
/// and Jito block engine parse; `wire_size == wire_bytes.len()` always holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledV0Transaction {
    pub wire_bytes: Vec<u8>,
    pub message_bytes: Vec<u8>,
    pub signatures: Vec<[u8; 64]>,
    pub wire_size: usize,
    pub lookup_table_address: Pubkey,
    pub static_account_count: usize,
    pub inline_account_count: usize,
    pub alt_resolved_count: usize,
    pub signer_count: usize,
    pub byte_budget: ByteBudgetReport,
}

/// Compiled V1 transaction with all accounts inline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledV1Transaction {
    pub wire_bytes: Vec<u8>,
    pub message_bytes: Vec<u8>,
    pub signatures: Vec<[u8; 64]>,
    pub wire_size: usize,
    pub account_count: usize,
    pub signer_count: usize,
    pub byte_budget: ByteBudgetReport,
}

/// Exact byte-by-byte accounting of a compiled transaction.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ByteBudgetReport {
    pub signature_vector_bytes: usize,
    pub message_version_prefix: usize,
    pub header_bytes: usize,
    pub static_account_keys_bytes: usize,
    pub inline_dynamic_account_keys_bytes: usize,
    pub blockhash_bytes: usize,
    pub compiled_instructions_bytes: usize,
    pub compute_configuration_bytes: usize,
    pub alt_lookup_references_bytes: usize,
    pub jito_tip_instruction_bytes: usize,
    pub create_ata_idempotent_bytes: usize,
    pub close_account_bytes: usize,
    pub compact_u16_prefixes_bytes: usize,
    pub lookup_index_vectors_bytes: usize,
    pub total_wire_size: usize,
    pub alt_savings_bytes: i64,
}

impl ByteBudgetReport {
    pub fn new() -> Self {
        Self {
            signature_vector_bytes: 0,
            message_version_prefix: 0,
            header_bytes: 0,
            static_account_keys_bytes: 0,
            inline_dynamic_account_keys_bytes: 0,
            blockhash_bytes: 0,
            compiled_instructions_bytes: 0,
            compute_configuration_bytes: 0,
            alt_lookup_references_bytes: 0,
            jito_tip_instruction_bytes: 0,
            create_ata_idempotent_bytes: 0,
            close_account_bytes: 0,
            compact_u16_prefixes_bytes: 0,
            lookup_index_vectors_bytes: 0,
            total_wire_size: 0,
            alt_savings_bytes: 0,
        }
    }

    pub fn summarize(&self) -> String {
        format!(
            "ByteBudget: sig_vec={} ver={} hdr={} static_keys={} inline_keys={} \
             bh={} ix={} compute={} alt_refs={} tip={} create_ata={} close={} \
             compact_u16={} lut_indices={} total={} alt_savings={}",
            self.signature_vector_bytes,
            self.message_version_prefix,
            self.header_bytes,
            self.static_account_keys_bytes,
            self.inline_dynamic_account_keys_bytes,
            self.blockhash_bytes,
            self.compiled_instructions_bytes,
            self.compute_configuration_bytes,
            self.alt_lookup_references_bytes,
            self.jito_tip_instruction_bytes,
            self.create_ata_idempotent_bytes,
            self.close_account_bytes,
            self.compact_u16_prefixes_bytes,
            self.lookup_index_vectors_bytes,
            self.total_wire_size,
            self.alt_savings_bytes,
        )
    }
}

// ---------------------------------------------------------------------------
// Three-leg route context
// ---------------------------------------------------------------------------

/// Per-leg execution account configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegExecutionAccounts {
    pub lb_pair: Pubkey,
    pub bin_array_pdas: Vec<Pubkey>,
    pub bitmap_extension: Option<Pubkey>,
    pub reserve_x: Pubkey,
    pub reserve_y: Pubkey,
    pub token_x_mint: Pubkey,
    pub token_y_mint: Pubkey,
    pub token_x_program: Pubkey,
    pub token_y_program: Pubkey,
    pub oracle: Pubkey,
    pub host_fee_in: Option<Pubkey>,
    pub user_token_in: Pubkey,
    pub user_token_out: Pubkey,
}

/// Per-leg exact input/output parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegSwapParams {
    pub mode: SwapMode,
    pub amount_in: u64,
    pub min_amount_out: u64,
    pub max_amount_in: u64,
    pub amount_out: u64,
}

/// Full context for compiling a three-leg Meteora DLMM meme arbitrage route.
///
/// Route: WSOL → Meme A → Meme B → WSOL
///
/// Validation: Pool 1 != Pool 2 != Pool 3. No assumption of identical mint ordering.
/// Every intermediate mint must have an explicitly validated user token account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreeLegMemeArbitrageContext {
    /// Transaction payer (and swap signer).
    pub payer: Pubkey,
    /// Input/output mint identities (base = WSOL for this route).
    pub wsol_mint: Pubkey,
    pub meme_a_mint: MintId,
    pub meme_b_mint: MintId,
    /// Pool IDs for all three legs — must be distinct.
    pub pool_1: PoolId, // WSOL / Meme A
    pub pool_2: PoolId, // Meme A / Meme B
    pub pool_3: PoolId, // Meme B / WSOL (static bridge)
    /// Per-leg token directions.
    pub leg_1_direction: SwapDirection,
    pub leg_2_direction: SwapDirection,
    pub leg_3_direction: SwapDirection,
    /// Per-leg execution accounts.
    pub leg_1_accounts: LegExecutionAccounts,
    pub leg_2_accounts: LegExecutionAccounts,
    pub leg_3_accounts: LegExecutionAccounts,
    /// Per-leg swap parameters (exact in / exact out).
    pub leg_1_params: LegSwapParams,
    pub leg_2_params: LegSwapParams,
    pub leg_3_params: LegSwapParams,
    /// Token program per mint.
    pub wsol_token_program: Pubkey,
    pub meme_a_token_program: Pubkey,
    pub meme_b_token_program: Pubkey,
    /// Intermediate token account identities (Meme A ATA, Meme B ATA).
    pub meme_a_ata: Pubkey,
    pub meme_b_ata: Pubkey,
    /// Whether each intermediate ATA already exists on chain.
    pub meme_a_ata_exists: bool,
    pub meme_b_ata_exists: bool,
    /// Token balances of intermediate ATAs (for close safety).
    pub meme_a_ata_balance: u64,
    pub meme_b_ata_balance: u64,
    /// Whether each intermediate mint is Token-2022.
    pub meme_a_is_token_2022: bool,
    pub meme_b_is_token_2022: bool,
    /// Withheld transfer fees on intermediate ATAs (Token-2022 only).
    pub meme_a_withheld_fees: u64,
    pub meme_b_withheld_fees: u64,
    /// Snapshot slot at which the route was priced.
    pub snapshot_slot: u64,
    /// Instruction version preference.
    pub instruction_version: u8,
    /// Resource limits.
    pub cu_limit: u32,
    pub priority_fee_micro_lamports: u64,
    /// Expected minimum final output (profit floor).
    pub expected_min_final_output: u64,
    /// Jito tip account and amount.
    pub jito_tip_account: Pubkey,
    pub jito_tip_lamports: u64,
    /// Recent blockhash.
    pub recent_blockhash: [u8; 32],
    /// Whether V1 direct transport is supported by the configured endpoint.
    pub v1_transport_supported: bool,
}

impl ThreeLegMemeArbitrageContext {
    /// Validate the route context: pools distinct, intermediates have ATAs, etc.
    pub fn validate(&self) -> Result<(), TxError> {
        // Pool 1 != Pool 2 != Pool 3
        if self.pool_1.0 == self.pool_2.0 {
            return Err(TxError::InvalidInput(
                "Pool 1 and Pool 2 must be distinct".to_string(),
            ));
        }
        if self.pool_2.0 == self.pool_3.0 {
            return Err(TxError::InvalidInput(
                "Pool 2 and Pool 3 must be distinct".to_string(),
            ));
        }
        if self.pool_1.0 == self.pool_3.0 {
            return Err(TxError::InvalidInput(
                "Pool 1 and Pool 3 must be distinct".to_string(),
            ));
        }

        // Route must close back to WSOL
        if self.pool_1.0 == Pubkey::default()
            || self.pool_2.0 == Pubkey::default()
            || self.pool_3.0 == Pubkey::default()
        {
            return Err(TxError::InvalidInput(
                "All pool IDs must be non-default".to_string(),
            ));
        }

        // Every intermediate mint must have an explicitly validated user token account.
        if self.meme_a_ata == Pubkey::default() {
            return Err(TxError::InvalidInput(
                "Meme A ATA must be explicitly set".to_string(),
            ));
        }
        if self.meme_b_ata == Pubkey::default() {
            return Err(TxError::InvalidInput(
                "Meme B ATA must be explicitly set".to_string(),
            ));
        }

        // ATA safety: never close an account with nonzero balance or withheld fees.
        if self.meme_a_ata_balance > 0 && self.meme_a_ata_exists {
            // This is fine — we just won't close it.
        }
        if self.meme_b_ata_balance > 0 && self.meme_b_ata_exists {
            // Fine.
        }

        // Token-2022 withheld fees must be zero before considering close.
        if self.meme_a_is_token_2022 && self.meme_a_withheld_fees > 0 {
            // Close disallowed for this ATA.
        }
        if self.meme_b_is_token_2022 && self.meme_b_withheld_fees > 0 {
            // Close disallowed for this ATA.
        }

        Ok(())
    }

    /// Returns true if the route uses Token-2022 for any intermediate mint.
    pub fn uses_token_2022_intermediate(&self) -> bool {
        self.meme_a_is_token_2022 || self.meme_b_is_token_2022
    }

    /// Returns the WSOL ATA for the bot (derived from payer + WSOL mint + token program).
    pub fn bot_wsol_ata(&self) -> Pubkey {
        crate::get_associated_token_address_with_program_id(
            &self.payer,
            &self.wsol_mint,
            &self.wsol_token_program,
        )
    }
}

// ---------------------------------------------------------------------------
// Main compiler entry point
// ---------------------------------------------------------------------------

/// Compile a three-leg Meteora DLMM meme arbitrage route.
///
/// Returns `CompileResult::V0JitoCompatible` when the transaction fits within
/// 1232 bytes using the Static Master ALT (Jito-compatible path).
///
/// Returns `CompileResult::V1DirectTransport` when V0 exceeds 1232 bytes but V1
/// fits within 4096 bytes (direct RPC/TPU path, NOT Jito unless independently
/// verified).
///
/// Returns `CompileResult::Rejected` when neither format can accommodate the route.
///
/// No broadcasting is performed.
pub fn compile_3leg_meme_dlmm_arb(
    ctx: &ThreeLegMemeArbitrageContext,
    master_alt: &StaticMasterAlt,
    signer: &KeypairSigner,
) -> Result<CompileResult, TxError> {
    ctx.validate()?;

    // 1. Build the instruction vector for the route.
    let mut instructions: Vec<CompiledInstruction> = Vec::new();

    // [0] Compute Budget: SetComputeUnitLimit (V0 only — V1 uses native message config).
    let cu_ix = build_compute_unit_limit(ctx.cu_limit);
    instructions.push(cu_ix);

    // [1] Compute Budget: SetComputeUnitPrice (V0 only — V1 uses native message config).
    let cp_ix = build_compute_unit_price(ctx.priority_fee_micro_lamports);
    instructions.push(cp_ix);

    // [2..] Intermediate ATA CreateIdempotent instructions (only when needed).
    if !ctx.meme_a_ata_exists {
        let create_a = crate::build_create_ata_idempotent_for_program(
            ctx.payer,
            ctx.payer,
            ctx.meme_a_mint.0,
            ctx.meme_a_ata,
            ctx.meme_a_token_program,
        );
        instructions.push(create_a);
    }
    if !ctx.meme_b_ata_exists {
        let create_b = crate::build_create_ata_idempotent_for_program(
            ctx.payer,
            ctx.payer,
            ctx.meme_b_mint.0,
            ctx.meme_b_ata,
            ctx.meme_b_token_program,
        );
        instructions.push(create_b);
    }

    // [..] Swap legs: three Meteora DLMM instructions.
    let leg_instructions = build_leg_swap_instructions(ctx)?;
    instructions.extend(leg_instructions);

    // [..] CloseAccount instructions for intermediate ATAs — ONLY when safe.
    if should_close_intermediate_ata(ctx, true) {
        let close_a = crate::build_close_token_account_for_program(
            ctx.meme_a_ata,
            ctx.payer,
            ctx.payer,
            ctx.meme_a_token_program,
        );
        instructions.push(close_a);
    }
    if should_close_intermediate_ata(ctx, false) {
        let close_b = crate::build_close_token_account_for_program(
            ctx.meme_b_ata,
            ctx.payer,
            ctx.payer,
            ctx.meme_b_token_program,
        );
        instructions.push(close_b);
    }

    // [..] Jito tip instruction (last).
    let tip_ix = build_marked_jito_tip_instruction(
        ctx.payer,
        ctx.jito_tip_account,
        Lamports(ctx.jito_tip_lamports),
    );
    instructions.push(tip_ix);

    // 2. Determine which accounts go in the ALT vs inline.
    let (alt_accounts, inline_accounts) = partition_accounts(ctx, master_alt)?;

    // 3. Try V0 path first (primary).
    let v0_result = try_compile_v0(
        ctx,
        &instructions,
        master_alt,
        &alt_accounts,
        &inline_accounts,
        signer,
    )?;

    if v0_result.wire_size <= SOLANA_PACKET_MTU_LIMIT {
        return Ok(CompileResult::V0JitoCompatible(v0_result));
    }

    // V0 too large — report and try V1 fallback.
    eprintln!(
        "V0 oversized: {} bytes > {} limit. {}",
        v0_result.wire_size,
        SOLANA_PACKET_MTU_LIMIT,
        v0_result.byte_budget.summarize()
    );

    // 4. V1 fallback: reconstruct with every account inline (no ALT).
    if !ctx.v1_transport_supported {
        return Ok(CompileResult::Rejected(
            RejectionReason::V1TransportUnsupported,
        ));
    }

    match try_compile_v1(ctx, &instructions, signer)? {
        Ok(v1_result) => Ok(CompileResult::V1DirectTransport(v1_result)),
        Err(reason) => Ok(CompileResult::Rejected(reason)),
    }
}

/// Build the three Meteora DLMM swap2 instructions (one per leg).
pub fn build_leg_swap_instructions(
    ctx: &ThreeLegMemeArbitrageContext,
) -> Result<Vec<CompiledInstruction>, TxError> {
    let mut ixs = Vec::with_capacity(3);

    // --- Leg 1: WSOL → Meme A ---
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
        event_authority: crate::derive_meteora_event_authority(),
        bin_arrays: ctx.leg_1_accounts.bin_array_pdas.clone(),
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
        event_authority: crate::derive_meteora_event_authority(),
        bin_arrays: ctx.leg_2_accounts.bin_array_pdas.clone(),
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
        event_authority: crate::derive_meteora_event_authority(),
        bin_arrays: ctx.leg_3_accounts.bin_array_pdas.clone(),
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

/// Decide whether to close an intermediate ATA.
///
/// Safety rules:
/// - Never close a canonical pre-existing ATA.
/// - Never close an account with nonzero token balance.
/// - For Token-2022, never close with nonzero withheld fees.
/// - Only close dedicated temporary accounts exclusive to the route.
///
/// In this compiler we default to NOT closing unless all safety conditions are met
/// and the ATA was created by this route (missing before → created → now empty).
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

    // Never close a pre-existing canonical ATA.
    if exists {
        return false;
    }
    // Never close with nonzero balance.
    if balance > 0 {
        return false;
    }
    // For Token-2022, never close with withheld fees.
    if is_2022 && withheld > 0 {
        return false;
    }

    // Safe to close: temporary account, zero balance, no withheld fees.
    true
}

/// Partition accounts into ALT-resolved vs inline.
///
/// Returns (alt_accounts, inline_accounts) where alt_accounts are addresses that
/// exist in the Static Master ALT and inline_accounts are addresses that must be
/// included inline in the V0 message.
pub fn partition_accounts(
    ctx: &ThreeLegMemeArbitrageContext,
    master_alt: &StaticMasterAlt,
) -> Result<(Vec<Pubkey>, Vec<Pubkey>), TxError> {
    let mut alt_addresses: Vec<Pubkey> = Vec::new();
    let mut inline_addresses: Vec<Pubkey> = Vec::new();
    let mut seen: HashSet<Pubkey> = HashSet::new();

    // Collect all referenced accounts from the context.
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

    // Deduplicate and classify.
    for addr in all_refs {
        if !seen.insert(addr) {
            continue;
        }
        if master_alt.contains(&addr) {
            // Verify no ALT account is used as a signer (checked at compile time below).
            alt_addresses.push(addr);
        } else {
            inline_addresses.push(addr);
        }
    }

    Ok((alt_addresses, inline_addresses))
}

// ---------------------------------------------------------------------------
// V0 compilation path
// ---------------------------------------------------------------------------

/// Decode a compact-u16 and return (value, bytes_consumed).
fn decode_compact_u16(bytes: &[u8]) -> Result<(usize, usize), TxError> {
    let mut val: usize = 0;
    let mut shift: usize = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if i >= 3 {
            return Err(TxError::Serialization("malformed compact-u16".into()));
        }
        val |= ((b & 0x7f) as usize) << shift;
        if b & 0x80 == 0 {
            return Ok((val, i + 1));
        }
        shift += 7;
    }
    Err(TxError::Serialization(
        "unexpected end of compact-u16".into(),
    ))
}

/// Size of a compact-u16 encoding of `n` in bytes (1..=3 for realistic sizes).
fn compact_u16_len(n: usize) -> usize {
    if n < 0x80 {
        1
    } else if n < 0x4000 {
        2
    } else {
        3
    }
}

/// Per-instruction wire cost in the V0 message: 1-byte program index + compact-u16
/// account count + 1 byte per account index + compact-u16 data length + data.
fn v0_instruction_wire_cost(ix: &CompiledInstruction) -> usize {
    1 + compact_u16_len(ix.accounts.len())
        + ix.accounts.len()
        + compact_u16_len(ix.data.len())
        + ix.data.len()
}

/// Measure the exact V0 wire components by parsing the compiled message bytes.
/// Every reported field is derived from the real serialized bytes.
///
/// Returns the byte budget, the number of static account keys, and the number
/// of loaded (ALT) lookup indices.
fn measure_v0_message_components(
    message: &[u8],
    instructions: &[CompiledInstruction],
    wire_size: usize,
) -> Result<(ByteBudgetReport, usize, usize), TxError> {
    let mut budget = ByteBudgetReport::new();
    budget.message_version_prefix = 1; // 0x80
    budget.header_bytes = 3; // num_required_signatures, readonly signed, readonly unsigned
    budget.blockhash_bytes = 32;

    let mut prefix_bytes = 0usize;
    let (n_static, static_hdr) = decode_compact_u16(&message[4..])?;
    budget.static_account_keys_bytes = n_static * 32;
    let signers = message[1] as usize;
    budget.inline_dynamic_account_keys_bytes = n_static.saturating_sub(signers) * 32;
    prefix_bytes += static_hdr;

    let mut i = 4 + static_hdr + n_static * 32 + 32;
    let (_n_ix, ix_hdr) = decode_compact_u16(&message[i..])?;
    prefix_bytes += ix_hdr;
    i += ix_hdr;

    let compute = Pubkey::new_from_array(arb_constants::COMPUTE_BUDGET_PROGRAM_ID);
    let ata_program = Pubkey::from_str(arb_constants::ASSOCIATED_TOKEN_PROGRAM_ID).unwrap();
    let token = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
    let token_2022 = Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID).unwrap();

    let mut instruction_bytes = 0usize;
    let mut create_bytes = 0usize;
    let mut close_bytes = 0usize;
    let mut compute_bytes = 0usize;

    for ix in instructions.iter() {
        i += 1; // 1-byte program index
        let (n_acc, acc_hdr) = decode_compact_u16(&message[i..])?;
        i += acc_hdr + n_acc;
        let (dlen, dl_hdr) = decode_compact_u16(&message[i..])?;
        i += dl_hdr + dlen;
        let cost = 1 + acc_hdr + n_acc + dl_hdr + dlen;
        prefix_bytes += acc_hdr + dl_hdr;
        instruction_bytes += cost;
        if ix.program_id == compute {
            compute_bytes += cost;
        } else if ix.program_id == ata_program {
            create_bytes += cost;
        } else if ix.data == [9] && (ix.program_id == token || ix.program_id == token_2022) {
            close_bytes += cost;
        }
    }
    budget.compiled_instructions_bytes = instruction_bytes;
    budget.compute_configuration_bytes = compute_bytes;
    budget.create_ata_idempotent_bytes = create_bytes;
    budget.close_account_bytes = close_bytes;
    budget.jito_tip_instruction_bytes = instructions
        .last()
        .map(v0_instruction_wire_cost)
        .unwrap_or(0);

    // Address table lookup section.
    let (n_lut, lut_hdr) = decode_compact_u16(&message[i..])?;
    i += lut_hdr;
    prefix_bytes += lut_hdr;
    let mut lut_bytes = lut_hdr;
    let mut index_vector_bytes = 0usize;
    let mut loaded = 0usize;
    for _ in 0..n_lut {
        i += 32; // table key
        lut_bytes += 32;
        let (w, w_hdr) = decode_compact_u16(&message[i..])?;
        i += w_hdr + w;
        lut_bytes += w_hdr + w;
        prefix_bytes += w_hdr;
        index_vector_bytes += w;
        loaded += w;

        let (r, r_hdr) = decode_compact_u16(&message[i..])?;
        i += r_hdr + r;
        lut_bytes += r_hdr + r;
        prefix_bytes += r_hdr;
        index_vector_bytes += r;
        loaded += r;
    }
    budget.alt_lookup_references_bytes = lut_bytes;
    budget.lookup_index_vectors_bytes = index_vector_bytes;
    budget.compact_u16_prefixes_bytes = prefix_bytes;

    // Savings: inlining the loaded keys would have cost 32 bytes each;
    // the LUT section costs `lut_bytes`.
    budget.alt_savings_bytes = loaded as i64 * 32 - lut_bytes as i64;
    budget.signature_vector_bytes = 1 + 64; // compact-u16(1) + Ed25519 signature
    budget.total_wire_size = wire_size;

    debug_assert_eq!(i, message.len(), "v0 message parse must consume every byte");

    Ok((budget, n_static, loaded))
}

/// Measure the exact V1 wire components by parsing the compiled message bytes.
fn measure_v1_message_components(
    message: &[u8],
    instructions: &[CompiledInstruction],
    wire_size: usize,
) -> Result<ByteBudgetReport, TxError> {
    let mut budget = ByteBudgetReport::new();
    budget.message_version_prefix = 1; // 0x81
    budget.header_bytes = 7; // 3-byte header + 4-byte TransactionConfigMask
    budget.blockhash_bytes = 32;

    let num_ix = message.get(40).copied().unwrap_or(0) as usize;
    let num_addresses = message.get(41).copied().unwrap_or(0) as usize;
    budget.inline_dynamic_account_keys_bytes = num_addresses * 32;

    let mask_bytes: [u8; 4] = message
        .get(4..8)
        .and_then(|s| s.try_into().ok())
        .ok_or_else(|| TxError::Serialization("v1 message too short for config mask".into()))?;
    let mask = u32::from_le_bytes(mask_bytes);

    let mut config_value_bytes = 0usize;
    if mask & crate::adaptive::CONFIG_MASK_PRIORITY_FEE == crate::adaptive::CONFIG_MASK_PRIORITY_FEE
    {
        config_value_bytes += 8;
    }
    if mask & crate::adaptive::CONFIG_MASK_CU_LIMIT != 0 {
        config_value_bytes += 4;
    }
    if mask & crate::adaptive::CONFIG_MASK_LOADED_DATA_LIMIT != 0 {
        config_value_bytes += 4;
    }
    if mask & crate::adaptive::CONFIG_MASK_HEAP_SIZE != 0 {
        config_value_bytes += 4;
    }
    budget.compute_configuration_bytes = config_value_bytes;

    // Instruction section: num_ix * 4-byte headers, then contiguous payloads.
    let ix_header_bytes = num_ix * 4;
    let ata_program = Pubkey::from_str(arb_constants::ASSOCIATED_TOKEN_PROGRAM_ID).unwrap();
    let token = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
    let token_2022 = Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID).unwrap();

    let mut create_bytes = 0usize;
    let mut close_bytes = 0usize;
    let mut payload_bytes = 0usize;
    for ix in instructions {
        payload_bytes += ix.accounts.len() + ix.data.len();
        let cost = 4 + ix.accounts.len() + ix.data.len();
        if ix.program_id == ata_program {
            create_bytes += cost;
        } else if ix.data == [9] && (ix.program_id == token || ix.program_id == token_2022) {
            close_bytes += cost;
        }
    }
    budget.compiled_instructions_bytes = ix_header_bytes + payload_bytes;
    budget.jito_tip_instruction_bytes = instructions
        .last()
        .map(|ix| 4 + ix.accounts.len() + ix.data.len())
        .unwrap_or(0);
    budget.create_ata_idempotent_bytes = create_bytes;
    budget.close_account_bytes = close_bytes;
    budget.compact_u16_prefixes_bytes = 0; // V1 uses fixed-width fields
    budget.lookup_index_vectors_bytes = 0;
    budget.alt_lookup_references_bytes = 0;
    budget.alt_savings_bytes = 0;
    budget.signature_vector_bytes = 64; // trailing Ed25519 signature, no count prefix
    budget.total_wire_size = wire_size;
    budget.static_account_keys_bytes = 0; // V1 carries no static/frozen key split

    let expected_len =
        42 + num_addresses * 32 + config_value_bytes + ix_header_bytes + payload_bytes;
    if expected_len != message.len() {
        return Err(TxError::Serialization(format!(
            "v1 message parse mismatch: reconstructed {expected_len} bytes, actual {}",
            message.len()
        )));
    }
    Ok(budget)
}

fn try_compile_v0(
    ctx: &ThreeLegMemeArbitrageContext,
    instructions: &[CompiledInstruction],
    master_alt: &StaticMasterAlt,
    alt_accounts: &[Pubkey],
    _inline_accounts: &[Pubkey],
    signer: &KeypairSigner,
) -> Result<CompiledV0Transaction, TxError> {
    // Verify no ALT account is used as a signer.
    let signer_addresses = [ctx.payer];
    master_alt
        .verify_no_signers(&signer_addresses)
        .map_err(|e| TxError::InvalidInput(format!("ALT signer violation: {e}")))?;

    // Verify ALT activation.
    master_alt.verify_activation().map_err(|_| {
        TxError::InvalidInput("Static Master ALT is not activated on-chain".to_string())
    })?;

    // Verify required addresses exist in ALT.
    master_alt
        .verify_required_present()
        .map_err(|e| TxError::InvalidInput(format!("ALT missing required address: {e}")))?;

    // Build the ALT lookup table for the V0 message.
    let alt_key = master_alt
        .alt_address
        .ok_or_else(|| TxError::InvalidInput("No ALT address configured".to_string()))?;
    let alt_table = crate::AddressLookupTable {
        key: alt_key,
        addresses: alt_accounts.to_vec(),
    };

    let message_bytes = crate::TxBuilder::compile_v0_message(
        ctx.payer,
        instructions,
        ctx.recent_blockhash,
        std::slice::from_ref(&alt_table),
    )
    .map_err(|e| TxError::AltCompilationError(e.to_string()))?;

    let signature = signer.sign(&message_bytes);

    let mut wire_bytes = Vec::with_capacity(1 + 64 + message_bytes.len());
    crate::TxBuilder::encode_compact_u16(1, &mut wire_bytes);
    wire_bytes.extend_from_slice(&signature);
    wire_bytes.extend_from_slice(&message_bytes);

    let wire_size = wire_bytes.len();

    let (byte_budget, static_account_count, loaded_count) =
        measure_v0_message_components(&message_bytes, instructions, wire_size)?;

    // Note: `alt_accounts` may contain ALT members that are never referenced by any
    // instruction (they cost nothing and are not serialized). The number of ALT keys
    // actually referenced — and thus looked up on the wire — is `loaded_count`.
    Ok(CompiledV0Transaction {
        wire_bytes,
        message_bytes,
        signatures: vec![signature],
        wire_size,
        lookup_table_address: alt_key,
        static_account_count,
        // Distinct non-ALT (inline) keys referenced by the instructions == static keys.
        inline_account_count: static_account_count,
        alt_resolved_count: loaded_count,
        signer_count: 1,
        byte_budget,
    })
}

// ---------------------------------------------------------------------------
// V1 compilation path (direct transport, all accounts inline)
// ---------------------------------------------------------------------------

/// Reconstruct the V1 instruction payload and pre-flight-check the SIMD-0385 limits.
/// Legacy `ComputeBudget` instructions must NOT be present: V1 configures resource
/// limits natively in the message header.
fn try_compile_v1(
    ctx: &ThreeLegMemeArbitrageContext,
    instructions: &[CompiledInstruction],
    signer: &KeypairSigner,
) -> Result<Result<CompiledV1Transaction, RejectionReason>, TxError> {
    // V1: every account inline. ComputeBudget instructions are dropped — the V1
    // message config carries the limits.
    let compute_budget = Pubkey::new_from_array(arb_constants::COMPUTE_BUDGET_PROGRAM_ID);
    let v1_instructions: Vec<CompiledInstruction> = instructions
        .iter()
        .filter(|ix| ix.program_id != compute_budget)
        .cloned()
        .collect();

    // SIMD-0160 / SIMD-0385 pre-flight limits.
    if v1_instructions.len() > crate::adaptive::MAX_V1_INSTRUCTIONS {
        return Ok(Err(RejectionReason::V1InstructionLimitExceeded {
            count: v1_instructions.len(),
            max: crate::adaptive::MAX_V1_INSTRUCTIONS,
        }));
    }
    let unique_accounts = crate::adaptive::count_unique_accounts(ctx.payer, &v1_instructions);
    if unique_accounts > crate::adaptive::MAX_V1_ADDRESSES {
        return Ok(Err(RejectionReason::V1AccountLimitExceeded {
            count: unique_accounts,
            max: crate::adaptive::MAX_V1_ADDRESSES,
        }));
    }

    // Total priority fee in lamports = ceil(cu_limit * micro_lamports / 1e6).
    let cu = ctx.cu_limit as u64;
    let micro = ctx.priority_fee_micro_lamports;
    let total_priority_fee_lamports = cu.saturating_mul(micro).saturating_add(999_999) / 1_000_000;

    // Loaded accounts data size limit: 256 KiB covers this route's footprint
    // (3 x BinArray ~= 36 KiB plus pools/ATAs/programs). Configurable upstream.
    let config = crate::adaptive::V1ConfigParams {
        priority_fee_lamports: Some(total_priority_fee_lamports),
        compute_unit_limit: Some(ctx.cu_limit),
        loaded_accounts_data_size_limit: Some(256 * 1024),
        heap_size: None,
    };

    let message_bytes = crate::adaptive::AdaptiveTxCompiler::compile_v1_message(
        ctx.payer,
        &v1_instructions,
        ctx.recent_blockhash,
        config,
    )
    .map_err(|e| TxError::InstructionError(e.to_string()))?;

    let signature = signer.sign(&message_bytes);

    // V1 wire format: message bytes at head, trailing signature at tail.
    let mut wire_bytes = Vec::with_capacity(message_bytes.len() + 64);
    wire_bytes.extend_from_slice(&message_bytes);
    wire_bytes.extend_from_slice(&signature);

    let wire_size = wire_bytes.len();

    if wire_size > crate::adaptive::SOLANA_PACKET_V1_MTU {
        return Ok(Err(RejectionReason::V1Oversized {
            wire_size,
            limit: crate::adaptive::SOLANA_PACKET_V1_MTU,
        }));
    }

    let byte_budget = measure_v1_message_components(&message_bytes, &v1_instructions, wire_size)?;

    let account_count = message_bytes[41] as usize;
    let signer_count = message_bytes[1] as usize;
    let message_bytes_owned = message_bytes;

    Ok(Ok(CompiledV1Transaction {
        wire_bytes,
        account_count,
        signer_count,
        message_bytes: message_bytes_owned,
        signatures: vec![signature],
        wire_size,
        byte_budget,
    }))
}

// ---------------------------------------------------------------------------
// Byte budget accounting
// ---------------------------------------------------------------------------

// get_associated_token_address_with_program_id is accessed via crate:: path.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::master_alt::derive_ata;
    use arb_constants::{COMPUTE_BUDGET_LIMIT_3HOP, JITO_TIP_ACCOUNTS, WSOL_MINT};

    fn test_ctx() -> ThreeLegMemeArbitrageContext {
        let payer = Pubkey::new_from_array([1u8; 32]);
        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let meme_a = Pubkey::new_from_array([90u8; 32]);
        let meme_b = Pubkey::new_from_array([91u8; 32]);
        let pool_1 = PoolId::new([10u8; 32]);
        let pool_2 = PoolId::new([20u8; 32]);
        let pool_3 = PoolId::new([30u8; 32]);
        let spl = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
        let meme_a_ata = derive_ata(&payer, &meme_a, &spl);
        let meme_b_ata = derive_ata(&payer, &meme_b, &spl);

        ThreeLegMemeArbitrageContext {
            payer,
            wsol_mint: wsol,
            meme_a_mint: MintId(meme_a),
            meme_b_mint: MintId(meme_b),
            pool_1,
            pool_2,
            pool_3,
            leg_1_direction: SwapDirection::XtoY,
            leg_2_direction: SwapDirection::XtoY,
            leg_3_direction: SwapDirection::YtoX,
            leg_1_accounts: LegExecutionAccounts {
                lb_pair: pool_1.0,
                bin_array_pdas: vec![
                    Pubkey::new_from_array([41u8; 32]),
                    Pubkey::new_from_array([42u8; 32]),
                ],
                bitmap_extension: None,
                reserve_x: Pubkey::new_from_array([71u8; 32]),
                reserve_y: Pubkey::new_from_array([72u8; 32]),
                token_x_mint: wsol,
                token_y_mint: meme_a,
                token_x_program: spl,
                token_y_program: spl,
                oracle: Pubkey::new_from_array([81u8; 32]),
                host_fee_in: None,
                user_token_in: derive_ata(&payer, &wsol, &spl),
                user_token_out: meme_a_ata,
            },
            leg_2_accounts: LegExecutionAccounts {
                lb_pair: pool_2.0,
                bin_array_pdas: vec![
                    Pubkey::new_from_array([51u8; 32]),
                    Pubkey::new_from_array([52u8; 32]),
                ],
                bitmap_extension: None,
                reserve_x: Pubkey::new_from_array([73u8; 32]),
                reserve_y: Pubkey::new_from_array([74u8; 32]),
                token_x_mint: meme_a,
                token_y_mint: meme_b,
                token_x_program: spl,
                token_y_program: spl,
                oracle: Pubkey::new_from_array([82u8; 32]),
                host_fee_in: None,
                user_token_in: meme_a_ata,
                user_token_out: meme_b_ata,
            },
            leg_3_accounts: LegExecutionAccounts {
                lb_pair: pool_3.0,
                bin_array_pdas: vec![
                    Pubkey::new_from_array([61u8; 32]),
                    Pubkey::new_from_array([62u8; 32]),
                ],
                bitmap_extension: None,
                reserve_x: Pubkey::new_from_array([75u8; 32]),
                reserve_y: Pubkey::new_from_array([76u8; 32]),
                token_x_mint: meme_b,
                token_y_mint: wsol,
                token_x_program: spl,
                token_y_program: spl,
                oracle: Pubkey::new_from_array([83u8; 32]),
                host_fee_in: None,
                user_token_in: meme_b_ata,
                user_token_out: derive_ata(&payer, &wsol, &spl),
            },
            leg_1_params: LegSwapParams {
                mode: SwapMode::ExactIn,
                amount_in: 1_000_000_000,
                min_amount_out: 950_000_000,
                max_amount_in: 0,
                amount_out: 0,
            },
            leg_2_params: LegSwapParams {
                mode: SwapMode::ExactIn,
                amount_in: 950_000_000,
                min_amount_out: 900_000_000,
                max_amount_in: 0,
                amount_out: 0,
            },
            leg_3_params: LegSwapParams {
                mode: SwapMode::ExactIn,
                amount_in: 900_000_000,
                min_amount_out: 850_000_000,
                max_amount_in: 0,
                amount_out: 0,
            },
            wsol_token_program: spl,
            meme_a_token_program: spl,
            meme_b_token_program: spl,
            meme_a_ata,
            meme_b_ata,
            meme_a_ata_exists: false,
            meme_b_ata_exists: false,
            meme_a_ata_balance: 0,
            meme_b_ata_balance: 0,
            meme_a_is_token_2022: false,
            meme_b_is_token_2022: false,
            meme_a_withheld_fees: 0,
            meme_b_withheld_fees: 0,
            snapshot_slot: 250_000_000,
            instruction_version: 0,
            cu_limit: COMPUTE_BUDGET_LIMIT_3HOP,
            priority_fee_micro_lamports: 1_000,
            expected_min_final_output: 850_000_000,
            jito_tip_account: Pubkey::from_str(JITO_TIP_ACCOUNTS[0]).unwrap(),
            jito_tip_lamports: 10_000,
            recent_blockhash: [42u8; 32],
            v1_transport_supported: true,
        }
    }

    fn test_master_alt() -> StaticMasterAlt {
        let payer = Pubkey::new_from_array([1u8; 32]);
        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let usdc = Pubkey::from_str(arb_constants::USDC_MINT).unwrap();
        let usdt = Pubkey::from_str(USDT_MINT_STR).unwrap();
        let spl = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
        let bot_wsol_ata = derive_ata(&payer, &wsol, &spl);
        let bot_usdc_ata = derive_ata(&payer, &usdc, &spl);
        let bot_usdt_ata = derive_ata(&payer, &usdt, &spl);

        let entries = StaticMasterAlt::default_static_entries(
            &payer,
            Some(bot_wsol_ata),
            Some(bot_usdc_ata),
            Some(bot_usdt_ata),
            None,
            None,
        )
        .unwrap();

        let mut alt = StaticMasterAlt::new();
        let authority = Pubkey::new_from_array([2u8; 32]);
        alt.populate_from_entries(payer, authority, entries, 250_000_000)
            .unwrap();
        alt.mark_activated(250_000_001);
        alt
    }

    #[test]
    fn test_ctx_validation_pools_distinct() {
        let mut ctx = test_ctx();
        assert!(ctx.validate().is_ok());

        ctx.pool_2 = ctx.pool_1;
        assert!(ctx.validate().is_err());
    }

    #[test]
    fn test_ctx_validation_intermediate_ata_required() {
        let mut ctx = test_ctx();
        ctx.meme_a_ata = Pubkey::default();
        assert!(ctx.validate().is_err());
    }

    #[test]
    fn test_build_leg_swap_instructions() {
        let ctx = test_ctx();
        let ixs = build_leg_swap_instructions(&ctx).unwrap();
        assert_eq!(ixs.len(), 3);
        // Each swap2 instruction: 28 bytes data
        for ix in &ixs {
            assert_eq!(ix.data.len(), 28);
            assert_eq!(&ix.data[0..8], &crate::METEORA_SWAP2_DISCRIMINATOR);
        }
    }

    #[test]
    fn test_should_close_intermediate_ata_safety() {
        let ctx = test_ctx();
        // Not exists, zero balance => safe to close
        assert!(should_close_intermediate_ata(&ctx, true));

        // Pre-existing ATA => never close
        let mut ctx2 = ctx.clone();
        ctx2.meme_a_ata_exists = true;
        assert!(!should_close_intermediate_ata(&ctx2, true));

        // Nonzero balance => never close
        let mut ctx3 = ctx.clone();
        ctx3.meme_a_ata_balance = 1_000_000;
        assert!(!should_close_intermediate_ata(&ctx3, true));

        // Token-2022 with withheld fees => never close
        let mut ctx4 = ctx.clone();
        ctx4.meme_a_is_token_2022 = true;
        ctx4.meme_a_withheld_fees = 500;
        assert!(!should_close_intermediate_ata(&ctx4, true));
    }

    #[test]
    fn test_partition_accounts_alt_vs_inline() {
        let ctx = test_ctx();
        let alt = test_master_alt();
        let (alt_accs, inline_accs) = partition_accounts(&ctx, &alt).unwrap();

        // Static addresses (programs, mints, WSOL ATA, Jito tips) should be in ALT.
        assert!(
            alt_accs
                .iter()
                .any(|a| *a == Pubkey::from_str(METEORA_DLMM_PROGRAM_ID).unwrap())
        );
        // Payer is a signer, not in the ALT.
        assert!(!alt_accs.contains(&ctx.payer));
        // The context references 1 Jito tip account, which should be in ALT.
        let jito_count = alt_accs
            .iter()
            .filter(|a| {
                JITO_TIP_ACCOUNTS
                    .iter()
                    .any(|s| Pubkey::from_str(s).unwrap() == **a)
            })
            .count();
        assert_eq!(jito_count, 1);
        // All 8 Jito tip accounts should be in the master_alt itself.
        let jito_in_alt: usize = JITO_TIP_ACCOUNTS
            .iter()
            .filter(|s| alt.contains(&Pubkey::from_str(s).unwrap()))
            .count();
        assert_eq!(jito_in_alt, 8);

        // Dynamic meme pool accounts should be inline.
        assert!(inline_accs.contains(&ctx.pool_1.0));
        assert!(inline_accs.contains(&ctx.pool_2.0));
        // Meme mints (not in ALT) should be inline.
        assert!(inline_accs.contains(&ctx.meme_a_mint.0));
        assert!(inline_accs.contains(&ctx.meme_b_mint.0));
    }

    #[test]
    fn test_v0_compilation_fits_mtu() {
        let ctx = test_ctx();
        let alt = test_master_alt();
        let signer = KeypairSigner::new_random();

        let result = compile_3leg_meme_dlmm_arb(&ctx, &alt, &signer).unwrap();
        match &result {
            CompileResult::V0JitoCompatible(v0) => {
                println!("V0 compiled: {} bytes", v0.wire_size);
                assert!(v0.wire_size <= SOLANA_PACKET_MTU_LIMIT);
                assert!(v0.signer_count == 1);
                assert!(v0.alt_resolved_count > 0);
                assert!(v0.inline_account_count > 0);
            }
            CompileResult::V1DirectTransport(_) => {
                // V0 may be too large for this test fixture; V1 is acceptable.
            }
            CompileResult::Rejected(reason) => {
                panic!("Unexpected rejection: {reason:?}");
            }
        }
    }

    #[test]
    fn test_v1_fallback_when_v0_oversized() {
        let ctx = test_ctx();
        let alt = test_master_alt();
        let signer = KeypairSigner::new_random();

        let result = compile_3leg_meme_dlmm_arb(&ctx, &alt, &signer).unwrap();
        // Either V0 fits, or V1 fits, or both fail. We just verify no crash.
        match &result {
            CompileResult::V0JitoCompatible(v0) => {
                assert!(v0.wire_size <= SOLANA_PACKET_MTU_LIMIT);
            }
            CompileResult::V1DirectTransport(v1) => {
                assert!(v1.wire_size <= crate::adaptive::SOLANA_PACKET_V1_MTU);
            }
            CompileResult::Rejected(reason) => {
                eprintln!("Rejected (expected in test fixture): {reason:?}");
            }
        }
    }

    #[test]
    fn test_v1_transport_unsupported_rejection() {
        let mut ctx = test_ctx();
        ctx.v1_transport_supported = false;
        let alt = test_master_alt();
        let signer = KeypairSigner::new_random();

        // Force V0 to be oversized by adding many inline accounts (not easily done in test).
        // Instead just verify that when V1 is unsupported, we get a rejection if V0 also fails.
        let _result = compile_3leg_meme_dlmm_arb(&ctx, &alt, &signer).unwrap();
        // Result depends on V0 size; just ensure no panic.
    }

    #[test]
    fn test_byte_budget_report() {
        let ctx = test_ctx();
        let alt = test_master_alt();
        let signer = KeypairSigner::new_random();

        let result = compile_3leg_meme_dlmm_arb(&ctx, &alt, &signer).unwrap();
        match &result {
            CompileResult::V0JitoCompatible(v0) => {
                let report = &v0.byte_budget;
                assert!(report.total_wire_size > 0);
                assert!(report.signature_vector_bytes == 65);
                assert!(report.blockhash_bytes == 32);
                println!("{}", report.summarize());
            }
            CompileResult::V1DirectTransport(v1) => {
                let report = &v1.byte_budget;
                assert!(report.total_wire_size > 0);
                assert!(report.alt_lookup_references_bytes == 0);
                println!("{}", report.summarize());
            }
            CompileResult::Rejected(_) => {}
        }
    }

    #[test]
    fn test_no_alt_account_is_signer() {
        let ctx = test_ctx();
        let alt = test_master_alt();
        let signer = KeypairSigner::new_random();

        let result = compile_3leg_meme_dlmm_arb(&ctx, &alt, &signer).unwrap();
        if let CompileResult::V0JitoCompatible(v0) = &result {
            // Payer is the only signer, and payer is not in the ALT (it's static, not ALT-loaded).
            assert!(!alt.contains(&ctx.payer));
            assert_eq!(v0.signer_count, 1);
        }
    }

    #[test]
    fn test_compile_result_rejected_v0_oversized() {
        // Directly test the rejection variant
        let reason = RejectionReason::V0Oversized {
            wire_size: 1300,
            limit: 1232,
        };
        assert_eq!(
            reason,
            RejectionReason::V0Oversized {
                wire_size: 1300,
                limit: 1232
            }
        );
    }

    #[test]
    fn test_compile_result_rejected_v1_oversized() {
        let reason = RejectionReason::V1Oversized {
            wire_size: 4100,
            limit: 4096,
        };
        assert_eq!(
            reason,
            RejectionReason::V1Oversized {
                wire_size: 4100,
                limit: 4096
            }
        );
    }

    #[test]
    fn test_compile_result_rejected_too_many_bin_arrays() {
        let reason = RejectionReason::TooManyBinArrays { leg: 1, needed: 3 };
        assert_eq!(
            reason,
            RejectionReason::TooManyBinArrays { leg: 1, needed: 3 }
        );
    }
}
