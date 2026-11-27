//! Read-only forensic decomposition of the "0 execution-eligible opportunities"
//! result. NEVER submits transactions.
//!
//! The discovery scan (validate --discover) reported: 10 gross-profitable 2-hop
//! cycles, all rejected by the economic gate with gross profits of only 1-15
//! lamports at dust-sized optimal borrows, while the inter-DEX mid-price spread
//! read 53.46 bps. This harness answers, with the repo's own verified quote
//! math, three questions:
//!
//!   1. Where does the mid-price spread evaporate between mid and the
//!      executable dust quote? (per-venue mid vs executable price, both
//!      directions)
//!   2. What does the gross-profit-vs-size curve look like for each rejected
//!      route, and where is its true optimum? (sweep ladder with exact
//!      protocol-native quotes)
//!   3. What fixed-cost stack (base fee + Jito tip + Kamino flash fee +
//!      priority fee + net-profit hurdle) must gross overcome, and how far is
//!      the best achievable gross from that breakeven?
//!
//! Usage: cargo run -p arb-bot --example forensic_edge_decomposition --locked

use anyhow::{Context, Result};
use arb_bot::verified_config;
use arb_engine::adapters::quote_via_dispatcher;
use arb_ingest::{AccountSnapshot, AccountSnapshotStore, MultiDexRegistry, RpcPool};
use arb_math::economic::{calculate_flash_fee_raw, calculate_net_profit_raw_signed};
use arb_risk::minimum_net_profit_hurdle_lamports;
use arb_types::{Lamports, MintId, PoolState, Slot};
use base64::Engine as _;
use solana_pubkey::Pubkey;
use std::collections::BTreeMap;
use std::str::FromStr;

/// The ten gross-profitable 2-hop cycles reported by the discovery scan at
/// slot 474501741 (route_id, leg1 SOL->USDC pool, leg2 USDC->SOL pool).
/// All legs are mainnet pool addresses copied verbatim from REJECTED_HOP logs.
const REJECTED_ROUTES: &[(&str, &str, &str)] = &[
    (
        "449",
        "5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6",
        "CgqwPLSFfht89pF5RSKGUUMFj5zRxoUt4861w2SkXaqY",
    ),
    (
        "535",
        "3ucNos4NbumPLZNWztqGHNFFgkHeRMBQAVemeeomsUxv",
        "CgqwPLSFfht89pF5RSKGUUMFj5zRxoUt4861w2SkXaqY",
    ),
    (
        "621",
        "7qbRF6YsyGuLUVs6Y1q64bdVrfe4ZcUUz1JRdoVNUJnm",
        "CgqwPLSFfht89pF5RSKGUUMFj5zRxoUt4861w2SkXaqY",
    ),
    (
        "660",
        "83v8iPyZihDEjDdY8RdZddyZNyUtXngz69Lgo9Kt5d6d",
        "7JuwJuNU88gurFnyWeiyGKbFmExMWcmRZntn9imEzdny",
    ),
    (
        "664",
        "83v8iPyZihDEjDdY8RdZddyZNyUtXngz69Lgo9Kt5d6d",
        "CgqwPLSFfht89pF5RSKGUUMFj5zRxoUt4861w2SkXaqY",
    ),
    (
        "837",
        "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE",
        "CgqwPLSFfht89pF5RSKGUUMFj5zRxoUt4861w2SkXaqY",
    ),
    (
        "875",
        "FpCMFDFGYotvufJ7HrFHsWEiiQCGbkLCtwHiDnh7o28Q",
        "7JuwJuNU88gurFnyWeiyGKbFmExMWcmRZntn9imEzdny",
    ),
    (
        "880",
        "FpCMFDFGYotvufJ7HrFHsWEiiQCGbkLCtwHiDnh7o28Q",
        "CgqwPLSFfht89pF5RSKGUUMFj5zRxoUt4861w2SkXaqY",
    ),
    (
        "918",
        "HTvjzsfX3yU6BUodCjZ5vZkUrAxMDTrBs3CJaq43ashR",
        "7JuwJuNU88gurFnyWeiyGKbFmExMWcmRZntn9imEzdny",
    ),
    (
        "923",
        "HTvjzsfX3yU6BUodCjZ5vZkUrAxMDTrBs3CJaq43ashR",
        "CgqwPLSFfht89pF5RSKGUUMFj5zRxoUt4861w2SkXaqY",
    ),
];

/// Borrow sizes probed for every route, in lamports (WSOL raw units).
/// Includes the discovery optima, a dust-to-cap ladder, and sizes above the
/// configured cap purely to expose the shape of the decay curve.
const SWEEP_LADDER: &[u64] = &[
    1_000,
    6_248,
    10_000,
    50_000,
    92_170,
    138_255,
    250_000,
    500_000,
    1_000_000,
    2_000_000,
    5_000_000,
    10_000_000,
    50_000_000,
    500_000_000,
];

fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run())
}

async fn run() -> Result<()> {
    println!("🔍 FORENSIC EDGE DECOMPOSITION (read-only, no transactions)");

    // ── 1. RPC + live pool states (same pipeline as validate --discover) ──
    let rpc = RpcPool::from_env();
    println!("RPC endpoints: {}", rpc.endpoints_count());
    let mut registry = MultiDexRegistry::mainnet_default(rpc.clone());
    let discovery = arb_ingest::PoolDiscovery::new(rpc.clone());
    let cfg = arb_ingest::DiscoveryConfig::default();
    let found = discovery.discover(&cfg).await;
    arb_ingest::register_discovered(&mut registry, &found);
    println!(
        "Registry pools: {} ({} discovered)",
        registry.pool_count(),
        found.len()
    );

    let slot = rpc.get_slot().await.unwrap_or(0);
    println!("Slot: {slot}");
    let live_pools = registry.fetch_live_pool_states(Slot(slot)).await;
    println!("Live priced pools: {}", live_pools.len());

    let mut by_id: BTreeMap<String, &PoolState> = BTreeMap::new();
    for pool in &live_pools {
        by_id.insert(pool.pool_id.0.to_string(), pool);
    }

    // ── 2. Verified economic inputs (fail closed), same contract as validate ──
    let wsol = Pubkey::from_str(arb_tx::WSOL_MINT)?;
    let usdc = Pubkey::from_str(arb_tx::USDC_MINT)?;
    let reserve = Pubkey::from_str(arb_tx::KAMINO_WSOL_RESERVE)?;
    let accounts = rpc
        .get_multiple_accounts(&[reserve.to_string().as_str(), wsol.to_string().as_str()])
        .await
        .context("account fetch failed for verified economic inputs")?;
    let store = AccountSnapshotStore::default();
    for (index, pubkey) in [&reserve, &wsol].into_iter().enumerate() {
        let value = accounts
            .get(index)
            .and_then(|account| account.as_ref())
            .context("verified-input account missing")?;
        let owner = value
            .get("owner")
            .and_then(|v| v.as_str())
            .context("verified-input account has no owner")?;
        let data_b64 = value
            .get("data")
            .and_then(|d| d.get(0))
            .and_then(|d| d.as_str())
            .context("verified-input account has no data")?;
        let data = base64::engine::general_purpose::STANDARD
            .decode(data_b64)
            .context("verified-input base64 decode failed")?;
        store.apply(AccountSnapshot {
            pubkey: *pubkey,
            owner: Pubkey::from_str(owner)?,
            slot,
            write_version: 1,
            is_startup: false,
            provider_id: arb_ingest::HTTP_RPC_PROVIDER_ID,
            bank_id: None,
            generation: 0,
            data: std::sync::Arc::new(data),
        });
    }

    let app_config =
        arb_bot::config::load("config/base.toml").context("failed to load config/base.toml")?;
    let configured_max_tip_lamports = app_config.jito.max_tip_lamports;
    let tip_lamports = {
        let oracle = arb_land::TipOracle::new(5_000);
        match oracle.refresh().await {
            Ok(floor) => {
                let bounded = floor.p75.0.min(configured_max_tip_lamports);
                println!(
                    "Jito tip estimate: p75={} bounded={} lamports",
                    floor.p75.0, bounded
                );
                bounded
            }
            Err(e) => {
                println!(
                    "⚠️ tip floor unavailable ({e}); using configured max {configured_max_tip_lamports}"
                );
                configured_max_tip_lamports
            }
        }
    };

    let verified = verified_config::build_verified_inputs_from_snapshots(
        &store,
        &MintId(wsol),
        &reserve,
        &wsol,
        Lamports(tip_lamports),
        Slot(slot),
        None,
    )
    .map_err(|e| anyhow::anyhow!("verified economic inputs unavailable: {e}"))?;
    let flash_fee_sf = verified.flash_loan_fee_sf.value;
    let base_fee = arb_constants::SOLANA_BASE_FEE_LAMPORTS;
    let min_profit_abs = app_config.strategy.min_net_profit_abs;
    let min_profit_bps = app_config.strategy.min_net_profit_bps;
    println!(
        "Fixed-cost inputs: base_fee={base_fee} tip={tip_lamports} flash_fee_sf={flash_fee_sf} hurdle={min_profit_abs} lamports / {min_profit_bps} bps"
    );

    // Conservative live SOL/USD price for context only (max observed mid).
    let sol_usd = live_pools
        .iter()
        .filter_map(|pool| {
            arb_bot::supervisor::strategy::extract_pool_mid_price(
                pool,
                arb_tx::WSOL_MINT,
                arb_tx::USDC_MINT,
                9,
                6,
            )
            .map(|(price, _)| price)
        })
        .fold(f64::MIN, f64::max);
    println!("Observed max mid price: ${sol_usd:.4}/SOL");

    // ── 3. Per-venue: mid price vs executable dust price, both directions ──
    println!();
    println!("================================================================================");
    println!("SECTION 1: MID PRICE vs EXECUTABLE DUST PRICE (where does the spread go?)");
    println!("================================================================================");
    let mut forensic_pools: Vec<&str> = Vec::new();
    for (_, leg1, leg2) in REJECTED_ROUTES {
        for leg in [leg1, leg2] {
            if !forensic_pools.contains(leg) {
                forensic_pools.push(*leg);
            }
        }
    }
    for address in &forensic_pools {
        let Some(pool) = by_id.get(*address) else {
            println!("\n[{}] NOT PRICED in this snapshot", &(*address)[..8]);
            continue;
        };
        let mid = arb_bot::supervisor::strategy::extract_pool_mid_price(
            pool,
            arb_tx::WSOL_MINT,
            arb_tx::USDC_MINT,
            9,
            6,
        )
        .map(|(price, _)| price);
        let gap = |exec: f64| mid.map(|m| (m - exec) / exec * 10_000.0);
        // Executable price from exact protocol-native quotes at 0.001 SOL —
        // large enough that USDC's 6-decimal granularity (< 0.2 bps) does not
        // masquerade as a spread, unlike 1k-lamport dust probes.
        let dust_probe_lamports = 1_000_000u64;
        let sol_to_usdc = quote_via_dispatcher(pool, dust_probe_lamports, pool.base_mint.0 == wsol)
            .ok()
            .map(|quote| executable_price_sol_to_usdc(dust_probe_lamports, quote.amount_out_raw));
        let usdc_to_sol = quote_via_dispatcher(pool, dust_probe_lamports, pool.base_mint.0 == usdc)
            .ok()
            .map(|quote| executable_price_usdc_to_sol(dust_probe_lamports, quote.amount_out_raw));
        println!(
            "\n[{}] {:?} kind={:?}",
            &address[..8],
            pool.protocol,
            pool.kind
        );
        match mid {
            Some(price) => println!("  mid (active-bin/spot) price       = {price:.4} USDC/SOL"),
            None => println!("  mid price: unavailable"),
        }
        match sol_to_usdc {
            Some(price) => println!(
                "  executable SOL->USDC @0.001 SOL   = {price:.4} USDC/SOL (mid-vs-exec {:+.1} bps)",
                gap(price).unwrap_or(f64::NAN)
            ),
            None => println!("  executable SOL->USDC quote: REJECTED"),
        }
        match usdc_to_sol {
            Some(price) => println!(
                "  executable USDC->SOL @0.001 SOL   = {price:.4} USDC/SOL (mid-vs-exec {:+.1} bps)",
                gap(price).unwrap_or(f64::NAN)
            ),
            None => println!("  executable USDC->SOL quote: REJECTED"),
        }
        if let Some(dlmm) = &pool.dlmm {
            print_dlmm_bin_ladder(dlmm, pool, &wsol, &usdc);
        }
    }

    // ── 4. Route sweep: gross-vs-size curve with the full cost ledger ──
    println!();
    println!("================================================================================");
    println!("SECTION 2: GROSS-PROFIT vs BORROW SIZE (exact quotes, full cost ledger)");
    println!("================================================================================");
    struct RouteBest {
        best_gross: i128,
        best_size: u64,
        best_net: i128,
        bins_crossed_at_best: (Option<u32>, Option<u32>),
        breakeven_gross: u128,
    }
    let mut route_bests: Vec<(&str, RouteBest)> = Vec::new();
    for (route, leg1_str, leg2_str) in REJECTED_ROUTES {
        let (Some(pool1), Some(pool2)) = (by_id.get(*leg1_str), by_id.get(*leg2_str)) else {
            println!("\nROUTE {route}: a leg is unpriced in this snapshot — skipped");
            continue;
        };
        // Leg 1 sells WSOL for USDC; leg 2 buys WSOL back with USDC.
        let leg1_a_to_b = pool1.base_mint.0 == wsol;
        let leg2_a_to_b = pool2.base_mint.0 == usdc;
        println!(
            "\nROUTE {route}: {} {:?} -> {} {:?}",
            &leg1_str[..8],
            pool1.protocol,
            &leg2_str[..8],
            pool2.protocol
        );
        println!(
            "  {:>12} {:>16} {:>16} {:>8} {:>8} {:>6} {:>6} {:>8} {:>10} {:>12}",
            "borrow(lp)",
            "out1(USDC raw)",
            "back(WSOL raw)",
            "gross(lp)",
            "grossbps",
            "bins1",
            "bins2",
            "flashfee",
            "net(lp)",
            "verdict"
        );
        let mut best: Option<RouteBest> = None;
        for &size in SWEEP_LADDER {
            let Ok(quote1) = quote_via_dispatcher(pool1, size, leg1_a_to_b) else {
                println!("  {size:>12} leg1 quote rejected");
                continue;
            };
            let Ok(quote2) = quote_via_dispatcher(pool2, quote1.amount_out_raw, leg2_a_to_b) else {
                println!(
                    "  {size:>12} leg2 quote rejected (out1={})",
                    quote1.amount_out_raw
                );
                continue;
            };
            let gross = i128::from(quote2.amount_out_raw) - i128::from(size);
            let gross_bps = gross as f64 / size as f64 * 10_000.0;
            let Ok(flash_fee) = calculate_flash_fee_raw(size, flash_fee_sf) else {
                continue;
            };
            let route_protocols = [pool1.protocol, pool2.protocol];
            let estimated_cu = arb_tx::estimate_route_cu(true, &route_protocols, 1);
            let cu_limit = arb_tx::calculate_dynamic_cu_limit(estimated_cu);
            let max_priority_share_bps = app_config.strategy.priority_fee_max_share_bps;
            let cu_price = arb_tx::cap_cu_price_by_profit(
                u64::try_from(gross.max(0)).unwrap_or(0),
                cu_limit,
                100_000,
                max_priority_share_bps,
            );
            let priority_fee = arb_tx::priority_fee_lamports(cu_price, cu_limit);
            let Ok(net) = calculate_net_profit_raw_signed(
                quote2.amount_out_raw,
                size,
                flash_fee,
                0,
                0,
                0,
                0,
                Lamports(base_fee),
                Lamports(priority_fee),
                Lamports(tip_lamports),
                1,
                1,
            ) else {
                println!("  {size:>12} net-profit ledger overflow");
                continue;
            };
            let hurdle = minimum_net_profit_hurdle_lamports(
                Lamports(size),
                Lamports(min_profit_abs),
                min_profit_bps,
            )
            .as_u64();
            let verdict = if gross <= 0 {
                "gross<=0"
            } else if net <= 0 {
                "costs>gross"
            } else if net < i128::from(hurdle) {
                "below-hurdle"
            } else {
                "ELIGIBLE"
            };
            println!(
                "  {size:>12} {:>16} {:>16} {gross:>8} {gross_bps:>8.2} {:>6} {:>6} {flash_fee:>8} {net:>10} {verdict:>12}",
                quote1.amount_out_raw,
                quote2.amount_out_raw,
                quote1
                    .dlmm_bins_crossed
                    .map_or("-".to_string(), |n| n.to_string()),
                quote2
                    .dlmm_bins_crossed
                    .map_or("-".to_string(), |n| n.to_string()),
            );
            let exceeds = best.as_ref().is_none_or(|b| gross > b.best_gross);
            if exceeds {
                let breakeven_gross = u128::from(flash_fee)
                    + u128::from(base_fee)
                    + u128::from(tip_lamports)
                    + u128::from(priority_fee)
                    + u128::from(hurdle);
                best = Some(RouteBest {
                    best_gross: gross,
                    best_size: size,
                    best_net: net,
                    bins_crossed_at_best: (quote1.dlmm_bins_crossed, quote2.dlmm_bins_crossed),
                    breakeven_gross,
                });
            }
        }
        if let Some(best) = best {
            println!(
                "  -> best gross {:+} lamports at borrow {} ({:.6} SOL); net {:+}; breakeven gross ≈ {} lamports; shortfall ×{:.0}; bins crossed at best = {:?}",
                best.best_gross,
                best.best_size,
                best.best_size as f64 / 1e9,
                best.best_net,
                best.breakeven_gross,
                best.breakeven_gross as f64 / best.best_gross.max(1) as f64,
                best.bins_crossed_at_best,
            );
            route_bests.push((route, best));
        }
    }

    // ── 5. Verdict: the binding constraint, quantified ──
    println!();
    println!("================================================================================");
    println!("SECTION 3: BINDING CONSTRAINT SUMMARY");
    println!("================================================================================");
    let (_, global_best) = route_bests
        .iter()
        .max_by_key(|(_, best)| best.best_gross)
        .context("no priced route produced a candidate")?;
    println!(
        "Global best gross profit across all {} priced rejected routes: {:+} lamports (borrow {} = {:.6} SOL ≈ ${:.4})",
        route_bests.len(),
        global_best.best_gross,
        global_best.best_size,
        global_best.best_size as f64 / 1e9,
        global_best.best_size as f64 / 1e9 * sol_usd
    );
    println!(
        "Required gross to clear the full fixed-cost stack + hurdle at that size: {} lamports (shortfall factor ×{:.0})",
        global_best.breakeven_gross,
        global_best.breakeven_gross as f64 / global_best.best_gross.max(1) as f64
    );
    println!(
        "Fixed stack at that size: base {base_fee} + tip {tip_lamports} + flash fee + priority; hurdle {min_profit_abs}/{min_profit_bps}bps"
    );
    println!("✅ Forensic decomposition complete — no transactions submitted");
    Ok(())
}

/// Executable SOL->USDC price in USDC/SOL from a dust quote.
fn executable_price_sol_to_usdc(sol_lamports: u64, usdc_raw: u64) -> f64 {
    (usdc_raw as f64 / 1e6) / (sol_lamports as f64 / 1e9)
}

/// Executable USDC->SOL price in USDC/SOL from a dust quote.
fn executable_price_usdc_to_sol(usdc_raw: u64, sol_lamports: u64) -> f64 {
    (usdc_raw as f64 / 1e6) / (sol_lamports as f64 / 1e9)
}

/// Print the DLMM bin ladder around the active bin with the human price of
/// each bin. For a pool whose base (token X) is WSOL: a WSOL->USDC swap
/// (X->Y) consumes amount_y at/above active; a USDC->WSOL swap (Y->X)
/// consumes amount_x at/below active. The mapping inverts when the pool's
/// base is USDC.
fn print_dlmm_bin_ladder(
    dlmm: &arb_types::DlmmState,
    pool: &PoolState,
    wsol: &Pubkey,
    usdc: &Pubkey,
) {
    let base_is_wsol = pool.base_mint.0 == *wsol;
    let base_is_usdc = pool.base_mint.0 == *usdc;
    if !base_is_wsol && !base_is_usdc {
        return;
    }
    let dec_x = if base_is_wsol { 9 } else { 6 };
    let dec_y = if base_is_wsol { 6 } else { 9 };
    let step_factor = 1.0 + f64::from(dlmm.bin_step_bps) / 10_000.0;
    println!(
        "  DLMM bins: active_id={} bin_step={}bps bins_cached={} support_limit_order={}",
        dlmm.active_bin_id,
        dlmm.bin_step_bps,
        dlmm.bins.len(),
        dlmm.support_limit_order
    );
    // The cached bins span a bin_id range; print around the ACTIVE bin, not
    // the lowest cached id, plus the nearest funded bin on each side. For a
    // WSOL-base pool: SOL->USDC consumes amount_y at/below active; USDC->SOL
    // consumes amount_x at/above active.
    let active_index = dlmm
        .bins
        .iter()
        .position(|bin| bin.bin_id == dlmm.active_bin_id);
    match active_index {
        Some(active_index) => {
            println!(
                "  {:>8} {:>10} {:>16} {:>16}",
                "bin_id", "Δprice%", "amt_x(human)", "amt_y(human)"
            );
            for bin in dlmm.bins
                [active_index.saturating_sub(2)..(active_index + 7).min(dlmm.bins.len())]
                .iter()
            {
                let bin_delta_pct =
                    (step_factor.powi(bin.bin_id - dlmm.active_bin_id) - 1.0) * 100.0;
                let amt_x = bin.amount_x as f64 / 10f64.powi(dec_x);
                let amt_y = bin.amount_y as f64 / 10f64.powi(dec_y);
                println!(
                    "  {:>8} {:>9.3}% {:>16.4} {:>16.4}",
                    bin.bin_id, bin_delta_pct, amt_x, amt_y
                );
            }
        }
        None => println!(
            "  active bin {} NOT present in cached bins (stale/partial bin hydration)",
            dlmm.active_bin_id
        ),
    }
    let x_above = dlmm
        .bins
        .iter()
        .filter(|b| b.bin_id >= dlmm.active_bin_id)
        .map(|b| b.amount_x)
        .sum::<u128>();
    let y_below = dlmm
        .bins
        .iter()
        .filter(|b| b.bin_id <= dlmm.active_bin_id)
        .map(|b| b.amount_y)
        .sum::<u128>();
    let nearest_x_above = dlmm
        .bins
        .iter()
        .filter(|b| b.bin_id >= dlmm.active_bin_id && b.amount_x > 0)
        .map(|b| b.bin_id - dlmm.active_bin_id)
        .min();
    let nearest_y_below = dlmm
        .bins
        .iter()
        .filter(|b| b.bin_id <= dlmm.active_bin_id && b.amount_y > 0)
        .map(|b| dlmm.active_bin_id - b.bin_id)
        .min();
    println!(
        "  cached-side depth: total x (SOL side) at/above active = {:.4}, total y (USDC side) at/below active = {:.4}",
        x_above as f64 / 10f64.powi(dec_x),
        y_below as f64 / 10f64.powi(dec_y)
    );
    println!(
        "  nearest funded bin: x-side +{:?} bins above active, y-side -{:?} bins below active",
        nearest_x_above, nearest_y_below
    );
}
