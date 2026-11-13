//! Cross-DEX Flash Loan Arbitrage
//!
//! This module provides support for flash loan arbitrage across multiple DEX platforms.
//! It enables arbitrage between pools on different DEXes (e.g., Raydium, Meteora, Orca)
//! using Kamino flash loans as the funding source.
//!
//! Supported scenarios:
//! - 2-pool arbitrage across 2 different DEX platforms
//! - 3-pool arbitrage across 2-3 different DEX platforms
//! - Mixed DEX routes with flash loan funding

use crate::{
    AddressLookupTable, CompiledInstruction, KaminoFlashParams, KeypairSigner, TxBuilder,
    build_compute_unit_limit, build_compute_unit_price, build_create_ata_idempotent_for_program,
    build_kamino_flash_borrow, build_kamino_flash_repay_with_index, build_swap_hop_instruction,
    get_associated_token_address_with_program_id,
};
use arb_constants::{
    METEORA_DLMM_PROGRAM_ID, ORCA_WHIRLPOOL_PROGRAM_ID, RAYDIUM_CLMM_PROGRAM_ID,
    RAYDIUM_CPMM_PROGRAM_ID, TOKEN_2022_PROGRAM_ID,
};
use arb_types::{DexProtocol, MintId, PoolExecutionAccounts, PoolId, SwapHop, TokenProgramKind};
use solana_pubkey::Pubkey;
use std::str::FromStr;

/// Supported DEX platforms for cross-DEX arbitrage
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DexPlatform {
    /// Meteora DLMM (Liquidity Book)
    MeteoraDlmm,
    /// Raydium CLMM (Concentrated Liquidity)
    RaydiumClmm,
    /// Raydium CPMM (Constant Product)
    RaydiumCpmm,
    /// Orca Whirlpool
    OrcaWhirlpool,
}

impl DexPlatform {
    /// Get the program ID for this DEX platform
    pub fn program_id(&self) -> Pubkey {
        match self {
            DexPlatform::MeteoraDlmm => Pubkey::from_str(METEORA_DLMM_PROGRAM_ID).unwrap(),
            DexPlatform::RaydiumClmm => Pubkey::from_str(RAYDIUM_CLMM_PROGRAM_ID).unwrap(),
            DexPlatform::RaydiumCpmm => Pubkey::from_str(RAYDIUM_CPMM_PROGRAM_ID).unwrap(),
            DexPlatform::OrcaWhirlpool => Pubkey::from_str(ORCA_WHIRLPOOL_PROGRAM_ID).unwrap(),
        }
    }

    /// Check if this platform supports the given pool
    pub fn supports_pool(&self, _pool_address: &Pubkey) -> bool {
        // In a real implementation, this would check against known pool addresses
        // For now, we assume all pools are supported
        true
    }

    /// Map onto the canonical protocol enum used by the swap-hop builder.
    #[inline]
    pub fn protocol(&self) -> DexProtocol {
        match self {
            DexPlatform::MeteoraDlmm => DexProtocol::MeteoraDlmm,
            DexPlatform::RaydiumClmm => DexProtocol::RaydiumClmm,
            DexPlatform::RaydiumCpmm => DexProtocol::RaydiumCpmm,
            DexPlatform::OrcaWhirlpool => DexProtocol::OrcaWhirlpool,
        }
    }
}

/// Map a token program `Pubkey` onto the typed program kind.
#[inline]
fn token_program_kind(program: &Pubkey) -> TokenProgramKind {
    if *program == Pubkey::from_str(TOKEN_2022_PROGRAM_ID).unwrap_or_default() {
        TokenProgramKind::Token2022
    } else {
        TokenProgramKind::SplToken
    }
}

/// A swap leg in a cross-DEX arbitrage route
#[derive(Debug, Clone)]
pub struct CrossDexSwapLeg {
    /// The DEX platform for this swap
    pub dex_platform: DexPlatform,
    /// The pool address (lb_pair for Meteora, pool_state for Raydium, etc.)
    pub pool_address: Pubkey,
    /// Input token mint
    pub token_in: Pubkey,
    /// Output token mint
    pub token_out: Pubkey,
    /// Token program for input token
    pub token_in_program: Pubkey,
    /// Token program for output token
    pub token_out_program: Pubkey,
    /// Exact input amount
    pub amount_in: u64,
    /// Minimum output amount (after slippage)
    pub min_amount_out: u64,
    /// Active bin id for Meteora DLMM legs.
    pub active_bin_id: Option<i32>,
    /// Verified on-chain execution accounts for this pool, resolved by the
    /// ingestion layer. Without them the canonical builder fails closed for
    /// every pool that is not the hardcoded SOL/USDC reference pool — that is
    /// deliberate: a leg built from guessed accounts is a losing transaction.
    ///
    /// Swap direction is *not* a leg field: it is derived from the pool's
    /// on-chain `token_x_mint` / `token_y_mint` inside the builder.
    pub execution_accounts: Option<PoolExecutionAccounts>,
}

impl CrossDexSwapLeg {
    /// Project this leg onto the canonical [`SwapHop`] consumed by
    /// [`build_swap_hop_instruction`]. Single source of truth: the cross-DEX
    /// assembler does not re-implement any protocol's account matrix.
    pub fn to_swap_hop(&self) -> SwapHop {
        SwapHop {
            pool_id: PoolId(self.pool_address),
            protocol: self.dex_platform.protocol(),
            mint_in: MintId(self.token_in),
            mint_out: MintId(self.token_out),
            fee_bps: 0,
            active_bin_id: self.active_bin_id,
            token_programs: [
                token_program_kind(&self.token_in_program),
                token_program_kind(&self.token_out_program),
            ],
            trade_intent: Default::default(),
            execution_accounts: self.execution_accounts.clone(),
        }
    }
}

/// Parameters for cross-DEX flash loan arbitrage
#[derive(Debug, Clone)]
pub struct CrossDexFlashArbParams {
    /// Transaction authority (payer)
    pub payer: Pubkey,
    /// Kamino flash loan parameters
    pub kamino: KaminoFlashParams,
    /// Flash principal amount
    pub borrow_amount: u64,
    /// Kamino flash fee
    pub flash_fee: u64,
    /// List of swap legs (2-4 legs)
    pub legs: Vec<CrossDexSwapLeg>,
    /// Compute unit limit
    pub cu_limit: u32,
    /// Priority fee in micro-lamports
    pub priority_fee_micro_lamports: u64,
    /// Jito tip account
    pub jito_tip_account: Pubkey,
    /// Jito tip in lamports
    pub jito_tip_lamports: u64,
    /// Recent blockhash
    pub recent_blockhash: [u8; 32],
    /// Whether V1 transport is supported
    pub v1_transport_supported: bool,
}

/// Errors for cross-DEX flash arbitrage
#[derive(Debug, thiserror::Error)]
pub enum CrossDexFlashArbError {
    #[error("invalid number of legs: {0}, expected 2-4")]
    InvalidNumberOfLegs(usize),
    #[error("all pools must be on different DEX platforms for cross-DEX arbitrage")]
    SameDexPlatform,
    #[error("zero borrow amount")]
    ZeroBorrowAmount,
    #[error("flash repayment {repay} does not cover borrow {borrow} + fee {fee}")]
    InsufficientRepayment { borrow: u64, repay: u64, fee: u64 },
    #[error("route does not close: expected {expected}, got {actual}")]
    NonClosingRoute { expected: String, actual: String },
    #[error("mint chain mismatch between leg {leg} and leg {next_leg}")]
    MintChainMismatch { leg: usize, next_leg: usize },
    #[error("leg {leg} minimum output {min_out} is below required {required}")]
    LegOutputBelowRequired {
        leg: usize,
        min_out: u64,
        required: u64,
    },
    #[error("transaction too large: {size} bytes, limit {limit}")]
    TransactionTooLarge { size: usize, limit: usize },
    #[error("V1 transaction requires explicit compute unit limit")]
    MissingComputeUnitLimit,
    #[error("V1 transaction requires explicit loaded accounts data size limit")]
    MissingLoadedAccountsDataLimit,
    #[error("unsupported DEX platform: {0}")]
    UnsupportedDexPlatform(String),
    #[error("leg {leg} could not be built: {reason}")]
    SwapBuildFailed { leg: usize, reason: String },
    #[error("atomicity check failed: {0}")]
    AtomicityFailed(String),
}

/// Result of cross-DEX flash arbitrage transaction assembly
#[derive(Debug, Clone)]
pub struct CrossDexFlashArbitrageTx {
    /// Serialized transaction bytes
    pub wire_bytes: Vec<u8>,
    /// Transaction size in bytes
    pub tx_size: usize,
    /// The assembled instructions
    pub instructions: Vec<CompiledInstruction>,
    /// Index of flash borrow instruction
    pub borrow_instruction_index: usize,
    /// Index of flash repay instruction
    pub repay_instruction_index: usize,
    /// Flash principal
    pub borrow_amount: u64,
    /// Flash fee
    pub flash_fee: u64,
    /// Repayment amount
    pub repay_amount: u64,
    /// Settlement account
    pub settlement_account: Pubkey,
    /// Intermediate accounts
    pub intermediate_accounts: Vec<Pubkey>,
    /// DEX platforms used
    pub dex_platforms: Vec<DexPlatform>,
    /// Pool addresses
    pub pool_addresses: Vec<Pubkey>,
}

impl CrossDexFlashArbParams {
    /// Calculate the repayment amount (principal + fee)
    #[inline]
    pub fn repay_amount(&self) -> u64 {
        self.borrow_amount.saturating_add(self.flash_fee)
    }

    /// Validate the cross-DEX arbitrage parameters
    pub fn validate(&self) -> Result<(), CrossDexFlashArbError> {
        // Check number of legs (2-4)
        if self.legs.len() < 2 || self.legs.len() > 4 {
            return Err(CrossDexFlashArbError::InvalidNumberOfLegs(self.legs.len()));
        }

        // Check non-zero borrow
        if self.borrow_amount == 0 {
            return Err(CrossDexFlashArbError::ZeroBorrowAmount);
        }

        // Check repayment coverage
        let repay = self.repay_amount();
        if repay < self.borrow_amount.saturating_add(self.flash_fee) {
            return Err(CrossDexFlashArbError::InsufficientRepayment {
                borrow: self.borrow_amount,
                repay,
                fee: self.flash_fee,
            });
        }

        // Check that we're actually doing cross-DEX (at least 2 different platforms)
        let platforms: std::collections::HashSet<_> =
            self.legs.iter().map(|leg| leg.dex_platform).collect();
        if platforms.len() < 2 {
            return Err(CrossDexFlashArbError::SameDexPlatform);
        }

        // Check closing route
        let first_mint = self.legs[0].token_in;
        let last_mint = self.legs.last().unwrap().token_out;
        if first_mint != last_mint {
            return Err(CrossDexFlashArbError::NonClosingRoute {
                expected: first_mint.to_string(),
                actual: last_mint.to_string(),
            });
        }

        // Check mint chaining
        for i in 0..(self.legs.len() - 1) {
            let current_leg = &self.legs[i];
            let next_leg = &self.legs[i + 1];

            if current_leg.token_out != next_leg.token_in
                || current_leg.token_out_program != next_leg.token_in_program
            {
                return Err(CrossDexFlashArbError::MintChainMismatch {
                    leg: i,
                    next_leg: i + 1,
                });
            }
        }

        // Check each leg's min output covers next leg's input
        for i in 0..(self.legs.len() - 1) {
            let current_leg = &self.legs[i];
            let next_leg = &self.legs[i + 1];

            if current_leg.min_amount_out < next_leg.amount_in {
                return Err(CrossDexFlashArbError::LegOutputBelowRequired {
                    leg: i,
                    min_out: current_leg.min_amount_out,
                    required: next_leg.amount_in,
                });
            }
        }

        // Check last leg covers repayment
        let last_leg = &self.legs[self.legs.len() - 1];
        if last_leg.min_amount_out < repay {
            return Err(CrossDexFlashArbError::LegOutputBelowRequired {
                leg: self.legs.len() - 1,
                min_out: last_leg.min_amount_out,
                required: repay,
            });
        }

        Ok(())
    }
}

/// Assemble cross-DEX flash arbitrage transaction
pub fn assemble_cross_dex_flash_arbitrage(
    params: &CrossDexFlashArbParams,
) -> Result<(Vec<CompiledInstruction>, usize, usize), CrossDexFlashArbError> {
    params.validate()?;

    let payer = params.payer;
    let flash_mint = params.legs[0].token_in;
    let flash_program = params.legs[0].token_in_program;

    // Create settlement account
    let settlement_account =
        get_associated_token_address_with_program_id(&payer, &flash_mint, &flash_program);

    // Create intermediate accounts for each leg's output
    let mut intermediate_accounts = Vec::with_capacity(params.legs.len() - 1);
    for i in 0..(params.legs.len() - 1) {
        let leg = &params.legs[i];
        let account = get_associated_token_address_with_program_id(
            &payer,
            &leg.token_out,
            &leg.token_out_program,
        );
        intermediate_accounts.push(account);
    }

    // Update Kamino to use settlement account
    let mut kamino = params.kamino.clone();
    kamino.user_destination_liquidity = settlement_account;

    // Build instructions
    let mut ixs: Vec<CompiledInstruction> = Vec::with_capacity(10 + params.legs.len() * 2);

    // V0 only: ComputeBudget instructions
    if !params.v1_transport_supported {
        ixs.push(build_compute_unit_limit(params.cu_limit));
        ixs.push(build_compute_unit_price(params.priority_fee_micro_lamports));
    }

    // Create settlement ATA
    ixs.push(build_create_ata_idempotent_for_program(
        payer,
        payer,
        flash_mint,
        settlement_account,
        flash_program,
    ));

    // Create intermediate ATAs
    for (i, &intermediate) in intermediate_accounts.iter().enumerate() {
        let leg = &params.legs[i];
        ixs.push(build_create_ata_idempotent_for_program(
            payer,
            payer,
            leg.token_out,
            intermediate,
            leg.token_out_program,
        ));
    }

    // Flash borrow
    let borrow_idx = ixs.len();
    ixs.push(build_kamino_flash_borrow(
        &kamino,
        payer,
        params.borrow_amount,
    ));

    // Build swap instructions for each leg through the canonical per-protocol
    // dispatcher. The previous implementation emitted an empty instruction per
    // leg (`accounts: vec![], data: vec![]`) and routed every leg's input to
    // `intermediate_accounts[0]`, so the transaction could never trade.
    for (leg_index, leg) in params.legs.iter().enumerate() {
        let swap_ix = build_cross_dex_swap(leg, leg_index, payer)?;
        ixs.push(swap_ix);
    }

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

    // Jito tip
    let tip_ix = crate::build_marked_jito_tip_instruction(
        payer,
        params.jito_tip_account,
        crate::Lamports(params.jito_tip_lamports),
    );
    ixs.push(tip_ix);

    Ok((ixs, borrow_idx, repay_idx))
}

/// Build the swap instruction for one cross-DEX leg.
///
/// Delegates to [`build_swap_hop_instruction`], the same dispatcher the native
/// (non-flash) route builder uses. There is no placeholder path: if the leg
/// cannot be turned into a real instruction — unknown pool, missing verified
/// execution accounts, a mint pair the pool does not hold — this fails closed
/// with the offending leg index instead of emitting an empty instruction.
fn build_cross_dex_swap(
    leg: &CrossDexSwapLeg,
    leg_index: usize,
    payer: Pubkey,
) -> Result<CompiledInstruction, CrossDexFlashArbError> {
    let hop = leg.to_swap_hop();
    let ix = build_swap_hop_instruction(&hop, payer, leg.amount_in, leg.min_amount_out).map_err(
        |e| CrossDexFlashArbError::SwapBuildFailed {
            leg: leg_index,
            reason: e.to_string(),
        },
    )?;
    if ix.accounts.is_empty() || ix.data.is_empty() {
        return Err(CrossDexFlashArbError::SwapBuildFailed {
            leg: leg_index,
            reason: "canonical swap builder produced an empty instruction".into(),
        });
    }
    Ok(ix)
}

/// Build cross-DEX flash arbitrage transaction (V1)
pub fn build_cross_dex_flash_arbitrage_v1(
    params: &CrossDexFlashArbParams,
    signer: &KeypairSigner,
) -> Result<CrossDexFlashArbitrageTx, CrossDexFlashArbError> {
    let (ixs, borrow_idx, repay_idx) = assemble_cross_dex_flash_arbitrage(params)?;

    // V1 requires explicit limits
    if params.cu_limit == 0 {
        return Err(CrossDexFlashArbError::MissingComputeUnitLimit);
    }

    // For V1, we need loaded accounts data size limit
    // This should be calculated based on the actual accounts used
    let loaded_accounts_data_size_limit = 256_000u64; // 256 KB default

    let v1_config = crate::adaptive::V1ConfigParams {
        priority_fee_lamports: Some(crate::primitives::fees::total_priority_fee_lamports(
            params.priority_fee_micro_lamports,
            params.cu_limit,
        )),
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
    .map_err(|e| CrossDexFlashArbError::AtomicityFailed(e.to_string()))?;

    // Check size
    let wire_bytes = compiled.wire_bytes.clone();
    if wire_bytes.len() > 4096 {
        return Err(CrossDexFlashArbError::TransactionTooLarge {
            size: wire_bytes.len(),
            limit: 4096,
        });
    }

    // Build intermediate accounts list
    let mut intermediate_accounts = Vec::with_capacity(params.legs.len() - 1);
    for i in 0..(params.legs.len() - 1) {
        let leg = &params.legs[i];
        let account = get_associated_token_address_with_program_id(
            &params.payer,
            &leg.token_out,
            &leg.token_out_program,
        );
        intermediate_accounts.push(account);
    }

    Ok(CrossDexFlashArbitrageTx {
        wire_bytes: wire_bytes.clone(),
        tx_size: wire_bytes.len(),
        instructions: ixs,
        borrow_instruction_index: borrow_idx,
        repay_instruction_index: repay_idx,
        borrow_amount: params.borrow_amount,
        flash_fee: params.flash_fee,
        repay_amount: params.repay_amount(),
        settlement_account: get_associated_token_address_with_program_id(
            &params.payer,
            &params.legs[0].token_in,
            &params.legs[0].token_in_program,
        ),
        intermediate_accounts,
        dex_platforms: params.legs.iter().map(|leg| leg.dex_platform).collect(),
        pool_addresses: params.legs.iter().map(|leg| leg.pool_address).collect(),
    })
}

/// Build cross-DEX flash arbitrage transaction (V0 with ALT)
pub fn build_cross_dex_flash_arbitrage_v0(
    params: &CrossDexFlashArbParams,
    signer: &KeypairSigner,
    lookup_tables: &[AddressLookupTable],
) -> Result<CrossDexFlashArbitrageTx, CrossDexFlashArbError> {
    let (ixs, borrow_idx, repay_idx) = assemble_cross_dex_flash_arbitrage(params)?;

    let wire_bytes = TxBuilder::compile_v0_message(
        signer.pubkey(),
        &ixs,
        params.recent_blockhash,
        lookup_tables,
    )
    .map_err(|e| CrossDexFlashArbError::AtomicityFailed(e.to_string()))?;

    // Check size
    if wire_bytes.len() > 1232 {
        return Err(CrossDexFlashArbError::TransactionTooLarge {
            size: wire_bytes.len(),
            limit: 1232,
        });
    }

    // Build intermediate accounts list
    let mut intermediate_accounts = Vec::with_capacity(params.legs.len() - 1);
    for i in 0..(params.legs.len() - 1) {
        let leg = &params.legs[i];
        let account = get_associated_token_address_with_program_id(
            &params.payer,
            &leg.token_out,
            &leg.token_out_program,
        );
        intermediate_accounts.push(account);
    }

    Ok(CrossDexFlashArbitrageTx {
        wire_bytes: wire_bytes.clone(),
        tx_size: wire_bytes.len(),
        instructions: ixs,
        borrow_instruction_index: borrow_idx,
        repay_instruction_index: repay_idx,
        borrow_amount: params.borrow_amount,
        flash_fee: params.flash_fee,
        repay_amount: params.repay_amount(),
        settlement_account: get_associated_token_address_with_program_id(
            &params.payer,
            &params.legs[0].token_in,
            &params.legs[0].token_in_program,
        ),
        intermediate_accounts,
        dex_platforms: params.legs.iter().map(|leg| leg.dex_platform).collect(),
        pool_addresses: params.legs.iter().map(|leg| leg.pool_address).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_constants::{TOKEN_PROGRAM_ID, WSOL_MINT};

    #[test]
    fn test_cross_dex_validation_2_legs() {
        let params = CrossDexFlashArbParams {
            payer: Pubkey::new_unique(),
            kamino: KaminoFlashParams::default_wsol(),
            borrow_amount: 1_000_000_000,
            flash_fee: 1_000,
            legs: vec![
                CrossDexSwapLeg {
                    dex_platform: DexPlatform::MeteoraDlmm,
                    pool_address: Pubkey::new_unique(),
                    token_in: Pubkey::from_str(WSOL_MINT).unwrap(),
                    token_out: Pubkey::new_unique(),
                    token_in_program: Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap(),
                    token_out_program: Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap(),
                    amount_in: 1_000_000_000,
                    min_amount_out: 900_000_000,
                    active_bin_id: None,
                    execution_accounts: None,
                },
                CrossDexSwapLeg {
                    dex_platform: DexPlatform::RaydiumClmm,
                    pool_address: Pubkey::new_unique(),
                    token_in: Pubkey::from_str(WSOL_MINT).unwrap(), // Should fail: doesn't match first leg's output
                    token_out: Pubkey::from_str(WSOL_MINT).unwrap(),
                    token_in_program: Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap(),
                    token_out_program: Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap(),
                    amount_in: 900_000_000,
                    min_amount_out: 1_001_000_000,
                    active_bin_id: None,
                    execution_accounts: None,
                },
            ],
            cu_limit: 300_000,
            priority_fee_micro_lamports: 1,
            jito_tip_account: Pubkey::new_unique(),
            jito_tip_lamports: 0,
            recent_blockhash: [0; 32],
            v1_transport_supported: true,
        };

        // This should fail because mint chain is broken
        let result = params.validate();
        assert!(matches!(
            result,
            Err(CrossDexFlashArbError::MintChainMismatch { .. })
        ));
    }

    #[test]
    fn test_cross_dex_validation_same_platform() {
        let params = CrossDexFlashArbParams {
            payer: Pubkey::new_unique(),
            kamino: KaminoFlashParams::default_wsol(),
            borrow_amount: 1_000_000_000,
            flash_fee: 1_000,
            legs: vec![
                CrossDexSwapLeg {
                    dex_platform: DexPlatform::MeteoraDlmm,
                    pool_address: Pubkey::new_unique(),
                    token_in: Pubkey::from_str(WSOL_MINT).unwrap(),
                    token_out: Pubkey::new_unique(),
                    token_in_program: Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap(),
                    token_out_program: Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap(),
                    amount_in: 1_000_000_000,
                    min_amount_out: 900_000_000,
                    active_bin_id: None,
                    execution_accounts: None,
                },
                CrossDexSwapLeg {
                    dex_platform: DexPlatform::MeteoraDlmm, // Same platform - should fail
                    pool_address: Pubkey::new_unique(),
                    token_in: Pubkey::from_str(WSOL_MINT).unwrap(),
                    token_out: Pubkey::from_str(WSOL_MINT).unwrap(),
                    token_in_program: Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap(),
                    token_out_program: Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap(),
                    amount_in: 900_000_000,
                    min_amount_out: 1_001_000_000,
                    active_bin_id: None,
                    execution_accounts: None,
                },
            ],
            cu_limit: 300_000,
            priority_fee_micro_lamports: 1,
            jito_tip_account: Pubkey::new_unique(),
            jito_tip_lamports: 0,
            recent_blockhash: [0; 32],
            v1_transport_supported: true,
        };

        let result = params.validate();
        assert!(matches!(
            result,
            Err(CrossDexFlashArbError::SameDexPlatform)
        ));
    }

    #[test]
    fn test_cross_dex_validation_valid() {
        let mint_a = Pubkey::new_unique();
        let _mint_b = Pubkey::new_unique();

        let params = CrossDexFlashArbParams {
            payer: Pubkey::new_unique(),
            kamino: KaminoFlashParams::default_wsol(),
            borrow_amount: 1_000_000_000,
            flash_fee: 1_000,
            legs: vec![
                CrossDexSwapLeg {
                    dex_platform: DexPlatform::MeteoraDlmm,
                    pool_address: Pubkey::new_unique(),
                    token_in: Pubkey::from_str(WSOL_MINT).unwrap(),
                    token_out: mint_a,
                    token_in_program: Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap(),
                    token_out_program: Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap(),
                    amount_in: 1_000_000_000,
                    min_amount_out: 900_000_000,
                    active_bin_id: None,
                    execution_accounts: None,
                },
                CrossDexSwapLeg {
                    dex_platform: DexPlatform::RaydiumClmm,
                    pool_address: Pubkey::new_unique(),
                    token_in: mint_a,
                    token_out: Pubkey::from_str(WSOL_MINT).unwrap(),
                    token_in_program: Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap(),
                    token_out_program: Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap(),
                    amount_in: 900_000_000,
                    min_amount_out: 1_001_000_000,
                    active_bin_id: None,
                    execution_accounts: None,
                },
            ],
            cu_limit: 300_000,
            priority_fee_micro_lamports: 1,
            jito_tip_account: Pubkey::new_unique(),
            jito_tip_lamports: 0,
            recent_blockhash: [0; 32],
            v1_transport_supported: true,
        };

        let result = params.validate();
        assert!(result.is_ok());
    }

    fn cpmm_accounts() -> PoolExecutionAccounts {
        PoolExecutionAccounts::RaydiumCpmm {
            amm_config: Pubkey::new_unique(),
            authority: Pubkey::new_unique(),
            vault_0: Pubkey::new_unique(),
            vault_1: Pubkey::new_unique(),
            observation_state: Pubkey::new_unique(),
        }
    }

    fn clmm_accounts() -> PoolExecutionAccounts {
        PoolExecutionAccounts::RaydiumClmm {
            amm_config: Pubkey::new_unique(),
            vault_0: Pubkey::new_unique(),
            vault_1: Pubkey::new_unique(),
            observation_state: Pubkey::new_unique(),
            tick_array_pdas: vec![
                Pubkey::new_unique(),
                Pubkey::new_unique(),
                Pubkey::new_unique(),
            ],
            bitmap_extension: None,
        }
    }

    /// The assembler must emit real, decodable swap instructions — never the
    /// `accounts: vec![], data: vec![]` placeholder it used to emit, and never
    /// route every leg's input to the first intermediate account.
    #[test]
    fn assembles_real_swap_instructions_per_leg() {
        let mint_a = Pubkey::new_unique();
        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let spl = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();

        let params = CrossDexFlashArbParams {
            payer: Pubkey::new_unique(),
            kamino: KaminoFlashParams::default_wsol(),
            borrow_amount: 1_000_000_000,
            flash_fee: 1_000,
            legs: vec![
                CrossDexSwapLeg {
                    dex_platform: DexPlatform::RaydiumCpmm,
                    pool_address: Pubkey::new_unique(),
                    token_in: wsol,
                    token_out: mint_a,
                    token_in_program: spl,
                    token_out_program: spl,
                    amount_in: 1_000_000_000,
                    min_amount_out: 900_000_000,
                    active_bin_id: None,
                    execution_accounts: Some(cpmm_accounts()),
                },
                CrossDexSwapLeg {
                    dex_platform: DexPlatform::RaydiumClmm,
                    pool_address: Pubkey::new_unique(),
                    token_in: mint_a,
                    token_out: wsol,
                    token_in_program: spl,
                    token_out_program: spl,
                    amount_in: 900_000_000,
                    min_amount_out: 1_001_000_000,
                    active_bin_id: None,
                    execution_accounts: Some(clmm_accounts()),
                },
            ],
            cu_limit: 400_000,
            priority_fee_micro_lamports: 50_000,
            jito_tip_account: Pubkey::new_unique(),
            jito_tip_lamports: 0,
            recent_blockhash: [7; 32],
            v1_transport_supported: false,
        };

        let (ixs, borrow_idx, repay_idx) = assemble_cross_dex_flash_arbitrage(&params).unwrap();
        assert!(borrow_idx < repay_idx);
        assert_eq!(repay_idx, borrow_idx + 1 + params.legs.len());

        // Skip the two ComputeBudget instructions and one ATA creation per token
        // (settlement + one intermediate), then the borrow.
        let swap_start = 2 + 1 + (params.legs.len() - 1) + 1;
        let swaps = &ixs[swap_start..swap_start + params.legs.len()];
        for (leg, ix) in params.legs.iter().zip(swaps.iter()) {
            assert!(
                !ix.accounts.is_empty() && !ix.data.is_empty(),
                "leg {:?} produced an empty instruction",
                leg.dex_platform
            );
            assert_eq!(ix.program_id, leg.dex_platform.program_id());
        }
        // Distinct legs must not both target the same pool/program pair by
        // accident: the CPMM leg's input account is the payer's WSOL ATA, the
        // CLMM leg's is the payer's `mint_a` ATA.
        assert_ne!(swaps[0].accounts, swaps[1].accounts);
    }

    /// Missing verified execution accounts on a non-reference pool is a hard
    /// error, not a silent fall back to the hardcoded SOL/USDC pool.
    #[test]
    fn fails_closed_without_execution_accounts() {
        let mint_a = Pubkey::new_unique();
        let spl = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
        let leg = CrossDexSwapLeg {
            dex_platform: DexPlatform::MeteoraDlmm,
            pool_address: Pubkey::new_unique(),
            token_in: Pubkey::from_str(WSOL_MINT).unwrap(),
            token_out: mint_a,
            token_in_program: spl,
            token_out_program: spl,
            amount_in: 1_000_000_000,
            min_amount_out: 900_000_000,
            active_bin_id: Some(0),
            execution_accounts: None,
        };

        let err = build_cross_dex_swap(&leg, 0, Pubkey::new_unique()).unwrap_err();
        assert!(
            matches!(err, CrossDexFlashArbError::SwapBuildFailed { leg: 0, .. }),
            "expected SwapBuildFailed, got {err:?}"
        );
    }
}
