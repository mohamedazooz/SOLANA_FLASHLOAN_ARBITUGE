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
        // Mapped to RaydiumCpmm / PumpFun family with AMM characteristics
        DexProtocol::RaydiumCpmm
    }

    fn quote_exact_in(&self, state: &PoolState, request: &QuoteRequest) -> Result<QuoteResult> {
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
        let (input_net_into_pool, input_transfer_fee) =
            if let Some(config) = request.input_transfer_fee_config {
                let fee = config.calculate_fee(request.amount);
                (request.amount.saturating_sub(fee), fee)
            } else {
                (request.amount, 0)
            };

        if input_net_into_pool == 0 {
            return Err(ArbError::ZeroAmount);
        }

        // 2. Fee calculation (default 30 bps = 0.3% total fee on PumpSwap AMM)
        let total_fee_bps = if state.fee_numerator > 0 && state.fee_denominator > 0 {
            (state.fee_numerator as u128 * 10_000) / state.fee_denominator as u128
        } else {
            30u128 // 0.3%
        };

        let num_fee = (input_net_into_pool as u128).saturating_mul(total_fee_bps);
        let fee = (num_fee + 10_000 - 1) / 10_000;
        let fee_u64 = u64::try_from(fee).unwrap_or(input_net_into_pool);

        let input_after_fee = input_net_into_pool.saturating_sub(fee_u64);
        if input_after_fee == 0 {
            return Err(ArbError::InsufficientLiquidity(
                "Input exhausted by swap fee".into(),
            ));
        }

        // 3. Constant product swap formula: dy = floor((y * dx') / (x + dx'))
        let dx_prime = input_after_fee as u128;
        let num_out = r_out.saturating_mul(dx_prime);
        let denom_out = r_in.saturating_add(dx_prime);
        let gross_out = (num_out / denom_out) as u64;

        if gross_out == 0 {
            return Err(ArbError::InsufficientLiquidity(
                "PumpSwap returned 0 output".into(),
            ));
        }

        // 4. Outbound Token-2022 transfer fee deduction
        let (output_net_to_next_hop, output_transfer_fee) =
            if let Some(config) = request.output_transfer_fee_config {
                let fee = config.calculate_fee(gross_out);
                (gross_out.saturating_sub(fee), fee)
            } else {
                (gross_out, 0)
            };

        // 5. Slippage calculation
        let min_amount_out = output_net_to_next_hop.saturating_sub(
            (output_net_to_next_hop as u128 * request.slippage_bps as u128 / 10_000) as u64,
        );

        let elapsed = start_time.elapsed().as_micros() as u64;

        Ok(QuoteResult {
            protocol: DexProtocol::RaydiumCpmm,
            pool_id: state.pool_id,
            amount_in_raw: request.amount,
            amount_out_raw: output_net_to_next_hop,
            minimum_amount_out_raw: min_amount_out,
            fees_breakdown: FeesBreakdown {
                lp_fee: (fee_u64 * 80) / 100,       // 80% to LPs
                protocol_fee: (fee_u64 * 20) / 100, // 20% to Protocol
                fund_fee: 0,
                creator_fee: 0,
                dynamic_fee: 0,
                total_fee: fee_u64,
            },
            transfer_fee_breakdown: HopTransferFeeBreakdown {
                input_gross: request.amount,
                input_transfer_fee,
                input_net_into_pool,
                output_gross_from_pool: gross_out,
                output_transfer_fee,
                output_net_to_next_hop,
            },
            price_impact: Bps(0),
            unfilled_input_raw: 0,
            required_accounts: vec![state.pool_id.0],
            state_slot: state.slot.0,
            state_hash: format!("{}_{}", state.slot.0, state.write_version),
            calculation_version: PUMP_SWAP_VERSION,
            quote_validity: true,
            computation_time_us: elapsed,
        })
    }

    fn quote_exact_out(&self, _state: &PoolState, _request: &QuoteRequest) -> Result<QuoteResult> {
        Err(ArbError::UnsupportedOperation(
            "PumpSwap quote_exact_out not yet supported".into(),
        ))
    }
}
