//! Multi-provider WebSocket Client for real-time Solana slot and account streams.
//! Subscribes to `slotSubscribe` and targeted `accountSubscribe`
//! with automatic failover across configured endpoints (Shyft, Helius, RPCFast, etc.),
//! live `MarketStateCache` updates, and instant slot notifications.

use crate::{
    AccountSnapshot, IngestClient, ProviderId, RegisteredPool, clmm_tick_array_covers_tick,
    compute_clmm_virtual_reserves, compute_dlmm_reserves, decode_clmm_state, decode_dlmm_fee_bps,
    decode_dlmm_state, decode_orca_fee_bps, decode_spl_token_amount_bytes, next_pool_write_version,
    slot_integrity,
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
fn max_ws_account_subs() -> usize {
    std::env::var("WS_MAX_ACCOUNT_SUBS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(48)
        .min(1_024)
}

pub struct HeliusWsClient {
    pub ws_urls: Vec<String>,
    pub current_idx: Arc<AtomicUsize>,
    pub latest_slot: Arc<AtomicU64>,
    pub is_connected: Arc<AtomicBool>,
    tracked_accounts: parking_lot::RwLock<HashMap<Pubkey, Pubkey>>, // account -> expected owner
    tracked_order: parking_lot::RwLock<Vec<Pubkey>>,
    registered_pools: parking_lot::RwLock<HashMap<PoolId, RegisteredPool>>,
    vault_to_pools: parking_lot::RwLock<HashMap<Pubkey, Vec<(PoolId, bool)>>>,
    req_to_account: parking_lot::RwLock<HashMap<u64, Pubkey>>,
    sub_to_account: parking_lot::RwLock<HashMap<u64, Pubkey>>,
    ws_backoffs: parking_lot::RwLock<HashMap<usize, Instant>>,
    account_sub_limited: AtomicBool,
    /// Current endpoint cannot stream accounts at all (capability failure).
    /// Reset on every successful connect; checked by the stream loop to break
    /// and fail over instead of sitting on a blind connection.
    endpoint_unsupported: AtomicBool,
    probe_confirmed: AtomicBool,
    slot_notify: Arc<Notify>,
}

impl HeliusWsClient {
    pub fn new(ws_urls: Vec<String>) -> Self {
        Self {
            ws_urls,
            current_idx: Arc::new(AtomicUsize::new(0)),
            latest_slot: Arc::new(AtomicU64::new(0)),
            is_connected: Arc::new(AtomicBool::new(false)),
            tracked_accounts: parking_lot::RwLock::new(HashMap::new()),
            tracked_order: parking_lot::RwLock::new(Vec::new()),
            registered_pools: parking_lot::RwLock::new(HashMap::new()),
            vault_to_pools: parking_lot::RwLock::new(HashMap::new()),
            req_to_account: parking_lot::RwLock::new(HashMap::new()),
            sub_to_account: parking_lot::RwLock::new(HashMap::new()),
            ws_backoffs: parking_lot::RwLock::new(HashMap::new()),
            account_sub_limited: AtomicBool::new(false),
            endpoint_unsupported: AtomicBool::new(false),
            probe_confirmed: AtomicBool::new(false),
            slot_notify: Arc::new(Notify::new()),
        }
    }

    pub fn from_env() -> Option<Self> {
        let urls = Self::collect_ws_urls(|k| std::env::var(k).ok());
        if urls.is_empty() {
            return None;
        }
        Some(Self::new(urls))
    }

    pub fn collect_ws_urls<F>(lookup: F) -> Vec<String>
    where
        F: Fn(&str) -> Option<String>,
    {
        let mut urls = Vec::new();
        // Priority 1: Role-Separated Endpoints
        for key in [
            "SOLANA_WS_PRIMARY",
            "SOLANA_WS_FALLBACK_1",
            "SOLANA_WS_FALLBACK_2",
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
            "SOLAMI_WS_URL".to_string(),
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
        if let Some(key) = lookup("HELIUS_API_KEY") {
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

        // Prioritize multi-subscription WS endpoints (Alchemy first, then other multi-sub, then single-sub rpcfast).
        let mut multi_sub: Vec<String> = urls
            .iter()
            .filter(|u| u.contains("alchemy.com"))
            .cloned()
            .collect();
        multi_sub.extend(
            urls.iter()
                .filter(|u| !u.contains("rpcfast.com") && !u.contains("alchemy.com"))
                .cloned(),
        );
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

    fn redact_url(url: &str) -> String {
        if let Some((base, _)) = url.split_once('?') {
            format!("{base}?***")
        } else {
            url.to_string()
        }
    }

    /// Share notifier fired whenever `slotNotification` advances the slot watermark.
    pub fn slot_notifier(&self) -> Arc<Notify> {
        Arc::clone(&self.slot_notify)
    }

    pub fn track_account(&self, pubkey: Pubkey, expected_owner: Pubkey) {
        let mut tracked = self.tracked_accounts.write();
        if tracked.insert(pubkey, expected_owner).is_none() {
            self.tracked_order.write().push(pubkey);
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
            DexProtocol::MeteoraDlmm => {
                if let Ok(owner) = Pubkey::from_str(arb_constants::METEORA_DLMM_PROGRAM_ID) {
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
    pub async fn seed_initial_accounts(&self, yellowstone: &IngestClient) {
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
                    (chunk, rpc.get_multiple_accounts_with_slot(&refs).await)
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
            let Ok((accounts, slot)) = chunk_res else {
                debug!(
                    "seed_initial_accounts: batch fetch of {} accounts failed: {}",
                    chunk_pks.len(),
                    chunk_res.unwrap_err()
                );
                continue;
            };
            latest_slot = latest_slot.max(slot);
            for (val, pk) in accounts.into_iter().zip(chunk_pks.iter()) {
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
                    slot,
                    write_version: 1,
                    is_startup: true,
                    provider_id: ProviderId(2),
                    bank_id: None,
                    generation: yellowstone.stream_generation(),
                    data: Arc::new(bytes),
                };
                yellowstone.account_store().apply(snap);
                seeded_count += 1;
            }
        }

        if seeded_count > 0 {
            if latest_slot > 0 {
                self.latest_slot.fetch_max(latest_slot, Ordering::AcqRel);
                yellowstone.record_helius_slot(latest_slot);
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
        yellowstone: Arc<IngestClient>,
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
        self.seed_initial_accounts(&yellowstone).await;

        let mut attempt = 0u32;
        let mut consecutive_failures = 0usize;

        while !cancel.is_cancelled() {
            attempt = attempt.saturating_add(1);
            let idx = self.select_active_endpoint_index();
            let url_str = self.ws_urls[idx].clone();
            let safe_url = Self::redact_url(&url_str);
            let endpoint_num = idx + 1;
            let total_endpoints = self.ws_urls.len();

            match connect_async(&url_str).await {
                Ok((mut ws_stream, _)) => {
                    self.ws_backoffs.write().remove(&idx);
                    self.req_to_account.write().clear();
                    self.sub_to_account.write().clear();
                    self.account_sub_limited.store(false, Ordering::Release);
                    self.endpoint_unsupported.store(false, Ordering::Release);
                    self.probe_confirmed.store(false, Ordering::Release);

                    self.is_connected.store(true, Ordering::Release);
                    yellowstone.mark_helius_connected();
                    yellowstone.mark_stream_connected();
                    info!(
                        "⚡ WebSocket [{endpoint_num}/{total_endpoints}] connected to mainnet via {safe_url}"
                    );

                    // 1. WebSocket Dedicated Mode: Do NOT send `slotSubscribe`.
                    // Many RPC providers (e.g. Helius free tier, RPCFast, Shyft) enforce
                    // `Concurrent websocket subscription limit reached: 1`. Sending `slotSubscribe`
                    // exhausts the single allowed subscription, causing subsequent `accountSubscribe`
                    // calls to be rejected with errors and tearing down the stream.
                    // Slots are advanced with zero RPC cost via HTTP getSlot/poll or incoming account update context slots.
                    debug!(
                        "WebSocket [{endpoint_num}/{total_endpoints}] connected; slotSubscribe omitted to reserve connection for pool accounts"
                    );

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
                        raw.sort_by_key(|acc| self.account_wsol_linkage(acc));
                        raw.into_iter().take(max_ws_account_subs()).collect()
                    };
                    let mut next_req_id = 100u64;
                    if let Some(first_acc) = pending_subs.pop_front() {
                        let frame = self.build_account_sub_frame(next_req_id, first_acc);
                        next_req_id += 1;
                        let _ = ws_stream.send(frame).await;
                    }

                    // 3. Ingestion Loop with 15s Heartbeat Ping and Resilient Streaming
                    let mut received_account_update = false;
                    let mut ping_interval = tokio::time::interval(Duration::from_secs(15));
                    ping_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

                    loop {
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
                        }

                        tokio::select! {
                            _ = cancel.cancelled() => {
                                self.is_connected.store(false, Ordering::Release);
                                return;
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
                                    warn!("WebSocket stream ended by peer ([{endpoint_num}/{total_endpoints}] {safe_url}); failing over");
                                    break;
                                };

                                match msg_res {
                                    Ok(Message::Text(text)) => {
                                        if let Ok(v) = serde_json::from_str::<Value>(&text) {
                                            match self.handle_message(&v, &yellowstone) {
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
                Err(e) => {
                    self.is_connected.store(false, Ordering::Release);
                    self.record_connect_backoff(idx, &e.to_string());
                    let next_idx = self.rotate_endpoint();
                    consecutive_failures += 1;
                    warn!(
                        "WebSocket [{endpoint_num}/{total_endpoints}] connect failed on {safe_url} ({e}); rotating to [{}/{}] (attempt={attempt})",
                        next_idx + 1,
                        total_endpoints
                    );
                }
            }

            self.is_connected.store(false, Ordering::Release);
            yellowstone.mark_helius_disconnected();

            // Resilient WS connection & cooldown:
            // Apply a 2.5s to 3.0s cooldown when rotating or reconnecting to prevent
            // hitting provider limits ("Concurrent websocket subscription limit reached: 1")
            // due to lingering dead TCP/WS sessions on the peer side.
            let backoff_ms = if consecutive_failures < total_endpoints {
                2500
            } else {
                3000
            };
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = sleep(Duration::from_millis(backoff_ms)) => {}
            }
        }
    }

    fn apply_live_pool_update_from_ws(
        &self,
        pk: Pubkey,
        bytes: &[u8],
        slot: u64,
        yellowstone: &IngestClient,
    ) {
        let effective_slot_u64 = slot
            .max(self.latest_slot.load(Ordering::Acquire))
            .max(yellowstone.current_slot());
        if effective_slot_u64 == 0 {
            return;
        }
        let effective_slot = Slot(effective_slot_u64);
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
                let Some(mut state) = yellowstone.cache().get_pool(&pool_id) else {
                    continue;
                };
                if is_base {
                    state.reserve_in = u128::from(amount);
                } else {
                    state.reserve_out = u128::from(amount);
                }
                if state.reserve_in == 0 || state.reserve_out == 0 {
                    continue;
                }
                state.slot = effective_slot;
                state.last_update_slot = effective_slot_u64;
                state.last_update_ms = now_ms;
                state.write_version = next_pool_write_version();
                let _ = yellowstone.apply_pool_update(state);
            }
            return;
        }

        // Case 2: Orca Whirlpool / Meteora DLMM pool account update
        let pool_id = PoolId(pk);
        let reg_opt = self.registered_pools.read().get(&pool_id).cloned();
        let Some(reg) = reg_opt else {
            return;
        };
        let Some(mut state) = yellowstone.cache().get_pool(&pool_id) else {
            return;
        };

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
                    return; // Fail closed until 750ms hydration fetches shifted tick arrays
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
                state.slot = effective_slot;
                state.last_update_slot = effective_slot_u64;
                state.last_update_ms = now_ms;
                state.write_version = next_pool_write_version();
                let _ = yellowstone.apply_pool_update(state);
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
                    return; // Fail closed until hydration fetches active BinArray
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
                    state.slot = effective_slot;
                    state.last_update_slot = effective_slot_u64;
                    state.last_update_ms = now_ms;
                    state.write_version = next_pool_write_version();
                    let _ = yellowstone.apply_pool_update(state);
                }
            }
            _ => {}
        }
    }

    /// Process an incoming JSON-RPC WebSocket frame.
    /// Returns `Ok(true)` when an update (accountNotification or slotNotification) was received,
    /// `Ok(false)` for other valid frames, or `Err(warning)` for soft subscription rejections.
    pub fn handle_message(&self, v: &Value, yellowstone: &IngestClient) -> Result<bool, String> {
        if let Some(err_obj) = v.get("error") {
            let req_id = v.get("id").and_then(|i| i.as_u64());
            let code = err_obj.get("code").and_then(|c| c.as_i64());
            let err_msg = err_obj
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown JSON-RPC error");
            let lower = err_msg.to_ascii_lowercase();
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
                        yellowstone.record_slot_update(
                            slot,
                            parent,
                            slot_integrity::SLOT_PROCESSED,
                        );
                        yellowstone.record_helius_slot(slot);
                        yellowstone.mark_helius_connected();
                        yellowstone.mark_stream_connected();
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

                    if slot > 0 && !yellowstone.slot_is_dead(slot) {
                        self.latest_slot.fetch_max(slot, Ordering::AcqRel);
                        yellowstone.record_helius_slot(slot);
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
                                self.apply_live_pool_update_from_ws(pk, &bytes, slot, yellowstone);
                                let snap = AccountSnapshot {
                                    pubkey: pk,
                                    owner,
                                    slot,
                                    write_version: next_pool_write_version(),
                                    is_startup: false,
                                    provider_id: ProviderId(2),
                                    bank_id: None,
                                    generation: yellowstone.stream_generation(),
                                    data: Arc::new(bytes),
                                };
                                yellowstone.account_store().apply(snap);
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
        let yellowstone = IngestClient::new();
        let err_frame = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": { "code": -32000, "message": "Concurrent websocket subscription limit reached: 1" }
        });
        let res = client.handle_message(&err_frame, &yellowstone);
        assert_eq!(res, Ok(false));

        // Non-slotSubscribe error (accountSubscribe id >= 100) should NOT fail the stream
        let acc_err = json!({
            "jsonrpc": "2.0",
            "id": 100,
            "error": { "code": -32000, "message": "limit" }
        });
        assert_eq!(client.handle_message(&acc_err, &yellowstone), Ok(false));
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
        let yellowstone = IngestClient::new();
        let cap_err = json!({
            "jsonrpc": "2.0",
            "id": 100,
            "error": { "code": -32601, "message": "Method 'accountSubscribe' not found" }
        });
        assert_eq!(client.handle_message(&cap_err, &yellowstone), Ok(false));
        assert!(client.endpoint_unsupported.load(Ordering::Acquire));
        // Orthogonality: a capability failure is not a subscription quota limit.
        assert!(!client.account_sub_limited.load(Ordering::Acquire));

        let unsupported_err = json!({
            "jsonrpc": "2.0",
            "id": 101,
            "error": { "code": -32600, "message": "unsupported method" }
        });
        assert_eq!(
            client.handle_message(&unsupported_err, &yellowstone),
            Ok(false)
        );
        assert!(client.endpoint_unsupported.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn slot_notification_advances_slot_and_notifies_waiters() {
        let client = Arc::new(HeliusWsClient::new(vec![
            "wss://ws1.example.com".to_string(),
        ]));
        let yellowstone = IngestClient::new();
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
        assert_eq!(client.handle_message(&frame, &yellowstone), Ok(true));
        assert_eq!(client.current_slot(), 467_500_010);
        assert_eq!(yellowstone.current_slot(), 467_500_010);
        assert_eq!(yellowstone.helius_slot(), 467_500_010);
        assert!(waited.await.unwrap().is_ok());
    }

    #[test]
    fn account_notification_updates_raydium_pool_state_in_cache() {
        let client = HeliusWsClient::new(vec!["wss://ws1.example.com".to_string()]);
        let yellowstone = IngestClient::new();

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
            is_active: true,
            has_risky_token: false,
            last_update_slot: 100,
            last_update_ms: 0,
            kind: arb_types::PoolKind::Cpmm,
            clmm: None,
            dlmm: None,
            pump_fun: None,
            token_programs: Default::default(),
            execution_accounts: None,
            transfer_fee_bps: 0,
        };
        assert!(yellowstone.apply_pool_update(initial).unwrap());

        // Map base_vault request id 100 -> sub_id 777
        client.req_to_account.write().insert(100, base_vault);
        assert_eq!(
            client.handle_message(
                &json!({"jsonrpc": "2.0", "id": 100, "result": 777}),
                &yellowstone
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
        assert_eq!(client.handle_message(&notif, &yellowstone), Ok(false));

        let updated = yellowstone
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
}
