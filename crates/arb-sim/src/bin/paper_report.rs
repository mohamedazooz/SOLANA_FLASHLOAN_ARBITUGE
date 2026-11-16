//! CLI: generate the Gate-H paper-trading evaluation report from a recorded
//! opportunity-history directory (the directory passed to
//! `OpportunityHistoryRecorder::new`, containing `candidates.jsonl`).
//!
//! Usage: `paper_report <log_dir> [out_dir]`
//!
//! Read-only over recorded data: never signs, submits, or authorizes anything.

use arb_sim::opportunity_recorder::OpportunityHistoryRecorder;
use arb_sim::paper_trading_report::{build_report, write_report};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(log_dir) = args.next() else {
        eprintln!("usage: paper_report <log_dir> [out_dir]");
        std::process::exit(2);
    };
    let out_dir = args.next().unwrap_or_else(|| log_dir.clone());

    let recorder = OpportunityHistoryRecorder::new(&log_dir);
    let records = match recorder.load_candidates() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("failed to load candidates from {}: {}", log_dir, e);
            std::process::exit(1);
        }
    };

    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let report = build_report(&records, now_unix);

    if let Err(e) = write_report(&out_dir, &report) {
        eprintln!("failed to write report to {}: {}", out_dir, e);
        std::process::exit(1);
    }

    println!(
        "paper-trading report: detected={} accepted={} rejected={} replayed={} reproduced={} net_total={}",
        report.counts.detected,
        report.counts.accepted,
        report.counts.rejected,
        report.quote_accuracy.replayed,
        report.quote_accuracy.reproduced,
        report.pnl.net_total_raw,
    );
    println!("confidence: {}", report.confidence.note);
    println!(
        "written: {}/paper_trading_report.json, {}/paper_trading_report.md",
        out_dir, out_dir
    );
}
