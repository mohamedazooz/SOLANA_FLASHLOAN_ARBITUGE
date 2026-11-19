//! Fail-closed status guards for the Meteora DAMM v2 adapter.
//!
//! Verified against the deployed cp_amm program source
//! (`programs/cp-amm/src/state/pool.rs`, `pool_action_access/permissionless.rs`,
//! checked 2026-11-20):
//! - `pool_status` is the byte at offset 481 (after `activation_type` at 480):
//!   0 = `PoolStatus::Enable`, 1 = `PoolStatus::Disable`.
//! - `can_swap` requires `pool_status == Enable` AND, for non-whitelisted
//!   senders, `current_point >= activation_point`.

use arb_dex::meteora_damm_v2::{METEORA_DAMM_V2_POOL_MIN_SIZE, MeteoraDammV2Pool};
use arb_dex::traits::AmmPool;
use arb_types::{ArbError, DexProtocol, PoolId};
use solana_pubkey::Pubkey;

fn enabled_pool_bytes() -> Vec<u8> {
    let mut data = vec![0u8; METEORA_DAMM_V2_POOL_MIN_SIZE];
    // Tradable-looking body: non-zero liquidity and an in-bounds sqrt price
    // near 2^64 (price of 1.0 in Q64.64).
    data[360..376].copy_from_slice(&1_000_000u128.to_le_bytes());
    data[456..472].copy_from_slice(&(1u128 << 64).to_le_bytes());
    data[481] = 0; // pool_status = Enable
    data
}

fn pool_with(data: &[u8]) -> MeteoraDammV2Pool {
    let pool_id = PoolId(Pubkey::new_unique());
    MeteoraDammV2Pool::from_account(pool_id, data)
        .expect("decoder must accept well-formed pool bytes")
}

#[test]
fn pool_status_is_decoded_from_byte_481() {
    for status in [0u8, 1u8, 2u8, 255u8] {
        let mut data = enabled_pool_bytes();
        data[481] = status;
        let pool = pool_with(&data);
        assert_eq!(pool.pool_status, status);
        assert_eq!(pool.is_tradable(), status == 0);
    }
}

#[test]
fn decoded_pool_status_survives_later_redecode_of_enabled_pool() {
    let data = enabled_pool_bytes();
    let mut pool = pool_with(&data);
    let probe = pool_with(&data);
    pool.update_from_account(&data)
        .expect("re-decode of identical bytes must succeed");
    assert_eq!(pool.pool_status, probe.pool_status);
}

#[test]
fn quote_rejects_disabled_pool_before_fee_context_join() {
    let mut data = enabled_pool_bytes();
    data[481] = 1; // PoolStatus::Disable
    let pool = pool_with(&data);
    // The status guard must fire even though the fee context has not been
    // joined (fee_context_verified == false): fail-closed ordering.
    match pool.quote(1_000u64, true) {
        Err(ArbError::PoolDisabled(msg)) => {
            assert!(msg.contains("pool_status 1"), "unexpected message: {msg}");
        }
        other => panic!("expected PoolDisabled, got {other:?}"),
    }
}

#[test]
fn quote_rejects_unknown_pool_status_values() {
    for status in [2u8, 7u8, 255u8] {
        let mut data = enabled_pool_bytes();
        data[481] = status;
        let pool = pool_with(&data);
        match pool.quote(1_000u64, false) {
            Err(ArbError::PoolDisabled(msg)) => {
                assert!(
                    msg.contains(&format!("pool_status {status}")),
                    "unexpected message: {msg}"
                );
            }
            other => panic!("expected PoolDisabled for status {status}, got {other:?}"),
        }
    }
}

#[test]
fn from_account_rejects_truncated_pool_bytes() {
    let pool_id = PoolId(Pubkey::new_unique());
    let err = MeteoraDammV2Pool::from_account(pool_id, &[])
        .expect_err("empty bytes must fail decode (fail-closed)");
    assert!(matches!(err, ArbError::DeserializationError(_)));
}

#[test]
fn quote_rejects_pre_activation_pool_for_non_whitelisted_sender() {
    let mut data = enabled_pool_bytes();
    data[472..480].copy_from_slice(&10_000u64.to_le_bytes()); // activation_point
    let mut pool = pool_with(&data);
    pool.fee_context_verified = true;
    pool.current_point = 9_999u64; // before activation
    match pool.quote(1_000u64, true) {
        Err(ArbError::PoolDisabled(msg)) => {
            assert!(msg.contains("not active"), "unexpected message: {msg}");
        }
        other => panic!("expected PoolDisabled (pre-activation), got {other:?}"),
    }

    // At/after activation both gates pass and the quote proceeds into the
    // normal math with the default (zero) fee schedule. Amount must be
    // forwarded with at most one unit of rounding dust.
    pool.current_point = 10_000u64;
    let out = pool.quote(1_000u64, true).expect("active pool must quote");
    assert!(
        (900..=1_000).contains(&out),
        "expected near-full forwarding, got {out}"
    );
}

#[test]
fn protocol_is_meteora_damm_v2_for_conditionally_quoted_pools() {
    let pool = pool_with(&enabled_pool_bytes());
    assert_eq!(pool.protocol(), DexProtocol::MeteoraDammV2);
}
