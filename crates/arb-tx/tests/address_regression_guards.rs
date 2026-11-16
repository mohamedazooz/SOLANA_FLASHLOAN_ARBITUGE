//! Regression guards for hardcoded mainnet addresses.
//!
//! Background: `arb-types` used to carry a duplicate `RaydiumV4Accounts`
//! whose `mainnet_sol_usdc()` contained a fabricated coin-vault address
//! (`DQyrAcCrDXYZujhgwgCRiaACVKAfdA3kh2v7iqWw5jz`) that does not exist on
//! mainnet (verified 2026-11-16 via read-only `getAccountInfo`: the canonical
//! vault is a Token-program account holding ~110k SOL; the fabricated value
//! returns null). The duplicate was removed; these tests prevent the class of
//! defect from returning.

use std::str::FromStr;

/// The fabricated address that once shipped in `arb-types`. Must never appear
/// in any workspace source file again.
const FABRICATED_RAYDIUM_VAULT: &str = "DQyrAcCrDXYZujhgwgCRiaACVKAfdA3kh2v7iqWw5jz";
/// The fabricated USDC reserve value from the historical audit document.
const FABRICATED_KAMINO_RESERVE: &str = "D6q6wuQSrifJKUvxCoXepEAbW39zWCuYAZBpqUg6xK25";

/// On-chain-verified (2026-11-16, mainnet getAccountInfo) values.
const VERIFIED_RAYDIUM_SOL_VAULT: &str = "DQyrAcCrDXQ7NeoqGgDCZwBvWDcYmFCjSb9JtteuvPpz";
const VERIFIED_KAMINO_USDC_RESERVE: &str = "D6q6wuQSrifJKZYpR1M8R4YawnLDtDsMmWM1NbBmgJ59";

#[test]
fn raydium_v4_mainnet_sol_usdc_uses_verified_vault() {
    let accs = arb_tx::RaydiumV4Accounts::mainnet_sol_usdc();
    assert_eq!(
        accs.pool_coin_vault,
        solana_pubkey::Pubkey::from_str(VERIFIED_RAYDIUM_SOL_VAULT).unwrap()
    );
    assert_eq!(
        accs.pool_pc_vault,
        solana_pubkey::Pubkey::from_str("HLmqeL62xR1QoZ1HKKbXRrdN1p3phKpxRMb2VVopvBBz").unwrap()
    );
    assert_eq!(
        accs.amm_pool,
        solana_pubkey::Pubkey::from_str(arb_constants::RAYDIUM_SOL_USDC_POOL).unwrap()
    );
}

#[test]
fn arb_constants_match_onchain_verified_addresses() {
    assert_eq!(arb_constants::RAYDIUM_SOL_VAULT, VERIFIED_RAYDIUM_SOL_VAULT);
    assert_eq!(
        arb_constants::KAMINO_USDC_RESERVE,
        VERIFIED_KAMINO_USDC_RESERVE
    );
}

#[test]
fn fabricated_addresses_absent_from_all_workspace_sources() {
    let crates_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/..");
    let self_path = std::fs::canonicalize(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/address_regression_guards.rs"
    ))
    .map(|p| p.to_string_lossy().into_owned())
    .unwrap_or_default();
    let mut offenders = Vec::new();
    for entry in walkdir_like(crates_dir) {
        if entry == self_path {
            continue;
        }
        if let Ok(src) = std::fs::read_to_string(&entry) {
            if src.contains(FABRICATED_RAYDIUM_VAULT) {
                offenders.push(format!("{}: fabricated Raydium vault", entry));
            }
            if src.contains(FABRICATED_KAMINO_RESERVE) {
                offenders.push(format!("{}: fabricated Kamino reserve", entry));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "fabricated mainnet addresses re-introduced into source:\n{}",
        offenders.join("\n")
    );
}

/// Minimal recursive `.rs` file walker (avoids adding a walkdir dependency).
fn walkdir_like(root: &str) -> Vec<String> {
    let mut out = Vec::new();
    let canonical_root = std::fs::canonicalize(root).map(|p| p.to_string_lossy().into_owned());
    let mut stack = vec![canonical_root.unwrap_or_else(|_| root.to_string())];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for e in entries.flatten() {
            let path = e.path();
            if path.is_dir() {
                if path.file_name().map(|n| n == "target").unwrap_or(false)
                    || path.file_name().map(|n| n == ".git").unwrap_or(false)
                {
                    continue;
                }
                stack.push(path.to_string_lossy().into_owned());
            } else if path.extension().map(|x| x == "rs").unwrap_or(false) {
                out.push(path.to_string_lossy().into_owned());
            }
        }
    }
    out
}
