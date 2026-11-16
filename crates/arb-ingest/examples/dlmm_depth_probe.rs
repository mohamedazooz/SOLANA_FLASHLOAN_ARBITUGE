//! Live read-only DLMM depth probe (NO broadcast, NO signing).
//!
//! Fetches the two registered mainnet SOL/USDC Meteora DLMM pools directly
//! via RPC, decodes LbPair + BinArray accounts, and runs the production
//! quote engine (`quote_dlmm_view`) over a size ladder to report:
//!
//! * per-pool multi-bin executable depth (same traversal as execution),
//! * per-size gross output for the 2-pool cycle (both directions),
//! * Kamino flash fee (exact U68F60), network cost, and NET profit,
//!
//! so the discovery hurdle (`EDGE_BELOW_HURDLE`) can be verified against
//! real numbers instead of trusted blindly.
//!
//! Run: `cargo run -p arb-ingest --example dlmm_depth_probe`

use std::str::FromStr;

use arb_ingest::RpcPool;
use arb_ingest::registry::{
    bin_id_to_bin_array_index, decode_account_data, decode_bin_array, decode_dlmm_state,
    derive_meteora_bin_array_pda,
};
use arb_math::dlmm::{
    DlmmBin, DlmmParameters, DlmmPoolState, DlmmVariableParameters, dlmm_multi_bin_max_input,
    quote_dlmm_view,
};
use arb_types::DexProtocol;
use solana_pubkey::Pubkey;

const KAMINO_WSOL_FLASH_FEE_SF: u128 = 11_529_215_046_068; // decoded on-chain U68F60 (0.1 bps)
const SCALE: u32 = 60;
const NETWORK_COST_LAMPORTS: u128 = 5_000 + 1_000 + 10_000; // base + priority + tip

fn pk(s: &str) -> Pubkey {
    Pubkey::from_str(s).expect("valid pubkey constant")
}

fn to_pool_state(d: &arb_types::DlmmState) -> Option<DlmmPoolState> {
    let p = d.params.as_ref()?;
    Some(DlmmPoolState {
        active_id: d.active_bin_id,
        bin_step: d.bin_step_bps,
        status: d.status,
        parameters: DlmmParameters {
            bin_step: d.bin_step_bps,
            base_factor: p.base_factor,
            filter_period: p.filter_period,
            decay_period: p.decay_period,
            reduction_factor: p.reduction_factor,
            variable_fee_control: p.variable_fee_control,
            max_volatility_accumulator: p.max_volatility_accumulator,
            protocol_share: p.protocol_share,
            base_fee_power_factor: p.base_fee_power_factor,
            collect_fee_mode: p.collect_fee_mode,
        },
        v_parameters: DlmmVariableParameters {
            volatility_accumulator: d.volatility_accumulator,
            volatility_reference: d.volatility_reference,
            index_reference: d.index_reference,
            last_update_timestamp: d.last_update_timestamp,
        },
    })
}

async fn fetch_pool(
    rpc: &RpcPool,
    pool: &Pubkey,
) -> anyhow::Result<(arb_types::DlmmState, Vec<arb_types::DlmmBin>)> {
    let accounts = rpc
        .get_multiple_accounts(&[&pool.to_string()])
        .await?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let acc = accounts
        .first()
        .ok_or_else(|| anyhow::anyhow!("LbPair {pool} not found"))?;
    let data = decode_account_data(acc).ok_or_else(|| anyhow::anyhow!("undecodable LbPair"))?;
    let state = decode_dlmm_state(&data).ok_or_else(|| anyhow::anyhow!("LbPair decode failed"))?;

    let idx = bin_id_to_bin_array_index(state.active_bin_id);
    let pdas: Vec<Pubkey> = [idx - 1, idx, idx + 1]
        .iter()
        .map(|i| derive_meteora_bin_array_pda(pool, *i))
        .collect();
    let keys: Vec<String> = pdas.iter().map(|p| p.to_string()).collect();
    let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
    let bin_accounts = rpc.get_multiple_accounts(&key_refs).await?;
    let mut bins = Vec::new();
    for acc in bin_accounts.into_iter().flatten() {
        if let Some(raw) = decode_account_data(&acc) {
            bins.extend(decode_bin_array(&raw));
        }
    }
    Ok((state, bins))
}

fn to_math_bins(bins: &[arb_types::DlmmBin]) -> Vec<DlmmBin> {
    bins.iter()
        .map(|b| DlmmBin {
            id: b.bin_id,
            amount_x: b.amount_x.min(u64::MAX as u128) as u64,
            amount_y: b.amount_y.min(u64::MAX as u128) as u64,
            price_q64: 0,
        })
        .collect()
}

fn flash_fee_lamports(borrow: u128) -> u128 {
    (borrow * KAMINO_WSOL_FLASH_FEE_SF + (1u128 << SCALE) - 1) >> SCALE
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let rpc = RpcPool::from_env();

    let pool_a = pk(arb_constants::METEORA_SOL_USDC_POOL);
    let pool_b = pk("BVRbyLjjfSBcoyiYFuxbgKYnWuiFaF9CSXEa5vdSZ9Hh");
    let (state_a, bins_a) = fetch_pool(&rpc, &pool_a).await?;
    let (state_b, bins_b) = fetch_pool(&rpc, &pool_b).await?;
    let pool_a_state = to_pool_state(&state_a).expect("params");
    let pool_b_state = to_pool_state(&state_b).expect("params");

    for (name, st, bins) in [
        ("5rCf1DM8 (SOL/USDC)", &pool_a_state, &bins_a),
        ("BVRbyLjj (SOL/USDC)", &pool_b_state, &bins_b),
    ] {
        let mb = to_math_bins(bins);
        let cap_y = dlmm_multi_bin_max_input(st.active_id, st.bin_step, &mb, true);
        let cap_x = dlmm_multi_bin_max_input(st.active_id, st.bin_step, &mb, false);
        println!(
            "{name}: active_id={} step={} bins_decoded={} cap(X->Y)={cap_y:?} lamports cap(Y->X)={cap_x:?} lamports",
            st.active_id,
            st.bin_step,
            bins.len()
        );
    }

    println!("\nsize ladder (WSOL lamports) — cycle 5rCf1DM8 X->Y then BVRbyLjj Y->X:");
    println!(
        "{:>12} {:>18} {:>18} {:>14} {:>12} {:>12} {:>14}",
        "borrow", "out_pool1", "final_out", "gross", "flash_fee", "network", "NET"
    );
    let ma = to_math_bins(&bins_a);
    let mbb = to_math_bins(&bins_b);
    for borrow in [
        10_000_000u128,
        100_000_000,
        500_000_000,
        1_000_000_000,
        5_000_000_000,
        10_000_000_000,
    ] {
        let borrow64 = u64::try_from(borrow).unwrap_or(u64::MAX);
        let out1 = match quote_dlmm_view(&pool_a_state, &ma, borrow64, true, None) {
            Ok(q) => q.amount_out as u128,
            Err(e) => {
                println!("{borrow64:>12} leg1 failed: {e}");
                continue;
            }
        };
        // USDC (6dp) -> WSOL leg input: raw passthrough of the quoted amount.
        let in2 = u64::try_from(out1).unwrap_or(u64::MAX);
        let final_out = match quote_dlmm_view(&pool_b_state, &mbb, in2, false, None) {
            Ok(q) => q.amount_out as u128,
            Err(e) => {
                println!("{borrow64:>12} leg2 failed: {e}");
                continue;
            }
        };
        let gross = final_out as i128 - borrow as i128;
        let fee = flash_fee_lamports(borrow);
        let net = gross - fee as i128 - NETWORK_COST_LAMPORTS as i128;
        println!(
            "{borrow64:>12} {out1:>18} {final_out:>18} {gross:>14} {fee:>12} {NETWORK_COST_LAMPORTS:>12} {net:>14}"
        );
    }
    let _ = DexProtocol::MeteoraDlmm;
    Ok(())
}
