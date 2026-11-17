//! Yellowstone gRPC (Dragon's Mouth) client for real-time Solana account streaming.
//!
//! Provides 5-15ms latency account updates via Geyser plugin interface.
//! Supports Triton, Alchemy, Helius LaserStream, QuickNode, Chainstack, Tatum, dRPC providers.
//!
//! Best practices (2026):
//! - Use as primary ingestion with WS/RPC as fallback
//! - Multi-node redundancy for HA
//! - Subscribe to specific program accounts (DEX pools, Kamino reserves)
//! - Handle reconnection with exponential backoff
//! - Track stream generation for stale-data detection

#[cfg(feature = "yellowstone-grpc")]
use {
    crate::IngestClient,
    arb_types::{ArbError, PoolId, PoolState, Result, Slot},
    futures::StreamExt,
    parking_lot::RwLock,
    solana_pubkey::Pubkey,
    std::collections::HashSet,
    std::sync::Arc,
    std::time::Duration,
    thiserror::Error,
    tokio::sync::mpsc,
    tonic::transport::{Channel, ClientTlsConfig, Endpoint},
    yellowstone_grpc_proto::geyser::{
        SubscribeRequest, SubscribeRequestFilterAccounts, SubscribeRequestFilterSlots,
        subscribe_update::UpdateOneof,
    },
};

/// Configuration for Yellowstone gRPC connection
#[cfg(feature = "yellowstone-grpc")]
#[derive(Debug, Clone)]
pub struct YellowstoneConfig {
    /// gRPC endpoint URL (e.g., "https://yellowstone.triton.one:443")
    pub endpoint: String,
    /// Optional API key for authenticated providers (Triton, Alchemy, Helius LaserStream)
    pub api_key: Option<String>,
    /// Programs to subscribe to (DEX programs, Kamino, etc.)
    pub program_ids: Vec<String>,
    /// Account filters (optional, for specific pools/reserves)
    pub account_filters: Vec<String>,
    /// Connection timeout
    pub connect_timeout: Duration,
    /// Request timeout
    pub request_timeout: Duration,
    /// Reconnection policy
    pub max_reconnect_attempts: u32,
    pub reconnect_base_delay: Duration,
    pub reconnect_max_delay: Duration,
    /// Enable slot updates for freshness tracking
    pub subscribe_slots: bool,
    /// Commitment level for account updates
    pub commitment: CommitmentLevel,
}

#[cfg(feature = "yellowstone-grpc")]
impl Default for YellowstoneConfig {
    fn default() -> Self {
        Self {
            endpoint: "https://yellowstone.triton.one:443".to_string(),
            api_key: None,
            program_ids: vec![
                // Meteora DLMM/DAMM/DBC
                "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo".to_string(),
                // Raydium V4/CPMM/CLMM
                "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8".to_string(),
                "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C".to_string(),
                "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK".to_string(),
                // Orca Whirlpools
                "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc".to_string(),
                // Pump.fun / PumpSwap
                "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P".to_string(),
                "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA".to_string(),
                // Kamino (klend)
                "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD".to_string(),
            ],
            account_filters: Vec::new(),
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(30),
            max_reconnect_attempts: 10,
            reconnect_base_delay: Duration::from_secs(1),
            reconnect_max_delay: Duration::from_secs(60),
            subscribe_slots: true,
            commitment: CommitmentLevel::Processed,
        }
    }
}

/// Commitment level for Geyser subscriptions
#[cfg(feature = "yellowstone-grpc")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitmentLevel {
    Processed = 0,
    Confirmed = 1,
    Finalized = 2,
}

impl CommitmentLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            CommitmentLevel::Processed => "processed",
            CommitmentLevel::Confirmed => "confirmed",
            CommitmentLevel::Finalized => "finalized",
        }
    }
}

/// Yellowstone gRPC event types
#[cfg(feature = "yellowstone-grpc")]
#[derive(Debug, Clone)]
pub enum YellowstoneEvent {
    /// Account update from Geyser stream
    AccountUpdate {
        pubkey: Pubkey,
        owner: Pubkey,
        lamports: u64,
        data: Vec<u8>,
        slot: u64,
        write_version: u64,
        txn_signature: Option<String>,
    },
    /// Slot update for freshness tracking
    SlotUpdate {
        slot: u64,
        parent: u64,
        status: String,
    },
    /// Connection status change
    ConnectionStatus { connected: bool, endpoint: String },
    /// Error from stream
    Error(String),
}

/// Yellowstone gRPC client
#[cfg(feature = "yellowstone-grpc")]
pub struct YellowstoneClient {
    config: YellowstoneConfig,
    ingest: Arc<IngestClient>,
    shutdown_tx: RwLock<Option<mpsc::Sender<()>>>,
    reconnect_count: Arc<std::sync::atomic::AtomicU32>,
}

#[cfg(feature = "yellowstone-grpc")]
impl YellowstoneClient {
    /// Create a new Yellowstone gRPC client
    pub fn new(config: YellowstoneConfig, ingest: Arc<IngestClient>) -> Self {
        Self {
            config,
            ingest,
            shutdown_tx: RwLock::new(None),
            reconnect_count: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        }
    }

    /// Start the gRPC stream and process updates
    pub async fn start(&self) -> Result<()> {
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel(1);
        *self.shutdown_tx.write() = Some(shutdown_tx);

        let mut attempt = 0;
        let max_attempts = self.config.max_reconnect_attempts;

        loop {
            if shutdown_rx.try_recv().is_ok() {
                tracing::info!("Yellowstone client shutdown requested");
                break;
            }

            match self.connect_and_stream().await {
                Ok(_) => {
                    tracing::info!("Yellowstone stream ended gracefully");
                    break;
                }
                Err(e) => {
                    attempt += 1;
                    self.reconnect_count
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

                    if attempt >= max_attempts {
                        tracing::error!("Yellowstone max reconnect attempts reached: {}", e);
                        return Err(e);
                    }

                    let delay = self
                        .config
                        .reconnect_base_delay
                        .mul_f32(2.0_f32.powi(attempt as i32 - 1))
                        .min(self.config.reconnect_max_delay);

                    tracing::warn!(
                        "Yellowstone connection failed (attempt {}/{}): {}. Reconnecting in {:?}",
                        attempt,
                        max_attempts,
                        e,
                        delay
                    );

                    tokio::time::sleep(delay).await;
                }
            }
        }

        Ok(())
    }

    /// Shutdown the client
    pub fn shutdown(&self) {
        if let Some(tx) = self.shutdown_tx.read().as_ref() {
            let _ = tx.try_send(());
        }
    }

    /// Get current reconnect count
    pub fn reconnect_count(&self) -> u32 {
        self.reconnect_count
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Connect and stream updates
    async fn connect_and_stream(&self) -> Result<()> {
        let mut endpoint = Endpoint::from_shared(self.config.endpoint.clone())
            .map_err(|e| ArbError::InvalidInput(format!("Invalid endpoint: {}", e)))?;

        // Configure TLS
        let tls_config = ClientTlsConfig::new()
            .with_native_roots()
            .with_enabled_roots();
        endpoint = endpoint
            .tls_config(tls_config)
            .map_err(|e| ArbError::InvalidInput(format!("TLS config error: {}", e)))?;

        // Set timeouts
        endpoint = endpoint
            .connect_timeout(self.config.connect_timeout)
            .timeout(self.config.request_timeout)
            .http2_keep_alive_interval(Duration::from_secs(30))
            .keep_alive_timeout(Duration::from_secs(10));

        // Add auth header if API key provided
        if let Some(api_key) = &self.config.api_key {
            endpoint = endpoint.add_header(
                "x-api-key",
                api_key
                    .parse()
                    .map_err(|e| ArbError::InvalidInput(e.to_string()))?,
            );
        }

        let channel = endpoint
            .connect()
            .await
            .map_err(|e| ArbError::NetworkError(e.to_string()))?;

        let mut client = yellowstone_grpc_proto::geyser::geyser_client::GeyserClient::new(channel);

        // Build subscription request
        let request = self.build_subscribe_request()?;

        let stream = client
            .subscribe(request)
            .await
            .map_err(|e| ArbError::NetworkError(format!("Subscribe failed: {}", e)))?
            .into_inner();

        // Notify connection status
        self.ingest.mark_stream_connected();
        self.ingest
            .stream_generation
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        tracing::info!("Yellowstone gRPC connected to {}", self.config.endpoint);

        // Process stream
        self.process_stream(stream).await
    }

    /// Build Geyser subscribe request
    fn build_subscribe_request(&self) -> Result<SubscribeRequest> {
        let mut accounts = HashSet::new();
        let mut slots = HashSet::new();

        // Subscribe to program accounts
        for program_id in &self.config.program_ids {
            accounts.insert(SubscribeRequestFilterAccounts {
                account: vec![],
                owner: vec![program_id.clone()],
                filters: vec![],
                nonempty_txn_signature: None,
            });
        }

        // Add specific account filters if provided
        for filter in &self.config.account_filters {
            accounts.insert(SubscribeRequestFilterAccounts {
                account: vec![filter.clone()],
                owner: vec![],
                filters: vec![],
                nonempty_txn_signature: None,
            });
        }

        // Subscribe to slots if enabled
        if self.config.subscribe_slots {
            slots.insert(SubscribeRequestFilterSlots {
                filter_by_commitment: Some(true),
                interslot_updates: Some(false),
            });
        }

        Ok(SubscribeRequest {
            accounts: accounts.into_iter().collect(),
            slots: slots.into_iter().collect(),
            transactions: vec![],
            transactions_status: vec![],
            blocks: vec![],
            blocks_meta: vec![],
            entry: vec![],
            commitment: Some(self.config.commitment as i32),
            from_slot: None,
        })
    }

    /// Process incoming stream updates
    async fn process_stream(
        &self,
        mut stream: tonic::Streaming<yellowstone_grpc_proto::geyser::SubscribeUpdate>,
    ) -> Result<()> {
        use yellowstone_grpc_proto::geyser::subscribe_update::UpdateOneof;

        while let Some(update) = stream.next().await {
            let update =
                update.map_err(|e| ArbError::NetworkError(format!("Stream error: {}", e)))?;

            match update.update_oneof {
                Some(UpdateOneof::Account(account)) => {
                    self.handle_account_update(account).await?;
                }
                Some(UpdateOneof::Slot(slot)) => {
                    self.handle_slot_update(slot).await?;
                }
                Some(UpdateOneof::Transaction(_)) => {
                    // Transaction updates not needed for pool state
                }
                Some(UpdateOneof::Block(_)) => {
                    // Block updates not needed
                }
                Some(UpdateOneof::BlockMeta(_)) => {}
                Some(UpdateOneof::Entry(_)) => {}
                Some(UpdateOneof::TransactionStatus(_)) => {}
                None => {}
            }
        }

        Ok(())
    }

    /// Handle account update from Geyser
    async fn handle_account_update(
        &self,
        account: yellowstone_grpc_proto::geyser::SubscribeUpdateAccount,
    ) -> Result<()> {
        let pubkey = Pubkey::try_from(account.pubkey.as_slice())
            .map_err(|e| ArbError::InvalidInput(format!("Invalid pubkey: {}", e)))?;

        let owner = Pubkey::try_from(account.owner.as_slice())
            .map_err(|e| ArbError::InvalidInput(format!("Invalid owner: {}", e)))?;

        // Apply to IngestClient account store
        let snap = crate::registry::AccountSnapshot {
            pubkey,
            owner,
            lamports: account.lamports,
            data: account.data,
            slot: account.slot,
            write_version: account.write_version,
        };

        self.ingest.account_store().apply(snap);

        // If this is a tracked pool, update derived state
        let pool_id = PoolId(pubkey);
        if let Some(mut pool_state) = self.ingest.cache().get_pool(&pool_id) {
            // Update pool state from account data
            pool_state.slot = Slot(account.slot);
            pool_state.write_version = account.write_version;
            // Re-decode pool-specific state
            if let Err(e) =
                self.update_pool_from_account(&pool_id, &pool_state, &account.data, account.slot)
            {
                tracing::debug!("Failed to update pool {}: {}", pool_id, e);
            }
            self.ingest.cache().upsert_pool(pool_state);
        }

        Ok(())
    }

    /// Update derived pool state from raw account data
    fn update_pool_from_account(
        &self,
        pool_id: &PoolId,
        pool_state: &mut PoolState,
        data: &[u8],
        slot: u64,
    ) -> Result<()> {
        use crate::registry::*;

        match pool_state.protocol {
            arb_types::DexProtocol::MeteoraDlmm => {
                decode_dlmm_state(data, pool_id, pool_state, Slot(slot))?;
            }
            arb_types::DexProtocol::RaydiumV4 => {
                decode_raydium_vaults(data, pool_id, pool_state, Slot(slot))?;
            }
            arb_types::DexProtocol::RaydiumCpmm => {
                decode_raydium_cpmm_vaults(data, pool_id, pool_state, Slot(slot))?;
            }
            arb_types::DexProtocol::RaydiumClmm => {
                decode_raydium_clmm_state(data, pool_id, pool_state, Slot(slot))?;
            }
            arb_types::DexProtocol::OrcaWhirlpool => {
                decode_orca_vaults(data, pool_id, pool_state, Slot(slot))?;
            }
            arb_types::DexProtocol::PumpFun => {
                // Pump.fun bonding curve state
                // decode_pump_fun_state(data, pool_id, pool_state, Slot(slot))?;
            }
            arb_types::DexProtocol::PumpSwap => {
                // PumpSwap pool state
                // decode_pump_swap_state(data, pool_id, pool_state, Slot(slot))?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Handle slot update for freshness tracking
    async fn handle_slot_update(
        &self,
        slot: yellowstone_grpc_proto::geyser::SubscribeUpdateSlot,
    ) -> Result<()> {
        self.ingest.record_helius_slot(slot.slot);
        self.ingest
            .latest_slot
            .store(slot.slot, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }
}

#[cfg(feature = "yellowstone-grpc")]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = YellowstoneConfig::default();
        assert!(!config.endpoint.is_empty());
        assert!(!config.program_ids.is_empty());
        assert_eq!(config.max_reconnect_attempts, 10);
    }

    #[test]
    fn test_commitment_levels() {
        assert_eq!(CommitmentLevel::Processed.as_str(), "processed");
        assert_eq!(CommitmentLevel::Confirmed.as_str(), "confirmed");
        assert_eq!(CommitmentLevel::Finalized.as_str(), "finalized");
    }
}
