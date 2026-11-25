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
fn real_pool_accounting_decodes_accrued_fees_and_creator_fee_mode() {
    let data = BASE64_STANDARD.decode(POOL_B64).expect("valid base64");
    let state = arb_ingest::decoders::decode_raydium_cpmm_pool_accounting(&data)
        .expect("official PoolState accounting fields decode");
    assert_eq!(
        state.amm_config,
        arb_ingest::decoders::raydium_cpmm_amm_config_key(&data).unwrap()
    );
    assert_eq!(
        state.token_0_vault,
        Pubkey::from_str(CPMM_SOL_VAULT).unwrap()
    );
    assert_eq!(
        state.token_1_vault,
        Pubkey::from_str(CPMM_USDC_VAULT).unwrap()
    );
    assert!(!state.enable_creator_fee);
    assert!(state.swap_enabled);
    assert_eq!(state.protocol_fees, [899_192, 290_158]);
    assert_eq!(state.fund_fees, [638_217, 125_380]);
    assert_eq!(state.creator_fees, [0, 0]);
    assert_eq!(
        arb_ingest::decoders::raydium_cpmm_curve_reserve(10_000_000, &state, 0),
        Some(8_462_591),
        "curve reserve excludes protocol + fund + creator fee counters"
    );
    assert_eq!(
        arb_ingest::decoders::raydium_cpmm_curve_reserve(1, &state, 0),
        None
    );
    let mut creator_enabled = data.clone();
    creator_enabled[390] = 1;
    assert!(
        arb_ingest::decoders::decode_raydium_cpmm_pool_accounting(&creator_enabled)
            .unwrap()
            .enable_creator_fee
    );
    let mut swap_disabled = data.clone();
    swap_disabled[329] |= 4;
    assert!(
        !arb_ingest::decoders::decode_raydium_cpmm_pool_accounting(&swap_disabled)
            .unwrap()
            .swap_enabled
    );
    let mut bad_discriminator = data.clone();
    bad_discriminator[0] ^= 0xff;
    assert!(
        arb_ingest::decoders::decode_raydium_cpmm_pool_accounting(&bad_discriminator).is_none()
    );
}

#[test]
fn real_amm_config_bytes_decode_to_verified_fee() {
    let data = BASE64_STANDARD
        .decode(AMM_CONFIG_B64)
        .expect("valid base64");
    assert_eq!(data.len(), 236);
    let mut bad_discriminator = data.clone();
    bad_discriminator[0] ^= 0xff;
    assert!(arb_ingest::decoders::decode_raydium_cpmm_fee_config(&bad_discriminator).is_none());
    let pool = BASE64_STANDARD.decode(POOL_B64).unwrap();
    let cfg_key = arb_ingest::raydium_cpmm_amm_config_key(&pool).expect("amm_config key");
    assert_eq!(cfg_key, Pubkey::from_str(CPMM_AMM_CONFIG).unwrap());
    let exact = arb_ingest::decoders::decode_raydium_cpmm_fee_config(&data)
        .expect("full on-chain fee config");
    assert_eq!(exact.trade_fee_rate, 2_500);
    assert_eq!(exact.protocol_fee_rate, 120_000);
    assert_eq!(exact.fund_fee_rate, 40_000);
    assert_eq!(exact.creator_fee_rate, 500);
    assert_eq!(
        arb_ingest::decode_raydium_cpmm_fee_bps(&data),
        Some(25),
        "trade_fee_rate 2500 / 1_000_000 => 25 bps (matches Raydium API v3)"
    );

    // A valid 0.005% tier is only 0.5 bps. The old u16-bps-only path
    // rounded it down to zero and silently removed the trading fee; it now rounds up conservatively for graph metadata.
    let mut sub_bps = data.clone();
    sub_bps[12..20].copy_from_slice(&50u64.to_le_bytes());
    assert_eq!(
        arb_ingest::decoders::decode_raydium_cpmm_fee_config(&sub_bps)
            .expect("valid sub-bps tier")
            .trade_fee_rate,
        50
    );
    assert_eq!(arb_ingest::decode_raydium_cpmm_fee_bps(&sub_bps), Some(1));
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// Versioned, same-context Mainnet snapshot covering every account used by the
/// CPMM loader: pool, AmmConfig, both vaults and both mints. Captured with one
/// getMultipleAccounts(confirmed) response; account data is retained verbatim.
#[test]
fn versioned_mainnet_snapshot_has_coherent_slot_owners_and_vault_mints() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/raydium_cpmm_mainnet/sol_usdc_confirmed_2026-11-25.json"
    ))
    .expect("versioned mainnet snapshot JSON");
    assert_eq!(fixture["schema_version"], 1);
    assert_eq!(fixture["commitment"], "confirmed");
    assert!(fixture["context_slot"].as_u64().unwrap() > 0);

    let accounts = &fixture["accounts"];
    let read_account = |name: &str| {
        let account = &accounts[name];
        let bytes = BASE64_STANDARD
            .decode(account["data_base64"].as_str().unwrap())
            .expect("account data base64");
        assert_eq!(
            account["data_length"].as_u64().unwrap() as usize,
            bytes.len()
        );
        (account, bytes)
    };

    let (pool_meta, pool_bytes) = read_account("pool");
    assert_eq!(pool_meta["address"], fixture["pool_address"]);
    assert_eq!(pool_meta["owner"], fixture["cpmm_program_id"]);
    assert_eq!(pool_bytes.len(), 637);
    let state = arb_ingest::decoders::decode_raydium_cpmm_pool_accounting(&pool_bytes)
        .expect("mainnet PoolState decodes");

    let (config_meta, config_bytes) = read_account("amm_config");
    assert_eq!(config_meta["owner"], fixture["cpmm_program_id"]);
    assert_eq!(
        state.amm_config,
        Pubkey::from_str(config_meta["address"].as_str().unwrap()).unwrap()
    );
    assert_eq!(config_bytes.len(), 236);
    let fees = arb_ingest::decoders::decode_raydium_cpmm_fee_config(&config_bytes)
        .expect("mainnet AmmConfig decodes");
    assert_eq!(fees.trade_fee_rate, 2_500);
    assert_eq!(fees.protocol_fee_rate, 120_000);
    assert_eq!(fees.fund_fee_rate, 40_000);

    for (index, vault_name, mint_name, token_program, mint, decimals) in [
        (
            0usize,
            "vault0",
            "mint0",
            state.token_0_program,
            state.token_0_mint,
            state.mint_0_decimals,
        ),
        (
            1usize,
            "vault1",
            "mint1",
            state.token_1_program,
            state.token_1_mint,
            state.mint_1_decimals,
        ),
    ] {
        let (vault_meta, vault_bytes) = read_account(vault_name);
        assert_eq!(vault_meta["owner"], token_program.to_string());
        assert_eq!(
            Pubkey::new_from_array(vault_bytes[0..32].try_into().unwrap()),
            mint,
            "vault {index} mint matches PoolState"
        );
        assert_eq!(vault_bytes.len(), 165);
        let raw_amount = u64::from_le_bytes(vault_bytes[64..72].try_into().unwrap());
        assert!(
            arb_ingest::decoders::raydium_cpmm_curve_reserve(raw_amount, &state, index).is_some(),
            "vault {index} balance covers accrued non-LP fee counters"
        );

        let (mint_meta, mint_bytes) = read_account(mint_name);
        assert_eq!(mint_meta["owner"], token_program.to_string());
        assert_eq!(mint_bytes.len(), 82);
        assert_eq!(
            mint_bytes[44], decimals,
            "mint {index} decimals match PoolState"
        );
    }
}
