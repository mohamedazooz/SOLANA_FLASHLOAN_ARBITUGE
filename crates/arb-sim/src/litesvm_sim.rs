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

        // Fail-closed pre-check: a fresh LiteSVM instance has NO deployed
        // programs and NO loaded accounts. Simulating a DEX transaction in it
        // proves nothing: undeployed programs fail with a cryptic on-chain
        // error, and system-program-only transactions "succeed" without
        // validating any trading logic. Every invoked program must be
        // explicitly deployed in the VM before the result may count as
        // evidence; otherwise the result is reported as unverified.
        const NATIVE_BUILTIN_BYTES: [[u8; 32]; 2] = [
            solana_pubkey::pubkey!("11111111111111111111111111111111").to_bytes(),
            solana_pubkey::pubkey!("ComputeBudget111111111111111111111111111111").to_bytes(),
        ];
        let mut missing_programs: Vec<String> = Vec::new();
        for ix in tx.message.instructions() {
            let Some(program_id) = tx
                .message
                .static_account_keys()
                .get(ix.program_id_index as usize)
            else {
                continue;
            };
            // LiteSVM (agave) uses `Address`; compare by raw bytes to stay
            // SDK-version agnostic.
            let bytes = program_id.as_ref();
            if NATIVE_BUILTIN_BYTES.iter().any(|b| *b == *bytes) {
                continue;
            }
            let key = program_id.to_string();
            if svm.get_account(program_id).is_none() && !missing_programs.contains(&key) {
                missing_programs.push(key);
            }
        }
        if !missing_programs.is_empty() {
            return LocalSimResult {
                success: false,
                compute_units_consumed: 0,
                logs: Vec::new(),
                error: Some(format!(
                    "unverified: empty LiteSVM VM \u{2014} invoked program(s) not deployed: {:?}. \
                     Load the program binaries and required accounts before local simulation",
                    missing_programs
                )),
            };
        }

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
