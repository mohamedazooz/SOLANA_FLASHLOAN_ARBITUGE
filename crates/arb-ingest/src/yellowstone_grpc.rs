//! Yellowstone gRPC ("Dragon's Mouth") client for real-time Solana account streaming.
//!
//! Provides ~5-15ms latency account updates via the Geyser plugin interface.
//! Intended providers: Triton, Alchemy, Helius LaserStream, QuickNode, Chainstack, Tatum, dRPC.
//!
//! Best practices (2026):
//! - Use as primary ingestion with WS/RPC as fallback
//! - Multi-node redundancy for HA
//! - Subscribe to specific program accounts (DEX pools, Kamino reserves)
//! - Handle reconnection with exponential backoff
//! - Track stream generation for stale-data detection

#[cfg(feature = "yellowstone-grpc")]
mod imp {
    use crate::{IngestClient, ProviderId};
    use arb_types::ArbError;
    use futures::{StreamExt, stream};
    use parking_lot::RwLock;
    use solana_pubkey::Pubkey;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::mpsc;
    use tonic::Request;
    use tonic::transport::{Channel, ClientTlsConfig, Endpoint};
    use yellowstone_grpc_proto::geyser::{
        CommitmentLevel, SubscribeRequest, SubscribeRequestFilterAccounts,
        SubscribeRequestFilterSlots, SubscribeUpdateAccountInfo, geyser_client::GeyserClient,
    };

    /// Configuration for a Yellowstone gRPC connection.
    #[derive(Debug, Clone)]
    pub struct YellowstoneConfig {
        /// gRPC endpoint URL (e.g., "https://yellowstone.triton.one:443")
        pub endpoint: String,
        /// Optional API key for authenticated providers (Triton, Alchemy, Helius LaserStream).
        pub api_key: Option<String>,
        /// Program IDs to subscribe to (DEX pools, Kamino reserves). Keys are filter names.
        pub program_ids: Vec<String>,
        /// Specific account filters (optional, for targeted pools/reserves).
        pub account_filters: Vec<String>,
        /// Connection timeout.
        pub connect_timeout: Duration,
        /// Request timeout.
        pub request_timeout: Duration,
        /// Reconnection policy.
        pub max_reconnect_attempts: u32,
        pub reconnect_base_delay: Duration,
        pub reconnect_max_delay: Duration,
        /// Enable slot updates for freshness tracking.
        pub subscribe_slots: bool,
        /// Commitment level for account updates.
        pub commitment: CommitmentLevel,
    }

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

    /// Yellowstone gRPC event types surfaced to callers.
    #[derive(Debug, Clone)]
    pub enum YellowstoneEvent {
        /// Account update from the Geyser stream.
        AccountUpdate {
            pubkey: Pubkey,
            owner: Pubkey,
            lamports: u64,
            data: Vec<u8>,
            slot: u64,
            write_version: u64,
            bank_id: Option<u64>,
            is_startup: bool,
        },
        /// Slot update for freshness/fork tracking.
        SlotUpdate {
            slot: u64,
            parent: Option<u64>,
            status: i32,
            bank_id: Option<u64>,
        },
        /// Connection status change.
        ConnectionStatus { connected: bool, endpoint: String },
        /// Error from the stream (stream remains fail-closed).
        Error(String),
    }

    /// Yellowstone gRPC client. Optional transport: built only with
    /// `--features yellowstone-grpc`.
    pub struct YellowstoneClient {
        config: YellowstoneConfig,
        ingest: Arc<IngestClient>,
        shutdown_tx: RwLock<Option<mpsc::Sender<()>>>,
        reconnect_count: Arc<std::sync::atomic::AtomicU32>,
    }

    impl YellowstoneClient {
        /// Create a new Yellowstone gRPC client.
        pub fn new(config: YellowstoneConfig, ingest: Arc<IngestClient>) -> Self {
            Self {
                config,
                ingest,
                shutdown_tx: RwLock::new(None),
                reconnect_count: Arc::new(std::sync::atomic::AtomicU32::new(0)),
            }
        }

        /// Start the gRPC stream and process updates.
        pub async fn start(&self) -> Result<(), ArbError> {
            let (shutdown_tx, mut shutdown_rx) = mpsc::channel(1);
            *self.shutdown_tx.write() = Some(shutdown_tx);

            let mut attempt: u32 = 0;
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
                            tracing::error!("Yellowstone max reconnect attempts reached: {e}");
                            return Err(e);
                        }

                        let delay = self
                            .config
                            .reconnect_base_delay
                            .mul_f32(2.0_f32.powi(attempt as i32 - 1))
                            .min(self.config.reconnect_max_delay);

                        tracing::warn!(
                            "Yellowstone connection failed (attempt {attempt}/{max_attempts}): {e}. \
                            Reconnecting in {delay:?}"
                        );

                        tokio::time::sleep(delay).await;
                    }
                }
            }

            Ok(())
        }

        /// Shut the client down.
        pub fn shutdown(&self) {
            if let Some(tx) = self.shutdown_tx.read().as_ref() {
                let _ = tx.try_send(());
            }
        }

        /// Number of reconnect attempts made so far.
        pub fn reconnect_count(&self) -> u32 {
            self.reconnect_count
                .load(std::sync::atomic::Ordering::Relaxed)
        }

        /// Connect to the endpoint and stream updates.
        async fn connect_and_stream(&self) -> Result<(), ArbError> {
            let mut endpoint = Endpoint::from_shared(self.config.endpoint.clone())
                .map_err(|e| ArbError::InvalidInput(format!("Invalid endpoint: {e}")))?;

            let tls_config = ClientTlsConfig::new()
                .with_native_roots()
                .with_enabled_roots();
            endpoint = endpoint
                .tls_config(tls_config)
                .map_err(|e| ArbError::InvalidInput(format!("TLS config error: {e}")))?;

            endpoint = endpoint
                .connect_timeout(self.config.connect_timeout)
                .timeout(self.config.request_timeout)
                .http2_keep_alive_interval(Duration::from_secs(30))
                .keep_alive_timeout(Duration::from_secs(10));

            let channel: Channel = endpoint
                .connect()
                .await
                .map_err(|e| ArbError::Internal(format!("Yellowstone connect: {e}")))?;

            let mut client = GeyserClient::new(channel);
            let request = self.build_subscribe_request()?;

            // Yellowstone SUBSCRIBE is a bidirectional stream: send our single
            // SubscribeRequest, then keep the send side open only as long as we
            // hold the channel. A stream::iter stream ends after the one item,
            // which is exactly what we want for a read subscription.
            let request_stream = stream::iter(vec![request]);
            let request = if let Some(api_key) = &self.config.api_key {
                let mut req = Request::new(request_stream);
                req.metadata_mut().insert(
                    "x-api-key",
                    api_key
                        .parse()
                        .map_err(|e| ArbError::InvalidInput(format!("Invalid API key: {e}")))?,
                );
                req
            } else {
                Request::new(request_stream)
            };

            let mut stream = client
                .subscribe(request)
                .await
                .map_err(|e| ArbError::Internal(format!("Yellowstone subscribe: {e}")))?
                .into_inner();

            self.ingest.mark_stream_connected();
            tracing::info!("Yellowstone gRPC connected to {}", self.config.endpoint);

            while let Some(update) = stream.next().await {
                let update =
                    update.map_err(|e| ArbError::Internal(format!("Yellowstone stream: {e}")))?;

                match update.update_oneof {
                    Some(
                        yellowstone_grpc_proto::geyser::subscribe_update::UpdateOneof::Account(acc),
                    ) => {
                        self.handle_account_update(acc)?;
                    }
                    Some(yellowstone_grpc_proto::geyser::subscribe_update::UpdateOneof::Slot(
                        slot,
                    )) => {
                        self.handle_slot_update(slot);
                    }
                    Some(yellowstone_grpc_proto::geyser::subscribe_update::UpdateOneof::Ping(
                        _,
                    )) => {
                        // Keepalive ping: no state to apply.
                    }
                    Some(yellowstone_grpc_proto::geyser::subscribe_update::UpdateOneof::Pong(
                        pong,
                    )) => {
                        tracing::debug!("Yellowstone pong: id={}", pong.id);
                    }
                    _ => {}
                }
            }

            Ok(())
        }

        /// Build the Geyser subscribe request. Filter keys must be unique;
        /// each filter maps a client-chosen label to an account filter.
        fn build_subscribe_request(&self) -> Result<SubscribeRequest, ArbError> {
            let mut accounts: HashMap<String, SubscribeRequestFilterAccounts> = HashMap::new();
            let mut slots: HashMap<String, SubscribeRequestFilterSlots> = HashMap::new();

            for program_id in &self.config.program_ids {
                accounts.insert(
                    format!("program:{program_id}"),
                    SubscribeRequestFilterAccounts {
                        account: vec![],
                        owner: vec![program_id.clone()],
                        filters: vec![],
                        nonempty_txn_signature: None,
                        cuckoo_accounts_filter: None,
                    },
                );
            }

            for account in &self.config.account_filters {
                accounts.insert(
                    format!("account:{account}"),
                    SubscribeRequestFilterAccounts {
                        account: vec![account.clone()],
                        owner: vec![],
                        filters: vec![],
                        nonempty_txn_signature: None,
                        cuckoo_accounts_filter: None,
                    },
                );
            }

            if self.config.subscribe_slots {
                slots.insert(
                    "slots".to_string(),
                    SubscribeRequestFilterSlots {
                        filter_by_commitment: Some(true),
                        interslot_updates: Some(false),
                    },
                );
            }

            Ok(SubscribeRequest {
                accounts,
                slots,
                transactions: HashMap::new(),
                transactions_status: HashMap::new(),
                blocks: HashMap::new(),
                blocks_meta: HashMap::new(),
                entry: HashMap::new(),
                block_footer: HashMap::new(),
                commitment: Some(self.config.commitment as i32),
                accounts_data_slice: vec![],
                ping: None,
                from_slot: None,
            })
        }

        /// Handle an account update from Geyser.
        fn handle_account_update(
            &self,
            update: yellowstone_grpc_proto::geyser::SubscribeUpdateAccount,
        ) -> Result<(), ArbError> {
            let info: SubscribeUpdateAccountInfo = update.account.ok_or_else(|| {
                ArbError::DeserializationError(
                    "SubscribeUpdateAccount missing account info".to_string(),
                )
            })?;

            let pubkey = Pubkey::try_from(info.pubkey.as_slice())
                .map_err(|e| ArbError::DeserializationError(format!("Invalid pubkey: {e}")))?;
            let owner = Pubkey::try_from(info.owner.as_slice())
                .map_err(|e| ArbError::DeserializationError(format!("Invalid owner: {e}")))?;

            // Apply to the shared account store (provider-id and generation aware).
            let snap = crate::AccountSnapshot {
                pubkey,
                owner,
                slot: update.slot,
                write_version: info.write_version,
                is_startup: update.is_startup,
                provider_id: ProviderId(4), // Yellowstone provider
                bank_id: update.bank_id,
                generation: self.ingest.stream_generation(),
                data: Arc::new(info.data.to_vec()),
            };
            self.ingest.account_store().apply(snap);

            // SCOPE (DATA-08): this stream feeds the shared account store
            // (provider-id and generation aware) and slot freshness only. It
            // does NOT register pools and does NOT update derived pool state —
            // that belongs to the helius_ws path. A client consuming this
            // stream must not treat pool-level state as updated by it.
            Ok(())
        }

        /// Handle a slot update for freshness/fork tracking.
        fn handle_slot_update(&self, slot: yellowstone_grpc_proto::geyser::SubscribeUpdateSlot) {
            let parent = slot.parent;
            let status = slot.status;
            let bank_id = slot.bank_id;
            // Proto slot-status values match crate::slot_integrity's integer codes.
            let _ = self
                .ingest
                .record_slot_update_with_bank(slot.slot, parent, status, bank_id);
        }
    }
}

#[cfg(feature = "yellowstone-grpc")]
pub use imp::{YellowstoneClient, YellowstoneConfig, YellowstoneEvent};
