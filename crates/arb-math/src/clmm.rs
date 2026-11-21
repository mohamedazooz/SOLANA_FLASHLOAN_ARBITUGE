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

/// Shared validation floor for all supported CLMM programs (identical in Orca,
/// Raydium CLMM, and Meteora DAMM v2 sources). See
/// `arb_constants::CLMM_MIN_SQRT_PRICE_X64`.
pub const MIN_SQRT_PRICE_X64: u128 = 4_295_048_016;
/// Bit-exact official Orca Whirlpool `MAX_SQRT_PRICE_X64`
/// (`programs/whirlpool/src/math/tick_math.rs`). It is also the exact output
/// of `tick_index_to_sqrt_price_x64(MAX_TICK)` below.
pub const MAX_SQRT_PRICE_X64: u128 = 79_226_673_515_401_279_992_447_579_055;
pub const Q64: u128 = 1u128 << 64;

pub const FEE_RATE_DENOMINATOR: u64 = 1_000_000;
pub const TICK_ARRAY_SIZE: usize = 88;
pub const ORCA_WHIRLPOOL_TICK_ARRAY_SIZE: usize = 88;
pub const RAYDIUM_CLMM_TICK_ARRAY_SIZE: usize = 60;
pub const WHIRLPOOL_TICK_ARRAY_SIZE: usize = 9988;
pub const RAYDIUM_CLMM_TICK_ARRAY_ACCOUNT_SIZE: usize = 10240;
pub const TICK_STRUCT_SIZE: usize = 113;
pub const RAYDIUM_CLMM_TICK_STRUCT_SIZE: usize = 168;

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

    #[error(
        "Raydium CLMM limit-order tick {0} in swap path; limit-order matching is not implemented - fail-closed"
    )]
    LimitOrderTickInSwapPath(i32),
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

    if tick_index >= 0 {
        // Official `get_sqrt_price_positive_tick`: Q96.32 constants,
        // `mul_shift_96` per set bit, final `>> 32` to Q64.64.
        let tick = tick_index;
        let mut ratio: u128 = if tick & 1 != 0 {
            79_232_123_823_359_799_118_286_999_567
        } else {
            79_228_162_514_264_337_593_543_950_336
        };
        if tick & 2 != 0 {
            ratio = mul_shift_96(ratio, 79_236_085_330_515_764_027_303_304_731)?;
        }
        if tick & 4 != 0 {
            ratio = mul_shift_96(ratio, 79_244_008_939_048_815_603_706_035_061)?;
        }
        if tick & 8 != 0 {
            ratio = mul_shift_96(ratio, 79_259_858_533_276_714_757_314_932_305)?;
        }
        if tick & 16 != 0 {
            ratio = mul_shift_96(ratio, 79_291_567_232_598_584_799_939_703_904)?;
        }
        if tick & 32 != 0 {
            ratio = mul_shift_96(ratio, 79_355_022_692_464_371_645_785_046_466)?;
        }
        if tick & 64 != 0 {
            ratio = mul_shift_96(ratio, 79_482_085_999_252_804_386_437_311_141)?;
        }
        if tick & 128 != 0 {
            ratio = mul_shift_96(ratio, 79_736_823_300_114_093_921_829_183_326)?;
        }
        if tick & 256 != 0 {
            ratio = mul_shift_96(ratio, 80_248_749_790_819_932_309_965_073_892)?;
        }
        if tick & 512 != 0 {
            ratio = mul_shift_96(ratio, 81_282_483_887_344_747_381_513_967_011)?;
        }
        if tick & 1024 != 0 {
            ratio = mul_shift_96(ratio, 83_390_072_131_320_151_908_154_831_281)?;
        }
        if tick & 2048 != 0 {
            ratio = mul_shift_96(ratio, 87_770_609_709_833_776_024_991_924_138)?;
        }
        if tick & 4096 != 0 {
            ratio = mul_shift_96(ratio, 97_234_110_755_111_693_312_479_820_773)?;
        }
        if tick & 8192 != 0 {
            ratio = mul_shift_96(ratio, 119_332_217_159_966_728_226_237_229_890)?;
        }
        if tick & 16384 != 0 {
            ratio = mul_shift_96(ratio, 179_736_315_981_702_064_433_883_588_727)?;
        }
        if tick & 32768 != 0 {
            ratio = mul_shift_96(ratio, 407_748_233_172_238_350_107_850_275_304)?;
        }
        if tick & 65536 != 0 {
            ratio = mul_shift_96(ratio, 2_098_478_828_474_011_932_436_660_412_517)?;
        }
        if tick & 131072 != 0 {
            ratio = mul_shift_96(ratio, 55_581_415_166_113_811_149_459_800_483_533)?;
        }
        if tick & 262144 != 0 {
            ratio = mul_shift_96(ratio, 38_992_368_544_603_139_932_233_054_999_993_551)?;
        }
        return Ok(ratio >> 32);
    }

    // Official `get_sqrt_price_negative_tick`: Q64.64 constants and
    // `(ratio * c) >> 64` per set bit. All products fit in u128.
    let abs_tick = tick_index.unsigned_abs();
    let mut ratio: u128 = if abs_tick & 1 != 0 {
        18_445_821_805_675_392_311
    } else {
        18_446_744_073_709_551_616
    };
    if abs_tick & 2 != 0 {
        ratio = (ratio * 18_444_899_583_751_176_498) >> 64;
    }
    if abs_tick & 4 != 0 {
        ratio = (ratio * 18_443_055_278_223_354_162) >> 64;
    }
    if abs_tick & 8 != 0 {
        ratio = (ratio * 18_439_367_220_385_604_838) >> 64;
    }
    if abs_tick & 16 != 0 {
        ratio = (ratio * 18_431_993_317_065_449_817) >> 64;
    }
    if abs_tick & 32 != 0 {
        ratio = (ratio * 18_417_254_355_718_160_513) >> 64;
    }
    if abs_tick & 64 != 0 {
        ratio = (ratio * 18_387_811_781_193_591_352) >> 64;
    }
    if abs_tick & 128 != 0 {
        ratio = (ratio * 18_329_067_761_203_520_168) >> 64;
    }
    if abs_tick & 256 != 0 {
        ratio = (ratio * 18_212_142_134_806_087_854) >> 64;
    }
    if abs_tick & 512 != 0 {
        ratio = (ratio * 17_980_523_815_641_551_639) >> 64;
    }
    if abs_tick & 1024 != 0 {
        ratio = (ratio * 17_526_086_738_831_147_013) >> 64;
    }
    if abs_tick & 2048 != 0 {
        ratio = (ratio * 16_651_378_430_235_024_244) >> 64;
    }
    if abs_tick & 4096 != 0 {
        ratio = (ratio * 15_030_750_278_693_429_944) >> 64;
    }
    if abs_tick & 8192 != 0 {
        ratio = (ratio * 12_247_334_978_882_834_399) >> 64;
    }
    if abs_tick & 16384 != 0 {
        ratio = (ratio * 8_131_365_268_884_726_200) >> 64;
    }
    if abs_tick & 32768 != 0 {
        ratio = (ratio * 3_584_323_654_723_342_297) >> 64;
    }
    if abs_tick & 65536 != 0 {
        ratio = (ratio * 696_457_651_847_595_233) >> 64;
    }
    if abs_tick & 131072 != 0 {
        ratio = (ratio * 26_294_789_957_452_057) >> 64;
    }
    if abs_tick & 262144 != 0 {
        ratio = (ratio * 37_481_735_321_082) >> 64;
    }
    Ok(ratio)
}

/// Official Whirlpool `mul_shift_96`: `(n0 * n1) >> 96` in a 256-bit
/// intermediate. The product is guaranteed to fit in `u128` for every
/// constant used by the positive-tick table.
fn mul_shift_96(n0: u128, n1: u128) -> Result<u128, ClmmMathError> {
    let product = U256::from(n0) * U256::from(n1);
    u128::try_from(product >> 96).map_err(|_| ClmmMathError::IntegerOverflow)
}

/// Official Raydium CLMM `MAX_SQRT_PRICE_X64` (`libraries/tick_math.rs`).
pub const RAYDIUM_MAX_SQRT_PRICE_X64: u128 = 79_226_673_521_066_979_257_578_248_091;

/// Bit-exact official Raydium CLMM `get_sqrt_price_at_tick`.
///
/// Raydium uses **truncated Q64.64 magic constants** and a 128-bit
/// `U128::MAX / ratio` inversion for positive ticks, which is a different
/// rounding regime from Orca's Q96.32 `mul_shift_96` table. Sharing one tick
/// function between the two programs is therefore not bit-exact.
pub fn tick_index_to_sqrt_price_x64_raydium(tick_index: i32) -> Result<u128, ClmmMathError> {
    if !(MIN_TICK_INDEX..=MAX_TICK_INDEX).contains(&tick_index) {
        return Err(ClmmMathError::TickIndexOutOfBounds(tick_index));
    }
    let abs_tick = tick_index.unsigned_abs();
    let mut ratio: u128 = if abs_tick & 0x1 != 0 {
        0xfffc_b933_bd6f_b800
    } else {
        1u128 << 64
    };
    const MAGIC: [u128; 18] = [
        0xfff9_7272_373d_4000,
        0xfff2_e50f_5f65_7000,
        0xffe5_caca_7e10_f000,
        0xffcb_9843_d60f_7000,
        0xff97_3b41_fa98_e800,
        0xff2e_a164_66c9_b000,
        0xfe5d_ee04_6a9a_3800,
        0xfcbe_86c7_900b_b000,
        0xf987_a725_3ac6_5800,
        0xf339_2b08_22bb_6000,
        0xe715_9475_a2ca_f000,
        0xd097_f3bd_fd2f_2000,
        0xa9f7_4646_2d9f_8000,
        0x70d8_69a1_56f3_1c00,
        0x31be_135f_97ed_3200,
        0x09aa_508b_5b85_a500,
        0x005d_6af8_dedc_582c,
        0x0000_2216_e584_f5fa,
    ];
    for (bit, magic) in MAGIC.iter().enumerate() {
        let mask = 1u32 << (bit + 1);
        if abs_tick & mask != 0 {
            ratio = (ratio * magic) >> 64;
        }
    }
    if tick_index > 0 {
        ratio = u128::MAX / ratio;
    }
    Ok(ratio)
}

/// Exact inverse of the Raydium CLMM tick conversion (floor semantics).
pub fn sqrt_price_x64_to_tick_index_raydium(sqrt_price_x64: u128) -> Result<i32, ClmmMathError> {
    if !(MIN_SQRT_PRICE_X64..=RAYDIUM_MAX_SQRT_PRICE_X64).contains(&sqrt_price_x64) {
        return Err(ClmmMathError::SqrtPriceOutOfBounds);
    }
    let mut low = MIN_TICK_INDEX;
    let mut high = MAX_TICK_INDEX;
    while low <= high {
        let mid = low + (high - low) / 2;
        let p_mid = tick_index_to_sqrt_price_x64_raydium(mid)?;
        if p_mid <= sqrt_price_x64 {
            let next_p = if mid < MAX_TICK_INDEX {
                tick_index_to_sqrt_price_x64_raydium(mid + 1)?
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

/// Program-specific tick math selected by the quote path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClmmTickMath {
    Orca,
    Raydium,
}

impl ClmmTickMath {
    pub fn sqrt_price_from_tick(self, tick: i32) -> Result<u128, ClmmMathError> {
        match self {
            Self::Orca => tick_index_to_sqrt_price_x64(tick),
            Self::Raydium => tick_index_to_sqrt_price_x64_raydium(tick),
        }
    }

    pub fn tick_from_sqrt_price(self, sqrt_price_x64: u128) -> Result<i32, ClmmMathError> {
        match self {
            Self::Orca => sqrt_price_x64_to_tick_index(sqrt_price_x64),
            Self::Raydium => sqrt_price_x64_to_tick_index_raydium(sqrt_price_x64),
        }
    }
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

/// Fail-closed pool-state validation against Orca Whirlpools' official
/// `MIN/MAX_SQRT_PRICE` range. A decoded `sqrt_price_x64` of `0` (uninitialized
/// pool) is intentionally not accepted here; quote paths already reject it.
pub fn sqrt_price_x64_in_orca_whirlpool_bounds(sqrt_price_x64: u128) -> bool {
    (arb_constants::CLMM_MIN_SQRT_PRICE_X64..=arb_constants::ORCA_WHIRLPOOL_MAX_SQRT_PRICE_X64)
        .contains(&sqrt_price_x64)
}

/// Fail-closed pool-state validation against Raydium CLMM's official
/// `MIN/MAX_SQRT_PRICE_X64` range (`tick_math.rs`).
pub fn sqrt_price_x64_in_raydium_clmm_bounds(sqrt_price_x64: u128) -> bool {
    (arb_constants::CLMM_MIN_SQRT_PRICE_X64..=arb_constants::RAYDIUM_CLMM_MAX_SQRT_PRICE_X64)
        .contains(&sqrt_price_x64)
}

/// Fail-closed pool-state validation against Meteora DAMM v2's official
/// `MIN/MAX_SQRT_PRICE` range (`constants.rs`).
pub fn sqrt_price_x64_in_meteora_damm_v2_bounds(sqrt_price_x64: u128) -> bool {
    (arb_constants::CLMM_MIN_SQRT_PRICE_X64..=arb_constants::METEORA_DAMM_V2_MAX_SQRT_PRICE_X64)
        .contains(&sqrt_price_x64)
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
    let product = l * delta_p;
    let numerator = product << 64;
    // Official `get_amount_delta_a` calls `checked_shl(64)` on the 256-bit
    // product and fails with `ShiftOverflow` when bits would be discarded.
    if (numerator >> 64) != product {
        return Err(ClmmMathError::IntegerOverflow);
    }
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

/// Wide (u128) amount ΔA between two sqrt prices. A `u128::MAX` result means
/// the amount is not representable in u128 — the step consumer only compares
/// it against a remaining input that fits u64, so the sentinel always means
/// "target unreachable within the remaining amount" (the official SDK's
/// `calculateAmountInRange` catches its own `MaxTokenOverflow` the same way
/// and completes the step from the price move alone).
fn amount_delta_a_wide(
    sqrt_price_0: u128,
    sqrt_price_1: u128,
    liquidity: u128,
    round_up: bool,
) -> u128 {
    if sqrt_price_0 == 0 || sqrt_price_1 == 0 || liquidity == 0 {
        return 0;
    }
    let (sqrt_price_lower, sqrt_price_upper) = if sqrt_price_0 < sqrt_price_1 {
        (sqrt_price_0, sqrt_price_1)
    } else {
        (sqrt_price_1, sqrt_price_0)
    };

    let delta_p = U256::from(sqrt_price_upper - sqrt_price_lower);
    let l = U256::from(liquidity);
    let product = l * delta_p;
    let numerator = product << 64;
    if (numerator >> 64) != product {
        return u128::MAX;
    }
    let denominator = U256::from(sqrt_price_lower) * U256::from(sqrt_price_upper);
    if denominator.is_zero() {
        return u128::MAX;
    }
    let mut amount = numerator / denominator;
    if round_up && numerator % denominator != U256::ZERO {
        amount += U256::from(1u8);
    }
    u128::try_from(amount).unwrap_or(u128::MAX)
}

/// Wide (u128) amount ΔB between two sqrt prices; sentinel semantics as in
/// [`amount_delta_a_wide`].
fn amount_delta_b_wide(
    sqrt_price_0: u128,
    sqrt_price_1: u128,
    liquidity: u128,
    round_up: bool,
) -> u128 {
    if sqrt_price_0 == 0 || sqrt_price_1 == 0 || liquidity == 0 {
        return 0;
    }
    let (sqrt_price_lower, sqrt_price_upper) = if sqrt_price_0 < sqrt_price_1 {
        (sqrt_price_0, sqrt_price_1)
    } else {
        (sqrt_price_1, sqrt_price_0)
    };

    let delta_p = U256::from(sqrt_price_upper - sqrt_price_lower);
    let l = U256::from(liquidity);
    let prod = l * delta_p;
    let mut amount = prod >> 64;
    if round_up && prod % U256::from(1u128 << 64) != U256::ZERO {
        amount += U256::from(1u8);
    }
    u128::try_from(amount).unwrap_or(u128::MAX)
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
    if amount_remaining == 0 {
        return Ok(StepSwapResult {
            amount_in: 0,
            amount_out: 0,
            next_sqrt_price_x64: sqrt_price_current_x64,
            sqrt_price_next_x64: sqrt_price_current_x64,
            fee_amount: 0,
        });
    }
    if u64::from(fee_rate) >= FEE_RATE_DENOMINATOR {
        return Err(ClmmMathError::DivisionByZero);
    }

    // Official `compute_swap`:
    //   amount_remaining_less_fee = checked_mul_div(amount_remaining,
    //       FEE_RATE_MUL_VALUE - fee_rate, FEE_RATE_MUL_VALUE)   // floor
    let amount_remaining_less_fee = u64::try_from(
        u128::from(amount_remaining) * u128::from(FEE_RATE_DENOMINATOR - u64::from(fee_rate))
            / u128::from(FEE_RATE_DENOMINATOR),
    )
    .map_err(|_| ClmmMathError::IntegerOverflow)?;

    // Amount needed to move all the way to the target tick (rounded up).
    // Computed wide first: with concentrated liquidity the gap can exceed
    // u64, which the official SDK survives (its `calculateAmountInRange`
    // catches `MaxTokenOverflow` and runs a partial step from the price
    // move); a naive u64 computation here would wrongly fail the quote.
    let amount_in_max_wide = if a_to_b {
        amount_delta_a_wide(
            sqrt_price_target_x64,
            sqrt_price_current_x64,
            liquidity,
            true,
        )
    } else {
        amount_delta_b_wide(
            sqrt_price_current_x64,
            sqrt_price_target_x64,
            liquidity,
            true,
        )
    };

    if u128::from(amount_remaining_less_fee) >= amount_in_max_wide {
        // Full step reaches the target tick. Fee is grossed up over the net
        // amount, exactly like the on-chain program. The gap amount fits u64
        // here by construction (it is tracked by `amount_remaining`), so the
        // narrow re-derivation below cannot overflow.
        let amount_in_max = if a_to_b {
            get_amount_delta_a(
                sqrt_price_target_x64,
                sqrt_price_current_x64,
                liquidity,
                true,
            )?
        } else {
            get_amount_delta_b(
                sqrt_price_current_x64,
                sqrt_price_target_x64,
                liquidity,
                true,
            )?
        };
        let fee_amount = div_ceil_u64(
            u128::from(amount_in_max) * u128::from(fee_rate),
            u128::from(FEE_RATE_DENOMINATOR - u64::from(fee_rate)),
        )?;
        let amount_in = amount_in_max
            .checked_add(fee_amount)
            .ok_or(ClmmMathError::IntegerOverflow)?;

        let amount_out = if a_to_b {
            get_amount_delta_b(
                sqrt_price_target_x64,
                sqrt_price_current_x64,
                liquidity,
                false,
            )?
        } else {
            get_amount_delta_a(
                sqrt_price_current_x64,
                sqrt_price_target_x64,
                liquidity,
                false,
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
        // Partial step: consume all remaining input inside this tick. The
        // realised input is re-derived from the actual price move (it can be
        // one unit below `amount_remaining_less_fee`), and the fee is the
        // exact remainder — never an independently rounded fee.
        let next_sqrt_price = if a_to_b {
            get_next_sqrt_price_from_a_input(
                sqrt_price_current_x64,
                liquidity,
                amount_remaining_less_fee,
            )?
        } else {
            get_next_sqrt_price_from_b_input(
                sqrt_price_current_x64,
                liquidity,
                amount_remaining_less_fee,
            )?
        };

        let amount_in_net = if a_to_b {
            get_amount_delta_a(next_sqrt_price, sqrt_price_current_x64, liquidity, true)?
        } else {
            get_amount_delta_b(sqrt_price_current_x64, next_sqrt_price, liquidity, true)?
        };

        let amount_out = if a_to_b {
            get_amount_delta_b(next_sqrt_price, sqrt_price_current_x64, liquidity, false)?
        } else {
            get_amount_delta_a(sqrt_price_current_x64, next_sqrt_price, liquidity, false)?
        };

        // `amount_in` is the GROSS input consumed by this step (net swap
        // amount + fee), matching the quote accumulator's contract.
        let fee_amount = amount_remaining
            .checked_sub(amount_in_net)
            .ok_or(ClmmMathError::IntegerOverflow)?;

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
    /// Raydium CLMM limit orders resident at this tick
    /// (`orders_amount + part_filled_orders_remaining`, official
    /// `limit_order_unfilled_amount`). Always 0 for Orca Whirlpool arrays.
    pub limit_order_unfilled: u64,
}

/// In-memory deserialized representation of a Whirlpool TickArray account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TickArrayFacade {
    pub start_tick_index: i32,
    pub ticks: Vec<TickFacade>,
}

/// `Tick::check_is_valid_start_tick` from the Whirlpool program: a start tick
/// must be a multiple of the tick spacing and inside the global tick range,
/// except for the single left-edge array whose start is below `MIN_TICK_INDEX`.
pub fn is_valid_tick_array_start_tick(start_tick_index: i32, tick_spacing: u16) -> bool {
    if tick_spacing == 0 {
        return false;
    }
    let ticks_in_array = (TICK_ARRAY_SIZE as i32) * i32::from(tick_spacing);
    if !(MIN_TICK_INDEX..=MAX_TICK_INDEX).contains(&start_tick_index) {
        if start_tick_index > MIN_TICK_INDEX {
            return false;
        }
        // Exact official expression (truncating `%`, not `rem_euclid`).
        let min_array_start_index =
            MIN_TICK_INDEX - (MIN_TICK_INDEX % ticks_in_array + ticks_in_array);
        return start_tick_index == min_array_start_index;
    }
    start_tick_index.rem_euclid(ticks_in_array) == 0
}

/// The EXACT ordered tick-array start indexes the on-chain Whirlpool `swap`
/// instruction requires.
///
/// Mirrors `SparseSwapTickSequenceBuilder::get_start_tick_indexes` in the
/// official program (`programs/whirlpool/src/util/sparse_swap.rs`):
///
/// * `a_to_b`  -> `[base, base - T, base - 2T]` (never shifted, even when
///   `tick_current == base`).
/// * `b_to_a`  -> `[base, base + T, base + 2T]`, or `[base + T, base + 2T,
///   base + 3T]` when `tick_current + tick_spacing` reaches the array's upper
///   edge.
///
/// `T = 88 * tick_spacing`. Invalid start ticks are dropped exactly like the
/// program does (the sequence then ends early). An empty result means no
/// usable Whirlpool tick array exists near the current tick.
pub fn orca_tick_array_start_indexes(
    tick_current: i32,
    tick_spacing: u16,
    a_to_b: bool,
) -> Vec<i32> {
    if tick_spacing == 0 {
        return Vec::new();
    }
    let spacing = i32::from(tick_spacing);
    let ticks_in_array = (TICK_ARRAY_SIZE as i32) * spacing;
    let base = tick_current.div_euclid(ticks_in_array) * ticks_in_array;
    let offsets: [i32; 3] = if a_to_b {
        [0, -1, -2]
    } else if tick_current.saturating_add(spacing) >= base.saturating_add(ticks_in_array) {
        [1, 2, 3]
    } else {
        [0, 1, 2]
    };
    offsets
        .iter()
        .filter_map(|&offset| {
            let start_tick = base.checked_add(offset.checked_mul(ticks_in_array)?)?;
            is_valid_tick_array_start_tick(start_tick, tick_spacing).then_some(start_tick)
        })
        .collect()
}

/// Calculate the start tick index for a tick given Whirlpool tick spacing.
pub fn get_start_tick_index(tick: i32, tick_spacing: u16) -> i32 {
    get_start_tick_index_with_size(tick, tick_spacing, TICK_ARRAY_SIZE)
}

/// Calculate the start tick index for a tick given custom ticks per array (e.g. 88 for Whirlpool, 60 for Raydium CLMM).
pub fn get_start_tick_index_with_size(
    tick: i32,
    tick_spacing: u16,
    ticks_per_array_count: usize,
) -> i32 {
    let ticks_per_array = (ticks_per_array_count as i32) * i32::from(tick_spacing.max(1));
    tick.div_euclid(ticks_per_array) * ticks_per_array
}

/// Parse an on-chain Whirlpool TickArray account (9,988 bytes).
///
/// Official fixed layout (`FixedTickArray`):
/// `[0..8]` Anchor discriminator, `[8..12]` `start_tick_index: i32`,
/// `[12..9956]` 88 packed `Tick` records of 113 bytes each,
/// `[9956..9988]` the owning `whirlpool: Pubkey`.
pub fn parse_tick_array_account(
    data: &[u8],
    tick_spacing: u16,
) -> Result<TickArrayFacade, ClmmMathError> {
    if data.len() < WHIRLPOOL_TICK_ARRAY_SIZE {
        return Err(ClmmMathError::InvalidAccountData);
    }
    let start_tick_bytes: [u8; 4] = data[8..12]
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
        let offset = 12 + i * TICK_STRUCT_SIZE;
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
            limit_order_unfilled: 0,
        });
    }

    Ok(TickArrayFacade {
        start_tick_index,
        ticks,
    })
}

/// Parse an on-chain Raydium CLMM TickArrayState account (10,240 bytes, 60 ticks of 168 bytes).
pub fn parse_raydium_clmm_tick_array_account(
    data: &[u8],
    tick_spacing: u16,
) -> Result<TickArrayFacade, ClmmMathError> {
    if data.len() < RAYDIUM_CLMM_TICK_ARRAY_ACCOUNT_SIZE {
        return Err(ClmmMathError::InvalidAccountData);
    }
    let start_tick_bytes: [u8; 4] = data[40..44]
        .try_into()
        .map_err(|_| ClmmMathError::InvalidAccountData)?;
    if tick_spacing == 0 {
        return Err(ClmmMathError::InvalidAccountData);
    }
    let start_tick_index = i32::from_le_bytes(start_tick_bytes);

    let mut ticks = Vec::with_capacity(RAYDIUM_CLMM_TICK_ARRAY_SIZE);
    let spacing = i32::from(tick_spacing);
    let ticks_per_array = (RAYDIUM_CLMM_TICK_ARRAY_SIZE as i32)
        .checked_mul(spacing)
        .ok_or(ClmmMathError::IntegerOverflow)?;
    if start_tick_index.rem_euclid(ticks_per_array) != 0 {
        return Err(ClmmMathError::InvalidAccountData);
    }

    for i in 0..RAYDIUM_CLMM_TICK_ARRAY_SIZE {
        let offset = 44 + i * RAYDIUM_CLMM_TICK_STRUCT_SIZE;
        let tick_bytes: [u8; 4] = data
            .get(offset..offset + 4)
            .ok_or(ClmmMathError::InvalidAccountData)?
            .try_into()
            .map_err(|_| ClmmMathError::InvalidAccountData)?;
        let tick_index = i32::from_le_bytes(tick_bytes);

        let net_bytes: [u8; 16] = data
            .get(offset + 4..offset + 20)
            .ok_or(ClmmMathError::InvalidAccountData)?
            .try_into()
            .map_err(|_| ClmmMathError::InvalidAccountData)?;
        let liquidity_net = i128::from_le_bytes(net_bytes);

        let gross_bytes: [u8; 16] = data
            .get(offset + 20..offset + 36)
            .ok_or(ClmmMathError::InvalidAccountData)?
            .try_into()
            .map_err(|_| ClmmMathError::InvalidAccountData)?;
        let liquidity_gross = u128::from_le_bytes(gross_bytes);

        // Limit-order fields at the end of TickState (VERIFIED against the
        // deployed program layout, checked 2026-11-20): order_phase @116,
        // orders_amount @124, part_filled_orders_remaining @132,
        // unfilled_ratio_x64 @140, padding [u32;3] @156..168.
        let orders_u = u64::from_le_bytes(
            data.get(offset + 124..offset + 132)
                .ok_or(ClmmMathError::InvalidAccountData)?
                .try_into()
                .map_err(|_| ClmmMathError::InvalidAccountData)?,
        );
        let part_filled_u = u64::from_le_bytes(
            data.get(offset + 132..offset + 140)
                .ok_or(ClmmMathError::InvalidAccountData)?
                .try_into()
                .map_err(|_| ClmmMathError::InvalidAccountData)?,
        );
        let limit_order_unfilled = orders_u
            .checked_add(part_filled_u)
            .ok_or(ClmmMathError::IntegerOverflow)?;
        // Official `is_initialized()`: has_liquidity() || has_limit_orders() --
        // a tick holding limit orders must stop the traversal even with zero
        // liquidity.
        let initialized = liquidity_gross > 0 || limit_order_unfilled > 0;

        ticks.push(TickFacade {
            tick_index,
            initialized,
            liquidity_net,
            liquidity_gross,
            limit_order_unfilled,
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
    find_next_tick_boundary_with_size(
        tick_current,
        tick_spacing,
        a_to_b,
        tick_arrays,
        TICK_ARRAY_SIZE,
    )
}

/// Find next tick boundary with custom ticks_per_array_count (e.g. 88 for Whirlpool, 60 for Raydium).
pub fn find_next_tick_boundary_with_size(
    tick_current: i32,
    tick_spacing: u16,
    a_to_b: bool,
    tick_arrays: &[TickArrayFacade],
    ticks_per_array_count: usize,
) -> Result<(i32, Option<TickFacade>), ClmmMathError> {
    if tick_spacing == 0 {
        return Err(ClmmMathError::InvalidAccountData);
    }
    if tick_arrays.is_empty() {
        return Err(ClmmMathError::MissingTickArrayCoverage(
            get_start_tick_index_with_size(tick_current, tick_spacing, ticks_per_array_count),
        ));
    }

    let spacing = i32::from(tick_spacing);
    let ticks_per_array = (ticks_per_array_count as i32)
        .checked_mul(spacing)
        .ok_or(ClmmMathError::IntegerOverflow)?;
    let mut current_array_start =
        get_start_tick_index_with_size(tick_current, tick_spacing, ticks_per_array_count);

    loop {
        let current_array = tick_arrays
            .iter()
            .find(|array| array.start_tick_index == current_array_start)
            .ok_or(ClmmMathError::MissingTickArrayCoverage(current_array_start))?;

        let candidate = if a_to_b {
            current_array
                .ticks
                .iter()
                .filter(|tick| {
                    tick.initialized
                        && tick.tick_index <= tick_current
                        && tick.tick_index >= current_array_start
                })
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

        // No initialized tick remains in this array in the swap direction.
        // Official semantics (Raydium `swapSimulator.ts` walks the
        // caller-ordered loaded-array list; whirlpools form the same list):
        // the price only ever moves to the next INITIALIZED tick, so the
        // traversal jumps straight to the next LOADED array in the swap
        // direction and never needs the literal adjacent array — empty spans
        // spanned by unallocated (closed) tick arrays are crossed without any
        // account. The Whirlpool list `[base, base - T, base - 2T]` is exactly
        // the nearest-lower case here, so adjacency semantics are unchanged
        // for contiguous lists.
        let next_start = if a_to_b {
            tick_arrays
                .iter()
                .map(|array| array.start_tick_index)
                .filter(|start| *start < current_array_start)
                .max()
        } else {
            tick_arrays
                .iter()
                .map(|array| array.start_tick_index)
                .filter(|start| *start > current_array_start)
                .min()
        };

        if let Some(next_start) = next_start {
            current_array_start = next_start;
            continue;
        }

        // Loaded sequence exhausted in the swap direction.
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
        return Ok((boundary, None));
    }
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

/// Compute exact swap quote traversing initialized ticks in local memory (defaulting to Whirlpool 88 ticks/array).
#[allow(clippy::too_many_arguments)]
pub fn compute_swap(
    sqrt_price_current_x64: u128,
    tick_current_index: i32,
    liquidity: u128,
    tick_spacing: u16,
    fee_rate: u16,
    amount_specified: u64,
    a_to_b: bool,
    tick_arrays: &[TickArrayFacade],
) -> Result<WhirlpoolQuote, ClmmMathError> {
    compute_swap_with_ticks_per_array(
        sqrt_price_current_x64,
        tick_current_index,
        liquidity,
        tick_spacing,
        fee_rate,
        amount_specified,
        a_to_b,
        tick_arrays,
        TICK_ARRAY_SIZE,
    )
}

/// Compute exact swap quote with custom ticks per array count (88 for Orca, 60 for Raydium CLMM).
#[allow(clippy::too_many_arguments)]
pub fn compute_swap_with_ticks_per_array(
    sqrt_price_current_x64: u128,
    tick_current_index: i32,
    liquidity: u128,
    tick_spacing: u16,
    fee_rate: u16,
    amount_specified: u64,
    a_to_b: bool,
    tick_arrays: &[TickArrayFacade],
    ticks_per_array_count: usize,
) -> Result<WhirlpoolQuote, ClmmMathError> {
    compute_swap_with_ticks_per_array_and_math(
        sqrt_price_current_x64,
        tick_current_index,
        liquidity,
        tick_spacing,
        fee_rate,
        amount_specified,
        a_to_b,
        tick_arrays,
        ticks_per_array_count,
        ClmmTickMath::Orca,
    )
}

/// Program-specific variant of [`compute_swap_with_ticks_per_array`].
#[allow(clippy::too_many_arguments)]
pub fn compute_swap_with_ticks_per_array_and_math(
    mut sqrt_price_current_x64: u128,
    mut tick_current_index: i32,
    mut liquidity: u128,
    tick_spacing: u16,
    fee_rate: u16,
    amount_specified: u64,
    a_to_b: bool,
    tick_arrays: &[TickArrayFacade],
    ticks_per_array_count: usize,
    tick_math: ClmmTickMath,
) -> Result<WhirlpoolQuote, ClmmMathError> {
    let mut amount_remaining = amount_specified;
    let mut total_amount_in = 0u64;
    let mut total_amount_out = 0u64;
    let mut total_fee = 0u64;
    let mut steps_taken = 0usize;
    let max_steps = tick_arrays
        .len()
        .checked_mul(ticks_per_array_count + 1)
        .and_then(|n| n.checked_add(tick_arrays.len()))
        .ok_or(ClmmMathError::IntegerOverflow)?
        .max(1);

    // The on-chain loop runs until the input is consumed or the price limit is
    // reached; it does NOT stop when active liquidity is zero. A zero-liquidity
    // range is traversed for free to the next initialized tick / array edge.
    while amount_remaining > 0 {
        if steps_taken >= max_steps {
            return Err(ClmmMathError::IterationLimitExceeded);
        }
        let (next_tick, next_initialized_tick) = find_next_tick_boundary_with_size(
            tick_current_index,
            tick_spacing,
            a_to_b,
            tick_arrays,
            ticks_per_array_count,
        )?;

        let target_sqrt_price = tick_math.sqrt_price_from_tick(next_tick)?;
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
                // On-chain, a tick holding unfilled Raydium CLMM limit orders
                // matches those orders at the exact tick price and changes the
                // swap's tick-advance convention; a pure-AMM traversal must
                // not silently skip that.
                if init_tick.limit_order_unfilled != 0 {
                    return Err(ClmmMathError::LimitOrderTickInSwapPath(
                        init_tick.tick_index,
                    ));
                }
                liquidity = update_liquidity_on_cross(liquidity, init_tick.liquidity_net, a_to_b)?;
            }
            tick_current_index = if a_to_b {
                next_tick.saturating_sub(1)
            } else {
                next_tick
            };
        } else {
            tick_current_index = tick_math.tick_from_sqrt_price(sqrt_price_current_x64)?;
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

/// Exact Orca Whirlpool swap traversal, ported verbatim from the official
/// whirlpools-core engine (`rust-sdk/core/src/quote/swap.rs` and
/// `math/tick_array.rs`, tag @orca-so/whirlpools-core v3.1.1).
///
/// Unlike [`compute_swap_with_ticks_per_array`], which steps array-edge to
/// array-edge, this walks the loaded tick-array sequence as one span: the step
/// target is the next initialized tick in the swap direction (sequence start
/// for `a_to_b`, sequence end for `b_to_a` when no further initialized tick
/// exists), with the official step split (`calc = remaining * (D - fee) / D`,
/// ceiling fee extraction on initialization-boundary hits, round-down output).
/// All arithmetic stays checked and fail-closed.
#[allow(clippy::too_many_arguments)]
pub fn compute_whirlpool_swap(
    mut sqrt_price_current_x64: u128,
    mut tick_current_index: i32,
    mut liquidity: u128,
    tick_spacing: u16,
    fee_rate: u16,
    amount_specified: u64,
    a_to_b: bool,
    tick_arrays: &[TickArrayFacade],
) -> Result<WhirlpoolQuote, ClmmMathError> {
    if tick_spacing == 0 {
        return Err(ClmmMathError::InvalidAccountData);
    }
    if fee_rate as u64 >= FEE_RATE_DENOMINATOR {
        return Err(ClmmMathError::InvalidAccountData);
    }
    let spacing = i32::from(tick_spacing);
    let array_span = (TICK_ARRAY_SIZE as i32)
        .checked_mul(spacing)
        .ok_or(ClmmMathError::IntegerOverflow)?;
    let mut starts = tick_arrays.iter().map(|array| array.start_tick_index);
    let seq_start = starts
        .next()
        .ok_or(ClmmMathError::MissingTickArrayCoverage(
            get_start_tick_index(tick_current_index, tick_spacing),
        ))?;
    let mut seq_start = seq_start;
    let mut seq_end = seq_start;
    for start in starts {
        seq_start = seq_start.min(start);
        seq_end = seq_end.max(start);
    }
    seq_end = seq_end
        .checked_add(array_span)
        .ok_or(ClmmMathError::IntegerOverflow)?
        .saturating_sub(1);

    let initialized_tick_at = |index: i32| -> Option<&TickFacade> {
        let array = tick_arrays.iter().find(|array| {
            index >= array.start_tick_index
                && index
                    < array
                        .start_tick_index
                        .checked_add(array_span)
                        .unwrap_or(i32::MAX)
        })?;
        let offset = index.checked_sub(array.start_tick_index)?;
        if offset < 0 || offset % spacing != 0 {
            return None;
        }
        array.ticks.get((offset / spacing) as usize)
    };

    // Official `TickArraySequence` next/prev-initialized-tick semantics: scan
    // the full sequence on the spacing grid; when no initialized tick survives,
    // the boundary is the sequence start (a_to_b) or end (b_to_a), not the
    // current array's edge. Out-of-sequence positions are fail-closed.
    let next_initialized = |tick_index: i32| -> Result<(Option<i128>, i32), ClmmMathError> {
        if tick_index >= seq_end {
            return Err(ClmmMathError::MissingTickArrayCoverage(
                seq_end.saturating_add(1),
            ));
        }
        let mut next = tick_index
            .div_euclid(spacing)
            .checked_add(1)
            .and_then(|n| n.checked_mul(spacing))
            .ok_or(ClmmMathError::IntegerOverflow)?;
        loop {
            if next > seq_end {
                return Ok((None, seq_end));
            }
            if let Some(tick) = initialized_tick_at(next)
                && tick.initialized
            {
                return Ok((Some(tick.liquidity_net), next));
            }
            next = next
                .checked_add(spacing)
                .ok_or(ClmmMathError::IntegerOverflow)?;
            if next > MAX_TICK_INDEX {
                return Err(ClmmMathError::IncompleteQuote);
            }
        }
    };
    let prev_initialized = |tick_index: i32| -> Result<(Option<i128>, i32), ClmmMathError> {
        if tick_index < seq_start {
            return Err(ClmmMathError::MissingTickArrayCoverage(
                seq_start.saturating_sub(array_span),
            ));
        }
        let mut prev = tick_index
            .div_euclid(spacing)
            .checked_mul(spacing)
            .ok_or(ClmmMathError::IntegerOverflow)?;
        loop {
            if prev < seq_start {
                return Ok((None, seq_start));
            }
            if let Some(tick) = initialized_tick_at(prev)
                && tick.initialized
            {
                return Ok((Some(tick.liquidity_net), prev));
            }
            prev = prev
                .checked_sub(spacing)
                .ok_or(ClmmMathError::IntegerOverflow)?;
            if prev < MIN_TICK_INDEX {
                return Err(ClmmMathError::IncompleteQuote);
            }
        }
    };

    let fee_denom = u128::from(FEE_RATE_DENOMINATOR);
    let fee_num = u128::from(fee_rate);
    let mut amount_remaining = amount_specified;
    let mut total_amount_out = 0u64;
    let mut total_fee = 0u64;
    let mut steps_taken = 0usize;
    let max_steps = tick_arrays
        .len()
        .checked_mul(tick_arrays.len().saturating_mul(2) + TICK_ARRAY_SIZE + 1)
        .and_then(|n| n.checked_add(tick_arrays.len()))
        .ok_or(ClmmMathError::IntegerOverflow)?
        .max(1);

    while amount_remaining > 0 {
        if steps_taken >= max_steps {
            return Err(ClmmMathError::IterationLimitExceeded);
        }
        let (init_net, next_tick_index) = if a_to_b {
            prev_initialized(tick_current_index)?
        } else {
            next_initialized(tick_current_index)?
        };
        let next_tick_sqrt = tick_index_to_sqrt_price_x64(next_tick_index)?;

        // Official `compute_swap` inner loop: bounded to the current
        // initialized-tick target (or sequence boundary); a boundary hit with
        // unconsumed input re-enters the search, exactly as the SDK does.
        loop {
            if steps_taken >= max_steps {
                return Err(ClmmMathError::IterationLimitExceeded);
            }
            let calc =
                u64::try_from(u128::from(amount_remaining) * (fee_denom - fee_num) / fee_denom)
                    .map_err(|_| ClmmMathError::IntegerOverflow)?;
            let initial_input = if a_to_b {
                get_amount_delta_a(sqrt_price_current_x64, next_tick_sqrt, liquidity, true)?
            } else {
                get_amount_delta_b(sqrt_price_current_x64, next_tick_sqrt, liquidity, true)?
            };
            let next_sqrt = if u128::from(initial_input) <= u128::from(calc) {
                next_tick_sqrt
            } else if a_to_b {
                get_next_sqrt_price_from_a_input(sqrt_price_current_x64, liquidity, calc)?
            } else {
                get_next_sqrt_price_from_b_input(sqrt_price_current_x64, liquidity, calc)?
            };
            let is_max = next_sqrt == next_tick_sqrt;
            let amount_out = if a_to_b {
                get_amount_delta_b(sqrt_price_current_x64, next_sqrt, liquidity, false)?
            } else {
                get_amount_delta_a(sqrt_price_current_x64, next_sqrt, liquidity, false)?
            };
            let amount_in = if is_max {
                initial_input
            } else if a_to_b {
                get_amount_delta_a(sqrt_price_current_x64, next_sqrt, liquidity, true)?
            } else {
                get_amount_delta_b(sqrt_price_current_x64, next_sqrt, liquidity, true)?
            };
            let fee = if is_max {
                u64::try_from((u128::from(amount_in) * fee_num).div_ceil(fee_denom - fee_num))
                    .map_err(|_| ClmmMathError::IntegerOverflow)?
            } else {
                amount_remaining
                    .checked_sub(amount_in)
                    .ok_or(ClmmMathError::IntegerOverflow)?
            };
            if fee > amount_remaining
                || amount_in
                    > amount_remaining
                        .checked_sub(fee)
                        .ok_or(ClmmMathError::IntegerOverflow)?
            {
                return Err(ClmmMathError::IntegerOverflow);
            }

            amount_remaining = amount_remaining
                .checked_sub(amount_in)
                .and_then(|v| v.checked_sub(fee))
                .ok_or(ClmmMathError::IntegerOverflow)?;
            total_amount_out = total_amount_out
                .checked_add(amount_out)
                .ok_or(ClmmMathError::IntegerOverflow)?;
            total_fee = total_fee
                .checked_add(fee)
                .ok_or(ClmmMathError::IntegerOverflow)?;
            let previous_sqrt_price = sqrt_price_current_x64;
            sqrt_price_current_x64 = next_sqrt;
            steps_taken += 1;

            if next_sqrt == next_tick_sqrt {
                if let Some(net) = init_net {
                    liquidity = update_liquidity_on_cross(liquidity, net, a_to_b)?;
                }
                tick_current_index = if a_to_b {
                    next_tick_index.saturating_sub(1)
                } else {
                    next_tick_index
                };
            } else if next_sqrt != previous_sqrt_price {
                tick_current_index = sqrt_price_x64_to_tick_index(next_sqrt)?;
            }
            if amount_remaining == 0 || sqrt_price_current_x64 == next_tick_sqrt {
                break;
            }
        }
    }

    Ok(WhirlpoolQuote {
        amount_in_consumed: amount_specified
            .checked_sub(amount_remaining)
            .ok_or(ClmmMathError::IntegerOverflow)?,
        amount_out_received: total_amount_out,
        fee_paid: total_fee,
        final_sqrt_price_x64: sqrt_price_current_x64,
        final_tick_index: tick_current_index,
        final_liquidity: liquidity,
        steps_taken,
    })
}

/// Convenient helper matching Whirlpool quote interface (88 ticks/array).
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
    quote_clmm_with_ticks_per_array(
        amount_in,
        sqrt_price,
        tick,
        liquidity,
        tick_spacing,
        fee_rate,
        a_to_b,
        tick_arrays,
        TICK_ARRAY_SIZE,
    )
}

/// General CLMM quote helper supporting custom ticks per array (e.g. 60 for Raydium CLMM).
#[allow(clippy::too_many_arguments)]
pub fn quote_clmm_with_ticks_per_array(
    amount_in: u64,
    sqrt_price: u128,
    tick: i32,
    liquidity: u128,
    tick_spacing: u16,
    fee_rate: u16,
    a_to_b: bool,
    tick_arrays: &[TickArrayFacade],
    ticks_per_array_count: usize,
) -> Result<u64, ClmmMathError> {
    quote_clmm_with_ticks_per_array_and_math(
        amount_in,
        sqrt_price,
        tick,
        liquidity,
        tick_spacing,
        fee_rate,
        a_to_b,
        tick_arrays,
        ticks_per_array_count,
        ClmmTickMath::Orca,
    )
}

/// Program-specific variant of [`quote_clmm_with_ticks_per_array`].
#[allow(clippy::too_many_arguments)]
pub fn quote_clmm_with_ticks_per_array_and_math(
    amount_in: u64,
    sqrt_price: u128,
    tick: i32,
    liquidity: u128,
    tick_spacing: u16,
    fee_rate: u16,
    a_to_b: bool,
    tick_arrays: &[TickArrayFacade],
    ticks_per_array_count: usize,
    tick_math: ClmmTickMath,
) -> Result<u64, ClmmMathError> {
    let quote = compute_swap_with_ticks_per_array_and_math(
        sqrt_price,
        tick,
        liquidity,
        tick_spacing,
        fee_rate,
        amount_in,
        a_to_b,
        tick_arrays,
        ticks_per_array_count,
        tick_math,
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

    /// Official program rule (`SparseSwapTickSequenceBuilder::get_start_tick_indexes`):
    /// a_to_b never shifts; b_to_a shifts only when the last tick of the array
    /// is reached.
    #[test]
    fn orca_tick_array_window_matches_program_rules() {
        let spacing = 4u16;
        let t = 88 * 4; // 352

        // a_to_b, tick inside the array: [base, base-T, base-2T].
        assert_eq!(
            orca_tick_array_start_indexes(0, spacing, true),
            vec![0, -t, -2 * t]
        );
        // a_to_b exactly at the array start: NO shift (the old code shifted).
        assert_eq!(
            orca_tick_array_start_indexes(t, spacing, true),
            vec![t, 0, -t]
        );

        // b_to_a, tick at the array start: [base, +T, +2T].
        assert_eq!(
            orca_tick_array_start_indexes(t, spacing, false),
            vec![t, 2 * t, 3 * t]
        );
        // b_to_a in the final tick of the array (tick + spacing reaches the
        // upper edge): [base+T, +2T, +3T].
        assert_eq!(
            orca_tick_array_start_indexes(2 * t - i32::from(spacing), spacing, false),
            vec![2 * t, 3 * t, 4 * t]
        );

        // Invalid start ticks (outside the global range) are dropped, exactly
        // like the program. At the top of the range the b_to_a window runs out
        // of valid start ticks.
        let top_base = MAX_TICK_INDEX - MAX_TICK_INDEX.rem_euclid(t);
        let window = orca_tick_array_start_indexes(top_base, spacing, false);
        assert!(window.iter().all(|s| *s <= MAX_TICK_INDEX));
        assert!(window.len() < 3, "window = {window:?}");
    }

    /// a_to_b at the array's lower edge must descend into the previous array
    /// rather than fabricate a tick at the boundary.
    #[test]
    fn a_to_b_lower_edge_descends_to_previous_array() {
        let spacing = 1u16;
        let array_a = TickArrayFacade {
            start_tick_index: 0,
            ticks: (0..TICK_ARRAY_SIZE)
                .map(|i| TickFacade {
                    tick_index: i as i32,
                    initialized: false,
                    liquidity_net: 0,
                    liquidity_gross: 0,
                    limit_order_unfilled: 0,
                })
                .collect(),
        };
        let mut prev_ticks: Vec<TickFacade> = (-(TICK_ARRAY_SIZE as i32)..0)
            .map(|tick_index| TickFacade {
                tick_index,
                initialized: false,
                liquidity_net: 0,
                liquidity_gross: 0,
                limit_order_unfilled: 0,
            })
            .collect();
        // One initialized tick inside the previous array.
        prev_ticks[TICK_ARRAY_SIZE - 5].initialized = true;
        prev_ticks[TICK_ARRAY_SIZE - 5].liquidity_net = 10;
        let array_prev = TickArrayFacade {
            start_tick_index: -(TICK_ARRAY_SIZE as i32),
            ticks: prev_ticks,
        };

        let (boundary, tick) =
            find_next_tick_boundary(0, spacing, true, &[array_a, array_prev]).unwrap();
        assert_eq!(boundary, -5);
        assert_eq!(tick.map(|t| t.tick_index), Some(-5));
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
                    limit_order_unfilled: 0,
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
