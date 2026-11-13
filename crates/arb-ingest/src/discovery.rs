//! Dynamic pool discovery across every liquid token on Solana.
//!
//! # Why not `getProgramAccounts`
//!
//! An exhaustive `getProgramAccounts` sweep of a DEX program returns tens of
//! thousands of accounts (Orca alone has ~15k pools) and either times out or is
//! rejected by the RPC provider. The boot path used to do exactly that, which
//! meant "dynamic discovery" could never actually succeed in production.
//!
//! # What this does instead
//!
//! Two bounded, cheap sources that already contain the metadata the registry
//! needs — pool address, both mint addresses and decimals, TVL and fee:
//!
//! 1. **Orca catalog** (`api.orca.so/v1/whirlpool/list`) — every Whirlpool.
//! 2. **DexScreener token pairs** for each hub mint (WSOL, USDC) — used to find
//!    Meteora DLMM pools, filtered by the `DLMM` label so the DAMM program's
//!    pools are never mistaken for LbPairs.
//!
//! Only pools above `min_tvl_usd` are kept, then the set is ranked by TVL and
//! capped at `max_pools`. A tick has to fetch every pool account, so an
//! unbounded set is not a feature — it is a stall.
//!
//! Everything that is *priced* (reserves, fees, active bin, sqrt price) is
//! still read from the pool account on chain every tick by
//! `MultiDexRegistry::fetch_live_pool_states`. Discovery only decides **which**
//! pools to watch; it never supplies a number that the quote trusts.

use crate::RpcPool;
use crate::registry::{
    decode_account_data, decode_raydium_clmm_vaults, decode_raydium_cpmm_vaults,
    decode_raydium_vaults, fetch_accounts_chunked,
};
use arb_types::{DexProtocol, MintId, PoolId};
use base64::Engine;
use solana_pubkey::Pubkey;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Orca's public catalog: address, both mints + decimals, TVL, LP fee.
pub const ORCA_CATALOG_URL: &str = "https://api.orca.so/v1/whirlpool/list";
/// Raydium's v3 catalog (standard AMM pools, sorted by liquidity server-side).
pub const RAYDIUM_POOL_LIST_URL: &str = "https://api-v3.raydium.io/pools/info/list";
/// DexScreener: every pair trading a given mint, ranked by liquidity.
pub const DEXSCREENER_TOKEN_PAIRS: &str = "https://api.dexscreener.com/token-pairs/v1/solana";
/// DexScreener: general search endpoint for active DEX pairs.
pub const DEXSCREENER_SEARCH_URL: &str = "https://api.dexscreener.com/latest/dex/search";

/// Maximum accounts per `getMultipleAccounts` request (RPC hard limit is 100).
const RPC_BATCH: usize = 100;

/// A pool discovered from a catalog, ready to register.
#[derive(Debug, Clone)]
pub struct DiscoveredPoolState {
    pub pool_id: PoolId,
    pub protocol: DexProtocol,
    pub base_mint: MintId,
    pub quote_mint: MintId,
    /// Vaults are informational for CLMM/DLMM: their reserves come from the
    /// pool account (sqrt price / bins), not from token-account balances.
    pub base_vault: Pubkey,
    pub quote_vault: Pubkey,
    pub decimals_a: u8,
    pub decimals_b: u8,
    /// Catalog fee in bps. The registry re-reads the real fee from the pool
    /// account every tick and skips the pool when it cannot, so this value is
    /// never trusted for pricing.
    pub fee_bps: u16,
    pub liquidity_usd: f64,
}

/// Bounds for a discovery sweep.
#[derive(Debug, Clone)]
pub struct DiscoveryConfig {
    /// Pools below this TVL are ignored.
    pub min_tvl_usd: f64,
    /// Hard cap on registered pools (each one costs an account fetch per tick).
    pub max_pools: usize,
    /// Maximum active pools to select per DEX venue (e.g. 33 per venue).
    pub max_per_venue: usize,
    /// Mints used as discovery hubs on DexScreener (SOL, USDC, USDT...).
    pub hub_mints: Vec<MintId>,
    /// Per-hub cap before the global ranking is applied.
    pub max_per_hub: usize,
    pub include_orca: bool,
    pub include_raydium: bool,
    pub include_meteora: bool,
    pub include_pump_fun: bool,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        let mut hubs = Vec::new();
        for s in [
            arb_constants::WSOL_MINT,
            arb_constants::USDC_MINT,
            // USDT — the second-deepest quote asset on Solana.
            "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB",
            // POPCAT — high volatility memecoin
            "7GCihgDB8fe6KNjn2MYtkzZcRjQy3t9GHdC8uHYmW2hr",
            // WIF — high volatility memecoin
            "EKpQGSJtjMFqKZ9KQanSqYXRcF8fBopzLHYxdM65zcjm",
            // JUP — Jupiter aggregator token
            "JUPyiwrYJFngUPXSiqZFkKHMXkGs3WEXKaJE1agWK",
            // JTO — Jito governance token
            "jtojtomepa8beP8uL5qyNtYQgwwJjA72yS6JwTL9CZq",
            // BONK — popular memecoin
            "DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB268",
            // BOME — Book of Meme
            "9MCPRCn3cpWFdSmV93hJ1kVXVVdjPpEqfnfYkaZ4TLws",
            // RENDER
            "rndrizKT3MK1iimdxRdWabcF7Zg7AR5T4nud4EkHBof",
            // RAY — Raydium DEX token
            "4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R",
            // ME — Magic Eden token
            "MEFNBXixkEbait3xn9bkm8FsMeqEikuY4qLQioNc3gU",
            // TRUMP — High volume political meme
            "6p6xgHyF7AeQHydaKVtRNVUjCAWoTeZwXHnkTXQKpump",
            // FARTCOIN — High volume meme
            "9BB6NFEcjBCtnNLFko2FqVQBq8HHM13kCyYcdQbgpump",
            // AI16Z — AI agent meme
            "HeLp6NuQkmYB4pYWo2zYs22mESHXPQYzXbB8n4V98jwC",
            // PYTH — Oracle network token
            "HZ1JovNiVvGrGNiiYvEozEVgZ58xaU3RKwX8eACQBCt3",
            // DRIFT — Drift Protocol DEX
            "DriFtupJYLTosbwoN8koMbEYSx54aFAVLddWsbksjwg7",
            // PENGU — Pudgy Penguins token
            "2zMMhcVQEXDtdE6vsFS7S7D5oUodfJHE8vd1gnBouauv",
            // mSOL — Marinade Staked SOL
            "mSoLzYCxHdYgdzU16g5QSh3i5K3z3KZK7ytfqcJm7So",
            // bSOL — BlazeStake Staked SOL
            "bSo13r4TkiE4KumL71LsHTPpL2euBYLFx6h9HP3piy1",
            // JitoSOL — Jito Liquid Staking SOL
            "J1toso1uCk3RLmjorhTtrVwY9HJ7X8V9yYac6Y7kGCPn",
            // GRASS
            "grass7B4RdKfBCjTKgSqnXkqjwiGvQyFbuSCUJr3XXjs",
            // KMNO — Kamino Finance
            "KMNo3nJsBXfcpJTVhZcXLW7RmTwTt4GVFE7suUBo9sS",
            // ORCA — Orca DEX token
            "orcaEKTdK7LKz57vaAYr9QeNsVEPfiu6QeMU1kektZE",
            // FWOG — High volume meme
            "A8C3xuqscfmyLrte3VmTqrAq8kgMASius9AFNANwpump",
            // MOODENG — Viral meme
            "ED5nyyWEzpPPiWimP8vYm7sD7TD3LAt3Q3gRTWHzPJBY",
        ] {
            if let Ok(pk) = Pubkey::from_str(s) {
                hubs.push(MintId(pk));
            }
        }
        Self {
            min_tvl_usd: 25000.0,
            max_pools: 200,
            max_per_venue: 33,
            hub_mints: hubs,
            max_per_hub: 100,
            include_orca: true,
            include_raydium: true,
            include_meteora: true,
            include_pump_fun: true,
        }
    }
}

/// Catalog-driven discovery engine.
pub struct PoolDiscovery {
    rpc: RpcPool,
    http: reqwest::Client,
    /// Immutable mint-decimal cache. Mint decimals can never change after
    /// creation, so a resolved value is valid for the process lifetime.
    /// Without this, every ~90s rediscovery cycle re-resolved the same meme
    /// mints over RPC; on free-tier endpoints those calls landed during
    /// 429-cooldown windows, `resolve_mint_decimals` failed, and the cycle
    /// registered a degraded pool set (measured soak5-soak8: 165 -> 93 pools,
    /// Raydium venues dropped entirely each cycle).
    decimals_cache: parking_lot::Mutex<HashMap<MintId, u8>>,
}

impl PoolDiscovery {
    pub fn new(rpc: RpcPool) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(35))
            .user_agent("arb-bot-discovery/1.0")
            .build()
            .unwrap_or_default();
        Self {
            rpc,
            http,
            decimals_cache: parking_lot::Mutex::new(HashMap::new()),
        }
    }

    /// Backwards-compatible entry point: sweep with default bounds, using
    /// `min_liquidity_usd` as the TVL floor.
    pub async fn discover_all(&self, min_liquidity_usd: f64) -> Vec<DiscoveredPoolState> {
        let cfg = DiscoveryConfig {
            min_tvl_usd: if min_liquidity_usd > 0.0 {
                min_liquidity_usd
            } else {
                DiscoveryConfig::default().min_tvl_usd
            },
            ..DiscoveryConfig::default()
        };
        self.discover(&cfg).await
    }

    /// Discover the watch set: bounded, ranked, deduplicated, fail-closed.
    pub async fn discover(&self, cfg: &DiscoveryConfig) -> Vec<DiscoveredPoolState> {
        info!(
            "🔍 Discovery: min_tvl=${:.0} max_per_venue={} cap={} hubs={}",
            cfg.min_tvl_usd,
            cfg.max_per_venue,
            cfg.max_pools,
            cfg.hub_mints.len()
        );

        let mut found: Vec<DiscoveredPoolState> = Vec::new();
        let catalog_fetch_limit = cfg.max_pools.max(cfg.max_per_venue * 6).max(250);

        if cfg.include_orca {
            match self
                .discover_orca(cfg.min_tvl_usd, catalog_fetch_limit)
                .await
            {
                Ok(pools) => {
                    info!("  Orca catalog: {} pools above TVL floor", pools.len());
                    found.extend(pools);
                }
                Err(e) => warn!("  Orca catalog discovery failed: {e}"),
            }
        }

        if cfg.include_raydium {
            match self
                .discover_raydium(cfg.min_tvl_usd, catalog_fetch_limit)
                .await
            {
                Ok(pools) => {
                    info!(
                        "  Raydium v3 catalog: {} pools above TVL floor",
                        pools.len()
                    );
                    found.extend(pools);
                }
                Err(e) => warn!("  Raydium catalog discovery failed: {e}"),
            }
        }

        if cfg.include_meteora {
            for hub in &cfg.hub_mints {
                match self
                    .discover_meteora_meteora_hub(hub, cfg.min_tvl_usd)
                    .await
                {
                    Ok(pools) => {
                        let n = pools.len();
                        found.extend(pools);
                        debug!("  Meteora via hub {}: {n} pools", hub.0);
                    }
                    Err(e) => warn!("  Meteora hub {} discovery failed: {e}", hub.0),
                }
            }

            match self.discover_meteora_search(cfg.min_tvl_usd).await {
                Ok(pools) => {
                    info!(
                        "  Meteora DLMM search discovered {} candidate pools",
                        pools.len()
                    );
                    found.extend(pools);
                }
                Err(e) => debug!("  Meteora DLMM search note: {e}"),
            }
        }

        if cfg.include_pump_fun {
            // 1. Dynamic API discovery of top active bonding curves (BankkRoll / pump.fun API)
            let pump_client = crate::pumpfun_discovery::PumpFunDiscoveryClient::new();
            match pump_client.fetch_top_coins(cfg.max_per_venue.max(50)).await {
                Ok(coins) => {
                    let top_pump = pump_client.select_top_n(&coins, cfg.max_per_venue);
                    if !top_pump.is_empty() {
                        info!(
                            "  Pump.fun active catalog discovered {} bonding curves",
                            top_pump.len()
                        );
                        found.extend(top_pump);
                    }
                }
                Err(e) => {
                    warn!("  Pump.fun direct API discovery failed, falling back to pairing: {e}");
                }
            }

            // 2. Cross-venue pairing: find multi-venue tokens existing on both AMMs and Pump.fun
            if let Ok(pump_prog) = Pubkey::from_str(arb_constants::PUMP_FUN_PROGRAM_ID) {
                if let Ok(wsol_pk) = Pubkey::from_str(arb_constants::WSOL_MINT) {
                    let wsol = MintId(wsol_pk);
                    let mut pump_pools = Vec::new();
                    for p in &found {
                        let target_mint = if p.quote_mint.0.to_string().ends_with("pump") {
                            Some((p.quote_mint, p.decimals_b))
                        } else if p.base_mint.0.to_string().ends_with("pump") {
                            Some((p.base_mint, p.decimals_a))
                        } else {
                            None
                        };

                        if let Some((meme_mint, dec)) = target_mint {
                            let (bc_pda, _) = Pubkey::find_program_address(
                                &[
                                    arb_constants::PUMP_FUN_BONDING_CURVE_SEED,
                                    meme_mint.0.as_ref(),
                                ],
                                &pump_prog,
                            );
                            pump_pools.push(DiscoveredPoolState {
                                pool_id: PoolId(bc_pda),
                                protocol: DexProtocol::PumpFun,
                                base_mint: wsol,
                                quote_mint: meme_mint,
                                base_vault: Pubkey::default(),
                                quote_vault: Pubkey::default(),
                                decimals_a: 9,
                                decimals_b: if dec > 0 { dec } else { 6 },
                                fee_bps: 100,
                                liquidity_usd: p.liquidity_usd,
                            });
                        }
                    }
                    if !pump_pools.is_empty() {
                        info!(
                            "  Pump.fun multi-venue bonding curves paired: {}",
                            pump_pools.len()
                        );
                        found.extend(pump_pools);
                    }
                }
            }
        }

        // Deduplicate by pool id — the same pool can surface from several hubs.
        let mut seen: HashSet<PoolId> = HashSet::new();
        found.retain(|p| seen.insert(p.pool_id));

        // Resolve missing decimals from chain (DexScreener does not publish
        // them). Anything still unknown, or owned by Token-2022 (transfer-fee /
        // extension risk), is dropped: a pool with unknown decimals would be
        // quoted off by 10^n, which is a fake edge, not a rounding error.
        let mut known: HashSet<MintId> = HashSet::new();
        for p in &found {
            if p.decimals_a > 0 {
                known.insert(p.base_mint);
            }
            if p.decimals_b > 0 {
                known.insert(p.quote_mint);
            }
        }
        let missing: Vec<MintId> = found
            .iter()
            .flat_map(|p| {
                let mut v = Vec::new();
                if !known.contains(&p.base_mint) {
                    v.push(p.base_mint);
                }
                if !known.contains(&p.quote_mint) {
                    v.push(p.quote_mint);
                }
                v
            })
            .collect();
        let resolved = self.resolve_mint_decimals(&missing).await;
        if !resolved.is_empty() {
            debug!("  Resolved {} mint decimals on chain", resolved.len());
        }
        known.extend(resolved.keys().copied());
        for p in &mut found {
            if let Some(d) = resolved.get(&p.base_mint) {
                p.decimals_a = *d;
            }
            if let Some(d) = resolved.get(&p.quote_mint) {
                p.decimals_b = *d;
            }
        }
        let before = found.len();
        found.retain(|p| known.contains(&p.base_mint) && known.contains(&p.quote_mint));
        if found.len() != before {
            debug!(
                "  Dropped {} pools with unresolved/Token-2022 decimals",
                before - found.len()
            );
        }

        // Rank by TVL and select up to cfg.max_per_venue (default 33) per DEX venue.
        found.sort_by(|a, b| {
            b.liquidity_usd
                .partial_cmp(&a.liquidity_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let selected = select_watch_set_per_venue(found, cfg.max_per_venue, cfg.max_pools);

        let mut venue_counts: HashMap<DexProtocol, usize> = HashMap::new();
        for p in &selected {
            *venue_counts.entry(p.protocol).or_default() += 1;
        }
        info!(
            "🔍 Discovery done: {} pools registered for live monitoring (top <= {} per venue): {:?}",
            selected.len(),
            cfg.max_per_venue,
            venue_counts
        );
        selected
    }

    /// GET a JSON document with retries and exponential backoff.
    /// Improved resilience for Orca and other catalog endpoints that may
    /// intermittently return malformed or empty responses.
    async fn get_json(&self, url: &str) -> anyhow::Result<serde_json::Value> {
        let mut last: Option<anyhow::Error> = None;
        for attempt in 0..4 {
            match self.http.get(url).send().await {
                Ok(resp) => match resp.error_for_status() {
                    Ok(ok) => match ok.json::<serde_json::Value>().await {
                        Ok(v) => return Ok(v),
                        Err(e) => {
                            let msg = e.to_string();
                            last = Some(e.into());
                            warn!(
                                "catalog JSON parse failed on attempt {}: {}",
                                attempt + 1,
                                msg
                            );
                        }
                    },
                    Err(e) => {
                        let msg = e.to_string();
                        last = Some(e.into());
                        warn!("catalog HTTP error on attempt {}: {}", attempt + 1, msg);
                    }
                },
                Err(e) => {
                    let msg = e.to_string();
                    last = Some(e.into());
                    warn!("catalog request failed on attempt {}: {}", attempt + 1, msg);
                }
            }
            if attempt < 3 {
                let backoff_ms = 500u64 * (2u64.pow(attempt as u32));
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            }
        }
        Err(last.unwrap_or_else(|| anyhow::anyhow!("catalog request failed after 4 attempts")))
    }

    /// Raydium pools from the v3 catalog across AMM v4, CPMM, and CLMM.
    ///
    /// The pool accounts are fetched in batches and their token vaults decoded:
    /// - Raydium V4: `decode_raydium_vaults`
    /// - Raydium CPMM: `decode_raydium_cpmm_vaults`
    /// - Raydium CLMM: `decode_raydium_clmm_vaults`
    ///
    /// Pools whose vaults cannot be decoded are dropped (fail closed).
    async fn discover_raydium(
        &self,
        min_tvl: f64,
        limit: usize,
    ) -> anyhow::Result<Vec<DiscoveredPoolState>> {
        struct RaydiumCandidate {
            pool_id: PoolId,
            protocol: DexProtocol,
            base_mint: MintId,
            quote_mint: MintId,
            decimals_a: u8,
            decimals_b: u8,
            fee_bps: u16,
            tvl: f64,
        }

        let url = format!(
            "{RAYDIUM_POOL_LIST_URL}?poolType=all&poolSortField=liquidity&sortType=desc&pageSize={limit}&page=1"
        );
        let body = self.get_json(&url).await?;
        let items = body
            .get("data")
            .and_then(|d| d.get("data"))
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("raydium catalog: missing data.data array"))?;

        let mut candidates: Vec<RaydiumCandidate> = Vec::new();
        for p in items {
            let prog_str = p.get("programId").and_then(|v| v.as_str()).unwrap_or("");
            let protocol = if prog_str == arb_constants::RAYDIUM_V4_PROGRAM_ID {
                DexProtocol::RaydiumV4
            } else if prog_str == arb_constants::RAYDIUM_CPMM_PROGRAM_ID {
                DexProtocol::RaydiumCpmm
            } else if prog_str == arb_constants::RAYDIUM_CLMM_PROGRAM_ID {
                DexProtocol::RaydiumClmm
            } else {
                continue;
            };

            let tvl = p.get("tvl").and_then(|v| v.as_f64()).unwrap_or(0.0);
            if tvl < min_tvl {
                continue;
            }
            let (Some(id), Some(ma), Some(mb)) = (
                p.get("id").and_then(|v| v.as_str()),
                p.get("mintA"),
                p.get("mintB"),
            ) else {
                continue;
            };
            let (Some(addr_a), Some(addr_b)) = (
                ma.get("address").and_then(|v| v.as_str()),
                mb.get("address").and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            // Token-2022 mints are not modelled in legacy/standard paths
            if ma.get("programId").and_then(|v| v.as_str()) != Some(arb_constants::TOKEN_PROGRAM_ID)
                || mb.get("programId").and_then(|v| v.as_str())
                    != Some(arb_constants::TOKEN_PROGRAM_ID)
            {
                continue;
            }
            let (Ok(pool), Ok(base), Ok(quote)) = (
                Pubkey::from_str(id),
                Pubkey::from_str(addr_a),
                Pubkey::from_str(addr_b),
            ) else {
                continue;
            };
            candidates.push(RaydiumCandidate {
                pool_id: PoolId(pool),
                protocol,
                base_mint: MintId(base),
                quote_mint: MintId(quote),
                decimals_a: ma.get("decimals").and_then(|v| v.as_u64()).unwrap_or(0) as u8,
                decimals_b: mb.get("decimals").and_then(|v| v.as_u64()).unwrap_or(0) as u8,
                fee_bps: p
                    .get("feeRate")
                    .and_then(|v| v.as_f64())
                    .map(|f| (f * 10_000.0).round() as u16)
                    .unwrap_or(25),
                tvl,
            });
        }
        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        // Pool accounts → vault pubkeys, batched requests.
        let keys: Vec<String> = candidates.iter().map(|c| c.pool_id.0.to_string()).collect();
        let accounts = fetch_accounts_chunked(&self.rpc, &keys).await;

        let mut out = Vec::new();
        for (i, cand) in candidates.into_iter().enumerate() {
            let Some(data) = accounts
                .get(i)
                .and_then(|a| a.as_ref())
                .and_then(decode_account_data)
            else {
                continue;
            };
            let vaults = match cand.protocol {
                DexProtocol::RaydiumV4 => decode_raydium_vaults(&data),
                DexProtocol::RaydiumCpmm => decode_raydium_cpmm_vaults(&data),
                DexProtocol::RaydiumClmm => decode_raydium_clmm_vaults(&data),
                _ => None,
            };
            let Some((base_vault, quote_vault)) = vaults else {
                continue;
            };
            out.push(DiscoveredPoolState {
                pool_id: cand.pool_id,
                protocol: cand.protocol,
                base_mint: cand.base_mint,
                quote_mint: cand.quote_mint,
                base_vault,
                quote_vault,
                decimals_a: cand.decimals_a,
                decimals_b: cand.decimals_b,
                fee_bps: cand.fee_bps,
                liquidity_usd: cand.tvl,
            });
        }
        Ok(out)
    }

    /// Orca Whirlpool catalog. Decentralised fee tiers (1, 2, 4, 5, 8, 16, 64,
    /// 128... bps) come straight from the catalog and are re-verified on chain.
    async fn discover_orca(
        &self,
        min_tvl: f64,
        limit: usize,
    ) -> anyhow::Result<Vec<DiscoveredPoolState>> {
        let body = self.get_json(ORCA_CATALOG_URL).await?;

        let pools = body
            .get("whirlpools")
            .and_then(|v| v.as_array())
            .ok_or_else(|| anyhow::anyhow!("orca catalog: missing `whirlpools` array"))?;

        let mut out = Vec::new();
        for p in pools {
            let tvl = p.get("tvl").and_then(|v| v.as_f64()).unwrap_or(0.0);
            if tvl < min_tvl {
                continue;
            }
            let token_a = match p.get("tokenA") {
                Some(t) => t,
                None => continue,
            };
            let token_b = match p.get("tokenB") {
                Some(t) => t,
                None => continue,
            };
            // Token-2022 pools can carry transfer fees/extensions the quote does
            // not model — skip them rather than misprice them.
            if token_a.get("token2022").and_then(|v| v.as_bool()) == Some(true)
                || token_b.get("token2022").and_then(|v| v.as_bool()) == Some(true)
            {
                continue;
            }
            let (Some(mint_a), Some(mint_b)) = (
                token_a.get("mint").and_then(|v| v.as_str()),
                token_b.get("mint").and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            let (Ok(base), Ok(quote)) = (Pubkey::from_str(mint_a), Pubkey::from_str(mint_b)) else {
                continue;
            };
            let Some(addr) = p.get("address").and_then(|v| v.as_str()) else {
                continue;
            };
            let Ok(pool) = Pubkey::from_str(addr) else {
                continue;
            };
            // Orca publishes the LP fee as a fraction (0.0004 == 4 bps).
            let fee_bps = p
                .get("lpFeeRate")
                .and_then(|v| v.as_f64())
                .map(|f| (f * 10_000.0).round() as u16)
                .unwrap_or(0);
            out.push(DiscoveredPoolState {
                pool_id: PoolId(pool),
                protocol: DexProtocol::OrcaWhirlpool,
                base_mint: MintId(base),
                quote_mint: MintId(quote),
                base_vault: pool,
                quote_vault: pool,
                decimals_a: token_a
                    .get("decimals")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u8,
                decimals_b: token_b
                    .get("decimals")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u8,
                fee_bps,
                liquidity_usd: tvl,
            });
            if out.len() >= limit {
                break;
            }
        }
        Ok(out)
    }

    /// Meteora DLMM pools that trade `hub`. The `DLMM` label separates the
    /// LbPair program from Meteora's DAMM (`DYN`) pools, which have a different
    /// account layout and must not be registered as DLMM.
    async fn discover_meteora_meteora_hub(
        &self,
        hub: &MintId,
        min_tvl: f64,
    ) -> anyhow::Result<Vec<DiscoveredPoolState>> {
        let url = format!("{DEXSCREENER_TOKEN_PAIRS}/{}", hub.0);
        let body = self.get_json(&url).await?;
        let pairs = body
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("dexscreener: non-array token pairs"))?;

        let mut out = Vec::new();
        for pair in pairs {
            if pair.get("dexId").and_then(|v| v.as_str()) != Some("meteora") {
                continue;
            }
            let labels = pair.get("labels").and_then(|v| v.as_array());
            let is_dlmm = labels
                .map(|ls| ls.iter().any(|l| l.as_str() == Some("DLMM")))
                .unwrap_or(false);
            if !is_dlmm {
                continue;
            }
            let liq = pair
                .get("liquidity")
                .and_then(|l| l.get("usd"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            if liq < min_tvl {
                continue;
            }
            let (Some(addr), Some(base), Some(quote)) = (
                pair.get("pairAddress").and_then(|v| v.as_str()),
                pair.get("baseToken")
                    .and_then(|t| t.get("address"))
                    .and_then(|v| v.as_str()),
                pair.get("quoteToken")
                    .and_then(|t| t.get("address"))
                    .and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            let (Ok(pool), Ok(base), Ok(quote)) = (
                Pubkey::from_str(addr),
                Pubkey::from_str(base),
                Pubkey::from_str(quote),
            ) else {
                continue;
            };
            out.push(DiscoveredPoolState {
                pool_id: PoolId(pool),
                protocol: DexProtocol::MeteoraDlmm,
                base_mint: MintId(base),
                quote_mint: MintId(quote),
                // Vaults are not used to price a DLMM (bins are), so the pool
                // address stands in for them rather than a guessed account.
                base_vault: pool,
                quote_vault: pool,
                // Filled in by `resolve_mint_decimals` below.
                decimals_a: 0,
                decimals_b: 0,
                fee_bps: 0,
                liquidity_usd: liq,
            });
        }
        Ok(out)
    }

    /// Search DexScreener for active Meteora DLMM pools across common Solana hubs.
    async fn discover_meteora_search(
        &self,
        min_tvl: f64,
    ) -> anyhow::Result<Vec<DiscoveredPoolState>> {
        let queries = ["meteora", "SOL%20meteora", "USDC%20meteora"];
        let mut out = Vec::new();
        let mut seen = HashSet::new();

        for q in queries {
            let url = format!("{DEXSCREENER_SEARCH_URL}?q={q}");
            let body = match self.get_json(&url).await {
                Ok(b) => b,
                Err(_) => continue,
            };
            let pairs = match body.get("pairs").and_then(|v| v.as_array()) {
                Some(p) => p,
                None => continue,
            };
            for pair in pairs {
                if pair.get("dexId").and_then(|v| v.as_str()) != Some("meteora") {
                    continue;
                }
                let is_dlmm = pair
                    .get("labels")
                    .and_then(|v| v.as_array())
                    .map(|ls| ls.iter().any(|l| l.as_str() == Some("DLMM")))
                    .unwrap_or(false);
                if !is_dlmm {
                    continue;
                }
                let liq = pair
                    .get("liquidity")
                    .and_then(|l| l.get("usd"))
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                if liq < min_tvl {
                    continue;
                }
                let (Some(addr), Some(base), Some(quote)) = (
                    pair.get("pairAddress").and_then(|v| v.as_str()),
                    pair.get("baseToken")
                        .and_then(|t| t.get("address"))
                        .and_then(|v| v.as_str()),
                    pair.get("quoteToken")
                        .and_then(|t| t.get("address"))
                        .and_then(|v| v.as_str()),
                ) else {
                    continue;
                };
                let (Ok(pool), Ok(base_pk), Ok(quote_pk)) = (
                    Pubkey::from_str(addr),
                    Pubkey::from_str(base),
                    Pubkey::from_str(quote),
                ) else {
                    continue;
                };
                if seen.insert(pool) {
                    out.push(DiscoveredPoolState {
                        pool_id: PoolId(pool),
                        protocol: DexProtocol::MeteoraDlmm,
                        base_mint: MintId(base_pk),
                        quote_mint: MintId(quote_pk),
                        base_vault: pool,
                        quote_vault: pool,
                        decimals_a: 0,
                        decimals_b: 0,
                        fee_bps: 0,
                        liquidity_usd: liq,
                    });
                }
            }
        }
        Ok(out)
    }

    /// Read mint decimals (offset 44) for each mint, in batched
    /// `getMultipleAccounts` calls. Mints owned by Token-2022 are deliberately
    /// omitted from the result so their pools are dropped.
    async fn resolve_mint_decimals(&self, mints: &[MintId]) -> HashMap<MintId, u8> {
        let mut out = HashMap::new();
        let unique: Vec<MintId> = {
            let mut seen = HashSet::new();
            mints
                .iter()
                .copied()
                .filter(|m| seen.insert(*m))
                .filter(|m| match self.decimals_cache.lock().get(m) {
                    Some(d) => {
                        out.insert(*m, *d);
                        false
                    }
                    None => true,
                })
                .collect()
        };
        if unique.is_empty() {
            return out;
        }

        for chunk in unique.chunks(RPC_BATCH) {
            let keys: Vec<String> = chunk.iter().map(|m| m.0.to_string()).collect();
            let refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
            let accounts = match self.rpc.get_multiple_accounts(&refs).await {
                Ok(a) => a,
                Err(e) => {
                    warn!("resolve_mint_decimals: {e}");
                    continue;
                }
            };
            for (mint, acc) in chunk.iter().zip(accounts.iter()) {
                let Some(acc) = acc else { continue };
                // Only the base token program is modelled: Token-2022 mints can
                // carry transfer fees that would silently shrink the output.
                if acc.get("owner").and_then(|v| v.as_str())
                    != Some(arb_constants::TOKEN_PROGRAM_ID)
                {
                    continue;
                }
                let Some(b64) = acc
                    .get("data")
                    .and_then(|d| d.get(0))
                    .and_then(|s| s.as_str())
                else {
                    continue;
                };
                let Ok(bytes) = base64::prelude::BASE64_STANDARD.decode(b64) else {
                    continue;
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
                    let mint_auth_tag =
                        u32::from_le_bytes(bytes[0..4].try_into().unwrap_or([1, 0, 0, 0]));
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
                out.insert(*mint, bytes[44]);
                self.decimals_cache.lock().insert(*mint, bytes[44]);
            }
        }
        out
    }
}

/// Choose the watch set from a TVL-sorted pool list, reserving up to
/// `max_per_venue` (default 33) most active pools for each DEX venue.
///
/// Pure "top N by TVL" is the wrong objective for arbitrage: the deepest pools
/// on Solana are all on one venue, so a global top list can be 100% Orca and the
/// bot can never see a cross-venue pair — zero cycles, forever. Selecting top N
/// active pools per venue guarantees multi-DEX coverage without RPC saturation.
pub fn select_watch_set_per_venue(
    found: Vec<DiscoveredPoolState>,
    max_per_venue: usize,
    max_pools: usize,
) -> Vec<DiscoveredPoolState> {
    if found.is_empty() || max_pools == 0 || max_per_venue == 0 {
        return Vec::new();
    }

    let mut venues: Vec<DexProtocol> = found.iter().map(|p| p.protocol).collect();
    venues.sort_by_key(|p| *p as u8);
    venues.dedup();

    let quota = if max_pools >= venues.len().saturating_mul(max_per_venue) {
        max_per_venue
    } else {
        max_pools.div_ceil(venues.len()).max(1)
    };

    let mut taken = vec![false; found.len()];
    let mut selected: Vec<usize> = Vec::with_capacity(max_pools.min(found.len()));
    for venue in &venues {
        let mut n = 0usize;
        for (i, p) in found.iter().enumerate() {
            if n >= quota || selected.len() >= max_pools {
                break;
            }
            if p.protocol == *venue && !taken[i] {
                taken[i] = true;
                selected.push(i);
                n += 1;
            }
        }
    }

    // Only fill remainder by global TVL if global max_pools is tighter than per-venue quotas
    if max_pools < venues.len().saturating_mul(max_per_venue) {
        for (i, _) in found.iter().enumerate() {
            if selected.len() >= max_pools {
                break;
            }
            if !taken[i] {
                taken[i] = true;
                selected.push(i);
            }
        }
    }

    found
        .into_iter()
        .enumerate()
        .filter(|(i, _)| taken[*i])
        .map(|(_, p)| p)
        .collect()
}

/// Convenience helper selecting the top 33 most active pools per DEX venue.
pub fn select_top_33_per_venue(found: Vec<DiscoveredPoolState>) -> Vec<DiscoveredPoolState> {
    select_watch_set_per_venue(found, 33, 33 * 8)
}

/// Backwards-compatible wrapper for `select_watch_set_per_venue` with default 33 per venue.
pub fn select_watch_set(
    found: Vec<DiscoveredPoolState>,
    max_pools: usize,
) -> Vec<DiscoveredPoolState> {
    select_watch_set_per_venue(found, 33, max_pools)
}

/// Selects the dual 30-pool watch set:
/// - Exactly up to 30 top Pump.fun bonding curves (sorted by volume/liquidity)
/// - Exactly up to 30 top Multi-DEX pools (Raydium, Orca Whirlpool, Meteora DLMM)
///
/// Returns a combined prioritized list of up to 60 pools for active sub-ms surveillance.
pub fn select_dual_30_watch_set(found: Vec<DiscoveredPoolState>) -> Vec<DiscoveredPoolState> {
    let mut pump_pools = Vec::new();
    let mut multidex_pools = Vec::new();

    for pool in found {
        if pool.protocol == DexProtocol::PumpFun {
            pump_pools.push(pool);
        } else {
            multidex_pools.push(pool);
        }
    }

    // Sort descending by liquidity / score
    pump_pools.sort_by(|a, b| {
        b.liquidity_usd
            .partial_cmp(&a.liquidity_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    multidex_pools.sort_by(|a, b| {
        b.liquidity_usd
            .partial_cmp(&a.liquidity_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut result = Vec::with_capacity(60);
    result.extend(pump_pools.into_iter().take(30));
    result.extend(multidex_pools.into_iter().take(30));
    result
}

/// Register discovered pools into a registry.
pub fn register_discovered(
    registry: &mut crate::registry::MultiDexRegistry,
    pools: &[DiscoveredPoolState],
) {
    for pool in pools {
        registry.register(crate::registry::RegisteredPool {
            pool_id: pool.pool_id,
            protocol: pool.protocol,
            base_mint: pool.base_mint,
            quote_mint: pool.quote_mint,
            decimals_a: pool.decimals_a,
            decimals_b: pool.decimals_b,
            base_vault: pool.base_vault,
            quote_vault: pool.quote_vault,
            fee_bps: pool.fee_bps,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_config_is_bounded() {
        let cfg = DiscoveryConfig::default();
        assert!(cfg.max_pools > 0 && cfg.max_pools <= 1500);
        assert!(cfg.min_tvl_usd >= 1_000.0);
        assert!(!cfg.hub_mints.is_empty());
    }

    #[test]
    fn default_hubs_include_sol_and_usdc() {
        let cfg = DiscoveryConfig::default();
        let wsol = Pubkey::from_str(arb_constants::WSOL_MINT).unwrap();
        let usdc = Pubkey::from_str(arb_constants::USDC_MINT).unwrap();
        assert!(cfg.hub_mints.iter().any(|m| m.0 == wsol));
        assert!(cfg.hub_mints.iter().any(|m| m.0 == usdc));
    }

    fn pool(protocol: DexProtocol, seed: u8, tvl: f64) -> DiscoveredPoolState {
        let mut id = [0u8; 32];
        id[0] = seed;
        DiscoveredPoolState {
            pool_id: PoolId(Pubkey::new_from_array(id)),
            protocol,
            base_mint: MintId(Pubkey::from_str(arb_constants::WSOL_MINT).unwrap()),
            quote_mint: MintId(Pubkey::from_str(arb_constants::USDC_MINT).unwrap()),
            base_vault: Pubkey::default(),
            quote_vault: Pubkey::default(),
            decimals_a: 9,
            decimals_b: 6,
            fee_bps: 4,
            liquidity_usd: tvl,
        }
    }

    #[test]
    fn watch_set_is_capped_and_venue_balanced() {
        // 80 deeper Orca pools vs 20 shallower Raydium pools: a pure "top by TVL"
        // cut would return zero Raydium pools and therefore zero cross-venue
        // pairs. The quota keeps both venues present.
        let mut pools: Vec<_> = (0..80)
            .map(|i| pool(DexProtocol::OrcaWhirlpool, i, 1_000_000.0 - i as f64))
            .collect();
        pools.extend((0..20).map(|i| pool(DexProtocol::RaydiumV4, 200 + i, 400_000.0 - i as f64)));
        let selected = select_watch_set(pools, 20);
        assert_eq!(selected.len(), 20);
        let orca = selected
            .iter()
            .filter(|p| p.protocol == DexProtocol::OrcaWhirlpool)
            .count();
        let raydium = selected
            .iter()
            .filter(|p| p.protocol == DexProtocol::RaydiumV4)
            .count();
        assert_eq!((orca, raydium), (10, 10));
    }

    #[test]
    fn watch_set_keeps_everything_under_the_cap() {
        let pools = (0..5)
            .map(|i| pool(DexProtocol::OrcaWhirlpool, i, 1.0))
            .collect::<Vec<_>>();
        assert_eq!(select_watch_set(pools, 60).len(), 5);
    }

    #[test]
    fn dual_30_watch_set_selects_30_pump_and_30_multidex() {
        let mut pools = Vec::new();
        // 50 pump fun pools
        for i in 0..50 {
            pools.push(pool(DexProtocol::PumpFun, i, 100_000.0 + i as f64));
        }
        // 50 Raydium pools
        for i in 50..100 {
            pools.push(pool(DexProtocol::RaydiumV4, i, 200_000.0 + i as f64));
        }

        let selected = select_dual_30_watch_set(pools);
        assert_eq!(selected.len(), 60);

        let pump_count = selected
            .iter()
            .filter(|p| p.protocol == DexProtocol::PumpFun)
            .count();
        let raydium_count = selected
            .iter()
            .filter(|p| p.protocol == DexProtocol::RaydiumV4)
            .count();

        assert_eq!(pump_count, 30);
        assert_eq!(raydium_count, 30);
    }

    #[test]
    fn watch_set_selects_33_per_venue() {
        let mut pools = Vec::new();
        for i in 0..50u8 {
            pools.push(pool(DexProtocol::RaydiumV4, i, 500_000.0 - i as f64));
            pools.push(pool(
                DexProtocol::OrcaWhirlpool,
                50 + i,
                1_000_000.0 - i as f64,
            ));
            pools.push(pool(
                DexProtocol::MeteoraDlmm,
                100 + i,
                200_000.0 - i as f64,
            ));
            pools.push(pool(DexProtocol::PumpFun, 150 + i, 100_000.0 - i as f64));
        }

        // Pre-sort descending by liquidity_usd just like discover() does
        pools.sort_by(|a, b| {
            b.liquidity_usd
                .partial_cmp(&a.liquidity_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let selected = select_watch_set_per_venue(pools, 33, 200);
        assert_eq!(selected.len(), 132);

        assert_eq!(
            selected
                .iter()
                .filter(|p| p.protocol == DexProtocol::RaydiumV4)
                .count(),
            33
        );
        assert_eq!(
            selected
                .iter()
                .filter(|p| p.protocol == DexProtocol::OrcaWhirlpool)
                .count(),
            33
        );
        assert_eq!(
            selected
                .iter()
                .filter(|p| p.protocol == DexProtocol::MeteoraDlmm)
                .count(),
            33
        );
        assert_eq!(
            selected
                .iter()
                .filter(|p| p.protocol == DexProtocol::PumpFun)
                .count(),
            33
        );
    }

    #[test]
    fn raydium_vaults_decode_from_real_offsets() {
        let a = Pubkey::new_from_array([7u8; 32]);
        let b = Pubkey::new_from_array([9u8; 32]);
        let mut data = vec![0u8; 752];
        data[336..368].copy_from_slice(a.as_ref());
        data[368..400].copy_from_slice(b.as_ref());
        assert_eq!(crate::registry::decode_raydium_vaults(&data), Some((a, b)));
        // Short or zeroed accounts must fail closed, never yield a default key.
        assert_eq!(crate::registry::decode_raydium_vaults(&[0u8; 100]), None);
        assert_eq!(
            crate::registry::decode_raydium_vaults(&vec![0u8; 752]),
            None
        );
    }

    #[test]
    fn watch_set_allocates_quotas_to_cpmm_and_clmm() {
        let mut pools = Vec::new();
        for i in 0..50u8 {
            pools.push(pool(DexProtocol::RaydiumV4, i, 500_000.0 - i as f64));
            pools.push(pool(DexProtocol::RaydiumCpmm, 50 + i, 400_000.0 - i as f64));
            pools.push(pool(
                DexProtocol::RaydiumClmm,
                100 + i,
                300_000.0 - i as f64,
            ));
            pools.push(pool(
                DexProtocol::OrcaWhirlpool,
                150 + i,
                600_000.0 - i as f64,
            ));
        }
        pools.sort_by(|a, b| b.liquidity_usd.partial_cmp(&a.liquidity_usd).unwrap());
        let selected = select_watch_set_per_venue(pools, 20, 100);
        assert_eq!(selected.len(), 80);
        assert_eq!(
            selected
                .iter()
                .filter(|p| p.protocol == DexProtocol::RaydiumCpmm)
                .count(),
            20
        );
        assert_eq!(
            selected
                .iter()
                .filter(|p| p.protocol == DexProtocol::RaydiumClmm)
                .count(),
            20
        );
        assert_eq!(
            selected
                .iter()
                .filter(|p| p.protocol == DexProtocol::RaydiumV4)
                .count(),
            20
        );
        assert_eq!(
            selected
                .iter()
                .filter(|p| p.protocol == DexProtocol::OrcaWhirlpool)
                .count(),
            20
        );
    }
}
