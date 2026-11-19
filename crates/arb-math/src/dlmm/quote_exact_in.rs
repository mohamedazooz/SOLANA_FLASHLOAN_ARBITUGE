use super::advance_active_id;
use super::error::{DlmmError, dlmm_fee_on_input};
use super::fee::{
    compute_total_fee_rate, update_references_with_decay, update_volatility_accumulator,
};
use super::price::{calculate_price_impact_bps, get_price_from_id};
use super::swap::{bin_max_amount_out, consume_bin_liquidity};
use super::types::{DlmmBin, DlmmPoolState, DlmmSwapQuote};

/// Execute a multi-bin quote over a mutable route-local pool view.
///
/// This function advances the supplied bins so repeated use of the same pool
/// inside one simulated route observes the preceding hop's state.
pub fn quote_dlmm_multi_bin(
    pool: &DlmmPoolState,
    bins: &mut [DlmmBin],
    amount_in: u64,
    swap_for_y: bool,
    current_timestamp: Option<i64>,
) -> Result<DlmmSwapQuote, DlmmError> {
    if amount_in == 0 {
        return Err(DlmmError::ZeroInput);
    }
    if pool.status != 0 {
        return Err(DlmmError::PoolDisabled);
    }

    let mut v_params = pool.v_parameters;
    if let Some(ts) = current_timestamp {
        update_references_with_decay(&mut v_params, &pool.parameters, pool.active_id, ts)?;
    }

    let mut remaining_in = amount_in;
    let mut total_out = 0u64;
    let mut total_fee = 0u64;
    let mut total_protocol_fee = 0u64;
    let mut current_bin_id = pool.active_id;
    let mut bins_crossed = 0u32;

    let start_price = get_price_from_id(pool.active_id, pool.bin_step)?;
    let fee_on_input = dlmm_fee_on_input(pool.parameters.collect_fee_mode, swap_for_y)?;

    while remaining_in > 0 {
        // Find bin in provided slice matching current_bin_id
        let bin_index = bins
            .iter()
            .position(|b| b.id == current_bin_id)
            .ok_or(DlmmError::BinNotFound(current_bin_id))?;
        let bin = &mut bins[bin_index];

        // The official engine only touches the volatility accumulator for bins
        // that can actually deliver output (`max_out > 0`).
        let max_out = bin_max_amount_out(bin, swap_for_y, pool.support_limit_order);

        if max_out > 0 {
            update_volatility_accumulator(&mut v_params, &pool.parameters, current_bin_id)?;

            let fee_rate = compute_total_fee_rate(
                &pool.parameters,
                pool.bin_step,
                v_params.volatility_accumulator,
            )?;

            let price_q64 = if bin.price_q64 != 0 {
                bin.price_q64
            } else {
                let p = get_price_from_id(bin.id, pool.bin_step)?;
                bin.price_q64 = p;
                p
            };

            let step = consume_bin_liquidity(
                bin,
                remaining_in,
                price_q64,
                swap_for_y,
                fee_rate,
                pool.parameters.protocol_share,
                fee_on_input,
                pool.support_limit_order,
            )?;

            remaining_in = remaining_in
                .checked_sub(step.amount_in_consumed)
                .ok_or(DlmmError::MathOverflow)?;
            total_out = total_out
                .checked_add(step.amount_out)
                .ok_or(DlmmError::MathOverflow)?;
            total_fee = total_fee
                .checked_add(step.fee_amount)
                .ok_or(DlmmError::MathOverflow)?;
            total_protocol_fee = total_protocol_fee
                .checked_add(step.protocol_fee)
                .ok_or(DlmmError::MathOverflow)?;
        }

        if remaining_in > 0 {
            current_bin_id = advance_active_id(current_bin_id, swap_for_y)?;
            bins_crossed = bins_crossed.saturating_add(1);
        }
    }

    // Price impact calculation in basis points
    let price_impact_bps =
        calculate_price_impact_bps(amount_in, total_out, start_price, swap_for_y);

    Ok(DlmmSwapQuote {
        amount_in,
        amount_out: total_out,
        fee_amount: total_fee,
        protocol_fee: total_protocol_fee,
        end_active_id: current_bin_id,
        end_volatility_accumulator: v_params.volatility_accumulator,
        bins_crossed,
        price_impact_bps,
    })
}

/// Non-mutating view wrapper over `quote_dlmm_multi_bin`. This clones only the
/// caller-supplied route-local bin window, never the global market cache.
pub fn quote_dlmm_view(
    pool: &DlmmPoolState,
    bins: &[DlmmBin],
    amount_in: u64,
    swap_for_y: bool,
    current_timestamp: Option<i64>,
) -> Result<DlmmSwapQuote, DlmmError> {
    let mut route_bins = bins.to_vec();
    quote_dlmm_multi_bin(
        pool,
        &mut route_bins,
        amount_in,
        swap_for_y,
        current_timestamp,
    )
}
