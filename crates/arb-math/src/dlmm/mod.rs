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

mod breakpoints;
mod capacity;
mod error;
mod fee;
mod price;
mod quote_exact_in;
mod quote_exact_out;
mod swap;
mod types;

#[cfg(test)]
mod tests;

pub use breakpoints::{DlmmDrainBreakpoint, dlmm_breakpoint_seed_sizes, dlmm_drain_breakpoints};
pub use capacity::{
    dlmm_active_bin_max_input, dlmm_multi_bin_max_input, scale_dlmm_three_hop_input,
};
pub use error::{CollectFeeMode, DlmmError, dlmm_fee_on_input, validate_swap_depth};
pub use fee::{
    compute_base_fee, compute_fee_from_amount, compute_fee_to_add, compute_protocol_fee,
    compute_total_fee_rate, compute_variable_fee, included_fee_amount,
    update_references_with_decay, update_volatility_accumulator,
};
pub use price::{advance_active_id, bin_array_index_for_bin, get_price_from_id, pow_q64};
pub use quote_exact_in::{quote_dlmm_multi_bin, quote_dlmm_view};
pub use quote_exact_out::quote_dlmm_exact_out;
pub use swap::{bin_max_amount_out, consume_bin_liquidity};
pub use types::{
    DlmmBin, DlmmParameters, DlmmPoolState, DlmmSwapQuote, DlmmSwapStepResult,
    DlmmVariableParameters,
};

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
