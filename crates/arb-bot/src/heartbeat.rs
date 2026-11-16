//! Process liveness heartbeat.
//!
//! A PID file alone cannot distinguish "alive and trading" from "alive but wedged
//! on a dead stream". This module owns a small, atomic, machine-readable liveness
//! record (`data/heartbeat.json` by default) that an external verifier can
//! cross-check against OS process metadata.
//!
//! Contract:
//! * The file is written **atomically** (`tmp` + rename) so a reader never sees a
//!   half-written document.
//! * `heartbeat_seq` increments on every loop iteration that reached the write
//!   point, so a stalled loop is visible as a non-advancing sequence number.
//! * `last_tick_age_ms` and `heartbeat_age_ms` are measured with a monotonic
//!   clock; wall-clock values are recorded alongside for audit.
//! * Every field is derived from observed state. Nothing here is estimated from
//!   configuration.
//!
//! This module never trades, never signs and never touches RPC.

use parking_lot::Mutex;
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Default location, relative to the process working directory.
pub const DEFAULT_HEARTBEAT_PATH: &str = "data/heartbeat.json";

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Format a Unix timestamp as `YYYY-MM-DDTHH:MM:SSZ` without a date library.
fn format_utc(unix: u64) -> String {
    let days = (unix / 86_400) as i64;
    let secs_of_day = unix % 86_400;
    let (hour, minute, second) = (
        secs_of_day / 3_600,
        (secs_of_day % 3_600) / 60,
        secs_of_day % 60,
    );

    // Howard Hinnant's civil_from_days, valid for the whole Unix epoch range.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };

    format!("{year:04}-{m:02}-{d:02}T{hour:02}:{minute:02}:{second:02}Z")
}

#[derive(Default)]
struct Counters {
    seq: u64,
    loop_entered_uptime_ms: Option<u64>,
    last_tick_unix: Option<u64>,
    last_tick_uptime_ms: u64,
    current_slot: u64,
    quotes_evaluated: u64,
    last_quote_unix: Option<u64>,
    last_quote_slot: Option<u64>,
    last_quote_opportunities: usize,
    rejections_total: u64,
    last_rejection_unix: Option<u64>,
    last_rejection_slot: Option<u64>,
    last_rejection_stage: Option<String>,
    last_rejection_reason: Option<String>,
}

/// Shared heartbeat writer. Cheap to clone behind an `Arc`.
pub struct Heartbeat {
    path: PathBuf,
    pid: u32,
    mode: String,
    binary: String,
    version: &'static str,
    started_unix: u64,
    started_at: Instant,
    counters: Mutex<Counters>,
}

impl std::fmt::Debug for Heartbeat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Heartbeat")
            .field("path", &self.path)
            .field("pid", &self.pid)
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

impl Heartbeat {
    /// Build a heartbeat for this process.
    ///
    /// The start timestamp is captured now, at construction, which is the
    /// earliest point the process can observe itself; the external verifier
    /// compares it to the OS-reported creation time and reports the delta.
    pub fn new(path: impl AsRef<Path>, mode: impl Into<String>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            pid: std::process::id(),
            mode: mode.into(),
            binary: std::env::current_exe()
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                .unwrap_or_else(|| "arb-bot".to_string()),
            version: env!("CARGO_PKG_VERSION"),
            started_unix: unix_now(),
            started_at: Instant::now(),
            counters: Mutex::new(Counters::default()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Resolve the heartbeat path from `HEARTBEAT_PATH`, falling back to
    /// [`DEFAULT_HEARTBEAT_PATH`].
    pub fn path_from_env() -> PathBuf {
        std::env::var("HEARTBEAT_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(DEFAULT_HEARTBEAT_PATH))
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Mark that the strategy loop has been entered (startup discovery done).
    ///
    /// Before this call the process is legitimately busy with a long startup
    /// phase, so a non-advancing tick counter is NOT evidence of a stall.
    pub fn mark_loop_entered(&self) {
        let mut c = self.counters.lock();
        c.loop_entered_uptime_ms
            .get_or_insert(self.started_at.elapsed().as_millis() as u64);
    }

    /// Record one supervisor loop iteration.
    pub fn observe_tick(&self, slot: u64) {
        let mut c = self.counters.lock();
        c.seq = c.seq.saturating_add(1);
        c.last_tick_unix = Some(unix_now());
        c.last_tick_uptime_ms = self.started_at.elapsed().as_millis() as u64;
        if slot != 0 {
            c.current_slot = slot;
        }
    }

    /// Record that a quoting pass completed with `opportunities` candidates.
    pub fn observe_quote(&self, slot: u64, opportunities: usize) {
        let mut c = self.counters.lock();
        c.quotes_evaluated = c.quotes_evaluated.saturating_add(opportunities as u64);
        c.last_quote_unix = Some(unix_now());
        c.last_quote_slot = Some(slot);
        c.last_quote_opportunities = opportunities;
    }

    /// Record an opportunity that was rejected by a gate.
    pub fn observe_rejection(&self, slot: u64, stage: &str, reason: &str) {
        let mut c = self.counters.lock();
        c.rejections_total = c.rejections_total.saturating_add(1);
        c.last_rejection_unix = Some(unix_now());
        c.last_rejection_slot = Some(slot);
        c.last_rejection_stage = Some(stage.to_string());
        c.last_rejection_reason = Some(reason.to_string());
    }

    fn snapshot(&self, rpc: Option<&Value>) -> Value {
        let c = self.counters.lock();
        let now_unix = unix_now();
        let uptime_ms = self.started_at.elapsed().as_millis() as u64;
        // Monotonic age: immune to wall-clock jumps (NTP, suspend/resume).
        let tick_age_monotonic_ms = uptime_ms.saturating_sub(c.last_tick_uptime_ms);

        let rpc_healthy = rpc
            .and_then(|r| r.get("healthy_count"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let rpc_total = rpc
            .and_then(|r| r.get("configured_count").or_else(|| r.get("total")))
            .and_then(Value::as_u64)
            .unwrap_or(0);

        let tick_fresh = c.last_tick_unix.is_some() && tick_age_monotonic_ms < 10_000;
        let loop_started = c.loop_entered_uptime_ms.is_some();
        let status = if !loop_started {
            "STARTING"
        } else if !tick_fresh {
            "STALLED"
        } else if rpc_total > 0 && rpc_healthy == 0 {
            "DEGRADED_RPC"
        } else {
            "OK"
        };

        json!({
            "schema": "sentinel.heartbeat.v1",
            "pid": self.pid,
            "binary": self.binary,
            "version": self.version,
            "mode": self.mode,
            "process_started_unix": self.started_unix,
            "process_started_utc": format_utc(self.started_unix),
            "uptime_ms": uptime_ms,
            "uptime_secs": uptime_ms / 1_000,
            "loop_started": loop_started,
            "startup_duration_ms": c.loop_entered_uptime_ms,
            "heartbeat_seq": c.seq,
            "heartbeat_written_unix": now_unix,
            "heartbeat_written_utc": format_utc(now_unix),
            "heartbeat_age_ms": 0,
            "last_tick_unix": c.last_tick_unix,
            "last_tick_utc": c.last_tick_unix.map(format_utc),
            "last_tick_age_ms": tick_age_monotonic_ms,
            "current_slot": c.current_slot,
            "quotes": {
                "evaluated_total": c.quotes_evaluated,
                "last_unix": c.last_quote_unix,
                "last_slot": c.last_quote_slot,
                "last_opportunities": c.last_quote_opportunities,
            },
            "rejections": {
                "total": c.rejections_total,
                "last_unix": c.last_rejection_unix,
                "last_slot": c.last_rejection_slot,
                "last_stage": c.last_rejection_stage,
                "last_reason": c.last_rejection_reason,
            },
            "rpc": rpc.cloned().unwrap_or(Value::Null),
            "safety": {
                "ARB_LIVE": std::env::var("ARB_LIVE").unwrap_or_else(|_| "<unset>".into()),
                "LIVE": std::env::var("LIVE").unwrap_or_else(|_| "<unset>".into()),
                "EXECUTE_LIVE": std::env::var("EXECUTE_LIVE").unwrap_or_else(|_| "<unset>".into()),
                "DRY_RUN": std::env::var("DRY_RUN").unwrap_or_else(|_| "<unset>".into()),
            },
            "status": status,
        })
    }

    /// Atomically write the heartbeat document.
    ///
    /// `rpc` is the serialized `RpcPoolHealth`; pass `None` only when the pool has
    /// not been probed yet.
    pub fn write(&self, rpc: Option<&Value>) -> std::io::Result<()> {
        let doc = self.snapshot(rpc);
        if let Some(parent) = self.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let tmp = self.path.with_extension("json.tmp");
        {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(serde_json::to_string_pretty(&doc)?.as_bytes())?;
            f.write_all(b"\n")?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_format_matches_known_instants() {
        assert_eq!(format_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_utc(1_000_000_000), "2001-09-09T01:46:40Z");
        assert_eq!(format_utc(1_700_000_000), "2023-11-14T22:13:20Z");
    }

    #[test]
    fn counters_and_status_advance() {
        let hb = Heartbeat::new(
            std::env::temp_dir().join("sentinel-heartbeat-test.json"),
            "simulate",
        );
        assert_eq!(hb.snapshot(None)["status"], "STARTING");
        hb.observe_tick(0);
        assert_eq!(hb.snapshot(None)["status"], "STARTING");
        hb.mark_loop_entered();
        hb.observe_tick(42);
        hb.observe_quote(42, 3);
        hb.observe_rejection(42, "FreshnessGate", "StaleQuote");
        let doc = hb.snapshot(None);
        assert_eq!(doc["heartbeat_seq"], 2);
        assert_eq!(doc["current_slot"], 42);
        assert_eq!(doc["loop_started"], true);
        assert_eq!(doc["quotes"]["evaluated_total"], 3);
        assert_eq!(doc["rejections"]["total"], 1);
        assert_eq!(doc["rejections"]["last_stage"], "FreshnessGate");
        assert_eq!(doc["status"], "OK");
        hb.write(None).expect("heartbeat write");
        let read: Value =
            serde_json::from_str(&std::fs::read_to_string(hb.path()).unwrap()).unwrap();
        assert_eq!(read["pid"], std::process::id());
    }
}
