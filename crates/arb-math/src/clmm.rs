//! Concentrated Liquidity (CLMM) Math Module for Orca Whirlpools and Raydium CLMM.
//!
//! Provides:
//! - Exact Q64.64 bitwise binary exponentiation for tick-to-sqrt-price (zero floats)
//! - Exact amount deltas with directional rounding (ceiling on inflows, floor on outflows)
//! - Exact fee deduction using hundredths of a basis point (`FEE_RATE_DENOMINATOR = 1_000_000`)
//! - Strict price ordering: Token A = min(mint1, mint2), Token B = max(mint1, mint2)
//! - TickArray binary layout parsing and initialized tick traversal replacing 8-tick jump heuristic

use ruint::aliases::U256;
use thiserror::Error;

pub const MIN_TICK_INDEX: i32 = -443_636;
pub const MAX_TICK_INDEX: i32 = 443_636;
pub const MIN_TICK: i32 = MIN_TICK_INDEX;
pub const MAX_TICK: i32 = MAX_TICK_INDEX;

pub const MIN_SQRT_PRICE_X64: u128 = 4_295_048_016;
pub const MAX_SQRT_PRICE_X64: u128 = 79_226_673_515_401_499_993_453_864_427;
pub const Q64: u128 = 1u128 << 64;

pub const FEE_RATE_DENOMINATOR: u64 = 1_000_000;
pub const TICK_ARRAY_SIZE: usize = 88;
pub const WHIRLPOOL_TICK_ARRAY_SIZE: usize = 9988;
pub const TICK_STRUCT_SIZE: usize = 113;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum ClmmMathError {
    #[error("Tick index out of bounds (-443636..=443636): {0}")]
    TickIndexOutOfBounds(i32),

    #[error("Sqrt price out of bounds")]
    SqrtPriceOutOfBounds,

    #[error("Integer overflow in calculation")]
    IntegerOverflow,

    #[error("Division by zero")]
    DivisionByZero,

    #[error("Liquidity underflow or overflow")]
    LiquidityOverflow,

    #[error("Invalid tick array account data")]
    InvalidAccountData,

    #[error("swap made no forward progress")]
    NoProgress,

    #[error("swap exceeded the configured tick traversal bound")]
    IterationLimitExceeded,

    #[error("swap could not consume the requested input with available liquidity/tick arrays")]
    IncompleteQuote,

    #[error("missing tick-array coverage at start tick {0}")]
    MissingTickArrayCoverage(i32),
}

/// Token ordering check: returns true if `mint_in` is Token A (lexicographically smaller).
pub fn is_token_a(mint_in: &[u8; 32], mint_out: &[u8; 32]) -> bool {
    mint_in < mint_out
}

/// Token ordering check with Solana Pubkey.
pub fn is_token_a_pubkey(
    mint_in: &solana_pubkey::Pubkey,
    mint_out: &solana_pubkey::Pubkey,
) -> bool {
    mint_in < mint_out
}

/// Exact integer division with ceiling rounding for U256.
pub fn div_ceil_u256(num: U256, denom: U256) -> Result<U256, ClmmMathError> {
    if denom.is_zero() {
        return Err(ClmmMathError::DivisionByZero);
    }
    let (quot, rem) = num.div_rem(denom);
    if rem.is_zero() {
        Ok(quot)
    } else {
        quot.checked_add(U256::from(1))
            .ok_or(ClmmMathError::IntegerOverflow)
    }
}

/// Exact integer division with ceiling rounding for u128 -> u64.
pub fn div_ceil_u64(num: u128, denom: u128) -> Result<u64, ClmmMathError> {
    if denom == 0 {
        return Err(ClmmMathError::DivisionByZero);
    }
    let quot = num / denom;
    let rem = num % denom;
    let res = if rem == 0 {
        quot
    } else {
        quot.checked_add(1).ok_or(ClmmMathError::IntegerOverflow)?
    };
    u64::try_from(res).map_err(|_| ClmmMathError::IntegerOverflow)
}

/// Pure integer bitwise binary exponentiation computing sqrt_price_x64 for a given tick index.
///
/// Returns sqrt_price in Q64.64 fixed-point format.
/// Matches on-chain Orca Whirlpool & Uniswap v3 integer math with zero floating-point operations.
pub fn tick_index_to_sqrt_price_x64(tick_index: i32) -> Result<u128, ClmmMathError> {
    if !(MIN_TICK_INDEX..=MAX_TICK_INDEX).contains(&tick_index) {
        return Err(ClmmMathError::TickIndexOutOfBounds(tick_index));
    }

    let abs_tick = tick_index.unsigned_abs();

    // 19 precomputed fixed-point constants in Q128.128 format representing 1 / sqrt(1.0001^(2^i))
    let mut ratio = if (abs_tick & 0x1) != 0 {
        U256::from(0xfffc_b933_bd6f_ad37_aa2d_162d_1a59_4001u128)
    } else {
        U256::from(1) << 128
    };

    if (abs_tick & 0x2) != 0 {
        ratio = (ratio * U256::from(0xfff9_7272_373d_4132_59a4_6990_580e_213au128)) >> 128;
    }
    if (abs_tick & 0x4) != 0 {
        ratio = (ratio * U256::from(0xfff2_e50f_5f65_6932_ef12_357c_f3c7_fdccu128)) >> 128;
    }
    if (abs_tick & 0x8) != 0 {
        ratio = (ratio * U256::from(0xffe5_caca_7e10_e4e6_1c36_24ea_a094_1cd0u128)) >> 128;
    }
    if (abs_tick & 0x10) != 0 {
        ratio = (ratio * U256::from(0xffcb_9843_d60f_6159_c9db_5883_5c92_6644u128)) >> 128;
    }
    if (abs_tick & 0x20) != 0 {
        ratio = (ratio * U256::from(0xff97_3b41_fa98_c081_472e_6896_dfb2_54c0u128)) >> 128;
    }
    if (abs_tick & 0x40) != 0 {
        ratio = (ratio * U256::from(0xff2e_a164_66c9_6a38_43ec_78b3_26b5_2861u128)) >> 128;
    }
    if (abs_tick & 0x80) != 0 {
        ratio = (ratio * U256::from(0xfe5d_ee04_6a99_a2a8_11c4_61f1_969c_3053u128)) >> 128;
    }
    if (abs_tick & 0x100) != 0 {
        ratio = (ratio * U256::from(0xfcbe_86c7_900a_88ae_dcff_c83b_479a_a3a4u128)) >> 128;
    }
    if (abs_tick & 0x200) != 0 {
        ratio = (ratio * U256::from(0xf987_a725_3ac4_1317_6f2b_074c_f781_5e54u128)) >> 128;
    }
    if (abs_tick & 0x400) != 0 {
        ratio = (ratio * U256::from(0xf339_2b08_22b7_0005_940c_7a39_8e4b_70f3u128)) >> 128;
    }
    if (abs_tick & 0x800) != 0 {
        ratio = (ratio * U256::from(0xe715_9475_a2c2_9b74_43b2_9c7f_a6e8_89d9u128)) >> 128;
    }
    if (abs_tick & 0x1000) != 0 {
        ratio = (ratio * U256::from(0xd097_f3bd_fd20_22b8_845a_d8f7_92aa_5825u128)) >> 128;
    }
    if (abs_tick & 0x2000) != 0 {
        ratio = (ratio * U256::from(0xa9f7_4646_2d87_0fdf_8a65_dc1f_90e0_61e5u128)) >> 128;
    }
    if (abs_tick & 0x4000) != 0 {
        ratio = (ratio * U256::from(0x70d8_69a1_56d2_a1b8_90bb_3df6_2baf_32f7u128)) >> 128;
    }
    if (abs_tick & 0x8000) != 0 {
        ratio = (ratio * U256::from(0x31be_135f_97d0_8fd9_8123_1505_542f_cfa6u128)) >> 128;
    }
    if (abs_tick & 0x10000) != 0 {
        ratio = (ratio * U256::from(0x09aa_508b_5b7a_84e1_c677_de54_f3e9_9bc9_u128)) >> 128;
    }
    if (abs_tick & 0x20000) != 0 {
        ratio = (ratio * U256::from(0x005d_6af8_dedb_8119_6699_c329_225e_e604_u128)) >> 128;
    }
    if (abs_tick & 0x40000) != 0 {
        ratio = (ratio * U256::from(0x0000_2216_e584_f5fa_1ea9_2604_1bed_fe98_u128)) >> 128;
    }

    if tick_index > 0 {
        ratio = U256::MAX / ratio;
    }

    let sqrt_price_x64_u256 = ratio >> 64;
    let res = u128::try_from(sqrt_price_x64_u256).map_err(|_| ClmmMathError::IntegerOverflow)?;

    Ok(res)
}

/// Checked tick-to-sqrt-price conversion. Invalid ticks are never mapped to a synthetic zero price.
pub fn tick_to_sqrt_price_x64(tick: i32) -> Result<u128, ClmmMathError> {
    tick_index_to_sqrt_price_x64(tick)
}

/// Exact inverse mapping from sqrt_price_x64 to tick index using integer binary search.
pub fn sqrt_price_x64_to_tick_index(sqrt_price_x64: u128) -> Result<i32, ClmmMathError> {
    if !(MIN_SQRT_PRICE_X64..=MAX_SQRT_PRICE_X64).contains(&sqrt_price_x64) {
        return Err(ClmmMathError::SqrtPriceOutOfBounds);
    }
    let mut low = MIN_TICK_INDEX;
    let mut high = MAX_TICK_INDEX;
    while low <= high {
        let mid = low + (high - low) / 2;
        let p_mid = tick_index_to_sqrt_price_x64(mid)?;
        if p_mid <= sqrt_price_x64 {
            let next_p = if mid < MAX_TICK_INDEX {
                tick_index_to_sqrt_price_x64(mid + 1)?
            } else {
                u128::MAX
            };
            if sqrt_price_x64 < next_p {
                return Ok(mid);
            }
            low = mid + 1;
        } else {
            high = mid - 1;
        }
    }
    Ok(low.clamp(MIN_TICK_INDEX, MAX_TICK_INDEX))
}

/// Compute input/output amount Delta A (Token X) between two sqrt prices with active liquidity L.
///
/// Formula: Delta A = L * (p_upper - p_lower) * 2^64 / (p_lower * p_upper)
pub fn get_amount_delta_a(
    sqrt_price_0: u128,
    sqrt_price_1: u128,
    liquidity: u128,
    round_up: bool,
) -> Result<u64, ClmmMathError> {
    if sqrt_price_0 == 0 || sqrt_price_1 == 0 || liquidity == 0 {
        return Ok(0);
    }
    let (sqrt_price_lower, sqrt_price_upper) = if sqrt_price_0 < sqrt_price_1 {
        (sqrt_price_0, sqrt_price_1)
    } else {
        (sqrt_price_1, sqrt_price_0)
    };

    let delta_p = U256::from(sqrt_price_upper - sqrt_price_lower);
    let l = U256::from(liquidity);
    let numerator = (l * delta_p) << 64;
    let denominator = U256::from(sqrt_price_lower) * U256::from(sqrt_price_upper);

    if denominator.is_zero() {
        return Err(ClmmMathError::DivisionByZero);
    }

    let amount = if round_up {
        div_ceil_u256(numerator, denominator)?
    } else {
        numerator / denominator
    };

    u64::try_from(amount).map_err(|_| ClmmMathError::IntegerOverflow)
}

/// Compute input/output amount Delta B (Token Y) between two sqrt prices with active liquidity L.
///
/// Formula: Delta B = L * (p_upper - p_lower) / 2^64
pub fn get_amount_delta_b(
    sqrt_price_0: u128,
    sqrt_price_1: u128,
    liquidity: u128,
    round_up: bool,
) -> Result<u64, ClmmMathError> {
    if sqrt_price_0 == 0 || sqrt_price_1 == 0 || liquidity == 0 {
        return Ok(0);
    }
    let (sqrt_price_lower, sqrt_price_upper) = if sqrt_price_0 < sqrt_price_1 {
        (sqrt_price_0, sqrt_price_1)
    } else {
        (sqrt_price_1, sqrt_price_0)
    };

    let delta_p = U256::from(sqrt_price_upper - sqrt_price_lower);
    let l = U256::from(liquidity);
    let prod = l * delta_p;

    let amount = if round_up {
        div_ceil_u256(prod, U256::from(1u128 << 64))?
    } else {
        prod >> 64
    };

    u64::try_from(amount).map_err(|_| ClmmMathError::IntegerOverflow)
}

/// Backward-compatible alias for Delta X (Token A).
pub fn get_delta_x(
    sqrt_price_a: u128,
    sqrt_price_b: u128,
    liquidity: u128,
    round_up: bool,
) -> Option<u64> {
    get_amount_delta_a(sqrt_price_a, sqrt_price_b, liquidity, round_up).ok()
}

/// Maximum input Delta X available in the current tick before crossing into the next tick boundary.
///
/// Mathematical formulation from SEEEVV.MD:
/// Delta x_max = L * (1 / sqrt(P_lower) - 1 / sqrt(P))
/// Computed in Q64.64 fixed point as:
/// Delta x_max = L * (sqrt(P) - sqrt(P_lower)) * 2^64 / (sqrt(P) * sqrt(P_lower))
pub fn clmm_max_delta_x_in_tick(
    sqrt_price_current: u128,
    sqrt_price_lower: u128,
    liquidity: u128,
) -> Option<u64> {
    if sqrt_price_current <= sqrt_price_lower || liquidity == 0 {
        return Some(0);
    }
    get_amount_delta_a(sqrt_price_lower, sqrt_price_current, liquidity, false).ok()
}

/// Maximum input Delta Y available in the current tick before crossing into the next upper tick boundary.
///
/// Mathematical formulation from SEEEVV.MD:
/// When selling Token B for Token A (b_to_a / price rises towards sqrt_price_upper):
/// Delta y_max = L * (sqrt(P_upper) - sqrt(P)) / 2^64
/// Computed in Q64.64 fixed point as:
/// Delta y_max = get_amount_delta_b(sqrt_price_current, sqrt_price_upper, liquidity, false)
pub fn clmm_max_delta_y_in_tick(
    sqrt_price_current: u128,
    sqrt_price_upper: u128,
    liquidity: u128,
) -> Option<u64> {
    if sqrt_price_current >= sqrt_price_upper || liquidity == 0 {
        return Some(0);
    }
    get_amount_delta_b(sqrt_price_current, sqrt_price_upper, liquidity, false).ok()
}

/// Compute maximum input token amount permitted in the current tick in either direction:
/// - If `a_to_b` is true (selling Token A): price drops towards `sqrt_price_lower`, returns Delta X max.
/// - If `a_to_b` is false (selling Token B): price rises towards `sqrt_price_upper`, returns Delta Y max.
pub fn clmm_max_input_in_tick(
    sqrt_price_current: u128,
    sqrt_price_lower: u128,
    sqrt_price_upper: u128,
    liquidity: u128,
    a_to_b: bool,
) -> Option<u64> {
    if a_to_b {
        clmm_max_delta_x_in_tick(sqrt_price_current, sqrt_price_lower, liquidity)
    } else {
        clmm_max_delta_y_in_tick(sqrt_price_current, sqrt_price_upper, liquidity)
    }
}

/// Calculate the active tick boundary input capacity for a CLMM pool:
/// Derives lower and upper tick boundaries from `tick_current` and `tick_spacing`,
/// and returns the maximum input amount in Q64.64 before crossing the boundary in either direction.
pub fn clmm_active_tick_max_input(
    sqrt_price_current: u128,
    tick_current: i32,
    tick_spacing: u16,
    liquidity: u128,
    a_to_b: bool,
) -> Option<u64> {
    if liquidity == 0 || tick_spacing == 0 || sqrt_price_current == 0 {
        return Some(0);
    }
    let s = tick_spacing as i32;
    let lower_tick = if tick_current >= 0 {
        (tick_current / s) * s
    } else {
        ((tick_current - s + 1) / s) * s
    };
    let upper_tick = lower_tick + s;
    let p_lower = tick_index_to_sqrt_price_x64(lower_tick).ok()?;
    let p_upper = tick_index_to_sqrt_price_x64(upper_tick).ok()?;
    clmm_max_input_in_tick(sqrt_price_current, p_lower, p_upper, liquidity, a_to_b)
}

/// Backward-compatible alias for Delta Y (Token B).
pub fn get_delta_y(
    sqrt_price_a: u128,
    sqrt_price_b: u128,
    liquidity: u128,
    round_up: bool,
) -> Option<u64> {
    get_amount_delta_b(sqrt_price_a, sqrt_price_b, liquidity, round_up).ok()
}

/// Compute next sqrt_price from Token A input ($A \to B$, price drops).
///
/// Formula: next_p = (L * P * 2^64) / (L * 2^64 + dx * P)
/// Rounded UP (ceiling) to ensure input delta does not exceed available amount.
pub fn get_next_sqrt_price_from_a_input(
    sqrt_price: u128,
    liquidity: u128,
    amount_in: u64,
) -> Result<u128, ClmmMathError> {
    if amount_in == 0 {
        return Ok(sqrt_price);
    }
    let l = U256::from(liquidity);
    let p = U256::from(sqrt_price);
    let dx = U256::from(amount_in);

    let numerator = (l * p) << 64;
    let denominator = (l << 64) + (dx * p);

    let next_price = div_ceil_u256(numerator, denominator)?;
    u128::try_from(next_price).map_err(|_| ClmmMathError::IntegerOverflow)
}

/// Compute next sqrt_price from Token B input ($B \to A$, price rises).
///
/// Formula: next_p = P + (dy * 2^64 / L)
/// Rounded DOWN (floor) to ensure input delta does not exceed available amount.
pub fn get_next_sqrt_price_from_b_input(
    sqrt_price: u128,
    liquidity: u128,
    amount_in: u64,
) -> Result<u128, ClmmMathError> {
    if amount_in == 0 {
        return Ok(sqrt_price);
    }
    if liquidity == 0 {
        return Err(ClmmMathError::DivisionByZero);
    }
    let p = U256::from(sqrt_price);
    let dy = U256::from(amount_in);
    let l = U256::from(liquidity);

    let delta_p = (dy << 64) / l;
    let next_price = p + delta_p;

    u128::try_from(next_price).map_err(|_| ClmmMathError::IntegerOverflow)
}

/// Result of a single tick-step swap calculation in CLMM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepSwapResult {
    pub amount_in: u64,
    pub amount_out: u64,
    pub next_sqrt_price_x64: u128,
    pub sqrt_price_next_x64: u128, // alias for backwards compatibility
    pub fee_amount: u64,
}

/// Computes a swap step within a tick boundary returning Result.
pub fn compute_swap_step_checked(
    sqrt_price_current_x64: u128,
    sqrt_price_target_x64: u128,
    liquidity: u128,
    amount_remaining: u64,
    fee_rate: u16, // hundredths of a bip (1_000_000 denominator)
    a_to_b: bool,
) -> Result<StepSwapResult, ClmmMathError> {
    if liquidity == 0 || amount_remaining == 0 {
        return Ok(StepSwapResult {
            amount_in: 0,
            amount_out: 0,
            next_sqrt_price_x64: sqrt_price_current_x64,
            sqrt_price_next_x64: sqrt_price_current_x64,
            fee_amount: 0,
        });
    }

    // 1. Calculate max amount_in needed to reach sqrt_price_target_x64
    let amount_in_max = if a_to_b {
        get_amount_delta_a(
            sqrt_price_target_x64,
            sqrt_price_current_x64,
            liquidity,
            true, // round_up = true for input
        )?
    } else {
        get_amount_delta_b(
            sqrt_price_current_x64,
            sqrt_price_target_x64,
            liquidity,
            true, // round_up = true for input
        )?
    };

    // 2. Net amount available after deducting fee from remaining input
    let fee_on_remaining = div_ceil_u64(
        u128::from(amount_remaining) * u128::from(fee_rate),
        u128::from(FEE_RATE_DENOMINATOR),
    )?;
    let amount_remaining_net = amount_remaining.saturating_sub(fee_on_remaining);

    if amount_remaining_net >= amount_in_max {
        // Full step reached target tick
        let fee_denom = u128::from(FEE_RATE_DENOMINATOR.saturating_sub(u64::from(fee_rate)));
        let fee_amount = div_ceil_u64(
            u128::from(amount_in_max) * u128::from(fee_rate),
            fee_denom.max(1),
        )?;
        let amount_in = amount_in_max.saturating_add(fee_amount);

        let amount_out = if a_to_b {
            get_amount_delta_b(
                sqrt_price_target_x64,
                sqrt_price_current_x64,
                liquidity,
                false, // round_up = false for output
            )?
        } else {
            get_amount_delta_a(
                sqrt_price_current_x64,
                sqrt_price_target_x64,
                liquidity,
                false, // round_up = false for output
            )?
        };

        Ok(StepSwapResult {
            amount_in,
            amount_out,
            next_sqrt_price_x64: sqrt_price_target_x64,
            sqrt_price_next_x64: sqrt_price_target_x64,
            fee_amount,
        })
    } else {
        // Partial step within tick: consumed all remaining amount
        let fee_amount = fee_on_remaining;
        let net_in = amount_remaining_net;

        let next_sqrt_price = if a_to_b {
            get_next_sqrt_price_from_a_input(sqrt_price_current_x64, liquidity, net_in)?
        } else {
            get_next_sqrt_price_from_b_input(sqrt_price_current_x64, liquidity, net_in)?
        };

        let amount_out = if a_to_b {
            get_amount_delta_b(
                next_sqrt_price,
                sqrt_price_current_x64,
                liquidity,
                false, // round_up = false
            )?
        } else {
            get_amount_delta_a(
                sqrt_price_current_x64,
                next_sqrt_price,
                liquidity,
                false, // round_up = false
            )?
        };

        Ok(StepSwapResult {
            amount_in: amount_remaining,
            amount_out,
            next_sqrt_price_x64: next_sqrt_price,
            sqrt_price_next_x64: next_sqrt_price,
            fee_amount,
        })
    }
}

/// Checked compatibility entry point. Arithmetic or state errors are never
/// converted into a fabricated quote or a synthetic price movement.
pub fn compute_swap_step(
    sqrt_price_current_x64: u128,
    sqrt_price_target_x64: u128,
    liquidity: u128,
    amount_remaining: u64,
    fee_rate: u16,
    a_to_b: bool,
) -> Result<StepSwapResult, ClmmMathError> {
    compute_swap_step_checked(
        sqrt_price_current_x64,
        sqrt_price_target_x64,
        liquidity,
        amount_remaining,
        fee_rate,
        a_to_b,
    )
}

/// Tick representation inside a TickArray.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TickFacade {
    pub tick_index: i32,
    pub initialized: bool,
    pub liquidity_net: i128,
    pub liquidity_gross: u128,
}

/// In-memory deserialized representation of a Whirlpool TickArray account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TickArrayFacade {
    pub start_tick_index: i32,
    pub ticks: Vec<TickFacade>,
}

/// Calculate the start tick index for a tick given Whirlpool tick spacing.
pub fn get_start_tick_index(tick: i32, tick_spacing: u16) -> i32 {
    let ticks_per_array = (TICK_ARRAY_SIZE as i32) * i32::from(tick_spacing.max(1));
    tick.div_euclid(ticks_per_array) * ticks_per_array
}

/// Parse an on-chain Whirlpool TickArray account (9,988 bytes).
pub fn parse_tick_array_account(
    data: &[u8],
    tick_spacing: u16,
) -> Result<TickArrayFacade, ClmmMathError> {
    if data.len() < WHIRLPOOL_TICK_ARRAY_SIZE {
        return Err(ClmmMathError::InvalidAccountData);
    }
    let start_tick_bytes: [u8; 4] = data[40..44]
        .try_into()
        .map_err(|_| ClmmMathError::InvalidAccountData)?;
    if tick_spacing == 0 {
        return Err(ClmmMathError::InvalidAccountData);
    }
    let start_tick_index = i32::from_le_bytes(start_tick_bytes);

    let mut ticks = Vec::with_capacity(TICK_ARRAY_SIZE);
    let spacing = i32::from(tick_spacing);
    let ticks_per_array = (TICK_ARRAY_SIZE as i32)
        .checked_mul(spacing)
        .ok_or(ClmmMathError::IntegerOverflow)?;
    if start_tick_index.rem_euclid(ticks_per_array) != 0 {
        return Err(ClmmMathError::InvalidAccountData);
    }

    for i in 0..TICK_ARRAY_SIZE {
        let offset = 44 + i * TICK_STRUCT_SIZE;
        let initialized_byte = *data.get(offset).ok_or(ClmmMathError::InvalidAccountData)?;
        if initialized_byte > 1 {
            return Err(ClmmMathError::InvalidAccountData);
        }
        let initialized = initialized_byte == 1;

        let net_bytes: [u8; 16] = data
            .get(offset + 1..offset + 17)
            .ok_or(ClmmMathError::InvalidAccountData)?
            .try_into()
            .map_err(|_| ClmmMathError::InvalidAccountData)?;
        let liquidity_net = i128::from_le_bytes(net_bytes);

        let gross_bytes: [u8; 16] = data
            .get(offset + 17..offset + 33)
            .ok_or(ClmmMathError::InvalidAccountData)?
            .try_into()
            .map_err(|_| ClmmMathError::InvalidAccountData)?;
        let liquidity_gross = u128::from_le_bytes(gross_bytes);
        if !initialized && (liquidity_net != 0 || liquidity_gross != 0) {
            return Err(ClmmMathError::InvalidAccountData);
        }

        let tick_offset = i32::try_from(i)
            .map_err(|_| ClmmMathError::IntegerOverflow)?
            .checked_mul(spacing)
            .ok_or(ClmmMathError::IntegerOverflow)?;
        let tick_index = start_tick_index
            .checked_add(tick_offset)
            .ok_or(ClmmMathError::IntegerOverflow)?;

        ticks.push(TickFacade {
            tick_index,
            initialized,
            liquidity_net,
            liquidity_gross,
        });
    }

    Ok(TickArrayFacade {
        start_tick_index,
        ticks,
    })
}

/// Find the next initialized tick within the covered array, or advance to its edge.
/// Missing array coverage is an explicit error; no synthetic spacing-boundary fallback exists.
pub fn find_next_tick_boundary(
    tick_current: i32,
    tick_spacing: u16,
    a_to_b: bool,
    tick_arrays: &[TickArrayFacade],
) -> Result<(i32, Option<TickFacade>), ClmmMathError> {
    if tick_spacing == 0 {
        return Err(ClmmMathError::InvalidAccountData);
    }
    if tick_arrays.is_empty() {
        return Err(ClmmMathError::MissingTickArrayCoverage(
            get_start_tick_index(tick_current, tick_spacing),
        ));
    }

    let spacing = i32::from(tick_spacing);
    let ticks_per_array = (TICK_ARRAY_SIZE as i32)
        .checked_mul(spacing)
        .ok_or(ClmmMathError::IntegerOverflow)?;
    let mut current_array_start = get_start_tick_index(tick_current, tick_spacing);
    if a_to_b && tick_current == current_array_start {
        current_array_start = current_array_start
            .checked_sub(ticks_per_array)
            .ok_or(ClmmMathError::IntegerOverflow)?;
    }
    let current_array = tick_arrays
        .iter()
        .find(|array| array.start_tick_index == current_array_start)
        .ok_or(ClmmMathError::MissingTickArrayCoverage(current_array_start))?;

    let candidate = if a_to_b {
        current_array
            .ticks
            .iter()
            .filter(|tick| tick.initialized && tick.tick_index <= tick_current)
            .max_by_key(|tick| tick.tick_index)
            .copied()
    } else {
        current_array
            .ticks
            .iter()
            .filter(|tick| tick.initialized && tick.tick_index > tick_current)
            .min_by_key(|tick| tick.tick_index)
            .copied()
    };

    if let Some(tick) = candidate {
        return Ok((tick.tick_index, Some(tick)));
    }

    // No initialized tick exists in this array. Move to its edge; the next
    // iteration must have the adjacent array loaded or fail with MissingTickArrayCoverage.
    let boundary = if a_to_b {
        current_array_start
    } else {
        current_array_start
            .checked_add(ticks_per_array)
            .ok_or(ClmmMathError::IntegerOverflow)?
    };
    let boundary = boundary.clamp(MIN_TICK_INDEX, MAX_TICK_INDEX);
    if boundary == tick_current {
        return Err(ClmmMathError::NoProgress);
    }
    Ok((boundary, None))
}

/// Updates liquidity L when crossing an initialized tick boundary.
pub fn update_liquidity_on_cross(
    current_liquidity: u128,
    liquidity_net: i128,
    a_to_b: bool,
) -> Result<u128, ClmmMathError> {
    if a_to_b {
        if liquidity_net < 0 {
            current_liquidity
                .checked_add(liquidity_net.unsigned_abs())
                .ok_or(ClmmMathError::LiquidityOverflow)
        } else {
            current_liquidity
                .checked_sub(liquidity_net as u128)
                .ok_or(ClmmMathError::LiquidityOverflow)
        }
    } else if liquidity_net < 0 {
        current_liquidity
            .checked_sub(liquidity_net.unsigned_abs())
            .ok_or(ClmmMathError::LiquidityOverflow)
    } else {
        current_liquidity
            .checked_add(liquidity_net as u128)
            .ok_or(ClmmMathError::LiquidityOverflow)
    }
}

/// End-to-end multi-step swap calculation across tick arrays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WhirlpoolQuote {
    pub amount_in_consumed: u64,
    pub amount_out_received: u64,
    pub fee_paid: u64,
    pub final_sqrt_price_x64: u128,
    pub final_tick_index: i32,
    pub final_liquidity: u128,
    pub steps_taken: usize,
}

/// Compute exact swap quote traversing initialized ticks in local memory.
#[allow(clippy::too_many_arguments)]
pub fn compute_swap(
    mut sqrt_price_current_x64: u128,
    mut tick_current_index: i32,
    mut liquidity: u128,
    tick_spacing: u16,
    fee_rate: u16,
    amount_specified: u64,
    a_to_b: bool,
    tick_arrays: &[TickArrayFacade],
) -> Result<WhirlpoolQuote, ClmmMathError> {
    let mut amount_remaining = amount_specified;
    let mut total_amount_in = 0u64;
    let mut total_amount_out = 0u64;
    let mut total_fee = 0u64;
    let mut steps_taken = 0usize;
    let max_steps = tick_arrays
        .len()
        .checked_mul(TICK_ARRAY_SIZE + 1)
        .and_then(|n| n.checked_add(tick_arrays.len()))
        .ok_or(ClmmMathError::IntegerOverflow)?
        .max(1);

    while amount_remaining > 0 && liquidity > 0 {
        if steps_taken >= max_steps {
            return Err(ClmmMathError::IterationLimitExceeded);
        }
        let (next_tick, next_initialized_tick) =
            find_next_tick_boundary(tick_current_index, tick_spacing, a_to_b, tick_arrays)?;

        let target_sqrt_price = tick_index_to_sqrt_price_x64(next_tick)?;
        let previous_sqrt_price = sqrt_price_current_x64;
        let step = compute_swap_step_checked(
            previous_sqrt_price,
            target_sqrt_price,
            liquidity,
            amount_remaining,
            fee_rate,
            a_to_b,
        )?;

        if step.amount_in == 0
            && step.next_sqrt_price_x64 == previous_sqrt_price
            && next_initialized_tick.is_none()
        {
            return Err(ClmmMathError::NoProgress);
        }
        amount_remaining = amount_remaining
            .checked_sub(step.amount_in)
            .ok_or(ClmmMathError::IntegerOverflow)?;
        total_amount_in = total_amount_in
            .checked_add(step.amount_in)
            .ok_or(ClmmMathError::IntegerOverflow)?;
        total_amount_out = total_amount_out
            .checked_add(step.amount_out)
            .ok_or(ClmmMathError::IntegerOverflow)?;
        total_fee = total_fee
            .checked_add(step.fee_amount)
            .ok_or(ClmmMathError::IntegerOverflow)?;
        sqrt_price_current_x64 = step.next_sqrt_price_x64;
        steps_taken += 1;

        if sqrt_price_current_x64 == target_sqrt_price {
            if let Some(init_tick) = next_initialized_tick
                && init_tick.tick_index == next_tick
            {
                liquidity = update_liquidity_on_cross(liquidity, init_tick.liquidity_net, a_to_b)?;
            }
            tick_current_index = if a_to_b {
                next_tick.saturating_sub(1)
            } else {
                next_tick
            };
        } else {
            tick_current_index = sqrt_price_x64_to_tick_index(sqrt_price_current_x64)?;
            break;
        }
    }

    if amount_remaining > 0 {
        return Err(ClmmMathError::IncompleteQuote);
    }

    Ok(WhirlpoolQuote {
        amount_in_consumed: total_amount_in,
        amount_out_received: total_amount_out,
        fee_paid: total_fee,
        final_sqrt_price_x64: sqrt_price_current_x64,
        final_tick_index: tick_current_index,
        final_liquidity: liquidity,
        steps_taken,
    })
}

/// Convenient helper matching Whirlpool quote interface.
#[allow(clippy::too_many_arguments)]
pub fn quote_clmm(
    amount_in: u64,
    sqrt_price: u128,
    tick: i32,
    liquidity: u128,
    tick_spacing: u16,
    fee_rate: u16,
    a_to_b: bool,
    tick_arrays: &[TickArrayFacade],
) -> Result<u64, ClmmMathError> {
    let quote = compute_swap(
        sqrt_price,
        tick,
        liquidity,
        tick_spacing,
        fee_rate,
        amount_in,
        a_to_b,
        tick_arrays,
    )?;
    Ok(quote.amount_out_received)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_traversal_requires_current_array_coverage() {
        let err = find_next_tick_boundary(0, 1, false, &[]).unwrap_err();
        assert_eq!(err, ClmmMathError::MissingTickArrayCoverage(0));
    }

    #[test]
    fn tick_traversal_moves_to_array_edge_before_requesting_adjacent_array() {
        let array = TickArrayFacade {
            start_tick_index: 0,
            ticks: (0..TICK_ARRAY_SIZE)
                .map(|i| TickFacade {
                    tick_index: i as i32,
                    initialized: false,
                    liquidity_net: 0,
                    liquidity_gross: 0,
                })
                .collect(),
        };
        let (boundary, initialized) = find_next_tick_boundary(0, 1, false, &[array]).unwrap();
        assert_eq!(boundary, TICK_ARRAY_SIZE as i32);
        assert!(initialized.is_none());
        let err = find_next_tick_boundary(boundary, 1, false, &[]).unwrap_err();
        assert_eq!(
            err,
            ClmmMathError::MissingTickArrayCoverage(TICK_ARRAY_SIZE as i32)
        );
    }

    #[test]
    fn compute_swap_rejects_zero_progress_instead_of_returning_partial_quote() {
        let result = compute_swap(Q64, 0, 1_000_000_000, 1, 65_535, 1, true, &[]);
        assert_eq!(result, Err(ClmmMathError::MissingTickArrayCoverage(0)));
    }

    #[test]
    fn test_tick_index_to_sqrt_price_boundary_cases() {
        // Tick 0 -> sqrt_price = 1.0 in Q64.64 = 2^64 = 18446744073709551616
        let p0 = tick_index_to_sqrt_price_x64(0).expect("invariant");
        assert_eq!(p0, 18_446_744_073_709_551_616);

        // Min tick bounds
        let p_min = tick_index_to_sqrt_price_x64(MIN_TICK_INDEX).expect("invariant");
        assert!(p_min >= MIN_SQRT_PRICE_X64);
        assert!(p_min < p0);

        // Max tick bounds
        let p_max = tick_index_to_sqrt_price_x64(MAX_TICK_INDEX).expect("invariant");
        assert!(p_max <= MAX_SQRT_PRICE_X64);
        assert!(p_max > p0);

        // Out of bounds error
        assert!(tick_index_to_sqrt_price_x64(MIN_TICK_INDEX - 1).is_err());
        assert!(tick_index_to_sqrt_price_x64(MAX_TICK_INDEX + 1).is_err());
    }

    #[test]
    fn test_sqrt_price_to_tick_roundtrip() {
        for tick in [-100_000, -5000, -1, 0, 1, 5000, 100_000] {
            let p = tick_index_to_sqrt_price_x64(tick).expect("invariant");
            let recovered = sqrt_price_x64_to_tick_index(p).expect("invariant");
            assert_eq!(recovered, tick, "Failed roundtrip for tick {tick}");
        }
    }

    #[test]
    fn test_token_delta_rounding_solvency() {
        let p_lower = 18_446_744_073_709_551_616u128; // 1.0
        let p_upper = 18_446_744_073_709_551_616u128 + 100_000_000;
        let l = 1_000_000_000_000u128;

        // Inflow (round_up = true) must be >= outflow (round_up = false)
        let dx_in = get_amount_delta_a(p_lower, p_upper, l, true).expect("invariant");
        let dx_out = get_amount_delta_a(p_lower, p_upper, l, false).expect("invariant");
        assert!(dx_in >= dx_out);

        let dy_in = get_amount_delta_b(p_lower, p_upper, l, true).expect("invariant");
        let dy_out = get_amount_delta_b(p_lower, p_upper, l, false).expect("invariant");
        assert!(dy_in >= dy_out);
    }

    #[test]
    fn test_compute_swap_step_fee_deduction() {
        let p_curr = 18_446_744_073_709_551_616u128; // 1.0
        let p_target = 18_446_744_073_709_551_616u128 - 1_000_000; // a_to_b price drops
        let l = 50_000_000_000_000u128;
        let fee_rate = 3000; // 30 bps (0.3% = 3000 / 1_000_000)

        let res = compute_swap_step(p_curr, p_target, l, 100_000_000, fee_rate, true)
            .expect("valid checked swap step");
        assert!(res.amount_in <= 100_000_000);
        assert!(res.fee_amount > 0);
        assert!(res.amount_out > 0);
        assert_eq!(res.next_sqrt_price_x64, p_target);
    }

    #[test]
    fn test_strict_token_ordering() {
        let mint_sol = [1u8; 32];
        let mint_usdc = [2u8; 32];
        assert!(is_token_a(&mint_sol, &mint_usdc));
        assert!(!is_token_a(&mint_usdc, &mint_sol));
    }

    #[test]
    fn tick_spacing_requires_tick_array_coverage() {
        let tick = -15;
        let spacing = 64u16;
        let missing = ClmmMathError::MissingTickArrayCoverage(-5_632);
        assert_eq!(
            find_next_tick_boundary(tick, spacing, true, &[]),
            Err(missing.clone())
        );
        assert_eq!(
            find_next_tick_boundary(tick, spacing, false, &[]),
            Err(missing)
        );
    }

    #[test]
    fn test_clmm_max_delta_x_in_tick() {
        let p_curr = 18_446_744_073_709_551_616u128; // 1.0 (Q64.64)
        let p_lower = 18_446_744_073_709_551_616u128 - 1_000_000;
        let l = 100_000_000_000_000u128;

        let max_dx = clmm_max_delta_x_in_tick(p_curr, p_lower, l);
        assert!(max_dx.is_some());
        assert!(max_dx.unwrap() > 0);

        // Edge case: current <= lower returns 0
        assert_eq!(clmm_max_delta_x_in_tick(p_lower, p_curr, l), Some(0));
        assert_eq!(clmm_max_delta_x_in_tick(p_curr, p_curr, l), Some(0));
    }

    #[test]
    fn test_clmm_max_delta_y_in_tick() {
        let p_curr = 18_446_744_073_709_551_616u128; // 1.0 (Q64.64)
        let p_upper = 18_446_744_073_709_551_616u128 + 1_000_000;
        let l = 100_000_000_000_000u128;

        let max_dy = clmm_max_delta_y_in_tick(p_curr, p_upper, l);
        assert!(max_dy.is_some());
        assert!(max_dy.unwrap() > 0);

        // Edge case: current >= upper returns 0
        assert_eq!(clmm_max_delta_y_in_tick(p_upper, p_curr, l), Some(0));
        assert_eq!(clmm_max_delta_y_in_tick(p_curr, p_curr, l), Some(0));
    }

    #[test]
    fn test_clmm_two_sided_input_clamping() {
        let p_curr = 18_446_744_073_709_551_616u128;
        let p_lower = p_curr - 1_000_000;
        let p_upper = p_curr + 1_000_000;
        let l = 100_000_000_000_000u128;

        // a_to_b = true tests Delta X
        let in_a = clmm_max_input_in_tick(p_curr, p_lower, p_upper, l, true).unwrap();
        assert!(in_a > 0);

        // a_to_b = false tests Delta Y
        let in_b = clmm_max_input_in_tick(p_curr, p_lower, p_upper, l, false).unwrap();
        assert!(in_b > 0);

        // Active tick helper with tick inside [0, 64)
        let tick = 32i32;
        let p_at_tick = tick_index_to_sqrt_price_x64(tick).unwrap();
        let active_in_a = clmm_active_tick_max_input(p_at_tick, tick, 64, l, true).unwrap();
        let active_in_b = clmm_active_tick_max_input(p_at_tick, tick, 64, l, false).unwrap();
        assert!(active_in_a > 0);
        assert!(active_in_b > 0);
    }

    // -------------------------------------------------------------------------
    // Property-based tests (proptest)
    // -------------------------------------------------------------------------

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(2_000))]

        #[test]
        fn tick_roundtrip_stable(tick in -443_635i32..=443_635i32) {
            let p = tick_index_to_sqrt_price_x64(tick)
                .expect("tick inside MIN/MAX must convert");
            let recovered = sqrt_price_x64_to_tick_index(p)
                .expect("price produced from a valid tick must decode");
            // Q64.64 truncation may shift by at most one tick.
            proptest::prop_assert!(
                (recovered - tick).abs() <= 1,
                "tick {tick} -> price {p} -> tick {recovered}"
            );
        }
    }
}
