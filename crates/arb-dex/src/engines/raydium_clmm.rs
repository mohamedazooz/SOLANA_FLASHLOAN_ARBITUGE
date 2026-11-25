//! Raydium Concentrated Liquidity (CLMM) Independent Quote Engine.
//! Program ID: `CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK`
//!
//! Enforces:
//! - Q64.64 sqrt price representation with bitwise binary exponentiation.
//! - Raydium CLMM standard tick array size (60 ticks per array).
//! - Active tick traversal, liquidity net updates upon crossing boundaries.
//! - Protocol fee rate with denominator 1,000,000.

use crate::quote_engine::{FeesBreakdown, QuoteEngine, QuoteRequest, QuoteResult};
use crate::token2022::HopTransferFeeBreakdown;
use arb_math::clmm::{
    ClmmTickMath, TickArrayFacade, TickFacade, compute_swap_with_ticks_per_array_and_math,
};
use arb_types::{ArbError, Bps, DexProtocol, PoolState, Result};
use std::time::Instant;

pub const RAYDIUM_CLMM_VERSION: &str = "raydium-clmm-v1.0";
pub const RAYDIUM_CLMM_TICK_ARRAY_SIZE: usize = 60;

#[derive(Debug, Default, Clone)]
pub struct RaydiumClmmQuoteEngine;

impl RaydiumClmmQuoteEngine {
    pub fn new() -> Self {
        Self
    }
}

impl QuoteEngine for RaydiumClmmQuoteEngine {
    fn protocol_id(&self) -> DexProtocol {
        DexProtocol::RaydiumClmm
    }

    fn quote_exact_in(&self, state: &PoolState, request: &QuoteRequest) -> Result<QuoteResult> {
        request.validate(state)?;
        let start_time = Instant::now();

        if !state.is_active {
            return Err(ArbError::PoolDisabled(format!(
                "Raydium CLMM pool {} is disabled",
                state.pool_id.0
            )));
        }

        let clmm_state = state.clmm.as_ref().ok_or_else(|| {
            ArbError::IncompleteState(format!("Missing CLMM state for pool {}", state.pool_id.0))
        })?;

        let a_to_b = request.input_mint == state.base_mint;
        if !a_to_b && request.input_mint != state.quote_mint {
            return Err(ArbError::InvalidToken(format!(
                "Input mint {} does not match pool mints",
                request.input_mint.0
            )));
        }

        // 1. Inbound Token-2022 transfer fee deduction
        let (input_net_into_pool, input_transfer_fee) = request.input_after_transfer_fee()?;

        if input_net_into_pool == 0 {
            return Err(ArbError::ZeroAmount);
        }

        if clmm_state.liquidity == 0 || clmm_state.sqrt_price == 0 {
            return Err(ArbError::InsufficientLiquidity(
                "Raydium CLMM zero liquidity/price".into(),
            ));
        }

        // 2. Prepare Tick Arrays
        let arrays: Vec<TickArrayFacade> = clmm_state
            .tick_arrays
            .iter()
            .map(|ta| TickArrayFacade {
                start_tick_index: ta.start_tick_index,
                ticks: ta
                    .ticks
                    .iter()
                    .map(|t| TickFacade {
                        tick_index: t.tick_index,
                        initialized: t.initialized,
                        liquidity_net: t.liquidity_net,
                        liquidity_gross: 0,
                        limit_order_unfilled: t.limit_order_unfilled,
                    })
                    .collect(),
            })
            .collect();

        // The decoded raw `fee_rate` is in 1e-6 (ppm) units — the same unit the
        // Raydium program uses. Never re-derive it from whole basis points and
        // never default it: a wrong fee is a wrong quote.
        if clmm_state.fee_rate == 0 {
            return Err(ArbError::IncompleteState(format!(
                "Raydium CLMM {} has no decoded fee_rate",
                state.pool_id.0
            )));
        }

        // 3. Compute CLMM swap step traversal with the Raydium tick-math regime.
        // Raydium's `tick_math` rounds differently from Orca's, so sharing the
        // Orca table would drift on every tick boundary.
        let swap_quote = compute_swap_with_ticks_per_array_and_math(
            clmm_state.sqrt_price,
            clmm_state.tick_current,
            clmm_state.liquidity,
            clmm_state.tick_spacing,
            clmm_state.fee_rate,
            input_net_into_pool,
            a_to_b,
            &arrays,
            RAYDIUM_CLMM_TICK_ARRAY_SIZE,
            ClmmTickMath::Raydium,
        )
        .map_err(|e| ArbError::CalculationError(format!("Raydium CLMM step swap failed: {e:?}")))?;
        let gross_out = swap_quote.amount_out_received;

        if gross_out == 0 {
            return Err(ArbError::InsufficientLiquidity(
                "Raydium CLMM produced 0 output".into(),
            ));
        }

        // 4. Outbound Token-2022 transfer fee deduction
        let (output_net_to_next_hop, output_transfer_fee) =
            request.output_after_transfer_fee(gross_out)?;

        // 5. Slippage calculation
        let min_amount_out = request.minimum_amount_out(output_net_to_next_hop);

        let fee_amount = swap_quote.fee_paid;
        let elapsed = start_time.elapsed().as_micros() as u64;

        Ok(QuoteResult {
            protocol: DexProtocol::RaydiumClmm,
            pool_id: state.pool_id,
            amount_in_raw: request.amount,
            amount_out_raw: output_net_to_next_hop,
            minimum_amount_out_raw: min_amount_out,
            fees_breakdown: FeesBreakdown {
                lp_fee: fee_amount,
                protocol_fee: 0,
                fund_fee: 0,
                creator_fee: 0,
                dynamic_fee: 0,
                total_fee: fee_amount,
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
            calculation_version: RAYDIUM_CLMM_VERSION,
            quote_validity: true,
            computation_time_us: elapsed,
            dlmm_bins_crossed: None,
        })
    }

    fn quote_exact_out(&self, _state: &PoolState, _request: &QuoteRequest) -> Result<QuoteResult> {
        Err(ArbError::UnsupportedOperation(
            "Raydium CLMM quote_exact_out not yet supported".into(),
        ))
    }
}
