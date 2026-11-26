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

#[test]
fn decodes_market_cap_exponential_and_dynamic_fee_snapshot_fields() {
    let mut data = pool_bytes(0);
    data[8..16].copy_from_slice(&100_000_000u64.to_le_bytes());
    data[16] = 4;
    data[22..24].copy_from_slice(&3u16.to_le_bytes());
    data[24..28].copy_from_slice(&250u32.to_le_bytes());
    data[28..32].copy_from_slice(&3_600u32.to_le_bytes());
    data[32..40].copy_from_slice(&1_000u64.to_le_bytes());
    data[48] = 20;
    data[50] = 25;
    data[54..56].copy_from_slice(&5_000u16.to_le_bytes());
    data[64..68].copy_from_slice(&1_000u32.to_le_bytes());
    data[56] = 1;
    data[68..72].copy_from_slice(&100_000u32.to_le_bytes());
    data[72..74].copy_from_slice(&10u16.to_le_bytes());
    data[120..136].copy_from_slice(&100u128.to_le_bytes());
    data[152..168].copy_from_slice(&(1u128 << 64).to_le_bytes());
    data[480] = 1;
    data[486] = 1;
    data[696] = 1;

    let mut pool = pool_with(&data);
    assert_eq!(pool.activation_type, 1);
    assert_eq!(pool.fee_version, 1);
    assert_eq!(pool.layout_version, 1);
    assert_eq!(pool.fee_params.fee_scheduler_mode, 4);
    assert_eq!(pool.fee_params.sqrt_price_step_bps, 250);
    assert_eq!(pool.fee_params.scheduler_expiration_duration, 3_600);
    assert_eq!(pool.fee_params.reduction_factor, 1_000);
    assert!(pool.fee_params.dynamic_fee_initialized);
    assert_eq!(pool.fee_params.variable_fee_control, 100_000);
    assert_eq!(pool.fee_params.max_volatility_accumulator, 1_000);
    assert_eq!(pool.fee_params.bin_step, 10);
    assert_eq!(pool.fee_params.volatility_accumulator, 100);
    assert_eq!(pool.fee_params.init_sqrt_price_x64, 1u128 << 64);
    assert!(
        pool.fee_bps().is_none(),
        "clock context must be explicitly joined"
    );

    pool.set_current_point(1, 10);
    assert!(pool.fee_context_verified);
    assert_eq!(pool.fee_bps(), Some(1_001));
    pool.set_current_point(0, 11);
    assert!(
        !pool.fee_context_verified,
        "clock type mismatch must invalidate the fee snapshot"
    );
    assert!(pool.fee_bps().is_none());
}

#[test]
fn unknown_fee_mode_never_becomes_a_verified_fee_snapshot() {
    let mut data = pool_bytes(0);
    data[8..16].copy_from_slice(&100_000_000u64.to_le_bytes());
    data[16] = 9;
    data[486] = 1;
    let mut pool = pool_with(&data);
    pool.set_current_point(0, 10);
    assert!(!pool.fee_context_verified);
    assert!(pool.fee_bps().is_none());
}
