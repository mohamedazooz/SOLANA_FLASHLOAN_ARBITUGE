//! Feature 24: Local LiteSVM / Mollusk Transaction Simulation — Tier 2 Boundary Tests.
//!
//! Validates:
//! 1. Insufficient compute budget simulation failure is captured as typed SimulationError within rejection.
//! 2. Program log truncation bounds excessive output to MAX_CAPTURED_LOG_LINES and character limits.
//! 3. Clock drift / slot lag beyond max_quote_slot_lag rejects stale simulation (fail-closed).
//! 4. Zero balance delta under non-zero execution costs triggers NetProfitNegative rejection.
//! 5. Unregistered / empty payloads fail closed cleanly in local simulation.

use arb_sim::forensic::{MAX_CAPTURED_LOG_LINE_CHARS, MAX_CAPTURED_LOG_LINES};
use arb_sim::{
    ExecutionCostBreakdown, ForensicBundle, ForkSimResult, LocalSimResult, ProfitabilityGate,
    ProfitabilityRejection, SimulationOutcome, simulate_locally,
};

fn mock_sim_result(
    success: bool,
    slot: u64,
    realized_net: i64,
    error: Option<String>,
) -> ForkSimResult {
    ForkSimResult {
        success,
        context_slot: Some(slot),
        replacement_blockhash: Some("mock_hash".into()),
        error,
        units_consumed: 150_000,
        loaded_accounts_data_size: Some(65536),
        logs: vec!["Program 11111111111111111111111111111111 invoke".into()],
        inner_instruction_count: 1,
        pre_balances: vec![1_000_000_000],
        post_balances: vec![1_000_000_000],
        fee_payer_lamport_delta: -5_000,
        pre_token_balances: vec![],
        post_token_balances: vec![],
        realized_net_lamports: Some(realized_net),
        expected_net_lamports: realized_net.max(0) as u64,
        accounts: vec![],
        return_data: None,
    }
}

/// Boundary Case 1: Insufficient compute budget reverts with ComputationalBudgetExceeded.
#[test]
fn test_litesvm_insufficient_compute_budget_reverts() {
    let gate = ProfitabilityGate::default();
    let cu_error = Some("InstructionError(0, ComputationalBudgetExceeded)".to_string());
    let sim = mock_sim_result(false, 100, 0, cu_error);
    let costs = ExecutionCostBreakdown::standard(10_000, 10_000, 0, 0, 0);

    let outcome = gate.evaluate_simulation(sim, Some(100), 50_000, costs);
    assert!(!outcome.is_passed());

    match outcome {
        SimulationOutcome::Rejected {
            reason: ProfitabilityRejection::SimulationError(err),
            ..
        } => {
            assert!(
                err.contains("ComputationalBudgetExceeded"),
                "Error must identify computational budget exhaustion: {}",
                err
            );
        }
        other => panic!("Expected Rejected with SimulationError, got: {other:?}"),
    }
}

/// Boundary Case 2: Program log truncation bounds excessive log volume safely.
#[test]
fn test_litesvm_program_log_truncation_under_excessive_logging() {
    // Generate 500 excessive log lines with 800 characters each
    let long_line_padding = "x".repeat(800);
    let mut logs: Vec<String> = (0..500)
        .map(|i| format!("Program log: line {} {}", i, long_line_padding))
        .collect();
    logs.insert(0, "head".to_string());

    let bounded = ForensicBundle::bound_logs(&logs);

    // Number of lines must be clamped to MAX_CAPTURED_LOG_LINES
    assert_eq!(
        bounded.len(),
        MAX_CAPTURED_LOG_LINES,
        "Log line count must be clamped to MAX_CAPTURED_LOG_LINES"
    );
    assert_eq!(bounded[0], "head");

    // Each individual line must be bounded
    for line in &bounded[1..] {
        let max_allowed = MAX_CAPTURED_LOG_LINE_CHARS + "...[truncated]".len();
        assert!(
            line.chars().count() <= max_allowed,
            "Line length {} must be <= max_allowed {}",
            line.chars().count(),
            max_allowed
        );
    }
}

/// Boundary Case 3: Clock drift / slot lag rejects stale quotes (fail-closed).
#[test]
fn test_litesvm_sysvar_clock_drift_future_slot() {
    let gate = ProfitabilityGate::new(10_000, 2); // Max lag = 2 slots
    let sim = mock_sim_result(true, 105, 50_000, None); // Sim runs at slot 105
    let quote_slot = Some(100); // Quote generated at slot 100 (lag = 5 > 2)
    let costs = ExecutionCostBreakdown::standard(5_000, 5_000, 0, 0, 0);

    let outcome = gate.evaluate_simulation(sim, quote_slot, 50_000, costs);
    assert!(!outcome.is_passed());

    match outcome {
        SimulationOutcome::Rejected {
            reason:
                ProfitabilityRejection::StaleQuote {
                    quote_slot,
                    simulation_slot,
                    max_allowed_lag,
                },
            ..
        } => {
            assert_eq!(quote_slot, 100);
            assert_eq!(simulation_slot, 105);
            assert_eq!(max_allowed_lag, 2);
        }
        other => panic!("Expected Rejected with StaleQuote, got: {other:?}"),
    }
}

/// Boundary Case 4: Zero balance delta under non-zero costs triggers NetProfitNegative revert.
#[test]
fn test_litesvm_zero_balance_delta_triggers_profit_guard_revert() {
    let gate = ProfitabilityGate::new(10_000, 2);
    // Realized delta is 0 lamports, gross edge is 0
    let sim = mock_sim_result(true, 100, 0, None);
    let costs = ExecutionCostBreakdown::standard(5_000, 10_000, 2_000, 0, 0); // Total costs = 22,000

    let outcome = gate.evaluate_simulation(sim, Some(100), 0, costs);
    assert!(!outcome.is_passed());

    match outcome {
        SimulationOutcome::Rejected {
            reason:
                ProfitabilityRejection::NetProfitNegative {
                    gross_edge,
                    total_costs,
                },
            ..
        } => {
            assert_eq!(gross_edge, 0);
            assert_eq!(total_costs, 22_000);
        }
        other => panic!("Expected Rejected with NetProfitNegative, got: {other:?}"),
    }
}

/// Boundary Case 5: Unregistered / empty payloads fail closed in local simulation.
#[test]
fn test_litesvm_unregistered_program_id_fails_closed() {
    // 5a. Empty bytes payload
    let res_empty = simulate_locally(&[]);
    assert_eq!(
        res_empty,
        LocalSimResult {
            success: false,
            compute_units_consumed: 0,
            logs: Vec::new(),
            error: Some("empty transaction payload".into()),
        }
    );

    // 5b. Corrupt / invalid bytes payload
    let res_corrupt = simulate_locally(&[0xde, 0xad, 0xbe, 0xef]);
    assert!(!res_corrupt.success);
    assert!(res_corrupt.error.is_some());
}
