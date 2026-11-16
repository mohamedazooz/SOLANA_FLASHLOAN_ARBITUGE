//! Production-Ready, High-Frequency Solana Webhook Endpoint Service Binary.

use solana_webhook_service::config::AppConfig;
use solana_webhook_service::dedup::DeduplicationEngine;
use solana_webhook_service::engine::{DefaultTelemetryListener, ExpressionEvaluator, WorkerEngine};
use solana_webhook_service::router::AmmRouter;
use solana_webhook_service::{
    handle_bitquery_query, handle_bitquery_status, handle_helius_pumpfun_get,
    handle_helius_pumpfun_webhook, handle_solami_webhook, handle_solami_webhook_get,
    handle_webhook, health_check, topology_snapshot, ws_topology_handler, AppState,
};

use axum::{
    extract::DefaultBodyLimit,
    routing::{get, post},
    Router,
};
use axum::http::HeaderValue;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

/// Build the CORS layer from an explicit origin allowlist. Per the OWASP
/// Webhook Security Cheat Sheet, `CorsLayer::permissive()` is never allowed:
/// an empty allowlist emits no CORS headers (browser cross-origin access
/// denied) rather than allowing every origin.
fn build_cors_layer(allowed_origins: &[String]) -> CorsLayer {
    let origins: Vec<HeaderValue> = allowed_origins
        .iter()
        .filter_map(|o| o.parse().ok())
        .collect();
    if origins.is_empty() {
        warn!(target: "security", "CORS allowlist empty: cross-origin browser access disabled");
    }
    CorsLayer::new().allow_origin(origins)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "solana_webhook_service=info,tower_http=warn".into()),
        )
        .with_target(true)
        .compact()
        .init();

    info!(target: "main", "Starting Sentinel Solana Webhook Service...");

    // Load TOML configuration
    let config = AppConfig::load_default().unwrap_or_else(|e| {
        warn!(target: "main", error = %e, "Failed to load default config, trying 'config.toml' in current directory");
        AppConfig::load_from_file("config.toml").expect("FATAL: Unable to load config.toml")
    });
    let config_arc = Arc::new(config.clone());

    info!(
        target: "main",
        host = %config.server.host,
        port = config.server.port,
        workers = config.server.worker_threads,
        "Configuration successfully loaded"
    );

    // Initialize Deduplication Engine (Local DashMap + Redis)
    let dedup = DeduplicationEngine::new(
        &config.dedup.redis_url,
        config.dedup.local_cache_ttl_secs,
        config.dedup.redis_ttl_ms,
    )
    .await?;

    // Start background DashMap pruning task
    dedup.start_pruning_task(Duration::from_secs(config.dedup.prune_interval_secs));

    // Initialize AMM Redundancy Router
    let router = AmmRouter::new(&config);

    // Initialize Expression Evaluator
    let evaluator = ExpressionEvaluator::new(config.filter.clone(), router.clone());

    // Initialize Async Worker Engine
    let telemetry_listener = Arc::new(DefaultTelemetryListener);
    let worker = Arc::new(WorkerEngine::new(
        config.server.channel_buffer_size,
        config.server.worker_threads,
        telemetry_listener,
    ));

    // Build shared state
    let state = AppState {
        config: config_arc.clone(),
        dedup,
        router,
        evaluator,
        worker,
    };

    // Define Axum application routes
    let app = Router::new()
        .route("/v1/webhook", post(handle_webhook))
        .route("/health", get(health_check))
        .route("/webhooks/SOLAMI", post(handle_solami_webhook).get(handle_solami_webhook_get))
        .route("/webhooks/solami", post(handle_solami_webhook).get(handle_solami_webhook_get))
        .route(
            "/webhooks/helius/pumpfun",
            post(handle_helius_pumpfun_webhook).get(handle_helius_pumpfun_get),
        )
        .route("/v1/bitquery/query", post(handle_bitquery_query))
        .route("/v1/bitquery/status", get(handle_bitquery_status))
        .route("/api/topology", get(topology_snapshot))
        .route("/ws/topology", get(ws_topology_handler))
        .layer(TraceLayer::new_for_http())
        .layer(build_cors_layer(&config.server.cors_allowed_origins))
        .layer(DefaultBodyLimit::max(config.server.max_body_bytes))
        .with_state(state);

    let addr: SocketAddr = format!("{}:{}", config.server.host, config.server.port)
        .parse()
        .expect("FATAL: Invalid bind address");

    info!(target: "main", %addr, "Webhook HTTP server listening");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!(target: "main", "Webhook service shutdown complete");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {
            info!(target: "main", "Ctrl+C signal received, initiating graceful shutdown");
        },
        _ = terminate => {
            info!(target: "main", "SIGTERM signal received, initiating graceful shutdown");
        },
    }
}
