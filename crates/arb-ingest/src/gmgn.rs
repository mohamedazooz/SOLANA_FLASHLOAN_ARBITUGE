//! GMGN.ai and DexScreener dynamic token discovery client.
//! Streams and polls trending Solana tokens, smart money inflows, and active liquidity pools.

use anyhow::{Context, Result};
use parking_lot::RwLock;
use reqwest::Client;
use reqwest::header::{
    ACCEPT, ACCEPT_LANGUAGE, HeaderMap, HeaderValue, ORIGIN, REFERER, USER_AGENT,
};
use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

/// Discovered token with market metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendingToken {
    pub address: Pubkey,
    pub symbol: String,
    pub name: String,
    pub price_usd: f64,
    pub liquidity_usd: f64,
    pub volume_24h_usd: f64,
    pub swaps_1h: u64,
}

#[derive(Clone)]
pub struct GmgnClient {
    client: Client,
    endpoint: String,
    #[allow(clippy::type_complexity)]
    snapshot: Arc<RwLock<Option<(Instant, Vec<TrendingToken>)>>>,
    ttl: Duration,
}

impl Default for GmgnClient {
    fn default() -> Self {
        Self::new()
    }
}

impl GmgnClient {
    pub fn new() -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36",
            ),
        );
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/json, text/plain, */*"),
        );
        headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.9"));
        headers.insert(REFERER, HeaderValue::from_static("https://gmgn.ai/"));
        headers.insert(ORIGIN, HeaderValue::from_static("https://gmgn.ai"));
        headers.insert(
            "sec-ch-ua",
            HeaderValue::from_static(
                "\"Chromium\";v=\"128\", \"Not;A=Brand\";v=\"24\", \"Google Chrome\";v=\"128\"",
            ),
        );
        headers.insert("sec-ch-ua-mobile", HeaderValue::from_static("?0"));
        headers.insert(
            "sec-ch-ua-platform",
            HeaderValue::from_static("\"Windows\""),
        );
        headers.insert("sec-fetch-dest", HeaderValue::from_static("empty"));
        headers.insert("sec-fetch-mode", HeaderValue::from_static("cors"));
        headers.insert("sec-fetch-site", HeaderValue::from_static("same-origin"));

        let client = Client::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(8))
            .build()
            .unwrap_or_default();

        Self {
            client,
            endpoint: "https://gmgn.ai/defi/quotation/v1/rank/sol/swaps/1h".to_string(),
            snapshot: Arc::new(RwLock::new(None)),
            ttl: Duration::from_secs(60),
        }
    }

    /// Read currently cached snapshot in memory without triggering network requests or rate limits.
    pub fn get_cached_snapshot(&self) -> Option<Vec<TrendingToken>> {
        self.snapshot
            .read()
            .as_ref()
            .map(|(_, tokens)| tokens.clone())
    }

    /// Fetch trending Solana tokens with snapshot caching and automated fallback.
    /// Re-uses local snapshot if within TTL (60s) to completely prevent 429/403 rate limits.
    pub async fn fetch_trending_tokens(&self, limit: usize) -> Result<Vec<TrendingToken>> {
        // 1. Return fresh snapshot if within TTL
        {
            let lock = self.snapshot.read();
            if let Some((ts, tokens)) = lock.as_ref()
                && ts.elapsed() < self.ttl
                && !tokens.is_empty()
            {
                debug!(
                    "Returning cached trending tokens snapshot (age: {:.1}s)",
                    ts.elapsed().as_secs_f64()
                );
                return Ok(tokens.clone());
            }
        }

        debug!("Refreshing GMGN API snapshot for trending Solana tokens...");
        let gmgn_url = format!(
            "{}?orderby=swaps&direction=desc&limit={}",
            self.endpoint, limit
        );

        match self.client.get(&gmgn_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(json_data) = resp.json::<serde_json::Value>().await
                    && let Some(tokens_array) = json_data["data"]["rank"].as_array()
                {
                    let mut tokens = Vec::new();
                    for item in tokens_array.iter().take(limit) {
                        if let Some(addr_str) = item["address"].as_str()
                            && let Ok(pubkey) = Pubkey::from_str(addr_str)
                        {
                            let symbol = item["symbol"].as_str().unwrap_or("UNKNOWN").to_string();
                            let name = item["name"].as_str().unwrap_or("").to_string();
                            let price_usd = item["price"].as_f64().unwrap_or(0.0);
                            let liquidity_usd = item["liquidity"].as_f64().unwrap_or(0.0);
                            let volume_24h_usd = item["volume"].as_f64().unwrap_or(0.0);
                            let swaps_1h = item["swaps_1h"].as_u64().unwrap_or(0);

                            tokens.push(TrendingToken {
                                address: pubkey,
                                symbol,
                                name,
                                price_usd,
                                liquidity_usd,
                                volume_24h_usd,
                                swaps_1h,
                            });
                        }
                    }
                    if !tokens.is_empty() {
                        info!(
                            "✅ GMGN snapshot refreshed with {} active trending Solana tokens",
                            tokens.len()
                        );
                        *self.snapshot.write() = Some((Instant::now(), tokens.clone()));
                        return Ok(tokens);
                    }
                }
            }
            Ok(resp) => {
                warn!(
                    "GMGN API returned non-success HTTP {}: falling back to DexScreener",
                    resp.status()
                );
            }
            Err(e) => {
                warn!(
                    "GMGN API connection failed ({}): falling back to DexScreener",
                    e
                );
            }
        }

        // Resilient Fallback: DexScreener Token Profiles & Boosts for Solana
        match self.fetch_dexscreener_trending(limit).await {
            Ok(tokens) if !tokens.is_empty() => {
                *self.snapshot.write() = Some((Instant::now(), tokens.clone()));
                Ok(tokens)
            }
            Ok(_) => {
                // If DexScreener returned empty, fallback to stale snapshot if available
                if let Some((_, tokens)) = self.snapshot.read().as_ref() {
                    warn!(
                        "DexScreener empty, reusing previous snapshot of {} tokens",
                        tokens.len()
                    );
                    Ok(tokens.clone())
                } else {
                    Ok(Vec::new())
                }
            }
            Err(e) => {
                if let Some((_, tokens)) = self.snapshot.read().as_ref() {
                    warn!(
                        "Trending refresh failed ({}), reusing previous snapshot of {} tokens",
                        e,
                        tokens.len()
                    );
                    Ok(tokens.clone())
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Fallback provider using DexScreener public REST API with live pair enrichment.
    async fn fetch_dexscreener_trending(&self, limit: usize) -> Result<Vec<TrendingToken>> {
        debug!("Querying DexScreener Solana trending tokens fallback...");
        let url = "https://api.dexscreener.com/token-boosts/top/v1";

        let resp = self
            .client
            .get(url)
            .send()
            .await
            .context("DexScreener API dispatch failed")?;

        let array: Vec<serde_json::Value> = resp.json().await.unwrap_or_default();
        let mut target_addresses = Vec::new();

        for item in array {
            if item["chainId"].as_str() == Some("solana")
                && let Some(token_address) = item["tokenAddress"].as_str()
            {
                target_addresses.push(token_address.to_string());
                if target_addresses.len() >= limit.min(30) {
                    break;
                }
            }
        }

        if target_addresses.is_empty() {
            return Ok(Vec::new());
        }

        // Enrich tokens with live prices, symbols, and liquidity in a single bulk request
        let enrich_url = format!(
            "https://api.dexscreener.com/latest/dex/tokens/{}",
            target_addresses.join(",")
        );

        let mut tokens = Vec::new();
        if let Ok(pairs_resp) = self.client.get(&enrich_url).send().await
            && let Ok(pairs_json) = pairs_resp.json::<serde_json::Value>().await
            && let Some(pairs) = pairs_json["pairs"].as_array()
        {
            let mut seen_addresses = std::collections::HashSet::new();
            for pair in pairs {
                if pair["chainId"].as_str() != Some("solana") {
                    continue;
                }
                if let Some(base) = pair["baseToken"].as_object()
                    && let Some(addr_str) = base.get("address").and_then(|a| a.as_str())
                    && let Ok(pubkey) = Pubkey::from_str(addr_str)
                {
                    if !seen_addresses.insert(pubkey) {
                        continue;
                    }
                    let symbol = base
                        .get("symbol")
                        .and_then(|s| s.as_str())
                        .unwrap_or("TOKEN")
                        .to_string();
                    let name = base
                        .get("name")
                        .and_then(|s| s.as_str())
                        .unwrap_or("")
                        .to_string();
                    let price_usd = pair["priceUsd"]
                        .as_str()
                        .and_then(|s| s.parse::<f64>().ok())
                        .or_else(|| pair["priceUsd"].as_f64())
                        .unwrap_or(0.0);
                    let liquidity_usd = pair["liquidity"]["usd"].as_f64().unwrap_or(0.0);
                    let volume_24h_usd = pair["volume"]["h24"].as_f64().unwrap_or(0.0);
                    let buys_1h = pair["txns"]["h1"]["buys"].as_u64().unwrap_or(0);
                    let sells_1h = pair["txns"]["h1"]["sells"].as_u64().unwrap_or(0);
                    let swaps_1h = buys_1h + sells_1h;

                    tokens.push(TrendingToken {
                        address: pubkey,
                        symbol,
                        name,
                        price_usd,
                        liquidity_usd,
                        volume_24h_usd,
                        swaps_1h,
                    });
                }
            }
        }

        info!(
            "✅ Discovered & enriched {} trending Solana tokens via DexScreener live API",
            tokens.len()
        );
        Ok(tokens)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gmgn_client_creation() {
        let client = GmgnClient::new();
        assert!(client.endpoint.contains("gmgn.ai"));
    }
}
