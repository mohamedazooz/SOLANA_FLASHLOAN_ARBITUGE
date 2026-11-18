//! Daily aggregated operations report (checklist 637).
//!
//! One authoritative UTC-daily aggregation of durable outcomes (`Store::pnl_daily`)
//! plus in-memory rejection rings and stream-integrity counters. Pure logic so
//! the aggregation, top-reason ranking, and UTC-date rollover are unit-testable
//! offline; `Supervisor::emit_daily_report` is the runtime caller.

use serde::Serialize;
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// One rejection ring entry projected down to what the report aggregates.
#[derive(Debug, Clone)]
pub struct RejectionReasoning {
    pub stage: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DailyReport {
    pub date: String,
    pub slot: u64,
    /// Dispatch attempts recorded today (durable outcomes table).
    pub attempts: u64,
    /// Attempts that landed today.
    pub landed: u64,
    /// Durable realized net profit today (lamports, SIGNED — losses are
    /// negative). On-chain-actual provenance only; estimates live in
    /// `simulated_net_lamports`.
    pub realized_net_lamports: i64,
    /// Simulated/canary net profit today (lamports). Kept separate from
    /// realized so a simulation can never be presented as realized PnL.
    pub simulated_net_lamports: i64,
    /// Dropped bundles / rejected opportunities accumulated in the reporting
    /// rings since the last drain (the 5-minute digest drains the same rings).
    pub dropped_recent: u64,
    pub rejected_recent: u64,
    pub opportunities_total: u64,
    pub stream_gaps_total: u64,
    /// Top rejection reasons by count, most frequent first (max 5).
    pub top_rejection_reasons: Vec<(String, u64)>,
}

/// Fraction of attempts that landed, in basis points (0..=10_000).
impl DailyReport {
    pub fn landing_rate_bps(&self) -> u64 {
        if self.attempts == 0 {
            return 0;
        }
        (self.landed.min(self.attempts) * 10_000) / self.attempts
    }

    /// Plain-text rendering (Telegram HTML mode tolerates plain text).
    pub fn render_text(&self) -> String {
        let reasons = if self.top_rejection_reasons.is_empty() {
            "• لا توجد أسباب رفض مُسجَّلة".to_string()
        } else {
            self.top_rejection_reasons
                .iter()
                .map(|(reason, n)| format!("• {reason}: {n}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        format!(
            "📊 <b>تقرير يومي — {}</b>\n\
             • محاولات: {} | منفذة: {} ({:.2}%)\n\
             • صافي الربح المحقق: {} lamports (actual only; السالب = خسارة)\n\
             • صافي المحاكى: {} lamports\n\
             • فرص: {} | فجوات الـ stream: {}\n\
             • Bundles مُسقطة منذ آخر drain: {} | مرفوضة: {}\n\
             <b>أسباب الرفض الأعلى:</b>\n{reasons}",
            self.date,
            self.attempts,
            self.landed,
            self.landing_rate_bps() as f64 / 100.0,
            self.realized_net_lamports,
            self.simulated_net_lamports,
            self.opportunities_total,
            self.stream_gaps_total,
            self.dropped_recent,
            self.rejected_recent,
        )
    }
}

/// Rank rejection reasons by count (stage-qualified), most frequent first, max 5.
fn rank_rejection_reasons(rejections: &[RejectionReasoning]) -> Vec<(String, u64)> {
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    for r in rejections {
        let key = if r.reason.is_empty() {
            format!("{}: <unspecified>", r.stage)
        } else {
            format!("{}: {}", r.stage, r.reason)
        };
        *counts.entry(key).or_insert(0) += 1;
    }
    let mut ranked: Vec<(String, u64)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked.truncate(5);
    ranked
}

/// Build the aggregated daily report from durable + in-memory inputs.
///
/// Pure so the aggregation contract is unit-testable without a store.
pub fn build_daily_report(
    date: String,
    slot: u64,
    pnl: arb_store::PnlDaily,
    dropped_recent: u64,
    rejected_recent: u64,
    rejections: &[RejectionReasoning],
    opportunities_total: u64,
    stream_gaps_total: u64,
) -> DailyReport {
    DailyReport {
        date,
        slot,
        attempts: pnl.attempts,
        landed: pnl.successful,
        realized_net_lamports: pnl.realized_net.as_i64(),
        simulated_net_lamports: pnl.simulated_net.as_i64(),
        dropped_recent,
        rejected_recent,
        opportunities_total,
        stream_gaps_total,
        top_rejection_reasons: rank_rejection_reasons(rejections),
    }
}

/// Current UTC calendar date as `YYYY-MM-DD` (civil-from-days, no deps).
pub fn current_utc_date() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    utc_date_from_epoch_days((now.as_secs() / 86_400) as i64)
}

/// Howard Hinnant's `civil_from_days`: days since 1970-01-01 → (Y, M, D).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn utc_date_from_epoch_days(days: i64) -> String {
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_date_epoch_and_calendar_boundaries() {
        assert_eq!(utc_date_from_epoch_days(0), "1970-01-01");
        assert_eq!(utc_date_from_epoch_days(30), "1970-01-31");
        assert_eq!(utc_date_from_epoch_days(31), "1970-02-01");
        assert_eq!(utc_date_from_epoch_days(58), "1970-02-28");
        assert_eq!(utc_date_from_epoch_days(59), "1970-03-01"); // non-leap 1970
        // 1972 is leap: Feb 29 exists.
        assert_eq!(utc_date_from_epoch_days(789), "1972-02-29");
        assert_eq!(utc_date_from_epoch_days(790), "1972-03-01");
        // Year rollover.
        assert_eq!(utc_date_from_epoch_days(364), "1970-12-31");
        assert_eq!(utc_date_from_epoch_days(365), "1971-01-01");
        // Pre-epoch (frame-time backdating) stays well-formed.
        assert_eq!(utc_date_from_epoch_days(-1), "1969-12-31");
    }

    #[test]
    fn build_daily_report_counts_ranks_and_rates() {
        let rejections = vec![
            RejectionReasoning {
                stage: "economics".into(),
                reason: "net below hurdle".into(),
            },
            RejectionReasoning {
                stage: "economics".into(),
                reason: "net below hurdle".into(),
            },
            RejectionReasoning {
                stage: "economics".into(),
                reason: "net below hurdle".into(),
            },
            RejectionReasoning {
                stage: "guard".into(),
                reason: "stale slot".into(),
            },
            RejectionReasoning {
                stage: "guard".into(),
                reason: "stale slot".into(),
            },
            RejectionReasoning {
                stage: "quote".into(),
                reason: "insufficient out".into(),
            },
            RejectionReasoning {
                stage: "quote".into(),
                reason: "".into(),
            },
        ];
        let report = build_daily_report(
            "2026-11-17".to_string(),
            470_365_241,
            arb_store::PnlDaily {
                attempts: 40,
                successful: 10,
                realized_net: arb_types::SignedLamports::new(123_456),
                simulated_net: arb_types::SignedLamports::new(987_000),
            },
            7,
            100,
            &rejections,
            9_000,
            3,
        );
        assert_eq!(report.date, "2026-11-17");
        assert_eq!(report.attempts, 40);
        assert_eq!(report.landed, 10);
        assert_eq!(report.realized_net_lamports, 123_456);
        assert_eq!(report.simulated_net_lamports, 987_000);
        assert_eq!(report.landing_rate_bps(), 2_500);
        assert_eq!(report.top_rejection_reasons.len(), 4);
        assert_eq!(
            report.top_rejection_reasons[0],
            ("economics: net below hurdle".to_string(), 3)
        );
        assert_eq!(
            report.top_rejection_reasons[1],
            ("guard: stale slot".to_string(), 2)
        );
        // Empty reason is never silently dropped: it is stage-qualified.
        assert!(
            report
                .top_rejection_reasons
                .contains(&("quote: <unspecified>".to_string(), 1))
        );
        let text = report.render_text();
        assert!(text.contains("2026-11-17"));
        assert!(text.contains("economics: net below hurdle"));
    }

    #[test]
    fn landing_rate_and_render_are_total_and_zero_safe() {
        let empty = DailyReport::default();
        assert_eq!(empty.landing_rate_bps(), 0);
        assert!(empty.render_text().contains('0'));

        let saturated = DailyReport {
            attempts: 10,
            landed: 11,
            ..Default::default()
        };
        assert_eq!(
            saturated.landing_rate_bps(),
            10_000,
            "landed > attempts must clamp, not overflow"
        );
    }
}
