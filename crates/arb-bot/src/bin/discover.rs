//! Read-only discovery probe.
//!
//! Prints the pool watch set the bot would build right now — how many pools per
//! venue, their TVL, fee tier and mint pair. No keypair, no signing, no orders.
//!
//! Usage: `cargo run -p arb-bot --bin discover [min_tvl_usd] [max_pools] [max_per_venue]`

use arb_ingest::{DiscoveryConfig, PoolDiscovery, RpcPool};
use std::collections::HashMap;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "discover=info,arb_ingest=info".into()),
        )
        .init();

    let mut args = std::env::args().skip(1);
    let mut cfg = DiscoveryConfig::default();
    if let Some(v) = args.next() {
        cfg.min_tvl_usd = v.parse().unwrap_or(cfg.min_tvl_usd);
    }
    if let Some(v) = args.next() {
        cfg.max_pools = v.parse().unwrap_or(cfg.max_pools);
    }
    if let Some(v) = args.next() {
        cfg.max_per_venue = v.parse().unwrap_or(cfg.max_per_venue);
    }

    let discovery = PoolDiscovery::new(RpcPool::from_env());
    let started = std::time::Instant::now();
    let pools = discovery.discover(&cfg).await;
    let elapsed = started.elapsed();

    println!(
        "\n=== discovery: {} pools in {:?} ===",
        pools.len(),
        elapsed
    );

    let mut by_venue: HashMap<String, usize> = HashMap::new();
    let mut by_fee: HashMap<u16, usize> = HashMap::new();
    for p in &pools {
        *by_venue.entry(format!("{:?}", p.protocol)).or_default() += 1;
        *by_fee.entry(p.fee_bps).or_default() += 1;
    }
    let mut venues: Vec<_> = by_venue.into_iter().collect();
    venues.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    println!("venues: {venues:?}");
    let mut fees: Vec<_> = by_fee.into_iter().collect();
    fees.sort_by_key(|(f, _)| *f);
    println!("fee tiers (bps): {fees:?}");

    println!(
        "\n{:<46} {:<8} {:>14} {:>6}  mints",
        "pool", "venue", "tvl_usd", "bps"
    );
    for p in pools.iter().take(25) {
        println!(
            "{:<46} {:<8} {:>14.0} {:>6}  {}/{}",
            p.pool_id.0,
            format!("{:?}", p.protocol),
            p.liquidity_usd,
            p.fee_bps,
            p.base_mint.0,
            p.quote_mint.0
        );
    }
    println!();
    Ok(())
}
