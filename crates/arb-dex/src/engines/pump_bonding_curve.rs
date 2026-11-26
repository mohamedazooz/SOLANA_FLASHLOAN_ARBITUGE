//! Pump.fun Bonding Curve Independent Quote Engine.
//! Program ID: `6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P`
//!
//! Enforces:
//! - Strict separation between pre-migration bonding curve and post-migration PumpSwap AMM.
//! - Virtual reserves formula `(x + dx)(y - dy) = k` with real reserves liquidity limits.
//! - Requires a verified aggregate fee ratio from the Pump bonding-curve FeeConfig.
//! - Rejects trading on graduated/complete pools (must use PumpSwap engine instead).

use crate::quote_engine::{FeesBreakdown, QuoteEngine, QuoteRequest, QuoteResult};
use crate::token2022::HopTransferFeeBreakdown;
use arb_types::{ArbError, Bps, DexProtocol, PoolState, Result};
use std::time::Instant;

pub const PUMP_BONDING_CURVE_VERSION: &str = "pump-bonding-curve-v1.0";

#[derive(Debug, Default, Clone)]
pub struct PumpBondingCurveQuoteEngine;

impl PumpBondingCurveQuoteEngine {
    pub fn new() -> Self {
        Self
    }
}

impl QuoteEngine for PumpBondingCurveQuoteEngine {
    fn protocol_id(&self) -> DexProtocol {
        DexProtocol::PumpFun
    }

    fn quote_exact_in(&self, state: &PoolState, request: &QuoteRequest) -> Result<QuoteResult> {
        request.validate(state)?;
        let start_time = Instant::now();

        if !state.is_active {
            return Err(ArbError::PoolDisabled(format!(
                "Pump.fun curve {} is disabled",
                state.pool_id.0
            )));
        }

        let pump_state = state.pump_fun.as_ref().ok_or_else(|| {
            ArbError::IncompleteState(format!(
                "Missing Pump.fun curve state for {}",
                state.pool_id.0
            ))
        })?;

        if pump_state.complete {
            return Err(ArbError::PoolMigrated(format!(
                "Bonding curve {} has graduated; use PumpSwap quote engine",
                state.pool_id.0
            )));
        }

        // Base mint is WSOL (SOL), Quote mint is the meme token
        let is_buy = request.input_mint == state.base_mint;
        if !is_buy && request.input_mint != state.quote_mint {
            return Err(ArbError::InvalidToken(format!(
                "Input mint {} does not match curve mints",
                request.input_mint.0
            )));
        }

        let v_token = pump_state.virtual_token_reserves as u128;
        let v_sol = pump_state.virtual_sol_reserves as u128;
        let r_token = pump_state.real_token_reserves as u128;
        let r_sol = pump_state.real_sol_reserves as u128;

        if v_token == 0 || v_sol == 0 {
            return Err(ArbError::InsufficientLiquidity(
                "Pump.fun virtual reserves are 0".into(),
            ));
        }

        // 1. Inbound Token-2022 transfer fee deduction
        let (input_net_into_pool, input_transfer_fee) = request.input_after_transfer_fee()?;

        if input_net_into_pool == 0 {
            return Err(ArbError::ZeroAmount);
        }

        // Pump.fun SOL/USDC bonding curves use a flat 1.25% trade-fee schedule;
        // market-cap tiers belong to canonical post-graduation PumpSwap pools.
        // PoolState must carry a verified aggregate rate from the bonding-curve
        // FeeConfig snapshot; discovery metadata is never an execution fallback.
        let fee_num = u128::from(state.fee_numerator);
        let fee_den = u128::from(state.fee_denominator);
        if fee_den == 0 || fee_num == 0 || fee_num > fee_den {
            return Err(ArbError::IncompleteState(
                "Pump.fun quote requires a verified nonzero bonding-curve FeeConfig ratio".into(),
            ));
        }
        let calculate_trade_fee = |amount: u64| -> Result<u64> {
            if amount == 0 || fee_num == 0 {
                return Ok(0);
            }
            let numerator = u128::from(amount)
                .checked_mul(fee_num)
                .ok_or(ArbError::Overflow)?;
            u64::try_from(numerator.div_ceil(fee_den)).map_err(|_| ArbError::Overflow)
        };

        let (gross_out, trade_fee, curve_gross_out) = if is_buy {
            // BUY: SOL In -> Meme Token Out; the trade fee is charged on SOL input.
            let fee_sol = calculate_trade_fee(input_net_into_pool)?;
            let sol_eff = u128::from(
                input_net_into_pool
                    .checked_sub(fee_sol)
                    .ok_or(ArbError::Overflow)?,
            );

            if sol_eff == 0 {
                return Err(ArbError::InsufficientLiquidity(
                    "Pump.fun trade fee exhausts SOL input".into(),
                ));
            }

            // tokens_out = floor((v_token * sol_eff) / (v_sol + sol_eff))
            let num = v_token.saturating_mul(sol_eff);
            let denom = v_sol.saturating_add(sol_eff);
            let tokens_out = (num / denom).min(r_token);

            if tokens_out == 0 {
                return Err(ArbError::InsufficientLiquidity(
                    "Pump.fun buy returned 0 tokens".into(),
                ));
            }

            (tokens_out as u64, fee_sol, tokens_out as u64)
        } else {
            // SELL: Meme Token In -> SOL Out
            // sol_gross = floor((v_sol * tokens_in) / (v_token + tokens_in))
            let tokens_in = input_net_into_pool as u128;
            let num = v_sol.saturating_mul(tokens_in);
            let denom = v_token.saturating_add(tokens_in);
            let sol_gross = num / denom;
            if sol_gross > r_sol {
                return Err(ArbError::PumpFunReserveExceeded {
                    requested: u64::try_from(sol_gross).unwrap_or(u64::MAX),
                    available: pump_state.real_sol_reserves,
                });
            }

            // SELL: the same verified trade fee is charged on gross SOL output.
            let sol_gross = u64::try_from(sol_gross).map_err(|_| ArbError::Overflow)?;
            let fee_sol = calculate_trade_fee(sol_gross)?;
            let sol_out = sol_gross.checked_sub(fee_sol).ok_or(ArbError::Overflow)?;

            if sol_out == 0 {
                return Err(ArbError::InsufficientLiquidity(
                    "Pump.fun sell returned 0 SOL".into(),
                ));
            }

            (sol_out, fee_sol, sol_gross)
        };

        // 2. Outbound Token-2022 transfer fee deduction
        let (output_net_to_next_hop, output_transfer_fee) =
            request.output_after_transfer_fee(gross_out)?;

        // 3. Slippage calculation
        let min_amount_out = request.minimum_amount_out(output_net_to_next_hop);

        let curve_input = if is_buy {
            input_net_into_pool
                .checked_sub(trade_fee)
                .ok_or(ArbError::Overflow)?
        } else {
            input_net_into_pool
        };
        let (spot_numerator, spot_denominator) = if is_buy {
            (
                u128::from(curve_input)
                    .checked_mul(v_token)
                    .ok_or(ArbError::Overflow)?,
                v_sol,
            )
        } else {
            (
                u128::from(curve_input)
                    .checked_mul(v_sol)
                    .ok_or(ArbError::Overflow)?,
                v_token,
            )
        };
        let spot_out = spot_numerator / spot_denominator;
        let impact_bps = u64::try_from(
            spot_out
                .saturating_sub(u128::from(curve_gross_out))
                .checked_mul(10_000)
                .ok_or(ArbError::Overflow)?
                .checked_div(spot_out)
                .unwrap_or(10_000),
        )
        .map_err(|_| ArbError::Overflow)?
        .min(10_000) as u16;

        let elapsed = start_time.elapsed().as_micros() as u64;

        Ok(QuoteResult {
            protocol: DexProtocol::PumpFun,
            pool_id: state.pool_id,
            total_fee_mint: request.input_mint,
            creator_fee_mint: request.input_mint,
            amount_in_raw: request.amount,
            amount_out_raw: output_net_to_next_hop,
            minimum_amount_out_raw: min_amount_out,
            fees_breakdown: FeesBreakdown {
                lp_fee: 0,
                compounding_fee: 0,
                protocol_fee: 0,
                fund_fee: 0,
                creator_fee: 0,
                referral_fee: 0,
                dynamic_fee: 0,
                total_fee: trade_fee,
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
            price_impact: Bps(impact_bps),
            unfilled_input_raw: 0,
            required_accounts: vec![state.pool_id.0],
            state_slot: state.slot.0,
            state_hash: format!("{}_{}", state.slot.0, state.write_version),
            calculation_version: PUMP_BONDING_CURVE_VERSION,
            quote_validity: true,
            computation_time_us: elapsed,
            dlmm_bins_crossed: None,
            clmm_steps_taken: None,
        })
    }

    fn quote_exact_out(&self, _state: &PoolState, _request: &QuoteRequest) -> Result<QuoteResult> {
        Err(ArbError::UnsupportedOperation(
            "Pump.fun bonding curve exact out not yet supported".into(),
        ))
    }
}
