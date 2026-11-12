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
use arb_types::{DexProtocol, PoolId, Slot};
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

const MAX_WS_ACCOUNT_SUBS: usize = 48;

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

        let fetcher = crate::registry::PacedAccountFetcher::from_env();
        let mut seeded_count = 0usize;
        let mut latest_slot = 0u64;

        for pk in keys {
            let expected_owner = self
                .tracked_accounts
                .read()
                .get(&pk)
                .copied()
                .unwrap_or_default();
            match fetcher.fetch_single_account(&pk).await {
                Ok((slot, bytes)) => {
                    latest_slot = latest_slot.max(slot);
                    let snap = AccountSnapshot {
                        pubkey: pk,
                        owner: expected_owner,
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
                Err(e) => {
                    debug!("seed_initial_accounts: paced fetch for {pk} failed: {e}");
                }
            }
        }

        if seeded_count > 0 {
            if latest_slot > 0 {
                self.latest_slot.fetch_max(latest_slot, Ordering::AcqRel);
                yellowstone.record_helius_slot(latest_slot);
            }
            info!(
                "Seeded {seeded_count} initial snapshots via paced single-account RPC at slot {latest_slot}"
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
                        raw.into_iter().take(MAX_WS_ACCOUNT_SUBS).collect()
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
                let Some((r_in, r_out)) =
                    compute_dlmm_reserves(&dlmm, bytes, &reg.base_mint.0, &reg.quote_mint.0)
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
            _ => {}
        }
    }

    /// Process an incoming JSON-RPC WebSocket frame.
    /// Returns `Ok(true)` when an update (accountNotification or slotNotification) was received,
    /// `Ok(false)` for other valid frames, or `Err(warning)` for soft subscription rejections.
    pub fn handle_message(&self, v: &Value, yellowstone: &IngestClient) -> Result<bool, String> {
        if let Some(err_obj) = v.get("error") {
            let req_id = v.get("id").and_then(|i| i.as_u64());
            let err_msg = err_obj
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown JSON-RPC error");
            let lower = err_msg.to_ascii_lowercase();
            if lower.contains("limit")
                || lower.contains("not found")
                || lower.contains("unsupported")
            {
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
}
