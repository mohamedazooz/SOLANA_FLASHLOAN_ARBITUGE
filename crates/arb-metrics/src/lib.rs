//!
//! No unwrap/panic. Hot paths use debug/trace, not INFO.

use std::net::SocketAddr;

use once_cell::sync::Lazy;
use prometheus::{
    Counter, Encoder, Gauge, Histogram, HistogramOpts, IntCounter, IntGauge, Registry, TextEncoder,
};
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
    pub const ESTIMATED_LANDED_NET_PROFIT_TOTAL: &str = "estimated_landed_net_profit_total";
    pub const TIP_SHARE_BPS: &str = "tip_share_bps";
    pub const BREAKER_STATE: &str = "breaker_state";
    pub const ATTEMPTS_TOTAL: &str = "attempts_total";
    pub const FILLS_TOTAL: &str = "fills_total";
    pub const PUMP_OPPORTUNITIES_TOTAL: &str = "pump_opportunities_total";
    pub const PUMP_SPREAD_BPS: &str = "pump_spread_bps";
    pub const GRADUATION_EVENTS_TOTAL: &str = "graduation_events_total";
    pub const WS_RECONNECTS_TOTAL: &str = "ws_reconnects_total";
    pub const FORK_INVALIDATIONS_TOTAL: &str = "fork_invalidations_total";
    pub const STREAM_SLOT_GAPS_TOTAL: &str = "stream_slot_gaps_total";
    pub const WS_SUBSCRIPTIONS_ACTIVE: &str = "ws_subscriptions_active";
    pub const SIMULATION_LATENCY_US: &str = "simulation_latency_us";
    pub const ASSEMBLY_LATENCY_US: &str = "assembly_latency_us";
    pub const SIGNING_LATENCY_US: &str = "signing_latency_us";
    pub const SUBMIT_RTT_MS: &str = "submit_rtt_ms";
    pub const LANDING_LATENCY_MS: &str = "landing_latency_ms";
    pub const E2E_LATENCY_MS: &str = "e2e_latency_ms";
    pub const STATE_PROCESSING_LATENCY_US: &str = "state_processing_latency_us";
    pub const ROUTE_DISCOVERY_LATENCY_US: &str = "route_discovery_latency_us";
    pub const EXACT_QUOTE_LATENCY_US: &str = "exact_quote_latency_us";
    pub const PROCESS_RSS_BYTES: &str = "process_rss_bytes";
    pub const PROCESS_CPU_PERCENT: &str = "process_cpu_percent";
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
static NET_PROFIT_TOTAL: Lazy<Gauge> = Lazy::new(|| {
    let gauge = Gauge::new(
        metric_names::NET_PROFIT_TOTAL,
        "Cumulative signed realized net profit in SOL",
    )
    .unwrap_or_else(|_| Gauge::new(metric_names::NET_PROFIT_TOTAL, "").expect("static metric"));
    let _ = REGISTRY.register(Box::new(gauge.clone()));
    gauge
});
static ESTIMATED_LANDED_NET_PROFIT_TOTAL: Lazy<Gauge> = Lazy::new(|| {
    let gauge = Gauge::new(
        metric_names::ESTIMATED_LANDED_NET_PROFIT_TOTAL,
        "Cumulative estimated net profit for landed transactions without balance metadata in SOL",
    )
    .unwrap_or_else(|_| {
        Gauge::new(metric_names::ESTIMATED_LANDED_NET_PROFIT_TOTAL, "").expect("static metric")
    });
    let _ = REGISTRY.register(Box::new(gauge.clone()));
    gauge
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

static WS_RECONNECTS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    let counter = IntCounter::new(
        metric_names::WS_RECONNECTS_TOTAL,
        "WebSocket reconnect attempts across all endpoints",
    )
    .unwrap_or_else(|_| {
        IntCounter::new(metric_names::WS_RECONNECTS_TOTAL, "").expect("static metric")
    });
    let _ = REGISTRY.register(Box::new(counter.clone()));
    counter
});

static WS_SUBSCRIPTIONS_ACTIVE: Lazy<IntGauge> = Lazy::new(|| {
    let gauge = IntGauge::new(
        metric_names::WS_SUBSCRIPTIONS_ACTIVE,
        "Confirmed live account subscriptions on the active WS endpoint",
    )
    .unwrap_or_else(|_| {
        IntGauge::new(metric_names::WS_SUBSCRIPTIONS_ACTIVE, "").expect("static metric")
    });
    let _ = REGISTRY.register(Box::new(gauge.clone()));
    gauge
});

static FORK_INVALIDATIONS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    let counter = IntCounter::new(
        metric_names::FORK_INVALIDATIONS_TOTAL,
        "State items invalidated by dead/uncleared fork banks",
    )
    .unwrap_or_else(|_| {
        IntCounter::new(metric_names::FORK_INVALIDATIONS_TOTAL, "").expect("static metric")
    });
    let _ = REGISTRY.register(Box::new(counter.clone()));
    counter
});

static SIMULATION_LATENCY_US: Lazy<Histogram> = Lazy::new(|| {
    let histogram = Histogram::with_opts(
        HistogramOpts::new(
            metric_names::SIMULATION_LATENCY_US,
            "Pre-flight RPC simulation duration in microseconds",
        )
        .buckets(vec![
            1_000.0,
            5_000.0,
            10_000.0,
            25_000.0,
            50_000.0,
            100_000.0,
            250_000.0,
            500_000.0,
            1_000_000.0,
            2_000_000.0,
            5_000_000.0,
        ]),
    )
    .expect("static metric");
    let _ = REGISTRY.register(Box::new(histogram.clone()));
    histogram
});

static ASSEMBLY_LATENCY_US: Lazy<Histogram> = Lazy::new(|| {
    let histogram = Histogram::with_opts(
        HistogramOpts::new(
            metric_names::ASSEMBLY_LATENCY_US,
            "Instruction building + tx compilation duration in microseconds",
        )
        .buckets(vec![
            1_000.0,
            5_000.0,
            10_000.0,
            25_000.0,
            50_000.0,
            100_000.0,
            250_000.0,
            500_000.0,
            1_000_000.0,
            2_000_000.0,
            5_000_000.0,
        ]),
    )
    .expect("static metric");
    let _ = REGISTRY.register(Box::new(histogram.clone()));
    histogram
});

static SIGNING_LATENCY_US: Lazy<Histogram> = Lazy::new(|| {
    let histogram = Histogram::with_opts(
        HistogramOpts::new(
            metric_names::SIGNING_LATENCY_US,
            "Ed25519 transaction signing duration in microseconds",
        )
        .buckets(vec![
            1_000.0,
            5_000.0,
            10_000.0,
            25_000.0,
            50_000.0,
            100_000.0,
            250_000.0,
            500_000.0,
            1_000_000.0,
            2_000_000.0,
            5_000_000.0,
        ]),
    )
    .expect("static metric");
    let _ = REGISTRY.register(Box::new(histogram.clone()));
    histogram
});

static SUBMIT_RTT_MS: Lazy<Histogram> = Lazy::new(|| {
    let histogram = Histogram::with_opts(
        HistogramOpts::new(
            metric_names::SUBMIT_RTT_MS,
            "Broadcast submission round-trip (local processing excluded) in milliseconds",
        )
        .buckets(vec![
            10.0, 25.0, 50.0, 100.0, 200.0, 400.0, 800.0, 1_600.0, 3_200.0, 6_400.0,
        ]),
    )
    .expect("static metric");
    let _ = REGISTRY.register(Box::new(histogram.clone()));
    histogram
});

static LANDING_LATENCY_MS: Lazy<Histogram> = Lazy::new(|| {
    let histogram = Histogram::with_opts(
        HistogramOpts::new(
            metric_names::LANDING_LATENCY_MS,
            "Dispatch-to-landing verification duration in milliseconds (includes leader wait)",
        )
        .buckets(vec![
            10.0, 25.0, 50.0, 100.0, 200.0, 400.0, 800.0, 1_600.0, 3_200.0, 6_400.0,
        ]),
    )
    .expect("static metric");
    let _ = REGISTRY.register(Box::new(histogram.clone()));
    histogram
});

static E2E_LATENCY_MS: Lazy<Histogram> = Lazy::new(|| {
    let histogram = Histogram::with_opts(
        HistogramOpts::new(
            metric_names::E2E_LATENCY_MS,
            "End-to-end opportunity lifecycle (ingest+quote+build+sign+broadcast+landing) in milliseconds",
        )
        .buckets(vec![
            10.0,
            25.0,
            50.0,
            100.0,
            200.0,
            400.0,
            800.0,
            1_600.0,
            3_200.0,
            6_400.0,
        ]),
    )
    .expect("static metric");
    let _ = REGISTRY.register(Box::new(histogram.clone()));
    histogram
});

static STATE_PROCESSING_LATENCY_US: Lazy<Histogram> = Lazy::new(|| {
    let histogram = Histogram::with_opts(
        HistogramOpts::new(
            metric_names::STATE_PROCESSING_LATENCY_US,
            "Strategy-cycle state-processing duration in microseconds (stream promotion + snapshot to pools ready)",
        )
        .buckets(vec![
            1_000.0,
            5_000.0,
            10_000.0,
            25_000.0,
            50_000.0,
            100_000.0,
            250_000.0,
            500_000.0,
            1_000_000.0,
            2_000_000.0,
            5_000_000.0,
        ]),
    )
    .expect("static metric");
    let _ = REGISTRY.register(Box::new(histogram.clone()));
    histogram
});

static ROUTE_DISCOVERY_LATENCY_US: Lazy<Histogram> = Lazy::new(|| {
    let histogram = Histogram::with_opts(
        HistogramOpts::new(
            metric_names::ROUTE_DISCOVERY_LATENCY_US,
            "Strategy-cycle candidate-route discovery duration in microseconds (graph classes + Bellman-Ford + multi-hop + 3-leg + boost scan)",
        )
        .buckets(vec![
            1_000.0,
            5_000.0,
            10_000.0,
            25_000.0,
            50_000.0,
            100_000.0,
            250_000.0,
            500_000.0,
            1_000_000.0,
            2_000_000.0,
            5_000_000.0,
        ]),
    )
    .expect("static metric");
    let _ = REGISTRY.register(Box::new(histogram.clone()));
    histogram
});

static EXACT_QUOTE_LATENCY_US: Lazy<Histogram> = Lazy::new(|| {
    let histogram = Histogram::with_opts(
        HistogramOpts::new(
            metric_names::EXACT_QUOTE_LATENCY_US,
            "Exact-quote engine invocation duration in microseconds (cross-pool size search, 3-leg chained quote pipeline)",
        )
        .buckets(vec![
            1_000.0,
            5_000.0,
            10_000.0,
            25_000.0,
            50_000.0,
            100_000.0,
            250_000.0,
            500_000.0,
            1_000_000.0,
            2_000_000.0,
            5_000_000.0,
        ]),
    )
    .expect("static metric");
    let _ = REGISTRY.register(Box::new(histogram.clone()));
    histogram
});

static PROCESS_RSS_BYTES: Lazy<IntGauge> = Lazy::new(|| {
    let gauge = IntGauge::new(
        metric_names::PROCESS_RSS_BYTES,
        "Resident set size in bytes",
    )
    .expect("static metric");
    let _ = REGISTRY.register(Box::new(gauge.clone()));
    gauge
});

static PROCESS_CPU_PERCENT: Lazy<Gauge> = Lazy::new(|| {
    let gauge = Gauge::new(
        metric_names::PROCESS_CPU_PERCENT,
        "Process CPU usage percent averaged over the sample interval",
    )
    .expect("static metric");
    let _ = REGISTRY.register(Box::new(gauge.clone()));
    gauge
});

static STREAM_SLOT_GAPS_TOTAL: Lazy<IntCounter> = Lazy::new(|| {
    let counter = IntCounter::new(
        metric_names::STREAM_SLOT_GAPS_TOTAL,
        "Slots never observed between consecutive stream slot notifications",
    )
    .unwrap_or_else(|_| {
        IntCounter::new(metric_names::STREAM_SLOT_GAPS_TOTAL, "").expect("static metric")
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
pub fn set_state_age(v: f64) {
    STATE_AGE_SLOTS.set(v);
}
pub fn add_net_profit(v: f64) {
    NET_PROFIT_TOTAL.add(v);
}
pub fn get_net_profit_total() -> f64 {
    NET_PROFIT_TOTAL.get()
}
pub fn add_estimated_landed_net_profit(v: f64) {
    ESTIMATED_LANDED_NET_PROFIT_TOTAL.add(v);
}
pub fn get_estimated_landed_net_profit_total() -> f64 {
    ESTIMATED_LANDED_NET_PROFIT_TOTAL.get()
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
pub fn inc_ws_reconnects() {
    WS_RECONNECTS_TOTAL.inc();
}
pub fn set_ws_subscriptions_active(v: u64) {
    WS_SUBSCRIPTIONS_ACTIVE.set(v as i64);
}

pub fn inc_fork_invalidations(n: u64) {
    FORK_INVALIDATIONS_TOTAL.inc_by(n);
}

pub fn inc_stream_slot_gaps(n: u64) {
    STREAM_SLOT_GAPS_TOTAL.inc_by(n);
}

pub fn observe_simulation_latency_us(us: f64) {
    SIMULATION_LATENCY_US.observe(us);
}

pub fn observe_assembly_latency_us(us: f64) {
    ASSEMBLY_LATENCY_US.observe(us);
}

pub fn observe_signing_latency_us(us: f64) {
    SIGNING_LATENCY_US.observe(us);
}

pub fn observe_submit_rtt_ms(ms: f64) {
    SUBMIT_RTT_MS.observe(ms);
}

pub fn observe_landing_latency_ms(ms: f64) {
    LANDING_LATENCY_MS.observe(ms);
}

pub fn observe_e2e_latency_ms(ms: f64) {
    E2E_LATENCY_MS.observe(ms);
}

pub fn observe_state_processing_latency_us(us: f64) {
    STATE_PROCESSING_LATENCY_US.observe(us);
}

pub fn observe_route_discovery_latency_us(us: f64) {
    ROUTE_DISCOVERY_LATENCY_US.observe(us);
}

pub fn observe_exact_quote_latency_us(us: f64) {
    EXACT_QUOTE_LATENCY_US.observe(us);
}

pub fn set_process_rss_bytes(v: u64) {
    PROCESS_RSS_BYTES.set(v as i64);
}

pub fn set_process_cpu_percent(v: f64) {
    PROCESS_CPU_PERCENT.set(v);
}

pub fn get_stale_rejections_total() -> u64 {
    STALE_REJECTIONS_TOTAL.get()
}

pub fn get_ws_reconnects_total() -> u64 {
    WS_RECONNECTS_TOTAL.get()
}

pub fn get_ws_subscriptions_active() -> u64 {
    WS_SUBSCRIPTIONS_ACTIVE.get().max(0) as u64
}

pub fn get_resyncs_total() -> u64 {
    RESYNCS_TOTAL.get()
}

pub fn get_decode_errors_total() -> u64 {
    DECODE_ERRORS_TOTAL.get()
}

pub fn get_fork_invalidations_total() -> u64 {
    FORK_INVALIDATIONS_TOTAL.get()
}

pub fn get_stream_slot_gaps_total() -> u64 {
    STREAM_SLOT_GAPS_TOTAL.get()
}

pub fn get_process_rss_bytes() -> u64 {
    PROCESS_RSS_BYTES.get().max(0) as u64
}

pub fn get_process_cpu_percent() -> f64 {
    PROCESS_CPU_PERCENT.get()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn latency_histograms_observe() {
        observe_assembly_latency_us(2_500.0);
        observe_signing_latency_us(300.0);
        observe_submit_rtt_ms(42.0);
        observe_landing_latency_ms(150.0);
        observe_e2e_latency_ms(220.0);
        observe_simulation_latency_us(9_000.0);
        let families = REGISTRY.gather();
        let metric_names_present: Vec<String> =
            families.iter().map(|f| f.name().to_string()).collect();
        for expected in [
            "assembly_latency_us",
            "signing_latency_us",
            "submit_rtt_ms",
            "landing_latency_ms",
            "e2e_latency_ms",
            "simulation_latency_us",
        ] {
            assert!(
                metric_names_present.iter().any(|n| n == expected),
                "metric family missing: {expected}"
            );
        }
    }

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
