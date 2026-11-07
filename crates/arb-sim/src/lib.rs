//! Pre-Flight Transaction Simulation Engine.
//!
//! Provides Solana RPC simulateTransaction verification, CU estimation,
//! and slippage validation before private bundle submission.

use arb_types::{ArbError, Lamports, Result};

pub mod litesvm_sim;
pub use litesvm_sim::{LocalSimResult, simulate_locally};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{debug, info};

/// Structured outcome of a transaction simulation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimResult {
    pub success: bool,
    pub units_consumed: u64,
    pub logs: Vec<String>,
    pub error: Option<String>,
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
    fn simulate(&self, tx_bytes: &[u8]) -> Result<SimResult>;
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
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { rpc_url, client }
    }

    /// Asynchronously simulate a transaction via Solana JSON-RPC.
    pub async fn simulate_async(&self, tx_bytes: &[u8]) -> Result<SimResult> {
        let b64 = base64_encode(tx_bytes);
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "simulateTransaction",
            "params": [
                b64,
                {
                    "encoding": "base64",
                    "commitment": "processed",
                    "replaceRecentBlockhash": true
                }
            ]
        });

        let resp = self
            .client
            .post(&self.rpc_url)
            .json(&body)
            .send()
            .await
            .map_err(|e| ArbError::Internal(format!("RPC connection error: {e}")))?;

        let val: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ArbError::Internal(format!("RPC JSON parse error: {e}")))?;

        if let Some(err_val) = val.get("error") {
            return Err(ArbError::SimulationFailed(err_val.to_string()));
        }

        let result_val = val
            .get("result")
            .and_then(|r| r.get("value"))
            .ok_or_else(|| ArbError::SimulationFailed("Missing simulation result value".into()))?;

        let err = result_val.get("err").and_then(|e| {
            if e.is_null() {
                None
            } else {
                Some(e.to_string())
            }
        });

        let units_consumed = result_val
            .get("unitsConsumed")
            .and_then(|u| u.as_u64())
            .unwrap_or(0);

        let logs = result_val
            .get("logs")
            .and_then(|l| l.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let success = err.is_none();
        debug!(
            "Simulation completed: success={} units={} error={:?}",
            success, units_consumed, err
        );

        Ok(SimResult {
            success,
            units_consumed,
            logs,
            error: err,
        })
    }
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
    pub async fn simulate_async(&self, tx_bytes: &[u8]) -> Result<SimResult> {
        match self.rpc_sim {
            Some(ref rpc) => rpc.simulate_async(tx_bytes).await,
            None => Err(ArbError::Internal(
                "simulation requested but no RPC endpoint is configured".into(),
            )),
        }
    }

    /// Synchronous simulation entry point.
    pub fn simulate(&self, tx_bytes: &[u8]) -> Result<SimResult> {
        if let Some(ref rpc) = self.rpc_sim {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                tokio::task::block_in_place(|| handle.block_on(rpc.simulate_async(tx_bytes)))
            } else {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| ArbError::Internal(e.to_string()))?;
                rt.block_on(rpc.simulate_async(tx_bytes))
            }
        } else {
            Err(ArbError::Internal(
                "simulation requested but no RPC endpoint is configured".into(),
            ))
        }
    }

    /// Fast in-memory LiteSVM local simulation (sub-millisecond pre-flight).
    pub fn simulate_local(&self, tx_bytes: &[u8]) -> LocalSimResult {
        simulate_locally(tx_bytes)
    }

    pub fn simulate_bundle(&self, txs: &[Vec<u8>]) -> Result<Vec<SimResult>> {
        txs.iter().map(|tx| self.simulate(tx)).collect()
    }
}

impl SimulatorTrait for Simulator {
    fn simulate(&self, tx_bytes: &[u8]) -> Result<SimResult> {
        self.simulate(tx_bytes)
    }
}

fn base64_encode(input: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(input)
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
    }

    #[test]
    fn shadow_check_test() {
        assert!(shadow_check(Lamports(50_000), Lamports(30_000)));
        assert!(!shadow_check(Lamports(10_000), Lamports(30_000)));
    }
}
