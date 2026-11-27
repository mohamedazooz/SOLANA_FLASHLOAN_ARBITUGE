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
use solana_address::Address;
use solana_program_error::{ProgramError, ProgramResult};

pub use error::ExecutorError;
pub use instructions::{
    ExecuteRouteArgs, ExecuteThreeLegArgs, ExecuteThreeLegWithTipArgs, HopDescriptor,
    ProfitGuardArgs, process_execute_route, process_execute_three_leg,
    process_execute_three_leg_with_tip, process_profit_guard,
    verify_sequential_balance_transitions,
};
pub use solana_account_info::AccountInfo;
pub use validation::*;

/// Arbitrage Executor Program ID.
pub const ARB_EXECUTOR_PROGRAM_ID: Address =
    Address::from_str_const("ArbExec1111111111111111111111111111111111111");

/// 1-byte instruction discriminator for `execute_swap`.
pub const DISCRIMINATOR_EXECUTE_SWAP: u8 = 0;
/// 1-byte instruction discriminator for `execute_route`.
pub const DISCRIMINATOR_EXECUTE_ROUTE: u8 = 1;
/// 1-byte instruction discriminator for `profit_guard`.
pub const DISCRIMINATOR_PROFIT_GUARD: u8 = 2;

/// 8-byte instruction discriminator for `profit_guard`.
pub const PROFIT_GUARD_DISCRIMINATOR: [u8; 8] = [2, 0, 0, 0, 0, 0, 0, 0];
/// 8-byte instruction discriminator for `execute_route`.
pub const EXECUTE_ROUTE_DISCRIMINATOR: [u8; 8] = [1, 0, 0, 0, 0, 0, 0, 0];

/// 8-byte instruction discriminator for `execute_three_leg` (sha256("arb_executor:execute_three_leg")[..8]).
pub const EXECUTE_THREE_LEG_DISCRIMINATOR: [u8; 8] = [173, 221, 98, 44, 189, 21, 102, 184];

/// 8-byte instruction discriminator for `execute_three_leg_with_tip`
/// (sha256("arb_executor:execute_three_leg_with_tip")[..8]).
pub const EXECUTE_THREE_LEG_WITH_TIP_DISCRIMINATOR: [u8; 8] = [124, 34, 60, 60, 78, 94, 133, 177];

#[cfg(all(target_os = "solana", not(feature = "no-entrypoint")))]
solana_program_entrypoint::entrypoint!(process_instruction);

#[cfg(test)]
#[path = "tests.rs"]
mod tests_unit;

/// Program entrypoint dispatcher.
pub fn process_instruction<'a>(
    _program_id: &Address,
    accounts: &'a [AccountInfo<'a>],
    instruction_data: &[u8],
) -> ProgramResult {
    if instruction_data.is_empty() {
        return Err(ExecutorError::InvalidInstructionData.into());
    }

    // Check the full-width discriminators before the legacy 1-byte tags.
    // In particular, EXECUTE_ROUTE_DISCRIMINATOR and
    // PROFIT_GUARD_DISCRIMINATOR begin with 1 and 2 respectively; dispatching
    // on the first byte first would consume those instructions as legacy data
    // and make the 8-byte compatibility paths unreachable.
    if instruction_data.len() >= 8 {
        let (disc, data) = instruction_data.split_at(8);

        if disc == EXECUTE_THREE_LEG_DISCRIMINATOR {
            let args = ExecuteThreeLegArgs::try_from_slice(data)
                .map_err(|_| ProgramError::from(ExecutorError::InvalidInstructionData))?;
            return process_execute_three_leg(accounts, args);
        }

        if disc == EXECUTE_THREE_LEG_WITH_TIP_DISCRIMINATOR {
            let args = ExecuteThreeLegWithTipArgs::try_from_slice(data)
                .map_err(|_| ProgramError::from(ExecutorError::InvalidInstructionData))?;
            return process_execute_three_leg_with_tip(accounts, args);
        }

        if disc == PROFIT_GUARD_DISCRIMINATOR {
            return process_profit_guard(accounts, data);
        }

        if disc == EXECUTE_ROUTE_DISCRIMINATOR {
            return process_execute_route(accounts, data);
        }
    }

    // 1-byte legacy discriminator routing.
    match instruction_data[0] {
        DISCRIMINATOR_EXECUTE_SWAP | DISCRIMINATOR_EXECUTE_ROUTE => {
            return process_execute_route(accounts, &instruction_data[1..]);
        }
        DISCRIMINATOR_PROFIT_GUARD => {
            return process_profit_guard(accounts, &instruction_data[1..]);
        }
        _ => {}
    }

    Err(ExecutorError::InvalidInstructionData.into())
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

    #[test]
    fn test_verify_sequential_balance_transitions_happy_path() {
        let borrow = 1_000_000_000u64; // 1 SOL
        let repay = 1_000_011_529u64; // 1 SOL + fee
        let args = ExecuteThreeLegArgs {
            leg1_amount_in: borrow,
            leg1_min_amount_out: 50_000_000,
            leg2_amount_in: 50_000_000,
            leg2_min_amount_out: 200_000_000,
            leg3_amount_in: 200_000_000,
            leg3_min_amount_out: 1_020_000_000,
            min_final_wsol_delta: repay,
            leg1_bin_array_count: 1,
            leg2_bin_array_count: 1,
            leg3_bin_array_count: 1,
        };

        // User starts with 0 pre-borrow WSOL.
        // At entry to arb-executor, Kamino flash borrow funded 1 SOL.
        let wsol_initial = borrow;
        let wsol_baseline = 0;
        // In Leg 1, 1 SOL is spent. User has 0 WSOL.
        let wsol_pre_leg3 = 0;
        // In Leg 3, Leg 3 yields 1.05 SOL WSOL.
        let wsol_final = 1_050_000_000;
        let meme_b_initial = 0;
        let meme_b_final = 10_000; // unspent residual

        let res = verify_sequential_balance_transitions(
            wsol_initial,
            wsol_baseline,
            wsol_pre_leg3,
            wsol_final,
            meme_b_initial,
            meme_b_final,
            &args,
        );
        assert!(res.is_ok());
    }

    #[test]
    fn test_verify_sequential_balance_transitions_with_preexisting_wsol() {
        let borrow = 1_000_000_000u64; // 1 SOL
        let repay = 1_000_011_529u64;
        let args = ExecuteThreeLegArgs {
            leg1_amount_in: borrow,
            leg1_min_amount_out: 50_000_000,
            leg2_amount_in: 50_000_000,
            leg2_min_amount_out: 200_000_000,
            leg3_amount_in: 200_000_000,
            leg3_min_amount_out: 1_020_000_000,
            min_final_wsol_delta: repay,
            leg1_bin_array_count: 1,
            leg2_bin_array_count: 1,
            leg3_bin_array_count: 1,
        };

        // User already had 3 SOL WSOL before the transaction.
        let pre_existing_wsol = 3_000_000_000u64;
        let wsol_initial = pre_existing_wsol + borrow; // 4 SOL
        let wsol_baseline = pre_existing_wsol; // 3 SOL
        let wsol_pre_leg3 = pre_existing_wsol; // 3 SOL
        // Leg 3 yields 1.05 SOL, so final balance is 3 + 1.05 = 4.05 SOL
        let wsol_final = pre_existing_wsol + 1_050_000_000;

        let res = verify_sequential_balance_transitions(
            wsol_initial,
            wsol_baseline,
            wsol_pre_leg3,
            wsol_final,
            0,
            0,
            &args,
        );
        assert!(res.is_ok());
    }

    #[test]
    fn test_verify_sequential_balance_transitions_rejects_leg3_underflow() {
        let borrow = 1_000_000_000u64;
        let repay = 1_000_011_529u64;
        let args = ExecuteThreeLegArgs {
            leg1_amount_in: borrow,
            leg1_min_amount_out: 50_000_000,
            leg2_amount_in: 50_000_000,
            leg2_min_amount_out: 200_000_000,
            leg3_amount_in: 200_000_000,
            leg3_min_amount_out: 1_020_000_000,
            min_final_wsol_delta: repay,
            leg1_bin_array_count: 1,
            leg2_bin_array_count: 1,
            leg3_bin_array_count: 1,
        };

        // Leg 3 only yields 1.01 SOL (below leg3_min_amount_out 1.02 SOL)
        let wsol_final = 1_010_000_000;

        let res = verify_sequential_balance_transitions(borrow, 0, 0, wsol_final, 0, 0, &args);
        assert_eq!(res, Err(ExecutorError::Leg3OutputBelowMinimum));
    }

    #[test]
    fn test_verify_sequential_balance_transitions_rejects_repay_shortfall() {
        let borrow = 1_000_000_000u64;
        let repay = 1_000_011_529u64;
        let args = ExecuteThreeLegArgs {
            leg1_amount_in: borrow,
            leg1_min_amount_out: 50_000_000,
            leg2_amount_in: 50_000_000,
            leg2_min_amount_out: 200_000_000,
            leg3_amount_in: 200_000_000,
            leg3_min_amount_out: 990_000_000, // lower than repay
            min_final_wsol_delta: repay,
            leg1_bin_array_count: 1,
            leg2_bin_array_count: 1,
            leg3_bin_array_count: 1,
        };

        // Leg 3 yields 995_000_000 (satisfies leg3_min_out, but does not cover repay)
        let wsol_final = 995_000_000;

        let res = verify_sequential_balance_transitions(borrow, 0, 0, wsol_final, 0, 0, &args);
        assert_eq!(res, Err(ExecutorError::FinalWsolDeltaBelowMinimum));
    }

    #[test]
    fn test_verify_sequential_balance_transitions_rejects_intermediate_subsidy() {
        let borrow = 1_000_000_000u64;
        let repay = 1_000_011_529u64;
        let args = ExecuteThreeLegArgs {
            leg1_amount_in: borrow,
            leg1_min_amount_out: 50_000_000,
            leg2_amount_in: 50_000_000,
            leg2_min_amount_out: 200_000_000,
            leg3_amount_in: 200_000_000,
            leg3_min_amount_out: 1_020_000_000,
            min_final_wsol_delta: repay,
            leg1_bin_array_count: 1,
            leg2_bin_array_count: 1,
            leg3_bin_array_count: 1,
        };

        // Initial Meme B had 500 units, but final has only 400 units (100 units consumed!)
        let res = verify_sequential_balance_transitions(
            borrow,
            0,
            0,
            1_050_000_000,
            500,
            400, // VIOLATION
            &args,
        );
        assert_eq!(res, Err(ExecutorError::IntermediateInventorySubsidized));
    }

    #[test]
    fn test_verify_sequential_balance_transitions_rejects_preexisting_wsol_subsidy() {
        let borrow = 1_000_000_000u64;
        let repay = 1_000_011_529u64;
        let args = ExecuteThreeLegArgs {
            leg1_amount_in: borrow,
            leg1_min_amount_out: 50_000_000,
            leg2_amount_in: 50_000_000,
            leg2_min_amount_out: 200_000_000,
            leg3_amount_in: 200_000_000,
            leg3_min_amount_out: 1_000_011_529,
            min_final_wsol_delta: repay,
            leg1_bin_array_count: 1,
            leg2_bin_array_count: 1,
            leg3_bin_array_count: 1,
        };

        // User started with 5 SOL WSOL.
        let pre_existing_wsol = 5_000_000_000u64;
        let wsol_initial = pre_existing_wsol + borrow; // 6 SOL
        let wsol_baseline = pre_existing_wsol; // 5 SOL
        let wsol_pre_leg3 = pre_existing_wsol; // 5 SOL

        // Leg 3 failed and produced only 0.5 SOL instead of covering the flash loan!
        // So wsol_final is 5.5 SOL.
        // If we didn't check baseline, 5.5 SOL > 1.000011529 SOL (repay), but it would drain
        // 0.5 SOL of pre-existing user funds to pay Kamino!
        let wsol_final = pre_existing_wsol + 500_000_000;

        let res = verify_sequential_balance_transitions(
            wsol_initial,
            wsol_baseline,
            wsol_pre_leg3,
            wsol_final,
            0,
            0,
            &args,
        );
        assert_eq!(res, Err(ExecutorError::Leg3OutputBelowMinimum));
    }

    #[test]
    fn test_execute_three_leg_with_tip_args_serialization_roundtrip() {
        let args = ExecuteThreeLegWithTipArgs {
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
            tip_bps: 7_000,
        };

        let mut buf = Vec::new();
        args.serialize(&mut buf).expect("serialization succeeds");
        let deserialized =
            ExecuteThreeLegWithTipArgs::try_from_slice(&buf).expect("deserialization succeeds");
        assert_eq!(args, deserialized);
    }

    #[test]
    fn test_process_instruction_rejects_over_limit_tip_bps() {
        let args = ExecuteThreeLegWithTipArgs {
            leg1_amount_in: 1_000,
            leg1_min_amount_out: 1_000,
            leg2_amount_in: 1_000,
            leg2_min_amount_out: 1_000,
            leg3_amount_in: 1_000,
            leg3_min_amount_out: 1_000,
            min_final_wsol_delta: 1_000,
            leg1_bin_array_count: 0,
            leg2_bin_array_count: 0,
            leg3_bin_array_count: 0,
            tip_bps: 10_001, // valid borsh, invalid business rule
        };

        let mut data = Vec::new();
        data.extend_from_slice(&EXECUTE_THREE_LEG_WITH_TIP_DISCRIMINATOR);
        args.serialize(&mut data).unwrap();

        let res = process_instruction(&ARB_EXECUTOR_PROGRAM_ID, &[], &data);
        assert_eq!(
            res,
            Err(ProgramError::Custom(
                ExecutorError::TipDenominatorInvalid as u32
            ))
        );
    }

    #[test]
    fn test_process_instruction_rejects_missing_tip_account() {
        let args = ExecuteThreeLegWithTipArgs {
            leg1_amount_in: 1_000,
            leg1_min_amount_out: 1_000,
            leg2_amount_in: 1_000,
            leg2_min_amount_out: 1_000,
            leg3_amount_in: 1_000,
            leg3_min_amount_out: 1_000,
            min_final_wsol_delta: 1_000,
            leg1_bin_array_count: 0,
            leg2_bin_array_count: 0,
            leg3_bin_array_count: 0,
            tip_bps: 7_000,
        };

        let mut data = Vec::new();
        data.extend_from_slice(&EXECUTE_THREE_LEG_WITH_TIP_DISCRIMINATOR);
        args.serialize(&mut data).unwrap();

        let res = process_instruction(&ARB_EXECUTOR_PROGRAM_ID, &[], &data);
        assert_eq!(
            res,
            Err(ProgramError::Custom(
                ExecutorError::NotEnoughAccountInfos as u32
            ))
        );
    }
}
