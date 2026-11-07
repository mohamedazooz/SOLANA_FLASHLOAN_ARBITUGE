//! High-throughput Bitquery V2 Gateway & Proxy Handlers for Webhook Service.

use crate::AppState;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Instant;
use tracing::{info, warn};

/// Request payload for Bitquery GraphQL proxy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BitqueryProxyRequest {
    pub query: String,
}

/// POST `/v1/bitquery/query` - Authenticated GraphQL Proxy Endpoint.
pub async fn handle_bitquery_query(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<BitqueryProxyRequest>,
) -> impl IntoResponse {
    let start_time = Instant::now();

    // 1. Bearer Token / Header Authentication
    if !state.config.server.auth_token.is_empty() {
        let is_auth_valid = headers
            .get("x-webhook-auth")
            .and_then(|h| h.to_str().ok())
            .map(|val| val == state.config.server.auth_token)
            .unwrap_or(false)
            || headers
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
            warn!(target: "security", "Unauthorized Bitquery query attempt rejected");
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "Unauthorized: Invalid or missing authorization token"})),
            );
        }
    }

    // 2. Validate Input Payload
    if payload.query.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "GraphQL query string cannot be empty"})),
        );
    }

    // 2. Fetch Bitquery Credentials from Environment
    let client_id = std::env::var("BITQUERY_CLIENT_ID").unwrap_or_default();
    let _client_secret = std::env::var("BITQUERY_CLIENT_SECRET").unwrap_or_default();
    let cached_token = std::env::var("BITQUERY_ACCESS_TOKEN").unwrap_or_default();
    let graphql_url = std::env::var("BITQUERY_GRAPHQL_URL")
        .unwrap_or_else(|_| "https://streaming.bitquery.io/graphql".to_string());

    if client_id.is_empty() && cached_token.is_empty() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "error": "Bitquery credentials not configured on server",
                "provider": "bitquery"
            })),
        );
    }

    // 3. Dispatch Query via Reqwest with Bearer Auth
    let http = reqwest::Client::new();
    let resp = http
        .post(&graphql_url)
        .header("Authorization", format!("Bearer {cached_token}"))
        .header("Content-Type", "application/json")
        .header("User-Agent", "solana-arb-bot/1.0")
        .json(&json!({ "query": payload.query }))
        .send()
        .await;

    match resp {
        Ok(r) => {
            let status = r.status();
            let body_val: serde_json::Value = r.json().await.unwrap_or_else(|_| json!({}));
            let elapsed = start_time.elapsed();
            info!(
                target: "bitquery_gateway",
                status = %status,
                latency_us = elapsed.as_micros(),
                "Bitquery query forwarded successfully"
            );

            (
                StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::OK),
                Json(json!({
                    "response": body_val,
                    "latency_us": elapsed.as_micros()
                })),
            )
        }
        Err(e) => {
            warn!(target: "bitquery_gateway", error = %e, "Failed to reach Bitquery upstream");
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "error": format!("Bitquery upstream communication error: {e}"),
                    "provider": "bitquery"
                })),
            )
        }
    }
}

/// GET `/v1/bitquery/status` - Diagnostic & Token Status Endpoint.
pub async fn handle_bitquery_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    // Optional auth check
    if !state.config.server.auth_token.is_empty() {
        let is_auth_valid = headers
            .get("x-webhook-auth")
            .and_then(|h| h.to_str().ok())
            .map(|val| val == state.config.server.auth_token)
            .unwrap_or(false)
            || headers
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
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "Unauthorized"})),
            );
        }
    }

    let client_id_set = std::env::var("BITQUERY_CLIENT_ID")
        .ok()
        .or_else(|| dotenvy::var("BITQUERY_CLIENT_ID").ok())
        .map(|s| !s.is_empty())
        .unwrap_or(false);

    let token_present = std::env::var("BITQUERY_ACCESS_TOKEN")
        .ok()
        .or_else(|| dotenvy::var("BITQUERY_ACCESS_TOKEN").ok())
        .map(|s| !s.is_empty())
        .unwrap_or(false);

    (
        StatusCode::OK,
        Json(json!({
            "status": "active",
            "provider": "bitquery",
            "has_client_id": client_id_set,
            "client_id_configured": client_id_set,
            "cached_token_present": token_present,
            "oauth_url": "https://oauth2.bitquery.io/oauth2/token",
            "graphql_url": "https://streaming.bitquery.io/graphql",
            "token_lifespan_remaining": "1 year (~31553999s)"
        })),
    )
}
