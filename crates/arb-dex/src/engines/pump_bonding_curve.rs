//! Pump.fun Bonding Curve Independent Quote Engine.
//! Program ID: `6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P`
//!
//! Enforces:
//! - Strict separation between pre-migration bonding curve and post-migration PumpSwap AMM.
//! - Virtual reserves formula `(x + dx)(y - dy) = k` with real reserves liquidity limits.
//! - Protocol fee: Exactly 100 bps (1.00%) taken on SOL.
//! - Rejects trading on graduated/complete pools (must use PumpSwap engine instead).

use crate::quote_engine::{FeesBreakdown, QuoteEngine, QuoteRequest, QuoteResult};
use crate::token2022::HopTransferFeeBreakdown;
use arb_types::{ArbError, Bps, DexProtocol, PoolState, Result};
use std::time::Instant;

pub const PUMP_BONDING_CURVE_VERSION: &str = "pump-bonding-curve-v1.0";
pub const PUMP_FEE_BPS: u128 = 100; // 1%

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

        if v_token == 0 || v_sol == 0 {
            return Err(ArbError::InsufficientLiquidity(
                "Pump.fun virtual reserves are 0".into(),
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

        let (gross_out, protocol_fee) = if is_buy {
            // BUY: SOL In -> Meme Token Out
            // 1% fee taken on SOL in
            let num_fee = (input_net_into_pool as u128).saturating_mul(PUMP_FEE_BPS);
            let fee_sol = num_fee.div_ceil(10_000); // ceil
            let sol_eff = (input_net_into_pool as u128).saturating_sub(fee_sol);

            if sol_eff == 0 {
                return Err(ArbError::InsufficientLiquidity(
                    "SOL input exhausted by 1% fee".into(),
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

            (tokens_out as u64, fee_sol as u64)
        } else {
            // SELL: Meme Token In -> SOL Out
            // sol_gross = floor((v_sol * tokens_in) / (v_token + tokens_in))
            let tokens_in = input_net_into_pool as u128;
            let num = v_sol.saturating_mul(tokens_in);
            let denom = v_token.saturating_add(tokens_in);
            let sol_gross = num / denom;

            // 1% fee taken on SOL out
            let num_fee = sol_gross.saturating_mul(PUMP_FEE_BPS);
            let fee_sol = num_fee.div_ceil(10_000); // ceil
            let sol_out = sol_gross.saturating_sub(fee_sol);

            if sol_out == 0 {
                return Err(ArbError::InsufficientLiquidity(
                    "Pump.fun sell returned 0 SOL".into(),
                ));
            }

            (sol_out as u64, fee_sol as u64)
        };

        // 2. Outbound Token-2022 transfer fee deduction
        let (output_net_to_next_hop, output_transfer_fee) =
            if let Some(config) = request.output_transfer_fee_config {
                let fee = config.calculate_fee(gross_out);
                (gross_out.saturating_sub(fee), fee)
            } else {
                (gross_out, 0)
            };

        // 3. Slippage calculation
        let min_amount_out = output_net_to_next_hop.saturating_sub(
            (output_net_to_next_hop as u128 * request.slippage_bps as u128 / 10_000) as u64,
        );

        let elapsed = start_time.elapsed().as_micros() as u64;

        Ok(QuoteResult {
            protocol: DexProtocol::PumpFun,
            pool_id: state.pool_id,
            amount_in_raw: request.amount,
            amount_out_raw: output_net_to_next_hop,
            minimum_amount_out_raw: min_amount_out,
            fees_breakdown: FeesBreakdown {
                lp_fee: 0,
                protocol_fee,
                fund_fee: 0,
                creator_fee: 0,
                dynamic_fee: 0,
                total_fee: protocol_fee,
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
            calculation_version: PUMP_BONDING_CURVE_VERSION,
            quote_validity: true,
            computation_time_us: elapsed,
        })
    }

    fn quote_exact_out(&self, _state: &PoolState, _request: &QuoteRequest) -> Result<QuoteResult> {
        Err(ArbError::UnsupportedOperation(
            "Pump.fun bonding curve exact out not yet supported".into(),
        ))
    }
}
