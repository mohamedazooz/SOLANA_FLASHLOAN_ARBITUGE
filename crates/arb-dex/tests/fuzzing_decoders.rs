//! Fuzzing Suite for DEX Account Deserializers.
//!
//! Feeds completely random and adversarial byte sequences into:
//! 1. Raydium AMM v4 Account Deserializer
//! 2. Orca Whirlpool Account Deserializer
//! 3. Pump.fun Bonding Curve Deserializer
//! 4. PumpSwap Pool Header Deserializer
//!
//! Core Security Invariant:
//! Under NO input of any length (empty, truncated, oversized, corrupted, malicious)
//! may any deserializer ever panic. It must either succeed with valid state or return
//! a clean, fail-closed `Err(...)`.

use arb_dex::{
    AmmPool, meteora_damm_v2::MeteoraDammV2Pool, orca_whirlpool::OrcaWhirlpool,
    pump_fun::PumpFunBondingCurve, pump_swap::PumpSwapPoolHeader, raydium_clmm::RaydiumClmmPool,
    raydium_v4::RaydiumV4Pool,
};
use arb_types::{MintId, PoolId};
use proptest::collection::vec;
use proptest::prelude::*;
use solana_pubkey::Pubkey;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    #[test]
    fn fuzz_raydium_v4_decoder_never_panics(data in vec(any::<u8>(), 0..=1200)) {
        let mut pool = RaydiumV4Pool::new(
            PoolId(Pubkey::new_unique()),
            MintId(Pubkey::new_unique()),
            MintId(Pubkey::new_unique()),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
        );

        // Must never panic!
        let res = pool.update_from_account(&data);
        if data.len() < arb_dex::raydium_v4::RAYDIUM_V4_LAYOUT_SIZE {
            prop_assert!(res.is_err(), "Truncated data must return Err");
        }
    }

    #[test]
    fn fuzz_orca_whirlpool_decoder_never_panics(data in vec(any::<u8>(), 0..=1200)) {
        let mut pool = OrcaWhirlpool::new(
            PoolId(Pubkey::new_unique()),
            MintId(Pubkey::new_unique()),
            MintId(Pubkey::new_unique()),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            64,
            25,
        );

        let res = pool.update_from_account(&data);
        if data.len() < arb_dex::orca_whirlpool::WHIRLPOOL_MIN_SIZE {
            prop_assert!(res.is_err(), "Truncated data must return Err");
        }
    }

    #[test]
    fn fuzz_pump_fun_decoder_never_panics(data in vec(any::<u8>(), 0..=600)) {
        let res = PumpFunBondingCurve::decode_account_data(&data);
        if data.len() < arb_dex::pump_fun::PUMP_FUN_BONDING_CURVE_SIZE {
            prop_assert!(res.is_err(), "Truncated data must return Err");
        }
    }

    #[test]
    fn fuzz_pump_swap_header_decoder_never_panics(data in vec(any::<u8>(), 0..=600)) {
        let res = PumpSwapPoolHeader::decode(&data);
        if data.len() < 245 {
            prop_assert!(res.is_err(), "Truncated data must return Err");
        }
    }

    #[test]
    fn fuzz_pump_swap_decoder_with_real_discriminator(
        mut body in vec(any::<u8>(), 0..=300)
    ) {
        let mut data = Vec::with_capacity(8 + body.len());
        data.extend_from_slice(&arb_dex::pump_swap::PUMP_SWAP_POOL_DISCRIMINATOR);
        data.append(&mut body);

        // Feeding correct discriminator with arbitrary body must never panic
        let _ = PumpSwapPoolHeader::decode(&data);
    }
}

#[test]
fn orca_whirlpool_rejects_sqrt_price_outside_official_bounds() {
    let mut pool = OrcaWhirlpool::new(
        PoolId(Pubkey::new_unique()),
        MintId(Pubkey::new_unique()),
        MintId(Pubkey::new_unique()),
        Pubkey::new_unique(),
        Pubkey::new_unique(),
        64,
        25,
    );
    let mut data = vec![0u8; arb_dex::orca_whirlpool::WHIRLPOOL_MIN_SIZE];
    data[49..65].copy_from_slice(&1u128.to_le_bytes());
    let orca_max = arb_constants::ORCA_WHIRLPOOL_MAX_SQRT_PRICE_X64;
    data[65..81].copy_from_slice(&(orca_max + 1).to_le_bytes());
    assert!(pool.update_from_account(&data).is_err());
    data[65..81].copy_from_slice(&orca_max.to_le_bytes());
    assert!(pool.update_from_account(&data).is_ok());
}

#[test]
fn raydium_clmm_rejects_sqrt_price_outside_official_bounds() {
    let mut data = vec![0u8; arb_dex::raydium_clmm::RAYDIUM_CLMM_POOL_MIN_SIZE];
    data[237..253].copy_from_slice(&1u128.to_le_bytes());
    let raydium_max = arb_constants::RAYDIUM_CLMM_MAX_SQRT_PRICE_X64;
    data[253..269].copy_from_slice(&(raydium_max + 1).to_le_bytes());
    assert!(RaydiumClmmPool::from_account(PoolId(Pubkey::new_unique()), &data).is_err());
    data[253..269].copy_from_slice(&raydium_max.to_le_bytes());
    assert!(RaydiumClmmPool::from_account(PoolId(Pubkey::new_unique()), &data).is_ok());
}

#[test]
fn meteora_damm_v2_rejects_sqrt_price_outside_official_bounds() {
    let mut data = vec![0u8; arb_dex::meteora_damm_v2::METEORA_DAMM_V2_POOL_MIN_SIZE];
    data[360..376].copy_from_slice(&1u128.to_le_bytes());
    let damm_max = arb_constants::METEORA_DAMM_V2_MAX_SQRT_PRICE_X64;
    data[456..472].copy_from_slice(&(damm_max + 1).to_le_bytes());
    assert!(MeteoraDammV2Pool::from_account(PoolId(Pubkey::new_unique()), &data).is_err());
    data[456..472].copy_from_slice(&damm_max.to_le_bytes());
    assert!(MeteoraDammV2Pool::from_account(PoolId(Pubkey::new_unique()), &data).is_ok());
}
