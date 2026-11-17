//! Block-engine rejection/acceptance contract tests against a local mock HTTP
//! server. A rejected send must surface as an Err carrying the engine's HTTP
//! status — never as a silent success — and an accepted send must return the
//! bundle id echoed by the engine.

use arb_land::JitoLander;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Serve exactly one HTTP response on a random local port and return its URL.
async fn spawn_engine(status_line: &'static str, body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let (mut sock, _) = match listener.accept().await {
            Ok(c) => c,
            Err(_) => return,
        };
        let mut buf = vec![0u8; 8192];
        let _ = sock.read(&mut buf).await;
        let resp = format!(
            "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = sock.write_all(resp.as_bytes()).await;
        let _ = sock.shutdown().await;
    });
    format!("http://{addr}")
}

fn lander_for(url: String) -> JitoLander {
    JitoLander::new(vec![url])
}

fn one_tx() -> Vec<Vec<u8>> {
    vec![vec![0u8; 64]]
}

#[tokio::test]
async fn http_400_rejection_is_an_error_not_a_success() {
    let url = spawn_engine(
        "HTTP/1.1 400 Bad Request",
        r#"{"error":"bundle rejected: tip below minimum"}"#,
    )
    .await;
    let lander = lander_for(url);
    let err = lander
        .send_bundle(&one_tx())
        .await
        .expect_err("HTTP 400 must not be reported as an accepted bundle");
    let msg = err.to_string();
    assert!(
        msg.contains("HTTP 400"),
        "error must carry the engine status, got: {msg}"
    );
    assert!(
        msg.contains("tip below minimum"),
        "error must carry the engine rejection reason, got: {msg}"
    );
}

#[tokio::test]
async fn http_200_without_result_is_not_an_accepted_bundle() {
    let url = spawn_engine("HTTP/1.1 200 OK", r#"{"jsonrpc":"2.0","id":1}"#).await;
    let lander = lander_for(url);
    let err = lander
        .send_bundle(&one_tx())
        .await
        .expect_err("HTTP 200 without a result field is not landing evidence");
    assert!(
        err.to_string().contains("HTTP 200"),
        "error must carry the engine status, got: {err}"
    );
}

#[tokio::test]
async fn accepted_bundle_returns_engine_bundle_id() {
    let url = spawn_engine(
        "HTTP/1.1 200 OK",
        r#"{"jsonrpc":"2.0","id":1,"result":"mockbundle123"}"#,
    )
    .await;
    let lander = lander_for(url);
    let bundle_id = lander
        .send_bundle(&one_tx())
        .await
        .expect("engine-accepted bundle must return its id");
    assert_eq!(bundle_id, "mockbundle123");
}

#[tokio::test]
async fn empty_bundle_is_rejected_before_any_network_call() {
    let lander = lander_for("http://127.0.0.1:1".to_string());
    let err = lander
        .send_bundle(&[])
        .await
        .expect_err("empty bundle must be rejected locally");
    assert!(err.to_string().contains("1..=5"));
}

#[tokio::test]
async fn oversized_bundle_is_rejected_before_any_network_call() {
    let lander = lander_for("http://127.0.0.1:1".to_string());
    let txs = vec![vec![0u8; 8]; 6];
    let err = lander
        .send_bundle(&txs)
        .await
        .expect_err("6-tx bundle must be rejected locally");
    assert!(err.to_string().contains("1..=5"));
}

#[tokio::test]
async fn unreachable_engine_reports_network_error() {
    // Port 1 on loopback is never our engine; connection must fail.
    let lander = lander_for("http://127.0.0.1:1".to_string());
    let err = lander
        .send_bundle(&one_tx())
        .await
        .expect_err("unreachable engine must be an error");
    assert!(
        err.to_string().contains("network error"),
        "error must classify as network error, got: {err}"
    );
}
