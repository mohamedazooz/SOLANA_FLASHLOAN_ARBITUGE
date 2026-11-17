//! Concurrency hardening tests (checklist 542).
//!
//! These stress the shared-state hot paths of the ingest layer under parallel
//! load and assert observable invariants — not merely "it didn't crash":
//! - `SlotIntegrity`: streamed slot updates are serialized behind one lock in
//!   the ingest path; hammering it from many threads must not stall (bounded
//!   wall clock) and its invariants (finalized ≤ highest, gap accounting stays
//!   live after contention) must hold.
//! - `HeliusWsClient`: endpoint rotation and account tracking are shared
//!   mutably from many threads; every rotation must yield a valid index.
//! - `RpcPool`: a concurrent burst against 429-limited endpoints must fail
//!   closed, and once every endpoint is in cooldown further calls must issue
//!   ZERO new HTTP requests — the cooldown closes the storm.

use arb_ingest::rpc_pool::RpcPool;
use arb_ingest::slot_integrity::{SLOT_CONFIRMED, SlotIntegrity};
use solana_pubkey::Pubkey;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const TARGET_DURATION: Duration = Duration::from_secs(10);

#[test]
fn slot_integrity_concurrent_observers_no_stall_and_invariants_hold() {
    let integrity = Arc::new(Mutex::new(SlotIntegrity::default()));
    let mut handles = Vec::new();
    for t in 0..8u64 {
        let shared = Arc::clone(&integrity);
        handles.push(thread::spawn(move || {
            for k in 0..250u64 {
                let slot = t * 1000 + k * 3;
                let parent = if k == 0 { None } else { Some(slot - 3) };
                let mut guard = shared.lock().unwrap();
                let _ =
                    guard.observe_with_bank(slot, parent, SLOT_CONFIRMED, Some(t * 100_000 + k));
            }
        }));
    }
    let started = Instant::now();
    for h in handles {
        h.join().expect("observer threads must not panic");
    }
    assert!(
        started.elapsed() < TARGET_DURATION,
        "contended observations must not stall, took {:?}",
        started.elapsed()
    );

    let mut guard = integrity.lock().unwrap();
    assert!(guard.highest_seen() > 0);
    assert!(
        guard.finalized_slot() <= guard.highest_seen(),
        "finalized watermark may never exceed the high-water mark"
    );
    let gaps_before = guard.gaps_detected();
    let outcome = guard.observe(9_999_999, None, SLOT_CONFIRMED);
    assert!(matches!(
        outcome,
        arb_ingest::slot_integrity::SlotUpdateOutcome::Observed
    ));
    assert!(
        guard.gaps_detected() > gaps_before,
        "gap accounting must still count after contention"
    );
}

#[test]
fn helius_ws_concurrent_rotation_and_tracking_unstalled_and_valid() {
    let client = Arc::new(arb_ingest::HeliusWsClient::new_with_options(
        vec![
            "wss://ws1.example.com".to_string(),
            "wss://ws2.example.com".to_string(),
            "wss://ws3.example.com".to_string(),
        ],
        false,
    ));
    let mut handles = Vec::new();
    for _t in 0..8 {
        let shared = Arc::clone(&client);
        handles.push(thread::spawn(move || {
            for _k in 0..100 {
                let idx = shared.rotate_endpoint();
                assert!(
                    idx < 3,
                    "rotation must always yield a valid endpoint index, got {idx}"
                );
                shared.track_account(Pubkey::new_unique(), Pubkey::new_unique());
            }
        }));
    }
    let started = Instant::now();
    for h in handles {
        h.join().expect("rotation threads must not panic");
    }
    assert!(
        started.elapsed() < TARGET_DURATION,
        "contended rotation/tracking must not stall, took {:?}",
        started.elapsed()
    );
}

/// A 429-only mock origin that counts accepted HTTP connections forever.
fn spawn_429_counter(counter: Arc<AtomicUsize>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    thread::spawn(move || {
        for conn in listener.incoming() {
            counter.fetch_add(1, Ordering::SeqCst);
            if let Ok(mut sock) = conn {
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf);
                let _ = sock.write_all(
                    b"HTTP/1.1 429 Too Many Requests\r\ncontent-length: 0\r\nretry-after: 5\r\nconnection: close\r\n\r\n",
                );
                let _ = sock.flush();
            }
        }
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn rpc_pool_concurrent_429_burst_fails_closed_and_cooldown_closes_the_storm() {
    let mut counters = Vec::new();
    let mut urls = Vec::new();
    for _ in 0..3 {
        let c = Arc::new(AtomicUsize::new(0));
        urls.push(spawn_429_counter(Arc::clone(&c)));
        counters.push(c);
    }
    let pool = Arc::new(RpcPool::new(urls));

    let started = Instant::now();
    let mut handles = Vec::new();
    for _ in 0..12 {
        let p = Arc::clone(&pool);
        handles.push(tokio::spawn(async move {
            p.call("getSlot", serde_json::json!([])).await
        }));
    }
    let mut errs = Vec::new();
    for h in handles {
        errs.push(h.await.expect("tasks must not panic"));
    }
    assert!(
        started.elapsed() < TARGET_DURATION,
        "the burst must resolve in bounded time, took {:?}",
        started.elapsed()
    );
    assert_eq!(errs.len(), 12);
    for e in &errs {
        let msg = e.as_ref().map(|_| ()).unwrap_err().to_string();
        assert!(
            msg.contains("429") || msg.contains("failed"),
            "every concurrent call must fail closed with the real reason, got: {msg}"
        );
    }

    // Every endpoint is now in a 5s Retry-After cooldown: further calls must
    // fail fast WITHOUT issuing any new HTTP request (the storm is closed).
    let before: usize = counters.iter().map(|c| c.load(Ordering::SeqCst)).sum();
    for _ in 0..3 {
        let e = pool
            .call("getSlot", serde_json::json!([]))
            .await
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("cooldown"),
            "expected cooldown fail-fast, got: {e}"
        );
    }
    let after: usize = counters.iter().map(|c| c.load(Ordering::SeqCst)).sum();
    assert_eq!(
        before, after,
        "cooldown enforcement must issue zero new HTTP requests to limited providers"
    );
}
