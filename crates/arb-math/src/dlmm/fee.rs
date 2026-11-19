use super::error::DlmmError;
use super::types::{DlmmParameters, DlmmVariableParameters};
use super::{BASIS_POINT_MAX, FEE_PRECISION, MAX_FEE_RATE};
use ruint::aliases::U256;

/// Calculate base fee rate in $10^9$ (`FEE_PRECISION`) precision units.
///
/// Formula: $\text{Base Fee} = \text{base\_factor} \times \text{bin\_step} \times 10 \times 10^{\text{power\_factor}}$
pub fn compute_base_fee(parameters: &DlmmParameters, bin_step: u16) -> Result<u128, DlmmError> {
    let bf = u128::from(parameters.base_factor);
    let bs = u128::from(bin_step);
    let base = bf
        .checked_mul(bs)
        .and_then(|v| v.checked_mul(10))
        .ok_or(DlmmError::MathOverflow)?;

    let power = parameters.base_fee_power_factor;
    if power == 0 {
        Ok(base)
    } else {
        let scale = 10u128
            .checked_pow(u32::from(power))
            .ok_or(DlmmError::MathOverflow)?;
        base.checked_mul(scale).ok_or(DlmmError::MathOverflow)
    }
}

/// Calculate dynamic variable fee rate in $10^9$ (`FEE_PRECISION`) precision units.
///
/// Formula:
/// $$\text{Variable Fee} = \left\lceil \frac{\text{variable\_fee\_control} \times (\text{volatility\_accumulator} \times \text{bin\_step})^2}{100{,}000{,}000{,}000} \right\rceil$$
pub fn compute_variable_fee(
    parameters: &DlmmParameters,
    bin_step: u16,
    volatility_accumulator: u32,
) -> Result<u128, DlmmError> {
    if parameters.variable_fee_control == 0 || volatility_accumulator == 0 {
        return Ok(0);
    }

    let va = U256::from(volatility_accumulator);
    let bs = U256::from(bin_step);
    let vfc = U256::from(parameters.variable_fee_control);

    let prod = va * bs;
    let square = prod * prod;
    let num = vfc * square;

    let denom = U256::from(100_000_000_000u64);
    let (quot, rem) = num.div_rem(denom);
    let variable_fee = if rem.is_zero() {
        quot
    } else {
        quot.checked_add(U256::from(1))
            .ok_or(DlmmError::MathOverflow)?
    };

    variable_fee.try_into().map_err(|_| DlmmError::MathOverflow)
}

/// Calculate total fee rate (base fee + variable fee), clamped at 10% (`MAX_FEE_RATE`).
pub fn compute_total_fee_rate(
    parameters: &DlmmParameters,
    bin_step: u16,
    volatility_accumulator: u32,
) -> Result<u128, DlmmError> {
    let base = compute_base_fee(parameters, bin_step)?;
    let variable = compute_variable_fee(parameters, bin_step, volatility_accumulator)?;
    let total = base.checked_add(variable).ok_or(DlmmError::MathOverflow)?;
    Ok(total.min(MAX_FEE_RATE))
}

/// Calculate fee needed on top of `amount` to drain a bin.
///
/// Ceil-rounded: $\lceil (\text{amount} \times \text{rate}) / (10^9 - \text{rate}) \rceil$.
pub fn compute_fee_to_add(amount: u64, fee_rate: u128) -> Result<u64, DlmmError> {
    if fee_rate == 0 || amount == 0 {
        return Ok(0);
    }
    let denom = FEE_PRECISION
        .checked_sub(fee_rate)
        .ok_or(DlmmError::MathOverflow)?;
    let num = U256::from(amount) * U256::from(fee_rate);
    let (quot, rem) = num.div_rem(U256::from(denom));
    let fee = if rem.is_zero() {
        quot
    } else {
        quot.checked_add(U256::from(1))
            .ok_or(DlmmError::MathOverflow)?
    };

    fee.try_into().map_err(|_| DlmmError::TypeCastFailed)
}

/// Fee-included amount for a fee-excluded amount, exactly as the official SDK's
/// `getIncludedFeeAmount` does:
/// $\lceil (\text{excluded} \times 10^9) / (10^9 - \text{rate}) \rceil$.
///
/// Equivalent to `excluded + compute_fee_to_add(excluded, rate)` (the identity
/// holds in exact integer arithmetic), but named for SDK traceability.
pub fn included_fee_amount(excluded: u64, fee_rate: u128) -> Result<u64, DlmmError> {
    if fee_rate == 0 || excluded == 0 {
        return Ok(excluded);
    }
    let denom = FEE_PRECISION
        .checked_sub(fee_rate)
        .ok_or(DlmmError::MathOverflow)?;
    if denom == 0 {
        return Err(DlmmError::MathOverflow);
    }
    let num = U256::from(excluded) * U256::from(FEE_PRECISION);
    let (quot, rem) = num.div_rem(U256::from(denom));
    let included = if rem.is_zero() {
        quot
    } else {
        quot.checked_add(U256::from(1))
            .ok_or(DlmmError::MathOverflow)?
    };
    included.try_into().map_err(|_| DlmmError::TypeCastFailed)
}

/// Extract fee portion from an `amount_with_fee`.
///
/// Ceil-rounded: $\lceil (\text{amount\_with\_fee} \times \text{rate}) / 10^9 \rceil$.
pub fn compute_fee_from_amount(amount_with_fee: u64, fee_rate: u128) -> Result<u64, DlmmError> {
    if fee_rate == 0 || amount_with_fee == 0 {
        return Ok(0);
    }
    let num = U256::from(amount_with_fee) * U256::from(fee_rate);
    let (quot, rem) = num.div_rem(U256::from(FEE_PRECISION));
    let fee = if rem.is_zero() {
        quot
    } else {
        quot.checked_add(U256::from(1))
            .ok_or(DlmmError::MathOverflow)?
    };

    fee.try_into().map_err(|_| DlmmError::TypeCastFailed)
}

/// Compute protocol fee portion from total fee.
pub fn compute_protocol_fee(fee_amount: u64, protocol_share_bps: u16) -> Result<u64, DlmmError> {
    if fee_amount == 0 || protocol_share_bps == 0 {
        return Ok(0);
    }
    let num = U256::from(fee_amount) * U256::from(protocol_share_bps);
    let p_fee = num / U256::from(BASIS_POINT_MAX);

    p_fee.try_into().map_err(|_| DlmmError::TypeCastFailed)
}

/// Update runtime volatility reference parameters based on elapsed timestamp.
pub fn update_references_with_decay(
    v_params: &mut DlmmVariableParameters,
    params: &DlmmParameters,
    active_id: i32,
    current_timestamp: i64,
) -> Result<(), DlmmError> {
    let elapsed = current_timestamp.saturating_sub(v_params.last_update_timestamp);

    if elapsed >= i64::from(params.filter_period) {
        v_params.index_reference = active_id;
        if elapsed < i64::from(params.decay_period) {
            let va = u64::from(v_params.volatility_accumulator);
            let rf = u64::from(params.reduction_factor);
            let decayed = (va * rf) / (BASIS_POINT_MAX as u64);
            v_params.volatility_reference = u32::try_from(decayed).unwrap_or(u32::MAX);
        } else {
            v_params.volatility_reference = 0;
        }
    }
    Ok(())
}

/// Recalculate volatility accumulator based on current distance from `index_reference`.
pub fn update_volatility_accumulator(
    v_params: &mut DlmmVariableParameters,
    params: &DlmmParameters,
    active_id: i32,
) -> Result<(), DlmmError> {
    let delta_id = i64::from(v_params.index_reference)
        .saturating_sub(i64::from(active_id))
        .unsigned_abs();

    let va = u64::from(v_params.volatility_reference)
        .saturating_add(delta_id.saturating_mul(BASIS_POINT_MAX as u64));

    let max_va = u64::from(params.max_volatility_accumulator);
    v_params.volatility_accumulator = u32::try_from(va.min(max_va)).unwrap_or(u32::MAX);

    Ok(())
}
