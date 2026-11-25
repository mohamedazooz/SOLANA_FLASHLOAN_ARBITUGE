//! Raydium CPMM (CP-Swap) Independent Quote Engine.
//! Program ID: `CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C`
//!
//! Note: CPMM (Constant Product Market Maker) is NOT CBMM (Constant Bin Market Maker).
//! Raydium CP-Swap is an Anchor-based constant product program `x * y = k` with
//! fee denominator 1,000,000 (parts-per-million) and sub-allocations for
//! protocol fees and fund fees.

use crate::quote_engine::{FeesBreakdown, QuoteEngine, QuoteRequest, QuoteResult};
use crate::token2022::HopTransferFeeBreakdown;
use arb_types::{ArbError, Bps, DexProtocol, PoolState, Result};
use std::time::Instant;

pub const RAYDIUM_CPMM_VERSION: &str = "raydium-cpmm-v1.0";
pub const CPMM_FEE_RATE_DENOMINATOR: u128 = 1_000_000;
pub const CPMM_PROTOCOL_FEE_DENOMINATOR: u128 = 100_000;

#[derive(Debug, Default, Clone)]
pub struct RaydiumCpmmQuoteEngine;

impl RaydiumCpmmQuoteEngine {
    pub fn new() -> Self {
        Self
    }
}

impl QuoteEngine for RaydiumCpmmQuoteEngine {
    fn protocol_id(&self) -> DexProtocol {
        DexProtocol::RaydiumCpmm
    }

    fn quote_exact_in(&self, state: &PoolState, request: &QuoteRequest) -> Result<QuoteResult> {
        request.validate(state)?;
        if !state.is_fee_valid() {
            return Err(ArbError::InvalidInput(
                "Raydium CPMM fee snapshot is missing or invalid".into(),
            ));
        }
        let start_time = Instant::now();

        if !state.is_active {
            return Err(ArbError::PoolDisabled(format!(
                "Raydium CPMM pool {} is disabled",
                state.pool_id.0
            )));
        }

        let a_to_b = request.input_mint == state.base_mint;
        if !a_to_b && request.input_mint != state.quote_mint {
            return Err(ArbError::InvalidToken(format!(
                "Input mint {} does not match pool mints",
                request.input_mint.0
            )));
        }

        let (r_in, r_out) = if a_to_b {
            (state.reserve_in, state.reserve_out)
        } else {
            (state.reserve_out, state.reserve_in)
        };

        if r_in == 0 || r_out == 0 {
            return Err(ArbError::InsufficientLiquidity(format!(
                "Pool {} has zero reserves ({r_in}, {r_out})",
                state.pool_id.0
            )));
        }

        // 1. Inbound Token-2022 transfer fee deduction
        let (input_net_into_pool, input_transfer_fee) =
            if let Some(config) = request.input_transfer_fee_config {
                let fee = config.calculate_fee(request.amount);
                (
                    request.amount.checked_sub(fee).ok_or(ArbError::Overflow)?,
                    fee,
                )
            } else {
                (request.amount, 0)
            };

        if input_net_into_pool == 0 {
            return Err(ArbError::ZeroAmount);
        }

        // Shared checked CPMM exact-in calculation. Missing fee state is rejected above.
        let fee_num = u128::from(state.fee_numerator);
        let fee_den = u128::from(state.fee_denominator);
        let trade_fee_u64 =
            arb_math::cpmm::cpmm_fee_ceil(u128::from(input_net_into_pool), fee_num, fee_den)
                .ok_or_else(|| {
                    ArbError::InvalidInput("Raydium CPMM fee ratio is invalid".into())
                })?;
        let gross_out = arb_math::cpmm::cpmm_out_upfront_fee_ceil(
            u128::from(input_net_into_pool),
            r_in,
            r_out,
            fee_num,
            fee_den,
        )
        .ok_or(ArbError::Overflow)?;
        if gross_out == 0 {
            return Err(ArbError::InsufficientLiquidity(
                "Raydium CPMM returned 0 output tokens".into(),
            ));
        }

        // 4. Outbound Token-2022 transfer fee deduction
        let (output_net_to_next_hop, output_transfer_fee) =
            if let Some(config) = request.output_transfer_fee_config {
                let fee = config.calculate_fee(gross_out);
                (gross_out.checked_sub(fee).ok_or(ArbError::Overflow)?, fee)
            } else {
                (gross_out, 0)
            };

        // 5. Slippage calculation
        let min_amount_out = arb_constants::slippage_floor_bps(
            output_net_to_next_hop,
            u64::from(request.slippage_bps),
        );

        // Price impact: impact = dx / (x + dx)
        let impact_bps = arb_math::cpmm::cpmm_price_impact_bps(
            u128::from(
                input_net_into_pool
                    .checked_sub(trade_fee_u64)
                    .ok_or(ArbError::Overflow)?,
            ),
            r_in,
            r_out,
            0,
        )
        .0;

        let elapsed = start_time.elapsed().as_micros() as u64;

        Ok(QuoteResult {
            protocol: DexProtocol::RaydiumCpmm,
            pool_id: state.pool_id,
            amount_in_raw: request.amount,
            amount_out_raw: output_net_to_next_hop,
            minimum_amount_out_raw: min_amount_out,
            fees_breakdown: FeesBreakdown {
                lp_fee: 0,
                protocol_fee: 0,
                fund_fee: 0,
                creator_fee: 0,
                dynamic_fee: 0,
                total_fee: trade_fee_u64,
            },
            transfer_fee_breakdown: HopTransferFeeBreakdown {
                input_gross: request.amount,
                input_transfer_fee,
                input_net_into_pool,
                output_gross_from_pool: gross_out,
                output_transfer_fee,
                output_net_to_next_hop,
            },
            price_impact: Bps(impact_bps),
            unfilled_input_raw: 0,
            required_accounts: vec![state.pool_id.0],
            state_slot: state.slot.0,
            state_hash: format!("{}_{}", state.slot.0, state.write_version),
            calculation_version: RAYDIUM_CPMM_VERSION,
            quote_validity: true,
            computation_time_us: elapsed,
            dlmm_bins_crossed: None,
        })
    }

    fn quote_exact_out(&self, state: &PoolState, request: &QuoteRequest) -> Result<QuoteResult> {
        request.validate(state)?;
        if !state.is_fee_valid() {
            return Err(ArbError::InvalidInput(
                "Raydium CPMM fee snapshot is missing or invalid".into(),
            ));
        }
        if request.input_transfer_fee_config.is_some()
            || request.output_transfer_fee_config.is_some()
        {
            return Err(ArbError::UnsupportedOperation(
                "Raydium CPMM exact-out with Token-2022 transfer fees is not implemented".into(),
            ));
        }
        let a_to_b = request.output_mint == state.quote_mint;
        let (r_in, r_out) = if a_to_b {
            (state.reserve_in, state.reserve_out)
        } else {
            (state.reserve_out, state.reserve_in)
        };
        let fee_num = u128::from(state.fee_numerator);
        let fee_den = u128::from(state.fee_denominator);
        let req_in = arb_math::cpmm::cpmm_in_for_exact_out_upfront_fee_ceil(
            u128::from(request.amount),
            r_in,
            r_out,
            fee_num,
            fee_den,
        )
        .ok_or_else(|| {
            ArbError::InsufficientLiquidity(
                "Exact out exceeds pool reserve or has an invalid fee ratio".into(),
            )
        })?;
        let total_fee = arb_math::cpmm::cpmm_fee_ceil(u128::from(req_in), fee_num, fee_den)
            .ok_or(ArbError::Overflow)?;

        Ok(QuoteResult {
            protocol: DexProtocol::RaydiumCpmm,
            pool_id: state.pool_id,
            amount_in_raw: req_in,
            amount_out_raw: request.amount,
            minimum_amount_out_raw: request.amount,
            fees_breakdown: FeesBreakdown {
                lp_fee: 0,
                protocol_fee: 0,
                fund_fee: 0,
                creator_fee: 0,
                dynamic_fee: 0,
                total_fee,
            },
            transfer_fee_breakdown: HopTransferFeeBreakdown {
                input_gross: req_in,
                input_transfer_fee: 0,
                input_net_into_pool: req_in.checked_sub(total_fee).ok_or(ArbError::Overflow)?,
                output_gross_from_pool: request.amount,
                output_transfer_fee: 0,
                output_net_to_next_hop: request.amount,
            },
            price_impact: arb_math::cpmm::cpmm_price_impact_bps(
                u128::from(req_in.checked_sub(total_fee).ok_or(ArbError::Overflow)?),
                r_in,
                r_out,
                0,
            ),
            unfilled_input_raw: 0,
            required_accounts: vec![state.pool_id.0],
            state_slot: state.slot.0,
            state_hash: format!("{}_{}", state.slot.0, state.write_version),
            calculation_version: RAYDIUM_CPMM_VERSION,
            quote_validity: true,
            computation_time_us: 1,
            dlmm_bins_crossed: None,
        })
    }
}
