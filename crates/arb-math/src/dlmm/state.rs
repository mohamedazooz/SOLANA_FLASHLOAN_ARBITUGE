//! Shared DLMM data types: pool configuration, bin and quote structures, the
//! typed error, and the `CollectFeeMode` fee-orientation semantics.

use arb_types::Bps;
use thiserror::Error;

use super::math::{MAX_BIN_ID, MIN_BIN_ID};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
    /// Limit-order layer: open (unprocessed) order liquidity, denominated in
    /// the order's committed output token. Zero for pools without limit orders.
    pub open_order_amount: u64,
    /// Limit-order layer: remaining fillable amount of already-processed
    /// orders, denominated in the order's committed output token.
    pub processed_order_remaining_amount: u64,
    /// Limit-order side flag from the on-chain `Bin` (`limit_order_ask_side`).
    /// Bid-side orders are fillable by X -> Y swaps (`swap_for_y = true`);
    /// ask-side orders by Y -> X swaps (`swap_for_y = false`).
    pub limit_order_ask_side: bool,
}

/// Snapshot of an `LbPair` DLMM pool state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
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
    /// Whether the pool's `function_type`/reward configuration enables the
    /// limit-order liquidity layer (`LbPair::is_support_limit_order` in the
    /// official SDK). When `false`, order layers are ignored: quotes use the
    /// MM reserves only, which underquotes rather than overquotes (fail-safe).
    /// Decoders MUST populate this from the on-chain account.
    pub support_limit_order: bool,
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
