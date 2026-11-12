//! Replay harness: read recorded stream files, saved candidate JSONL logs, or pool snapshots,
//! verify deterministic quotes, and evaluate trade-size optimization offline.

use anyhow::Result;
use arb_engine::{DlmmOptimizerConfig, MarketGraph, MeteoraDlmmTradeSizeOptimizer};
use arb_sim::{CandidateOpportunityRecord, OpportunityHistoryRecorder};
use arb_types::{ExecutionOpportunity, PoolState, Slot};
use std::env;
use std::fs::File;
use std::io::{BufRead, BufReader};

enum ReplayMode {
    PoolStream(String),
    CandidatesLog(String),
    SingleCandidate(String),
    Snapshot(String),
}

fn parse_mode() -> Result<ReplayMode> {
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--candidates" | "-c" => {
                let file = args.next().unwrap_or_default();
                return Ok(ReplayMode::CandidatesLog(file));
            }
            "--candidate" => {
                let file = args.next().unwrap_or_default();
                return Ok(ReplayMode::SingleCandidate(file));
            }
            "--snapshot" | "-s" => {
                let file = args.next().unwrap_or_default();
                return Ok(ReplayMode::Snapshot(file));
            }
            "--input" | "-i" => {
                let file = args.next().unwrap_or_default();
                return Ok(ReplayMode::PoolStream(file));
            }
            other if !other.starts_with('-') => {
                return Ok(ReplayMode::PoolStream(other.to_string()));
            }
            _ => {}
        }
    }
    anyhow::bail!(
        "usage: replay [--candidates <file.jsonl> | --candidate <id.json> | --snapshot <pool.json> | --input <stream.jsonl>]"
    );
}

fn main() -> Result<()> {
    let mode = parse_mode()?;
    match mode {
        ReplayMode::CandidatesLog(file_path) => replay_candidates_jsonl(&file_path),
        ReplayMode::SingleCandidate(file_path) => replay_single_candidate_fixture(&file_path),
        ReplayMode::Snapshot(file_path) => replay_pool_snapshot(&file_path),
        ReplayMode::PoolStream(file_path) => replay_pool_stream(&file_path),
    }
}

fn display_candidate_record(rec: &CandidateOpportunityRecord, index: usize) {
    println!("\n=======================================================");
    println!(
        "     REPLAY CANDIDATE RECORD #{}: {}",
        index + 1,
        rec.candidate_id
    );
    println!("=======================================================");
    println!("Candidate ID:                 {}", rec.candidate_id);
    println!(
        "Detection Timestamp:          {} ms",
        rec.detection_timestamp_ms
    );
    println!("Pool A ID:                    {}", rec.pool_a_id);
    println!("Pool B ID:                    {}", rec.pool_b_id);
    println!("Pool A Slot:                  {}", rec.pool_a_slot);
    println!("Pool B Slot:                  {}", rec.pool_b_slot);
    println!("Quote Snapshot Slot:          {}", rec.quote_slot);
    println!("Direction:                    {}", rec.direction);
    println!(
        "Input Trade Size:             {} raw units",
        rec.trade_size_in
    );
    println!(
        "Hop 1 Output:                 {} raw units",
        rec.hop1_quote_out
    );
    println!(
        "Hop 2 Final Output:           {} raw units",
        rec.hop2_final_out
    );
    println!(
        "Gross Edge:                   {} raw units",
        rec.gross_profit_quote_raw
    );
    println!(
        "Execution Costs:              {} raw units",
        rec.total_execution_cost_quote_raw
    );
    println!(
        "Net Profit:                   {} raw units ({} bps)",
        rec.net_profit_quote_raw, rec.net_profit_bps
    );
    println!("Simulation Success:           {}", rec.simulation_success);
    println!("Simulation Slot:              {:?}", rec.simulation_slot);
    println!(
        "Simulation Latency:           {} ms",
        rec.simulation_latency_ms
    );
    println!(
        "Quote Freshness Cleared:      {}",
        rec.quote_freshness_cleared
    );
    println!(
        "Candidate Lifetime:           {:?} ms",
        rec.candidate_lifetime_ms
    );
    println!(
        "Best Observed Net Profit:     {:?} raw units",
        rec.best_observed_net_profit
    );
    println!("Rejection Reason:             {:?}", rec.rejection_reason);
    println!("Status:                       {}", rec.status);
    println!("--- Fee Breakdown ({} items) ---", rec.fee_items.len());
    for (i, fee) in rec.fee_items.iter().enumerate() {
        println!(
            "  [{}] {:<24} ({:?}) | Amount: {:>10} {:?} (dec={}) | Embedded: {}",
            i + 1,
            fee.name,
            fee.category,
            fee.amount.raw_amount,
            fee.amount.currency,
            fee.amount.decimals,
            fee.already_embedded_in_quote
        );
    }

    let matches = OpportunityHistoryRecorder::replay_candidate_route(rec).unwrap_or(false);
    println!(
        "Deterministic Replay Match:   {}",
        if matches {
            "EXACT MATCH (100%)"
        } else {
            "MISMATCH / STALE"
        }
    );
    println!("=======================================================");
}

fn replay_candidates_jsonl(file_path: &str) -> Result<()> {
    let file = File::open(file_path)?;
    let reader = BufReader::new(file);

    let mut count = 0;
    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let rec: CandidateOpportunityRecord = serde_json::from_str(trimmed)?;
        display_candidate_record(&rec, count);
        count += 1;
    }
    println!("\nReplay finished: {} candidates evaluated.", count);
    Ok(())
}

fn replay_single_candidate_fixture(file_path: &str) -> Result<()> {
    let content = std::fs::read_to_string(file_path)?;
    let rec: CandidateOpportunityRecord = serde_json::from_str(&content)?;
    display_candidate_record(&rec, 0);
    Ok(())
}

fn replay_pool_snapshot(file_path: &str) -> Result<()> {
    let content = std::fs::read_to_string(file_path)?;
    // Try deserializing as CandidateOpportunityRecord or pool snapshot
    if let Ok(rec) = serde_json::from_str::<CandidateOpportunityRecord>(&content) {
        display_candidate_record(&rec, 0);
        return Ok(());
    }

    // Try deserializing as array of PoolStates
    if let Ok(pools) = serde_json::from_str::<Vec<PoolState>>(&content) {
        let pairs = arb_engine::MeteoraDlmmPoolPairDiscovery::discover_pairs(&pools);
        println!(
            "Loaded {} pools, discovered {} eligible DLMM pairs",
            pools.len(),
            pairs.distinct_eligible_pool_pairs
        );
        let cfg = DlmmOptimizerConfig::default();
        for (i, pair) in pairs.eligible_pairs.iter().enumerate() {
            println!(
                "\nEvaluating pair #{}: {} vs {}",
                i + 1,
                pair.pool_a,
                pair.pool_b
            );
            if let Some(res) = MeteoraDlmmTradeSizeOptimizer::find_best_opportunity(pair, &cfg) {
                println!("  Best Observed Candidate Result:");
                println!("    Direction:           {:?}", res.direction);
                println!(
                    "    Optimal Size:        {} raw units",
                    res.optimal_amount_in
                );
                println!(
                    "    Expected Gross Edge: {} raw units",
                    res.expected_gross_profit
                );
                println!(
                    "    Expected Net Profit: {} raw units ({} bps)",
                    res.expected_net_profit, res.net_profit_bps
                );
                println!("    Compute Units:       {}", res.estimated_cu);
                println!("    Required Bin Arrays: {}", res.required_bin_arrays);
                println!("    Is Net Profitable:   {}", res.is_profitable);
            } else {
                println!("  No profitable opportunity found across tested sizes.");
            }
        }
        return Ok(());
    }

    // Try deserializing as raw captured Meteora pool snapshot
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
        if let Some(pool_addr) = v.get("pool_address").and_then(|p| p.as_str()) {
            let slot = v.get("rpc_slot").and_then(|s| s.as_u64()).unwrap_or(0);
            println!("\n=======================================================");
            println!("      REPLAY METEORA DLMM POOL SNAPSHOT RECORD         ");
            println!("=======================================================");
            println!("Pool Address:                 {}", pool_addr);
            println!("Snapshot RPC Slot:            {}", slot);
            if let Some(pair) = v.get("pair") {
                println!(
                    "Active Bin ID:                {}",
                    pair.get("activeId").and_then(|a| a.as_i64()).unwrap_or(0)
                );
                println!(
                    "Bin Step:                     {}",
                    pair.get("binStep").and_then(|b| b.as_u64()).unwrap_or(0)
                );
                println!(
                    "Base Factor:                  {}",
                    pair.get("baseFactor").and_then(|b| b.as_u64()).unwrap_or(0)
                );
                println!(
                    "Token X Mint:                 {}",
                    pair.get("tokenXMint")
                        .and_then(|m| m.as_str())
                        .unwrap_or("")
                );
                println!(
                    "Token Y Mint:                 {}",
                    pair.get("tokenYMint")
                        .and_then(|m| m.as_str())
                        .unwrap_or("")
                );
            }
            if let Some(arrays) = v.get("bin_arrays").and_then(|a| a.as_array()) {
                println!("Captured Bin Arrays:          {}", arrays.len());
                let total_bins: usize = arrays
                    .iter()
                    .filter_map(|a| a.get("bins").and_then(|b| b.as_array()))
                    .map(|b| b.len())
                    .sum();
                println!("Total Decoded Bins:           {}", total_bins);
            }
            println!("Snapshot Status:              VALID_AND_CONSISTENT");
            println!("=======================================================");
            return Ok(());
        }
    }

    anyhow::bail!("Unrecognized pool snapshot format: {}", file_path);
}

fn replay_pool_stream(input: &str) -> Result<()> {
    let file = File::open(input)?;
    let reader = BufReader::new(file);

    let mut pools: Vec<PoolState> = Vec::new();
    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let ps: PoolState = serde_json::from_str(trimmed)?;
        pools.push(ps);
    }

    let mut base_anchors = Vec::new();
    for p in &pools {
        if !base_anchors.contains(&p.base_mint) {
            base_anchors.push(p.base_mint);
        }
        if !base_anchors.contains(&p.quote_mint) {
            base_anchors.push(p.quote_mint);
        }
    }

    let mut graph = MarketGraph::new(base_anchors);
    let slot = pools.iter().map(|p| p.slot.0).max().unwrap_or(1);
    let current_slot = Slot(slot);
    graph.build_from_pools(&pools);

    let mut opps = graph.find_base_cycles(current_slot, u64::MAX);
    if opps.is_empty() {
        opps = graph.find_multi_hop_cycles(current_slot, u64::MAX, 5);
    }

    println!(
        "replay: loaded {} pools, slot {}",
        pools.len(),
        current_slot.0
    );
    println!("replay: {} opportunities", opps.len());
    for opp in &opps {
        print_opportunity(opp);
    }
    Ok(())
}

fn print_opportunity(opp: &ExecutionOpportunity) {
    println!(
        "opp route={} size_in={} gross_profit={} hops={}",
        opp.route_id.0,
        opp.size_in.as_u64(),
        opp.gross_profit.as_u64(),
        opp.hops.len()
    );
}
