use super::error::DlmmError;
use super::{BASIS_POINT_MAX, MAX_BIN_ID, MAX_EXPONENTIAL, MIN_BIN_ID, ONE_Q64, SCALE_OFFSET};
use arb_types::Bps;
use ruint::aliases::U256;

/// Compute `base^exp` in Q64.64 fixed-point representation.
///
/// Ported verbatim from on-chain `lb_clmm::u64x64_math`.
/// Handles signed 32-bit exponents by inverting the base when `exp < 0`
/// and unrolls 19 iterations of binary exponentiation ($2^{19} = 524{,}288$).
pub fn pow_q64(base: u128, exp: i32) -> Result<u128, DlmmError> {
    if exp == 0 {
        return Ok(ONE_Q64);
    }

    let mut invert = exp < 0;
    let exp_abs = exp.unsigned_abs();

    if exp_abs >= MAX_EXPONENTIAL {
        return Err(DlmmError::MathOverflow);
    }

    let mut squared_base = base;
    let mut result = ONE_Q64;

    // When base >= 1.0 (in Q64.64), invert to ensure upper 64 bits are zero
    // so that subsequent squaring does not overflow u128.
    if squared_base >= result {
        squared_base = u128::MAX
            .checked_div(squared_base)
            .ok_or(DlmmError::MathOverflow)?;
        invert = !invert;
    }

    for bit in 0..19 {
        if (exp_abs & (1 << bit)) > 0 {
            let prod = U256::from(result) * U256::from(squared_base);
            let res = prod >> SCALE_OFFSET;
            result = res.try_into().map_err(|_| DlmmError::MathOverflow)?;
        }
        let sq = U256::from(squared_base) * U256::from(squared_base);
        let next_sq = sq >> SCALE_OFFSET;
        squared_base = next_sq.try_into().map_err(|_| DlmmError::MathOverflow)?;
    }

    // A zero fixed-point result is underflow, not a valid price. Do not
    // manufacture a one-unit price that could enter the executable graph.
    if result == 0 {
        return Err(DlmmError::MathOverflow);
    }

    if invert {
        let inv = (U256::from(ONE_Q64) << SCALE_OFFSET) / U256::from(result);
        result = inv.try_into().map_err(|_| DlmmError::MathOverflow)?;
        if result == 0 {
            return Err(DlmmError::MathOverflow);
        }
    }

    Ok(result)
}

/// Compute the exact Q64.64 price of a bin from `(active_id, bin_step)`.
///
/// Formula: $P_i = (1 + \text{bin\_step}/10{,}000)^i$
/// Returns the price of 1 unit of Token X in terms of Token Y.
pub fn get_price_from_id(active_id: i32, bin_step: u16) -> Result<u128, DlmmError> {
    if bin_step == 0 || bin_step > arb_constants::METEORA_MAX_BIN_STEP {
        return Err(DlmmError::InvalidBinStep(bin_step));
    }
    if !(MIN_BIN_ID..=MAX_BIN_ID).contains(&active_id) {
        return Err(DlmmError::BinIdOutOfRange(active_id));
    }

    let bs = U256::from(bin_step) << SCALE_OFFSET;
    let bps_q64 = bs / U256::from(BASIS_POINT_MAX);
    let base = U256::from(ONE_Q64) + bps_q64;
    let base_u128: u128 = base.try_into().map_err(|_| DlmmError::MathOverflow)?;

    pow_q64(base_u128, active_id)
}

/// Move the active bin index by 1 step in the direction of the trade.
pub fn advance_active_id(active_id: i32, swap_for_y: bool) -> Result<i32, DlmmError> {
    let next = if swap_for_y {
        active_id.checked_sub(1).ok_or(DlmmError::MathOverflow)?
    } else {
        active_id.checked_add(1).ok_or(DlmmError::MathOverflow)?
    };

    if !(MIN_BIN_ID..=MAX_BIN_ID).contains(&next) {
        return Err(DlmmError::BinIdOutOfRange(next));
    }

    Ok(next)
}

/// Compute the bin-array index (i64) that contains a given DLMM bin_id.
/// Uses Euclidean division so negative bin IDs map correctly.
pub fn bin_array_index_for_bin(bin_id: i32) -> i64 {
    (bin_id as i64).div_euclid(70)
}

/// Calculate execution price impact against starting spot price in basis points.
pub(super) fn calculate_price_impact_bps(
    amount_in: u64,
    amount_out: u64,
    start_price_q64: u128,
    swap_for_y: bool,
) -> Bps {
    if amount_in == 0 || amount_out == 0 || start_price_q64 == 0 {
        return Bps(0);
    }

    // Expected ideal output without price impact
    let ideal_out: u64 = if swap_for_y {
        let prod = U256::from(amount_in) * U256::from(start_price_q64);
        match (prod >> SCALE_OFFSET).try_into() {
            Ok(v) => v,
            Err(_) => return Bps(10_000), // Fail-closed: assume maximal price impact on overflow
        }
    } else {
        let num = U256::from(amount_in) << SCALE_OFFSET;
        match (num / U256::from(start_price_q64)).try_into() {
            Ok(v) => v,
            Err(_) => return Bps(10_000), // Fail-closed: assume maximal price impact on overflow
        }
    };

    if ideal_out <= amount_out {
        return Bps(0);
    }

    let diff = ideal_out.saturating_sub(amount_out);
    let numerator = U256::from(diff) * U256::from(10_000);
    let denominator = U256::from(ideal_out);
    let bps = (numerator + denominator - U256::from(1u8)) / denominator;
    let val = u16::try_from(bps.to::<u64>().min(10_000)).unwrap_or(10_000);
    Bps(val)
}
