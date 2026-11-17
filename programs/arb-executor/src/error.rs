//! Custom errors for the Arbitrage Executor program.

use solana_program_error::ProgramError;
use thiserror::Error;

#[derive(Error, Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u32)]
pub enum ExecutorError {
    #[error("Invalid instruction data")]
    InvalidInstructionData = 6000,

    #[error("Invalid Meteora DLMM program ID")]
    InvalidMeteoraProgramId = 6001,

    #[error("Invalid token program ID")]
    InvalidTokenProgramId = 6002,

    #[error("Invalid memo program ID")]
    InvalidMemoProgramId = 6003,

    #[error("Invalid event authority PDA")]
    InvalidEventAuthority = 6004,

    #[error("User must be a transaction signer")]
    MissingRequiredSigner = 6005,

    #[error("Not enough account infos provided")]
    NotEnoughAccountInfos = 6006,

    #[error("Leg 1 output received is below min_amount_out")]
    Leg1OutputBelowMinimum = 6007,

    #[error("Leg 2 input amount exceeds Leg 1 received output")]
    Leg2InputExceedsLeg1Output = 6008,

    #[error("Leg 2 output received is below min_amount_out")]
    Leg2OutputBelowMinimum = 6009,

    #[error("Leg 3 input amount exceeds Leg 2 received output")]
    Leg3InputExceedsLeg2Output = 6010,

    #[error("Leg 3 output received is below min_amount_out")]
    Leg3OutputBelowMinimum = 6011,

    #[error(
        "Final WSOL net delta is below required threshold to cover repayment and minimum profit"
    )]
    FinalWsolDeltaBelowMinimum = 6012,

    #[error("Pre-existing intermediate inventory would subsidize arbitrage execution")]
    IntermediateInventorySubsidized = 6013,

    #[error("Arithmetic calculation overflowed")]
    CalculationOverflow = 6014,

    #[error("Invalid token account owner or mint mismatch")]
    InvalidTokenAccount = 6015,

    #[error("Arbitrage route pools must all be distinct")]
    PoolsNotDistinct = 6016,

    #[error("Token program mismatch")]
    MismatchedTokenProgram = 6017,

    #[error("Invalid or empty bin arrays")]
    InvalidBinArray = 6018,
}

impl From<ExecutorError> for ProgramError {
    fn from(e: ExecutorError) -> Self {
        ProgramError::Custom(e as u32)
    }
}
