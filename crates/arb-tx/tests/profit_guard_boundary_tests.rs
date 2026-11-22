//! Feature 7: On-Chain Balance Delta Profit Guard — Tier 2 Boundary Tests.
//!
//! Validates exact breakeven boundary, 1-lamport shortfall rejection,
//! balance subtraction underflow, baseline overflow protection, and intermediate
//! inventory subsidization prevention.

use arb_executor::error::ExecutorError;
use arb_executor::instructions::execute_three_leg::{
    ExecuteThreeLegArgs, verify_sequential_balance_transitions,
};

fn make_sample_args(min_final_wsol_delta: u64, leg3_min_out: u64) -> ExecuteThreeLegArgs {
    ExecuteThreeLegArgs {
        leg1_amount_in: 1_000_000_000,
        leg1_min_amount_out: 100_000_000,
        leg2_amount_in: 100_000_000,
        leg2_min_amount_out: 50_000_000,
        leg3_amount_in: 50_000_000,
        leg3_min_amount_out: leg3_min_out,
        min_final_wsol_delta,
        leg1_bin_array_count: 1,
        leg2_bin_array_count: 1,
        leg3_bin_array_count: 1,
    }
}

/// Boundary Case 1: Exact breakeven delta boundary satisfies profit guard.
#[test]
fn test_boundary_f7_exact_breakeven_success() {
    let args = make_sample_args(10_000_000, 10_000_000);
    let wsol_baseline = 1_000_000_000;
    let wsol_pre_leg3 = 0;
    let wsol_final = 1_010_000_000; // delta is exactly 10_000_000
    let meme_b_initial = 1000;
    let meme_b_final = 1000;

    let res = verify_sequential_balance_transitions(
        0,
        wsol_baseline,
        wsol_pre_leg3,
        wsol_final,
        meme_b_initial,
        meme_b_final,
        &args,
    );
    assert_eq!(res, Ok(()), "exact breakeven delta boundary must succeed");
}

/// Boundary Case 2: Exactly 1-lamport shortfall below required delta is rejected.
#[test]
fn test_boundary_f7_one_lamport_shortfall_rejection() {
    let args = make_sample_args(10_000_000, 10_000_000);
    let wsol_baseline = 1_000_000_000;
    let wsol_pre_leg3 = 0;
    let wsol_final = 1_009_999_999; // exactly 1 lamport short
    let meme_b_initial = 1000;
    let meme_b_final = 1000;

    let res = verify_sequential_balance_transitions(
        0,
        wsol_baseline,
        wsol_pre_leg3,
        wsol_final,
        meme_b_initial,
        meme_b_final,
        &args,
    );
    assert_eq!(
        res,
        Err(ExecutorError::FinalWsolDeltaBelowMinimum),
        "1-lamport shortfall must fail closed"
    );
}

/// Boundary Case 3: Balance decrease underflow (`wsol_final < wsol_pre_leg3`).
#[test]
fn test_boundary_f7_balance_decrease_underflow() {
    let args = make_sample_args(10_000_000, 10_000_000);
    let wsol_baseline = 1_000_000_000;
    let wsol_pre_leg3 = 600_000_000;
    let wsol_final = 500_000_000; // smaller than pre_leg3, would underflow
    let meme_b_initial = 1000;
    let meme_b_final = 1000;

    let res = verify_sequential_balance_transitions(
        0,
        wsol_baseline,
        wsol_pre_leg3,
        wsol_final,
        meme_b_initial,
        meme_b_final,
        &args,
    );
    assert_eq!(
        res,
        Err(ExecutorError::CalculationOverflow),
        "underflow must return CalculationOverflow"
    );
}

/// Boundary Case 4: Baseline plus hurdle overflow (`wsol_baseline + min_delta > u64::MAX`).
#[test]
fn test_boundary_f7_baseline_plus_hurdle_overflow() {
    let args = make_sample_args(10, 10);
    let wsol_baseline = u64::MAX - 5; // adding 10 overflows u64
    let wsol_pre_leg3 = 0;
    let wsol_final = u64::MAX;
    let meme_b_initial = 1000;
    let meme_b_final = 1000;

    let res = verify_sequential_balance_transitions(
        0,
        wsol_baseline,
        wsol_pre_leg3,
        wsol_final,
        meme_b_initial,
        meme_b_final,
        &args,
    );
    assert_eq!(
        res,
        Err(ExecutorError::CalculationOverflow),
        "overflow in required_final_wsol must return CalculationOverflow"
    );
}

/// Boundary Case 5: Intermediate token bleed by even 1 token unit fails closed.
#[test]
fn test_boundary_f7_intermediate_token_bleed_one_unit() {
    let args = make_sample_args(10_000_000, 10_000_000);
    let wsol_baseline = 1_000_000_000;
    let wsol_pre_leg3 = 0;
    let wsol_final = 1_010_000_000;
    let meme_b_initial = 1_000;
    let meme_b_final = 999; // 1 unit missing!

    let res = verify_sequential_balance_transitions(
        0,
        wsol_baseline,
        wsol_pre_leg3,
        wsol_final,
        meme_b_initial,
        meme_b_final,
        &args,
    );
    assert_eq!(
        res,
        Err(ExecutorError::IntermediateInventorySubsidized),
        "intermediate inventory deficit must fail closed"
    );
}

/// Boundary Case 6: `build_profit_guard_instruction` format and serialization.
#[test]
fn test_profit_guard_instruction_structure() {
    use arb_tx::{
        ARB_EXECUTOR_PROGRAM_ID_STR, DISCRIMINATOR_PROFIT_GUARD, build_profit_guard_instruction,
    };
    use solana_pubkey::Pubkey;
    use std::str::FromStr;

    let target_account = Pubkey::new_unique();
    let min_balance: u64 = 5_432_100_000;
    let ix = build_profit_guard_instruction(target_account, min_balance);

    assert_eq!(
        ix.program_id,
        Pubkey::from_str(ARB_EXECUTOR_PROGRAM_ID_STR).unwrap()
    );
    assert_eq!(ix.accounts.len(), 1);
    assert_eq!(ix.accounts[0].pubkey, target_account);
    assert!(
        !ix.accounts[0].is_signer,
        "profit guard must not require signer"
    );
    assert!(!ix.accounts[0].is_writable, "profit guard must be readonly");

    // Exactly 9 bytes: [DISCRIMINATOR_PROFIT_GUARD (1 byte)][min_balance (8 bytes LE)]
    assert_eq!(ix.data.len(), 9);
    assert_eq!(ix.data[0], DISCRIMINATOR_PROFIT_GUARD);
    assert_eq!(&ix.data[1..9], &min_balance.to_le_bytes());
}

/// Boundary Case 7: Atomic integrity enforces `profit_guard` strictly precedes `repay`.
#[test]
fn test_atomic_integrity_enforces_profit_guard_ordering() {
    use arb_tx::{
        CompiledInstruction, ExecutionMode, build_profit_guard_instruction, verify_atomic_integrity,
    };
    use solana_pubkey::Pubkey;

    let payer = Pubkey::new_unique();
    let wsol_ata = Pubkey::new_unique();
    let compute_budget = Pubkey::new_from_array(arb_constants::COMPUTE_BUDGET_PROGRAM_ID);
    let klend = Pubkey::new_unique();
    let dex = Pubkey::new_unique();

    let cb_ix = CompiledInstruction {
        program_id: compute_budget,
        accounts: vec![],
        data: vec![0],
    };
    let borrow_ix = CompiledInstruction {
        program_id: klend,
        accounts: vec![arb_tx::AccountMeta::new(payer, true)],
        data: vec![0; 16],
    };
    let swap_ix = CompiledInstruction {
        program_id: dex,
        accounts: vec![],
        data: vec![1; 24],
    };
    let guard_ix = build_profit_guard_instruction(wsol_ata, 1_000_000);
    let repay_ix = CompiledInstruction {
        program_id: klend,
        accounts: vec![arb_tx::AccountMeta::new(payer, true)],
        data: {
            let mut d = vec![0; 8];
            d.extend_from_slice(&1_000_000u64.to_le_bytes());
            d
        },
    };

    // Valid order: CB -> Borrow -> Swap -> Guard -> Repay
    let valid_ixs = vec![
        cb_ix.clone(),
        borrow_ix.clone(),
        swap_ix.clone(),
        guard_ix.clone(),
        repay_ix.clone(),
    ];
    let res = verify_atomic_integrity(
        ExecutionMode::SingleTransaction,
        &valid_ixs,
        Some(1),
        Some(4),
        1_000_000,
        1_000_000,
        0,
    );
    assert!(res.is_ok(), "valid ordering must pass");
    assert!(res.unwrap().is_atomic);

    // Invalid order: Guard placed after Repay
    let invalid_ixs = vec![cb_ix, borrow_ix, swap_ix, repay_ix, guard_ix];
    let res_invalid = verify_atomic_integrity(
        ExecutionMode::SingleTransaction,
        &invalid_ixs,
        Some(1),
        Some(3),
        1_000_000,
        1_000_000,
        0,
    );
    assert!(res_invalid.is_ok());
    let verification = res_invalid.unwrap();
    assert!(!verification.is_atomic);
    assert!(
        verification
            .rejection_reason
            .unwrap()
            .contains("profit guard index 4 must precede repay index 3")
    );
}
