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
                (request.amount.saturating_sub(fee), fee)
            } else {
                (request.amount, 0)
            };

        if input_net_into_pool == 0 {
            return Err(ArbError::ZeroAmount);
        }

        // 2. CPMM Fee calculation (PPM: Parts Per Million)
        // Fee numerator in state: e.g. 2500 (0.25% in 1M units)
        let trade_fee_rate = if state.fee_denominator == 1_000_000 {
            state.fee_numerator as u128
        } else if state.fee_denominator > 0 {
            (state.fee_numerator as u128 * 1_000_000) / state.fee_denominator as u128
        } else {
            2500 // 0.25% default Raydium CPMM trade fee
        };

        // On-chain: trade_fee = ceil(dx * trade_fee_rate / 1_000_000)
        let num_fee = (input_net_into_pool as u128).saturating_mul(trade_fee_rate);
        let trade_fee = num_fee.div_ceil(CPMM_FEE_RATE_DENOMINATOR);
        let trade_fee_u64 = u64::try_from(trade_fee).unwrap_or(input_net_into_pool);

        let input_after_fee = input_net_into_pool.saturating_sub(trade_fee_u64);
        if input_after_fee == 0 {
            return Err(ArbError::InsufficientLiquidity(
                "Input exhausted by CPMM swap fee".into(),
            ));
        }

        // 3. Constant product swap formula: dy = floor((y * dx') / (x + dx'))
        let dx_prime = input_after_fee as u128;
        let num_out = r_out.saturating_mul(dx_prime);
        let denom_out = r_in.saturating_add(dx_prime);
        let gross_out = u64::try_from(num_out / denom_out).map_err(|_| ArbError::Overflow)?;

        if gross_out == 0 {
            return Err(ArbError::InsufficientLiquidity(
                "Raydium CPMM returned 0 output tokens".into(),
            ));
        }

        // Sub-allocations (standard 12% protocol fee share on Raydium CPMM)
        let protocol_fee = (trade_fee * 12_000) / CPMM_PROTOCOL_FEE_DENOMINATOR;
        let fund_fee = 0u128;
        let lp_fee = trade_fee
            .saturating_sub(protocol_fee)
            .saturating_sub(fund_fee);

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

        // Price impact: impact = dx / (x + dx)
        let impact_bps = ((dx_prime * 10_000) / denom_out).min(10_000) as u16;

        let elapsed = start_time.elapsed().as_micros() as u64;

        Ok(QuoteResult {
            protocol: DexProtocol::RaydiumCpmm,
            pool_id: state.pool_id,
            amount_in_raw: request.amount,
            amount_out_raw: output_net_to_next_hop,
            minimum_amount_out_raw: min_amount_out,
            fees_breakdown: FeesBreakdown {
                lp_fee: lp_fee as u64,
                protocol_fee: protocol_fee as u64,
                fund_fee: fund_fee as u64,
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
        })
    }

    fn quote_exact_out(&self, state: &PoolState, request: &QuoteRequest) -> Result<QuoteResult> {
        // Explicitly validate the output mint against the pool's two mints; reject
        // unsupported mints instead of silently treating them as the base side.
        if request.output_mint != state.base_mint && request.output_mint != state.quote_mint {
            return Err(ArbError::InvalidInput("output mint not in pool".into()));
        }
        let a_to_b = request.output_mint == state.quote_mint;
        let (r_in, r_out) = if a_to_b {
            (state.reserve_in, state.reserve_out)
        } else {
            (state.reserve_out, state.reserve_in)
        };

        let target_dy = request.amount as u128;
        if target_dy >= r_out {
            return Err(ArbError::InsufficientLiquidity(
                "Exact out exceeds pool reserve".into(),
            ));
        }

        // dx' = ceil((x * dy) / (y - dy))
        let num = r_in.saturating_mul(target_dy);
        let denom = r_out.saturating_sub(target_dy);
        let dx_prime = num.div_ceil(denom);

        let trade_fee_rate = if state.fee_denominator == 1_000_000 {
            state.fee_numerator as u128
        } else {
            2500
        };

        // dx = ceil(dx' * 1_000_000 / (1_000_000 - trade_fee_rate))
        let denom_fee = CPMM_FEE_RATE_DENOMINATOR.saturating_sub(trade_fee_rate);
        let dx = dx_prime
            .saturating_mul(CPMM_FEE_RATE_DENOMINATOR)
            .div_ceil(denom_fee);
        let req_in = u64::try_from(dx)
            .map_err(|_| ArbError::CalculationError("Overflow in exact out".into()))?;

        // Fee portion in input terms = dx - dx' (the markup over the fee-free input).
        let total_fee = u64::try_from(dx.saturating_sub(dx_prime)).unwrap_or(u64::MAX);

        Ok(QuoteResult {
            protocol: DexProtocol::RaydiumCpmm,
            pool_id: state.pool_id,
            amount_in_raw: req_in,
            amount_out_raw: request.amount,
            minimum_amount_out_raw: request.amount,
            fees_breakdown: FeesBreakdown {
                lp_fee: total_fee,
                protocol_fee: 0,
                fund_fee: 0,
                creator_fee: 0,
                dynamic_fee: 0,
                total_fee,
            },
            transfer_fee_breakdown: HopTransferFeeBreakdown::default(),
            price_impact: Bps(0),
            unfilled_input_raw: 0,
            required_accounts: vec![state.pool_id.0],
            state_slot: state.slot.0,
            state_hash: format!("{}_{}", state.slot.0, state.write_version),
            calculation_version: RAYDIUM_CPMM_VERSION,
            quote_validity: true,
            computation_time_us: 1,
        })
    }
}
