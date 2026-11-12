//! High-throughput POST `/webhooks/SOLAMI` handler for Solami live streams and webhooks.

use crate::engine::TransactionJob;
use crate::solami_models::SolamiWebhookPayload;
use crate::AppState;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
};
use serde_json::json;
use std::sync::Arc;
use std::time::Instant;
use tracing::{error, trace, warn};

/// High-throughput POST handler for `/webhooks/SOLAMI` and `/webhooks/solami`.
pub async fn handle_solami_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<SolamiWebhookPayload>,
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
            .or_else(|| {
                // Also accept Solami x-api-key if matched
                headers
                    .get("x-api-key")
                    .and_then(|h| h.to_str().ok())
                    .map(|val| val == state.config.server.auth_token)
            })
            .unwrap_or(false);

        if !is_auth_valid {
            warn!(target: "security", "Unauthorized Solami webhook attempt rejected");
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "Unauthorized: Invalid or missing authorization token"})),
            );
        }
    }

    // 2. Unpack Batch or Single Solami Payload
    let events = payload.into_events();
    let total_events = events.len();
    let mut dispatched_count: usize = 0;

    for event in events {
        let tx = event.to_transaction_payload();
        let sig = tx.signature.clone();

        // 3. Deduplication Check (Local DashMap -> Redis Atomic SET NX PX)
        match state.dedup.is_duplicate_and_set(&sig).await {
            Ok(true) => {
                trace!(target: "solami_webhook", signature = %sig, "Duplicate Solami event dropped");
                continue;
            }
            Ok(false) => {
                // Event is fresh! Proceed.
            }
            Err(e) => {
                error!(target: "solami_webhook", signature = %sig, error = %e, "Deduplication check error");
            }
        }

        // 4. Resolve Modified AMM Pools
        let resolved_pools = state.router.resolve_modified_pools(&tx);

        // 5. Non-blocking Offload to Async Worker Pool
        let job = TransactionJob {
            payload: Arc::new(tx),
            resolved_pools,
            received_at: start_time,
        };

        if state.worker.dispatch(job).is_ok() {
            dispatched_count += 1;
        } else {
            warn!(
                target: "solami_webhook",
                signature = %sig,
                "Worker channel buffer saturated, event dropped"
            );
        }
    }

    let elapsed = start_time.elapsed();
    trace!(
        target: "solami_webhook",
        total = total_events,
        dispatched = dispatched_count,
        latency_micros = elapsed.as_micros(),
        "Solami webhook batch processed"
    );

    (
        StatusCode::OK,
        Json(json!({
            "status": "processed",
            "provider": "solami",
            "received": total_events,
            "dispatched": dispatched_count,
            "latency_us": elapsed.as_micros()
        })),
    )
}

/// Diagnostic GET handler for `/webhooks/SOLAMI`
pub async fn handle_solami_webhook_get() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(json!({
            "status": "active",
            "provider": "solami",
            "endpoint": "/webhooks/SOLAMI",
            "supported_events": [
                "swap",
                "liquidity",
                "token_create",
                "pool_create",
                "transfer",
                "candle",
                "stats",
                "meme",
                "graduation",
                "surge",
                "radar",
                "trending",
                "launches",
                "graduating",
                "graduated",
                "metadata"
            ]
        })),
    )
}
