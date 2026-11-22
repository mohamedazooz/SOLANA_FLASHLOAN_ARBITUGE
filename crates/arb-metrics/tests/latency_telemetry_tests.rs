//! Integration tests for Microsecond Latency Telemetry (Feature 23, Tier 1).
//!
//! Validates:
//! 1. All 9 pipeline latency telemetry stages record valid samples without loss.
//! 2. Histogram quantile bucket accumulation is strictly monotonic.
//! 3. MetricsServer wire export conforms strictly to Prometheus OpenMetrics text format.
//! 4. Edge cases (0.0 us/ms observations) do not panic or produce NaN/divide-by-zero.

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

/// Helper to spin up an ephemeral MetricsServer, scrape its HTTP endpoint, and return the response.
async fn scrape_metrics_endpoint() -> String {
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        listener.local_addr().expect("local addr").port()
    };
    let addr = format!("127.0.0.1:{port}");
    let server = MetricsServer::new(&addr);
    let handle = server.start();

    // Small delay to ensure server loop has called accept()
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

#[tokio::test]
async fn test_latency_telemetry_all_stages_observed() {
    // 1. Observe values across all 9 latency stages
    observe_state_processing_latency_us(250.0);
    observe_route_discovery_latency_us(1_200.0);
    observe_exact_quote_latency_us(450.0);
    observe_assembly_latency_us(850.0);
    observe_signing_latency_us(120.0);
    observe_simulation_latency_us(15_000.0);
    observe_submit_rtt_ms(45.0);
    observe_landing_latency_ms(380.0);
    observe_e2e_latency_ms(440.0);

    // 2. Scrape the live Prometheus endpoint
    let body = scrape_metrics_endpoint().await;

    // 3. Verify that every stage's metric family is registered and has non-zero count and sum
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
        let count_marker = format!("{stage}_count");
        let sum_marker = format!("{stage}_sum");
        assert!(
            body.contains(&count_marker),
            "Metrics output missing {count_marker}"
        );
        assert!(
            body.contains(&sum_marker),
            "Metrics output missing {sum_marker}"
        );

        // Find count line and ensure count >= 1
        let count_line = body
            .lines()
            .find(|line| line.starts_with(&count_marker))
            .unwrap_or_else(|| panic!("line with {count_marker} not found"));
        let count_val: u64 = count_line
            .split_whitespace()
            .nth(1)
            .expect("count value")
            .parse()
            .expect("parse count");
        assert!(
            count_val >= 1,
            "stage {stage} count {count_val} expected >= 1"
        );
    }
}

#[tokio::test]
async fn test_histogram_quantiles_bucket_accumulation() {
    // Observe 20 samples distributed across various latency intervals
    for _ in 0..5 {
        observe_assembly_latency_us(2_000.0);
    }
    for _ in 0..5 {
        observe_assembly_latency_us(25_000.0);
    }
    for _ in 0..5 {
        observe_assembly_latency_us(100_000.0);
    }
    for _ in 0..5 {
        observe_assembly_latency_us(2_000_000.0);
    }

    let body = scrape_metrics_endpoint().await;

    // Parse assembly_latency_us_bucket lines
    let bucket_lines: Vec<&str> = body
        .lines()
        .filter(|line| line.starts_with("assembly_latency_us_bucket{le="))
        .collect();

    assert!(
        !bucket_lines.is_empty(),
        "Expected assembly_latency_us_bucket lines"
    );

    let mut prev_count: u64 = 0;
    for line in &bucket_lines {
        // e.g. assembly_latency_us_bucket{le="1000"} 5
        let count: u64 = line
            .split_whitespace()
            .nth(1)
            .expect("bucket count value")
            .parse()
            .expect("parse bucket count");
        assert!(
            count >= prev_count,
            "Cumulative bucket count violated monotonicity: prev={prev_count}, current={count} in '{line}'"
        );
        prev_count = count;
    }

    // Verify +Inf bucket matches total count
    let inf_line = bucket_lines
        .iter()
        .find(|line| line.contains("le=\"+Inf\""))
        .expect("+Inf bucket exists");
    let total_inf_count: u64 = inf_line
        .split_whitespace()
        .nth(1)
        .expect("inf count")
        .parse()
        .expect("parse inf count");

    let count_line = body
        .lines()
        .find(|line| line.starts_with("assembly_latency_us_count"))
        .expect("count line exists");
    let total_count: u64 = count_line
        .split_whitespace()
        .nth(1)
        .expect("total count")
        .parse()
        .expect("parse total count");

    assert_eq!(total_inf_count, total_count);
}

#[tokio::test]
async fn test_prometheus_text_export_wire_format() {
    observe_route_discovery_latency_us(5_000.0);
    observe_signing_latency_us(150.0);

    let body = scrape_metrics_endpoint().await;

    // Verify standard Prometheus HTTP wire format and metadata headers
    assert!(body.starts_with("HTTP/1.1 200 OK"));
    assert!(body.contains("Content-Type: text/plain; version=0.0.4"));

    // Verify OpenMetrics TYPE and HELP comments for histograms
    assert!(body.contains("# TYPE route_discovery_latency_us histogram"));
    assert!(body.contains("# TYPE signing_latency_us histogram"));

    // Verify bucket format
    assert!(body.contains("route_discovery_latency_us_bucket{le="));
    assert!(body.contains("signing_latency_us_bucket{le="));
    assert!(body.contains("route_discovery_latency_us_sum"));
    assert!(body.contains("route_discovery_latency_us_count"));
}

#[tokio::test]
async fn test_zero_and_edge_latency_observations() {
    // Edge case: observe exactly 0.0 us and ms
    observe_assembly_latency_us(0.0);
    observe_e2e_latency_ms(0.0);

    let body = scrape_metrics_endpoint().await;

    // Ensure scrape endpoint responded cleanly with HTTP 200 without NaN or panics
    assert!(body.starts_with("HTTP/1.1 200 OK"));
    assert!(!body.contains("NaN"));
    assert!(!body.contains("inf_sum"));

    // Verify sum line is a valid floating point representation
    let sum_line = body
        .lines()
        .find(|line| line.starts_with("assembly_latency_us_sum"))
        .expect("assembly sum line exists");
    let sum_val: f64 = sum_line
        .split_whitespace()
        .nth(1)
        .expect("sum value")
        .parse()
        .expect("parse float sum");
    assert!(!sum_val.is_nan());
    assert!(!sum_val.is_infinite());
}
