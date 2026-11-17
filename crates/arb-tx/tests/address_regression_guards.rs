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
/// Fabricated OpenBook SOL/USDC market that once shipped in
/// `arb-tx/src/primitives/builder.rs` (shares the real market's 20-char prefix;
/// returns null via getAccountInfo on mainnet).
const FABRICATED_OPENBOOK_MARKET: &str = "8BnEgHoWFysVcuFFX7QBpDmvu3xei5qGE9KitUfqWQH9";
/// Fabricated Raydium CPMM SOL/USDC pool that once shipped in `arb-constants`
/// (returns null via getAccountInfo on mainnet; measured 2026 session).
const FABRICATED_CPMM_POOL: &str = "79RbHpxoYSB3EJnN8aGLouuRrDJVGWLzxM3r1qVKsKJ7";
/// Fabricated Raydium CPMM SOL vault constant (nonexistent on mainnet).
const FABRICATED_CPMM_SOL_VAULT: &str = "DJgKsuDtTVvT1VqjC1vHTu6LHjq5Z6KRX1jaLKZJ3VwW";
/// Fabricated Raydium CPMM USDC vault constant (nonexistent on mainnet).
const FABRICATED_CPMM_USDC_VAULT: &str = "HYbZV1vHH8rDvn3a2uPtR8nCnBkL5xXwK7yP1uJ9nC2L";
/// Fabricated Raydium CPMM AmmConfig constant (nonexistent on mainnet).
const FABRICATED_CPMM_AMM_CONFIG: &str = "D4A2sqNAqqDy1bMT7BpTNNYTubbmMcETcpKEqZiiNtSZ";

/// On-chain-verified (2026-11-16, mainnet getAccountInfo) values.
const VERIFIED_RAYDIUM_SOL_VAULT: &str = "DQyrAcCrDXQ7NeoqGgDCZwBvWDcYmFCjSb9JtteuvPpz";
const VERIFIED_KAMINO_USDC_RESERVE: &str = "D6q6wuQSrifJKZYpR1M8R4YawnLDtDsMmWM1NbBmgJ59";
/// Verified via openserum.io market page and Solana StackExchange references
/// (the market bound to Raydium pool 58oQChx4yWmvKdwLLZzBi4ChoCc2fqCUWBkwMihLYQo2).
const VERIFIED_OPENBOOK_SOL_USDC_MARKET: &str = "8BnEgHoWFysVcuFFX7QztDmzuH8r5ZFvyP3sYwn1XTh6";
/// On-chain-verified (mainnet getAccountInfo + Raydium API v3
/// `/pools/info/ids?ids=...`, 2026 session): discriminator
/// sha256("account:PoolState")[..8] == f7ede3f5d7c3de46, owner == CPMM program,
/// amm_config @8 == D4FPEru..., token_0_vault @72 (WSOL), token_1_vault @104
/// (USDC), both owned by the pool-authority PDA GpMZbSM2....
const VERIFIED_CPMM_POOL: &str = "7JuwJuNU88gurFnyWeiyGKbFmExMWcmRZntn9imEzdny";
const VERIFIED_CPMM_SOL_VAULT: &str = "7VLUXrnSSDo9BfCa4NWaQs68g7ddDY1sdXBKW6Xswj9Y";
const VERIFIED_CPMM_USDC_VAULT: &str = "3rzbbW5Q8MA7sCaowf28hNgACNPecdS2zceWy7Ptzua9";
const VERIFIED_CPMM_AMM_CONFIG: &str = "D4FPEruKEHrG5TenZ2mpDGEfu1iUvTiqBxvpU8HLBvC2";
const VERIFIED_CPMM_POOL_AUTHORITY: &str = "GpMZbSM2GgvTKHJirzeGfMFoaZ8UR2X7F4v8vHTvxFbL";

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
fn raydium_cpmm_constants_use_onchain_verified_addresses() {
    assert_eq!(
        arb_constants::RAYDIUM_CPMM_SOL_USDC_POOL,
        VERIFIED_CPMM_POOL
    );
    assert_eq!(
        arb_constants::RAYDIUM_CPMM_SOL_VAULT,
        VERIFIED_CPMM_SOL_VAULT
    );
    assert_eq!(
        arb_constants::RAYDIUM_CPMM_USDC_VAULT,
        VERIFIED_CPMM_USDC_VAULT
    );
    assert_eq!(
        arb_constants::RAYDIUM_CPMM_SOL_USDC_AMM_CONFIG,
        VERIFIED_CPMM_AMM_CONFIG
    );
    assert_eq!(
        arb_constants::RAYDIUM_CPMM_SOL_USDC_POOL_AUTHORITY,
        VERIFIED_CPMM_POOL_AUTHORITY
    );
}

#[test]
fn raydium_v4_mainnet_sol_usdc_uses_verified_openbook_market() {
    let accs = arb_tx::RaydiumV4Accounts::mainnet_sol_usdc();
    assert_eq!(
        accs.serum_market,
        solana_pubkey::Pubkey::from_str(VERIFIED_OPENBOOK_SOL_USDC_MARKET).unwrap()
    );
    assert_eq!(
        accs.serum_program,
        solana_pubkey::Pubkey::from_str(arb_constants::OPENBOOK_PROGRAM_ID).unwrap()
    );
    assert_eq!(
        arb_constants::RAYDIUM_SOL_USDC_OPENBOOK_MARKET,
        VERIFIED_OPENBOOK_SOL_USDC_MARKET
    );
}

#[test]
fn strip_test_modules_masks_test_guards_but_never_production_code() {
    // 1. A `#[cfg(test)]` block containing a fabricated address is masked.
    let with_guard = "const KEEP: &str = \"real\";\n#[cfg(test)]\nmod t {\n    // DqyrAcCrDXYZujhgwgCRiaACVKAfdA3kh2v7iqWw5jz\n    #[test]\n    fn x() { assert!(true); }\n}\n";
    let stripped = strip_test_modules(with_guard);
    assert!(
        !stripped.contains("DQyrAcCrDXYZujhgwgCRiaACVKAfdA3kh2v7iqWw5jz"),
        "test-module literal must be masked"
    );
    assert!(
        stripped.contains("const KEEP: &str = \"real\";"),
        "production code outside the test module must survive"
    );

    // 2. The same address in PRODUCTION code must NOT be masked — the stripper
    //    cannot become a way to hide a real reintroduction.
    let in_prod = "const TYPO: &str = \"DQyrAcCrDXYZujhgwgCRiaACVKAfdA3kh2v7iqWw5jz\";\n";
    assert!(
        strip_test_modules(in_prod).contains("DQyrAcCrDXYZujhgwgCRiaACVKAfdA3kh2v7iqWw5jz"),
        "production hits must survive stripping"
    );

    // 3. `mod name;` without a body is a declaration, not a test module.
    let decl = "#[cfg(test)]\nmod external_tests;\nconst X: u8 = 1;\n";
    let stripped = strip_test_modules(decl);
    assert!(
        stripped.contains("const X: u8 = 1;"),
        "a bodyless mod declaration must not mask following items"
    );

    // 4. Idempotent on already-clean input.
    let clean = "fn main() {}\n";
    assert_eq!(strip_test_modules(clean), clean);
}

#[test]
fn fabricated_openbook_market_absent_from_all_workspace_sources() {
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
            let src = strip_test_modules(&src);
            if src.contains(FABRICATED_OPENBOOK_MARKET) {
                offenders.push(format!("{}: fabricated OpenBook market", entry));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "fabricated OpenBook market re-introduced into source:\n{}",
        offenders.join("\n")
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
            let src = strip_test_modules(&src);
            if src.contains(FABRICATED_RAYDIUM_VAULT) {
                offenders.push(format!("{}: fabricated Raydium vault", entry));
            }
            if src.contains(FABRICATED_KAMINO_RESERVE) {
                offenders.push(format!("{}: fabricated Kamino reserve", entry));
            }
            if src.contains(FABRICATED_CPMM_POOL) {
                offenders.push(format!("{}: fabricated Raydium CPMM pool", entry));
            }
            if src.contains(FABRICATED_CPMM_SOL_VAULT) {
                offenders.push(format!("{}: fabricated Raydium CPMM SOL vault", entry));
            }
            if src.contains(FABRICATED_CPMM_USDC_VAULT) {
                offenders.push(format!("{}: fabricated Raydium CPMM USDC vault", entry));
            }
            if src.contains(FABRICATED_CPMM_AMM_CONFIG) {
                offenders.push(format!("{}: fabricated Raydium CPMM AmmConfig", entry));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "fabricated mainnet addresses re-introduced into source:\n{}",
        offenders.join("\n")
    );
}

/// Strip `#[cfg(test)] mod ... { ... }` blocks from a Rust source file so the
/// fabricated-address scan inspects production code only.
///
/// A regression test that asserts `pool_coin_vault != TYPO` necessarily contains
/// the typo as a string literal. Scanning test modules as production code
/// therefore reports every such guard as a reintroduction — the guard and the
/// thing it guards against become indistinguishable. Masking those bytes makes
/// the scan mean what it claims: "no fabricated address in shipped code".
///
/// The stripper is brace-depth aware and handles the `mod name;` (no body) form.
/// It is deliberately conservative: on unbalanced input it returns the source
/// unchanged, so a parsing surprise can never hide a real production hit.
fn strip_test_modules(src: &str) -> String {
    const MARKER: &str = "#[cfg(test)]";
    let mut out = String::with_capacity(src.len());
    let mut search_from = 0usize;
    let mut masked_until = 0usize;

    while let Some(rel) = src[search_from..].find(MARKER) {
        let marker_at = search_from + rel;
        search_from = marker_at + MARKER.len();
        if marker_at < masked_until {
            continue;
        }
        // Only strip when the marker is followed by a `mod` item with a body.
        let Some(rest) = src.get(search_from..) else {
            continue;
        };
        let Some(mod_at) = rest.find("mod ") else {
            continue;
        };
        // Reject anything that is not whitespace between the attribute and `mod`
        // (e.g. a doc comment or a second attribute): those are not test modules.
        if !rest[..mod_at].trim().is_empty() {
            continue;
        }
        let body_open = search_from + mod_at + 4;
        // `mod name;` (declaration without body) has no block to strip.
        let Some(semi_or_brace) = src[body_open..].find(['{', ';']) else {
            continue;
        };
        let brace_at = body_open + semi_or_brace;
        if src.as_bytes().get(brace_at) != Some(&b'{') {
            continue;
        }
        // Walk to the matching closing brace, tracking depth from the opener.
        let mut depth = 0usize;
        let mut end = None;
        for (i, b) in src.as_bytes()[brace_at..].iter().enumerate() {
            match b {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(brace_at + i + 1);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else { continue };
        out.push_str(&src[masked_until..marker_at]);
        masked_until = end;
        search_from = end;
    }
    out.push_str(&src[masked_until..]);
    out
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
