use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::json;
use solana_webhook_service::config::AppConfig;
use solana_webhook_service::dedup::DeduplicationEngine;
use solana_webhook_service::engine::{DefaultTelemetryListener, ExpressionEvaluator, WorkerEngine};
use solana_webhook_service::models::*;
use solana_webhook_service::router::AmmRouter;
use solana_webhook_service::{handle_webhook, health_check, AppState};
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
        .with_state(state)
}

#[tokio::test]
async fn test_numeric_precision_deserialization() {
    let raw_json = json!({
        "signature": "5KjP5eZ...test_sig_numeric",
        "slot": "291039485", // Provided as string
        "timestamp": 1730000000,
        "fee": 5000,          // Provided as number
        "feePayer": "11111111111111111111111111111111",
        "type": "SWAP",
        "source": "RAYDIUM",
        "tokenTransfers": [
            {
                "mint": "So11111111111111111111111111111111111111112",
                "tokenAmount": "18446744073709551615", // u64::MAX value as string
                "rawTokenAmount": 1000000000
            },
            {
                "mint": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
                "tokenAmount": "340282366920938463463374607431768211455", // u128::MAX value string
                "rawTokenAmount": 5000000
            }
        ]
    });

    let payload: SolanaTransactionPayload =
        serde_json::from_value(raw_json).expect("Deserialization failed");

    assert_eq!(payload.slot, 291039485_u64);
    assert_eq!(payload.fee, 5000_u64);
    assert_eq!(payload.token_transfers[0].token_amount, u64::MAX as u128);
    assert_eq!(payload.token_transfers[1].token_amount, u128::MAX);
}

#[tokio::test]
async fn test_amm_router_resolution() {
    let config = AppConfig::load_from_file("webhook-service/config.toml")
        .or_else(|_| AppConfig::load_from_file("config.toml"))
        .expect("Failed to load config.toml");
    let router = AmmRouter::new(&config);

    // Create a payload interacting with Raydium SOL/USDC pool
    let payload = SolanaTransactionPayload {
        signature: "sig_router_test_1".to_string(),
        slot: 100,
        timestamp: 1730000000,
        fee: 5000,
        fee_payer: "User111".to_string(),
        transaction_type: "SWAP".to_string(),
        source: "RAYDIUM".to_string(),
        description: None,
        transaction_error: None,
        token_transfers: vec![],
        native_transfers: vec![],
        account_data: vec![],
        instructions: vec![Instruction {
            accounts: vec![
                "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2".to_string(), // Raydium V4 Pool
                "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8".to_string(), // Raydium Program
            ],
            data: "swap_data".to_string(),
            program_id: "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8".to_string(),
            inner_instructions: vec![],
        }],
        inner_instructions: vec![],
        raw: None,
        events: None,
    };

    let resolved = router.resolve_modified_pools(&payload);
    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].pool_id, "58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2");
    assert_eq!(resolved[0].amm_type, "raydium_v4");
}

#[tokio::test]
async fn test_expression_evaluator_filtering() {
    let config = AppConfig::load_from_file("webhook-service/config.toml")
        .or_else(|_| AppConfig::load_from_file("config.toml"))
        .expect("Failed to load config.toml");
    let router = AmmRouter::new(&config);
    let evaluator = ExpressionEvaluator::new(config.filter.clone(), router);

    // 1. Transaction with transaction_error should be dropped
    let failed_tx = SolanaTransactionPayload {
        signature: "failed_tx_sig".to_string(),
        slot: 101,
        timestamp: 1730000000,
        fee: 5000,
        fee_payer: "User111".to_string(),
        transaction_type: "SWAP".to_string(),
        source: "RAYDIUM".to_string(),
        description: None,
        transaction_error: Some(json!({"InstructionError": [0, "Custom"]})),
        token_transfers: vec![],
        native_transfers: vec![],
        account_data: vec![],
        instructions: vec![],
        inner_instructions: vec![],
        raw: None,
        events: None,
    };
    assert!(!evaluator.should_process(&failed_tx));

    // 2. Transaction without monitored AMM should be dropped
    let unmonitored_tx = SolanaTransactionPayload {
        signature: "unmonitored_tx_sig".to_string(),
        slot: 102,
        timestamp: 1730000000,
        fee: 5000,
        fee_payer: "User111".to_string(),
        transaction_type: "TRANSFER".to_string(),
        source: "SYSTEM".to_string(),
        description: None,
        transaction_error: None,
        token_transfers: vec![],
        native_transfers: vec![],
        account_data: vec![],
        instructions: vec![Instruction {
            accounts: vec![],
            data: "".to_string(),
            program_id: "11111111111111111111111111111111".to_string(), // System program
            inner_instructions: vec![],
        }],
        inner_instructions: vec![],
        raw: None,
        events: None,
    };
    assert!(!evaluator.should_process(&unmonitored_tx));
}

#[tokio::test]
async fn test_webhook_endpoints_integration() {
    let app = create_test_app().await;

    // Test /health
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);

    // Test /v1/webhook without Auth (Should fail with 401 Unauthorized)
    let body_json = json!({
        "signature": "webhook_sig_test_1001",
        "slot": 200,
        "type": "SWAP",
        "source": "RAYDIUM",
        "instructions": [{
            "accounts": ["58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2"],
            "data": "",
            "programId": "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8",
            "innerInstructions": []
        }]
    });

    let res_unauth = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/webhook")
                .method("POST")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_vec(&body_json).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res_unauth.status(), StatusCode::UNAUTHORIZED);

    // Test /v1/webhook with Bearer Auth (Should succeed with 200 OK)
    let res_auth = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/webhook")
                .method("POST")
                .header("Content-Type", "application/json")
                .header("Authorization", "Bearer test-only-webhook-token")
                .body(Body::from(serde_json::to_vec(&body_json).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res_auth.status(), StatusCode::OK);

    // Test consecutive duplicate call (Deduplication should detect duplicate and return 200 OK with dispatched = 0)
    let res_dup = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/webhook")
                .method("POST")
                .header("Content-Type", "application/json")
                .header("Authorization", "Bearer test-only-webhook-token")
                .body(Body::from(serde_json::to_vec(&body_json).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res_dup.status(), StatusCode::OK);
}
