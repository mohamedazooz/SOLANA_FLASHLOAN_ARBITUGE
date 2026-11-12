//! Market-state ingestion (WebSocket + RPC only). gRPC transport removed.
//! Per-account snapshots and update-only market state.

pub mod gmgn;
pub use gmgn::{GmgnClient, TrendingToken};
pub mod dependency_manifest;
pub mod dexscreener;
pub mod provider_supervisor;
pub mod quote_readiness;
pub mod rpc_reconcile;
pub mod slot_integrity;
pub use dependency_manifest::{
    AccountDependency, AccountRole, DependencyManifestError, DependencyManifestRegistry,
    PoolDependencyManifest, ProtocolDependencyError, ProtocolDependencyInput,
    resolve_protocol_dependencies,
};
pub use dexscreener::{DexPair, DexScreenerClient, DiscoveredPool, MultiVenueToken};
pub use provider_supervisor::{
    CoherenceError, DependencyStamp, ProviderHealth, ProviderId, ProviderStatus,
    ProviderSupervisor, SupervisorPolicy, validate_dependency_group,
};
pub use quote_readiness::{
    ExecutionReadinessError, QuoteReadinessContext, QuoteReadinessError,
    build_quote_dependency_set, validate_execution_readiness,
};
pub use rpc_reconcile::{RpcSnapshotClient, RpcSnapshotConfig, RpcSnapshotReport};
pub use slot_integrity::{ReconcileError, SlotIntegrity, SlotUpdateOutcome, SnapshotReconciler};
pub mod rpc_pool;
pub use rpc_pool::{ProviderHealthReport, RpcPool, RpcPoolHealth, SignatureStatus};
pub mod registry;
pub use registry::{
    DecodedPoolInfo, MultiDexRegistry, RegisteredPool, clmm_tick_array_covers_tick,
    compute_clmm_virtual_reserves, compute_dlmm_reserves, decode_account_data, decode_clmm_state,
    decode_dlmm_fee_bps, decode_dlmm_state, decode_orca_fee_bps, decode_orca_vaults,
    decode_pool_info, decode_raydium_clmm_state, decode_raydium_clmm_vaults,
    decode_raydium_cpmm_fee_bps, decode_raydium_cpmm_vaults, decode_raydium_fee_bps,
    decode_raydium_vaults, decode_spl_token_amount_bytes, fetch_accounts_chunked,
    next_pool_write_version, raydium_clmm_tick_array_pdas, resolve_mints_decimals,
};
pub mod dual_ingest;
pub use dual_ingest::{DualIngestShadow, PoolConsistencyCheck, StreamingPoolCache};
pub mod discovery;
pub use discovery::{
    DiscoveredPoolState, DiscoveryConfig, PoolDiscovery, register_discovered,
    select_dual_30_watch_set, select_top_33_per_venue, select_watch_set,
    select_watch_set_per_venue,
};
pub mod pump_state;
pub use pump_state::{
    PumpFeeComponentsBps, PumpFeeTier, PumpFeesFeeConfig, PumpSwapGlobalConfigState,
    PumpSwapJoinedState, PumpSwapRefreshReport, decode_pump_swap_disable_flags,
    decode_pump_swap_global_config, join_pump_swap_state,
};
pub mod pumpfun_discovery;
pub use pumpfun_discovery::{PumpFunCoin, PumpFunDiscoveryClient};
pub mod pumpdev_stream;
pub use pumpdev_stream::{PumpDevEvent, PumpDevWebSocketClient};
pub mod solami_stream;
pub use solami_stream::{
    SolamiFilterMessage, SolamiStreamClient, SolamiStreamConfig, SolamiStreamEvent,
};
pub mod bitquery;
pub use bitquery::{
    BitqueryClient, BitqueryConfig, BitqueryTradeEntry, BitqueryWsClient, BitqueryWsMessage,
};
pub mod helius_ws;
pub use helius_ws::HeliusWsClient;

use arb_dex::traits::AmmPool;
use arb_types::{AccountUpdate, ArbError, PoolId, PoolState, Result, Slot};
use parking_lot::RwLock;
use solana_pubkey::Pubkey;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;
use tokio::time::sleep;
use tracing::debug;

pub const DEDUPE_WINDOW_SLOTS: u64 = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DedupeKey {
    pub pool_id: PoolId,
    pub slot: Slot,
    pub write_version: u64,
}

/// Shared market state with per-pool mutation; strategy snapshots are copied once per cycle.
#[derive(Debug, Default)]
pub struct MarketStateCache {
    // Per-pool mutation: updates never clone the full market map.
    pools: RwLock<HashMap<PoolId, PoolState>>,
    version: std::sync::atomic::AtomicU64,
    latest_slot: std::sync::atomic::AtomicU64,
}

#[derive(Debug, Clone)]
pub struct AccountSnapshot {
    pub pubkey: Pubkey,
    pub owner: Pubkey,
    pub slot: u64,
    pub write_version: u64,
    pub is_startup: bool,
    /// Yellowstone source identity; RPC bootstrap uses ProviderId(0).
    pub provider_id: ProviderId,
    /// Bank identity supplied by Yellowstone; unavailable for HTTP RPC snapshots.
    pub bank_id: Option<u64>,
    /// Stream generation prevents snapshots surviving a reconnect boundary.
    pub generation: u64,
    pub data: Arc<Vec<u8>>,
}

#[derive(Debug, Default)]
pub struct AccountSnapshotStore {
    accounts: RwLock<HashMap<Pubkey, AccountSnapshot>>,
    invalidated_slots: RwLock<std::collections::HashSet<u64>>,
    invalidated_banks: RwLock<std::collections::HashSet<(u64, u64)>>,
}

impl AccountSnapshotStore {
    /// Apply a single account update in O(1) average time; reject older slot/write-version data.
    pub fn apply(&self, update: AccountSnapshot) -> bool {
        // Keep tombstone read locks until publication completes. An invalidation either
        // happens before this write (and rejects it) or after it (and removes it).
        let dead_slots = self.invalidated_slots.read();
        let dead_banks = self.invalidated_banks.read();
        if dead_slots.contains(&update.slot)
            || update
                .bank_id
                .is_some_and(|id| dead_banks.contains(&(update.slot, id)))
        {
            return false;
        }
        let mut accounts = self.accounts.write();
        if let Some(current) = accounts.get(&update.pubkey) {
            // A live stream write always supersedes bootstrap, but bootstrap must never
            // overwrite live state, even when its context slot is numerically higher.
            if !current.is_startup && update.is_startup {
                return false;
            }
            if !current.is_startup
                && !update.is_startup
                && current.provider_id != update.provider_id
            {
                // Provider-local slot/write-version values are not globally comparable.
                // A provider switch requires an explicit source-generation reset.
                return false;
            }
            if !current.is_startup
                && !update.is_startup
                && current.slot == update.slot
                && current.bank_id != update.bank_id
            {
                // Never choose between competing banks by write_version. Wait for
                // SLOT_DEAD invalidation or a coherent fresh dependency set.
                return false;
            }
            if (update.slot, update.write_version) <= (current.slot, current.write_version) {
                return false;
            }
        }
        accounts.insert(update.pubkey, update);
        true
    }

    /// Atomically publish one coherent RPC snapshot batch. Every record must share the
    /// same context slot and the entire batch must be newer than existing records.
    /// This prevents partial dependency-group visibility to quote readers.
    pub fn apply_batch(&self, updates: Vec<AccountSnapshot>) -> bool {
        if updates.is_empty() {
            return false;
        }
        let dead_slots = self.invalidated_slots.read();
        let dead_banks = self.invalidated_banks.read();
        if updates.iter().any(|u| {
            dead_slots.contains(&u.slot)
                || u.bank_id
                    .is_some_and(|id| dead_banks.contains(&(u.slot, id)))
        }) {
            return false;
        }
        let first = &updates[0];
        let slot = first.slot;
        let provider_id = first.provider_id;
        let bank_id = first.bank_id;
        let generation = first.generation;
        let is_startup = first.is_startup;
        let mut seen = std::collections::HashSet::with_capacity(updates.len());
        if updates.iter().any(|u| {
            u.slot != slot
                || u.provider_id != provider_id
                || u.bank_id != bank_id
                || u.generation != generation
                || u.is_startup != is_startup
                || !seen.insert(u.pubkey)
        }) {
            return false;
        }
        let mut accounts = self.accounts.write();
        if updates.iter().any(|u| {
            accounts.get(&u.pubkey).is_some_and(|old| {
                (!old.is_startup && u.is_startup)
                    || (!old.is_startup && !u.is_startup && old.provider_id != u.provider_id)
                    || (!old.is_startup
                        && !u.is_startup
                        && old.slot == u.slot
                        && old.bank_id != u.bank_id)
                    || (u.slot, u.write_version) <= (old.slot, old.write_version)
            })
        }) {
            return false;
        }
        for update in updates {
            accounts.insert(update.pubkey, update);
        }
        true
    }

    pub fn get(&self, pubkey: &Pubkey) -> Option<AccountSnapshot> {
        self.accounts.read().get(pubkey).cloned()
    }

    /// Read a dependency set under one read lock so a concurrent writer cannot
    /// interleave snapshots while a quote context is being assembled.
    pub fn get_many(&self, pubkeys: &[Pubkey]) -> Vec<Option<AccountSnapshot>> {
        let accounts = self.accounts.read();
        pubkeys
            .iter()
            .map(|key| accounts.get(key).cloned())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.accounts.read().len()
    }

    /// Remove snapshots written in invalidated slots. This is source-local: callers must
    /// rebuild these accounts before marking a replacement snapshot quote-ready.
    pub fn invalidate_bank(&self, slot: u64, bank_id: u64) -> usize {
        {
            let mut tombstones = self.invalidated_banks.write();
            tombstones.insert((slot, bank_id));
            let floor = slot.saturating_sub(8_192);
            tombstones.retain(|(seen_slot, _)| *seen_slot >= floor);
        }
        let mut accounts = self.accounts.write();
        let before = accounts.len();
        accounts
            .retain(|_, snapshot| !(snapshot.slot == slot && snapshot.bank_id == Some(bank_id)));
        before - accounts.len()
    }

    /// A SLOT_CREATED_BANK after SLOT_DEAD reopens the slot for the replacement bank.
    /// Old candidate banks remain tombstoned independently by (slot, bank_id).
    pub fn reopen_slot_for_new_bank(&self, slot: u64) {
        self.invalidated_slots.write().remove(&slot);
    }

    pub fn invalidate_slots(&self, slots: &std::collections::HashSet<u64>) -> usize {
        {
            let mut tombstones = self.invalidated_slots.write();
            tombstones.extend(slots.iter().copied());
            if let Some(max_slot) = tombstones.iter().copied().max() {
                let floor = max_slot.saturating_sub(8_192);
                tombstones.retain(|slot| *slot >= floor);
            }
        }
        let mut accounts = self.accounts.write();
        let before = accounts.len();
        accounts.retain(|_, snapshot| !slots.contains(&snapshot.slot));
        before - accounts.len()
    }

    /// Return a copy of snapshots owned by one on-chain program.
    /// The caller should filter by owner before protocol-specific decoding.
    pub fn snapshots_by_owner(&self, owner: &Pubkey) -> Vec<AccountSnapshot> {
        self.accounts
            .read()
            .values()
            .filter(|snapshot| snapshot.owner == *owner)
            .cloned()
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.accounts.read().is_empty()
    }

    /// Clear all snapshots at a stream-generation boundary.
    pub fn clear(&self) {
        self.invalidated_slots.write().clear();
        self.invalidated_banks.write().clear();
        self.accounts.write().clear();
    }
}

impl MarketStateCache {
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    pub fn latest_slot(&self) -> u64 {
        self.latest_slot.load(Ordering::Acquire)
    }

    /// Copy a read snapshot once per strategy cycle, never once per account update.
    pub fn snapshot_pools(&self) -> Vec<PoolState> {
        self.pools.read().values().cloned().collect()
    }

    pub fn get_pool(&self, pool_id: &PoolId) -> Option<PoolState> {
        self.pools.read().get(pool_id).cloned()
    }

    pub fn pool_count(&self) -> usize {
        self.pools.read().len()
    }

    /// Insert a pool state directly into the cache. Used by benchmarks and
    /// tests to pre-populate the HTTP-polling cache for read-latency comparison.
    pub fn set_pool_for_testing(&self, state: PoolState) {
        self.pools.write().insert(state.pool_id, state);
        self.version.fetch_add(1, Ordering::AcqRel);
    }
}

/// Slot-bounded ring buffer deduplicator to prevent unbounded memory growth.
#[derive(Debug, Default)]
pub struct BoundedDedupeBuffer {
    items: HashMap<DedupeKey, u64>,
}

impl BoundedDedupeBuffer {
    pub fn new() -> Self {
        Self {
            items: HashMap::new(),
        }
    }

    /// Insert a key. Returns true if key was NOT present and is accepted.
    pub fn check_and_insert(&mut self, key: DedupeKey, current_slot: u64) -> bool {
        // Evict items older than current_slot - DEDUPE_WINDOW_SLOTS
        let min_slot = current_slot.saturating_sub(DEDUPE_WINDOW_SLOTS);
        self.items.retain(|_, slot| *slot >= min_slot);

        match self.items.entry(key) {
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(current_slot);
                true
            }
            std::collections::hash_map::Entry::Occupied(_) => false,
        }
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }
}

/// Ingest client: central market-state container used by WebSocket + RPC paths.
/// gRPC transport has been removed; this holds `MarketStateCache`,
/// `AccountSnapshotStore`, slot watermarks and stream-generation tracking.
pub struct IngestClient {
    cache: Arc<MarketStateCache>,
    latest_slot: Arc<AtomicU64>,
    dedupe: parking_lot::Mutex<BoundedDedupeBuffer>,
    reconnect_attempts: Arc<AtomicU32>,
    account_store: Arc<AccountSnapshotStore>,
    tracked_accounts_tx: tokio::sync::watch::Sender<Vec<String>>,
    tracked_accounts_cache: parking_lot::RwLock<Vec<String>>,
    slot_integrity: parking_lot::Mutex<SlotIntegrity>,
    stream_generation: AtomicU64,
    stream_connected: std::sync::atomic::AtomicBool,
    helius_connected: std::sync::atomic::AtomicBool,
    helius_slot: Arc<AtomicU64>,
    provider_id: ProviderId,
    /// Optional streaming-only pool cache for dual-ingestion shadow mode.
    /// When set, account updates are decoded into PoolState and written here
    /// instead of (or in addition to) the shared MarketStateCache, enabling
    /// comparison against the HTTP-polling path.
    streaming_cache: parking_lot::Mutex<Option<Arc<StreamingPoolCache>>>,
}

impl Default for IngestClient {
    fn default() -> Self {
        Self::new()
    }
}

impl IngestClient {
    pub fn new() -> Self {
        let (tracked_accounts_tx, _) = tokio::sync::watch::channel(Vec::new());
        Self {
            cache: Arc::new(MarketStateCache::default()),
            latest_slot: Arc::new(AtomicU64::new(0)),
            dedupe: parking_lot::Mutex::new(BoundedDedupeBuffer::new()),
            reconnect_attempts: Arc::new(AtomicU32::new(0)),
            account_store: Arc::new(AccountSnapshotStore::default()),
            tracked_accounts_tx,
            tracked_accounts_cache: parking_lot::RwLock::new(Vec::new()),
            slot_integrity: parking_lot::Mutex::new(SlotIntegrity::default()),
            stream_generation: AtomicU64::new(0),
            stream_connected: std::sync::atomic::AtomicBool::new(false),
            helius_connected: std::sync::atomic::AtomicBool::new(false),
            helius_slot: Arc::new(AtomicU64::new(0)),
            provider_id: ProviderId(1),
            streaming_cache: parking_lot::Mutex::new(None),
        }
    }

    pub fn account_store(&self) -> Arc<AccountSnapshotStore> {
        Arc::clone(&self.account_store)
    }

    pub fn provider_id(&self) -> ProviderId {
        self.provider_id
    }
    pub fn stream_generation(&self) -> u64 {
        self.stream_generation.load(Ordering::Acquire)
    }
    pub fn stream_connected(&self) -> bool {
        self.stream_connected.load(Ordering::Acquire)
            || self.helius_connected.load(Ordering::Acquire)
    }

    pub fn helius_connected(&self) -> bool {
        self.helius_connected.load(Ordering::Acquire)
    }

    pub fn mark_helius_connected(&self) {
        self.helius_connected.store(true, Ordering::Release);
        if self.stream_generation.load(Ordering::Acquire) == 0 {
            self.stream_generation.store(1, Ordering::Release);
        }
    }

    pub fn mark_helius_disconnected(&self) {
        self.helius_connected.store(false, Ordering::Release);
    }

    pub fn helius_slot(&self) -> u64 {
        self.helius_slot.load(Ordering::Relaxed)
    }

    pub fn record_helius_slot(&self, slot: u64) {
        self.helius_slot.fetch_max(slot, Ordering::Relaxed);
        self.latest_slot.fetch_max(slot, Ordering::Relaxed);
        self.cache.latest_slot.fetch_max(slot, Ordering::Relaxed);
    }

    pub fn subscribe_tracked_accounts(&self) -> tokio::sync::watch::Receiver<Vec<String>> {
        self.tracked_accounts_tx.subscribe()
    }

    pub fn tracked_accounts(&self) -> Vec<String> {
        self.tracked_accounts_cache.read().clone()
    }

    /// Begin a fresh source epoch. Old account/pool state is discarded before accepting data.
    pub fn begin_stream_generation(&self) -> u64 {
        self.stream_connected.store(false, Ordering::Release);
        if !self.helius_connected.load(Ordering::Acquire) {
            self.account_store.clear();
            self.cache.pools.write().clear();
            self.cache.latest_slot.store(0, Ordering::Release);
            self.latest_slot.store(0, Ordering::Release);
            self.cache.version.fetch_add(1, Ordering::AcqRel);
            self.dedupe.lock().clear();
        }
        self.stream_generation
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1)
    }

    pub fn mark_stream_connected(&self) {
        self.stream_connected.store(true, Ordering::Release);
    }

    /// Fail closed on disconnect: no stale quote state remains eligible.
    pub fn mark_stream_disconnected(&self) {
        self.stream_connected.store(false, Ordering::Release);
        if !self.helius_connected.load(Ordering::Acquire) {
            self.account_store.clear();
            self.cache.pools.write().clear();
            self.cache.latest_slot.store(0, Ordering::Release);
            self.latest_slot.store(0, Ordering::Release);
            self.cache.version.fetch_add(1, Ordering::AcqRel);
            self.dedupe.lock().clear();
        }
    }

    /// Replace the explicitly tracked account set. Yellowstone requests are full-state
    /// replacements, so the gRPC worker always resends owner + address + slot filters together.
    /// Synchronize Yellowstone's explicit address filter from the dependency manifest.
    /// This does not subscribe to every account owned by a DEX program.
    pub fn sync_manifest_subscriptions(
        &self,
        manifests: &DependencyManifestRegistry,
    ) -> Result<()> {
        self.set_tracked_accounts(&manifests.tracked_accounts())
    }

    pub fn set_tracked_accounts(&self, accounts: &[Pubkey]) -> Result<()> {
        const MAX_TRACKED_ACCOUNTS: usize = 4_096;
        let mut values = accounts.iter().map(ToString::to_string).collect::<Vec<_>>();
        values.sort_unstable();
        values.dedup();
        if values.len() > MAX_TRACKED_ACCOUNTS {
            return Err(ArbError::InvalidInput(format!(
                "tracked-account set exceeds safety limit: {} > {MAX_TRACKED_ACCOUNTS}",
                values.len()
            )));
        }
        let mut current = self.tracked_accounts_cache.write();
        if *current != values {
            self.tracked_accounts_tx.send_replace(values.clone());
            *current = values;
        }
        Ok(())
    }

    /// Shared market-state handle. Per-pool reads use a short read lock.
    pub fn cache(&self) -> Arc<MarketStateCache> {
        Arc::clone(&self.cache)
    }

    /// Shared market-state handle. Call `snapshot_pools` once per strategy cycle.
    pub fn snapshot(&self) -> Arc<MarketStateCache> {
        Arc::clone(&self.cache)
    }

    /// Calculate exponential backoff delay with jitter.
    pub fn backoff_delay(attempt: u32) -> Duration {
        let base_ms: u64 = 100;
        let exp = attempt.saturating_add(1).min(6);
        let mult = 1u64 << exp;
        let jitter_ms = (attempt as u64 * 37) % 50;
        Duration::from_millis(base_ms.saturating_mul(mult).saturating_add(jitter_ms))
    }

    /// Apply an account update atomically to the cache.
    pub fn apply_pool_update(&self, pool_state: PoolState) -> Result<bool> {
        let slot_val = pool_state.slot.0;
        if self.slot_is_dead(slot_val) {
            return Ok(false);
        }
        let key = DedupeKey {
            pool_id: pool_state.pool_id,
            slot: pool_state.slot,
            write_version: pool_state.write_version,
        };

        {
            let mut dedupe = self.dedupe.lock();
            if !dedupe.check_and_insert(key, slot_val) {
                return Ok(false);
            }
        }

        // Update exactly one pool. Reject an older state even when its dedupe key differs.
        {
            let mut pools = self.cache.pools.write();
            if let Some(current) = pools.get(&pool_state.pool_id) {
                if (pool_state.slot.0, pool_state.write_version)
                    <= (current.slot.0, current.write_version)
                {
                    return Ok(false);
                }
            }
            pools.insert(pool_state.pool_id, pool_state);
        }
        self.cache.version.fetch_add(1, Ordering::AcqRel);
        self.cache.latest_slot.fetch_max(slot_val, Ordering::AcqRel);
        self.latest_slot.fetch_max(slot_val, Ordering::AcqRel);
        Ok(true)
    }

    /// Process Yellowstone slot status and invalidate every known descendant of a dead fork.
    pub fn record_slot_update(
        &self,
        slot: u64,
        parent: Option<u64>,
        status: i32,
    ) -> SlotUpdateOutcome {
        self.record_slot_update_with_bank(slot, parent, status, None)
    }

    pub fn record_slot_update_with_bank(
        &self,
        slot: u64,
        parent: Option<u64>,
        status: i32,
        bank_id: Option<u64>,
    ) -> SlotUpdateOutcome {
        let outcome = self
            .slot_integrity
            .lock()
            .observe_with_bank(slot, parent, status, bank_id);
        if let SlotUpdateOutcome::Dead { invalidated_slots } = &outcome {
            let invalidated: std::collections::HashSet<u64> =
                invalidated_slots.iter().copied().collect();
            let bank_keys = self
                .slot_integrity
                .lock()
                .bank_keys_for_slots(invalidated_slots);
            self.account_store.invalidate_slots(&invalidated);
            for (dead_slot, dead_bank) in bank_keys {
                self.account_store.invalidate_bank(dead_slot, dead_bank);
            }
            {
                let mut pools = self.cache.pools.write();
                pools.retain(|_, pool| !invalidated.contains(&pool.slot.0));
            }
            self.dedupe
                .lock()
                .items
                .retain(|key, _| !invalidated.contains(&key.slot.0));
            self.cache.version.fetch_add(1, Ordering::AcqRel);
        }
        if status == slot_integrity::SLOT_CREATED_BANK
            && bank_id.is_some()
            && !self.slot_is_dead(slot)
        {
            self.account_store.reopen_slot_for_new_bank(slot);
        }
        self.record_slot(slot);
        outcome
    }

    pub fn slot_is_dead(&self, slot: u64) -> bool {
        self.slot_integrity.lock().is_dead(slot)
    }

    pub fn record_account_bank_id(&self, slot: u64, bank_id: Option<u64>) {
        if let Some(bank_id) = bank_id {
            self.slot_integrity.lock().record_bank_id(slot, bank_id);
        }
    }

    pub fn record_entry_update_parent(&self, slot: u64, cleared_bank_id: u64) -> usize {
        self.slot_integrity
            .lock()
            .invalidate_bank(slot, cleared_bank_id);
        let removed = self.account_store.invalidate_bank(slot, cleared_bank_id);
        // PoolState currently has slot/write_version but no bank_id. Clear all pool
        // states at this slot rather than risk keeping one from the cleared candidate.
        self.cache
            .pools
            .write()
            .retain(|_, pool| pool.slot.0 != slot);
        self.dedupe.lock().items.retain(|key, _| key.slot.0 != slot);
        self.cache.version.fetch_add(1, Ordering::AcqRel);
        removed
    }

    pub fn bank_is_dead(&self, slot: u64, bank_id: Option<u64>) -> bool {
        self.slot_integrity.lock().is_bank_dead(slot, bank_id)
    }

    /// Record observed slot in thread-safe atomic tracker.
    pub fn record_slot(&self, slot: u64) {
        self.latest_slot.fetch_max(slot, Ordering::Relaxed);
    }

    /// Read the latest slot observed from the Yellowstone stream (atomic read).
    pub fn current_slot(&self) -> u64 {
        self.latest_slot.load(Ordering::Relaxed)
    }

    /// Decode a previously validated account update via an AMM pool adapter and apply.
    ///
    /// Pool state is decoded per-protocol: `kind`, `clmm`/`dlmm`, and mint
    /// decimals come from the pool implementation. `decimals_a`/`decimals_b`
    /// default to 0 (unknown) here because mint account data is not available
    /// in the stream update; the registry's `fetch_live_pool_states` resolves
    /// them from verified mint accounts. Pools with decimals == 0 are rejected
    /// by the economic engine (fail closed).
    pub fn apply_raw_account(
        &self,
        update: &AccountUpdate,
        pool: &mut dyn AmmPool,
    ) -> Result<bool> {
        pool.update_from_account(&update.data)?;
        let (r_in, r_out) = pool.reserves();
        let (decimals_a, decimals_b) = pool.decimals();

        let clmm = match pool.kind() {
            arb_types::PoolKind::Clmm => pool.clmm_state(),
            _ => None,
        };
        let dlmm = match pool.kind() {
            arb_types::PoolKind::Dlmm => pool.dlmm_state(),
            _ => None,
        };

        let state = PoolState {
            pool_id: pool.pool_id(),
            protocol: pool.protocol(),
            slot: update.slot,
            write_version: update.write_version,
            base_mint: pool.base_mint(),
            quote_mint: pool.quote_mint(),
            decimals_a,
            decimals_b,
            reserve_in: r_in,
            reserve_out: r_out,
            fee_numerator: pool.fee_bps().unwrap_or(0) as u64,
            fee_denominator: if pool.fee_bps().is_some() { 10_000 } else { 0 },
            is_active: r_in > 0 && r_out > 0 && pool.fee_bps().is_some(),
            has_risky_token: pool.has_risky_token(),
            transfer_fee_bps: 0,
            last_update_slot: update.slot.0,
            last_update_ms: u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0),
            )
            .unwrap_or(u64::MAX),
            kind: pool.kind(),
            clmm,
            dlmm,
            pump_fun: pool.pump_fun_state(),
            token_programs: Default::default(),
            execution_accounts: None,
        };

        self.apply_pool_update(state)
    }

    /// Build a quote dependency set while holding the slot-integrity read guard.
    /// Call again immediately before transaction construction/submission.
    pub fn quote_dependencies(
        &self,
        required: &[Pubkey],
        expected_owners: &HashMap<Pubkey, Pubkey>,
        target_slot: u64,
        max_age_slots: u64,
        max_slot_skew: u64,
    ) -> std::result::Result<Vec<AccountSnapshot>, QuoteReadinessError> {
        let slots = self.slot_integrity.lock();
        let generation = self.stream_generation();
        let connected = self.stream_connected();
        let observed_slot = self.current_slot();
        // Report the root cause first: a disconnected stream also clears the
        // slot watermark, and "disconnected" is the actionable diagnosis.
        if !connected {
            return Err(QuoteReadinessError::StreamDisconnected);
        }
        if observed_slot == 0 {
            return Err(QuoteReadinessError::NoSlotWatermark);
        }
        if target_slot > observed_slot {
            return Err(QuoteReadinessError::FutureSlot);
        }
        let snapshots = build_quote_dependency_set(
            &self.account_store,
            &slots,
            required,
            expected_owners,
            QuoteReadinessContext {
                stream_connected: connected,
                provider_id: self.provider_id,
                generation,
                target_slot: observed_slot,
                max_age_slots,
                max_slot_skew,
            },
        )?;
        // Close the race where disconnect/reconnect occurs while snapshots are copied.
        if !self.stream_connected() {
            return Err(QuoteReadinessError::StreamDisconnected);
        }
        if self.stream_generation() != generation {
            return Err(QuoteReadinessError::WrongGeneration);
        }
        Ok(snapshots)
    }

    /// Attach a streaming-only pool cache for dual-ingestion shadow mode.
    /// When set, Yellowstone account updates are decoded into PoolState and
    /// written to this cache instead of the shared MarketStateCache, enabling
    /// comparison against the HTTP-polling path.
    pub fn with_streaming_cache(self, cache: Arc<StreamingPoolCache>) -> Self {
        *self.streaming_cache.lock() = Some(cache);
        self
    }

    /// Get the streaming cache if attached.
    pub fn streaming_cache(&self) -> Option<Arc<StreamingPoolCache>> {
        self.streaming_cache.lock().clone()
    }

    /// Resolve route-specific account dependencies from the manifest, then apply the
    /// same fail-closed quote readiness checks used by direct dependency callers.
    pub fn quote_dependencies_for_route(
        &self,
        manifests: &DependencyManifestRegistry,
        pool_ids: &[PoolId],
        quote_slot: u64,
        max_age_slots: u64,
        max_slot_skew: u64,
    ) -> std::result::Result<Vec<AccountSnapshot>, QuoteReadinessError> {
        let (required, owners) = manifests
            .route_requirements(pool_ids)
            .map_err(|_| QuoteReadinessError::InvalidDependencyManifest)?;
        self.quote_dependencies(&required, &owners, quote_slot, max_age_slots, max_slot_skew)
    }

    /// Handle keepalive ping/pong.
    pub async fn handle_ping(&self) -> Result<()> {
        debug!("Ping/Pong keepalive exchanged (WebSocket/RPC ingestion)");
        Ok(())
    }

    /// Reconnect loop after stream disconnection.
    pub async fn reconnect(&self) -> Result<()> {
        let attempt = self.reconnect_attempts.fetch_add(1, Ordering::Relaxed);
        let delay = Self::backoff_delay(attempt);
        sleep(delay).await;

        let mut dedupe = self.dedupe.lock();
        dedupe.clear();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_types::{DexProtocol, MintId, PoolId, Slot};

    fn make_test_pool(slot: u64, wv: u64) -> PoolState {
        PoolState {
            pool_id: PoolId::new([1; 32]),
            protocol: DexProtocol::RaydiumV4,
            slot: Slot(slot),
            write_version: wv,
            base_mint: MintId::new([2; 32]),
            quote_mint: MintId::new([3; 32]),
            decimals_a: 9,
            decimals_b: 6,
            reserve_in: 100_000,
            reserve_out: 200_000,
            fee_numerator: 25,
            fee_denominator: 10_000,
            is_active: true,
            has_risky_token: false,
            transfer_fee_bps: 0,
            last_update_slot: 0,
            last_update_ms: 0,
            kind: arb_types::PoolKind::Cpmm,
            clmm: None,
            dlmm: None,
            pump_fun: None,
            token_programs: Default::default(),
            execution_accounts: None,
        }
    }

    #[test]
    fn account_snapshot_store_rejects_older_slot_or_write_version() {
        let store = AccountSnapshotStore::default();
        let key = Pubkey::new_from_array([7; 32]);
        let owner = Pubkey::new_from_array([8; 32]);
        let mk = |slot, write_version, byte| AccountSnapshot {
            pubkey: key,
            owner,
            slot,
            write_version,
            is_startup: false,
            provider_id: ProviderId(1),
            bank_id: Some(1),
            generation: 0,
            data: Arc::new(vec![byte]),
        };
        assert!(store.apply(mk(100, 10, 1)));
        assert!(!store.apply(mk(99, 99, 2)));
        assert!(!store.apply(mk(100, 9, 3)));
        assert!(store.apply(mk(100, 11, 4)));
        assert_eq!(
            store.get(&key).expect("stored account").data.as_slice(),
            &[4]
        );
    }

    #[test]
    fn market_update_changes_one_pool_without_replacing_cache_handle() {
        let client = IngestClient::new();
        let handle_before = client.snapshot();
        assert!(client.apply_pool_update(make_test_pool(10, 1)).unwrap());
        assert!(client.apply_pool_update(make_test_pool(11, 2)).unwrap());
        let handle_after = client.snapshot();
        assert!(Arc::ptr_eq(&handle_before, &handle_after));
        assert_eq!(handle_after.pool_count(), 1);
        assert_eq!(handle_after.version(), 2);
        assert!(!client.apply_pool_update(make_test_pool(9, 99)).unwrap());
        assert_eq!(
            handle_after.get_pool(&PoolId::new([1; 32])).unwrap().slot,
            Slot(11)
        );
    }

    #[test]
    fn dedupe_ring_buffer() {
        let mut dedupe = BoundedDedupeBuffer::new();
        let key1 = DedupeKey {
            pool_id: PoolId::new([1; 32]),
            slot: Slot(100),
            write_version: 1,
        };
        assert!(dedupe.check_and_insert(key1, 100));
        assert!(!dedupe.check_and_insert(key1, 100)); // Duplicate rejected

        // Beyond window slot 100 + 33 => key1 evicted
        let key2 = DedupeKey {
            pool_id: PoolId::new([2; 32]),
            slot: Slot(135),
            write_version: 1,
        };
        assert!(dedupe.check_and_insert(key2, 135));
        assert_eq!(dedupe.items.len(), 1); // key1 was safely evicted
    }

    #[tokio::test]
    async fn atomic_cache_update() {
        let client = IngestClient::new();
        let pool = make_test_pool(50, 1);
        assert!(client.apply_pool_update(pool).unwrap());

        let snap = client.snapshot();
        assert_eq!(snap.version(), 1);
        assert_eq!(snap.latest_slot(), 50);
        assert!(snap.get_pool(&PoolId::new([1; 32])).is_some());
    }

    #[test]
    fn account_store_rejects_same_slot_cross_bank_and_bootstrap_over_live() {
        let store = AccountSnapshotStore::default();
        let key = Pubkey::new_from_array([31; 32]);
        let owner = Pubkey::new_from_array([32; 32]);
        let live = AccountSnapshot {
            pubkey: key,
            owner,
            slot: 100,
            write_version: 8,
            is_startup: false,
            provider_id: ProviderId(1),
            bank_id: Some(10),
            generation: 2,
            data: Arc::new(vec![1]),
        };
        assert!(store.apply(live));
        let competing_bank = AccountSnapshot {
            bank_id: Some(11),
            write_version: 9,
            data: Arc::new(vec![2]),
            ..store.get(&key).unwrap()
        };
        assert!(!store.apply(competing_bank));
        let bootstrap = AccountSnapshot {
            provider_id: ProviderId(0),
            bank_id: None,
            is_startup: true,
            slot: 101,
            write_version: 0,
            data: Arc::new(vec![3]),
            ..store.get(&key).unwrap()
        };
        assert!(!store.apply(bootstrap));
        let other_provider = AccountSnapshot {
            provider_id: ProviderId(2),
            slot: 101,
            write_version: 99,
            data: Arc::new(vec![4]),
            ..store.get(&key).unwrap()
        };
        assert!(!store.apply(other_provider));
        assert_eq!(store.get(&key).unwrap().data.as_slice(), &[1]);
    }

    #[test]
    fn stream_generation_discards_stale_state_and_requires_new_connection() {
        let client = IngestClient::new();
        let generation = client.begin_stream_generation();
        assert_eq!(generation, 1);
        client.mark_stream_connected();
        assert!(client.stream_connected());
        client.mark_stream_disconnected();
        assert!(!client.stream_connected());
        assert!(client.account_store.is_empty());
        assert_eq!(client.cache().pool_count(), 0);
    }

    #[test]
    fn quote_dependency_gate_requires_live_current_generation_data() {
        let client = IngestClient::new();
        let generation = client.begin_stream_generation();
        client.record_slot_update(100, Some(99), slot_integrity::SLOT_PROCESSED);
        client.mark_stream_connected();
        let key = Pubkey::new_from_array([21; 32]);
        let owner = Pubkey::new_from_array([22; 32]);
        assert!(client.account_store.apply(AccountSnapshot {
            pubkey: key,
            owner,
            slot: 100,
            write_version: 9,
            is_startup: false,
            provider_id: ProviderId(1),
            bank_id: Some(1001),
            generation,
            data: Arc::new(vec![1, 2])
        }));
        let owners = HashMap::from([(key, owner)]);
        assert!(
            client
                .quote_dependencies(&[key], &owners, 100, 0, 0)
                .is_ok()
        );
        assert!(matches!(
            client.quote_dependencies(&[key], &owners, 101, 0, 0),
            Err(QuoteReadinessError::FutureSlot)
        ));
        client.mark_stream_disconnected();
        assert!(matches!(
            client.quote_dependencies(&[key], &owners, 100, 0, 0),
            Err(QuoteReadinessError::StreamDisconnected)
        ));
    }

    #[test]
    fn entry_update_parent_invalidates_only_cleared_bank_snapshot() {
        let client = IngestClient::new();
        let key = Pubkey::new_from_array([17; 32]);
        let owner = Pubkey::new_from_array([18; 32]);
        let snapshot = AccountSnapshot {
            pubkey: key,
            owner,
            slot: 77,
            write_version: 3,
            is_startup: false,
            provider_id: ProviderId(1),
            bank_id: Some(900),
            generation: 1,
            data: Arc::new(vec![1]),
        };
        assert!(client.account_store.apply(snapshot));
        assert!(client.apply_pool_update(make_test_pool(77, 1)).unwrap());
        assert_eq!(client.record_entry_update_parent(77, 900), 1);
        assert_eq!(client.cache().pool_count(), 0);
        assert!(client.account_store.get(&key).is_none());
        assert!(client.bank_is_dead(77, Some(900)));
        assert!(!client.bank_is_dead(77, Some(901)));
        let late = AccountSnapshot {
            pubkey: key,
            owner,
            slot: 77,
            write_version: 4,
            is_startup: false,
            provider_id: ProviderId(1),
            bank_id: Some(900),
            generation: 1,
            data: Arc::new(vec![2]),
        };
        assert!(!client.account_store.apply(late));
    }

    #[test]
    fn recreated_bank_reopens_slot_but_old_bank_remains_rejected() {
        let client = IngestClient::new();
        let key = Pubkey::new_from_array([27; 32]);
        let owner = Pubkey::new_from_array([28; 32]);
        client.record_slot_update_with_bank(
            88,
            Some(87),
            slot_integrity::SLOT_CREATED_BANK,
            Some(1000),
        );
        client.record_account_bank_id(88, Some(1000));
        assert!(client.account_store.apply(AccountSnapshot {
            pubkey: key,
            owner,
            slot: 88,
            write_version: 1,
            is_startup: false,
            provider_id: ProviderId(1),
            bank_id: Some(1000),
            generation: 1,
            data: Arc::new(vec![1])
        }));
        client.record_slot_update_with_bank(88, Some(87), slot_integrity::SLOT_DEAD, None);
        client.record_slot_update_with_bank(
            88,
            Some(87),
            slot_integrity::SLOT_CREATED_BANK,
            Some(1001),
        );
        assert!(!client.slot_is_dead(88));
        assert!(client.bank_is_dead(88, Some(1000)));
        assert!(!client.bank_is_dead(88, Some(1001)));
        let old = AccountSnapshot {
            pubkey: key,
            owner,
            slot: 88,
            write_version: 2,
            is_startup: false,
            provider_id: ProviderId(1),
            bank_id: Some(1000),
            generation: 1,
            data: Arc::new(vec![2]),
        };
        assert!(!client.account_store.apply(old));
        let replacement = AccountSnapshot {
            pubkey: key,
            owner,
            slot: 88,
            write_version: 1,
            is_startup: false,
            provider_id: ProviderId(1),
            bank_id: Some(1001),
            generation: 1,
            data: Arc::new(vec![3]),
        };
        assert!(client.account_store.apply(replacement));
    }

    #[test]
    fn dead_slot_invalidates_account_and_pool_cache_and_rejects_late_pool_update() {
        let client = IngestClient::new();
        let key = Pubkey::new_from_array([7; 32]);
        let owner = Pubkey::new_from_array([8; 32]);
        assert!(client.account_store.apply(AccountSnapshot {
            pubkey: key,
            owner,
            slot: 10,
            write_version: 1,
            is_startup: false,
            provider_id: ProviderId(1),
            bank_id: Some(1),
            generation: 0,
            data: Arc::new(vec![1])
        }));
        assert!(client.apply_pool_update(make_test_pool(10, 1)).unwrap());
        client.record_slot_update(10, Some(9), slot_integrity::SLOT_PROCESSED);
        client.record_slot_update(11, Some(10), slot_integrity::SLOT_PROCESSED);
        let result = client.record_slot_update(10, Some(9), slot_integrity::SLOT_DEAD);
        assert_eq!(
            result,
            SlotUpdateOutcome::Dead {
                invalidated_slots: vec![10, 11]
            }
        );
        assert!(client.account_store.get(&key).is_none());
        assert_eq!(client.cache().pool_count(), 0);
        assert!(!client.apply_pool_update(make_test_pool(11, 2)).unwrap());
    }
}

#[cfg(test)]
mod atomic_snapshot_batch_tests {
    use super::*;
    #[test]
    fn batch_publish_is_all_or_nothing_and_requires_one_slot() {
        let store = AccountSnapshotStore::default();
        let a = Pubkey::from([1u8; 32]);
        let b = Pubkey::from([2u8; 32]);
        let owner = Pubkey::from([3u8; 32]);
        let mk = |pubkey, slot| AccountSnapshot {
            pubkey,
            owner,
            slot,
            write_version: 0,
            is_startup: true,
            provider_id: ProviderId(0),
            bank_id: None,
            generation: 0,
            data: Arc::new(vec![1]),
        };
        assert!(!store.apply_batch(vec![mk(a, 10), mk(b, 11)]));
        assert!(store.is_empty());
        assert!(store.apply_batch(vec![mk(a, 10), mk(b, 10)]));
        assert_eq!(store.len(), 2);
        assert!(!store.apply_batch(vec![mk(a, 10), mk(b, 10)]));
        assert_eq!(store.len(), 2);
    }
}
