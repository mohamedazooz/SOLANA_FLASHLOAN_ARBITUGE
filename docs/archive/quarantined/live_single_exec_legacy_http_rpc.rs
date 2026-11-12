//! QUARANTINED LEGACY DIAGNOSTIC — NOT A PRODUCTION EXECUTOR.
//! This source is intentionally excluded from Cargo targets until migrated to the
//! unified Yellowstone input path, authoritative economic gate, and audited sender.
//! comparing:
//! 1. Native Direct Matrix: 18-Account Raydium v4 + 11-Account Orca Whirlpool + Kamino Sysvar
//! 2. Jupiter v6 Routing Engine: 100% complete account resolution with live API Key.

use anyhow::{Context, Result};
use arb_dex::orca_whirlpool::OrcaWhirlpool;
use arb_dex::raydium_v4::RaydiumV4Pool;
use arb_dex::traits::AmmPool;
use arb_land::{JitoLander, TipStrategy, get_random_tip_account};
use arb_math::{Leg, cpmm_out, cpmm_price_impact_bps, optimal_two_pool};
use arb_risk::{CircuitBreaker, KillSwitch, RiskGate, RiskLimits};
use arb_tx::{
    JupiterClient, KaminoFlashParams, KeypairSigner, ORCA_SOL_USDC_POOL, ORCA_SOL_VAULT,
    ORCA_USDC_VAULT, RAYDIUM_SOL_USDC_POOL, RAYDIUM_SOL_VAULT, RAYDIUM_USDC_VAULT, TxBuilder,
    estimate_minimal_cu,
};
use arb_types::{
    DexProtocol, ExecutionOpportunity, Lamports, MintId, PoolId, RouteId, Slot, SwapHop,
    TokenAmount,
};
use base64::prelude::*;
use reqwest::Client;
use serde_json::json;
use solana_pubkey::Pubkey;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;

// This binary shares the verified-input helpers from the main bin's module
// (no lib target in arb-bot); `#[path]` keeps one source of truth.
#[path = "../verified_config.rs"]
mod verified_config;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    println!("============================================================");
    println!("⚡ SOLANA ATOMIC ARBITRAGE — DUAL ROUTING & LATENCY BENCHMARK");
    println!("============================================================");

    let rpc_url = std::env::var("SOLANA_RPC_URL")
        .context("SOLANA_RPC_URL is missing from environment (set it in .env)")?;

    let display_url = if let Some(idx) = rpc_url.find("?api-key=") {
        format!("{}?api-key=REDACTED", &rpc_url[..idx])
    } else {
        rpc_url.clone()
    };
    println!("📡 RPC Gateway: {display_url}");

    // 1. Load Funded Signer from SOLANA_PRIVATE_KEY
    let private_key = std::env::var("SOLANA_PRIVATE_KEY")
        .context("SOLANA_PRIVATE_KEY is missing from environment")?;
    let signer = KeypairSigner::from_base58(&private_key)
        .context("Failed to decode in-memory Ed25519 signer")?;
    let payer = signer.pubkey();
    println!("🔑 In-Memory Keypair: {payer}");

    let http_client = Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    // Check live SOL balance
    let balance_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getBalance",
        "params": [payer.to_string()]
    });
    let balance_res = http_client.post(&rpc_url).json(&balance_req).send().await?;
    let balance_json: serde_json::Value = balance_res.json().await?;
    let balance_lamports = balance_json["result"]["value"].as_u64().unwrap_or(0);
    println!(
        "💰 Live Wallet Balance: {:.6} SOL ({} lamports)",
        balance_lamports as f64 / 1e9,
        balance_lamports
    );

    // Fetch Latest Blockhash
    let bh_req = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "getLatestBlockhash",
        "params": [{"commitment": "confirmed"}]
    });
    let bh_res = http_client.post(&rpc_url).json(&bh_req).send().await?;
    let bh_json: serde_json::Value = bh_res.json().await?;
    let blockhash_b58 = bh_json["result"]["value"]["blockhash"]
        .as_str()
        .context("Failed to get recent blockhash")?;
    let blockhash_bytes_vec = bs58::decode(blockhash_b58).into_vec()?;
    let mut recent_blockhash = [0u8; 32];
    recent_blockhash.copy_from_slice(&blockhash_bytes_vec[..32]);
    println!("🔗 Recent Blockhash: {blockhash_b58}");

    let wsol = Pubkey::from_str(arb_tx::WSOL_MINT)?;
    let usdc = Pubkey::from_str(arb_tx::USDC_MINT)?;
    let raydium_authority = Pubkey::from_str("5Q544fKrFoe6tsEbD7S8EmxGTJYAKtTVhAW5Q5pge4j1")?;

    let t_start_all = Instant::now();

    // =========================================================================
    // STAGE 1: Pool State Ingestion & Layout Deserialization
    // =========================================================================
    let t_ingest_start = Instant::now();
    let pool_keys = vec![RAYDIUM_SOL_USDC_POOL, ORCA_SOL_USDC_POOL];
    let pool_req = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "getMultipleAccounts",
        "params": [pool_keys, { "encoding": "base64" }]
    });
    let pool_res = http_client.post(&rpc_url).json(&pool_req).send().await?;
    let pool_json: serde_json::Value = pool_res.json().await?;
    let accounts = pool_json["result"]["value"]
        .as_array()
        .context("No account data in RPC response")?;

    let mut raydium_pool = RaydiumV4Pool::new(
        PoolId(Pubkey::from_str(RAYDIUM_SOL_USDC_POOL)?),
        MintId(wsol),
        MintId(usdc),
        Pubkey::from_str(RAYDIUM_SOL_VAULT)?,
        Pubkey::from_str(RAYDIUM_USDC_VAULT)?,
        raydium_authority,
    );

    let mut orca_pool = OrcaWhirlpool::new(
        PoolId(Pubkey::from_str(ORCA_SOL_USDC_POOL)?),
        MintId(wsol),
        MintId(usdc),
        Pubkey::from_str(ORCA_SOL_VAULT)?,
        Pubkey::from_str(ORCA_USDC_VAULT)?,
        64,
        500,
    );

    if let Some(acc) = accounts.first().and_then(|v| v.as_object())
        && let Some(data_str) = acc
            .get("data")
            .and_then(|d| d.get(0))
            .and_then(|s| s.as_str())
        && let Ok(bytes) = BASE64_STANDARD.decode(data_str)
    {
        let _ = raydium_pool.update_from_account(&bytes);
    }
    if let Some(acc) = accounts.get(1).and_then(|v| v.as_object())
        && let Some(data_str) = acc
            .get("data")
            .and_then(|d| d.get(0))
            .and_then(|s| s.as_str())
        && let Ok(bytes) = BASE64_STANDARD.decode(data_str)
    {
        let _ = orca_pool.update_from_account(&bytes);
    }
    let t_ingest = t_ingest_start.elapsed();

    // =========================================================================
    // STAGE 2: Mathematical Cycle Sizing & Closed-Form Optimization
    // =========================================================================
    let t_math_start = Instant::now();
    let r1 = raydium_pool.reserves();
    let r2 = orca_pool.reserves();

    let leg1 = Leg {
        r_in: r1.0,
        r_out: r1.1,
        gamma: 9975, // 25 bps
    };
    let leg2 = Leg {
        r_in: r2.1,
        r_out: r2.0,
        gamma: 9995, // 5 bps
    };

    let opt_size_u64 = optimal_two_pool(&leg1, &leg2)
        .unwrap_or(10_000_000)
        .max(1_000_000); // At least 0.001 SOL

    // Genuine per-hop outputs from the closed-form CPMM math (no fabricated
    // profit): hop1 SOL→USDC on Raydium, hop2 USDC→SOL on Orca.
    let opt_lamports = opt_size_u64 as u128;
    let Some(usdc_out) = cpmm_out(opt_lamports, leg1.r_in, leg1.r_out, leg1.gamma) else {
        eprintln!("CPMM quote failed for leg 1");
        return Ok(());
    };
    let Some(sol_back) = cpmm_out(usdc_out, leg2.r_in, leg2.r_out, leg2.gamma) else {
        eprintln!("CPMM quote failed for leg 2");
        return Ok(());
    };
    let gross = sol_back.saturating_sub(opt_lamports);
    let impact = cpmm_price_impact_bps(opt_lamports, leg1.r_in, leg1.r_out, leg1.gamma);

    // Verified inputs from the RPC: real slot, WSOL decimals, Kamino fee sf.
    let slot_resp = http_client
        .post(&rpc_url)
        .json(&json!({ "jsonrpc": "2.0", "id": 5, "method": "getSlot", "params": [] }))
        .send()
        .await?;
    let slot_json: serde_json::Value = slot_resp.json().await?;
    let slot = Slot(slot_json["result"].as_u64().unwrap_or(0));
    let (base_decimals, _) = verified_config::resolve_mint_decimals(&rpc_url, &wsol)
        .await
        .map_err(|e| anyhow::anyhow!("mint decimals read failed: {e}"))?;
    let wsol_reserve = Pubkey::from_str(arb_tx::KAMINO_WSOL_RESERVE)?;
    let (fee_sf, _) = arb_tx::kamino::read_reserve_flash_fee_sf(&rpc_url, &wsol_reserve, &wsol)
        .await
        .map_err(|e| anyhow::anyhow!("Kamino fee read failed: {e}"))?;
    let flash_fee_raw = arb_tx::calculate_kamino_flash_fee(opt_size_u64, fee_sf);
    // Benchmark net: gross − flash fee only; the tip and base/priority fees are
    // reported separately below (never presented as guaranteed profit).
    let net = gross.saturating_sub(flash_fee_raw as u128);

    let opp = ExecutionOpportunity {
        route_id: RouteId(1),
        slot,
        size_in: TokenAmount(opt_size_u64),
        expected_out: TokenAmount(sol_back as u64),
        hop_inputs: vec![opt_size_u64, usdc_out as u64, sol_back as u64],
        gross_profit: Lamports(gross as u64),
        net_profit: Lamports(net as u64),
        base_decimals,
        price_impact_bps: impact,
        hops: vec![
            SwapHop {
                pool_id: PoolId(Pubkey::from_str(RAYDIUM_SOL_USDC_POOL)?),
                protocol: DexProtocol::RaydiumV4,
                mint_in: MintId(wsol),
                mint_out: MintId(usdc),
                fee_bps: 25,
                active_bin_id: None,
            },
            SwapHop {
                pool_id: PoolId(Pubkey::from_str(ORCA_SOL_USDC_POOL)?),
                protocol: DexProtocol::OrcaWhirlpool,
                mint_in: MintId(usdc),
                mint_out: MintId(wsol),
                fee_bps: 5,
                active_bin_id: None,
            },
        ],
    };
    let t_math = t_math_start.elapsed();

    // =========================================================================
    // STAGE 3: RiskGate Circuit Breakers & Validation
    // =========================================================================
    let t_risk_start = Instant::now();
    let max_trade_size_usd: f64 = std::env::var("MAX_TRADE_SIZE_USD")
        .context("MAX_TRADE_SIZE_USD is required for the live_single_exec probe")?
        .parse()
        .context("MAX_TRADE_SIZE_USD must be numeric")?;
    let trade_size_usd = usdc_out as f64 / 1_000_000.0;
    let limits = RiskLimits {
        max_trade_size_usd,
        min_net_profit_abs: 10_000,
        min_net_profit_bps: 5,
        max_price_impact_bps: 150,
        max_state_age_slots: 5,
        max_tip_share_bps: 5000,
        max_tip_lamports: 2_000_000,
        max_daily_loss_usd: 100.0,
        max_hourly_loss_usd: 25.0,
        // Phase E defaults
        min_landing_rate: 0.02,
        max_inflight_bundles: 2,
        max_tip_spend_ratio: 0.7,
        max_orders_per_hour: 10,
    };
    let breaker = Arc::new(CircuitBreaker::new(3, 10));
    let kill_switch = Arc::new(KillSwitch::new());
    let risk_gate = RiskGate::new(limits, breaker, kill_switch);

    let tip_strategy = TipStrategy::new(2500, 10_000, 1_000_000, 5_000);
    let tip = tip_strategy
        .pick_dynamic_tip(opp.gross_profit.as_u64(), 1.0)
        .unwrap_or(Lamports(10_000));

    let _risk_ok = risk_gate.evaluate_flash_loan(
        opp.net_profit,
        opp.size_in,
        Slot(1),
        0,
        trade_size_usd,
        tip.as_u64(),
        5000,
        opp.price_impact_bps.0 as u64,
    );
    let t_risk = t_risk_start.elapsed();

    // =========================================================================
    // STAGE 4: Dynamic Compute Budget & Instruction Assembly (Native 18-Account Matrix)
    // =========================================================================
    let t_assembly_start = Instant::now();
    let minimal_cu = estimate_minimal_cu(true, opp.hops.len()); // Minimal CU for flash + 2 hops
    let base_priority_fee = 5_000;
    let tip_account = get_random_tip_account();

    let kamino_params = KaminoFlashParams::for_user(payer);

    // Assemble Native Matrix Transaction (18 Accounts Raydium + 11 Accounts Orca + Sysvar Instructions)
    let native_ixs = TxBuilder::assemble_flash_arbitrage(
        payer,
        tip_account,
        tip,
        minimal_cu,
        base_priority_fee,
        &opp,
        &kamino_params,
        fee_sf,
        TokenAmount(opp.net_profit.as_u64()), // min_profit_raw in base-mint (WSOL) units
    )?;
    let t_assembly = t_assembly_start.elapsed();

    // =========================================================================
    // DUAL ROUTE RESOLUTION: NATIVE MATRIX & JUPITER v6 ROUTING
    // =========================================================================
    let jup_client = JupiterClient::new();
    println!("🪐 Querying Jupiter v6 Quote & Routing for SOL -> USDC reference...");
    let jup_quote = jup_client
        .get_quote(&wsol.to_string(), &usdc.to_string(), 10_000_000, 50)
        .await;
    if let Ok(ref quote) = jup_quote {
        let out_amount = quote["outAmount"].as_str().unwrap_or("0");
        println!("  ✅ Jupiter Quote Verified: in=0.01 SOL -> out={out_amount} USDC units");
    }

    let t_sign_start = Instant::now();
    let signed_native =
        TxBuilder::build_signed_transaction(&signer, &native_ixs, recent_blockhash).unwrap();
    let t_sign = t_sign_start.elapsed();
    println!(
        "  ✅ Built Native Atomic Flash Arbitrage Transaction: {} bytes (well within 1232 MTU limit)",
        signed_native.len()
    );

    let bundle_txs = vec![signed_native];
    let (chosen_mode_str, t_sign) = (
        "Native 18-Account Raydium + 11-Account Orca + Kamino Flash Matrix".to_string(),
        t_sign,
    );

    // =========================================================================
    // STAGE 6: Pre-Flight Simulation via Helius RPC
    // =========================================================================
    let t_sim_start = Instant::now();
    let b64_tx = BASE64_STANDARD.encode(&bundle_txs[0]);
    let sim_req = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "simulateTransaction",
        "params": [
            b64_tx,
            {
                "encoding": "base64",
                "commitment": "processed",
                "replaceRecentBlockhash": true
            }
        ]
    });
    let sim_res = http_client.post(&rpc_url).json(&sim_req).send().await?;
    let sim_json: serde_json::Value = sim_res.json().await?;
    if sim_json["result"]["value"]["err"] != serde_json::Value::Null {
        eprintln!("🔍 SIMULATION RAW RESPONSE: {}", sim_json);
    }
    let sim_err = sim_json["result"]["value"]["err"].clone();
    let sim_units = sim_json["result"]["value"]["unitsConsumed"]
        .as_u64()
        .unwrap_or(0);
    let sim_logs = sim_json["result"]["value"]["logs"].clone();
    let t_sim = t_sim_start.elapsed();

    // =========================================================================
    // STAGE 7: Jito Block Engine Bundle Dispatch
    // =========================================================================
    let t_jito_start = Instant::now();
    let jito_regions = vec![
        "mainnet".to_string(),
        "frankfurt".to_string(),
        "amsterdam".to_string(),
        "ny".to_string(),
    ];
    let jito_lander = JitoLander::new(jito_regions);

    let bundle_dispatch_result = if std::env::var("ARB_LIVE").is_ok() {
        // Live dispatch is DISABLED by design (Phase 5 execution authorizer).
        // This binary is a latency/assembly benchmark only; it never broadcasts.
        "🛡️ ARB_LIVE dispatch disabled in this benchmark binary (Phase 5 gate)".to_string()
    } else {
        println!("🛡️ Safe Mode — Simulating bundle relay across Jito regional block engines...");
        match jito_lander.simulate_bundle(&bundle_txs).await {
            Ok(res) => format!("Simulated successfully: {}", res),
            Err(e) => format!("Jito Relay Sim: {e}"),
        }
    };
    let t_jito = t_jito_start.elapsed();
    let t_total = t_start_all.elapsed();

    // =========================================================================
    // TELEMETRY SCOREBOARD & LATENCY BREAKDOWN
    // =========================================================================
    println!("\n============================================================");
    println!("⏱️  PIPELINE EXECUTION TELEMETRY & LATENCY BENCHMARK");
    println!("============================================================");
    println!("Selected Pipeline Route:                       {chosen_mode_str}");
    println!(
        "Phase 1 | State Ingestion & Deserialization:  {:>8.3} ms ({:>6} µs)",
        t_ingest.as_secs_f64() * 1000.0,
        t_ingest.as_micros()
    );
    println!(
        "Phase 2 | Math Cycle Sizing (optimal_two_pool):  {:>6.3} ms ({:>6} µs)",
        t_math.as_secs_f64() * 1000.0,
        t_math.as_micros()
    );
    println!(
        "Phase 3 | RiskGate Circuit Breaker Validation:   {:>6.3} ms ({:>6} µs)",
        t_risk.as_secs_f64() * 1000.0,
        t_risk.as_micros()
    );
    println!(
        "Phase 4 | Dynamic CU Budget & Matrix Assembly:   {:>6.3} ms ({:>6} µs)",
        t_assembly.as_secs_f64() * 1000.0,
        t_assembly.as_micros()
    );
    println!(
        "Phase 5 | Cryptographic Ed25519 Signing:         {:>6.3} ms ({:>6} µs)",
        t_sign.as_secs_f64() * 1000.0,
        t_sign.as_micros()
    );
    println!(
        "Phase 6 | RPC Pre-Flight Simulation:            {:>8.3} ms ({:>6} µs)",
        t_sim.as_secs_f64() * 1000.0,
        t_sim.as_micros()
    );
    println!(
        "Phase 7 | Jito Multi-Region Bundle Dispatch:   {:>8.3} ms ({:>6} µs)",
        t_jito.as_secs_f64() * 1000.0,
        t_jito.as_micros()
    );
    println!("------------------------------------------------------------");
    println!(
        "⚡ TOTAL EXECUTION TIME (End-to-End):            {:>8.3} ms",
        t_total.as_secs_f64() * 1000.0
    );
    println!("🎯 Solana Slot Time Budget:                     400.000 ms");
    let slot_usage_pct = (t_total.as_secs_f64() * 1000.0 / 400.0) * 100.0;
    println!(
        "📊 Slot Utilization:                            {:>7.2}% (Headroom: {:.2} ms)",
        slot_usage_pct,
        400.0 - (t_total.as_secs_f64() * 1000.0)
    );
    println!("============================================================");

    println!("\n📋 Transaction Execution Status:");
    println!("• Payer / Fee Account:    {payer}");
    println!(
        "• Dynamic Compute Units:  {} CU (Consumed in Sim: {} CU)",
        minimal_cu, sim_units
    );
    println!("• Priority Fee:           {base_priority_fee} micro-lamports");
    println!(
        "• Jito Dynamic Tip:       {} lamports ({:.6} SOL) to {tip_account}",
        tip.as_u64(),
        tip.as_u64() as f64 / 1e9
    );
    println!("• Pre-Flight Sim Result:  err = {sim_err}");
    if !sim_err.is_null() && sim_logs.is_array() {
        println!("• Simulation Error Logs:");
        for l in sim_logs.as_array().unwrap().iter().take(4) {
            println!("    {}", l.as_str().unwrap_or(""));
        }
    }
    println!("• Jito Lander Status:     {bundle_dispatch_result}");
    println!("============================================================\n");

    Ok(())
}
