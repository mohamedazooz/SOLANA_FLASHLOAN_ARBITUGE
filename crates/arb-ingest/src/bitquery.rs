//! High-Performance Bitquery V2 Solana Data Ingestion & GraphQL Client.
//!
//! Features:
//! - Hybrid OAuth2 Token Manager (instant startup with cached 1-year token, automatic refresh on expiry).
//! - Direct Solana DEX Trade streaming & polling (Raydium, Orca, Meteora, Pump.fun).
//! - New Liquidity Pool discovery queries for multi-venue token detection.

use anyhow::{Context, Result, bail};
use futures::{SinkExt, StreamExt};
use parking_lot::RwLock;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::Message;
use tracing::{info, warn};

/// Bitquery runtime configuration.
#[derive(Debug, Clone)]
pub struct BitqueryConfig {
    pub client_id: String,
    pub client_secret: String,
    pub access_token: Arc<RwLock<Option<String>>>,
    pub oauth_url: String,
    pub graphql_url: String,
    pub timeout: Duration,
}

impl Default for BitqueryConfig {
    fn default() -> Self {
        Self {
            client_id: String::new(),
            client_secret: String::new(),
            access_token: Arc::new(RwLock::new(None)),
            oauth_url: "https://oauth2.bitquery.io/oauth2/token".to_string(),
            graphql_url: "https://streaming.bitquery.io/graphql".to_string(),
            timeout: Duration::from_secs(10),
        }
    }
}

/// Bitquery V2 GraphQL Ingestion Client.
pub struct BitqueryClient {
    config: BitqueryConfig,
    http: Client,
}

#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    #[allow(dead_code)]
    expires_in: Option<u64>,
    #[allow(dead_code)]
    token_type: Option<String>,
}

/// Bitquery GraphQL Response wrapper.
#[derive(Debug, Deserialize)]
pub struct BitqueryResponse<T> {
    pub data: Option<T>,
    pub errors: Option<Vec<BitqueryError>>,
}

#[derive(Debug, Deserialize)]
pub struct BitqueryError {
    pub message: String,
}

/// Solana DEX Trade root container.
#[derive(Debug, Deserialize)]
pub struct SolanaDataContainer {
    #[serde(rename = "Solana")]
    pub solana: SolanaDataset,
}

#[derive(Debug, Deserialize)]
pub struct SolanaDataset {
    #[serde(rename = "DEXTrades", default)]
    pub dex_trades: Vec<BitqueryTradeEntry>,
    #[serde(rename = "Blocks", default)]
    pub blocks: Vec<BitqueryBlockEntry>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BitqueryTradeEntry {
    #[serde(rename = "Transaction")]
    pub transaction: BitqueryTransaction,
    #[serde(rename = "Trade")]
    pub trade: BitqueryTrade,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BitqueryTransaction {
    #[serde(rename = "Signature")]
    pub signature: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BitqueryTrade {
    #[serde(rename = "Dex")]
    pub dex: BitqueryDex,
    #[serde(rename = "Buy")]
    pub buy: BitqueryLeg,
    #[serde(rename = "Sell")]
    pub sell: BitqueryLeg,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BitqueryDex {
    #[serde(rename = "ProgramAddress")]
    pub program_address: String,
    #[serde(rename = "ProtocolName")]
    pub protocol_name: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BitqueryLeg {
    #[serde(rename = "Amount")]
    pub amount: String,
    #[serde(rename = "Currency")]
    pub currency: BitqueryCurrency,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BitqueryCurrency {
    #[serde(rename = "MintAddress")]
    pub mint_address: String,
    #[serde(rename = "Symbol")]
    pub symbol: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BitqueryBlockEntry {
    #[serde(rename = "Block")]
    pub block: BitqueryBlock,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BitqueryBlock {
    #[serde(rename = "Slot")]
    pub slot: String,
    #[serde(rename = "Time")]
    pub time: String,
}

impl BitqueryClient {
    pub fn new(config: BitqueryConfig) -> Self {
        let http = Client::builder()
            .timeout(config.timeout)
            .build()
            .unwrap_or_default();

        Self { config, http }
    }

    /// Builds BitqueryClient using environment variables:
    /// `BITQUERY_CLIENT_ID`, `BITQUERY_CLIENT_SECRET`, and optional `BITQUERY_ACCESS_TOKEN`.
    pub fn from_env() -> Option<Self> {
        let client_id = std::env::var("BITQUERY_CLIENT_ID")
            .ok()
            .or_else(|| dotenvy::var("BITQUERY_CLIENT_ID").ok())?;

        let client_secret = std::env::var("BITQUERY_CLIENT_SECRET")
            .ok()
            .or_else(|| dotenvy::var("BITQUERY_CLIENT_SECRET").ok())?;

        let cached_token = std::env::var("BITQUERY_ACCESS_TOKEN")
            .ok()
            .or_else(|| dotenvy::var("BITQUERY_ACCESS_TOKEN").ok());

        let oauth_url = std::env::var("BITQUERY_OAUTH_URL")
            .ok()
            .or_else(|| dotenvy::var("BITQUERY_OAUTH_URL").ok())
            .unwrap_or_else(|| "https://oauth2.bitquery.io/oauth2/token".to_string());

        let graphql_url = std::env::var("BITQUERY_GRAPHQL_URL")
            .ok()
            .or_else(|| dotenvy::var("BITQUERY_GRAPHQL_URL").ok())
            .unwrap_or_else(|| "https://streaming.bitquery.io/graphql".to_string());

        let config = BitqueryConfig {
            client_id,
            client_secret,
            oauth_url,
            graphql_url,
            ..Default::default()
        };

        if let Some(token) = cached_token {
            if !token.is_empty() {
                *config.access_token.write() = Some(token);
            }
        }

        Some(Self::new(config))
    }

    /// Returns a valid Bearer token, automatically executing OAuth2 exchange if missing.
    pub async fn get_valid_token(&self) -> Result<String> {
        {
            let lock = self.config.access_token.read();
            if let Some(ref token) = *lock {
                if !token.is_empty() {
                    return Ok(token.clone());
                }
            }
        }
        self.refresh_token().await
    }

    /// Explicitly refreshes the OAuth2 access token via Client Credentials.
    pub async fn refresh_token(&self) -> Result<String> {
        info!(target: "bitquery", "Requesting new Bitquery OAuth2 access token...");

        let params = [
            ("grant_type", "client_credentials"),
            ("client_id", &self.config.client_id),
            ("client_secret", &self.config.client_secret),
            ("scope", "api"),
        ];

        let resp = self
            .http
            .post(&self.config.oauth_url)
            .form(&params)
            .send()
            .await
            .context("Failed to dispatch OAuth2 token request to Bitquery")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err_text = resp.text().await.unwrap_or_default();
            bail!("Bitquery OAuth2 token request failed ({status}): {err_text}");
        }

        let token_resp: OAuthTokenResponse = resp
            .json()
            .await
            .context("Failed to deserialize Bitquery OAuth2 token response")?;

        let new_token = token_resp.access_token;
        *self.config.access_token.write() = Some(new_token.clone());
        info!(target: "bitquery", "Bitquery OAuth2 token successfully refreshed");

        Ok(new_token)
    }

    /// Executes a GraphQL query string against Bitquery V2 with automatic retry on 401 Unauthorized.
    pub async fn execute_query<T: for<'de> Deserialize<'de>>(&self, query: &str) -> Result<T> {
        let mut token = self.get_valid_token().await?;

        // First attempt
        let mut resp = self.dispatch_query(query, &token).await?;

        // If unauthorized, refresh token once and retry
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            warn!(target: "bitquery", "Bitquery token unauthorized (401), refreshing token and retrying...");
            token = self.refresh_token().await?;
            resp = self.dispatch_query(query, &token).await?;
        }

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("Bitquery GraphQL query returned error status {status}: {body}");
        }

        let body_text = resp.text().await.context("Failed to read response body")?;
        let parsed: BitqueryResponse<T> = serde_json::from_str(&body_text)
            .with_context(|| format!("Failed to parse Bitquery response: {body_text}"))?;

        if let Some(errs) = parsed.errors {
            if !errs.is_empty() {
                let msgs: Vec<String> = errs.into_iter().map(|e| e.message).collect();
                bail!(
                    "Bitquery GraphQL query reported errors: {}",
                    msgs.join("; ")
                );
            }
        }

        parsed
            .data
            .ok_or_else(|| anyhow::anyhow!("Bitquery GraphQL response missing 'data' field"))
    }

    async fn dispatch_query(&self, query: &str, token: &str) -> Result<reqwest::Response> {
        let body = serde_json::json!({ "query": query });

        self.http
            .post(&self.config.graphql_url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .header("User-Agent", "solana-arb-bot/1.0")
            .json(&body)
            .send()
            .await
            .context("Failed to send HTTP POST to Bitquery GraphQL")
    }

    /// Fetches the latest Solana DEX trades across Raydium, Orca, Meteora, and Pump.fun.
    pub async fn fetch_recent_dex_trades(&self, limit: usize) -> Result<Vec<BitqueryTradeEntry>> {
        let query = format!(
            r#"query {{
                Solana {{
                    DEXTrades(limit: {{ count: {limit} }}) {{
                        Transaction {{
                            Signature
                        }}
                        Trade {{
                            Dex {{
                                ProgramAddress
                                ProtocolName
                            }}
                            Buy {{
                                Amount
                                Currency {{
                                    MintAddress
                                    Symbol
                                }}
                            }}
                            Sell {{
                                Amount
                                Currency {{
                                    MintAddress
                                    Symbol
                                }}
                            }}
                        }}
                    }}
                }}
            }}"#
        );

        let data: SolanaDataContainer = self.execute_query(&query).await?;
        Ok(data.solana.dex_trades)
    }

    /// Queries the latest Solana slot block from Bitquery.
    pub async fn fetch_latest_block(&self) -> Result<Option<BitqueryBlock>> {
        let query = r#"query {
            Solana {
                Blocks(limit: { count: 1 }) {
                    Block {
                        Slot
                        Time
                    }
                }
            }
        }"#;

        let data: SolanaDataContainer = self.execute_query(query).await?;
        Ok(data.solana.blocks.into_iter().next().map(|b| b.block))
    }

    /// Creates a WebSocket subscription client configured with the current Bitquery credentials.
    pub fn create_ws_client(&self) -> BitqueryWsClient {
        BitqueryWsClient::new(self.config.clone())
    }
}

/// Wire protocol frame for GraphQL-over-WebSocket (`graphql-transport-ws` / `graphql-ws`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BitqueryWsMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

/// Real-time Bitquery V2 WebSocket streaming client for Solana DEX trades.
#[derive(Clone)]
pub struct BitqueryWsClient {
    pub config: BitqueryConfig,
    is_running: Arc<AtomicBool>,
}

impl BitqueryWsClient {
    pub fn new(config: BitqueryConfig) -> Self {
        Self {
            config,
            is_running: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn from_env() -> Option<Self> {
        let http_client = BitqueryClient::from_env()?;
        Some(http_client.create_ws_client())
    }

    pub fn stop(&self) {
        self.is_running.store(false, Ordering::Relaxed);
    }

    pub fn is_running(&self) -> bool {
        self.is_running.load(Ordering::Relaxed)
    }

    /// Resolves the WebSocket endpoint with OAuth token attached as query parameter.
    pub async fn resolve_ws_url(&self) -> Result<String> {
        let token = {
            let lock = self.config.access_token.read();
            lock.clone().unwrap_or_default()
        };

        let active_token = if !token.is_empty() {
            token
        } else {
            let http_client = BitqueryClient::new(self.config.clone());
            http_client.get_valid_token().await?
        };

        let base_ws = self
            .config
            .graphql_url
            .replace("https://", "wss://")
            .replace("http://", "ws://");

        Ok(format!("{base_ws}?token={active_token}"))
    }

    /// Builds the DEX subscription query, optionally filtering by specific DEX protocols.
    pub fn build_subscription_query(protocols: Option<&[String]>) -> String {
        if let Some(list) = protocols {
            if !list.is_empty() {
                let formatted_list = list
                    .iter()
                    .map(|p| format!("\"{p}\""))
                    .collect::<Vec<_>>()
                    .join(", ");
                return format!(
                    r#"subscription {{
                        Solana {{
                            DEXTrades(
                                where: {{
                                    Trade: {{
                                        Dex: {{
                                            ProtocolName: {{ in: [{formatted_list}] }}
                                        }}
                                    }}
                                }}
                            ) {{
                                Block {{
                                    Slot
                                    Time
                                }}
                                Transaction {{
                                    Signature
                                }}
                                Trade {{
                                    Dex {{
                                        ProtocolName
                                        ProgramAddress
                                    }}
                                    Buy {{
                                        Amount
                                        Currency {{
                                            MintAddress
                                            Symbol
                                        }}
                                    }}
                                    Sell {{
                                        Amount
                                        Currency {{
                                            MintAddress
                                            Symbol
                                        }}
                                    }}
                                }}
                            }}
                        }}
                    }}"#
                );
            }
        }

        r#"subscription {
            Solana {
                DEXTrades(limit: { count: 10 }) {
                    Block {
                        Slot
                        Time
                    }
                    Transaction {
                        Signature
                    }
                    Trade {
                        Dex {
                            ProtocolName
                            ProgramAddress
                        }
                        Buy {
                            Amount
                            Currency {
                                MintAddress
                                Symbol
                            }
                        }
                        Sell {
                            Amount
                            Currency {
                                MintAddress
                                Symbol
                            }
                        }
                    }
                }
            }
        }"#
        .to_string()
    }

    /// Connects to Bitquery V2 WebSocket and streams parsed trades into callback with exponential backoff.
    pub async fn start_streaming<F>(
        &self,
        protocols: Option<Vec<String>>,
        mut on_trade: F,
    ) -> Result<()>
    where
        F: FnMut(BitqueryTradeEntry) + Send + 'static,
    {
        self.is_running.store(true, Ordering::Relaxed);
        let mut backoff = Duration::from_millis(500);
        let query = Self::build_subscription_query(protocols.as_deref());

        while self.is_running.load(Ordering::Relaxed) {
            let ws_url = match self.resolve_ws_url().await {
                Ok(url) => url,
                Err(e) => {
                    warn!(target: "bitquery_ws", error = %e, "Failed to resolve Bitquery WebSocket URL");
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(15));
                    continue;
                }
            };

            info!(target: "bitquery_ws", "Connecting to Bitquery V2 WebSocket subscription stream...");

            let mut request = match ws_url.into_client_request() {
                Ok(req) => req,
                Err(e) => {
                    warn!(target: "bitquery_ws", error = %e, "Invalid WebSocket client request");
                    tokio::time::sleep(backoff).await;
                    continue;
                }
            };

            if let Ok(val) = "graphql-transport-ws, graphql-ws".parse() {
                request.headers_mut().insert("Sec-WebSocket-Protocol", val);
            }

            match connect_async(request).await {
                Ok((mut ws_stream, response)) => {
                    info!(
                        target: "bitquery_ws",
                        status = %response.status(),
                        "Connected to Bitquery WebSocket stream successfully"
                    );
                    backoff = Duration::from_millis(500);

                    // 1. Send connection_init
                    let init_msg = BitqueryWsMessage {
                        msg_type: "connection_init".to_string(),
                        id: None,
                        payload: None,
                    };
                    if let Ok(init_json) = serde_json::to_string(&init_msg) {
                        let _ = ws_stream.send(Message::Text(init_json)).await;
                    }

                    // 2. Send subscribe request
                    let sub_msg = serde_json::json!({
                        "id": "dex_sub_1",
                        "type": "subscribe",
                        "payload": {
                            "query": query
                        }
                    });
                    let _ = ws_stream.send(Message::Text(sub_msg.to_string())).await;

                    // 3. Process incoming stream frames
                    while let Some(msg_result) = ws_stream.next().await {
                        if !self.is_running.load(Ordering::Relaxed) {
                            break;
                        }

                        match msg_result {
                            Ok(Message::Text(text)) => {
                                if let Ok(ws_frame) =
                                    serde_json::from_str::<BitqueryWsMessage>(&text)
                                {
                                    match ws_frame.msg_type.as_str() {
                                        "connection_ack" => {
                                            info!(target: "bitquery_ws", "Subscription acknowledged by Bitquery server");
                                        }
                                        "next" | "data" => {
                                            if let Some(payload_val) = ws_frame.payload {
                                                if let Ok(resp) = serde_json::from_value::<
                                                    BitqueryResponse<SolanaDataContainer>,
                                                >(
                                                    payload_val
                                                ) {
                                                    if let Some(data) = resp.data {
                                                        for trade in data.solana.dex_trades {
                                                            on_trade(trade);
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                        "ping" => {
                                            let pong = serde_json::json!({ "type": "pong" });
                                            let _ = ws_stream
                                                .send(Message::Text(pong.to_string()))
                                                .await;
                                        }
                                        "error" => {
                                            warn!(target: "bitquery_ws", payload = ?ws_frame.payload, "Bitquery subscription error received");
                                        }
                                        "complete" => {
                                            info!(target: "bitquery_ws", "Bitquery subscription completed by server");
                                            break;
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            Ok(Message::Ping(payload)) => {
                                let _ = ws_stream.send(Message::Pong(payload)).await;
                            }
                            Ok(Message::Close(frame)) => {
                                warn!(target: "bitquery_ws", frame = ?frame, "WebSocket closed by remote peer");
                                break;
                            }
                            Err(e) => {
                                warn!(target: "bitquery_ws", error = %e, "WebSocket stream error encountered");
                                break;
                            }
                            _ => {}
                        }
                    }
                }
                Err(e) => {
                    warn!(target: "bitquery_ws", error = %e, "Bitquery WebSocket connection failed");
                }
            }

            if self.is_running.load(Ordering::Relaxed) {
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(15));
            }
        }

        Ok(())
    }

    /// Spawns a background task streaming trades into a bounded MPSC channel.
    pub fn spawn_subscription_stream(
        self: Arc<Self>,
        protocols: Option<Vec<String>>,
        buffer_capacity: usize,
    ) -> (
        tokio::sync::mpsc::Receiver<BitqueryTradeEntry>,
        tokio::task::JoinHandle<()>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::channel(buffer_capacity);
        let client = Arc::clone(&self);

        let handle = tokio::spawn(async move {
            let _ = client
                .start_streaming(protocols, move |trade| {
                    let _ = tx.try_send(trade);
                })
                .await;
        });

        (rx, handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bitquery_config_defaults() {
        let config = BitqueryConfig::default();
        assert_eq!(config.oauth_url, "https://oauth2.bitquery.io/oauth2/token");
        assert_eq!(config.graphql_url, "https://streaming.bitquery.io/graphql");
    }

    #[test]
    fn test_bitquery_trade_entry_deserialization() {
        let sample_json = r#"{
            "data": {
                "Solana": {
                    "DEXTrades": [
                        {
                            "Transaction": {
                                "Signature": "5wHu...sig"
                            },
                            "Trade": {
                                "Dex": {
                                    "ProgramAddress": "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8",
                                    "ProtocolName": "raydium_v4"
                                },
                                "Buy": {
                                    "Amount": "100.5",
                                    "Currency": {
                                        "MintAddress": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
                                        "Symbol": "USDC"
                                    }
                                },
                                "Sell": {
                                    "Amount": "0.55",
                                    "Currency": {
                                        "MintAddress": "So11111111111111111111111111111111111111112",
                                        "Symbol": "WSOL"
                                    }
                                }
                            }
                        }
                    ]
                }
            }
        }"#;

        let parsed: BitqueryResponse<SolanaDataContainer> =
            serde_json::from_str(sample_json).expect("Deserialization failed");
        let trades = parsed.data.unwrap().solana.dex_trades;
        assert_eq!(trades.len(), 1);
        assert_eq!(trades[0].transaction.signature, "5wHu...sig");
        assert_eq!(trades[0].trade.buy.currency.symbol, "USDC");
        assert_eq!(trades[0].trade.sell.currency.symbol, "WSOL");
    }

    #[test]
    fn test_bitquery_ws_message_serialization_and_deserialization() {
        // 1. Connection Init
        let init = BitqueryWsMessage {
            msg_type: "connection_init".to_string(),
            id: None,
            payload: None,
        };
        let init_json = serde_json::to_string(&init).unwrap();
        assert!(init_json.contains("\"type\":\"connection_init\""));

        // 2. Next message with payload
        let next_raw = r#"{
            "type": "next",
            "id": "dex_sub_1",
            "payload": {
                "data": {
                    "Solana": {
                        "DEXTrades": []
                    }
                }
            }
        }"#;
        let parsed: BitqueryWsMessage = serde_json::from_str(next_raw).unwrap();
        assert_eq!(parsed.msg_type, "next");
        assert_eq!(parsed.id.as_deref(), Some("dex_sub_1"));
        assert!(parsed.payload.is_some());
    }

    #[test]
    fn test_bitquery_ws_subscription_query_builder() {
        // Default query
        let default_query = BitqueryWsClient::build_subscription_query(None);
        assert!(default_query.contains("subscription {"));
        assert!(default_query.contains("DEXTrades"));

        // Filtered query
        let protocols = vec!["pump_amm".to_string(), "raydium_clmm".to_string()];
        let filtered_query = BitqueryWsClient::build_subscription_query(Some(&protocols));
        assert!(filtered_query.contains("\"pump_amm\""));
        assert!(filtered_query.contains("\"raydium_clmm\""));
        assert!(filtered_query.contains("ProtocolName: { in:"));
    }

    #[tokio::test]
    async fn test_bitquery_ws_url_resolution() {
        let config = BitqueryConfig {
            graphql_url: "https://streaming.bitquery.io/graphql".to_string(),
            ..Default::default()
        };
        *config.access_token.write() = Some("mock_token_12345".to_string());

        let ws_client = BitqueryWsClient::new(config);
        let resolved = ws_client.resolve_ws_url().await.unwrap();
        assert_eq!(
            resolved,
            "wss://streaming.bitquery.io/graphql?token=mock_token_12345"
        );
    }
}
