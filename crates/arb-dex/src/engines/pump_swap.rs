//! PumpSwap (Post-Graduation AMM) Independent Quote Engine.
//! Program ID: `BSfD6SHZigAfDWSzqZ5QNxUauWCdH9zU8CXmgzb1MmYZ`
//!
//! Validates:
//! - Post-migration AMM pools created upon Pump.fun graduation.
//! - Constant product swap on actual token reserves.
//! - Protocol fee + LP fee + Creator fee accounting.

use crate::quote_engine::{FeesBreakdown, QuoteEngine, QuoteRequest, QuoteResult};
use crate::token2022::HopTransferFeeBreakdown;
use arb_types::{ArbError, Bps, DexProtocol, PoolState, Result};
use std::time::Instant;

pub const PUMP_SWAP_VERSION: &str = "pumpswap-amm-v1.0";

#[derive(Debug, Default, Clone)]
pub struct PumpSwapQuoteEngine;

impl PumpSwapQuoteEngine {
    pub fn new() -> Self {
        Self
    }
}

impl QuoteEngine for PumpSwapQuoteEngine {
    fn protocol_id(&self) -> DexProtocol {
        DexProtocol::PumpSwap
    }

    fn quote_exact_in(&self, state: &PoolState, request: &QuoteRequest) -> Result<QuoteResult> {
        request.validate(state)?;
        let start_time = Instant::now();

        if !state.is_active {
            return Err(ArbError::PoolDisabled(format!(
                "PumpSwap pool {} is disabled",
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
            return Err(ArbError::InsufficientLiquidity(
                "PumpSwap pool has 0 reserves".into(),
            ));
        }

        // 1. Inbound Token-2022 transfer fee deduction
        let (input_net_into_pool, input_transfer_fee) = request.input_after_transfer_fee()?;

        if input_net_into_pool == 0 {
            return Err(ArbError::ZeroAmount);
        }

        // PumpSwap charges quote-token fees on input for buys, and on quote
        // output for sells. PoolState carries the exact aggregate PPM ratio.
        // +2 raw units conservatively cover up to three independently rounded
        // LP/protocol/creator fee components when only their sum is available.
        let fee_denominator = u128::from(state.fee_denominator);
        let fee_numerator = u128::from(state.fee_numerator);
        if fee_denominator == 0 || fee_numerator > fee_denominator {
            return Err(ArbError::InvalidInput(
                "PumpSwap fee ratio is missing or invalid".into(),
            ));
        }
        let fee_for_amount = |amount: u64| -> Result<u64> {
            if fee_numerator == 0 || amount == 0 {
                return Ok(0);
            }
            let numerator = u128::from(amount)
                .checked_mul(fee_numerator)
                .ok_or(ArbError::Overflow)?;
            let aggregate = numerator.div_ceil(fee_denominator);
            let conservative = aggregate.checked_add(2).ok_or(ArbError::Overflow)?;
            u64::try_from(conservative).map_err(|_| ArbError::Overflow)
        };
        let mut fee_u64 = 0u64;
        let curve_input = if a_to_b {
            // Sell: fee is charged after the curve on quote-token output.
            input_net_into_pool
        } else {
            // Buy: fee is deducted from quote-token input before the curve.
            fee_u64 = fee_for_amount(input_net_into_pool)?;
            input_net_into_pool
                .checked_sub(fee_u64)
                .filter(|amount| *amount > 0)
                .ok_or_else(|| {
                    ArbError::InsufficientLiquidity("Input exhausted by PumpSwap fee".into())
                })?
        };
        // 3. Constant product swap formula: dy = floor((y * dx') / (x + dx'))
        let dx_prime = curve_input as u128;
        let num_out = r_out.checked_mul(dx_prime).ok_or(ArbError::Overflow)?;
        let denom_out = r_in.checked_add(dx_prime).ok_or(ArbError::Overflow)?;
        let gross_out = u64::try_from(num_out / denom_out).map_err(|_| ArbError::Overflow)?;
        let spot_out = num_out.checked_div(r_in).ok_or(ArbError::Overflow)?;
        if spot_out == 0 {
            return Err(ArbError::InsufficientLiquidity(
                "PumpSwap spot output rounds to zero".into(),
            ));
        }
        let impact_numerator = spot_out.saturating_sub(u128::from(gross_out));
        let impact_bps = impact_numerator
            .checked_mul(10_000)
            .ok_or(ArbError::Overflow)?
            / spot_out;
        let price_impact = Bps(u16::try_from(impact_bps).map_err(|_| ArbError::Overflow)?);
        if gross_out == 0 {
            return Err(ArbError::InsufficientLiquidity(
                "PumpSwap returned 0 output".into(),
            ));
        }
        let pool_output_after_trade_fee = if a_to_b {
            fee_u64 = fee_for_amount(gross_out)?;
            gross_out
                .checked_sub(fee_u64)
                .filter(|amount| *amount > 0)
                .ok_or_else(|| {
                    ArbError::InsufficientLiquidity("PumpSwap sell fees consume output".into())
                })?
        } else {
            gross_out
        };

        // Outbound Token-2022 transfer fee applies after the PumpSwap trade fee.
        let (output_net_to_next_hop, output_transfer_fee) =
            request.output_after_transfer_fee(pool_output_after_trade_fee)?;
        // 5. Slippage calculation
        let min_amount_out = request.minimum_amount_out(output_net_to_next_hop);

        let elapsed = start_time.elapsed().as_micros() as u64;

        Ok(QuoteResult {
            protocol: DexProtocol::PumpSwap,
            pool_id: state.pool_id,
            total_fee_mint: request.input_mint,
            creator_fee_mint: request.input_mint,
            amount_in_raw: request.amount,
            amount_out_raw: output_net_to_next_hop,
            minimum_amount_out_raw: min_amount_out,
            // This generic PoolState contains only a conservative total fee ratio;
            // component allocation is not available here. Do not invent an 80/20 split.
            fees_breakdown: FeesBreakdown {
                lp_fee: 0,
                compounding_fee: 0,
                protocol_fee: 0,
                fund_fee: 0,
                creator_fee: 0,
                referral_fee: 0,
                dynamic_fee: 0,
                total_fee: fee_u64,
            },
            transfer_fee_breakdown: HopTransferFeeBreakdown {
                input_gross: request.amount,
                input_transfer_fee,
                input_net_into_pool,
                output_gross_from_pool: gross_out,
                output_curve_reserve_debit: gross_out,
                output_transfer_fee,
                output_net_to_next_hop,
            },
            price_impact,
            unfilled_input_raw: 0,
            required_accounts: vec![state.pool_id.0],
            state_slot: state.slot.0,
            state_hash: format!("{}_{}", state.slot.0, state.write_version),
            calculation_version: PUMP_SWAP_VERSION,
            quote_validity: true,
            computation_time_us: elapsed,
            dlmm_bins_crossed: None,
            clmm_steps_taken: None,
        })
    }

    fn quote_exact_out(&self, _state: &PoolState, _request: &QuoteRequest) -> Result<QuoteResult> {
        Err(ArbError::UnsupportedOperation(
            "PumpSwap quote_exact_out not yet supported".into(),
        ))
    }
}
