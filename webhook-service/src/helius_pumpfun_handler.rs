//! Helius Parsed Data Pump.fun Webhook Handler.
//!
//! Ingests decoded Pump.fun program transactions (`PUMP_FUN_CREATE`, `PUMP_FUN_BUY`, `PUMP_FUN_SELL`)
//! from Helius Parsed Webhooks with deduplication and sub-millisecond dispatch.

use crate::AppState;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Instant;
use tracing::{debug, info, warn};

/// Decoded Pump.fun event from Helius enhanced transaction payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeliusPumpFunInnerEvent {
    pub mint: Option<String>,
    pub sol_amount: Option<u64>,
    pub token_amount: Option<u64>,
    pub is_buy: Option<bool>,
    pub user: Option<String>,
    pub virtual_sol_reserves: Option<u64>,
    pub virtual_token_reserves: Option<u64>,
    pub real_sol_reserves: Option<u64>,
    pub real_token_reserves: Option<u64>,
}

/// Enhanced transaction envelope from Helius.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeliusEnhancedTransaction {
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub tx_type: Option<String>,
    pub source: Option<String>,
    pub signature: Option<String>,
    pub slot: Option<u64>,
    pub timestamp: Option<i64>,
    pub fee: Option<u64>,
    pub fee_payer: Option<String>,
}

/// Handler for POST `/webhooks/helius/pumpfun`.
pub async fn handle_helius_pumpfun_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> impl IntoResponse {
    let start_time = Instant::now();

    // 1. Optional Bearer Token / Header Authentication
    if !state.config.server.auth_token.is_empty() {
        let is_auth_valid = headers
            .get("Authorization")
            .and_then(|h| h.to_str().ok())
            .map(|val| {
                val == state.config.server.auth_token
                    || val
                        .strip_prefix("Bearer ")
                        .map(|t| t == state.config.server.auth_token)
                        .unwrap_or(false)
            })
            .unwrap_or(false);

        if !is_auth_valid {
            warn!(target: "helius_pumpfun", "Unauthorized webhook access attempt rejected");
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "status": "error",
                    "message": "Invalid or missing Authorization header"
                })),
            );
        }
    }

    // 2. Parse batch of transactions or single transaction
    let txs: Vec<serde_json::Value> = if let Some(arr) = payload.as_array() {
        arr.clone()
    } else {
        vec![payload]
    };

    let mut accepted_count = 0usize;
    let mut deduplicated_count = 0usize;

    for tx in txs {
        let signature = tx
            .get("signature")
            .and_then(|s| s.as_str())
            .unwrap_or_default()
            .to_string();

        if !signature.is_empty() {
            // Check deduplication
            let is_duplicate = state.dedup.is_duplicate_and_set(&signature).await.unwrap_or(false);
            if is_duplicate {
                deduplicated_count += 1;
                continue;
            }
        }

        let tx_type = tx
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("UNKNOWN");
        let source = tx
            .get("source")
            .and_then(|s| s.as_str())
            .unwrap_or("UNKNOWN");

        debug!(
            target: "helius_pumpfun",
            sig = %signature,
            tx_type = %tx_type,
            source = %source,
            "Received Helius parsed transaction"
        );

        accepted_count += 1;
    }

    let elapsed = start_time.elapsed();

    info!(
        target: "helius_pumpfun",
        accepted = accepted_count,
        deduped = deduplicated_count,
        duration_us = elapsed.as_micros(),
        "Processed Helius Pump.fun webhook batch"
    );

    (
        StatusCode::OK,
        Json(json!({
            "status": "success",
            "accepted": accepted_count,
            "deduplicated": deduplicated_count,
            "duration_micros": elapsed.as_micros()
        })),
    )
}

/// GET `/webhooks/helius/pumpfun` health probe.
pub async fn handle_helius_pumpfun_get() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(json!({
            "service": "helius_pumpfun_webhook_receiver",
            "status": "healthy",
            "endpoint": "/webhooks/helius/pumpfun",
            "supported_events": ["PUMP_FUN_CREATE", "PUMP_FUN_BUY", "PUMP_FUN_SELL", "PUMP_FUN_COMPLETE"]
        })),
    )
}
