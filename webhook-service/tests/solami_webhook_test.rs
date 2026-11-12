use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::json;
use solana_webhook_service::config::AppConfig;
use solana_webhook_service::dedup::DeduplicationEngine;
use solana_webhook_service::engine::{DefaultTelemetryListener, ExpressionEvaluator, WorkerEngine};
use solana_webhook_service::router::AmmRouter;
use solana_webhook_service::{
    handle_solami_webhook, handle_solami_webhook_get, handle_webhook, health_check, AppState,
};
use std::sync::Arc;
use tower::ServiceExt;

async fn create_test_app() -> axum::Router {
    let config = AppConfig::load_from_file("webhook-service/config.toml")
        .or_else(|_| AppConfig::load_from_file("config.toml"))
        .expect("Failed to load config.toml");
    let dedup = DeduplicationEngine::new("redis://127.0.0.1:6379", 2, 5000)
        .await
        .expect("DeduplicationEngine init failed");
    let router = AmmRouter::new(&config);
    let evaluator = ExpressionEvaluator::new(config.filter.clone(), router.clone());
    let worker = Arc::new(WorkerEngine::new(
        1024,
        2,
        Arc::new(DefaultTelemetryListener),
    ));

    let state = AppState {
        config: Arc::new(config),
        dedup,
        router,
        evaluator,
        worker,
    };

    axum::Router::new()
        .route("/v1/webhook", axum::routing::post(handle_webhook))
        .route("/health", axum::routing::get(health_check))
        .route(
            "/webhooks/SOLAMI",
            axum::routing::post(handle_solami_webhook).get(handle_solami_webhook_get),
        )
        .route(
            "/webhooks/solami",
            axum::routing::post(handle_solami_webhook).get(handle_solami_webhook_get),
        )
        .with_state(state)
}

#[tokio::test]
async fn test_solami_webhook_get_diagnostic() {
    let app = create_test_app().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/webhooks/SOLAMI")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_solami_webhook_auth_protection() {
    let app = create_test_app().await;

    // Send payload with NO authorization header
    let payload = json!({
        "type": "swap",
        "signature": "solami_test_unauth_sig_12345",
        "pool": "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2",
        "dex": "raydium_clmm",
        "amount_usd": 150.0
    });

    let response = app
        .oneshot(
            Request::builder()
                .uri("/webhooks/SOLAMI")
                .method("POST")
                .header("Content-Type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_solami_webhook_authorized_processing_and_dedup() {
    let app = create_test_app().await;
    let auth_token = "test-only-webhook-token";
    let test_sig = format!("solami_test_swap_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos());

    let payload = json!({
        "type": "swap",
        "signature": test_sig,
        "slot": 298102931,
        "dex": "raydium_clmm",
        "pool": "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2",
        "address": "So11111111111111111111111111111111111111112",
        "side": "buy",
        "amount_usd": 2500.0,
        "base_amount": 15000000000u64,
        "trader": "7yqBsmfN24FmG29G84dE6p1C4j1V3bK6wY7tM4sD2p1"
    });

    let from_str_event: Result<solana_webhook_service::SolamiEvent, _> =
        serde_json::from_str(&payload.to_string());
    println!("From str event result: {:?}", from_str_event);

    // 1. First execution - should process
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/webhooks/SOLAMI")
                .method("POST")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {auth_token}"))
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // 2. Second execution with identical signature - deduplication should drop it
    let dedup_response = app
        .oneshot(
            Request::builder()
                .uri("/webhooks/SOLAMI")
                .method("POST")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {auth_token}"))
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(dedup_response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_solami_webhook_batch_processing() {
    let app = create_test_app().await;
    let auth_token = "test-only-webhook-token";
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();

    let batch_payload = json!([
        {
            "type": "token_create",
            "signature": format!("solami_batch_create_{now}"),
            "mint": "So11111111111111111111111111111111111111112",
            "dex": "pumpswap"
        },
        {
            "type": "graduation",
            "signature": format!("solami_batch_grad_{now}"),
            "pool": "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2",
            "dex": "raydium_cpmm"
        }
    ]);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/webhooks/solami")
                .method("POST")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {auth_token}"))
                .body(Body::from(batch_payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}
