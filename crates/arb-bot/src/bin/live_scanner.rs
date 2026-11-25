use anyhow::{Context, Result, bail};
use arb_tx::jupiter::JupiterClient;
use std::env;

/// Read-only quote diagnostic. No taker is supplied to Jupiter /order,
/// which must return transaction: null; no signer or broadcast path exists.
#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    println!("============================================================");
    println!("SOLANA ARBITRAGE BOT — READ-ONLY QUOTE DIAGNOSTIC");
    println!("============================================================");
    println!(
        "Mode: Jupiter Swap API v2 /order; no taker, no transaction, no signing, no broadcast."
    );

    let client = JupiterClient::new();
    let wsol = arb_tx::WSOL_MINT;
    let usdc = arb_tx::USDC_MINT;

    let max_trade_sol = positive_env_f64("MAX_TRADE_SIZE_SOL", 0.02)?;
    let max_lamports = (max_trade_sol * 1_000_000_000.0).floor();
    if !max_lamports.is_finite() || max_lamports < 1.0 || max_lamports > u64::MAX as f64 {
        bail!("MAX_TRADE_SIZE_SOL does not convert to a valid lamport amount");
    }
    let max_lamports = max_lamports as u64;
    let slippage_bps = env::var("MAX_SLIPPAGE_BPS")
        .ok()
        .map(|value| {
            value
                .parse::<u16>()
                .context("MAX_SLIPPAGE_BPS must be an integer")
        })
        .transpose()?
        .unwrap_or(10);
    if u64::from(slippage_bps) > arb_constants::HOP_SLIPPAGE_MAX_BPS {
        bail!("MAX_SLIPPAGE_BPS exceeds the protocol safety ceiling");
    }

    let mut sizes = vec![max_lamports];
    let small = max_lamports / 2;
    if small > 0 && small != max_lamports {
        sizes.push(small);
    }
    sizes.sort_unstable();
    sizes.dedup();

    println!(
        "Pair: WSOL <-> USDC | Max configured input: {:.6} SOL | quote slippage: {} bps",
        max_lamports as f64 / 1e9,
        slippage_bps
    );

    for amount_in in sizes {
        let forward = match client.get_quote(wsol, usdc, amount_in, slippage_bps).await {
            Ok(quote) => quote,
            Err(error) => {
                println!(
                    "QUOTE size={:.6} SOL status=NO_FORWARD_QUOTE reason={error:#}",
                    amount_in as f64 / 1e9
                );
                continue;
            }
        };
        let usdc_out = parse_amount(&forward, "outAmount")?;
        let forward_in = parse_amount(&forward, "inAmount")?;
        if forward_in != amount_in || usdc_out == 0 {
            println!("QUOTE status=INVALID_FORWARD_AMOUNT; refusing to calculate a cycle");
            continue;
        }

        tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
        let reverse = match client.get_quote(usdc, wsol, usdc_out, slippage_bps).await {
            Ok(quote) => quote,
            Err(error) => {
                println!(
                    "QUOTE size={:.6} SOL status=NO_RETURN_QUOTE reason={error:#}",
                    amount_in as f64 / 1e9
                );
                continue;
            }
        };
        let sol_out = parse_amount(&reverse, "outAmount")?;
        let reverse_in = parse_amount(&reverse, "inAmount")?;
        if reverse_in != usdc_out || sol_out == 0 {
            println!("QUOTE status=INVALID_RETURN_AMOUNT; refusing to calculate a cycle");
            continue;
        }

        let edge_lamports = i128::from(sol_out) - i128::from(amount_in);
        let edge_bps = edge_lamports as f64 / amount_in as f64 * 10_000.0;
        let edge_sol = edge_lamports as f64 / 1e9;
        println!(
            "QUOTE size={:.6} SOL -> {:.6} USDC -> {:.9} SOL | quoted_edge={:+.9} SOL ({:+.3} bps)",
            amount_in as f64 / 1e9,
            usdc_out as f64 / 1e6,
            sol_out as f64 / 1e9,
            edge_sol,
            edge_bps,
        );
        println!(
            "  Routes: {} -> {} | price impact: {} -> {}",
            route_label(&forward),
            route_label(&reverse),
            field_text(&forward, "priceImpactPct"),
            field_text(&reverse, "priceImpactPct"),
        );
        println!(
            "  Status: QUOTE_ONLY_NOT_NET_PROFIT — excludes flash-loan premium, network base fee, priority fee, Jito tip, account/rent costs and landing risk."
        );
    }

    println!(
        "Scan complete. The execution engine remains the sole source of executable opportunities."
    );
    Ok(())
}

fn positive_env_f64(name: &str, default: f64) -> Result<f64> {
    let value = match env::var(name) {
        Ok(raw) => raw
            .trim()
            .parse::<f64>()
            .with_context(|| format!("{name} must be numeric"))?,
        Err(_) => default,
    };
    if !value.is_finite() || value <= 0.0 {
        bail!("{name} must be positive and finite");
    }
    Ok(value)
}

fn parse_amount(quote: &serde_json::Value, field: &str) -> Result<u64> {
    quote
        .get(field)
        .and_then(serde_json::Value::as_str)
        .and_then(|value| value.parse::<u64>().ok())
        .with_context(|| format!("Jupiter quote has no valid {field}"))
}

fn route_label(quote: &serde_json::Value) -> String {
    let mut labels = Vec::new();
    if let Some(plan) = quote.get("routePlan").and_then(serde_json::Value::as_array) {
        for step in plan {
            if let Some(label) = step
                .get("swapInfo")
                .and_then(|info| info.get("label"))
                .and_then(serde_json::Value::as_str)
            {
                if !labels.iter().any(|existing| existing == label) {
                    labels.push(label.to_string());
                }
            }
        }
    }
    let router = quote
        .get("router")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    if labels.is_empty() {
        router.to_string()
    } else {
        format!("{router}:{}", labels.join(">"))
    }
}

fn field_text<'a>(quote: &'a serde_json::Value, field: &str) -> &'a str {
    quote
        .get(field)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("n/a")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amount_parser_rejects_missing_or_invalid_amounts() {
        assert_eq!(
            parse_amount(&serde_json::json!({"outAmount":"42"}), "outAmount").unwrap(),
            42
        );
        assert!(parse_amount(&serde_json::json!({}), "outAmount").is_err());
        assert!(parse_amount(&serde_json::json!({"outAmount":"0x10"}), "outAmount").is_err());
    }

    #[test]
    fn route_label_includes_router_and_venue_labels() {
        let quote = serde_json::json!({
            "router": "metis",
            "routePlan": [
                {"swapInfo":{"label":"Raydium"}},
                {"swapInfo":{"label":"Orca"}},
                {"swapInfo":{"label":"Raydium"}}
            ]
        });
        assert_eq!(route_label(&quote), "metis:Raydium>Orca");
    }
}
