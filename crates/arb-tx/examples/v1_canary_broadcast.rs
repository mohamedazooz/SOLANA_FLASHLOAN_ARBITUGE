//! V1 (SIMD-0385) transport canary — the only sanctioned path to flip
//! `v1_transport_supported` from fail-closed `false` to runtime-verified.
//!
//! What it does (in order, all evidence recorded):
//!   1. Loads the signer from `SOLANA_PRIVATE_KEY` (64-byte base58, same env as
//!      the bot). The key is NEVER printed, logged, or written to evidence.
//!   2. Fetches a REAL recent blockhash from the configured RPC
//!      (`SOLANA_RPC_PRIMARY`, falling back to `SOLANA_RPC_URL`).
//!   3. Builds a NON-FINANCIAL V1 transaction: a single SPL-Memo instruction
//!      ("SOLANA_FLASHLOAN_ARBITUGE V1 TRANSPORT CANARY"), no token accounts,
//!      no pools, no flash loan. Worst-case cost = base fee + configured
//!      priority fee (lamports), bounded well under the risk envelope.
//!   4. Compiles the real V1 message via `AdaptiveTxCompiler::compile_v1_message`,
//!      signs it (Ed25519), and assembles the V1 wire format
//!      (message at head, trailing signature).
//!   5. Submits base64 via `sendTransaction` with `skipPreflight=false` — an
//!      honest acceptance test: a node that cannot parse V1 fails here with a
//!      recorded error instead of silently dropping the wire.
//!   6. Polls `getSignatureStatuses` until processed/confirmed or timeout, and
//!      records the on-chain confirmation slot + status.
//!   7. Writes a JSON evidence record to `reports/v1_canary/`.
//!
//! Exit code 0 + `"landed": true` in the evidence is the ONLY output that may
//! be cited as V1 transport support. Any failure is recorded verbatim and the
//! flag stays `false`. This tool never retries as V0 (no silent downgrade).
//!
//! Run: `cargo run -p arb-tx --example v1_canary_broadcast --release`

use arb_tx::{
    KeypairSigner,
    adaptive::{AdaptiveTxCompiler, V1ConfigParams},
};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use serde_json::json;
use solana_pubkey::Pubkey;
use std::str::FromStr;
use std::time::{Duration, Instant};

const MEMO_PROGRAM: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";
const CANARY_MEMO: &[u8] = b"SOLANA_FLASHLOAN_ARBITUGE V1 TRANSPORT CANARY";
const STATUS_POLL_TIMEOUT: Duration = Duration::from_secs(45);
const STATUS_POLL_INTERVAL: Duration = Duration::from_millis(1_500);

/// Payload selector: `ARB_CANARY_PAYLOAD=memo` (default, SPL-Memo v2, no accounts)
/// or `system` (native System transfer of 0 lamports to self — zero BPF load,
/// zero accounts beyond the payer, deterministic success). The `system` variant
/// exists because a memo failure would otherwise be indistinguishable from a
/// V1 transport defect; a native-instruction success isolates transport from
/// program behavior.
fn canary_instruction(signer: &KeypairSigner) -> anyhow::Result<arb_tx::CompiledInstruction> {
    match std::env::var("ARB_CANARY_PAYLOAD").as_deref() {
        Ok("system") => {
            let system_program = Pubkey::from_str("11111111111111111111111111111111")?;
            let data = {
                let mut d = Vec::with_capacity(12);
                d.extend_from_slice(&2u32.to_le_bytes());
                d.extend_from_slice(&0u64.to_le_bytes());
                d
            };
            Ok(arb_tx::CompiledInstruction {
                program_id: system_program,
                accounts: vec![
                    arb_tx::AccountMeta::new(signer.pubkey(), true),
                    arb_tx::AccountMeta::new(signer.pubkey(), false),
                ],
                data,
            })
        }
        _ => Ok(arb_tx::CompiledInstruction {
            program_id: Pubkey::from_str(MEMO_PROGRAM)?,
            accounts: vec![],
            data: CANARY_MEMO.to_vec(),
        }),
    }
}

fn rpc_endpoint() -> anyhow::Result<String> {
    for key in ["SOLANA_RPC_PRIMARY", "SOLANA_RPC_URL"] {
        if let Ok(v) = std::env::var(key) {
            let v = v.trim().to_string();
            if !v.is_empty() {
                return Ok(v);
            }
        }
    }
    anyhow::bail!("no RPC endpoint configured (set SOLANA_RPC_PRIMARY)")
}

async fn rpc_call(
    client: &reqwest::Client,
    endpoint: &str,
    method: &str,
    params: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let resp = client
        .post(endpoint)
        .json(&json!({"jsonrpc": "2.0", "id": "canary", "method": method, "params": params}))
        .timeout(Duration::from_secs(20))
        .send()
        .await?;
    let val: serde_json::Value = resp.json().await?;
    if let Some(err) = val.get("error") {
        anyhow::bail!("RPC {method} error: {err}");
    }
    val.get("result")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("RPC {method} returned no result"))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let started = Instant::now();
    let secret = std::env::var("SOLANA_PRIVATE_KEY").map_err(|_| {
        anyhow::anyhow!("SOLANA_PRIVATE_KEY not set — refusing to run the canary unsigned")
    })?;
    let signer = KeypairSigner::from_base58(&secret)?;
    let endpoint = rpc_endpoint()?;
    let client = reqwest::Client::new();

    // 2. Real blockhash — a canary with a synthetic blockhash cannot land.
    let bh = rpc_call(
        &client,
        &endpoint,
        "getLatestBlockhash",
        json!([{"commitment": "confirmed"}]),
    )
    .await?;
    let blockhash_str = bh["value"]["blockhash"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("no blockhash in response"))?
        .to_string();
    let last_valid_block_height = bh["value"]["lastValidBlockHeight"]
        .as_u64()
        .unwrap_or_default();
    let blockhash: [u8; 32] = bs58::decode(&blockhash_str)
        .into_vec()?
        .try_into()
        .map_err(|_| anyhow::anyhow!("blockhash decode failed"))?;

    // 3. Non-financial V1 transaction: single instruction, no token accounts.
    let canary_ix = canary_instruction(&signer)?;
    let payload_kind = if canary_ix.program_id == Pubkey::from_str(MEMO_PROGRAM)? {
        "spl-memo"
    } else {
        "system-transfer-0"
    };
    let instructions = vec![canary_ix];
    let config = V1ConfigParams {
        priority_fee_lamports: Some(1_000),
        compute_unit_limit: Some(200_000),
        loaded_accounts_data_size_limit: Some(67_108_864),
        heap_size: None,
    };
    let message_bytes =
        AdaptiveTxCompiler::compile_v1_message(signer.pubkey(), &instructions, blockhash, config)?;
    let signature = signer.sign(&message_bytes);
    let mut wire_bytes = Vec::with_capacity(message_bytes.len() + 64);
    wire_bytes.extend_from_slice(&message_bytes);
    wire_bytes.extend_from_slice(&signature);

    // Version byte sanity: 0x80 | version = 0x81 for V1 (SIMD-0385).
    let version_byte = wire_bytes[0];
    let signature_b58 = bs58::encode(signature).into_string();

    // 4b. Preflight simulation with full logs — diagnostic evidence recorded
    // verbatim so a program-level failure is never confused with a transport
    // rejection. sigVerify=false because the RPC cannot reconstruct the
    // trailing-signature layout for simulation; the signed send below is the
    // actual acceptance test.
    let sim_result = rpc_call(
        &client,
        &endpoint,
        "simulateTransaction",
        json!([
            BASE64_STANDARD.encode(wire_bytes.as_slice()),
            {
                "encoding": "base64",
                "sigVerify": false,
                "replaceRecentBlockhash": true,
                "commitment": "processed"
            }
        ]),
    )
    .await;
    let (sim_ok, sim_error, sim_logs, sim_cu) = match &sim_result {
        Ok(v) => (
            v.get("value")
                .and_then(|x| x.get("err"))
                .is_none_or(|e| e.is_null()),
            v.get("value")
                .and_then(|x| x.get("err"))
                .filter(|e| !e.is_null())
                .map(|e| e.to_string()),
            v["value"]["logs"]
                .as_array()
                .map(|l| {
                    l.iter()
                        .filter_map(|x| x.as_str())
                        .map(String::from)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
            v["value"]["unitsConsumed"].as_u64(),
        ),
        Err(e) => (false, Some(e.to_string()), Vec::new(), None),
    };

    // 5. Honest acceptance test — preflight ON.
    let b64 = BASE64_STANDARD.encode(wire_bytes.as_slice());
    let send_start = Instant::now();
    let send_result = rpc_call(
        &client,
        &endpoint,
        "sendTransaction",
        json!([
            b64,
            {
                "encoding": "base64",
                "skipPreflight": false,
                "preflightCommitment": "processed",
                "maxRetries": 0
            }
        ]),
    )
    .await;
    let send_ms = send_start.elapsed().as_millis() as u64;

    let signature_submitted = send_result
        .as_ref()
        .ok()
        .and_then(|v| v.as_str())
        .map(String::from);

    let mut landed = false;
    let mut confirmation_status: Option<String> = None;
    let mut confirmed_slot: Option<u64> = None;
    let mut status_error: Option<String> = None;

    if let Some(sig) = &signature_submitted {
        let deadline = Instant::now() + STATUS_POLL_TIMEOUT;
        while Instant::now() < deadline {
            match rpc_call(
                &client,
                &endpoint,
                "getSignatureStatuses",
                json!([[sig], {"searchTransactionHistory": false}]),
            )
            .await
            {
                Ok(statuses) => {
                    let entry = &statuses["value"][0];
                    if !entry.is_null() {
                        if let Some(err) = entry.get("err") {
                            if !err.is_null() {
                                status_error = Some(format!("on-chain failure: {err}"));
                                break;
                            }
                        }
                        confirmation_status =
                            entry["confirmationStatus"].as_str().map(String::from);
                        confirmed_slot = entry["slot"].as_u64();
                        if matches!(
                            confirmation_status.as_deref(),
                            Some("processed") | Some("confirmed") | Some("finalized")
                        ) {
                            landed = true;
                            break;
                        }
                    }
                }
                Err(e) => {
                    status_error = Some(e.to_string());
                }
            }
            tokio::time::sleep(STATUS_POLL_INTERVAL).await;
        }
    }

    let evidence = json!({
        "generated_at_unix": started.elapsed().as_secs(),
        "transport": endpoint,
        "transaction_version": "V1",
        "version_byte": version_byte,
        "wire_size_bytes": wire_bytes.len(),
        "account_count": 1,
        "instruction_count": 1,
        "instruction_summary": format!("{payload_kind} (non-financial canary)"),
        "simulation_ok": sim_ok,
        "simulation_error": sim_error,
        "simulation_logs": sim_logs,
        "simulation_units_consumed": sim_cu,
        "blockhash": blockhash_str,
        "last_valid_block_height": last_valid_block_height,
        "signature": signature_b58,
        "rpc_acceptance": send_result.is_ok(),
        "send_latency_ms": send_ms,
        "send_error": send_result.as_ref().err().map(|e| e.to_string()),
        "simulation_preflight": "skipPreflight=false (honest acceptance test)",
        "landed": landed,
        "confirmation_status": confirmation_status,
        "confirmed_slot": confirmed_slot,
        "status_poll_error": status_error,
        "total_runtime_secs": started.elapsed().as_secs(),
        "verdict": if landed { "V1_TRANSPORT_VERIFIED" } else { "V1_TRANSPORT_NOT_VERIFIED" },
    });

    let out_dir = std::path::Path::new("reports/v1_canary");
    let _ = std::fs::create_dir_all(out_dir);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let out_path = out_dir.join(format!("canary_{ts}.json"));
    std::fs::write(&out_path, serde_json::to_string_pretty(&evidence)?)?;

    println!("{}", serde_json::to_string_pretty(&evidence)?);
    println!("evidence written to {}", out_path.display());

    if !landed {
        anyhow::bail!("V1 canary did not land; v1_transport_supported must remain false");
    }
    Ok(())
}
