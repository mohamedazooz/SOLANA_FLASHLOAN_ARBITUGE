//! Simulation & Profitability Gate (Phase 4).
//!
//! # Responsibilities
//! 1. Non-broadcast simulation result evaluation against live mainnet bank state.
//! 2. Accurate, non-overlapping execution cost breakdown:
//!    - Base transaction fee (5,000 lamports per signature)
//!    - Priority fee (CU limit * micro-lamports / 1,000,000)
//!    - Jito tip (dynamic bundle auction bid)
//!    - DEX protocol swap fees
//!    - Token-2022 transfer fees / hooks
//!    - Kamino / flash-loan borrow fee
//!    - Account rent (when new accounts/ATAs are created)
//! 3. Rejection of stale quotes (slot lag > configured threshold) and inconsistent balances.
//! 4. Enforcement of the strict net profit hurdle (e.g. >= 50,000 lamports net floor).
//! 5. Explicit distinction between **Simulation Success** (necessary pre-flight gate)
//!    and **Actual Realized Inclusion** (probabilistic bundle settlement on-chain).

use crate::fork_sim::ForkSimResult;
use serde::{Deserialize, Serialize};

/// Default strict net profit hurdle floor in lamports.
pub const DEFAULT_STRICT_PROFIT_HURDLE_LAMPORTS: u64 = 50_000;

/// Default maximum allowable slot lag between quote observation and simulation bank context.
pub const DEFAULT_MAX_QUOTE_SLOT_LAG: u64 = 2;

/// Standard Solana base fee per signature in lamports.
pub const STANDARD_SOLANA_BASE_FEE_LAMPORTS: u64 = 5_000;

/// Granular, non-overlapping execution cost breakdown for an arbitrage transaction or bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ExecutionCostBreakdown {
    /// Base transaction fee (5,000 lamports per signature).
    pub base_fee_lamports: u64,
    /// Priority fee (CU limit * priority fee micro-lamports / 1_000_000).
    pub priority_fee_lamports: u64,
    /// Jito private bundle tip paid to the validator tip account.
    pub jito_tip_lamports: u64,
    /// Sum of all swap fees charged by pools along the route.
    pub dex_fees_lamports: u64,
    /// Token-2022 transfer fees or transfer hooks incurred along the hops.
    pub transfer_fees_lamports: u64,
    /// Flash loan borrow fee (e.g. Kamino flash fee).
    pub flash_borrow_fee_lamports: u64,
    /// Any rent-exemption lamports required for new accounts (e.g. temporary ATAs).
    pub rent_lamports: u64,
}

impl ExecutionCostBreakdown {
    /// Calculate total execution cost in lamports without double-counting.
    pub fn total_cost_lamports(&self) -> u64 {
        self.base_fee_lamports
            .saturating_add(self.priority_fee_lamports)
            .saturating_add(self.jito_tip_lamports)
            .saturating_add(self.dex_fees_lamports)
            .saturating_add(self.transfer_fees_lamports)
            .saturating_add(self.flash_borrow_fee_lamports)
            .saturating_add(self.rent_lamports)
    }

    /// Construct standard cost breakdown for a single-signature arbitrage transaction.
    pub fn standard(
        priority_fee_lamports: u64,
        jito_tip_lamports: u64,
        dex_fees_lamports: u64,
        transfer_fees_lamports: u64,
        flash_borrow_fee_lamports: u64,
    ) -> Self {
        Self {
            base_fee_lamports: STANDARD_SOLANA_BASE_FEE_LAMPORTS,
            priority_fee_lamports,
            jito_tip_lamports,
            dex_fees_lamports,
            transfer_fees_lamports,
            flash_borrow_fee_lamports,
            rent_lamports: 0,
        }
    }
}

/// Reasons why a simulated route is rejected by the profitability gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProfitabilityRejection {
    /// RPC simulation reported an execution error (instruction reverted, slippage exceeded, etc.).
    SimulationError(String),
    /// Quote is stale: bank slot advanced beyond acceptable tolerance.
    StaleQuote {
        quote_slot: u64,
        simulation_slot: u64,
        max_allowed_lag: u64,
    },
    /// Post-simulation token balance delta is less than the expected net amount.
    RealizedDeltaShortfall { realized: i64, expected: u64 },
    /// Net profit after all execution costs is below the strict profit hurdle floor.
    NetProfitBelowHurdle {
        gross_edge: u64,
        total_costs: u64,
        net_profit: u64,
        strict_hurdle: u64,
    },
    /// Pre/post balances are inconsistent or missing.
    InconsistentBalances(String),
    /// Simulation caused net lamport loss on fee payer exceeding gross edge.
    UnprofitableSimulation { fee_payer_delta: i128 },
}

/// Evaluated outcome of the simulation profitability gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimulationOutcome {
    /// Passed all pre-flight simulation and profitability checks.
    ///
    /// NOTE: `requires_inclusion_check` is ALWAYS true. Passing simulation does NOT
    /// guarantee block inclusion or on-chain execution; it qualifies the candidate
    /// for private Jito bundle auction submission.
    PassedGate {
        sim_result: ForkSimResult,
        cost_breakdown: ExecutionCostBreakdown,
        gross_edge_lamports: u64,
        net_profit_lamports: u64,
        strict_hurdle_lamports: u64,
        requires_inclusion_check: bool,
    },
    /// Route failed simulation or failed profitability hurdle.
    Rejected {
        reason: ProfitabilityRejection,
        sim_result: Option<ForkSimResult>,
    },
}

impl SimulationOutcome {
    /// Returns true if the transaction passed the simulation and profitability gate.
    pub fn is_passed(&self) -> bool {
        matches!(self, Self::PassedGate { .. })
    }

    /// Net profit in lamports if passed, None otherwise.
    pub fn net_profit(&self) -> Option<u64> {
        match self {
            Self::PassedGate {
                net_profit_lamports,
                ..
            } => Some(*net_profit_lamports),
            Self::Rejected { .. } => None,
        }
    }

    /// Rejection reason if rejected, None otherwise.
    pub fn rejection_reason(&self) -> Option<&ProfitabilityRejection> {
        match self {
            Self::Rejected { reason, .. } => Some(reason),
            Self::PassedGate { .. } => None,
        }
    }
}

/// Pre-flight Simulation & Profitability Gate engine.
#[derive(Debug, Clone)]
pub struct ProfitabilityGate {
    /// Strict minimum net profit floor required to dispatch a transaction.
    pub strict_profit_hurdle: u64,
    /// Maximum allowed lag in slots between quote generation and simulation execution.
    pub max_quote_slot_lag: u64,
}

impl Default for ProfitabilityGate {
    fn default() -> Self {
        Self {
            strict_profit_hurdle: DEFAULT_STRICT_PROFIT_HURDLE_LAMPORTS,
            max_quote_slot_lag: DEFAULT_MAX_QUOTE_SLOT_LAG,
        }
    }
}

impl ProfitabilityGate {
    /// Construct gate with custom hurdle and slot lag tolerance.
    pub fn new(strict_profit_hurdle: u64, max_quote_slot_lag: u64) -> Self {
        Self {
            strict_profit_hurdle,
            max_quote_slot_lag,
        }
    }

    /// Evaluate non-broadcast simulation result against quote freshness, realized deltas,
    /// and all-in execution cost breakdown.
    pub fn evaluate_simulation(
        &self,
        sim_result: ForkSimResult,
        quote_slot: Option<u64>,
        gross_edge_lamports: u64,
        cost_breakdown: ExecutionCostBreakdown,
    ) -> SimulationOutcome {
        // 1. Program execution success check
        if !sim_result.success {
            let err_msg = sim_result
                .error
                .clone()
                .unwrap_or_else(|| "Simulation failed with unknown program error".into());
            return SimulationOutcome::Rejected {
                reason: ProfitabilityRejection::SimulationError(err_msg),
                sim_result: Some(sim_result),
            };
        }

        // 2. Quote freshness / slot lag check
        if let (Some(q_slot), Some(ctx_slot)) = (quote_slot, sim_result.context_slot) {
            if ctx_slot > q_slot && (ctx_slot - q_slot) > self.max_quote_slot_lag {
                return SimulationOutcome::Rejected {
                    reason: ProfitabilityRejection::StaleQuote {
                        quote_slot: q_slot,
                        simulation_slot: ctx_slot,
                        max_allowed_lag: self.max_quote_slot_lag,
                    },
                    sim_result: Some(sim_result),
                };
            }
        }

        // 3. Realized delta check (if watch account token delta was measured)
        if let Some(realized) = sim_result.realized_net_lamports {
            if sim_result.expected_net_lamports > 0
                && realized < sim_result.expected_net_lamports as i64
            {
                return SimulationOutcome::Rejected {
                    reason: ProfitabilityRejection::RealizedDeltaShortfall {
                        realized,
                        expected: sim_result.expected_net_lamports,
                    },
                    sim_result: Some(sim_result),
                };
            }
        }

        // 4. Net profit hurdle evaluation: gross_edge - total_costs >= strict_profit_hurdle
        let total_costs = cost_breakdown.total_cost_lamports();
        let net_profit = gross_edge_lamports.saturating_sub(total_costs);

        if net_profit < self.strict_profit_hurdle {
            return SimulationOutcome::Rejected {
                reason: ProfitabilityRejection::NetProfitBelowHurdle {
                    gross_edge: gross_edge_lamports,
                    total_costs,
                    net_profit,
                    strict_hurdle: self.strict_profit_hurdle,
                },
                sim_result: Some(sim_result),
            };
        }

        // 5. Fee-payer sanity check (prevent pathological drain on payer)
        if sim_result.fee_payer_lamport_delta < 0 {
            let payer_loss = (-sim_result.fee_payer_lamport_delta) as u64;
            // Fee payer loss must not exceed total expected execution costs + 10,000 lamport buffer
            if payer_loss > total_costs.saturating_add(10_000) {
                return SimulationOutcome::Rejected {
                    reason: ProfitabilityRejection::UnprofitableSimulation {
                        fee_payer_delta: sim_result.fee_payer_lamport_delta,
                    },
                    sim_result: Some(sim_result),
                };
            }
        }

        // 6. Passed all gates: qualified candidate for Jito bundle execution
        SimulationOutcome::PassedGate {
            sim_result,
            cost_breakdown,
            gross_edge_lamports,
            net_profit_lamports: net_profit,
            strict_hurdle_lamports: self.strict_profit_hurdle,
            requires_inclusion_check: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_success_sim_result(context_slot: u64, realized_net: i64) -> ForkSimResult {
        ForkSimResult {
            success: true,
            context_slot: Some(context_slot),
            replacement_blockhash: Some("mock_blockhash".into()),
            error: None,
            units_consumed: 140_000,
            loaded_accounts_data_size: Some(65536),
            logs: vec!["Program 11111111111111111111111111111111 success".into()],
            inner_instruction_count: 2,
            pre_balances: vec![1_000_000_000],
            post_balances: vec![999_995_000],
            fee_payer_lamport_delta: -5_000,
            pre_token_balances: vec![],
            post_token_balances: vec![],
            realized_net_lamports: Some(realized_net),
            expected_net_lamports: realized_net.max(0) as u64,
            accounts: vec![],
            return_data: None,
        }
    }

    #[test]
    fn test_cost_breakdown_no_double_counting() {
        let costs = ExecutionCostBreakdown {
            base_fee_lamports: 5_000,
            priority_fee_lamports: 15_000,
            jito_tip_lamports: 25_000,
            dex_fees_lamports: 8_000,
            transfer_fees_lamports: 2_000,
            flash_borrow_fee_lamports: 3_000,
            rent_lamports: 0,
        };
        // 5k + 15k + 25k + 8k + 2k + 3k = 58,000 lamports
        assert_eq!(costs.total_cost_lamports(), 58_000);
    }

    #[test]
    fn test_profitability_gate_passes_when_profit_exceeds_hurdle() {
        let gate = ProfitabilityGate::new(50_000, 2);
        let sim = mock_success_sim_result(100, 200_000);
        let costs = ExecutionCostBreakdown::standard(10_000, 30_000, 5_000, 0, 0); // total 50,000
        let gross_edge = 200_000; // net = 150_000 >= 50_000 hurdle

        let outcome = gate.evaluate_simulation(sim, Some(100), gross_edge, costs);
        assert!(outcome.is_passed());
        assert_eq!(outcome.net_profit(), Some(150_000));
        match outcome {
            SimulationOutcome::PassedGate {
                requires_inclusion_check,
                ..
            } => assert!(requires_inclusion_check),
            _ => panic!("Expected PassedGate"),
        }
    }

    #[test]
    fn test_profitability_gate_rejects_when_net_below_hurdle() {
        let gate = ProfitabilityGate::new(50_000, 2);
        let sim = mock_success_sim_result(100, 60_000);
        let costs = ExecutionCostBreakdown::standard(10_000, 20_000, 5_000, 0, 0); // total 40,000
        let gross_edge = 60_000; // net = 20_000 < 50_000 hurdle

        let outcome = gate.evaluate_simulation(sim, Some(100), gross_edge, costs);
        assert!(!outcome.is_passed());
        match outcome.rejection_reason().unwrap() {
            ProfitabilityRejection::NetProfitBelowHurdle {
                net_profit,
                strict_hurdle,
                ..
            } => {
                assert_eq!(*net_profit, 20_000);
                assert_eq!(*strict_hurdle, 50_000);
            }
            other => panic!("Expected NetProfitBelowHurdle, got: {other:?}"),
        }
    }

    #[test]
    fn test_profitability_gate_rejects_stale_quote_slot_lag() {
        let gate = ProfitabilityGate::new(50_000, 2);
        // Quote was at slot 95, simulation ran at slot 100 (lag = 5 > max_lag 2)
        let sim = mock_success_sim_result(100, 300_000);
        let costs = ExecutionCostBreakdown::default();

        let outcome = gate.evaluate_simulation(sim, Some(95), 300_000, costs);
        assert!(!outcome.is_passed());
        match outcome.rejection_reason().unwrap() {
            ProfitabilityRejection::StaleQuote {
                quote_slot,
                simulation_slot,
                max_allowed_lag,
            } => {
                assert_eq!(*quote_slot, 95);
                assert_eq!(*simulation_slot, 100);
                assert_eq!(*max_allowed_lag, 2);
            }
            other => panic!("Expected StaleQuote, got: {other:?}"),
        }
    }

    #[test]
    fn test_profitability_gate_rejects_simulation_error() {
        let gate = ProfitabilityGate::default();
        let mut sim = mock_success_sim_result(100, 0);
        sim.success = false;
        sim.error = Some("Custom(6001): SlippageExceeded".into());

        let outcome =
            gate.evaluate_simulation(sim, Some(100), 100_000, ExecutionCostBreakdown::default());
        assert!(!outcome.is_passed());
        match outcome.rejection_reason().unwrap() {
            ProfitabilityRejection::SimulationError(err) => {
                assert!(err.contains("SlippageExceeded"));
            }
            other => panic!("Expected SimulationError, got: {other:?}"),
        }
    }
}
