use super::error::DlmmError;
use super::fee::{compute_fee_from_amount, compute_fee_to_add};
use super::types::{DlmmBin, DlmmSwapStepResult};
use super::{BASIS_POINT_MAX, LIMIT_ORDER_FEE_SHARE, SCALE_OFFSET};
use ruint::aliases::U256;

/// Input required to produce an exact gross output amount from one bin.
pub(super) fn input_for_gross_out(
    gross_out: u64,
    price_q64: u128,
    swap_for_y: bool,
) -> Result<u64, DlmmError> {
    if gross_out == 0 {
        return Ok(0);
    }
    let res = if swap_for_y {
        // X -> Y: net_in = ceil(out * 2^64 / P)
        let num = U256::from(gross_out) << SCALE_OFFSET;
        let denom = U256::from(price_q64);
        if denom.is_zero() {
            return Err(DlmmError::MathOverflow);
        }
        let (quot, rem) = num.div_rem(denom);
        if rem.is_zero() {
            quot
        } else {
            quot.checked_add(U256::from(1))
                .ok_or(DlmmError::MathOverflow)?
        }
    } else {
        // Y -> X: net_in = ceil(out * P / 2^64)
        let prod = U256::from(gross_out) * U256::from(price_q64);
        let denom = U256::from(1) << SCALE_OFFSET;
        let (quot, rem) = prod.div_rem(denom);
        if rem.is_zero() {
            quot
        } else {
            quot.checked_add(U256::from(1))
                .ok_or(DlmmError::MathOverflow)?
        }
    };
    res.try_into().map_err(|_| DlmmError::TypeCastFailed)
}

/// Gross output produced by an exact net input amount, floored at the pool.
fn gross_out_for_input(net_in: u64, price_q64: u128, swap_for_y: bool) -> Result<u64, DlmmError> {
    if net_in == 0 {
        return Ok(0);
    }
    let res = if swap_for_y {
        // X -> Y: out = floor(net_in * P / 2^64)
        (U256::from(net_in) * U256::from(price_q64)) >> SCALE_OFFSET
    } else {
        // Y -> X: out = floor(net_in * 2^64 / P)
        let num = U256::from(net_in) << SCALE_OFFSET;
        let denom = U256::from(price_q64);
        if denom.is_zero() {
            return Err(DlmmError::MathOverflow);
        }
        num / denom
    };
    res.try_into().map_err(|_| DlmmError::TypeCastFailed)
}

/// Liquidity fill result for a single layer within a bin (MM, processed limit
/// orders, or open limit orders). Mirrors the official SDK `FillResult`.
struct LayerFill {
    /// Net (fee-excluded) input consumed by this layer.
    amount_in: u64,
    /// Input not absorbed by this layer.
    amount_left: u64,
    /// Gross output produced by this layer.
    out_amount: u64,
    /// True when the layer's full output capacity was consumed (including the
    /// vacuous zero-capacity case). A bin is drained only when every layer
    /// that received flow was exhausted.
    exhausted: bool,
}

/// Aggregate fill result across all liquidity layers of a bin. Mirrors the
/// official SDK `ExactInFillResult`, extended with per-layer outputs so the
/// caller can commit route-local state per layer.
struct BinFillResult {
    /// Total net (fee-excluded) input consumed across all layers.
    amount_in: u64,
    /// Input not absorbed by the bin (bin fully drained when `> 0`).
    amount_left: u64,
    /// Total gross output produced across all layers.
    out_amount: u64,
    /// Net input consumed by the market-maker (reserve) layer.
    mm_amount_in: u64,
    /// Gross output taken from the market-maker (reserve) layer.
    mm_out: u64,
    /// Gross output taken from the processed-order layer.
    processed_out: u64,
    /// Gross output taken from the open-order layer.
    open_out: u64,
    /// True when the bin's deliverable output liquidity was fully consumed.
    drained: bool,
}

/// Limit-order liquidity fillable in this swap direction.
///
/// Bid-side orders (`limit_order_ask_side = false`) are filled by X -> Y swaps
/// (`swap_for_y = true`); ask-side orders (`limit_order_ask_side = true`) by
/// Y -> X swaps (`swap_for_y = false`). Returns
/// `(open_order_amount, processed_order_remaining_amount)`, or `(0, 0)` when
/// the bin's order side does not match the swap direction. Mirrors the
/// official SDK `Bin::get_limit_order_amounts_by_direction`.
pub(super) fn limit_order_amounts_by_direction(bin: &DlmmBin, swap_for_y: bool) -> (u64, u64) {
    let is_ask_side = bin.limit_order_ask_side;
    if (swap_for_y && !is_ask_side) || (!swap_for_y && is_ask_side) {
        (bin.open_order_amount, bin.processed_order_remaining_amount)
    } else {
        (0, 0)
    }
}

/// Maximum gross output this bin can deliver, including the limit-order
/// layers when the pool supports them. Mirrors the official SDK
/// `Bin::get_max_amount_out_with_limit_orders`.
pub fn bin_max_amount_out(bin: &DlmmBin, swap_for_y: bool, support_limit_order: bool) -> u64 {
    let mm_amount = if swap_for_y {
        bin.amount_y
    } else {
        bin.amount_x
    };
    if !support_limit_order {
        return mm_amount;
    }
    let (open_order, processed_remaining) = limit_order_amounts_by_direction(bin, swap_for_y);
    mm_amount
        .saturating_add(open_order)
        .saturating_add(processed_remaining)
}

/// Fill a single liquidity layer: consume up to `max_amount_out` worth of
/// output for at most `amount` of net input. Mirrors the official SDK
/// `calculate_exact_in_fill_amount` (`Rounding::Up` for the full-layer input,
/// `Rounding::Down` for partial-fill output).
fn fill_liquidity_layer(
    amount: u64,
    max_amount_out: u64,
    price_q64: u128,
    swap_for_y: bool,
) -> Result<LayerFill, DlmmError> {
    if max_amount_out == 0 {
        return Ok(LayerFill {
            amount_in: 0,
            amount_left: amount,
            out_amount: 0,
            exhausted: true,
        });
    }
    let max_amount_in = input_for_gross_out(max_amount_out, price_q64, swap_for_y)?;
    if amount >= max_amount_in {
        Ok(LayerFill {
            amount_in: max_amount_in,
            amount_left: amount
                .checked_sub(max_amount_in)
                .ok_or(DlmmError::MathOverflow)?,
            out_amount: max_amount_out,
            exhausted: true,
        })
    } else {
        let out_amount = gross_out_for_input(amount, price_q64, swap_for_y)?;
        Ok(LayerFill {
            amount_in: amount,
            amount_left: 0,
            out_amount,
            exhausted: false,
        })
    }
}

/// Fill a bin's liquidity layers in official order: market-maker reserves
/// first, then processed limit orders, then open limit orders. Mirrors the
/// official SDK `get_exact_in_fill_amount_result`.
fn fill_bin_layers(
    bin: &DlmmBin,
    amount_in: u64,
    price_q64: u128,
    swap_for_y: bool,
    support_limit_order: bool,
) -> Result<BinFillResult, DlmmError> {
    let mm_amount = if swap_for_y {
        bin.amount_y
    } else {
        bin.amount_x
    };
    let mm_fill = fill_liquidity_layer(amount_in, mm_amount, price_q64, swap_for_y)?;

    if !support_limit_order {
        return Ok(BinFillResult {
            amount_in: mm_fill.amount_in,
            amount_left: mm_fill.amount_left,
            out_amount: mm_fill.out_amount,
            mm_amount_in: mm_fill.amount_in,
            mm_out: mm_fill.out_amount,
            processed_out: 0,
            open_out: 0,
            drained: mm_fill.exhausted,
        });
    }

    let mut total_amount_in = mm_fill.amount_in;
    let mut total_amount_out = mm_fill.out_amount;
    let mut amount_left = mm_fill.amount_left;
    let mut processed_out = 0u64;
    let mut open_out = 0u64;
    let mut drained = mm_fill.exhausted;

    if amount_left > 0 {
        let (open_order_amount, processed_order_remaining) =
            limit_order_amounts_by_direction(bin, swap_for_y);

        let processed_fill = fill_liquidity_layer(
            amount_left,
            processed_order_remaining,
            price_q64,
            swap_for_y,
        )?;
        total_amount_in = total_amount_in
            .checked_add(processed_fill.amount_in)
            .ok_or(DlmmError::MathOverflow)?;
        total_amount_out = total_amount_out
            .checked_add(processed_fill.out_amount)
            .ok_or(DlmmError::MathOverflow)?;
        processed_out = processed_fill.out_amount;
        amount_left = processed_fill.amount_left;
        drained &= processed_fill.exhausted;

        if amount_left > 0 {
            let open_fill =
                fill_liquidity_layer(amount_left, open_order_amount, price_q64, swap_for_y)?;
            total_amount_in = total_amount_in
                .checked_add(open_fill.amount_in)
                .ok_or(DlmmError::MathOverflow)?;
            total_amount_out = total_amount_out
                .checked_add(open_fill.out_amount)
                .ok_or(DlmmError::MathOverflow)?;
            open_out = open_fill.out_amount;
            amount_left = open_fill.amount_left;
            drained &= open_fill.exhausted;
        }
    }

    Ok(BinFillResult {
        amount_in: total_amount_in,
        amount_left,
        out_amount: total_amount_out,
        mm_amount_in: mm_fill.amount_in,
        mm_out: mm_fill.out_amount,
        processed_out,
        open_out,
        drained,
    })
}

/// Split the total trading fee between the LP (user) fee and the protocol fee,
/// accounting for the limit-order fee share. Mirrors the official SDK
/// `split_fee`:
///
/// - `mm_fee = ceil(trading_fee * mm_amount_in / total_amount_in)`
/// - `total_lo_fee = trading_fee - mm_fee`
/// - `lo_fee = total_lo_fee * LIMIT_ORDER_FEE_SHARE / 10_000` (order placer)
/// - `lo_protocol_fee = total_lo_fee - lo_fee`
/// - `mm_protocol_fee = floor(mm_fee * protocol_share / 10_000)`
/// - `protocol_fee = lo_protocol_fee + mm_protocol_fee`
/// - `user_fee = trading_fee - protocol_fee`
fn split_fee(
    trading_fee: u64,
    protocol_share_bps: u16,
    mm_amount_in: u64,
    total_amount_in: u64,
) -> Result<(u64, u64), DlmmError> {
    if total_amount_in == 0 || trading_fee == 0 {
        return Ok((0, 0));
    }

    let mm_fee: u64 = u128::from(trading_fee)
        .checked_mul(u128::from(mm_amount_in))
        .ok_or(DlmmError::MathOverflow)?
        .checked_add(
            u128::from(total_amount_in)
                .checked_sub(1)
                .ok_or(DlmmError::MathOverflow)?,
        )
        .ok_or(DlmmError::MathOverflow)?
        .checked_div(u128::from(total_amount_in))
        .ok_or(DlmmError::MathOverflow)?
        .try_into()
        .map_err(|_| DlmmError::TypeCastFailed)?;

    let total_lo_fee = trading_fee
        .checked_sub(mm_fee)
        .ok_or(DlmmError::MathOverflow)?;

    let lo_fee: u64 = u128::from(total_lo_fee)
        .checked_mul(LIMIT_ORDER_FEE_SHARE)
        .ok_or(DlmmError::MathOverflow)?
        .checked_div(BASIS_POINT_MAX)
        .ok_or(DlmmError::MathOverflow)?
        .try_into()
        .map_err(|_| DlmmError::TypeCastFailed)?;

    let lo_protocol_fee = total_lo_fee
        .checked_sub(lo_fee)
        .ok_or(DlmmError::MathOverflow)?;

    let mm_protocol_fee: u64 = u128::from(mm_fee)
        .checked_mul(u128::from(protocol_share_bps))
        .ok_or(DlmmError::MathOverflow)?
        .checked_div(BASIS_POINT_MAX)
        .ok_or(DlmmError::MathOverflow)?
        .try_into()
        .map_err(|_| DlmmError::TypeCastFailed)?;

    let total_protocol_fee = lo_protocol_fee
        .checked_add(mm_protocol_fee)
        .ok_or(DlmmError::MathOverflow)?;

    let total_user_fee = trading_fee
        .checked_sub(total_protocol_fee)
        .ok_or(DlmmError::MathOverflow)?;

    Ok((total_user_fee, total_protocol_fee))
}

/// Swap into a single discrete bin, consuming input across all liquidity
/// layers (MM reserves, then processed limit orders, then open limit orders).
///
/// Direction convention:
/// - `swap_for_y = true` (X -> Y): Inbound is Token X, Outbound is Token Y.
/// - `swap_for_y = false` (Y -> X): Inbound is Token Y, Outbound is Token X.
///
/// `fee_on_input` mirrors the official SDK `getFeeMode(...).feeOnInput`:
/// - `true`: the fee is removed from the input before the bin is priced.
/// - `false`: the full input is priced and the fee is removed from the output.
///
/// Fee semantics follow the official SDK `swap_exact_in_quote_at_bin`:
/// `fee_amount` is the total trading fee; when the bin cannot absorb the full
/// input the fee is recomputed over the actually consumed fee-excluded input
/// (`compute_fee`); the protocol share is derived via `split_fee`, which
/// apportions the fee between the MM and limit-order layers.
///
/// Route-local state commit: the MM reserve layer exchanges reserves; the
/// limit-order layers pay out from their own committed liquidity (their
/// fields are decremented by the filled output amounts); input paid to the
/// order layers is escrowed by the order placers, not added to the reserves.
#[allow(clippy::too_many_arguments)]
pub fn consume_bin_liquidity(
    bin: &mut DlmmBin,
    amount_in: u64,
    price_q64: u128,
    swap_for_y: bool,
    fee_rate: u128,
    protocol_share_bps: u16,
    fee_on_input: bool,
    support_limit_order: bool,
) -> Result<DlmmSwapStepResult, DlmmError> {
    if amount_in == 0 {
        return Ok(DlmmSwapStepResult {
            amount_in_consumed: 0,
            amount_out: 0,
            fee_amount: 0,
            protocol_fee: 0,
            drained_bin: false,
        });
    }

    let bin_max_out = bin_max_amount_out(bin, swap_for_y, support_limit_order);
    if bin_max_out == 0 {
        return Ok(DlmmSwapStepResult {
            amount_in_consumed: 0,
            amount_out: 0,
            fee_amount: 0,
            protocol_fee: 0,
            drained_bin: true,
        });
    }

    let mut trading_fee: u64 = 0;
    let mut excluded_fee_amount_in = amount_in;

    if fee_on_input {
        trading_fee = compute_fee_from_amount(amount_in, fee_rate)?;
        excluded_fee_amount_in = amount_in
            .checked_sub(trading_fee)
            .ok_or(DlmmError::MathOverflow)?;
    }

    let fill = fill_bin_layers(
        bin,
        excluded_fee_amount_in,
        price_q64,
        swap_for_y,
        support_limit_order,
    )?;

    let mut included_fee_amount_in = amount_in;
    if fill.amount_left > 0 {
        let excluded_consumed = excluded_fee_amount_in
            .checked_sub(fill.amount_left)
            .ok_or(DlmmError::MathOverflow)?;
        if fee_on_input {
            trading_fee = compute_fee_to_add(excluded_consumed, fee_rate)?;
            included_fee_amount_in = excluded_consumed
                .checked_add(trading_fee)
                .ok_or(DlmmError::MathOverflow)?;
        } else {
            included_fee_amount_in = excluded_consumed;
        }
    }

    let mut excluded_fee_amount_out = fill.out_amount;
    if !fee_on_input {
        trading_fee = compute_fee_from_amount(fill.out_amount, fee_rate)?;
        excluded_fee_amount_out = fill
            .out_amount
            .checked_sub(trading_fee)
            .ok_or(DlmmError::MathOverflow)?;
    }

    let (_user_fee, protocol_fee) = split_fee(
        trading_fee,
        protocol_share_bps,
        fill.mm_amount_in,
        fill.amount_in,
    )?;

    if swap_for_y {
        bin.amount_x = bin
            .amount_x
            .checked_add(fill.mm_amount_in)
            .ok_or(DlmmError::MathOverflow)?;
        bin.amount_y = bin
            .amount_y
            .checked_sub(fill.mm_out)
            .ok_or(DlmmError::MathOverflow)?;
    } else {
        bin.amount_y = bin
            .amount_y
            .checked_add(fill.mm_amount_in)
            .ok_or(DlmmError::MathOverflow)?;
        bin.amount_x = bin
            .amount_x
            .checked_sub(fill.mm_out)
            .ok_or(DlmmError::MathOverflow)?;
    }
    bin.processed_order_remaining_amount = bin
        .processed_order_remaining_amount
        .checked_sub(fill.processed_out)
        .ok_or(DlmmError::MathOverflow)?;
    bin.open_order_amount = bin
        .open_order_amount
        .checked_sub(fill.open_out)
        .ok_or(DlmmError::MathOverflow)?;

    Ok(DlmmSwapStepResult {
        amount_in_consumed: included_fee_amount_in,
        amount_out: excluded_fee_amount_out,
        fee_amount: trading_fee,
        protocol_fee,
        drained_bin: fill.drained,
    })
}
