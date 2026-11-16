//! Sentinel Solana Webhook Service Library Core.

pub mod bitquery_handler;
pub mod config;
pub mod dedup;
pub mod engine;
pub mod helius_pumpfun_handler;
pub mod models;
pub mod rate_limit;
pub mod router;
pub mod secrets;
pub mod solami_handler;
pub mod solami_models;

pub use bitquery_handler::{handle_bitquery_query, handle_bitquery_status};
pub use helius_pumpfun_handler::{handle_helius_pumpfun_get, handle_helius_pumpfun_webhook};
pub use solami_handler::{handle_solami_webhook, handle_solami_webhook_get};
pub use solami_models::{SolamiEvent, SolamiWebhookPayload};

use crate::config::AppConfig;
use crate::dedup::DeduplicationEngine;
use crate::engine::{ExpressionEvaluator, TransactionJob, WorkerEngine};
use crate::models::WebhookPayload;
use crate::router::AmmRouter;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, State,
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
};
use hmac::{Hmac, KeyInit, Mac};
use serde_json::json;
use sha2::Sha256;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tracing::{error, trace, warn};

type HmacSha256 = Hmac<Sha256>;

/// Shared Application State passed into Axum routes.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub dedup: DeduplicationEngine,
    pub router: AmmRouter,
    pub evaluator: ExpressionEvaluator,
    pub worker: Arc<WorkerEngine>,
    pub rate_limiter: Arc<rate_limit::RateLimiter>,
}

/// Constant-time byte equality. Per the OWASP Webhook Security Cheat Sheet,
/// authentication tokens must never be compared with plain string equality,
/// which leaks timing information byte by byte.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Verify a raw header value against the configured token in constant time.
pub fn verify_webhook_token(provided: Option<&str>, expected: &str) -> bool {
    provided.is_some_and(|p| constant_time_eq(p.as_bytes(), expected.as_bytes()))
}

/// Verify a header value that may carry the token raw or as `Bearer <token>`.
fn bearer_or_raw_token_valid(val: &str, expected: &str) -> bool {
    verify_webhook_token(Some(val), expected)
        || val
            .strip_prefix("Bearer ")
            .is_some_and(|t| verify_webhook_token(Some(t), expected))
}

/// Verify an HMAC-SHA256 request signature over the raw body bytes.
///
/// Accepts lowercase/uppercase hex and an optional `sha256=` prefix
/// (GitHub / Standard-Webhooks convention). Comparison is constant-time.
/// Returns `false` when the header is missing, malformed, or mismatched.
pub fn verify_hmac_signature(secret: &str, raw_body: &[u8], provided: Option<&str>) -> bool {
    let Some(provided) = provided else {
        return false;
    };
    let provided = provided.trim();
    let provided = provided.strip_prefix("sha256=").unwrap_or(provided);
    let Ok(provided_bytes) = hex::decode(provided) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(raw_body);
    let tag = mac.finalize().into_bytes();
    constant_time_eq(&tag[..], &provided_bytes)
}

/// Replay-protection guard based on the event timestamp.
///
/// `max_age_secs == 0` disables the guard (returns `true` unconditionally).
/// When enabled: events older than `max_age_secs` are rejected, timestamps
/// more than 5 minutes in the future are rejected (clock skew beyond that is
/// a replay/forgery signal), and events with no timestamp are rejected
/// (fail-closed) because their age cannot be established.
pub fn event_age_allowed(max_age_secs: u64, event_timestamp: Option<i64>, now_secs: i64) -> bool {
    if max_age_secs == 0 {
        return true;
    }
    match event_timestamp {
        None => false,
        Some(ts) => {
            const FUTURE_SKEW_ALLOWANCE_SECS: i64 = 300;
            now_secs - ts <= max_age_secs as i64 && ts - now_secs <= FUTURE_SKEW_ALLOWANCE_SECS
        }
    }
}

/// Extract the client identity used for rate limiting: the direct peer address
/// when available, else the first `X-Forwarded-For` hop (reverse-proxy
/// deployments), else a shared global bucket.
fn client_key(req: &axum::extract::Request) -> String {
    if let Some(ConnectInfo(addr)) = req.extensions().get::<ConnectInfo<SocketAddr>>() {
        return addr.ip().to_string();
    }
    if let Some(xff) = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
    {
        if let Some(first) = xff.split(',').next() {
            let first = first.trim();
            if !first.is_empty() {
                return first.to_string();
            }
        }
    }
    "global".to_string()
}

/// Axum middleware enforcing the configured per-client rate limit.
pub async fn rate_limit_middleware(
    State(state): State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    if !state.rate_limiter.is_enabled() {
        return next.run(req).await;
    }
    let key = client_key(&req);
    if !state.rate_limiter.check(&key) {
        warn!(target: "security", client = %key, "Rate limit exceeded, rejecting request");
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(json!({"error": "rate limit exceeded"})),
        )
            .into_response();
    }
    next.run(req).await
}

/// High-throughput POST `/v1/webhook` handler.
pub async fn handle_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<WebhookPayload>,
) -> impl IntoResponse {
    let start_time = Instant::now();

    // 1. Optional Bearer Token / Header Authentication
    if !state.config.server.auth_token.is_empty() {
        let is_auth_valid = headers
            .get("Authorization")
            .and_then(|h| h.to_str().ok())
            .map(|val| bearer_or_raw_token_valid(val, &state.config.server.auth_token))
            .unwrap_or(false);

        if !is_auth_valid {
            warn!(target: "security", "Unauthorized webhook attempt rejected");
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "Unauthorized"})),
            );
        }
    }

    // 2. Unpack Batch or Single Payload
    let transactions = payload.into_transactions();
    let mut dispatched_count: usize = 0;

    for tx in transactions {
        let sig = tx.signature.clone();

        // 3. Deduplication Check (Local DashMap -> Redis Atomic SET NX PX)
        match state.dedup.is_duplicate_and_set(&sig).await {
            Ok(true) => {
                trace!(target: "webhook", signature = %sig, "Duplicate transaction signature dropped");
                continue;
            }
            Ok(false) => {
                // Signature is fresh! Proceed.
            }
            Err(e) => {
                error!(target: "webhook", signature = %sig, error = %e, "Deduplication check error");
            }
        }

        // 4. Expression Webhook: Filter Failed or Irrelevant Transactions
        if !state.evaluator.should_process(&tx) {
            continue;
        }

        // 5. AMM & Account Redundancy Resolution (O(1))
        let resolved_pools = state.router.resolve_modified_pools(&tx);

        // 6. Non-blocking Offload to Async Worker Pool
        let job = TransactionJob {
            payload: Arc::new(tx),
            resolved_pools,
            received_at: start_time,
        };

        match state.worker.dispatch(job) {
            Ok(_) => {
                dispatched_count += 1;
            }
            Err(e) => {
                warn!(target: "webhook", signature = %sig, "Worker channel buffer full: {:?}", e);
            }
        }
    }

    let elapsed = start_time.elapsed();
    trace!(
        target: "webhook",
        latency_us = elapsed.as_micros(),
        dispatched = dispatched_count,
        "Webhook request completed"
    );

    (
        StatusCode::OK,
        Json(json!({
            "status": "ok",
            "dispatched": dispatched_count,
            "latency_us": elapsed.as_micros()
        })),
    )
}

/// Health check endpoint for uptime monitors and load balancers.
pub async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({"status": "healthy"})))
}

/// Returns the current MarketGraph topology representation (Nodes and Edges).
pub async fn topology_snapshot() -> impl IntoResponse {
    let sample_topology = json!({
        "nodes": [
            {"id": "WSOL", "label": "Wrapped SOL", "mint": "So11111111111111111111111111111111111111112", "type": "base"},
            {"id": "USDC", "label": "USD Coin", "mint": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v", "type": "quote"},
            {"id": "TRUMP", "label": "TRUMP (pump)", "mint": "6p6xgHyF7AeQHydaKVtRNVUjCAWoTeZwXHnkTXQKpump", "type": "meme"},
            {"id": "FARTCOIN", "label": "FARTCOIN (pump)", "mint": "9BB6NFEcjBCtnNLFko2FqVQBq8HHM13kCyYcdQbgpump", "type": "meme"}
        ],
        "edges": [
            {"source": "WSOL", "target": "TRUMP", "protocol": "PumpFun", "kind": "bonding_curve", "fee_bps": 100},
            {"source": "TRUMP", "target": "WSOL", "protocol": "RaydiumV4", "kind": "cpmm", "fee_bps": 25},
            {"source": "WSOL", "target": "FARTCOIN", "protocol": "PumpFun", "kind": "bonding_curve", "fee_bps": 100},
            {"source": "FARTCOIN", "target": "WSOL", "protocol": "MeteoraDlmm", "kind": "dlmm", "fee_bps": 15},
            {"source": "WSOL", "target": "USDC", "protocol": "OrcaWhirlpool", "kind": "clmm", "fee_bps": 4}
        ],
        "active_arbitrage_cycles": [
            {
                "path": ["WSOL", "TRUMP", "WSOL"],
                "protocols": ["PumpFun", "RaydiumV4"],
                "spread_bps": 420,
                "net_profit_est_sol": 0.085
            }
        ]
    });
    (StatusCode::OK, Json(sample_topology))
}

/// WebSocket endpoint for live 3D / 2D Network Topology HUD.
pub async fn ws_topology_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_topology_socket)
}

async fn handle_topology_socket(mut socket: WebSocket) {
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
    loop {
        interval.tick().await;
        let event = json!({
            "type": "topology_tick",
            "timestamp_ms": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
            "venues_active": ["PumpFun", "RaydiumV4", "OrcaWhirlpool", "MeteoraDlmm"],
            "top_opportunity": {
                "route": "WSOL -> Pump.fun -> TRUMP -> Raydium -> WSOL",
                "spread_bps": 340,
                "status": "simulated"
            }
        });
        if socket.send(Message::Text(event.to_string())).await.is_err() {
            break;
        }
    }
}
