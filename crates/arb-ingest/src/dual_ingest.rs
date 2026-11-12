//! Dual-Ingestion Shadow Mode — WebSocket Streaming + HTTP RPC Polling.
//!
//! Runs WebSocket streaming and HTTP RPC polling in parallel, writing
//! decoded pool state to separate caches. A periodic comparison task checks
//! cache consistency and reports discrepancies via metrics. HTTP polling
//! remains the source of truth for trading decisions until streaming is
//! proven correct; polling is NOT removed until correctness is demonstrated.
//!
//! This is the shadow-mode component: both ingestion paths operate
//! simultaneously, but only the HTTP-polling cache is consumed by the
//! supervisor's route search.

use crate::{IngestClient, MarketStateCache};
use arb_types::{PoolId, PoolState};
use parking_lot::{Mutex, RwLock};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::sleep;
use tracing::{debug, warn};

/// Separate cache for Yellowstone-streaming-decoded pool state.
/// Isolated from `MarketStateCache` (HTTP polling path) so the two can be
/// compared for consistency without cross-contamination.
#[derive(Debug, Default)]
pub struct StreamingPoolCache {
    pools: RwLock<HashMap<PoolId, PoolState>>,
    version: std::sync::atomic::AtomicU64,
    latest_slot: std::sync::atomic::AtomicU64,
}

impl StreamingPoolCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or update a pool state from the streaming path.
    /// Rejects older slot/write-version data (same freshness rules as
    /// `MarketStateCache`).
    pub fn apply(&self, state: PoolState) -> bool {
        let slot_val = state.slot.0;
        let mut pools = self.pools.write();
        if let Some(current) = pools.get(&state.pool_id) {
            if (slot_val, state.write_version) <= (current.slot.0, current.write_version) {
                return false; // stale
            }
        }
        pools.insert(state.pool_id, state);
        self.version
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        self.latest_slot
            .fetch_max(slot_val, std::sync::atomic::Ordering::AcqRel);
        true
    }

    pub fn snapshot(&self) -> Vec<PoolState> {
        self.pools.read().values().cloned().collect()
    }

    pub fn get(&self, pool_id: &PoolId) -> Option<PoolState> {
        self.pools.read().get(pool_id).cloned()
    }

    pub fn version(&self) -> u64 {
        self.version.load(std::sync::atomic::Ordering::Acquire)
    }

    pub fn latest_slot(&self) -> u64 {
        self.latest_slot.load(std::sync::atomic::Ordering::Acquire)
    }

    pub fn pool_count(&self) -> usize {
        self.pools.read().len()
    }

    pub fn clear(&self) {
        self.pools.write().clear();
        self.version
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        self.latest_slot
            .store(0, std::sync::atomic::Ordering::Release);
    }
}

/// Comparison result between streaming and HTTP caches for a single pool.
#[derive(Debug, Clone)]
pub struct PoolConsistencyCheck {
    pub pool_id: PoolId,
    pub streaming_slot: u64,
    pub http_slot: u64,
    pub streaming_reserve_in: u128,
    pub http_reserve_in: u128,
    pub streaming_reserve_out: u128,
    pub http_reserve_out: u128,
    pub consistent: bool,
}

/// Dual-ingestion shadow manager.
///
/// Coordinates Yellowstone streaming and HTTP polling, maintaining separate
/// caches and periodically comparing them for consistency.
pub struct DualIngestShadow {
    yellowstone: Arc<IngestClient>,
    streaming_cache: Arc<StreamingPoolCache>,
    comparison_interval: Duration,
    last_comparison: Mutex<Instant>,
    pools_checked: Mutex<usize>,
    mismatches: Mutex<usize>,
}

impl DualIngestShadow {
    /// Create a new dual-ingestion shadow manager.
    pub fn new(yellowstone: Arc<IngestClient>) -> Self {
        Self {
            yellowstone,
            streaming_cache: Arc::new(StreamingPoolCache::new()),
            comparison_interval: Duration::from_secs(30),
            last_comparison: Mutex::new(Instant::now()),
            pools_checked: Mutex::new(0),
            mismatches: Mutex::new(0),
        }
    }

    /// Set the comparison interval.
    pub fn with_comparison_interval(mut self, interval: Duration) -> Self {
        self.comparison_interval = interval;
        self
    }

    /// Apply a pool state decoded from a Yellowstone account update.
    /// Called from the Yellowstone stream loop for each DEX account update.
    pub fn apply_streaming_pool(&self, state: PoolState) -> bool {
        let accepted = self.streaming_cache.apply(state.clone());
        if !accepted {
            arb_metrics::inc_stale_rejections();
        }
        accepted
    }

    /// Get the streaming cache handle.
    pub fn streaming_cache(&self) -> Arc<StreamingPoolCache> {
        Arc::clone(&self.streaming_cache)
    }

    /// Run one consistency comparison pass between streaming and HTTP caches.
    /// Returns the number of pools checked and mismatches found.
    pub fn compare_caches(&self, http_cache: &MarketStateCache) -> (usize, usize) {
        let streaming = self.streaming_cache.snapshot();
        let http_pools = http_cache.snapshot_pools();

        // Build HTTP lookup by pool id.
        let http_map: HashMap<PoolId, PoolState> =
            http_pools.into_iter().map(|p| (p.pool_id, p)).collect();

        let mut checked = 0usize;
        let mut mismatches = 0usize;

        for s in &streaming {
            checked += 1;
            if let Some(h) = http_map.get(&s.pool_id) {
                // Compare reserves and slot. Allow small slot skew.
                let slot_diff = s.slot.0.abs_diff(h.slot.0);
                let reserve_match = s.reserve_in == h.reserve_in && s.reserve_out == h.reserve_out;
                let fresh_enough = slot_diff <= 5; // within 5 slots

                let consistent = reserve_match && fresh_enough;
                if !consistent {
                    mismatches += 1;
                    warn!(
                        pool_id = ?s.pool_id,
                        streaming_slot = s.slot.0,
                        http_slot = h.slot.0,
                        streaming_reserve_in = ?s.reserve_in,
                        http_reserve_in = ?h.reserve_in,
                        streaming_reserve_out = ?s.reserve_out,
                        http_reserve_out = ?h.reserve_out,
                        "Dual-ingestion cache mismatch"
                    );
                }
            }
            // Pools only in streaming (not yet in HTTP) are not mismatches;
            // they may simply not have been polled yet.
        }

        *self.pools_checked.lock() += checked;
        *self.mismatches.lock() += mismatches;
        *self.last_comparison.lock() = Instant::now();

        let total = *self.pools_checked.lock();
        let bad = *self.mismatches.lock();
        let consistent = if total == 0 {
            true
        } else {
            (total - bad) as f64 / total as f64 >= 0.95
        };
        arb_metrics::set_cache_consistency(consistent);

        debug!(
            pools_checked = checked,
            mismatches = mismatches,
            consistent = consistent,
            "Dual-ingestion cache comparison complete"
        );

        (checked, mismatches)
    }

    /// Run the background comparison loop.
    /// Polls `http_cache` at `comparison_interval` and compares against
    /// the streaming cache.
    pub async fn run_comparison_loop(
        self: Arc<Self>,
        http_cache: Arc<MarketStateCache>,
        cancel: tokio_util::sync::CancellationToken,
    ) {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = sleep(self.comparison_interval) => {
                    let _ = self.compare_caches(&http_cache);
                }
            }
        }
    }

    /// Record a resync event (after Yellowstone reconnect).
    pub fn record_resync(&self) {
        arb_metrics::inc_resyncs();
        warn!("Dual-ingestion: resync recorded after Yellowstone reconnect");
    }

    /// Record a decode error.
    pub fn record_decode_error(&self) {
        arb_metrics::inc_decode_errors();
    }

    /// Get total pools checked across all comparisons.
    pub fn pools_checked(&self) -> usize {
        *self.pools_checked.lock()
    }

    /// Get total mismatches across all comparisons.
    pub fn mismatches(&self) -> usize {
        *self.mismatches.lock()
    }

    /// Get the timestamp of the last cache comparison.
    pub fn last_comparison(&self) -> Instant {
        *self.last_comparison.lock()
    }
}

impl Clone for DualIngestShadow {
    fn clone(&self) -> Self {
        Self {
            yellowstone: Arc::clone(&self.yellowstone),
            streaming_cache: Arc::clone(&self.streaming_cache),
            comparison_interval: self.comparison_interval,
            last_comparison: Mutex::new(Instant::now()),
            pools_checked: Mutex::new(*self.pools_checked.lock()),
            mismatches: Mutex::new(*self.mismatches.lock()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_types::{DexProtocol, MintId, PoolKind, Slot, TokenProgramKind};

    fn make_stream_pool(slot: u64, wv: u64, ri: u128, ro: u128) -> PoolState {
        PoolState {
            pool_id: PoolId::new([1; 32]),
            protocol: DexProtocol::RaydiumV4,
            slot: Slot(slot),
            write_version: wv,
            base_mint: MintId::new([2; 32]),
            quote_mint: MintId::new([3; 32]),
            decimals_a: 9,
            decimals_b: 6,
            reserve_in: ri,
            reserve_out: ro,
            fee_numerator: 25,
            fee_denominator: 10_000,
            is_active: true,
            has_risky_token: false,
            kind: PoolKind::Cpmm,
            clmm: None,
            dlmm: None,
            pump_fun: None,
            transfer_fee_bps: 0,
            token_programs: [TokenProgramKind::SplToken; 2],
            execution_accounts: None,
            last_update_slot: slot,
            last_update_ms: 0,
        }
    }

    #[test]
    fn streaming_cache_rejects_stale_updates() {
        let cache = StreamingPoolCache::new();
        let p1 = make_stream_pool(100, 1, 1000, 2000);
        let p2 = make_stream_pool(99, 99, 3000, 4000); // older slot
        let p3 = make_stream_pool(100, 0, 5000, 6000); // same slot, older write_version
        assert!(cache.apply(p1));
        assert!(!cache.apply(p2));
        assert!(!cache.apply(p3));
        let p4 = make_stream_pool(100, 2, 7000, 8000); // same slot, newer write_version
        assert!(cache.apply(p4));
        assert_eq!(cache.get(&PoolId::new([1; 32])).unwrap().reserve_in, 7000);
    }

    #[test]
    fn cache_consistency_detects_reserve_mismatch() {
        let shadow = DualIngestShadow::new(Arc::new(crate::IngestClient::new()));
        let streaming = shadow.streaming_cache();

        let http_cache = MarketStateCache::default();

        // Streaming pool with different reserves than HTTP
        let s = make_stream_pool(100, 1, 1000, 2000);
        streaming.apply(s.clone());

        let mut http_pools = http_cache.pools.write();
        let h = make_stream_pool(100, 1, 9999, 2000); // different reserve_in
        http_pools.insert(h.pool_id, h);
        drop(http_pools);

        let (checked, mismatches) = shadow.compare_caches(&http_cache);
        assert_eq!(checked, 1);
        assert_eq!(mismatches, 1);
    }

    #[test]
    fn cache_consistency_detects_slot_mismatch() {
        let shadow = DualIngestShadow::new(Arc::new(crate::IngestClient::new()));
        let streaming = shadow.streaming_cache();

        let http_cache = MarketStateCache::default();

        let s = make_stream_pool(100, 1, 1000, 2000);
        streaming.apply(s.clone());

        let mut http_pools = http_cache.pools.write();
        let h = make_stream_pool(200, 1, 1000, 2000); // slot 100 slots ahead
        http_pools.insert(h.pool_id, h);
        drop(http_pools);

        let (checked, mismatches) = shadow.compare_caches(&http_cache);
        assert_eq!(checked, 1);
        assert_eq!(mismatches, 1);
    }

    #[test]
    fn consistent_caches_report_no_mismatch() {
        let shadow = DualIngestShadow::new(Arc::new(crate::IngestClient::new()));
        let streaming = shadow.streaming_cache();

        let http_cache = MarketStateCache::default();

        let s = make_stream_pool(100, 1, 1000, 2000);
        streaming.apply(s.clone());

        let mut http_pools = http_cache.pools.write();
        let h = make_stream_pool(100, 1, 1000, 2000); // identical
        http_pools.insert(h.pool_id, h);
        drop(http_pools);

        let (checked, mismatches) = shadow.compare_caches(&http_cache);
        assert_eq!(checked, 1);
        assert_eq!(mismatches, 0);
    }
}
