//! Q64.64 fixed-point primitives, binary exponentiation, and shared DLMM
//! constants. Zero floating point; all division is explicit-precision.

use ruint::aliases::U256;

use super::state::DlmmError;

/// Number of fractional bits in Q64.64 fixed-point numbers.
pub const SCALE_OFFSET: u8 = 64;

/// 1.0 in Q64.64 representation ($2^{64}$).
pub const ONE_Q64: u128 = 1u128 << SCALE_OFFSET;

/// Basis points divisor ($10{,}000 = 100\%$).
pub const BASIS_POINT_MAX: u128 = 10_000;

/// Internal fee precision denominator ($10^9 = 100\%$).
pub const FEE_PRECISION: u128 = 1_000_000_000;

/// Maximum fee rate enforced by the on-chain protocol ($10\% = 100{,}000{,}000$ in $10^9$ units).
pub const MAX_FEE_RATE: u128 = 100_000_000;

/// Share of the limit-order fee that goes to the order placer (50% = 5000 bps).
/// The remaining 50% is protocol fee. Source: official Meteora DLMM SDK
/// `MeteoraAg/dlmm-sdk` `commons/src/quote.rs` (`LIMIT_ORDER_FEE_SHARE`).
pub const LIMIT_ORDER_FEE_SHARE: u128 = 5000;

/// Minimum valid Bin ID on Solana Meteora DLMM.
pub const MIN_BIN_ID: i32 = -443_636;

/// Maximum valid Bin ID on Solana Meteora DLMM.
pub const MAX_BIN_ID: i32 = 443_636;

/// Upper limit for binary exponentiation ($2^{19} = 524{,}288$).
pub const MAX_EXPONENTIAL: u32 = 0x80_000;

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
    if !(MIN_BIN_ID..=MAX_BIN_ID).contains(&active_id) {
        return Err(DlmmError::BinIdOutOfRange(active_id));
    }

    let bs = U256::from(bin_step) << SCALE_OFFSET;
    let bps_q64 = bs / U256::from(BASIS_POINT_MAX);
    let base = U256::from(ONE_Q64) + bps_q64;
    let base_u128: u128 = base.try_into().map_err(|_| DlmmError::MathOverflow)?;

    pow_q64(base_u128, active_id)
}
