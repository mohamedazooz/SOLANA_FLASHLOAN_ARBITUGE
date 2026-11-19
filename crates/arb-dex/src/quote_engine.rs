//! Unified Rust Quote Engine Interface & Standardized Quote Result.
//!
//! Enforces:
//! - Strict independent mathematical implementation per protocol.
//! - Pure integer arithmetic and explicit directional rounding.
//! - Detailed fee breakdown and Token-2022 accounting.
//! - Required account resolution for transaction packing.

use crate::token2022::{HopTransferFeeBreakdown, TransferFeeConfig};
use arb_types::{Bps, DexProtocol, MintId, PoolId, PoolState, Result};
use solana_pubkey::Pubkey;

/// Inbound quote request parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteRequest {
    pub input_mint: MintId,
    pub output_mint: MintId,
    pub amount: u64,
    pub slippage_bps: u16,
    pub input_transfer_fee_config: Option<TransferFeeConfig>,
    pub output_transfer_fee_config: Option<TransferFeeConfig>,
}

/// Detailed protocol fee breakdown for transparency and accounting.
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
