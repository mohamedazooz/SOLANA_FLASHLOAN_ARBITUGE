//! Server health monitor: watches every critical dependency of the bot and
//! logs the exact reason whenever something goes down.
//!
//! Checks:
//!   * Solami Yellowstone gRPC — at least one live slot watermark must be observed.
//!   * Pipeline Latency — alert if processing age exceeds acceptable bounds (>1000ms).
//!   * Disk space — data directory must not fill up (logs + state DB grow).
//!   * Telegram — best-effort ping; never blocks trading.
//!
//! On any failure it logs `error!` with the reason and sends a Telegram
//! alert (cooldown-protected). When a failed service recovers, it sends a
//! detailed recovery alert explaining the outage duration and resolution.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::{debug, error, info};

/// Threshold below which free disk space triggers an alert (bytes).
const DISK_FREE_WARN_BYTES: u64 = 2 * 1024 * 1024 * 1024; // 2 GiB

/// Minimum cooldown between repeat alerts for the same failing check.
const ALERT_COOLDOWN_SECS: u64 = 300;

/// Result of one health check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CheckStatus {
    #[default]
    Ok,
    Fail,
}

/// One check's last result, used for cooldown + recovery tracking.
#[derive(Debug, Clone, Default)]
struct CheckState {
    status: CheckStatus,
    last_alert: Option<Instant>,
    fail_start: Option<Instant>,
}

/// The monitor. Cheap to clone (Arc internally).
#[derive(Clone)]
pub struct HealthMonitor {
    ingest: Arc<arb_ingest::YellowstoneClient>,
    data_dir: PathBuf,
    telegram: crate::telegram::TelegramNotifier,
    checks: Arc<Mutex<HashMap<&'static str, CheckState>>>,
}

impl HealthMonitor {
    pub fn new(
        ingest: Arc<arb_ingest::YellowstoneClient>,
        data_dir: PathBuf,
        telegram: crate::telegram::TelegramNotifier,
    ) -> Self {
        Self {
            ingest,
            data_dir,
            telegram,
            checks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Run one sweep of all checks. Returns a human-readable summary line.
    /// Never propagates errors — every check catches its own failure.
    pub async fn sweep(&self, slot: u64) -> String {
        let mut results: Vec<(&'static str, CheckStatus, Option<String>)> = Vec::new();

        results.push(self.check_yellowstone().await);
        results.push(self.check_disk().await);
        results.push(self.check_telegram().await);

        let mut summary_parts: Vec<String> = Vec::new();
        let mut any_fail = false;

        for (name, status, reason) in results {
            let label = match status {
                CheckStatus::Ok => "ok",
                CheckStatus::Fail => {
                    any_fail = true;
                    "FAIL"
                }
            };
            summary_parts.push(format!("{name}={label}"));
            if let Some(ref r) = reason {
                summary_parts.push(format!("({r})"));
            }
            self.record(name, status, &reason, slot).await;
        }

        if any_fail {
            let summary = summary_parts.join(" ");
            error!("health sweep: {summary}");
        } else {
            debug!("health sweep: all checks ok ({})", summary_parts.join(" "));
        }

        summary_parts.join(" ")
    }

    /// Record observation of processing latency; alerts if a spike occurs,
    /// and alerts recovery when latency normalizes.
    pub async fn record_latency(&self, latency_ms: f64, slot: u64) {
        if latency_ms > 1000.0 {
            let reason = format!(
                "طفرة بطء معالجة (Latency Spike): {:.1}ms تجاوزت الحد المسموح (1000ms)",
                latency_ms
            );
            self.record("latency", CheckStatus::Fail, &Some(reason), slot)
                .await;
        } else if latency_ms <= 400.0 {
            self.record("latency", CheckStatus::Ok, &None, slot).await;
        }
    }

    /// Yellowstone gRPC must have produced a real slot watermark. No HTTP RPC probe/fallback.
    async fn check_yellowstone(&self) -> (&'static str, CheckStatus, Option<String>) {
        let slot = self.ingest.current_slot();
        if slot > 0 {
            ("yellowstone_grpc", CheckStatus::Ok, None)
        } else {
            (
                "yellowstone_grpc",
                CheckStatus::Fail,
                Some("no slot watermark received from Solami Yellowstone gRPC".to_string()),
            )
        }
    }

    /// Disk: free space under `data_dir` must stay above the warn threshold.
    async fn check_disk(&self) -> (&'static str, CheckStatus, Option<String>) {
        match std::fs::metadata(&self.data_dir) {
            Ok(_) => {}
            Err(e) => {
                return (
                    "disk",
                    CheckStatus::Fail,
                    Some(format!("data dir stat failed: {e}")),
                );
            }
        }

        // `std::fs` has no free-space API on stable; shell out to `df` on Linux.
        // On non-Linux we skip the check gracefully.
        if cfg!(target_os = "linux") {
            let output = std::process::Command::new("df")
                .arg("-B1")
                .arg(self.data_dir.to_string_lossy().as_ref())
                .output();
            match output {
                Ok(out) if out.status.success() => {
                    let text = String::from_utf8_lossy(&out.stdout);
                    let free = parse_df_free_bytes(&text);
                    match free {
                        Some(f) if f < DISK_FREE_WARN_BYTES => (
                            "disk",
                            CheckStatus::Fail,
                            Some(format!(
                                "free disk {} bytes < {} warn",
                                f, DISK_FREE_WARN_BYTES
                            )),
                        ),
                        _ => ("disk", CheckStatus::Ok, None),
                    }
                }
                _ => ("disk", CheckStatus::Ok, None), // can't measure → don't fail trading
            }
        } else {
            ("disk", CheckStatus::Ok, None)
        }
    }

    /// Telegram: best-effort ping. A Telegram outage is logged, never
    /// propagated to trading — we only report it so the operator knows.
    async fn check_telegram(&self) -> (&'static str, CheckStatus, Option<String>) {
        if !self.telegram.is_enabled() {
            return ("telegram", CheckStatus::Ok, None); // not configured → not a failure
        }
        match self.telegram.ping().await {
            Ok(()) => ("telegram", CheckStatus::Ok, None),
            Err(e) => (
                "telegram",
                CheckStatus::Fail,
                Some(format!("ping failed: {e}")),
            ),
        }
    }

    /// Record a check result, honoring alert cooldown and sending recovery notifications.
    async fn record(
        &self,
        name: &'static str,
        status: CheckStatus,
        reason: &Option<String>,
        slot: u64,
    ) {
        let mut checks = self.checks.lock().await;
        let state = checks.entry(name).or_default();

        match status {
            CheckStatus::Ok => {
                // Recovering from a failure: calculate duration and send recovery alert
                if state.status == CheckStatus::Fail {
                    let duration_secs = state
                        .fail_start
                        .map(|t| t.elapsed().as_secs_f64())
                        .unwrap_or(0.0);
                    info!(
                        "health recovered: {name} (outage duration: {:.1}s)",
                        duration_secs
                    );

                    let (service_name, resolution) = match name {
                        "rpc" => (
                            "Solana RPC Pool (Primary/Secondary)",
                            "تم التبديل التلقائي واستعادة الاتصال بعقدة الـ RPC واستقرار استجابة getSlot.",
                        ),
                        "latency" => (
                            "Pipeline Latency Engine (Hot Path)",
                            "عادت سرعة المعالجة إلى النطاق الطبيعي السريع (<400ms) وتفريغ الطابور اللحظي.",
                        ),
                        "disk" => (
                            "Disk Storage / Data Partition",
                            "المساحة التخزينية المتاحة كافية ومستقرة الآن.",
                        ),
                        "telegram" => (
                            "Telegram API Telemetry",
                            "استعادة الاتصال بسيرفرات Telegram API بنجاح.",
                        ),
                        _ => (name, "تم استعادة استقرار الخدمة وتجاوز العطل بنجاح."),
                    };

                    self.telegram
                        .alert_service_recovery(service_name, duration_secs, resolution, slot)
                        .await;
                }
                state.status = CheckStatus::Ok;
                state.last_alert = None;
                state.fail_start = None;
            }
            CheckStatus::Fail => {
                if state.status == CheckStatus::Fail {
                    // Already failing; only re-alert after cooldown.
                    let should_alert = state
                        .last_alert
                        .map(|t| t.elapsed() >= Duration::from_secs(ALERT_COOLDOWN_SECS))
                        .unwrap_or(true);
                    if !should_alert {
                        return; // still in cooldown, don't spam
                    }
                } else {
                    // Newly failing
                    state.fail_start = Some(Instant::now());
                }

                state.status = CheckStatus::Fail;
                state.last_alert = Some(Instant::now());

                let service_name = match name {
                    "rpc" => "Solana RPC Pool (Primary/Secondary)",
                    "latency" => "Pipeline Latency Engine (Hot Path)",
                    "disk" => "Disk Storage / Data Partition",
                    "telegram" => "Telegram API Telemetry",
                    _ => name,
                };

                let reason_str = reason
                    .as_deref()
                    .unwrap_or("انقطاع غير متوقع في استجابة الخدمة");

                self.telegram
                    .alert_service_failure(service_name, reason_str, slot)
                    .await;
            }
        }
    }
}

/// Parse the "Avail" column from `df -B1` output (second line, 4th field).
fn parse_df_free_bytes(text: &str) -> Option<u64> {
    let mut lines = text.lines();
    lines.next()?; // skip header
    let fields: Vec<&str> = lines.next()?.split_whitespace().collect();
    fields.get(3)?.parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_df_free_basic() {
        let out = "Filesystem     1B-blocks      Used Available Use% Mounted on\n/dev/root      50000000000 20000000000 30000000000  40% /\n";
        assert_eq!(parse_df_free_bytes(out), Some(30_000_000_000));
    }

    #[test]
    fn parse_df_free_truncated() {
        let out =
            "Filesystem 1B-blocks Used Available Use% Mounted on\n/dev/root 1000 500 500 50% /\n";
        assert_eq!(parse_df_free_bytes(out), Some(500));
    }
}
