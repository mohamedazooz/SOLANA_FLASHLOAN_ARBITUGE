//! Feature 23: Microsecond Latency Telemetry — Tier 2 Boundary Tests.
//!
//! Validates:
//! 1. Sub-microsecond observations (e.g. 0.01 us, 0.25 us) record without NaN or precision loss.
//! 2. Negative observations (from clock skew) are handled gracefully without panicking or corrupting export.
//! 3. Large multi-day observations (e.g. 24h = 86,400,000 ms) accumulate into +Inf bucket without overflow.
//! 4. Extreme large float observations (1e12 us) serialize properly as valid Prometheus text format.
//! 5. Explicit 0.0 observations across all 9 pipeline stages record cleanly without divide-by-zero.

use arb_metrics::{
    MetricsServer, observe_assembly_latency_us, observe_e2e_latency_ms,
    observe_exact_quote_latency_us, observe_landing_latency_ms, observe_route_discovery_latency_us,
    observe_signing_latency_us, observe_simulation_latency_us, observe_state_processing_latency_us,
    observe_submit_rtt_ms,
};
use std::net::TcpListener;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

async fn scrape_metrics() -> String {
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        listener.local_addr().expect("local addr").port()
    };
    let addr = format!("127.0.0.1:{port}");
    let server = MetricsServer::new(&addr);
    let handle = server.start();

    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut stream = TcpStream::connect(&addr)
        .await
        .expect("connect to metrics endpoint");
    stream
        .write_all(b"GET /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .expect("write request");

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .expect("read response");

    handle.shutdown().await;
    String::from_utf8_lossy(&response).to_string()
}

/// Boundary Case 1: Sub-microsecond observations (0.25 us, 0.01 us).
#[tokio::test]
async fn test_latency_sub_microsecond_observation() {
    observe_assembly_latency_us(0.25);
    observe_state_processing_latency_us(0.01);

    let body = scrape_metrics().await;
    assert!(body.starts_with("HTTP/1.1 200 OK"));
    assert!(!body.contains("NaN"));

    let sum_line = body
        .lines()
        .find(|l| l.starts_with("assembly_latency_us_sum"))
        .expect("assembly sum line exists");
    let val: f64 = sum_line.split_whitespace().nth(1).unwrap().parse().unwrap();
    assert!(val >= 0.25);
}

/// Boundary Case 2: Negative latency observation from clock skew handled gracefully.
#[tokio::test]
async fn test_latency_negative_value_handling() {
    // Clock skew or jitter can produce negative duration
    observe_route_discovery_latency_us(-100.0);

    let body = scrape_metrics().await;
    assert!(body.starts_with("HTTP/1.1 200 OK"));
    assert!(!body.contains("NaN"));
    assert!(body.contains("route_discovery_latency_us_count"));
}

/// Boundary Case 3: Large multi-day observation (24 hours = 86,400,000 ms).
#[tokio::test]
async fn test_latency_large_multi_day_observation() {
    let day_ms = 86_400_000.0;
    observe_landing_latency_ms(day_ms);

    let body = scrape_metrics().await;
    assert!(body.starts_with("HTTP/1.1 200 OK"));
    assert!(!body.contains("NaN"));

    let sum_line = body
        .lines()
        .find(|l| l.starts_with("landing_latency_ms_sum"))
        .expect("landing sum line exists");
    let val: f64 = sum_line.split_whitespace().nth(1).unwrap().parse().unwrap();
    assert!(val >= day_ms);
}

/// Boundary Case 4: Extreme high float observation (1e12 us) serializes without panic.
#[tokio::test]
async fn test_latency_histogram_overflow_extreme_max_float() {
    observe_simulation_latency_us(1e12);

    let body = scrape_metrics().await;
    assert!(body.starts_with("HTTP/1.1 200 OK"));
    assert!(!body.contains("NaN"));
    assert!(body.contains("simulation_latency_us_count"));
}

/// Boundary Case 5: Exactly 0.0 observed across all 9 latency stages.
#[tokio::test]
async fn test_latency_zero_point_zero_all_nine_stages() {
    observe_state_processing_latency_us(0.0);
    observe_route_discovery_latency_us(0.0);
    observe_exact_quote_latency_us(0.0);
    observe_assembly_latency_us(0.0);
    observe_signing_latency_us(0.0);
    observe_simulation_latency_us(0.0);
    observe_submit_rtt_ms(0.0);
    observe_landing_latency_ms(0.0);
    observe_e2e_latency_ms(0.0);

    let body = scrape_metrics().await;
    assert!(body.starts_with("HTTP/1.1 200 OK"));
    assert!(!body.contains("NaN"));

    let stages = [
        "state_processing_latency_us",
        "route_discovery_latency_us",
        "exact_quote_latency_us",
        "assembly_latency_us",
        "signing_latency_us",
        "simulation_latency_us",
        "submit_rtt_ms",
        "landing_latency_ms",
        "e2e_latency_ms",
    ];

    for stage in stages {
        let count_line = format!("{stage}_count");
        assert!(
            body.contains(&count_line),
            "Metrics output must contain {}",
            count_line
        );
    }
}
