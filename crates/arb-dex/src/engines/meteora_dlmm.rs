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
use arb_constants::{
    METEORA_BINS_PER_BIN_ARRAY, METEORA_DLMM_PROGRAM_ID, METEORA_MAX_BIN_STEP,
    meteora_bin_array_index_requires_bitmap_extension,
};
use arb_math::dlmm::{
    DlmmBin, DlmmParameters, DlmmPoolState, DlmmVariableParameters, quote_dlmm_view,
};
use arb_types::{ArbError, DexProtocol, PoolExecutionAccounts, PoolState, Result};
use solana_pubkey::Pubkey;
use std::collections::HashSet;
use std::str::FromStr;
use std::time::Instant;

pub const METEORA_DLMM_VERSION: &str = "meteora-dlmm-v1.4";

/// Require the existence-proven bitmap extension when a swap crosses outside
/// the LbPair default bitmap range. A derived PDA alone is not sufficient:
/// hydration must have fetched the account and attached it to execution state.
fn verified_bitmap_extension(
    execution_accounts: Option<&PoolExecutionAccounts>,
    array_indices: &[i64],
) -> Result<Option<Pubkey>> {
    if !array_indices
        .iter()
        .any(|index| meteora_bin_array_index_requires_bitmap_extension(*index))
    {
        return Ok(None);
    }

    match execution_accounts {
        Some(PoolExecutionAccounts::MeteoraDlmm {
            bitmap_extension: Some(extension),
            ..
        }) => Ok(Some(*extension)),
        _ => Err(ArbError::IncompleteState(
            "Meteora DLMM traversal requires BinArrayBitmapExtension, but no existence-proven account is hydrated"
                .into(),
        )),
    }
}

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
        request.validate(state)?;
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

        // 2. Require verified LbPair static and variable parameters. Guessing
        // fee or volatility fields can overstate output and create phantom arbitrage.
        if dlmm_state.status != 0 {
            return Err(ArbError::PoolDisabled(format!(
                "Meteora DLMM pool {} has non-enabled on-chain status {}",
                state.pool_id.0, dlmm_state.status
            )));
        }
        if !(1..=METEORA_MAX_BIN_STEP).contains(&dlmm_state.bin_step_bps) {
            return Err(ArbError::InvalidInput(format!(
                "Meteora DLMM pool {} has invalid bin_step {}",
                state.pool_id.0, dlmm_state.bin_step_bps
            )));
        }
        let fee_params = dlmm_state.params.as_ref().ok_or_else(|| {
            ArbError::IncompleteState(format!(
                "Meteora DLMM pool {} is missing verified LbPair fee parameters",
                state.pool_id.0
            ))
        })?;
        if fee_params.function_type > 1 || fee_params.collect_fee_mode > 1 {
            return Err(ArbError::UnsupportedOperation(format!(
                "Meteora DLMM function_type {} / collect_fee_mode {} is not modeled",
                fee_params.function_type, fee_params.collect_fee_mode
            )));
        }
        if fee_params.protocol_share > 10_000 || fee_params.reduction_factor > 10_000 {
            return Err(ArbError::InvalidInput(
                "Meteora DLMM protocol_share or reduction_factor exceeds 10000 bps".into(),
            ));
        }
        let params = DlmmParameters {
            bin_step: dlmm_state.bin_step_bps,
            base_factor: fee_params.base_factor,
            filter_period: fee_params.filter_period,
            decay_period: fee_params.decay_period,
            reduction_factor: fee_params.reduction_factor,
            variable_fee_control: fee_params.variable_fee_control,
            max_volatility_accumulator: fee_params.max_volatility_accumulator,
            protocol_share: fee_params.protocol_share,
            base_fee_power_factor: fee_params.base_fee_power_factor,
            collect_fee_mode: fee_params.collect_fee_mode,
        };
        let v_params = DlmmVariableParameters {
            volatility_accumulator: dlmm_state.volatility_accumulator,
            volatility_reference: dlmm_state.volatility_reference,
            index_reference: dlmm_state.index_reference,
            last_update_timestamp: dlmm_state.last_update_timestamp,
        };

        // `last_update_ms` records local snapshot receipt time, not the on-chain Clock sysvar.
        // Do not use it to decay the variable fee: a local clock ahead of the bank
        // clock could understate fees. Without a trusted bank timestamp, retain the
        // on-chain volatility reference and intentionally quote conservatively.

        let pool_math_state = DlmmPoolState {
            active_id: dlmm_state.active_bin_id,
            bin_step: dlmm_state.bin_step_bps,
            status: dlmm_state.status,
            parameters: params,
            v_parameters: v_params,
            support_limit_order: dlmm_state.support_limit_order,
        };

        let mut seen_bin_ids = HashSet::with_capacity(dlmm_state.bins.len());
        let bins: Vec<DlmmBin> = dlmm_state
            .bins
            .iter()
            .map(|b| {
                if !seen_bin_ids.insert(b.bin_id) {
                    return Err(ArbError::IncompleteState(format!(
                        "duplicate Meteora DLMM bin id {} in pool {}",
                        b.bin_id, state.pool_id.0
                    )));
                }
                Ok(DlmmBin {
                    id: b.bin_id,
                    amount_x: u64::try_from(b.amount_x).map_err(|_| ArbError::Overflow)?,
                    amount_y: u64::try_from(b.amount_y).map_err(|_| ArbError::Overflow)?,
                    price_q64: 0,
                    limit_order_ask_side: b.limit_order_ask_side,
                    open_order_amount: b.open_order_amount,
                    processed_order_remaining_amount: b.processed_order_remaining_amount,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        if bins.is_empty() || !seen_bin_ids.contains(&dlmm_state.active_bin_id) {
            return Err(ArbError::IncompleteState(format!(
                "Meteora DLMM pool {} has no active bin snapshot",
                state.pool_id.0
            )));
        }
        // 3. Exact multi-bin discrete quoting
        let quote = quote_dlmm_view(&pool_math_state, &bins, input_net_into_pool, a_to_b, None)
            .map_err(|e| ArbError::CalculationError(format!("Meteora DLMM math error: {e:?}")))?;

        if quote.amount_in != input_net_into_pool {
            return Err(ArbError::InsufficientLiquidity(format!(
                "Meteora DLMM consumed {} of {} input raw units",
                quote.amount_in, input_net_into_pool
            )));
        }
        arb_math::dlmm::validate_swap_depth(quote.bins_crossed).map_err(|e| {
            ArbError::InsufficientLiquidity(format!("Meteora DLMM swap is not executable: {e}"))
        })?;
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
                (gross_out.checked_sub(fee).ok_or(ArbError::Overflow)?, fee)
            } else {
                (gross_out, 0)
            };

        // 5. Slippage calculation
        let min_amount_out = arb_constants::slippage_floor_bps(
            output_net_to_next_hop,
            u64::from(request.slippage_bps),
        );

        // 6. Include every BinArray traversed by the exact quote, in swap order.
        let lb_pair = state.pool_id.0;
        let start_array = Self::bin_id_to_bin_array_index(dlmm_state.active_bin_id);
        let end_array = Self::bin_id_to_bin_array_index(quote.end_active_id);
        let direction: i64 = if end_array >= start_array { 1 } else { -1 };
        let mut array_index = start_array;
        let mut required_accounts = Vec::with_capacity(8);
        let mut array_indices = Vec::with_capacity(
            usize::try_from((end_array - start_array).unsigned_abs())
                .unwrap_or(0)
                .saturating_add(1),
        );
        required_accounts.push(lb_pair);
        loop {
            array_indices.push(array_index);
            let (bin_array, _) = Self::derive_bin_array_pda(&lb_pair, array_index);
            let initialized_arrays = match state.execution_accounts.as_ref() {
                Some(PoolExecutionAccounts::MeteoraDlmm { bin_array_pdas, .. }) => bin_array_pdas,
                _ => {
                    return Err(ArbError::IncompleteState(
                        "Meteora DLMM quote has no bitmap-selected BinArray account set".into(),
                    ));
                }
            };
            // Only pass accounts proven initialized by the LbPair bitmap and,
            // when needed, the validated Bitmap Extension. Empty gaps are
            // represented in the local quote view, never as fake writable PDAs.
            if initialized_arrays.contains(&bin_array) && !required_accounts.contains(&bin_array) {
                required_accounts.push(bin_array);
            }
            if array_index == end_array {
                break;
            }
            array_index = array_index
                .checked_add(direction)
                .ok_or(ArbError::Overflow)?;
        }

        if let Some(bitmap_extension) =
            verified_bitmap_extension(state.execution_accounts.as_ref(), &array_indices)?
        {
            required_accounts.push(bitmap_extension);
        }
        let elapsed = start_time.elapsed().as_micros() as u64;

        Ok(QuoteResult {
            protocol: DexProtocol::MeteoraDlmm,
            pool_id: state.pool_id,
            amount_in_raw: request.amount,
            amount_out_raw: output_net_to_next_hop,
            minimum_amount_out_raw: min_amount_out,
            fees_breakdown: FeesBreakdown {
                lp_fee: quote
                    .fee_amount
                    .checked_sub(quote.protocol_fee)
                    .ok_or(ArbError::Overflow)?,
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
            unfilled_input_raw: 0,
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

#[cfg(test)]
mod bitmap_extension_tests {
    use super::*;

    fn execution_accounts(bitmap_extension: Option<Pubkey>) -> PoolExecutionAccounts {
        PoolExecutionAccounts::MeteoraDlmm {
            reserve_x: Pubkey::new_unique(),
            reserve_y: Pubkey::new_unique(),
            oracle: Pubkey::new_unique(),
            bin_array_pdas: vec![Pubkey::new_unique()],
            bitmap_extension,
            pool_token_x: Pubkey::new_unique(),
            pool_token_y: Pubkey::new_unique(),
        }
    }

    #[test]
    fn default_bitmap_range_does_not_require_extension() {
        assert_eq!(
            verified_bitmap_extension(None, &[-512, 0, 511]).unwrap(),
            None
        );
    }

    #[test]
    fn overflow_range_requires_existence_proven_extension() {
        assert!(verified_bitmap_extension(None, &[511, 512]).is_err());
        assert!(verified_bitmap_extension(None, &[-513, -512]).is_err());
    }

    #[test]
    fn overflow_range_returns_only_hydrated_extension() {
        let extension = Pubkey::new_unique();
        let accounts = execution_accounts(Some(extension));
        assert_eq!(
            verified_bitmap_extension(Some(&accounts), &[511, 512]).unwrap(),
            Some(extension)
        );
    }

    #[test]
    fn missing_extension_is_not_replaced_by_derived_placeholder() {
        let accounts = execution_accounts(None);
        assert!(verified_bitmap_extension(Some(&accounts), &[-513]).is_err());
    }
}
