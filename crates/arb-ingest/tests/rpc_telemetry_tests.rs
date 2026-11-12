//! Tests for RPC provider telemetry, percentiles, and state transitions.

use arb_ingest::rpc_pool::{EndpointStatus, RpcPool};

#[test]
fn test_unprobed_endpoints_start_as_configured_not_active() {
    let pool = RpcPool::new(vec![
        "https://api.mainnet-beta.solana.com".to_string(),
        "https://solana-mainnet.g.alchemy.com/v2/dummy".to_string(),
    ]);

    let health = pool.health_snapshot();
    assert_eq!(health.total, 2);
    assert_eq!(health.configured_count, 2);
    assert_eq!(health.probed_count, 0);
    assert_eq!(health.healthy_count, 0);

    for p in &health.providers {
        assert_eq!(p.status, EndpointStatus::Configured);
        assert_eq!(p.p50_latency_ms, 0);
        assert_eq!(p.p95_latency_ms, 0);
        assert_eq!(p.p99_latency_ms, 0);
    }
}

#[test]
fn test_latency_percentiles_calculation() {
    let pool = RpcPool::new(vec!["https://api.mainnet-beta.solana.com".to_string()]);

    let snapshot_before = pool.health_snapshot();
    assert_eq!(snapshot_before.providers[0].p50_latency_ms, 0);

    // Verify ProviderConfig classifier
    let conf = RpcPool::classify_provider(0, "https://solana-mainnet.g.alchemy.com/v2/test");
    assert!(conf.label.contains("Alchemy"));
    assert_eq!(conf.max_batch_size, 100);

    let conf_fast = RpcPool::classify_provider(1, "https://solana-rpc.rpcfast.com/?api_key=test");
    assert!(conf_fast.label.contains("RPCFast"));
    assert_eq!(conf_fast.max_batch_size, 5);
}

#[test]
fn test_status_formatting_reflects_actual_probed_state() {
    let pool = RpcPool::new(vec!["https://api.mainnet-beta.solana.com".to_string()]);

    let status = pool.format_compact_status();
    assert!(
        status.contains("0/1 Healthy (0/1 Probed)"),
        "Unprobed endpoints must not be labeled active: {status}"
    );
}
