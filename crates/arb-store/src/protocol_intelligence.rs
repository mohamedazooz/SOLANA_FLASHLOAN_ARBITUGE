//! Protocol Intelligence Database Models, Schema, and Async Persistence Layer.
//!
//! Provides the source of truth for:
//! 1. Protocol Registry
//! 2. Protocol Mathematical Specifications
//! 3. Pool Registry
//! 4. Pool State Snapshots
//! 5. Fee Registry
//! 6. Quote Records
//! 7. Route Registry
//! 8. Route Simulation Records
//! 9. SDK Reconciliation Records
//!
//! Enforces:
//! - Raw integer representations for amounts, reserves, and fees (never floats).
//! - Non-blocking asynchronous persistence queue outside the critical execution hot path.
//! - In-memory validated state cache for sub-microsecond quote access.

use arb_types::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::warn;

pub const PROTOCOL_INTELLIGENCE_SQLITE_MIGRATION: &str =
    include_str!("../migrations/002_protocol_intelligence_sqlite.sql");
pub const PROTOCOL_INTELLIGENCE_POSTGRES_MIGRATION: &str =
    include_str!("../migrations/002_protocol_intelligence_postgres.sql");

// ============================================================================
// 1. DATA MODELS
// ============================================================================

/// Protocol Registry Record (Section 5.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolRegistryRecord {
    pub protocol_id: String,
    pub protocol_name: String,
    pub protocol_type: String, // "CPMM", "CLMM", "DLMM", "BONDING_CURVE"
    pub program_id: String,
    pub network: String,
    pub protocol_version: String,
    pub supported_features: String,
    pub sdk_name: Option<String>,
    pub sdk_version: Option<String>,
    pub sdk_commit: Option<String>,
    pub source_repository: Option<String>,
    pub documentation_url: Option<String>,
    pub verification_status: String, // "IMPLEMENTED_UNVERIFIED", "SDK_VERIFIED", "EXECUTION_READY"
    pub enabled: bool,
}

/// Protocol Mathematical Specification (Section 5.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtocolMathSpecRecord {
    pub protocol_id: String,
    pub formula_version: String,
    pub input_amount_semantics: String,
    pub output_amount_semantics: String,
    pub price_formula: String,
    pub fee_formula: String,
    pub rounding_mode: String,
    pub integer_precision: String,
    pub fee_components: String,
    pub supported_swap_modes: String,
    pub unsupported_features: Option<String>,
    pub reference_source: String,
    pub verification_test_version: String,
}

/// Pool Registry Record (Section 5.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolRegistryRecord {
    pub pool_address: String,
    pub protocol_id: String,
    pub pool_type: String,
    pub token_mint_a: String,
    pub token_mint_b: String,
    pub token_decimals_a: u8,
    pub token_decimals_b: u8,
    pub token_program_a: String,
    pub token_program_b: String,
    pub vault_a: String,
    pub vault_b: String,
    pub configuration_account: Option<String>,
    pub tick_spacing: Option<i32>,
    pub bin_step: Option<i32>,
    pub active_bin_id: Option<i32>,
    pub current_tick: Option<i32>,
    pub pool_status: String,
    pub discovery_source: String,
}

/// Pool State Snapshot Record (Section 5.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolStateSnapshotRecord {
    pub pool_address: String,
    pub slot: u64,
    pub write_version: u64,
    pub state_hash: String,
    pub reserve_a: String, // Raw integer string
    pub reserve_b: String, // Raw integer string
    pub sqrt_price: Option<String>,
    pub current_tick: Option<i32>,
    pub active_bin: Option<i32>,
    pub liquidity: Option<String>,
    pub protocol_specific_state: Option<String>,
    pub source_provider: String,
    pub state_validity: String,
    pub invalidation_reason: Option<String>,
}

/// Fee Registry Record (Section 5.5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeRegistryRecord {
    pub protocol_id: String,
    pub pool_address: Option<String>,
    pub fee_type: String, // "LP", "PROTOCOL", "FUND", "CREATOR", "DYNAMIC", "TOKEN2022", "FLASH_LOAN"
    pub configuration_source: String,
    pub calculation_version: String,
    pub effective_slot: u64,
    pub fee_numerator: u64,
    pub fee_denominator: u64,
    pub fee_bps: u16,
    pub rounding_rule: String,
    pub verification_status: String,
}

/// Quote Record (Section 5.6).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuoteRecord {
    pub quote_id: String,
    pub protocol_id: String,
    pub pool_address: String,
    pub input_mint: String,
    pub output_mint: String,
    pub amount_in_raw: String,
    pub amount_out_raw: String,
    pub fees_breakdown: String,
    pub price_impact_bps: u16,
    pub unfilled_input_raw: String,
    pub minimum_output_raw: String,
    pub state_slot: u64,
    pub state_hash: String,
    pub quote_engine_version: String,
    pub sdk_reference_version: Option<String>,
    pub quote_latency_us: u64,
    pub quote_validity: String,
}

/// Route Registry Record (Section 5.7).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteRegistryRecord {
    pub route_id: String,
    pub route_type: String, // "SPATIAL_2HOP", "TRIANGULAR_3HOP"
    pub number_of_pools: u8,
    pub ordered_pool_addresses: String, // JSON array
    pub ordered_protocol_ids: String,   // JSON array
    pub input_mint: String,
    pub intermediate_mints: String, // JSON array
    pub output_mint: String,
    pub route_version: String,
    pub enabled: bool,
}

/// Route Simulation Record (Section 5.8).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteSimulationRecord {
    pub simulation_id: String,
    pub route_id: String,
    pub input_amount_raw: String,
    pub output_amount_raw: String,
    pub principal_raw: String,
    pub flash_loan_fee_raw: String,
    pub network_fee_raw: String,
    pub priority_fee_raw: String,
    pub tip_raw: String,
    pub token_transfer_fees_raw: String,
    pub total_costs_raw: String,
    pub net_profit_raw: String,
    pub transaction_size_bytes: u32,
    pub compute_units_consumed: u64,
    pub simulation_slot: u64,
    pub simulation_result: String, // "SUCCESS", "FAILED"
    pub simulation_error: Option<String>,
}

/// SDK Reconciliation Record (Section 5.9).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SdkReconciliationRecord {
    pub protocol_id: String,
    pub pool_address: String,
    pub input_amount_raw: String,
    pub local_quote_raw: String,
    pub sdk_quote_raw: String,
    pub onchain_simulated_quote_raw: Option<String>,
    pub difference_raw: String,
    pub difference_bps: i32,
    pub state_slot: u64,
    pub result: String, // "EXACT_MATCH", "WITHIN_TOLERANCE", "MISMATCH", "FAILED"
    pub failure_reason: Option<String>,
}

// ============================================================================
// 2. PROTOCOL INTELLIGENCE REPOSITORY TRAIT
// ============================================================================

pub trait ProtocolIntelligenceRepository: Send + Sync {
    fn register_protocol(&self, record: &ProtocolRegistryRecord) -> Result<()>;
    fn register_math_spec(&self, record: &ProtocolMathSpecRecord) -> Result<()>;
    fn register_pool(&self, record: &PoolRegistryRecord) -> Result<()>;
    fn record_pool_snapshot(&self, snapshot: &PoolStateSnapshotRecord) -> Result<()>;
    fn register_fee(&self, fee: &FeeRegistryRecord) -> Result<()>;
    fn record_quote(&self, quote: &QuoteRecord) -> Result<()>;
    fn register_route(&self, route: &RouteRegistryRecord) -> Result<()>;
    fn record_simulation(&self, sim: &RouteSimulationRecord) -> Result<()>;
    fn record_sdk_reconciliation(&self, rec: &SdkReconciliationRecord) -> Result<()>;

    fn get_protocol(&self, protocol_id: &str) -> Result<Option<ProtocolRegistryRecord>>;
    fn get_pool(&self, pool_address: &str) -> Result<Option<PoolRegistryRecord>>;
    fn get_latest_snapshot(&self, pool_address: &str) -> Result<Option<PoolStateSnapshotRecord>>;
    fn get_recent_reconciliations(
        &self,
        protocol_id: &str,
        limit: usize,
    ) -> Result<Vec<SdkReconciliationRecord>>;
}

// ============================================================================
// 3. ASYNC PERSISTENCE QUEUE (ZERO HOT-PATH LOCKING)
// ============================================================================

pub enum PersistenceEvent {
    PoolSnapshot(Box<PoolStateSnapshotRecord>),
    Quote(Box<QuoteRecord>),
    Simulation(Box<RouteSimulationRecord>),
    SdkReconciliation(Box<SdkReconciliationRecord>),
}

/// Bounded asynchronous persistence buffer that batches writes to SQLite or Postgres
/// completely decoupled from HFT quote and execution loops.
#[derive(Clone)]
pub struct AsyncIntelligencePersistence {
    sender: mpsc::Sender<PersistenceEvent>,
}

impl AsyncIntelligencePersistence {
    pub const DEFAULT_CHANNEL_CAPACITY: usize = 10_000;

    pub fn new(repo: Arc<dyn ProtocolIntelligenceRepository>) -> Self {
        let (sender, mut receiver) =
            mpsc::channel::<PersistenceEvent>(Self::DEFAULT_CHANNEL_CAPACITY);

        // Background worker draining events asynchronously
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to build background persistence runtime");

            rt.block_on(async move {
                let mut batch = Vec::with_capacity(128);
                let flush_interval = tokio::time::Duration::from_millis(50);
                let mut interval = tokio::time::interval(flush_interval);

                loop {
                    tokio::select! {
                        Some(event) = receiver.recv() => {
                            batch.push(event);
                            if batch.len() >= 128 {
                                Self::flush_batch(&repo, &mut batch);
                            }
                        }
                        _ = interval.tick() => {
                            if !batch.is_empty() {
                                Self::flush_batch(&repo, &mut batch);
                            }
                        }
                        else => break,
                    }
                }
            });
        });

        Self { sender }
    }

    fn flush_batch(
        repo: &Arc<dyn ProtocolIntelligenceRepository>,
        batch: &mut Vec<PersistenceEvent>,
    ) {
        for event in batch.drain(..) {
            let res = match event {
                PersistenceEvent::PoolSnapshot(s) => repo.record_pool_snapshot(&s),
                PersistenceEvent::Quote(q) => repo.record_quote(&q),
                PersistenceEvent::Simulation(sim) => repo.record_simulation(&sim),
                PersistenceEvent::SdkReconciliation(r) => repo.record_sdk_reconciliation(&r),
            };
            if let Err(e) = res {
                warn!("Async persistence error: {e}");
            }
        }
    }

    /// Enqueue a snapshot without blocking the caller. If the channel is full, drop to prevent backpressure.
    pub fn try_record_snapshot(&self, snapshot: PoolStateSnapshotRecord) {
        if let Err(e) = self
            .sender
            .try_send(PersistenceEvent::PoolSnapshot(Box::new(snapshot)))
        {
            warn!("Persistence queue full or closed, dropping snapshot: {e}");
        }
    }

    /// Enqueue a quote without blocking the caller.
    pub fn try_record_quote(&self, quote: QuoteRecord) {
        if let Err(e) = self
            .sender
            .try_send(PersistenceEvent::Quote(Box::new(quote)))
        {
            warn!("Persistence queue full or closed, dropping quote: {e}");
        }
    }

    /// Enqueue a simulation record without blocking the caller.
    pub fn try_record_simulation(&self, sim: RouteSimulationRecord) {
        if let Err(e) = self
            .sender
            .try_send(PersistenceEvent::Simulation(Box::new(sim)))
        {
            warn!("Persistence queue full or closed, dropping simulation record: {e}");
        }
    }

    /// Enqueue an SDK reconciliation record without blocking the caller.
    pub fn try_record_reconciliation(&self, rec: SdkReconciliationRecord) {
        if let Err(e) = self
            .sender
            .try_send(PersistenceEvent::SdkReconciliation(Box::new(rec)))
        {
            warn!("Persistence queue full or closed, dropping reconciliation record: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SqliteStore;

    #[test]
    fn test_protocol_intelligence_sqlite_roundtrip() {
        let store = Arc::new(SqliteStore::open_in_memory().unwrap());

        // 1. Register protocol
        let proto = ProtocolRegistryRecord {
            protocol_id: "METEORA_DLMM".into(),
            protocol_name: "Meteora Dynamic Liquidity Market Maker".into(),
            protocol_type: "DLMM".into(),
            program_id: "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo".into(),
            network: "mainnet-beta".into(),
            protocol_version: "1.0.0".into(),
            supported_features: "BIN_ARRAYS,DYNAMIC_FEES,VOLATILITY_ACCUMULATOR".into(),
            sdk_name: Some("@meteora-ag/dlmm".into()),
            sdk_version: Some("1.2.0".into()),
            sdk_commit: Some("abcd123".into()),
            source_repository: Some("https://github.com/MeteoraAg/dlmm-sdk".into()),
            documentation_url: Some("https://docs.meteora.ag".into()),
            verification_status: "EXECUTION_READY".into(),
            enabled: true,
        };
        store.register_protocol(&proto).unwrap();

        let fetched = store.get_protocol("METEORA_DLMM").unwrap().unwrap();
        assert_eq!(fetched.protocol_id, "METEORA_DLMM");
        assert_eq!(
            fetched.program_id,
            "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo"
        );
        assert!(fetched.enabled);

        // 2. Register pool
        let pool = PoolRegistryRecord {
            pool_address: "AR13_POOL_ADDRESS".into(),
            protocol_id: "METEORA_DLMM".into(),
            pool_type: "DLMM".into(),
            token_mint_a: "So11111111111111111111111111111111111111112".into(),
            token_mint_b: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".into(),
            token_decimals_a: 9,
            token_decimals_b: 6,
            token_program_a: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".into(),
            token_program_b: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".into(),
            vault_a: "VAULT_A".into(),
            vault_b: "VAULT_B".into(),
            configuration_account: None,
            tick_spacing: None,
            bin_step: Some(10),
            active_bin_id: Some(12345),
            current_tick: None,
            pool_status: "ACTIVE".into(),
            discovery_source: "HELIUS_STREAM".into(),
        };
        store.register_pool(&pool).unwrap();

        let fetched_pool = store.get_pool("AR13_POOL_ADDRESS").unwrap().unwrap();
        assert_eq!(fetched_pool.pool_address, "AR13_POOL_ADDRESS");
        assert_eq!(fetched_pool.bin_step, Some(10));
        assert_eq!(fetched_pool.active_bin_id, Some(12345));

        // 3. Test Async Persistence channel
        let async_queue = AsyncIntelligencePersistence::new(store.clone());
        let snapshot = PoolStateSnapshotRecord {
            pool_address: "AR13_POOL_ADDRESS".into(),
            slot: 310_000_000,
            write_version: 1,
            state_hash: "hash_001".into(),
            reserve_a: "100000000000".into(),
            reserve_b: "25000000000".into(),
            sqrt_price: None,
            current_tick: None,
            active_bin: Some(12345),
            liquidity: Some("50000000".into()),
            protocol_specific_state: None,
            source_provider: "RPC_POLL".into(),
            state_validity: "VALID".into(),
            invalidation_reason: None,
        };
        async_queue.try_record_snapshot(snapshot);

        // Sleep briefly to let the background thread drain
        std::thread::sleep(std::time::Duration::from_millis(100));

        let latest_snap = store
            .get_latest_snapshot("AR13_POOL_ADDRESS")
            .unwrap()
            .unwrap();
        assert_eq!(latest_snap.slot, 310_000_000);
        assert_eq!(latest_snap.state_hash, "hash_001");
    }
}
