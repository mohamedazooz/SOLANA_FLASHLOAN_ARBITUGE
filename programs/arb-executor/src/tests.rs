//! Comprehensive unit tests for `arb-executor`.

use crate::error::ExecutorError;
use crate::instructions::{ExecuteRouteArgs, HopDescriptor, process_execute_route};
use crate::validation::{
    DexProtocolTag, METEORA_DLMM_PROGRAM_ID, ORCA_WHIRLPOOL_PROGRAM_ID, PUMP_FUN_PROGRAM_ID,
    PUMP_SWAP_PROGRAM_ID, RAYDIUM_CLMM_PROGRAM_ID, RAYDIUM_CPMM_PROGRAM_ID, SPL_TOKEN_PROGRAM_ID,
    validate_account_signer, validate_account_writable, validate_dex_program,
    validate_distinct_pools_2leg,
};
use crate::{
    DISCRIMINATOR_EXECUTE_ROUTE, DISCRIMINATOR_PROFIT_GUARD, PROFIT_GUARD_DISCRIMINATOR,
    process_instruction,
};
use solana_account_info::AccountInfo;
use solana_address::Address;
use solana_program_error::ProgramError;

fn create_mock_account<'a>(
    key: &'a Address,
    is_signer: bool,
    is_writable: bool,
    lamports: &'a mut u64,
    data: &'a mut [u8],
    owner: &'a Address,
) -> AccountInfo<'a> {
    AccountInfo::new(key, is_signer, is_writable, lamports, data, owner, false)
}

#[test]
fn test_allowlisted_dex_programs() {
    assert_eq!(
        validate_dex_program(&METEORA_DLMM_PROGRAM_ID),
        Ok(DexProtocolTag::MeteoraDlmm)
    );
    assert_eq!(
        validate_dex_program(&RAYDIUM_CPMM_PROGRAM_ID),
        Ok(DexProtocolTag::RaydiumCpmm)
    );
    assert_eq!(
        validate_dex_program(&RAYDIUM_CLMM_PROGRAM_ID),
        Ok(DexProtocolTag::RaydiumClmm)
    );
    assert_eq!(
        validate_dex_program(&ORCA_WHIRLPOOL_PROGRAM_ID),
        Ok(DexProtocolTag::OrcaWhirlpool)
    );
    assert_eq!(
        validate_dex_program(&PUMP_SWAP_PROGRAM_ID),
        Ok(DexProtocolTag::PumpSwap)
    );
    assert_eq!(
        validate_dex_program(&PUMP_FUN_PROGRAM_ID),
        Ok(DexProtocolTag::PumpSwap)
    );

    let unauthorized = Address::from_str_const("11111111111111111111111111111111");
    assert_eq!(
        validate_dex_program(&unauthorized),
        Err(ExecutorError::UnauthorizedProgramId)
    );
}

#[test]
fn test_distinct_pools_2leg_validation() {
    let p1 = Address::from_str_const("LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo");
    let p2 = Address::from_str_const("CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C");

    assert!(validate_distinct_pools_2leg(&p1, &p2).is_ok());
    assert_eq!(
        validate_distinct_pools_2leg(&p1, &p1),
        Err(ExecutorError::PoolsNotDistinct.into())
    );
}

#[test]
fn test_signer_and_writable_privilege_checks() {
    let key = Address::from([7u8; 32]);
    let owner = SPL_TOKEN_PROGRAM_ID;
    let mut lamports = 100;
    let mut data = vec![0u8; 165];

    // Non-signer, non-writable
    let acc_ro = create_mock_account(&key, false, false, &mut lamports, &mut data, &owner);
    assert_eq!(
        validate_account_signer(&acc_ro),
        Err(ExecutorError::MissingRequiredSigner.into())
    );
    assert_eq!(
        validate_account_writable(&acc_ro),
        Err(ExecutorError::AccountNotWritable.into())
    );

    // Signer, writable
    let acc_rw_signer = create_mock_account(&key, true, true, &mut lamports, &mut data, &owner);
    assert!(validate_account_signer(&acc_rw_signer).is_ok());
    assert!(validate_account_writable(&acc_rw_signer).is_ok());
}

#[test]
fn test_execute_route_deserialization_and_validation() {
    let hop0 = HopDescriptor {
        protocol: DexProtocolTag::RaydiumCpmm as u8,
        flags: 0,
        account_count: 14,
        _reserved: [0; 5],
        amount_in: 1_000_000_000,
        min_amount_out: 1_050_000_000,
    };
    let hop1 = HopDescriptor {
        protocol: DexProtocolTag::MeteoraDlmm as u8,
        flags: 0,
        account_count: 17,
        _reserved: [0; 5],
        amount_in: 1_050_000_000,
        min_amount_out: 1_100_000_000,
    };
    let dummy_hop = HopDescriptor {
        protocol: 0,
        flags: 0,
        account_count: 0,
        _reserved: [0; 5],
        amount_in: 0,
        min_amount_out: 0,
    };

    let args = ExecuteRouteArgs {
        hop_count: 2,
        _reserved: [0; 7],
        min_final_delta: 50_000_000,
        hops: [hop0, hop1, dummy_hop],
    };

    let bytes = bytemuck::bytes_of(&args);
    let parsed = ExecuteRouteArgs::from_slice(bytes).expect("valid parse");
    assert_eq!(parsed.hop_count, 2);
    assert_eq!(parsed.min_final_delta, 50_000_000);
    assert_eq!(parsed.hops[0].amount_in, 1_000_000_000);
    assert_eq!(parsed.hops[0].min_amount_out, 1_050_000_000);
    assert_eq!(parsed.hops[1].amount_in, 1_050_000_000);
    assert_eq!(parsed.hops[1].min_amount_out, 1_100_000_000);
}

#[test]
fn test_execute_route_rejects_insolvent_hops() {
    // Leg 1 produces min 100, but Leg 2 requires 101 -> Must reject!
    let hop0 = HopDescriptor {
        protocol: 1,
        flags: 0,
        account_count: 14,
        _reserved: [0; 5],
        amount_in: 90,
        min_amount_out: 100,
    };
    let hop1 = HopDescriptor {
        protocol: 0,
        flags: 0,
        account_count: 17,
        _reserved: [0; 5],
        amount_in: 101, // Insolvent!
        min_amount_out: 110,
    };
    let dummy_hop = HopDescriptor {
        protocol: 0,
        flags: 0,
        account_count: 0,
        _reserved: [0; 5],
        amount_in: 0,
        min_amount_out: 0,
    };

    let args = ExecuteRouteArgs {
        hop_count: 2,
        _reserved: [0; 7],
        min_final_delta: 10,
        hops: [hop0, hop1, dummy_hop],
    };

    let bytes = bytemuck::bytes_of(&args);
    let accounts: [AccountInfo; 0] = [];
    let res = process_execute_route(&accounts, bytes);
    assert_eq!(
        res,
        Err(ProgramError::Custom(
            ExecutorError::Leg2InputExceedsLeg1Output as u32
        ))
    );
}

#[test]
fn test_execute_route_rejects_invalid_hop_counts() {
    let dummy_hop = HopDescriptor {
        protocol: 0,
        flags: 0,
        account_count: 0,
        _reserved: [0; 5],
        amount_in: 0,
        min_amount_out: 0,
    };

    // 0 hops -> rejected
    let args_0 = ExecuteRouteArgs {
        hop_count: 0,
        _reserved: [0; 7],
        min_final_delta: 10,
        hops: [dummy_hop; 3],
    };
    let accounts: [AccountInfo; 0] = [];
    assert_eq!(
        process_execute_route(&accounts, bytemuck::bytes_of(&args_0)),
        Err(ProgramError::Custom(ExecutorError::InvalidHopCount as u32))
    );

    // 1 hop -> rejected (must be 2 or 3)
    let args_1 = ExecuteRouteArgs {
        hop_count: 1,
        _reserved: [0; 7],
        min_final_delta: 10,
        hops: [dummy_hop; 3],
    };
    assert_eq!(
        process_execute_route(&accounts, bytemuck::bytes_of(&args_1)),
        Err(ProgramError::Custom(ExecutorError::InvalidHopCount as u32))
    );

    // 4 hops -> rejected
    let args_4 = ExecuteRouteArgs {
        hop_count: 4,
        _reserved: [0; 7],
        min_final_delta: 10,
        hops: [dummy_hop; 3],
    };
    assert_eq!(
        process_execute_route(&accounts, bytemuck::bytes_of(&args_4)),
        Err(ProgramError::Custom(ExecutorError::InvalidHopCount as u32))
    );
}

#[test]
fn test_process_instruction_discriminators_routing() {
    let dummy_program_id = Address::default();
    let accounts: [AccountInfo; 0] = [];

    // Empty instruction data
    assert_eq!(
        process_instruction(&dummy_program_id, &accounts, &[]),
        Err(ProgramError::Custom(
            ExecutorError::InvalidInstructionData as u32
        ))
    );

    // Discriminator 2 (profit_guard) with insufficient data
    let short_data = [DISCRIMINATOR_PROFIT_GUARD, 1, 2, 3];
    assert_eq!(
        process_instruction(&dummy_program_id, &accounts, &short_data),
        Err(ProgramError::Custom(
            ExecutorError::NotEnoughAccountInfos as u32
        ))
    );

    // 8-byte discriminator for profit guard
    let mut disc8_data = PROFIT_GUARD_DISCRIMINATOR.to_vec();
    disc8_data.extend_from_slice(&100u64.to_le_bytes());
    assert_eq!(
        process_instruction(&dummy_program_id, &accounts, &disc8_data),
        Err(ProgramError::Custom(
            ExecutorError::NotEnoughAccountInfos as u32
        ))
    );

    // Discriminator 1 (execute_route) with insufficient data
    let route_1byte = [DISCRIMINATOR_EXECUTE_ROUTE, 0, 0];
    assert_eq!(
        process_instruction(&dummy_program_id, &accounts, &route_1byte),
        Err(ProgramError::Custom(
            ExecutorError::InvalidInstructionData as u32
        ))
    );
}
