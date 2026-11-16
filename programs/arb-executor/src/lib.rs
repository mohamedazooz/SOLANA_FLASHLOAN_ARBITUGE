//! Production-Grade Solana On-Chain Arbitrage Executor Program (`arb-executor`).
//!
//! Executes multi-leg atomic arbitrage via Cross-Program Invocation (CPI)
//! while strictly enforcing:
//! - Genuine on-chain CPI swap execution into Meteora DLMM Swap2
//! - Sequential token balance measurement and minimum output delta validation
//! - Zero pre-existing intermediate inventory subsidization
//! - Atomicity: any leg failure or economic deficit aborts the entire transaction

pub mod cpi;
pub mod error;
pub mod instructions;
pub mod validation;

use borsh::BorshDeserialize;
use solana_account_info::AccountInfo;
use solana_address::Address;
use solana_program_error::{ProgramError, ProgramResult};

pub use error::ExecutorError;
pub use instructions::{ExecuteThreeLegArgs, process_execute_three_leg};
pub use validation::*;

/// Arbitrage Executor Program ID.
pub const ARB_EXECUTOR_PROGRAM_ID: Address =
    Address::from_str_const("ArbExec1111111111111111111111111111111111111");

/// 8-byte instruction discriminator for `execute_three_leg` (sha256("arb_executor:execute_three_leg")[..8]).
pub const EXECUTE_THREE_LEG_DISCRIMINATOR: [u8; 8] = [173, 221, 98, 44, 189, 21, 102, 184];

#[cfg(all(target_os = "solana", not(feature = "no-entrypoint")))]
solana_program_entrypoint::entrypoint!(process_instruction);

/// Program entrypoint dispatcher.
pub fn process_instruction<'a>(
    _program_id: &Address,
    accounts: &'a [AccountInfo<'a>],
    instruction_data: &[u8],
) -> ProgramResult {
    if instruction_data.len() < 8 {
        return Err(ExecutorError::InvalidInstructionData.into());
    }

    let (disc, data) = instruction_data.split_at(8);
    if disc != EXECUTE_THREE_LEG_DISCRIMINATOR {
        return Err(ExecutorError::InvalidInstructionData.into());
    }

    let args = ExecuteThreeLegArgs::try_from_slice(data)
        .map_err(|_| ProgramError::from(ExecutorError::InvalidInstructionData))?;

    process_execute_three_leg(accounts, args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use borsh::BorshSerialize;

    #[test]
    fn test_execute_three_leg_args_serialization_roundtrip() {
        let args = ExecuteThreeLegArgs {
            leg1_amount_in: 1_000_000_000,
            leg1_min_amount_out: 25_000_000,
            leg2_amount_in: 25_000_000,
            leg2_min_amount_out: 50_000_000_000,
            leg3_amount_in: 50_000_000_000,
            leg3_min_amount_out: 1_050_000_000,
            min_final_wsol_delta: 1_010_000_000,
            leg1_bin_array_count: 2,
            leg2_bin_array_count: 3,
            leg3_bin_array_count: 2,
        };

        let mut buf = Vec::new();
        args.serialize(&mut buf).expect("serialization succeeds");

        let deserialized =
            ExecuteThreeLegArgs::try_from_slice(&buf).expect("deserialization succeeds");
        assert_eq!(args, deserialized);
    }

    #[test]
    fn test_process_instruction_rejects_truncated_data() {
        let res = process_instruction(&ARB_EXECUTOR_PROGRAM_ID, &[], &[1, 2, 3]);
        assert_eq!(
            res,
            Err(ProgramError::Custom(
                ExecutorError::InvalidInstructionData as u32
            ))
        );
    }

    #[test]
    fn test_process_instruction_rejects_bad_discriminator() {
        let bad_disc = [0u8; 8];
        let res = process_instruction(&ARB_EXECUTOR_PROGRAM_ID, &[], &bad_disc);
        assert_eq!(
            res,
            Err(ProgramError::Custom(
                ExecutorError::InvalidInstructionData as u32
            ))
        );
    }

    #[test]
    fn test_execute_three_leg_rejects_hop_input_contract_violation() {
        // leg1_min_amount_out < leg2_amount_in should fail before reading accounts!
        let args = ExecuteThreeLegArgs {
            leg1_amount_in: 100,
            leg1_min_amount_out: 90,
            leg2_amount_in: 95, // VIOLATION: 90 < 95
            leg2_min_amount_out: 200,
            leg3_amount_in: 200,
            leg3_min_amount_out: 110,
            min_final_wsol_delta: 105,
            leg1_bin_array_count: 1,
            leg2_bin_array_count: 1,
            leg3_bin_array_count: 1,
        };

        let mut data = Vec::new();
        data.extend_from_slice(&EXECUTE_THREE_LEG_DISCRIMINATOR);
        args.serialize(&mut data).unwrap();

        let res = process_instruction(&ARB_EXECUTOR_PROGRAM_ID, &[], &data);
        assert_eq!(
            res,
            Err(ProgramError::Custom(
                ExecutorError::Leg2InputExceedsLeg1Output as u32
            ))
        );
    }
}
