//! Meteora DLMM Mainnet Simulation Harness (`MeteoraDlmmMainnetSimulationHarness`).
//!
//! Submits constructed atomic two-swap Meteora DLMM transactions to real Mainnet
//! via JSON-RPC `simulateTransaction` with:
//! - `replaceRecentBlockhash: true`
//! - `sigVerify: false`
//! - `innerInstructions: true`
//! - Strictly read-only simulation: NEVER broadcasts to mempool or Jito bundle.
//!
//! Captures comprehensive diagnostic telemetry:
//! - Context slot, CU consumed, logs, inner instructions, balance deltas
//! - Discrepancy between expected DLMM quote and simulation bank state

use crate::fork_sim::{ForkSimulator, ForkState, SimTokenBalance};
use crate::profitability_gate::{ExecutionCostBreakdown, ProfitabilityGate, SimulationOutcome};
use arb_types::Result;
use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;

/// Full telemetry record for a simulated Meteora DLMM candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeteoraSimulationRecord {
    pub candidate_id: String,
    pub pool_a: Pubkey,
    pub pool_b: Pubkey,
    pub direction: String,
    pub input_amount: u64,
    pub expected_hop1_out: u64,
    pub expected_final_out: u64,
    pub expected_gross_profit: i64,
    pub expected_net_profit: i64,
    pub execution_costs: ExecutionCostBreakdown,
    pub snapshot_slot: u64,
    pub simulation_context_slot: Option<u64>,
    pub tx_size_bytes: usize,
    pub loaded_accounts_data_size: Option<u64>,
    pub compute_units_consumed: u64,
    pub inner_instructions_count: usize,
    pub simulation_success: bool,
    pub error_status: Option<String>,
    pub program_logs: Vec<String>,
    pub fee_payer_lamport_delta: i128,
    pub quote_vs_sim_discrepancy: i64,
    pub profitability_cleared: bool,
    pub rejection_reason: Option<String>,
    /// Pre-simulation token balances reported by the RPC bank.
    #[serde(default)]
    pub pre_token_balances: Vec<SimTokenBalance>,
    /// Post-simulation token balances reported by the RPC bank.
    #[serde(default)]
    pub post_token_balances: Vec<SimTokenBalance>,
    /// Realized net in raw token units of the first watch account, when supplied.
    #[serde(default)]
    pub realized_net_raw: Option<i128>,
    /// Pre-simulation lamport balances keyed by account index.
    #[serde(default)]
    pub pre_balances: Vec<u64>,
    /// Post-simulation lamport balances keyed by account index.
    #[serde(default)]
    pub post_balances: Vec<u64>,
    /// RPC return data, when present.
    #[serde(default)]
    pub return_data: Option<String>,
}

pub struct MeteoraDlmmMainnetSimulationHarness {
    simulator: ForkSimulator,
    profitability_gate: ProfitabilityGate,
}

impl MeteoraDlmmMainnetSimulationHarness {
    pub fn from_env() -> Self {
        Self {
            simulator: ForkSimulator::from_env(),
            profitability_gate: ProfitabilityGate::default(),
        }
    }

    pub fn new(rpc_url: &str) -> Self {
        Self {
            simulator: ForkSimulator::new(ForkState::default(), Some(rpc_url.to_string())),
            profitability_gate: ProfitabilityGate::default(),
        }
    }

    /// Simulate a constructed atomic two-swap Meteora transaction against live Solana Mainnet.
    pub async fn simulate_candidate(
        &self,
        candidate_id: &str,
        tx_wire_bytes: &[u8],
        pool_a: Pubkey,
        pool_b: Pubkey,
        direction: &str,
        input_amount: u64,
        expected_hop1_out: u64,
        expected_final_out: u64,
        expected_gross_profit: i64,
        costs: ExecutionCostBreakdown,
        snapshot_slot: u64,
    ) -> Result<MeteoraSimulationRecord> {
        self.simulate_candidate_with_watch(
            candidate_id,
            tx_wire_bytes,
            pool_a,
            pool_b,
            direction,
            input_amount,
            expected_hop1_out,
            expected_final_out,
            expected_gross_profit,
            costs,
            snapshot_slot,
            &[],
        )
        .await
    }

    /// Simulate with explicit watch accounts so the RPC returns their pre/post
    /// token balances. This is still a non-broadcast `simulateTransaction`.
    #[allow(clippy::too_many_arguments)]
    pub async fn simulate_candidate_with_watch(
        &self,
        candidate_id: &str,
        tx_wire_bytes: &[u8],
        pool_a: Pubkey,
        pool_b: Pubkey,
        direction: &str,
        input_amount: u64,
        expected_hop1_out: u64,
        expected_final_out: u64,
        expected_gross_profit: i64,
        costs: ExecutionCostBreakdown,
        snapshot_slot: u64,
        watch_accounts: &[Pubkey],
    ) -> Result<MeteoraSimulationRecord> {
        let expected_net_profit = expected_gross_profit - (costs.total_cost_lamports() as i64);
        let tx_size_bytes = tx_wire_bytes.len();

        let sim_result = self
            .simulator
            .simulate_route(
                tx_wire_bytes,
                watch_accounts,
                expected_gross_profit.max(0) as u64,
            )
            .await?;

        // Evaluate against profitability gate
        let outcome = self.profitability_gate.evaluate_simulation(
            sim_result.clone(),
            Some(snapshot_slot),
            expected_gross_profit.max(0) as u64,
            costs,
        );

        let quote_vs_sim_discrepancy =
            (sim_result.fee_payer_lamport_delta as i64).saturating_sub(expected_gross_profit);

        let (profitability_cleared, rejection_reason) = match outcome {
            SimulationOutcome::PassedGate { .. } => (true, None),
            SimulationOutcome::Rejected { reason, .. } => (false, Some(format!("{:?}", reason))),
        };

        Ok(MeteoraSimulationRecord {
            candidate_id: candidate_id.to_string(),
            pool_a,
            pool_b,
            direction: direction.to_string(),
            input_amount,
            expected_hop1_out,
            expected_final_out,
            expected_gross_profit,
            expected_net_profit,
            execution_costs: costs,
            snapshot_slot,
            simulation_context_slot: sim_result.context_slot,
            tx_size_bytes,
            loaded_accounts_data_size: sim_result.loaded_accounts_data_size,
            compute_units_consumed: sim_result.units_consumed,
            inner_instructions_count: sim_result.inner_instruction_count,
            simulation_success: sim_result.success,
            error_status: sim_result.error,
            program_logs: sim_result.logs,
            fee_payer_lamport_delta: sim_result.fee_payer_lamport_delta,
            quote_vs_sim_discrepancy,
            profitability_cleared,
            rejection_reason,
            pre_token_balances: sim_result.pre_token_balances,
            post_token_balances: sim_result.post_token_balances,
            realized_net_raw: sim_result.realized_net_lamports.map(i128::from),
            pre_balances: sim_result.pre_balances,
            post_balances: sim_result.post_balances,
            return_data: sim_result.return_data,
        })
    }
}
