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
/// lp_fee, compounding_fee, protocol_fee, fund_fee and referral_fee partition
/// total_fee and use QuoteResult::total_fee_mint. dynamic_fee is a diagnostic
/// subset of total_fee, not an extra charge. creator_fee is reported separately
/// and uses QuoteResult::creator_fee_mint: Raydium CPMM may collect it on the
/// output token even when the trade fee is charged on input. Never add raw
/// amounts across different mints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FeesBreakdown {
    /// LP claiming share, excluding the separately tracked compounding share.
    pub lp_fee: u64,
    /// Share added to the DAMM v2 compounded token B reserve.
    pub compounding_fee: u64,
    pub protocol_fee: u64,
    pub fund_fee: u64,
    pub creator_fee: u64,
    /// Referral allocation; a subset of the charged pool fee.
    pub referral_fee: u64,
    pub dynamic_fee: u64,
    pub total_fee: u64,
}

/// Standardized output quotation across all DEX protocols.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteResult {
    pub protocol: DexProtocol,
    pub pool_id: PoolId,
    /// Raw-unit denomination of fees_breakdown.total_fee.
    pub total_fee_mint: MintId,
    /// Raw-unit denomination of fees_breakdown.creator_fee.
    pub creator_fee_mint: MintId,
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
    /// Exact CLMM swap-step traversal count; None for non-CLMM protocols.
    pub clmm_steps_taken: Option<u32>,
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
            match (state.token_programs[token_index], fee_config) {
                (TokenProgramKind::Token2022, None) => {
                    return Err(ArbError::IncompleteState(format!(
                        "{side} Token-2022 mint has no verified current-epoch transfer-fee configuration"
                    )));
                }
                (TokenProgramKind::SplToken, Some(_)) => {
                    return Err(ArbError::InvalidInput(format!(
                        "{side} legacy SPL Token mint must not carry a Token-2022 transfer-fee configuration"
                    )));
                }
                _ => {}
            }
        }
        let family_matches = if state.protocol == DexProtocol::MeteoraDammV2 {
            // Missing protocol state must reach its specialized adapter so it
            // can report IncompleteState/UnsupportedOperation. Never route it
            // through a generic CPMM or CLMM fallback.
            state.damm_v2.as_ref().is_none_or(|damm| {
                state.kind
                    == if damm.is_compounding() {
                        arb_types::PoolKind::Cpmm
                    } else {
                        arb_types::PoolKind::Clmm
                    }
            })
        } else {
            state.kind == state.protocol.family()
        };
        if !family_matches {
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

    /// Apply the verified input-mint Token-2022 transfer fee once, before venue math.
    pub fn input_after_transfer_fee(&self) -> Result<(u64, u64)> {
        let fee = self
            .input_transfer_fee_config
            .map_or(0, |config| config.calculate_fee(self.amount));
        let net = self.amount.checked_sub(fee).ok_or(ArbError::Overflow)?;
        if net == 0 {
            return Err(ArbError::ZeroAmount);
        }
        Ok((net, fee))
    }

    /// Apply the verified output-mint Token-2022 transfer fee after venue math.
    pub fn output_after_transfer_fee(&self, gross_output: u64) -> Result<(u64, u64)> {
        let fee = self
            .output_transfer_fee_config
            .map_or(0, |config| config.calculate_fee(gross_output));
        let net = gross_output.checked_sub(fee).ok_or(ArbError::Overflow)?;
        Ok((net, fee))
    }

    /// Compute the common conservative min-out floor from the receiver's net output.
    pub fn minimum_amount_out(&self, net_output: u64) -> u64 {
        arb_constants::slippage_floor_bps(net_output, u64::from(self.slippage_bps))
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
