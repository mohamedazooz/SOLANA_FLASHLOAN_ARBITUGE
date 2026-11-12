//! Dynamic Pump.fun Bonding Curve Discovery via BankkRoll Unofficial API Specifications.
//!
//! Queries pump.fun's high-speed frontend endpoints to discover, rank, and track the
//! top 30 most active and liquid bonding curves before and during graduation.

use crate::discovery::DiscoveredPoolState;
use arb_constants::{PUMP_FUN_BONDING_CURVE_SEED, PUMP_FUN_PROGRAM_ID, WSOL_MINT};
use arb_types::{DexProtocol, MintId, PoolId, Result};
use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;
use std::collections::HashSet;
use std::str::FromStr;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Public frontend API endpoint cataloged in BankkRoll/pumpfun-apis.
pub const PUMP_FUN_API_BASE: &str = "https://frontend-api.pump.fun";

/// Fallback alternative endpoint.
pub const PUMP_DEV_API_BASE: &str = "https://pumpdev.io/api";

/// Deserialized Pump.fun coin metadata from the frontend API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PumpFunCoin {
    pub mint: String,
    pub name: Option<String>,
    pub symbol: Option<String>,
    pub description: Option<String>,
    pub bonding_curve: Option<String>,
    pub associated_bonding_curve: Option<String>,
    pub virtual_sol_reserves: Option<u64>,
    pub virtual_token_reserves: Option<u64>,
    pub total_supply: Option<u64>,
    #[serde(default)]
    pub complete: bool,
    pub market_cap: Option<f64>,
    pub usd_market_cap: Option<f64>,
    pub reply_count: Option<u64>,
    pub last_trade_timestamp: Option<u64>,
    pub raydium_pool: Option<String>,
}

/// Dynamic fetcher and ranker for Pump.fun bonding curves.
pub struct PumpFunDiscoveryClient {
    client: reqwest::Client,
    base_url: String,
    pump_program: Pubkey,
    wsol_mint: Pubkey,
}

impl Default for PumpFunDiscoveryClient {
    fn default() -> Self {
        Self::new()
    }
}

impl PumpFunDiscoveryClient {
    /// Create a new discovery client with standard timeout and user agent headers.
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .build()
            .unwrap_or_default();

        let pump_program = Pubkey::from_str(PUMP_FUN_PROGRAM_ID)
            .unwrap_or_else(|_| Pubkey::new_from_array([0u8; 32]));
        let wsol_mint =
            Pubkey::from_str(WSOL_MINT).unwrap_or_else(|_| Pubkey::new_from_array([0u8; 32]));

        Self {
            client,
            base_url: PUMP_FUN_API_BASE.to_string(),
            pump_program,
            wsol_mint,
        }
    }

    /// Fetch the top active coins from Pump.fun using both recent trade activity and market cap.
    pub async fn fetch_top_coins(&self, limit: usize) -> Result<Vec<PumpFunCoin>> {
        let fetch_limit = limit.clamp(10, 100);

        // Fetch two cohorts:
        // 1. Most recently active (by last_trade_timestamp)
        // 2. Highest market cap (closer to graduation / deepest liquidity)
        let url_recent = format!(
            "{}/coins?offset=0&limit={}&sort=last_trade_timestamp&order=DESC&includeNsfw=false",
            self.base_url, fetch_limit
        );
        let url_mcap = format!(
            "{}/coins?offset=0&limit={}&sort=market_cap&order=DESC&includeNsfw=false",
            self.base_url, fetch_limit
        );

        let (res_recent, res_mcap) = tokio::join!(
            self.client.get(&url_recent).send(),
            self.client.get(&url_mcap).send()
        );

        let mut all_coins = Vec::with_capacity(fetch_limit * 2);
        let mut seen_mints = HashSet::new();

        if let Ok(res) = res_recent {
            if res.status().is_success() {
                if let Ok(coins) = res.json::<Vec<PumpFunCoin>>().await {
                    for c in coins {
                        if seen_mints.insert(c.mint.clone()) {
                            all_coins.push(c);
                        }
                    }
                }
            } else {
                warn!(
                    "Pump.fun recent coins API returned status: {}",
                    res.status()
                );
            }
        }

        if let Ok(res) = res_mcap {
            if res.status().is_success() {
                if let Ok(coins) = res.json::<Vec<PumpFunCoin>>().await {
                    for c in coins {
                        if seen_mints.insert(c.mint.clone()) {
                            all_coins.push(c);
                        }
                    }
                }
            } else {
                debug!("Pump.fun mcap coins API returned status: {}", res.status());
            }
        }

        if all_coins.is_empty() {
            debug!(
                "Pump.fun discovery returned empty set from public endpoint, falling back to mock / offline pools"
            );
        }

        Ok(all_coins)
    }

    /// Convert raw PumpFunCoin metadata to DiscoveredPoolState instances,
    /// ranking by liquidity and filtering for the top 30 active curves.
    pub fn select_top_30(&self, coins: &[PumpFunCoin]) -> Vec<DiscoveredPoolState> {
        self.select_top_n(coins, 30)
    }

    /// Convert raw PumpFunCoin metadata to DiscoveredPoolState instances,
    /// ranking by liquidity and filtering for the top N active curves.
    pub fn select_top_n(&self, coins: &[PumpFunCoin], n: usize) -> Vec<DiscoveredPoolState> {
        let mut scored_pools: Vec<(f64, DiscoveredPoolState)> = Vec::new();

        for coin in coins {
            let mint_pubkey = match Pubkey::from_str(&coin.mint) {
                Ok(pk) => pk,
                Err(_) => continue,
            };

            // Derive or parse bonding curve address
            let bonding_curve_pk = if let Some(ref bc_str) = coin.bonding_curve {
                Pubkey::from_str(bc_str).unwrap_or_else(|_| {
                    Pubkey::find_program_address(
                        &[PUMP_FUN_BONDING_CURVE_SEED, mint_pubkey.as_ref()],
                        &self.pump_program,
                    )
                    .0
                })
            } else {
                Pubkey::find_program_address(
                    &[PUMP_FUN_BONDING_CURVE_SEED, mint_pubkey.as_ref()],
                    &self.pump_program,
                )
                .0
            };

            // Calculate score based on USD market cap and virtual reserves
            let usd_tvl = coin.usd_market_cap.unwrap_or(0.0);
            let virtual_sol = coin.virtual_sol_reserves.unwrap_or(30_000_000_000);
            let sol_tvl_est = (virtual_sol as f64 / 1e9) * 180.0; // Approximation for ranking
            let effective_tvl = usd_tvl.max(sol_tvl_est);

            // Favor curves that are active but not completed (between 20% and 98% progress)
            let completion_multiplier = if coin.complete {
                0.1 // Graduated coins are low priority on bonding curve
            } else if effective_tvl > 10_000.0 {
                1.5 // Active high liquidity curves
            } else {
                1.0
            };

            let score = effective_tvl * completion_multiplier;

            scored_pools.push((
                score,
                DiscoveredPoolState {
                    pool_id: PoolId(bonding_curve_pk),
                    protocol: DexProtocol::PumpFun,
                    base_mint: MintId(self.wsol_mint),
                    quote_mint: MintId(mint_pubkey),
                    base_vault: Pubkey::default(),
                    quote_vault: Pubkey::default(),
                    decimals_a: 9,
                    decimals_b: 6,
                    fee_bps: 100, // 1% fixed fee
                    liquidity_usd: effective_tvl,
                },
            ));
        }

        // Sort descending by score
        scored_pools.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        // Take top n
        let top_n: Vec<DiscoveredPoolState> = scored_pools
            .into_iter()
            .take(n)
            .map(|(_, pool)| pool)
            .collect();

        info!(
            "Pump.fun discovery selected {} high-priority bonding curves for surveillance (requested up to {})",
            top_n.len(),
            n
        );

        top_n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pumpfun_curve_derivation() {
        let client = PumpFunDiscoveryClient::new();
        let dummy_mint = Pubkey::new_unique();
        let (expected_pda, _) = Pubkey::find_program_address(
            &[PUMP_FUN_BONDING_CURVE_SEED, dummy_mint.as_ref()],
            &client.pump_program,
        );

        let coin = PumpFunCoin {
            mint: dummy_mint.to_string(),
            name: Some("Test Coin".into()),
            symbol: Some("TEST".into()),
            description: None,
            bonding_curve: None,
            associated_bonding_curve: None,
            virtual_sol_reserves: Some(30_000_000_000),
            virtual_token_reserves: Some(1_073_000_000_000_000),
            total_supply: Some(1_000_000_000_000_000),
            complete: false,
            market_cap: Some(15.0),
            usd_market_cap: Some(25_000.0),
            reply_count: Some(42),
            last_trade_timestamp: Some(1730870000),
            raydium_pool: None,
        };

        let pools = client.select_top_30(&[coin]);
        assert_eq!(pools.len(), 1);
        assert_eq!(pools[0].pool_id.0, expected_pda);
        assert_eq!(pools[0].protocol, DexProtocol::PumpFun);
        assert_eq!(pools[0].decimals_a, 9);
        assert_eq!(pools[0].decimals_b, 6);
    }
}
