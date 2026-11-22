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
    AdaptiveTxCompiler, CompiledInstruction, KaminoFlashParams, TxBuilder, V1ConfigParams,
};
use arb_constants::{
    METEORA_DLMM_PROGRAM_ID, ORCA_WHIRLPOOL_PROGRAM_ID, RAYDIUM_CLMM_PROGRAM_ID,
    RAYDIUM_CPMM_PROGRAM_ID,
};
use arb_types::{DexProtocol, ExecutionOpportunity, Lamports, TokenAmount};
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
    /// Whether to swap for Y (true) or X (false) - relevant for Meteora DLMM
    pub swap_for_y: bool,
    /// Additional accounts required for this swap (bin arrays, vaults, etc.)
    pub additional_accounts: Vec<Pubkey>,
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
    #[error("atomicity check failed: {0}")]
    AtomicityFailed(String),
    #[error("tx error: {0}")]
    Tx(#[from] crate::TxError),
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

impl TryFrom<DexProtocol> for DexPlatform {
    type Error = CrossDexFlashArbError;

    fn try_from(p: DexProtocol) -> Result<Self, Self::Error> {
        match p {
            DexProtocol::MeteoraDlmm => Ok(DexPlatform::MeteoraDlmm),
            DexProtocol::RaydiumClmm => Ok(DexPlatform::RaydiumClmm),
            DexProtocol::RaydiumCpmm => Ok(DexPlatform::RaydiumCpmm),
            DexProtocol::OrcaWhirlpool => Ok(DexPlatform::OrcaWhirlpool),
            other => Err(CrossDexFlashArbError::UnsupportedDexPlatform(format!(
                "{other:?}"
            ))),
        }
    }
}

/// Checks whether an opportunity spans two or more distinct DEX platforms.
pub fn is_cross_dex_opportunity(opp: &ExecutionOpportunity) -> bool {
    if opp.hops.len() < 2 {
        return false;
    }
    let mut platforms = std::collections::HashSet::new();
    for hop in &opp.hops {
        if let Ok(plat) = DexPlatform::try_from(hop.protocol) {
            platforms.insert(plat);
        } else {
            return false;
        }
    }
    platforms.len() >= 2
}

/// Validates that an ExecutionOpportunity satisfies cross-DEX invariants.
pub fn validate_cross_dex_opportunity(
    opp: &ExecutionOpportunity,
) -> Result<Vec<DexPlatform>, CrossDexFlashArbError> {
    if opp.hops.len() < 2 || opp.hops.len() > 4 {
        return Err(CrossDexFlashArbError::InvalidNumberOfLegs(opp.hops.len()));
    }
    if opp.size_in.as_u64() == 0 {
        return Err(CrossDexFlashArbError::ZeroBorrowAmount);
    }
    let mut platforms = Vec::with_capacity(opp.hops.len());
    let mut unique_platforms = std::collections::HashSet::new();
    for hop in &opp.hops {
        let plat = DexPlatform::try_from(hop.protocol)?;
        platforms.push(plat);
        unique_platforms.insert(plat);
    }
    if unique_platforms.len() < 2 {
        return Err(CrossDexFlashArbError::SameDexPlatform);
    }
    // Route must close
    let first_mint = opp.hops.first().unwrap().mint_in;
    let last_mint = opp.hops.last().unwrap().mint_out;
    if first_mint != last_mint {
        return Err(CrossDexFlashArbError::NonClosingRoute {
            expected: first_mint.0.to_string(),
            actual: last_mint.0.to_string(),
        });
    }
    // Mint chaining
    for i in 0..(opp.hops.len() - 1) {
        let current_hop = &opp.hops[i];
        let next_hop = &opp.hops[i + 1];
        if current_hop.mint_out != next_hop.mint_in
            || current_hop.token_programs[1] != next_hop.token_programs[0]
        {
            return Err(CrossDexFlashArbError::MintChainMismatch {
                leg: i,
                next_leg: i + 1,
            });
        }
    }
    Ok(platforms)
}

/// Assembles a specialized cross-DEX flash-loan arbitrage execution instruction bundle.
///
/// Ensures all hop-specific execution accounts (e.g., DLMM bin arrays & bitmap extension,
/// CPMM vault_0/vault_1 & amm_config) are strictly preserved with zero leakage between hops.
/// Enforces on-chain atomic integrity: Kamino Borrow -> Hop 1 -> Hop 2 -> Kamino Repay -> Rent Reclaim -> Jito Tip.
pub fn assemble_cross_dex_opportunity(
    payer: Pubkey,
    tip_account: Pubkey,
    tip_amount: Lamports,
    cu_limit: u32,
    priority_fee_micro_lamports: u64,
    opportunity: &ExecutionOpportunity,
    kamino_params: &KaminoFlashParams,
    flash_loan_fee_sf: u64,
    min_profit_raw: TokenAmount,
    skip_close_intermediate_mints: &[Pubkey],
) -> crate::Result<Vec<CompiledInstruction>> {
    // 1. Validate cross-DEX invariants
    let platforms = validate_cross_dex_opportunity(opportunity)
        .map_err(|e| anyhow::anyhow!("Cross-DEX validation failed: {e}"))?;

    // 2. Delegate to the canonical assembly pipeline, ensuring all hop accounts and atomic invariants hold
    let ixs = TxBuilder::assemble_flash_arbitrage(
        payer,
        tip_account,
        tip_amount,
        cu_limit,
        priority_fee_micro_lamports,
        opportunity,
        kamino_params,
        flash_loan_fee_sf,
        min_profit_raw,
        skip_close_intermediate_mints,
    )?;

    tracing::info!(
        target: "arb_events",
        "CROSS_DEX_FLASH_ARB_ASSEMBLED: platforms={:?} hops={} payer={}",
        platforms,
        opportunity.hops.len(),
        payer
    );

    Ok(ixs)
}

/// Build a full cross-DEX flash arbitrage transaction from CrossDexFlashArbParams.
pub fn assemble_cross_dex_flash_arbitrage(
    params: &CrossDexFlashArbParams,
) -> Result<CrossDexFlashArbitrageTx, CrossDexFlashArbError> {
    params.validate()?;

    let mut ixs = Vec::new();
    let effective_cu = crate::calculate_dynamic_cu_limit(params.cu_limit);
    ixs.push(crate::build_compute_unit_limit(effective_cu));
    ixs.push(crate::build_compute_unit_price(
        params.priority_fee_micro_lamports,
    ));

    let wsol = Pubkey::from_str(arb_constants::WSOL_MINT).unwrap();
    let usdc = Pubkey::from_str(arb_constants::USDC_MINT).unwrap();
    let user_wsol_ata = crate::get_associated_token_address(&params.payer, &wsol);
    let mut resolved_params = params.kamino.clone();
    if resolved_params.user_destination_liquidity == Pubkey::default()
        || resolved_params.user_destination_liquidity == wsol
    {
        resolved_params.user_destination_liquidity = user_wsol_ata;
    }

    ixs.push(crate::build_create_ata_idempotent(
        params.payer,
        params.payer,
        wsol,
        user_wsol_ata,
    ));

    let mut intermediate_atas: Vec<(Pubkey, Pubkey, Pubkey)> = Vec::new();
    for leg in &params.legs {
        for (mint, program) in [
            (leg.token_in, leg.token_in_program),
            (leg.token_out, leg.token_out_program),
        ] {
            if mint != wsol && mint != usdc && !intermediate_atas.iter().any(|(m, _, _)| *m == mint)
            {
                let ata = crate::get_associated_token_address_with_program_id(
                    &params.payer,
                    &mint,
                    &program,
                );
                intermediate_atas.push((mint, ata, program));
            }
        }
    }

    for (mint, ata, program) in &intermediate_atas {
        ixs.push(crate::build_create_ata_idempotent_for_program(
            params.payer,
            params.payer,
            *mint,
            *ata,
            *program,
        ));
    }

    let borrow_idx = ixs.len();
    ixs.push(crate::build_kamino_flash_borrow(
        &resolved_params,
        params.payer,
        params.borrow_amount,
    ));

    for leg in &params.legs {
        let user_source = crate::get_associated_token_address_with_program_id(
            &params.payer,
            &leg.token_in,
            &leg.token_in_program,
        );
        let user_destination = crate::get_associated_token_address_with_program_id(
            &params.payer,
            &leg.token_out,
            &leg.token_out_program,
        );

        match leg.dex_platform {
            DexPlatform::MeteoraDlmm => {
                if leg.additional_accounts.len() < 3 {
                    return Err(CrossDexFlashArbError::AtomicityFailed(
                        "Meteora DLMM requires at least reserve_x, reserve_y, oracle in additional_accounts".into(),
                    ));
                }
                let reserve_x = leg.additional_accounts[0];
                let reserve_y = leg.additional_accounts[1];
                let oracle = leg.additional_accounts[2];
                let bin_array_pdas = leg.additional_accounts[3..].to_vec();

                let (token_x_mint, token_y_mint) = if leg.swap_for_y {
                    (leg.token_in, leg.token_out)
                } else {
                    (leg.token_out, leg.token_in)
                };
                let (token_x_prog, token_y_prog) = if leg.swap_for_y {
                    (leg.token_in_program, leg.token_out_program)
                } else {
                    (leg.token_out_program, leg.token_in_program)
                };

                let dlmm_accounts = crate::MeteoraDlmmAccounts {
                    lb_pair: leg.pool_address,
                    reserve_x,
                    reserve_y,
                    token_x_mint,
                    token_y_mint,
                    oracle,
                    event_authority: crate::derive_meteora_event_authority(),
                    program_id: leg.dex_platform.program_id(),
                };
                ixs.push(crate::build_meteora_dlmm_swap_instruction_with_bin_arrays(
                    &dlmm_accounts,
                    params.payer,
                    user_source,
                    user_destination,
                    token_x_prog,
                    token_y_prog,
                    leg.amount_in,
                    leg.min_amount_out,
                    &bin_array_pdas,
                    None,
                ));
            }
            DexPlatform::RaydiumCpmm => {
                if leg.additional_accounts.len() < 3 {
                    return Err(CrossDexFlashArbError::AtomicityFailed(
                        "Raydium CPMM requires amm_config, vault_0, vault_1 in additional_accounts"
                            .into(),
                    ));
                }
                let amm_config = leg.additional_accounts[0];
                let vault_0 = leg.additional_accounts[1];
                let vault_1 = leg.additional_accounts[2];
                let a_to_b = leg.token_in.to_bytes() < leg.token_out.to_bytes();
                let (input_vault, output_vault) = if a_to_b {
                    (vault_0, vault_1)
                } else {
                    (vault_1, vault_0)
                };
                let mut accs = crate::RaydiumCpmmAccounts::for_pool(
                    leg.pool_address,
                    params.payer,
                    user_source,
                    user_destination,
                    leg.token_in,
                    leg.token_out,
                    input_vault,
                    output_vault,
                    amm_config,
                );
                accs.input_token_program = leg.token_in_program;
                accs.output_token_program = leg.token_out_program;
                ixs.push(crate::build_raydium_cpmm_swap_instruction(
                    &accs,
                    leg.amount_in,
                    leg.min_amount_out,
                )?);
            }
            other => {
                return Err(CrossDexFlashArbError::UnsupportedDexPlatform(format!(
                    "{other:?}"
                )));
            }
        }
    }

    // Standalone on-chain profit guard assertion before FlashRepay
    let min_balance_after = params.repay_amount();
    ixs.push(crate::build_profit_guard_instruction(
        user_wsol_ata,
        min_balance_after,
    ));

    let repay_idx = ixs.len();
    ixs.push(crate::build_kamino_flash_repay_with_index(
        &resolved_params,
        params.payer,
        params.borrow_amount,
        borrow_idx as u8,
    ));

    for (_mint, ata, program) in &intermediate_atas {
        ixs.push(crate::build_close_token_account_for_program(
            *ata,
            params.payer,
            params.payer,
            *program,
        ));
    }

    if params.jito_tip_lamports > 0 {
        ixs.push(crate::build_marked_jito_tip_instruction(
            params.payer,
            params.jito_tip_account,
            Lamports(params.jito_tip_lamports),
        ));
    }

    let verification = crate::atomic_integrity::verify_atomic_integrity(
        crate::atomic_integrity::ExecutionMode::SingleTransaction,
        &ixs,
        Some(borrow_idx),
        Some(repay_idx),
        params.borrow_amount,
        params.repay_amount(),
        params.flash_fee,
    )?;
    if !verification.is_atomic {
        return Err(CrossDexFlashArbError::AtomicityFailed(
            verification
                .rejection_reason
                .unwrap_or_else(|| "atomic integrity check failed".into()),
        ));
    }

    let priority_fee_lamports =
        crate::priority_fee_lamports(params.priority_fee_micro_lamports, effective_cu);
    let v1_config = V1ConfigParams {
        priority_fee_lamports: Some(priority_fee_lamports),
        compute_unit_limit: Some(effective_cu),
        loaded_accounts_data_size_limit: Some(67_108_864),
        heap_size: None,
    };
    let compiled = AdaptiveTxCompiler::build_unsigned_v1_simulation_transaction(
        params.payer,
        &ixs,
        params.recent_blockhash,
        v1_config,
    )?;

    let wire_bytes = compiled;
    let tx_size = wire_bytes.len();
    let dex_platforms = params.legs.iter().map(|l| l.dex_platform).collect();
    let pool_addresses = params.legs.iter().map(|l| l.pool_address).collect();

    Ok(CrossDexFlashArbitrageTx {
        wire_bytes,
        tx_size,
        instructions: ixs,
        borrow_instruction_index: borrow_idx,
        repay_instruction_index: repay_idx,
        borrow_amount: params.borrow_amount,
        flash_fee: params.flash_fee,
        repay_amount: params.repay_amount(),
        settlement_account: user_wsol_ata,
        intermediate_accounts: intermediate_atas
            .into_iter()
            .map(|(_, ata, _)| ata)
            .collect(),
        dex_platforms,
        pool_addresses,
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
                    swap_for_y: true,
                    additional_accounts: vec![],
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
                    swap_for_y: true,
                    additional_accounts: vec![],
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
                    swap_for_y: true,
                    additional_accounts: vec![],
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
                    swap_for_y: true,
                    additional_accounts: vec![],
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
                    swap_for_y: true,
                    additional_accounts: vec![],
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
                    swap_for_y: true,
                    additional_accounts: vec![],
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
}
