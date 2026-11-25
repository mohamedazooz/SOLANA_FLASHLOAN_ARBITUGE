//! Meteora DLMM cross-pool routing through the canonical DEX quote engine.
//!
//! This module performs route composition only. Pool/bin decoding, bitmap
//! verification, Token-2022 checks and the authoritative quote are delegated
//! to `arb-dex::MeteoraDlmmQuoteEngine` so cross-pool and single-pool paths
//! cannot drift into separate DLMM implementations.

use crate::meteora_pair_discovery::MeteoraDlmmEligiblePair;
use arb_dex::engines::MeteoraDlmmQuoteEngine;
use arb_dex::quote_engine::{QuoteEngine, QuoteRequest, QuoteResult};
use arb_math::dlmm::{DlmmBin as ArbDlmmBin, DlmmPoolState};
use arb_types::{MintId, PoolState, TokenProgramKind};
use serde::{Deserialize, Serialize};

/// Result of evaluating a two-hop cross-pool DLMM swap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TwoHopDlmmQuote {
    pub initial_amount_in: u64,
    pub hop1_amount_out: u64,
    pub hop1_minimum_amount_out: u64,
    pub hop2_final_amount_out: u64,
    pub hop2_minimum_amount_out: u64,
    /// Expected gross edge, retained for telemetry.
    pub gross_profit: i64,
    pub spread_bps: u64,
    /// Conservative minimum-output edge used for route acceptance and sizing.
    pub minimum_gross_profit: i64,
    pub minimum_spread_bps: u64,
    pub hop1_bins_traversed: u32,
    pub hop2_bins_traversed: u32,
    pub hop1_fee: u64,
    pub hop1_fee_mint: MintId,
    pub hop2_fee: u64,
    pub hop2_fee_mint: MintId,
    pub hop1_swap_for_y: bool,
    pub hop2_swap_for_y: bool,
}

/// Canonical DLMM cross-pool route composer.
pub struct MeteoraDlmmCrossPoolQuoteEngine;

impl MeteoraDlmmCrossPoolQuoteEngine {
    /// Compatibility view for diagnostics/tests. Canonical pool/bin conversion
    /// and hydrated-account validation live in arb-dex only.
    pub fn build_dlmm_pool_and_bins(pool: &PoolState) -> Option<(DlmmPoolState, Vec<ArbDlmmBin>)> {
        if pool.protocol != arb_types::DexProtocol::MeteoraDlmm
            || pool.has_risky_token
            || pool.transfer_fee_bps != 0
            || pool.token_programs != [TokenProgramKind::SplToken; 2]
        {
            return None;
        }
        MeteoraDlmmQuoteEngine::validate_pool_mint_snapshot(pool).ok()?;
        let dlmm = pool.dlmm.as_ref()?;
        MeteoraDlmmQuoteEngine::validate_bin_array_snapshot(
            pool.pool_id,
            dlmm,
            pool.execution_accounts.as_ref(),
        )
        .ok()?;
        MeteoraDlmmQuoteEngine::build_dlmm_math_state(pool).ok()
    }

    /// Compatibility quote using the shared default hop slippage.
    pub fn quote_single_hop(
        pool: &PoolState,
        amount_in: u64,
        from_mint: MintId,
        to_mint: MintId,
    ) -> Option<(QuoteResult, bool)> {
        Self::quote_single_hop_with_slippage(
            pool,
            amount_in,
            from_mint,
            to_mint,
            arb_constants::DEFAULT_HOP_SLIPPAGE_BPS as u16,
        )
    }

    /// Quote one DLMM hop with the exact per-hop slippage policy used by the
    /// transaction builder. Token-2022 is rejected until complete mint fee
    /// schedules and hook accounts are carried through this route API.
    pub fn quote_single_hop_with_slippage(
        pool: &PoolState,
        amount_in: u64,
        from_mint: MintId,
        to_mint: MintId,
        slippage_bps: u16,
    ) -> Option<(QuoteResult, bool)> {
        if slippage_bps > 10_000
            || amount_in == 0
            || pool.protocol != arb_types::DexProtocol::MeteoraDlmm
        {
            return None;
        }
        let swap_for_y = if pool.base_mint == from_mint && pool.quote_mint == to_mint {
            true
        } else if pool.quote_mint == from_mint && pool.base_mint == to_mint {
            false
        } else {
            return None;
        };
        if pool.has_risky_token
            || pool.transfer_fee_bps != 0
            || pool.token_programs != [TokenProgramKind::SplToken; 2]
        {
            return None;
        }
        MeteoraDlmmQuoteEngine::validate_pool_mint_snapshot(pool).ok()?;

        let request = QuoteRequest {
            input_mint: from_mint,
            output_mint: to_mint,
            amount: amount_in,
            slippage_bps,
            input_transfer_fee_config: None,
            output_transfer_fee_config: None,
        };
        let quote = MeteoraDlmmQuoteEngine::new()
            .quote_exact_in(pool, &request)
            .ok()?;
        if !quote.quote_validity
            || quote.amount_in_raw != amount_in
            || quote.unfilled_input_raw != 0
            || quote.amount_out_raw == 0
            || quote.dlmm_bins_crossed.is_none()
        {
            return None;
        }
        Some((quote, swap_for_y))
    }

    /// Quote two hops: `start_mint -> mid_mint` on pool 1, then back on pool 2.
    pub fn quote_two_hops(
        pool_1: &PoolState,
        pool_2: &PoolState,
        amount_in: u64,
        start_mint: MintId,
        mid_mint: MintId,
    ) -> Option<TwoHopDlmmQuote> {
        Self::quote_two_hops_with_slippage(
            pool_1,
            pool_2,
            amount_in,
            start_mint,
            mid_mint,
            arb_constants::DEFAULT_HOP_SLIPPAGE_BPS as u16,
        )
    }

    pub fn quote_two_hops_with_slippage(
        pool_1: &PoolState,
        pool_2: &PoolState,
        amount_in: u64,
        start_mint: MintId,
        mid_mint: MintId,
        slippage_bps: u16,
    ) -> Option<TwoHopDlmmQuote> {
        let (hop1, swap_for_y1) = Self::quote_single_hop_with_slippage(
            pool_1,
            amount_in,
            start_mint,
            mid_mint,
            slippage_bps,
        )?;
        let (hop2_expected, swap_for_y2) = Self::quote_single_hop_with_slippage(
            pool_2,
            hop1.amount_out_raw,
            mid_mint,
            start_mint,
            slippage_bps,
        )?;
        let (hop2_conservative, _) = Self::quote_single_hop_with_slippage(
            pool_2,
            hop1.minimum_amount_out_raw,
            mid_mint,
            start_mint,
            slippage_bps,
        )?;
        let hop1_fee_mint = MeteoraDlmmQuoteEngine::fee_mint_for_quote(pool_1, start_mint).ok()?;
        let hop2_fee_mint = MeteoraDlmmQuoteEngine::fee_mint_for_quote(pool_2, mid_mint).ok()?;

        let gross_profit =
            i64::try_from(i128::from(hop2_expected.amount_out_raw) - i128::from(amount_in)).ok()?;
        let minimum_gross_profit = i64::try_from(
            i128::from(hop2_conservative.minimum_amount_out_raw) - i128::from(amount_in),
        )
        .ok()?;
        let spread = |profit: i64| -> Option<u64> {
            if profit <= 0 {
                return Some(0);
            }
            u64::try_from(
                u128::try_from(profit)
                    .ok()?
                    .checked_mul(10_000)?
                    .checked_div(u128::from(amount_in.max(1)))?,
            )
            .ok()
        };
        let spread_bps = spread(gross_profit)?;
        let minimum_spread_bps = spread(minimum_gross_profit)?;

        Some(TwoHopDlmmQuote {
            initial_amount_in: amount_in,
            hop1_amount_out: hop1.amount_out_raw,
            hop1_minimum_amount_out: hop1.minimum_amount_out_raw,
            hop2_final_amount_out: hop2_expected.amount_out_raw,
            hop2_minimum_amount_out: hop2_conservative.minimum_amount_out_raw,
            gross_profit,
            spread_bps,
            minimum_gross_profit,
            minimum_spread_bps,
            hop1_bins_traversed: hop1.dlmm_bins_crossed?,
            hop2_bins_traversed: hop2_expected.dlmm_bins_crossed?,
            hop1_fee: hop1.fees_breakdown.total_fee,
            hop1_fee_mint,
            hop2_fee: hop2_expected.fees_breakdown.total_fee,
            hop2_fee_mint,
            hop1_swap_for_y: swap_for_y1,
            hop2_swap_for_y: swap_for_y2,
        })
    }

    /// Direction A: Pool A quote-to-base, then Pool B base-to-quote.
    pub fn quote_direction_a(
        pair: &MeteoraDlmmEligiblePair,
        amount_in: u64,
    ) -> Option<TwoHopDlmmQuote> {
        Self::quote_direction_a_with_slippage(
            pair,
            amount_in,
            arb_constants::DEFAULT_HOP_SLIPPAGE_BPS as u16,
        )
    }

    pub fn quote_direction_a_with_slippage(
        pair: &MeteoraDlmmEligiblePair,
        amount_in: u64,
        slippage_bps: u16,
    ) -> Option<TwoHopDlmmQuote> {
        Self::quote_two_hops_with_slippage(
            &pair.pool_a_state,
            &pair.pool_b_state,
            amount_in,
            pair.canonical_mint_y,
            pair.canonical_mint_x,
            slippage_bps,
        )
    }

    /// Direction B: Pool B quote-to-base, then Pool A base-to-quote.
    pub fn quote_direction_b(
        pair: &MeteoraDlmmEligiblePair,
        amount_in: u64,
    ) -> Option<TwoHopDlmmQuote> {
        Self::quote_direction_b_with_slippage(
            pair,
            amount_in,
            arb_constants::DEFAULT_HOP_SLIPPAGE_BPS as u16,
        )
    }

    pub fn quote_direction_b_with_slippage(
        pair: &MeteoraDlmmEligiblePair,
        amount_in: u64,
        slippage_bps: u16,
    ) -> Option<TwoHopDlmmQuote> {
        Self::quote_two_hops_with_slippage(
            &pair.pool_b_state,
            &pair.pool_a_state,
            amount_in,
            pair.canonical_mint_y,
            pair.canonical_mint_x,
            slippage_bps,
        )
    }
}
