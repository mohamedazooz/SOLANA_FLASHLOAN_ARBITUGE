//! Fail-closed account and quote guards for the Meteora DAMM v2 adapter.
//!
//! The official pool layout is 1,104 bytes. This adapter currently decodes only
//! the leading fields and does not yet join the complete fee/config/mint context.
//! In particular Pool::whitelisted_vault at offset 296 is not a swap config account.

use arb_dex::meteora_damm_v2::{METEORA_DAMM_V2_POOL_MIN_SIZE, MeteoraDammV2Pool};
use arb_dex::traits::AmmPool;
use arb_types::{ArbError, DexProtocol, PoolId};
use solana_pubkey::Pubkey;

fn pool_bytes(status: u8) -> Vec<u8> {
    let mut data = vec![0u8; METEORA_DAMM_V2_POOL_MIN_SIZE];
    data[360..376].copy_from_slice(&1_000_000u128.to_le_bytes());
    data[456..472].copy_from_slice(&(1u128 << 64).to_le_bytes());
    data[481] = status;
    data
}

fn pool_with(data: &[u8]) -> MeteoraDammV2Pool {
    MeteoraDammV2Pool::from_account(PoolId(Pubkey::new_unique()), data)
        .expect("well-formed prefix must decode")
}

#[test]
fn pool_status_is_decoded_from_byte_481() {
    for status in [0u8, 1u8, 2u8, 255u8] {
        let pool = pool_with(&pool_bytes(status));
        assert_eq!(pool.pool_status, status);
        assert_eq!(pool.is_tradable(), status == 0);
    }
}

#[test]
fn pool_account_offset_296_is_whitelisted_vault_not_config() {
    let mut data = pool_bytes(0);
    let expected = Pubkey::new_unique();
    data[296..328].copy_from_slice(expected.as_ref());
    let pool = pool_with(&data);
    assert_eq!(pool.whitelisted_vault, expected);
    assert!(pool.execution_accounts().is_none());
}

#[test]
fn quote_never_promotes_partial_pool_bytes_to_a_valid_quote() {
    let pool = pool_with(&pool_bytes(0));
    assert!(pool.is_tradable());
    assert!(pool.fee_bps().is_none());
    assert!(matches!(
        pool.quote(1_000, true),
        Err(ArbError::UnsupportedOperation(_))
    ));
}

#[test]
fn status_values_remain_observable_but_do_not_enable_quote_fallback() {
    for status in [1u8, 2u8, 255u8] {
        let pool = pool_with(&pool_bytes(status));
        assert!(!pool.is_tradable());
        assert!(matches!(
            pool.quote(1_000, false),
            Err(ArbError::UnsupportedOperation(_))
        ));
    }
}

#[test]
fn from_account_rejects_truncated_pool_bytes() {
    let err = MeteoraDammV2Pool::from_account(PoolId(Pubkey::new_unique()), &[])
        .expect_err("empty account data must fail closed");
    assert!(matches!(err, ArbError::DeserializationError(_)));
}

#[test]
fn protocol_identity_remains_damm_v2() {
    let pool = pool_with(&pool_bytes(0));
    assert_eq!(pool.protocol(), DexProtocol::MeteoraDammV2);
}
