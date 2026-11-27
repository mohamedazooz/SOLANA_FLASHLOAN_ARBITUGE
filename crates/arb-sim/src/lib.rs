//! Pre-Flight Transaction Simulation Engine.
//!
//! Provides Solana RPC simulateTransaction verification, CU estimation,
//! and slippage validation before private bundle submission.

use arb_types::Lamports;
use thiserror::Error;

pub mod forensic;
pub mod fork_sim;
pub mod litesvm_sim;
pub mod meteora_fresh_pipeline;
pub mod meteora_simulation_harness;
pub mod opportunity_recorder;
pub mod paper_trading_report;
pub mod profitability_gate;
pub use forensic::{FORENSIC_CAPTURE_VERSION, ForensicBundle, HopProvenance};
pub use fork_sim::{
    DEFAULT_FORK_SIM_TIMEOUT_MS, ForkSimResult, ForkSimulator, ForkState,
    LandedAssetBalanceChanges, LandedBalanceChanges, SimAccountSnapshot, SimTokenBalance,
    meta_fee_lamports, parse_landed_asset_balance_changes, parse_landed_balance_changes,
};
pub use litesvm_sim::{LocalSimResult, simulate_locally};
pub use meteora_fresh_pipeline::{
    MeteoraDlmmFreshStatePipeline, MeteoraFreshPipelineConfig, MeteoraProfitabilityReport,
};
pub use meteora_simulation_harness::{
    MeteoraDlmmMainnetSimulationHarness, MeteoraSimulationRecord,
};
pub use opportunity_recorder::{CandidateOpportunityRecord, OpportunityHistoryRecorder};
pub use profitability_gate::{
    DEFAULT_MAX_QUOTE_SLOT_LAG, DEFAULT_STRICT_PROFIT_HURDLE_LAMPORTS, ExecutionCostBreakdown,
    ProfitabilityGate, ProfitabilityRejection, STANDARD_SOLANA_BASE_FEE_LAMPORTS,
    SimulationOutcome,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{debug, info};

/// Explicit status for a simulation attempt. Only `Passed` is eligible for dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimulationStatus {
    Passed,
    Failed,
    Unavailable,
    Timeout,
    Stale,
    Unsupported,
}

/// Error returned when the RPC could not produce a valid simulation result.
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize, Deserialize)]
#[error("simulation {status:?}: {message}")]
pub struct SimulationError {
    pub status: SimulationStatus,
    pub message: String,
}

impl SimulationError {
    fn new(status: SimulationStatus, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

pub type SimulationResult<T> = std::result::Result<T, SimulationError>;

/// Structured outcome of a transaction simulation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimResult {
    pub status: SimulationStatus,
    pub units_consumed: u64,
    pub logs: Vec<String>,
    pub error: Option<String>,
    pub context_slot: Option<u64>,
}

impl SimResult {
    /// Mark successful output stale when the RPC bank is outside the quote's freshness window.
    pub fn enforce_quote_freshness(&mut self, quote_slot: u64, max_slot_lag: u64) {
        if self.status != SimulationStatus::Passed {
            return;
        }

        let Some(context_slot) = self.context_slot else {
            self.status = SimulationStatus::Unavailable;
            self.error = Some("simulation response omitted context.slot".into());
            return;
        };

        if context_slot.abs_diff(quote_slot) > max_slot_lag {
            self.status = SimulationStatus::Stale;
            self.error = Some(format!(
                "simulation context slot {context_slot} differs from quote slot {quote_slot} by more than {max_slot_lag}"
            ));
        }
    }
}

/// Offline / shadow profit check.
pub fn shadow_check(expected_net: Lamports, min_profit: Lamports) -> bool {
    let profitable = expected_net.as_u64() >= min_profit.as_u64();
    info!(
        "Shadow check: expected_net={} min_profit={} -> {}",
        expected_net.as_u64(),
        min_profit.as_u64(),
        profitable
    );
    profitable
}

/// Trait for pre-flight simulators.
pub trait SimulatorTrait: Send + Sync {
    fn simulate(&self, tx_bytes: &[u8]) -> SimulationResult<SimResult>;
}

/// Live Solana RPC Simulator using `simulateTransaction`.
#[derive(Debug, Clone)]
pub struct RpcSimulator {
    pub rpc_url: String,
    client: reqwest::Client,
}

/// Default timeout for a single `simulateTransaction` round trip.
pub const DEFAULT_SIM_TIMEOUT_MS: u64 = 1500;

impl RpcSimulator {
    pub fn new(rpc_url: String) -> Self {
        let timeout_ms = std::env::var("ARB_SIM_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(DEFAULT_SIM_TIMEOUT_MS);
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(timeout_ms))
            .tcp_nodelay(true)
            .pool_max_idle_per_host(8)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { rpc_url, client }
    }

    /// Asynchronously simulate a transaction via Solana JSON-RPC.
    pub async fn simulate_async(&self, tx_bytes: &[u8]) -> SimulationResult<SimResult> {
        let b64 = base64_encode(tx_bytes);
        let params = simulate_transaction_params(&b64, tx_bytes);
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "simulateTransaction",
            "params": params,
        });

        let resp = self
            .client
            .post(&self.rpc_url)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                let status = if e.is_timeout() {
                    SimulationStatus::Timeout
                } else {
                    SimulationStatus::Unavailable
                };
                SimulationError::new(status, format!("RPC connection error: {e}"))
            })?;

        if !resp.status().is_success() {
            return Err(SimulationError::new(
                SimulationStatus::Unavailable,
                format!("RPC returned HTTP status {}", resp.status()),
            ));
        }

        let val: serde_json::Value = resp.json().await.map_err(|e| {
            SimulationError::new(
                SimulationStatus::Unavailable,
                format!("RPC JSON parse error: {e}"),
            )
        })?;

        if let Some(err_val) = val.get("error") {
            let error = err_val.to_string();
            let status = if error
                .to_ascii_lowercase()
                .contains("unsupported transaction version")
                || error
                    .to_ascii_lowercase()
                    .contains("maxsupportedtransactionversion")
            {
                SimulationStatus::Unsupported
            } else {
                SimulationStatus::Unavailable
            };
            return Err(SimulationError::new(status, error));
        }

        let result = val.get("result").ok_or_else(|| {
            SimulationError::new(SimulationStatus::Unavailable, "missing simulation result")
        })?;

        parse_simulation_response(result)
    }
}

/// Parse a Solana `simulateTransaction` result object (`result`, including `context` and `value`).
/// Missing execution evidence is classified as unavailable and can never pass the gate.
pub fn parse_simulation_response(value: &serde_json::Value) -> SimulationResult<SimResult> {
    let result_value = value.get("value").ok_or_else(|| {
        SimulationError::new(
            SimulationStatus::Unavailable,
            "missing simulation result value",
        )
    })?;

    let err_value = result_value.get("err").ok_or_else(|| {
        SimulationError::new(
            SimulationStatus::Unavailable,
            "simulation result omitted err field",
        )
    })?;

    let error = (!err_value.is_null()).then(|| err_value.to_string());

    let units_consumed = result_value
        .get("unitsConsumed")
        .and_then(|u| u.as_u64())
        .ok_or_else(|| {
            SimulationError::new(
                SimulationStatus::Unavailable,
                "simulation result omitted unitsConsumed",
            )
        })?;

    let context_slot = value
        .get("context")
        .and_then(|context| context.get("slot"))
        .and_then(serde_json::Value::as_u64);

    let logs = result_value
        .get("logs")
        .and_then(|l| l.as_array())
        .map(|arr| {
            const MAX_SIMULATION_LOG_LINES: usize = 64;
            const MAX_SIMULATION_LOG_BYTES: usize = 512;
            arr.iter()
                .filter_map(|item| item.as_str())
                .take(MAX_SIMULATION_LOG_LINES)
                .map(|line| {
                    if line.len() <= MAX_SIMULATION_LOG_BYTES {
                        line.to_owned()
                    } else {
                        let mut end = MAX_SIMULATION_LOG_BYTES;
                        while !line.is_char_boundary(end) {
                            end -= 1;
                        }
                        format!("{}…[truncated]", &line[..end])
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    if error.is_none() && units_consumed == 0 {
        return Err(SimulationError::new(
            SimulationStatus::Unavailable,
            "simulation reported success without positive unitsConsumed",
        ));
    }

    let status = if error.is_some() {
        SimulationStatus::Failed
    } else if context_slot.is_none() {
        SimulationStatus::Unavailable
    } else {
        SimulationStatus::Passed
    };

    debug!(
        "Simulation completed: status={status:?} slot={context_slot:?} units={units_consumed} error={error:?}"
    );

    Ok(SimResult {
        status,
        units_consumed,
        logs,
        error,
        context_slot,
    })
}

/// Pre-Flight Transaction Simulator with Live RPC support.
#[derive(Debug, Clone, Default)]
pub struct Simulator {
    rpc_sim: Option<RpcSimulator>,
}

impl Simulator {
    pub fn new() -> Self {
        let rpc_url = std::env::var("SOLANA_RPC_URL").ok();
        Self {
            rpc_sim: rpc_url.map(RpcSimulator::new),
        }
    }

    pub fn with_rpc_url(rpc_url: String) -> Self {
        Self {
            rpc_sim: Some(RpcSimulator::new(rpc_url)),
        }
    }

    /// Simulate transaction asynchronously using live Solana JSON-RPC `simulateTransaction`.
    pub async fn simulate_async(&self, tx_bytes: &[u8]) -> SimulationResult<SimResult> {
        match self.rpc_sim {
            Some(ref rpc) => rpc.simulate_async(tx_bytes).await,
            None => Err(SimulationError::new(
                SimulationStatus::Unavailable,
                "no RPC endpoint is configured",
            )),
        }
    }

    /// Synchronous simulation entry point.
    pub fn simulate(&self, tx_bytes: &[u8]) -> SimulationResult<SimResult> {
        if let Some(ref rpc) = self.rpc_sim {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                tokio::task::block_in_place(|| handle.block_on(rpc.simulate_async(tx_bytes)))
            } else {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| {
                        SimulationError::new(SimulationStatus::Unavailable, e.to_string())
                    })?;
                rt.block_on(rpc.simulate_async(tx_bytes))
            }
        } else {
            Err(SimulationError::new(
                SimulationStatus::Unavailable,
                "no RPC endpoint is configured",
            ))
        }
    }

    /// Fast in-memory LiteSVM local simulation (sub-millisecond pre-flight).
    pub fn simulate_local(&self, tx_bytes: &[u8]) -> LocalSimResult {
        simulate_locally(tx_bytes)
    }

    pub fn simulate_bundle(&self, txs: &[Vec<u8>]) -> SimulationResult<Vec<SimResult>> {
        txs.iter().map(|tx| self.simulate(tx)).collect()
    }
}

impl SimulatorTrait for Simulator {
    fn simulate(&self, tx_bytes: &[u8]) -> SimulationResult<SimResult> {
        self.simulate(tx_bytes)
    }
}

fn base64_encode(input: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(input)
}

/// Build the `simulateTransaction` params for the given wire bytes.
///
/// Solana RPC refuses to deserialize versioned transactions above
/// `maxSupportedTransactionVersion` (0 = v0, 1 = v1), so simulation must pass
/// the correct maximum supported version explicitly.
fn simulate_transaction_params(b64_tx: &str, wire_bytes: &[u8]) -> serde_json::Value {
    let mut config = serde_json::Map::new();
    config.insert("encoding".into(), json!("base64"));
    config.insert("commitment".into(), json!("processed"));
    config.insert("replaceRecentBlockhash".into(), json!(true));
    config.insert("sigVerify".into(), json!(false));
    if let Ok(Some(version)) = arb_tx::rpc_max_supported_version_from_wire(wire_bytes) {
        config.insert("maxSupportedTransactionVersion".into(), json!(version));
    }
    json!([b64_tx, serde_json::Value::Object(config)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simulator_without_rpc_returns_error_instead_of_fake_success() {
        // A missing RPC endpoint must surface as a failure, never as `success: true`.
        let res = Simulator::default().simulate(&[1, 2, 3, 4, 5]);
        assert!(
            res.is_err(),
            "missing RPC endpoint was reported as a successful simulation"
        );
        assert_eq!(res.unwrap_err().status, SimulationStatus::Unavailable);
    }

    fn rpc_result(err: serde_json::Value, units: u64, slot: Option<u64>) -> serde_json::Value {
        let mut result = json!({ "value": { "err": err, "unitsConsumed": units, "logs": [] } });
        if let Some(slot) = slot {
            result["context"] = json!({ "slot": slot });
        }
        result
    }

    #[test]
    fn simulation_parser_requires_execution_and_context_evidence() {
        let passed = parse_simulation_response(&rpc_result(serde_json::Value::Null, 10, Some(100)))
            .expect("valid RPC result");
        assert_eq!(passed.status, SimulationStatus::Passed);
        assert_eq!(passed.context_slot, Some(100));

        let failed = parse_simulation_response(&rpc_result(
            json!({"InstructionError": [0, "x"]}),
            10,
            Some(100),
        ))
        .expect("on-chain simulation failure is structured output");
        assert_eq!(failed.status, SimulationStatus::Failed);

        assert_eq!(
            parse_simulation_response(&json!({"value": {"unitsConsumed": 10}}))
                .unwrap_err()
                .status,
            SimulationStatus::Unavailable
        );
        assert_eq!(
            parse_simulation_response(&json!({"value": {"err": null}}))
                .unwrap_err()
                .status,
            SimulationStatus::Unavailable
        );
        assert_eq!(
            parse_simulation_response(&rpc_result(serde_json::Value::Null, 10, None))
                .unwrap()
                .status,
            SimulationStatus::Unavailable
        );
        assert_eq!(
            parse_simulation_response(&rpc_result(serde_json::Value::Null, 0, Some(100)))
                .unwrap_err()
                .status,
            SimulationStatus::Unavailable
        );
    }

    #[test]
    fn simulation_quote_freshness_is_enforced() {
        let mut within_window =
            parse_simulation_response(&rpc_result(serde_json::Value::Null, 10, Some(99))).unwrap();
        within_window.enforce_quote_freshness(100, 2);
        assert_eq!(within_window.status, SimulationStatus::Passed);

        let mut stale =
            parse_simulation_response(&rpc_result(serde_json::Value::Null, 10, Some(103))).unwrap();
        stale.enforce_quote_freshness(100, 2);
        assert_eq!(stale.status, SimulationStatus::Stale);
    }

    #[test]
    fn shadow_check_test() {
        assert!(shadow_check(Lamports(50_000), Lamports(30_000)));
        assert!(!shadow_check(Lamports(10_000), Lamports(30_000)));
    }

    #[test]
    fn simulate_params_carry_version_for_versioned_transactions() {
        use arb_tx::{KeypairSigner, V1ConfigParams};

        // Legacy wire bytes: first byte is compact-u16 sig count (0x01) -> no version param.
        let signer = KeypairSigner::new_random();
        let legacy = arb_tx::TxBuilder::build_signed_transaction(&signer, &[], [1u8; 32])
            .expect("legacy tx");
        let params = simulate_transaction_params("b64", &legacy);
        assert!(params[1].get("maxSupportedTransactionVersion").is_none());

        // V1 wire bytes (0x81): the RPC must be told the version number (1).
        let v1 = arb_tx::AdaptiveTxCompiler::build_signed_v1_transaction(
            &signer,
            &[],
            [1u8; 32],
            V1ConfigParams {
                compute_unit_limit: Some(200_000),
                loaded_accounts_data_size_limit: Some(65_536),
                ..Default::default()
            },
            arb_tx::FormatSelectionReason::ExplicitRequested(arb_tx::TransactionVersion::V1),
        )
        .expect("v1 tx");
        assert_eq!(v1.wire_bytes[0], 0x81);
        let params = simulate_transaction_params("b64", &v1.wire_bytes);
        assert_eq!(
            params[1].get("maxSupportedTransactionVersion").unwrap(),
            &json!(1)
        );
    }
}
