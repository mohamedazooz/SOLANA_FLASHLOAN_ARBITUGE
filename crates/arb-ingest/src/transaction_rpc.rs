//! Isolated Transaction RPC Adapter.
//!
//! Strictly constrained to transaction submission, preflight simulation,
//! recent blockhash queries, and on-chain confirmation verification.
//!
//! Solana Standard JSON-RPC does not offer full transaction submission and
//! simulation over WebSocket PubSub, so this isolated adapter provides the
//! HTTP JSON-RPC bridge exclusively for:
//! 1. getLatestBlockhash
//! 2. simulateTransaction
//! 3. sendTransaction
//! 4. getSignatureStatuses
//! 5. getTransaction
//!
//! Market state updates (pools, vaults, prices) MUST NOT flow through this adapter;
//! market data belongs exclusively to the WebSocket streaming plane.

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::time::Duration;

use crate::rpc_pool::{RpcPool, SignatureStatus};

/// Isolated Transaction RPC Adapter for atomic arbitrage transaction lifecycle.
#[derive(Clone)]
pub struct TransactionRpcAdapter {
    rpc_pool: RpcPool,
}

impl TransactionRpcAdapter {
    /// Create a new adapter wrapping an existing `RpcPool`.
    pub fn new(rpc_pool: RpcPool) -> Self {
        Self { rpc_pool }
    }

    /// Instantiate from environment configuration.
    pub fn from_env() -> Self {
        Self::new(RpcPool::from_env())
    }

    /// Check whether at least one healthy HTTP RPC endpoint is configured.
    pub fn is_configured(&self) -> bool {
        self.rpc_pool.endpoints_count() > 0
    }

    /// Number of configured RPC endpoints.
    pub fn endpoints_count(&self) -> usize {
        self.rpc_pool.endpoints_count()
    }

    /// Underlying RpcPool handle.
    pub fn rpc_pool(&self) -> &RpcPool {
        &self.rpc_pool
    }

    /// 1. getLatestBlockhash: fetches recent blockhash and optional lastValidBlockHeight.
    pub async fn get_latest_blockhash(&self) -> Result<([u8; 32], Option<i64>)> {
        if !self.is_configured() {
            bail!("HTTP Transaction RPC endpoint is not configured; cannot fetch blockhash");
        }
        let req_params = json!([{"commitment": "confirmed"}]);
        let val = self.rpc_pool.call("getLatestBlockhash", req_params).await?;
        let value = val
            .get("value")
            .cloned()
            .or_else(|| val.get("result").and_then(|r| r.get("value")).cloned())
            .context("Missing 'value' in getLatestBlockhash response")?;
        let bh_str = value
            .get("blockhash")
            .and_then(|b| b.as_str())
            .context("Missing blockhash string in response")?;
        let lvbh = value.get("lastValidBlockHeight").and_then(|h| h.as_i64());
        let pk: solana_pubkey::Pubkey =
            std::str::FromStr::from_str(bh_str).context("Failed to decode base58 blockhash")?;
        let arr: [u8; 32] = pk.to_bytes();
        Ok((arr, lvbh))
    }

    /// 2. simulateTransaction: executes preflight simulation with maxSupportedTransactionVersion.
    pub async fn simulate_transaction(
        &self,
        tx_b64: &str,
        max_version: Option<u8>,
    ) -> Result<Value> {
        if !self.is_configured() {
            bail!("HTTP Transaction RPC endpoint is not configured; cannot simulate transaction");
        }
        self.rpc_pool
            .simulate_transaction(tx_b64, max_version)
            .await
    }

    /// 3. sendTransaction: sends raw base64 transaction wire bytes.
    pub async fn send_transaction(&self, tx_b64: &str) -> Result<String> {
        if !self.is_configured() {
            bail!("HTTP Transaction RPC endpoint is not configured; cannot send transaction");
        }
        self.rpc_pool.send_raw_transaction(tx_b64).await
    }

    /// 4. getSignatureStatuses: polls signature confirmation status.
    pub async fn get_signature_statuses(
        &self,
        signature: &str,
        max_attempts: usize,
        delay: Duration,
    ) -> Result<(SignatureStatus, Option<u64>, Option<String>)> {
        if !self.is_configured() {
            bail!("HTTP Transaction RPC endpoint is not configured; cannot check signature status");
        }
        self.rpc_pool
            .poll_signature_with_details(signature, max_attempts, delay)
            .await
    }

    /// 5. getTransaction: retrieves on-chain transaction metadata for balance delta & execution verification.
    pub async fn get_transaction(&self, signature: &str) -> Result<Option<Value>> {
        if !self.is_configured() {
            bail!(
                "HTTP Transaction RPC endpoint is not configured; cannot fetch transaction metadata"
            );
        }
        self.rpc_pool.get_transaction_with_metadata(signature).await
    }

    /// Helper: get current block height for blockhash expiration evidence.
    pub async fn get_block_height(&self) -> Result<u64> {
        if !self.is_configured() {
            bail!("HTTP Transaction RPC endpoint is not configured; cannot query block height");
        }
        self.rpc_pool.get_block_height().await
    }

    /// Helper: get wallet SOL balance in lamports.
    pub async fn get_balance(&self, pubkey: &solana_pubkey::Pubkey) -> Result<u64> {
        if !self.is_configured() {
            bail!("HTTP Transaction RPC endpoint is not configured; cannot check balance");
        }
        self.rpc_pool.get_balance(pubkey).await
    }
}
