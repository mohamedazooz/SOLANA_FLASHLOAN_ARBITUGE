//! Unified error types for the workspace.

use super::primitives::*;
use thiserror::Error;

/// Result type for bot operations.
pub type Result<T> = std::result::Result<T, ArbError>;

/// Unified error types across workspace crates.
#[derive(Error, Debug)]
pub enum ArbError {
    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("overflow in calculation")]
    Overflow,

    #[error("pool not found: {0}")]
    PoolNotFound(PoolId),

    #[error("route not found: {0:?}")]
    RouteNotFound(RouteId),

    #[error("insufficient balance: have {have}, need {need}")]
    InsufficientBalance { have: Lamports, need: Lamports },

    #[error("stale state: slot {slot:?} too old, max {max:?}")]
    StaleState { slot: Slot, max: Slot },

    #[error("risk gate denied: {reason}")]
    RiskDenied { reason: String },

    #[error("simulation failed: {0}")]
    SimulationFailed(String),

    #[error("internal error: {0}")]
    Internal(String),

    #[error("deserialization error: {0}")]
    DeserializationError(String),

    #[error("dimensional arithmetic error: {0}")]
    DimensionalArithmetic(#[from] super::currency::DimensionalArithmeticError),

    #[error("pump.fun bonding curve complete/migrated")]
    PumpFunComplete,

    #[error("pump.fun reserve exceeded: requested {requested}, available {available}")]
    PumpFunReserveExceeded { requested: u64, available: u64 },

    #[error("pool disabled: {0}")]
    PoolDisabled(String),

    #[error("incomplete pool state: {0}")]
    IncompleteState(String),

    #[error("invalid token: {0}")]
    InvalidToken(String),

    #[error("zero amount input")]
    ZeroAmount,

    #[error("insufficient liquidity: {0}")]
    InsufficientLiquidity(String),

    #[error("calculation error: {0}")]
    CalculationError(String),

    #[error("unsupported operation: {0}")]
    UnsupportedOperation(String),

    #[error("pool migrated: {0}")]
    PoolMigrated(String),
}

// ============================================================================
// Regression tests — address conflicts (P0 C-01 / C-02)
// ============================================================================
//
// These tests lock the canonical on-chain addresses in place so the historical
// typo values (which existed only in the audit document, never in source) can
// never be reintroduced:
//
//   C-01: Kamino USDC reserve must match arb_constants::KAMINO_USDC_RESERVE
//         (canonical ...NbBmgJ59, NOT the audit-doc typo ...Ug6xK25).
//   C-02: Raydium v4 pool_coin_vault must match arb-constants::RAYDIUM_SOL_VAULT
//         (canonical ...JtteuvPpz, NOT the audit-doc typo ...Ww5jz).
//
// On-chain identity of each address remains UNVERIFIED in this repo; these tests
// only enforce intra-repo consistency. Verify against mainnet account state
// before mainnet use.
//
// C-02 (Raydium v4 pool_coin_vault) now lives in arb-tx (the struct moved there
// during the arb-tx protocols refactor); see
// crates/arb-tx/src/protocols/raydium_v4.rs.
