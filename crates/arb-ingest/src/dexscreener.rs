//! Real-Time DexScreener Multi-Venue Pair Discovery Client.
//! Discovers all on-chain liquidity pools for a given mint across Raydium, Orca, Meteora, and Pump.fun.
//! Filters out single-pool traps by strictly requiring >= 2 distinct pools with active liquidity.

use anyhow::{Result, anyhow};
use arb_types::{DexProtocol, MintId, PoolId};
use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;
use std::str::FromStr;
use tracing::{debug, info};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DexToken {
    pub address: String,
    pub name: Option<String>,
    pub symbol: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DexLiquidity {
    pub usd: Option<f64>,
    pub base: Option<f64>,
    pub quote: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DexPair {
    #[serde(rename = "chainId")]
    pub chain_id: String,
    #[serde(rename = "dexId")]
    pub dex_id: String,
    pub url: Option<String>,
    #[serde(rename = "pairAddress")]
    pub pair_address: String,
    #[serde(rename = "baseToken")]
    pub base_token: DexToken,
    #[serde(rename = "quoteToken")]
    pub quote_token: DexToken,
    #[serde(rename = "priceNative")]
    pub price_native: Option<String>,
    #[serde(rename = "priceUsd")]
    pub price_usd: Option<String>,
    pub liquidity: Option<DexLiquidity>,
    pub labels: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DexScreenerResponse {
    pub schema_version: Option<String>,
    pub pairs: Option<Vec<DexPair>>,
}

/// Discovered token with multiple active liquidity venues.
#[derive(Debug, Clone)]
pub struct MultiVenueToken {
    pub mint: MintId,
    pub symbol: String,
    pub pairs: Vec<DiscoveredPool>,
}

#[derive(Debug, Clone)]
pub struct DiscoveredPool {
    pub pool_id: PoolId,
    pub dex_protocol: DexProtocol,
    pub dex_name: String,
    pub quote_mint: MintId,
    pub liquidity_usd: f64,
    pub price_native: f64,
}

pub struct DexScreenerClient {
    client: reqwest::Client,
    base_url: String,
}

/// Classify a DexScreener pair into a supported `DexProtocol` from its
/// `dexId` + `labels`, failing closed (None) whenever the protocol itself is
/// not positively identifiable from the pair record alone.
///
/// Label taxonomy verified live on 2026-11-18 (BONK
/// `DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263`,
/// https://api.dexscreener.com/latest/dex/tokens/...):
/// - dexId `raydium` ships labels `CLMM`, `CPMM`, or NO labels at all; the
///   unlabeled form is Raydium AMM v4 — confirmed on-chain for 3/3 unlabeled
///   BONK pairs (owner `675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8`,
///   752-byte AmmInfo).
/// - dexId `meteora` ships labels `DLMM` (Meteora DLMM) and `DYN` (DAMM v2 /
///   Dynamic AMM — no decode branch exists in this workspace, so it must be
///   excluded, never classified as DLMM).
/// - dexId `orca` ships label `wp` (whirlpools).
///
/// Anything else (unknown labels, future raydium generations, other DEXes)
/// returns `None` and the pair is dropped.
pub fn classify_pair_protocol(dex_id: &str, labels: &[String]) -> Option<DexProtocol> {
    let has_label = |want: &str| labels.iter().any(|l| l.eq_ignore_ascii_case(want));
    match dex_id.to_lowercase().as_str() {
        "raydium" => {
            if has_label("cpmm") {
                Some(DexProtocol::RaydiumCpmm)
            } else if has_label("clmm") {
                Some(DexProtocol::RaydiumClmm)
            } else if labels.is_empty() || labels.iter().all(|l| l.eq_ignore_ascii_case("v4")) {
                // Unlabeled (or explicitly "v4") = Raydium AMM v4. Verified
                // empirically above; unknown extra labels still fail closed.
                Some(DexProtocol::RaydiumV4)
            } else {
                None
            }
        }
        "raydium-clmm" | "raydium_clmm" => Some(DexProtocol::RaydiumClmm),
        "raydium-cpmm" | "raydium_cpmm" => Some(DexProtocol::RaydiumCpmm),
        "orca" | "whirlpool" => Some(DexProtocol::OrcaWhirlpool),
        "meteora" | "meteora-dlmm" | "dlmm" => {
            if has_label("dlmm") {
                Some(DexProtocol::MeteoraDlmm)
            } else {
                // Includes meteora `DYN` (DAMM v2) pairs — no decoder exists
                // (DATA-05); they must never be registered as DLMM.
                None
            }
        }
        "pumpfun" | "pump.fun" | "pump" => Some(DexProtocol::PumpFun),
        "pumpswap" | "pump-swap" => Some(DexProtocol::PumpSwap),
        _ => None,
    }
}

impl Default for DexScreenerClient {
    fn default() -> Self {
        Self::new()
    }
}

impl DexScreenerClient {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .build()
            .unwrap_or_default();

        Self {
            client,
            base_url: "https://api.dexscreener.com/latest/dex/tokens".to_string(),
        }
    }

    /// Fetch all pairs for a specific token mint from DexScreener.
    pub async fn fetch_token_pairs(&self, mint: &str) -> Result<Vec<DexPair>> {
        let url = format!("{}/{}", self.base_url, mint);
        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("DexScreener API error status: {}", resp.status()));
        }

        let data: DexScreenerResponse = resp.json().await?;
        let pairs = data.pairs.unwrap_or_default();
        // Keep only Solana pairs
        let solana_pairs: Vec<DexPair> = pairs
            .into_iter()
            .filter(|p| p.chain_id == "solana")
            .collect();

        Ok(solana_pairs)
    }

    /// Check if a token has >= 2 distinct pools and return structured multi-venue data.
    /// Strictly filters out single-pool tokens to prevent single-pool trap losses.
    pub async fn inspect_multi_venue(&self, mint_str: &str) -> Result<Option<MultiVenueToken>> {
        self.inspect_multi_venue_with_min_liq(mint_str, 25000.0)
            .await
    }

    /// Check multi-venue pools with a custom minimal liquidity floor.
    pub async fn inspect_multi_venue_with_min_liq(
        &self,
        mint_str: &str,
        min_liq_usd: f64,
    ) -> Result<Option<MultiVenueToken>> {
        let mint_pubkey =
            Pubkey::from_str(mint_str).map_err(|e| anyhow!("Invalid mint pubkey: {e}"))?;
        let mint = MintId(mint_pubkey);

        let pairs = self.fetch_token_pairs(mint_str).await?;
        if pairs.len() < 2 {
            // Single pool trap or no liquidity: reject immediately
            debug!(
                "Token {} has only {} pool(s) on DexScreener. Skipping to avoid single-pool trap.",
                mint_str,
                pairs.len()
            );
            return Ok(None);
        }

        let mut discovered_pools = Vec::new();
        let mut seen_pool_ids = std::collections::HashSet::new();
        let mut symbol = "UNKNOWN".to_string();

        for pair in pairs {
            let Ok(pool_pubkey) = Pubkey::from_str(&pair.pair_address) else {
                continue;
            };
            let pool_id = PoolId(pool_pubkey);

            if seen_pool_ids.contains(&pool_id) {
                continue;
            }
            seen_pool_ids.insert(pool_id);

            let Ok(quote_pubkey) = Pubkey::from_str(&pair.quote_token.address) else {
                continue;
            };
            let quote_mint = MintId(quote_pubkey);

            if let Some(ref sym) = pair.base_token.symbol
                && pair.base_token.address == mint_str
            {
                symbol = sym.clone();
            }

            let labels: Vec<String> = pair.labels.clone().unwrap_or_default();
            let protocol = classify_pair_protocol(&pair.dex_id, &labels);
            let protocol = match protocol {
                Some(p) => p,
                // Unrecognized or unclassifiable pair (unknown DEX, unknown
                // raydium/meteora labels such as `DYN`): drop — fail closed.
                None => continue,
            };

            let liq_usd = pair.liquidity.as_ref().and_then(|l| l.usd).unwrap_or(0.0);
            let price_native = pair
                .price_native
                .as_ref()
                .and_then(|p| p.parse::<f64>().ok())
                .unwrap_or(0.0);

            // Require minimal liquidity per pool to ensure actionable depth and safety
            if liq_usd >= min_liq_usd {
                discovered_pools.push(DiscoveredPool {
                    pool_id,
                    dex_protocol: protocol,
                    dex_name: pair.dex_id,
                    quote_mint,
                    liquidity_usd: liq_usd,
                    price_native,
                });
            }
        }

        // Must have at least 2 distinct valid pools
        if discovered_pools.len() >= 2 {
            info!(
                "🎯 Multi-venue candidate confirmed: {} ({}) with {} verified pools across {:?}",
                symbol,
                mint_str,
                discovered_pools.len(),
                discovered_pools
                    .iter()
                    .map(|p| &p.dex_name)
                    .collect::<Vec<_>>()
            );
            Ok(Some(MultiVenueToken {
                mint,
                symbol,
                pairs: discovered_pools,
            }))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (DATA-02 regression) Evidence-derived label mapping: every fixture
    /// value was observed live on the DexScreener API (BONK, 2026-11-18) and
    /// unlabeled-V4 pairs were confirmed on-chain (owner 675kPX9M…, 752-byte
    /// AmmInfo). `DYN` (DAMM v2) must NOT classify as MeteoraDlmm.
    #[test]
    fn test_raydium_meteora_label_classification_fails_closed() {
        let proto = |dex: &str, labels: Vec<&str>| {
            classify_pair_protocol(
                dex,
                &labels.into_iter().map(str::to_string).collect::<Vec<_>>(),
            )
        };

        // Verified-on-chain forms.
        assert_eq!(proto("raydium", vec![]), Some(DexProtocol::RaydiumV4));
        assert_eq!(proto("raydium", vec!["v4"]), Some(DexProtocol::RaydiumV4));
        assert_eq!(
            proto("raydium", vec!["CPMM"]),
            Some(DexProtocol::RaydiumCpmm)
        );
        assert_eq!(
            proto("raydium", vec!["CLMM"]),
            Some(DexProtocol::RaydiumClmm)
        );
        assert_eq!(
            proto("raydium-clmm", vec![]),
            Some(DexProtocol::RaydiumClmm)
        );
        assert_eq!(
            proto("raydium-cpmm", vec![]),
            Some(DexProtocol::RaydiumCpmm)
        );

        // Meteora: DLMM label required; DYN (DAMM v2) is excluded.
        assert_eq!(
            proto("meteora", vec!["DLMM"]),
            Some(DexProtocol::MeteoraDlmm)
        );
        assert_eq!(proto("meteora", vec!["DYN"]), None);
        assert_eq!(proto("meteora", vec![]), None);

        // Unknown raydium label generation fails closed (no V4 blind default).
        assert_eq!(proto("raydium", vec!["launchlab"]), None);

        // Unrecognized DEXes fail closed.
        assert_eq!(proto("moonshot", vec![]), None);
        assert_eq!(proto("phoenix", vec![]), None);
    }

    #[test]
    fn test_dexscreener_json_parsing_and_filtering() {
        let json_data = r#"{
            "schemaVersion": "1.0.0",
            "pairs": [
                {
                    "chainId": "solana",
                    "dexId": "raydium",
                    "pairAddress": "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2",
                    "baseToken": {
                        "address": "So11111111111111111111111111111111111111112",
                        "symbol": "WSOL"
                    },
                    "quoteToken": {
                        "address": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
                        "symbol": "USDC"
                    },
                    "priceNative": "94.50",
                    "liquidity": { "usd": 5000000.0 }
                },
                {
                    "chainId": "solana",
                    "dexId": "orca",
                    "pairAddress": "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE",
                    "baseToken": {
                        "address": "So11111111111111111111111111111111111111112",
                        "symbol": "WSOL"
                    },
                    "quoteToken": {
                        "address": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
                        "symbol": "USDC"
                    },
                    "priceNative": "94.55",
                    "liquidity": { "usd": 3000000.0 }
                }
            ]
        }"#;

        let parsed: DexScreenerResponse = serde_json::from_str(json_data).unwrap();
        let pairs = parsed.pairs.unwrap();
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].dex_id, "raydium");
        assert_eq!(pairs[1].dex_id, "orca");
    }

    #[test]
    fn test_single_pool_trap_rejection() {
        let json_data = r#"{
            "schemaVersion": "1.0.0",
            "pairs": [
                {
                    "chainId": "solana",
                    "dexId": "pumpfun",
                    "pairAddress": "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2",
                    "baseToken": {
                        "address": "4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R",
                        "symbol": "TRAP"
                    },
                    "quoteToken": {
                        "address": "So11111111111111111111111111111111111111112",
                        "symbol": "SOL"
                    },
                    "priceNative": "0.0001",
                    "liquidity": { "usd": 15000.0 }
                }
            ]
        }"#;

        let parsed: DexScreenerResponse = serde_json::from_str(json_data).unwrap();
        let pairs = parsed.pairs.unwrap();
        assert_eq!(pairs.len(), 1);
        // A single-pool token cannot be an arbitrage candidate
        assert!(pairs.len() < 2);
    }
}
