//! Read-only wallet probe: prints pubkey, SOL balance, and WSOL/USDC ATA
//! existence for the configured signer. Never broadcasts anything.
//!
//! Usage: cargo run -p arb-tx --example wallet_probe

use arb_tx::KeypairSigner;
use serde_json::json;
use solana_pubkey::Pubkey;
use std::str::FromStr;

#[tokio::main]
async fn main() {
    let mut secret: Option<String> = std::env::var("SOLANA_PRIVATE_KEY").ok();
    if secret.is_none() {
        for path in [".env", "../../.env"] {
            if let Ok(content) = std::fs::read_to_string(path) {
                for line in content.lines() {
                    if let Some(v) = line.strip_prefix("SOLANA_PRIVATE_KEY=") {
                        secret = Some(v.trim().trim_matches('"').to_string());
                        break;
                    }
                }
            }
            if secret.is_some() {
                break;
            }
        }
    }
    let Some(secret) = secret else {
        eprintln!("no SOLANA_PRIVATE_KEY configured");
        return;
    };
    let signer = KeypairSigner::from_base58(secret.trim()).expect("valid SOLANA_PRIVATE_KEY");
    println!("pubkey: {}", signer.pubkey());
    probe(&signer).await;
}

async fn rpc(
    client: &reqwest::Client,
    url: &str,
    body: serde_json::Value,
) -> Option<serde_json::Value> {
    client
        .post(url)
        .json(&body)
        .send()
        .await
        .ok()?
        .json::<serde_json::Value>()
        .await
        .ok()
}

async fn probe(signer: &KeypairSigner) {
    let url = std::env::var("SOLANA_RPC_PRIMARY")
        .ok()
        .filter(|u| !u.trim().is_empty())
        .or_else(|| {
            std::fs::read_to_string(".env")
                .ok()
                .and_then(|c| {
                    c.lines()
                        .find_map(|l| l.strip_prefix("SOLANA_RPC_PRIMARY="))
                        .map(|v| v.trim().trim_matches('"').to_string())
                })
                .filter(|u| !u.is_empty())
        })
        .unwrap_or_else(|| "https://solana-rpc.publicnode.com".to_string());

    let client = reqwest::Client::new();
    let pk = signer.pubkey().to_string();

    let bal = rpc(
        &client,
        &url,
        json!({"jsonrpc":"2.0","id":1,"method":"getBalance","params":[&pk]}),
    )
    .await
    .and_then(|v| v["result"]["value"].as_u64());
    println!("SOL balance: {:?} lamports", bal);

    for (name, mint) in [
        ("WSOL", arb_constants::WSOL_MINT),
        ("USDC", arb_constants::USDC_MINT),
    ] {
        let mint = Pubkey::from_str(mint).unwrap();
        let ata = arb_tx::get_associated_token_address(&signer.pubkey(), &mint);
        let acc = rpc(
            &client,
            &url,
            json!({"jsonrpc":"2.0","id":1,"method":"getAccountInfo","params":[ata.to_string(), {"encoding":"base64"}]}),
        )
        .await;
        let exists = acc
            .map(|v| !v["result"]["value"].is_null())
            .unwrap_or(false);
        println!(
            "{name} ATA {ata}: {}",
            if exists { "EXISTS" } else { "missing" }
        );
        if exists {
            let amt = rpc(
                &client,
                &url,
                json!({"jsonrpc":"2.0","id":1,"method":"getTokenAccountBalance","params":[ata.to_string()]}),
            )
            .await
            .and_then(|v| v["result"]["value"]["amount"].as_str().map(String::from));
            println!("{name} balance: {:?}", amt);
        }
    }
}
