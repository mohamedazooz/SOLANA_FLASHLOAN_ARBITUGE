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
    AmmPool, orca_whirlpool::OrcaWhirlpool, pump_fun::PumpFunBondingCurve,
    pump_swap::PumpSwapPoolHeader, raydium_v4::RaydiumV4Pool,
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
