//! Meteora DLMM Cross-Pool Quote Engine (`MeteoraDlmmCrossPoolQuoteEngine`).
//!
//! Evaluates executable quotes across both directions of two distinct Meteora DLMM pools
//! using exact multi-bin discrete DLMM math (no constant-product approximation).
//!
//! Handles:
//! - Exact multi-bin traversal and discrete active ID pricing
//! - Base fee, variable fee, and protocol fee accounting
//! - Token transfer fees (Token-2022 extensions)
//! - Directional swap logic: SwapForY (X -> Y) vs SwapForX (Y -> X)
//! - Integer token base units and checked arithmetic

use crate::meteora_pair_discovery::MeteoraDlmmEligiblePair;
use arb_math::dlmm::{
    DlmmBin as ArbDlmmBin, DlmmParameters, DlmmPoolState, DlmmSwapQuote, DlmmVariableParameters,
    quote_dlmm_view,
};
use arb_types::{MintId, PoolState};
use serde::{Deserialize, Serialize};

/// Result of evaluating a two-hop cross-pool DLMM swap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TwoHopDlmmQuote {
    /// Inbound amount to Hop 1 in base units.
    pub initial_amount_in: u64,
    /// Intermediate amount out from Hop 1 delivered as input to Hop 2.
    pub hop1_amount_out: u64,
    /// Final amount out from Hop 2 delivered to payer.
    pub hop2_final_amount_out: u64,
    /// Gross profit in base units: `hop2_final_amount_out - initial_amount_in`.
    pub gross_profit: i64,
    /// Cycle spread in basis points: `(gross_profit * 10_000) / initial_amount_in`.
    pub spread_bps: u64,
    /// Total discrete bins traversed in Hop 1.
    pub hop1_bins_traversed: u32,
    /// Total discrete bins traversed in Hop 2.
    pub hop2_bins_traversed: u32,
    /// Fees incurred on Hop 1.
    pub hop1_fee: u64,
    /// Fees incurred on Hop 2.
    pub hop2_fee: u64,
    /// Whether Hop 1 is X -> Y.
    pub hop1_swap_for_y: bool,
    /// Whether Hop 2 is X -> Y.
    pub hop2_swap_for_y: bool,
}

/// Real DLMM cross-pool quoting engine.
pub struct MeteoraDlmmCrossPoolQuoteEngine;

impl MeteoraDlmmCrossPoolQuoteEngine {
    /// Convert `PoolState` and its DLMM state to `DlmmPoolState` and `Vec<ArbDlmmBin>`.
    pub fn build_dlmm_pool_and_bins(pool: &PoolState) -> Option<(DlmmPoolState, Vec<ArbDlmmBin>)> {
        let dlmm = pool.dlmm.as_ref()?;
        let fp = dlmm.params.as_ref()?;
        // The local math implementation currently models the Spot function type.
        // Unknown/future function types must not be priced with the wrong curve.
        if dlmm.status != 0
            || fp.function_type != 0
            || fp.collect_fee_mode > 1
            || pool.has_risky_token
            || pool.transfer_fee_bps != 0
        {
            return None;
        }

        let pool_state = DlmmPoolState {
            active_id: dlmm.active_bin_id,
            bin_step: dlmm.bin_step_bps,
            status: dlmm.status,
            parameters: DlmmParameters {
                bin_step: dlmm.bin_step_bps,
                base_factor: fp.base_factor,
                filter_period: fp.filter_period,
                decay_period: fp.decay_period,
                reduction_factor: fp.reduction_factor,
                variable_fee_control: fp.variable_fee_control,
                max_volatility_accumulator: fp.max_volatility_accumulator,
                protocol_share: fp.protocol_share,
                base_fee_power_factor: fp.base_fee_power_factor,
                collect_fee_mode: fp.collect_fee_mode,
            },
            v_parameters: DlmmVariableParameters {
                volatility_accumulator: dlmm.volatility_accumulator,
                volatility_reference: dlmm.volatility_reference,
                index_reference: dlmm.index_reference,
                last_update_timestamp: dlmm.last_update_timestamp,
            },
        };

        let bins: Option<Vec<ArbDlmmBin>> = dlmm
            .bins
            .iter()
            .map(|b| {
                Some(ArbDlmmBin {
                    id: b.bin_id,
                    amount_x: u64::try_from(b.amount_x).ok()?,
                    amount_y: u64::try_from(b.amount_y).ok()?,
                    price_q64: 0,
                })
            })
            .collect();

        Some((pool_state, bins?))
    }

    /// Quote a single DLMM hop from `from_mint` to `to_mint`.
    pub fn quote_single_hop(
        pool: &PoolState,
        amount_in: u64,
        from_mint: MintId,
        to_mint: MintId,
    ) -> Option<(u64, DlmmSwapQuote, bool)> {
        if amount_in == 0 {
            return None;
        }

        let swap_for_y = if pool.base_mint == from_mint && pool.quote_mint == to_mint {
            true
        } else if pool.quote_mint == from_mint && pool.base_mint == to_mint {
            false
        } else {
            return None;
        };

        // The PoolState BPS field is not a complete Token-2022 fee schedule.
        // Refuse these mints until epoch selection, max-fee cap and transfer-hook
        // account metadata are all carried by the snapshot.
        if pool.has_risky_token || pool.transfer_fee_bps != 0 {
            return None;
        }

        let (dlmm_pool, bins) = Self::build_dlmm_pool_and_bins(pool)?;
        let quote = quote_dlmm_view(&dlmm_pool, &bins, amount_in, swap_for_y, None).ok()?;
        if quote.amount_out == 0 {
            return None;
        }
        Some((quote.amount_out, quote, swap_for_y))
    }

    /// Quote two hops: `start_mint -> mid_mint` on `pool_1`, then `mid_mint -> start_mint` on `pool_2`.
    pub fn quote_two_hops(
        pool_1: &PoolState,
        pool_2: &PoolState,
        amount_in: u64,
        start_mint: MintId,
        mid_mint: MintId,
    ) -> Option<TwoHopDlmmQuote> {
        let (hop1_out, q1, swap_for_y1) =
            Self::quote_single_hop(pool_1, amount_in, start_mint, mid_mint)?;

        let (hop2_out, q2, swap_for_y2) =
            Self::quote_single_hop(pool_2, hop1_out, mid_mint, start_mint)?;

        let gross_profit = (hop2_out as i64) - (amount_in as i64);
        let spread_bps = if gross_profit > 0 {
            ((gross_profit as u64).saturating_mul(10_000)) / amount_in.max(1)
        } else {
            0
        };

        Some(TwoHopDlmmQuote {
            initial_amount_in: amount_in,
            hop1_amount_out: hop1_out,
            hop2_final_amount_out: hop2_out,
            gross_profit,
            spread_bps,
            hop1_bins_traversed: q1.bins_crossed,
            hop2_bins_traversed: q2.bins_crossed,
            hop1_fee: q1.fee_amount,
            hop2_fee: q2.fee_amount,
            hop1_swap_for_y: swap_for_y1,
            hop2_swap_for_y: swap_for_y2,
        })
    }

    /// Quote Direction A on an eligible pair:
    /// Pool A: Quote -> Base (`canonical_mint_y -> canonical_mint_x`)
    /// Pool B: Base -> Quote (`canonical_mint_x -> canonical_mint_y`)
    pub fn quote_direction_a(
        pair: &MeteoraDlmmEligiblePair,
        amount_in: u64,
    ) -> Option<TwoHopDlmmQuote> {
        Self::quote_two_hops(
            &pair.pool_a_state,
            &pair.pool_b_state,
            amount_in,
            pair.canonical_mint_y,
            pair.canonical_mint_x,
        )
    }

    /// Quote Direction B on an eligible pair:
    /// Pool B: Quote -> Base (`canonical_mint_y -> canonical_mint_x`)
    /// Pool A: Base -> Quote (`canonical_mint_x -> canonical_mint_y`)
    pub fn quote_direction_b(
        pair: &MeteoraDlmmEligiblePair,
        amount_in: u64,
    ) -> Option<TwoHopDlmmQuote> {
        Self::quote_two_hops(
            &pair.pool_b_state,
            &pair.pool_a_state,
            amount_in,
            pair.canonical_mint_y,
            pair.canonical_mint_x,
        )
    }
}
