//! Feature 6: On-Chain Program ID & Privilege Validation — Tier 2 Boundary Tests.
//!
//! Validates required signer flags, verified DEX and Memo program IDs,
//! event authority checks, and strictly distinct pool addresses in on-chain execution.

use arb_executor::AccountInfo;
use arb_executor::error::ExecutorError;
use arb_executor::validation::{
    MEMO_PROGRAM_ID, METEORA_DLMM_PROGRAM_ID, METEORA_EVENT_AUTHORITY, validate_distinct_pools,
    validate_protocol_programs,
};
use solana_address::Address;

fn make_account_info<'a>(
    key: &'a Address,
    is_signer: bool,
    is_writable: bool,
    lamports: &'a mut u64,
    data: &'a mut [u8],
    owner: &'a Address,
) -> AccountInfo<'a> {
    AccountInfo::new(key, is_signer, is_writable, lamports, data, owner, false)
}

/// Boundary Case 1: Unauthorized user signer rejection (`user.is_signer == false`).
#[test]
fn test_boundary_f6_unauthorized_user_signer_rejection() {
    let dlmm_key = METEORA_DLMM_PROGRAM_ID;
    let memo_key = MEMO_PROGRAM_ID;
    let event_auth_key = METEORA_EVENT_AUTHORITY;
    let user_key = Address::from([1u8; 32]);
    let system_prog = Address::from([0u8; 32]);

    let mut l1 = 0;
    let mut l2 = 0;
    let mut l3 = 0;
    let mut l4 = 0;
    let mut d1 = [];
    let mut d2 = [];
    let mut d3 = [];
    let mut d4 = [];

    let dlmm_acc = make_account_info(&dlmm_key, false, false, &mut l1, &mut d1, &system_prog);
    let memo_acc = make_account_info(&memo_key, false, false, &mut l2, &mut d2, &system_prog);
    let event_acc = make_account_info(
        &event_auth_key,
        false,
        false,
        &mut l3,
        &mut d3,
        &system_prog,
    );
    // User is NOT a signer
    let user_acc = make_account_info(&user_key, false, true, &mut l4, &mut d4, &system_prog);

    let res = validate_protocol_programs(&dlmm_acc, &memo_acc, &event_acc, &user_acc);
    assert_eq!(res, Err(ExecutorError::MissingRequiredSigner.into()));
}

/// Boundary Case 2: Rogue Meteora DLMM program ID rejection.
#[test]
fn test_boundary_f6_rogue_dlmm_program_id_rejection() {
    let rogue_dlmm = Address::from([0xAA; 32]);
    let memo_key = MEMO_PROGRAM_ID;
    let event_auth_key = METEORA_EVENT_AUTHORITY;
    let user_key = Address::from([1u8; 32]);
    let system_prog = Address::from([0u8; 32]);

    let mut l1 = 0;
    let mut l2 = 0;
    let mut l3 = 0;
    let mut l4 = 0;
    let mut d1 = [];
    let mut d2 = [];
    let mut d3 = [];
    let mut d4 = [];

    let dlmm_acc = make_account_info(&rogue_dlmm, false, false, &mut l1, &mut d1, &system_prog);
    let memo_acc = make_account_info(&memo_key, false, false, &mut l2, &mut d2, &system_prog);
    let event_acc = make_account_info(
        &event_auth_key,
        false,
        false,
        &mut l3,
        &mut d3,
        &system_prog,
    );
    let user_acc = make_account_info(&user_key, true, true, &mut l4, &mut d4, &system_prog);

    let res = validate_protocol_programs(&dlmm_acc, &memo_acc, &event_acc, &user_acc);
    assert_eq!(res, Err(ExecutorError::InvalidMeteoraProgramId.into()));
}

/// Boundary Case 3: Rogue Memo program ID rejection.
#[test]
fn test_boundary_f6_rogue_memo_program_id_rejection() {
    let dlmm_key = METEORA_DLMM_PROGRAM_ID;
    let rogue_memo = Address::from([0xBB; 32]);
    let event_auth_key = METEORA_EVENT_AUTHORITY;
    let user_key = Address::from([1u8; 32]);
    let system_prog = Address::from([0u8; 32]);

    let mut l1 = 0;
    let mut l2 = 0;
    let mut l3 = 0;
    let mut l4 = 0;
    let mut d1 = [];
    let mut d2 = [];
    let mut d3 = [];
    let mut d4 = [];

    let dlmm_acc = make_account_info(&dlmm_key, false, false, &mut l1, &mut d1, &system_prog);
    let memo_acc = make_account_info(&rogue_memo, false, false, &mut l2, &mut d2, &system_prog);
    let event_acc = make_account_info(
        &event_auth_key,
        false,
        false,
        &mut l3,
        &mut d3,
        &system_prog,
    );
    let user_acc = make_account_info(&user_key, true, true, &mut l4, &mut d4, &system_prog);

    let res = validate_protocol_programs(&dlmm_acc, &memo_acc, &event_acc, &user_acc);
    assert_eq!(res, Err(ExecutorError::InvalidMemoProgramId.into()));
}

/// Boundary Case 4: Rogue Event Authority rejection.
#[test]
fn test_boundary_f6_rogue_event_authority_rejection() {
    let dlmm_key = METEORA_DLMM_PROGRAM_ID;
    let memo_key = MEMO_PROGRAM_ID;
    let rogue_event_auth = Address::from([0xCC; 32]);
    let user_key = Address::from([1u8; 32]);
    let system_prog = Address::from([0u8; 32]);

    let mut l1 = 0;
    let mut l2 = 0;
    let mut l3 = 0;
    let mut l4 = 0;
    let mut d1 = [];
    let mut d2 = [];
    let mut d3 = [];
    let mut d4 = [];

    let dlmm_acc = make_account_info(&dlmm_key, false, false, &mut l1, &mut d1, &system_prog);
    let memo_acc = make_account_info(&memo_key, false, false, &mut l2, &mut d2, &system_prog);
    let event_acc = make_account_info(
        &rogue_event_auth,
        false,
        false,
        &mut l3,
        &mut d3,
        &system_prog,
    );
    let user_acc = make_account_info(&user_key, true, true, &mut l4, &mut d4, &system_prog);

    let res = validate_protocol_programs(&dlmm_acc, &memo_acc, &event_acc, &user_acc);
    assert_eq!(res, Err(ExecutorError::InvalidEventAuthority.into()));
}

/// Boundary Case 5: Distinct pools triangular collision permutations ((P1, P1, P3), (P1, P2, P1), (P1, P2, P2), (P1, P1, P1)).
#[test]
fn test_boundary_f6_distinct_pools_triangular_collision_permutations() {
    let p1 = Address::from([1u8; 32]);
    let p2 = Address::from([2u8; 32]);
    let p3 = Address::from([3u8; 32]);

    // Valid: all three distinct
    assert!(validate_distinct_pools(&p1, &p2, &p3).is_ok());

    // Permutation 1: P1 == P2
    assert_eq!(
        validate_distinct_pools(&p1, &p1, &p3),
        Err(ExecutorError::PoolsNotDistinct.into())
    );

    // Permutation 2: P1 == P3
    assert_eq!(
        validate_distinct_pools(&p1, &p2, &p1),
        Err(ExecutorError::PoolsNotDistinct.into())
    );

    // Permutation 3: P2 == P3
    assert_eq!(
        validate_distinct_pools(&p1, &p2, &p2),
        Err(ExecutorError::PoolsNotDistinct.into())
    );

    // Permutation 4: P1 == P2 == P3
    assert_eq!(
        validate_distinct_pools(&p1, &p1, &p1),
        Err(ExecutorError::PoolsNotDistinct.into())
    );
}
