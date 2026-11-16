//! Forensic probe: verify live mainnet CLMM account layouts and orientation.
//!
//! Decisive questions this probe answers with on-chain evidence:
//! 1. Which Whirlpool layout is deployed on mainnet?
//!    - OLD (pre-adaptive-fee): mint_a @101, mint_b @133, vault_a @165, vault_b @197 (LEN 525)
//!    - NEW (interleaved, orca-so/whirlpools main): mint_a @101, vault_a @133,
//!      fee_growth_a @165, mint_b @181, vault_b @213 (LEN 653)
//! 2. Do the registry's vault/mint offsets (`decode_orca_vaults`,
//!    `decode_pool_info`) match the LIVE layout?
//! 3. Does the catalog's (Orca API / Raydium v3 API) tokenA/tokenB order match
//!    the on-chain A/B mint order that `quote_hop`'s
//!    `a_to_b = (pool_base == from_mint)` mapping assumes?
//!
//! Known anchors (arb-constants + Raydium v3 API, fetched live):
//!   WSOL  mint = So11111111111111111111111111111111111111112
//!   USDC  mint = EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v
//!   Orca SOL-USDC pool  = Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE
//!   Orca SOL vault      = EUuUbDcafPrmVTD5M6qoJAoyyNbihBhugADAxRMn5he9
//!   Orca USDC vault     = 2WLWEuKDgkDUccTpbwYp1GToYktiSB1cXvreHUwiSUVP
//!   Raydium CLMM CC/USDC  pool = 5mfjy2Ns9MWuVB6z3N1oFGJKSd6RyG2PAdyH2wDKDaMY
//!     (catalog mintA = AHyKLHgodnahePddu4eQysxFuz2JkDVdpXqMu9x6Yw1P "CC",
//!      catalog mintB = USDC)
//!   Raydium CLMM WSOL/CC  pool = 344moB3XUcunm6Ynwk3XhyUsMVnxxMqpAZYU3NHcoi3h
//!     (catalog mintA = WSOL, catalog mintB = FHc5J2nBr2HtmYrHZ2fDDBRNpz1qr5qyY1hooBDsgJk7 "CC")
//!
//! Run: cargo run --release -p arb-ingest --example clmm_probe

use base64::prelude::*;
use solana_pubkey::Pubkey;
use std::str::FromStr;

const WSOL: &str = "So11111111111111111111111111111111111111112";
const USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const ORCA_POOL: &str = "Czfq3xZZDmsdGdUyrNLtRhGc47cXcZtLG4crryfu44zE";
const ORCA_SOL_VAULT: &str = "EUuUbDcafPrmVTD5M6qoJAoyyNbihBhugADAxRMn5he9";
const ORCA_USDC_VAULT: &str = "2WLWEuKDgkDUccTpbwYp1GToYktiSB1cXvreHUwiSUVP";
const RAY_CLMM_CC_USDC: &str = "5mfjy2Ns9MWuVB6z3N1oFGJKSd6RyG2PAdyH2wDKDaMY";
const RAY_CLMM_CC_MINT: &str = "AHyKLHgodnahePddu4eQysxFuz2JkDVdpXqMu9x6Yw1P";
const RAY_CLMM_WSOL_CC: &str = "344moB3XUcunm6Ynwk3XhyUsMVnxxMqpAZYU3NHcoi3h";
const RAY_CLMM_CC_MINT_2: &str = "FHc5J2nBr2HtmYrHZ2fDDBRNpz1qr5qyY1hooBDsgJk7";

fn label(pk: &Pubkey) -> &'static str {
    let s = pk.to_string();
    match s.as_str() {
        WSOL => "WSOL-mint",
        USDC => "USDC-mint",
        ORCA_SOL_VAULT => "ORCA-SOL-VAULT",
        ORCA_USDC_VAULT => "ORCA-USDC-VAULT",
        RAY_CLMM_CC_MINT | RAY_CLMM_CC_MINT_2 => "CC-mint",
        _ => "?",
    }
}

#[tokio::main]
async fn main() {
    let rpc = arb_ingest::RpcPool::from_env();

    let orca_windows: [usize; 6] = [101, 133, 165, 181, 197, 213];
    let ray_windows: [usize; 4] = [73, 105, 137, 169];
    for (name, addr, windows) in [
        ("ORCA whirlpool (SOL/USDC)", ORCA_POOL, &orca_windows[..]),
        ("RAYDIUM CLMM (CC/USDC)", RAY_CLMM_CC_USDC, &ray_windows[..]),
        ("RAYDIUM CLMM (WSOL/CC)", RAY_CLMM_WSOL_CC, &ray_windows[..]),
    ] {
        println!("=== {name} [{addr}] ===");
        let _pool = Pubkey::from_str(addr).expect("valid pool");
        let refs = [addr];
        let (accounts, slot) = rpc
            .get_multiple_accounts_with_slot(&refs)
            .await
            .expect("rpc fetch failed");
        let acc = accounts
            .first()
            .expect("account missing")
            .as_ref()
            .expect("null account");
        let owner = acc
            .get("owner")
            .and_then(|o| o.as_str())
            .unwrap_or("<no owner>");
        let data_arr = acc.get("data").and_then(|d| d.as_array()).expect("data");
        let b64 = data_arr.first().and_then(|s| s.as_str()).expect("b64");
        let bytes = BASE64_STANDARD.decode(b64).expect("b64 decode");
        println!("slot={slot} data_len={} owner={owner}", bytes.len());
        println!("  (525 = OLD whirlpool layout, 653 = NEW interleaved layout)");

        let mut candidates: Vec<Pubkey> = Vec::new();
        for &w in windows {
            if w + 32 > bytes.len() {
                println!("  @{w}: OUT OF RANGE");
                continue;
            }
            let pk = Pubkey::new_from_array(bytes[w..w + 32].try_into().unwrap());
            println!("  @{w}..{}: {} {}", w + 32, pk, label(&pk));
            candidates.push(pk);
        }

        // Classify each candidate pubkey by fetching it: a mint account is 82
        // bytes, an SPL token account is 165 bytes, both owned by the Token
        // program. This catches misaligned reads that accidentally produce
        // valid-looking but wrong pubkeys.
        let uniq: Vec<String> = {
            let mut seen = std::collections::BTreeSet::new();
            candidates
                .iter()
                .filter(|p| **p != Pubkey::default())
                .map(|p| p.to_string())
                .filter(|s| seen.insert(s.clone()))
                .collect()
        };
        let refs: Vec<&str> = uniq.iter().map(|s| s.as_str()).collect();
        let (sub, _) = rpc
            .get_multiple_accounts_with_slot(&refs)
            .await
            .expect("candidate fetch failed");
        println!("  --- candidate account classification ---");
        for (s, val) in uniq.iter().zip(sub.iter()) {
            match val {
                Some(v) => {
                    let o = v.get("owner").and_then(|o| o.as_str()).unwrap_or("?");
                    let d = v
                        .get("data")
                        .and_then(|d| d.as_array())
                        .and_then(|a| a.first())
                        .and_then(|s| s.as_str())
                        .map(|b64| BASE64_STANDARD.decode(b64).map(|b| b.len()))
                        .and_then(|r| r.ok())
                        .unwrap_or(0);
                    let kind = match (o, d) {
                        ("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", 82) => "MINT (SPL)",
                        ("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA", 165) => "TOKEN ACCOUNT",
                        _ => "OTHER",
                    };
                    println!("  {s}: owner={o} len={d} -> {kind}");
                }
                None => println!("  {s}: NULL (not an account — garbage pubkey)"),
            }
        }
        println!();
    }
}
