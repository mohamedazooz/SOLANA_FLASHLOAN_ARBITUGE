//! Regression tests for the V1 loaded-accounts data size limit (G15).
//!
//! Live mainnet evidence (Agave 4.4.1, 2026-11-17): executable program data
//! counts toward the loaded-accounts data limit; a minimal Kamino flash
//! borrow/repay spine loads 10,714,291 bytes. Limits of 32 KiB, 256 KiB,
//! 1 MiB and 4 MiB were all rejected pre-execution with
//! `MaxLoadedAccountsDataSizeExceeded` at units=0, while a 64 MiB limit
//! simulated successfully (err=None).
//!
//! These tests pin the wire-level behavior: whatever limit the production
//! builders place in `V1ConfigParams` must reach the signed wire verbatim,
//! and the shared flash-route default must be the runtime maximum.

use arb_tx::{
    KeypairSigner,
    adaptive::{
        AdaptiveTxCompiler, FormatSelectionReason, V1_MAX_LOADED_ACCOUNTS_DATA_SIZE_LIMIT,
        V1ConfigParams,
    },
    flash::MeteoraTwoPoolV1Config,
    transport::adaptive::{
        CONFIG_MASK_CU_LIMIT, CONFIG_MASK_LOADED_DATA_LIMIT, CONFIG_MASK_PRIORITY_FEE,
    },
};
use solana_pubkey::Pubkey;

const RUNTIME_MAX_LOADED_DATA_LIMIT: u32 = 67_108_864;

/// Extract `(config_mask, config_value_bytes)` from a compiled V1 message.
///
/// Wire layout (SIMD-0385): `[0]` version, `[1..4]` legacy header,
/// `[4..8]` config mask (u32 LE), `[8..40]` lifetime, `[40]` num instructions,
/// `[41]` num addresses, then addresses, then config values in bit order.
fn parse_v1_config(message: &[u8]) -> (u32, &[u8]) {
    assert_eq!(message[0], 0x81, "V1 version discriminator");
    let mask = u32::from_le_bytes(message[4..8].try_into().expect("config mask"));
    let num_addresses = message[41] as usize;
    let values_start = 42 + num_addresses * 32;
    assert!(
        values_start <= message.len(),
        "addresses must fit inside the message"
    );
    (mask, &message[values_start..])
}

/// Decode the loaded-accounts data size limit from the config value bytes.
fn decode_loaded_data_limit(mask: u32, values: &[u8]) -> u32 {
    let mut offset = 0usize;
    if mask & CONFIG_MASK_PRIORITY_FEE == CONFIG_MASK_PRIORITY_FEE {
        offset += 8;
    }
    assert_ne!(
        mask & CONFIG_MASK_CU_LIMIT,
        0,
        "test config sets a CU limit"
    );
    offset += 4;
    assert_ne!(
        mask & CONFIG_MASK_LOADED_DATA_LIMIT,
        0,
        "test config sets a loaded-data limit"
    );
    u32::from_le_bytes(
        values[offset..offset + 4]
            .try_into()
            .expect("loaded-data value"),
    )
}

fn flash_spine_instructions(payer: Pubkey) -> Vec<arb_tx::CompiledInstruction> {
    let other = Pubkey::new_unique();
    vec![arb_tx::CompiledInstruction {
        program_id: Pubkey::new_unique(),
        accounts: vec![
            arb_tx::AccountMeta {
                pubkey: payer,
                is_signer: true,
                is_writable: true,
            },
            arb_tx::AccountMeta {
                pubkey: other,
                is_signer: false,
                is_writable: true,
            },
        ],
        data: vec![1, 2, 3],
    }]
}

#[test]
fn constant_matches_runtime_maximum() {
    assert_eq!(
        V1_MAX_LOADED_ACCOUNTS_DATA_SIZE_LIMIT, RUNTIME_MAX_LOADED_DATA_LIMIT,
        "the shared flash-route limit must equal the 64 MiB runtime maximum"
    );
}

#[test]
fn signed_v1_wire_carries_the_loaded_data_limit_verbatim() {
    let signer = KeypairSigner::from_seed(&[9u8; 32]);
    let ixs = flash_spine_instructions(signer.pubkey());
    let config = V1ConfigParams {
        priority_fee_lamports: Some(100_000),
        compute_unit_limit: Some(1_200_000),
        loaded_accounts_data_size_limit: Some(V1_MAX_LOADED_ACCOUNTS_DATA_SIZE_LIMIT),
        heap_size: None,
    };

    let tx = AdaptiveTxCompiler::build_signed_v1_transaction(
        &signer,
        &ixs,
        [7u8; 32],
        config,
        FormatSelectionReason::ExplicitRequested(arb_tx::TransactionVersion::V1),
    )
    .expect("flash-shaped V1 transaction must compile and sign");

    let (mask, values) = parse_v1_config(&tx.message_bytes);
    assert_eq!(
        mask,
        CONFIG_MASK_PRIORITY_FEE | CONFIG_MASK_CU_LIMIT | CONFIG_MASK_LOADED_DATA_LIMIT
    );
    assert_eq!(
        decode_loaded_data_limit(mask, values),
        V1_MAX_LOADED_ACCOUNTS_DATA_SIZE_LIMIT,
        "the signed wire must carry the runtime-maximum loaded-data limit, \
         otherwise mainnet rejects the transaction pre-execution"
    );
}

#[test]
fn undersized_wire_limit_is_reproduced_not_silently_grown() {
    let signer = KeypairSigner::from_seed(&[10u8; 32]);
    let ixs = flash_spine_instructions(signer.pubkey());
    let config = V1ConfigParams {
        priority_fee_lamports: Some(100_000),
        compute_unit_limit: Some(1_200_000),
        loaded_accounts_data_size_limit: Some(256 * 1024),
        heap_size: None,
    };

    let tx = AdaptiveTxCompiler::build_signed_v1_transaction(
        &signer,
        &ixs,
        [7u8; 32],
        config,
        FormatSelectionReason::ExplicitRequested(arb_tx::TransactionVersion::V1),
    )
    .expect("256 KiB limit is within the compiler's accepted range");

    let (mask, values) = parse_v1_config(&tx.message_bytes);
    assert_eq!(decode_loaded_data_limit(mask, values), 256 * 1024);
}

#[test]
fn meteora_two_pool_default_requests_the_runtime_maximum() {
    let config = MeteoraTwoPoolV1Config::default();
    let params = config
        .to_v1_config_params()
        .expect("default two-pool config must validate");

    assert_eq!(
        params.loaded_accounts_data_size_limit,
        Some(V1_MAX_LOADED_ACCOUNTS_DATA_SIZE_LIMIT),
        "8 MiB under-fits the live-measured ~10.2 MiB flash spine; \
         the default must be the runtime maximum"
    );
}

#[test]
fn build_signed_v1_rejects_limits_above_the_runtime_maximum() {
    let signer = KeypairSigner::from_seed(&[11u8; 32]);
    let ixs = flash_spine_instructions(signer.pubkey());
    let config = V1ConfigParams {
        priority_fee_lamports: Some(100_000),
        compute_unit_limit: Some(1_200_000),
        loaded_accounts_data_size_limit: Some(RUNTIME_MAX_LOADED_DATA_LIMIT + 1),
        heap_size: None,
    };

    let err = AdaptiveTxCompiler::build_signed_v1_transaction(
        &signer,
        &ixs,
        [7u8; 32],
        config,
        FormatSelectionReason::ExplicitRequested(arb_tx::TransactionVersion::V1),
    )
    .expect_err("a limit above the runtime maximum must fail closed");

    assert!(err.to_string().contains("loaded-accounts data limit"));
}
