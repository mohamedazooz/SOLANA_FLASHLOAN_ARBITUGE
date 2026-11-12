//! Jito tip-floor oracle.
//!
//! The landed-tip distribution is the only public price signal for the Jito
//! auction. It is published as a REST snapshot, polled well inside the
//! documented per-IP rate limit, and read through a freshness contract: a stale
//! snapshot is treated as absent rather than silently reused, because pricing a
//! bid against yesterday's floor is indistinguishable from bidding blind.
//!
//! A WebSocket stream (`tip_stream`) supplements REST polling with push updates
//! so the bidder can react to market moves within milliseconds instead of waiting
//! for the next poll.

use arb_types::{ArbError, Lamports, Result as ArbResult, TipFloor};
use futures::StreamExt;
use parking_lot::RwLock;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, error, warn};
use url::Url;

/// Public tip-floor REST endpoint.
pub const TIP_FLOOR_URL: &str = "https://bundles.jito.wtf/api/v1/bundles/tip_floor";
/// WebSocket endpoint for real-time tip updates.
pub const TIP_WEBSOCKET_URL: &str = "wss://bundles.jito.wtf/api/v1/bundles/tip_stream";

/// Default freshness window. The endpoint republishes continuously; five
/// seconds of staleness already spans more than a dozen slots.
pub const DEFAULT_MAX_AGE_MS: u64 = 5_000;

/// Lamports in one SOL.
const LAMPORTS_PER_SOL: f64 = 1_000_000_000.0;

/// Polls and caches the Jito tip floor via REST, with an optional WebSocket
/// push channel for real-time updates.
#[derive(Debug)]
pub struct TipOracle {
    url: String,
    ws_url: String,
    client: reqwest::Client,
    max_age_ms: u64,
    snapshot: RwLock<Option<TipFloor>>,
    /// Broadcast channel fed by the WebSocket reader; `None` until `run_ws`
    /// starts it.
    ws_tx: RwLock<Option<broadcast::Sender<TipFloor>>>,
}

impl TipOracle {
    /// Build an oracle against the public endpoint.
    pub fn new(max_age_ms: u64) -> Self {
        Self::with_urls(TIP_FLOOR_URL, TIP_WEBSOCKET_URL, max_age_ms)
    }

    /// Build an oracle against explicit endpoints.
    pub fn with_urls(url: &str, ws_url: &str, max_age_ms: u64) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap_or_default();
        Self {
            url: url.to_string(),
            ws_url: ws_url.to_string(),
            client,
            max_age_ms,
            snapshot: RwLock::new(None),
            ws_tx: RwLock::new(None),
        }
    }

    /// Freshness window applied by [`Self::current`].
    pub fn max_age_ms(&self) -> u64 {
        self.max_age_ms
    }

    /// Fetch and cache one snapshot via REST.
    pub async fn refresh(&self) -> ArbResult<TipFloor> {
        let response = self
            .client
            .get(&self.url)
            .send()
            .await
            .map_err(|e| ArbError::Internal(format!("tip floor request failed: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            return Err(ArbError::Internal(format!(
                "tip floor returned HTTP {status}"
            )));
        }

        let body: Value = response
            .json()
            .await
            .map_err(|e| ArbError::DeserializationError(format!("tip floor body: {e}")))?;

        let snapshot = parse_tip_floor(&body, now_ms())?;
        self.publish(snapshot);

        // Fan out to WebSocket subscribers if any.
        if let Some(tx) = self.ws_tx.read().as_ref() {
            let _ = tx.send(snapshot);
        }

        Ok(snapshot)
    }

    /// Install a snapshot directly.
    ///
    /// The REST poller and the WebSocket reader are the only producers in
    /// production; a replay or a test needs to price against a known market
    /// without a network round trip.
    pub fn publish(&self, snapshot: TipFloor) {
        *self.snapshot.write() = Some(snapshot);
    }

    /// Most recent fresh snapshot, or `None` when absent or stale.
    pub fn current(&self, now_ms: u64) -> Option<TipFloor> {
        let guard = self.snapshot.read();
        let snapshot = (*guard)?;
        if snapshot.is_fresh(now_ms, self.max_age_ms) {
            Some(snapshot)
        } else {
            None
        }
    }

    /// Most recent snapshot regardless of age, for reporting only.
    pub fn last_seen(&self) -> Option<TipFloor> {
        *self.snapshot.read()
    }

    /// Subscribe to real-time tip updates via WebSocket broadcast channel.
    ///
    /// Call [`Self::run_ws`] separately to start the background reader that feeds
    /// this channel. Returns a receiver you can `.await` in the hot path.
    pub fn subscribe(&self) -> broadcast::Receiver<TipFloor> {
        let tx = self.ws_tx.read().clone().unwrap_or_else(|| {
            let (tx, _) = broadcast::channel(16);
            *self.ws_tx.write() = Some(tx.clone());
            tx
        });
        tx.subscribe()
    }

    /// Start the WebSocket reader in the background until the task is dropped.
    ///
    /// Every parsed tip floor is broadcast on [`Self::subscribe`] receivers and
    /// also written into the cached snapshot. A transient WebSocket failure only
    /// logs and retries; the REST poller in [`Self::run_rest`] keeps the cache
    /// fresh independently.
    pub async fn run_ws(self: Arc<Self>) {
        let ws_url = match Url::parse(&self.ws_url) {
            Ok(u) => u,
            Err(e) => {
                error!("invalid tip WebSocket URL {}: {e}", self.ws_url);
                return;
            }
        };

        loop {
            match connect_async(&ws_url).await {
                Ok((mut ws_stream, _)) => {
                    debug!("tip WebSocket connected: {}", self.ws_url);
                    // Iterate directly over the stream; no need to split.
                    while let Some(msg) = ws_stream.next().await {
                        match msg {
                            Ok(Message::Text(text)) => match serde_json::from_str::<Value>(&text) {
                                Ok(json) => {
                                    if let Ok(floor) = parse_tip_floor(&json, now_ms()) {
                                        self.publish(floor);
                                        if let Some(tx) = self.ws_tx.read().as_ref() {
                                            let _ = tx.send(floor);
                                        }
                                    }
                                }
                                Err(e) => warn!("tip WebSocket JSON parse failed: {e}"),
                            },
                            Ok(Message::Close(_)) => {
                                warn!("tip WebSocket closed by server");
                                break;
                            }
                            Ok(_) => {}
                            Err(e) => {
                                warn!("tip WebSocket read error: {e}");
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("tip WebSocket connect failed: {e}; retrying in 2s");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        }
    }

    /// Start both feeds: the WebSocket push reader and the REST poller.
    ///
    /// The socket reacts within milliseconds, while REST seeds the first
    /// snapshot and keeps the cache fresh whenever the socket is down. Running
    /// both means neither a cold start nor a dropped connection leaves pricing
    /// blind.
    pub async fn run(self: Arc<Self>, rest_interval: Duration) {
        let ws = Arc::clone(&self);
        tokio::spawn(async move { ws.run_ws().await });
        self.run_rest(rest_interval).await;
    }

    /// Poll the REST endpoint until the task is dropped, tolerating individual
    /// failures. A failed poll leaves the previous snapshot in place; `current`
    /// then ages it out on its own.
    pub async fn run_rest(self: Arc<Self>, interval: Duration) {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            match self.refresh().await {
                Ok(snapshot) => debug!(
                    p50 = snapshot.p50.0,
                    p95 = snapshot.p95.0,
                    "tip floor refreshed via REST"
                ),
                Err(e) => warn!("tip floor refresh failed: {e}"),
            }
        }
    }
}

/// Parse a tip-floor payload.
///
/// The documented shape is a one-element array of SOL-denominated floats; a bare
/// object is also accepted so a proxy rewrapping the body does not break pricing.
pub fn parse_tip_floor(body: &Value, observed_at_ms: u64) -> ArbResult<TipFloor> {
    let object = match body {
        Value::Array(items) => items.first(),
        other => Some(other),
    }
    .filter(|v| v.is_object())
    .ok_or_else(|| ArbError::DeserializationError("tip floor payload is empty".to_string()))?;

    let field = |name: &str| object.get(name).and_then(Value::as_f64);
    let p50 = field("landed_tips_50th_percentile")
        .ok_or_else(|| {
            ArbError::DeserializationError("tip floor payload missing 50th percentile".to_string())
        })
        .map(sol_to_lamports)?;

    // Every other rung degrades to the median rather than to zero: a missing
    // percentile must not make a rung look free.
    let or_median = |value: Option<f64>| value.map_or(p50, sol_to_lamports);

    Ok(TipFloor {
        observed_at_ms,
        p25: or_median(field("landed_tips_25th_percentile")),
        p50,
        p75: or_median(field("landed_tips_75th_percentile")),
        p95: or_median(field("landed_tips_95th_percentile")),
        p99: or_median(field("landed_tips_99th_percentile")),
        ema50: or_median(field("ema_landed_tips_50th_percentile")),
    })
}

/// Convert a SOL-denominated tip to lamports, saturating at the type bounds.
fn sol_to_lamports(sol: f64) -> Lamports {
    if !sol.is_finite() || sol <= 0.0 {
        return Lamports::ZERO;
    }
    let lamports = (sol * LAMPORTS_PER_SOL).round();
    let limit = u64::MAX as f64;
    if lamports >= limit {
        return Lamports(u64::MAX);
    }
    // Safe: finite, non-negative and strictly below `u64::MAX`.
    Lamports(lamports as u64)
}

/// Wall-clock milliseconds since the UNIX epoch.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn documented_payload() -> Value {
        json!([{
            "time": "2024-09-01T12:58:00Z",
            "landed_tips_25th_percentile": 6.001e-06,
            "landed_tips_50th_percentile": 1e-05,
            "landed_tips_75th_percentile": 3.6196500000000005e-05,
            "landed_tips_95th_percentile": 0.0014479055000000002,
            "landed_tips_99th_percentile": 0.010007999,
            "ema_landed_tips_50th_percentile": 9.836078125000002e-06
        }])
    }

    #[test]
    fn parses_documented_payload() {
        let floor = parse_tip_floor(&documented_payload(), 1_000).expect("parse");
        assert_eq!(floor.p50.0, 10_000);
        assert_eq!(floor.p25.0, 6_001);
        assert_eq!(floor.p99.0, 10_007_999);
        assert_eq!(floor.observed_at_ms, 1_000);
        assert!(floor.p25 <= floor.p50);
        assert!(floor.p50 <= floor.p75);
        assert!(floor.p75 <= floor.p95);
        assert!(floor.p95 <= floor.p99);
    }

    #[test]
    fn accepts_bare_object() {
        let mut body = documented_payload();
        let inner = body
            .as_array_mut()
            .and_then(|a| a.first_mut())
            .map(Value::take)
            .expect("array");
        let floor = parse_tip_floor(&inner, 0).expect("parse");
        assert_eq!(floor.p50.0, 10_000);
    }

    #[test]
    fn rejects_empty_payload() {
        assert!(parse_tip_floor(&json!([]), 0).is_err());
        assert!(parse_tip_floor(&json!({}), 0).is_err());
    }

    #[test]
    fn missing_rung_falls_back_to_median_not_zero() {
        let body = json!([{
            "landed_tips_50th_percentile": 5e-05
        }]);
        let floor = parse_tip_floor(&body, 0).expect("parse");
        assert_eq!(floor.p50.0, 50_000);
        assert_eq!(floor.p99.0, 50_000);
        assert_eq!(floor.p25.0, 50_000);
    }

    #[test]
    fn interpolates_unpublished_rungs() {
        let floor = TipFloor {
            observed_at_ms: 0,
            p25: Lamports(0),
            p50: Lamports(100),
            p75: Lamports(200),
            p95: Lamports(600),
            p99: Lamports(1_000),
            ema50: Lamports(100),
        };
        assert_eq!(floor.at(arb_types::Percentile::P50).0, 100);
        assert_eq!(floor.at(arb_types::Percentile::P65).0, 160);
        assert_eq!(floor.at(arb_types::Percentile::P75).0, 200);
        assert_eq!(floor.at(arb_types::Percentile::P85).0, 400);
        assert_eq!(floor.at(arb_types::Percentile::P90).0, 500);
        assert_eq!(floor.at(arb_types::Percentile::P95).0, 600);
        assert_eq!(floor.at(arb_types::Percentile::P99).0, 1_000);
    }

    #[test]
    fn staleness_is_a_hard_gate() {
        let floor = TipFloor {
            observed_at_ms: 1_000,
            p25: Lamports(1),
            p50: Lamports(1),
            p75: Lamports(1),
            p95: Lamports(1),
            p99: Lamports(1),
            ema50: Lamports(1),
        };
        assert!(floor.is_fresh(1_500, 500));
        assert!(!floor.is_fresh(1_501, 500));
        // Clock skew must not produce a negative age.
        assert_eq!(floor.age_ms(500), 0);
        assert!(floor.is_fresh(500, 0));
    }

    #[test]
    fn saturates_absurd_values() {
        assert_eq!(sol_to_lamports(f64::MAX).0, u64::MAX);
        assert_eq!(sol_to_lamports(-1.0).0, 0);
        assert_eq!(sol_to_lamports(f64::NAN).0, 0);
    }
}
