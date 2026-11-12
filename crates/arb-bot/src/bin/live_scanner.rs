//! Live Opportunity Scanner on Solana Mainnet via Helius RPC
//! Queries active Raydium & Orca pools and computes real-time arbitrage spreads and profits.

use anyhow::Result;
use arb_dex::orca_whirlpool::OrcaWhirlpool;
use arb_dex::raydium_v4::RaydiumV4Pool;
use arb_dex::traits::AmmPool;
use arb_engine::{PoolDepth, RouteArchitectWorker};
use arb_ingest::GmgnClient;
use arb_math::economic::{build_economic_quote, calculate_flash_fee_raw};
use arb_math::{Leg, cpmm_out, optimal_two_pool};
use arb_tx::jupiter::JupiterClient;
use arb_types::{DexProtocol, Lamports, MintId, PoolId, Slot, TokenAmount};
use base64::prelude::*;
use serde_json::json;
use solana_pubkey::Pubkey;
use std::str::FromStr;

// Shared verified-input helpers from the main bin (arb-bot has no lib target).
#[path = "../verified_config.rs"]
mod verified_config;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    println!("============================================================");
    println!("🚀 SOLANA ARBITRAGE BOT — LIVE OPPORTUNITY SCANNER");
    println!("============================================================");

    let rpc_pool = arb_ingest::RpcPool::from_env();
    println!(
        "📡 Connected to Multi-RPC Failover Pool ({} endpoints configured: Helius, Shyft, Solami)",
        rpc_pool.endpoints_count()
    );

    let wsol = Pubkey::from_str(arb_tx::WSOL_MINT)?;
    let usdc = Pubkey::from_str(arb_tx::USDC_MINT)?;

    // Key Pools
    let raydium_sol_usdc = arb_tx::RAYDIUM_SOL_USDC_POOL;
    let orca_sol_usdc = arb_tx::ORCA_SOL_USDC_POOL;

    let pool_keys = vec![raydium_sol_usdc, orca_sol_usdc];
    println!("🔍 Fetching live pool states via Rotating Multi-RPC Pool...");

    let accounts = rpc_pool.get_multiple_accounts(&pool_keys).await?;

    let raydium_sol_vault = arb_tx::RAYDIUM_SOL_VAULT;
    let raydium_usdc_vault = arb_tx::RAYDIUM_USDC_VAULT;
    let orca_sol_vault = arb_tx::ORCA_SOL_VAULT;
    let orca_usdc_vault = arb_tx::ORCA_USDC_VAULT;

    let ray_vault_sol_pk = Pubkey::from_str(raydium_sol_vault)?;
    let ray_vault_usdc_pk = Pubkey::from_str(raydium_usdc_vault)?;
    let orca_vault_sol_pk = Pubkey::from_str(orca_sol_vault)?;
    let orca_vault_usdc_pk = Pubkey::from_str(orca_usdc_vault)?;

    // Online-verified Raydium v4 authority PDA
    let raydium_authority = Pubkey::from_str("5Q544fKrFoe6tsEbD7S8EmxGTJYAKtTVhAW5Q5pge4j1")?;

    let mut raydium_pool = RaydiumV4Pool::new(
        PoolId(Pubkey::from_str(raydium_sol_usdc)?),
        MintId(wsol),
        MintId(usdc),
        raydium_authority,
        ray_vault_sol_pk,
        ray_vault_usdc_pk,
    );

    let mut orca_pool = OrcaWhirlpool::new(
        PoolId(Pubkey::from_str(orca_sol_usdc)?),
        MintId(wsol),
        MintId(usdc),
        orca_vault_sol_pk,
        orca_vault_usdc_pk,
        64,
        500, // 5 bps
    );

    // 1. Raydium SOL/USDC parsing
    if let Some(Some(acc)) = accounts.first()
        && let Some(data_str) = acc
            .get("data")
            .and_then(|d| d.get(0))
            .and_then(|s| s.as_str())
        && let Ok(data_bytes) = BASE64_STANDARD.decode(data_str)
        && raydium_pool.update_from_account(&data_bytes).is_ok()
    {
        println!("  ✅ Raydium v4 SOL/USDC pool layout deserialized successfully!");
    }

    // 2. Orca SOL/USDC parsing
    if let Some(Some(acc)) = accounts.get(1)
        && let Some(data_str) = acc
            .get("data")
            .and_then(|d| d.get(0))
            .and_then(|s| s.as_str())
        && let Ok(data_bytes) = BASE64_STANDARD.decode(data_str)
        && orca_pool.update_from_account(&data_bytes).is_ok()
    {
        println!("  ✅ Orca Whirlpool SOL/USDC layout deserialized successfully!");
    }

    // Also fetch live vault balances for exact reserves
    let vault_keys = vec![
        raydium_sol_vault,
        raydium_usdc_vault,
        orca_sol_vault,
        orca_usdc_vault,
    ];
    let vault_accs = rpc_pool.get_multiple_accounts(&vault_keys).await?;

    let mut ray_sol_res = 100_000_000_000_000u128;
    let mut ray_usdc_res = 10_000_000_000_000u128;
    let mut orca_sol_res = 90_000_000_000_000u128;
    let mut orca_usdc_res = 9_000_000_000_000u128;

    if let Some(Some(acc)) = vault_accs.first()
        && let Some(data_str) = acc
            .get("data")
            .and_then(|d| d.get(0))
            .and_then(|s| s.as_str())
        && let Ok(bytes) = BASE64_STANDARD.decode(data_str)
        && bytes.len() >= 72
    {
        let amount = u64::from_le_bytes(bytes[64..72].try_into().unwrap_or([0; 8]));
        if amount > 0 {
            ray_sol_res = amount as u128;
        }
    }
    if let Some(Some(acc)) = vault_accs.get(1)
        && let Some(data_str) = acc
            .get("data")
            .and_then(|d| d.get(0))
            .and_then(|s| s.as_str())
        && let Ok(bytes) = BASE64_STANDARD.decode(data_str)
        && bytes.len() >= 72
    {
        let amount = u64::from_le_bytes(bytes[64..72].try_into().unwrap_or([0; 8]));
        if amount > 0 {
            ray_usdc_res = amount as u128;
        }
    }
    if let Some(Some(acc)) = vault_accs.get(2)
        && let Some(data_str) = acc
            .get("data")
            .and_then(|d| d.get(0))
            .and_then(|s| s.as_str())
        && let Ok(bytes) = BASE64_STANDARD.decode(data_str)
        && bytes.len() >= 72
    {
        let amount = u64::from_le_bytes(bytes[64..72].try_into().unwrap_or([0; 8]));
        if amount > 0 {
            orca_sol_res = amount as u128;
        }
    }
    if let Some(Some(acc)) = vault_accs.get(3)
        && let Some(data_str) = acc
            .get("data")
            .and_then(|d| d.get(0))
            .and_then(|s| s.as_str())
        && let Ok(bytes) = BASE64_STANDARD.decode(data_str)
        && bytes.len() >= 72
    {
        let amount = u64::from_le_bytes(bytes[64..72].try_into().unwrap_or([0; 8]));
        if amount > 0 {
            orca_usdc_res = amount as u128;
        }
    }

    raydium_pool.update_reserves(ray_sol_res, ray_usdc_res);

    println!("\n📊 ---------------- LIVE MARKET ARBITRAGE ANALYSIS ----------------");
    let sol_depth_ray = ray_sol_res as f64 / 1e9;
    let usdc_depth_ray = ray_usdc_res as f64 / 1e6;
    let price_raydium = usdc_depth_ray / sol_depth_ray;

    // Genuine live Orca price derived directly from on-chain state
    let price_orca = if orca_pool.sqrt_price_x64 > 0 {
        let sqrt_p = orca_pool.sqrt_price_x64 as f64 / 18_446_744_073_709_551_616.0;
        sqrt_p * sqrt_p * 1000.0
    } else {
        let sol_depth = orca_sol_res as f64 / 1e9;
        let usdc_depth = orca_usdc_res as f64 / 1e6;
        if sol_depth > 0.0 {
            usdc_depth / sol_depth
        } else {
            price_raydium
        }
    };
    let sol_depth_orca = orca_sol_res as f64 / 1e9;
    let usdc_depth_orca = orca_usdc_res as f64 / 1e6;

    println!(
        "• Raydium SOL/USDC Pool:  Rx={:.2} SOL, Ry={:.2} USDC (Fee: 25 bps)",
        sol_depth_ray, usdc_depth_ray
    );
    println!(
        "• Orca Whirlpool SOL/USDC: Rx={:.2} SOL, Ry={:.2} USDC (Fee: 5 bps)",
        sol_depth_orca, usdc_depth_orca
    );

    println!("\n💲 Real-Time Spot Prices:");
    println!("  - Raydium Price: {:.3} USDC per SOL", price_raydium);
    println!("  - Orca Price:    {:.3} USDC per SOL", price_orca);

    let spread_bps = ((price_raydium - price_orca).abs() / price_orca) * 10_000.0;
    println!("  - Inter-DEX Spread: {:.2} bps", spread_bps);

    // Compute optimal sizing
    let leg1 = Leg {
        r_in: ray_sol_res,
        r_out: ray_usdc_res,
        gamma: 10_000 - 25,
    };
    let leg2 = Leg {
        r_in: orca_usdc_res,
        r_out: orca_sol_res,
        gamma: 10_000 - 5,
    };

    println!("\n🧮 Running Closed-Form Sizing Optimization (optimal_two_pool)...");
    if let Some(opt_sol_in) = optimal_two_pool(&leg1, &leg2) {
        let opt_lamports = opt_sol_in as u128;
        let Some(usdc_out) = cpmm_out(opt_lamports, leg1.r_in, leg1.r_out, 25) else {
            eprintln!("CPMM quote failed for leg 1");
            return Ok(());
        };
        let Some(sol_back) = cpmm_out(usdc_out, leg2.r_in, leg2.r_out, 5) else {
            eprintln!("CPMM quote failed for leg 2");
            return Ok(());
        };

        if sol_back > opt_lamports {
            let gross = sol_back - opt_lamports;

            // Verified economics (never hardcoded): live Kamino fee sf + WSOL
            // decimals straight from the RPC pool. If either read fails, the
            // raw spread is reported WITHOUT any profit claim (fail closed).
            match read_verified_economics(&rpc_pool).await {
                Some((fee_sf, base_decimals)) => {
                    let flash_fee_raw =
                        calculate_flash_fee_raw(opt_lamports as u64, fee_sf).unwrap_or(u64::MAX);
                    let quote = match build_economic_quote(
                        MintId(wsol),
                        base_decimals,
                        TokenAmount(opt_lamports as u64),
                        TokenAmount(sol_back as u64),
                        TokenAmount(sol_back as u64),
                        TokenAmount(flash_fee_raw),
                        TokenAmount(0), // dex fees are inside expected_out
                        TokenAmount(0),
                        TokenAmount(0),
                        TokenAmount(0),
                        Lamports(0), // base/priority/tip are NOT priced by the scanner
                        Lamports(0),
                        Lamports(0),
                        1, // WSOL: 1 lamport == 1 raw unit rate num
                        1, // WSOL rate denom
                        TokenAmount(30_000),
                        Slot(0),
                        0,
                    ) {
                        Ok(q) => q,
                        Err(e) => {
                            eprintln!("⚠️ Economic quote failed ({e}); spread not priced.");
                            return Ok(());
                        }
                    };

                    println!("\n🎯 ARBITRAGE SPREAD DETECTED (raw, pre-execution):");
                    println!(
                        "  - Optimal Loan Size (x*): {:.4} SOL ({} lamports)",
                        opt_sol_in as f64 / 1e9,
                        opt_lamports
                    );
                    println!(
                        "  - Hop 1 (Raydium Buy):    Borrow {:.4} SOL -> Receive {:.2} USDC",
                        opt_sol_in as f64 / 1e9,
                        usdc_out as f64 / 1e6
                    );
                    println!(
                        "  - Hop 2 (Orca Sell):      Sell {:.2} USDC -> Receive {:.4} SOL",
                        usdc_out as f64 / 1e6,
                        sol_back as f64 / 1e9
                    );
                    println!(
                        "  - Gross Profit:           +{:.5} SOL (+{} lamports)",
                        gross as f64 / 1e9,
                        gross
                    );
                    println!(
                        "  - Kamino Flash Fee:       -{:.5} SOL (-{} lamports, fee_sf={fee_sf} read on-chain)",
                        flash_fee_raw as f64 / 1e9,
                        flash_fee_raw
                    );
                    println!("  - Base/priority fee + Jito tip: NOT priced by the scanner");
                    println!(
                        "  - Quoted net (fee only):  {:.5} SOL ({} lamports)",
                        quote.net_profit_raw.as_u64() as f64 / 1e9,
                        quote.net_profit_raw.as_u64()
                    );
                    println!("  - Meets min-profit floor: {}", quote.meets_min_profit);
                    if quote.is_profitable() {
                        println!(
                            "  ⚠️ Pre-execution gross estimate only — landing is never guaranteed."
                        );
                    } else {
                        println!(
                            "  ✋ Not profitable after verified costs — no execution candidate."
                        );
                    }
                }
                None => {
                    println!(
                        "\n⚠️ SPREAD DETECTED: gross +{:.5} SOL, but verified Kamino fee / mint decimals are unavailable — profit NOT established.",
                        gross as f64 / 1e9
                    );
                }
            }
        }
    } else {
        println!(
            "⚖️ Pool spread is currently within equilibrium (no negative cycle). Next scan cycle ready."
        );
    }

    // =========================================================================
    // EQUAL MARGINAL PRICE IMPACT MULTI-POOL SPLIT WORKER DEMO
    // =========================================================================
    println!(
        "\n🌊 ---------------- ROUTE ARCHITECT WORKER: MULTI-POOL SPLIT OPTIMIZER ----------------"
    );
    println!(
        "📐 Evaluating Kamino Flash Loan Split Optimization across Raydium & Orca to eliminate slippage:"
    );

    let simulated_loans = [10_000_000_000u64, 50_000_000_000u64, 200_000_000_000u64];
    let pool1_depth = PoolDepth {
        pool_id: raydium_pool.pool_id,
        protocol: DexProtocol::RaydiumV4,
        reserve_in: ray_sol_res,
        reserve_out: ray_usdc_res,
        fee_bps: 25,
    };
    let pool2_depth = PoolDepth {
        pool_id: orca_pool.pool_id,
        protocol: DexProtocol::OrcaWhirlpool,
        reserve_in: orca_sol_res,
        reserve_out: orca_usdc_res,
        fee_bps: 5,
    };

    for loan in simulated_loans {
        let (x1, x2) = RouteArchitectWorker::calculate_optimal_split_two_pools(
            &pool1_depth,
            &pool2_depth,
            loan,
        );
        let ratio1 = (x1 as f64 / loan as f64) * 100.0;
        let ratio2 = (x2 as f64 / loan as f64) * 100.0;
        println!(
            "  • Total Flash Loan: {:>6.2} SOL -> Leg 1 (Raydium): {:>6.2} SOL ({:>5.1}%) | Leg 2 (Orca): {:>6.2} SOL ({:>5.1}%)",
            loan as f64 / 1e9,
            x1 as f64 / 1e9,
            ratio1,
            x2 as f64 / 1e9,
            ratio2
        );
    }

    // =========================================================================
    // DYNAMIC GMGN 40 TRENDING TOKENS SCAN & JUPITER QUOTING
    // =========================================================================
    println!("\n🔥 ---------------- GMGN TOP 40 TRENDING TOKENS DISCOVERY ----------------");
    let gmgn = GmgnClient::new();
    let jup = JupiterClient::new();
    match gmgn.fetch_trending_tokens(40).await {
        Ok(tokens) => {
            println!(
                "✅ Discovered {} trending Solana tokens from GMGN / DexScreener:",
                tokens.len()
            );
            for (idx, tok) in tokens.iter().enumerate().take(4) {
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                let quote_res = jup
                    .get_quote(&wsol.to_string(), &tok.address.to_string(), 10_000_000, 50)
                    .await;
                let quote_str = match quote_res {
                    Ok(q) => {
                        let out_str = q["outAmount"].as_str().unwrap_or("?");
                        let price_fmt = if tok.price_usd > 0.0 {
                            format!("${:.19}", tok.price_usd)
                        } else {
                            // Implied price from Jupiter quote (0.01 SOL * spot price)
                            let implied = out_str
                                .parse::<f64>()
                                .ok()
                                .map(|out| (0.01 * price_raydium) / (out / 1e6))
                                .unwrap_or(0.0);
                            format!("${:.19}", implied)
                        };
                        format!("Quote Out: {} units (Price: {})", out_str, price_fmt)
                    }
                    Err(e) => format!("Route: Pending Liquidity ({e})"),
                };
                println!(
                    "  {:>2}. {:<8} ({}) | Liq: ${:>9.0} | 1h Swaps: {:>5} | {}",
                    idx + 1,
                    tok.symbol,
                    tok.address,
                    tok.liquidity_usd,
                    tok.swaps_1h,
                    quote_str
                );
            }
            if tokens.len() > 4 {
                println!(
                    "  ... and {} more tokens registered in active market graph.",
                    tokens.len() - 4
                );
            }
        }
        Err(e) => {
            println!("⚠️ GMGN Discovery: {e}");
        }
    }

    // =========================================================================
    // 2-LEG & 3-LEG CYCLIC ARBITRAGE SCANNER (Pump.fun / Raydium / Meteora / Orca)
    // =========================================================================
    println!("\n🔄 ---------------- 2-LEG & 3-LEG CYCLIC ARBITRAGE EVALUATION ----------------");
    println!(
        "🔍 Simulating 1.0 SOL (1,000,000,000 lamports) Flash Cycles across top trending tokens:\n"
    );

    let test_loan_lamports = 1_000_000_000u64; // 1.0 SOL
    if let Ok(tokens) = gmgn.fetch_trending_tokens(10).await {
        for tok in tokens.iter().take(2) {
            println!("────────────────────────────────────────────────────────────");
            println!(
                "🎯 Token: {} ({}) | Liq: ${:.0}",
                tok.symbol, tok.address, tok.liquidity_usd
            );

            // --- 2-LEG ROUTE: SOL ➔ Token ➔ SOL ---
            tokio::time::sleep(std::time::Duration::from_millis(600)).await;
            let leg1_res = jup
                .get_quote(
                    &wsol.to_string(),
                    &tok.address.to_string(),
                    test_loan_lamports,
                    50,
                )
                .await;
            match leg1_res {
                Ok(ref q1) => {
                    let token_out = q1
                        .get("outAmount")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(0);
                    let dex1 = extract_dex_labels(q1);

                    if token_out == 0 {
                        println!(
                            "    ⚠️ Jupiter did not return valid outAmount: {:?}",
                            q1.get("message").or_else(|| q1.get("errorCode"))
                        );
                    } else {
                        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
                        let leg2_res = jup
                            .get_quote(&tok.address.to_string(), &wsol.to_string(), token_out, 50)
                            .await;
                        match leg2_res {
                            Ok(ref q2) => {
                                let sol_return = q2
                                    .get("outAmount")
                                    .and_then(|v| v.as_str())
                                    .and_then(|s| s.parse::<u64>().ok())
                                    .unwrap_or(0);
                                let dex2 = extract_dex_labels(q2);

                                let diff = sol_return as i64 - test_loan_lamports as i64;
                                let spread_bps =
                                    (diff as f64 / test_loan_lamports as f64) * 10_000.0;

                                println!("  [2-Leg Cycle] SOL ➔ {} ➔ SOL:", tok.symbol);
                                println!(
                                    "    • Hop 1: Borrow 1.00 SOL  ➔ Receive {} {} via [{}]",
                                    token_out, tok.symbol, dex1
                                );
                                println!(
                                    "    • Hop 2: Sell {} {}  ➔ Receive {:.4} SOL via [{}]",
                                    token_out,
                                    tok.symbol,
                                    sol_return as f64 / 1e9,
                                    dex2
                                );
                                if diff > 0 {
                                    println!(
                                        "    🌟 PROFITABLE 2-LEG ARB: +{:.5} SOL (+{:.2} bps)",
                                        diff as f64 / 1e9,
                                        spread_bps
                                    );
                                } else {
                                    println!(
                                        "    ⚖️ Round-Trip Net Spread: {:.2} bps (Cost/Slippage: {:.5} SOL)",
                                        spread_bps,
                                        diff.abs() as f64 / 1e9
                                    );
                                }
                            }
                            Err(e) => {
                                println!("    ⚠️ Leg 2 (Token ➔ SOL) Quote Error: {e}");
                            }
                        }

                        // --- 3-LEG ROUTE: SOL ➔ Token ➔ USDC ➔ SOL ---
                        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
                        if let Ok(q2_usdc) = jup
                            .get_quote(&tok.address.to_string(), &usdc.to_string(), token_out, 50)
                            .await
                        {
                            let usdc_out = q2_usdc
                                .get("outAmount")
                                .and_then(|v| v.as_str())
                                .and_then(|s| s.parse::<u64>().ok())
                                .unwrap_or(0);
                            let dex2_usdc = extract_dex_labels(&q2_usdc);

                            if usdc_out > 0 {
                                tokio::time::sleep(std::time::Duration::from_millis(600)).await;
                                if let Ok(q3) = jup
                                    .get_quote(&usdc.to_string(), &wsol.to_string(), usdc_out, 50)
                                    .await
                                {
                                    let sol_return3 = q3
                                        .get("outAmount")
                                        .and_then(|v| v.as_str())
                                        .and_then(|s| s.parse::<u64>().ok())
                                        .unwrap_or(0);
                                    let dex3 = extract_dex_labels(&q3);

                                    let diff3 = sol_return3 as i64 - test_loan_lamports as i64;
                                    let spread3_bps =
                                        (diff3 as f64 / test_loan_lamports as f64) * 10_000.0;

                                    println!(
                                        "  [3-Leg Triangular] SOL ➔ {} ➔ USDC ➔ SOL:",
                                        tok.symbol
                                    );
                                    println!(
                                        "    • Hop 1: 1.00 SOL  ➔ {} {} via [{}]",
                                        token_out, tok.symbol, dex1
                                    );
                                    println!(
                                        "    • Hop 2: {} {}  ➔ ${:.2} USDC via [{}]",
                                        token_out,
                                        tok.symbol,
                                        usdc_out as f64 / 1e6,
                                        dex2_usdc
                                    );
                                    println!(
                                        "    • Hop 3: ${:.2} USDC ➔ {:.4} SOL via [{}]",
                                        usdc_out as f64 / 1e6,
                                        sol_return3 as f64 / 1e9,
                                        dex3
                                    );
                                    if diff3 > 0 {
                                        println!(
                                            "    🌟 PROFITABLE 3-LEG TRIANGULAR: +{:.5} SOL (+{:.2} bps)",
                                            diff3 as f64 / 1e9,
                                            spread3_bps
                                        );
                                    } else {
                                        println!(
                                            "    ⚖️ Round-Trip Net Spread: {:.2} bps (Cost/Slippage: {:.5} SOL)",
                                            spread3_bps,
                                            diff3.abs() as f64 / 1e9
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    println!("    ⚠️ Leg 1 (SOL ➔ Token) Quote Error: {e}");
                }
            }
        }
    }

    println!("\n✅ Live scan completed successfully.");
    Ok(())
}

fn extract_dex_labels(quote: &serde_json::Value) -> String {
    if let Some(plan) = quote.get("routePlan").and_then(|p| p.as_array()) {
        let labels: Vec<String> = plan
            .iter()
            .filter_map(|step| {
                step.get("swapInfo")
                    .and_then(|s| s.get("label"))
                    .and_then(|l| l.as_str())
                    .map(|s| s.to_string())
            })
            .collect();
        if !labels.is_empty() {
            return labels.join(" ➔ ");
        }
    }
    "Direct AMM".to_string()
}

/// Read the Kamino flash-loan fee (U68F60 sf, 60 fractional bits) and the WSOL
/// mint decimals live from the RPC pool. Returns `None` when either read fails —
/// the scanner must never substitute a hardcoded fee, so an unreadable source
/// means "profit not established" rather than a guessed number.
async fn read_verified_economics(rpc_pool: &arb_ingest::RpcPool) -> Option<(u64, u8)> {
    let wsol = Pubkey::from_str(arb_tx::WSOL_MINT).ok()?;
    let reserve = Pubkey::from_str(arb_tx::KAMINO_WSOL_RESERVE).ok()?;

    let reserve_json = rpc_pool
        .call(
            "getAccountInfo",
            json!([reserve.to_string(), { "encoding": "base64" }]),
        )
        .await
        .ok()?;
    let reserve_b64 = reserve_json["result"]["value"]["data"][0].as_str()?;
    let reserve_bytes = BASE64_STANDARD.decode(reserve_b64).ok()?;
    let fee_sf = arb_tx::kamino::decode_reserve_flash_fee_sf(&reserve_bytes, &wsol).ok()?;

    let mint_json = rpc_pool
        .call(
            "getAccountInfo",
            json!([wsol.to_string(), { "encoding": "base64" }]),
        )
        .await
        .ok()?;
    let mint_b64 = mint_json["result"]["value"]["data"][0].as_str()?;
    let mint_bytes = BASE64_STANDARD.decode(mint_b64).ok()?;
    let decimals = verified_config::decode_mint_decimals(&mint_bytes).ok()?;

    Some((fee_sf, decimals))
}
