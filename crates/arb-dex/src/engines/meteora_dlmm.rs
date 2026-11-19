//! Meteora DLMM Independent Quote Engine.
//! Program ID: `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`
//!
//! Enforces:
//! - Exact discrete bin price $P(i) = (1 + s)^i$ in Q64.64 fixed-point arithmetic.
//! - Non-continuous bin walking across discrete bins.
//! - Directional liquidity consumption: draining Token Y when moving right, Token X when moving left.
//! - Base fee + Volatility accumulator surge dynamic fee modeling.
//! - Protocol fee capture and Token-2022 transfer fee deduction.
//! - Derivation of required BinArray PDAs for the swap instruction.

use crate::quote_engine::{FeesBreakdown, QuoteEngine, QuoteRequest, QuoteResult};
use crate::token2022::HopTransferFeeBreakdown;
use arb_constants::{METEORA_BINS_PER_BIN_ARRAY, METEORA_DLMM_PROGRAM_ID};
use arb_math::dlmm::{
    DlmmBin, DlmmParameters, DlmmPoolState, DlmmVariableParameters, quote_dlmm_view,
};
use arb_types::{ArbError, DexProtocol, PoolState, Result};
use solana_pubkey::Pubkey;
use std::str::FromStr;
use std::time::Instant;

pub const METEORA_DLMM_VERSION: &str = "meteora-dlmm-v1.4";

#[derive(Debug, Default, Clone)]
pub struct MeteoraDlmmQuoteEngine;

impl MeteoraDlmmQuoteEngine {
    pub fn new() -> Self {
        Self
    }

    /// Derive BinArray PDA for Meteora DLMM:
    /// seeds = [b"bin_array", lb_pair.as_ref(), &bin_array_index.to_le_bytes()]
    pub fn derive_bin_array_pda(lb_pair: &Pubkey, bin_array_index: i64) -> (Pubkey, u8) {
        let program_id = Pubkey::from_str(METEORA_DLMM_PROGRAM_ID).unwrap();
        Pubkey::find_program_address(
            &[
                b"bin_array",
                lb_pair.as_ref(),
                &bin_array_index.to_le_bytes(),
            ],
            &program_id,
        )
    }

    /// Compute bin array index for a given bin ID.
    pub fn bin_id_to_bin_array_index(bin_id: i32) -> i64 {
        let bins_per_array = METEORA_BINS_PER_BIN_ARRAY as i32;
        bin_id.div_euclid(bins_per_array) as i64
    }
}

impl QuoteEngine for MeteoraDlmmQuoteEngine {
    fn protocol_id(&self) -> DexProtocol {
        DexProtocol::MeteoraDlmm
    }

    fn quote_exact_in(&self, state: &PoolState, request: &QuoteRequest) -> Result<QuoteResult> {
        let start_time = Instant::now();

        if !state.is_active {
            return Err(ArbError::PoolDisabled(format!(
                "Meteora DLMM pool {} is disabled",
                state.pool_id.0
            )));
        }

        let dlmm_state = state.dlmm.as_ref().ok_or_else(|| {
            ArbError::IncompleteState(format!("Missing DLMM state for pool {}", state.pool_id.0))
        })?;

        let a_to_b = request.input_mint == state.base_mint;
        if !a_to_b && request.input_mint != state.quote_mint {
            return Err(ArbError::InvalidToken(format!(
                "Input mint {} does not match pool mints",
                request.input_mint.0
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

        // 2. Prepare DLMM parameters
        let params = if let Some(p) = &dlmm_state.params {
            DlmmParameters {
                bin_step: dlmm_state.bin_step_bps,
                base_factor: p.base_factor,
                filter_period: p.filter_period,
                decay_period: p.decay_period,
                reduction_factor: p.reduction_factor,
                variable_fee_control: p.variable_fee_control,
                max_volatility_accumulator: p.max_volatility_accumulator,
                protocol_share: p.protocol_share,
                base_fee_power_factor: p.base_fee_power_factor,
                collect_fee_mode: 0,
            }
        } else {
            DlmmParameters {
                bin_step: dlmm_state.bin_step_bps,
                base_factor: 100,
                filter_period: 30,
                decay_period: 600,
                reduction_factor: 5000,
                variable_fee_control: 40_000,
                max_volatility_accumulator: 350_000,
                protocol_share: 1000, // 10%
                base_fee_power_factor: 0,
                collect_fee_mode: 0,
            }
        };

        let v_params = DlmmVariableParameters {
            volatility_accumulator: dlmm_state.volatility_accumulator,
            volatility_reference: 0,
            index_reference: dlmm_state.active_bin_id,
            last_update_timestamp: 0,
        };

        let pool_math_state = DlmmPoolState {
            active_id: dlmm_state.active_bin_id,
            bin_step: dlmm_state.bin_step_bps,
            status: 0,
            parameters: params,
            v_parameters: v_params,
            support_limit_order: false,
        };

        let bins: Vec<DlmmBin> = dlmm_state
            .bins
            .iter()
            .map(|b| DlmmBin {
                id: b.bin_id,
                amount_x: b.amount_x as u64,
                amount_y: b.amount_y as u64,
                price_q64: 0,
                limit_order_ask_side: false,
                open_order_amount: 0,
                processed_order_remaining_amount: 0,
            })
            .collect();

        if bins.is_empty() {
            return Err(ArbError::IncompleteState(format!(
                "No liquidity bins cached for Meteora DLMM pool {}",
                state.pool_id.0
            )));
        }

        // 3. Exact multi-bin discrete quoting
        let quote = quote_dlmm_view(&pool_math_state, &bins, input_net_into_pool, a_to_b, None)
            .map_err(|e| ArbError::CalculationError(format!("Meteora DLMM math error: {e:?}")))?;

        let gross_out = quote.amount_out;
        if gross_out == 0 {
            return Err(ArbError::InsufficientLiquidity(
                "Meteora DLMM returned 0 output tokens".into(),
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

        // 6. Required Accounts resolution (including BinArray PDAs)
        let lb_pair = state.pool_id.0;
        let start_bin_idx = Self::bin_id_to_bin_array_index(dlmm_state.active_bin_id);
        let end_bin_idx = Self::bin_id_to_bin_array_index(quote.end_active_id);
        let mut required_accounts = Vec::with_capacity(8);
        required_accounts.push(lb_pair);

        // Include current, start, and end bin array PDAs
        let (ta_curr, _) = Self::derive_bin_array_pda(&lb_pair, start_bin_idx);
        required_accounts.push(ta_curr);
        if start_bin_idx != end_bin_idx {
            let (ta_end, _) = Self::derive_bin_array_pda(&lb_pair, end_bin_idx);
            required_accounts.push(ta_end);
        }

        let elapsed = start_time.elapsed().as_micros() as u64;

        Ok(QuoteResult {
            protocol: DexProtocol::MeteoraDlmm,
            pool_id: state.pool_id,
            amount_in_raw: request.amount,
            amount_out_raw: output_net_to_next_hop,
            minimum_amount_out_raw: min_amount_out,
            fees_breakdown: FeesBreakdown {
                lp_fee: quote.fee_amount.saturating_sub(quote.protocol_fee),
                protocol_fee: quote.protocol_fee,
                fund_fee: 0,
                creator_fee: 0,
                dynamic_fee: quote.fee_amount, // Dlmm fee is dynamic
                total_fee: quote.fee_amount,
            },
            transfer_fee_breakdown: HopTransferFeeBreakdown {
                input_gross: request.amount,
                input_transfer_fee,
                input_net_into_pool,
                output_gross_from_pool: gross_out,
                output_transfer_fee,
                output_net_to_next_hop,
            },
            price_impact: quote.price_impact_bps,
            unfilled_input_raw: input_net_into_pool.saturating_sub(quote.amount_in),
            required_accounts,
            state_slot: state.slot.0,
            state_hash: format!("{}_{}", state.slot.0, state.write_version),
            calculation_version: METEORA_DLMM_VERSION,
            quote_validity: true,
            computation_time_us: elapsed,
        })
    }

    fn quote_exact_out(&self, _state: &PoolState, _request: &QuoteRequest) -> Result<QuoteResult> {
        // DLMM exact-out requires reverse bin simulation with ceiling fees
        Err(ArbError::UnsupportedOperation(
            "Meteora DLMM quote_exact_out not yet supported".into(),
        ))
    }
}
