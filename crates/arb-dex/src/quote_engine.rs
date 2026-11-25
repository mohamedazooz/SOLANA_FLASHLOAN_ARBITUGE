//! Unified Rust Quote Engine Interface & Standardized Quote Result.
//!
//! Enforces:
//! - Strict independent mathematical implementation per protocol.
//! - Pure integer arithmetic and explicit directional rounding.
//! - Detailed fee breakdown and Token-2022 accounting.
//! - Required account resolution for transaction packing.

use crate::token2022::{HopTransferFeeBreakdown, TransferFeeConfig};
use arb_types::{ArbError, Bps, DexProtocol, MintId, PoolId, PoolState, Result, TokenProgramKind};
use solana_pubkey::Pubkey;

/// Inbound quote request parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteRequest {
    pub input_mint: MintId,
    pub output_mint: MintId,
    pub amount: u64,
    pub slippage_bps: u16,
    /// Verified fee schedule selected for the current Clock epoch. `None` means
    /// the schedule is unavailable/unverified, not that the mint has zero fees.
    pub input_transfer_fee_config: Option<TransferFeeConfig>,
    /// Verified fee schedule selected for the current Clock epoch. `None` means
    /// the schedule is unavailable/unverified, not that the mint has zero fees.
    pub output_transfer_fee_config: Option<TransferFeeConfig>,
}

/// Detailed protocol fee breakdown for transparency and accounting.
/// `total_fee` is authoritative. Some venues expose only an aggregate rate in
/// the quote state; their component fields remain zero rather than inventing
/// an LP/protocol/fund allocation that was not decoded from the live config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FeesBreakdown {
    pub lp_fee: u64,
    pub protocol_fee: u64,
    pub fund_fee: u64,
    pub creator_fee: u64,
    pub dynamic_fee: u64,
    pub total_fee: u64,
}

/// Standardized output quotation across all DEX protocols.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteResult {
    pub protocol: DexProtocol,
    pub pool_id: PoolId,
    pub amount_in_raw: u64,
    pub amount_out_raw: u64,
    pub minimum_amount_out_raw: u64,
    pub fees_breakdown: FeesBreakdown,
    pub transfer_fee_breakdown: HopTransferFeeBreakdown,
    pub price_impact: Bps,
    pub unfilled_input_raw: u64,
    pub required_accounts: Vec<Pubkey>,
    pub state_slot: u64,
    pub state_hash: String,
    pub calculation_version: &'static str,
    pub quote_validity: bool,
    pub computation_time_us: u64,
    /// Exact DLMM bin traversal count; `None` for non-DLMM protocols.
    pub dlmm_bins_crossed: Option<u32>,
}

impl QuoteRequest {
    /// Validate the common invariants shared by every protocol-specific quote engine.
    ///
    /// Amounts are raw units of input_mint; output is always the other mint in
    /// the pool. A request with a foreign output mint or more than 100% slippage
    /// must not reach protocol math, because it can otherwise produce a
    /// plausible-looking quote with no meaningful on-chain minimum-out protection.
    pub fn validate(&self, state: &PoolState) -> Result<()> {
        if !state.is_active {
            return Err(ArbError::PoolDisabled(format!(
                "pool {} is disabled",
                state.pool_id.0
            )));
        }
        if self.amount == 0 {
            return Err(ArbError::ZeroAmount);
        }
        if !arb_constants::is_valid_slippage_bps(u64::from(self.slippage_bps)) {
            return Err(ArbError::InvalidInput(format!(
                "slippage_bps {} exceeds 10000",
                self.slippage_bps
            )));
        }
        if self.input_mint == self.output_mint {
            return Err(ArbError::InvalidToken(
                "quote input and output mints must be different".into(),
            ));
        }
        let valid_direction = (self.input_mint == state.base_mint
            && self.output_mint == state.quote_mint)
            || (self.input_mint == state.quote_mint && self.output_mint == state.base_mint);
        if !valid_direction {
            return Err(ArbError::InvalidToken(format!(
                "quote direction {} -> {} does not match pool mints {} / {}",
                self.input_mint.0, self.output_mint.0, state.base_mint.0, state.quote_mint.0
            )));
        }
        if state.has_risky_token {
            return Err(ArbError::InvalidToken(
                "quote contains an unsupported or unverified token extension".into(),
            ));
        }
        // PoolState token_programs are ordered (base_mint, quote_mint). Resolve
        // fee schedules by the actual hop direction so every protocol engine
        // fails closed consistently when a Token-2022 schedule is missing.
        let input_index = usize::from(self.input_mint != state.base_mint);
        let output_index = 1 - input_index;
        for (side, token_index, fee_config) in [
            ("input", input_index, self.input_transfer_fee_config),
            ("output", output_index, self.output_transfer_fee_config),
        ] {
            if state.token_programs[token_index] == TokenProgramKind::Token2022
                && fee_config.is_none()
            {
                return Err(ArbError::IncompleteState(format!(
                    "{side} Token-2022 mint has no verified current-epoch transfer-fee configuration"
                )));
            }
        }
        if state.kind != state.protocol.family() {
            return Err(ArbError::InvalidInput(format!(
                "pool {} protocol {:?} conflicts with pricing family {:?}",
                state.pool_id.0, state.protocol, state.kind
            )));
        }
        for (name, config) in [
            ("input", self.input_transfer_fee_config),
            ("output", self.output_transfer_fee_config),
        ] {
            if let Some(config) = config {
                if config.transfer_fee_basis_points > 10_000 {
                    return Err(ArbError::InvalidInput(format!(
                        "{name} Token-2022 transfer fee {} bps exceeds 10000",
                        config.transfer_fee_basis_points
                    )));
                }
            }
        }
        Ok(())
    }
}
/// Authoritative Quote Engine trait implemented independently by each protocol.
pub trait QuoteEngine: Send + Sync {
    /// Associated DEX protocol identifier.
    fn protocol_id(&self) -> DexProtocol;

    /// Calculate exact output given exact input amount.
    fn quote_exact_in(&self, state: &PoolState, request: &QuoteRequest) -> Result<QuoteResult>;

    /// Calculate required input given exact target output amount.
    fn quote_exact_out(&self, state: &PoolState, request: &QuoteRequest) -> Result<QuoteResult>;
}
