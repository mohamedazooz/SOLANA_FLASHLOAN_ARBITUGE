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
    handle_bitquery_query, handle_bitquery_status, AppState,
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
        .route("/v1/bitquery/query", axum::routing::post(handle_bitquery_query))
        .route("/v1/bitquery/status", axum::routing::get(handle_bitquery_status))
        .with_state(state)
}

#[tokio::test]
async fn test_bitquery_status_endpoint() {
    let app = create_test_app().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/bitquery/status")
                .method("GET")
                .header("x-webhook-auth", "test-only-webhook-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    assert_eq!(body_json.get("status").and_then(|s| s.as_str()), Some("active"));
    assert_eq!(body_json.get("provider").and_then(|s| s.as_str()), Some("bitquery"));
    assert!(body_json.get("has_client_id").unwrap().as_bool().unwrap_or(false));
}

#[tokio::test]
async fn test_bitquery_unauthorized_access() {
    let app = create_test_app().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/bitquery/status")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_bitquery_query_empty_body_validation() {
    let app = create_test_app().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/bitquery/query")
                .method("POST")
                .header("x-webhook-auth", "test-only-webhook-token")
                .header("content-type", "application/json")
                .body(Body::from(json!({ "query": "" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body_bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(body_json.get("error").and_then(|s| s.as_str()), Some("GraphQL query string cannot be empty"));
}
