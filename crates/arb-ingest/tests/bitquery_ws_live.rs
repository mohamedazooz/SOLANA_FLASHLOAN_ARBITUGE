use arb_ingest::bitquery::BitqueryWsClient;
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn test_live_bitquery_ws_handshake_and_stream() {
    let ws_client = match BitqueryWsClient::from_env() {
        Some(c) => c,
        None => {
            eprintln!(
                "Skipping live WS test: BITQUERY_CLIENT_ID or BITQUERY_ACCESS_TOKEN not found in env"
            );
            return;
        }
    };

    println!("[Live Bitquery WS] Resolving WS URL with active token...");
    let ws_url = ws_client
        .resolve_ws_url()
        .await
        .expect("Failed to resolve WS URL");
    assert!(ws_url.starts_with("wss://streaming.bitquery.io/graphql?token="));
    println!(
        "[Live Bitquery WS] Resolved URL: {}...{}",
        &ws_url[..45],
        &ws_url[ws_url.len() - 10..]
    );

    let ws_arc = Arc::new(ws_client);
    let (mut rx, handle) = ws_arc.clone().spawn_subscription_stream(None, 64);

    // Collect trades for up to 8 seconds
    let mut collected = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);

    println!("[Live Bitquery WS] Listening for incoming DEX trades...");
    while tokio::time::Instant::now() < deadline {
        tokio::select! {
            trade_opt = rx.recv() => {
                if let Some(trade) = trade_opt {
                    collected += 1;
                    println!(
                        "  [Live Trade #{}] Protocol: {} | Sig: {} | Swap: {} {} -> {} {}",
                        collected,
                        trade.trade.dex.protocol_name,
                        &trade.transaction.signature[..trade.transaction.signature.len().min(24)],
                        trade.trade.sell.amount,
                        trade.trade.sell.currency.symbol,
                        trade.trade.buy.amount,
                        trade.trade.buy.currency.symbol
                    );
                    if collected >= 2 {
                        break;
                    }
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }

    ws_arc.stop();
    handle.abort();

    println!(
        "[Live Bitquery WS] Collected {} live trades via WebSocket stream.",
        collected
    );
}
