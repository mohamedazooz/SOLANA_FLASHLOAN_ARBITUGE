//! Local simulation helpers backed by LiteSVM.

#[cfg(feature = "litesvm")]
use litesvm::LiteSVM;
#[cfg(feature = "litesvm")]
use solana_transaction::versioned::VersionedTransaction;

/// Outcome of a local LiteSVM simulation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalSimResult {
    pub success: bool,
    pub compute_units_consumed: u64,
    pub logs: Vec<String>,
    pub error: Option<String>,
}

/// Simulate a serialized transaction locally in memory using LiteSVM.
///
/// Bypasses network/RPC latency (sub-millisecond execution).
/// Returns compute units consumed, execution logs, and any error message.
pub fn simulate_locally(tx_bytes: &[u8]) -> LocalSimResult {
    if tx_bytes.is_empty() {
        return LocalSimResult {
            success: false,
            compute_units_consumed: 0,
            logs: Vec::new(),
            error: Some("empty transaction payload".into()),
        };
    }

    #[cfg(feature = "litesvm")]
    {
        let tx: VersionedTransaction = match bincode::deserialize(tx_bytes) {
            Ok(t) => t,
            Err(e) => {
                return LocalSimResult {
                    success: false,
                    compute_units_consumed: 0,
                    logs: Vec::new(),
                    error: Some(format!("transaction deserialization failed: {e}")),
                };
            }
        };

        let svm = LiteSVM::new();
        match svm.simulate_transaction(tx) {
            Ok(sim_info) => LocalSimResult {
                success: true,
                compute_units_consumed: sim_info.meta.compute_units_consumed,
                logs: sim_info.meta.logs,
                error: None,
            },
            Err(failed) => LocalSimResult {
                success: false,
                compute_units_consumed: failed.meta.compute_units_consumed,
                logs: failed.meta.logs,
                error: Some(format!("{:?}", failed.err)),
            },
        }
    }

    #[cfg(not(feature = "litesvm"))]
    {
        LocalSimResult {
            success: false,
            compute_units_consumed: 0,
            logs: Vec::new(),
            error: Some("litesvm feature disabled: local in-memory simulation unavailable (use RPC simulation)".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simulate_empty_bytes_fails() {
        let res = simulate_locally(&[]);
        assert!(!res.success);
        assert_eq!(res.error, Some("empty transaction payload".into()));
    }

    #[test]
    fn test_simulate_invalid_bytes_fails() {
        let res = simulate_locally(&[1, 2, 3, 4, 5]);
        #[cfg(feature = "litesvm")]
        {
            assert!(!res.success);
            assert!(res.error.unwrap().contains("deserialization failed"));
        }
        #[cfg(not(feature = "litesvm"))]
        {
            assert!(!res.success);
            assert!(res.error.unwrap().contains("litesvm feature disabled"));
        }
    }

    #[cfg(feature = "litesvm")]
    #[test]
    fn test_litesvm_instance() {
        let svm = litesvm::LiteSVM::new();
        assert_eq!(svm.latest_blockhash().to_bytes().len(), 32);
    }
}
