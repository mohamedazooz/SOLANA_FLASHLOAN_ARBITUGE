//! Phase 17+ — Kamino flash-loan funded THREE-pool Meteora DLMM arbitrage.
//!
//! Assembles **one** Solana transaction that performs, in order:
//!
//! ```text
//! [0] ComputeBudget::SetComputeUnitLimit
//! [1] ComputeBudget::SetComputeUnitPrice
//! [2] ATA CreateIdempotent  (settlement account, flash mint = WSOL)
//! [3] ATA CreateIdempotent  (intermediate account A, meme mint A)
//! [4] ATA CreateIdempotent  (intermediate account B, meme mint B)
//! [5] klend::flash_borrow_reserve_liquidity      (borrow_instruction_index = 5)
//! [6] lb_clmm::swap2   (Pool 1: WSOL -> Meme A)
//! [7] lb_clmm::swap2   (Pool 2: Meme A -> Meme B)
//! [8] lb_clmm::swap2   (Pool 3: Meme B -> WSOL)
//! [9] klend::flash_repay_reserve_liquidity       (borrow_instruction_index = 5)
//! ```
//!
//! Invariants (all fail closed):
//! 1. **Three distinct pools** — all lb_pair addresses different
//! 2. **Closing route** — flash mint in (WSOL), flash mint out (WSOL)
//! 3. **Mint chaining** — Pool1 out = Pool2 in, Pool2 out = Pool3 in, same token program per hop
//! 4. **No inventory subsidy** — each leg's amount_in bound to previous leg's *conservative min* output
//! 5. **Repayment coverage** — repay >= borrow + flash_fee, Pool3 min_out >= repay
//! 6. **Borrow/repay ordering** — repay instruction carries real borrow instruction index
//! 7. **Atomicity self-check** — verify_atomic_integrity before signing
//! 8. **Size limit** — v0 with ALT (1232 bytes) or V1 (4096 bytes)

use crate::atomic_integrity::{ExecutionMode, verify_atomic_integrity};
use crate::meteora_dlmm::{DlmmSwapAccountSet, MEMO_PROGRAM_ID_STR, build_swap2_exact_in};
use crate::{
    AddressLookupTable, CompiledInstruction, KaminoFlashParams, KeypairSigner, TxBuilder,
    build_compute_unit_limit, build_compute_unit_price, build_create_ata_idempotent_for_program,
    build_kamino_flash_borrow, build_kamino_flash_repay_with_index,
    get_associated_token_address_with_program_id,
};
use solana_pubkey::Pubkey;
use std::collections::BTreeMap;
use std::str::FromStr;

/// Errors raised while assembling the 3-leg flash-funded route.
#[derive(Debug, thiserror::Error)]
pub enum FlashArb3LegError {
    #[error("all three pools must be distinct")]
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
    #[error("alt compilation failed: {0}")]
    AltCompilation(String),
    #[error("params.payer {} does not match the signing key {}", payer, signer)]
    PayerSignerMismatch { payer: String, signer: String },
}

/// One Meteora DLMM swap leg of the 3-leg flash-funded route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlashArb3LegPoolLeg {
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

impl FlashArb3LegPoolLeg {
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

/// Everything needed to assemble the 3-leg flash-funded transaction.
#[derive(Debug, Clone)]
pub struct FlashArb3LegParams {
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
    pub leg_1: FlashArb3LegPoolLeg,
    /// Second DLMM swap leg (Pool 2: Meme A -> Meme B).
    pub leg_2: FlashArb3LegPoolLeg,
    /// Third DLMM swap leg (Pool 3: Meme B -> WSOL).
    pub leg_3: FlashArb3LegPoolLeg,
    /// Compute-unit limit.
    pub cu_limit: u32,
    /// Priority fee in micro-lamports.
    pub priority_fee_micro_lamports: u64,
    /// Jito tip account.
    pub jito_tip_account: Pubkey,
    /// Jito tip in lamports.
    pub jito_tip_lamports: u64,
    /// Recent blockhash.
    pub recent_blockhash: [u8; 32],
    /// Whether V1 transport (4096 bytes) is supported by downstream.
    pub v1_transport_supported: bool,
}

impl FlashArb3LegParams {
    /// Amount that must be repaid: principal plus the flash fee.
    #[inline]
    pub fn repay_amount(&self) -> u64 {
        self.borrow_amount.saturating_add(self.flash_fee)
    }
}

/// Result of assembling the 3-leg flash-funded transaction.
#[derive(Debug, Clone)]
pub struct FlashThreeLegArbitrageTx {
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
    /// Settlement account (flash mint) used for borrow, final output and repayment.
    pub settlement_account: Pubkey,
    /// Intermediate account A (meme A mint).
    pub intermediate_account_a: Pubkey,
    /// Intermediate account B (meme B mint).
    pub intermediate_account_b: Pubkey,
    /// Pool 1 `LbPair`.
    pub pool_1: Pubkey,
    /// Pool 2 `LbPair`.
    pub pool_2: Pubkey,
    /// Pool 3 `LbPair`.
    pub pool_3: Pubkey,
}

/// Assemble the instruction list for the 3-leg flash-funded route.
pub fn assemble_flash_funded_three_pool(
    params: &FlashArb3LegParams,
) -> Result<(Vec<CompiledInstruction>, usize, usize), FlashArb3LegError> {
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

    let flash_mint = params.leg_1.token_in();
    let mint_a = params.leg_1.token_out();
    let mint_b = params.leg_2.token_out();
    let final_mint = params.leg_3.token_out();

    // ── Invariant 4: closing route (flash mint in, flash mint out) ──
    if final_mint != flash_mint {
        return Err(FlashArb3LegError::NonClosingRoute {
            expected: flash_mint.to_string(),
            actual: final_mint.to_string(),
        });
    }
    // ── Invariant 5: mint chaining between legs (mint + token program) ──
    if params.leg_2.token_in() != mint_a
        || params.leg_2.token_in_program() != params.leg_1.token_out_program()
    {
        return Err(FlashArb3LegError::MintChainMismatch {
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
        return Err(FlashArb3LegError::MintChainMismatch {
            detail: format!(
                "leg3 token_in {} (prog {}) != leg2 token_out {} (prog {})",
                params.leg_3.token_in(),
                params.leg_3.token_in_program(),
                params.leg_2.token_out(),
                params.leg_2.token_out_program()
            ),
        });
    }
    // ── Invariant 6: each leg's min output covers the next leg's required input ──
    // Leg 1 min_out must cover Leg 2 amount_in
    if params.leg_1.min_amount_out < params.leg_2.amount_in {
        return Err(FlashArb3LegError::LegOutputBelowRequired {
            leg: 1,
            min_out: params.leg_1.min_amount_out,
            required: params.leg_2.amount_in,
        });
    }
    // Leg 2 min_out must cover Leg 3 amount_in
    if params.leg_2.min_amount_out < params.leg_3.amount_in {
        return Err(FlashArb3LegError::LegOutputBelowRequired {
            leg: 2,
            min_out: params.leg_2.min_amount_out,
            required: params.leg_3.amount_in,
        });
    }
    // Leg 3 min_out must cover repayment
    if params.leg_3.min_amount_out < repay_amount {
        return Err(FlashArb3LegError::LegOutputBelowRequired {
            leg: 3,
            min_out: params.leg_3.min_amount_out,
            required: repay_amount,
        });
    }

    let payer = params.payer;
    let flash_program = params.leg_1.token_in_program();
    let mint_a_program = params.leg_1.token_out_program();
    let mint_b_program = params.leg_2.token_out_program();

    // Settlement account: receives the flash principal, receives Pool 3's output and pays the repayment.
    let settlement_account =
        get_associated_token_address_with_program_id(&payer, &flash_mint, &flash_program);
    let intermediate_account_a =
        get_associated_token_address_with_program_id(&payer, &mint_a, &mint_a_program);
    let intermediate_account_b =
        get_associated_token_address_with_program_id(&payer, &mint_b, &mint_b_program);

    // Kamino accounts: the principal must land in the settlement account.
    let mut kamino = params.kamino.clone();
    kamino.user_destination_liquidity = settlement_account;

    // ── Instruction layout ──
    let mut ixs: Vec<CompiledInstruction> = Vec::with_capacity(11);
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

    // ── Flash borrow ──
    let borrow_idx = ixs.len();
    ixs.push(build_kamino_flash_borrow(
        &kamino,
        payer,
        params.borrow_amount,
    ));

    // ── Pool 1 swap ──
    let accounts_1 = params.leg_1.account_set(&payer);
    ixs.push(build_swap2_exact_in(
        &accounts_1,
        params.leg_1.amount_in,
        params.leg_1.min_amount_out,
    ));

    // ── Pool 2 swap ──
    let accounts_2 = params.leg_2.account_set(&payer);
    ixs.push(build_swap2_exact_in(
        &accounts_2,
        params.leg_2.amount_in,
        params.leg_2.min_amount_out,
    ));

    // ── Pool 3 swap ──
    let accounts_3 = params.leg_3.account_set(&payer);
    ixs.push(build_swap2_exact_in(
        &accounts_3,
        params.leg_3.amount_in,
        params.leg_3.min_amount_out,
    ));

    // ── Flash repay (borrow instruction index must point at the real borrow) ──
    let repay_idx = ixs.len();
    ixs.push(build_kamino_flash_repay_with_index(
        &kamino,
        payer,
        repay_amount,
        borrow_idx as u8,
    ));

    // ── Jito tip (last) ──
    let tip_ix = crate::build_marked_jito_tip_instruction(
        payer,
        params.jito_tip_account,
        crate::Lamports(params.jito_tip_lamports),
    );
    ixs.push(tip_ix);

    Ok((ixs, borrow_idx, repay_idx))
}

/// Assemble **and sign** the 3-leg flash-funded transaction as a v0 versioned transaction.
pub fn build_flash_funded_three_pool_transaction(
    params: &FlashArb3LegParams,
    signer: &KeypairSigner,
    lookup_tables: &[AddressLookupTable],
) -> Result<FlashThreeLegArbitrageTx, FlashArb3LegError> {
    let (ixs, borrow_idx, repay_idx) = {
        // Fail closed: the instructions are assembled for `params.payer` as the
        // fee payer and only signer. A mismatched `signer` would either add a
        // second required signature (V1) or sign a message the key cannot
        // verify (V0) — both unusable.
        if signer.pubkey() != params.payer {
            return Err(FlashArb3LegError::PayerSignerMismatch {
                payer: params.payer.to_string(),
                signer: signer.pubkey().to_string(),
            });
        }
        assemble_flash_funded_three_pool(params)?
    };

    // ── Atomicity self-check ──
    let verification = verify_atomic_integrity(
        ExecutionMode::SingleTransaction,
        &ixs,
        Some(borrow_idx),
        Some(repay_idx),
        params.borrow_amount,
        params.repay_amount(),
        params.flash_fee,
    )
    .map_err(|e| FlashArb3LegError::AltCompilation(e.to_string()))?;
    if !verification.is_atomic {
        return Err(FlashArb3LegError::AtomicityFailed(
            verification
                .rejection_reason
                .unwrap_or_else(|| "unknown".to_string()),
        ));
    }

    // Choose format: V0 (with ALT) or V1 (4096 bytes)
    let wire_bytes = if params.v1_transport_supported {
        // V1: native resource config in message, no ALT needed.
        // SIMD-0385: priority fee in the header is the TOTAL fee in
        // lamports, not micro-lamports per CU. Convert with ceil.
        let priority_fee_lamports = ((params.priority_fee_micro_lamports as u128)
            .saturating_mul(params.cu_limit as u128)
            .saturating_add(999_999))
            / 1_000_000;
        if priority_fee_lamports > u64::MAX as u128 {
            return Err(FlashArb3LegError::AltCompilation(
                "V1 priority fee exceeds u64 lamports".to_string(),
            ));
        }
        let config = crate::adaptive::V1ConfigParams {
            priority_fee_lamports: Some(priority_fee_lamports as u64),
            compute_unit_limit: Some(params.cu_limit),
            // 1 MiB: DLMM bin arrays are ~15-25 KB each; a 3-leg route with
            // 2-4 arrays per leg can load ~200-400 KB of account data.
            // Re-measure against loaded-accounts-data-size in the mainnet
            // simulation before enabling live.
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
                wire_size: 0, // fixed up with the measured size below
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
        // V0 with ALT
        let message = TxBuilder::compile_v0_message(
            signer.pubkey(),
            &ixs,
            params.recent_blockhash,
            lookup_tables,
        )
        .map_err(|e| FlashArb3LegError::AltCompilation(e.to_string()))?;
        let signature = signer.sign(&message);
        let mut wire = Vec::with_capacity(1 + 64 + message.len());
        wire.push(1); // compact-u16 signature count = 1
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

    let flash_mint = params.leg_1.token_in();
    let mint_a = params.leg_1.token_out();
    let mint_a_program = params.leg_1.token_out_program();
    let mint_b = params.leg_2.token_out();
    let mint_b_program = params.leg_2.token_out_program();
    let flash_program = params.leg_1.token_in_program();

    let settlement_account =
        get_associated_token_address_with_program_id(&params.payer, &flash_mint, &flash_program);
    let intermediate_account_a =
        get_associated_token_address_with_program_id(&params.payer, &mint_a, &mint_a_program);
    let intermediate_account_b =
        get_associated_token_address_with_program_id(&params.payer, &mint_b, &mint_b_program);

    let repay = params.repay_amount();
    Ok(FlashThreeLegArbitrageTx {
        wire_bytes,
        tx_size,
        instructions: ixs,
        borrow_instruction_index: borrow_idx,
        repay_instruction_index: repay_idx,
        borrow_amount: params.borrow_amount,
        flash_fee: params.flash_fee,
        repay_amount: repay,
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

/// Estimated serialized size of a v0 transaction carrying `instructions`.
pub fn estimate_v0_transaction_size_3leg(
    instructions: &[CompiledInstruction],
    lookup_tables: &[AddressLookupTable],
) -> usize {
    let mut in_table: std::collections::HashSet<Pubkey> = std::collections::HashSet::new();
    for table in lookup_tables {
        for addr in &table.addresses {
            in_table.insert(*addr);
        }
    }

    let mut account_map: BTreeMap<Pubkey, (bool, bool)> = BTreeMap::new();
    for ix in instructions {
        for acc in &ix.accounts {
            let entry = account_map.entry(acc.pubkey).or_insert((false, false));
            entry.0 |= acc.is_signer;
            entry.1 |= acc.is_writable;
        }
        account_map.entry(ix.program_id).or_insert((false, false));
    }

    let static_len = account_map
        .iter()
        .filter(|(pk, (is_signer, _))| !*is_signer && !in_table.contains(*pk))
        .count()
        + account_map
            .values()
            .filter(|(is_signer, _)| *is_signer)
            .count();

    let mut size = 1usize; // signature count
    size += 64; // one signature
    size += 4; // version prefix + 3 header bytes
    size += crate::dlmm_bin_discovery::compact_u16_len(static_len);
    size += 32 * static_len;
    size += 32; // recent blockhash
    size += crate::dlmm_bin_discovery::compact_u16_len(instructions.len());

    for ix in instructions {
        size += 1; // program index
        size += crate::dlmm_bin_discovery::compact_u16_len(ix.accounts.len());
        size += ix.accounts.len();
        size += crate::dlmm_bin_discovery::compact_u16_len(ix.data.len());
        size += ix.data.len();
    }

    // Address table lookups section.
    let mut used_tables: Vec<(usize, usize)> = Vec::new();
    for table in lookup_tables {
        let mut writable = 0usize;
        let mut readonly = 0usize;
        for addr in &table.addresses {
            let Some((is_signer, is_writable)) = account_map.get(addr) else {
                continue;
            };
            if *is_signer {
                continue;
            }
            if *is_writable {
                writable += 1;
            } else {
                readonly += 1;
            }
        }
        if writable > 0 || readonly > 0 {
            used_tables.push((writable, readonly));
        }
    }
    size += crate::dlmm_bin_discovery::compact_u16_len(used_tables.len());
    for (writable, readonly) in &used_tables {
        size += 32; // table address
        size += crate::dlmm_bin_discovery::compact_u16_len(*writable) + *writable;
        size += crate::dlmm_bin_discovery::compact_u16_len(*readonly) + *readonly;
    }

    size
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_types::TokenProgramKind;

    const WSOL: &str = "So11111111111111111111111111111111111111112";
    // Synthetic meme mints (valid base58, 32 bytes = 44 chars)
    const MEME_A: &str = "9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a9a";
    const MEME_B: &str = "9b9b9b9b9b9b9b9b9b9b9b9b9b9b9b9b9b9b9b9b9b9b";
    const POOL_1: &str = "5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6";
    const POOL_2: &str = "BGm1tav58oGcsQJehL9WXBFXF7D27vZsKefj4xJKD5Y";
    const POOL_3: &str = "7eKvG8nR2sVw9xZcL3pQfM1yT4hJ6bNuD5aBcE8fGhJk";

    fn wsol() -> Pubkey {
        Pubkey::from_str(WSOL).unwrap()
    }
    fn meme_a() -> Pubkey {
        Pubkey::from_str(MEME_A).unwrap()
    }
    fn meme_b() -> Pubkey {
        Pubkey::from_str(MEME_B).unwrap()
    }
    fn pool_1() -> Pubkey {
        Pubkey::from_str(POOL_1).unwrap()
    }
    fn pool_2() -> Pubkey {
        Pubkey::from_str(POOL_2).unwrap()
    }
    fn pool_3() -> Pubkey {
        Pubkey::from_str(POOL_3).unwrap()
    }

    fn make_leg(
        lb_pair: Pubkey,
        token_x: Pubkey,
        token_y: Pubkey,
        swap_for_y: bool,
    ) -> FlashArb3LegPoolLeg {
        FlashArb3LegPoolLeg {
            lb_pair,
            reserve_x: crate::derive_meteora_reserve_pda(&lb_pair, &token_x),
            reserve_y: crate::derive_meteora_reserve_pda(&lb_pair, &token_y),
            token_x_mint: token_x,
            token_y_mint: token_y,
            oracle: crate::derive_meteora_oracle_pda(&lb_pair),
            token_x_program: TokenProgramKind::SplToken.program_id(),
            token_y_program: TokenProgramKind::SplToken.program_id(),
            swap_for_y,
            amount_in: 1_000_000,
            min_amount_out: 1,
            bin_array_pdas: vec![crate::derive_meteora_bin_array_pda(&lb_pair, 0)],
            bitmap_extension: None,
            host_fee_in: None,
        }
    }

    fn params() -> FlashArb3LegParams {
        let payer = Pubkey::new_unique();
        FlashArb3LegParams {
            payer,
            kamino: KaminoFlashParams::default(),
            borrow_amount: 1_000_000,
            flash_fee: 625,
            // Leg 1: WSOL -> Meme A: on-chain (WSOL, Meme A), swap X->Y
            leg_1: make_leg(pool_1(), wsol(), meme_a(), true),
            // Leg 2: Meme A -> Meme B: on-chain (Meme A, Meme B), swap X->Y
            leg_2: make_leg(pool_2(), meme_a(), meme_b(), true),
            // Leg 3: Meme B -> WSOL: on-chain (WSOL, Meme B), swap Y->X (swap_for_y=false)
            leg_3: make_leg(pool_3(), wsol(), meme_b(), false),
            cu_limit: 800_000,
            priority_fee_micro_lamports: 10_000,
            jito_tip_account: Pubkey::new_unique(),
            jito_tip_lamports: 10_000,
            recent_blockhash: [42u8; 32],
            v1_transport_supported: false,
        }
    }

    #[test]
    fn non_distinct_pools_rejected() {
        let mut p = params();
        p.leg_3.lb_pair = p.leg_1.lb_pair;
        assert!(matches!(
            assemble_flash_funded_three_pool(&p),
            Err(FlashArb3LegError::PoolsNotDistinct)
        ));
    }

    #[test]
    fn zero_borrow_rejected() {
        let mut p = params();
        p.borrow_amount = 0;
        assert!(matches!(
            assemble_flash_funded_three_pool(&p),
            Err(FlashArb3LegError::ZeroBorrowAmount)
        ));
    }

    #[test]
    fn non_closing_route_rejected() {
        let mut p = params();
        // Leg 3 outputs Meme B instead of WSOL
        p.leg_3.token_x_mint = meme_b();
        p.leg_3.token_y_mint = meme_a();
        p.leg_3.swap_for_y = false;
        assert!(matches!(
            assemble_flash_funded_three_pool(&p),
            Err(FlashArb3LegError::NonClosingRoute { .. })
        ));
    }

    #[test]
    fn mint_chain_mismatch_rejected() {
        let mut p = params();
        // Leg 2 takes WSOL in while Leg 1 delivers Meme A out
        p.leg_2.token_x_mint = wsol();
        p.leg_2.token_y_mint = meme_a();
        p.leg_2.swap_for_y = true;
        assert!(matches!(
            assemble_flash_funded_three_pool(&p),
            Err(FlashArb3LegError::MintChainMismatch { .. })
        ));
    }

    #[test]
    fn leg_output_below_required_rejected() {
        let mut p = params();
        p.leg_1.min_amount_out = 1;
        assert!(matches!(
            assemble_flash_funded_three_pool(&p),
            Err(FlashArb3LegError::LegOutputBelowRequired { leg: 1, .. })
        ));
    }

    #[test]
    fn layout_is_borrow_three_swaps_repay_tip() {
        let mut p = params();
        p.leg_1.min_amount_out = 1_000_000;
        p.leg_2.amount_in = 1_000_000;
        p.leg_2.min_amount_out = 1_000_000;
        p.leg_3.amount_in = 1_000_000;
        p.leg_3.min_amount_out = p.repay_amount();
        let (ixs, borrow_idx, repay_idx) = assemble_flash_funded_three_pool(&p).unwrap();
        assert_eq!(ixs.len(), 11);
        assert_eq!(borrow_idx, 5);
        assert_eq!(repay_idx, 9);
        // Borrow carries the principal
        assert_eq!(&ixs[borrow_idx].data[8..16], &p.borrow_amount.to_le_bytes());
        // Repay carries principal + fee and points back at the borrow index
        let repay = p.repay_amount();
        assert_eq!(&ixs[repay_idx].data[8..16], &repay.to_le_bytes());
        assert_eq!(ixs[repay_idx].data[16], borrow_idx as u8);
        // Tip is last instruction
        assert_eq!(ixs.len() - 1, 10);
    }

    #[test]
    fn atomicity_verification_passes() {
        let mut p = params();
        p.leg_1.min_amount_out = 1_000_000;
        p.leg_2.amount_in = 1_000_000;
        p.leg_2.min_amount_out = 1_000_000;
        p.leg_3.amount_in = 1_000_000;
        p.leg_3.min_amount_out = p.repay_amount();
        let (ixs, borrow_idx, repay_idx) = assemble_flash_funded_three_pool(&p).unwrap();
        let v = verify_atomic_integrity(
            ExecutionMode::SingleTransaction,
            &ixs,
            Some(borrow_idx),
            Some(repay_idx),
            p.borrow_amount,
            p.repay_amount(),
            p.flash_fee,
        )
        .unwrap();
        assert!(v.is_atomic, "{:?}", v.rejection_reason);
    }

    #[test]
    fn reversed_mint_ordering_supported() {
        // Pool 1: on-chain (Meme A, WSOL) -> trading WSOL->Meme A is Y->X
        // Pool 2: on-chain (Meme A, Meme B) -> trading Meme A->Meme B is X->Y
        // Pool 3: on-chain (WSOL, Meme B) -> trading Meme B->WSOL is Y->X
        let mut p = params();
        p.leg_1 = make_leg(pool_1(), meme_a(), wsol(), false);
        p.leg_2 = make_leg(pool_2(), meme_a(), meme_b(), true);
        p.leg_3 = make_leg(pool_3(), wsol(), meme_b(), false);
        p.leg_1.min_amount_out = 1_000_000;
        p.leg_2.amount_in = 1_000_000;
        p.leg_2.min_amount_out = 1_000_000;
        p.leg_3.amount_in = 1_000_000;
        p.leg_3.min_amount_out = p.repay_amount();
        let (ixs, borrow_idx, repay_idx) = assemble_flash_funded_three_pool(&p).unwrap();
        assert_eq!(borrow_idx, 5);
        assert_eq!(repay_idx, 9);
        assert_eq!(ixs.len(), 11);
    }
}
