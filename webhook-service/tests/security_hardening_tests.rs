//! Security-hardening integration tests: rate limiting middleware, HMAC-SHA256
//! signature verification, replay-age guard, and secret redaction.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use solana_webhook_service::config::AppConfig;
use solana_webhook_service::dedup::DeduplicationEngine;
use solana_webhook_service::engine::{DefaultTelemetryListener, ExpressionEvaluator, WorkerEngine};
use solana_webhook_service::rate_limit::RateLimiter;
use solana_webhook_service::router::AmmRouter;
use solana_webhook_service::{
    event_age_allowed, handle_webhook, rate_limit_middleware, verify_hmac_signature, AppState,
};
use std::sync::Arc;
use tower::ServiceExt;

async fn create_test_app(rpm: u32) -> axum::Router {
    let mut config = AppConfig::load_from_file("webhook-service/config.toml")
        .or_else(|_| AppConfig::load_from_file("config.toml"))
        .expect("Failed to load config.toml");
    config.server.auth_token = "test-only-webhook-token".to_string();
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
        rate_limiter: Arc::new(RateLimiter::new(rpm)),
    };

    axum::Router::new()
        .route("/v1/webhook", axum::routing::post(handle_webhook))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            rate_limit_middleware,
        ))
        .with_state(state)
}

async fn post_webhook(app: axum::Router, body: &str) -> StatusCode {
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/webhook")
                .header("content-type", "application/json")
                .header("authorization", "Bearer test-only-webhook-token")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .expect("request");
    response.status()
}

#[tokio::test]
async fn rate_limit_middleware_rejects_over_limit() {
    let app = create_test_app(2).await;
    let body = r#"{"signature":"rl_test_sig","slot":1,"timestamp":1730000000,"fee":5000,"feePayer":"11111111111111111111111111111111","type":"SWAP","source":"RAYDIUM","tokenTransfers":[]}"#;

    assert_eq!(post_webhook(app.clone(), body).await, StatusCode::OK);
    assert_eq!(post_webhook(app.clone(), body).await, StatusCode::OK);
    assert_eq!(
        post_webhook(app.clone(), body).await,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn rate_limit_disabled_accepts_all() {
    let app = create_test_app(0).await;
    let body = r#"{"signature":"rl_disabled_sig","slot":1,"timestamp":1730000000,"fee":5000,"feePayer":"11111111111111111111111111111111","type":"SWAP","source":"RAYDIUM","tokenTransfers":[]}"#;
    for _ in 0..5 {
        assert_eq!(post_webhook(app.clone(), body).await, StatusCode::OK);
    }
}

#[test]
fn hmac_signature_verification() {
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;

    let secret = "unit-test-signing-secret";
    let body = br#"{"hello":"world"}"#;

    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    let tag = hex::encode(mac.finalize().into_bytes());

    assert!(verify_hmac_signature(secret, body, Some(&tag)));
    assert!(verify_hmac_signature(
        secret,
        body,
        Some(&format!("sha256={tag}"))
    ));
    assert!(verify_hmac_signature(
        secret,
        body,
        Some(&tag.to_uppercase())
    ));
    assert!(!verify_hmac_signature(secret, body, Some("deadbeef")));
    assert!(!verify_hmac_signature(secret, body, Some("not-hex!!")));
    assert!(!verify_hmac_signature(secret, body, None));
    assert!(!verify_hmac_signature("wrong-secret", body, Some(&tag)));
    assert!(!verify_hmac_signature(secret, b"tampered body", Some(&tag)));
}

#[test]
fn replay_age_guard_semantics() {
    let now: i64 = 1_730_000_000;

    assert!(
        event_age_allowed(0, Some(now - 10_000), now),
        "0 = disabled"
    );
    assert!(event_age_allowed(60, Some(now - 30), now));
    assert!(!event_age_allowed(60, Some(now - 61), now));
    assert!(
        !event_age_allowed(60, None, now),
        "missing ts = fail-closed"
    );
    assert!(
        event_age_allowed(60, Some(now + 100), now),
        "small future skew ok"
    );
    assert!(
        !event_age_allowed(60, Some(now + 301), now),
        "far-future rejected"
    );
}

#[test]
fn server_config_debug_redacts_token() {
    let mut config = AppConfig::load_from_file("webhook-service/config.toml")
        .or_else(|_| AppConfig::load_from_file("config.toml"))
        .expect("config");
    config.server.auth_token = "super-secret-live-token-9876".to_string();
    let rendered = format!("{:?}", config);
    assert!(!rendered.contains("super-secret-live-token-9876"));
    assert!(rendered.contains("***"));
}

#[test]
fn config_validation_rejects_absurd_rate_limit() {
    let mut config = AppConfig::load_from_file("webhook-service/config.toml")
        .or_else(|_| AppConfig::load_from_file("config.toml"))
        .expect("config");
    config.server.rate_limit_requests_per_minute = 3;
    assert!(config.validate().is_err());
    config.server.rate_limit_requests_per_minute = 0;
    assert!(config.validate().is_ok());
}
