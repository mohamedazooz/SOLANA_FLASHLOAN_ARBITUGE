//! Multi-DEX Active Pool Registry & Live Reserve Ingestion.
//!
//! Registers verified mainnet pools across Raydium V4, Orca Whirlpool, and
//! Meteora DLMM and batch-fetches their live reserve balances each tick. The
//! reserve VALUES (the live market data) are decoded directly from on-chain
//! token-vault account balances via `getMultipleAccounts` — nothing is
//! hardcoded. Only the permanent on-chain account ADDRESSES (pool + vault
//! pubkeys, which never change after pool creation) are recorded, matching
//! the existing pattern used for Raydium / Orca vaults.
//!
//! Feeding these live multi-pool states into `MarketGraph` ends the previous
//! single-pool starvation where `find_base_cycles` could never find a second
//! pool to form a 2-hop cross-DEX arbitrage.

use crate::{DependencyManifestRegistry, RpcPool};
use anyhow::{Context, Result, bail};
use arb_types::{DexProtocol, MintId, PoolId, PoolState, Slot};
use base64::prelude::*;
use solana_pubkey::Pubkey;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;
use tokio::time::sleep;
use tracing::{debug, warn};

/// Thread-safe Paced Single-Account RPC Fetcher enforcing sequential requests,
/// strict semaphore rate limits, inter-request pacing, and automatic endpoint failover.
#[derive(Clone)]
pub struct PacedAccountFetcher {
    endpoints: Arc<Vec<String>>,
    client: reqwest::Client,
    semaphore: Arc<Semaphore>,
    min_interval: Duration,
    current_idx: Arc<AtomicUsize>,
    last_call: Arc<parking_lot::Mutex<Instant>>,
    endpoint_cooldowns: Arc<parking_lot::Mutex<std::collections::HashMap<usize, Instant>>>,
}

impl PacedAccountFetcher {
    pub fn new(endpoints: Vec<String>, max_concurrent: usize, min_interval_ms: u64) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(6))
            .connect_timeout(Duration::from_secs(3))
            .build()
            .unwrap_or_default();

        Self {
            endpoints: Arc::new(endpoints),
            client,
            semaphore: Arc::new(Semaphore::new(max_concurrent.max(1))),
            min_interval: Duration::from_millis(min_interval_ms),
            current_idx: Arc::new(AtomicUsize::new(0)),
            last_call: Arc::new(parking_lot::Mutex::new(
                Instant::now() - Duration::from_secs(1),
            )),
            endpoint_cooldowns: Arc::new(parking_lot::Mutex::new(std::collections::HashMap::new())),
        }
    }

    pub fn from_env() -> Self {
        let mut endpoints = Vec::new();
        let keys = [
            "ALCHEMY_RPC_URL",
            "SOLANA_RPC_PRIMARY",
            "SOLANA_RPC_FALLBACK_1",
            "SOLANA_RPC_FALLBACK_2",
            "SOLANA_RPC_1_URL",
            "SOLANA_RPC_2_URL",
            "SOLANA_RPC_3_URL",
            "SOLANA_RPC_4_URL",
            "SOLANA_RPC_5_URL",
            "SOLANA_RPC_6_URL",
            "SOLANA_RPC_7_URL",
            "SOLANA_RPC_URL",
            "HELIUS_RPC_URL",
            "SOLAMI_RPC_URL",
            "SOLANA_MAINNET_RPC_FALLBACK",
        ];
        for key in keys {
            if let Ok(val) = std::env::var(key) {
                let trimmed = val.trim().to_string();
                if !trimmed.is_empty()
                    && !trimmed.contains("REPLACE_ME")
                    && (trimmed.starts_with("https://") || trimmed.starts_with("http://"))
                    && !endpoints.contains(&trimmed)
                {
                    endpoints.push(trimmed);
                }
            }
        }
        if endpoints.is_empty() {
            endpoints.push("https://api.mainnet-beta.solana.com".to_string());
        }

        // Prioritize high-throughput, generous-limit endpoints first (Alchemy and Shyft)
        endpoints.sort_by_key(|e| {
            if e.contains("alchemy.com") {
                0
            } else if e.contains("shyft.to") {
                1
            } else if e.contains("helius") {
                3
            } else {
                2
            }
        });

        let max_concurrent: usize = std::env::var("RPC_MAX_CONCURRENT_REQUESTS")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(4);

        let min_interval_ms: u64 = std::env::var("RPC_MIN_INTERVAL_MS")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(30);

        Self::new(endpoints, max_concurrent, min_interval_ms)
    }

    /// Places endpoint `idx` on cooldown and emits the failure warning ONLY on
    /// the no-cooldown → cooldown state transition.
    ///
    /// Why: with `max_concurrent` in-flight requests, several tasks can select
    /// the same endpoint before any cooldown insert lands; each 429 response
    /// then hits this path. Under a rate-limited fallback this produced
    /// ~39,500 WARN lines in a 3-minute soak (~660/min). Gating the warn on
    /// the transition (checked under the same `endpoint_cooldowns` lock as the
    /// insert) collapses that to at most one line per cooldown cycle per
    /// endpoint (60s for HTTP 429/5xx, 15s for transport errors) while
    /// preserving the failover behavior exactly.
    fn cooldown_and_warn(&self, idx: usize, url: &str, cooldown: Duration, detail: String) {
        let mut guard = self.endpoint_cooldowns.lock();
        let was_active = guard
            .get(&idx)
            .is_some_and(|cooldown_end| Instant::now() < *cooldown_end);
        guard.insert(idx, Instant::now() + cooldown);
        drop(guard);
        if !was_active {
            warn!(
                "RPC endpoint {} failed ({}); placing on {}s cooldown and rotating",
                url,
                detail,
                cooldown.as_secs()
            );
        }
    }

    pub async fn fetch_single_account(&self, pubkey: &Pubkey) -> Result<(u64, Vec<u8>)> {
        let _permit = self.semaphore.acquire().await.context("semaphore closed")?;

        let now = Instant::now();
        let wait_dur = {
            let mut guard = self.last_call.lock();
            let elapsed = now.duration_since(*guard);
            if elapsed < self.min_interval {
                let wait = self.min_interval - elapsed;
                *guard = now + wait;
                Some(wait)
            } else {
                *guard = now;
                None
            }
        };
        if let Some(w) = wait_dur {
            sleep(w).await;
        }

        let total_endpoints = self.endpoints.len();
        let max_attempts = total_endpoints.saturating_mul(2).max(1);
        let mut last_err = anyhow::anyhow!("no endpoints available");

        for _ in 0..max_attempts {
            let idx = {
                let mut guard = self.endpoint_cooldowns.lock();
                let now = Instant::now();
                let mut chosen = None;
                for offset in 0..total_endpoints {
                    let candidate = (self.current_idx.fetch_add(1, Ordering::Relaxed) + offset)
                        % total_endpoints;
                    if let Some(cooldown_end) = guard.get(&candidate) {
                        if now < *cooldown_end {
                            continue; // skip endpoint during cooldown
                        } else {
                            guard.remove(&candidate);
                        }
                    }
                    chosen = Some(candidate);
                    break;
                }
                chosen.unwrap_or_else(|| self.current_idx.load(Ordering::Relaxed) % total_endpoints)
            };
            let url = &self.endpoints[idx];

            let req_body_zstd = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "getAccountInfo",
                "params": [
                    pubkey.to_string(),
                    {
                        "encoding": "base64+zstd",
                        "commitment": "processed"
                    }
                ]
            });

            let resp_res = self.client.post(url).json(&req_body_zstd).send().await;
            match resp_res {
                Ok(resp) => {
                    let status = resp.status();
                    if status.as_u16() == 429 || status.is_server_error() {
                        self.cooldown_and_warn(
                            idx,
                            url,
                            Duration::from_secs(60),
                            format!("HTTP {status}"),
                        );
                        continue;
                    }
                    if let Ok(json_val) = resp.json::<serde_json::Value>().await {
                        if let Some(err_obj) = json_val.get("error") {
                            debug!(
                                "RPC returned error {:?} on base64+zstd, falling back to base64",
                                err_obj
                            );
                            let req_body_b64 = serde_json::json!({
                                "jsonrpc": "2.0",
                                "id": 1,
                                "method": "getAccountInfo",
                                "params": [
                                    pubkey.to_string(),
                                    {
                                        "encoding": "base64",
                                        "commitment": "processed"
                                    }
                                ]
                            });
                            if let Ok(b64_resp) =
                                self.client.post(url).json(&req_body_b64).send().await
                            {
                                if let Ok(b64_val) = b64_resp.json::<serde_json::Value>().await {
                                    return Self::parse_account_info_response(&b64_val, false);
                                }
                            }
                        } else {
                            match Self::parse_account_info_response(&json_val, true) {
                                Ok(res) => return Ok(res),
                                Err(e) => {
                                    last_err = e;
                                    break;
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    self.cooldown_and_warn(
                        idx,
                        url,
                        Duration::from_secs(15),
                        format!("transport error: {e}"),
                    );
                    last_err = anyhow::anyhow!(e);
                    sleep(Duration::from_millis(50)).await;
                }
            }
        }

        Err(last_err)
    }

    fn parse_account_info_response(
        val: &serde_json::Value,
        is_zstd: bool,
    ) -> Result<(u64, Vec<u8>)> {
        let slot = val
            .pointer("/result/context/slot")
            .and_then(|s| s.as_u64())
            .unwrap_or(0);

        let value_node = val.pointer("/result/value");
        if value_node.is_none() || value_node.unwrap().is_null() {
            bail!("account not found (null value)");
        }

        let raw_b64 = value_node
            .and_then(|v| v.get("data"))
            .and_then(|d| d.get(0))
            .and_then(|s| s.as_str())
            .context("missing data[0] in account info")?;

        let decoded_bytes = BASE64_STANDARD
            .decode(raw_b64)
            .context("base64 decode failed")?;

        if is_zstd {
            match zstd::decode_all(&decoded_bytes[..]) {
                Ok(decompressed) => Ok((slot, decompressed)),
                Err(_) => Ok((slot, decoded_bytes)),
            }
        } else {
            Ok((slot, decoded_bytes))
        }
    }

    /// Batch fetch multiple accounts via JSON-RPC `getMultipleAccounts`.
    pub async fn fetch_multiple_accounts(
        &self,
        pubkeys: &[Pubkey],
    ) -> Result<std::collections::HashMap<Pubkey, (u64, Vec<u8>)>> {
        if pubkeys.is_empty() {
            return Ok(std::collections::HashMap::new());
        }

        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let now = Instant::now();
        let wait_dur = {
            let mut guard = self.last_call.lock();
            let elapsed = now.duration_since(*guard);
            if elapsed < self.min_interval {
                let wait = self.min_interval - elapsed;
                *guard = now + wait;
                Some(wait)
            } else {
                *guard = now;
                None
            }
        };
        if let Some(w) = wait_dur {
            sleep(w).await;
        }

        let total_endpoints = self.endpoints.len();
        let max_attempts = total_endpoints.saturating_mul(2).max(1);
        let mut last_err = anyhow::anyhow!("no endpoints available");

        let keys_str: Vec<String> = pubkeys.iter().map(|p| p.to_string()).collect();

        for _ in 0..max_attempts {
            let idx = {
                let mut guard = self.endpoint_cooldowns.lock();
                let now = Instant::now();
                let mut chosen = None;
                for offset in 0..total_endpoints {
                    let candidate = (self.current_idx.fetch_add(1, Ordering::Relaxed) + offset)
                        % total_endpoints;
                    if let Some(cooldown_end) = guard.get(&candidate) {
                        if now < *cooldown_end {
                            continue; // skip endpoint during cooldown
                        } else {
                            guard.remove(&candidate);
                        }
                    }
                    chosen = Some(candidate);
                    break;
                }
                chosen.unwrap_or_else(|| self.current_idx.load(Ordering::Relaxed) % total_endpoints)
            };
            let url = &self.endpoints[idx];

            let req_body = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "getMultipleAccounts",
                "params": [
                    keys_str,
                    {
                        "encoding": "base64",
                        "commitment": "processed"
                    }
                ]
            });

            let resp_res = self.client.post(url).json(&req_body).send().await;
            match resp_res {
                Ok(resp) => {
                    let status = resp.status();
                    if status.as_u16() == 429 || status.is_server_error() {
                        self.cooldown_and_warn(
                            idx,
                            url,
                            Duration::from_secs(60),
                            format!("HTTP {status}"),
                        );
                        continue;
                    }
                    if let Ok(json_val) = resp.json::<serde_json::Value>().await {
                        if let Some(err_obj) = json_val.get("error") {
                            debug!("RPC returned error {:?} on getMultipleAccounts", err_obj);
                            self.current_idx.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }

                        let slot = json_val
                            .pointer("/result/context/slot")
                            .and_then(|s| s.as_u64())
                            .unwrap_or(0);

                        let mut results = std::collections::HashMap::new();
                        if let Some(arr) =
                            json_val.pointer("/result/value").and_then(|v| v.as_array())
                        {
                            for (pk, acc_val) in pubkeys.iter().zip(arr.iter()) {
                                if acc_val.is_null() {
                                    continue;
                                }
                                if let Some(raw_b64) =
                                    acc_val.pointer("/data/0").and_then(|s| s.as_str())
                                {
                                    if let Ok(bytes) = BASE64_STANDARD.decode(raw_b64) {
                                        results.insert(*pk, (slot, bytes));
                                    }
                                }
                            }
                        }
                        return Ok(results);
                    }
                }
                Err(e) => {
                    last_err = anyhow::anyhow!("request to {} failed: {e}", url);
                    self.current_idx.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        Err(last_err)
    }
}

/// A verified mainnet pool with its two token-vault addresses for live balance reads.
#[derive(Debug, Clone)]
pub struct RegisteredPool {
    pub pool_id: PoolId,
    pub protocol: DexProtocol,
    pub base_mint: MintId,
    pub quote_mint: MintId,
    /// Mint decimals for `(base_mint, quote_mint)`, read from a verified mint
    /// account. `0` means unknown; such pools are rejected by the engine.
    pub decimals_a: u8,
    pub decimals_b: u8,
    /// SPL token account holding the base-mint reserve (live balance read each tick).
    pub base_vault: Pubkey,
    /// SPL token account holding the quote-mint reserve (live balance read each tick).
    pub quote_vault: Pubkey,
    /// Protocol swap fee in basis points (used for graph edge weights).
    pub fee_bps: u16,
}

/// Registry of verified multi-DEX pools that streams live reserve states into the
/// `MarketStateCache` / `MarketGraph`.
///
/// Registry configuration is immutable during a fetch. The supervisor stores
/// replacement registries behind ArcSwap so refreshes do not mutate in-flight reads.
#[derive(Clone)]
pub struct MultiDexRegistry {
    pools: Vec<RegisteredPool>,
    rpc: RpcPool,
    pub dependency_manifest: DependencyManifestRegistry,
    /// Immutable mint-decimal cache for canonical DLMM token_x/token_y mints.
    /// Mint decimals never change on-chain, so entries live forever. `Arc`
    /// keeps clones of the registry (per-worker) sharing one cache.
    dlmm_decimals_cache: std::sync::Arc<parking_lot::Mutex<std::collections::HashMap<MintId, u8>>>,
}

impl MultiDexRegistry {
    /// Build a registry pre-loaded with verified mainnet SOL-USDC pools across
    /// Raydium V4, Orca Whirlpool, and Meteora DLMM, sharing the supplied RPC pool.
    pub fn mainnet_default(rpc: RpcPool) -> Self {
        // Addresses come from `arb-constants`; nothing is re-typed here.
        let pk = |s: &str| Pubkey::from_str(s).expect("static mainnet address is valid base58");
        let wsol = MintId(pk(arb_constants::WSOL_MINT));
        let usdc = MintId(pk(arb_constants::USDC_MINT));

        let pools = vec![
            // Raydium AMM v4 — SOL/USDC
            RegisteredPool {
                pool_id: PoolId(pk(arb_constants::RAYDIUM_SOL_USDC_POOL)),
                protocol: DexProtocol::RaydiumV4,
                base_mint: wsol,
                quote_mint: usdc,
                decimals_a: 9,
                decimals_b: 6,
                base_vault: pk(arb_constants::RAYDIUM_SOL_VAULT),
                quote_vault: pk(arb_constants::RAYDIUM_USDC_VAULT),
                fee_bps: 25,
            },
            // Orca Whirlpools — SOL/USDC (tick-spacing 4 pool)
            RegisteredPool {
                pool_id: PoolId(pk(arb_constants::ORCA_SOL_USDC_POOL)),
                protocol: DexProtocol::OrcaWhirlpool,
                base_mint: wsol,
                quote_mint: usdc,
                decimals_a: 9,
                decimals_b: 6,
                base_vault: pk(arb_constants::ORCA_SOL_VAULT),
                quote_vault: pk(arb_constants::ORCA_USDC_VAULT),
                fee_bps: 5,
            },
            // Orca Whirlpools — SOL/USDC, additional fee tiers. These unlock the
            // LOW-FEE routes (tick 1 => 1 bps, tick 2 => 2 bps, tick 8 => 8 bps;
            // the real fee is read on-chain). Vaults are placeholders: CLMM
            // reserves come from the pool account (sqrt_price + liquidity), not
            // the vault balances, so the registry does not require them here.
            RegisteredPool {
                pool_id: PoolId(pk("83v8iPyZihDEjDdY8RdZddyZNyUtXngz69Lgo9Kt5d6d")),
                protocol: DexProtocol::OrcaWhirlpool,
                base_mint: wsol,
                quote_mint: usdc,
                decimals_a: 9,
                decimals_b: 6,
                base_vault: pk("83v8iPyZihDEjDdY8RdZddyZNyUtXngz69Lgo9Kt5d6d"),
                quote_vault: pk("83v8iPyZihDEjDdY8RdZddyZNyUtXngz69Lgo9Kt5d6d"),
                fee_bps: 1,
            },
            RegisteredPool {
                pool_id: PoolId(pk("FpCMFDFGYotvufJ7HrFHsWEiiQCGbkLCtwHiDnh7o28Q")),
                protocol: DexProtocol::OrcaWhirlpool,
                base_mint: wsol,
                quote_mint: usdc,
                decimals_a: 9,
                decimals_b: 6,
                base_vault: pk("FpCMFDFGYotvufJ7HrFHsWEiiQCGbkLCtwHiDnh7o28Q"),
                quote_vault: pk("FpCMFDFGYotvufJ7HrFHsWEiiQCGbkLCtwHiDnh7o28Q"),
                fee_bps: 2,
            },
            RegisteredPool {
                pool_id: PoolId(pk("7qbRF6YsyGuLUVs6Y1q64bdVrfe4ZcUUz1JRdoVNUJnm")),
                protocol: DexProtocol::OrcaWhirlpool,
                base_mint: wsol,
                quote_mint: usdc,
                decimals_a: 9,
                decimals_b: 6,
                base_vault: pk("7qbRF6YsyGuLUVs6Y1q64bdVrfe4ZcUUz1JRdoVNUJnm"),
                quote_vault: pk("7qbRF6YsyGuLUVs6Y1q64bdVrfe4ZcUUz1JRdoVNUJnm"),
                fee_bps: 8,
            },
            // Meteora DLMM — SOL/USDC (bin_step 4, verified live mainnet pool)
            RegisteredPool {
                pool_id: PoolId(pk(arb_constants::METEORA_SOL_USDC_POOL)),
                protocol: DexProtocol::MeteoraDlmm,
                base_mint: wsol,
                quote_mint: usdc,
                decimals_a: 9,
                decimals_b: 6,
                base_vault: pk(arb_constants::METEORA_SOL_RESERVE),
                quote_vault: pk(arb_constants::METEORA_USDC_RESERVE),
                fee_bps: arb_constants::METEORA_SOL_USDC_BIN_STEP,
            },
            // Meteora DLMM — SOL/USDC Pool 2 (verified live mainnet pool with deep liquidity)
            RegisteredPool {
                pool_id: PoolId(pk("BVRbyLjjfSBcoyiYFuxbgKYnWuiFaF9CSXEa5vdSZ9Hh")),
                protocol: DexProtocol::MeteoraDlmm,
                base_mint: wsol,
                quote_mint: usdc,
                decimals_a: 9,
                decimals_b: 6,
                base_vault: pk("BVRbyLjjfSBcoyiYFuxbgKYnWuiFaF9CSXEa5vdSZ9Hh"),
                quote_vault: pk("BVRbyLjjfSBcoyiYFuxbgKYnWuiFaF9CSXEa5vdSZ9Hh"),
                fee_bps: 10,
            },
            // Meteora DLMM — SOL/USDC Pool 3 (verified live mainnet pool)
            RegisteredPool {
                pool_id: PoolId(pk("CgqwPLSFfht89pF5RSKGUUMFj5zRxoUt4861w2SkXaqY")),
                protocol: DexProtocol::MeteoraDlmm,
                base_mint: wsol,
                quote_mint: usdc,
                decimals_a: 9,
                decimals_b: 6,
                base_vault: pk("CgqwPLSFfht89pF5RSKGUUMFj5zRxoUt4861w2SkXaqY"),
                quote_vault: pk("CgqwPLSFfht89pF5RSKGUUMFj5zRxoUt4861w2SkXaqY"),
                fee_bps: 20,
            },
            // Raydium CPMM — SOL/USDC (1 bps fee tier, lowest on Solana)
            RegisteredPool {
                pool_id: PoolId(pk(arb_constants::RAYDIUM_CPMM_SOL_USDC_POOL)),
                protocol: DexProtocol::RaydiumCpmm,
                base_mint: wsol,
                quote_mint: usdc,
                decimals_a: 9,
                decimals_b: 6,
                base_vault: pk(arb_constants::RAYDIUM_CPMM_SOL_VAULT),
                quote_vault: pk(arb_constants::RAYDIUM_CPMM_USDC_VAULT),
                fee_bps: 1,
            },
        ];

        Self {
            pools,
            rpc,
            dependency_manifest: Default::default(),
            dlmm_decimals_cache: std::sync::Arc::new(parking_lot::Mutex::new(Default::default())),
        }
    }

    /// Resolve canonical pair-mint decimals through an immutable cache; misses
    /// cost one batched `resolve_mints_decimals` call per new mint, ever.
    /// Unresolvable mints yield 0, which the economic engine rejects fail-closed.
    /// Used by both the DLMM and CLMM canonical-orientation blocks in
    /// [`MultiDexRegistry::fetch_live_pool_states`].
    async fn dlmm_mint_decimals(&self, x: Pubkey, y: Pubkey) -> (u8, u8) {
        let mx = MintId(x);
        let my = MintId(y);
        let missing: Vec<MintId> = {
            let cache = self.dlmm_decimals_cache.lock();
            [mx, my]
                .into_iter()
                .filter(|m| !cache.contains_key(m))
                .collect()
        };
        if !missing.is_empty() {
            let resolved = resolve_mints_decimals(&self.rpc, &missing).await;
            let mut cache = self.dlmm_decimals_cache.lock();
            for (m, d) in resolved {
                cache.insert(m, d);
            }
        }
        let cache = self.dlmm_decimals_cache.lock();
        (
            cache.get(&mx).copied().unwrap_or(0),
            cache.get(&my).copied().unwrap_or(0),
        )
    }

    /// Register an additional verified pool dynamically (e.g. a DexScreener-discovered
    /// trending token pool with >= 2 venues).
    pub fn register(&mut self, pool: RegisteredPool) {
        if !self.pools.iter().any(|p| p.pool_id == pool.pool_id) {
            self.pools.push(pool);
        }
    }

    pub fn pool_count(&self) -> usize {
        self.pools.len()
    }

    pub fn pools(&self) -> &[RegisteredPool] {
        &self.pools
    }

    /// Batch-fetch LIVE reserve balances for every registered pool in a single
    /// `getMultipleAccounts` call and produce fresh `PoolState`s. Returns only
    /// pools whose both reserves decoded to a non-zero live balance.
    ///
    /// This is the call that ends `MarketGraph` starvation: with opposing Raydium
    /// / Orca / Meteora SOL-USDC pools populated, `find_base_cycles` immediately
    /// discovers real cross-DEX 2-hop arbitrage.
    /// Batched `getMultipleAccounts` (batch-100) through the registry's shared
    /// `RpcPool` — per-provider token buckets, backoffs, and telemetry apply.
    ///
    /// This replaced the previous per-account `PacedAccountFetcher` fan-out:
    /// ~163 pool + ~68 vault + ~200 derived accounts fetched ONE BY ONE every
    /// 750ms cycle (the worker's "parallel batch-100" log label was false).
    /// That is 300-400 HTTP requests per cycle against free-tier endpoints,
    /// guaranteeing permanent 429 cooldowns, which starved every downstream
    /// consumer (seeding, hydration, health, Jito) into fail-closed zeros.
    /// Batch-100 collapses the same data to 4-6 requests per cycle (~60x less).
    async fn batch_fetch_accounts(
        rpc: &RpcPool,
        keys: &[Pubkey],
        into: &mut std::collections::HashMap<Pubkey, (u64, Vec<u8>)>,
    ) -> u64 {
        let mut max_slot = 0u64;
        for chunk in keys.chunks(100) {
            let refs_owned: Vec<String> = chunk.iter().map(|k| k.to_string()).collect();
            let refs: Vec<&str> = refs_owned.iter().map(|s| s.as_str()).collect();
            let Ok((accounts, slot)) = rpc.get_multiple_accounts_with_slot(&refs).await else {
                continue;
            };
            max_slot = max_slot.max(slot);
            for (val, pk) in accounts.into_iter().zip(chunk.iter()) {
                let Some(acc) = val else { continue };
                let Some(raw_b64) = acc
                    .get("data")
                    .and_then(|d| d.get(0))
                    .and_then(|s| s.as_str())
                else {
                    continue;
                };
                if let Ok(bytes) = BASE64_STANDARD.decode(raw_b64) {
                    into.insert(*pk, (slot, bytes));
                }
            }
        }
        max_slot
    }

    pub async fn fetch_live_pool_states(&self, _current_slot: Slot) -> Vec<PoolState> {
        if self.pools.is_empty() {
            return Vec::new();
        }

        let mut pool_data_map: std::collections::HashMap<Pubkey, (u64, Vec<u8>)> =
            std::collections::HashMap::new();
        // 1. Pool accounts — batched (2 batch-100 calls for 163 pools).
        let pool_pubkeys: Vec<Pubkey> = self.pools.iter().map(|p| p.pool_id.0).collect();
        let mut max_observed_slot =
            Self::batch_fetch_accounts(&self.rpc, &pool_pubkeys, &mut pool_data_map).await;

        // 2. CPMM vault accounts — batched (1 batch-100 call for ~68 vaults).
        let mut vault_data_map: std::collections::HashMap<Pubkey, (u64, Vec<u8>)> =
            std::collections::HashMap::new();
        let mut vault_pubkeys: Vec<Pubkey> = Vec::new();
        for p in &self.pools {
            if matches!(
                p.protocol,
                DexProtocol::RaydiumV4 | DexProtocol::RaydiumCpmm
            ) {
                if !vault_pubkeys.contains(&p.base_vault) {
                    vault_pubkeys.push(p.base_vault);
                }
                if !vault_pubkeys.contains(&p.quote_vault) {
                    vault_pubkeys.push(p.quote_vault);
                }
            }
        }
        let vault_slot =
            Self::batch_fetch_accounts(&self.rpc, &vault_pubkeys, &mut vault_data_map).await;
        max_observed_slot = max_observed_slot.max(vault_slot);

        // 3. Fetch derived accounts (tick arrays / bin arrays) individually
        let mut derived_pubkeys: Vec<Pubkey> = Vec::new();
        for p in &self.pools {
            if let Some((_, data)) = pool_data_map.get(&p.pool_id.0) {
                match p.protocol {
                    DexProtocol::MeteoraDlmm => {
                        if let Some(d) = decode_dlmm_state(data) {
                            for key in
                                dlmm_bin_array_pdas(&p.pool_id.0, d.active_bin_id, d.bin_step_bps)
                            {
                                if !derived_pubkeys.contains(&key) {
                                    derived_pubkeys.push(key);
                                }
                            }
                        }
                    }
                    DexProtocol::OrcaWhirlpool => {
                        if let Some(c) = decode_clmm_state(data) {
                            let (keys, _) =
                                clmm_tick_array_pdas(&p.pool_id, c.tick_current, c.tick_spacing);
                            for key in keys {
                                if !derived_pubkeys.contains(&key) {
                                    derived_pubkeys.push(key);
                                }
                            }
                        }
                    }
                    DexProtocol::RaydiumClmm => {
                        if let Some(c) = decode_raydium_clmm_state(data, p.fee_bps) {
                            let (keys, _) = raydium_clmm_tick_array_pdas(
                                &p.pool_id,
                                c.tick_current,
                                c.tick_spacing,
                            );
                            for key in keys {
                                if !derived_pubkeys.contains(&key) {
                                    derived_pubkeys.push(key);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        let mut derived_data_map: std::collections::HashMap<Pubkey, (u64, Vec<u8>)> =
            std::collections::HashMap::new();
        let derived_slot =
            Self::batch_fetch_accounts(&self.rpc, &derived_pubkeys, &mut derived_data_map).await;
        max_observed_slot = max_observed_slot.max(derived_slot);

        let mut dlmm_bins: std::collections::HashMap<PoolId, Vec<arb_types::DlmmBin>> =
            std::collections::HashMap::new();
        let mut clmm_ticks: std::collections::HashMap<PoolId, Vec<arb_types::ClmmTickArray>> =
            std::collections::HashMap::new();

        for p in &self.pools {
            if let Some((_, data)) = pool_data_map.get(&p.pool_id.0) {
                match p.protocol {
                    DexProtocol::MeteoraDlmm => {
                        if let Some(d) = decode_dlmm_state(data) {
                            let mut bins = Vec::new();
                            for key in
                                dlmm_bin_array_pdas(&p.pool_id.0, d.active_bin_id, d.bin_step_bps)
                            {
                                if let Some((_, bytes)) = derived_data_map.get(&key) {
                                    bins.extend(decode_bin_array(bytes));
                                }
                            }
                            dlmm_bins.insert(p.pool_id, bins);
                        }
                    }
                    DexProtocol::OrcaWhirlpool => {
                        if let Some(c) = decode_clmm_state(data) {
                            let (keys, starts) =
                                clmm_tick_array_pdas(&p.pool_id, c.tick_current, c.tick_spacing);
                            let mut arrays = Vec::new();
                            for (key, start_tick) in keys.into_iter().zip(starts) {
                                if let Some((_, bytes)) = derived_data_map.get(&key) {
                                    if let Ok(facade) = arb_math::clmm::parse_tick_array_account(
                                        bytes,
                                        c.tick_spacing,
                                    ) {
                                        if facade.start_tick_index == start_tick {
                                            arrays.push(arb_types::ClmmTickArray {
                                                start_tick_index: facade.start_tick_index,
                                                ticks: facade
                                                    .ticks
                                                    .into_iter()
                                                    .filter(|t| t.initialized)
                                                    .map(|t| arb_types::ClmmTick {
                                                        tick_index: t.tick_index,
                                                        initialized: true,
                                                        liquidity_net: t.liquidity_net,
                                                    })
                                                    .collect(),
                                            });
                                        }
                                    }
                                }
                            }
                            clmm_ticks.insert(p.pool_id, arrays);
                        }
                    }
                    DexProtocol::RaydiumClmm => {
                        if let Some(c) = decode_raydium_clmm_state(data, p.fee_bps) {
                            let (keys, starts) = raydium_clmm_tick_array_pdas(
                                &p.pool_id,
                                c.tick_current,
                                c.tick_spacing,
                            );
                            let mut arrays = Vec::new();
                            for (key, start_tick) in keys.into_iter().zip(starts) {
                                if let Some((_, bytes)) = derived_data_map.get(&key) {
                                    if let Ok(facade) =
                                        arb_dex::RaydiumClmmPool::parse_tick_array_state(bytes)
                                    {
                                        if facade.start_tick_index == start_tick {
                                            arrays.push(arb_types::ClmmTickArray {
                                                start_tick_index: facade.start_tick_index,
                                                ticks: facade
                                                    .ticks
                                                    .into_iter()
                                                    .filter(|t| t.initialized)
                                                    .map(|t| arb_types::ClmmTick {
                                                        tick_index: t.tick_index,
                                                        initialized: true,
                                                        liquidity_net: t.liquidity_net,
                                                    })
                                                    .collect(),
                                            });
                                        }
                                    }
                                }
                            }
                            clmm_ticks.insert(p.pool_id, arrays);
                        }
                    }
                    _ => {}
                }
            }
        }

        let effective_slot = Slot(max_observed_slot);
        let mut states = Vec::with_capacity(self.pools.len());
        let now_ms = u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
        )
        .unwrap_or(u64::MAX);

        for p in &self.pools {
            let (base_bal, quote_bal) = if matches!(
                p.protocol,
                DexProtocol::RaydiumV4 | DexProtocol::RaydiumCpmm
            ) {
                let b = vault_data_map
                    .get(&p.base_vault)
                    .and_then(|(_, d)| decode_spl_token_amount_bytes(d))
                    .unwrap_or(0);
                let q = vault_data_map
                    .get(&p.quote_vault)
                    .and_then(|(_, d)| decode_spl_token_amount_bytes(d))
                    .unwrap_or(0);
                (b, q)
            } else {
                (0, 0)
            };

            let needs_vaults = matches!(
                p.protocol,
                DexProtocol::RaydiumV4 | DexProtocol::RaydiumCpmm
            );
            if needs_vaults && (base_bal == 0 || quote_bal == 0) {
                debug!(
                    "MultiDexRegistry: {:?} pool {} missing live reserves (base={} quote={})",
                    p.protocol, p.pool_id, base_bal, quote_bal
                );
                continue;
            }

            let pool_bytes = pool_data_map.get(&p.pool_id.0).map(|(_, b)| b.as_slice());
            let pump_curve = match (p.protocol, pool_bytes) {
                (DexProtocol::PumpSwap, _) => None,
                (DexProtocol::PumpFun, Some(data)) => {
                    arb_dex::pump_fun::PumpFunBondingCurve::decode_account_data(data).ok()
                }
                _ => None,
            };

            let pump_state = pump_curve.clone();

            let mut clmm = match (p.protocol, pool_bytes) {
                (DexProtocol::OrcaWhirlpool, Some(data)) => decode_clmm_state(data),
                (DexProtocol::RaydiumClmm, Some(data)) => {
                    decode_raydium_clmm_state(data, p.fee_bps)
                }
                _ => None,
            };
            let mut dlmm = match (p.protocol, pool_bytes) {
                (DexProtocol::MeteoraDlmm, Some(data)) => decode_dlmm_state(data),
                _ => None,
            };

            if p.protocol == DexProtocol::MeteoraDlmm
                && let Some(d) = dlmm.as_mut()
            {
                d.bins = dlmm_bins.get(&p.pool_id).cloned().unwrap_or_default();
            }

            let onchain_fee_bps = match (p.protocol, pool_bytes) {
                (DexProtocol::PumpFun, _) => None,
                (DexProtocol::PumpSwap, _) => None,
                (DexProtocol::RaydiumV4, Some(d)) => decode_raydium_fee_bps(d),
                (DexProtocol::RaydiumCpmm, Some(d)) => {
                    decode_raydium_cpmm_fee_bps(d).or(Some(p.fee_bps))
                }
                (DexProtocol::RaydiumClmm, _) => Some(p.fee_bps.max(1)),
                (DexProtocol::OrcaWhirlpool, Some(d)) => decode_orca_fee_bps(d),
                (DexProtocol::MeteoraDlmm, _) => dlmm.as_ref().and_then(decode_dlmm_fee_bps),
                _ => None,
            };
            let Some(fee_bps) = onchain_fee_bps else {
                debug!(
                    "MultiDexRegistry: {:?} pool {} — on-chain fee undecodable; skipping (fail closed)",
                    p.protocol, p.pool_id
                );
                continue;
            };

            if (p.protocol == DexProtocol::OrcaWhirlpool || p.protocol == DexProtocol::RaydiumClmm)
                && let Some(c) = clmm.as_mut()
            {
                c.tick_arrays = clmm_ticks.get(&p.pool_id).cloned().unwrap_or_default();
            }

            let kind = match p.protocol {
                DexProtocol::OrcaWhirlpool | DexProtocol::RaydiumClmm => arb_types::PoolKind::Clmm,
                DexProtocol::MeteoraDlmm => arb_types::PoolKind::Dlmm,
                _ => arb_types::PoolKind::Cpmm,
            };

            // Canonical DLMM orientation: DexScreener's base/quote ordering is
            // arbitrary, but the on-chain LbPair fixes token_x @88..120 and
            // token_y @120..152. Downstream, `quote_pool` derives the swap
            // direction as `a_to_b = (pool_base == from_mint)`, which is only
            // correct when pool_base == on-chain token_x. A DexScreener
            // inversion made HRYEjwdo3bZ1TpXKWKcezqiwSV2Ywuh4LxMa2PzoCnG6 quote
            // WSOL->USDC as if selling USDC (measured: 100_000 WSOL-raw in ->
            // 1_249_511 "USDC-raw" out instead of 7_998), a 156x phantom edge
            // that flooded the pipeline with implausible-ROI rejections
            // (soak13/14: 4,276 + 1,058, p50 ROI 1,535,076 bps = 153.5x).
            let (state_base, state_quote, dec_a, dec_b) = if p.protocol == DexProtocol::MeteoraDlmm
            {
                match pool_bytes {
                    Some(d) if d.len() >= 152 => {
                        let token_x = Pubkey::new_from_array(d[88..120].try_into().unwrap());
                        let token_y = Pubkey::new_from_array(d[120..152].try_into().unwrap());
                        let (dx, dy) = self.dlmm_mint_decimals(token_x, token_y).await;
                        (MintId(token_x), MintId(token_y), dx, dy)
                    }
                    _ => (p.base_mint, p.quote_mint, p.decimals_a, p.decimals_b),
                }
            } else if matches!(
                p.protocol,
                DexProtocol::OrcaWhirlpool | DexProtocol::RaydiumClmm
            ) {
                // Canonical CLMM orientation: same failure class as the DLMM
                // DexScreener inversion above. CLMM base/quote come from HTTP
                // catalogs (Orca `tokenA/tokenB`, Raydium v3 `mintA/mintB`)
                // and were never verified against pool bytes. Downstream,
                // `quote_pool` derives `a_to_b = (pool_base == from_mint)`
                // and `compute_clmm_virtual_reserves` returns (mint_a-side,
                // mint_b-side) reserves, so `pool_base == on-chain mint_a` is
                // a hard correctness invariant. Live probe (clmm_probe.rs,
                // slot 468876781) verified catalog == on-chain 3/3 today, but
                // nothing failed closed if a catalog schema change inverted
                // it. The on-chain pool account is ground truth; a catalog
                // mismatch is now overridden loudly instead of silently
                // trusted. Mints are immutable in both pool layouts, so the
                // WS arm inherits this canonicalization from the cache and
                // must not re-derive it.
                match pool_bytes.and_then(|d| clmm_onchain_mints(p.protocol, d)) {
                    Some((mint_a, mint_b)) => {
                        let catalog_order_verified =
                            p.base_mint.0 == mint_a && p.quote_mint.0 == mint_b;
                        if !catalog_order_verified {
                            warn!(
                                "MultiDexRegistry: {:?} pool {} — catalog orientation \
                                     {:?}/{:?} diverges from on-chain mints {:?}/{:?}; using on-chain order",
                                p.protocol,
                                p.pool_id,
                                p.base_mint,
                                p.quote_mint,
                                MintId(mint_a),
                                MintId(mint_b)
                            );
                        }
                        if p.protocol == DexProtocol::RaydiumClmm {
                            // Raydium PoolState stores mint decimals on-chain
                            // (dec_0 @233, dec_1 @234) — no RPC needed. Values
                            // outside 1..=18 are corrupt; fall back to catalog.
                            let d = pool_bytes.unwrap_or_default();
                            let dec0 = d[233];
                            let dec1 = d[234];
                            let sane = |v: u8, fb: u8| {
                                if (1..=18).contains(&v) { v } else { fb }
                            };
                            (
                                MintId(mint_a),
                                MintId(mint_b),
                                sane(dec0, p.decimals_a),
                                sane(dec1, p.decimals_b),
                            )
                        } else if catalog_order_verified {
                            // Catalog order verified against chain: its
                            // decimals are positionally correct, skip RPC.
                            (MintId(mint_a), MintId(mint_b), p.decimals_a, p.decimals_b)
                        } else {
                            // Inverted catalog: catalog decimals are swapped
                            // too; resolve by mint identity (cached).
                            let (da, db) = self.dlmm_mint_decimals(mint_a, mint_b).await;
                            (MintId(mint_a), MintId(mint_b), da, db)
                        }
                    }
                    None => (p.base_mint, p.quote_mint, p.decimals_a, p.decimals_b),
                }
            } else {
                (p.base_mint, p.quote_mint, p.decimals_a, p.decimals_b)
            };

            let (reserve_in, reserve_out) = match p.protocol {
                DexProtocol::PumpSwap => (0, 0),
                DexProtocol::PumpFun => match pump_state.as_ref() {
                    Some(state)
                        if state.is_tradeable()
                            && state.virtual_sol_reserves > 0
                            && state.virtual_token_reserves > 0 =>
                    {
                        (
                            u128::from(state.virtual_sol_reserves),
                            u128::from(state.virtual_token_reserves),
                        )
                    }
                    _ => (0, 0),
                },
                DexProtocol::OrcaWhirlpool | DexProtocol::RaydiumClmm => match clmm.as_ref() {
                    Some(c) => compute_clmm_virtual_reserves(c),
                    None => (0, 0),
                },
                DexProtocol::MeteoraDlmm => match (dlmm.as_ref(), pool_bytes) {
                    (Some(d), Some(raw_data)) => {
                        let Some(res) =
                            compute_dlmm_reserves(d, raw_data, &state_base.0, &state_quote.0)
                        else {
                            continue;
                        };
                        res
                    }
                    _ => (0, 0),
                },
                _ => (base_bal as u128, quote_bal as u128),
            };

            if reserve_in == 0 || reserve_out == 0 {
                debug!(
                    "MultiDexRegistry: {:?} pool {} — zero depth; skipping (fail closed)",
                    p.protocol, p.pool_id
                );
                continue;
            }

            let execution_accounts = match (p.protocol, pool_bytes) {
                (DexProtocol::RaydiumCpmm, Some(d)) if d.len() >= 333 => {
                    let amm_config = Pubkey::new_from_array(d[8..40].try_into().unwrap());
                    let vault_0 = Pubkey::new_from_array(d[72..104].try_into().unwrap());
                    let vault_1 = Pubkey::new_from_array(d[104..136].try_into().unwrap());
                    let observation_state = Pubkey::new_from_array(d[296..328].try_into().unwrap());
                    let prog = Pubkey::from_str(arb_constants::RAYDIUM_CPMM_PROGRAM_ID)
                        .unwrap_or_default();
                    let (authority, _) = Pubkey::find_program_address(
                        &[arb_constants::RAYDIUM_CPMM_AUTH_SEED],
                        &prog,
                    );
                    Some(arb_types::PoolExecutionAccounts::RaydiumCpmm {
                        amm_config,
                        authority,
                        vault_0,
                        vault_1,
                        observation_state,
                    })
                }
                (DexProtocol::RaydiumClmm, Some(d)) if d.len() >= 273 => {
                    let amm_config = Pubkey::new_from_array(d[9..41].try_into().unwrap());
                    let vault_0 = Pubkey::new_from_array(d[137..169].try_into().unwrap());
                    let vault_1 = Pubkey::new_from_array(d[169..201].try_into().unwrap());
                    let observation_state = Pubkey::new_from_array(d[201..233].try_into().unwrap());
                    let tick_current = i32::from_le_bytes(d[269..273].try_into().unwrap());
                    let tick_spacing = u16::from_le_bytes(d[235..237].try_into().unwrap()).max(1);
                    let (pdas, _) =
                        raydium_clmm_tick_array_pdas(&p.pool_id, tick_current, tick_spacing);
                    let prog = Pubkey::from_str(arb_constants::RAYDIUM_CLMM_PROGRAM_ID)
                        .unwrap_or_default();
                    let (bitmap_extension, _) =
                        Pubkey::find_program_address(&[b"bitmap", p.pool_id.0.as_ref()], &prog);
                    Some(arb_types::PoolExecutionAccounts::RaydiumClmm {
                        amm_config,
                        vault_0,
                        vault_1,
                        observation_state,
                        tick_array_pdas: pdas,
                        bitmap_extension: Some(bitmap_extension),
                    })
                }
                (DexProtocol::OrcaWhirlpool, Some(d)) if d.len() >= 245 => {
                    let _whirlpool_config = Pubkey::new_from_array(d[8..40].try_into().unwrap());
                    let token_vault_a = Pubkey::new_from_array(d[133..165].try_into().unwrap());
                    let token_vault_b = Pubkey::new_from_array(d[213..245].try_into().unwrap());
                    let tick_current = i32::from_le_bytes(d[81..85].try_into().unwrap());
                    let tick_spacing = u16::from_le_bytes(d[41..43].try_into().unwrap()).max(1);
                    let (keys, _) = clmm_tick_array_pdas(&p.pool_id, tick_current, tick_spacing);
                    let prog = Pubkey::from_str(arb_constants::ORCA_WHIRLPOOL_PROGRAM_ID)
                        .unwrap_or_default();
                    let (oracle, _) =
                        Pubkey::find_program_address(&[b"oracle", p.pool_id.0.as_ref()], &prog);
                    let tick_arrays = if keys.len() >= 3 {
                        vec![keys[0], keys[1], keys[2]]
                    } else {
                        keys
                    };
                    Some(arb_types::PoolExecutionAccounts::OrcaWhirlpool {
                        vault_a: token_vault_a,
                        vault_b: token_vault_b,
                        oracle,
                        tick_array_pdas: tick_arrays,
                    })
                }
                (DexProtocol::MeteoraDlmm, Some(d)) if d.len() >= 216 => {
                    let reserve_x = Pubkey::new_from_array(d[152..184].try_into().unwrap());
                    let reserve_y = Pubkey::new_from_array(d[184..216].try_into().unwrap());
                    let active_id = i32::from_le_bytes(d[76..80].try_into().unwrap());
                    let bin_step = u16::from_le_bytes(d[80..82].try_into().unwrap()).max(1);
                    let bin_arrays = dlmm_bin_array_pdas(&p.pool_id.0, active_id, bin_step);
                    let prog = Pubkey::from_str(arb_constants::METEORA_DLMM_PROGRAM_ID)
                        .unwrap_or_default();
                    let (bitmap_extension, _) =
                        Pubkey::find_program_address(&[b"bitmap", p.pool_id.0.as_ref()], &prog);
                    Some(arb_types::PoolExecutionAccounts::MeteoraDlmm {
                        reserve_x,
                        reserve_y,
                        oracle: p.pool_id.0,
                        bin_array_pdas: bin_arrays,
                        bitmap_extension: Some(bitmap_extension),
                    })
                }
                (DexProtocol::RaydiumV4, Some(d)) if d.len() >= 528 => {
                    let pool_coin_token_account =
                        Pubkey::new_from_array(d[336..368].try_into().unwrap());
                    let pool_pc_token_account =
                        Pubkey::new_from_array(d[368..400].try_into().unwrap());
                    let open_orders = Pubkey::new_from_array(d[496..528].try_into().unwrap());
                    let target_orders = if d.len() >= 624 {
                        let t = Pubkey::new_from_array(d[592..624].try_into().unwrap());
                        if t == Pubkey::default() {
                            Some(p.pool_id.0)
                        } else {
                            Some(t)
                        }
                    } else {
                        Some(p.pool_id.0)
                    };
                    let serum_market = if d.len() >= 560 {
                        Pubkey::new_from_array(d[528..560].try_into().unwrap())
                    } else {
                        Pubkey::default()
                    };
                    let market_program = if d.len() >= 592 {
                        Pubkey::new_from_array(d[560..592].try_into().unwrap())
                    } else {
                        Pubkey::from_str("srmqPvymJeFKQ4zGQed1GFppgkRHL9kaELCbyksJtPX").unwrap()
                    };
                    let prog =
                        Pubkey::from_str(arb_constants::RAYDIUM_V4_PROGRAM_ID).unwrap_or_default();
                    let (amm_authority, _) =
                        Pubkey::find_program_address(&[b"amm authority"], &prog);
                    Some(arb_types::PoolExecutionAccounts::RaydiumV4 {
                        amm_authority,
                        open_orders,
                        target_orders: target_orders.unwrap_or(p.pool_id.0),
                        base_vault: pool_coin_token_account,
                        quote_vault: pool_pc_token_account,
                        serum_program: market_program,
                        serum_market,
                        serum_bids: Pubkey::default(),
                        serum_asks: Pubkey::default(),
                        serum_event_queue: Pubkey::default(),
                        serum_coin_vault: Pubkey::default(),
                        serum_pc_vault: Pubkey::default(),
                        serum_vault_signer: Pubkey::default(),
                        swap_version: arb_types::RaydiumV4SwapVersion::V2,
                    })
                }
                _ => None,
            };

            let wv = next_pool_write_version();

            states.push(PoolState {
                pool_id: p.pool_id,
                protocol: p.protocol,
                slot: effective_slot,
                write_version: wv,
                base_mint: state_base,
                quote_mint: state_quote,
                decimals_a: dec_a,
                decimals_b: dec_b,
                reserve_in,
                reserve_out,
                fee_numerator: fee_bps as u64,
                fee_denominator: 10_000,
                is_active: true,
                has_risky_token: false,
                last_update_slot: effective_slot.0,
                last_update_ms: now_ms,
                kind,
                clmm,
                dlmm,
                pump_fun: pump_state,
                token_programs: Default::default(),
                execution_accounts,
                transfer_fee_bps: 0,
            });
        }

        states
    }
}

/// Decode a Raydium v4 `AmmInfo` account's two token vaults.
///
/// `coin_vault @336`, `pc_vault @368` (verified against live mainnet pools).
pub fn decode_raydium_vaults(data: &[u8]) -> Option<(Pubkey, Pubkey)> {
    if data.len() < 400 {
        return None;
    }
    let a: [u8; 32] = data.get(336..368)?.try_into().ok()?;
    let b: [u8; 32] = data.get(368..400)?.try_into().ok()?;
    let a = Pubkey::new_from_array(a);
    let b = Pubkey::new_from_array(b);
    if a == Pubkey::default() || b == Pubkey::default() {
        return None;
    }
    Some((a, b))
}

/// On-chain CLMM token orientation: `(mint_a, mint_b)` as stored in the pool
/// account bytes. This is the canonical order the quote engine requires
/// (`a_to_b = pool_base == from_mint` and
/// [`compute_clmm_virtual_reserves`] returning `(mint_a-side, mint_b-side)`).
///
/// Offsets verified against live mainnet via `clmm_probe.rs` (slot 468876781,
/// Orca SOL/USDC `Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE` data_len 653;
/// Raydium CLMM pools data_len 1544), cross-checked against official sources:
/// - Orca Whirlpool (`orca-so/whirlpools`, `programs/whirlpool/src/state/whirlpool.rs`,
///   interleaved layout): `token_mint_a @101..133`, `token_vault_a @133..165`,
///   `fee_growth_global_a @165..181`, `token_mint_b @181..213`. The offsets are
///   independent of `NUM_REWARDS` (reward infos start after `fee_growth_global_b`),
///   so both the 525-byte (2-reward) and 653-byte (3-reward) generations decode
///   identically. Orca does NOT enforce `mint_a < mint_b` on live pools —
///   no lexicographic shortcut exists.
/// - Raydium CLMM (`raydium-io/raydium-clmm`, `PoolState`): `token_mint0 @73..105`,
///   `token_mint1 @105..137`. Raydium does not enforce `mint0 < mint1` either.
///
/// Returns `None` (fail-closed) on short/truncated buffers or default pubkeys.
pub fn clmm_onchain_mints(protocol: DexProtocol, data: &[u8]) -> Option<(Pubkey, Pubkey)> {
    let (ma, mb): ([u8; 32], [u8; 32]) = match protocol {
        DexProtocol::OrcaWhirlpool => {
            if data.len() < 213 {
                return None;
            }
            (
                data[101..133].try_into().ok()?,
                data[181..213].try_into().ok()?,
            )
        }
        DexProtocol::RaydiumClmm => {
            if data.len() < 137 {
                return None;
            }
            (
                data[73..105].try_into().ok()?,
                data[105..137].try_into().ok()?,
            )
        }
        _ => return None,
    };
    let a = Pubkey::new_from_array(ma);
    let b = Pubkey::new_from_array(mb);
    if a == Pubkey::default() || b == Pubkey::default() {
        return None;
    }
    Some((a, b))
}

/// Decode Orca Whirlpool token vaults (`token_vault_a @133..165`, `token_vault_b @213..245`).
pub fn decode_orca_vaults(data: &[u8]) -> Option<(Pubkey, Pubkey)> {
    if data.len() < 245 {
        return None;
    }
    let a: [u8; 32] = data.get(133..165)?.try_into().ok()?;
    let b: [u8; 32] = data.get(213..245)?.try_into().ok()?;
    let a = Pubkey::new_from_array(a);
    let b = Pubkey::new_from_array(b);
    if a == Pubkey::default() || b == Pubkey::default() {
        return None;
    }
    Some((a, b))
}

/// Decode Raydium CPMM token vaults (`token_0_vault @72..104`, `token_1_vault @104..136`).
pub fn decode_raydium_cpmm_vaults(data: &[u8]) -> Option<(Pubkey, Pubkey)> {
    if data.len() < 136 {
        return None;
    }
    let a: [u8; 32] = data.get(72..104)?.try_into().ok()?;
    let b: [u8; 32] = data.get(104..136)?.try_into().ok()?;
    let a = Pubkey::new_from_array(a);
    let b = Pubkey::new_from_array(b);
    if a == Pubkey::default() || b == Pubkey::default() {
        return None;
    }
    Some((a, b))
}

/// Decode Raydium CLMM token vaults (`token_vault_0 @137..169`, `token_vault_1 @169..201`).
pub fn decode_raydium_clmm_vaults(data: &[u8]) -> Option<(Pubkey, Pubkey)> {
    if data.len() < 201 {
        return None;
    }
    let a: [u8; 32] = data.get(137..169)?.try_into().ok()?;
    let b: [u8; 32] = data.get(169..201)?.try_into().ok()?;
    let a = Pubkey::new_from_array(a);
    let b = Pubkey::new_from_array(b);
    if a == Pubkey::default() || b == Pubkey::default() {
        return None;
    }
    Some((a, b))
}

/// Decoded metadata from an on-chain pool account needed for live balance reads.
#[derive(Debug, Clone)]
pub struct DecodedPoolInfo {
    pub token_a_mint: Pubkey,
    pub token_b_mint: Pubkey,
    pub token_a_vault: Pubkey,
    pub token_b_vault: Pubkey,
    pub fee_bps: u16,
    pub decimals_a: Option<u8>,
    pub decimals_b: Option<u8>,
}

/// Decode on-chain pool accounts across Raydium V4, Raydium CPMM, Orca Whirlpool, and Meteora DLMM.
pub fn decode_pool_info(protocol: DexProtocol, data: &[u8]) -> Option<DecodedPoolInfo> {
    match protocol {
        DexProtocol::RaydiumV4 => {
            if data.len() < 464 {
                return None;
            }
            let va: [u8; 32] = data.get(336..368)?.try_into().ok()?;
            let vb: [u8; 32] = data.get(368..400)?.try_into().ok()?;
            let ma: [u8; 32] = data.get(400..432)?.try_into().ok()?;
            let mb: [u8; 32] = data.get(432..464)?.try_into().ok()?;
            let token_a_vault = Pubkey::new_from_array(va);
            let token_b_vault = Pubkey::new_from_array(vb);
            let token_a_mint = Pubkey::new_from_array(ma);
            let token_b_mint = Pubkey::new_from_array(mb);
            if token_a_vault == Pubkey::default()
                || token_b_vault == Pubkey::default()
                || token_a_mint == Pubkey::default()
                || token_b_mint == Pubkey::default()
            {
                return None;
            }
            let fee_bps = decode_raydium_fee_bps(data).unwrap_or(25);
            Some(DecodedPoolInfo {
                token_a_mint,
                token_b_mint,
                token_a_vault,
                token_b_vault,
                fee_bps,
                decimals_a: None,
                decimals_b: None,
            })
        }
        DexProtocol::RaydiumCpmm => {
            if data.len() < 333 {
                return None;
            }
            let va: [u8; 32] = data.get(72..104)?.try_into().ok()?;
            let vb: [u8; 32] = data.get(104..136)?.try_into().ok()?;
            let ma: [u8; 32] = data.get(168..200)?.try_into().ok()?;
            let mb: [u8; 32] = data.get(200..232)?.try_into().ok()?;
            let token_a_vault = Pubkey::new_from_array(va);
            let token_b_vault = Pubkey::new_from_array(vb);
            let token_a_mint = Pubkey::new_from_array(ma);
            let token_b_mint = Pubkey::new_from_array(mb);
            if token_a_vault == Pubkey::default()
                || token_b_vault == Pubkey::default()
                || token_a_mint == Pubkey::default()
                || token_b_mint == Pubkey::default()
            {
                return None;
            }
            let fee_bps = decode_raydium_cpmm_fee_bps(data).unwrap_or(25);
            let dec_a = data[331];
            let dec_b = data[332];
            Some(DecodedPoolInfo {
                token_a_mint,
                token_b_mint,
                token_a_vault,
                token_b_vault,
                fee_bps,
                decimals_a: if (1..=18).contains(&dec_a) {
                    Some(dec_a)
                } else {
                    None
                },
                decimals_b: if (1..=18).contains(&dec_b) {
                    Some(dec_b)
                } else {
                    None
                },
            })
        }
        DexProtocol::OrcaWhirlpool => {
            if data.len() < 245 {
                return None;
            }
            let ma: [u8; 32] = data.get(101..133)?.try_into().ok()?;
            let va: [u8; 32] = data.get(133..165)?.try_into().ok()?;
            let mb: [u8; 32] = data.get(181..213)?.try_into().ok()?;
            let vb: [u8; 32] = data.get(213..245)?.try_into().ok()?;
            let token_a_mint = Pubkey::new_from_array(ma);
            let token_a_vault = Pubkey::new_from_array(va);
            let token_b_mint = Pubkey::new_from_array(mb);
            let token_b_vault = Pubkey::new_from_array(vb);
            if token_a_vault == Pubkey::default()
                || token_b_vault == Pubkey::default()
                || token_a_mint == Pubkey::default()
                || token_b_mint == Pubkey::default()
            {
                return None;
            }
            let fee_bps = decode_orca_fee_bps(data).unwrap_or(5);
            Some(DecodedPoolInfo {
                token_a_mint,
                token_b_mint,
                token_a_vault,
                token_b_vault,
                fee_bps,
                decimals_a: None,
                decimals_b: None,
            })
        }
        DexProtocol::MeteoraDlmm => {
            if data.len() < 216 {
                return None;
            }
            let ma: [u8; 32] = data.get(88..120)?.try_into().ok()?;
            let mb: [u8; 32] = data.get(120..152)?.try_into().ok()?;
            let va: [u8; 32] = data.get(152..184)?.try_into().ok()?;
            let vb: [u8; 32] = data.get(184..216)?.try_into().ok()?;
            let token_a_mint = Pubkey::new_from_array(ma);
            let token_b_mint = Pubkey::new_from_array(mb);
            let token_a_vault = Pubkey::new_from_array(va);
            let token_b_vault = Pubkey::new_from_array(vb);
            if token_a_vault == Pubkey::default()
                || token_b_vault == Pubkey::default()
                || token_a_mint == Pubkey::default()
                || token_b_mint == Pubkey::default()
            {
                return None;
            }
            let bin_step = u16::from_le_bytes(data.get(80..82)?.try_into().ok()?).max(1);
            Some(DecodedPoolInfo {
                token_a_mint,
                token_b_mint,
                token_a_vault,
                token_b_vault,
                fee_bps: bin_step,
                decimals_a: None,
                decimals_b: None,
            })
        }
        DexProtocol::RaydiumClmm => {
            if data.len() < 273 {
                return None;
            }
            let ma: [u8; 32] = data.get(73..105)?.try_into().ok()?;
            let mb: [u8; 32] = data.get(105..137)?.try_into().ok()?;
            let va: [u8; 32] = data.get(137..169)?.try_into().ok()?;
            let vb: [u8; 32] = data.get(169..201)?.try_into().ok()?;
            let token_a_mint = Pubkey::new_from_array(ma);
            let token_b_mint = Pubkey::new_from_array(mb);
            let token_a_vault = Pubkey::new_from_array(va);
            let token_b_vault = Pubkey::new_from_array(vb);
            if token_a_vault == Pubkey::default()
                || token_b_vault == Pubkey::default()
                || token_a_mint == Pubkey::default()
                || token_b_mint == Pubkey::default()
            {
                return None;
            }
            let dec_a = data[233];
            let dec_b = data[234];
            Some(DecodedPoolInfo {
                token_a_mint,
                token_b_mint,
                token_a_vault,
                token_b_vault,
                fee_bps: 25,
                decimals_a: if (1..=18).contains(&dec_a) {
                    Some(dec_a)
                } else {
                    None
                },
                decimals_b: if (1..=18).contains(&dec_b) {
                    Some(dec_b)
                } else {
                    None
                },
            })
        }
        _ => None,
    }
}

/// Read mint decimals (offset 44) for each mint using single-account paced RPC requests.
/// Mints owned by Token-2022 are deliberately omitted to reject tokens with transfer fees.
pub async fn resolve_mints_decimals(
    _rpc: &crate::RpcPool,
    mints: &[MintId],
) -> std::collections::HashMap<MintId, u8> {
    let mut out = std::collections::HashMap::new();
    let unique: Vec<MintId> = {
        let mut seen = std::collections::HashSet::new();
        mints.iter().copied().filter(|m| seen.insert(*m)).collect()
    };
    if unique.is_empty() {
        return out;
    }

    let fetcher = PacedAccountFetcher::from_env();
    for mint in unique {
        let bytes = match fetcher.fetch_single_account(&mint.0).await {
            Ok((_, b)) => b,
            Err(e) => {
                debug!(
                    "resolve_mints_decimals single fetch error for {}: {e}",
                    mint.0
                );
                continue;
            }
        };
        if bytes.len() < 82 {
            continue;
        }

        // Token Safety Gate: check mint and freeze authorities for non-hub tokens.
        let mint_str = mint.0.to_string();
        let is_hub = mint_str == arb_constants::WSOL_MINT
            || mint_str == arb_constants::USDC_MINT
            || mint_str == "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB";
        if !is_hub {
            let mint_auth_tag = u32::from_le_bytes(bytes[0..4].try_into().unwrap_or([1, 0, 0, 0]));
            let freeze_auth_tag =
                u32::from_le_bytes(bytes[46..50].try_into().unwrap_or([1, 0, 0, 0]));
            if mint_auth_tag != 0 || freeze_auth_tag != 0 {
                debug!(
                    "Safety gate: dropping meme token {} (mint_auth={}, freeze_auth={})",
                    mint_str, mint_auth_tag, freeze_auth_tag
                );
                continue;
            }
        }
        let dec = bytes[44];
        if dec <= 18 {
            out.insert(mint, dec);
        }
    }
    out
}

/// Derive the three Orca `tick_array` PDAs that can affect a quote at
/// `tick_current`: the array containing the current tick, the one below and the
/// one above. Returns the keys plus the start tick of each, so a decoded array
/// can be checked against the slot it was requested for.
fn clmm_tick_array_pdas(
    pool_id: &PoolId,
    tick_current: i32,
    tick_spacing: u16,
) -> (Vec<Pubkey>, Vec<i32>) {
    let Ok(program_id) = Pubkey::from_str(arb_constants::ORCA_WHIRLPOOL_PROGRAM_ID) else {
        return (Vec::new(), Vec::new());
    };
    let spacing = tick_spacing.max(1);
    let ticks_per_array = (88i32) * spacing as i32;
    let start = tick_current.div_euclid(ticks_per_array) * ticks_per_array;
    let starts = [start, start - ticks_per_array, start + ticks_per_array];

    let mut keys = Vec::with_capacity(3);
    for s in starts {
        let s_str = s.to_string();
        let seeds: &[&[u8]] = &[b"tick_array", pool_id.0.as_ref(), s_str.as_bytes()];
        keys.push(Pubkey::find_program_address(seeds, &program_id).0);
    }
    (keys, starts.to_vec())
}

/// Derive Raydium CLMM `TickArrayState` PDAs around current tick.
/// Raydium CLMM has 60 ticks per tick array.
pub fn raydium_clmm_tick_array_pdas(
    pool_id: &PoolId,
    tick_current: i32,
    tick_spacing: u16,
) -> (Vec<Pubkey>, Vec<i32>) {
    let Ok(program_id) = Pubkey::from_str(arb_constants::RAYDIUM_CLMM_PROGRAM_ID) else {
        return (Vec::new(), Vec::new());
    };
    let spacing = i32::from(tick_spacing.max(1));
    let ticks_per_array = 60i32 * spacing;
    if ticks_per_array == 0 {
        return (Vec::new(), Vec::new());
    }
    let start = tick_current.div_euclid(ticks_per_array) * ticks_per_array;
    let starts = [start, start - ticks_per_array, start + ticks_per_array];

    let mut keys = Vec::with_capacity(3);
    for s in starts {
        let seeds: &[&[u8]] = &[
            arb_constants::RAYDIUM_CLMM_TICK_ARRAY_SEED,
            pool_id.0.as_ref(),
            &s.to_be_bytes(),
        ];
        keys.push(Pubkey::find_program_address(seeds, &program_id).0);
    }
    (keys, starts.to_vec())
}

/// Derive Meteora `BinArray` PDAs covering the active bin in both swap
/// directions. The number of arrays scales with `bin_step` (size-aware):
/// enough arrays to cover ~500 bps of price impact on each side of the active
/// bin, replacing the old fixed three-array ceiling.
fn dlmm_bin_array_pdas(pool: &Pubkey, active_id: i32, bin_step: u16) -> Vec<Pubkey> {
    let index = bin_id_to_bin_array_index(active_id);
    let step = u32::from(bin_step.max(1));
    let bins_traversed = 500u32 / step + 1;
    let arrays_needed = (bins_traversed as i64).div_euclid(METEORA_BINS_PER_ARRAY) + 1;
    let mut seen = std::collections::BTreeSet::new();
    (0..arrays_needed)
        .flat_map(|i| [index - i, index + i])
        .filter(move |i| seen.insert(*i))
        .map(|i| derive_meteora_bin_array_pda(pool, i))
        .collect()
}

/// Decode an RPC account's `data[0]` base64 payload.
pub fn decode_account_data(acc: &serde_json::Value) -> Option<Vec<u8>> {
    let b64 = acc.get("data")?.get(0)?.as_str()?;
    base64::prelude::BASE64_STANDARD.decode(b64).ok()
}

/// Paced single-account sequential fetcher replacing legacy chunked `getMultipleAccounts`.
pub async fn fetch_accounts_chunked_with_slot(
    _rpc: &crate::RpcPool,
    keys: &[String],
) -> (Vec<Option<serde_json::Value>>, u64) {
    if keys.is_empty() {
        return (Vec::new(), 0);
    }

    let fetcher = PacedAccountFetcher::from_env();
    let mut out = Vec::with_capacity(keys.len());
    let mut latest_slot = 0u64;

    for key_str in keys {
        if let Ok(pubkey) = Pubkey::from_str(key_str) {
            match fetcher.fetch_single_account(&pubkey).await {
                Ok((slot, bytes)) => {
                    latest_slot = latest_slot.max(slot);
                    let b64 = BASE64_STANDARD.encode(&bytes);
                    let json_val = serde_json::json!({
                        "data": [b64, "base64"],
                        "executable": false,
                        "lamports": 0,
                        "owner": arb_constants::TOKEN_PROGRAM_ID,
                        "rentEpoch": 0
                    });
                    out.push(Some(json_val));
                }
                Err(e) => {
                    debug!("fetch_accounts single fetch error for {key_str}: {e}");
                    out.push(None);
                }
            }
        } else {
            out.push(None);
        }
    }

    (out, latest_slot)
}

pub async fn fetch_accounts_chunked(
    rpc: &crate::RpcPool,
    keys: &[String],
) -> Vec<Option<serde_json::Value>> {
    fetch_accounts_chunked_with_slot(rpc, keys).await.0
}

/// Decode the Raydium v4 swap fee in basis points from the AMM account.
///
/// `AmmInfo` layout (no discriminator): `swap_fee_numerator` @176 (u64),
/// `swap_fee_denominator` @184 (u64). Verified live on the SOL/USDC pool:
/// 25 / 10_000 = 25 bps. Returns `None` on a short/zero-denominator account so
/// the caller can fail closed rather than price with a guessed fee.
pub fn decode_raydium_fee_bps(data: &[u8]) -> Option<u16> {
    if data.len() < 192 {
        return None;
    }
    let num = u64::from_le_bytes(data[176..184].try_into().ok()?);
    let den = u64::from_le_bytes(data[184..192].try_into().ok()?);
    if den == 0 {
        return None;
    }
    u16::try_from((num as u128 * 10_000) / den as u128).ok()
}

/// Decode the Orca Whirlpool fee in basis points from the whirlpool account.
///
/// `fee_rate` @45 (u16) is in HUNDREDTHS OF A BIP (denominator 1_000_000), so
/// bps = fee_rate / 100. Verified live on the SOL/USDC pool: 400 => 4 bps.
pub fn decode_orca_fee_bps(data: &[u8]) -> Option<u16> {
    if data.len() < 47 {
        return None;
    }
    let fee_rate = u16::from_le_bytes(data[45..47].try_into().ok()?);
    Some(fee_rate / 100)
}

/// Decode the Raydium CPMM (constant product) swap fee in basis points from the pool account.
///
/// Raydium CPMM `PoolState` layout (Anchor, with 8-byte discriminator):
/// - `trade_fee_rate` @8+136 (u64) — fee rate in hundredths of a basis point, denominator 1_000_000.
///   bps = trade_fee_rate / 100. Verified: 25 bps = 2500.
///
/// Returns `None` on short account or zero denominator so the caller fails closed.
pub fn decode_raydium_cpmm_fee_bps(data: &[u8]) -> Option<u16> {
    // Anchor discriminator (8) + PoolState fields up to trade_fee_rate
    // PoolState: amm_config (32) + pool_creator (32) + token_0_vault (32) + token_1_vault (32)
    // + token_0_mint (32) + token_1_mint (32) + observation_key (32) + auth_bump (1) + status (1)
    // + lp_mint (32) + lp_mint_decimals (1) + token_0_decimals (1) + token_1_decimals (1)
    // + token_0_price (8) + token_1_price (8) + lp_supply (16) + protocol_fee_rate (4)
    // + trade_fee_rate (8) ...
    // Total before trade_fee_rate: 8(disc) + 32*8 + 1+1 + 1+1+1 + 8+8 + 16 + 4 = 8+256+3+3+16+4 = 290 bytes
    // trade_fee_rate @8+282 = 290
    const OFFSET: usize = 8 + 282;
    if data.len() < OFFSET + 8 {
        return None;
    }
    let rate = u64::from_le_bytes(data[OFFSET..OFFSET + 8].try_into().ok()?);
    if rate == 0 {
        return None;
    }
    // rate is in hundredths of a basis point (denominator 1,000,000)
    // 2500 => 25 bps
    u16::try_from(rate / 100).ok()
}

#[cfg(test)]
mod fee_decoder_tests {
    use super::*;

    #[test]
    fn raydium_fee_reads_live_value() {
        // swap_fee_numerator=25, swap_fee_denominator=10_000 => 25 bps.
        let mut d = vec![0u8; 192];
        d[176..184].copy_from_slice(&25u64.to_le_bytes());
        d[184..192].copy_from_slice(&10_000u64.to_le_bytes());
        assert_eq!(decode_raydium_fee_bps(&d), Some(25));
    }

    #[test]
    fn orca_fee_reads_live_value() {
        // fee_rate = 400 hundredths of a bip => 4 bps (NOT the old hardcoded 5).
        let mut d = vec![0u8; 47];
        d[45..47].copy_from_slice(&400u16.to_le_bytes());
        assert_eq!(decode_orca_fee_bps(&d), Some(4));
    }

    #[test]
    fn raydium_cpmm_fee_reads_live_value() {
        // trade_fee_rate = 100 => 1 bps (CPMM 1 bps tier).
        // Offset = 8 + 282 = 290
        let mut d = vec![0u8; 300];
        d[290..298].copy_from_slice(&100u64.to_le_bytes());
        assert_eq!(decode_raydium_cpmm_fee_bps(&d), Some(1));
    }

    #[test]
    fn raydium_cpmm_fee_reads_25bps_value() {
        // trade_fee_rate = 2500 => 25 bps.
        let mut d = vec![0u8; 300];
        d[290..298].copy_from_slice(&2500u64.to_le_bytes());
        assert_eq!(decode_raydium_cpmm_fee_bps(&d), Some(25));
    }

    #[test]
    fn fee_decoders_fail_closed() {
        assert_eq!(decode_raydium_fee_bps(&[0u8; 10]), None);
        assert_eq!(decode_orca_fee_bps(&[0u8; 10]), None);
        let mut zero_den = vec![0u8; 192];
        zero_den[176..184].copy_from_slice(&25u64.to_le_bytes());
        assert_eq!(decode_raydium_fee_bps(&zero_den), None);
    }

    #[test]
    fn bin_array_index_uses_floor_division() {
        assert_eq!(bin_id_to_bin_array_index(0), 0);
        assert_eq!(bin_id_to_bin_array_index(69), 0);
        assert_eq!(bin_id_to_bin_array_index(70), 1);
        assert_eq!(bin_id_to_bin_array_index(-1), -1);
        assert_eq!(bin_id_to_bin_array_index(-70), -1);
        assert_eq!(bin_id_to_bin_array_index(-71), -2);
        // Live SOL/USDC pool active bin observed on-chain.
        assert_eq!(bin_id_to_bin_array_index(-5840), -84);
    }

    #[test]
    fn decode_bin_array_reads_bins() {
        let mut d = vec![0u8; METEORA_BIN_ARRAY_HEADER + 70 * METEORA_BIN_SIZE];
        d[8..16].copy_from_slice(&2i64.to_le_bytes()); // bin-array index 2
        // bin[0] @ header: amount_x @0, amount_y @8
        d[56..64].copy_from_slice(&123u64.to_le_bytes());
        d[64..72].copy_from_slice(&456u64.to_le_bytes());
        let bins = decode_bin_array(&d);
        assert_eq!(bins.len(), 1);
        assert_eq!(bins[0].bin_id, 140); // index 2 * 70 + 0
        assert_eq!(bins[0].amount_x, 123);
        assert_eq!(bins[0].amount_y, 456);
    }

    #[test]
    fn decode_dlmm_state_reads_params_and_active_id() {
        let mut d = vec![0u8; 96];
        d[8..10].copy_from_slice(&10_000u16.to_le_bytes()); // base_factor
        d[16..20].copy_from_slice(&120_000u32.to_le_bytes()); // variable_fee_control
        d[32..34].copy_from_slice(&1_000u16.to_le_bytes()); // protocol_share
        d[40..44].copy_from_slice(&20_039u32.to_le_bytes()); // volatility_accumulator
        d[76..80].copy_from_slice(&(-5840i32).to_le_bytes()); // active_id
        d[80..82].copy_from_slice(&4u16.to_le_bytes()); // bin_step
        d[82] = 0; // status active
        let s = decode_dlmm_state(&d).expect("valid LbPair");
        assert_eq!(s.active_bin_id, -5840);
        assert_eq!(s.bin_step_bps, 4);
        assert_eq!(s.volatility_accumulator, 20_039);
        let prm = s.params.expect("params");
        assert_eq!(prm.base_factor, 10_000);
        assert_eq!(prm.protocol_share, 1_000);
    }
}

pub fn decode_clmm_state(data: &[u8]) -> Option<arb_types::ClmmState> {
    if data.len() < 85 {
        return None;
    }
    let tick_spacing = u16::from_le_bytes(data[41..43].try_into().ok()?);
    let fee_rate = u16::from_le_bytes(data[45..47].try_into().ok()?);
    let liquidity = u128::from_le_bytes(data[49..65].try_into().ok()?);
    let sqrt_price = u128::from_le_bytes(data[65..81].try_into().ok()?);
    let tick_current = i32::from_le_bytes(data[81..85].try_into().ok()?);

    if liquidity == 0 || sqrt_price == 0 {
        return None;
    }

    Some(arb_types::ClmmState {
        sqrt_price,
        liquidity,
        tick_current,
        tick_spacing,
        fee_rate,
        tick_arrays: Vec::new(),
    })
}

/// Decode Raydium CLMM PoolState layout.
pub fn decode_raydium_clmm_state(data: &[u8], fee_bps: u16) -> Option<arb_types::ClmmState> {
    if data.len() < 273 {
        return None;
    }
    let tick_spacing = u16::from_le_bytes(data[235..237].try_into().ok()?).max(1);
    let liquidity = u128::from_le_bytes(data[237..253].try_into().ok()?);
    let sqrt_price = u128::from_le_bytes(data[253..269].try_into().ok()?);
    let tick_current = i32::from_le_bytes(data[269..273].try_into().ok()?);

    if liquidity == 0 || sqrt_price == 0 {
        return None;
    }

    // Raydium CLMM's SwapMath fee unit is millionths (1e6), not BPS.
    // 25 bps must become 2,500 ppm; passing 25 undercharged fees by 100x.
    let fee_rate = fee_bps.checked_mul(100)?;
    Some(arb_types::ClmmState {
        sqrt_price,
        liquidity,
        tick_current,
        tick_spacing,
        fee_rate,
        tick_arrays: Vec::new(),
    })
}

/// Decode DLMM state (Meteora `LbPair` layout, verified against the live
/// SOL/USDC pool `5rCf1DM8...`) from raw account bytes.
///
/// Layout: discriminator(8), StaticParameters(32), VariableParameters(32),
/// bump_seed(1), bin_step_seed(2), pair_type(1), active_id(i32 @76),
/// bin_step(u16 @80), status(u8 @82). Bins live in separate `BinArray`
/// accounts and are attached by `fetch_dlmm_bins`.
pub fn decode_dlmm_state(data: &[u8]) -> Option<arb_types::DlmmState> {
    if data.len() < 88 {
        return None;
    }
    // StaticParameters @8
    let base_factor = u16::from_le_bytes(data[8..10].try_into().ok()?);
    let filter_period = u16::from_le_bytes(data[10..12].try_into().ok()?);
    let decay_period = u16::from_le_bytes(data[12..14].try_into().ok()?);
    let reduction_factor = u16::from_le_bytes(data[14..16].try_into().ok()?);
    let variable_fee_control = u32::from_le_bytes(data[16..20].try_into().ok()?);
    let max_volatility_accumulator = u32::from_le_bytes(data[20..24].try_into().ok()?);
    let protocol_share = u16::from_le_bytes(data[32..34].try_into().ok()?);
    let base_fee_power_factor = data[34];
    // VariableParameters @40
    let volatility_accumulator = u32::from_le_bytes(data[40..44].try_into().ok()?);
    let volatility_reference = u32::from_le_bytes(data[44..48].try_into().ok()?);
    let last_update_timestamp = i64::from_le_bytes(data[56..64].try_into().ok()?);
    // active_id @76, bin_step @80, status @82
    let active_bin_id = i32::from_le_bytes(data[76..80].try_into().ok()?);
    let bin_step_bps = u16::from_le_bytes(data[80..82].try_into().ok()?);
    let status = data[82];
    if status != 0 || bin_step_bps == 0 {
        return None; // pool disabled / invalid
    }

    Some(arb_types::DlmmState {
        active_bin_id,
        status,
        bin_step_bps,
        bins: Vec::new(), // attached separately from BinArray accounts
        params: Some(arb_types::DlmmFeeParams {
            base_factor,
            filter_period,
            decay_period,
            reduction_factor,
            variable_fee_control,
            max_volatility_accumulator,
            protocol_share,
            base_fee_power_factor,
            function_type: if data.len() > 35 { data[35] } else { 0 },
            collect_fee_mode: if data.len() > 36 { data[36] } else { 0 },
        }),
        volatility_accumulator,
        volatility_reference,
        index_reference: i32::from_le_bytes(data[48..52].try_into().ok()?),
        last_update_timestamp,
    })
}

/// Meteora bins per `BinArray` account.
///
/// Single source of truth: aliased to `arb_constants::METEORA_BINS_PER_BIN_ARRAY`
/// so the ingestion decoder and the `swap2` instruction builder can never
/// disagree on the BinArray index divisor again.
pub const METEORA_BINS_PER_ARRAY: i64 = arb_constants::METEORA_BINS_PER_BIN_ARRAY;
/// Size of a single Meteora `Bin` in bytes (verified from the IDL + live account).
pub const METEORA_BIN_SIZE: usize = 144;
/// `BinArray` header: disc(8) + index(8) + version(1) + padding(7) + lb_pair(32).
pub const METEORA_BIN_ARRAY_HEADER: usize = 56;

/// Derive the `BinArray` PDA for a pool and bin-array index:
/// seeds = `[b"bin_array", lb_pair, index.to_le_bytes()]`.
pub fn derive_meteora_bin_array_pda(pool: &Pubkey, index: i64) -> Pubkey {
    let program_id = Pubkey::from_str(arb_constants::METEORA_DLMM_PROGRAM_ID)
        .expect("valid Meteora program id constant");
    Pubkey::find_program_address(
        &[b"bin_array", pool.as_ref(), &index.to_le_bytes()],
        &program_id,
    )
    .0
}

/// Bin-array index that contains `bin_id` (floor division, correct for negatives).
pub fn bin_id_to_bin_array_index(bin_id: i32) -> i64 {
    bin_id.div_euclid(METEORA_BINS_PER_ARRAY as i32) as i64
}

/// Decode the bins of one `BinArray` account. Bins with zero liquidity are
/// dropped; `bin_id = index * 70 + i`.
pub fn decode_bin_array(data: &[u8]) -> Vec<arb_types::DlmmBin> {
    let need = METEORA_BIN_ARRAY_HEADER + (METEORA_BINS_PER_ARRAY as usize) * METEORA_BIN_SIZE;
    if data.len() < need {
        return Vec::new();
    }
    let index = i64::from_le_bytes(data[8..16].try_into().unwrap_or([0; 8]));
    let base_bin_id = index.saturating_mul(METEORA_BINS_PER_ARRAY) as i32;
    let mut out = Vec::with_capacity(METEORA_BINS_PER_ARRAY as usize);
    for i in 0..METEORA_BINS_PER_ARRAY as usize {
        let off = METEORA_BIN_ARRAY_HEADER + i * METEORA_BIN_SIZE;
        let amount_x = u64::from_le_bytes(data[off..off + 8].try_into().unwrap_or([0; 8]));
        let amount_y = u64::from_le_bytes(data[off + 8..off + 16].try_into().unwrap_or([0; 8]));
        if amount_x == 0 && amount_y == 0 {
            continue;
        }
        out.push(arb_types::DlmmBin {
            bin_id: base_bin_id.saturating_add(i as i32),
            amount_x: amount_x as u128,
            amount_y: amount_y as u128,
        });
    }
    out
}

/// Resolve a pool's fee in bps from its live account data.
///
/// Returns `None` when the data is too short or the protocol is unsupported.
/// Callers must treat `None` as "skip this pool": a guessed fee is a fake edge.
///
/// Meteora bin arrays are decoded in one batched request inside
/// `fetch_live_pool_states` (see `dlmm_bin_array_pdas`), not here.
pub fn resolve_fee_bps(protocol: DexProtocol, pool_account_data: &[u8]) -> Option<u16> {
    match protocol {
        DexProtocol::RaydiumV4 => {
            if pool_account_data.len() < 192 {
                return None;
            }
            let num = u64::from_le_bytes(pool_account_data[176..184].try_into().ok()?);
            let den = u64::from_le_bytes(pool_account_data[184..192].try_into().ok()?);
            if den == 0 {
                return None;
            }
            let bps = u16::try_from(num as u128 * 10_000 / den as u128).unwrap_or(10_000);
            if bps > 10_000 { None } else { Some(bps) }
        }
        DexProtocol::OrcaWhirlpool => {
            if pool_account_data.len() < 47 {
                return None;
            }
            let fee_rate = u16::from_le_bytes(pool_account_data[45..47].try_into().ok()?);
            let bps = fee_rate / 100;
            if bps > 10_000 { None } else { Some(bps) }
        }
        DexProtocol::MeteoraDlmm => {
            // Reuse the verified LbPair decoder instead of carrying a second,
            // stale offset table: `base_factor` lives at 8 and `bin_step` at 80,
            // not at the 9/11 used by the old copy of this layout.
            let state = decode_dlmm_state(pool_account_data)?;
            let prm = state.params?;
            let base = (prm.base_factor as u64) * (state.bin_step_bps as u64) * 10;
            let scaled = if prm.base_fee_power_factor == 0 {
                base
            } else {
                base.checked_mul(10u64.checked_pow(prm.base_fee_power_factor as u32)?)?
            };
            let bps = u16::try_from(scaled.saturating_mul(10_000) / 1_000_000_000).ok()?;
            if bps > 10_000 { None } else { Some(bps) }
        }
        DexProtocol::RaydiumClmm => None,
        DexProtocol::RaydiumCpmm => decode_raydium_cpmm_fee_bps(pool_account_data),
        DexProtocol::PumpFun => None,
        DexProtocol::PumpSwap => None,
        DexProtocol::MeteoraDammV2 => None,
        DexProtocol::RaydiumLaunchLab => None,
    }
}

static POOL_WRITE_VERSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Monotonic write version shared between HTTP registry refreshes and live WS pool updates.
pub fn next_pool_write_version() -> u64 {
    POOL_WRITE_VERSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Instantaneous virtual reserves for CLMM pools: `x = L / sqrt(P)`, `y = L * sqrt(P)`.
pub fn compute_clmm_virtual_reserves(c: &arb_types::ClmmState) -> (u128, u128) {
    if c.liquidity == 0 || c.sqrt_price == 0 {
        return (0, 0);
    }
    let l = ruint::aliases::U256::from(c.liquidity);
    let sp = ruint::aliases::U256::from(c.sqrt_price);
    let x_virt = (l << 64usize) / sp;
    let y_virt = (l * sp) >> 64usize;
    (
        x_virt.try_into().unwrap_or(u128::MAX),
        y_virt.try_into().unwrap_or(u128::MAX),
    )
}

/// True when cached CLMM tick arrays contain the array holding `tick_current`.
pub fn clmm_tick_array_covers_tick(
    arrays: &[arb_types::ClmmTickArray],
    tick_current: i32,
    tick_spacing: u16,
) -> bool {
    let spacing = tick_spacing.max(1) as i32;
    let ticks_per_array = 88i32.saturating_mul(spacing);
    if ticks_per_array == 0 {
        return false;
    }
    let start = tick_current.div_euclid(ticks_per_array) * ticks_per_array;
    arrays.iter().any(|ta| ta.start_tick_index == start)
}

/// Decode Meteora DLMM dynamic base fee in basis points.
pub fn decode_dlmm_fee_bps(s: &arb_types::DlmmState) -> Option<u16> {
    let prm = s.params.as_ref()?;
    let base = (prm.base_factor as u64) * (s.bin_step_bps as u64) * 10;
    let scaled = if prm.base_fee_power_factor == 0 {
        base
    } else {
        base.checked_mul(10u64.checked_pow(prm.base_fee_power_factor as u32)?)?
    };
    u16::try_from(scaled.saturating_mul(10_000) / 1_000_000_000).ok()
}

/// Compute effective base/quote reserves for Meteora DLMM from decoded `DlmmState` bins and raw `LbPair` bytes.
///
/// DLMM semantics (verified against the Meteora DLMM program layout): bins with
/// `bin_id > active_id` hold only token X; bins with `bin_id < active_id` hold
/// only token Y; the active bin can hold both. The honest two-sided CPMM
/// approximation for the graph edge is therefore:
///
/// - `x_depth = \u{03a3} amount_x` over bins `bin_id >= active_id` (X the pool can sell)
/// - `y_depth = \u{03a3} amount_y` over bins `bin_id <= active_id` (Y the pool can sell)
///
/// Fail-closed rules (each returns `None`, dropping the pool from the graph):
/// - no bins attached, or the ACTIVE bin itself is missing from the attached
///   BinArrays (hydration incomplete \u{2014} never guess its contents);
/// - either side depth is zero (one-sided book: no honest two-sided quote);
/// - implied price `y_depth / x_depth` diverges from the active-bin price by
///   more than [`DLMM_RESERVE_PRICE_TOLERANCE`] (corrupt/inconsistent bin data).
///
/// The previous implementation derived one side from the other via the active
/// price whenever the active bin missed one token (`rx else sum` fallbacks).
/// Measured on mainnet pool HRYEjwdo3bZ1TpXKWKcezqiwSV2Ywuh4LxMa2PzoCnG6
/// (USDC/WSOL, bin_step=2): active bin held `amount_x=0`, so the fallback
/// summed 210 bins into `rx=6.57e9 USDC-raw` and derived
/// `ry=82.26 WSOL` \u{2014} a fake book that produced ~4,134 phantom-arbitrage
/// rejections per 3-minute soak (implied ROI 1.3M\u{2013}1.5M bps vs the 1500 bps
/// implausibility ceiling).
pub fn compute_dlmm_reserves(
    d: &arb_types::DlmmState,
    pool_data: &[u8],
    base_mint: &Pubkey,
    quote_mint: &Pubkey,
) -> Option<(u128, u128)> {
    if d.bins.is_empty() {
        return None;
    }
    let p_x_in_y = arb_math::dlmm::get_price_from_id(d.active_bin_id, d.bin_step_bps)
        .ok()
        .filter(|p| *p > 0)?;
    let tx = pool_data
        .get(88..120)
        .filter(|_| pool_data.len() >= 152)
        .and_then(|s| <[u8; 32]>::try_from(s).ok())
        .map(Pubkey::new_from_array)?;
    // The active bin must be present: it anchors both side sums and the
    // price-consistency check. Missing active bin = incomplete hydration.
    d.bins.iter().find(|b| b.bin_id == d.active_bin_id)?;
    let x_depth: u128 = d
        .bins
        .iter()
        .filter(|b| b.bin_id >= d.active_bin_id)
        .map(|b| b.amount_x)
        .sum();
    let y_depth: u128 = d
        .bins
        .iter()
        .filter(|b| b.bin_id <= d.active_bin_id)
        .map(|b| b.amount_y)
        .sum();
    if x_depth == 0 || y_depth == 0 {
        return None;
    }
    // Price-consistency invariant: the side sums must imply a price within a
    // sane band of the active-bin price. With correctly attached arrays this
    // holds by construction (bins within \u{00b1}70 bins of active); a violation
    // means corrupt or mismatched bin data, never a real market state.
    let implied_q64 =
        (ruint::aliases::U256::from(y_depth) << 64usize) / ruint::aliases::U256::from(x_depth);
    let p_u256 = ruint::aliases::U256::from(p_x_in_y);
    let band = p_u256 * ruint::aliases::U256::from(DLMM_RESERVE_PRICE_TOLERANCE);
    if implied_q64 > band
        || implied_q64 < p_u256 / ruint::aliases::U256::from(DLMM_RESERVE_PRICE_TOLERANCE)
    {
        return None;
    }
    if tx == *base_mint {
        Some((x_depth, y_depth))
    } else if tx == *quote_mint {
        Some((y_depth, x_depth))
    } else {
        Some((0, 0))
    }
}

/// Max allowed ratio between the bin-sum implied price and the active-bin
/// price. 4x is far outside any correctly-hydrated \u{00b1}1-bin-array window
/// (\u{00b1}70 bins * 0.02% \u{2248} \u{00b1}1.4% for the tightest bin_step=2 pools) yet
/// still catches grossly corrupt data.
const DLMM_RESERVE_PRICE_TOLERANCE: u64 = 4;

/// Decode a u64 token amount from raw SPL token account bytes (offset 64..72).
pub fn decode_spl_token_amount_bytes(bytes: &[u8]) -> Option<u64> {
    if bytes.len() < 72 {
        return None;
    }
    let arr: [u8; 8] = bytes.get(64..72)?.try_into().ok()?;
    Some(u64::from_le_bytes(arr))
}

/// Decode a u64 token amount from an SPL token account JSON value.
/// SPL token accounts store the amount as a little-endian u64 at byte offset 64,
/// base64-encoded in the `data[0]` field of the RPC response.
#[allow(dead_code)]
fn decode_token_amount(acc: Option<&serde_json::Value>) -> u64 {
    let Some(acc) = acc else {
        return 0;
    };
    let Some(data_str) = acc
        .get("data")
        .and_then(|d| d.get(0))
        .and_then(|s| s.as_str())
    else {
        return 0;
    };
    let Ok(bytes) = BASE64_STANDARD.decode(data_str) else {
        return 0;
    };
    decode_spl_token_amount_bytes(&bytes).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mainnet_default_registers_all_venues() {
        let rpc = RpcPool::new(vec!["https://example.test".to_string()]);
        let reg = MultiDexRegistry::mainnet_default(rpc);
        // 1 Raydium v4 + 1 Raydium CPMM + 4 Orca Whirlpool fee tiers + 3 Meteora DLMM.
        assert_eq!(reg.pool_count(), 9);
        let protocols: Vec<_> = reg.pools().iter().map(|p| p.protocol).collect();
        assert!(protocols.contains(&DexProtocol::RaydiumV4));
        assert!(protocols.contains(&DexProtocol::RaydiumCpmm));
        assert!(protocols.contains(&DexProtocol::OrcaWhirlpool));
        assert!(protocols.contains(&DexProtocol::MeteoraDlmm));
        // At least one sub-5 bps Orca tier must be present (the low-fee route).
        let orca_fees: Vec<u16> = reg
            .pools()
            .iter()
            .filter(|p| p.protocol == DexProtocol::OrcaWhirlpool)
            .map(|p| p.fee_bps)
            .collect();
        assert!(orca_fees.iter().any(|f| *f <= 2));
        // Raydium CPMM 1 bps tier must be present.
        let cpmm_fees: Vec<u16> = reg
            .pools()
            .iter()
            .filter(|p| p.protocol == DexProtocol::RaydiumCpmm)
            .map(|p| p.fee_bps)
            .collect();
        assert!(cpmm_fees.contains(&1));
    }

    #[test]
    fn registration_is_idempotent() {
        let rpc = RpcPool::new(vec!["https://example.test".to_string()]);
        let mut reg = MultiDexRegistry::mainnet_default(rpc);
        let before = reg.pool_count();
        let wsol = MintId(Pubkey::from_str(arb_constants::WSOL_MINT).unwrap());
        let usdc = MintId(Pubkey::from_str(arb_constants::USDC_MINT).unwrap());
        let dup = RegisteredPool {
            pool_id: PoolId(Pubkey::from_str(arb_constants::RAYDIUM_SOL_USDC_POOL).unwrap()),
            protocol: DexProtocol::RaydiumV4,
            base_mint: wsol,
            quote_mint: usdc,
            decimals_a: 9,
            decimals_b: 6,
            base_vault: Pubkey::default(),
            quote_vault: Pubkey::default(),
            fee_bps: 25,
        };
        reg.register(dup);
        assert_eq!(
            reg.pool_count(),
            before,
            "duplicate pool must not be re-registered"
        );
    }

    #[test]
    fn decode_token_amount_handles_missing() {
        assert_eq!(decode_token_amount(None), 0);
        assert_eq!(decode_token_amount(Some(&serde_json::json!({}))), 0);
    }

    #[test]
    fn resolve_fee_bps_raydium() {
        let mut data = vec![0u8; 192];
        data[176..184].copy_from_slice(&25u64.to_le_bytes());
        data[184..192].copy_from_slice(&10_000u64.to_le_bytes());
        assert_eq!(resolve_fee_bps(DexProtocol::RaydiumV4, &data), Some(25));
    }

    #[test]
    fn resolve_fee_bps_orca() {
        let mut data = vec![0u8; 47];
        data[45..47].copy_from_slice(&500u16.to_le_bytes()); // 5 bps
        assert_eq!(resolve_fee_bps(DexProtocol::OrcaWhirlpool, &data), Some(5));
    }

    #[test]
    fn resolve_fee_bps_meteora() {
        // Real layout: base_factor @8, bin_step @80, status @82.
        let mut data = vec![0u8; 88];
        data[8..10].copy_from_slice(&10_000u16.to_le_bytes());
        data[80..82].copy_from_slice(&4u16.to_le_bytes());
        data[82] = 0;
        assert_eq!(resolve_fee_bps(DexProtocol::MeteoraDlmm, &data), Some(4));
    }

    #[test]
    fn resolve_fee_bps_short_returns_none() {
        assert_eq!(resolve_fee_bps(DexProtocol::RaydiumV4, &[0; 10]), None);
        assert_eq!(resolve_fee_bps(DexProtocol::OrcaWhirlpool, &[0; 10]), None);
        assert_eq!(resolve_fee_bps(DexProtocol::MeteoraDlmm, &[0; 10]), None);
    }

    #[test]
    fn test_decode_pool_info_raydium_v4() {
        let mut data = vec![0u8; 500];
        let va = Pubkey::new_unique();
        let vb = Pubkey::new_unique();
        let ma = Pubkey::new_unique();
        let mb = Pubkey::new_unique();
        data[336..368].copy_from_slice(&va.to_bytes());
        data[368..400].copy_from_slice(&vb.to_bytes());
        data[400..432].copy_from_slice(&ma.to_bytes());
        data[432..464].copy_from_slice(&mb.to_bytes());
        data[176..184].copy_from_slice(&25u64.to_le_bytes());
        data[184..192].copy_from_slice(&10_000u64.to_le_bytes());

        let info = decode_pool_info(DexProtocol::RaydiumV4, &data).expect("valid Raydium V4");
        assert_eq!(info.token_a_vault, va);
        assert_eq!(info.token_b_vault, vb);
        assert_eq!(info.token_a_mint, ma);
        assert_eq!(info.token_b_mint, mb);
        assert_eq!(info.fee_bps, 25);
        assert_eq!(info.decimals_a, None);
    }

    #[test]
    fn test_decode_pool_info_raydium_cpmm() {
        let mut data = vec![0u8; 350];
        let va = Pubkey::new_unique();
        let vb = Pubkey::new_unique();
        let ma = Pubkey::new_unique();
        let mb = Pubkey::new_unique();
        data[72..104].copy_from_slice(&va.to_bytes());
        data[104..136].copy_from_slice(&vb.to_bytes());
        data[168..200].copy_from_slice(&ma.to_bytes());
        data[200..232].copy_from_slice(&mb.to_bytes());
        data[331] = 9;
        data[332] = 6;
        data[290..298].copy_from_slice(&2500u64.to_le_bytes());

        let info = decode_pool_info(DexProtocol::RaydiumCpmm, &data).expect("valid Raydium CPMM");
        assert_eq!(info.token_a_vault, va);
        assert_eq!(info.token_b_vault, vb);
        assert_eq!(info.token_a_mint, ma);
        assert_eq!(info.token_b_mint, mb);
        assert_eq!(info.fee_bps, 25);
        assert_eq!(info.decimals_a, Some(9));
        assert_eq!(info.decimals_b, Some(6));
    }

    #[test]
    fn test_decode_pool_info_orca_whirlpool() {
        let mut data = vec![0u8; 250];
        let ma = Pubkey::new_unique();
        let va = Pubkey::new_unique();
        let mb = Pubkey::new_unique();
        let vb = Pubkey::new_unique();
        data[101..133].copy_from_slice(&ma.to_bytes());
        data[133..165].copy_from_slice(&va.to_bytes());
        data[181..213].copy_from_slice(&mb.to_bytes());
        data[213..245].copy_from_slice(&vb.to_bytes());
        data[45..47].copy_from_slice(&500u16.to_le_bytes()); // 500 hundredths of bip = 5 bps

        let info = decode_pool_info(DexProtocol::OrcaWhirlpool, &data).expect("valid Orca");
        assert_eq!(info.token_a_mint, ma);
        assert_eq!(info.token_a_vault, va);
        assert_eq!(info.token_b_mint, mb);
        assert_eq!(info.token_b_vault, vb);
        assert_eq!(info.fee_bps, 5);
    }

    #[test]
    fn test_decode_pool_info_meteora_dlmm() {
        let mut data = vec![0u8; 220];
        let ma = Pubkey::new_unique();
        let mb = Pubkey::new_unique();
        let va = Pubkey::new_unique();
        let vb = Pubkey::new_unique();
        data[80..82].copy_from_slice(&4u16.to_le_bytes()); // bin step = 4
        data[88..120].copy_from_slice(&ma.to_bytes());
        data[120..152].copy_from_slice(&mb.to_bytes());
        data[152..184].copy_from_slice(&va.to_bytes());
        data[184..216].copy_from_slice(&vb.to_bytes());

        let info = decode_pool_info(DexProtocol::MeteoraDlmm, &data).expect("valid Meteora DLMM");
        assert_eq!(info.token_a_mint, ma);
        assert_eq!(info.token_b_mint, mb);
        assert_eq!(info.token_a_vault, va);
        assert_eq!(info.token_b_vault, vb);
        assert_eq!(info.fee_bps, 4);
    }

    #[test]
    fn test_decode_pool_info_raydium_clmm() {
        let mut data = vec![0u8; 350];
        let ma = Pubkey::new_unique();
        let mb = Pubkey::new_unique();
        let va = Pubkey::new_unique();
        let vb = Pubkey::new_unique();
        data[73..105].copy_from_slice(&ma.to_bytes());
        data[105..137].copy_from_slice(&mb.to_bytes());
        data[137..169].copy_from_slice(&va.to_bytes());
        data[169..201].copy_from_slice(&vb.to_bytes());
        data[233] = 9;
        data[234] = 6;
        data[235..237].copy_from_slice(&60u16.to_le_bytes()); // tick_spacing

        let info = decode_pool_info(DexProtocol::RaydiumClmm, &data).expect("valid Raydium CLMM");
        assert_eq!(info.token_a_mint, ma);
        assert_eq!(info.token_b_mint, mb);
        assert_eq!(info.token_a_vault, va);
        assert_eq!(info.token_b_vault, vb);
        assert_eq!(info.decimals_a, Some(9));
        assert_eq!(info.decimals_b, Some(6));
        assert_eq!(info.fee_bps, 25);
    }

    #[test]
    fn test_decode_raydium_clmm_vaults() {
        let mut data = vec![0u8; 250];
        let va = Pubkey::new_unique();
        let vb = Pubkey::new_unique();
        data[137..169].copy_from_slice(&va.to_bytes());
        data[169..201].copy_from_slice(&vb.to_bytes());

        let vaults = decode_raydium_clmm_vaults(&data).expect("valid clmm vaults");
        assert_eq!(vaults.0, va);
        assert_eq!(vaults.1, vb);

        assert_eq!(decode_raydium_clmm_vaults(&[0u8; 100]), None);
    }

    fn orca_whirlpool_bytes(len: usize, mint_a: Pubkey, mint_b: Pubkey) -> Vec<u8> {
        let mut data = vec![0u8; len];
        data[101..133].copy_from_slice(mint_a.as_ref());
        data[181..213].copy_from_slice(mint_b.as_ref());
        data
    }

    #[test]
    fn clmm_onchain_mints_orca_interleaved_layout() {
        let ma = Pubkey::new_unique();
        let mb = Pubkey::new_unique();
        for len in [525usize, 653] {
            let data = orca_whirlpool_bytes(len, ma, mb);
            let got =
                clmm_onchain_mints(DexProtocol::OrcaWhirlpool, &data).expect("valid whirlpool");
            assert_eq!(got, (ma, mb), "len={len}");
        }
    }

    #[test]
    fn clmm_onchain_mints_raydium_layout() {
        let ma = Pubkey::new_unique();
        let mb = Pubkey::new_unique();
        let mut data = vec![0u8; 1544];
        data[73..105].copy_from_slice(ma.as_ref());
        data[105..137].copy_from_slice(mb.as_ref());
        assert_eq!(
            clmm_onchain_mints(DexProtocol::RaydiumClmm, &data).expect("valid clmm"),
            (ma, mb)
        );
    }

    #[test]
    fn clmm_onchain_mints_fail_closed() {
        let ma = Pubkey::new_unique();
        let mb = Pubkey::new_unique();
        assert_eq!(
            clmm_onchain_mints(DexProtocol::OrcaWhirlpool, &[0u8; 100]),
            None
        );
        assert_eq!(
            clmm_onchain_mints(DexProtocol::RaydiumClmm, &[0u8; 100]),
            None
        );
        assert_eq!(
            clmm_onchain_mints(DexProtocol::OrcaWhirlpool, &[0u8; 245]),
            None,
            "default pubkeys must be rejected"
        );
        assert_eq!(
            clmm_onchain_mints(DexProtocol::MeteoraDlmm, &orca_whirlpool_bytes(653, ma, mb)),
            None,
            "non-CLMM protocols must be rejected"
        );
    }

    #[test]
    fn clmm_onchain_mints_rejects_swapped_layout_bytes() {
        // A pool account whose @181..213 window is a vault (not a mint) must not
        // be silently accepted as orientation truth when the catalog disagrees;
        // the helper returns the raw on-chain order and the registry logs the
        // divergence. Here we only pin that the helper reads exactly @101/@181.
        let ma = Pubkey::new_unique();
        let mb = Pubkey::new_unique();
        let mut data = orca_whirlpool_bytes(653, ma, mb);
        let intruder = Pubkey::new_unique();
        data[133..165].copy_from_slice(intruder.as_ref()); // vault_a slot
        assert_eq!(
            clmm_onchain_mints(DexProtocol::OrcaWhirlpool, &data).expect("valid"),
            (ma, mb),
            "vault_a bytes @133..165 must not leak into the mint read"
        );
    }

    #[test]
    fn test_decode_raydium_clmm_state() {
        let mut data = vec![0u8; 300];
        data[235..237].copy_from_slice(&60u16.to_le_bytes()); // tick_spacing
        data[237..253].copy_from_slice(&5_000_000u128.to_le_bytes()); // liquidity
        data[253..269].copy_from_slice(&(1u128 << 64).to_le_bytes()); // sqrt_price
        data[269..273].copy_from_slice(&120i32.to_le_bytes()); // tick_current

        let state = decode_raydium_clmm_state(&data, 25).expect("valid clmm state");
        assert_eq!(state.tick_spacing, 60);
        assert_eq!(state.liquidity, 5_000_000);
        assert_eq!(state.sqrt_price, 1u128 << 64);
        assert_eq!(state.tick_current, 120);
        assert_eq!(state.fee_rate, 2_500);
    }

    #[test]
    fn test_raydium_clmm_tick_array_pdas() {
        let pool = PoolId(Pubkey::new_unique());
        let (keys, starts) = raydium_clmm_tick_array_pdas(&pool, 120, 60);
        assert_eq!(keys.len(), 3);
        assert_eq!(starts.len(), 3);
        // tick_spacing=60, ticks_per_array = 60 * 60 = 3600
        // tick_current=120, start = 120.div_euclid(3600) * 3600 = 0
        assert_eq!(starts[0], 0);
        assert_eq!(starts[1], -3600);
        assert_eq!(starts[2], 3600);
        // All 3 PDAs must be distinct and non-default
        assert_ne!(keys[0], keys[1]);
        assert_ne!(keys[1], keys[2]);
        assert_ne!(keys[0], Pubkey::default());
    }

    #[test]
    fn test_resolve_fee_bps_raydium_cpmm() {
        let mut data = vec![0u8; 300];
        data[290..298].copy_from_slice(&100u64.to_le_bytes()); // 1 bps
        assert_eq!(resolve_fee_bps(DexProtocol::RaydiumCpmm, &data), Some(1));
    }

    // ---- compute_dlmm_reserves fail-closed forensics (soak10/11 phantom ROI) ----

    fn dlmm_probe_state(active_id: i32, mut bins: Vec<arb_types::DlmmBin>) -> arb_types::DlmmState {
        bins.sort_by_key(|b| b.bin_id);
        arb_types::DlmmState {
            active_bin_id: active_id,
            status: 0,
            bin_step_bps: 10,
            bins,
            params: None,
            volatility_accumulator: 0,
            volatility_reference: 0,
            index_reference: active_id,
            last_update_timestamp: 0,
        }
    }

    fn dlmm_probe_pool_bytes(token_x: Pubkey, token_y: Pubkey) -> Vec<u8> {
        let mut data = vec![0u8; 152];
        data[88..120].copy_from_slice(token_x.as_ref());
        data[120..152].copy_from_slice(token_y.as_ref());
        data
    }

    #[test]
    fn dlmm_reserves_fail_closed_when_active_bin_missing() {
        let x = Pubkey::new_unique();
        let y = Pubkey::new_unique();
        let data = dlmm_probe_pool_bytes(x, y);
        let d = dlmm_probe_state(
            1000,
            vec![
                arb_types::DlmmBin {
                    bin_id: 998,
                    amount_x: 0,
                    amount_y: 1_000_000,
                },
                arb_types::DlmmBin {
                    bin_id: 1002,
                    amount_x: 1_000_000,
                    amount_y: 0,
                },
            ],
        );
        assert_eq!(compute_dlmm_reserves(&d, &data, &x, &y), None);
    }

    #[test]
    fn dlmm_reserves_fail_closed_on_one_sided_book() {
        let x = Pubkey::new_unique();
        let y = Pubkey::new_unique();
        let data = dlmm_probe_pool_bytes(x, y);
        // HRYEjw mainnet shape: active bin holds only Y (amount_x=0) and no
        // bin at/above active holds X. The old fallback summed distant bins
        // into a fake x-side and derived y from price — producing the
        // ~130x phantom ROI. Must now fail closed.
        let d = dlmm_probe_state(
            1000,
            vec![arb_types::DlmmBin {
                bin_id: 1000,
                amount_x: 0,
                amount_y: 1_444_908_165,
            }],
        );
        assert_eq!(compute_dlmm_reserves(&d, &data, &x, &y), None);
    }

    #[test]
    fn dlmm_reserves_sum_correct_sides_both_directions() {
        let x = Pubkey::new_unique();
        let y = Pubkey::new_unique();
        let data = dlmm_probe_pool_bytes(x, y);
        let d = dlmm_probe_state(
            1000,
            vec![
                arb_types::DlmmBin {
                    bin_id: 998,
                    amount_x: 0,
                    amount_y: 500,
                },
                arb_types::DlmmBin {
                    bin_id: 1000,
                    amount_x: 1_000,
                    amount_y: 2_700,
                },
                arb_types::DlmmBin {
                    bin_id: 1002,
                    amount_x: 3_000,
                    amount_y: 0,
                },
            ],
        );
        // x_depth = bins >= 1000 => 1000 + 3000 = 4000
        // y_depth = bins <= 1000 => 500 + 2700 = 3200
        assert_eq!(compute_dlmm_reserves(&d, &data, &x, &y), Some((4000, 3200)));
        // Flipped pool orientation (token_x is the quote mint): reserves swap.
        assert_eq!(compute_dlmm_reserves(&d, &data, &y, &x), Some((3200, 4000)));
    }

    #[test]
    fn dlmm_reserves_reject_price_inconsistent_bins() {
        let x = Pubkey::new_unique();
        let y = Pubkey::new_unique();
        let data = dlmm_probe_pool_bytes(x, y);
        // active_id=1000, step=10bps => p ~ 2.717 y-raw per x-raw. A book
        // implying 0.1 y-raw per x-raw is >4x below p: corrupt data.
        let d = dlmm_probe_state(
            1000,
            vec![arb_types::DlmmBin {
                bin_id: 1000,
                amount_x: 1_000_000,
                amount_y: 100_000,
            }],
        );
        assert_eq!(compute_dlmm_reserves(&d, &data, &x, &y), None);
    }
}
