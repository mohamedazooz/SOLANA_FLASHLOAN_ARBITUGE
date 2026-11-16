//! One-shot: run live discovery and print the FULL pool ids of meme DLMM
//! groups (meme/SOL and meme/USDC), for diagnostics. Read-only.

use arb_ingest::{DiscoveredPoolState, DiscoveryConfig, PoolDiscovery, RpcPool};
use arb_types::DexProtocol;
use solana_pubkey::Pubkey;
use std::collections::HashMap;

const WSOL: &str = "So11111111111111111111111111111111111111112";
const ANCHOR_MINTS: [&str; 3] = [
    "So11111111111111111111111111111111111111112",
    "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
    "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB",
];

fn is_anchor(mint: &Pubkey) -> bool {
    ANCHOR_MINTS
        .iter()
        .any(|a| a.parse::<Pubkey>().as_ref() == Ok(mint))
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    let mut cfg = DiscoveryConfig::default();
    cfg.min_tvl_usd = 1_000.0;
    cfg.max_pools = 500;
    cfg.max_per_venue = 100;
    let pools = PoolDiscovery::new(RpcPool::from_env()).discover(&cfg).await;

    let mut groups: HashMap<String, Vec<&DiscoveredPoolState>> = HashMap::new();
    for p in pools.iter().filter(|p| p.protocol == DexProtocol::MeteoraDlmm) {
        let (a, b) = (p.base_mint.0, p.quote_mint.0);
        let (meme, quote) = if is_anchor(&a) && !is_anchor(&b) {
            (b, a)
        } else if is_anchor(&b) && !is_anchor(&a) {
            (a, b)
        } else {
            continue;
        };
        groups
            .entry(format!("{}|{}", meme, quote))
            .or_default()
            .push(p);
    }
    for (k, v) in groups.iter().filter(|(_, v)| v.len() >= 2) {
        let (meme, quote) = k.split_once('|').unwrap();
        println!("GROUP meme={} quote={} pools={}", meme, quote, v.len());
        for p in v {
            println!(
                "  {} liq=${:.0} base={} quote={}",
                p.pool_id.0, p.liquidity_usd, p.base_mint.0, p.quote_mint.0
            );
        }
    }
}
