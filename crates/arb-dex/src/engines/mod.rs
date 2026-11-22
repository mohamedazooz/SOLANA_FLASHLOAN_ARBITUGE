//! Protocol Quote Engines Module.

pub mod meteora_dlmm;
pub mod orca_whirlpool;
pub mod pump_bonding_curve;
pub mod pump_swap;
pub mod raydium_clmm;
pub mod raydium_cpmm;
pub mod raydium_v4;

pub use meteora_dlmm::MeteoraDlmmQuoteEngine;
pub use orca_whirlpool::OrcaWhirlpoolQuoteEngine;
pub use pump_bonding_curve::PumpBondingCurveQuoteEngine;
pub use pump_swap::PumpSwapQuoteEngine;
pub use raydium_clmm::RaydiumClmmQuoteEngine;
pub use raydium_cpmm::RaydiumCpmmQuoteEngine;
pub use raydium_v4::RaydiumAmmV4QuoteEngine;

use crate::quote_engine::{FeesBreakdown, QuoteEngine, QuoteRequest, QuoteResult};
use crate::token2022::HopTransferFeeBreakdown;
use arb_types::{ArbError, Bps, DexProtocol, PoolState, Result};
use std::time::Instant;

/// Unified Quote Dispatcher delegating to the appropriate independent mathematical engine.
#[derive(Debug, Default, Clone)]
pub struct UniversalQuoteDispatcher {
    pub dlmm: MeteoraDlmmQuoteEngine,
    pub cpmm: RaydiumCpmmQuoteEngine,
    pub raydium_v4: RaydiumAmmV4QuoteEngine,
    pub raydium_clmm: RaydiumClmmQuoteEngine,
    pub orca: OrcaWhirlpoolQuoteEngine,
    pub pump_swap: PumpSwapQuoteEngine,
    pub pump_curve: PumpBondingCurveQuoteEngine,
}

impl UniversalQuoteDispatcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Quote exact-in routing to the correct independent protocol engine.
    pub fn quote_exact_in(&self, state: &PoolState, request: &QuoteRequest) -> Result<QuoteResult> {
        match state.protocol {
            DexProtocol::MeteoraDlmm => self.dlmm.quote_exact_in(state, request),
            DexProtocol::RaydiumCpmm => self.cpmm.quote_exact_in(state, request),
            DexProtocol::RaydiumV4 => self.raydium_v4.quote_exact_in(state, request),
            DexProtocol::RaydiumClmm => self.raydium_clmm.quote_exact_in(state, request),
            DexProtocol::OrcaWhirlpool => self.orca.quote_exact_in(state, request),
            DexProtocol::PumpFun => {
                if let Some(p) = &state.pump_fun {
                    if p.complete {
                        self.pump_swap.quote_exact_in(state, request)
                    } else {
                        self.pump_curve.quote_exact_in(state, request)
                    }
                } else {
                    self.pump_curve.quote_exact_in(state, request)
                }
            }
            DexProtocol::PumpSwap => self.pump_swap.quote_exact_in(state, request),
            DexProtocol::MeteoraDammV2 => self.quote_meteora_damm_v2_exact_in(state, request),
            DexProtocol::MeteoraDbc => self.quote_meteora_dbc_exact_in(state, request),
            DexProtocol::MeteoraDlmmPro => self.quote_meteora_dlmm_pro_exact_in(state, request),
            DexProtocol::RaydiumLaunchLab => self.quote_raydium_launchlab_exact_in(state, request),
        }
    }

    /// Quote exact-out routing to the correct independent protocol engine.
    pub fn quote_exact_out(
        &self,
        state: &PoolState,
        request: &QuoteRequest,
    ) -> Result<QuoteResult> {
        match state.protocol {
            DexProtocol::MeteoraDlmm => self.dlmm.quote_exact_out(state, request),
            DexProtocol::RaydiumCpmm => self.cpmm.quote_exact_out(state, request),
            DexProtocol::RaydiumV4 => self.raydium_v4.quote_exact_out(state, request),
            DexProtocol::RaydiumClmm => self.raydium_clmm.quote_exact_out(state, request),
            DexProtocol::OrcaWhirlpool => self.orca.quote_exact_out(state, request),
            DexProtocol::PumpFun => self.pump_curve.quote_exact_out(state, request),
            DexProtocol::PumpSwap => self.pump_swap.quote_exact_out(state, request),
            DexProtocol::MeteoraDammV2
            | DexProtocol::MeteoraDbc
            | DexProtocol::MeteoraDlmmPro
            | DexProtocol::RaydiumLaunchLab => self.quote_standard_exact_out(state, request),
        }
    }

    fn quote_meteora_damm_v2_exact_in(
        &self,
        state: &PoolState,
        request: &QuoteRequest,
    ) -> Result<QuoteResult> {
        self.quote_constant_product_exact_in(state, request, DexProtocol::MeteoraDammV2, 25)
    }

    fn quote_meteora_dbc_exact_in(
        &self,
        state: &PoolState,
        request: &QuoteRequest,
    ) -> Result<QuoteResult> {
        self.quote_constant_product_exact_in(state, request, DexProtocol::MeteoraDbc, 25)
    }

    fn quote_meteora_dlmm_pro_exact_in(
        &self,
        state: &PoolState,
        request: &QuoteRequest,
    ) -> Result<QuoteResult> {
        self.quote_constant_product_exact_in(state, request, DexProtocol::MeteoraDlmmPro, 10)
    }

    fn quote_raydium_launchlab_exact_in(
        &self,
        state: &PoolState,
        request: &QuoteRequest,
    ) -> Result<QuoteResult> {
        self.quote_constant_product_exact_in(state, request, DexProtocol::RaydiumLaunchLab, 25)
    }

    fn quote_constant_product_exact_in(
        &self,
        state: &PoolState,
        request: &QuoteRequest,
        protocol: DexProtocol,
        default_fee_bps: u128,
    ) -> Result<QuoteResult> {
        let start_time = Instant::now();
        if !state.is_active {
            return Err(ArbError::PoolDisabled(format!(
                "Pool {} is disabled",
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
                "Pool has 0 reserves".into(),
            ));
        }

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

        let total_fee_bps = if state.fee_numerator > 0 && state.fee_denominator > 0 {
            (state.fee_numerator as u128 * 10_000) / state.fee_denominator as u128
        } else {
            default_fee_bps
        };

        let num_fee = (input_net_into_pool as u128).saturating_mul(total_fee_bps);
        let fee = num_fee.div_ceil(10_000);
        let fee_u64 = u64::try_from(fee).unwrap_or(input_net_into_pool);

        let input_after_fee = input_net_into_pool.saturating_sub(fee_u64);
        if input_after_fee == 0 {
            return Err(ArbError::InsufficientLiquidity(
                "Input exhausted by swap fee".into(),
            ));
        }

        let dx_prime = input_after_fee as u128;
        let num_out = r_out.saturating_mul(dx_prime);
        let denom_out = r_in.saturating_add(dx_prime);
        let gross_out = (num_out / denom_out) as u64;

        if gross_out == 0 {
            return Err(ArbError::InsufficientLiquidity("Returned 0 output".into()));
        }

        let (output_net_to_next_hop, output_transfer_fee) =
            if let Some(config) = request.output_transfer_fee_config {
                let fee = config.calculate_fee(gross_out);
                (gross_out.saturating_sub(fee), fee)
            } else {
                (gross_out, 0)
            };

        let min_amount_out = output_net_to_next_hop.saturating_sub(
            (output_net_to_next_hop as u128 * request.slippage_bps as u128 / 10_000) as u64,
        );

        let elapsed = start_time.elapsed().as_micros() as u64;

        Ok(QuoteResult {
            protocol,
            pool_id: state.pool_id,
            amount_in_raw: request.amount,
            amount_out_raw: output_net_to_next_hop,
            minimum_amount_out_raw: min_amount_out,
            fees_breakdown: FeesBreakdown {
                lp_fee: fee_u64,
                protocol_fee: 0,
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
            calculation_version: "universal-dispatcher-v1.0",
            quote_validity: true,
            computation_time_us: elapsed,
        })
    }

    fn quote_standard_exact_out(
        &self,
        state: &PoolState,
        request: &QuoteRequest,
    ) -> Result<QuoteResult> {
        let start_time = Instant::now();
        if !state.is_active {
            return Err(ArbError::PoolDisabled(format!(
                "Pool {} is disabled",
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

        let target_out = request.amount as u128;
        if target_out == 0 {
            return Err(ArbError::ZeroAmount);
        }
        if target_out >= r_out {
            return Err(ArbError::InsufficientLiquidity(
                "Target output exceeds reserve".into(),
            ));
        }

        let fee_bps = if state.fee_numerator > 0 && state.fee_denominator > 0 {
            (state.fee_numerator as u128 * 10_000) / state.fee_denominator as u128
        } else {
            25u128
        };

        let num = r_in.saturating_mul(target_out);
        let denom = r_out.saturating_sub(target_out);
        let dx_net = num.div_ceil(denom);

        let fee_factor = 10_000u128.saturating_sub(fee_bps);
        let gross_in = dx_net.saturating_mul(10_000).div_ceil(fee_factor) as u64;
        let fee_u64 = gross_in.saturating_sub(dx_net as u64);

        let elapsed = start_time.elapsed().as_micros() as u64;

        Ok(QuoteResult {
            protocol: state.protocol,
            pool_id: state.pool_id,
            amount_in_raw: gross_in,
            amount_out_raw: request.amount,
            minimum_amount_out_raw: request.amount,
            fees_breakdown: FeesBreakdown {
                lp_fee: fee_u64,
                protocol_fee: 0,
                fund_fee: 0,
                creator_fee: 0,
                dynamic_fee: 0,
                total_fee: fee_u64,
            },
            transfer_fee_breakdown: HopTransferFeeBreakdown {
                input_gross: gross_in,
                input_transfer_fee: 0,
                input_net_into_pool: gross_in,
                output_gross_from_pool: request.amount,
                output_transfer_fee: 0,
                output_net_to_next_hop: request.amount,
            },
            price_impact: Bps(0),
            unfilled_input_raw: 0,
            required_accounts: vec![state.pool_id.0],
            state_slot: state.slot.0,
            state_hash: format!("{}_{}", state.slot.0, state.write_version),
            calculation_version: "universal-dispatcher-v1.0",
            quote_validity: true,
            computation_time_us: elapsed,
        })
    }
}
