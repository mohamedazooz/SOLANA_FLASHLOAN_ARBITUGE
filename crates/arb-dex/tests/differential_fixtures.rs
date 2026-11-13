//! Differential fixture harness (master-prompt Phase 12.13 / 13.2).
//!
//! Compares the local quote engines in `arb-dex` against golden values that
//! were produced OFFLINE by the official protocol SDKs on identical pool
//! state. The fixture format and per-SDK generation commands are documented
//! in `fixtures/differential/README.md`.
//!
//! Rules:
//! - Golden values are never fabricated here. When no fixtures exist the
//!   suite passes vacuously and prints an explicit reminder that Phase
//!   12.13 is still OPEN.
//! - A fixture whose decoder rejects the account, whose dependency accounts
//!   are not recognized by the pool, or whose quote errors is a hard
//!   failure, not a silent skip.
//! - Differences are classified: exact_match, expected_rounding (within the
//!   fixture's tolerance_bps), or MISMATCH. Per the reconciliation policy a
//!   mismatch disables that pool from execution until resolved.

use arb_dex::{decode_amm_pool, AmmPool};
use arb_types::{DexProtocol, PoolId};
use serde_json::Value;
use solana_pubkey::Pubkey;
use std::path::{Path, PathBuf};
use std::str::FromStr;

fn fixtures_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("ARB_DIFFERENTIAL_FIXTURES_DIR") {
        let path = PathBuf::from(dir);
        return if path.is_dir() { Some(path) } else { None };
    }
    // `cargo test` runs with the working directory set to the crate root.
    let path = PathBuf::from("../../fixtures/differential");
    if path.is_dir() { Some(path) } else { None }
}

/// Protocols the shared dispatcher can currently decode. DAMM v2, LaunchLab,
/// and PumpSwap are intentionally absent: `decode_amm_pool` does not dispatch
/// them yet, and the harness fails such fixtures as HARNESS_GAP instead of
/// skipping them silently.
fn parse_protocol(name: &str) -> Option<DexProtocol> {
    match name {
        "raydium_v4" => Some(DexProtocol::RaydiumV4),
        "raydium_cpmm" => Some(DexProtocol::RaydiumCpmm),
        "raydium_clmm" => Some(DexProtocol::RaydiumClmm),
        "orca_whirlpool" => Some(DexProtocol::OrcaWhirlpool),
        "meteora_dlmm" => Some(DexProtocol::MeteoraDlmm),
        "pump_fun" => Some(DexProtocol::PumpFun),
        _ => None,
    }
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    let s = s.strip_prefix("0x").unwrap_or(s);
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

fn pool_id_from_hex(s: &str) -> Option<PoolId> {
    let bytes = decode_hex(s)?;
    let arr: [u8; 32] = bytes.try_into().ok()?;
    Some(PoolId::new(arr))
}

fn collect_json_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_json_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("json") {
            out.push(path);
        }
    }
}

/// Returns the classification on success, or a failure reason.
fn run_fixture(fixture: &Value) -> Result<&'static str, String> {
    let protocol_name = fixture["protocol"].as_str().ok_or("missing 'protocol'")?;
    match protocol_name {
        "meteora_damm_v2" | "raydium_launchlab" | "pump_swap" => {
            return Err(format!(
                "HARNESS_GAP: decode_amm_pool does not dispatch {protocol_name} yet; extend the dispatcher before generating fixtures for it"
            ));
        }
        other => {
            if parse_protocol(other).is_none() {
                return Err(format!("unknown protocol '{other}'"));
            }
        }
    }
    let protocol = parse_protocol(protocol_name).expect("checked above");

    let pool_hex = fixture["pool_account_hex"].as_str().ok_or("missing 'pool_account_hex'")?;
    let account_data = decode_hex(pool_hex).ok_or("'pool_account_hex' is not valid hex")?;
    let pool_address = fixture["pool_address"].as_str().ok_or("missing 'pool_address'")?;
    let pool_id = pool_id_from_hex(pool_address)
        .ok_or("'pool_address' is not a 32-byte hex pubkey")?;

    let mut pool = decode_amm_pool(protocol, pool_id, &account_data)
        .ok_or("decoder rejected the pool account bytes (check protocol, layout version, and account size)")?;

    if let Some(deps) = fixture["dependency_accounts"].as_array() {
        for dep in deps {
            let pubkey_str = dep["pubkey"].as_str().ok_or("dependency missing 'pubkey'")?;
            let data_hex = dep["data_hex"].as_str().ok_or("dependency missing 'data_hex'")?;
            let dep_bytes = decode_hex(data_hex).ok_or("dependency 'data_hex' is not valid hex")?;
            let pubkey = Pubkey::from_str(pubkey_str)
                .map_err(|e| format!("bad dependency pubkey '{pubkey_str}': {e}"))?;
            let applied = pool
                .apply_dependency_account(&pubkey, &dep_bytes)
                .map_err(|e| format!("apply_dependency_account({pubkey_str}) failed: {e}"))?;
            if !applied {
                return Err(format!("dependency {pubkey_str} was not recognized by this pool"));
            }
        }
    }

    let a_to_b = match fixture["direction"].as_str() {
        Some("a_to_b") => true,
        Some("b_to_a") => false,
        _ => return Err("missing/invalid 'direction' (expected 'a_to_b' or 'b_to_a')".to_string()),
    };
    let amount_in = fixture["amount_in"].as_u64().ok_or("missing 'amount_in'")?;
    let expected = fixture["expected_amount_out"].as_u64().ok_or("missing 'expected_amount_out'")?;
    let tolerance_bps = fixture["tolerance_bps"].as_u64().unwrap_or(0);

    let actual = pool
        .quote(amount_in, a_to_b)
        .map_err(|e| format!("local quote failed: {e}"))?;

    if actual == expected {
        return Ok("exact_match");
    }
    let diff = actual.abs_diff(expected);
    let diff_bps = (diff as u128).saturating_mul(10_000) / u128::from(expected.max(1));
    if diff_bps <= u128::from(tolerance_bps) {
        return Ok("expected_rounding");
    }
    Err(format!(
        "MISMATCH: expected_amount_out={expected} local_quote={actual} diff={diff} ({diff_bps} bps, tolerance {tolerance_bps} bps); per the reconciliation policy this pool must be disabled from execution until resolved"
    ))
}

#[test]
fn differential_fixtures_match_local_quote_engines() {
    let Some(dir) = fixtures_dir() else {
        eprintln!(
            "DIFFERENTIAL FIXTURES: none found (set ARB_DIFFERENTIAL_FIXTURES_DIR or create fixtures/differential). Phase 12.13 remains OPEN: no SDK parity evidence exists yet."
        );
        return;
    };

    let mut files = Vec::new();
    collect_json_files(&dir, &mut files);
    files.sort();
    assert!(
        !files.is_empty(),
        "fixtures directory exists at {} but contains no .json fixtures",
        dir.display()
    );

    let mut exact = 0usize;
    let mut rounding = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for path in &files {
        let raw = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let fixture: Value = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
        match run_fixture(&fixture) {
            Ok("exact_match") => exact += 1,
            Ok("expected_rounding") => rounding += 1,
            Ok(other) => failures.push(format!("{}: unexpected classification {other}", path.display())),
            Err(err) => failures.push(format!("{}: {err}", path.display())),
        }
    }
    eprintln!(
        "DIFFERENTIAL FIXTURES: {exact} exact, {rounding} within rounding tolerance, {} failed",
        failures.len()
    );
    assert!(
        failures.is_empty(),
        "differential mismatches:\n{}",
        failures.join("\n")
    );
}
