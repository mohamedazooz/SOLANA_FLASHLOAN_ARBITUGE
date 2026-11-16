//! High-Performance Solami WebSocket Real-Time Ingestion Client.
//!
//! Subscribes to `wss://ws.solami.dev/data/subscribe?chain=solana&api_key=...`
//! with live-changeable socket filters for:
//! - Swaps (`pumpswap`, `raydium_clmm`, `meteora_dlmm`, `raydium_cpmm`)
//! - Liquidity changes (mint, burn, dynamic pools)
//! - New token creations & pool launches
//! - Bonding curve graduations (`graduating`, `graduated`)
//! - Shred latency commitments (`processed` / `confirmed`)

use anyhow::Result;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::time::sleep;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, info, warn};

/// Solami live socket filter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolamiStreamConfig {
    pub ws_base_url: String,
    pub api_key: String,
    pub chain: String,
    pub event_types: Vec<String>,
    pub dexes: Option<Vec<String>>,
    pub min_volume_usd: Option<f64>,
    pub commitment: String,
    pub backfill: u32,
}

impl Default for SolamiStreamConfig {
    fn default() -> Self {
        Self {
            ws_base_url: "wss://ws.solami.dev/data/subscribe".to_string(),
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
            commitment: "processed".to_string(), // Shred latency
            backfill: 5,
        }
    }
}

/// Dynamic filter update payload that can be dispatched live over the socket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolamiFilterMessage {
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

/// Incoming event frame from Solami WebSocket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolamiStreamEvent {
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

/// Resolves the WebSocket API key with role separation from the general API
/// key: an explicit override wins, then an `api_key` query parameter embedded
/// in the stream URL, then the general API key fallback.
fn resolve_ws_key(explicit: Option<&str>, ws_url: &str, fallback: &str) -> String {
    if let Some(k) = explicit.map(str::trim).filter(|k| !k.is_empty()) {
        return k.to_string();
    }
    if let Some(qpos) = ws_url.find('?') {
        for pair in ws_url[qpos + 1..].split('&') {
            if let Some(v) = pair.strip_prefix("api_key=") {
                let v = v.trim();
                if !v.is_empty() {
                    return v.to_string();
                }
            }
        }
    }
    fallback.to_string()
}

/// Solami WebSocket Stream Consumer.
pub struct SolamiStreamClient {
    pub config: SolamiStreamConfig,
    is_running: Arc<AtomicBool>,
}

impl SolamiStreamClient {
    pub fn new(config: SolamiStreamConfig) -> Self {
        Self {
            config,
            is_running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Attempts to build client using environment variables:
    /// `SOLAMI_API_KEY`, optional `SOLAMI_WS_API_KEY` (role-separated stream
    /// key) and optional `SOLAMI_WS_URL`.
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("SOLAMI_API_KEY")
            .ok()
            .or_else(|| std::env::var("solami_api_key").ok())
            .or_else(|| dotenvy::var("SOLAMI_API_KEY").ok())
            .or_else(|| dotenvy::var("solami_api_key").ok())?;

        if api_key.is_empty() {
            return None;
        }

        let ws_base = std::env::var("SOLAMI_WS_URL")
            .ok()
            .or_else(|| std::env::var("solami_ws_url").ok())
            .or_else(|| dotenvy::var("SOLAMI_WS_URL").ok())
            .or_else(|| dotenvy::var("solami_ws_url").ok())
            .unwrap_or_else(|| "wss://ws.solami.dev/data/subscribe".to_string());

        let explicit_ws_key = std::env::var("SOLAMI_WS_API_KEY")
            .ok()
            .or_else(|| dotenvy::var("SOLAMI_WS_API_KEY").ok());

        let mut config = SolamiStreamConfig {
            api_key: resolve_ws_key(explicit_ws_key.as_deref(), &ws_base, &api_key),
            ..Default::default()
        };
        if !ws_base.is_empty() {
            // Strip any query params if provided raw
            if let Some(pos) = ws_base.find('?') {
                config.ws_base_url = ws_base[..pos].to_string();
            } else {
                config.ws_base_url = ws_base;
            }
        }

        Some(Self::new(config))
    }

    /// Builds the authorized connection URL with query parameters.
    pub fn build_connection_url(&self) -> String {
        format!(
            "{}?chain={}&api_key={}",
            self.config.ws_base_url, self.config.chain, self.config.api_key
        )
    }

    /// Stops the streaming loop.
    pub fn stop(&self) {
        self.is_running.store(false, Ordering::Relaxed);
    }

    /// Starts streaming events with auto-reconnect and delivers parsed events to the callback.
    pub async fn stream_events<F>(&self, mut on_event: F) -> Result<()>
    where
        F: FnMut(SolamiStreamEvent) + Send + 'static,
    {
        self.is_running.store(true, Ordering::Relaxed);
        let mut reconnect_delay = Duration::from_millis(500);

        while self.is_running.load(Ordering::Relaxed) {
            let url = self.build_connection_url();
            info!(target: "solami_stream", "Connecting to Solami WebSocket stream...");

            match connect_async(&url).await {
                Ok((mut ws_stream, response)) => {
                    info!(
                        target: "solami_stream",
                        status = %response.status(),
                        "Connected to Solami WebSocket successfully"
                    );
                    reconnect_delay = Duration::from_millis(500); // Reset backoff

                    // Send initial dynamic subscription filters
                    let subscribe_msg = SolamiFilterMessage {
                        action: "subscribe".to_string(),
                        types: Some(self.config.event_types.clone()),
                        dex: self.config.dexes.clone(),
                        address: None,
                        pool: None,
                        min_volume_usd: self.config.min_volume_usd,
                        commitment: Some(self.config.commitment.clone()),
                        backfill: Some(self.config.backfill),
                    };

                    if let Ok(filter_json) = serde_json::to_string(&subscribe_msg) {
                        debug!(target: "solami_stream", filter = %filter_json, "Sending filter config");
                        let _ = ws_stream.send(Message::Text(filter_json)).await;
                    }

                    // Process stream frames
                    while let Some(msg_result) = ws_stream.next().await {
                        if !self.is_running.load(Ordering::Relaxed) {
                            break;
                        }

                        match msg_result {
                            Ok(Message::Text(text)) => {
                                // Parse event or batch of events
                                if let Ok(event) = serde_json::from_str::<SolamiStreamEvent>(&text)
                                {
                                    on_event(event);
                                } else if let Ok(events) =
                                    serde_json::from_str::<Vec<SolamiStreamEvent>>(&text)
                                {
                                    for ev in events {
                                        on_event(ev);
                                    }
                                } else {
                                    debug!(target: "solami_stream", raw = %text, "Unparsed Solami frame");
                                }
                            }
                            Ok(Message::Binary(_)) => {
                                // Ignore raw binary frames
                            }
                            Ok(Message::Ping(p)) => {
                                let _ = ws_stream.send(Message::Pong(p)).await;
                            }
                            Ok(Message::Pong(_)) => {}
                            Ok(Message::Close(cf)) => {
                                warn!(target: "solami_stream", close_frame = ?cf, "Solami stream closed by peer");
                                break;
                            }
                            Ok(Message::Frame(_)) => {}
                            Err(e) => {
                                error!(target: "solami_stream", error = %e, "WebSocket stream read error");
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    error!(target: "solami_stream", error = %e, "Failed to connect to Solami WebSocket");
                }
            }

            if self.is_running.load(Ordering::Relaxed) {
                warn!(
                    target: "solami_stream",
                    retry_in = ?reconnect_delay,
                    "Reconnecting to Solami stream in backoff period"
                );
                sleep(reconnect_delay).await;
                reconnect_delay = (reconnect_delay * 2).min(Duration::from_secs(10));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_solami_stream_config_url_builder() {
        let config = SolamiStreamConfig {
            ws_base_url: "wss://ws.solami.dev/data/subscribe".to_string(),
            api_key: "sk_test_api_key_12345".to_string(),
            chain: "solana".to_string(),
            event_types: vec!["swap".to_string(), "liquidity".to_string()],
            dexes: Some(vec!["raydium_clmm".to_string(), "meteora_dlmm".to_string()]),
            min_volume_usd: Some(10.0),
            commitment: "processed".to_string(),
            backfill: 20,
        };

        let client = SolamiStreamClient::new(config);
        let url = client.build_connection_url();
        assert_eq!(
            url,
            "wss://ws.solami.dev/data/subscribe?chain=solana&api_key=sk_test_api_key_12345"
        );
    }

    #[test]
    fn resolve_ws_key_prefers_explicit_override() {
        assert_eq!(
            resolve_ws_key(
                Some("ws_key_explicit"),
                "wss://ws.solami.dev/ws/sol?api_key=ws_key_embedded",
                "sk_general"
            ),
            "ws_key_explicit"
        );
        assert_eq!(
            resolve_ws_key(Some("   "), "wss://ws.solami.dev/ws/sol", "sk_general"),
            "sk_general"
        );
    }

    #[test]
    fn resolve_ws_key_extracts_embedded_url_param() {
        assert_eq!(
            resolve_ws_key(
                None,
                "wss://ws.solami.dev/ws/sol?api_key=ws_key_embedded",
                "sk_general"
            ),
            "ws_key_embedded"
        );
        assert_eq!(
            resolve_ws_key(
                None,
                "wss://ws.solami.dev/ws/sol?chain=solana&api_key=ws_key_embedded&x=1",
                "sk_general"
            ),
            "ws_key_embedded"
        );
        assert_eq!(
            resolve_ws_key(
                None,
                "wss://ws.solami.dev/ws/sol?chain=solana",
                "sk_general"
            ),
            "sk_general"
        );
    }

    #[test]
    fn resolve_ws_key_falls_back_to_general_api_key() {
        assert_eq!(
            resolve_ws_key(None, "wss://ws.solami.dev/ws/sol", "sk_general"),
            "sk_general"
        );
        assert_eq!(
            resolve_ws_key(None, "wss://ws.solami.dev/data/subscribe", "sk_general"),
            "sk_general"
        );
    }

    #[test]
    fn test_solami_filter_message_serialization() {
        let filter = SolamiFilterMessage {
            action: "subscribe".to_string(),
            types: Some(vec!["swap".to_string(), "pool_create".to_string()]),
            dex: Some(vec!["pumpswap".to_string(), "raydium_clmm".to_string()]),
            address: None,
            pool: None,
            min_volume_usd: Some(5.0),
            commitment: Some("processed".to_string()),
            backfill: Some(50),
        };

        let json_str = serde_json::to_string(&filter).expect("Serialization failed");
        assert!(json_str.contains("\"action\":\"subscribe\""));
        assert!(json_str.contains("\"types\":[\"swap\",\"pool_create\"]"));
        assert!(json_str.contains("\"commitment\":\"processed\""));
        assert!(json_str.contains("\"backfill\":50"));
    }

    #[test]
    fn test_solami_stream_event_deserialization() {
        let raw_json = r#"{
            "type": "swap",
            "signature": "5tV8M...solami_stream_sig",
            "slot": 298103991,
            "dex": "pumpswap",
            "pool": "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2",
            "address": "So11111111111111111111111111111111111111112",
            "side": "buy",
            "amount_usd": 1500.50,
            "base_amount": 10000000000,
            "quote_amount": 1500500000,
            "backfill": false
        }"#;

        let event: SolamiStreamEvent =
            serde_json::from_str(raw_json).expect("Deserialization failed");
        assert_eq!(event.event_type, "swap");
        assert_eq!(event.dex.as_deref(), Some("pumpswap"));
        assert_eq!(event.side.as_deref(), Some("buy"));
        assert_eq!(event.amount_usd, Some(1500.50));
        assert_eq!(event.slot, Some(298103991));
    }
}
