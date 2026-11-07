//! Replay harness: read a recorded stream file (JSON lines of PoolState updates)
//! and run the pricing loop in dry-run, printing opportunities.

use anyhow::Result;
use arb_engine::MarketGraph;
use arb_types::{ExecutionOpportunity, PoolState, Slot};
use std::env;
use std::fs::File;
use std::io::{BufRead, BufReader};

fn parse_args() -> (String,) {
    let mut args = env::args().skip(1);
    let mut input = String::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--input" | "-i" => {
                input = args.next().unwrap_or_default();
            }
            other if input.is_empty() => {
                input = other.to_string();
            }
            _ => {}
        }
    }
    (input,)
}

fn main() -> Result<()> {
    let (input,) = parse_args();
    if input.is_empty() {
        anyhow::bail!("usage: replay --input <file>");
    }

    let file = File::open(&input)?;
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

    // Derive base anchors from pools.
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
