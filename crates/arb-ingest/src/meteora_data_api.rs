//! Meteora Official Hosted Data APIs Client.
//!
//! Connects to `https://dlmm-api.meteora.ag` and `https://cp-amm-api.meteora.ag`
//! for background pool discovery, 24h volume ranking, and TVL filtering.
//!
//! Architectural Boundary:
//! - This client runs strictly in the background / offline discovery thread.
//! - Real-time hot-path execution does NOT block on REST HTTP calls.
//! - Filtered pool addresses are forwarded to Yellowstone gRPC Geyser for sub-millisecond state streaming.

use anyhow::{Result, anyhow};
use arb_types::{DexProtocol, MintId, PoolId};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;
use std::str::FromStr;
use std::time::Duration;
use tracing::{info, warn};

pub const DLMM_API_BASE_URL: &str = "https://dlmm-api.meteora.ag";
pub const CP_AMM_API_BASE_URL: &str = "https://cp-amm-api.meteora.ag";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeteoraPoolItem {
    pub address: String,
    pub name: Option<String>,
    #[serde(rename = "mint_x")]
    pub mint_x: Option<String>,
    #[serde(rename = "mint_y")]
    pub mint_y: Option<String>,
    #[serde(rename = "reserve_x")]
    pub reserve_x: Option<String>,
    #[serde(rename = "reserve_y")]
    pub reserve_y: Option<String>,
    #[serde(rename = "reserve_x_amount")]
    pub reserve_x_amount: Option<u64>,
    #[serde(rename = "reserve_y_amount")]
    pub reserve_y_amount: Option<u64>,
    pub bin_step: Option<u16>,
    pub base_fee_percentage: Option<f64>,
    pub max_fee_percentage: Option<f64>,
    pub protocol_fee_percentage: Option<f64>,
    pub liquidity: Option<String>,
    pub trade_volume_24h: Option<f64>,
    pub fee_volume_24h: Option<f64>,
    pub current_price: Option<f64>,
    pub apr: Option<f64>,
    pub is_blacklisted: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeteoraPoolsResponse {
    pub data: Option<Vec<MeteoraPoolItem>>,
    pub total: Option<usize>,
}

/// Discovered Meteora pool candidate for hot-path monitoring.
#[derive(Debug, Clone)]
pub struct DiscoveredMeteoraPool {
    pub pool_id: PoolId,
    pub protocol: DexProtocol,
    pub mint_a: MintId,
    pub mint_b: MintId,
    pub tvl_usd: f64,
    pub volume_24h_usd: f64,
    pub base_fee_bps: u16,
}

pub struct MeteoraDataApiClient {
    client: Client,
    dlmm_base: String,
    cp_amm_base: String,
}

impl Default for MeteoraDataApiClient {
    fn default() -> Self {
        Self::new()
    }
}

impl MeteoraDataApiClient {
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            client,
            dlmm_base: DLMM_API_BASE_URL.to_string(),
            cp_amm_base: CP_AMM_API_BASE_URL.to_string(),
        }
    }

    pub fn with_base_urls(dlmm_url: &str, cp_amm_url: &str) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            client,
            dlmm_base: dlmm_url.to_string(),
            cp_amm_base: cp_amm_url.to_string(),
        }
    }

    /// Fetch DLMM pools page from official hosted data API.
    pub async fn fetch_dlmm_pools(
        &self,
        page: usize,
        limit: usize,
    ) -> Result<Vec<MeteoraPoolItem>> {
        let url = format!("{}/pools?page={}&limit={}", self.dlmm_base, page, limit);
        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("DLMM API error HTTP {}", resp.status()));
        }
        let body: MeteoraPoolsResponse = resp.json().await?;
        Ok(body.data.unwrap_or_default())
    }

    /// Fetch DAMM v2 pools page from official hosted data API.
    pub async fn fetch_damm_v2_pools(
        &self,
        page: usize,
        limit: usize,
    ) -> Result<Vec<MeteoraPoolItem>> {
        let url = format!("{}/pools?page={}&limit={}", self.cp_amm_base, page, limit);
        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("DAMM v2 API error HTTP {}", resp.status()));
        }
        let body: MeteoraPoolsResponse = resp.json().await?;
        Ok(body.data.unwrap_or_default())
    }

    /// Discover high-volume candidate pools across both DLMM and DAMM v2.
    pub async fn discover_top_pools(
        &self,
        min_volume_24h_usd: f64,
        max_pools: usize,
    ) -> Result<Vec<DiscoveredMeteoraPool>> {
        let mut candidates = Vec::new();

        // 1. Fetch DLMM pools
        match self.fetch_dlmm_pools(0, 100).await {
            Ok(pools) => {
                for p in pools {
                    if p.is_blacklisted.unwrap_or(false) {
                        continue;
                    }
                    let vol = p.trade_volume_24h.unwrap_or(0.0);
                    if vol < min_volume_24h_usd {
                        continue;
                    }
                    let Ok(pool_pk) = Pubkey::from_str(&p.address) else {
                        continue;
                    };
                    let Some(mx) = p.mint_x.as_deref().and_then(|s| Pubkey::from_str(s).ok())
                    else {
                        continue;
                    };
                    let Some(my) = p.mint_y.as_deref().and_then(|s| Pubkey::from_str(s).ok())
                    else {
                        continue;
                    };

                    let fee_bps = p
                        .base_fee_percentage
                        .map(|f| (f * 100.0) as u16)
                        .unwrap_or(0);
                    candidates.push(DiscoveredMeteoraPool {
                        pool_id: PoolId(pool_pk),
                        protocol: DexProtocol::MeteoraDlmm,
                        mint_a: MintId(mx),
                        mint_b: MintId(my),
                        tvl_usd: p
                            .liquidity
                            .as_deref()
                            .and_then(|l| l.parse::<f64>().ok())
                            .unwrap_or(0.0),
                        volume_24h_usd: vol,
                        base_fee_bps: fee_bps,
                    });
                }
            }
            Err(e) => {
                warn!("Meteora DLMM Data API discovery error: {}", e);
            }
        }

        // 2. Fetch DAMM v2 pools
        match self.fetch_damm_v2_pools(0, 100).await {
            Ok(pools) => {
                for p in pools {
                    if p.is_blacklisted.unwrap_or(false) {
                        continue;
                    }
                    let vol = p.trade_volume_24h.unwrap_or(0.0);
                    if vol < min_volume_24h_usd {
                        continue;
                    }
                    let Ok(pool_pk) = Pubkey::from_str(&p.address) else {
                        continue;
                    };
                    let Some(mx) = p.mint_x.as_deref().and_then(|s| Pubkey::from_str(s).ok())
                    else {
                        continue;
                    };
                    let Some(my) = p.mint_y.as_deref().and_then(|s| Pubkey::from_str(s).ok())
                    else {
                        continue;
                    };

                    let fee_bps = p
                        .base_fee_percentage
                        .map(|f| (f * 100.0) as u16)
                        .unwrap_or(0);
                    candidates.push(DiscoveredMeteoraPool {
                        pool_id: PoolId(pool_pk),
                        protocol: DexProtocol::MeteoraDammV2,
                        mint_a: MintId(mx),
                        mint_b: MintId(my),
                        tvl_usd: p
                            .liquidity
                            .as_deref()
                            .and_then(|l| l.parse::<f64>().ok())
                            .unwrap_or(0.0),
                        volume_24h_usd: vol,
                        base_fee_bps: fee_bps,
                    });
                }
            }
            Err(e) => {
                warn!("Meteora DAMM v2 Data API discovery error: {}", e);
            }
        }

        // Sort by 24h volume descending and clamp to max_pools
        candidates.sort_by(|a, b| {
            b.volume_24h_usd
                .partial_cmp(&a.volume_24h_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        candidates.truncate(max_pools);

        info!(
            "Discovered {} top Meteora pools by 24h volume",
            candidates.len()
        );
        Ok(candidates)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_initialization() {
        let client = MeteoraDataApiClient::new();
        assert_eq!(client.dlmm_base, DLMM_API_BASE_URL);
        assert_eq!(client.cp_amm_base, CP_AMM_API_BASE_URL);
    }
}
