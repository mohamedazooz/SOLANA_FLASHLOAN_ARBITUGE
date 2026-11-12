//! Bounded, HTTP-only account bootstrap/reconciliation. Never use this in the market-data hot path.
use crate::{AccountSnapshot, AccountSnapshotStore, SnapshotReconciler};
use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use serde::Deserialize;
use solana_pubkey::Pubkey;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

pub const MAX_GET_MULTIPLE_ACCOUNTS: usize = 100;

#[derive(Debug, Clone)]
pub struct RpcSnapshotConfig {
    pub endpoint: String,
    pub commitment: String,
    pub request_timeout: Duration,
}
impl RpcSnapshotConfig {
    pub fn confirmed(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            commitment: "confirmed".into(),
            request_timeout: Duration::from_secs(5),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcSnapshotReport {
    pub context_slot: u64,
    pub account_count: usize,
    pub bytes_received: usize,
}

#[derive(Debug, Deserialize)]
struct RpcEnvelope {
    result: Option<RpcResult>,
    error: Option<RpcError>,
}
#[derive(Debug, Deserialize)]
struct RpcError {
    code: i64,
    message: String,
}
#[derive(Debug, Deserialize)]
struct RpcResult {
    context: RpcContext,
    value: Vec<Option<RpcAccount>>,
}
#[derive(Debug, Deserialize)]
struct RpcContext {
    slot: u64,
}
#[derive(Debug, Deserialize)]
struct RpcAccount {
    owner: String,
    data: (String, String),
}

pub struct RpcSnapshotClient {
    http: reqwest::Client,
    config: RpcSnapshotConfig,
}
impl RpcSnapshotClient {
    pub fn new(config: RpcSnapshotConfig) -> Result<Self> {
        if config.endpoint.trim().is_empty() {
            bail!("RPC snapshot endpoint is empty");
        }
        if !matches!(
            config.commitment.as_str(),
            "processed" | "confirmed" | "finalized"
        ) {
            bail!("unsupported commitment: {}", config.commitment);
        }
        let http = reqwest::Client::builder()
            .timeout(config.request_timeout)
            .pool_max_idle_per_host(2)
            .build()
            .context("build RPC snapshot HTTP client")?;
        Ok(Self { http, config })
    }

    /// Fetch and atomically publish exactly one dependency group (1..=100 accounts).
    /// `expected_owners` must contain the expected program owner for every requested key.
    pub async fn fetch_and_publish(
        &self,
        keys: &[Pubkey],
        expected_owners: &HashMap<Pubkey, Pubkey>,
        min_context_slot: u64,
        generation: u64,
        store: &AccountSnapshotStore,
    ) -> Result<RpcSnapshotReport> {
        if keys.is_empty() || keys.len() > MAX_GET_MULTIPLE_ACCOUNTS {
            bail!(
                "dependency group must contain 1..={MAX_GET_MULTIPLE_ACCOUNTS} accounts; got {}",
                keys.len()
            );
        }
        let mut unique = std::collections::HashSet::with_capacity(keys.len());
        if keys.iter().any(|k| !unique.insert(*k)) {
            bail!("duplicate pubkey in reconciliation request");
        }
        if keys.iter().any(|k| !expected_owners.contains_key(k)) {
            bail!("missing expected owner for one or more dependency accounts");
        }

        let body = serde_json::json!({
            "jsonrpc": "2.0", "id": generation,
            "method": "getMultipleAccounts",
            "params": [keys.iter().map(ToString::to_string).collect::<Vec<_>>(), {
                "encoding": "base64", "commitment": self.config.commitment,
                "minContextSlot": min_context_slot
            }]
        });
        let response = self
            .http
            .post(&self.config.endpoint)
            .json(&body)
            .send()
            .await
            .context("getMultipleAccounts request failed")?;
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .context("read getMultipleAccounts response")?;
        if !status.is_success() {
            bail!(
                "RPC HTTP status {status}: {}",
                String::from_utf8_lossy(&bytes)
            );
        }
        let envelope: RpcEnvelope =
            serde_json::from_slice(&bytes).context("decode getMultipleAccounts JSON")?;
        if let Some(error) = envelope.error {
            bail!("RPC error {}: {}", error.code, error.message);
        }
        let result = envelope
            .result
            .ok_or_else(|| anyhow!("RPC response missing result"))?;
        if result.context.slot < min_context_slot {
            bail!(
                "RPC context slot {} below requested minimum {}",
                result.context.slot,
                min_context_slot
            );
        }
        if result.value.len() != keys.len() {
            bail!(
                "RPC result length mismatch: {} != {}",
                result.value.len(),
                keys.len()
            );
        }

        let mut snapshots = Vec::with_capacity(keys.len());
        let mut reconciler = SnapshotReconciler::default();
        let raw_keys = keys.iter().map(|k| k.to_bytes()).collect::<Vec<_>>();
        reconciler
            .begin(generation, min_context_slot, raw_keys.iter().copied())
            .map_err(|e| anyhow!("snapshot reconciler begin failed: {e:?}"))?;
        let mut total_data_bytes = 0usize;
        for (key, account) in keys.iter().zip(result.value.into_iter()) {
            let account = account.ok_or_else(|| anyhow!("required account {key} is null"))?;
            let owner: Pubkey = account
                .owner
                .parse()
                .with_context(|| format!("invalid owner for {key}"))?;
            if expected_owners.get(key) != Some(&owner) {
                bail!(
                    "owner mismatch for {key}: got {owner}, expected {}",
                    expected_owners[key]
                );
            }
            if account.data.1 != "base64" {
                bail!("unexpected account encoding {} for {key}", account.data.1);
            }
            let data = base64::engine::general_purpose::STANDARD
                .decode(account.data.0)
                .with_context(|| format!("invalid base64 account data for {key}"))?;
            total_data_bytes = total_data_bytes.saturating_add(data.len());
            snapshots.push(AccountSnapshot {
                pubkey: *key,
                owner,
                slot: result.context.slot,
                write_version: 0,
                // RPC bootstrap is not a live Geyser write. Keep it explicitly non-tradeable
                // until a live stream update replaces it.
                is_startup: true,
                provider_id: crate::ProviderId(0),
                bank_id: None,
                generation: 0,
                data: Arc::new(data),
            });
        }
        if !reconciler
            .record_batch(generation, result.context.slot, raw_keys)
            .map_err(|e| anyhow!("snapshot reconciler batch failed: {e:?}"))?
            || !reconciler.ready()
        {
            bail!("snapshot reconciliation incomplete");
        }
        if !store.apply_batch(snapshots) {
            bail!("atomic snapshot publication rejected (stale/conflicting existing state)");
        }
        Ok(RpcSnapshotReport {
            context_slot: result.context.slot,
            account_count: keys.len(),
            bytes_received: total_data_bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn config_rejects_invalid_commitment_and_empty_endpoint() {
        assert!(RpcSnapshotClient::new(RpcSnapshotConfig::confirmed(" ")).is_err());
        let mut cfg = RpcSnapshotConfig::confirmed("http://localhost");
        cfg.commitment = "latest".into();
        assert!(RpcSnapshotClient::new(cfg).is_err());
    }
    #[test]
    fn response_parser_preserves_null_slots_and_base64_shape() {
        let raw = br#"{"result":{"context":{"slot":42},"value":[{"owner":"11111111111111111111111111111111","data":["AQID","base64"]},null]}}"#;
        let env: RpcEnvelope = serde_json::from_slice(raw).unwrap();
        let result = env.result.unwrap();
        assert_eq!(result.context.slot, 42);
        assert_eq!(result.value.len(), 2);
        assert!(result.value[1].is_none());
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(&result.value[0].as_ref().unwrap().data.0)
                .unwrap(),
            vec![1, 2, 3]
        );
    }
}
