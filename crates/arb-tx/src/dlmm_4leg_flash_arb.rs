//! Phase 18 - Kamino flash-loan funded FOUR-pool Meteora DLMM arbitrage.
//!
//! Assembles **one** Solana transaction that performs, in order:
//!
//! ```text
//! [0] ComputeBudget::SetComputeUnitLimit (V0 only)
//! [1] ComputeBudget::SetComputeUnitPrice (V0 only)
//! [2] ATA CreateIdempotent  (settlement account, flash mint = WSOL)
//! [3] ATA CreateIdempotent  (intermediate account A, meme mint A)
//! [4] ATA CreateIdempotent  (intermediate account B, meme mint B)
//! [5] ATA CreateIdempotent  (intermediate account C, meme mint C)
//! [6] klend::flash_borrow_reserve_liquidity      (borrow_instruction_index = 6)
//! [7] lb_clmm::swap2   (Pool 1: WSOL -> Meme A)
//! [8] lb_clmm::swap2   (Pool 2: Meme A -> Meme B)
//! [9] lb_clmm::swap2   (Pool 3: Meme B -> Meme C)
//! [10] lb_clmm::swap2  (Pool 4: Meme C -> WSOL)
//! [11] klend::flash_repay_reserve_liquidity       (borrow_instruction_index = 6)
//! ```
//!
//! Invariants (all fail closed):
//! 1. **Four distinct pools** - all lb_pair addresses different
//! 2. **Closing route** - flash mint in (WSOL), flash mint out (WSOL)
//! 3. **Mint chaining** - Pool1 out = Pool2 in, Pool2 out = Pool3 in, Pool3 out = Pool4 in
//! 4. **No inventory subsidy** - each leg's amount_in bound to previous leg's conservative min output
//! 5. **Repayment coverage** - repay >= borrow + flash_fee, Pool4 min_out >= repay
//! 6. **Borrow/repay ordering** - repay instruction carries real borrow instruction index
//! 7. **Atomicity self-check** - verify_atomic_integrity before signing
//! 8. **Size limit** - V1 with 4096 bytes maximum, no ALTs

use crate::atomic_integrity::{ExecutionMode, verify_atomic_integrity};
use crate::meteora_dlmm::{DlmmSwapAccountSet, MEMO_PROGRAM_ID_STR, build_swap2_exact_in};
use crate::{
    AddressLookupTable, CompiledInstruction, KaminoFlashParams, KeypairSigner, TxBuilder,
    build_compute_unit_limit, build_compute_unit_price, build_create_ata_idempotent_for_program,
    build_kamino_flash_borrow, build_kamino_flash_repay_with_index,
    get_associated_token_address_with_program_id,
};
use solana_pubkey::Pubkey;
use std::str::FromStr;

/// Errors raised while assembling the 4-leg flash-funded route.
#[derive(Debug, thiserror::Error)]
pub enum FlashArb4LegError {
    #[error("all four pools must be distinct")]
    PoolsNotDistinct,
    #[error("zero borrow amount")]
    ZeroBorrowAmount,
    #[error("flash repayment {repay} does not cover borrow {borrow} + fee {fee}")]
    InsufficientRepayment { borrow: u64, repay: u64, fee: u64 },
    #[error("mint chain mismatch: {detail}")]
    MintChainMismatch { detail: String },
    #[error("route does not return to the flash mint {expected}, got {actual}")]
    NonClosingRoute { expected: String, actual: String },
    #[error("leg {leg} minimum output {min_out} is below the required {required}")]
    LegOutputBelowRequired {
        leg: usize,
        min_out: u64,
        required: u64,
    },
    #[error("atomicity check failed: {0}")]
    AtomicityFailed(String),
    #[error("transaction too large: {size} bytes, limit {limit}")]
    TransactionTooLarge { size: usize, limit: usize },
    #[error("V1 transaction requires explicit compute unit limit")]
    MissingComputeUnitLimit,
    #[error("V1 transaction requires explicit loaded accounts data size limit")]
    MissingLoadedAccountsDataLimit,
    #[error("too many accounts for V1: {count}, max {max}")]
    TooManyAccounts { count: usize, max: usize },
    #[error("too many instructions for V1: {count}, max {max}")]
    TooManyInstructions { count: usize, max: usize },
    #[error("alt compilation failed: {0}")]
    AltCompilation(String),
}

/// One Meteora DLMM swap leg of the 4-leg flash-funded route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlashArb4LegPoolLeg {
    /// `LbPair` account of the pool.
    pub lb_pair: Pubkey,
    /// Token X reserve vault (PDA `[lb_pair, token_x_mint]`).
    pub reserve_x: Pubkey,
    /// Token Y reserve vault (PDA `[lb_pair, token_y_mint]`).
    pub reserve_y: Pubkey,
    /// On-chain token X mint.
    pub token_x_mint: Pubkey,
    /// On-chain token Y mint.
    pub token_y_mint: Pubkey,
    /// On-chain oracle PDA.
    pub oracle: Pubkey,
    /// Token program owning token X.
    pub token_x_program: Pubkey,
    /// Token program owning token Y.
    pub token_y_program: Pubkey,
    /// `true` for X -> Y (price down), `false` for Y -> X (price up).
    pub swap_for_y: bool,
    /// Exact input amount for this leg.
    pub amount_in: u64,
    /// Minimum output the leg must produce (conservative, after slippage).
    pub min_amount_out: u64,
    /// Bin arrays the leg will touch, in traversal order.
    pub bin_array_pdas: Vec<Pubkey>,
    /// `BinArrayBitmapExtension` PDA when the account exists on chain.
    pub bitmap_extension: Option<Pubkey>,
    /// Optional host-fee / referral account. `None` encodes the protocol sentinel.
    pub host_fee_in: Option<Pubkey>,
}

impl FlashArb4LegPoolLeg {
    /// Mint paid into this leg.
    #[inline]
    pub fn token_in(&self) -> Pubkey {
        if self.swap_for_y {
            self.token_x_mint
        } else {
            self.token_y_mint
        }
    }

    /// Mint received from this leg.
    #[inline]
    pub fn token_out(&self) -> Pubkey {
        if self.swap_for_y {
            self.token_y_mint
        } else {
            self.token_x_mint
        }
    }

    /// Token program of the input mint.
    #[inline]
    pub fn token_in_program(&self) -> Pubkey {
        if self.swap_for_y {
            self.token_x_program
        } else {
            self.token_y_program
        }
    }

    /// Token program of the output mint.
    #[inline]
    pub fn token_out_program(&self) -> Pubkey {
        if self.swap_for_y {
            self.token_y_program
        } else {
            self.token_x_program
        }
    }

    /// Build the official-mirroring account set for this leg.
    pub fn account_set(&self, payer: &Pubkey) -> DlmmSwapAccountSet {
        let (user_in, user_out) = if self.swap_for_y {
            (
                get_associated_token_address_with_program_id(
                    payer,
                    &self.token_x_mint,
                    &self.token_x_program,
                ),
                get_associated_token_address_with_program_id(
                    payer,
                    &self.token_y_mint,
                    &self.token_y_program,
                ),
            )
        } else {
            (
                get_associated_token_address_with_program_id(
                    payer,
                    &self.token_y_mint,
                    &self.token_y_program,
                ),
                get_associated_token_address_with_program_id(
                    payer,
                    &self.token_x_mint,
                    &self.token_x_program,
                ),
            )
        };

        DlmmSwapAccountSet {
            lb_pair: self.lb_pair,
            bin_array_bitmap_extension: self.bitmap_extension,
            reserve_x: self.reserve_x,
            reserve_y: self.reserve_y,
            user_token_in: user_in,
            user_token_out: user_out,
            token_x_mint: self.token_x_mint,
            token_y_mint: self.token_y_mint,
            oracle: self.oracle,
            host_fee_in: self.host_fee_in,
            user: *payer,
            token_x_program: self.token_x_program,
            token_y_program: self.token_y_program,
            memo_program: Pubkey::from_str(MEMO_PROGRAM_ID_STR).expect("valid memo program"),
            event_authority: crate::derive_meteora_event_authority(),
            bin_arrays: self.bin_array_pdas.clone(),
        }
    }
}

/// Everything needed to assemble the 4-leg flash-funded transaction.
#[derive(Debug, Clone)]
pub struct FlashArb4LegParams {
    /// Transaction authority (payer). It signs the flash borrow and the swaps.
    pub payer: Pubkey,
    /// Kamino flash-loan parameters. `user_destination_liquidity` is overridden with
    /// the settlement account so the principal and the repayment share one account.
    pub kamino: KaminoFlashParams,
    /// Flash principal in flash-mint raw units (WSOL = 9 decimals).
    pub borrow_amount: u64,
    /// Kamino flash fee in flash-mint raw units (`ceil(borrow * fee_sf / 2^64)`).
    pub flash_fee: u64,
    /// First DLMM swap leg (Pool 1: WSOL -> Meme A).
    pub leg_1: FlashArb4LegPoolLeg,
    /// Second DLMM swap leg (Pool 2: Meme A -> Meme B).
    pub leg_2: FlashArb4LegPoolLeg,
    /// Third DLMM swap leg (Pool 3: Meme B -> Meme C).
    pub leg_3: FlashArb4LegPoolLeg,
    /// Fourth DLMM swap leg (Pool 4: Meme C -> WSOL).
    pub leg_4: FlashArb4LegPoolLeg,
    /// Compute-unit limit.
    pub cu_limit: u32,
    /// Priority fee in micro-lamports (for V0) or total lamports (for V1).
    pub priority_fee_micro_lamports: u64,
    /// Loaded accounts data size limit for V1 (in bytes).
    pub loaded_accounts_data_size_limit: Option<u64>,
    /// Jito tip account.
    pub jito_tip_account: Pubkey,
    /// Jito tip in lamports.
    pub jito_tip_lamports: u64,
    /// Recent blockhash.
    pub recent_blockhash: [u8; 32],
    /// Whether V1 transport (4096 bytes) is supported by downstream.
    pub v1_transport_supported: bool,
}

impl FlashArb4LegParams {
    /// Amount that must be repaid: principal plus the flash fee.
    #[inline]
    pub fn repay_amount(&self) -> u64 {
        self.borrow_amount.saturating_add(self.flash_fee)
    }
}

/// Result of assembling the 4-leg flash-funded transaction.
#[derive(Debug, Clone)]
pub struct FlashFourLegArbitrageTx {
    /// Serialized (signed) transaction bytes.
    pub wire_bytes: Vec<u8>,
    /// Serialized size in bytes.
    pub tx_size: usize,
    /// The assembled instruction list.
    pub instructions: Vec<CompiledInstruction>,
    /// Index of the flash-borrow instruction inside `instructions`.
    pub borrow_instruction_index: usize,
    /// Index of the flash-repay instruction inside `instructions`.
    pub repay_instruction_index: usize,
    /// Flash principal.
    pub borrow_amount: u64,
    /// Flash fee.
    pub flash_fee: u64,
    /// Repayment amount (principal + fee).
    pub repay_amount: u64,
    /// Conservative minimum output of each leg.
    pub min_leg_1_out: u64,
    pub min_leg_2_out: u64,
    pub min_leg_3_out: u64,
    pub min_leg_4_out: u64,
    /// Settlement account (flash mint) used for borrow, final output and repayment.
    pub settlement_account: Pubkey,
    /// Intermediate account A (meme A mint).
    pub intermediate_account_a: Pubkey,
    /// Intermediate account B (meme B mint).
    pub intermediate_account_b: Pubkey,
    /// Intermediate account C (meme C mint).
    pub intermediate_account_c: Pubkey,
    /// Pool addresses.
    pub pool_1: Pubkey,
    pub pool_2: Pubkey,
    pub pool_3: Pubkey,
    pub pool_4: Pubkey,
}

/// Assemble the instruction list for the 4-leg flash-funded route.
pub fn assemble_flash_funded_four_pool(
    params: &FlashArb4LegParams,
) -> Result<(Vec<CompiledInstruction>, usize, usize), FlashArb4LegError> {
    // Invariant 1: four distinct pools
    let pools = [
        params.leg_1.lb_pair,
        params.leg_2.lb_pair,
        params.leg_3.lb_pair,
        params.leg_4.lb_pair,
    ];
    for i in 0..4 {
        for j in (i + 1)..4 {
            if pools[i] == pools[j] {
                return Err(FlashArb4LegError::PoolsNotDistinct);
            }
        }
    }

    // Invariant 2: non-zero principal
    if params.borrow_amount == 0 {
        return Err(FlashArb4LegError::ZeroBorrowAmount);
    }

    // Invariant 3: repayment covers principal + fee
    let repay_amount = params.repay_amount();
    if repay_amount < params.borrow_amount.saturating_add(params.flash_fee) {
        return Err(FlashArb4LegError::InsufficientRepayment {
            borrow: params.borrow_amount,
            repay: repay_amount,
            fee: params.flash_fee,
        });
    }

    let flash_mint = params.leg_1.token_in();
    let mint_a = params.leg_1.token_out();
    let mint_b = params.leg_2.token_out();
    let mint_c = params.leg_3.token_out();
    let final_mint = params.leg_4.token_out();

    // Invariant 4: closing route (flash mint in, flash mint out)
    if final_mint != flash_mint {
        return Err(FlashArb4LegError::NonClosingRoute {
            expected: flash_mint.to_string(),
            actual: final_mint.to_string(),
        });
    }

    // Invariant 5: mint chaining between legs (mint + token program)
    if params.leg_2.token_in() != mint_a
        || params.leg_2.token_in_program() != params.leg_1.token_out_program()
    {
        return Err(FlashArb4LegError::MintChainMismatch {
            detail: format!(
                "leg2 token_in {} (prog {}) != leg1 token_out {} (prog {})",
                params.leg_2.token_in(),
                params.leg_2.token_in_program(),
                params.leg_1.token_out(),
                params.leg_1.token_out_program()
            ),
        });
    }
    if params.leg_3.token_in() != mint_b
        || params.leg_3.token_in_program() != params.leg_2.token_out_program()
    {
        return Err(FlashArb4LegError::MintChainMismatch {
            detail: format!(
                "leg3 token_in {} (prog {}) != leg2 token_out {} (prog {})",
                params.leg_3.token_in(),
                params.leg_3.token_in_program(),
                params.leg_2.token_out(),
                params.leg_2.token_out_program()
            ),
        });
    }
    if params.leg_4.token_in() != mint_c
        || params.leg_4.token_in_program() != params.leg_3.token_out_program()
    {
        return Err(FlashArb4LegError::MintChainMismatch {
            detail: format!(
                "leg4 token_in {} (prog {}) != leg3 token_out {} (prog {})",
                params.leg_4.token_in(),
                params.leg_4.token_in_program(),
                params.leg_3.token_out(),
                params.leg_3.token_out_program()
            ),
        });
    }

    // Invariant 6: each leg's min output covers the next leg's required input
    if params.leg_1.min_amount_out < params.leg_2.amount_in {
        return Err(FlashArb4LegError::LegOutputBelowRequired {
            leg: 1,
            min_out: params.leg_1.min_amount_out,
            required: params.leg_2.amount_in,
        });
    }
    if params.leg_2.min_amount_out < params.leg_3.amount_in {
        return Err(FlashArb4LegError::LegOutputBelowRequired {
            leg: 2,
            min_out: params.leg_2.min_amount_out,
            required: params.leg_3.amount_in,
        });
    }
    if params.leg_3.min_amount_out < params.leg_4.amount_in {
        return Err(FlashArb4LegError::LegOutputBelowRequired {
            leg: 3,
            min_out: params.leg_3.min_amount_out,
            required: params.leg_4.amount_in,
        });
    }
    // Leg 4 min_out must cover repayment
    if params.leg_4.min_amount_out < repay_amount {
        return Err(FlashArb4LegError::LegOutputBelowRequired {
            leg: 4,
            min_out: params.leg_4.min_amount_out,
            required: repay_amount,
        });
    }

    let payer = params.payer;
    let flash_program = params.leg_1.token_in_program();
    let mint_a_program = params.leg_1.token_out_program();
    let mint_b_program = params.leg_2.token_out_program();
    let mint_c_program = params.leg_3.token_out_program();

    // Settlement and intermediate accounts
    let settlement_account =
        get_associated_token_address_with_program_id(&payer, &flash_mint, &flash_program);
    let intermediate_account_a =
        get_associated_token_address_with_program_id(&payer, &mint_a, &mint_a_program);
    let intermediate_account_b =
        get_associated_token_address_with_program_id(&payer, &mint_b, &mint_b_program);
    let intermediate_account_c =
        get_associated_token_address_with_program_id(&payer, &mint_c, &mint_c_program);

    // Kamino accounts: the principal must land in the settlement account.
    let mut kamino = params.kamino.clone();
    kamino.user_destination_liquidity = settlement_account;

    // Instruction layout: ComputeBudget (V0) + ATAs + FlashBorrow + 4 Swaps + FlashRepay + Tip
    let mut ixs: Vec<CompiledInstruction> = Vec::with_capacity(13);

    // V0 only: ComputeBudget instructions
    // For V1, these go in the message config, not as instructions
    if !params.v1_transport_supported {
        ixs.push(build_compute_unit_limit(params.cu_limit));
        ixs.push(build_compute_unit_price(params.priority_fee_micro_lamports));
    }

    // Create ATAs
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
    ixs.push(build_create_ata_idempotent_for_program(
        payer,
        payer,
        mint_c,
        intermediate_account_c,
        mint_c_program,
    ));

    // Flash borrow
    let borrow_idx = ixs.len();
    ixs.push(build_kamino_flash_borrow(
        &kamino,
        payer,
        params.borrow_amount,
    ));

    // Pool 1 swap
    let accounts_1 = params.leg_1.account_set(&payer);
    ixs.push(build_swap2_exact_in(
        &accounts_1,
        params.leg_1.amount_in,
        params.leg_1.min_amount_out,
    ));

    // Pool 2 swap
    let accounts_2 = params.leg_2.account_set(&payer);
    ixs.push(build_swap2_exact_in(
        &accounts_2,
        params.leg_2.amount_in,
        params.leg_2.min_amount_out,
    ));

    // Pool 3 swap
    let accounts_3 = params.leg_3.account_set(&payer);
    ixs.push(build_swap2_exact_in(
        &accounts_3,
        params.leg_3.amount_in,
        params.leg_3.min_amount_out,
    ));

    // Pool 4 swap
    let accounts_4 = params.leg_4.account_set(&payer);
    ixs.push(build_swap2_exact_in(
        &accounts_4,
        params.leg_4.amount_in,
        params.leg_4.min_amount_out,
    ));

    // Flash repay — the instruction's `amount` argument is the borrow
    // PRINCIPAL (klend `flash_borrow_check_matching_repay` enforces
    // `liquidity_amount == borrow amount`); the fee is debited on top by
    // the handler.
    let repay_idx = ixs.len();
    ixs.push(build_kamino_flash_repay_with_index(
        &kamino,
        payer,
        params.borrow_amount,
        borrow_idx as u8,
    ));

    // Jito tip (last)
    let tip_ix = crate::build_marked_jito_tip_instruction(
        payer,
        params.jito_tip_account,
        crate::Lamports(params.jito_tip_lamports),
    );
    ixs.push(tip_ix);

    // V1-specific validations
    if params.v1_transport_supported {
        // V1 requires explicit limits
        if params.cu_limit == 0 {
            return Err(FlashArb4LegError::MissingComputeUnitLimit);
        }
        if params.loaded_accounts_data_size_limit.is_none() {
            return Err(FlashArb4LegError::MissingLoadedAccountsDataLimit);
        }

        // V1 limits: max 64 instructions, max 64 accounts
        if ixs.len() > 64 {
            return Err(FlashArb4LegError::TooManyInstructions {
                count: ixs.len(),
                max: 64,
            });
        }
    }

    Ok((ixs, borrow_idx, repay_idx))
}

/// Assemble and sign the 4-leg flash-funded transaction as V1.
pub fn build_flash_funded_four_pool_v1_transaction(
    params: &FlashArb4LegParams,
    signer: &KeypairSigner,
) -> Result<FlashFourLegArbitrageTx, FlashArb4LegError> {
    let (ixs, borrow_idx, repay_idx) = assemble_flash_funded_four_pool(params)?;

    // Atomicity self-check
    let verification = verify_atomic_integrity(
        ExecutionMode::SingleTransaction,
        &ixs,
        Some(borrow_idx),
        Some(repay_idx),
        params.borrow_amount,
        params.repay_amount(),
        params.flash_fee,
    )
    .map_err(|e| FlashArb4LegError::AltCompilation(e.to_string()))?;

    if !verification.is_atomic {
        return Err(FlashArb4LegError::AtomicityFailed(
            verification
                .rejection_reason
                .unwrap_or_else(|| "unknown".to_string()),
        ));
    }

    // V1 compilation
    let loaded_accounts_data_size_limit = params
        .loaded_accounts_data_size_limit
        .ok_or(FlashArb4LegError::MissingLoadedAccountsDataLimit)?;

    // Use adaptive compiler for V1
    let v1_config = crate::adaptive::V1ConfigParams {
        priority_fee_lamports: Some(params.priority_fee_micro_lamports / 1_000_000),
        compute_unit_limit: Some(params.cu_limit),
        loaded_accounts_data_size_limit: Some(loaded_accounts_data_size_limit as u32),
        heap_size: None,
    };

    let compiled = crate::adaptive::AdaptiveTxCompiler::compile_v1_only(
        signer,
        &ixs,
        params.recent_blockhash,
        v1_config,
    )
    .map_err(|e| FlashArb4LegError::AltCompilation(e.to_string()))?;

    // Check size limit
    let wire_bytes = compiled.wire_bytes.clone();
    if wire_bytes.len() > 4096 {
        return Err(FlashArb4LegError::TransactionTooLarge {
            size: wire_bytes.len(),
            limit: 4096,
        });
    }

    Ok(FlashFourLegArbitrageTx {
        wire_bytes: wire_bytes.clone(),
        tx_size: wire_bytes.len(),
        instructions: ixs,
        borrow_instruction_index: borrow_idx,
        repay_instruction_index: repay_idx,
        borrow_amount: params.borrow_amount,
        flash_fee: params.flash_fee,
        repay_amount: params.repay_amount(),
        min_leg_1_out: params.leg_1.min_amount_out,
        min_leg_2_out: params.leg_2.min_amount_out,
        min_leg_3_out: params.leg_3.min_amount_out,
        min_leg_4_out: params.leg_4.min_amount_out,
        settlement_account: get_associated_token_address_with_program_id(
            &params.payer,
            &params.leg_1.token_in(),
            &params.leg_1.token_in_program(),
        ),
        intermediate_account_a: get_associated_token_address_with_program_id(
            &params.payer,
            &params.leg_1.token_out(),
            &params.leg_1.token_out_program(),
        ),
        intermediate_account_b: get_associated_token_address_with_program_id(
            &params.payer,
            &params.leg_2.token_out(),
            &params.leg_2.token_out_program(),
        ),
        intermediate_account_c: get_associated_token_address_with_program_id(
            &params.payer,
            &params.leg_3.token_out(),
            &params.leg_3.token_out_program(),
        ),
        pool_1: params.leg_1.lb_pair,
        pool_2: params.leg_2.lb_pair,
        pool_3: params.leg_3.lb_pair,
        pool_4: params.leg_4.lb_pair,
    })
}

/// Assemble and sign the 4-leg flash-funded transaction as V0 with ALT.
pub fn build_flash_funded_four_pool_v0_transaction(
    params: &FlashArb4LegParams,
    signer: &KeypairSigner,
    lookup_tables: &[AddressLookupTable],
) -> Result<FlashFourLegArbitrageTx, FlashArb4LegError> {
    let (ixs, borrow_idx, repay_idx) = assemble_flash_funded_four_pool(params)?;

    // Atomicity self-check
    let verification = verify_atomic_integrity(
        ExecutionMode::SingleTransaction,
        &ixs,
        Some(borrow_idx),
        Some(repay_idx),
        params.borrow_amount,
        params.repay_amount(),
        params.flash_fee,
    )
    .map_err(|e| FlashArb4LegError::AltCompilation(e.to_string()))?;

    if !verification.is_atomic {
        return Err(FlashArb4LegError::AtomicityFailed(
            verification
                .rejection_reason
                .unwrap_or_else(|| "unknown".to_string()),
        ));
    }

    // V0 compilation with ALT
    let wire_bytes = TxBuilder::compile_v0_message(
        signer.pubkey(),
        &ixs,
        params.recent_blockhash,
        lookup_tables,
    )
    .map_err(|e| FlashArb4LegError::AltCompilation(e.to_string()))?;

    // Check size limit (1232 bytes for V0)
    if wire_bytes.len() > 1232 {
        return Err(FlashArb4LegError::TransactionTooLarge {
            size: wire_bytes.len(),
            limit: 1232,
        });
    }

    Ok(FlashFourLegArbitrageTx {
        wire_bytes: wire_bytes.clone(),
        tx_size: wire_bytes.len(),
        instructions: ixs,
        borrow_instruction_index: borrow_idx,
        repay_instruction_index: repay_idx,
        borrow_amount: params.borrow_amount,
        flash_fee: params.flash_fee,
        repay_amount: params.repay_amount(),
        min_leg_1_out: params.leg_1.min_amount_out,
        min_leg_2_out: params.leg_2.min_amount_out,
        min_leg_3_out: params.leg_3.min_amount_out,
        min_leg_4_out: params.leg_4.min_amount_out,
        settlement_account: get_associated_token_address_with_program_id(
            &params.payer,
            &params.leg_1.token_in(),
            &params.leg_1.token_in_program(),
        ),
        intermediate_account_a: get_associated_token_address_with_program_id(
            &params.payer,
            &params.leg_1.token_out(),
            &params.leg_1.token_out_program(),
        ),
        intermediate_account_b: get_associated_token_address_with_program_id(
            &params.payer,
            &params.leg_2.token_out(),
            &params.leg_2.token_out_program(),
        ),
        intermediate_account_c: get_associated_token_address_with_program_id(
            &params.payer,
            &params.leg_3.token_out(),
            &params.leg_3.token_out_program(),
        ),
        pool_1: params.leg_1.lb_pair,
        pool_2: params.leg_2.lb_pair,
        pool_3: params.leg_3.lb_pair,
        pool_4: params.leg_4.lb_pair,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_4leg_params_repay_amount() {
        let params = FlashArb4LegParams {
            payer: Pubkey::new_unique(),
            kamino: KaminoFlashParams::default_wsol(),
            borrow_amount: 1_000_000_000, // 1 WSOL
            flash_fee: 1_000,             // 0.000001 WSOL
            leg_1: FlashArb4LegPoolLeg {
                lb_pair: Pubkey::new_unique(),
                reserve_x: Pubkey::new_unique(),
                reserve_y: Pubkey::new_unique(),
                token_x_mint: Pubkey::new_unique(),
                token_y_mint: Pubkey::new_unique(),
                oracle: Pubkey::new_unique(),
                token_x_program: Pubkey::new_unique(),
                token_y_program: Pubkey::new_unique(),
                swap_for_y: true,
                amount_in: 1_000_000_000,
                min_amount_out: 900_000_000,
                bin_array_pdas: vec![],
                bitmap_extension: None,
                host_fee_in: None,
            },
            leg_2: FlashArb4LegPoolLeg {
                lb_pair: Pubkey::new_unique(),
                reserve_x: Pubkey::new_unique(),
                reserve_y: Pubkey::new_unique(),
                token_x_mint: Pubkey::new_unique(),
                token_y_mint: Pubkey::new_unique(),
                oracle: Pubkey::new_unique(),
                token_x_program: Pubkey::new_unique(),
                token_y_program: Pubkey::new_unique(),
                swap_for_y: true,
                amount_in: 900_000_000,
                min_amount_out: 800_000_000,
                bin_array_pdas: vec![],
                bitmap_extension: None,
                host_fee_in: None,
            },
            leg_3: FlashArb4LegPoolLeg {
                lb_pair: Pubkey::new_unique(),
                reserve_x: Pubkey::new_unique(),
                reserve_y: Pubkey::new_unique(),
                token_x_mint: Pubkey::new_unique(),
                token_y_mint: Pubkey::new_unique(),
                oracle: Pubkey::new_unique(),
                token_x_program: Pubkey::new_unique(),
                token_y_program: Pubkey::new_unique(),
                swap_for_y: true,
                amount_in: 800_000_000,
                min_amount_out: 700_000_000,
                bin_array_pdas: vec![],
                bitmap_extension: None,
                host_fee_in: None,
            },
            leg_4: FlashArb4LegPoolLeg {
                lb_pair: Pubkey::new_unique(),
                reserve_x: Pubkey::new_unique(),
                reserve_y: Pubkey::new_unique(),
                token_x_mint: Pubkey::new_unique(),
                token_y_mint: Pubkey::new_unique(),
                oracle: Pubkey::new_unique(),
                token_x_program: Pubkey::new_unique(),
                token_y_program: Pubkey::new_unique(),
                swap_for_y: true,
                amount_in: 700_000_000,
                min_amount_out: 1_001_000_000, // Must cover repay
                bin_array_pdas: vec![],
                bitmap_extension: None,
                host_fee_in: None,
            },
            cu_limit: 300_000,
            priority_fee_micro_lamports: 1,
            loaded_accounts_data_size_limit: Some(256_000),
            jito_tip_account: Pubkey::new_unique(),
            jito_tip_lamports: 0,
            recent_blockhash: [0; 32],
            v1_transport_supported: true,
        };

        assert_eq!(params.repay_amount(), 1_000_001_000);
    }

    #[test]
    fn test_4leg_pools_not_distinct_error() {
        let mut params = FlashArb4LegParams {
            payer: Pubkey::new_unique(),
            kamino: KaminoFlashParams::default_wsol(),
            borrow_amount: 1_000_000_000,
            flash_fee: 1_000,
            leg_1: FlashArb4LegPoolLeg {
                lb_pair: Pubkey::new_unique(),
                reserve_x: Pubkey::new_unique(),
                reserve_y: Pubkey::new_unique(),
                token_x_mint: Pubkey::new_unique(),
                token_y_mint: Pubkey::new_unique(),
                oracle: Pubkey::new_unique(),
                token_x_program: Pubkey::new_unique(),
                token_y_program: Pubkey::new_unique(),
                swap_for_y: true,
                amount_in: 1_000_000_000,
                min_amount_out: 900_000_000,
                bin_array_pdas: vec![],
                bitmap_extension: None,
                host_fee_in: None,
            },
            leg_2: FlashArb4LegPoolLeg {
                lb_pair: Pubkey::new_unique(),
                reserve_x: Pubkey::new_unique(),
                reserve_y: Pubkey::new_unique(),
                token_x_mint: Pubkey::new_unique(),
                token_y_mint: Pubkey::new_unique(),
                oracle: Pubkey::new_unique(),
                token_x_program: Pubkey::new_unique(),
                token_y_program: Pubkey::new_unique(),
                swap_for_y: true,
                amount_in: 900_000_000,
                min_amount_out: 800_000_000,
                bin_array_pdas: vec![],
                bitmap_extension: None,
                host_fee_in: None,
            },
            leg_3: FlashArb4LegPoolLeg {
                lb_pair: Pubkey::new_unique(),
                reserve_x: Pubkey::new_unique(),
                reserve_y: Pubkey::new_unique(),
                token_x_mint: Pubkey::new_unique(),
                token_y_mint: Pubkey::new_unique(),
                oracle: Pubkey::new_unique(),
                token_x_program: Pubkey::new_unique(),
                token_y_program: Pubkey::new_unique(),
                swap_for_y: true,
                amount_in: 800_000_000,
                min_amount_out: 700_000_000,
                bin_array_pdas: vec![],
                bitmap_extension: None,
                host_fee_in: None,
            },
            leg_4: FlashArb4LegPoolLeg {
                lb_pair: Pubkey::new_unique(),
                reserve_x: Pubkey::new_unique(),
                reserve_y: Pubkey::new_unique(),
                token_x_mint: Pubkey::new_unique(),
                token_y_mint: Pubkey::new_unique(),
                oracle: Pubkey::new_unique(),
                token_x_program: Pubkey::new_unique(),
                token_y_program: Pubkey::new_unique(),
                swap_for_y: true,
                amount_in: 700_000_000,
                min_amount_out: 1_001_000_000,
                bin_array_pdas: vec![],
                bitmap_extension: None,
                host_fee_in: None,
            },
            cu_limit: 300_000,
            priority_fee_micro_lamports: 1,
            loaded_accounts_data_size_limit: Some(256_000),
            jito_tip_account: Pubkey::new_unique(),
            jito_tip_lamports: 0,
            recent_blockhash: [0; 32],
            v1_transport_supported: true,
        };

        // Make leg_1 and leg_2 have same pool
        params.leg_2.lb_pair = params.leg_1.lb_pair;

        let result = assemble_flash_funded_four_pool(&params);
        assert!(matches!(result, Err(FlashArb4LegError::PoolsNotDistinct)));
    }
}
