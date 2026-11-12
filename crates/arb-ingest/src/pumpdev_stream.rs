//! Pumpdev.io Real-Time WebSocket Streaming Client (`wss://pumpdev.io/ws`).
//!
//! Streams instant Pump.fun token creation alerts, live bonding curve trades,
//! and graduation events with minimal latency to feed the arbitrage engine.

use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::time::sleep;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::{info, warn};

/// Public WebSocket endpoint for Pumpdev.
pub const PUMPDEV_WS_URL: &str = "wss://pumpdev.io/ws";

/// Incoming trade or creation event from Pumpdev WebSocket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PumpDevEvent {
    pub tx_type: Option<String>,
    pub mint: Option<String>,
    pub name: Option<String>,
    pub symbol: Option<String>,
    pub sol_amount: Option<f64>,
    pub token_amount: Option<f64>,
    pub is_buy: Option<bool>,
    pub trader_public_key: Option<String>,
    pub v_sol_in_bonding_curve: Option<f64>,
    pub v_tokens_in_bonding_curve: Option<f64>,
    pub market_cap_sol: Option<f64>,
    pub signature: Option<String>,
}

/// Pumpdev WebSocket streaming client.
pub struct PumpDevWebSocketClient {
    ws_url: String,
    event_tx: broadcast::Sender<PumpDevEvent>,
}

impl Default for PumpDevWebSocketClient {
    fn default() -> Self {
        Self::new(PUMPDEV_WS_URL)
    }
}

impl PumpDevWebSocketClient {
    /// Create a new instance with a broadcast sender buffer.
    pub fn new(url: &str) -> Self {
        let (event_tx, _) = broadcast::channel(1024);
        Self {
            ws_url: url.to_string(),
            event_tx,
        }
    }

    /// Subscribe to the broadcast channel of Pumpdev events.
    pub fn subscribe(&self) -> broadcast::Receiver<PumpDevEvent> {
        self.event_tx.subscribe()
    }

    /// Start the background streaming loop with exponential backoff and automatic reconnect.
    pub async fn start_stream(self: Arc<Self>, cancel: tokio_util::sync::CancellationToken) {
        let mut backoff_ms = 500u64;

        while !cancel.is_cancelled() {
            info!("Connecting to Pumpdev WebSocket at: {}", self.ws_url);

            match connect_async(&self.ws_url).await {
                Ok((ws_stream, response)) => {
                    info!(
                        "Connected to Pumpdev WebSocket! HTTP Status: {}",
                        response.status()
                    );
                    backoff_ms = 500; // Reset backoff upon successful handshake

                    let (mut write, mut read) = ws_stream.split();

                    // Send subscription commands for new tokens and trades
                    let sub_tokens = serde_json::json!({
                        "method": "subscribeNewToken"
                    })
                    .to_string();

                    if let Err(e) = write.send(Message::Text(sub_tokens)).await {
                        warn!("Failed to send subscribeNewToken: {}", e);
                    }

                    // Loop over incoming messages
                    loop {
                        tokio::select! {
                            _ = cancel.cancelled() => {
                                info!("Pumpdev WebSocket streamer received shutdown cancellation.");
                                let _ = write.send(Message::Close(None)).await;
                                return;
                            }
                            msg = read.next() => {
                                match msg {
                                    Some(Ok(Message::Text(text))) => {
                                        if let Ok(event) = serde_json::from_str::<PumpDevEvent>(&text) {
                                            let _ = self.event_tx.send(event);
                                        }
                                    }
                                    Some(Ok(Message::Ping(payload))) => {
                                        let _ = write.send(Message::Pong(payload)).await;
                                    }
                                    Some(Ok(Message::Close(_))) => {
                                        warn!("Pumpdev WebSocket server sent close frame.");
                                        break;
                                    }
                                    Some(Err(e)) => {
                                        warn!("Pumpdev WebSocket read error: {}", e);
                                        break;
                                    }
                                    None => {
                                        warn!("Pumpdev WebSocket stream ended.");
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to connect to Pumpdev WebSocket: {}", e);
                }
            }

            // Exponential backoff capped at 10s
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = sleep(Duration::from_millis(backoff_ms)) => {
                    backoff_ms = (backoff_ms * 2).min(10_000);
                }
            }
        }
    }
}
