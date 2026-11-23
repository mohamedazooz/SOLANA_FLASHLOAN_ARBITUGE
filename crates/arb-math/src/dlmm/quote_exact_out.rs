use super::error::{DlmmError, dlmm_fee_on_input};
use super::fee::{
    compute_fee_to_add, compute_total_fee_rate, update_references_with_decay,
    update_volatility_accumulator,
};
use super::price::{advance_active_id, calculate_price_impact_bps, get_price_from_id};
use super::swap::{
    bin_max_amount_out, consume_bin_liquidity, input_for_gross_out,
    limit_order_amounts_by_direction,
};
use super::types::{DlmmBin, DlmmPoolState, DlmmSwapQuote, DlmmSwapStepResult};

/// Fee-excluded input required to deliver `included_fee_amount_out` of gross
/// output from this bin, filling the MM reserve layer first, then processed
/// limit orders, then open limit orders (`Rounding::Up` per layer). Mirrors
/// the official SDK `get_excluded_fee_amount_in`.
fn excluded_fee_amount_in_for_out(
    bin: &DlmmBin,
    swap_for_y: bool,
    included_fee_amount_out: u64,
    price_q64: u128,
) -> Result<u64, DlmmError> {
    let mm_amount = if swap_for_y {
        bin.amount_y
    } else {
        bin.amount_x
    };

    let (open_order_amount, processed_order_remaining_amount) =
        limit_order_amounts_by_direction(bin, swap_for_y);

    let mut remaining_amount_out = included_fee_amount_out;
    let mut total_amount_in: u64 = 0;

    let mm_take = remaining_amount_out.min(mm_amount);
    total_amount_in = total_amount_in
        .checked_add(input_for_gross_out(mm_take, price_q64, swap_for_y)?)
        .ok_or(DlmmError::MathOverflow)?;
    remaining_amount_out = remaining_amount_out
        .checked_sub(mm_take)
        .ok_or(DlmmError::MathOverflow)?;

    if remaining_amount_out > 0 {
        let processed_take = remaining_amount_out.min(processed_order_remaining_amount);
        total_amount_in = total_amount_in
            .checked_add(input_for_gross_out(processed_take, price_q64, swap_for_y)?)
            .ok_or(DlmmError::MathOverflow)?;
        remaining_amount_out = remaining_amount_out
            .checked_sub(processed_take)
            .ok_or(DlmmError::MathOverflow)?;

        if remaining_amount_out > 0 {
            let open_take = remaining_amount_out.min(open_order_amount);
            total_amount_in = total_amount_in
                .checked_add(input_for_gross_out(open_take, price_q64, swap_for_y)?)
                .ok_or(DlmmError::MathOverflow)?;
        }
    }

    Ok(total_amount_in)
}

/// Per-bin exact-out quote over all liquidity layers. Mirrors the official SDK
/// `swap_exact_out_quote_at_bin`: when the requested (fee-included) output
/// meets or exceeds the bin's maximum deliverable output the bin is drained
/// via an unbounded exact-in quote; otherwise the required fee-excluded input
/// is computed layer-by-layer, re-included, and re-quoted as an exact-in fill.
/// Any rounding surplus above one unit is assigned to the protocol fee.
#[allow(clippy::too_many_arguments)]
fn swap_exact_out_at_bin(
    bin: &mut DlmmBin,
    out_amount: u64,
    price_q64: u128,
    swap_for_y: bool,
    fee_rate: u128,
    protocol_share_bps: u16,
    fee_on_input: bool,
    support_limit_order: bool,
) -> Result<DlmmSwapStepResult, DlmmError> {
    let mut included_fee_amount_out = out_amount;

    if !fee_on_input {
        let fee = compute_fee_to_add(out_amount, fee_rate)?;
        included_fee_amount_out = out_amount.checked_add(fee).ok_or(DlmmError::MathOverflow)?;
    }

    let max_amount_out = bin_max_amount_out(bin, swap_for_y, support_limit_order);

    if included_fee_amount_out >= max_amount_out {
        return consume_bin_liquidity(
            bin,
            u64::MAX,
            price_q64,
            swap_for_y,
            fee_rate,
            protocol_share_bps,
            fee_on_input,
            support_limit_order,
        );
    }

    let excluded_fee_amount_in =
        excluded_fee_amount_in_for_out(bin, swap_for_y, included_fee_amount_out, price_q64)?;

    let included_fee_amount_in = if fee_on_input {
        let fee = compute_fee_to_add(excluded_fee_amount_in, fee_rate)?;
        excluded_fee_amount_in
            .checked_add(fee)
            .ok_or(DlmmError::MathOverflow)?
    } else {
        excluded_fee_amount_in
    };

    let mut step = consume_bin_liquidity(
        bin,
        included_fee_amount_in,
        price_q64,
        swap_for_y,
        fee_rate,
        protocol_share_bps,
        fee_on_input,
        support_limit_order,
    )?;

    // Delta between the quoted output and the requested output goes to the
    // protocol (rounding dust), exactly as the official SDK does. The dust is
    // collected from the output side, so the TOTAL trading fee rises by the
    // same amount (`fee_amount` must stay the sum of LP + protocol shares,
    // matching the SDK's `fee + protocolFee` accounting).
    if step.amount_out > out_amount {
        let delta = step
            .amount_out
            .checked_sub(out_amount)
            .ok_or(DlmmError::MathOverflow)?;
        if delta > 1 {
            step.protocol_fee = step
                .protocol_fee
                .checked_add(delta)
                .ok_or(DlmmError::MathOverflow)?;
            step.fee_amount = step
                .fee_amount
                .checked_add(delta)
                .ok_or(DlmmError::MathOverflow)?;
        }
    }

    step.amount_out = out_amount;

    Ok(step)
}

/// Execute an Exact-Out quote: compute the exact required input amount and fees to receive `amount_out`.
pub fn quote_dlmm_exact_out(
    pool: &DlmmPoolState,
    bins: &[DlmmBin],
    amount_out: u64,
    swap_for_y: bool,
    current_timestamp: Option<i64>,
) -> Result<DlmmSwapQuote, DlmmError> {
    if amount_out == 0 {
        return Err(DlmmError::ZeroInput);
    }
    if pool.status != 0 {
        return Err(DlmmError::PoolDisabled);
    }

    let mut v_params = pool.v_parameters;
    if let Some(ts) = current_timestamp {
        update_references_with_decay(&mut v_params, &pool.parameters, pool.active_id, ts)?;
    }

    let mut remaining_out = amount_out;
    let mut total_in = 0u64;
    let mut total_fee = 0u64;
    let mut total_protocol_fee = 0u64;
    let mut current_bin_id = pool.active_id;
    let mut bins_crossed = 0u32;
    let mut cloned_bins = bins.to_vec();

    let start_price = get_price_from_id(pool.active_id, pool.bin_step)?;
    let fee_on_input = dlmm_fee_on_input(pool.parameters.collect_fee_mode, swap_for_y)?;

    while remaining_out > 0 {
        let bin_index = cloned_bins
            .iter()
            .position(|b| b.id == current_bin_id)
            .ok_or(DlmmError::BinNotFound(current_bin_id))?;
        let bin = &mut cloned_bins[bin_index];

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

            let step = swap_exact_out_at_bin(
                bin,
                remaining_out,
                price_q64,
                swap_for_y,
                fee_rate,
                pool.parameters.protocol_share,
                fee_on_input,
                pool.support_limit_order,
            )?;

            if step.amount_out > 0 {
                remaining_out = remaining_out
                    .checked_sub(step.amount_out)
                    .ok_or(DlmmError::MathOverflow)?;
                total_in = total_in
                    .checked_add(step.amount_in_consumed)
                    .ok_or(DlmmError::MathOverflow)?;
                total_fee = total_fee
                    .checked_add(step.fee_amount)
                    .ok_or(DlmmError::MathOverflow)?;
                total_protocol_fee = total_protocol_fee
                    .checked_add(step.protocol_fee)
                    .ok_or(DlmmError::MathOverflow)?;
            }
        }

        if remaining_out > 0 {
            bins_crossed = bins_crossed.checked_add(1).ok_or(DlmmError::MathOverflow)?;
            current_bin_id = advance_active_id(current_bin_id, swap_for_y)?;
        }
    }

    let price_impact_bps =
        calculate_price_impact_bps(total_in, amount_out, start_price, swap_for_y);

    Ok(DlmmSwapQuote {
        amount_in: total_in,
        amount_out,
        fee_amount: total_fee,
        protocol_fee: total_protocol_fee,
        end_active_id: current_bin_id,
        end_volatility_accumulator: v_params.volatility_accumulator,
        bins_crossed,
        price_impact_bps,
    })
}
