//! Exact-in quote engine for Meteora DAMM v2.
//!
//! This engine consumes a fully verified same-context Pool + Vaults + Mints +
//! Clock snapshot. It deliberately rejects Token-2022 until transfer-fee and
//! hook account metadata have been carried end-to-end into transaction building.

use crate::quote_engine::{FeesBreakdown, QuoteEngine, QuoteRequest, QuoteResult};
use crate::token2022::HopTransferFeeBreakdown;
use arb_math::{
    DAMM_V2_FEE_DENOMINATOR, DammV2FeeParams, quote_damm_v2_compounding_exact_in,
    quote_damm_v2_exact_in_with_fee_mode,
};
use arb_types::{
    ArbError, Bps, DexProtocol, PoolExecutionAccounts, PoolState, Result, TokenProgramKind,
};
use ruint::aliases::U256;
use solana_pubkey::Pubkey;
use std::str::FromStr;
use std::time::Instant;

pub const METEORA_DAMM_V2_QUOTE_VERSION: &str = "meteora-damm-v2-upstream-state-v1";

#[derive(Debug, Default, Clone)]
pub struct MeteoraDammV2QuoteEngine;

impl MeteoraDammV2QuoteEngine {
    pub fn new() -> Self {
        Self
    }

    fn fee_params(state: &arb_types::DammV2State) -> DammV2FeeParams {
        DammV2FeeParams {
            cliff_fee_numerator: state.cliff_fee_numerator,
            fee_scheduler_mode: state.fee_scheduler_mode,
            number_of_period: state.number_of_period,
            period_frequency: state.period_frequency,
            reduction_factor: state.reduction_factor,
            sqrt_price_step_bps: state.sqrt_price_step_bps,
            scheduler_expiration_duration: state.scheduler_expiration_duration,
            init_sqrt_price_x64: state.init_sqrt_price_x64,
            fee_version: state.fee_version,
            protocol_fee_percent: state.protocol_fee_percent,
            referral_fee_percent: state.referral_fee_percent,
            compounding_fee_bps: state.compounding_fee_bps,
            dynamic_fee_initialized: state.dynamic_fee_initialized,
            variable_fee_control: state.variable_fee_control,
            max_volatility_accumulator: state.max_volatility_accumulator,
            bin_step: state.bin_step,
            volatility_accumulator: state.volatility_accumulator,
            filter_period: state.filter_period,
            decay_period: state.decay_period,
            dynamic_reduction_factor: state.dynamic_reduction_factor,
            last_update_timestamp: state.last_update_timestamp,
            bin_step_u128: state.bin_step_u128,
            sqrt_price_reference: state.sqrt_price_reference,
            volatility_reference: state.volatility_reference,
            collect_fee_mode: state.collect_fee_mode,
        }
    }

    fn fee_ceil(amount: u64, numerator: u128) -> Result<u64> {
        let product = u128::from(amount)
            .checked_mul(numerator)
            .ok_or(ArbError::Overflow)?;
        let rounded = product
            .checked_add(DAMM_V2_FEE_DENOMINATOR - 1)
            .ok_or(ArbError::Overflow)?
            / DAMM_V2_FEE_DENOMINATOR;
        u64::try_from(rounded).map_err(|_| ArbError::Overflow)
    }

    fn verified_accounts(state: &PoolState) -> Result<Vec<Pubkey>> {
        let Some(PoolExecutionAccounts::MeteoraDammV2 {
            token_a_mint,
            token_b_mint,
            authority,
            vault_a,
            vault_b,
            event_authority,
        }) = state.execution_accounts.as_ref()
        else {
            return Err(ArbError::IncompleteState(
                "DAMM v2 verified swap account graph is unavailable".into(),
            ));
        };
        let damm = state
            .damm_v2
            .as_ref()
            .ok_or_else(|| ArbError::IncompleteState("DAMM v2 quote state is missing".into()))?;
        if *token_a_mint != state.base_mint.0
            || *token_b_mint != state.quote_mint.0
            || *vault_a == Pubkey::default()
            || *vault_b == Pubkey::default()
            || *authority == Pubkey::default()
            || *event_authority == Pubkey::default()
            || !damm.account_graph_verified
        {
            return Err(ArbError::InvalidInput(
                "DAMM v2 execution account graph does not match canonical pool mint order".into(),
            ));
        }
        let program = Pubkey::from_str(arb_constants::METEORA_DAMM_V2_PROGRAM_ID)
            .map_err(|_| ArbError::InvalidInput("invalid DAMM v2 program id".into()))?;
        Ok(vec![
            state.pool_id.0,
            *vault_a,
            *vault_b,
            *token_a_mint,
            *token_b_mint,
            *authority,
            *event_authority,
            program,
        ])
    }

    /// Price impact against the pre-swap spot rate, excluding pool and transfer
    /// fees. Concentrated liquidity uses virtual reserves from sqrt price/liquidity;
    /// Compounding uses the Pool V1 tracked reserves.
    fn price_impact_bps(
        curve_input: u64,
        curve_output: u64,
        reserve_in: u128,
        reserve_out: u128,
    ) -> Result<Bps> {
        if curve_input == 0 || curve_output == 0 || reserve_in == 0 || reserve_out == 0 {
            return Err(ArbError::CalculationError(
                "DAMM v2 price-impact input/reserves are zero".into(),
            ));
        }
        let ideal_output = U256::from(curve_input)
            .checked_mul(U256::from(reserve_out))
            .ok_or(ArbError::Overflow)?
            / U256::from(reserve_in);
        if ideal_output == U256::ZERO || U256::from(curve_output) >= ideal_output {
            return Ok(Bps(0));
        }
        let loss = ideal_output
            .checked_sub(U256::from(curve_output))
            .ok_or(ArbError::Overflow)?;
        let numerator = loss
            .checked_mul(U256::from(10_000u64))
            .ok_or(ArbError::Overflow)?;
        let impact = numerator
            .checked_add(ideal_output - U256::from(1u8))
            .ok_or(ArbError::Overflow)?
            / ideal_output;
        Ok(Bps(u16::try_from(impact).unwrap_or(10_000).min(10_000)))
    }
    fn curve_output(amount_in: u64, a_to_b: bool, state: &arb_types::DammV2State) -> Result<u64> {
        let output = if state.is_compounding() {
            if state.layout_version != 1 {
                return Err(ArbError::UnsupportedOperation(
                    "DAMM v2 Compounding requires LayoutVersion::V1 tracked reserve counters"
                        .into(),
                ));
            }
            quote_damm_v2_compounding_exact_in(
                amount_in,
                a_to_b,
                state.token_a_amount.ok_or_else(|| {
                    ArbError::IncompleteState("DAMM v2 token_a_amount is missing".into())
                })?,
                state.token_b_amount.ok_or_else(|| {
                    ArbError::IncompleteState("DAMM v2 token_b_amount is missing".into())
                })?,
                0,
                false,
            )
        } else {
            quote_damm_v2_exact_in_with_fee_mode(
                amount_in,
                a_to_b,
                state.sqrt_price_x64,
                state.liquidity,
                state.sqrt_min_price_x64,
                state.sqrt_max_price_x64,
                0,
                false,
            )
        };
        output.ok_or_else(|| {
            ArbError::CalculationError(
                "DAMM v2 curve quote overflowed or crossed its price-range boundary".into(),
            )
        })
    }
}

impl QuoteEngine for MeteoraDammV2QuoteEngine {
    fn protocol_id(&self) -> DexProtocol {
        DexProtocol::MeteoraDammV2
    }

    fn quote_exact_in(&self, state: &PoolState, request: &QuoteRequest) -> Result<QuoteResult> {
        request.validate(state)?;
        let started = Instant::now();
        if state.protocol != DexProtocol::MeteoraDammV2 {
            return Err(ArbError::InvalidInput(
                "DAMM v2 engine received another protocol".into(),
            ));
        }
        let damm = state.damm_v2.as_ref().ok_or_else(|| {
            ArbError::IncompleteState("DAMM v2 state is missing from PoolState".into())
        })?;
        if !damm.fee_context_matches(state.slot.0) {
            return Err(ArbError::IncompleteState(
                "DAMM v2 requires a same-context Pool/Vault/Mint/Clock snapshot".into(),
            ));
        }
        if state.token_programs != [TokenProgramKind::SplToken; 2]
            || state.has_risky_token
            || request.input_transfer_fee_config.is_some()
            || request.output_transfer_fee_config.is_some()
        {
            return Err(ArbError::UnsupportedOperation(
                "DAMM v2 Token-2022 remains disabled until epoch-aware transfer fees and Transfer Hook account resolution are wired into execution".into(),
            ));
        }
        let required_accounts = Self::verified_accounts(state)?;
        let a_to_b = request.input_mint == state.base_mint;
        let params = Self::fee_params(damm);
        let total_fee_numerator = params
            .effective_fee_numerator_at(
                damm.current_point,
                damm.activation_point,
                damm.sqrt_price_x64,
            )
            .ok_or_else(|| {
                ArbError::IncompleteState("DAMM v2 base/dynamic fee schedule is invalid".into())
            })?;
        if total_fee_numerator >= DAMM_V2_FEE_DENOMINATOR {
            return Err(ArbError::InvalidInput(
                "DAMM v2 fee numerator is outside the valid range".into(),
            ));
        }
        let base_fee_numerator = params
            .base_fee_numerator_at(
                damm.current_point,
                damm.activation_point,
                damm.sqrt_price_x64,
            )
            .ok_or_else(|| {
                ArbError::IncompleteState("DAMM v2 base fee schedule is invalid".into())
            })?;
        let fees_on_input = params.fees_on_input(a_to_b).ok_or_else(|| {
            ArbError::UnsupportedOperation("unknown DAMM v2 collect_fee_mode".into())
        })?;

        // Token transfer fees (if ever enabled) happen outside the pool fee model.
        let (input_net_into_pool, input_transfer_fee) = request.input_after_transfer_fee()?;
        let (curve_input, pool_fee_mint) = if fees_on_input {
            let fee = Self::fee_ceil(input_net_into_pool, total_fee_numerator)?;
            let net = input_net_into_pool
                .checked_sub(fee)
                .ok_or(ArbError::Overflow)?;
            if net == 0 {
                return Err(ArbError::ZeroAmount);
            }
            (net, request.input_mint)
        } else {
            (input_net_into_pool, request.output_mint)
        };
        let gross_curve_output = Self::curve_output(curve_input, a_to_b, damm)?;
        if gross_curve_output == 0 {
            return Err(ArbError::InsufficientLiquidity(
                "DAMM v2 returned zero curve output".into(),
            ));
        }
        let pool_fee = if fees_on_input {
            Self::fee_ceil(input_net_into_pool, total_fee_numerator)?
        } else {
            Self::fee_ceil(gross_curve_output, total_fee_numerator)?
        };
        let pool_output_before_transfer = if fees_on_input {
            gross_curve_output
        } else {
            gross_curve_output.checked_sub(pool_fee).ok_or_else(|| {
                ArbError::CalculationError("DAMM v2 output fee exceeds curve output".into())
            })?
        };
        if pool_output_before_transfer == 0 {
            return Err(ArbError::InsufficientLiquidity(
                "DAMM v2 output is zero after pool fee".into(),
            ));
        }
        let split = params.split_trading_fee(pool_fee, false).ok_or_else(|| {
            ArbError::CalculationError(
                "DAMM v2 fee split does not reconcile to one trading fee".into(),
            )
        })?;
        let (reserve_a, reserve_b) = if damm.is_compounding() {
            (
                u128::from(damm.token_a_amount.ok_or_else(|| {
                    ArbError::IncompleteState("DAMM v2 Compounding reserve A is missing".into())
                })?),
                u128::from(damm.token_b_amount.ok_or_else(|| {
                    ArbError::IncompleteState("DAMM v2 Compounding reserve B is missing".into())
                })?),
            )
        } else {
            arb_math::damm_v2_virtual_reserves(damm.sqrt_price_x64, damm.liquidity)
        };
        let (reserve_in, reserve_out) = if a_to_b {
            (reserve_a, reserve_b)
        } else {
            (reserve_b, reserve_a)
        };
        let price_impact =
            Self::price_impact_bps(curve_input, gross_curve_output, reserve_in, reserve_out)?;
        let (output_net_to_next_hop, output_transfer_fee) =
            request.output_after_transfer_fee(pool_output_before_transfer)?;
        if output_net_to_next_hop == 0 {
            return Err(ArbError::ZeroAmount);
        }
        let minimum_amount_out = request.minimum_amount_out(output_net_to_next_hop);
        let lp_fee = split.lp_claiming_fee;
        let computation_time_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        let state_hash = format!(
            "{}_{}_dammv2_f{}_l{}_m{}_p{}_t{}_sp{}_liq{}_min{}_max{}_base{}_mode{}_np{}_freq{}_red{}_step{}_expire{}_init{}_protocol{}_referral{}_compound{}_dyn{}_vcontrol{}_maxvol{}_bin{}_filter{}_decay{}_dreduction{}_last{}_binx{}_sref{}_vacc{}_vref{}_resA{:?}_resB{:?}",
            state.slot.0,
            state.write_version,
            damm.fee_version,
            damm.layout_version,
            damm.collect_fee_mode,
            damm.current_point,
            damm.current_timestamp,
            damm.sqrt_price_x64,
            damm.liquidity,
            damm.sqrt_min_price_x64,
            damm.sqrt_max_price_x64,
            damm.cliff_fee_numerator,
            damm.fee_scheduler_mode,
            damm.number_of_period,
            damm.period_frequency,
            damm.reduction_factor,
            damm.sqrt_price_step_bps,
            damm.scheduler_expiration_duration,
            damm.init_sqrt_price_x64,
            damm.protocol_fee_percent,
            damm.referral_fee_percent,
            damm.compounding_fee_bps,
            damm.dynamic_fee_initialized,
            damm.variable_fee_control,
            damm.max_volatility_accumulator,
            damm.bin_step,
            damm.filter_period,
            damm.decay_period,
            damm.dynamic_reduction_factor,
            damm.last_update_timestamp,
            damm.bin_step_u128,
            damm.sqrt_price_reference,
            damm.volatility_accumulator,
            damm.volatility_reference,
            damm.token_a_amount,
            damm.token_b_amount,
        );
        Ok(QuoteResult {
            protocol: DexProtocol::MeteoraDammV2,
            pool_id: state.pool_id,
            total_fee_mint: pool_fee_mint,
            creator_fee_mint: pool_fee_mint,
            amount_in_raw: request.amount,
            amount_out_raw: output_net_to_next_hop,
            minimum_amount_out_raw: minimum_amount_out,
            fees_breakdown: FeesBreakdown {
                lp_fee,
                compounding_fee: split.compounding_fee,
                protocol_fee: split.protocol_fee,
                fund_fee: 0,
                creator_fee: 0,
                referral_fee: split.referral_fee,
                // Use the exact marginal difference of two independently rounded
                // totals; this preserves the protocol's single ceil operation.
                dynamic_fee: pool_fee.saturating_sub(Self::fee_ceil(
                    if fees_on_input {
                        input_net_into_pool
                    } else {
                        gross_curve_output
                    },
                    base_fee_numerator,
                )?),
                total_fee: pool_fee,
            },
            transfer_fee_breakdown: HopTransferFeeBreakdown {
                input_gross: request.amount,
                input_transfer_fee,
                input_net_into_pool,
                output_gross_from_pool: pool_output_before_transfer,
                output_curve_reserve_debit: gross_curve_output,
                output_transfer_fee,
                output_net_to_next_hop,
            },
            price_impact,
            unfilled_input_raw: 0,
            required_accounts,
            state_slot: state.slot.0,
            state_hash,
            calculation_version: METEORA_DAMM_V2_QUOTE_VERSION,
            quote_validity: true,
            computation_time_us,
            dlmm_bins_crossed: None,
            clmm_steps_taken: None,
        })
    }

    fn quote_exact_out(&self, _state: &PoolState, _request: &QuoteRequest) -> Result<QuoteResult> {
        Err(ArbError::UnsupportedOperation(
            "DAMM v2 exact-out is not enabled until a differential exact-out/min-out parity suite is available".into(),
        ))
    }
}
