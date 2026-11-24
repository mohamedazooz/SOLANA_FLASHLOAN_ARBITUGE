//! Diagnostic for persistent QUOTE_FAILED pairs: decode the LbPair + bin
//! arrays fresh and run the quote engine in both directions, printing the
//! exact failure reason. Read-only RPC only.
//!
//! Usage: DIAG_POOL_A=<id> DIAG_POOL_B=<id> cargo run -p arb-sim --example quote_failure_probe

use arb_ingest::RpcPool;
use arb_sim::MeteoraFreshPipelineConfig;
use arb_sim::meteora_fresh_pipeline::MeteoraDlmmFreshStatePipeline;
use arb_types::MintId;
use solana_pubkey::Pubkey;
use std::sync::Arc;

const WSOL: &str = "So11111111111111111111111111111111111111112";
const USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    let pool_a: Pubkey = std::env::var("DIAG_POOL_A").expect("DIAG_POOL_A").parse()?;
    let pool_b: Pubkey = std::env::var("DIAG_POOL_B").expect("DIAG_POOL_B").parse()?;

    let cfg = MeteoraFreshPipelineConfig {
        max_retries: 2,
        max_allowed_slot_lag: 2,
        base_backoff_ms: 30,
        max_backoff_ms: 150,
        slippage_bps: 100,
        cu_limit: arb_constants::APPLICATION_SAFETY_CU_LIMIT,
        priority_fee_micro_lamports: arb_constants::FLASH_ARB_PRIORITY_FEE_MICRO_LAMPORTS,
        min_input_amount: 10_000_000,
        max_input_amount: 200_000_000,
        sol_usd_price: Some(180),
        requested_bin_array_depth: 1,
        two_pool_v1: true,
    };
    let pipeline = MeteoraDlmmFreshStatePipeline::new(Arc::new(RpcPool::from_env()), cfg);
    let (sa, sb, slot, ba_a, ba_b) = pipeline.fetch_fresh_pool_pair(pool_a, pool_b, None).await?;
    println!(
        "slot={slot} arrays_a={} arrays_b={}",
        ba_a.len(),
        ba_b.len()
    );

    for (label, s) in [("A", &sa), ("B", &sb)] {
        let Some(d) = s.dlmm.as_ref() else {
            println!("pool {label}: NO dlmm state");
            continue;
        };
        println!(
            "pool {label}: active_id={} bin_step={} status={} bins_decoded={} support_limit_order={} volatile_acc={} risky={} transfer_fee_bps={}",
            d.active_bin_id,
            d.bin_step_bps,
            d.status,
            d.bins.len(),
            d.support_limit_order,
            d.volatility_accumulator,
            s.has_risky_token,
            s.transfer_fee_bps
        );
        if let Some(fp) = d.params.as_ref() {
            println!(
                "  params: base_factor={} base_fee_power_factor={} function_type={} collect_fee_mode={} protocol_share={} vfc={} filter/decay/reduction={}/{}/{}",
                fp.base_factor,
                fp.base_fee_power_factor,
                fp.function_type,
                fp.collect_fee_mode,
                fp.protocol_share,
                fp.variable_fee_control,
                fp.filter_period,
                fp.decay_period,
                fp.reduction_factor
            );
        } else {
            println!("  params: MISSING (quote will fail)");
        }
        let near: Vec<_> = d
            .bins
            .iter()
            .filter(|b| (b.bin_id - d.active_bin_id).abs() <= 5)
            .collect();
        for b in near {
            println!(
                "  bin {}: x={} y={} open_order={} processed_rem={} ask={}",
                b.bin_id,
                b.amount_x,
                b.amount_y,
                b.open_order_amount,
                b.processed_order_remaining_amount,
                b.limit_order_ask_side
            );
        }
    }

    // Quote probes: each pool's own mints, both directions, at several sizes.
    let mint_label = |m: &Pubkey| -> &str {
        if *m == WSOL.parse::<Pubkey>().unwrap() {
            "WSOL"
        } else if *m == USDC.parse::<Pubkey>().unwrap() {
            "USDC"
        } else {
            "MEME"
        }
    };
    for (label, s) in [("A", &sa), ("B", &sb)] {
        let (x, y) = (s.base_mint.0, s.quote_mint.0);
        println!(
            "pool {label}: base={}({}) quote={}({})",
            x,
            mint_label(&x),
            y,
            mint_label(&y)
        );
        for (from, to, dir) in [(x, y, "X->Y"), (y, x, "Y->X")] {
            for amount in [10_000_000u64, 50_000_000, 100_000_000] {
                let q = arb_engine::MeteoraDlmmCrossPoolQuoteEngine::quote_single_hop(
                    s,
                    amount,
                    MintId(from),
                    MintId(to),
                );
                match q {
                    Some((quote, sfy)) => println!(
                        "quote {label} {dir} amt={amount}: out={} fee={} bins={:?} sfy={sfy}",
                        quote.amount_out_raw,
                        quote.fees_breakdown.total_fee,
                        quote.dlmm_bins_crossed
                    ),
                    None => println!("quote {label} {dir} amt={amount}: None (rejected)"),
                }
            }
        }
    }
    Ok(())
}
