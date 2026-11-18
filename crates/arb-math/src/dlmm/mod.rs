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

//!    Split into responsibility submodules (file-size review threshold, item 651):
//!    [`math`] — Q64.64 fixed-point primitives and shared constants; [`state`] —
//!    pool/bin/quote data types and the shared error; [`limits`] — executable
//!    input-capacity bounds; [`fee`] — fee dynamics and volatility accumulation;
//!    [`fill`] — per-bin liquidity consumption layers; [`quote`]/[`exact_out`] —
//!    the multi-bin traversal engines. Glob re-exports preserve every
//!    `arb_math::dlmm::` path used across the workspace verbatim.

mod exact_out;
mod fee;
mod fill;
mod limits;
mod math;
mod quote;
mod state;
#[cfg(test)]
mod tests;

pub use exact_out::*;
pub use fee::*;
pub use fill::*;
pub use limits::*;
pub use math::*;
pub use quote::*;
pub use state::*;
