//! MCP Tool Definitions and Executors for Pump.fun and Solana Arbitrage Bot.

use arb_constants::{PUMP_FUN_BONDING_CURVE_SEED, PUMP_FUN_PROGRAM_ID, WSOL_MINT};
use arb_dex::pump_fun::PumpFunBondingCurve;
use arb_ingest::pumpfun_discovery::PumpFunDiscoveryClient;
use arb_types::{MintId, PoolId};
use serde_json::{Value, json};
use solana_pubkey::Pubkey;
use std::str::FromStr;

/// Tool definition structure for MCP `tools/list`.
pub fn get_tool_definitions() -> Value {
    json!([
        {
            "name": "get_top_pumpfun_pools",
            "description": "Discovery-only list of Pump.fun candidates from the discovery provider. This is not authoritative on-chain state; verify accounts through Yellowstone before quoting or trading.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "description": "Number of top pools to return (default 30, max 50)",
                        "default": 30
                    }
                }
            }
        },
        {
            "name": "quote_pumpfun",
            "description": "Quote a Pump.fun bonding curve from explicit caller-supplied account state and fee BPS. No baseline reserves or fee defaults are used; the tool does not independently verify the snapshot.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "mint": {"type": "string", "description": "Token mint address"},
                    "creator": {"type": "string", "description": "BondingCurve creator pubkey from the same account snapshot"},
                    "amount_in_raw": {"type": "integer", "description": "Exact input amount in raw units: lamports for buy, token raw units for sell"},
                    "is_buy": {"type": "boolean", "description": "True for SOL-to-token buy; false for token-to-SOL sell"},
                    "virtual_sol_reserves": {"type": "integer"},
                    "virtual_token_reserves": {"type": "integer"},
                    "real_sol_reserves": {"type": "integer"},
                    "real_token_reserves": {"type": "integer"},
                    "token_total_supply": {"type": "integer"},
                    "fee_bps": {"type": "integer", "description": "Fee BPS decoded from the current Pump FeeConfig/Global state"},
                    "token_decimals": {"type": "integer", "description": "Decimals decoded from the mint account"},
                    "complete": {"type": "boolean", "description": "BondingCurve complete flag from the same snapshot"},
                    "is_mayhem_mode": {"type": "boolean", "description": "BondingCurve mayhem mode flag"},
                    "is_cashback_coin": {"type": "boolean", "description": "BondingCurve cashback flag"}
                },
                "required": ["mint", "creator", "amount_in_raw", "is_buy", "virtual_sol_reserves", "virtual_token_reserves", "real_sol_reserves", "real_token_reserves", "token_total_supply", "fee_bps", "token_decimals", "complete", "is_mayhem_mode", "is_cashback_coin"]
            }
        },
    ])
}

/// Execute a called tool and return JSON text output.
pub async fn execute_tool(name: &str, arguments: &Value) -> Result<Value, String> {
    match name {
        "get_top_pumpfun_pools" => {
            let limit = arguments
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(30) as usize;

            let client = PumpFunDiscoveryClient::new();
            match client.fetch_top_coins(limit).await {
                Ok(coins) => {
                    let pools = client.select_top_30(&coins);
                    Ok(json!({
                        "count": pools.len(),
                        "source": "BankkRoll/pumpfun-apis + pump.fun frontend",
                        "pools": pools.into_iter().map(|p| {
                            json!({
                                "pool_id": p.pool_id.0.to_string(),
                                "protocol": "PumpFun",
                                "base_mint": p.base_mint.0.to_string(),
                                "quote_mint": p.quote_mint.0.to_string(),
                                "fee_bps": p.fee_bps,
                                "liquidity_usd": p.liquidity_usd,
                            })
                        }).collect::<Vec<_>>()
                    }))
                }
                Err(e) => Err(format!("Failed to fetch Pump.fun top pools: {e}")),
            }
        }
        "quote_pumpfun" => {
            let mint_str = arguments.get("mint").and_then(Value::as_str)
                .ok_or_else(|| "Missing required parameter 'mint'".to_string())?;
            let mint_pk = Pubkey::from_str(mint_str)
                .map_err(|e| format!("Invalid mint address: {e}"))?;
            let creator_str = arguments.get("creator").and_then(Value::as_str)
                .ok_or_else(|| "Missing required parameter 'creator'".to_string())?;
            let creator = Pubkey::from_str(creator_str)
                .map_err(|e| format!("Invalid creator address: {e}"))?;
            let amount_in = arguments.get("amount_in_raw").and_then(Value::as_u64)
                .ok_or_else(|| "Missing/invalid amount_in_raw".to_string())?;
            let is_buy = arguments.get("is_buy").and_then(Value::as_bool)
                .ok_or_else(|| "Missing/invalid is_buy".to_string())?;
            let fee_bps = arguments.get("fee_bps").and_then(Value::as_u64)
                .and_then(|v| u16::try_from(v).ok())
                .ok_or_else(|| "Missing/invalid fee_bps".to_string())?;
            let token_decimals = arguments.get("token_decimals").and_then(Value::as_u64)
                .and_then(|v| u8::try_from(v).ok())
                .ok_or_else(|| "Missing/invalid token_decimals".to_string())?;
            let read_u64 = |key: &str| -> Result<u64, String> {
                arguments.get(key).and_then(Value::as_u64)
                    .ok_or_else(|| format!("Missing/invalid {key}"))
            };
            let pump_prog = Pubkey::from_str(PUMP_FUN_PROGRAM_ID).map_err(|e| e.to_string())?;
            let (bc_pda, _) = Pubkey::find_program_address(
                &[PUMP_FUN_BONDING_CURVE_SEED, mint_pk.as_ref()],
                &pump_prog,
            );
            let wsol_pk = Pubkey::from_str(WSOL_MINT).map_err(|e| e.to_string())?;
            let is_mayhem_mode = arguments.get("is_mayhem_mode").and_then(Value::as_bool)
                .ok_or_else(|| "Missing/invalid is_mayhem_mode".to_string())?;
            let is_cashback_coin = arguments.get("is_cashback_coin").and_then(Value::as_bool)
                .ok_or_else(|| "Missing/invalid is_cashback_coin".to_string())?;
            let mut curve = PumpFunBondingCurve::new(
                PoolId(bc_pda), MintId(wsol_pk), MintId(mint_pk),
            ).with_fee_bps(fee_bps)
                .map_err(|e| format!("Invalid fee config: {e}"))?
                .with_token_decimals(token_decimals)
                .with_mode_flags(is_mayhem_mode, is_cashback_coin);
            curve.creator = creator;
            curve.virtual_sol_reserves = read_u64("virtual_sol_reserves")?;
            curve.virtual_token_reserves = read_u64("virtual_token_reserves")?;
            curve.real_sol_reserves = read_u64("real_sol_reserves")?;
            curve.real_token_reserves = read_u64("real_token_reserves")?;
            curve.token_total_supply = read_u64("token_total_supply")?;
            curve.complete = arguments.get("complete").and_then(Value::as_bool)
                .ok_or_else(|| "Missing/invalid complete flag".to_string())?;

            let (amount_out, fee_paid) = curve.quote_with_fee(amount_in, is_buy)
                .map_err(|e| format!("Quote failed: {e}"))?;
            Ok(json!({
                "action": if is_buy { "BUY" } else { "SELL" },
                "bonding_curve": bc_pda.to_string(),
                "mint": mint_str,
                "amount_in_raw": amount_in,
                "amount_out_raw": amount_out,
                "fee_quote_raw": fee_paid,
                "fee_bps": fee_bps,
                "token_decimals": token_decimals,
                "is_mayhem_mode": is_mayhem_mode,
                "is_cashback_coin": is_cashback_coin,
                "progress_bps": curve.progress_bps(),
                "graduation_progress_pct": (curve.progress_bps() as f64) / 100.0,
                "state_source": "caller-supplied account snapshot; not independently verified",
                "execution_authorized": false
            }))
        }
        _ => Err(format!("Unknown tool: {name}")),
    }
}
