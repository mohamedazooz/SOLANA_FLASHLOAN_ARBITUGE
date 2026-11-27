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
        let (input_net_into_pool, input_transfer_fee) = request.input_after_transfer_fee()?;

        if input_net_into_pool == 0 {
            return Err(ArbError::ZeroAmount);
        }

        // Raydium CPMM has two independent fees. Trade fee is always on input;
        // creator fee may be on input or output according to PoolState.creator_fee_on.
        // A missing or inconsistent fee snapshot is never approximated.
        let cpmm_fees = state.raydium_cpmm_fee.ok_or_else(|| {
            ArbError::IncompleteState(
                "Raydium CPMM creator-fee mode is missing; refusing generic-fee fallback".into(),
            )
        })?;
        if !cpmm_fees.is_valid()
            || state.fee_numerator != cpmm_fees.trade_fee_rate_ppm
            || state.fee_denominator != CPMM_FEE_RATE_DENOMINATOR as u64
        {
            return Err(ArbError::InvalidInput(
                "Raydium CPMM PoolState and exact AmmConfig fee snapshot disagree".into(),
            ));
        }
        let trade_rate = cpmm_fees.trade_fee_rate_ppm;
        let creator_rate = cpmm_fees.effective_creator_fee_rate();
        let creator_on_input = cpmm_fees.creator_fee_on_input(a_to_b).ok_or_else(|| {
            ArbError::InvalidInput("Raydium CPMM creator fee mode is invalid".into())
        })?;
        let input_fee_rate = trade_rate
            .checked_add(if creator_on_input { creator_rate } else { 0 })
            .ok_or(ArbError::Overflow)?;
        let input_fee = arb_math::cpmm::cpmm_fee_ceil(
            u128::from(input_net_into_pool),
            u128::from(input_fee_rate),
            CPMM_FEE_RATE_DENOMINATOR,
        )
        .ok_or_else(|| ArbError::InvalidInput("Raydium CPMM input fee overflow".into()))?;
        let trade_fee = if creator_on_input && creator_rate > 0 {
            let creator = u128::from(input_fee)
                .checked_mul(u128::from(creator_rate))
                .ok_or(ArbError::Overflow)?
                / u128::from(input_fee_rate);
            input_fee
                .checked_sub(u64::try_from(creator).map_err(|_| ArbError::Overflow)?)
                .ok_or(ArbError::Overflow)?
        } else {
            input_fee
        };
        let creator_fee_input = if creator_on_input && creator_rate > 0 {
            input_fee.checked_sub(trade_fee).ok_or(ArbError::Overflow)?
        } else {
            0
        };
        let curve_output = arb_math::cpmm::cpmm_out_upfront_fee_ceil(
            u128::from(input_net_into_pool),
            r_in,
            r_out,
            u128::from(input_fee_rate),
            CPMM_FEE_RATE_DENOMINATOR,
        )
        .ok_or(ArbError::Overflow)?;
        if curve_output == 0 {
            return Err(ArbError::InsufficientLiquidity(
                "Raydium CPMM returned 0 output tokens".into(),
            ));
        }
        let creator_fee_output = if !creator_on_input && creator_rate > 0 {
            let fee = u128::from(curve_output)
                .checked_mul(u128::from(creator_rate))
                .and_then(|v| v.checked_add(CPMM_FEE_RATE_DENOMINATOR - 1))
                .ok_or(ArbError::Overflow)?
                / CPMM_FEE_RATE_DENOMINATOR;
            u64::try_from(fee).map_err(|_| ArbError::Overflow)?
        } else {
            0
        };
        let gross_out = curve_output
            .checked_sub(creator_fee_output)
            .ok_or_else(|| {
                ArbError::InvalidInput("Raydium CPMM creator fee exceeds curve output".into())
            })?;
        if gross_out == 0 {
            return Err(ArbError::InsufficientLiquidity(
                "Raydium CPMM creator fee exhausted output".into(),
            ));
        }
        let creator_fee = creator_fee_input
            .checked_add(creator_fee_output)
            .ok_or(ArbError::Overflow)?;
        let protocol_fee = u64::try_from(
            u128::from(trade_fee) * u128::from(cpmm_fees.protocol_fee_rate)
                / CPMM_FEE_RATE_DENOMINATOR,
        )
        .map_err(|_| ArbError::Overflow)?;
        let fund_fee = u64::try_from(
            u128::from(trade_fee) * u128::from(cpmm_fees.fund_fee_rate) / CPMM_FEE_RATE_DENOMINATOR,
        )
        .map_err(|_| ArbError::Overflow)?;
        let lp_fee = trade_fee
            .checked_sub(protocol_fee)
            .and_then(|v| v.checked_sub(fund_fee))
            .ok_or_else(|| {
                ArbError::InvalidInput("Raydium CPMM fee splits exceed trade fee".into())
            })?;

        // 4. Outbound Token-2022 transfer fee deduction (after creator output fee).
        let (output_net_to_next_hop, output_transfer_fee) =
            request.output_after_transfer_fee(gross_out)?;

        // 5. Slippage calculation
        let min_amount_out = request.minimum_amount_out(output_net_to_next_hop);

        // Price impact: impact = dx / (x + dx)
        let impact_bps = arb_math::cpmm::cpmm_price_impact_bps(
            u128::from(
                input_net_into_pool
                    .checked_sub(input_fee)
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
            total_fee_mint: request.input_mint,
            creator_fee_mint: if creator_fee_output > 0 {
                request.output_mint
            } else {
                request.input_mint
            },
            amount_in_raw: request.amount,
            amount_out_raw: output_net_to_next_hop,
            minimum_amount_out_raw: min_amount_out,
            fees_breakdown: FeesBreakdown {
                lp_fee,
                compounding_fee: 0,
                protocol_fee,
                fund_fee,
                creator_fee,
                referral_fee: 0,
                dynamic_fee: 0,
                // Input-side pool fees only (input mint raw units). When creator
                // fee is charged on output, creator_fee is in output mint units.
                total_fee: input_fee,
            },
            transfer_fee_breakdown: HopTransferFeeBreakdown {
                input_gross: request.amount,
                input_transfer_fee,
                input_net_into_pool,
                output_gross_from_pool: gross_out,
                output_curve_reserve_debit: curve_output,
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
            clmm_steps_taken: None,
        })
    }

    fn quote_exact_out(&self, state: &PoolState, request: &QuoteRequest) -> Result<QuoteResult> {
        request.validate(state)?;
        if !state.is_fee_valid() {
            return Err(ArbError::InvalidInput(
                "Raydium CPMM fee snapshot is missing or invalid".into(),
            ));
        }
        if !state.is_active {
            return Err(ArbError::PoolDisabled(format!(
                "Raydium CPMM pool {} is disabled",
                state.pool_id.0
            )));
        }
        let cpmm_fees = state.raydium_cpmm_fee.ok_or_else(|| {
            ArbError::IncompleteState("Raydium CPMM exact fee mode is missing".into())
        })?;
        if !cpmm_fees.is_valid()
            || state.fee_numerator != cpmm_fees.trade_fee_rate_ppm
            || state.fee_denominator != CPMM_FEE_RATE_DENOMINATOR as u64
        {
            return Err(ArbError::InvalidInput(
                "Raydium CPMM fee snapshot mismatch".into(),
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
        let creator_rate = cpmm_fees.effective_creator_fee_rate();
        let creator_on_input = cpmm_fees.creator_fee_on_input(a_to_b).ok_or_else(|| {
            ArbError::InvalidInput("Raydium CPMM creator fee mode is invalid".into())
        })?;
        let (curve_target_out, creator_fee_output) = if !creator_on_input && creator_rate > 0 {
            let denom = CPMM_FEE_RATE_DENOMINATOR
                .checked_sub(u128::from(creator_rate))
                .filter(|d| *d > 0)
                .ok_or_else(|| ArbError::InvalidInput("Raydium CPMM creator fee >= 100%".into()))?;
            let gross_curve = u128::from(request.amount)
                .checked_mul(CPMM_FEE_RATE_DENOMINATOR)
                .and_then(|v| v.checked_add(denom - 1))
                .ok_or(ArbError::Overflow)?
                / denom;
            let gross_u64 = u64::try_from(gross_curve).map_err(|_| ArbError::Overflow)?;
            let c_fee = gross_u64
                .checked_sub(request.amount)
                .ok_or(ArbError::Overflow)?;
            (gross_u64, c_fee)
        } else {
            (request.amount, 0)
        };
        let input_fee_rate = u128::from(cpmm_fees.trade_fee_rate_ppm)
            .checked_add(if creator_on_input {
                u128::from(creator_rate)
            } else {
                0
            })
            .ok_or(ArbError::Overflow)?;
        let fee_den = u128::from(state.fee_denominator);
        let req_in = arb_math::cpmm::cpmm_in_for_exact_out_upfront_fee_ceil(
            u128::from(curve_target_out),
            r_in,
            r_out,
            input_fee_rate,
            fee_den,
        )
        .ok_or_else(|| {
            ArbError::InsufficientLiquidity(
                "Exact out exceeds pool reserve or has an invalid fee ratio".into(),
            )
        })?;
        let total_input_fee =
            arb_math::cpmm::cpmm_fee_ceil(u128::from(req_in), input_fee_rate, fee_den)
                .ok_or(ArbError::Overflow)?;
        let trade_fee = if creator_on_input && creator_rate > 0 {
            let creator = u128::from(total_input_fee)
                .checked_mul(u128::from(creator_rate))
                .ok_or(ArbError::Overflow)?
                / input_fee_rate;
            total_input_fee
                .checked_sub(u64::try_from(creator).map_err(|_| ArbError::Overflow)?)
                .ok_or(ArbError::Overflow)?
        } else {
            total_input_fee
        };
        let creator_fee_input = if creator_on_input && creator_rate > 0 {
            total_input_fee
                .checked_sub(trade_fee)
                .ok_or(ArbError::Overflow)?
        } else {
            0
        };
        let protocol_fee = u64::try_from(
            u128::from(trade_fee) * u128::from(cpmm_fees.protocol_fee_rate)
                / CPMM_FEE_RATE_DENOMINATOR,
        )
        .map_err(|_| ArbError::Overflow)?;
        let fund_fee = u64::try_from(
            u128::from(trade_fee) * u128::from(cpmm_fees.fund_fee_rate) / CPMM_FEE_RATE_DENOMINATOR,
        )
        .map_err(|_| ArbError::Overflow)?;
        let lp_fee = trade_fee
            .checked_sub(protocol_fee)
            .and_then(|v| v.checked_sub(fund_fee))
            .ok_or_else(|| {
                ArbError::InvalidInput("Raydium CPMM fee splits exceed trade fee".into())
            })?;

        Ok(QuoteResult {
            protocol: DexProtocol::RaydiumCpmm,
            pool_id: state.pool_id,
            total_fee_mint: request.input_mint,
            creator_fee_mint: if creator_fee_output > 0 {
                request.output_mint
            } else {
                request.input_mint
            },
            amount_in_raw: req_in,
            amount_out_raw: request.amount,
            minimum_amount_out_raw: request.amount,
            fees_breakdown: FeesBreakdown {
                lp_fee,
                compounding_fee: 0,
                protocol_fee,
                fund_fee,
                creator_fee: creator_fee_input.saturating_add(creator_fee_output),
                referral_fee: 0,
                dynamic_fee: 0,
                total_fee: total_input_fee,
            },
            transfer_fee_breakdown: HopTransferFeeBreakdown {
                input_gross: req_in,
                input_transfer_fee: 0,
                input_net_into_pool: req_in
                    .checked_sub(total_input_fee)
                    .ok_or(ArbError::Overflow)?,
                output_gross_from_pool: curve_target_out,
                output_curve_reserve_debit: curve_target_out,
                output_transfer_fee: 0,
                output_net_to_next_hop: request.amount,
            },
            price_impact: arb_math::cpmm::cpmm_price_impact_bps(
                u128::from(
                    req_in
                        .checked_sub(total_input_fee)
                        .ok_or(ArbError::Overflow)?,
                ),
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
            clmm_steps_taken: None,
        })
    }
}

#[cfg(test)]
mod official_fee_rounding_parity_tests {
    use super::*;
    use arb_types::{MintId, PoolId, PoolKind, RaydiumCpmmFeeState, Slot};
    use solana_pubkey::Pubkey;

    fn pool() -> PoolState {
        PoolState {
            pool_id: PoolId::new([9; 32]),
            protocol: DexProtocol::RaydiumCpmm,
            slot: Slot(1),
            write_version: 1,
            base_mint: MintId(Pubkey::new_from_array([1; 32])),
            quote_mint: MintId(Pubkey::new_from_array([2; 32])),
            decimals_a: 6,
            decimals_b: 6,
            reserve_in: 1_000_000,
            reserve_out: 1_000_000,
            fee_numerator: 2_500,
            fee_denominator: CPMM_FEE_RATE_DENOMINATOR as u64,
            raydium_cpmm_fee: Some(RaydiumCpmmFeeState {
                trade_fee_rate_ppm: 2_500,
                creator_fee_rate_ppm: 1_000,
                protocol_fee_rate: 120_000,
                fund_fee_rate: 80_000,
                enable_creator_fee: true,
                creator_fee_on: 0,
            }),
            is_active: true,
            has_risky_token: false,
            kind: PoolKind::Cpmm,
            clmm: None,
            dlmm: None,
            pump_fun: None,
            damm_v2: None,
            transfer_fee_bps: 0,
            token_programs: Default::default(),
            token2022_fee_context: None,
            execution_accounts: None,
            last_update_slot: 1,
            last_update_ms: 0,
        }
    }

    fn request(amount: u64) -> QuoteRequest {
        let state = pool();
        QuoteRequest {
            input_mint: state.base_mint,
            output_mint: state.quote_mint,
            amount,
            slippage_bps: 50,
            input_transfer_fee_config: None,
            output_transfer_fee_config: None,
        }
    }

    #[test]
    fn exact_in_matches_official_combined_ceil_then_creator_split() {
        // Official CP-Swap: ceil(input * (trade + creator) / 1e6),
        // then floor(total_fee * creator / (trade + creator)).
        let quote = RaydiumCpmmQuoteEngine::new()
            .quote_exact_in(&pool(), &request(100_001))
            .unwrap();

        assert_eq!(quote.fees_breakdown.total_fee, 351);
        assert_eq!(quote.fees_breakdown.creator_fee, 100);
        assert_eq!(quote.fees_breakdown.lp_fee, 201);
        assert_eq!(quote.fees_breakdown.protocol_fee, 30);
        assert_eq!(quote.fees_breakdown.fund_fee, 20);
        assert_eq!(quote.transfer_fee_breakdown.input_net_into_pool, 100_001);
        assert_eq!(
            quote.transfer_fee_breakdown.output_curve_reserve_debit,
            90_619
        );
        // Curve impact is calculated from 99,650 after input-side fees, not
        // from gross input (100,001); swap fees are not counted as price impact.
        assert_eq!(quote.price_impact.0, 907);
    }

    #[test]
    fn exact_out_matches_official_pre_fee_inverse_and_round_trips() {
        let state = pool();
        let exact_out = RaydiumCpmmQuoteEngine::new()
            .quote_exact_out(
                &state,
                &QuoteRequest {
                    input_mint: state.base_mint,
                    output_mint: state.quote_mint,
                    amount: 1_000,
                    slippage_bps: 50,
                    input_transfer_fee_config: None,
                    output_transfer_fee_config: None,
                },
            )
            .unwrap();

        assert_eq!(exact_out.amount_in_raw, 1_006);
        assert_eq!(exact_out.fees_breakdown.total_fee, 4);
        assert_eq!(exact_out.fees_breakdown.creator_fee, 1);
        assert_eq!(exact_out.transfer_fee_breakdown.input_net_into_pool, 1_002);

        let exact_in = RaydiumCpmmQuoteEngine::new()
            .quote_exact_in(&state, &request(exact_out.amount_in_raw))
            .unwrap();
        assert_eq!(exact_in.amount_out_raw, 1_000);
    }
}
