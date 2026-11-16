//! Unified control service — the single command layer shared by the web API
//! and Telegram. Every mutating action goes through here so both interfaces
//! have identical authorization, validation, audit logging, and event
//! emission. No interface may mutate bot state directly.

use crate::BotMode;
use arb_risk::KillSwitch;
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Hard cap on retained opportunity records (bounded memory).
const OPPORTUNITY_RING_CAP: usize = 256;
/// Hard cap on retained event-log records (bounded memory).
const LOG_RING_CAP: usize = 1_000;
/// Hard cap on retained execution records (bounded memory).
const EXECUTION_RING_CAP: usize = 256;

#[derive(Debug, Clone, Serialize)]
pub struct OpportunityRecord {
    pub captured_at_unix: u64,
    pub slot: u64,
    pub route: String,
    pub protocol: String,
    pub borrow_lamports: u64,
    pub expected_output_lamports: u64,
    pub net_profit_lamports: i64,
    pub eligible: bool,
    pub reject_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogRecord {
    pub at_unix: u64,
    pub level: String,
    pub stage: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExecutionRecord {
    pub at_unix: u64,
    pub outcome: String,
    pub route: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ControlEvent {
    pub kind: String,
    pub at_unix: u64,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    Paper,
    Live,
}

/// Status snapshot returned by `status()` — the single shape consumed by both
/// the web dashboard and the Telegram `/status` command.
#[derive(Debug, Clone, Serialize)]
pub struct StatusSnapshot {
    pub mode: String,
    pub execution_enabled: bool,
    pub paused: bool,
    pub emergency_stopped: bool,
    pub slot: u64,
    pub pools_tracked: usize,
    pub stream_connected: bool,
    pub helius_connected: bool,
    pub opportunities_total: u64,
    pub profitable_total: u64,
    pub executed_total: u64,
    pub successful_total: u64,
    pub failed_total: u64,
    pub net_pnl_sol: f64,
    pub started_at_unix: u64,
    pub uptime_secs: u64,
    pub route_coverage: Vec<ProtocolCoverage>,
}

struct ControlInner {
    supervisor_mode: Arc<std::sync::atomic::AtomicU8>,
    kill_switch: Arc<KillSwitch>,
    execution_enabled: AtomicBool,
    emergency_stopped: AtomicBool,
    events: tokio::sync::broadcast::Sender<ControlEvent>,
    opportunities: Mutex<VecDeque<OpportunityRecord>>,
    executions: Mutex<VecDeque<ExecutionRecord>>,
    logs: Mutex<VecDeque<LogRecord>>,
    profitable_total: AtomicU64,
    executed_total: AtomicU64,
    successful_total: AtomicU64,
    failed_total: AtomicU64,
    started_at: Instant,
    audit_path: String,
    /// Live state providers, injected from the supervisor at construction.
    state: Mutex<ControlDeps>,
    overrides: Mutex<ConfigOverrides>,
}

/// Per-protocol route coverage published by the supervisor for the unified
/// route monitor (web section J / Telegram companion). Fees are the registry's
/// per-pool swap fee in basis points, averaged over that protocol's pools.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ProtocolCoverage {
    pub protocol: String,
    pub pools: usize,
    pub avg_fee_bps: f64,
}

/// Read-only view of supervisor state, refreshed by the supervisor loop.
/// Stored as data (not closures) so the control service never locks the
/// supervisor itself.
#[derive(Default, Clone)]
pub struct ControlDeps {
    pub slot: u64,
    pub pools_tracked: usize,
    pub stream_connected: bool,
    pub helius_connected: bool,
    pub route_coverage: Vec<ProtocolCoverage>,
}

#[derive(Debug, Clone)]
pub struct ControlError(pub String);

impl std::fmt::Display for ControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ControlError {}

pub type ControlResult<T> = Result<T, ControlError>;

/// Runtime strategy overrides set through the unified control service (web
/// section E / Telegram `/config`). Every field is optional; `None` keeps the
/// TOML value. Updates are validated at the control boundary and audited.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct ConfigOverrides {
    pub min_net_profit_usd: Option<f64>,
    pub min_borrow_lamports: Option<u64>,
    pub max_borrow_lamports: Option<u64>,
    pub max_slippage_bps: Option<u64>,
}

impl ConfigOverrides {
    /// Validates a partial update; returns the merged overrides.
    pub fn merged_with(&self, update: &ConfigOverrides) -> ControlResult<ConfigOverrides> {
        if let Some(usd) = update.min_net_profit_usd {
            if !usd.is_finite() || usd < 0.0 {
                return Err(ControlError(
                    "min_net_profit_usd must be a finite value >= 0".into(),
                ));
            }
        }
        let mut merged = self.clone();
        if update.min_net_profit_usd.is_some() {
            merged.min_net_profit_usd = update.min_net_profit_usd;
        }
        if update.min_borrow_lamports.is_some() {
            merged.min_borrow_lamports = update.min_borrow_lamports;
        }
        if update.max_borrow_lamports.is_some() {
            merged.max_borrow_lamports = update.max_borrow_lamports;
        }
        if update.max_slippage_bps.is_some() {
            merged.max_slippage_bps = update.max_slippage_bps;
        }
        if let (Some(min), Some(max)) = (merged.min_borrow_lamports, merged.max_borrow_lamports) {
            if min > max {
                return Err(ControlError(
                    "min_borrow_lamports must be <= max_borrow_lamports".into(),
                ));
            }
        }
        if let Some(bps) = merged.max_slippage_bps {
            if bps == 0 || bps > 10_000 {
                return Err(ControlError("max_slippage_bps must be in 1..=10000".into()));
            }
        }
        Ok(merged)
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub struct ControlService {
    inner: Arc<ControlInner>,
}

impl Clone for ControlService {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl ControlService {
    /// `supervisor_mode` is the supervisor's shared mode atomic; `kill_switch`
    /// is the same instance the run loop checks every iteration.
    pub fn new(
        supervisor_mode: Arc<std::sync::atomic::AtomicU8>,
        kill_switch: Arc<KillSwitch>,
        audit_path: impl Into<String>,
    ) -> Self {
        let (events, _) = tokio::sync::broadcast::channel(512);
        Self {
            inner: Arc::new(ControlInner {
                supervisor_mode,
                kill_switch,
                execution_enabled: AtomicBool::new(true),
                emergency_stopped: AtomicBool::new(false),
                events,
                opportunities: Mutex::new(VecDeque::with_capacity(OPPORTUNITY_RING_CAP)),
                executions: Mutex::new(VecDeque::with_capacity(EXECUTION_RING_CAP)),
                logs: Mutex::new(VecDeque::with_capacity(LOG_RING_CAP)),
                profitable_total: AtomicU64::new(0),
                executed_total: AtomicU64::new(0),
                successful_total: AtomicU64::new(0),
                failed_total: AtomicU64::new(0),
                started_at: Instant::now(),
                audit_path: audit_path.into(),
                state: Mutex::new(ControlDeps::default()),
                overrides: Mutex::new(ConfigOverrides::default()),
            }),
        }
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<ControlEvent> {
        self.inner.events.subscribe()
    }

    pub fn refresh_deps(&self, deps: ControlDeps) {
        *self.inner.state.lock() = deps;
    }

    fn emit(&self, kind: &str, payload: serde_json::Value) {
        let event = ControlEvent {
            kind: kind.to_string(),
            at_unix: now_unix(),
            payload,
        };
        let _ = self.inner.events.send(event);
    }

    fn audit(&self, action: &str, detail: &str) {
        let entry = serde_json::json!({
            "at_unix": now_unix(),
            "action": action,
            "detail": detail,
        });
        let line = entry.to_string();
        // Audit writes never block control decisions on failure: the append is
        // best-effort but every attempt is logged. sync_data() flushes the
        // entry to disk so a crash cannot silently drop the audit trail for
        // safety-relevant actions (emergency stop, live confirmation).
        let result = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.inner.audit_path)
            .and_then(|mut f| {
                std::io::Write::write_all(&mut f, format!("{line}\n").as_bytes())
                    .and_then(|()| f.sync_data())
            });
        if let Err(err) = result {
            tracing::warn!("audit log append failed for {action}: {err}");
        }
    }

    // ---- read commands -------------------------------------------------

    pub fn status(&self) -> StatusSnapshot {
        let deps = self.inner.state.lock().clone();
        let mode = self.current_mode();
        let net_pnl_sol = if mode == BotMode::Live {
            arb_metrics::get_net_profit_total()
        } else {
            arb_metrics::get_simulated_net_profit_total()
        };
        StatusSnapshot {
            mode: mode.to_string(),
            execution_enabled: self.inner.execution_enabled.load(Ordering::Acquire),
            paused: self.inner.kill_switch.is_blocked(),
            emergency_stopped: self.inner.emergency_stopped.load(Ordering::Acquire),
            slot: deps.slot,
            pools_tracked: deps.pools_tracked,
            stream_connected: deps.stream_connected,
            helius_connected: deps.helius_connected,
            opportunities_total: arb_metrics::get_opportunities_total(),
            profitable_total: self.inner.profitable_total.load(Ordering::Relaxed),
            executed_total: self.inner.executed_total.load(Ordering::Relaxed),
            successful_total: self.inner.successful_total.load(Ordering::Relaxed),
            failed_total: self.inner.failed_total.load(Ordering::Relaxed),
            net_pnl_sol,
            started_at_unix: now_unix().saturating_sub(self.inner.started_at.elapsed().as_secs()),
            uptime_secs: self.inner.started_at.elapsed().as_secs(),
            route_coverage: deps.route_coverage,
        }
    }

    /// Per-protocol route coverage for the dedicated route monitor endpoint.
    pub fn route_coverage(&self) -> Vec<ProtocolCoverage> {
        self.inner.state.lock().route_coverage.clone()
    }

    pub fn opportunities(&self, limit: usize) -> Vec<OpportunityRecord> {
        let ring = self.inner.opportunities.lock();
        ring.iter()
            .rev()
            .take(limit.clamp(1, OPPORTUNITY_RING_CAP))
            .cloned()
            .collect()
    }

    pub fn logs(&self, limit: usize) -> Vec<LogRecord> {
        let ring = self.inner.logs.lock();
        ring.iter()
            .rev()
            .take(limit.clamp(1, LOG_RING_CAP))
            .cloned()
            .collect()
    }

    pub fn pnl(&self) -> serde_json::Value {
        let status = self.status();
        serde_json::json!({
            "mode": status.mode,
            "net_pnl_sol": status.net_pnl_sol,
            "executed_total": status.executed_total,
            "successful_total": status.successful_total,
            "failed_total": status.failed_total,
        })
    }

    pub fn providers(&self) -> serde_json::Value {
        let deps = self.inner.state.lock().clone();
        serde_json::json!({
            "primary_stream": { "connected": deps.stream_connected, "slot": deps.slot },
            "helius": { "connected": deps.helius_connected, "slot": deps.slot },
        })
    }

    pub fn config_summary(&self) -> serde_json::Value {
        let status = self.status();
        let overrides = self.inner.overrides.lock().clone();
        serde_json::json!({
            "mode": status.mode,
            "min_net_profit_usd_env": std::env::var("MIN_NET_PROFIT_USD").ok(),
            "arb_live": std::env::var("ARB_LIVE").ok(),
            "execution_enabled": status.execution_enabled,
            "overrides": overrides,
        })
    }

    /// Validated runtime strategy update; audited and broadcast.
    pub fn update_config(&self, update: &ConfigOverrides) -> ControlResult<ConfigOverrides> {
        let merged = self.inner.overrides.lock().merged_with(update)?;
        *self.inner.overrides.lock() = merged.clone();
        self.audit(
            "update_config",
            &serde_json::to_string(&merged).unwrap_or_default(),
        );
        self.emit("config", serde_json::to_value(&merged).unwrap_or_default());
        Ok(merged)
    }

    pub fn overrides(&self) -> ConfigOverrides {
        self.inner.overrides.lock().clone()
    }

    /// Audit entry for actions taken by attached interfaces (web/Telegram).
    pub fn audit_action(&self, action: &str, detail: &str) {
        self.audit(action, detail);
    }

    /// Live emergency-latch state, consumed by the dispatch fire-time gate so
    /// an emergency stop blocks in-flight submissions, not just new captures.
    pub fn is_emergency_stopped(&self) -> bool {
        self.inner.emergency_stopped.load(Ordering::Acquire)
    }

    pub fn current_mode(&self) -> BotMode {
        BotMode::from_u8(self.inner.supervisor_mode.load(Ordering::Acquire))
    }

    // ---- telemetry ingestion (called by the supervisor loop) ------------

    pub fn record_opportunity(&self, mut record: OpportunityRecord) {
        record.captured_at_unix = now_unix();
        if record.eligible {
            self.inner.profitable_total.fetch_add(1, Ordering::Relaxed);
        }
        let mut ring = self.inner.opportunities.lock();
        if ring.len() == OPPORTUNITY_RING_CAP {
            ring.pop_front();
        }
        ring.push_back(record.clone());
        drop(ring);
        self.emit(
            "opportunity",
            serde_json::to_value(&record).unwrap_or_default(),
        );
    }

    pub fn record_log(&self, level: &str, stage: &str, message: impl Into<String>) {
        let record = LogRecord {
            at_unix: now_unix(),
            level: level.to_string(),
            stage: stage.to_string(),
            message: message.into(),
        };
        let mut ring = self.inner.logs.lock();
        if ring.len() == LOG_RING_CAP {
            ring.pop_front();
        }
        ring.push_back(record.clone());
        drop(ring);
        self.emit("log", serde_json::to_value(&record).unwrap_or_default());
    }

    pub fn record_execution_result(&self, success: bool) {
        self.inner.executed_total.fetch_add(1, Ordering::Relaxed);
        if success {
            self.inner.successful_total.fetch_add(1, Ordering::Relaxed);
        } else {
            self.inner.failed_total.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn record_execution(&self, outcome: &str, route: &str, detail: &str) {
        let record = ExecutionRecord {
            at_unix: now_unix(),
            outcome: outcome.to_string(),
            route: route.to_string(),
            detail: detail.to_string(),
        };
        let mut ring = self.inner.executions.lock();
        if ring.len() == EXECUTION_RING_CAP {
            ring.pop_front();
        }
        ring.push_back(record.clone());
        drop(ring);
        self.emit(
            "execution",
            serde_json::to_value(&record).unwrap_or_default(),
        );
    }

    pub fn executions(&self, limit: usize) -> Vec<ExecutionRecord> {
        let ring = self.inner.executions.lock();
        ring.iter()
            .rev()
            .take(limit.clamp(1, EXECUTION_RING_CAP))
            .cloned()
            .collect()
    }

    // ---- mutating commands (shared by web + Telegram) -------------------

    pub fn start(&self) -> ControlResult<&'static str> {
        if self.inner.emergency_stopped.load(Ordering::Acquire) {
            return Err(ControlError(
                "emergency stop is active — clear it explicitly before starting".into(),
            ));
        }
        self.inner.kill_switch.deactivate();
        self.inner.execution_enabled.store(true, Ordering::Release);
        self.audit("start", "execution enabled, kill switch deactivated");
        self.emit("control", serde_json::json!({"action": "start"}));
        Ok("execution enabled")
    }

    pub fn stop(&self) -> ControlResult<&'static str> {
        self.inner.kill_switch.activate();
        self.inner.execution_enabled.store(false, Ordering::Release);
        self.audit("stop", "execution paused via kill switch");
        self.emit("control", serde_json::json!({"action": "stop"}));
        Ok("execution paused")
    }

    pub fn emergency_stop(&self) -> ControlResult<&'static str> {
        self.inner.kill_switch.activate();
        self.inner.emergency_stopped.store(true, Ordering::Release);
        self.inner.execution_enabled.store(false, Ordering::Release);
        self.audit(
            "emergency_stop",
            "latch engaged; start blocked until cleared",
        );
        self.emit("control", serde_json::json!({"action": "emergency_stop"}));
        Ok("emergency stop engaged")
    }

    pub fn clear_emergency_stop(&self) -> ControlResult<&'static str> {
        self.inner.emergency_stopped.store(false, Ordering::Release);
        self.audit(
            "clear_emergency_stop",
            "latch cleared; execution still paused",
        );
        self.emit(
            "control",
            serde_json::json!({"action": "clear_emergency_stop"}),
        );
        Ok("emergency latch cleared — execution remains paused until start")
    }

    pub fn set_mode(
        &self,
        requested: ExecutionMode,
        confirm: Option<&str>,
    ) -> ControlResult<String> {
        if self.inner.emergency_stopped.load(Ordering::Acquire) {
            return Err(ControlError(
                "emergency stop is active — clear it before changing mode".into(),
            ));
        }
        match requested {
            ExecutionMode::Live => {
                let live_allowed = crate::live_execution_enabled();
                let confirmed = matches!(confirm, Some("CONFIRM-LIVE"));
                if !live_allowed {
                    self.audit("mode_live_rejected", "ARB_LIVE not explicitly truthy");
                    return Err(ControlError(
                        "live mode requires ARB_LIVE=1 in the bot environment".into(),
                    ));
                }
                if !confirmed {
                    self.audit("mode_live_rejected", "missing CONFIRM-LIVE token");
                    return Err(ControlError(
                        "live mode requires the explicit confirmation token CONFIRM-LIVE".into(),
                    ));
                }
            }
            ExecutionMode::Paper => {}
        }
        let bot_mode = match requested {
            ExecutionMode::Paper => BotMode::Simulate,
            ExecutionMode::Live => BotMode::Live,
        };
        self.inner
            .supervisor_mode
            .store(bot_mode as u8, Ordering::Release);
        let detail = format!("mode -> {requested:?}");
        self.audit("set_mode", &detail);
        self.emit(
            "control",
            serde_json::json!({"action": "set_mode", "mode": bot_mode.to_string()}),
        );
        Ok(detail)
    }

    /// Convenience combined command used by the Telegram `/opportunities`
    /// handler: the latest eligible opportunities formatted as compact lines.
    pub fn opportunities_digest(&self, limit: usize) -> String {
        let opps = self.opportunities(limit);
        if opps.is_empty() {
            return "لا توجد فرص مسجلة بعد.".into();
        }
        opps.iter()
            .map(|o| {
                format!(
                    "{} | {} | net {:.6} SOL | {}",
                    humantime_unix(o.captured_at_unix),
                    o.route,
                    o.net_profit_lamports as f64 / 1e9,
                    if o.eligible { "✅" } else { "⛔" }
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn humantime_unix(unix: u64) -> String {
    // Compact UTC HH:MM:SS without pulling a chrono dependency.
    let secs_of_day = unix % 86_400;
    format!(
        "{:02}:{:02}:{:02}Z",
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60
    )
}

pub const CONTROL_COMMANDS: &[&str] = &[
    "status",
    "start",
    "stop",
    "paper",
    "live",
    "opportunities",
    "pnl",
    "providers",
    "config",
];

/// Executes a unified command string (same grammar for web and Telegram).
pub fn execute_command(
    service: &ControlService,
    command: &str,
    arg: Option<&str>,
) -> ControlResult<String> {
    let command = command.trim().trim_start_matches('/').to_ascii_lowercase();
    match command.as_str() {
        "status" => {
            let s = service.status();
            Ok(format!(
                "mode={} paused={} emergency={} slot={} pools={} pnl={:.6} SOL",
                s.mode, s.paused, s.emergency_stopped, s.slot, s.pools_tracked, s.net_pnl_sol
            ))
        }
        "start" => service.start().map(str::to_string),
        "stop" => service.stop().map(str::to_string),
        "paper" => service
            .set_mode(ExecutionMode::Paper, arg)
            .map(|_| "mode -> paper".to_string()),
        "live" => service
            .set_mode(ExecutionMode::Live, arg)
            .map(|_| "mode -> live".to_string()),
        "opportunities" => Ok(service.opportunities_digest(10)),
        "pnl" => Ok(service.pnl().to_string()),
        "providers" => Ok(service.providers().to_string()),
        "config" => Ok(service.config_summary().to_string()),
        "emergency-stop" | "emergency_stop" => service.emergency_stop().map(str::to_string),
        "clear-emergency-stop" | "clear_emergency_stop" => {
            service.clear_emergency_stop().map(str::to_string)
        }
        other => Err(ControlError(format!(
            "unknown command '{other}' — available: {}",
            CONTROL_COMMANDS.join(", ")
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service() -> ControlService {
        let mode = Arc::new(std::sync::atomic::AtomicU8::new(0));
        ControlService::new(mode, Arc::new(KillSwitch::new()), "/tmp/arb_audit_test.log")
    }

    #[test]
    fn start_stop_roundtrip_and_emergency_latch() {
        let svc = service();
        assert!(svc.start().is_ok());
        assert!(svc.status().execution_enabled);
        assert!(svc.stop().is_ok());
        assert!(!svc.status().execution_enabled);
        assert!(svc.status().paused);
        assert!(svc.start().is_ok());

        assert!(svc.emergency_stop().is_ok());
        assert!(svc.status().emergency_stopped);
        assert!(svc.start().is_err());
        assert!(
            svc.set_mode(ExecutionMode::Live, Some("CONFIRM-LIVE"))
                .is_err()
        );
        assert!(svc.clear_emergency_stop().is_ok());
        assert!(!svc.status().emergency_stopped);
        assert!(
            svc.status().paused,
            "start must remain explicit after the latch clears"
        );
    }

    #[test]
    fn live_mode_requires_arb_live_and_confirmation() {
        let svc = service();
        // Sandbox env has ARB_LIVE unset/0 — live must be refused twice.
        if !crate::live_execution_enabled() {
            assert!(
                svc.set_mode(ExecutionMode::Live, Some("CONFIRM-LIVE"))
                    .is_err()
            );
            assert!(svc.set_mode(ExecutionMode::Live, None).is_err());
        }
        assert!(svc.set_mode(ExecutionMode::Paper, None).is_ok());
        assert_eq!(svc.current_mode(), BotMode::Simulate);
    }

    #[test]
    fn unified_commands_route_to_the_same_service() {
        let svc = service();
        assert!(execute_command(&svc, "/status", None).is_ok());
        assert!(execute_command(&svc, "stop", None).is_ok());
        assert!(svc.status().paused);
        assert!(execute_command(&svc, "/paper", None).is_ok());
        assert_eq!(svc.current_mode(), BotMode::Simulate);
        assert!(execute_command(&svc, "opportunities", None).is_ok());
        assert!(execute_command(&svc, "definitely-not-a-command", None).is_err());
    }

    #[test]
    fn opportunity_ring_is_bounded_and_returns_newest_first() {
        let svc = service();
        for i in 0..(OPPORTUNITY_RING_CAP + 10) {
            svc.record_opportunity(OpportunityRecord {
                captured_at_unix: 0,
                slot: i as u64,
                route: format!("route-{i}"),
                protocol: "dlmm".into(),
                borrow_lamports: 1,
                expected_output_lamports: 2,
                net_profit_lamports: 3,
                eligible: true,
                reject_reason: None,
            });
        }
        let latest = svc.opportunities(5);
        assert_eq!(latest.len(), 5);
        assert_eq!(
            latest[0].route,
            format!("route-{}", OPPORTUNITY_RING_CAP + 9)
        );
        assert_eq!(
            svc.status().profitable_total,
            OPPORTUNITY_RING_CAP as u64 + 10
        );
    }

    #[test]
    fn execution_counters_track_success_and_failure() {
        let svc = service();
        svc.record_execution_result(true);
        svc.record_execution_result(false);
        let s = svc.status();
        assert_eq!(
            (s.executed_total, s.successful_total, s.failed_total),
            (2, 1, 1)
        );
    }

    #[test]
    fn events_broadcast_to_subscribers() {
        let svc = service();
        let mut rx = svc.subscribe();
        svc.record_log("info", "test", "hello");
        let event = rx.try_recv().expect("event delivered");
        assert_eq!(event.kind, "log");
    }

    #[test]
    fn audit_file_receives_mutating_actions() {
        let path = std::env::temp_dir().join(format!(
            "arb_audit_{}_{}.log",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ));
        let mode = Arc::new(std::sync::atomic::AtomicU8::new(0));
        let svc = ControlService::new(
            mode,
            Arc::new(KillSwitch::new()),
            path.display().to_string(),
        );
        let _ = svc.stop();
        let contents = std::fs::read_to_string(&path).expect("audit file written");
        assert!(contents.contains("\"action\":\"stop\""));
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod config_override_tests {
    use super::*;

    fn update(
        min_net_profit_usd: Option<f64>,
        min_borrow: Option<u64>,
        max_borrow: Option<u64>,
        slippage: Option<u64>,
    ) -> ConfigOverrides {
        ConfigOverrides {
            min_net_profit_usd,
            min_borrow_lamports: min_borrow,
            max_borrow_lamports: max_borrow,
            max_slippage_bps: slippage,
        }
    }

    #[test]
    fn valid_update_merges_and_validates() {
        let base = ConfigOverrides::default();
        let merged = base
            .merged_with(&update(
                Some(0.05),
                Some(100_000),
                Some(2_000_000_000),
                Some(100),
            ))
            .expect("valid update");
        assert_eq!(merged.min_net_profit_usd, Some(0.05));
        assert_eq!(merged.max_borrow_lamports, Some(2_000_000_000));
    }

    #[test]
    fn negative_profit_usd_rejected() {
        let base = ConfigOverrides::default();
        assert!(
            base.merged_with(&update(Some(-0.5), None, None, None))
                .is_err()
        );
        assert!(
            base.merged_with(&update(Some(f64::NAN), None, None, None))
                .is_err()
        );
    }

    #[test]
    fn min_borrow_above_max_rejected() {
        let base = update(None, Some(10), Some(5), None);
        assert!(ConfigOverrides::default().merged_with(&base).is_err());
    }

    #[test]
    fn slippage_bounds_enforced() {
        let base = ConfigOverrides::default();
        assert!(
            base.merged_with(&update(None, None, None, Some(0)))
                .is_err()
        );
        assert!(
            base.merged_with(&update(None, None, None, Some(10_001)))
                .is_err()
        );
        assert!(
            base.merged_with(&update(None, None, None, Some(5_000)))
                .is_ok()
        );
    }

    #[test]
    fn update_config_persists_and_audits() {
        let mode = Arc::new(std::sync::atomic::AtomicU8::new(0));
        let path = std::env::temp_dir().join(format!(
            "arb_audit_cfg_{}_{}.log",
            std::process::id(),
            Instant::now().elapsed().as_nanos()
        ));
        let svc = ControlService::new(
            mode,
            Arc::new(KillSwitch::new()),
            path.display().to_string(),
        );
        let merged = svc
            .update_config(&update(Some(0.25), None, None, None))
            .expect("valid");
        assert_eq!(merged.min_net_profit_usd, Some(0.25));
        assert_eq!(svc.overrides().min_net_profit_usd, Some(0.25));
        let audit = std::fs::read_to_string(&path).expect("audit written");
        assert!(audit.contains("update_config"));
        let _ = std::fs::remove_file(&path);
    }
}
