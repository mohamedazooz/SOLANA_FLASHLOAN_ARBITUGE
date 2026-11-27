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

use anyhow::{Context, Result};
use arb_engine::MarketGraph;
use arb_engine::adapters::quote_via_dispatcher;
use arb_ingest::{AccountSnapshot, AccountSnapshotStore, MultiDexRegistry, RpcPool};
use arb_math::economic::calculate_flash_fee_raw;
use arb_math::{Leg, optimal_two_pool};
use arb_risk::minimum_net_profit_hurdle_lamports;
use arb_tx::JupiterClient;
use arb_types::{Lamports, MintId, PoolKind, PoolState, Slot, TokenAmount};
use base64::Engine as _;
use solana_pubkey::Pubkey;
use std::str::FromStr;
use tracing::{info, warn};

use arb_bot::verified_config;

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
    for skipped in registry.last_hydration_skips() {
        warn!(
            "POOL_HYDRATION_REJECTED protocol={:?} pool={} reason={}",
            skipped.protocol, skipped.pool_id, skipped.reason
        );
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

    // Implied mid price per SOL/USDC pool + inter-DEX spread. Uses protocol-native
    // spot mid-prices (CLMM sqrt_price, DLMM active_bin_id, CPMM/V4 reserves)
    // instead of dividing raw vault balances on concentrated-liquidity pools.
    let wsol_str_mid = arb_tx::WSOL_MINT;
    let usdc_str_mid = arb_tx::USDC_MINT;
    let mut priced: Vec<(&str, f64)> = Vec::new();
    for p in &live_pools {
        let Some((price, venue)) = arb_bot::supervisor::strategy::extract_pool_mid_price(
            p,
            wsol_str_mid,
            usdc_str_mid,
            9,
            6,
        ) else {
            continue;
        };
        info!("  💲 {venue} implied price = {price:.4} USDC/SOL");
        priced.push((venue, price));
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
            // RPC snapshots are bootstrap evidence, not a live stream.
            provider_id: arb_ingest::HTTP_RPC_PROVIDER_ID,
            bank_id: None,
            generation: 0,
            data: std::sync::Arc::new(data),
        });
    }

    let app_config = Some(
        arb_bot::config::load("config/base.toml")
            .context("failed to load config/base.toml for read-only validation")?,
    );
    let configured_max_tip_lamports = app_config
        .as_ref()
        .map(|config| config.jito.max_tip_lamports)
        .unwrap_or(u64::MAX);

    // Resolve one bounded discovery tip estimate before constructing the graph
    // so the canonical cycle gate and explicit edge report price the same
    // liability. On oracle failure, use the configured ceiling, never the
    // protocol minimum; dispatch still reprices from fresh landing data.
    let (tip_lamports, tip_floor_available) = {
        let oracle = arb_land::TipOracle::new(5_000);
        match oracle.refresh().await {
            Ok(f) => {
                let bounded_tip = f.p75.0.min(configured_max_tip_lamports);
                info!(
                    "Jito tip floor (p75) = {} lamports; bounded estimate = {} lamports",
                    f.p75.0, bounded_tip
                );
                (bounded_tip, true)
            }
            Err(e) => {
                warn!(
                    "tip floor unavailable ({e}); using configured maximum {configured_max_tip_lamports} lamports for conservative diagnostics"
                );
                (configured_max_tip_lamports, false)
            }
        }
    };

    let verified = match verified_config::build_verified_inputs_from_snapshots(
        &store,
        &MintId(wsol),
        &reserve,
        &wsol,
        Lamports(tip_lamports),
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
    // Match the read-only scan size to the same caps used by the running bot.
    // The previous 1..100 SOL stress ladder was unrelated to this wallet's
    // 0.02 SOL / USD caps and could make the diagnostic look artificially bad.
    let config_max_trade_usd = app_config
        .as_ref()
        .map(|c| c.risk.max_trade_size_usd)
        .unwrap_or(1.5);
    let max_trade_usd = positive_env_f64("MAX_TRADE_SIZE_USD", config_max_trade_usd)?;
    let max_flash_borrow_usd = positive_env_f64("MAX_FLASH_BORROW_USD", max_trade_usd)?;
    let max_trade_sol = positive_env_f64("MAX_TRADE_SIZE_SOL", 1.0)?;
    let portfolio_cap_sol =
        optional_positive_env_f64("PORTFOLIO_CAPITAL_SOL")?.unwrap_or(f64::INFINITY);
    let mut spot_prices: Vec<f64> = priced
        .iter()
        .map(|(_, price)| *price)
        .filter(|price| price.is_finite() && *price > 0.0)
        .collect();
    spot_prices.sort_by(f64::total_cmp);
    // Use the highest observed live pool price for a conservative USD-to-SOL cap.
    let live_max_sol_usd = spot_prices.iter().copied().reduce(f64::max);
    let sol_usd_price = live_max_sol_usd
        .or(optional_positive_env_f64("SOL_USD_PRICE")?)
        .ok_or_else(|| {
            anyhow::anyhow!("No fresh SOL/USD price available; refusing to ignore USD trade caps")
        })?;
    let usd_cap_sol = max_trade_usd.min(max_flash_borrow_usd) / sol_usd_price;
    let max_probe_sol = max_trade_sol.min(portfolio_cap_sol).min(usd_cap_sol);
    let max_probe_lamports_f64 = max_probe_sol * 1_000_000_000.0;
    if !max_probe_lamports_f64.is_finite()
        || max_probe_lamports_f64 <= 0.0
        || max_probe_lamports_f64 >= u64::MAX as f64
    {
        anyhow::bail!("Configured trade caps resolve to an invalid probe size");
    }
    let max_probe_lamports = max_probe_lamports_f64.floor() as u64;
    if max_probe_lamports == 0 {
        anyhow::bail!("Configured trade caps resolve below one lamport");
    }
    let min_profit_abs = app_config
        .as_ref()
        .map(|c| c.strategy.min_net_profit_abs)
        .unwrap_or(100_000);
    let min_profit_bps = app_config
        .as_ref()
        .map(|c| c.strategy.min_net_profit_bps)
        .unwrap_or(50);
    let math_check_slippage_bps = u16::try_from(
        app_config
            .as_ref()
            .map(|c| c.strategy.hop_slippage_bps)
            .unwrap_or(10),
    )
    .context("configured hop slippage does not fit u16")?;
    info!(
        "Execution-sized probe cap: {:.6} SOL (${:.4} at ${:.4}/SOL); min net hurdle={} lamports / {} bps; slippage={} bps",
        max_probe_lamports as f64 / 1e9,
        max_probe_lamports as f64 / 1e9 * sol_usd_price,
        sol_usd_price,
        min_profit_abs,
        min_profit_bps,
        math_check_slippage_bps
    );
    let min_profit = app_config
        .as_ref()
        .map(|c| c.strategy.min_net_profit_abs)
        .unwrap_or(arb_constants::DEFAULT_MIN_PROFIT_LAMPORTS);
    let mut graph = MarketGraph::new(vec![MintId(wsol)]);
    graph.set_economic_config(
        verified.clone().into_economic_config(
            TokenAmount(min_profit),
            app_config
                .as_ref()
                .map(|c| c.strategy.hop_slippage_bps)
                .unwrap_or(arb_constants::DEFAULT_HOP_SLIPPAGE_BPS),
        ),
    );
    graph.build_from_pools(&live_pools);
    let edge_count = graph.edges_by_from.values().map(|v| v.len()).sum::<usize>();
    info!("MarketGraph built: {edge_count} edges");

    let (opportunities, cycle_stats) = graph.find_base_cycles_with_stats(slot, max_probe_lamports);
    info!(
        "Cycle scan: {} base anchor(s), {} gross-profitable 2-hop, {} gross-profitable 3-hop, {} gross-negative, {} quote-rejected, {} rejected by verified economic gate, {} execution-eligible",
        cycle_stats.base_anchors,
        cycle_stats.two_hop_gross_profitable,
        cycle_stats.three_hop_gross_profitable,
        cycle_stats.gross_negative,
        cycle_stats.quote_rejected,
        cycle_stats.gate_rejected,
        cycle_stats.opportunities,
    );
    for rejected in &cycle_stats.rejected_cycles {
        info!(
            "REJECTED_CYCLE route={} base={:?} borrow_raw={} final_raw={} gross_raw={} hop_amounts_raw={:?} quote_dispatch_error={:?}",
            rejected.route_id,
            rejected.base_mint,
            rejected.borrow_amount_raw,
            rejected.expected_final_amount_raw,
            rejected.gross_profit_raw,
            rejected.hop_amounts_raw,
            rejected.quote_dispatch_error,
        );
        for (index, hop) in rejected.hops.iter().enumerate() {
            info!(
                "  REJECTED_HOP route={} leg={} pool={:?} dex={:?} kind={:?} direction={:?}->{:?} dex_fee_bps={} amount_in_raw={:?} amount_out_raw={:?} dex_fee_paid_raw={:?} transfer_fee_paid_raw={:?}",
                rejected.route_id,
                index + 1,
                hop.pool_id,
                hop.protocol,
                hop.kind,
                hop.from_mint,
                hop.to_mint,
                hop.fee_bps,
                hop.amount_in_raw,
                hop.amount_out_raw,
                hop.dex_fee_paid_raw,
                hop.transfer_fee_paid_raw,
            );
        }
    }
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

    // ── 4. MATH VALIDATION: native adapter quote vs Jupiter same-pool quote ──
    // Pick the live SOL/USDC pool with the largest reserves and compare the
    // protocol-native quote (including CLMM ticks when applicable) against Jupiter.
    let amount_in = max_probe_lamports;
    let sol_usdc = live_pools
        .iter()
        .filter(|p| {
            (p.base_mint.0 == wsol && p.quote_mint.0 == usdc)
                || (p.base_mint.0 == usdc && p.quote_mint.0 == wsol)
        })
        .max_by_key(|p| p.reserve_in.saturating_mul(p.reserve_out));

    let jupiter = if let Some(cfg) = &app_config {
        JupiterClient::with_endpoint_and_timeout(
            &cfg.jupiter.base_url,
            std::time::Duration::from_millis(cfg.jupiter.timeout_ms),
        )
    } else {
        JupiterClient::new()
    };
    if jupiter.api_keys.is_empty() {
        warn!("JUPITER_API_KEY not set — skipping math-vs-Jupiter comparison");
    } else if let Some(pool) = sol_usdc {
        // Our quote: SOL -> USDC via protocol-native dispatcher (CLMM/DLMM/CPMM).
        let a_to_b = pool.base_mint.0 == wsol;
        match quote_via_dispatcher(pool, amount_in, a_to_b) {
            Ok(quote_res) => {
                let ours = u128::from(quote_res.amount_out_raw);
                let label = match pool.protocol {
                    arb_types::DexProtocol::RaydiumV4 => "Raydium",
                    arb_types::DexProtocol::OrcaWhirlpool => "Whirlpool",
                    arb_types::DexProtocol::MeteoraDlmm => "Meteora DLMM",
                    arb_types::DexProtocol::RaydiumCpmm => "Raydium CPMM",
                    _ => "Unknown",
                };

                match jupiter
                    .get_quote_dexes(
                        &wsol.to_string(),
                        &usdc.to_string(),
                        amount_in,
                        math_check_slippage_bps,
                        Some(&[label]),
                    )
                    .await
                {
                    Ok(quote) => {
                        let expected_pool = pool.pool_id.0.to_string();
                        let same_single_pool = quote["routePlan"].as_array().is_some_and(|plan| {
                            plan.len() == 1
                                && plan[0]["swapInfo"]["ammKey"]
                                    .as_str()
                                    .is_some_and(|key| key == expected_pool)
                        });
                        if !same_single_pool {
                            warn!(
                                "Jupiter venue reference is not the same single pool (target={}); comparison intentionally skipped",
                                expected_pool
                            );
                        } else {
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
                                "📐 MATH CHECK [{}] {:.6} SOL → USDC: ours={} (fee_raw={} impact={}bps) jupiter={} diff={:.4}%",
                                label,
                                amount_in as f64 / 1e9,
                                ours,
                                quote_res.fees_breakdown.total_fee,
                                quote_res.price_impact.0,
                                jup_out,
                                diff
                            );
                            info!(
                                "   (a small diff is expected: Jupiter includes its own fee tier / \
                             account state; a LARGE diff flags a math or reserve-decoding bug)"
                            );
                        }
                    }
                    Err(e) => warn!("Jupiter {label} quote failed: {e}"),
                }
            }
            Err(e) => {
                warn!(
                    "Skipping math comparison for {:?} pool {}: protocol-native quote failed ({e})",
                    pool.protocol, pool.pool_id
                );
            }
        }
    } else {
        warn!("No live SOL/USDC pool found in the registry — cannot run the math check");
    }

    // ── 5. LIVE EDGE ANALYSIS (both directions) ──
    // The question the whole system exists to answer: after the VERIFIED flash
    // fee + market tip + network/priority fee, is there any edge left?
    // Solana base fee is charged per transaction signature. This route builder
    // uses one payer signature and no signature precompiles.
    let base_fee = arb_constants::SOLANA_BASE_FEE_LAMPORTS;
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

    // This reserve-based cross-check uses the constant-product formula only.
    // Never feed Orca/CLMM or Meteora/DLMM reserves into cpmm_out: those curves
    // require their native tick/bin quote engines. The exact MarketGraph scan
    // above is authoritative for every supported venue.
    let all_sol_usdc_pool_count = live_pools.iter().filter(|p| is_sol_usdc(p)).count();
    let sol_usdc_pools: Vec<&arb_types::PoolState> = live_pools
        .iter()
        .filter(|p| {
            is_sol_usdc(p)
                && p.kind == arb_types::PoolKind::Cpmm
                && matches!(
                    p.protocol,
                    arb_types::DexProtocol::RaydiumV4 | arb_types::DexProtocol::RaydiumCpmm
                )
        })
        .collect();
    info!(
        "CPMM-only reserve cross-check includes {}/{} SOL/USDC pools; CLMM/DLMM and non-Raydium curves excluded",
        sol_usdc_pools.len(),
        all_sol_usdc_pool_count
    );

    // Every ordered pair of venues (a: buy SOL -> USDC, b: sell USDC -> SOL),
    // priced with each venue's exact protocol math (CLMM ticks, DLMM bins, CPMM).
    let mut dirs: Vec<(String, &PoolState, &PoolState)> = Vec::new();
    for a in &sol_usdc_pools {
        for b in &sol_usdc_pools {
            if a.pool_id == b.pool_id {
                continue;
            }
            let Some(a_fee) = a.fee_bps() else {
                continue;
            };
            let Some(b_fee) = b.fee_bps() else {
                continue;
            };
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
                *a,
                *b,
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
        hurdle: u64,
        priority_fee: u64,
        impact_bps: u64,
        analytical_seed: Option<u64>,
    }

    if !dirs.is_empty() {
        let mut results_summary: Vec<RouteEdgeResult> = Vec::new();
        for (name, pool_a, pool_b) in dirs {
            let a_to_b_1 = pool_a.base_mint.0 == wsol;
            let a_to_b_2 = pool_b.base_mint.0 == usdc;
            // This closed-form model is only a seed. It uses rounded BPS and
            // does not model every CPMM creator-fee mode; final sizing below
            // always selects from protocol-native exact quotes and full costs.
            let analytical_seed = if pool_a.kind == PoolKind::Cpmm && pool_b.kind == PoolKind::Cpmm
            {
                match (pool_a.fee_bps(), pool_b.fee_bps()) {
                    (Some(a_fee), Some(b_fee)) => {
                        let (a_sol, a_usdc) = legs_of(pool_a);
                        let (b_sol, b_usdc) = legs_of(pool_b);
                        let l1 = Leg {
                            r_in: a_sol,
                            r_out: a_usdc,
                            gamma: 10_000u128 - u128::from(a_fee),
                        };
                        let l2 = Leg {
                            r_in: b_usdc,
                            r_out: b_sol,
                            gamma: 10_000u128 - u128::from(b_fee),
                        };
                        optimal_two_pool(&l1, &l2)
                            .filter(|size| *size > 0 && *size <= max_probe_lamports)
                    }
                    _ => None,
                }
            } else {
                None
            };

            let mut probes: Vec<u64> = [10_000u64, 7_500, 5_000, 2_500, 1_000, 500, 100, 10, 1]
                .into_iter()
                .map(|fraction_bps| {
                    (u128::from(max_probe_lamports) * u128::from(fraction_bps) / 10_000) as u64
                })
                .filter(|size| *size > 0)
                .collect();
            if let Some(seed) = analytical_seed {
                probes.push(seed);
            }
            probes.sort_unstable();
            probes.dedup();

            let mut best: Option<RouteEdgeResult> = None;
            let evaluate = |size: u64| -> Option<RouteEdgeResult> {
                if size == 0 || size > max_probe_lamports {
                    return None;
                }
                let q1 = quote_via_dispatcher(pool_a, size, a_to_b_1).ok()?;
                if q1.amount_out_raw == 0 {
                    return None;
                }
                let q2 = quote_via_dispatcher(pool_b, q1.amount_out_raw, a_to_b_2).ok()?;
                let back = q2.amount_out_raw;
                let gross = i128::from(back) - i128::from(size);
                let retained_1 = 10_000u64.saturating_sub(u64::from(q1.price_impact.0));
                let retained_2 = 10_000u64.saturating_sub(u64::from(q2.price_impact.0));
                let route_impact_bps = 10_000u64.saturating_sub((retained_1 * retained_2) / 10_000);
                let route_protocols = [pool_a.protocol, pool_b.protocol];
                let estimated_cu = arb_tx::estimate_route_cu(true, &route_protocols, 1);
                let cu_limit = arb_tx::calculate_dynamic_cu_limit(estimated_cu);
                let max_priority_share_bps = app_config
                    .as_ref()
                    .map(|config| config.strategy.priority_fee_max_share_bps)
                    .unwrap_or(500);
                let cu_price = arb_tx::cap_cu_price_by_profit(
                    u64::try_from(gross.max(0)).unwrap_or(0),
                    cu_limit,
                    100_000,
                    max_priority_share_bps,
                );
                let priority_fee = arb_tx::priority_fee_lamports(cu_price, cu_limit);
                // A fee-calculation error or signed-PnL overflow invalidates this
                // candidate. Never turn either error into a fabricated maximum fee
                // or a synthetic i128::MIN loss that can contaminate ranking.
                let flash_fee = calculate_flash_fee_raw(size, sf).ok()?;
                let net = arb_math::economic::calculate_net_profit_raw_signed(
                    back,
                    size,
                    flash_fee,
                    0, // DEX fees are reflected in the protocol-native outputs
                    0, // no separately charged Token-2022 transfer fees in this diagnostic
                    0, // no account creation cost included
                    0, // no additional verified raw costs
                    Lamports(base_fee),
                    Lamports(priority_fee),
                    Lamports(tip_lamports),
                    1,
                    1,
                )
                .ok()?;
                let costs = gross.checked_sub(net)?;
                let edge_bps = net as f64 / size as f64 * 10_000.0;
                let hurdle = minimum_net_profit_hurdle_lamports(
                    Lamports(size),
                    Lamports(min_profit_abs),
                    min_profit_bps,
                )
                .as_u64();
                Some(RouteEdgeResult {
                    name: name.clone(),
                    size,
                    gross,
                    costs,
                    net,
                    edge_bps,
                    hurdle,
                    priority_fee,
                    impact_bps: route_impact_bps,
                    analytical_seed,
                })
            };

            for size in probes {
                if let Some(candidate) = evaluate(size)
                    && best
                        .as_ref()
                        .is_none_or(|current| candidate.net > current.net)
                {
                    best = Some(candidate);
                }
            }

            // The closed-form CPMM result is not accepted as the optimum. Refine
            // around the best exact-quote candidate in four bounded scales.
            // This preserves protocol-specific fees/rounding and catches the
            // optimum between the coarse ladder's six original points.
            if pool_a.kind == PoolKind::Cpmm && pool_b.kind == PoolKind::Cpmm {
                for round in 0..4u32 {
                    let Some(center) = best.as_ref().map(|candidate| candidate.size) else {
                        break;
                    };
                    let span = (max_probe_lamports / 10u64.pow(round + 1)).max(1);
                    let step = (span / 16).max(1);
                    let start = center.saturating_sub(span).max(1);
                    let end = center.saturating_add(span).min(max_probe_lamports);
                    let mut size = start;
                    loop {
                        if let Some(candidate) = evaluate(size)
                            && best
                                .as_ref()
                                .is_none_or(|current| candidate.net > current.net)
                        {
                            best = Some(candidate);
                        }
                        if size >= end {
                            break;
                        }
                        size = size.saturating_add(step).min(end);
                    }
                }
            }
            if let Some(candidate) = best {
                results_summary.push(candidate);
            }
        }

        // Maximize absolute net PnL under the configured cap; edge bps is diagnostic only.
        results_summary.sort_by_key(|result| std::cmp::Reverse(result.net));

        info!("\n================================================================================");
        info!(
            "📊 MULTI-DEX NET-PnL RANKING (SOL/USDC, PROTOCOL-NATIVE EXACT QUOTES; bounded size refinement)"
        );
        info!("================================================================================");
        for res in &results_summary {
            info!(
                "\n💠 EDGE [{}]\n   best exact-quote probe size = {:.6} SOL (analytical CPMM seed, heuristic only = {:?})\n   curve impact = {} bps\n   gross   = {:+} lamports\n   costs   = {} lamports (flash + tip {} + base {} + route-prio {})\n   NET     = {:+} lamports ({:+.9} SOL)\n   hurdle  = {} lamports (max absolute floor / {} bps)\n   EDGE    = {:+.3} bps\n   verdict = {}",
                res.name,
                res.size as f64 / 1e9,
                res.analytical_seed
                    .map(|seed| format!("{:.6} SOL", seed as f64 / 1e9)),
                res.impact_bps,
                res.gross,
                res.costs,
                tip_lamports,
                base_fee,
                res.priority_fee,
                res.net,
                res.net as f64 / 1e9,
                res.hurdle,
                min_profit_bps,
                res.edge_bps,
                route_verdict(res.gross, res.net, res.hurdle, tip_floor_available)
            );
        }
    } else {
        warn!("Edge analysis skipped: no two live SOL/USDC venues to compare");
    }

    info!("✅ Validation path complete — no transactions submitted");
    Ok(())
}

/// Explain whether a candidate lost before costs or only after costs.
/// A negative gross route can never be described as fees exceeding the spread.
fn route_verdict(gross: i128, net: i128, hurdle: u64, tip_floor_available: bool) -> &'static str {
    if gross <= 0 {
        "❌ GROSS ROUTE LOSS — protocol quotes return less than the input before execution costs"
    } else if net <= 0 {
        "❌ EXECUTION COSTS EXCEED GROSS EDGE — gross route is positive, net is non-positive"
    } else if net < i128::from(hurdle) {
        "⚠️ POSITIVE BUT BELOW CONFIGURED NET-PROFIT FLOOR — reject"
    } else if !tip_floor_available {
        "⚠️ ABOVE NET-PROFIT FLOOR, BUT JITO TIP FLOOR UNAVAILABLE — diagnostic only, not eligible"
    } else {
        "✅ ABOVE CONFIGURED NET-PROFIT FLOOR (still subject to Chronos/landing gate)"
    }
}

fn optional_positive_env_f64(name: &str) -> Result<Option<f64>> {
    match std::env::var(name) {
        Ok(raw) => {
            let value = raw
                .trim()
                .parse::<f64>()
                .map_err(|error| anyhow::anyhow!("{name} must be a positive number: {error}"))?;
            if !value.is_finite() || value <= 0.0 {
                anyhow::bail!("{name} must be finite and greater than zero");
            }
            Ok(Some(value))
        }
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            anyhow::bail!("{name} is not valid Unicode")
        }
    }
}

fn positive_env_f64(name: &str, fallback: f64) -> Result<f64> {
    Ok(optional_positive_env_f64(name)?.unwrap_or(fallback))
}

#[cfg(test)]
mod route_verdict_tests {
    use super::route_verdict;

    #[test]
    fn negative_gross_is_not_misreported_as_fee_loss() {
        assert!(route_verdict(-10_000, -11_000, 10_000, true).contains("GROSS ROUTE LOSS"));
        assert!(route_verdict(0, -1, 10_000, true).contains("GROSS ROUTE LOSS"));
    }

    #[test]
    fn positive_gross_lost_to_costs_has_distinct_verdict() {
        assert!(route_verdict(10_000, 0, 1_000, true).contains("COSTS EXCEED GROSS EDGE"));
        assert!(route_verdict(10_000, -1, 1_000, true).contains("COSTS EXCEED GROSS EDGE"));
    }

    #[test]
    fn positive_net_is_compared_with_hurdle() {
        assert!(route_verdict(20_000, 999, 1_000, true).contains("BELOW CONFIGURED"));
        assert!(route_verdict(20_000, 1_000, 1_000, true).contains("ABOVE CONFIGURED"));
    }

    #[test]
    fn unavailable_tip_floor_never_marks_candidate_eligible() {
        assert!(route_verdict(20_000, 20_000, 1_000, false).contains("TIP FLOOR UNAVAILABLE"));
    }
}
