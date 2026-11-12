//! Pure Mathematical Leg Pricing & Bin-Walk Traversal for Meteora DLMM (Dynamic Liquidity Market Maker).
//!
//! Reimplemented from the on-chain Meteora DLMM (`lb_clmm` program: `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`)
//! using deterministic integer and fixed-point math (`U256`).
//!
//! Architectural Invariants:
//! 1. Zero Floating Point: All price, fee, and liquidity computations evaluate strictly over Q64.64 or wide integer types (`U256`).
//! 2. Zero Panic / Zero Unwrap: All error paths return a typed `DlmmError`.
//! 3. On-Chain Rounding Conformance:
//!    - Fees and required input amounts use ceiling division (`div_ceil`) to prevent protocol loss.
//!    - Token output amounts use floor division (`div_floor`) to prevent token fabrication.
//! 4. Volatility Surge Dynamics: Dynamically scales variable swap fees as trades traverse multiple bins.

use arb_types::Bps;
use ruint::aliases::U256;
use thiserror::Error;

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

/// Minimum valid Bin ID on Solana Meteora DLMM.
pub const MIN_BIN_ID: i32 = -443_636;

/// Maximum valid Bin ID on Solana Meteora DLMM.
pub const MAX_BIN_ID: i32 = 443_636;

/// Upper limit for binary exponentiation ($2^{19} = 524{,}288$).
pub const MAX_EXPONENTIAL: u32 = 0x80_000;

/// DLMM-specific mathematical and validation errors.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum DlmmError {
    #[error("math overflow in calculation")]
    MathOverflow,

    #[error("type cast failed")]
    TypeCastFailed,

    #[error("bin ID out of range: {0} not in [{MIN_BIN_ID}, {MAX_BIN_ID}]")]
    BinIdOutOfRange(i32),

    #[error("pool is disabled or not activated")]
    PoolDisabled,

    #[error("insufficient liquidity in pool: required {required}, provided {available}")]
    InsufficientLiquidity { required: u64, available: u64 },

    #[error("zero input amount")]
    ZeroInput,

    #[error("active bin array not found in cache for bin {0}")]
    BinNotFound(i32),

    #[error("invalid CollectFeeMode: {0} (expected 0 = InputOnly or 1 = OnlyY)")]
    InvalidCollectFeeMode(u8),
}

/// Explicit CollectFeeMode enum, named after the official Meteora IDL field
/// `StaticParameters.collect_fee_mode` (IDL constant values 0 and 1).
///
/// Source: `fixtures/dlmm.json` -> `LbPair.staticParameters.collectFeeMode`
/// (u8). SDK: `@meteora-ag/dlmm@1.9.14` `getFeeMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectFeeMode {
    /// 0 = Input only: fee is always charged on the input token of the swap.
    InputOnly = 0,
    /// 1 = Only token Y: fee is always charged on token Y.
    OnlyY = 1,
}

impl CollectFeeMode {
    pub fn from_u8(v: u8) -> Result<Self, DlmmError> {
        match v {
            0 => Ok(Self::InputOnly),
            1 => Ok(Self::OnlyY),
            other => Err(DlmmError::InvalidCollectFeeMode(other)),
        }
    }
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Resolve whether the DLMM swap fee is charged on the input token for this
/// direction, exactly as the official SDK's `getFeeMode` does.
///
/// * `CollectFeeMode::InputOnly` (0) — fee is always on the input mint.
/// * `CollectFeeMode::OnlyY` (1) — fee is always on token Y, therefore:
///   * X -> Y (`swap_for_y = true`): fee is on the **output** (Y).
///   * Y -> X (`swap_for_y = false`): fee is on the **input** (Y).
pub fn dlmm_fee_on_input(collect_fee_mode: u8, swap_for_y: bool) -> Result<bool, DlmmError> {
    CollectFeeMode::from_u8(collect_fee_mode)?;
    match collect_fee_mode {
        0 => Ok(true),
        1 => Ok(!swap_for_y),
        other => Err(DlmmError::InvalidCollectFeeMode(other)),
    }
}

/// Static parameters of an `LbPair` pool configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DlmmParameters {
    /// Bin step in basis points (e.g. 10 = 0.1%, 25 = 0.25%, 100 = 1.0%).
    pub bin_step: u16,
    /// Base factor for base fee scaling.
    pub base_factor: u16,
    /// Filter period in seconds before decay begins.
    pub filter_period: u16,
    /// Decay period in seconds when volatility drops to zero.
    pub decay_period: u16,
    /// Volatility reference reduction factor in basis points (e.g. 5000 = 50%).
    pub reduction_factor: u16,
    /// Variable fee control multiplier ($A$).
    pub variable_fee_control: u32,
    /// Maximum cap for the volatility accumulator.
    pub max_volatility_accumulator: u32,
    /// Protocol fee share in basis points (e.g. 1000 = 10%).
    pub protocol_share: u16,
    /// Optional exponential scaling power factor ($10^{\text{base\_fee\_power\_factor}}$).
    pub base_fee_power_factor: u8,
    /// Mode for fee collection: 0 = InputToken (default), 1 = OnlyY.
    pub collect_fee_mode: u8,
}

/// Runtime variable parameters of an `LbPair` tracking volatility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DlmmVariableParameters {
    /// Real-time volatility accumulator tracking bin movement intensity.
    pub volatility_accumulator: u32,
    /// Decayed volatility reference benchmark.
    pub volatility_reference: u32,
    /// Bin index reference point.
    pub index_reference: i32,
    /// Unix timestamp of the last volatility update.
    pub last_update_timestamp: i64,
}

/// State representation of a discrete Liquidity Bin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DlmmBin {
    /// Integer Bin ID $i \in [-443{,}636, 443{,}636]$.
    pub id: i32,
    /// Available Token X reserve in this bin.
    pub amount_x: u64,
    /// Available Token Y reserve in this bin.
    pub amount_y: u64,
    /// Cached Q64.64 price of Token X in terms of Token Y. (0 if uninitialized).
    pub price_q64: u128,
}

/// Snapshot of an `LbPair` DLMM pool state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DlmmPoolState {
    /// Currently active bin ID.
    pub active_id: i32,
    /// Pool bin step.
    pub bin_step: u16,
    /// Pool status (0 = Enabled, != 0 = Disabled).
    pub status: u8,
    /// Static configuration parameters.
    pub parameters: DlmmParameters,
    /// Runtime volatility tracking state.
    pub v_parameters: DlmmVariableParameters,
}

/// Result of swapping into a single bin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DlmmSwapStepResult {
    /// Inbound amount consumed from user (including fees).
    pub amount_in_consumed: u64,
    /// Outbound amount produced for user.
    pub amount_out: u64,
    /// Total fee charged on this step.
    pub fee_amount: u64,
    /// Protocol fee portion of `fee_amount`.
    pub protocol_fee: u64,
    /// Whether this step completely drained the bin's available reserve.
    pub drained_bin: bool,
}

/// Output quotation of a multi-bin DLMM swap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DlmmSwapQuote {
    /// Total amount of inbound tokens consumed.
    pub amount_in: u64,
    /// Total amount of outbound tokens delivered.
    pub amount_out: u64,
    /// Total swap fees incurred across all traversed bins.
    pub fee_amount: u64,
    /// Total protocol fees captured.
    pub protocol_fee: u64,
    /// Active bin ID upon trade completion.
    pub end_active_id: i32,
    /// Volatility accumulator upon trade completion.
    pub end_volatility_accumulator: u32,
    /// Total number of discrete bins crossed during the swap.
    pub bins_crossed: u32,
    /// Aggregate price impact in Basis Points.
    pub price_impact_bps: Bps,
}

// ============================================================================
// 1. DISCRETE BIN PRICING (Q64.64 FIXED-POINT MATH)
// ============================================================================

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

/// Compute the maximum absorbable input liquidity in an active bin.
///
/// In Meteora DLMM:
/// - When `a_to_b` is true (User deposits Token X, receives Token Y):
///   The pool pays out Token Y from the active bin.
///   The maximum Token X deposit that can be absorbed before active bin's
///   Token Y (`amount_y`) is depleted is: `(amount_y << 64) / price_q64`.
/// - When `a_to_b` is false (User deposits Token Y, receives Token X):
///   The pool pays out Token X from the active bin.
///   The maximum Token Y deposit that can be absorbed before active bin's
///   Token X (`amount_x`) is depleted is: `(amount_x * price_q64) >> 64`.
pub fn dlmm_active_bin_max_input(
    active_bin_id: i32,
    bin_step_bps: u16,
    active_bin_amount_x: u64,
    active_bin_amount_y: u64,
    a_to_b: bool,
) -> Option<u64> {
    let price_q64 = get_price_from_id(active_bin_id, bin_step_bps).ok()?;
    if price_q64 == 0 {
        return None;
    }

    if a_to_b {
        if active_bin_amount_y == 0 {
            return None;
        }
        let y_wide = U256::from(active_bin_amount_y) << SCALE_OFFSET;
        let max_x = y_wide / U256::from(price_q64);
        let cap_u64: u64 = max_x.try_into().unwrap_or(u64::MAX);
        Some(cap_u64)
    } else {
        if active_bin_amount_x == 0 {
            return None;
        }
        let x_wide = U256::from(active_bin_amount_x);
        let max_y = (x_wide * U256::from(price_q64)) >> SCALE_OFFSET;
        let cap_u64: u64 = max_y.try_into().unwrap_or(u64::MAX);
        Some(cap_u64)
    }
}

/// Scale trade input across a 3-hop Meteora DLMM cycle to extract maximum active bin liquidity.
///
/// In Meteora DLMM, price is constant-sum inside each bin (zero price impact within the active bin).
/// This calculates the bottleneck across all three hops, applies a 92% safety factor to avoid
/// crossing into suboptimal bins, and scales the gross edge linearly.
pub fn scale_dlmm_three_hop_input(
    probe_amount_in: u64,
    probe_gross_profit: u64,
    hop1_active_bin_max_in: u64,
    hop2_active_bin_max_in_equiv: u64,
    hop3_active_bin_max_in_equiv: u64,
    max_wallet_cap: u64,
) -> (u64, u64) {
    let bottleneck_in = hop1_active_bin_max_in
        .min(hop2_active_bin_max_in_equiv)
        .min(hop3_active_bin_max_in_equiv)
        .min(max_wallet_cap);

    let safe_optimal_in = (bottleneck_in * 92) / 100;

    if safe_optimal_in <= probe_amount_in || probe_amount_in == 0 {
        return (probe_amount_in, probe_gross_profit);
    }

    let scale_factor = safe_optimal_in as u128;
    let scaled_profit =
        ((probe_gross_profit as u128) * scale_factor / (probe_amount_in as u128)) as u64;

    (safe_optimal_in, scaled_profit)
}

// ============================================================================
// 2. FEE DYNAMICS & VOLATILITY ACCUMULATOR MATH
// ============================================================================

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

// ============================================================================
// 3. SINGLE BIN SWAP EXECUTION (ACTIVE BIN LIQUIDITY CONSUMPTION)
// ============================================================================

/// Input required to produce an exact gross output amount from one bin.
fn input_for_gross_out(
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

/// Swap into a single discrete bin, consuming input and draining opposite-side reserves.
///
/// Direction convention:
/// - `swap_for_y = true` (X -> Y): Inbound is Token X, Outbound is Token Y.
/// - `swap_for_y = false` (Y -> X): Inbound is Token Y, Outbound is Token X.
///
/// `fee_on_input` mirrors the official SDK `getFeeMode(...).feeOnInput`:
/// - `true`: the fee is removed from the input before the bin is priced.
/// - `false`: the full input is priced and the fee is removed from the output.
pub fn consume_bin_liquidity(
    bin: &mut DlmmBin,
    amount_in: u64,
    price_q64: u128,
    swap_for_y: bool,
    fee_rate: u128,
    protocol_share_bps: u16,
    fee_on_input: bool,
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

    let bin_max_out = if swap_for_y {
        bin.amount_y
    } else {
        bin.amount_x
    };
    if bin_max_out == 0 {
        return Ok(DlmmSwapStepResult {
            amount_in_consumed: 0,
            amount_out: 0,
            fee_amount: 0,
            protocol_fee: 0,
            drained_bin: true,
        });
    }

    if fee_on_input {
        // Calculate maximum net inbound amount required to completely drain this bin.
        let net_in_max = input_for_gross_out(bin_max_out, price_q64, swap_for_y)?;
        let max_fee = compute_fee_to_add(net_in_max, fee_rate)?;
        let gross_in_max = net_in_max
            .checked_add(max_fee)
            .ok_or(DlmmError::MathOverflow)?;

        if amount_in >= gross_in_max {
            // Trade fully drains the available reserve of this bin.
            let protocol_fee = compute_protocol_fee(max_fee, protocol_share_bps)?;
            if swap_for_y {
                bin.amount_x = bin
                    .amount_x
                    .checked_add(net_in_max)
                    .ok_or(DlmmError::MathOverflow)?;
                bin.amount_y = 0;
            } else {
                bin.amount_y = bin
                    .amount_y
                    .checked_add(net_in_max)
                    .ok_or(DlmmError::MathOverflow)?;
                bin.amount_x = 0;
            }

            Ok(DlmmSwapStepResult {
                amount_in_consumed: gross_in_max,
                amount_out: bin_max_out,
                fee_amount: max_fee,
                protocol_fee,
                drained_bin: true,
            })
        } else {
            // Trade terminates inside this bin.
            let fee = compute_fee_from_amount(amount_in, fee_rate)?;
            let net_in = amount_in.checked_sub(fee).ok_or(DlmmError::MathOverflow)?;

            let amount_out = gross_out_for_input(net_in, price_q64, swap_for_y)?.min(bin_max_out);

            let protocol_fee = compute_protocol_fee(fee, protocol_share_bps)?;
            if swap_for_y {
                bin.amount_x = bin
                    .amount_x
                    .checked_add(net_in)
                    .ok_or(DlmmError::MathOverflow)?;
                bin.amount_y = bin
                    .amount_y
                    .checked_sub(amount_out)
                    .ok_or(DlmmError::MathOverflow)?;
            } else {
                bin.amount_y = bin
                    .amount_y
                    .checked_add(net_in)
                    .ok_or(DlmmError::MathOverflow)?;
                bin.amount_x = bin
                    .amount_x
                    .checked_sub(amount_out)
                    .ok_or(DlmmError::MathOverflow)?;
            }

            Ok(DlmmSwapStepResult {
                amount_in_consumed: amount_in,
                amount_out,
                fee_amount: fee,
                protocol_fee,
                drained_bin: false,
            })
        }
    } else {
        // Fee is charged on the output token (CollectFeeMode::OnlyY, X -> Y).
        // The pool fill consumes *gross* output; the user receives gross minus fee.
        let net_in_max = input_for_gross_out(bin_max_out, price_q64, swap_for_y)?;

        let (consumed_in, gross_out, drained) = if amount_in >= net_in_max {
            (net_in_max, bin_max_out, true)
        } else {
            let gross = gross_out_for_input(amount_in, price_q64, swap_for_y)?.min(bin_max_out);
            (amount_in, gross, false)
        };

        let fee = compute_fee_from_amount(gross_out, fee_rate)?;
        let user_out = gross_out.checked_sub(fee).ok_or(DlmmError::MathOverflow)?;
        let protocol_fee = compute_protocol_fee(fee, protocol_share_bps)?;

        if swap_for_y {
            bin.amount_x = bin
                .amount_x
                .checked_add(consumed_in)
                .ok_or(DlmmError::MathOverflow)?;
            bin.amount_y = bin
                .amount_y
                .checked_sub(gross_out)
                .ok_or(DlmmError::MathOverflow)?;
        } else {
            bin.amount_y = bin
                .amount_y
                .checked_add(consumed_in)
                .ok_or(DlmmError::MathOverflow)?;
            bin.amount_x = bin
                .amount_x
                .checked_sub(gross_out)
                .ok_or(DlmmError::MathOverflow)?;
        }

        Ok(DlmmSwapStepResult {
            amount_in_consumed: consumed_in,
            amount_out: user_out,
            fee_amount: fee,
            protocol_fee,
            drained_bin: drained,
        })
    }
}

// ============================================================================
// 4. MULTI-BIN TRAVERSAL QUOTING ENGINE
// ============================================================================

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

    while remaining_in > 0 {
        // Find bin in provided slice matching current_bin_id
        let bin = bins
            .iter_mut()
            .find(|b| b.id == current_bin_id)
            .ok_or(DlmmError::BinNotFound(current_bin_id))?;

        // Update volatility accumulator for this bin
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
            dlmm_fee_on_input(pool.parameters.collect_fee_mode, swap_for_y)?,
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

    while remaining_out > 0 {
        let bin = cloned_bins
            .iter_mut()
            .find(|b| b.id == current_bin_id)
            .ok_or(DlmmError::BinNotFound(current_bin_id))?;

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

        let fee_on_input = dlmm_fee_on_input(pool.parameters.collect_fee_mode, swap_for_y)?;

        let available_gross_out = if swap_for_y {
            bin.amount_y
        } else {
            bin.amount_x
        };
        if available_gross_out == 0 {
            current_bin_id = advance_active_id(current_bin_id, swap_for_y)?;
            bins_crossed = bins_crossed.saturating_add(1);
            continue;
        }

        // Gross output required so the user receives `remaining_out` net.
        // Mirrors the SDK's `includedFeeAmountOut`: when the fee is on the input it
        // is the requested output itself, otherwise `getIncludedFeeAmount`.
        let desired_gross_out = if fee_on_input {
            U256::from(remaining_out)
        } else {
            let denom = FEE_PRECISION
                .checked_sub(fee_rate)
                .ok_or(DlmmError::MathOverflow)?;
            let num = U256::from(remaining_out) * U256::from(FEE_PRECISION);
            let (q, r) = num.div_rem(U256::from(denom));
            if r.is_zero() {
                q
            } else {
                q.checked_add(U256::from(1))
                    .ok_or(DlmmError::MathOverflow)?
            }
        };

        // Probe the bin without committing state: the SDK computes the exact-in
        // quote over the derived input and reports that quote's fee split.
        let mut probe = *bin;
        let (step, user_out) = if desired_gross_out >= U256::from(available_gross_out) {
            // Requested output exceeds this bin. The SDK re-quotes the bin with an
            // unbounded input, i.e. it fills the bin completely.
            let step = consume_bin_liquidity(
                &mut probe,
                u64::MAX,
                price_q64,
                swap_for_y,
                fee_rate,
                pool.parameters.protocol_share,
                fee_on_input,
            )?;
            let user_out = step.amount_out;
            (step, user_out)
        } else {
            let gross_from_bin: u64 = desired_gross_out
                .try_into()
                .map_err(|_| DlmmError::TypeCastFailed)?;
            // Required fee-excluded input for that gross output, then the fee-included
            // input the user actually pays (`getIncludedFeeAmount`).
            let net_in = input_for_gross_out(gross_from_bin, price_q64, swap_for_y)?;
            let included_in = if fee_on_input {
                included_fee_amount(net_in, fee_rate)?
            } else {
                net_in
            };
            let mut step = consume_bin_liquidity(
                &mut probe,
                included_in,
                price_q64,
                swap_for_y,
                fee_rate,
                pool.parameters.protocol_share,
                fee_on_input,
            )?;
            // Rounding dust. The exact-in quote over `included_in` can produce
            // slightly more than the requested net output; the SDK assigns any
            // surplus greater than one unit to the protocol fee (see
            // `swapExactOutQuoteAtBin`). The total trading fee increases by the
            // same delta, while the user fee (fee_amount - protocol_fee) stays
            // constant. We mirror this by adding the dust to both `protocol_fee`
            // and `fee_amount` (the latter represents the total trading fee).
            if !step.drained_bin {
                let dust_threshold = remaining_out.saturating_add(1);
                if step.amount_out > dust_threshold {
                    let dust = step.amount_out - remaining_out;
                    step.protocol_fee = step
                        .protocol_fee
                        .checked_add(dust)
                        .ok_or(DlmmError::MathOverflow)?;
                    step.fee_amount = step
                        .fee_amount
                        .checked_add(dust)
                        .ok_or(DlmmError::MathOverflow)?;
                }
            }
            (step, remaining_out)
        };
        let fee = step.fee_amount;
        let gross_in = step.amount_in_consumed;
        // Fee-excluded input that moves into the bin, and the gross reserve the bin
        // gives up, both derived from the same step result.
        let net_in = if fee_on_input {
            gross_in.checked_sub(fee).ok_or(DlmmError::MathOverflow)?
        } else {
            gross_in
        };
        let gross_from_bin = if fee_on_input {
            step.amount_out
        } else {
            step.amount_out
                .checked_add(fee)
                .ok_or(DlmmError::MathOverflow)?
        };

        if user_out == 0 && remaining_out > 0 {
            // Cannot make progress: the gross output is entirely consumed by fees.
            return Err(DlmmError::InsufficientLiquidity {
                required: remaining_out,
                available: gross_from_bin,
            });
        }

        let p_fee = step.protocol_fee;

        if swap_for_y {
            bin.amount_x = bin
                .amount_x
                .checked_add(net_in)
                .ok_or(DlmmError::MathOverflow)?;
            bin.amount_y = bin
                .amount_y
                .checked_sub(gross_from_bin)
                .ok_or(DlmmError::MathOverflow)?;
        } else {
            bin.amount_y = bin
                .amount_y
                .checked_add(net_in)
                .ok_or(DlmmError::MathOverflow)?;
            bin.amount_x = bin
                .amount_x
                .checked_sub(gross_from_bin)
                .ok_or(DlmmError::MathOverflow)?;
        }

        total_in = total_in
            .checked_add(gross_in)
            .ok_or(DlmmError::MathOverflow)?;
        total_fee = total_fee.checked_add(fee).ok_or(DlmmError::MathOverflow)?;
        total_protocol_fee = total_protocol_fee
            .checked_add(p_fee)
            .ok_or(DlmmError::MathOverflow)?;
        remaining_out = remaining_out
            .checked_sub(user_out)
            .ok_or(DlmmError::MathOverflow)?;

        if remaining_out > 0 {
            current_bin_id = advance_active_id(current_bin_id, swap_for_y)?;
            bins_crossed = bins_crossed.saturating_add(1);
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

/// Calculate execution price impact against starting spot price in basis points.
fn calculate_price_impact_bps(
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
        (prod >> SCALE_OFFSET).try_into().unwrap_or(u64::MAX)
    } else {
        let num = U256::from(amount_in) << SCALE_OFFSET;
        (num / U256::from(start_price_q64))
            .try_into()
            .unwrap_or(u64::MAX)
    };

    if ideal_out <= amount_out {
        return Bps(0);
    }

    let diff = ideal_out.saturating_sub(amount_out);
    let bps = (U256::from(diff) * U256::from(10_000)) / U256::from(ideal_out);
    let val = u16::try_from(bps.to::<u64>().min(10_000)).unwrap_or(10_000);
    Bps(val)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_parameters() -> DlmmParameters {
        DlmmParameters {
            bin_step: 25, // 0.25%
            base_factor: 1000,
            filter_period: 30,
            decay_period: 600,
            reduction_factor: 5000, // 50%
            variable_fee_control: 10,
            max_volatility_accumulator: 350_000,
            protocol_share: 0,
            base_fee_power_factor: 0,
            collect_fee_mode: 0,
        }
    }

    fn create_test_pool(active_id: i32) -> DlmmPoolState {
        let parameters = create_test_parameters();
        DlmmPoolState {
            active_id,
            bin_step: parameters.bin_step,
            status: 0,
            parameters,
            v_parameters: DlmmVariableParameters {
                volatility_accumulator: 0,
                volatility_reference: 0,
                index_reference: active_id,
                last_update_timestamp: 1_700_000_000,
            },
        }
    }

    #[test]
    fn pow_q64_rejects_unrepresentable_prices() {
        assert!(pow_q64(u128::MAX, 1).is_err());
        assert!(pow_q64(ONE_Q64 / 2, (MAX_EXPONENTIAL - 1) as i32).is_err());
    }

    #[test]
    fn view_quote_does_not_mutate_the_supplied_bin_snapshot() {
        let pool = create_test_pool(10);
        let bins = vec![DlmmBin {
            id: 10,
            amount_x: 0,
            amount_y: 1_000_000,
            price_q64: 0,
        }];
        let before = bins.clone();
        let _ = quote_dlmm_view(&pool, &bins, 10_000, true, None);
        assert_eq!(bins, before);
    }

    #[test]
    fn test_bin_price_at_zero() {
        let price = get_price_from_id(0, 25).expect("price at bin 0");
        assert_eq!(price, ONE_Q64, "P_0 must equal exactly 1.0 in Q64.64");
    }

    #[test]
    fn test_bin_price_positive_exponent() {
        let price_1 = get_price_from_id(1, 100).expect("price at bin 1");
        // 1.01 * 2^64
        let expected = (U256::from(ONE_Q64) * U256::from(101)) / U256::from(100);
        assert_eq!(U256::from(price_1), expected);

        let price_10 = get_price_from_id(10, 25).expect("price at bin 10");
        assert!(price_10 > ONE_Q64);
    }

    #[test]
    fn test_bin_price_negative_exponent() {
        let price_neg1 = get_price_from_id(-1, 100).expect("price at bin -1");
        // 1/1.01 * 2^64
        assert!(price_neg1 < ONE_Q64);
        assert!(price_neg1 > 0);

        let price_1 = get_price_from_id(1, 100).expect("price at bin 1");
        let prod = (U256::from(price_neg1) * U256::from(price_1)) >> SCALE_OFFSET;
        // Product of P_1 and P_{-1} must be approximately 1.0 in Q64.64
        let diff = if prod > U256::from(ONE_Q64) {
            prod - U256::from(ONE_Q64)
        } else {
            U256::from(ONE_Q64) - prod
        };
        assert!(
            diff <= U256::from(100),
            "Reciprocal price product roundoff within 100 units"
        );
    }

    #[test]
    fn test_fee_calculation() {
        let params = create_test_parameters();
        // base_fee = 1000 * 25 * 10 = 250_000 (0.025% = 2.5 bps)
        let base_fee = compute_base_fee(&params, params.bin_step).expect("base fee");
        assert_eq!(base_fee, 250_000);

        // variable_fee with va = 10_000:
        // (10_000 * 25)^2 = 6.25e10. vfc = 10 -> 6.25e11. ceil(6.25e11 / 1e11) = 7
        let var_fee = compute_variable_fee(&params, params.bin_step, 10_000).expect("variable fee");
        assert_eq!(var_fee, 7);

        let total_fee =
            compute_total_fee_rate(&params, params.bin_step, 10_000).expect("total fee");
        assert_eq!(total_fee, 250_007);
    }

    #[test]
    fn test_fee_rate_capped_at_max() {
        let mut params = create_test_parameters();
        params.base_factor = 50_000;
        params.base_fee_power_factor = 2; // 50k * 25 * 10 * 100 = 1.25B > 100M cap
        let total = compute_total_fee_rate(&params, params.bin_step, 0).expect("total fee");
        assert_eq!(total, MAX_FEE_RATE, "Fee rate must be clamped at 10%");
    }

    #[test]
    fn test_single_bin_swap_x_to_y_partial() {
        let _pool = create_test_pool(0);
        let mut bin = DlmmBin {
            id: 0,
            amount_x: 0,
            amount_y: 1_000_000,
            price_q64: ONE_Q64,
        };

        let step = consume_bin_liquidity(
            &mut bin, 100_000, ONE_Q64, true,    // X -> Y
            250_000, // 2.5 bps fee
            0, true, // fee on input
        )
        .expect("swap step");

        assert_eq!(step.amount_in_consumed, 100_000);
        assert!(!step.drained_bin);
        assert!(step.fee_amount > 0);
        assert_eq!(step.amount_out, 100_000 - step.fee_amount);
        assert_eq!(bin.amount_y, 1_000_000 - step.amount_out);
        assert_eq!(bin.amount_x, 100_000 - step.fee_amount);
    }

    #[test]
    fn test_single_bin_swap_x_to_y_full_drain() {
        let _pool = create_test_pool(0);
        let mut bin = DlmmBin {
            id: 0,
            amount_x: 0,
            amount_y: 100_000,
            price_q64: ONE_Q64,
        };

        let step = consume_bin_liquidity(
            &mut bin, 1_000_000, // much larger than available 100k
            ONE_Q64, true, // X -> Y
            250_000, 0, true, // fee on input
        )
        .expect("swap step");

        assert!(step.drained_bin);
        assert_eq!(step.amount_out, 100_000);
        assert_eq!(bin.amount_y, 0);
        assert!(step.amount_in_consumed > 100_000); // includes fee
    }

    #[test]
    fn test_multi_bin_traversal_x_to_y() {
        let pool = create_test_pool(10);
        let mut bins = vec![
            DlmmBin {
                id: 10,
                amount_x: 0,
                amount_y: 1_000_000,
                price_q64: 0,
            },
            DlmmBin {
                id: 9,
                amount_x: 0,
                amount_y: 2_000_000,
                price_q64: 0,
            },
            DlmmBin {
                id: 8,
                amount_x: 0,
                amount_y: 5_000_000,
                price_q64: 0,
            },
        ];

        let quote = quote_dlmm_multi_bin(
            &pool, &mut bins, 2_000_000, true, // X -> Y
            None,
        )
        .expect("multi-bin quote");

        assert_eq!(quote.amount_in, 2_000_000);
        assert_eq!(quote.end_active_id, 9);
        assert_eq!(quote.bins_crossed, 1);
        assert!(quote.amount_out > 2_000_000); // Price is > 1.0 at bin 10 & 9
        assert_eq!(quote.fee_amount, 501);
    }

    #[test]
    fn test_multi_bin_traversal_y_to_x() {
        let pool = create_test_pool(10);
        let mut bins = vec![
            DlmmBin {
                id: 10,
                amount_x: 1_000_000,
                amount_y: 0,
                price_q64: 0,
            },
            DlmmBin {
                id: 11,
                amount_x: 2_000_000,
                amount_y: 0,
                price_q64: 0,
            },
            DlmmBin {
                id: 12,
                amount_x: 5_000_000,
                amount_y: 0,
                price_q64: 0,
            },
        ];

        let quote = quote_dlmm_multi_bin(
            &pool, &mut bins, 2_000_000, false, // Y -> X
            None,
        )
        .expect("multi-bin quote");

        assert_eq!(quote.amount_in, 2_000_000);
        assert_eq!(quote.end_active_id, 11);
        assert_eq!(quote.bins_crossed, 1);
        assert!(quote.amount_out < 2_000_000); // Token X is more expensive than Y
        assert_eq!(quote.fee_amount, 501);
    }

    #[test]
    fn test_collect_fee_mode_matrix() {
        // Mode 0 = InputOnly: fee always on input, both directions.
        assert!(dlmm_fee_on_input(0, true).unwrap()); // X->Y
        assert!(dlmm_fee_on_input(0, false).unwrap()); // Y->X

        // Mode 1 = OnlyY.
        assert!(!dlmm_fee_on_input(1, true).unwrap()); // X->Y: fee on output Y
        assert!(dlmm_fee_on_input(1, false).unwrap()); // Y->X: fee on input Y

        // Invalid mode rejected.
        assert!(dlmm_fee_on_input(2, true).is_err());

        // Enum round-trip.
        assert_eq!(CollectFeeMode::InputOnly.as_u8(), 0);
        assert_eq!(CollectFeeMode::OnlyY.as_u8(), 1);
        assert_eq!(
            CollectFeeMode::from_u8(0).unwrap(),
            CollectFeeMode::InputOnly
        );
        assert_eq!(CollectFeeMode::from_u8(1).unwrap(), CollectFeeMode::OnlyY);

        // Regression: reversed enum values must not silently pass.
        assert_ne!(CollectFeeMode::from_u8(0).unwrap(), CollectFeeMode::OnlyY);
        assert_ne!(
            CollectFeeMode::from_u8(1).unwrap(),
            CollectFeeMode::InputOnly
        );
    }

    #[test]
    fn test_volatility_decay() {
        let params = create_test_parameters();
        let mut v_params = DlmmVariableParameters {
            volatility_accumulator: 20_000,
            volatility_reference: 20_000,
            index_reference: 0,
            last_update_timestamp: 1_000,
        };

        // Within filter period (elapsed 10s < 30s) -> no change
        update_references_with_decay(&mut v_params, &params, 5, 1_010).expect("decay update");
        assert_eq!(v_params.volatility_reference, 20_000);
        assert_eq!(v_params.index_reference, 0);

        // Between filter & decay period (elapsed 100s, filter=30, decay=600) -> 50% decay
        update_references_with_decay(&mut v_params, &params, 5, 1_100).expect("decay update");
        assert_eq!(v_params.index_reference, 5);
        assert_eq!(v_params.volatility_reference, 10_000); // 20k * 50%

        // Past decay period (elapsed 700s > 600s) -> drops to 0
        update_references_with_decay(&mut v_params, &params, 5, 1_700).expect("decay update");
        assert_eq!(v_params.volatility_reference, 0);
    }

    #[test]
    fn test_dlmm_active_bin_max_input() {
        // Price at bin 0 with 25 bps is 1.0 (ONE_Q64)
        // a_to_b: pool pays Y, so max input X depends on amount_y
        let cap_x = dlmm_active_bin_max_input(0, 25, 5_000_000, 10_000_000, true);
        assert_eq!(cap_x, Some(10_000_000));

        // b_to_a: pool pays X, so max input Y depends on amount_x
        let cap_y = dlmm_active_bin_max_input(0, 25, 5_000_000, 10_000_000, false);
        assert_eq!(cap_y, Some(5_000_000));

        // If active bin has zero output token liquidity, returns None
        assert_eq!(dlmm_active_bin_max_input(0, 25, 5_000_000, 0, true), None);
        assert_eq!(dlmm_active_bin_max_input(0, 25, 0, 10_000_000, false), None);
    }

    #[test]
    fn test_scale_dlmm_three_hop_input() {
        // Probe: 10,000 lamports giving 23,000 lamports profit
        let probe_in = 10_000;
        let probe_profit = 23_000;

        // Bottleneck is 1,000,000 across 3 hops
        let hop1_cap = 2_000_000;
        let hop2_cap = 1_000_000;
        let hop3_cap = 3_000_000;
        let max_wallet = 10_000_000;

        let (scaled_in, scaled_profit) = scale_dlmm_three_hop_input(
            probe_in,
            probe_profit,
            hop1_cap,
            hop2_cap,
            hop3_cap,
            max_wallet,
        );

        // Safe optimal in = (1,000,000 * 92) / 100 = 920,000
        assert_eq!(scaled_in, 920_000);
        // Scaled profit = 23,000 * 920,000 / 10,000 = 2,116,000 lamports!
        assert_eq!(scaled_profit, 2_116_000);
    }
}
