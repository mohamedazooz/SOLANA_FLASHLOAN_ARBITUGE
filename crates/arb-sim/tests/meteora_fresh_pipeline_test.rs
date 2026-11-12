//! Integration Test for Fresh-State Meteora DLMM Cross-Pool Pipeline.
//!
//! Verifies:
//! 1. Atomic batch account fetching with real RPC context slot.
//! 2. Zero stale quotes via bounded retry with exponential backoff + jitter.
//! 3. Simulation against real Mainnet bank state (`simulateTransaction`).
//! 4. Full numerical profitability accounting.

use arb_ingest::RpcPool;
use arb_sim::{MeteoraDlmmFreshStatePipeline, MeteoraFreshPipelineConfig};
use arb_tx::KeypairSigner;
use solana_pubkey::Pubkey;
use std::str::FromStr;

fn get_funded_signer() -> KeypairSigner {
    if let Ok(key_str) = std::env::var("SOLANA_PRIVATE_KEY") {
        if let Ok(signer) = KeypairSigner::from_base58(&key_str) {
            return signer;
        }
    }
    for path in &[".env", "../../.env", "../.env"] {
        if let Ok(content) = std::fs::read_to_string(path) {
            for line in content.lines() {
                if let Some(val) = line.strip_prefix("SOLANA_PRIVATE_KEY=") {
                    let clean = val.trim().trim_matches('"').trim_matches('\'');
                    if let Ok(signer) = KeypairSigner::from_base58(clean) {
                        return signer;
                    }
                }
            }
        }
    }
    KeypairSigner::from_seed(&[55u8; 32])
}

#[tokio::test]
async fn test_fresh_state_meteora_cross_pool_cycle() {
    dotenvy::dotenv().ok();
    let rpc = std::sync::Arc::new(RpcPool::from_env());
    if rpc.endpoints_count() == 0 {
        eprintln!("Skipping fresh-state test: no RPC endpoints configured");
        return;
    }

    let pool_a = Pubkey::from_str("5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6").unwrap();
    let pool_b = Pubkey::from_str("BVRbyLjjfSBcoyiYFuxbgKYnWuiFaF9CSXEa5vdSZ9Hh").unwrap();

    let cfg = MeteoraFreshPipelineConfig {
        max_retries: 3,
        max_allowed_slot_lag: 2,
        base_backoff_ms: 30,
        max_backoff_ms: 150,
        slippage_bps: 100,
        cu_limit: 400_000,
        priority_fee_micro_lamports: 10_000,
        min_input_amount: 100_000,
        max_input_amount: 10_000_000,
        sol_usd_price: Some(180),
        requested_bin_array_depth: 1,
        two_pool_v1: false,
    };

    let pipeline = MeteoraDlmmFreshStatePipeline::new(rpc.clone(), cfg);
    let signer = get_funded_signer();

    println!(
        "Executing fresh-state arbitrage cycle for pools {} and {}...",
        pool_a, pool_b
    );
    let report = pipeline
        .execute_fresh_arbitrage_cycle("CANDIDATE-FRESH-001", &signer, pool_a, pool_b)
        .await
        .expect("Fresh pipeline execution must succeed");

    println!("\n=======================================================");
    println!("          METEORA DLMM FRESH-STATE ARBITRAGE REPORT    ");
    println!("=======================================================");
    println!("Candidate ID:                 {}", report.candidate_id);
    println!("Status:                       {}", report.status);
    println!("Direction:                    {}", report.direction);
    println!("Pool A:                       {}", report.pool_a);
    println!("Pool B:                       {}", report.pool_b);
    println!(
        "Quote Mint:                   {} (decimals={})",
        report.quote_mint, report.quote_decimals
    );
    println!(
        "Initial Quote Amount:         {} raw {} ({:.6} {})",
        report.initial_quote_amount,
        report.quote_mint,
        report.initial_quote_amount as f64 / 10f64.powi(report.quote_decimals as i32),
        report.quote_mint
    );
    println!(
        "Hop 1 Output:                 {} raw units",
        report.hop1_amount_out
    );
    println!(
        "Hop 2 Final Output:           {} raw {} ({:.6} {})",
        report.hop2_final_amount_out,
        report.quote_mint,
        report.hop2_final_amount_out as f64 / 10f64.powi(report.quote_decimals as i32),
        report.quote_mint
    );
    println!(
        "Gross Spread:                 {} raw {} ({:.6} {})",
        report.gross_spread_quote_raw,
        report.quote_mint,
        report.gross_spread_quote_raw as f64 / 10f64.powi(report.quote_decimals as i32),
        report.quote_mint
    );
    println!(
        "Hop 1 DEX Fee:                {} raw {}",
        report.dex_fees_hop1_raw, report.dex_fees_hop1_currency
    );
    println!(
        "Hop 2 DEX Fee:                {} raw {}",
        report.dex_fees_hop2_raw, report.dex_fees_hop2_currency
    );
    println!(
        "Total DEX Fees:               {} raw units (embedded)",
        report.total_dex_fees_raw
    );
    println!(
        "Base Tx Fee:                  {} lamports (SOL)",
        report.base_fee_lamports
    );
    println!(
        "Priority Fee:                 {} lamports (SOL)",
        report.priority_fee_lamports
    );
    println!(
        "Jito Tip:                     {} lamports (SOL)",
        report.jito_tip_lamports
    );
    println!(
        "Refundable Rent:              {} lamports (SOL)",
        report.refundable_rent_lamports
    );
    println!(
        "Total Execution Costs:        {} raw {} (converted)",
        report.total_execution_cost_quote_raw, report.quote_mint
    );
    println!(
        "Net Expected Profit:          {} raw {} ({:.6} {})",
        report.net_expected_profit_quote_raw,
        report.quote_mint,
        report.net_expected_profit_quote_raw as f64 / 10f64.powi(report.quote_decimals as i32),
        report.quote_mint
    );
    println!(
        "Net Profit (bps):             {} bps",
        report.net_profit_bps
    );
    println!("Quote Snapshot Slot:          {}", report.quote_slot);
    println!("Simulation Context Slot:      {:?}", report.simulation_slot);
    println!(
        "Observed Slot Lag:            {:?} slots (Max allowed: 2)",
        report.slot_lag
    );
    println!("Quote Freshness Cleared:      {}", report.is_fresh);
    println!("Retries Used:                 {}/3", report.retries_used);
    println!(
        "Compute Units Consumed:       {} / 400,000",
        report.cu_consumed
    );
    println!(
        "Wire Transaction Size:        {} bytes (Solana MTU limit: 1232)",
        report.tx_size_bytes
    );
    println!(
        "Simulation Success:           {}",
        report.simulation_success
    );
    println!(
        "Rejection Reason:             {:?}",
        report.rejection_reason
    );
    println!("=======================================================\n");

    println!("=== RPC Telemetry & Health ===");
    let health = rpc.health_snapshot();
    println!("RPC Pool Status: {}", rpc.format_compact_status());
    for p in &health.providers {
        println!(
            "  - Provider: {:<12} | OK: {:<4} | 429: {:<2} | Errors: {:<2} | Cooldown: {}s | Quota Exhausted: {}",
            p.label,
            p.ok_count,
            p.rate_limited_429,
            p.error_count,
            p.cooldown_remaining_secs,
            p.quota_exhausted
        );
    }
    println!("==============================\n");

    // Assert that simulation executed with non-zero CU and valid size within MTU
    assert!(
        report.tx_size_bytes <= 1232,
        "Tx size must not exceed MTU limit"
    );
    assert!(report.tx_size_bytes > 500, "Tx size must reflect two swaps");
    assert!(
        report.simulation_success,
        "Both swaps in atomic transaction must succeed on Mainnet"
    );
}

#[test]
fn test_dimensional_accounting_invariants() {
    use arb_types::currency::{
        ConversionSource, Currency, DimensionalProfitabilityReport, FeeItem, TimestampedRate,
        TypedTokenAmount,
    };

    // Scenario: User trades 100,000 raw USDC (0.10 USDC).
    // Hop 2 delivers 99,914 raw USDC (0.099914 USDC).
    // Raw difference: -86 raw USDC.
    // Base fee: 5,000 lamports. Priority fee: 4,000 lamports. Total SOL fees = 9,000 lamports.
    let quote_slot = 320_000_000;
    let now_ms = 1_700_000_000_000;

    let initial_input = TypedTokenAmount::usdc(100_000, quote_slot, now_ms);
    let hop1_amount = TypedTokenAmount::sol_lamports(1_160_000, quote_slot, now_ms);
    let final_output = TypedTokenAmount::usdc(99_914, quote_slot, now_ms);

    let fees = vec![
        FeeItem::network_base_fee(5_000, quote_slot, now_ms),
        FeeItem::priority_fee(4_000, quote_slot, now_ms),
        FeeItem::dex_swap_fee("Hop 1 Fee", 400, Currency::Sol, 9, quote_slot, now_ms),
        FeeItem::dex_swap_fee("Hop 2 Fee", 350, Currency::usdc(), 6, quote_slot, now_ms),
    ];

    // Conservative rate: 1 SOL = 180 USDC
    let sol_rate = TimestampedRate::sol_to_usdc(
        180,
        1,
        quote_slot,
        now_ms,
        ConversionSource::ConfiguredRate {
            label: "Conservative".into(),
        },
    )
    .unwrap();

    let report = DimensionalProfitabilityReport::compute(
        "TEST-CANDIDATE-001",
        "Test DLMM Cycle",
        initial_input,
        hop1_amount,
        final_output,
        fees,
        Some(sol_rate),
        quote_slot,
    )
    .expect("Profitability calculation must succeed");

    // 1. Gross profit must be exactly -86 raw USDC units, NOT -86 lamports!
    assert_eq!(report.gross_profit_quote_raw, -86);

    // 2. Converted SOL network fee: 9,000 lamports * 180 USD / 10^9 lamports * 10^6 = 1,620 raw USDC units
    assert_eq!(report.total_network_fees_in_quote.raw_amount, 1_620);
    assert_eq!(report.total_execution_cost_quote_raw, 1_620);

    // 3. Net profit must be -86 - 1,620 = -1,706 raw USDC units, NOT -9,086 lamports!
    assert_eq!(report.net_profit_quote_raw, -1_706);

    // 4. Net bps: (-1,706 * 10,000) / 100,000 = -170 bps, NOT -908 bps!
    assert_eq!(report.net_profit_bps, -170);

    // 5. Embedded DEX fees are tracked (400 + 350 = 750 raw) but NOT deducted again
    assert_eq!(report.total_embedded_dex_fees_raw, 750);
}
