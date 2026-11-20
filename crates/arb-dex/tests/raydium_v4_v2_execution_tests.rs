//! Raydium AMM v4 execution-account resolution tests (Phase 5).
//!
//! Behavior contract: `SwapBaseInV2` (opcode 16, 8 accounts) needs no
//! OpenBook/Serum accounts, so a pool whose market accounts were never
//! hydrated remains executable on the V2 path with default serum keys,
//! while an explicit V1 build fails closed in arb-tx.

use arb_dex::{AmmPool, RaydiumV4Pool};
use arb_types::PoolExecutionAccounts;
use arb_types::{MintId, PoolId, RaydiumV4SwapVersion};
use solana_pubkey::Pubkey;

const LAYOUT: usize = arb_dex::raydium_v4::RAYDIUM_V4_LAYOUT_SIZE;

fn pool_account_bytes(market: Option<Pubkey>) -> Vec<u8> {
    let mut data = vec![0u8; LAYOUT];
    data[0..8].copy_from_slice(&6u64.to_le_bytes());
    data[176..184].copy_from_slice(&25u64.to_le_bytes());
    data[184..192].copy_from_slice(&10_000u64.to_le_bytes());
    data[336..368].copy_from_slice(Pubkey::new_unique().as_ref());
    data[368..400].copy_from_slice(Pubkey::new_unique().as_ref());
    data[400..432].copy_from_slice(MintId::new([1; 32]).0.as_ref());
    data[432..464].copy_from_slice(MintId::new([2; 32]).0.as_ref());
    data[496..528].copy_from_slice(Pubkey::new_unique().as_ref());
    if let Some(market) = market {
        data[528..560].copy_from_slice(market.as_ref());
        data[560..592].copy_from_slice(Pubkey::new_unique().as_ref());
    }
    data
}

fn decoded_pool(market: Option<Pubkey>) -> RaydiumV4Pool {
    let mut pool = RaydiumV4Pool::new(
        PoolId::new([5; 32]),
        MintId::new([0; 32]),
        MintId::new([0; 32]),
        Pubkey::default(),
        Pubkey::default(),
        Pubkey::default(),
    );
    pool.update_from_account(&pool_account_bytes(market))
        .expect("pool decodes");
    pool
}

#[test]
fn v2_execution_accounts_issued_without_hydrated_market() {
    let pool = decoded_pool(None);
    let accounts = pool
        .execution_accounts()
        .expect("V2 path must not require OpenBook hydration");
    match accounts {
        PoolExecutionAccounts::RaydiumV4 {
            base_vault,
            quote_vault,
            serum_market,
            serum_vault_signer,
            swap_version,
            ..
        } => {
            assert_ne!(base_vault, Pubkey::default());
            assert_ne!(quote_vault, Pubkey::default());
            assert_eq!(serum_market, Pubkey::default());
            assert_eq!(serum_vault_signer, Pubkey::default());
            assert_eq!(swap_version, RaydiumV4SwapVersion::V2);
        }
        other => panic!("expected RaydiumV4 accounts, got {other:?}"),
    }
}

#[test]
fn hydrated_market_is_injected_with_v2_default() {
    let market = Pubkey::new_unique();
    let pool = decoded_pool(Some(market));
    let accounts = pool.execution_accounts().expect("accounts resolve");
    match accounts {
        PoolExecutionAccounts::RaydiumV4 {
            serum_market,
            swap_version,
            ..
        } => {
            assert_eq!(serum_market, market);
            assert_eq!(swap_version, RaydiumV4SwapVersion::V2);
        }
        other => panic!("expected RaydiumV4 accounts, got {other:?}"),
    }
}

#[test]
fn execution_accounts_none_without_vaults() {
    let mut pool = decoded_pool(None);
    pool.base_vault = Pubkey::default();
    pool.quote_vault = Pubkey::default();
    assert!(pool.execution_accounts().is_none());
}

#[test]
fn quote_against_amm_reserves_survives_missing_market() {
    let mut pool = decoded_pool(None);
    pool.update_reserves(1_000_000, 4_000_000_000_000);
    let out = pool.quote(1_000_000, true).expect("V2 swap quotes");
    assert!(out > 0);
}
