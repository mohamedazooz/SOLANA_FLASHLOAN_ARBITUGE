//! Forensic probe: fetch a live Meteora DLMM pool via the real RpcPool and
//! replicate the EXACT registry pipeline: decode state -> attach the same
//! bin arrays `dlmm_bin_array_pdas` derives (±arrays_needed) -> compute
//! reserves -> quote both swap directions with the real bin-traversal quote.
//! Run: cargo run --release -p arb-ingest --example dlmm_probe -- <POOL_PUBKEY>

use base64::prelude::*;
use solana_pubkey::Pubkey;
use std::str::FromStr;

#[tokio::main]
async fn main() {
    let pk = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "HRYEjwdo3bZ1TpXKWKcezqiwSV2Ywuh4LxMa2PzoCnG6".to_string());
    let pool = Pubkey::from_str(&pk).expect("valid pool pubkey");

    let rpc = arb_ingest::RpcPool::from_env();
    let pk_str = pool.to_string();
    let refs = [pk_str.as_str()];
    let (accounts, slot) = rpc
        .get_multiple_accounts_with_slot(&refs)
        .await
        .expect("rpc fetch failed");
    let acc = accounts
        .first()
        .expect("account missing")
        .as_ref()
        .expect("null account");
    let data_arr = acc.get("data").and_then(|d| d.as_array()).expect("data");
    let b64 = data_arr.first().and_then(|s| s.as_str()).expect("b64");
    let bytes = BASE64_STANDARD.decode(b64).expect("b64 decode");
    println!("slot={slot} data_len={}", bytes.len());

    let Some(d) = arb_ingest::decode_dlmm_state(&bytes) else {
        println!("decode_dlmm_state: None");
        return;
    };
    println!(
        "state: active_bin_id={} bin_step_bps={} fee_bps={:?}",
        d.active_bin_id,
        d.bin_step_bps,
        arb_ingest::decode_dlmm_fee_bps(&d)
    );
    let tx = Pubkey::new_from_array(bytes[88..120].try_into().unwrap());
    let ty = Pubkey::new_from_array(bytes[120..152].try_into().unwrap());
    println!("token_x={tx} token_y={ty}");

    // Mirror dlmm_bin_array_pdas: arrays_needed = (500/step + 1)/70 + 1, indices
    // index-i..index+i for i in 0..arrays_needed (deduped).
    let index = arb_ingest::registry::bin_id_to_bin_array_index(d.active_bin_id);
    let step = u32::from(d.bin_step_bps.max(1));
    let bins_traversed = 500u32 / step + 1;
    let arrays_needed = (bins_traversed as i64).div_euclid(70) + 1;
    let mut pdas: Vec<Pubkey> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for i in 0..arrays_needed {
        for idx in [index - i, index + i] {
            if seen.insert(idx) {
                pdas.push(arb_ingest::registry::derive_meteora_bin_array_pda(
                    &pool, idx,
                ));
            }
        }
    }
    println!("arrays_needed={arrays_needed} pdas={}", pdas.len());
    let pda_strings: Vec<String> = pdas.iter().map(|p| p.to_string()).collect();
    let refs: Vec<&str> = pda_strings.iter().map(|s| s.as_str()).collect();
    let (baccs, _) = rpc
        .get_multiple_accounts_with_slot(&refs)
        .await
        .expect("bin array fetch failed");
    let mut bins = Vec::new();
    for (val, pda) in baccs.iter().zip(pdas.iter()) {
        let Some(val) = val else {
            println!("bin_array {pda}: null");
            continue;
        };
        let Some(arr) = val.get("data").and_then(|d| d.as_array()) else {
            continue;
        };
        let Some(b64s) = arr.first().and_then(|s| s.as_str()) else {
            continue;
        };
        let Ok(raw) = BASE64_STANDARD.decode(b64s) else {
            continue;
        };
        bins.extend(arb_ingest::registry::decode_bin_array(&raw));
    }
    println!("total bins={}", bins.len());
    let active = bins.iter().find(|b| b.bin_id == d.active_bin_id);
    println!(
        "active bin: {:?}",
        active.map(|b| (b.bin_id, b.amount_x, b.amount_y))
    );

    let mut d2 = d.clone();
    d2.bins = bins;
    let x_depth: u128 = d2
        .bins
        .iter()
        .filter(|b| b.bin_id >= d2.active_bin_id)
        .map(|b| b.amount_x)
        .sum();
    let y_depth: u128 = d2
        .bins
        .iter()
        .filter(|b| b.bin_id <= d2.active_bin_id)
        .map(|b| b.amount_y)
        .sum();
    println!("x_depth(bins>=active)={x_depth} y_depth(bins<=active)={y_depth}");

    println!(
        "compute_dlmm_reserves(x=tx,y=ty): {:?}",
        arb_ingest::compute_dlmm_reserves(&d2, &bytes, &tx, &ty)
    );
    println!(
        "compute_dlmm_reserves(x=ty,y=tx): {:?}",
        arb_ingest::compute_dlmm_reserves(&d2, &bytes, &ty, &tx)
    );
    quote_forensics::run(&d2);
}

// ---- Directional quote forensics: replicate quote_pool's Dlmm arm ----
// pool_base orientation determines a_to_b = (pool_base == from_mint).
// Test both orientations for a 100_000 raw-unit input from each side.
mod quote_forensics {
    use arb_math::dlmm::{
        DlmmBin as ArbDlmmBin, DlmmParameters, DlmmPoolState, DlmmVariableParameters,
        quote_dlmm_view,
    };

    pub fn run(d: &arb_types::DlmmState) {
        let pool = DlmmPoolState {
            active_id: d.active_bin_id,
            bin_step: d.bin_step_bps,
            status: d.status,
            parameters: DlmmParameters {
                bin_step: d.bin_step_bps,
                base_factor: d.params.as_ref().map(|p| p.base_factor).unwrap_or(0),
                filter_period: d.params.as_ref().map(|p| p.filter_period).unwrap_or(0),
                decay_period: d.params.as_ref().map(|p| p.decay_period).unwrap_or(0),
                reduction_factor: d.params.as_ref().map(|p| p.reduction_factor).unwrap_or(0),
                variable_fee_control: d
                    .params
                    .as_ref()
                    .map(|p| p.variable_fee_control)
                    .unwrap_or(0),
                max_volatility_accumulator: d
                    .params
                    .as_ref()
                    .map(|p| p.max_volatility_accumulator)
                    .unwrap_or(0),
                protocol_share: d.params.as_ref().map(|p| p.protocol_share).unwrap_or(0),
                base_fee_power_factor: d
                    .params
                    .as_ref()
                    .map(|p| p.base_fee_power_factor)
                    .unwrap_or(0),
                collect_fee_mode: d.params.as_ref().map(|p| p.collect_fee_mode).unwrap_or(0),
            },
            v_parameters: DlmmVariableParameters {
                volatility_accumulator: d.volatility_accumulator,
                volatility_reference: d.volatility_reference,
                index_reference: d.index_reference,
                last_update_timestamp: d.last_update_timestamp,
            },
        };
        let bins: Vec<ArbDlmmBin> = d
            .bins
            .iter()
            .filter_map(|b| {
                Some(ArbDlmmBin {
                    id: b.bin_id,
                    amount_x: u64::try_from(b.amount_x).ok()?,
                    amount_y: u64::try_from(b.amount_y).ok()?,
                    price_q64: 0,
                })
            })
            .collect();
        for (label, swap_for_y) in [
            ("x->y (swap_for_y=true)", true),
            ("y->x (swap_for_y=false)", false),
        ] {
            match quote_dlmm_view(&pool, &bins, 100_000, swap_for_y, None) {
                Ok(q) => println!("quote 100_000 {label}: out={}", q.amount_out),
                Err(e) => println!("quote 100_000 {label}: ERR {e}"),
            }
        }
    }
}
