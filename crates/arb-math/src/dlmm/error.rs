use super::{MAX_BIN_ID, MIN_BIN_ID};
use thiserror::Error;

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
