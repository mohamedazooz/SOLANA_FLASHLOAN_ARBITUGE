//!
//! No unwrap/panic. Hot paths use debug/trace, not INFO.

use std::net::SocketAddr;

use once_cell::sync::Lazy;
use prometheus::{Counter, Encoder, Gauge, IntCounter, IntGauge, Registry, TextEncoder};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tracing::{debug, error};

/// Metric name constants as required.
pub mod metric_names {
    pub const STATE_AGE_SLOTS: &str = "state_age_slots";
    pub const ACCOUNT_UPDATE_LAG_MS: &str = "account_update_lag_ms";
    pub const RESYNCS_TOTAL: &str = "resyncs_total";
    pub const DECODE_ERRORS_TOTAL: &str = "decode_errors_total";
    pub const STALE_REJECTIONS_TOTAL: &str = "stale_rejections_total";
    pub const CACHE_CONSISTENCY: &str = "cache_consistency";
    pub const OPPORTUNITIES_TOTAL: &str = "opportunities_total";
    pub const NET_PROFIT_TOTAL: &str = "net_profit_total";
    pub const TIP_SHARE_BPS: &str = "tip_share_bps";
    pub const BREAKER_STATE: &str = "breaker_state";
    pub const ATTEMPTS_TOTAL: &str = "attempts_total";
    pub const FILLS_TOTAL: &str = "fills_total";
    pub const PUMP_OPPORTUNITIES_TOTAL: &str = "pump_opportunities_total";
    pub const PUMP_SPREAD_BPS: &str = "pump_spread_bps";
    pub const GRADUATION_EVENTS_TOTAL: &str = "graduation_events_total";
}

static REGISTRY: Lazy<Registry> = Lazy::new(Registry::new);
static ACCOUNT_UPDATE_LAG_MS: Lazy<Gauge> = Lazy::new(|| {
    let gauge = Gauge::new(
        metric_names::ACCOUNT_UPDATE_LAG_MS,
        "Account update lag in milliseconds",
    )
    .unwrap_or_else(|_| {
        Gauge::new(metric_names::ACCOUNT_UPDATE_LAG_MS, "").expect("static metric")
    });
    let _ = REGISTRY.register(Box::new(gauge.clone()));
    gauge
});
static RESYNCS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    let counter = IntCounter::new(metric_names::RESYNCS_TOTAL, "Total resyncs after reconnect")
        .unwrap_or_else(|_| {
            IntCounter::new(metric_names::RESYNCS_TOTAL, "").expect("static metric")
        });
    let _ = REGISTRY.register(Box::new(counter.clone()));
    counter
});
static DECODE_ERRORS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    let counter = IntCounter::new(
        metric_names::DECODE_ERRORS_TOTAL,
        "Total account decode errors",
    )
    .unwrap_or_else(|_| {
        IntCounter::new(metric_names::DECODE_ERRORS_TOTAL, "").expect("static metric")
    });
    let _ = REGISTRY.register(Box::new(counter.clone()));
    counter
});
static STALE_REJECTIONS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    let counter = IntCounter::new(
        metric_names::STALE_REJECTIONS_TOTAL,
        "Total stale account update rejections",
    )
    .unwrap_or_else(|_| {
        IntCounter::new(metric_names::STALE_REJECTIONS_TOTAL, "").expect("static metric")
    });
    let _ = REGISTRY.register(Box::new(counter.clone()));
    counter
});
static CACHE_CONSISTENCY: Lazy<Gauge> = Lazy::new(|| {
    let gauge = Gauge::new(
        metric_names::CACHE_CONSISTENCY,
        "Cache consistency check result (1=consistent, 0=divergent)",
    )
    .unwrap_or_else(|_| Gauge::new(metric_names::CACHE_CONSISTENCY, "").expect("static metric"));
    let _ = REGISTRY.register(Box::new(gauge.clone()));
    gauge
});
static STATE_AGE_SLOTS: Lazy<Gauge> = Lazy::new(|| {
    let gauge = Gauge::new(metric_names::STATE_AGE_SLOTS, "State age in slots")
        .unwrap_or_else(|_| Gauge::new(metric_names::STATE_AGE_SLOTS, "").expect("static metric"));
    let _ = REGISTRY.register(Box::new(gauge.clone()));
    gauge
});
static OPPORTUNITIES_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    let counter = IntCounter::new(
        metric_names::OPPORTUNITIES_TOTAL,
        "Total opportunities found",
    )
    .unwrap_or_else(|_| {
        IntCounter::new(metric_names::OPPORTUNITIES_TOTAL, "").expect("static metric")
    });
    let _ = REGISTRY.register(Box::new(counter.clone()));
    counter
});
static NET_PROFIT_TOTAL: Lazy<Counter> = Lazy::new(|| {
    let counter =
        Counter::new(metric_names::NET_PROFIT_TOTAL, "Net profit total").unwrap_or_else(|_| {
            Counter::new(metric_names::NET_PROFIT_TOTAL, "").expect("static metric")
        });
    let _ = REGISTRY.register(Box::new(counter.clone()));
    counter
});
static SIMULATED_NET_PROFIT_TOTAL: Lazy<Counter> = Lazy::new(|| {
    let counter = Counter::new("simulated_net_profit_total", "Simulated net profit total")
        .unwrap_or_else(|_| Counter::new("simulated_net_profit_total", "").expect("static metric"));
    let _ = REGISTRY.register(Box::new(counter.clone()));
    counter
});
static TIP_SHARE_BPS: Lazy<Gauge> = Lazy::new(|| {
    let gauge = Gauge::new(metric_names::TIP_SHARE_BPS, "Tip share in bps")
        .unwrap_or_else(|_| Gauge::new(metric_names::TIP_SHARE_BPS, "").expect("static metric"));
    let _ = REGISTRY.register(Box::new(gauge.clone()));
    gauge
});
static BREAKER_STATE: Lazy<IntGauge> = Lazy::new(|| {
    let gauge = IntGauge::new(
        metric_names::BREAKER_STATE,
        "Circuit breaker state (0=closed,1=open,2=half-open)",
    )
    .unwrap_or_else(|_| IntGauge::new(metric_names::BREAKER_STATE, "").expect("static metric"));
    let _ = REGISTRY.register(Box::new(gauge.clone()));
    gauge
});
static ATTEMPTS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    let counter =
        IntCounter::new(metric_names::ATTEMPTS_TOTAL, "Total attempts").unwrap_or_else(|_| {
            IntCounter::new(metric_names::ATTEMPTS_TOTAL, "").expect("static metric")
        });
    let _ = REGISTRY.register(Box::new(counter.clone()));
    counter
});
static FILLS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    let counter = IntCounter::new(metric_names::FILLS_TOTAL, "Total fills")
        .unwrap_or_else(|_| IntCounter::new(metric_names::FILLS_TOTAL, "").expect("static metric"));
    let _ = REGISTRY.register(Box::new(counter.clone()));
    counter
});
static PUMP_OPPORTUNITIES_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    let counter = IntCounter::new(
        metric_names::PUMP_OPPORTUNITIES_TOTAL,
        "Total Pump.fun bonding curve opportunities",
    )
    .unwrap_or_else(|_| {
        IntCounter::new(metric_names::PUMP_OPPORTUNITIES_TOTAL, "").expect("static metric")
    });
    let _ = REGISTRY.register(Box::new(counter.clone()));
    counter
});
static PUMP_SPREAD_BPS: Lazy<Gauge> = Lazy::new(|| {
    let gauge = Gauge::new(
        metric_names::PUMP_SPREAD_BPS,
        "Latest Pump.fun opportunity spread in bps",
    )
    .unwrap_or_else(|_| Gauge::new(metric_names::PUMP_SPREAD_BPS, "").expect("static metric"));
    let _ = REGISTRY.register(Box::new(gauge.clone()));
    gauge
});
static GRADUATION_EVENTS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    let counter = IntCounter::new(
        metric_names::GRADUATION_EVENTS_TOTAL,
        "Total Pump.fun token graduation/migration events detected",
    )
    .unwrap_or_else(|_| {
        IntCounter::new(metric_names::GRADUATION_EVENTS_TOTAL, "").expect("static metric")
    });
    let _ = REGISTRY.register(Box::new(counter.clone()));
    counter
});

#[derive(Debug)]
pub struct MetricsHandle {
    handle: JoinHandle<()>,
}

impl MetricsHandle {
    pub async fn shutdown(self) {
        self.handle.abort();
        let _ = self.handle.await;
    }
}

pub struct MetricsServer {
    addr: SocketAddr,
}

impl MetricsServer {
    pub fn new(listen: &str) -> Self {
        let addr: SocketAddr = listen
            .parse()
            .unwrap_or_else(|_| "127.0.0.1:9090".parse().expect("static metric"));
        Self { addr }
    }

    pub fn start(self) -> MetricsHandle {
        let handle = tokio::spawn(async move {
            let listener = match TcpListener::bind(self.addr).await {
                Ok(l) => l,
                Err(e) => {
                    error!("metrics bind failed: {}", e);
                    return;
                }
            };
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let mut buf = [0u8; 1024];
                let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await;
                let encoder = TextEncoder::new();
                let metric_families = REGISTRY.gather();
                let mut buffer = Vec::new();
                if let Err(e) = encoder.encode(&metric_families, &mut buffer) {
                    debug!("encode metrics err: {}", e);
                    continue;
                }
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain; version=0.0.4\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    buffer.len(),
                    String::from_utf8_lossy(&buffer)
                );
                let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, response.as_bytes()).await;
            }
        });
        MetricsHandle { handle }
    }
}

pub fn inc_opportunities() {
    OPPORTUNITIES_TOTAL.inc();
}
pub fn get_opportunities_total() -> u64 {
    OPPORTUNITIES_TOTAL.get()
}
pub fn inc_attempts() {
    ATTEMPTS_TOTAL.inc();
}
pub fn inc_fills() {
    FILLS_TOTAL.inc();
}
pub fn get_fills_total() -> u64 {
    FILLS_TOTAL.get()
}
pub fn set_account_update_lag_ms(v: f64) {
    ACCOUNT_UPDATE_LAG_MS.set(v);
}
pub fn inc_resyncs() {
    RESYNCS_TOTAL.inc();
}
pub fn inc_decode_errors() {
    DECODE_ERRORS_TOTAL.inc();
}
pub fn inc_stale_rejections() {
    STALE_REJECTIONS_TOTAL.inc();
}
pub fn set_cache_consistency(consistent: bool) {
    CACHE_CONSISTENCY.set(if consistent { 1.0 } else { 0.0 });
}
pub fn get_cache_consistency() -> bool {
    CACHE_CONSISTENCY.get() >= 0.5
}
pub fn set_state_age(v: f64) {
    STATE_AGE_SLOTS.set(v);
}
pub fn add_net_profit(v: f64) {
    NET_PROFIT_TOTAL.inc_by(v);
}
pub fn get_net_profit_total() -> f64 {
    NET_PROFIT_TOTAL.get()
}
pub fn add_simulated_net_profit(v: f64) {
    SIMULATED_NET_PROFIT_TOTAL.inc_by(v);
}
pub fn get_simulated_net_profit_total() -> f64 {
    SIMULATED_NET_PROFIT_TOTAL.get()
}
pub fn set_tip_share_bps(v: f64) {
    TIP_SHARE_BPS.set(v);
}
pub fn set_breaker_state(v: i64) {
    BREAKER_STATE.set(v);
}
pub fn inc_pump_opportunities() {
    PUMP_OPPORTUNITIES_TOTAL.inc();
}
pub fn set_pump_spread_bps(bps: f64) {
    PUMP_SPREAD_BPS.set(bps);
}
pub fn inc_graduation_events() {
    GRADUATION_EVENTS_TOTAL.inc();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metrics_register() {
        inc_opportunities();
        inc_attempts();
        inc_pump_opportunities();
        set_pump_spread_bps(250.0);
        inc_graduation_events();
        assert!(!REGISTRY.gather().is_empty());
    }
}
