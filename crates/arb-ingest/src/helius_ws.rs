//! Multi-provider WebSocket Client for real-time Solana slot and account streams.
//! Subscribes to `slotSubscribe` and targeted `accountSubscribe`
//! with automatic failover across configured endpoints (Shyft, Helius, RPCFast, etc.),
//! live `MarketStateCache` updates, and instant slot notifications.

use crate::{
    AccountSnapshot, IngestClient, RegisteredPool, clmm_tick_array_covers_tick,
    compute_clmm_virtual_reserves, compute_dlmm_reserves, decode_clmm_state, decode_dlmm_fee_bps,
    decode_dlmm_state, decode_orca_fee_bps, decode_raydium_clmm_state,
    decode_spl_token_amount_bytes, next_pool_write_version, slot_integrity,
};
use arb_types::{DexProtocol, MintId, PoolId, Slot};
use base64::Engine;
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use solana_pubkey::Pubkey;
use std::collections::{HashMap, VecDeque};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Notify;
use tokio::time::sleep;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

/// Maximum time allowed for a provider WebSocket handshake before rotating.
const WS_CONNECT_TIMEOUT: Duration = Duration::from_secs(8);

/// Hard ceiling on concurrent `accountSubscribe` streams per connection.
///
/// With 160+ registered pools this budget is the binding coverage constraint:
/// every Raydium V4/CPMM pool consumes TWO subscriptions (base + quote vault)
/// and every Orca/DLMM pool consumes ONE (pool account), so the historical
/// default of 48 left most of the registered market graph permanently blind —
/// those pools keep their zero-value discovery-time state, the graph
/// freshness filter drops them, and `find_base_cycles` sees no WSOL-anchored
/// cycles (measured: 49/320 directed edges, 0 opportunities in a 3-min run).
///
/// Overridable via `WS_MAX_ACCOUNT_SUBS`. Raising it is safe against
/// restrictive providers: a rejected subscription flips `account_sub_limited`
/// and the worker stops queueing (fail-closed, no teardown loop).
///
/// Capability failures (`Method '...' not found` / `unsupported`, e.g. Alchemy
/// WS rejecting `accountSubscribe` with -32601) are FATAL for the endpoint:
/// `endpoint_unsupported` is raised, the connection is torn down with a backoff,
/// and the worker fails over. Treating these as soft warnings strands the bot
/// on a blind endpoint forever (measured live: 0 account updates, SOL/USD
/// reference never resolved, graph sizing permanently skipped).
fn validated_hydration_slot(
    context_slot: u64,
    snapshot_slot: u64,
    trigger_slot: u64,
) -> Option<u64> {
    (context_slot > 0 && context_slot >= snapshot_slot.max(trigger_slot)).then_some(context_slot)
}

fn max_ws_account_subs() -> usize {
    std::env::var("WS_MAX_ACCOUNT_SUBS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(48)
        .min(1_024)
}

fn slot_subscribe_enabled_from_env_value(v: Option<String>) -> bool {
    !matches!(
        v.as_deref().map(str::trim),
        Some("0") | Some("false") | Some("off") | Some("no")
    )
}

fn slot_subscribe_enabled_from_env() -> bool {
    slot_subscribe_enabled_from_env_value(std::env::var("WS_SLOT_SUBSCRIBE").ok())
}

pub struct HeliusWsClient {
    pub ws_urls: Vec<String>,
    pub current_idx: Arc<AtomicUsize>,
    pub latest_slot: Arc<AtomicU64>,
    pub is_connected: Arc<AtomicBool>,
    tracked_accounts: parking_lot::RwLock<HashMap<Pubkey, Pubkey>>, // account -> expected owner
    tracked_order: parking_lot::RwLock<Vec<Pubkey>>,
    /// Accounts registered while a WS connection is active; drained by the stream loop.
    pending_dynamic_subscriptions: parking_lot::Mutex<VecDeque<Pubkey>>,
    dynamic_subscribe_notify: Arc<Notify>,
    registered_pools: parking_lot::RwLock<HashMap<PoolId, RegisteredPool>>,
    vault_to_pools: parking_lot::RwLock<HashMap<Pubkey, Vec<(PoolId, bool)>>>,
    req_to_account: parking_lot::RwLock<HashMap<u64, Pubkey>>,
    sub_to_account: parking_lot::RwLock<HashMap<u64, Pubkey>>,
    ws_backoffs: parking_lot::RwLock<HashMap<usize, Instant>>,
    account_sub_limited: AtomicBool,
    /// Whether to send `slotSubscribe` on connect. Gated by the
    /// `WS_SLOT_SUBSCRIBE` env flag and by live capability: a provider that
    /// rejects the slot subscription keeps streaming accounts, and slot
    /// freshness falls back to `accountNotification` context slots. Slot
    /// subscriptions share the same concurrent-subscription budget as account
    /// subscriptions, so on a 1-sub provider the slot sub must not consume it.
    slot_subscribe_enabled: AtomicBool,
    /// Current endpoint cannot stream accounts at all (capability failure).
    /// Reset on every successful connect; checked by the stream loop to break
    /// and fail over instead of sitting on a blind connection.
    endpoint_unsupported: AtomicBool,
    probe_confirmed: AtomicBool,
    slot_notify: Arc<Notify>,
    /// Wakes the derived-account hydrator as soon as a WS update queues work.
    /// The pending map coalesces repeated updates by pool before the worker drains it.
    derived_hydration_notify: Arc<Notify>,
    /// Shared RPC pool for the startup seed and event-driven derived-account
    /// hydration (tick arrays / bin arrays). One pool per client, not per call.
    rpc_pool: crate::rpc_pool::RpcPool,
    /// Pools whose derived-account coverage failed a WS update and need a
    /// targeted hydration fetch. Value = trigger slot.
    pending_derived: parking_lot::Mutex<HashMap<PoolId, u64>>,
    /// Per-pool hydrated coverage: (covered array indices/starts, last attempt).
    derived_covered: parking_lot::Mutex<HashMap<PoolId, (Vec<i64>, Instant)>>,
}

impl HeliusWsClient {
    pub fn new(ws_urls: Vec<String>) -> Self {
        Self::new_with_options(ws_urls, slot_subscribe_enabled_from_env())
    }

    /// Deterministic constructor for capability-gating tests and callers that
    /// already know the provider's slot-subscription capability.
    pub fn new_with_options(ws_urls: Vec<String>, slot_subscribe_enabled: bool) -> Self {
        Self {
            ws_urls,
            current_idx: Arc::new(AtomicUsize::new(0)),
            latest_slot: Arc::new(AtomicU64::new(0)),
            is_connected: Arc::new(AtomicBool::new(false)),
            tracked_accounts: parking_lot::RwLock::new(HashMap::new()),
            tracked_order: parking_lot::RwLock::new(Vec::new()),
            pending_dynamic_subscriptions: parking_lot::Mutex::new(VecDeque::new()),
            dynamic_subscribe_notify: Arc::new(Notify::new()),
            registered_pools: parking_lot::RwLock::new(HashMap::new()),
            vault_to_pools: parking_lot::RwLock::new(HashMap::new()),
            req_to_account: parking_lot::RwLock::new(HashMap::new()),
            sub_to_account: parking_lot::RwLock::new(HashMap::new()),
            ws_backoffs: parking_lot::RwLock::new(HashMap::new()),
            account_sub_limited: AtomicBool::new(false),
            slot_subscribe_enabled: AtomicBool::new(slot_subscribe_enabled),
            endpoint_unsupported: AtomicBool::new(false),
            probe_confirmed: AtomicBool::new(false),
            slot_notify: Arc::new(Notify::new()),
            derived_hydration_notify: Arc::new(Notify::new()),
            rpc_pool: crate::rpc_pool::RpcPool::from_env(),
            pending_derived: parking_lot::Mutex::new(HashMap::new()),
            derived_covered: parking_lot::Mutex::new(HashMap::new()),
        }
    }

    pub fn from_env() -> Option<Self> {
        let mut urls = Self::collect_ws_urls(|k| std::env::var(k).ok());
        if urls.is_empty() {
            // Configurable fallback to Solana Public WebSockets
            let primary = std::env::var("SOLANA_MAINNET_PUBLIC_WS")
                .unwrap_or_else(|_| "wss://api.mainnet-beta.solana.com".to_string());
            let fallback = std::env::var("SOLANA_PUBLICNODE_WS")
                .unwrap_or_else(|_| "wss://solana-rpc.publicnode.com".to_string());
            urls.push(primary);
            urls.push(fallback);
        }
        Some(Self::new(urls))
    }

    pub fn collect_ws_urls<F>(lookup: F) -> Vec<String>
    where
        F: Fn(&str) -> Option<String>,
    {
        let mut urls = Vec::new();
        // Priority 1: Multi-provider endpoints prioritizing explicit WS_URLs, SOLAMI, SHYFT, and HELIUS
        for key in [
            "WS_URL_PRIMARY",
            "WS_URL_SECONDARY",
            "WS_URL_FALLBACK",
            "SOLAMI_RPC_WS_URL",
            "SOLAMI_WS",
            "SHYFT_WS",
            "SHYFT_WS_URL",
            "SHYFT_RPC_WS_URL",
            "SOLANA_RPC_WS_URL",
            "HELIUS_WS_URL",
            "HELIUS_WS",
            "SOLANA_WS_PRIMARY",
            "SOLANA_WS_FALLBACK_1",
            "SOLANA_WS_FALLBACK_2",
            "SOLANA_WS_URL",
        ] {
            if let Some(u) = lookup(key) {
                let trimmed = u.trim().to_string();
                if !trimmed.is_empty()
                    && !trimmed.contains("REPLACE_ME")
                    && (trimmed.starts_with("wss://") || trimmed.starts_with("ws://"))
                    && !urls.contains(&trimmed)
                {
                    urls.push(trimmed);
                }
            }
        }

        // Priority 2: Numbered WS slots & named fallbacks
        let mut ordered_keys: Vec<String> =
            (1..=10).map(|n| format!("SOLANA_WS_{n}_URL")).collect();
        ordered_keys.extend([
            "ALCHEMY_WS_URL".to_string(),
            "FAST_WEBSOCKET".to_string(),
            "HELIUS_WS_URL".to_string(),
            "SOLAMI_RPC_WS_URL".to_string(),
        ]);

        for key in &ordered_keys {
            if let Some(u) = lookup(key) {
                let trimmed = u.trim().to_string();
                if !trimmed.is_empty()
                    && !trimmed.contains("REPLACE_ME")
                    && (trimmed.starts_with("wss://") || trimmed.starts_with("ws://"))
                    && !urls.contains(&trimmed)
                {
                    urls.push(trimmed);
                }
            }
        }
        if let Some(key) = lookup("HELIUS_API_KEY").or_else(|| lookup("HELIUS_API")) {
            let k = key.trim();
            if !k.is_empty() && !k.contains("REPLACE_ME") {
                let clean_key = if let Some(p) = k.split("api-key=").nth(1) {
                    p.split('&').next().unwrap_or(p)
                } else {
                    k
                };
                if !clean_key.is_empty() {
                    let u = format!("wss://mainnet.helius-rpc.com/?api-key={clean_key}");
                    if !urls.contains(&u) {
                        urls.push(u);
                    }
                }
            }
        }

        // Priority 3: Alchemy WebSocket from RPC URLs if present
        for n in 1..=10 {
            let key = format!("SOLANA_RPC_{n}_URL");
            if let Some(u) = lookup(&key) {
                let trimmed = u.trim();
                if trimmed.contains("alchemy.com") && trimmed.starts_with("https://") {
                    let ws_u = trimmed.replace("https://", "wss://");
                    if !urls.contains(&ws_u) {
                        urls.push(ws_u);
                    }
                }
            }
        }

        // Prioritize WebSocket endpoints that fully support account subscriptions.
        // Helius & general multi-sub endpoints come first.
        // Alchemy WS does not support accountSubscribe on standard tiers, so push it to the back.
        let mut multi_sub: Vec<String> = urls
            .iter()
            .filter(|u| !u.contains("rpcfast.com") && !u.contains("alchemy.com"))
            .cloned()
            .collect();
        multi_sub.extend(urls.iter().filter(|u| u.contains("alchemy.com")).cloned());
        multi_sub.extend(urls.into_iter().filter(|u| u.contains("rpcfast.com")));
        multi_sub.dedup();
        multi_sub
    }

    /// Advance to the next configured WebSocket endpoint and return the new index.
    pub fn rotate_endpoint(&self) -> usize {
        if self.ws_urls.is_empty() {
            return 0;
        }
        let prev = self.current_idx.load(Ordering::Acquire);
        let next = (prev + 1) % self.ws_urls.len();
        self.current_idx.store(next, Ordering::Release);
        next
    }

    /// Pick the first available (not backed-off) endpoint starting from `current_idx`.
    fn select_active_endpoint_index(&self) -> usize {
        let len = self.ws_urls.len();
        if len <= 1 {
            return 0;
        }
        let start = self.current_idx.load(Ordering::Acquire) % len;
        let now = Instant::now();
        let backoffs = self.ws_backoffs.read();
        for offset in 0..len {
            let candidate = (start + offset) % len;
            if backoffs.get(&candidate).is_none_or(|until| now >= *until) {
                self.current_idx.store(candidate, Ordering::Release);
                return candidate;
            }
        }
        // All endpoints are in backoff — pick the one whose cooldown expires earliest.
        let best = (0..len)
            .min_by_key(|idx| backoffs.get(idx).copied().unwrap_or(now))
            .unwrap_or(start);
        self.current_idx.store(best, Ordering::Release);
        best
    }

    fn redact_url(_url: &str) -> String {
        // Do not log provider URLs: credentials may be in query strings, userinfo,
        // or path segments depending on the provider.
        "configured WebSocket endpoint".to_string()
    }

    fn classify_connect_error(error_text: &str) -> &'static str {
        let lower = error_text.to_ascii_lowercase();
        if lower.contains("429") || lower.contains("too many") {
            "429 rate limited"
        } else if lower.contains("401") {
            "401 Unauthorized"
        } else if lower.contains("403") {
            "403 Forbidden"
        } else if lower.contains("400") {
            "400 Bad Request"
        } else if lower.contains("405") {
            "405 Method Not Allowed"
        } else {
            "websocket transport error"
        }
    }

    /// Share notifier fired whenever `slotNotification` advances the slot watermark.
    pub fn slot_notifier(&self) -> Arc<Notify> {
        Arc::clone(&self.slot_notify)
    }

    pub fn track_account(&self, pubkey: Pubkey, expected_owner: Pubkey) {
        let inserted = {
            let mut tracked = self.tracked_accounts.write();
            if tracked.insert(pubkey, expected_owner).is_none() {
                self.tracked_order.write().push(pubkey);
                true
            } else {
                false
            }
        };

        // Startup registrations are picked up by the initial subscription list.
        // Only accounts added to an already-connected stream need the dynamic queue.
        if inserted && self.is_connected.load(Ordering::Acquire) {
            self.pending_dynamic_subscriptions.lock().push_back(pubkey);
            self.dynamic_subscribe_notify.notify_one();
        }
    }

    /// Register a DEX pool so incoming WS `accountNotification` updates immediately
    /// refresh `MarketStateCache` without waiting for an HTTP poll cycle.
    pub fn register_pool(&self, pool: &RegisteredPool) {
        self.registered_pools
            .write()
            .insert(pool.pool_id, pool.clone());
        match pool.protocol {
            DexProtocol::RaydiumV4 | DexProtocol::RaydiumCpmm => {
                if let Ok(token_prog) = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID) {
                    self.track_account(pool.base_vault, token_prog);
                    self.track_account(pool.quote_vault, token_prog);
                    let mut vmap = self.vault_to_pools.write();
                    let base_list = vmap.entry(pool.base_vault).or_default();
                    if !base_list
                        .iter()
                        .any(|(id, is_b)| *id == pool.pool_id && *is_b)
                    {
                        base_list.push((pool.pool_id, true));
                    }
                    let quote_list = vmap.entry(pool.quote_vault).or_default();
                    if !quote_list
                        .iter()
                        .any(|(id, is_b)| *id == pool.pool_id && !*is_b)
                    {
                        quote_list.push((pool.pool_id, false));
                    }
                }
            }
            DexProtocol::OrcaWhirlpool => {
                if let Ok(owner) = Pubkey::from_str(arb_constants::ORCA_WHIRLPOOL_PROGRAM_ID) {
                    self.track_account(pool.pool_id.0, owner);
                }
            }
            DexProtocol::RaydiumClmm => {
                if let Ok(owner) = Pubkey::from_str(arb_constants::RAYDIUM_CLMM_PROGRAM_ID) {
                    self.track_account(pool.pool_id.0, owner);
                }
            }
            DexProtocol::MeteoraDlmm => {
                if let Ok(owner) = Pubkey::from_str(arb_constants::METEORA_DLMM_PROGRAM_ID) {
                    self.track_account(pool.pool_id.0, owner);
                }
            }
            DexProtocol::PumpFun => {
                if let Ok(owner) = Pubkey::from_str(arb_constants::PUMP_FUN_PROGRAM_ID) {
                    self.track_account(pool.pool_id.0, owner);
                }
            }
            _ => {}
        }
    }

    /// Subscription-priority score for a tracked account: 0 when the account
    /// belongs to a pool paired with the WSOL base anchor (vault accounts map
    /// through `vault_to_pools`; pool accounts are their own `PoolId`),
    /// 1 otherwise. O(1) map lookups, no allocation on the hot path.
    fn account_wsol_linkage(&self, acc: &Pubkey) -> u8 {
        let wsol = match Pubkey::from_str(arb_constants::WSOL_MINT) {
            Ok(k) => k,
            Err(_) => return 1,
        };
        let pools_of = |pool_id: &PoolId| {
            self.registered_pools
                .read()
                .get(pool_id)
                .map(|p| p.base_mint.0 == wsol || p.quote_mint.0 == wsol)
                .unwrap_or(false)
        };
        if let Some(targets) = self.vault_to_pools.read().get(acc) {
            if targets.iter().any(|(id, _)| pools_of(id)) {
                return 0;
            }
        }
        if pools_of(&PoolId(*acc)) {
            return 0;
        }
        1
    }

    pub fn current_slot(&self) -> u64 {
        self.latest_slot.load(Ordering::Acquire)
    }

    pub fn is_connected(&self) -> bool {
        self.is_connected.load(Ordering::Acquire)
    }

    /// Pre-populate authoritative account snapshots using sequential paced RPC
    /// so that WSOL mint (9 decimals) and Kamino reserve are present and verified immediately
    /// without sending concurrent bursts or triggering HTTP 429.
    pub async fn seed_initial_accounts(&self, ingest: &IngestClient) {
        let keys: Vec<Pubkey> = {
            let order = self.tracked_order.read();
            let map = self.tracked_accounts.read();
            if order.is_empty() {
                map.keys().copied().collect()
            } else {
                order
                    .iter()
                    .filter(|k| map.contains_key(k))
                    .copied()
                    .collect()
            }
        };
        if keys.is_empty() {
            return;
        }

        // Batched bootstrap via the shared RpcPool: ~3 `getMultipleAccounts`
        // batch-100 requests for ~250 tracked accounts instead of 167
        // sequential single-account calls. The sequential path was measured
        // (soak5/soak7) seeding only 2 of ~167 accounts: it starts while both
        // RPC endpoints are still in 429 cooldown from the discovery burst,
        // and each single-account failure burns a paced round-trip, so the
        // run began with ~98% of pools having no initial state — WS vault
        // updates then had nothing to mutate and the graph sat at 2 edges.
        let rpc = crate::rpc_pool::RpcPool::from_env();

        let mut seeded_count = 0usize;
        let mut latest_slot = 0u64;

        // Retry with backoff: this bootstraps the ENTIRE run's initial state.
        // It fires right after the discovery burst, when both free-tier
        // endpoints are routinely in 429 cooldown (measured soak8: 0/148
        // seeded, then the whole run sat at 2 graph edges). A blind single
        // attempt here is a silent total-seed failure.
        let mut chunk_results = Vec::new();
        for attempt in 0..3u32 {
            if attempt > 0 {
                sleep(Duration::from_secs(5)).await;
            }
            chunk_results = futures::future::join_all(keys.chunks(100).map(|chunk| {
                let rpc = &rpc;
                let chunk: Vec<Pubkey> = chunk.to_vec();
                async move {
                    let refs_owned: Vec<String> = chunk.iter().map(|k| k.to_string()).collect();
                    let refs: Vec<&str> = refs_owned.iter().map(|s| s.as_str()).collect();
                    // DATA-01: per-account slots — the 5-key fallback fans out
                    // across endpoints; a max would mask a slow replica.
                    (chunk, rpc.get_multiple_accounts_with_slots(&refs).await)
                }
            }))
            .await;
            if chunk_results.iter().any(|(_, r)| r.is_ok()) {
                break;
            }
            warn!(
                "seed_initial_accounts: attempt {} of 3 failed (all RPC endpoints rate-limited); retrying in 5s",
                attempt + 1
            );
        }

        let expected_owners = self.tracked_accounts.read().clone();
        for (chunk_pks, chunk_res) in chunk_results {
            let Ok((accounts, slots)) = chunk_res else {
                debug!(
                    "seed_initial_accounts: batch fetch of {} accounts failed: {}",
                    chunk_pks.len(),
                    chunk_res.unwrap_err()
                );
                continue;
            };
            latest_slot = latest_slot.max(slots.iter().copied().max().unwrap_or(0));
            for ((val, acct_slot), pk) in accounts.into_iter().zip(slots).zip(chunk_pks.iter()) {
                let Some(acc) = val else { continue };
                let Some(owner_str) = acc.get("owner").and_then(|o| o.as_str()) else {
                    continue;
                };
                let Ok(onchain_owner) = Pubkey::from_str(owner_str) else {
                    continue;
                };
                // Fail-closed identity check: never seed an account whose
                // on-chain owner differs from the program we registered it
                // under (account-substitution defense at bootstrap).
                match expected_owners.get(pk) {
                    Some(expected)
                        if *expected != Pubkey::default() && *expected != onchain_owner =>
                    {
                        debug!(
                            "seed_initial_accounts: owner mismatch for {pk}: expected {expected}, got {onchain_owner}; skipping"
                        );
                        continue;
                    }
                    _ => {}
                }
                let Some(data_arr) = acc.get("data").and_then(|d| d.as_array()) else {
                    continue;
                };
                let Some(raw_b64) = data_arr.first().and_then(|s| s.as_str()) else {
                    continue;
                };
                let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(raw_b64) else {
                    continue;
                };
                let snap = AccountSnapshot {
                    pubkey: *pk,
                    owner: onchain_owner,
                    slot: acct_slot,
                    write_version: 1,
                    is_startup: true,
                    provider_id: crate::HTTP_RPC_PROVIDER_ID,
                    bank_id: None,
                    generation: ingest.stream_generation(),
                    data: Arc::new(bytes),
                };
                ingest.account_store().apply(snap);
                seeded_count += 1;
            }
        }

        if seeded_count > 0 {
            if latest_slot > 0 {
                // The account batch came from HTTP RPC. It may pace the
                // cluster clock, but must not count as WebSocket liveness.
                ingest.record_slot(latest_slot);
            }
            info!(
                "Seeded {seeded_count}/{} initial snapshots via batched getMultipleAccounts at slot {latest_slot}",
                keys.len()
            );
        } else {
            warn!(
                "seed_initial_accounts: 0/{} snapshots seeded (all RPC endpoints failed or rate-limited); pools stay unseeded until WS updates or the ingestion worker hydrates them",
                keys.len()
            );
        }
    }

    /// Reset per-connection state when a WebSocket (re)connection is accepted.
    /// Verbatim extraction from `start_stream` (behavior-preserving) so the
    /// reconnect lifecycle is regression-testable: subscription request/response
    /// maps are cleared before resubscription, capability/quota flags reset, the
    /// per-epoch slot watermark restarts at 0, and the ingest side marks the new
    /// stream epoch (generation bump + stale account snapshot eviction, pool
    /// cache preserved).
    fn reset_connection_state(&self, idx: usize, ingest: &IngestClient) {
        self.ws_backoffs.write().remove(&idx);
        self.req_to_account.write().clear();
        self.sub_to_account.write().clear();
        arb_metrics::set_ws_subscriptions_active(0);
        self.pending_dynamic_subscriptions.lock().clear();
        self.account_sub_limited.store(false, Ordering::Release);
        self.endpoint_unsupported.store(false, Ordering::Release);
        self.probe_confirmed.store(false, Ordering::Release);

        self.is_connected.store(true, Ordering::Release);
        self.latest_slot.store(0, Ordering::Release);
        ingest.mark_helius_connected();
    }

    fn record_connect_backoff(&self, idx: usize, err_text: &str) {
        let lower = err_text.to_ascii_lowercase();
        let secs = if lower.contains("429") || lower.contains("too many") {
            60
        } else if lower.contains("400")
            || lower.contains("401")
            || lower.contains("403")
            || lower.contains("405")
        {
            120
        } else {
            5
        };
        self.ws_backoffs
            .write()
            .insert(idx, Instant::now() + Duration::from_secs(secs));
    }

    fn build_account_sub_frame(&self, req_id: u64, acc: Pubkey) -> Message {
        self.req_to_account.write().insert(req_id, acc);
        let commitment = std::env::var("WS_COMMITMENT")
            .unwrap_or_else(|_| arb_constants::DEFAULT_WS_COMMITMENT.to_string());
        let encoding = std::env::var("WS_ENCODING")
            .unwrap_or_else(|_| arb_constants::DEFAULT_WS_ENCODING.to_string());

        let acc_sub = json!({
            "jsonrpc": "2.0",
            "id": req_id,
            "method": "accountSubscribe",
            "params": [
                acc.to_string(),
                {
                    "encoding": encoding,
                    "commitment": commitment
                }
            ]
        });
        Message::Text(acc_sub.to_string())
    }

    /// Run the persistent WebSocket ingestion stream in the background,
    /// directly driving `IngestClient` slots, snapshots, and `MarketStateCache`.
    pub async fn start_stream(
        self: Arc<Self>,
        ingest: Arc<IngestClient>,
        cancel: CancellationToken,
    ) {
        if self.ws_urls.is_empty() {
            warn!("No WebSocket endpoints configured; skipping WS worker");
            return;
        }
        info!(
            "Starting WebSocket slot & account stream worker (endpoints: {})",
            self.ws_urls.len()
        );
        self.seed_initial_accounts(&ingest).await;

        // Event-driven derived-account hydration worker: wakes immediately on
        // `hydration_notify` when a WS pool update hits an unhydrated active
        // tick/bin array (with a 5s fallback sweep only).
        {
            let hydration_client = Arc::clone(&self);
            let hydration_ingest = Arc::clone(&ingest);
            let hydration_cancel = cancel.clone();
            let hydration_notify = Arc::clone(&self.derived_hydration_notify);
            tokio::spawn(async move {
                let mut fallback_sweep = tokio::time::interval(Duration::from_secs(5));
                fallback_sweep.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    tokio::select! {
                        _ = hydration_cancel.cancelled() => break,
                        _ = hydration_notify.notified() => {
                            // Coalesce bursts from multiple pool updates into one drain.
                            tokio::select! {
                                _ = hydration_cancel.cancelled() => break,
                                _ = sleep(Duration::from_millis(50)) => {}
                            }
                            hydration_client
                                .hydrate_pending_derived(&hydration_ingest)
                                .await;
                        }
                        _ = fallback_sweep.tick() => {
                            hydration_client
                                .hydrate_pending_derived(&hydration_ingest)
                                .await;
                        }
                    }
                }
            });
        }

        let mut attempt = 0u32;
        let mut consecutive_failures = 0usize;

        while !cancel.is_cancelled() {
            attempt = attempt.saturating_add(1);
            let idx = self.select_active_endpoint_index();
            let url_str = self.ws_urls[idx].clone();
            let safe_url = Self::redact_url(&url_str);
            let endpoint_num = idx + 1;
            let total_endpoints = self.ws_urls.len();

            let connect_result =
                match tokio::time::timeout(WS_CONNECT_TIMEOUT, connect_async(&url_str)).await {
                    Ok(Ok(connection)) => Ok(connection),
                    Ok(Err(error)) => Err(Self::classify_connect_error(&error.to_string())),
                    Err(_) => Err("websocket connection timeout"),
                };

            match connect_result {
                Ok((mut ws_stream, _)) => {
                    self.reset_connection_state(idx, &ingest);
                    info!(
                        "⚡ WebSocket [{endpoint_num}/{total_endpoints}] connected to mainnet via {safe_url}"
                    );
                    info!(target: "arb_events", "WEBSOCKET_CONNECTED: endpoint={safe_url}");

                    // 1. Subscribe to real-time slot notifications (capability-gated:
                    //    skipped when WS_SLOT_SUBSCRIBE=0 or when this provider
                    //    rejected the slot subscription before). Slot freshness
                    //    still advances via accountNotification context slots.
                    if self.slot_subscribe_enabled.load(Ordering::Acquire) {
                        let slot_sub = json!({
                            "jsonrpc": "2.0",
                            "id": 1,
                            "method": "slotSubscribe"
                        });
                        if let Err(e) = ws_stream.send(Message::Text(slot_sub.to_string())).await {
                            warn!(
                                "WebSocket [{endpoint_num}/{total_endpoints}] failed to send slotSubscribe: {e}"
                            );
                        } else {
                            debug!(
                                "WebSocket [{endpoint_num}/{total_endpoints}] subscribed to slotSubscribe"
                            );
                        }
                    } else {
                        debug!(
                            "WebSocket [{endpoint_num}/{total_endpoints}] slotSubscribe disabled (config/capability); slot freshness via account context slots"
                        );
                    }

                    // 2. Probe first priority account; queue the rest up to MAX_WS_ACCOUNT_SUBS
                    //    to avoid spamming error frames against single-sub peers (e.g. RPCFast).
                    let mut pending_subs: VecDeque<Pubkey> = {
                        let order = self.tracked_order.read();
                        let map = self.tracked_accounts.read();
                        let raw: Vec<Pubkey> = if order.is_empty() {
                            map.keys().copied().collect()
                        } else {
                            order
                                .iter()
                                .filter(|k| map.contains_key(k))
                                .copied()
                                .collect()
                        };
                        // Subscription priority: accounts of pools paired with a
                        // base anchor (WSOL) first. `find_base_cycles` only emits
                        // opportunities for cycles that start and end at a base
                        // anchor, so under a tight subscription budget every slot
                        // spent on a pool that cannot close a WSOL cycle is a slot
                        // taken from one that can. Stable sort keeps discovery
                        // order inside each tier.
                        let mut raw = raw;
                        let tracked_count = raw.len();
                        raw.sort_by_key(|acc| self.account_wsol_linkage(acc));
                        let subscription_limit = max_ws_account_subs();
                        let selected: VecDeque<Pubkey> =
                            raw.into_iter().take(subscription_limit).collect();
                        let deferred_count = tracked_count.saturating_sub(selected.len());
                        if deferred_count > 0 {
                            warn!(
                                tracked_accounts = tracked_count,
                                selected_accounts = selected.len(),
                                deferred_accounts = deferred_count,
                                subscription_limit,
                                "WebSocket subscription budget leaves tracked accounts without live updates; freshness gates remain authoritative"
                            );
                        } else {
                            info!(
                                tracked_accounts = tracked_count,
                                selected_accounts = selected.len(),
                                subscription_limit,
                                "WebSocket initial account subscription coverage"
                            );
                        }
                        selected
                    };
                    // Remember every account included in this connection's initial
                    // snapshot. A concurrent registration can also enter the dynamic
                    // queue; this set prevents that race from creating duplicate subs.
                    let mut queued_or_subscribed: std::collections::HashSet<Pubkey> =
                        pending_subs.iter().copied().collect();
                    let mut next_req_id = 100u64;
                    let probe_sent_at = if let Some(first_acc) = pending_subs.pop_front() {
                        let frame = self.build_account_sub_frame(next_req_id, first_acc);
                        next_req_id += 1;
                        let _ = ws_stream.send(frame).await;
                        Some(Instant::now())
                    } else {
                        None
                    };

                    // 3. Ingestion Loop with 15s Heartbeat Ping and Resilient Streaming
                    let mut received_account_update = false;
                    let mut ping_interval = tokio::time::interval(Duration::from_secs(15));
                    ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

                    loop {
                        // Newly discovered pools are subscribed on the existing
                        // socket once the provider has acknowledged the probe.
                        // The account cap counts requested/active accounts, not
                        // duplicate request+subscription map entries.
                        if self.probe_confirmed.load(Ordering::Acquire)
                            && !self.account_sub_limited.load(Ordering::Acquire)
                        {
                            let requested = self.req_to_account.read().len();
                            let active = self.sub_to_account.read().len();
                            let reserved = requested.max(active).saturating_add(pending_subs.len());
                            let remaining = max_ws_account_subs().saturating_sub(reserved);
                            if remaining > 0 {
                                let mut to_send = Vec::new();
                                {
                                    let mut pending = self.pending_dynamic_subscriptions.lock();
                                    while to_send.len() < remaining {
                                        let Some(acc) = pending.pop_front() else {
                                            break;
                                        };
                                        if queued_or_subscribed.insert(acc) {
                                            to_send.push(acc);
                                        }
                                    }
                                }
                                let mut send_failed = false;
                                for acc in to_send {
                                    let frame = self.build_account_sub_frame(next_req_id, acc);
                                    next_req_id = next_req_id.saturating_add(1);
                                    if let Err(error) = ws_stream.send(frame).await {
                                        warn!(
                                            "WebSocket dynamic accountSubscribe send failed: {error}; reconnecting"
                                        );
                                        send_failed = true;
                                        break;
                                    }
                                }
                                if send_failed {
                                    break;
                                }
                            }
                        }

                        if self.account_sub_limited.load(Ordering::Acquire) {
                            pending_subs.clear();
                        } else if !pending_subs.is_empty()
                            && self.probe_confirmed.load(Ordering::Acquire)
                        {
                            while let Some(acc) = pending_subs.pop_front() {
                                let frame = self.build_account_sub_frame(next_req_id, acc);
                                next_req_id += 1;
                                let _ = ws_stream.send(frame).await;
                            }
                            arb_metrics::set_ws_subscriptions_active(
                                self.sub_to_account.read().len() as u64,
                            );
                        }

                        // Subscription-ack timeout: if the probe subscription was
                        // never acknowledged while the transport itself is alive
                        // (pings/pongs still flowing), this connection is blind —
                        // fail over instead of sitting on it forever.
                        if let Some(sent_at) = probe_sent_at
                            && !self.probe_confirmed.load(Ordering::Acquire)
                            && sent_at.elapsed() > Duration::from_secs(15)
                        {
                            warn!(
                                "WebSocket [{endpoint_num}/{total_endpoints}] {safe_url} probe subscription unacknowledged for 15s (transport alive); failing over"
                            );
                            break;
                        }

                        tokio::select! {
                            _ = cancel.cancelled() => {
                                self.is_connected.store(false, Ordering::Release);
                                return;
                            }
                            _ = self.dynamic_subscribe_notify.notified() => {
                                // The next loop iteration drains the queue. Keeping
                                // the send outside select avoids borrowing the socket
                                // concurrently with its receive future.
                            }
                            _ = ping_interval.tick() => {
                                if let Err(e) = ws_stream.send(Message::Ping(vec![])).await {
                                    warn!("WebSocket [{endpoint_num}/{total_endpoints}] failed to send ping: {e}; failing over");
                                    break;
                                }
                            }
                            recv_res = tokio::time::timeout(Duration::from_secs(30), ws_stream.next()) => {
                                let msg_opt = match recv_res {
                                    Ok(opt) => opt,
                                    Err(_) => {
                                        warn!("WebSocket [{endpoint_num}/{total_endpoints}] {safe_url} stalled (no frames or pong for 30s); failing over");
                                        break;
                                    }
                                };
                                let Some(msg_res) = msg_opt else {
                                    if !received_account_update {
                                        self.record_connect_backoff(idx, "stream ended immediately by peer");
                                    }
                                    warn!("WebSocket stream ended by peer ([{endpoint_num}/{total_endpoints}] {safe_url}); failing over");
                                    break;
                                };

                                match msg_res {
                                    Ok(Message::Text(text)) => {
                                        if let Ok(v) = serde_json::from_str::<Value>(&text) {
                                            match self.handle_message(&v, &ingest) {
                                                Ok(got_activity) => {
                                                    if got_activity {
                                                        attempt = 0;
                                                        consecutive_failures = 0;
                                                        received_account_update = true;
                                                    }
                                                }
                                                Err(err_msg) => {
                                                    warn!(
                                                        "WebSocket [{endpoint_num}/{total_endpoints}] {safe_url} subscription warning: {err_msg}"
                                                    );
                                                    // Soft warning: do NOT break or fail over on non-fatal subscription rejections
                                                }
                                            }
                                            // Capability failure (e.g. -32601 "Method 'accountSubscribe'
                                            // not found"): this endpoint can never deliver account
                                            // updates. Back it off and fail over instead of sitting on a
                                            // blind connection forever.
                                            if self.endpoint_unsupported.load(Ordering::Acquire) {
                                                warn!(
                                                    "WebSocket [{endpoint_num}/{total_endpoints}] {safe_url} does not support account subscriptions; backing off and failing over"
                                                );
                                                self.record_connect_backoff(idx, "accountSubscribe unsupported");
                                                break;
                                            }
                                        }
                                    }
                                    Ok(Message::Ping(payload)) => {
                                        let _ = ws_stream.send(Message::Pong(payload)).await;
                                    }
                                    Ok(Message::Pong(_)) => {
                                        // Heartbeat acknowledgement received
                                    }
                                    Ok(Message::Close(_)) => {
                                        if !received_account_update {
                                            self.record_connect_backoff(idx, "closed immediately by peer");
                                        }
                                        warn!("WebSocket closed by server ([{endpoint_num}/{total_endpoints}] {safe_url}); failing over");
                                        break;
                                    }
                                    Err(e) => {
                                        warn!("WebSocket read error ([{endpoint_num}/{total_endpoints}] {safe_url}): {e}; failing over");
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }

                    let next_idx = self.rotate_endpoint();
                    arb_metrics::inc_ws_reconnects();
                    if !received_account_update {
                        consecutive_failures += 1;
                    } else {
                        consecutive_failures = 0;
                    }
                    info!(
                        "🔄 Rotating WebSocket to endpoint [{}/{}]",
                        next_idx + 1,
                        total_endpoints
                    );
                }
                Err(error_label) => {
                    self.is_connected.store(false, Ordering::Release);
                    self.record_connect_backoff(idx, error_label);
                    let next_idx = self.rotate_endpoint();
                    arb_metrics::inc_ws_reconnects();
                    consecutive_failures += 1;
                    warn!(
                        "WebSocket [{endpoint_num}/{total_endpoints}] connect failed ({error_label}); rotating to [{}/{}] (attempt={attempt})",
                        next_idx + 1,
                        total_endpoints
                    );
                }
            }

            self.is_connected.store(false, Ordering::Release);
            arb_metrics::set_ws_subscriptions_active(0);
            self.latest_slot.store(0, Ordering::Release);
            ingest.mark_helius_disconnected();

            // Resilient WS connection & cooldown:
            // Apply a 2.5s to 3.0s cooldown when rotating or reconnecting to prevent
            // hitting provider limits ("Concurrent websocket subscription limit reached: 1")
            // due to lingering dead TCP/WS sessions on the peer side.
            // A ±25% jitter overlay desynchronizes reconnect storms when several
            // endpoints fail at once (thundering herd) instead of retrying in lockstep.
            let base_backoff_ms: u64 = if consecutive_failures < total_endpoints {
                2500
            } else {
                3000
            };
            let jitter_span = (base_backoff_ms / 4).max(1);
            let entropy = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos() as u64)
                .unwrap_or(0);
            let jitter_ms = entropy % (jitter_span * 2);
            let backoff_ms = base_backoff_ms - jitter_span / 2 + jitter_ms;
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = sleep(Duration::from_millis(backoff_ms)) => {}
            }
        }
    }

    /// Queue a pool for targeted dependency hydration (DAMM v2 Pool/Vault/Mint/Clock, tick arrays /
    /// bin arrays). Called from the WS update path when coverage fails; the
    /// hydration worker fetches ONLY the needed accounts (event-driven, not
    /// the blanket 750ms poll this replaces).
    /// Throttled to one attempt per pool per 1.5s.
    fn queue_derived_hydration(&self, pool_id: PoolId, slot: u64) {
        let mut covered = self.derived_covered.lock();
        if let Some((_, last)) = covered.get(&pool_id)
            && last.elapsed() < Duration::from_millis(1500)
        {
            return;
        }
        covered.insert(pool_id, (Vec::new(), Instant::now()));
        drop(covered);
        self.pending_derived.lock().insert(pool_id, slot);
        self.derived_hydration_notify.notify_one();
    }

    /// Drain the hydration queue: fetch each queued pool's needed derived
    /// accounts (batched), decode, and apply the refreshed pool state.
    /// Fail-closed: a pool that cannot be decoded or fetched stays unhydrated
    /// and its WS updates keep being rejected until a later attempt succeeds.
    pub async fn hydrate_pending_derived(&self, ingest: &IngestClient) {
        let batch: Vec<(PoolId, u64)> = {
            let mut q = self.pending_derived.lock();
            if q.is_empty() {
                return;
            }
            q.drain().collect()
        };
        let now_ms = u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
        )
        .unwrap_or(u64::MAX);

        for (pool_id, trigger_slot) in batch {
            let Some(reg) = self.registered_pools.read().get(&pool_id).cloned() else {
                continue;
            };
            let Some(snap) = ingest.account_store().get(&pool_id.0) else {
                continue;
            };
            let pool_bytes: &[u8] = &snap.data;

            match reg.protocol {
                DexProtocol::MeteoraDlmm => {
                    let Some(d) = decode_dlmm_state(pool_bytes) else {
                        continue;
                    };
                    let mut keys = crate::registry::dlmm_bin_array_pdas(
                        &pool_id.0,
                        d.active_bin_id,
                        d.bin_step_bps,
                    );
                    let ext = crate::registry::bitmap_extension_pda(&pool_id.0);
                    keys.push(ext);
                    // Fetch the pool and every derived TickArray in ONE RPC batch.
                    // This gives the quote a common context slot and lets us prove
                    // the pool bytes used by the local snapshot are unchanged at it.
                    let mut refs: Vec<String> = Vec::with_capacity(keys.len() + 1);
                    refs.push(pool_id.0.to_string());
                    refs.extend(keys.iter().map(|key| key.to_string()));
                    let ref_strs: Vec<&str> = refs.iter().map(|s| s.as_str()).collect();
                    let Ok((accounts, hydration_context_slot)) = self
                        .rpc_pool
                        .get_multiple_accounts_with_slot(&ref_strs)
                        .await
                    else {
                        continue;
                    };
                    // Never promote derived accounts using the trigger slot or
                    // an older pool snapshot. The returned RPC context is the
                    // only slot that proves when this batch was read.
                    let Some(slot) =
                        validated_hydration_slot(hydration_context_slot, snap.slot, trigger_slot)
                    else {
                        continue;
                    };
                    let mut bins = Vec::new();
                    for (acc, key) in accounts.into_iter().zip(keys.iter()) {
                        let Some(acc) = acc else { continue };
                        let Some(raw) = acc
                            .get("data")
                            .and_then(|d| d.get(0))
                            .and_then(|s| s.as_str())
                        else {
                            continue;
                        };
                        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(raw)
                        else {
                            continue;
                        };
                        if *key == ext {
                            continue;
                        }
                        bins.extend(crate::registry::decode_bin_array(&bytes));
                    }
                    if bins.is_empty() {
                        continue;
                    }
                    let source_snapshot_unchanged = ingest
                        .account_store()
                        .get(&pool_id.0)
                        .is_some_and(|current| {
                            current.owner == snap.owner
                                && current.slot == snap.slot
                                && current.write_version == snap.write_version
                                && current.provider_id == snap.provider_id
                                && current.bank_id == snap.bank_id
                                && current.generation == snap.generation
                                && current.data.as_ref() == snap.data.as_ref()
                        });
                    if !source_snapshot_unchanged {
                        continue;
                    }
                    if let Some(mut state) = ingest.cache().get_pool(&pool_id)
                        && slot >= state.last_update_slot
                        && let Some(dlmm) = state.dlmm.as_mut()
                        && pool_bytes.len() >= 152
                    {
                        dlmm.bins = bins;
                        if let Some(fee_bps) = decode_dlmm_fee_bps(dlmm) {
                            let token_x =
                                Pubkey::new_from_array(pool_bytes[88..120].try_into().unwrap());
                            let token_y =
                                Pubkey::new_from_array(pool_bytes[120..152].try_into().unwrap());
                            state.base_mint = MintId(token_x);
                            state.quote_mint = MintId(token_y);
                            if let Some((r_in, r_out)) =
                                compute_dlmm_reserves(dlmm, pool_bytes, &token_x, &token_y)
                                && r_in > 0
                                && r_out > 0
                            {
                                state.fee_numerator = u64::from(fee_bps);
                                state.fee_denominator = 10_000;
                                state.reserve_in = r_in;
                                state.reserve_out = r_out;
                                state.slot = Slot(slot);
                                state.last_update_slot = slot;
                                state.last_update_ms = now_ms;
                                if matches!(ingest.apply_pool_update(state), Ok(true)) {
                                    let covered = crate::registry::dlmm_bin_array_indices(
                                        d.active_bin_id,
                                        d.bin_step_bps,
                                    );
                                    self.derived_covered
                                        .lock()
                                        .insert(pool_id, (covered, Instant::now()));
                                    if let Ok(dlmm_owner) =
                                        Pubkey::from_str(arb_constants::METEORA_DLMM_PROGRAM_ID)
                                        && self.tracked_accounts.read().len()
                                            < max_ws_account_subs()
                                    {
                                        for k in &keys {
                                            if *k != ext {
                                                self.track_account(*k, dlmm_owner);
                                            }
                                        }
                                    }
                                    debug!(
                                        "hydration: DLMM {} rehydrated bins at RPC context slot {}",
                                        pool_id.0, slot
                                    );
                                }
                            }
                        }
                    }
                }
                DexProtocol::OrcaWhirlpool | DexProtocol::RaydiumClmm => {
                    let decoded = if reg.protocol == DexProtocol::OrcaWhirlpool {
                        decode_clmm_state(pool_bytes)
                    } else {
                        decode_raydium_clmm_state(pool_bytes, reg.fee_bps)
                    };
                    let Some(c) = decoded else {
                        continue;
                    };
                    let (keys, starts) = if reg.protocol == DexProtocol::OrcaWhirlpool {
                        crate::registry::clmm_tick_array_pdas(
                            &pool_id,
                            c.tick_current,
                            c.tick_spacing,
                        )
                    } else {
                        crate::registry::raydium_clmm_tick_array_pdas(
                            &pool_id,
                            c.tick_current,
                            c.tick_spacing,
                        )
                    };
                    let refs: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
                    let ref_strs: Vec<&str> = refs.iter().map(|s| s.as_str()).collect();
                    let Ok((accounts, hydration_context_slot)) = self
                        .rpc_pool
                        .get_multiple_accounts_with_slot(&ref_strs)
                        .await
                    else {
                        continue;
                    };
                    // Never promote derived accounts using the trigger slot or
                    // an older pool snapshot. The returned RPC context is the
                    // only slot that proves when this batch was read.
                    let Some(slot) =
                        validated_hydration_slot(hydration_context_slot, snap.slot, trigger_slot)
                    else {
                        continue;
                    };
                    let Some(pool_account) = accounts.first().and_then(Option::as_ref) else {
                        continue;
                    };
                    let expected_program = if reg.protocol == DexProtocol::OrcaWhirlpool {
                        arb_constants::ORCA_WHIRLPOOL_PROGRAM_ID
                    } else {
                        arb_constants::RAYDIUM_CLMM_PROGRAM_ID
                    };
                    let pool_owner_ok = pool_account
                        .get("owner")
                        .and_then(serde_json::Value::as_str)
                        == Some(expected_program);
                    let pool_bytes_match = pool_account
                        .get("data")
                        .and_then(|data| data.get(0))
                        .and_then(serde_json::Value::as_str)
                        .and_then(|raw| base64::engine::general_purpose::STANDARD.decode(raw).ok())
                        .is_some_and(|bytes| bytes.as_slice() == snap.data.as_ref());
                    if !pool_owner_ok || !pool_bytes_match {
                        continue;
                    }

                    let mut arrays = Vec::new();
                    for ((acc, account_key), start_tick) in accounts
                        .into_iter()
                        .skip(1)
                        .zip(keys.iter())
                        .zip(starts.iter())
                    {
                        let Some(acc) = acc else { continue };
                        let owner_ok = acc.get("owner").and_then(serde_json::Value::as_str)
                            == Some(expected_program);
                        if !owner_ok {
                            continue;
                        }
                        let Some(raw) = acc
                            .get("data")
                            .and_then(|data| data.get(0))
                            .and_then(serde_json::Value::as_str)
                        else {
                            continue;
                        };
                        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(raw)
                        else {
                            continue;
                        };
                        let parsed = if reg.protocol == DexProtocol::OrcaWhirlpool {
                            arb_math::clmm::parse_tick_array_account(&bytes, c.tick_spacing)
                                .ok()
                                .map(|f| (f.start_tick_index, f.ticks))
                        } else {
                            arb_dex::RaydiumClmmPool::parse_tick_array_state(&bytes)
                                .ok()
                                .map(|f| (f.start_tick_index, f.ticks))
                        };
                        if let Some((start, ticks)) = parsed
                            && start == *start_tick
                        {
                            arrays.push(arb_types::ClmmTickArray {
                                start_tick_index: start,
                                account_pubkey: Some(account_key.to_bytes()),
                                account_slot: Some(slot),
                                ticks: ticks
                                    .into_iter()
                                    .filter(|tick| tick.initialized)
                                    .map(|tick| arb_types::ClmmTick {
                                        tick_index: tick.tick_index,
                                        initialized: true,
                                        liquidity_net: tick.liquidity_net,
                                        limit_order_unfilled: tick.limit_order_unfilled,
                                    })
                                    .collect(),
                            });
                        }
                    }
                    if arrays.is_empty() {
                        continue;
                    }
                    let source_snapshot_unchanged = ingest
                        .account_store()
                        .get(&pool_id.0)
                        .is_some_and(|current| {
                            current.owner == snap.owner
                                && current.slot == snap.slot
                                && current.write_version == snap.write_version
                                && current.provider_id == snap.provider_id
                                && current.bank_id == snap.bank_id
                                && current.generation == snap.generation
                                && current.data.as_ref() == snap.data.as_ref()
                        });
                    if !source_snapshot_unchanged {
                        continue;
                    }
                    if let Some(mut state) = ingest.cache().get_pool(&pool_id)
                        && slot >= state.last_update_slot
                        && let Some(clmm) = state.clmm.as_mut()
                    {
                        clmm.tick_arrays = arrays;
                        let (r_in, r_out) = compute_clmm_virtual_reserves(clmm);
                        if r_in > 0 && r_out > 0 {
                            state.reserve_in = r_in;
                            state.reserve_out = r_out;
                            state.slot = Slot(slot);
                            state.last_update_slot = slot;
                            state.last_update_ms = now_ms;
                            let n_arrays = clmm.tick_arrays.len();
                            if matches!(ingest.apply_pool_update(state), Ok(true)) {
                                self.derived_covered.lock().insert(
                                    pool_id,
                                    (
                                        starts.iter().map(|v| i64::from(*v)).collect(),
                                        Instant::now(),
                                    ),
                                );
                                let array_owner_str = if reg.protocol == DexProtocol::OrcaWhirlpool
                                {
                                    arb_constants::ORCA_WHIRLPOOL_PROGRAM_ID
                                } else {
                                    arb_constants::RAYDIUM_CLMM_PROGRAM_ID
                                };
                                if let Ok(array_owner) = Pubkey::from_str(array_owner_str)
                                    && self.tracked_accounts.read().len() < max_ws_account_subs()
                                {
                                    for k in &keys {
                                        self.track_account(*k, array_owner);
                                    }
                                }
                                debug!(
                                    "hydration: {:?} {} rehydrated {} tick arrays at RPC context slot {}",
                                    reg.protocol, pool_id.0, n_arrays, slot
                                );
                            }
                        }
                    }
                }
                DexProtocol::MeteoraDammV2 => {
                    // Refresh all Pool/Vault/Mint/Clock dependencies in one RPC
                    // context whenever the pool account notification changes.
                    let Ok(pool_decoder) =
                        arb_dex::MeteoraDammV2Pool::from_account(pool_id, pool_bytes)
                    else {
                        continue;
                    };
                    let Ok(clock_key) =
                        Pubkey::from_str("SysvarC1ock11111111111111111111111111111111")
                    else {
                        continue;
                    };
                    let keys = [
                        pool_id.0,
                        pool_decoder.token_a_vault,
                        pool_decoder.token_b_vault,
                        pool_decoder.token_a_mint.0,
                        pool_decoder.token_b_mint.0,
                        clock_key,
                    ];
                    let key_strings: Vec<String> = keys.iter().map(ToString::to_string).collect();
                    let key_refs: Vec<&str> = key_strings.iter().map(String::as_str).collect();
                    let Ok((accounts, rpc_context_slot)) = self
                        .rpc_pool
                        .get_multiple_accounts_with_slot(&key_refs)
                        .await
                    else {
                        continue;
                    };
                    let Some(slot) =
                        validated_hydration_slot(rpc_context_slot, snap.slot, trigger_slot)
                    else {
                        continue;
                    };
                    let source_snapshot_unchanged = ingest
                        .account_store()
                        .get(&pool_id.0)
                        .is_some_and(|current| {
                            current.owner == snap.owner
                                && current.slot == snap.slot
                                && current.write_version == snap.write_version
                                && current.provider_id == snap.provider_id
                                && current.bank_id == snap.bank_id
                                && current.generation == snap.generation
                                && current.data.as_ref() == snap.data.as_ref()
                        });
                    if !source_snapshot_unchanged {
                        continue;
                    }
                    let decoded_accounts: Option<Vec<(Vec<u8>, Pubkey)>> = accounts
                        .iter()
                        .map(|account| {
                            let account = account.as_ref()?;
                            let owner = Pubkey::from_str(account.get("owner")?.as_str()?).ok()?;
                            let raw = account.get("data")?.get(0)?.as_str()?;
                            let bytes =
                                base64::engine::general_purpose::STANDARD.decode(raw).ok()?;
                            Some((bytes, owner))
                        })
                        .collect();
                    let Some(decoded) = decoded_accounts.filter(|accounts| accounts.len() == 6)
                    else {
                        continue;
                    };
                    let Some(mut refreshed) = crate::hydration::decode_damm_v2_coherent_pool_state(
                        pool_id,
                        slot,
                        &decoded[0].0,
                        decoded[0].1,
                        keys[1],
                        keys[2],
                        keys[3],
                        keys[4],
                        &decoded[1].0,
                        decoded[1].1,
                        &decoded[2].0,
                        decoded[2].1,
                        &decoded[3].0,
                        decoded[3].1,
                        &decoded[4].0,
                        decoded[4].1,
                        &decoded[5].0,
                        decoded[5].1,
                    ) else {
                        continue;
                    };
                    let Some(previous) = ingest.cache().get_pool(&pool_id) else {
                        continue;
                    };
                    if slot < previous.last_update_slot {
                        continue;
                    }
                    refreshed.write_version = if slot > previous.last_update_slot {
                        1
                    } else {
                        previous.write_version.saturating_add(1)
                    };
                    refreshed.last_update_ms = now_ms;
                    if matches!(ingest.apply_pool_update(refreshed), Ok(true)) {
                        self.derived_covered
                            .lock()
                            .insert(pool_id, (Vec::new(), Instant::now()));
                        debug!(
                            "hydration: DAMM v2 {} refreshed Pool/Vault/Mint/Clock at coherent RPC slot {}",
                            pool_id.0, slot
                        );
                    }
                }
                _ => {}
            }
        }
    }

    fn apply_live_pool_update_from_ws(
        &self,
        pk: Pubkey,
        bytes: &[u8],
        slot: u64,
        ingest: &IngestClient,
    ) {
        // Enforce strict slot integrity: never upgrade an old notification to a newer slot.
        // Drops updates from dead slots or zero slots immediately.
        if slot == 0 || ingest.slot_is_dead(slot) {
            return;
        }
        let update_slot = Slot(slot);
        let now_ms = u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
        )
        .unwrap_or(u64::MAX);

        // Case 1: Raydium V4 / Raydium CPMM SPL token vault update
        let vault_matches = self.vault_to_pools.read().get(&pk).cloned();
        if let Some(targets) = vault_matches {
            let Some(amount) = decode_spl_token_amount_bytes(bytes) else {
                return;
            };
            if amount == 0 {
                return;
            }
            for (pool_id, is_base) in targets {
                let Some(mut state) = ingest.cache().get_pool(&pool_id) else {
                    continue;
                };
                // Reject out-of-order notifications where notification slot is older than pool's known slot
                if slot < state.last_update_slot {
                    continue;
                }
                if is_base {
                    state.reserve_in = u128::from(amount);
                } else {
                    state.reserve_out = u128::from(amount);
                }
                if state.reserve_in == 0 || state.reserve_out == 0 {
                    continue;
                }
                state.slot = update_slot;
                state.write_version = if slot > state.last_update_slot {
                    1
                } else {
                    state.write_version.saturating_add(1)
                };
                state.last_update_slot = slot;
                state.last_update_ms = now_ms;
                let _ = ingest.apply_pool_update(state);
            }
            return;
        }

        // Case 2: Orca Whirlpool / Meteora DLMM pool account update
        let pool_id = PoolId(pk);
        let reg_opt = self.registered_pools.read().get(&pool_id).cloned();
        let Some(reg) = reg_opt else {
            return;
        };
        let Some(mut state) = ingest.cache().get_pool(&pool_id) else {
            return;
        };
        // Reject out-of-order notifications
        if slot < state.last_update_slot {
            return;
        }

        match reg.protocol {
            DexProtocol::OrcaWhirlpool => {
                let Some(mut clmm) = decode_clmm_state(bytes) else {
                    return;
                };
                let Some(fee_bps) = decode_orca_fee_bps(bytes) else {
                    return;
                };
                let existing_ticks = state
                    .clmm
                    .as_ref()
                    .map(|c| c.tick_arrays.clone())
                    .unwrap_or_default();
                if existing_ticks.is_empty()
                    || !clmm_tick_array_covers_tick(
                        &existing_ticks,
                        clmm.tick_current,
                        clmm.tick_spacing,
                    )
                {
                    self.queue_derived_hydration(pool_id, slot); // fail closed + event-driven hydration
                    return;
                }
                clmm.tick_arrays = existing_ticks;
                let (r_in, r_out) = compute_clmm_virtual_reserves(&clmm);
                if r_in == 0 || r_out == 0 {
                    return;
                }
                state.clmm = Some(clmm);
                state.fee_numerator = u64::from(fee_bps);
                state.fee_denominator = 10_000;
                state.reserve_in = r_in;
                state.reserve_out = r_out;
                state.slot = update_slot;
                state.write_version = if slot > state.last_update_slot {
                    1
                } else {
                    state.write_version.saturating_add(1)
                };
                state.last_update_slot = slot;
                state.last_update_ms = now_ms;
                let _ = ingest.apply_pool_update(state);
            }
            DexProtocol::MeteoraDlmm => {
                let Some(mut dlmm) = decode_dlmm_state(bytes) else {
                    return;
                };
                let existing_bins = state
                    .dlmm
                    .as_ref()
                    .map(|d| d.bins.clone())
                    .unwrap_or_default();
                if !existing_bins.iter().any(|b| b.bin_id == dlmm.active_bin_id) {
                    self.queue_derived_hydration(pool_id, slot); // fail closed + event-driven hydration
                    return;
                }
                dlmm.bins = existing_bins;
                let Some(fee_bps) = decode_dlmm_fee_bps(&dlmm) else {
                    return;
                };
                // Canonical orientation from the on-chain LbPair (token_x
                // @88..120, token_y @120..152): discovery base/quote comes from
                // DexScreener and may be inverted, which flips the swap
                // direction (`a_to_b = pool_base == from_mint`) and produced
                // 156x phantom quotes on mainnet (see registry.rs).
                if bytes.len() >= 152 {
                    let token_x = Pubkey::new_from_array(bytes[88..120].try_into().unwrap());
                    let token_y = Pubkey::new_from_array(bytes[120..152].try_into().unwrap());
                    state.base_mint = MintId(token_x);
                    state.quote_mint = MintId(token_y);
                    let Some((r_in, r_out)) =
                        compute_dlmm_reserves(&dlmm, bytes, &token_x, &token_y)
                    else {
                        return;
                    };
                    if r_in == 0 || r_out == 0 {
                        return;
                    }
                    state.dlmm = Some(dlmm);
                    state.fee_numerator = u64::from(fee_bps);
                    state.fee_denominator = 10_000;
                    state.reserve_in = r_in;
                    state.reserve_out = r_out;
                    state.slot = update_slot;
                    state.write_version = if slot > state.last_update_slot {
                        1
                    } else {
                        state.write_version.saturating_add(1)
                    };
                    state.last_update_slot = slot;
                    state.last_update_ms = now_ms;
                    let _ = ingest.apply_pool_update(state);
                }
            }
            DexProtocol::RaydiumClmm => {
                // Do not promote catalog-fee CLMM state. The live AmmConfig and
                // dynamic-fee configuration are not part of this coherent WS snapshot.
            }
            DexProtocol::MeteoraDammV2 => {
                // Invalidate the previous immutable quote state immediately. The
                // graph must not rank against an old fee/price snapshot while the
                // same-context Pool/Vault/Mint/Clock refresh is in flight.
                state.is_active = false;
                if let Some(damm) = state.damm_v2.as_mut() {
                    damm.account_graph_verified = false;
                }
                state.slot = update_slot;
                state.write_version = if slot > state.last_update_slot {
                    1
                } else {
                    state.write_version.saturating_add(1)
                };
                state.last_update_slot = slot;
                state.last_update_ms = now_ms;
                let _ = ingest.apply_pool_update(state);
                self.queue_derived_hydration(pool_id, slot);
            }
            DexProtocol::PumpFun => {
                let Ok(pump_state) =
                    arb_dex::pump_fun::PumpFunBondingCurve::decode_account_data(bytes)
                else {
                    return;
                };
                if !pump_state.is_tradeable()
                    || pump_state.virtual_sol_reserves == 0
                    || pump_state.virtual_token_reserves == 0
                {
                    return;
                }
                state.reserve_in = u128::from(pump_state.virtual_sol_reserves);
                state.reserve_out = u128::from(pump_state.virtual_token_reserves);
                state.slot = update_slot;
                state.write_version = if slot > state.last_update_slot {
                    1
                } else {
                    state.write_version.saturating_add(1)
                };
                state.last_update_slot = slot;
                state.last_update_ms = now_ms;
                let _ = ingest.apply_pool_update(state);
            }
            _ => {}
        }
    }

    /// Process an incoming JSON-RPC WebSocket frame.
    /// Returns `Ok(true)` when an update (accountNotification or slotNotification) was received,
    /// `Ok(false)` for other valid frames, or `Err(warning)` for soft subscription rejections.
    pub fn handle_message(&self, v: &Value, ingest: &IngestClient) -> Result<bool, String> {
        if let Some(err_obj) = v.get("error") {
            let req_id = v.get("id").and_then(|i| i.as_u64());
            let code = err_obj.get("code").and_then(|c| c.as_i64());
            let err_msg = err_obj
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown JSON-RPC error");
            let lower = err_msg.to_ascii_lowercase();
            // Slot-subscription rejection (request id 1): this endpoint cannot
            // (or will not) stream slot notifications, but it may still deliver
            // account updates. Disable the slot sub for this client instead of
            // failing the endpoint — slot freshness falls back to the context
            // slot carried by every accountNotification.
            if req_id == Some(1) {
                self.slot_subscribe_enabled.store(false, Ordering::Release);
                warn!(
                    "WS: slotSubscribe rejected on this endpoint (slot subs disabled, accounts unaffected): {err_msg}"
                );
                return Ok(false);
            }
            // Capability failure: the endpoint does not implement the method at
            // all (e.g. Alchemy WS: -32601 "Method 'accountSubscribe' not found").
            // Retrying on this endpoint is pointless — it can NEVER deliver
            // account updates. Flag it so the stream loop tears down and fails
            // over with a backoff.
            if lower.contains("not found") || lower.contains("unsupported") || code == Some(-32601)
            {
                self.endpoint_unsupported.store(true, Ordering::Release);
            } else if lower.contains("limit") {
                // Transient quota/concurrency limit: stop queueing on this
                // connection but keep it (existing fail-closed behavior).
                self.account_sub_limited.store(true, Ordering::Release);
            }
            warn!("WS: subscription rejection warning (non-fatal) for id={req_id:?}: {err_msg}");
            return Ok(false);
        }

        // Handle subscription responses: {"jsonrpc":"2.0","result":sub_id,"id":req_id}
        if let (Some(req_id), Some(sub_id)) = (
            v.get("id").and_then(|i| i.as_u64()),
            v.get("result").and_then(|r| r.as_u64()),
        ) {
            if let Some(target_pk) = self.req_to_account.read().get(&req_id).copied() {
                self.sub_to_account.write().insert(sub_id, target_pk);
                arb_metrics::set_ws_subscriptions_active(self.sub_to_account.read().len() as u64);
                if req_id >= 100 {
                    self.probe_confirmed.store(true, Ordering::Release);
                }
                debug!("WS: subscription established: id={sub_id} -> {target_pk}");
            }
            return Ok(false);
        }

        if let Some(method) = v.get("method").and_then(|m| m.as_str()) {
            match method {
                "slotNotification" => {
                    if let Some(slot) = v.pointer("/params/result/slot").and_then(|s| s.as_u64()) {
                        let parent = v.pointer("/params/result/parent").and_then(|p| p.as_u64());
                        self.latest_slot.store(slot, Ordering::Release);
                        ingest.record_slot_update(slot, parent, slot_integrity::SLOT_PROCESSED);
                        ingest.record_helius_slot(slot);
                        ingest.mark_helius_connected();
                        self.slot_notify.notify_waiters();
                        return Ok(true);
                    }
                }
                "accountNotification" => {
                    let slot = v
                        .pointer("/params/result/context/slot")
                        .and_then(|s| s.as_u64())
                        .unwrap_or(0);
                    let sub_id = v.pointer("/params/subscription").and_then(|s| s.as_u64());
                    let target_pk =
                        sub_id.and_then(|sid| self.sub_to_account.read().get(&sid).copied());

                    if slot > 0 && !ingest.slot_is_dead(slot) {
                        self.latest_slot.fetch_max(slot, Ordering::AcqRel);
                        ingest.record_helius_slot(slot);
                    }

                    if let Some(pk) = target_pk {
                        if let Some(value) = v.pointer("/params/result/value") {
                            let owner_str = value
                                .get("owner")
                                .and_then(|o| o.as_str())
                                .unwrap_or_default();
                            let owner = Pubkey::from_str(owner_str).unwrap_or_default();

                            let data_bytes =
                                if let Some(arr) = value.get("data").and_then(|d| d.as_array()) {
                                    if let Some(b64_str) = arr.first().and_then(|s| s.as_str()) {
                                        base64::engine::general_purpose::STANDARD
                                            .decode(b64_str)
                                            .ok()
                                    } else {
                                        None
                                    }
                                } else {
                                    None
                                };

                            if let Some(bytes) = data_bytes {
                                let decode_start = Instant::now();
                                self.apply_live_pool_update_from_ws(pk, &bytes, slot, ingest);
                                let snap = AccountSnapshot {
                                    pubkey: pk,
                                    owner,
                                    slot,
                                    write_version: next_pool_write_version(),
                                    is_startup: false,
                                    provider_id: ingest.provider_id(),
                                    bank_id: None,
                                    generation: ingest.stream_generation(),
                                    data: Arc::new(bytes),
                                };
                                ingest.account_store().apply(snap);
                                arb_metrics::observe_state_processing_latency_us(
                                    decode_start.elapsed().as_micros() as f64,
                                );
                                debug!("WS: Applied live account update for {pk} at slot {slot}");
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_types::{MintId, PoolState};

    #[test]
    fn track_account_queues_only_new_accounts_after_connection() {
        let client = HeliusWsClient::new(vec!["wss://ws1.example.com".to_string()]);
        let owner = Pubkey::new_unique();
        let startup_account = Pubkey::new_unique();
        let dynamic_account = Pubkey::new_unique();

        client.track_account(startup_account, owner);
        assert!(client.pending_dynamic_subscriptions.lock().is_empty());

        client.is_connected.store(true, Ordering::Release);
        client.track_account(dynamic_account, owner);
        client.track_account(dynamic_account, owner);

        let pending = client.pending_dynamic_subscriptions.lock();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending.front().copied(), Some(dynamic_account));
        assert_eq!(
            client.tracked_accounts.read().get(&dynamic_account),
            Some(&owner)
        );
    }

    #[test]
    fn hydration_context_slot_never_promotes_zero_or_older_rpc_data() {
        assert_eq!(validated_hydration_slot(0, 10, 10), None);
        assert_eq!(validated_hydration_slot(9, 10, 8), None);
        assert_eq!(validated_hydration_slot(10, 9, 10), Some(10));
        assert_eq!(validated_hydration_slot(12, 10, 11), Some(12));
    }

    #[test]
    fn collect_ws_urls_registers_numbered_slots_and_dedups() {
        let mut vars = HashMap::new();
        vars.insert("SOLANA_WS_1_URL", "wss://ws1.example.com/?key=1");
        vars.insert("SOLANA_WS_2_URL", "wss://ws2.example.com/?key=2");
        vars.insert("SOLANA_WS_3_URL", "wss://ws3.example.com/?key=3");
        vars.insert("SOLANA_WS_4_URL", "wss://ws4.example.com/?key=4");
        vars.insert("FAST_WEBSOCKET", "wss://ws2.example.com/?key=2");
        vars.insert("SOLANA_WS_5_URL", "REPLACE_ME_WS_5");

        let urls = HeliusWsClient::collect_ws_urls(|k| vars.get(k).map(|v| v.to_string()));
        assert_eq!(
            urls,
            vec![
                "wss://ws1.example.com/?key=1",
                "wss://ws2.example.com/?key=2",
                "wss://ws3.example.com/?key=3",
                "wss://ws4.example.com/?key=4",
            ]
        );
    }

    #[test]
    fn collect_ws_urls_prioritizes_multi_sub_over_rpcfast() {
        let mut vars = HashMap::new();
        vars.insert(
            "SOLANA_WS_1_URL",
            "wss://solana-rpc.rpcfast.com/?api_key=abc",
        );
        vars.insert("SOLANA_WS_2_URL", "wss://devnet-rpc.shyft.to/?api_key=def");
        vars.insert(
            "SOLANA_WS_3_URL",
            "wss://mainnet.helius-rpc.com/?api-key=ghi",
        );

        let urls = HeliusWsClient::collect_ws_urls(|k| vars.get(k).map(|v| v.to_string()));
        assert_eq!(
            urls,
            vec![
                "wss://devnet-rpc.shyft.to/?api_key=def",
                "wss://mainnet.helius-rpc.com/?api-key=ghi",
                "wss://solana-rpc.rpcfast.com/?api_key=abc",
            ]
        );
    }

    #[test]
    fn collect_ws_urls_prioritizes_solami_shyft_helius_multi_provider() {
        let mut vars: HashMap<&str, String> = HashMap::new();
        vars.insert(
            "HELIUS_WS_URL",
            "wss://mainnet.helius-rpc.com/?api-key=test".into(),
        );
        vars.insert(
            "SOLAMI_RPC_WS_URL",
            "wss://rpc.solami.dev/?api_key=test".into(),
        );
        vars.insert("SHYFT_WS", "wss://rpc.fra.shyft.to?api_key=test".into());
        vars.insert("SOLANA_WS_PRIMARY", "wss://solana-ws-primary.com".into());

        let urls = HeliusWsClient::collect_ws_urls(|k| vars.get(k).map(|v| v.to_string()));
        assert_eq!(
            urls,
            vec![
                "wss://rpc.solami.dev/?api_key=test",
                "wss://rpc.fra.shyft.to?api_key=test",
                "wss://mainnet.helius-rpc.com/?api-key=test",
                "wss://solana-ws-primary.com",
            ]
        );
    }

    #[test]
    fn collect_ws_urls_excludes_solami_custom_event_stream_from_json_rpc_pool() {
        let mut vars = HashMap::new();
        vars.insert(
            "SOLAMI_WS_URL",
            "wss://ws.solami.dev/data/subscribe?api_key=must-not-be-forwarded".to_string(),
        );
        let urls = HeliusWsClient::collect_ws_urls(|k| vars.get(k).cloned());
        assert!(
            urls.is_empty(),
            "custom Solami event stream is not JSON-RPC"
        );
    }
    #[test]
    fn collect_ws_urls_resolves_helius_api_when_key_unset() {
        let mut vars = HashMap::new();
        vars.insert("HELIUS_API", "test-helius-key-123");
        let urls = HeliusWsClient::collect_ws_urls(|k| vars.get(k).map(|v| v.to_string()));
        assert_eq!(
            urls,
            vec!["wss://mainnet.helius-rpc.com/?api-key=test-helius-key-123"]
        );
    }

    #[test]
    fn rotate_endpoint_wraps_across_all_endpoints() {
        let client = HeliusWsClient::new(vec![
            "wss://ws1.example.com".to_string(),
            "wss://ws2.example.com".to_string(),
            "wss://ws3.example.com".to_string(),
        ]);
        assert_eq!(client.current_idx.load(Ordering::Acquire), 0);
        assert_eq!(client.rotate_endpoint(), 1);
        assert_eq!(client.rotate_endpoint(), 2);
        assert_eq!(client.rotate_endpoint(), 0);
    }

    #[test]
    fn handle_message_treats_subscription_errors_as_non_fatal() {
        let client = HeliusWsClient::new(vec!["wss://ws1.example.com".to_string()]);
        let ingest = IngestClient::new();
        let err_frame = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": { "code": -32000, "message": "Concurrent websocket subscription limit reached: 1" }
        });
        let res = client.handle_message(&err_frame, &ingest);
        assert_eq!(res, Ok(false));

        // Non-slotSubscribe error (accountSubscribe id >= 100) should NOT fail the stream
        let acc_err = json!({
            "jsonrpc": "2.0",
            "id": 100,
            "error": { "code": -32000, "message": "limit" }
        });
        assert_eq!(client.handle_message(&acc_err, &ingest), Ok(false));
        assert!(client.account_sub_limited.load(Ordering::Acquire));
        // A limit rejection must NOT be misclassified as a capability failure.
        assert!(!client.endpoint_unsupported.load(Ordering::Acquire));
    }

    /// Regression (live mainnet evidence): Alchemy WS rejects `accountSubscribe`
    /// with -32601 "Method 'accountSubscribe' not found". The old code treated
    /// this as a soft warning and kept the connection alive with all pending
    /// subscriptions cleared — the bot sat on a blind endpoint forever (0
    /// account updates, no SOL/USD reference, sizing permanently skipped).
    /// A capability failure must flag the endpoint as unsupported so the stream
    /// loop backs it off and fails over.
    #[test]
    fn handle_message_flags_unsupported_method_as_capability_failure() {
        let client = HeliusWsClient::new(vec!["wss://ws1.example.com".to_string()]);
        let ingest = IngestClient::new();
        let cap_err = json!({
            "jsonrpc": "2.0",
            "id": 100,
            "error": { "code": -32601, "message": "Method 'accountSubscribe' not found" }
        });
        assert_eq!(client.handle_message(&cap_err, &ingest), Ok(false));
        assert!(client.endpoint_unsupported.load(Ordering::Acquire));
        // Orthogonality: a capability failure is not a subscription quota limit.
        assert!(!client.account_sub_limited.load(Ordering::Acquire));

        let unsupported_err = json!({
            "jsonrpc": "2.0",
            "id": 101,
            "error": { "code": -32600, "message": "unsupported method" }
        });
        assert_eq!(client.handle_message(&unsupported_err, &ingest), Ok(false));
        assert!(client.endpoint_unsupported.load(Ordering::Acquire));
    }

    /// Malformed / hostile frames must be skipped without panicking, without
    /// poisoning slot state, and without tearing down the endpoint.
    #[test]
    fn malformed_frames_are_skipped_without_poisoning_state() {
        let client = HeliusWsClient::new(vec!["wss://ws1.example.com".to_string()]);
        let ingest = IngestClient::new();
        let slot_before = client.latest_slot.load(Ordering::Acquire);

        // Structurally valid JSON but semantically empty.
        assert_eq!(client.handle_message(&json!({}), &ingest), Ok(false));
        assert_eq!(
            client.handle_message(&json!({"jsonrpc": "2.0"}), &ingest),
            Ok(false)
        );
        // Wrong types for known fields.
        assert_eq!(
            client.handle_message(&json!({"method": 12345}), &ingest),
            Ok(false)
        );
        assert_eq!(
            client.handle_message(&json!({"id": "not-a-number", "result": {}}), &ingest),
            Ok(false)
        );
        // Notifications with missing / malformed payloads.
        assert_eq!(
            client.handle_message(&json!({"method": "slotNotification"}), &ingest),
            Ok(false)
        );
        assert_eq!(
            client.handle_message(
                &json!({"method": "slotNotification", "params": {"result": {"slot": "NaN"}}}),
                &ingest
            ),
            Ok(false)
        );
        assert_eq!(
            client.handle_message(
                &json!({"method": "accountNotification", "params": {"subscription": null}}),
                &ingest
            ),
            Ok(false)
        );
        // No valid slot metadata so far: slot state must be untouched.
        assert_eq!(client.latest_slot.load(Ordering::Acquire), slot_before);

        // accountNotification with an unknown subscription id and corrupt
        // base64 payload: no panic, no endpoint teardown. The provider-supplied
        // context slot is legitimate metadata and is recorded, but no snapshot
        // can be applied because the subscription id is unknown.
        assert_eq!(
            client.handle_message(
                &json!({
                    "method": "accountNotification",
                    "params": {
                        "subscription": 999_999,
                        "result": {"context": {"slot": 123}, "value": {"owner": "not-a-pubkey", "data": ["!!!not-base64!!!", "base64"]}}
                    }
                }),
                &ingest
            ),
            Ok(false)
        );
        assert_eq!(client.latest_slot.load(Ordering::Acquire), 123);

        assert!(!client.endpoint_unsupported.load(Ordering::Acquire));
        assert!(!client.account_sub_limited.load(Ordering::Acquire));
    }

    /// G8: a provider that rejects `slotSubscribe` (request id 1) may still
    /// deliver account updates. The rejection must disable the slot sub for
    /// this client -- NOT fail the endpoint -- because slot freshness falls
    /// back to the context slot carried by every accountNotification.
    #[test]
    fn slot_subscribe_rejection_disables_slot_sub_without_failing_endpoint() {
        let client =
            HeliusWsClient::new_with_options(vec!["wss://ws1.example.com".to_string()], true);
        let ingest = IngestClient::new();
        assert!(client.slot_subscribe_enabled.load(Ordering::Acquire));

        let slot_err = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": { "code": -32601, "message": "Method 'slotSubscribe' not found" }
        });
        assert_eq!(client.handle_message(&slot_err, &ingest), Ok(false));
        assert!(!client.slot_subscribe_enabled.load(Ordering::Acquire));
        // The endpoint still streams accounts: it must NOT be torn down.
        assert!(!client.endpoint_unsupported.load(Ordering::Acquire));

        // An account-sub capability failure on the same client still fails the
        // endpoint: the two capability gates are orthogonal.
        let acc_err = json!({
            "jsonrpc": "2.0",
            "id": 100,
            "error": { "code": -32601, "message": "Method 'accountSubscribe' not found" }
        });
        assert_eq!(client.handle_message(&acc_err, &ingest), Ok(false));
        assert!(client.endpoint_unsupported.load(Ordering::Acquire));
    }

    /// G8: the slot subscription is opt-out via WS_SLOT_SUBSCRIBE and the
    /// deterministic constructor; strict value parsing keeps typos fail-open
    /// only for unrecognized values while explicit off values are honored.
    #[test]
    fn slot_subscribe_env_gate_parses_strict_values() {
        assert!(slot_subscribe_enabled_from_env_value(None));
        assert!(slot_subscribe_enabled_from_env_value(Some("1".to_string())));
        assert!(slot_subscribe_enabled_from_env_value(Some(
            " true ".to_string()
        )));
        assert!(!slot_subscribe_enabled_from_env_value(Some(
            "0".to_string()
        )));
        assert!(!slot_subscribe_enabled_from_env_value(Some(
            "false".to_string()
        )));
        assert!(!slot_subscribe_enabled_from_env_value(Some(
            " off ".to_string()
        )));
        assert!(!slot_subscribe_enabled_from_env_value(Some(
            "no".to_string()
        )));

        let client =
            HeliusWsClient::new_with_options(vec!["wss://ws1.example.com".to_string()], false);
        assert!(!client.slot_subscribe_enabled.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn slot_notification_advances_slot_and_notifies_waiters() {
        let client = Arc::new(HeliusWsClient::new(vec![
            "wss://ws1.example.com".to_string(),
        ]));
        let ingest = IngestClient::new();
        let notifier = client.slot_notifier();

        let waited = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_secs(1), notifier.notified()).await
        });
        tokio::task::yield_now().await;

        let frame = json!({
            "jsonrpc": "2.0",
            "method": "slotNotification",
            "params": { "result": { "slot": 467_500_010u64, "parent": 467_500_009u64 } }
        });
        assert_eq!(client.handle_message(&frame, &ingest), Ok(true));
        assert_eq!(client.current_slot(), 467_500_010);
        assert_eq!(ingest.current_slot(), 467_500_010);
        assert_eq!(ingest.helius_slot(), 467_500_010);
        assert!(waited.await.unwrap().is_ok());
    }

    #[test]
    fn account_notification_updates_raydium_pool_state_in_cache() {
        let client = HeliusWsClient::new(vec!["wss://ws1.example.com".to_string()]);
        let ingest = IngestClient::new();

        let pool_id = PoolId(Pubkey::new_unique());
        let base_vault = Pubkey::new_unique();
        let quote_vault = Pubkey::new_unique();
        let reg = RegisteredPool {
            pool_id,
            protocol: DexProtocol::RaydiumV4,
            base_mint: MintId(Pubkey::new_unique()),
            quote_mint: MintId(Pubkey::new_unique()),
            decimals_a: 9,
            decimals_b: 6,
            base_vault,
            quote_vault,
            fee_bps: 25,
        };
        client.register_pool(&reg);

        // Seed pool state initially at slot 100
        let initial = PoolState {
            pool_id,
            protocol: DexProtocol::RaydiumV4,
            slot: Slot(100),
            write_version: 1,
            base_mint: reg.base_mint,
            quote_mint: reg.quote_mint,
            decimals_a: 9,
            decimals_b: 6,
            reserve_in: 1_000_000,
            reserve_out: 2_000_000,
            fee_numerator: 25,
            fee_denominator: 10_000,
            raydium_cpmm_fee: None,
            is_active: true,
            has_risky_token: false,
            last_update_slot: 100,
            last_update_ms: 0,
            kind: arb_types::PoolKind::Cpmm,
            clmm: None,
            dlmm: None,
            pump_fun: None,
            damm_v2: None,
            token_programs: Default::default(),
            token2022_fee_context: None,
            execution_accounts: None,
            transfer_fee_bps: 0,
        };
        assert!(ingest.apply_pool_update(initial).unwrap());

        // Map base_vault request id 100 -> sub_id 777
        client.req_to_account.write().insert(100, base_vault);
        assert_eq!(
            client.handle_message(
                &json!({"jsonrpc": "2.0", "id": 100, "result": 777}),
                &ingest
            ),
            Ok(false)
        );
        assert!(client.probe_confirmed.load(Ordering::Acquire));

        // Send SPL token account payload updating base_vault to 3_500_000 at slot 101
        let mut spl = vec![0u8; 165];
        spl[64..72].copy_from_slice(&3_500_000u64.to_le_bytes());
        let b64 = base64::engine::general_purpose::STANDARD.encode(&spl);
        let notif = json!({
            "jsonrpc": "2.0",
            "method": "accountNotification",
            "params": {
                "subscription": 777,
                "result": {
                    "context": { "slot": 101 },
                    "value": {
                        "owner": arb_constants::TOKEN_PROGRAM_ID,
                        "data": [b64, "base64"]
                    }
                }
            }
        });
        assert_eq!(client.handle_message(&notif, &ingest), Ok(false));

        let updated = ingest
            .cache()
            .get_pool(&pool_id)
            .expect("cached pool updated");
        assert_eq!(updated.slot, Slot(101));
        assert_eq!(updated.reserve_in, 3_500_000);
        assert_eq!(updated.reserve_out, 2_000_000);
    }

    /// Serializes the env-mutating `max_ws_account_subs` tests: cargo runs
    /// tests in parallel threads and `std::env::set_var` is process-global.
    static ENV_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

    #[test]
    fn max_ws_account_subs_parses_env_rejects_zero_and_caps() {
        let _guard = ENV_LOCK.lock();

        // Safety: `set_var`/`remove_var` are unsafe in edition 2024 because
        // concurrent readers observe UB; every reader of `WS_MAX_ACCOUNT_SUBS`
        // in this test binary runs under `ENV_LOCK`, so the mutation is
        // serialized against all reads.
        unsafe {
            // Unset -> historical default 48.
            std::env::remove_var("WS_MAX_ACCOUNT_SUBS");
        }
        assert_eq!(max_ws_account_subs(), 48);

        // Valid override passes through.
        unsafe {
            std::env::set_var("WS_MAX_ACCOUNT_SUBS", "512");
        }
        assert_eq!(max_ws_account_subs(), 512);

        // Provider fail-safe ceiling: never exceed 1024 account subs.
        unsafe {
            std::env::set_var("WS_MAX_ACCOUNT_SUBS", "9999");
        }
        assert_eq!(max_ws_account_subs(), 1_024);

        // 0 would mean "subscribe to nothing" -> rejected, default applies.
        unsafe {
            std::env::set_var("WS_MAX_ACCOUNT_SUBS", "0");
        }
        assert_eq!(max_ws_account_subs(), 48);

        // Garbage parses to nothing -> default.
        unsafe {
            std::env::set_var("WS_MAX_ACCOUNT_SUBS", "not-a-number");
        }
        assert_eq!(max_ws_account_subs(), 48);

        unsafe {
            std::env::remove_var("WS_MAX_ACCOUNT_SUBS");
        }
    }

    #[tokio::test]
    async fn queue_derived_hydration_notifies_worker_without_polling() {
        let client =
            HeliusWsClient::new_with_options(vec!["wss://ws1.example.com".to_string()], false);
        let pool_id = PoolId(Pubkey::new_unique());
        let notified = client.derived_hydration_notify.notified();

        client.queue_derived_hydration(pool_id, 77);

        tokio::time::timeout(Duration::from_millis(100), notified)
            .await
            .expect("queueing derived hydration must wake the worker");
        assert_eq!(client.pending_derived.lock().get(&pool_id), Some(&77));
    }

    #[test]
    fn account_wsol_linkage_prioritizes_wsol_pools_over_others() {
        let client = HeliusWsClient::new(vec!["wss://ws1.example.com".to_string()]);
        let wsol = Pubkey::from_str(arb_constants::WSOL_MINT).unwrap();

        // Orca pool paired against WSOL: registers the pool account only.
        let wsol_pool = RegisteredPool {
            pool_id: PoolId(Pubkey::new_unique()),
            protocol: DexProtocol::OrcaWhirlpool,
            base_mint: MintId(wsol),
            quote_mint: MintId(Pubkey::new_unique()),
            decimals_a: 9,
            decimals_b: 6,
            base_vault: Pubkey::new_unique(),
            quote_vault: Pubkey::new_unique(),
            fee_bps: 25,
        };
        client.register_pool(&wsol_pool);
        assert_eq!(client.account_wsol_linkage(&wsol_pool.pool_id.0), 0);

        // Raydium pool paired against WSOL: both vaults map through
        // `vault_to_pools` and must score 0 (they cost 2 sub slots each).
        let raydium_pool = RegisteredPool {
            pool_id: PoolId(Pubkey::new_unique()),
            protocol: DexProtocol::RaydiumV4,
            base_mint: MintId(Pubkey::new_unique()),
            quote_mint: MintId(wsol),
            decimals_a: 6,
            decimals_b: 9,
            base_vault: Pubkey::new_unique(),
            quote_vault: Pubkey::new_unique(),
            fee_bps: 25,
        };
        client.register_pool(&raydium_pool);
        assert_eq!(client.account_wsol_linkage(&raydium_pool.base_vault), 0);
        assert_eq!(client.account_wsol_linkage(&raydium_pool.quote_vault), 0);

        // Non-WSOL pool (USDC/USDT style) still gets subscribed, just last.
        let non_wsol_pool = RegisteredPool {
            pool_id: PoolId(Pubkey::new_unique()),
            protocol: DexProtocol::OrcaWhirlpool,
            base_mint: MintId(Pubkey::new_unique()),
            quote_mint: MintId(Pubkey::new_unique()),
            decimals_a: 6,
            decimals_b: 6,
            base_vault: Pubkey::new_unique(),
            quote_vault: Pubkey::new_unique(),
            fee_bps: 25,
        };
        client.register_pool(&non_wsol_pool);
        assert_eq!(client.account_wsol_linkage(&non_wsol_pool.pool_id.0), 1);

        // Untracked account: no linkage, lowest priority.
        assert_eq!(client.account_wsol_linkage(&Pubkey::new_unique()), 1);
    }

    /// Provider recovery cycle (checklist 222): rate-limited endpoints are
    /// rotated away from, and once their backoff expires the selector returns
    /// to the recovered endpoints while still-limited ones stay excluded.
    #[test]
    fn provider_recovery_cycle_backoff_rotation_then_return_after_expiry() {
        let client = HeliusWsClient::new_with_options(
            vec![
                "wss://ws1.example.com".to_string(),
                "wss://ws2.example.com".to_string(),
                "wss://ws3.example.com".to_string(),
            ],
            false,
        );

        // Primary (0) is rate-limited during connect: classify backoff, then rotate.
        client.record_connect_backoff(0, "handshake error: HTTP 429 too many requests");
        assert_eq!(client.select_active_endpoint_index(), 1);
        // Fallback 1 gets limited too: rotation must land on 2.
        client.record_connect_backoff(1, "handshake error: HTTP 429");
        assert_eq!(client.select_active_endpoint_index(), 2);

        // Both rate-limit cooldowns expire; endpoint 2 is still backed off.
        {
            let mut backoffs = client.ws_backoffs.write();
            backoffs.insert(0, Instant::now() - Duration::from_secs(1));
            backoffs.insert(1, Instant::now() - Duration::from_secs(1));
            backoffs.insert(2, Instant::now() + Duration::from_secs(60));
        }
        // current_idx == 2: selection skips the still-limited candidate and
        // returns to the recovered endpoints (earliest expired first).
        assert_eq!(client.select_active_endpoint_index(), 0);
        assert_eq!(client.current_idx.load(Ordering::Acquire), 0);
    }

    /// When every endpoint is in backoff the selector must elect the one whose
    /// cooldown expires earliest and update the rotation cursor (no deadlock,
    /// no panic, deterministic choice).
    #[test]
    fn recovery_cycle_all_endpoints_backoff_elects_earliest_expiry() {
        let client = HeliusWsClient::new_with_options(
            vec![
                "wss://ws1.example.com".to_string(),
                "wss://ws2.example.com".to_string(),
                "wss://ws3.example.com".to_string(),
            ],
            false,
        );
        let now = Instant::now();
        {
            let mut backoffs = client.ws_backoffs.write();
            backoffs.insert(0, now + Duration::from_secs(30));
            backoffs.insert(1, now + Duration::from_secs(5));
            backoffs.insert(2, now + Duration::from_secs(60));
        }
        assert_eq!(client.select_active_endpoint_index(), 1);
        assert_eq!(client.current_idx.load(Ordering::Acquire), 1);
    }

    /// Phase 3 task 3 — reconnection/backoff lifecycle. `record_connect_backoff`
    /// classifies provider failure text into escalating cooldowns: 429/"too many"
    /// → 60s, 4xx auth/capability codes → 120s, transient errors → 5s.
    #[test]
    fn connect_error_classifier_preserves_cooldown_without_leaking_provider_details() {
        assert_eq!(
            HeliusWsClient::classify_connect_error(
                "HTTP 429 Too Many Requests: api_key=do-not-log"
            ),
            "429 rate limited"
        );
        assert_eq!(
            HeliusWsClient::classify_connect_error("HTTP 401 Unauthorized: secret"),
            "401 Unauthorized"
        );
        assert_eq!(
            HeliusWsClient::classify_connect_error("connection reset by peer"),
            "websocket transport error"
        );
    }
    #[test]
    fn record_connect_backoff_classifies_provider_error_classes() {
        let client = HeliusWsClient::new(vec![
            "wss://ws1.example.com".to_string(),
            "wss://ws2.example.com".to_string(),
            "wss://ws3.example.com".to_string(),
        ]);
        client.record_connect_backoff(0, "429 rate limited: too many requests");
        client.record_connect_backoff(1, "401 Unauthorized");
        client.record_connect_backoff(2, "connection refused");
        let now = Instant::now();
        let remaining = |idx: usize| {
            client
                .ws_backoffs
                .read()
                .get(&idx)
                .copied()
                .unwrap()
                .saturating_duration_since(now)
        };
        // Band, not exact value: wall-clock advances between insert and read.
        assert!(
            remaining(0) > Duration::from_secs(58) && remaining(0) <= Duration::from_secs(60),
            "429 cooldown must be 60s, got {:?}",
            remaining(0)
        );
        assert!(
            remaining(1) > Duration::from_secs(118) && remaining(1) <= Duration::from_secs(120),
            "4xx cooldown must be 120s, got {:?}",
            remaining(1)
        );
        assert!(
            remaining(2) > Duration::from_secs(3) && remaining(2) <= Duration::from_secs(5),
            "transient cooldown must be 5s, got {:?}",
            remaining(2)
        );
    }

    /// Phase 3 task 3 — provider selection must skip non-expired backoffs and
    /// prefer a healthy endpoint over the rotation cursor; with a single
    /// endpoint no rotation is possible, so it is always returned.
    #[test]
    fn select_active_endpoint_skips_non_expired_backoffs() {
        let client = HeliusWsClient::new(vec![
            "wss://ws1.example.com".to_string(),
            "wss://ws2.example.com".to_string(),
            "wss://ws3.example.com".to_string(),
        ]);
        assert_eq!(client.current_idx.load(Ordering::Acquire), 0);
        client.record_connect_backoff(0, "429 too many");
        client.record_connect_backoff(1, "401 unauthorized");
        assert_eq!(client.select_active_endpoint_index(), 2);
        assert_eq!(client.current_idx.load(Ordering::Acquire), 2);

        let single = HeliusWsClient::new(vec!["wss://ws1.example.com".to_string()]);
        single.record_connect_backoff(0, "429 too many");
        assert_eq!(single.select_active_endpoint_index(), 0);
    }

    /// Phase 3 task 3 — the reconnect boundary must reset per-connection
    /// subscription state (req/response maps, probe confirmation, capability
    /// flags), restart the per-epoch slot watermark, clear the WS backoff that
    /// preceded the successful connect, and push the ingest side into a new
    /// stream epoch — WITHOUT discarding the decoded pool cache, so live
    /// notifications can refresh existing pools after reconnect.
    #[test]
    fn reconnect_reset_clears_subscription_state_and_flags_preserving_pool_cache() {
        let client = HeliusWsClient::new(vec!["wss://ws1.example.com".to_string()]);
        let ingest = Arc::new(IngestClient::new());

        let pool_id = PoolId(Pubkey::new_unique());
        client.register_pool(&RegisteredPool {
            pool_id,
            protocol: DexProtocol::RaydiumV4,
            base_mint: MintId(Pubkey::new_unique()),
            quote_mint: MintId(Pubkey::new_unique()),
            decimals_a: 9,
            decimals_b: 6,
            base_vault: Pubkey::new_unique(),
            quote_vault: Pubkey::new_unique(),
            fee_bps: 25,
        });
        let mut state = test_pool_state(pool_id);
        state.slot = Slot(100);
        state.last_update_slot = 100;
        assert!(ingest.apply_pool_update(state.clone()).unwrap());
        let snapshot_key = Pubkey::new_unique();
        assert!(ingest.account_store().apply(crate::AccountSnapshot {
            pubkey: snapshot_key,
            owner: Pubkey::new_unique(),
            slot: 100,
            write_version: 1,
            is_startup: true,
            provider_id: crate::HTTP_RPC_PROVIDER_ID,
            bank_id: None,
            generation: ingest.stream_generation(),
            data: std::sync::Arc::new(vec![0u8; 165]),
        }));

        // Stale per-connection state from the previous attempt.
        client.req_to_account.write().insert(100, snapshot_key);
        client.sub_to_account.write().insert(777, snapshot_key);
        client.probe_confirmed.store(true, Ordering::Release);
        client.endpoint_unsupported.store(true, Ordering::Release);
        client.account_sub_limited.store(true, Ordering::Release);
        client.record_connect_backoff(0, "429 too many");
        client.is_connected.store(false, Ordering::Release);
        client.latest_slot.store(123, Ordering::Release);
        ingest.mark_helius_disconnected();

        client.reset_connection_state(0, &ingest);

        // Resubscription will start from a clean subscription ledger.
        assert!(client.req_to_account.read().is_empty());
        assert!(client.sub_to_account.read().is_empty());
        assert!(!client.probe_confirmed.load(Ordering::Acquire));
        // Capability/quota flags clear so both capabilities are re-attempted.
        assert!(!client.account_sub_limited.load(Ordering::Acquire));
        assert!(!client.endpoint_unsupported.load(Ordering::Acquire));
        // The preceding failed attempt's cooldown is gone.
        assert!(
            client.ws_backoffs.read().get(&0).is_none(),
            "successful connect must clear the endpoint backoff"
        );
        // Per-epoch watermark restarts; the connection is live again.
        assert!(client.is_connected.load(Ordering::Acquire));
        assert_eq!(client.latest_slot.load(Ordering::Acquire), 0);
        // Ingest side entered a new fail-closed epoch.
        assert!(ingest.stream_connected());
        assert_eq!(ingest.stream_generation(), 1);
        // Previous-epoch snapshots are evicted; decoded pool state is preserved.
        assert!(ingest.account_store().get(&snapshot_key).is_none());
        let preserved = ingest
            .cache()
            .get_pool(&pool_id)
            .expect("pool cache must survive reconnect");
        assert_eq!(preserved.slot, Slot(100));
        assert_eq!(preserved.reserve_in, state.reserve_in);
    }

    /// Phase 3 task 3 — epoch accounting: repeated `connected` marks while the
    /// stream stays up are idempotent; only a reconnect after a disconnect
    /// advances the generation, evicts stale account snapshots, and clears the
    /// per-epoch slot watermark.
    #[test]
    fn reconnect_epoch_advances_only_across_disconnect_boundary() {
        let ingest = Arc::new(IngestClient::new());
        let pool_id = PoolId(Pubkey::new_unique());
        let mut state = test_pool_state(pool_id);
        state.slot = Slot(100);
        state.last_update_slot = 100;
        assert!(ingest.apply_pool_update(state).unwrap());
        let snapshot_key = Pubkey::new_unique();

        ingest.mark_helius_connected();
        let generation = ingest.stream_generation();
        assert!(ingest.stream_connected());
        assert!(ingest.account_store().apply(crate::AccountSnapshot {
            pubkey: snapshot_key,
            owner: Pubkey::new_unique(),
            slot: 100,
            write_version: 1,
            is_startup: false,
            provider_id: crate::ACCOUNT_WEBSOCKET_PROVIDER_ID,
            bank_id: None,
            generation,
            data: std::sync::Arc::new(vec![1, 2]),
        }));

        // Idempotent while connected: no generation bump, no snapshot eviction.
        ingest.mark_helius_connected();
        assert_eq!(ingest.stream_generation(), generation);
        assert!(ingest.account_store().get(&snapshot_key).is_some());

        ingest.record_helius_slot(150);
        ingest.mark_helius_disconnected();
        assert!(!ingest.stream_connected());
        assert!(!ingest.helius_connected());
        assert_eq!(ingest.helius_slot(), 0);

        ingest.mark_helius_connected();
        assert_eq!(ingest.stream_generation(), generation + 1);
        assert!(
            ingest.account_store().get(&snapshot_key).is_none(),
            "previous-epoch snapshot must not survive a reconnect"
        );
        assert!(
            ingest.cache().get_pool(&pool_id).is_some(),
            "decoded pool cache must survive a reconnect"
        );
        assert!(ingest.stream_connected());
    }

    fn test_pool_state(pool_id: PoolId) -> arb_types::PoolState {
        PoolState {
            pool_id,
            protocol: DexProtocol::RaydiumV4,
            slot: Slot(0),
            write_version: 1,
            base_mint: MintId(Pubkey::new_unique()),
            quote_mint: MintId(Pubkey::new_unique()),
            decimals_a: 9,
            decimals_b: 6,
            reserve_in: 1_000_000,
            reserve_out: 2_000_000,
            fee_numerator: 25,
            fee_denominator: 10_000,
            raydium_cpmm_fee: None,
            is_active: true,
            has_risky_token: false,
            last_update_slot: 0,
            last_update_ms: 0,
            kind: arb_types::PoolKind::Cpmm,
            clmm: None,
            dlmm: None,
            pump_fun: None,
            damm_v2: None,
            token_programs: Default::default(),
            token2022_fee_context: None,
            execution_accounts: None,
            transfer_fee_bps: 0,
        }
    }

    /// Regression for the P3-002 rectification: bootstrap (startup) snapshots
    /// from a previous epoch are refused after a reconnect, while live writes
    /// whose generation matches the current epoch are accepted.
    #[test]
    fn account_store_accepts_live_and_refuses_prev_epoch_bootstrap() {
        let ingest = Arc::new(IngestClient::new());
        let key = Pubkey::new_unique();

        // Bootstrap seed before any connect: generation 0.
        assert!(ingest.account_store().apply(crate::AccountSnapshot {
            pubkey: key,
            owner: Pubkey::new_unique(),
            slot: 100,
            write_version: 1,
            is_startup: true,
            provider_id: crate::HTTP_RPC_PROVIDER_ID,
            bank_id: None,
            generation: 0,
            data: std::sync::Arc::new(vec![0u8; 165]),
        }));
        ingest.mark_helius_connected();
        let generation = ingest.stream_generation();

        // Same-epoch live write replaces bootstrap even with an older slot.
        assert!(ingest.account_store().apply(crate::AccountSnapshot {
            pubkey: key,
            owner: Pubkey::new_unique(),
            slot: 90,
            write_version: 1,
            is_startup: false,
            provider_id: crate::ACCOUNT_WEBSOCKET_PROVIDER_ID,
            bank_id: None,
            generation,
            data: std::sync::Arc::new(vec![1u8; 165]),
        }));
        // Bootstrap from the new generation still cannot overwrite live state.
        assert!(!ingest.account_store().apply(crate::AccountSnapshot {
            pubkey: key,
            owner: Pubkey::new_unique(),
            slot: 100,
            write_version: 1,
            is_startup: true,
            provider_id: crate::HTTP_RPC_PROVIDER_ID,
            bank_id: None,
            generation,
            data: std::sync::Arc::new(vec![2u8; 165]),
        }));
    }
}
