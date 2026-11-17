//! Quota-exhaustion (HTTP 429) contract tests against local mock RPC servers.
//!
//! Contract under test (`RpcPool::call`):
//! - HTTP 429 with quota-exhaustion markers (max usage / quota / credits) must
//!   fail closed with a long cooldown (1800s) and mark the endpoint
//!   QuotaExhausted — never silently retried in the same dispatch.
//! - HTTP 429 plain rate-limit must back off (Retry-After) and fail over to
//!   the next healthy endpoint in the same dispatch.
//! - A JSON-RPC `-32005` in-body error must be classified as rate limiting.

use arb_ingest::rpc_pool::RpcPool;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;

/// Serve `responses` sequentially (one per accepted connection) on a random
/// local port and return its URL. Each connection is read then answered with
/// the next queued HTTP response.
fn spawn_rpc(responses: Vec<(u16, Vec<(&'static str, String)>, String)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (tx, rx) = mpsc::channel::<(u16, Vec<(&'static str, String)>, String)>();
    for r in responses {
        tx.send(r).expect("send");
    }
    drop(tx);
    thread::spawn(move || {
        for listener_conn in listener.incoming() {
            let Ok(response) = rx.try_recv() else {
                return;
            };
            let Ok(mut sock) = listener_conn else {
                return;
            };
            let mut buf = [0u8; 8192];
            let _ = sock.read(&mut buf);
            let (status, headers, body) = response;
            let reason = match status {
                200 => "OK",
                429 => "Too Many Requests",
                _ => "Error",
            };
            let mut resp = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                body.len()
            );
            for (k, v) in &headers {
                resp.push_str(&format!("{k}: {v}\r\n"));
            }
            resp.push_str("\r\n");
            resp.push_str(&body);
            let _ = sock.write_all(resp.as_bytes());
            let _ = sock.flush();
        }
    });
    format!("http://{addr}")
}

fn json_result(v: &str) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":1,"result":{v}}}"#)
}

#[tokio::test]
async fn http_429_quota_exhaustion_fails_closed_with_long_cooldown() {
    let url = spawn_rpc(vec![(
        429,
        vec![],
        r#"{"error":"max usage reached - monthly quota exhausted"}"#.to_string(),
    )]);
    let pool = RpcPool::new(vec![url]);

    let err = pool
        .call("getSlot", serde_json::json!([]))
        .await
        .expect_err("quota exhaustion must fail closed, not return stale/zero data");
    assert!(
        err.to_string().contains("429"),
        "error must carry the 429 classification, got: {err}"
    );

    let health = pool.health_snapshot();
    assert_eq!(
        health.quota_exhausted_count, 1,
        "endpoint must be marked QuotaExhausted"
    );
    let p = &health.providers[0];
    assert!(p.quota_exhausted, "provider flag must be set");
    assert!(
        p.cooldown_remaining_secs >= 1700,
        "quota exhaustion must apply the long 1800s cooldown, got {}",
        p.cooldown_remaining_secs
    );
    assert!(health.total_429 >= 1, "429 counter must be recorded");
}

#[tokio::test]
async fn http_429_rate_limit_fails_over_to_healthy_endpoint_same_dispatch() {
    let bad = spawn_rpc(vec![(
        429,
        vec![("Retry-After", "5".to_string())],
        r#"{"error":"too many requests"}"#.to_string(),
    )]);
    let good = spawn_rpc(vec![(200, vec![], json_result("12345"))]);
    let pool = RpcPool::new(vec![bad, good]);

    let val = pool
        .call("getSlot", serde_json::json!([]))
        .await
        .expect("dispatch must fail over to the healthy endpoint");
    assert_eq!(val.as_u64(), Some(12345));

    let health = pool.health_snapshot();
    assert_eq!(
        health.total_429, 1,
        "the 429 must be attributed to the limited endpoint"
    );
    assert_eq!(
        health.total_ok, 1,
        "the healthy endpoint's success must be recorded"
    );
    assert!(
        health.providers[0].cooldown_remaining_secs > 0,
        "rate-limited endpoint must be cooling down (Retry-After honored)"
    );
}

#[tokio::test]
async fn json_rpc_error_32005_is_classified_as_rate_limit() {
    let bad = spawn_rpc(vec![(
        200,
        vec![],
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32005,"message":"Node is behind or rate limited"}}"#.to_string(),
    )]);
    let good = spawn_rpc(vec![(200, vec![], json_result("777"))]);
    let pool = RpcPool::new(vec![bad, good]);

    let val = pool
        .call("getSlot", serde_json::json!([]))
        .await
        .expect("dispatch must fail over to the healthy endpoint");
    assert_eq!(val.as_u64(), Some(777));

    let health = pool.health_snapshot();
    assert_eq!(
        health.total_429, 1,
        "in-body -32005 must be classified as rate limiting, not a generic error"
    );
    assert!(
        health.providers[0].cooldown_remaining_secs > 0,
        "rate-limited endpoint must receive a backoff cooldown"
    );
}
