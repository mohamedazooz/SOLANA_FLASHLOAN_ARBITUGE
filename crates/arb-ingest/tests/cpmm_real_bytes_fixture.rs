//! Differential fixture: REAL mainnet Raydium CPMM SOL/USDC bytes.
//!
//! Snapshot source: `getMultipleAccounts` (base64, commitment=confirmed) for
//! pool `7JuwJuNU88gurFnyWeiyGKbFmExMWcmRZntn9imEzdny` and its AmmConfig
//! `D4FPEruKEHrG5TenZ2mpDGEfu1iUvTiqBxvpU8HLBvC2`, fetched via a paid RPC
//! provider during the Session-10 fault diagnosis. The pool identity is
//! cross-confirmed by the official Raydium API v3
//! (`/pools/info/ids?ids=7JuwJuNU88gurFnyWeiyGKbFmExMWcmRZntn9imEzdny`):
//! mintA = WSOL, mintB = USDC, tradeFeeRate 2500 (25 bps).
//!
//! These tests pin the decoders to real on-chain bytes so a layout drift
//! (wrong offsets) can never pass again — the previous fee decoder read
//! offset 290 of the POOL account (fabricated bytes) instead of offset 12
//! of the AmmConfig account.

use base64::prelude::*;
use solana_pubkey::Pubkey;
use std::str::FromStr;

/// Pool account: 7JuwJuNU88gurFnyWeiyGKbFmExMWcmRZntn9imEzdny (637 bytes).
const POOL_B64: &str = "9+3j9dfD3kazIT+6i/nIf6keR4GWKMOD4AvqfpjHoD4DuhBpz8P28w+jAIjpxrO4hg8wkpeIJkWLpb82ExPdmXlraB+ke2nsYGhngSyj0j/lZ9dFWFbcPxi4TkFhP4pIRMj8kXve4ecqhfVBKxwS12zO10bk8wUk+nNLLqZqFB2i/SowwT4QKtnFe8azvtkU9UqUud+B6Vlg6w/XFjX2D0vNroSmSj/EBpuIV/6rgYT7aH9jRhjANdrEOdwa6ztVmKDwAAAAAAHG+nrzvtutOj1l82qryXQxsbvkwtL24OR8pgIDRS9dYQbd9uHXZaGT2cvhRs7reawctIXtX1s3kTqM9YV+/wCpBt324ddloZPZy+FGzut5rBy0he1fWzeROoz1hX7/AKkx1rEPw5lFSreBcq1oR8MwsMUbXGFPMBEKVo2jP2jElf0ACQkGhn1UAgAAAAB4uA0AAAAAAG5tBAAAAAAACb0JAAAAAADE6QEAAAAAAIs2RGYAAAAAQAQAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==";
/// AmmConfig account: D4FPEruKEHrG5TenZ2mpDGEfu1iUvTiqBxvpU8HLBvC2 (236 bytes).
const AMM_CONFIG_B64: &str = "2vQhaMvLK2//AAAAxAkAAAAAAADA1AEAAAAAAECcAAAAAAAAgNHwCAAAAAAF2xU6D/SH9pzeiQAf7ExGlbbUIV6iRLh71Otyh/hXqNcGSjj20XsRPl9m3PtjuHN//OJkSr8KALmLNDVtY4WD9AEAAAAAAABQwwAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

const CPMM_AMM_CONFIG: &str = "D4FPEruKEHrG5TenZ2mpDGEfu1iUvTiqBxvpU8HLBvC2";
const CPMM_SOL_VAULT: &str = "7VLUXrnSSDo9BfCa4NWaQs68g7ddDY1sdXBKW6Xswj9Y";
const CPMM_USDC_VAULT: &str = "3rzbbW5Q8MA7sCaowf28hNgACNPecdS2zceWy7Ptzua9";

#[test]
fn real_pool_bytes_decode_to_verified_vaults() {
    let data = BASE64_STANDARD.decode(POOL_B64).expect("valid base64");
    assert_eq!(data.len(), 637);
    assert_eq!(
        &data[0..8],
        &hex("f7ede3f5d7c3de46"),
        "PoolState discriminator (sha256(\"account:PoolState\")[..8])"
    );
    let (v0, v1) = arb_ingest::decode_raydium_cpmm_vaults(&data).expect("vaults decode");
    assert_eq!(v0, Pubkey::from_str(CPMM_SOL_VAULT).unwrap());
    assert_eq!(v1, Pubkey::from_str(CPMM_USDC_VAULT).unwrap());
}

#[test]
fn real_amm_config_bytes_decode_to_verified_fee() {
    let data = BASE64_STANDARD.decode(AMM_CONFIG_B64).expect("valid base64");
    assert_eq!(data.len(), 236);
    let pool = BASE64_STANDARD.decode(POOL_B64).unwrap();
    let cfg_key = arb_ingest::raydium_cpmm_amm_config_key(&pool).expect("amm_config key");
    assert_eq!(cfg_key, Pubkey::from_str(CPMM_AMM_CONFIG).unwrap());
    assert_eq!(
        arb_ingest::decode_raydium_cpmm_fee_bps(&data),
        Some(25),
        "trade_fee_rate 2500 / 1_000_000 => 25 bps (matches Raydium API v3)"
    );
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}
