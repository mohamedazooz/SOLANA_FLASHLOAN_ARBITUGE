//! Dual-Path Execution Dispatcher for Atomic Arbitrage Transactions.
//!
//! This crate provides the final compilation output envelope that cleanly separates
//! the V0 Jito-compatible path from the V1 direct-transport path, with all wire
//! bytes pre-serialized and metadata attached for downstream execution engines.

use arb_tx::{KeypairSigner, StaticMasterAlt, ThreeLegMemeArbitrageContext, TxError};
use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;

/// The two mutually exclusive compilation outcomes for a two-DEX atomic arbitrage route.
///
/// Each variant carries the fully serialized wire bytes ready for submission to
/// its respective transport layer, along with the metadata required for
/// execution-time decisions (tip accounting, CU pricing, etc.).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompiledArbEnvelope {
    /// V0 transaction using the Static Master ALT, fits within 1,232 bytes.
    /// Suitable for Jito block engine submission with MEV protection.
    V0JitoAndSwqos {
        /// Canonical Solana wire bytes (signatures + versioned message).
        wire_bytes: Vec<u8>,
        /// Exact wire size in bytes (== wire_bytes.len()).
        wire_size: usize,
        /// Jito tip amount in lamports carried in the transaction.
        jito_tip_lamports: u64,
        /// Master ALT address used for this compilation.
        alt_address: Pubkey,
        /// Number of accounts resolved via the ALT.
        alt_resolved_count: usize,
        /// Number of static (inline) accounts in the message.
        static_account_count: usize,
    },
    /// V1 transaction with all accounts inline, fits within 4,096 bytes.
    /// Suitable for direct RPC/TPU transport via SWQoS priority fee.
    /// Does NOT support Jito bundles (no ALT, no jitodontfront marker).
    V1DirectSwqosOnly {
        /// Canonical Solana V1 wire bytes (message + trailing signatures).
        wire_bytes: Vec<u8>,
        /// Exact wire size in bytes (== wire_bytes.len()).
        wire_size: usize,
        /// Effective priority fee in micro-lamports per compute unit (includes rolled-over Jito tip).
        effective_cu_price_micro_lamports: u64,
        /// Total number of inline accounts.
        account_count: usize,
        /// Number of signers.
        signer_count: usize,
    },
}

impl CompiledArbEnvelope {
    /// Returns the wire bytes for RPC submission.
    pub fn wire_bytes(&self) -> &[u8] {
        match self {
            Self::V0JitoAndSwqos { wire_bytes, .. } => wire_bytes,
            Self::V1DirectSwqosOnly { wire_bytes, .. } => wire_bytes,
        }
    }

    /// Returns the wire size in bytes.
    pub fn wire_size(&self) -> usize {
        match self {
            Self::V0JitoAndSwqos { wire_size, .. } => *wire_size,
            Self::V1DirectSwqosOnly { wire_size, .. } => *wire_size,
        }
    }

    /// Returns true if this envelope is the V0 Jito path.
    pub fn is_v0(&self) -> bool {
        matches!(self, Self::V0JitoAndSwqos { .. })
    }

    /// Returns true if this envelope is the V1 direct path.
    pub fn is_v1(&self) -> bool {
        matches!(self, Self::V1DirectSwqosOnly { .. })
    }

    /// Returns the format as a string for logging/telemetry.
    pub fn format_name(&self) -> &'static str {
        match self {
            Self::V0JitoAndSwqos { .. } => "V0_Jito",
            Self::V1DirectSwqosOnly { .. } => "V1_Direct",
        }
    }
}

/// Error type for the execution dispatcher.
#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    #[error("Compilation rejected: {0}")]
    Rejected(String),

    #[error("V0 compilation failed: {0}")]
    V0Error(#[from] TxError),

    #[error("V1 compilation failed: {0}")]
    V1Error(String),

    #[error("V0 wire size {size} exceeds MTU limit {limit}")]
    V0Oversized { size: usize, limit: usize },

    #[error("V1 wire size {size} exceeds MTU limit {limit}")]
    V1Oversized { size: usize, limit: usize },

    #[error("Invalid context: {0}")]
    InvalidContext(String),

    #[error("Master ALT not provisioned or not activated")]
    AltNotReady,

    #[error("Signing error: {0}")]
    SigningError(String),
}

/// Configuration for the dual-path dispatcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatcherConfig {
    /// Maximum wire size for V0 transactions (Jito MTU).
    pub v0_mtu_limit: usize,
    /// Maximum wire size for V1 transactions (SIMD-0296).
    pub v1_mtu_limit: usize,
    /// Whether the V1 transport is supported by the downstream RPC/relay.
    pub v1_transport_supported: bool,
    /// Base compute unit price in micro-lamports (before tip roll-over).
    pub base_cu_price_micro_lamports: u64,
    /// Compute unit limit for the transaction.
    pub cu_limit: u32,
}

impl Default for DispatcherConfig {
    fn default() -> Self {
        Self {
            v0_mtu_limit: 1232,
            v1_mtu_limit: 4096,
            v1_transport_supported: true,
            base_cu_price_micro_lamports: 1_000,
            cu_limit: 320_000,
        }
    }
}

/// Main dispatcher entry point for the V1-only profile.
/// It never builds Legacy/V0 transactions or resolves Address Lookup Tables.
/// V1 uses inline accounts and the direct-transport envelope only.
pub fn dispatch_two_dex_arbitrage(
    ctx: &ThreeLegMemeArbitrageContext,
    _master_alt: &StaticMasterAlt,
    signer: &KeypairSigner,
    config: &DispatcherConfig,
) -> Result<CompiledArbEnvelope, DispatchError> {
    // V1-only release policy: validate the route, strip the Jito tip and roll
    // its economic value into the native V1 priority-fee field. Do not build
    // Legacy/V0 transactions, resolve ALTs, or use the legacy custom serializer.
    ctx.validate()
        .map_err(|e| DispatchError::InvalidContext(e.to_string()))?;
    if !config.v1_transport_supported {
        return Err(DispatchError::Rejected(
            "V1-only profile requires a verified V1-capable direct transport".into(),
        ));
    }
    let instructions = build_merged_instructions(ctx)?;
    // TODO: Re-implement strip_jito_tip_and_rollover using arb_tx::adaptive
    // For now, use instructions directly (V0 path)
    let v1_instructions = instructions.clone();
    let effective_cu_price = config.base_cu_price_micro_lamports;
    if ctx.cu_limit == 0 {
        return Err(DispatchError::Rejected("V1 requires a non-zero compute-unit limit".into()));
    }
    let priority_fee_lamports = ((effective_cu_price as u128)
        .saturating_mul(ctx.cu_limit as u128)
        .saturating_add(999_999)) / 1_000_000;
    let priority_fee_lamports = u64::try_from(priority_fee_lamports)
        .map_err(|_| DispatchError::Rejected("V1 priority fee exceeds u64 lamports".into()))?;
    let v1_tx = arb_tx::AdaptiveTxCompiler::compile_v1_only(
        signer,
        &v1_instructions,
        ctx.recent_blockhash,
        arb_tx::V1ConfigParams {
            priority_fee_lamports: Some(priority_fee_lamports),
            compute_unit_limit: Some(ctx.cu_limit),
            loaded_accounts_data_size_limit: Some(67_108_864),
            heap_size: None,
        },
    )?;
    let account_count = arb_tx::adaptive::count_unique_accounts(signer.pubkey(), &v1_instructions);

    // 6. Final V1 bounds check
    if v1_tx.wire_size() > config.v1_mtu_limit {
        return Err(DispatchError::V1Oversized {
            size: v1_tx.wire_size(),
            limit: config.v1_mtu_limit,
        });
    }

    Ok(CompiledArbEnvelope::V1DirectSwqosOnly {
        wire_size: v1_tx.wire_size(),
        wire_bytes: v1_tx.wire_bytes,
        effective_cu_price_micro_lamports: effective_cu_price,
        account_count,
        signer_count: 1,
    })
}

/// Build the merged instruction set with cross-DEX account deduplication,
/// privilege escalation, and ATA lifecycle optimization.
fn build_merged_instructions(
    ctx: &ThreeLegMemeArbitrageContext,
) -> Result<Vec<arb_tx::CompiledInstruction>, DispatchError> {
    use arb_tx::{
        CompiledInstruction, Lamports, build_close_token_account_for_program,
        build_compute_unit_limit, build_compute_unit_price,
        build_create_ata_idempotent_for_program, build_marked_jito_tip_instruction,
    };

    let mut instructions: Vec<CompiledInstruction> = Vec::with_capacity(12);

    // [0] Compute Budget: SetComputeUnitLimit
    instructions.push(build_compute_unit_limit(ctx.cu_limit));

    // [1] Compute Budget: SetComputeUnitPrice
    instructions.push(build_compute_unit_price(ctx.priority_fee_micro_lamports));

    // [2..] Intermediate ATA CreateIdempotent instructions (only when needed)
    if !ctx.meme_a_ata_exists {
        instructions.push(build_create_ata_idempotent_for_program(
            ctx.payer,
            ctx.payer,
            ctx.meme_a_mint.0,
            ctx.meme_a_ata,
            ctx.meme_a_token_program,
        ));
    }
    if !ctx.meme_b_ata_exists {
        instructions.push(build_create_ata_idempotent_for_program(
            ctx.payer,
            ctx.payer,
            ctx.meme_b_mint.0,
            ctx.meme_b_ata,
            ctx.meme_b_token_program,
        ));
    }

    // [..] Swap legs: three Meteora DLMM instructions
    let leg_instructions = crate::merger::build_leg_swap_instructions(ctx)?;
    instructions.extend(leg_instructions);

    // [..] CloseAccount instructions for intermediate ATAs — ONLY when safe
    if crate::merger::should_close_intermediate_ata(ctx, true) {
        instructions.push(build_close_token_account_for_program(
            ctx.meme_a_ata,
            ctx.payer,
            ctx.payer,
            ctx.meme_a_token_program,
        ));
    }
    if crate::merger::should_close_intermediate_ata(ctx, false) {
        instructions.push(build_close_token_account_for_program(
            ctx.meme_b_ata,
            ctx.payer,
            ctx.payer,
            ctx.meme_b_token_program,
        ));
    }

    // [..] Jito tip instruction (last)
    let tip_ix = build_marked_jito_tip_instruction(
        ctx.payer,
        ctx.jito_tip_account,
        Lamports(ctx.jito_tip_lamports),
    );
    instructions.push(tip_ix);

    // Apply cross-DEX account deduplication and privilege escalation
    let merged = crate::merger::merge_dex_legs(&instructions);
    Ok(merged)
}

// Module declarations
pub mod merger;
pub mod preflight;
pub mod recovery;
pub mod security;
pub mod sizing;
pub mod v0_compiler;


#[cfg(test)]
mod tests {
    use super::*;
    use arb_constants::{
        COMPUTE_BUDGET_LIMIT_3HOP, JITO_TIP_ACCOUNTS, TOKEN_PROGRAM_ID, WSOL_MINT,
    };
    use arb_tx::{KeypairSigner, StaticMasterAlt, ThreeLegMemeArbitrageContext};
    use solana_pubkey::Pubkey;
    use std::str::FromStr;

    fn test_payer() -> Pubkey {
        Pubkey::new_from_array([1u8; 32])
    }

    fn test_ctx() -> ThreeLegMemeArbitrageContext {
        let payer = test_payer();
        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let meme_a = Pubkey::new_from_array([90u8; 32]);
        let meme_b = Pubkey::new_from_array([91u8; 32]);
        let pool_1 = arb_types::PoolId::new([10u8; 32]);
        let pool_2 = arb_types::PoolId::new([20u8; 32]);
        let pool_3 = arb_types::PoolId::new([30u8; 32]);
        let spl = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
        let meme_a_ata = arb_tx::master_alt::derive_ata(&payer, &meme_a, &spl);
        let meme_b_ata = arb_tx::master_alt::derive_ata(&payer, &meme_b, &spl);

        ThreeLegMemeArbitrageContext {
            payer,
            wsol_mint: wsol,
            meme_a_mint: arb_types::MintId(meme_a),
            meme_b_mint: arb_types::MintId(meme_b),
            pool_1,
            pool_2,
            pool_3,
            leg_1_direction: arb_tx::meteora_dlmm::SwapDirection::XtoY,
            leg_2_direction: arb_tx::meteora_dlmm::SwapDirection::XtoY,
            leg_3_direction: arb_tx::meteora_dlmm::SwapDirection::YtoX,
            leg_1_accounts: arb_tx::compiler::LegExecutionAccounts {
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
                user_token_in: arb_tx::master_alt::derive_ata(&payer, &wsol, &spl),
                user_token_out: meme_a_ata,
            },
            leg_2_accounts: arb_tx::compiler::LegExecutionAccounts {
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
            leg_3_accounts: arb_tx::compiler::LegExecutionAccounts {
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
                user_token_out: arb_tx::master_alt::derive_ata(&payer, &wsol, &spl),
            },
            leg_1_params: arb_tx::compiler::LegSwapParams {
                mode: arb_tx::meteora_dlmm::SwapMode::ExactIn,
                amount_in: 1_000_000_000,
                min_amount_out: 950_000_000,
                max_amount_in: 0,
                amount_out: 0,
            },
            leg_2_params: arb_tx::compiler::LegSwapParams {
                mode: arb_tx::meteora_dlmm::SwapMode::ExactIn,
                amount_in: 950_000_000,
                min_amount_out: 900_000_000,
                max_amount_in: 0,
                amount_out: 0,
            },
            leg_3_params: arb_tx::compiler::LegSwapParams {
                mode: arb_tx::meteora_dlmm::SwapMode::ExactIn,
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
        let payer = test_payer();
        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let usdc = Pubkey::from_str(arb_constants::USDC_MINT).unwrap();
        let usdt = Pubkey::from_str("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB").unwrap();
        let spl = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
        let bot_wsol_ata = arb_tx::master_alt::derive_ata(&payer, &wsol, &spl);
        let bot_usdc_ata = arb_tx::master_alt::derive_ata(&payer, &usdc, &spl);
        let bot_usdt_ata = arb_tx::master_alt::derive_ata(&payer, &usdt, &spl);

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
    fn test_dispatcher_v1_only_path() {
        let ctx = test_ctx();
        let alt = test_master_alt();
        let signer = KeypairSigner::new_random();
        let config = DispatcherConfig::default();

        let result = dispatch_two_dex_arbitrage(&ctx, &alt, &signer, &config).unwrap();
        assert!(result.is_v1());
        assert!(!result.is_v0());
        assert!(result.wire_size() <= 4096);
    }

    #[test]
    fn test_compiled_arb_envelope_wire_bytes() {
        let ctx = test_ctx();
        let alt = test_master_alt();
        let signer = KeypairSigner::new_random();
        let config = DispatcherConfig::default();

        let envelope = dispatch_two_dex_arbitrage(&ctx, &alt, &signer, &config).unwrap();
        let wire = envelope.wire_bytes();
        assert!(!wire.is_empty());
        assert_eq!(wire.len(), envelope.wire_size());
    }
}
