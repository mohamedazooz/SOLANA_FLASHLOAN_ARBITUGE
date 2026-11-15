//! Dedicated Solami Blur Real-Time Event Stream Client.
//!
//! Solami Blur is a decoded event stream (JSON over WebSocket).
//! Endpoint: `wss://ws.solami.dev/data/subscribe?chain=solana&api_key=...`
//!
//! Provides real-time decoded events:
//! - Token launch events (`token_create`)
//! - Pool creation events (`pool_create`)
//! - Decoded DEX trades (`swap`)
//! - Liquidity changes (`liquidity`)
//! - Bonding curve graduations (`graduating`, `graduated`)
//!
//! NOTE: Standard Solana JSON-RPC methods must NEVER be sent to Blur.

use anyhow::Result;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tokio::time::sleep;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolamiBlurConfig {
    pub ws_url: String,
    pub api_key: String,
    pub chain: String,
    pub event_types: Vec<String>,
    pub dexes: Option<Vec<String>>,
    pub min_volume_usd: Option<f64>,
    pub commitment: String,
    pub backfill: u32,
    pub dedupe_window: usize,
}

impl Default for SolamiBlurConfig {
    fn default() -> Self {
        Self {
            ws_url: "wss://ws.solami.dev/data/subscribe".to_string(),
            api_key: String::new(),
            chain: "solana".to_string(),
            event_types: vec![
                "swap".to_string(),
                "liquidity".to_string(),
                "token_create".to_string(),
                "pool_create".to_string(),
                "graduating".to_string(),
                "graduated".to_string(),
            ],
            dexes: None,
            min_volume_usd: Some(1.0),
            commitment: "processed".to_string(),
            backfill: 5,
            dedupe_window: 10_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolamiBlurFilterMessage {
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub types: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dex: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pool: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_volume_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commitment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backfill: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolamiBlurEvent {
    #[serde(default, rename = "type")]
    pub event_type: String,
    pub signature: Option<String>,
    pub slot: Option<u64>,
    pub timestamp: Option<i64>,
    pub dex: Option<String>,
    pub pool: Option<String>,
    pub address: Option<String>,
    pub mint: Option<String>,
    pub trader: Option<String>,
    pub side: Option<String>,
    pub amount_usd: Option<f64>,
    pub volume_usd: Option<f64>,
    pub base_amount: Option<u64>,
    pub quote_amount: Option<u64>,
    pub backfill: Option<bool>,
}

pub struct SolamiBlurClient {
    pub config: SolamiBlurConfig,
    pub is_connected: Arc<AtomicBool>,
    pub last_observed_slot: Arc<AtomicU64>,
    seen_signatures: parking_lot::RwLock<HashSet<String>>,
}

impl SolamiBlurClient {
    pub fn new(config: SolamiBlurConfig) -> Self {
        Self {
            config,
            is_connected: Arc::new(AtomicBool::new(false)),
            last_observed_slot: Arc::new(AtomicU64::new(0)),
            seen_signatures: parking_lot::RwLock::new(HashSet::new()),
        }
    }

    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("SOLAMI_BLUR_API_KEY")
            .or_else(|_| std::env::var("SOLAMI_API_KEY"))
            .or_else(|_| dotenvy::var("SOLAMI_BLUR_API_KEY"))
            .or_else(|_| dotenvy::var("SOLAMI_API_KEY"))
            .ok()?;

        let ws_url = std::env::var("SOLAMI_BLUR_WS_URL")
            .or_else(|_| std::env::var("SOLAMI_WS_URL"))
            .or_else(|_| dotenvy::var("SOLAMI_BLUR_WS_URL"))
            .or_else(|_| dotenvy::var("SOLAMI_WS_URL"))
            .unwrap_or_else(|_| "wss://ws.solami.dev/data/subscribe".to_string());

        let config = SolamiBlurConfig {
            api_key,
            ws_url,
            ..Default::default()
        };

        Some(Self::new(config))
    }

    pub fn build_url(&self) -> String {
        let mut url = self.config.ws_url.clone();
        let query_delim = if url.contains('?') { "&" } else { "?" };
        if !url.contains("api_key=") && !self.config.api_key.is_empty() {
            url.push_str(&format!(
                "{query_delim}chain={}&api_key={}",
                self.config.chain, self.config.api_key
            ));
        }
        url
    }

    pub fn is_duplicate(&self, signature: &str) -> bool {
        let mut seen = self.seen_signatures.write();
        if seen.contains(signature) {
            return true;
        }
        if seen.len() >= self.config.dedupe_window {
            seen.clear();
        }
        seen.insert(signature.to_string());
        false
    }

    pub async fn run_stream<F>(&self, shutdown: Arc<AtomicBool>, mut on_event: F) -> Result<()>
    where
        F: FnMut(SolamiBlurEvent) + Send + 'static,
    {
        let url = self.build_url();
        let mut backoff = Duration::from_millis(500);

        while !shutdown.load(Ordering::Relaxed) {
            info!(target: "solami_blur", "Connecting to Solami Blur at {}", self.config.ws_url);
            match connect_async(&url).await {
                Ok((ws_stream, _)) => {
                    info!(target: "solami_blur", "Connected to Solami Blur successfully");
                    self.is_connected.store(true, Ordering::Release);
                    backoff = Duration::from_millis(500);

                    let (mut write, mut read) = ws_stream.split();

                    let filter = SolamiBlurFilterMessage {
                        action: "subscribe".to_string(),
                        types: Some(self.config.event_types.clone()),
                        dex: self.config.dexes.clone(),
                        address: None,
                        pool: None,
                        min_volume_usd: self.config.min_volume_usd,
                        commitment: Some(self.config.commitment.clone()),
                        backfill: Some(self.config.backfill),
                    };

                    if let Ok(filter_json) = serde_json::to_string(&filter) {
                        debug!(target: "solami_blur", filter = %filter_json, "Subscribing Blur filters");
                        let _ = write.send(Message::Text(filter_json)).await;
                    }

                    while let Some(msg_result) = read.next().await {
                        if shutdown.load(Ordering::Relaxed) {
                            break;
                        }

                        match msg_result {
                            Ok(Message::Text(text)) => {
                                if let Ok(event) = serde_json::from_str::<SolamiBlurEvent>(&text) {
                                    if let Some(ref sig) = event.signature {
                                        if self.is_duplicate(sig) {
                                            continue;
                                        }
                                    }
                                    if let Some(slot) = event.slot {
                                        self.last_observed_slot.fetch_max(slot, Ordering::AcqRel);
                                    }
                                    on_event(event);
                                } else if let Ok(events) =
                                    serde_json::from_str::<Vec<SolamiBlurEvent>>(&text)
                                {
                                    for ev in events {
                                        if let Some(ref sig) = ev.signature {
                                            if self.is_duplicate(sig) {
                                                continue;
                                            }
                                        }
                                        if let Some(slot) = ev.slot {
                                            self.last_observed_slot
                                                .fetch_max(slot, Ordering::AcqRel);
                                        }
                                        on_event(ev);
                                    }
                                } else {
                                    debug!(target: "solami_blur", raw = %text, "Unparsed Blur frame");
                                }
                            }
                            Ok(Message::Ping(payload)) => {
                                let _ = write.send(Message::Pong(payload)).await;
                            }
                            Ok(Message::Close(cf)) => {
                                warn!(target: "solami_blur", close_frame = ?cf, "Blur stream closed by peer");
                                break;
                            }
                            Err(e) => {
                                error!(target: "solami_blur", error = %e, "Blur stream error");
                                break;
                            }
                            _ => {}
                        }
                    }
                }
                Err(e) => {
                    error!(target: "solami_blur", error = %e, "Failed to connect to Solami Blur");
                }
            }

            self.is_connected.store(false, Ordering::Release);
            sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(10));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blur_url_builder() {
        let config = SolamiBlurConfig {
            ws_url: "wss://ws.solami.dev/data/subscribe".to_string(),
            api_key: "sk_test_123".to_string(),
            ..Default::default()
        };
        let client = SolamiBlurClient::new(config);
        let url = client.build_url();
        assert_eq!(
            url,
            "wss://ws.solami.dev/data/subscribe?chain=solana&api_key=sk_test_123"
        );
    }

    #[test]
    fn test_blur_deduplication() {
        let client = SolamiBlurClient::new(SolamiBlurConfig::default());
        assert!(!client.is_duplicate("sig_abc"));
        assert!(client.is_duplicate("sig_abc"));
        assert!(!client.is_duplicate("sig_xyz"));
    }
}
