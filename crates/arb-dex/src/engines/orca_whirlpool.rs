//! Orca Whirlpools Independent Quote Engine.
//! Program ID: `whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc`
//!
//! Enforces:
//! - Q64.64 sqrt price representation with bitwise binary exponentiation.
//! - Orca Whirlpool 88-tick array size.
//! - Exact tick crossing and liquidity updates.
//! - Fee rate in hundredths of a bip (`FEE_RATE_DENOMINATOR = 1_000_000`).
//! - Exact TickArray PDA derivations for the 3 consecutive tick arrays.

use crate::quote_engine::{FeesBreakdown, QuoteEngine, QuoteRequest, QuoteResult};
use crate::token2022::HopTransferFeeBreakdown;
use arb_constants::ORCA_WHIRLPOOL_PROGRAM_ID;
use arb_math::clmm::{TickArrayFacade, TickFacade, quote_clmm};
use arb_types::{ArbError, Bps, DexProtocol, PoolState, Result};
use solana_pubkey::Pubkey;
use std::str::FromStr;
use std::time::Instant;

pub const ORCA_WHIRLPOOL_VERSION: &str = "orca-whirlpool-v1.0";
pub const ORCA_TICK_ARRAY_SIZE: i32 = 88;

#[derive(Debug, Default, Clone)]
pub struct OrcaWhirlpoolQuoteEngine;

impl OrcaWhirlpoolQuoteEngine {
    pub fn new() -> Self {
        Self
    }

    /// Derive Whirlpool TickArray PDA for a given start tick index.
    pub fn derive_tick_array_pda(pool_id: &Pubkey, start_tick: i32) -> Pubkey {
        let program_id = Pubkey::from_str(ORCA_WHIRLPOOL_PROGRAM_ID).unwrap();
        let start_tick_str = start_tick.to_string();
        let seeds: &[&[u8]] = &[b"tick_array", pool_id.as_ref(), start_tick_str.as_bytes()];
        let (pda, _) = Pubkey::find_program_address(seeds, &program_id);
        pda
    }

    /// Get the 3 consecutive TickArray PDAs needed for the swap instruction.
    pub fn get_tick_arrays_for_swap(
        pool_id: &Pubkey,
        tick_current: i32,
        tick_spacing: u16,
        a_to_b: bool,
    ) -> (Pubkey, Pubkey, Pubkey) {
        let spacing = (tick_spacing as i32).max(1);
        let ticks_in_array = ORCA_TICK_ARRAY_SIZE * spacing;
        let start_tick = tick_current.div_euclid(ticks_in_array) * ticks_in_array;

        let ta0 = Self::derive_tick_array_pda(pool_id, start_tick);
        if a_to_b {
            let ta1 = Self::derive_tick_array_pda(pool_id, start_tick - ticks_in_array);
            let ta2 = Self::derive_tick_array_pda(pool_id, start_tick - (2 * ticks_in_array));
            (ta0, ta1, ta2)
        } else {
            let ta1 = Self::derive_tick_array_pda(pool_id, start_tick + ticks_in_array);
            let ta2 = Self::derive_tick_array_pda(pool_id, start_tick + (2 * ticks_in_array));
            (ta0, ta1, ta2)
        }
    }
}

impl QuoteEngine for OrcaWhirlpoolQuoteEngine {
    fn protocol_id(&self) -> DexProtocol {
        DexProtocol::OrcaWhirlpool
    }

    fn quote_exact_in(&self, state: &PoolState, request: &QuoteRequest) -> Result<QuoteResult> {
        let start_time = Instant::now();

        if !state.is_active {
            return Err(ArbError::PoolDisabled(format!(
                "Orca Whirlpool {} is disabled",
                state.pool_id.0
            )));
        }

        let clmm_state = state.clmm.as_ref().ok_or_else(|| {
            ArbError::IncompleteState(format!(
                "Missing CLMM state for Orca Whirlpool {}",
                state.pool_id.0
            ))
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

        if clmm_state.liquidity == 0 || clmm_state.sqrt_price == 0 {
            return Err(ArbError::InsufficientLiquidity(
                "Orca Whirlpool zero liquidity/price".into(),
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
                    })
                    .collect(),
            })
            .collect();

        let fee_rate = if let Some(bps) = state.fee_bps() {
            bps.saturating_mul(100)
        } else {
            3000 // 0.3% default
        };

        // 3. Compute CLMM swap step traversal
        let gross_out = quote_clmm(
            input_net_into_pool,
            clmm_state.sqrt_price,
            clmm_state.tick_current,
            clmm_state.liquidity,
            clmm_state.tick_spacing,
            fee_rate,
            a_to_b,
            &arrays,
        )
        .map_err(|e| {
            ArbError::CalculationError(format!("Orca Whirlpool step swap failed: {e:?}"))
        })?;

        if gross_out == 0 {
            return Err(ArbError::InsufficientLiquidity(
                "Orca Whirlpool produced 0 output".into(),
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

        let fee_amount = (input_net_into_pool as u128 * fee_rate as u128 / 1_000_000) as u64;

        // 6. Required Accounts (3 TickArray PDAs)
        let (ta0, ta1, ta2) = Self::get_tick_arrays_for_swap(
            &state.pool_id.0,
            clmm_state.tick_current,
            clmm_state.tick_spacing,
            a_to_b,
        );
        let required_accounts = vec![state.pool_id.0, ta0, ta1, ta2];

        let elapsed = start_time.elapsed().as_micros() as u64;

        Ok(QuoteResult {
            protocol: DexProtocol::OrcaWhirlpool,
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
            required_accounts,
            state_slot: state.slot.0,
            state_hash: format!("{}_{}", state.slot.0, state.write_version),
            calculation_version: ORCA_WHIRLPOOL_VERSION,
            quote_validity: true,
            computation_time_us: elapsed,
        })
    }

    fn quote_exact_out(&self, _state: &PoolState, _request: &QuoteRequest) -> Result<QuoteResult> {
        Err(ArbError::UnsupportedOperation(
            "Orca Whirlpool quote_exact_out not yet supported".into(),
        ))
    }
}
