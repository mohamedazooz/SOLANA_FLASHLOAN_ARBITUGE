//! Read-only validation path: live on-chain pool reserves vs our AMM math vs
//! Jupiter quotes. This binary NEVER submits transactions.
//!
//! It answers one question: "does our on-chain math match reality?"
//!  1. Fetches live pool reserves via MultiDexRegistry (getMultipleAccounts).
//!  2. Reads VERIFIED economic inputs (Kamino fee sf, mint decimals) on-chain.
//!  3. Builds MarketGraph with the authoritative EconomicConfig and detects cycles.
//!  4. Compares OUR cpmm_out() for a live pool against Jupiter's venue-restricted
//!     quote for the SAME venue — isolating math accuracy from route-finding.
//!
//! Usage: cargo run -p arb-bot --bin validate

use anyhow::Result;
use arb_engine::MarketGraph;
use arb_ingest::{AccountSnapshot, AccountSnapshotStore, MultiDexRegistry, ProviderId, RpcPool};
use arb_math::economic::calculate_flash_fee_raw;
use arb_math::{Leg, cpmm_out, optimal_two_pool};
use arb_tx::JupiterClient;
use arb_types::{Lamports, MintId, Slot, TokenAmount};
use base64::Engine as _;
use solana_pubkey::Pubkey;
use std::str::FromStr;
use tracing::{info, warn};

#[path = "../verified_config.rs"]
mod verified_config;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    info!("🔍 Starting read-only validation path (no transactions)...");

    // ── 1. RPC + live pool reserves ──
    let rpc = RpcPool::from_env();
    info!(
        "Multi-RPC pool initialized with {} endpoint(s)",
        rpc.endpoints_count()
    );
    let mut registry = MultiDexRegistry::mainnet_default(rpc.clone());
    info!("Registry has {} registered pools", registry.pool_count());

    // Optional: validate the DYNAMIC watch set instead of only the hardcoded
    // SOL/USDC baselines. `validate --discover` runs the same catalog discovery
    // the supervisor uses, so the read-only path proves the production set (every
    // liquid token, capped by TVL) is decodable and pricable before it is trusted.
    if std::env::args().any(|a| a == "--discover") {
        let cfg = arb_ingest::DiscoveryConfig::default();
        info!(
            "🔍 Discovery enabled: min_tvl=${:.0} cap={} hubs={}",
            cfg.min_tvl_usd,
            cfg.max_pools,
            cfg.hub_mints.len()
        );
        let discovery = arb_ingest::PoolDiscovery::new(rpc.clone());
        let found = discovery.discover(&cfg).await;
        let registered = found.len();
        arb_ingest::register_discovered(&mut registry, &found);
        info!(
            "Registry now has {} registered pools ({} discovered)",
            registry.pool_count(),
            registered
        );
    }

    let slot = match rpc.get_slot().await {
        Ok(s) => {
            info!("Current slot: {s}");
            Slot(s)
        }
        Err(e) => {
            warn!("getSlot failed: {e}; using slot 0");
            Slot(0)
        }
    };

    let live_pools = registry.fetch_live_pool_states(slot).await;
    info!(
        "Live pool states fetched: {} pools with non-zero reserves",
        live_pools.len()
    );
    {
        let registered = registry.pool_count();
        if let Some(covered) = (live_pools.len() * 100).checked_div(registered) {
            let mut by_venue: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            for p in &live_pools {
                *by_venue.entry(format!("{:?}", p.protocol)).or_default() += 1;
            }
            let mut venues: Vec<_> = by_venue.into_iter().collect();
            venues.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
            info!(
                "📊 Coverage: {}/{} pools priced ({covered}%) — skipped pools are fail-closed (undecodable fee/reserves), not mispriced | {venues:?}",
                live_pools.len(),
                registered
            );
        }
    }

    let wsol = Pubkey::from_str(arb_tx::WSOL_MINT)?;
    let usdc = Pubkey::from_str(arb_tx::USDC_MINT)?;

    for p in &live_pools {
        info!(
            "  Pool: {:?} {} reserve_in={} reserve_out={} fee_bps={:?}",
            p.protocol,
            &p.pool_id.0.to_string()[..8],
            p.reserve_in,
            p.reserve_out,
            p.fee_bps()
        );
    }

    // Implied mid price per SOL/USDC pool + inter-DEX spread. A large spread
    // between venues is the raw signal; whether it survives fees is a separate
    // question answered by the cycle detection below.
    let mut priced: Vec<(&str, f64)> = Vec::new();
    for p in &live_pools {
        let is_sol_usdc = (p.base_mint.0 == wsol && p.quote_mint.0 == usdc)
            || (p.base_mint.0 == usdc && p.quote_mint.0 == wsol);
        if !is_sol_usdc || p.reserve_in == 0 || p.reserve_out == 0 {
            continue;
        }
        let (sol_res, usdc_res) = if p.base_mint.0 == wsol {
            (p.reserve_in as f64 / 1e9, p.reserve_out as f64 / 1e6)
        } else {
            (p.reserve_out as f64 / 1e9, p.reserve_in as f64 / 1e6)
        };
        if sol_res > 0.0 {
            let label = match p.protocol {
                arb_types::DexProtocol::RaydiumV4 => "RaydiumV4",
                arb_types::DexProtocol::OrcaWhirlpool => "OrcaWhirlpool",
                arb_types::DexProtocol::MeteoraDlmm => "MeteoraDlmm",
                _ => "Other",
            };
            let price = usdc_res / sol_res;
            info!("  💲 {label} implied price = {price:.4} USDC/SOL");
            priced.push((label, price));
        }
    }
    if priced.len() >= 2 {
        let max = priced.iter().cloned().fold(f64::MIN, |a, b| a.max(b.1));
        let min = priced.iter().cloned().fold(f64::MAX, |a, b| a.min(b.1));
        if min > 0.0 {
            let spread_bps = (max - min) / min * 10_000.0;
            info!("  📊 Inter-DEX spread = {spread_bps:.2} bps (max {max:.4} / min {min:.4})");
        }
    }

    // ── 2. VERIFIED economic inputs (fail closed) ──
    // The runtime reads verified inputs only from account snapshots. This
    // read-only diagnostic mirrors that contract: it fetches the two accounts
    // over RPC and places them in the same snapshot store, instead of using the
    // retired HTTP JSON-RPC shortcut that always fails closed.
    let reserve = Pubkey::from_str(arb_tx::KAMINO_WSOL_RESERVE)?;
    let reserve_str = reserve.to_string();
    let wsol_str = wsol.to_string();
    let accounts = match rpc
        .get_multiple_accounts(&[reserve_str.as_str(), wsol_str.as_str()])
        .await
    {
        Ok(accounts) => accounts,
        Err(e) => {
            warn!("❌ Account fetch failed ({e}) — cannot price; aborting.");
            return Ok(());
        }
    };

    let store = AccountSnapshotStore::default();
    for (index, (label, pubkey)) in [("Kamino reserve", &reserve), ("WSOL mint", &wsol)]
        .into_iter()
        .enumerate()
    {
        let Some(value) = accounts.get(index).and_then(|a| a.as_ref()) else {
            warn!("❌ {label} account missing — cannot price; aborting.");
            return Ok(());
        };
        let Some(owner) = value.get("owner").and_then(|v| v.as_str()) else {
            warn!("❌ {label} account has no owner — cannot price; aborting.");
            return Ok(());
        };
        let Some(data_b64) = value
            .get("data")
            .and_then(|d| d.get(0))
            .and_then(|d| d.as_str())
        else {
            warn!("❌ {label} account has no base64 data — cannot price; aborting.");
            return Ok(());
        };
        let data = match base64::engine::general_purpose::STANDARD.decode(data_b64) {
            Ok(data) => data,
            Err(e) => {
                warn!("❌ {label} base64 decode failed ({e}) — cannot price; aborting.");
                return Ok(());
            }
        };
        store.apply(AccountSnapshot {
            pubkey: *pubkey,
            owner: Pubkey::from_str(owner)?,
            slot: slot.0,
            write_version: 1,
            is_startup: false,
            // ProviderId(0) is the documented RPC-bootstrap identity.
            provider_id: ProviderId(0),
            bank_id: None,
            generation: 0,
            data: std::sync::Arc::new(data),
        });
    }

    let verified = match verified_config::build_verified_inputs_from_snapshots(
        &store,
        &MintId(wsol),
        &reserve,
        &wsol,
        Lamports(0), // tip not needed for validation
        slot,
        None,
    ) {
        Ok(v) => {
            info!(
                "✅ Verified on-chain: Kamino flash fee_sf={} (source={}), base_decimals={} (source={}), rate={}",
                v.flash_loan_fee_sf.value,
                v.flash_loan_fee_sf.source,
                v.base_decimals.value,
                v.base_decimals.source,
                v.lamports_per_base_raw.value,
            );
            v
        }
        Err(e) => {
            warn!("❌ Verified inputs unavailable ({e}) — cannot price; aborting.");
            return Ok(());
        }
    };

    // ── 3. Build MarketGraph with the authoritative config ──
    let mut graph = MarketGraph::new(vec![MintId(wsol)]);
    graph.set_economic_config(verified.clone().into_economic_config(TokenAmount(30_000)));
    graph.build_from_pools(&live_pools);
    let edge_count = graph.edges_by_from.values().map(|v| v.len()).sum::<usize>();
    info!("MarketGraph built: {edge_count} edges");

    let opportunities = graph.find_base_cycles(slot, 1_000_000_000);
    info!("Detected {} arbitrage cycle(s)", opportunities.len());
    for opp in &opportunities {
        info!(
            "  Cycle: route={} size={} gross={} net={} impact={}bps hops={}",
            opp.route_id.0,
            opp.size_in.0,
            opp.gross_profit.0,
            opp.net_profit.0,
            opp.price_impact_bps.0,
            opp.hops.len()
        );
        // The hop list is what makes a suspicious cycle diagnosable: a 100x
        // "profit" is almost always two venues disagreeing about mint order.
        for h in &opp.hops {
            info!(
                "    hop {:?} pool={} {} -> {} fee={}bps",
                h.protocol,
                &h.pool_id.0.to_string()[..8],
                &h.mint_in.0.to_string()[..6],
                &h.mint_out.0.to_string()[..6],
                h.fee_bps
            );
        }
    }

    // ── 4. MATH VALIDATION: our cpmm_out vs Jupiter venue quote ──
    // Pick the live SOL/USDC pool with the largest reserves and compare our
    // constant-product quote against Jupiter's quote restricted to that venue.
    let amount_in = 1_000_000_000u64; // 1 SOL
    let sol_usdc = live_pools.iter().find(|p| {
        (p.base_mint.0 == wsol && p.quote_mint.0 == usdc)
            || (p.base_mint.0 == usdc && p.quote_mint.0 == wsol)
    });

    let jupiter = JupiterClient::new();
    if jupiter.api_keys.is_empty() {
        warn!("JUPITER_API_KEY not set — skipping math-vs-Jupiter comparison");
    } else if let Some(pool) = sol_usdc {
        // Our quote: SOL -> USDC.
        let (reserve_in, reserve_out) = if pool.base_mint.0 == wsol {
            (pool.reserve_in, pool.reserve_out)
        } else {
            (pool.reserve_out, pool.reserve_in)
        };
        let Some(fee_bps) = pool.fee_bps() else {
            eprintln!("Pool {} has invalid fee, skipping comparison", pool.pool_id);
            return Ok(());
        };
        let gamma = 10_000u128.saturating_sub(fee_bps as u128);
        let Some(ours) = cpmm_out(amount_in as u128, reserve_in, reserve_out, gamma) else {
            eprintln!("CPMM quote overflow for pool {}", pool.pool_id);
            return Ok(());
        };

        let label = match pool.protocol {
            arb_types::DexProtocol::RaydiumV4 => "Raydium",
            arb_types::DexProtocol::OrcaWhirlpool => "Whirlpool",
            _ => "Unknown",
        };

        match jupiter
            .get_quote_dexes(
                &wsol.to_string(),
                &usdc.to_string(),
                amount_in,
                50,
                Some(&[label]),
            )
            .await
        {
            Ok(quote) => {
                let jup_out = quote["outAmount"]
                    .as_str()
                    .and_then(|s| s.parse::<u128>().ok())
                    .unwrap_or(0);
                let diff = if jup_out > 0 && ours > jup_out {
                    (ours - jup_out) as f64 / jup_out as f64 * 100.0
                } else if jup_out > 0 {
                    (jup_out - ours) as f64 / jup_out as f64 * 100.0
                } else {
                    f64::NAN
                };
                info!(
                    "📐 MATH CHECK [{}] 1 SOL → USDC: ours={} jupiter={} diff={:.4}%",
                    label, ours, jup_out, diff
                );
                info!(
                    "   (a small diff is expected: Jupiter includes its own fee tier / \
                     account state; a LARGE diff flags a math or reserve-decoding bug)"
                );
            }
            Err(e) => warn!("Jupiter {label} quote failed: {e}"),
        }
    } else {
        warn!("No live SOL/USDC pool found in the registry — cannot run the math check");
    }

    // ── 5. LIVE EDGE ANALYSIS (both directions) ──
    // The question the whole system exists to answer: after the VERIFIED flash
    // fee + market tip + network/priority fee, is there any edge left?
    let tip_lamports: u64 = {
        let oracle = arb_land::TipOracle::new(5_000);
        match oracle.refresh().await {
            Ok(f) => {
                info!("Jito tip floor (p50) = {} lamports", f.p50.0);
                f.p50.0
            }
            Err(e) => {
                warn!("tip floor unavailable ({e}); edge analysis uses 0 tip (optimistic)");
                0
            }
        }
    };
    let base_fee: u64 = 5_000;
    let priority_fee: u64 = 5_000;
    let sf = verified.flash_loan_fee_sf.value;

    let legs_of = |p: &arb_types::PoolState| -> (u128, u128) {
        if p.base_mint.0 == wsol {
            (p.reserve_in, p.reserve_out)
        } else {
            (p.reserve_out, p.reserve_in)
        }
    };
    let is_sol_usdc = |p: &arb_types::PoolState| {
        (p.base_mint.0 == wsol && p.quote_mint.0 == usdc)
            || (p.base_mint.0 == usdc && p.quote_mint.0 == wsol)
    };

    let sol_usdc_pools: Vec<&arb_types::PoolState> =
        live_pools.iter().filter(|p| is_sol_usdc(p)).collect();

    // Every ordered pair of venues (a: buy SOL -> USDC, b: sell USDC -> SOL), so
    // the LOW-FEE routes (e.g. Orca 4 bps + Meteora 4 bps) are evaluated too.
    let mut dirs: Vec<(String, Leg, Leg)> = Vec::new();
    for a in &sol_usdc_pools {
        for b in &sol_usdc_pools {
            if a.pool_id == b.pool_id {
                continue;
            }
            let (a_sol, a_usdc) = legs_of(a);
            let (b_sol, b_usdc) = legs_of(b);
            let Some(a_fee) = a.fee_bps() else {
                continue;
            };
            let Some(b_fee) = b.fee_bps() else {
                continue;
            };
            // These are valid because we're in a loop and continue diverges
            let a_id = a.pool_id.0.to_string();
            let b_id = b.pool_id.0.to_string();
            dirs.push((
                format!(
                    "SOL->USDC@{:?}({}bps,{}) ->SOL@{:?}({}bps,{})",
                    a.protocol,
                    a_fee,
                    &a_id[..4.min(a_id.len())],
                    b.protocol,
                    b_fee,
                    &b_id[..4.min(b_id.len())]
                ),
                Leg {
                    r_in: a_sol,
                    r_out: a_usdc,
                    gamma: 10_000u128.saturating_sub(a_fee as u128),
                },
                Leg {
                    r_in: b_usdc,
                    r_out: b_sol,
                    gamma: 10_000u128.saturating_sub(b_fee as u128),
                },
            ));
        }
    }

    struct RouteEdgeResult {
        name: String,
        size: u64,
        gross: i128,
        costs: i128,
        net: i128,
        edge_bps: f64,
        optimum: Option<u64>,
    }

    if !dirs.is_empty() {
        let mut results_summary: Vec<RouteEdgeResult> = Vec::new();
        for (name, l1, l2) in dirs {
            // Probe several loan sizes and report the BEST net edge. A negative
            // best net is the honest answer "no arbitrage"; the numbers are shown
            // so the equation is visible, not hidden behind a boolean.
            let probes: [u64; 6] = [
                1_000_000_000,
                5_000_000_000,
                10_000_000_000,
                25_000_000_000,
                50_000_000_000,
                100_000_000_000,
            ];
            let mut best: Option<(u64, i128, i128, i128, f64)> = None; // (size, gross, costs, net, edge_bps)
            for size in probes {
                let Some(mid) = cpmm_out(size as u128, l1.r_in, l1.r_out, l1.gamma) else {
                    continue;
                };
                let Some(back) = cpmm_out(mid, l2.r_in, l2.r_out, l2.gamma) else {
                    continue;
                };
                let gross = back as i128 - size as i128;
                let flash_fee = calculate_flash_fee_raw(size, sf).unwrap_or(u64::MAX) as i128;
                let costs =
                    flash_fee + tip_lamports as i128 + base_fee as i128 + priority_fee as i128;
                let net = gross - costs;
                let edge_bps = net as f64 / size as f64 * 10_000.0;
                if best.is_none_or(|b| net > b.3) {
                    best = Some((size, gross, costs, net, edge_bps));
                }
            }
            let optimum = optimal_two_pool(&l1, &l2);
            if let Some((size, gross, costs, net, edge_bps)) = best {
                results_summary.push(RouteEdgeResult {
                    name,
                    size,
                    gross,
                    costs,
                    net,
                    edge_bps,
                    optimum,
                });
            }
        }

        // Sort routes by net edge descending (most profitable first)
        results_summary.sort_by(|a, b| {
            b.edge_bps
                .partial_cmp(&a.edge_bps)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        info!("\n================================================================================");
        info!("📊 MULTI-DEX ROUTE PROFITABILITY RANKING (SOL/USDC)");
        info!("================================================================================");
        for res in &results_summary {
            info!(
                "\n💠 EDGE [{}]\n   best probe size = {:.3} SOL   (closed-form optimum = {:?})\n   gross   = {:+} lamports\n   costs   = {} lamports (flash + tip {} + base {} + prio {})\n   NET     = {:+} lamports ({:+.6} SOL)\n   EDGE    = {:+.3} bps\n   verdict = {}",
                res.name,
                res.size as f64 / 1e9,
                res.optimum.map(|o| format!("{:.3} SOL", o as f64 / 1e9)),
                res.gross,
                res.costs,
                tip_lamports,
                base_fee,
                priority_fee,
                res.net,
                res.net as f64 / 1e9,
                res.edge_bps,
                if res.net > 0 {
                    "✅ POSITIVE — executable profitable edge"
                } else {
                    "❌ NEGATIVE — no edge (fees exceed spread)"
                }
            );
        }
    } else {
        warn!("Edge analysis skipped: no two live SOL/USDC venues to compare");
    }

    info!("✅ Validation path complete — no transactions submitted");
    Ok(())
}
