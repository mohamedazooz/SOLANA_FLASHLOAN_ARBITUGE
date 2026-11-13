//! Error types for transaction assembly, ALT compilation, and bundle execution.

use solana_pubkey::Pubkey;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TxError {
    #[error("Missing signer for transaction")]
    MissingSigner,

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("Invalid hop count: {0} (must be 2 or 3)")]
    InvalidHopCount(usize),

    #[error("Zero borrow amount")]
    ZeroBorrowAmount,

    #[error("route does not return to borrowed mint: in={in_mint} out={out_mint}")]
    NonClosingRoute { in_mint: String, out_mint: String },

    #[error("Transaction size exceeds Solana MTU: {size} bytes > {limit} bytes limit")]
    TransactionTooLarge { size: usize, limit: usize },

    #[error("Account resolution failure for pubkey: {0}")]
    AccountResolutionError(Pubkey),

    #[error("Address lookup table compilation error: {0}")]
    AltCompilationError(String),

    #[error("Instruction encoding error: {0}")]
    InstructionError(String),

    #[error("Cryptographic signing error: {0}")]
    SigningError(String),

    #[error("Serialization failed: {0}")]
    Serialization(String),

    #[error("Kamino flash loan error: {0}")]
    KaminoError(String),

    #[error("Net profit invariant violation: {0} lamports < 30,000 floor")]
    Unprofitable(u64),

    #[error("Template offset resolution error: {0}")]
    TemplateOffsetError(String),

    #[error("Missing execution accounts for pool {pool_id}: {reason}")]
    MissingExecutionAccounts { pool_id: Pubkey, reason: String },

    #[error("Invalid execution accounts for pool {pool_id}: {reason}")]
    InvalidExecutionAccounts { pool_id: Pubkey, reason: String },

    #[error(
        "Stale execution accounts for pool {pool_id}: resolved at slot {resolved_slot}, current slot {current_slot}"
    )]
    StaleExecutionAccounts {
        pool_id: Pubkey,
        resolved_slot: u64,
        current_slot: u64,
    },

    #[error("Unsupported protocol for live execution: {0}")]
    UnsupportedProtocol(String),

    #[error("Multi-hop route is missing hop_inputs")]
    MissingHopInputs,

    #[error("Final hop expected output {expected} below profit floor {floor}")]
    ProfitFloorNotMet { expected: u64, floor: u64 },

    #[error("Flash-loan base mint must be WSOL; got {0}")]
    NonWsolFlashLoan(String),

    #[error("Unsupported transaction version: {0}")]
    UnsupportedTransactionVersion(u8),

    #[error("Transaction sanitization failure: {0}")]
    SanitizationError(String),

    #[error("Atomicity violation: {0}")]
    AtomicityViolation(String),
}
