//! Differential, integration, and golden vector tests for Solana Adaptive Transaction Compiler.
//!
//! Validates:
//! 1. Legacy wire layout (max 1232 bytes, compact-u16 signature at head).
//! 2. V0 wire layout with Address Lookup Tables (0x80 version discriminator, max 1232 bytes).
//! 3. V1 wire layout (SIMD-0160, SIMD-0296, SIMD-0385): 0x81 discriminator, inline configs,
//!    trailing signatures, support for > 1232 up to 4096 bytes MTU.
//! 4. Cryptographic Ed25519 signature validity across all formats.
//! 5. Fail-closed invariants on MTU breach and sanitization violations.

use arb_constants::SYSTEM_PROGRAM_ID;
use arb_tx::{
    AccountMeta, AdaptiveTxCompiler, AddressLookupTable, CONFIG_MASK_CU_LIMIT,
    CONFIG_MASK_HEAP_SIZE, CONFIG_MASK_LOADED_DATA_LIMIT, CONFIG_MASK_PRIORITY_FEE,
    CompiledAdaptiveTransaction, CompiledInstruction, CompilerCapabilities, FormatSelectionReason,
    KeypairSigner, SOLANA_PACKET_LEGACY_MTU, SOLANA_PACKET_V1_MTU, TransactionVersion, TxError,
    V0_VERSION_DISCRIMINATOR, V1_VERSION_DISCRIMINATOR, V1ConfigParams,
    extract_adaptive_transaction_signature,
};
use solana_pubkey::Pubkey;

fn test_signer() -> KeypairSigner {
    KeypairSigner::new_random()
}

/// A v1 resource configuration that can actually be signed.
///
/// v1 has no implicit defaults: a signed transaction with a zero compute-unit limit or a
/// zero loaded-accounts data limit is guaranteed to fail at runtime, so the compiler refuses
/// it. Tests that only care about the wire format still have to supply these two fields.
fn valid_v1_config() -> V1ConfigParams {
    V1ConfigParams {
        priority_fee_lamports: Some(0),
        compute_unit_limit: Some(300_000),
        loaded_accounts_data_size_limit: Some(67_108_864),
        heap_size: None,
    }
}

fn sample_system_instruction(from: Pubkey, to: Pubkey, lamports: u64) -> CompiledInstruction {
    let mut data = Vec::with_capacity(12);
    data.extend_from_slice(&2u32.to_le_bytes()); // SystemInstruction::Transfer = 2
    data.extend_from_slice(&lamports.to_le_bytes());

    CompiledInstruction {
        program_id: Pubkey::new_from_array(SYSTEM_PROGRAM_ID),
        accounts: vec![AccountMeta::new(from, true), AccountMeta::new(to, false)],
        data,
    }
}

#[test]
fn test_v1_strips_compute_budget_and_requires_resource_limits() {
    let signer = test_signer();
    let blockhash = [11u8; 32];
    let swap = sample_system_instruction(signer.pubkey(), Pubkey::new_unique(), 1);
    let compute_budget = arb_tx::build_compute_unit_limit(300_000);
    let config = V1ConfigParams {
        priority_fee_lamports: Some(0),
        compute_unit_limit: Some(300_000),
        loaded_accounts_data_size_limit: Some(67_108_864),
        heap_size: None,
    };
    let v1 = AdaptiveTxCompiler::build_signed_v1_transaction(
        &signer,
        &[compute_budget, swap.clone()],
        blockhash,
        config,
        FormatSelectionReason::ExplicitRequested(TransactionVersion::V1),
    )
    .expect("valid V1 transaction");
    assert_eq!(
        v1.message_bytes[40], 1,
        "ComputeBudget ix must not consume a V1 instruction slot"
    );

    let invalid = V1ConfigParams {
        priority_fee_lamports: Some(0),
        compute_unit_limit: Some(300_000),
        loaded_accounts_data_size_limit: None,
        heap_size: None,
    };
    assert!(
        AdaptiveTxCompiler::build_signed_v1_transaction(
            &signer,
            &[swap],
            blockhash,
            invalid,
            FormatSelectionReason::ExplicitRequested(TransactionVersion::V1),
        )
        .is_err()
    );
}

#[test]
fn test_v1_wire_layout_and_discriminator() {
    let signer = test_signer();
    let recipient = Pubkey::new_unique();
    let blockhash = [7u8; 32];

    let ix = sample_system_instruction(signer.pubkey(), recipient, 1_000_000);
    let config = V1ConfigParams {
        priority_fee_lamports: Some(50_000),
        compute_unit_limit: Some(300_000),
        loaded_accounts_data_size_limit: Some(67_108_864),
        heap_size: None,
    };

    let tx = AdaptiveTxCompiler::build_signed_v1_transaction(
        &signer,
        &[ix],
        blockhash,
        config,
        FormatSelectionReason::ExplicitRequested(TransactionVersion::V1),
    )
    .expect("compile v1 transaction");

    // 1. Version Discriminator byte at offset 0
    assert_eq!(tx.wire_bytes[0], V1_VERSION_DISCRIMINATOR);
    assert_eq!(tx.version, TransactionVersion::V1);
    assert_eq!(tx.max_supported_transaction_version(), Some(1));

    // 2. Trailing signature: exactly 64 bytes at the tail
    let sig_start = tx.wire_bytes.len() - 64;
    assert_eq!(&tx.wire_bytes[sig_start..], &tx.signatures[0]);

    // 3. Message bytes should match wire_bytes[0..sig_start]
    assert_eq!(&tx.wire_bytes[..sig_start], &tx.message_bytes[..]);

    // 4. Cryptographic Ed25519 signature verification against message
    let verified = tx
        .verify_signatures(&[signer.pubkey()])
        .expect("verify signature");
    assert!(verified);

    // 5. Signature extraction matches
    let extracted_sig =
        extract_adaptive_transaction_signature(&tx.wire_bytes).expect("extract signature");
    assert_eq!(extracted_sig, tx.signature().unwrap());

    // 6. Wire inspection parses V1
    let parsed_ver = CompiledAdaptiveTransaction::parse_version_from_wire(&tx.wire_bytes)
        .expect("parse version");
    assert_eq!(parsed_ver, TransactionVersion::V1);

    // 7. Base64 export is valid
    let b64 = tx.to_base64();
    assert!(!b64.is_empty());
}

#[test]
fn test_v1_resource_config_bitmask_and_values() {
    let signer = test_signer();
    let blockhash = [9u8; 32];
    let ix = sample_system_instruction(signer.pubkey(), Pubkey::new_unique(), 500);

    let priority_fee = 125_000u64;
    let cu_limit = 450_000u32;
    let loaded_data = 128_000u32;
    let heap_size = 65_536u32;

    let config = V1ConfigParams {
        priority_fee_lamports: Some(priority_fee),
        compute_unit_limit: Some(cu_limit),
        loaded_accounts_data_size_limit: Some(loaded_data),
        heap_size: Some(heap_size),
    };

    let (mask, values) = config.compute_mask_and_values();
    assert_eq!(
        mask,
        CONFIG_MASK_PRIORITY_FEE
            | CONFIG_MASK_CU_LIMIT
            | CONFIG_MASK_LOADED_DATA_LIMIT
            | CONFIG_MASK_HEAP_SIZE
    );
    // 8 bytes (priority fee) + 4 bytes (CU) + 4 bytes (loaded data) + 4 bytes (heap) = 20 bytes
    assert_eq!(values.len(), 20);

    let tx = AdaptiveTxCompiler::build_signed_v1_transaction(
        &signer,
        &[ix],
        blockhash,
        config,
        FormatSelectionReason::ExplicitRequested(TransactionVersion::V1),
    )
    .expect("compile v1 with full config");

    // Check message bytes offset 4..8 contains mask
    let msg_mask = u32::from_le_bytes(tx.message_bytes[4..8].try_into().unwrap());
    assert_eq!(msg_mask, mask);

    assert!(tx.verify_signatures(&[signer.pubkey()]).unwrap());
}

#[test]
fn test_v1_sanitization_rules() {
    let signer = test_signer();
    let blockhash = [1u8; 32];
    let config = V1ConfigParams::default();

    // 1. Empty instructions are accepted (matches official SDK behavior).
    let empty_result =
        AdaptiveTxCompiler::compile_v1_message(signer.pubkey(), &[], blockhash, config);
    assert!(
        empty_result.is_ok(),
        "empty instructions must be accepted (matches SDK)"
    );

    // 2. SIMD-0160: Reject > 64 instructions
    let mut instructions = Vec::new();
    for _ in 0..65 {
        instructions.push(sample_system_instruction(
            signer.pubkey(),
            Pubkey::new_unique(),
            1,
        ));
    }
    let err_ix_limit =
        AdaptiveTxCompiler::compile_v1_message(signer.pubkey(), &instructions, blockhash, config)
            .unwrap_err();
    match err_ix_limit {
        TxError::SanitizationError(msg) => assert!(msg.contains("SIMD-0160 instruction limit")),
        other => panic!("Expected SanitizationError for >64 instructions, got: {other:?}"),
    }

    // 3. SIMD-0385: Reject > 64 unique addresses
    let mut wide_ix_accounts = Vec::new();
    for _ in 0..66 {
        wide_ix_accounts.push(AccountMeta::new_readonly(Pubkey::new_unique(), false));
    }
    let wide_ix = CompiledInstruction {
        program_id: Pubkey::new_from_array(SYSTEM_PROGRAM_ID),
        accounts: wide_ix_accounts,
        data: vec![1, 2, 3],
    };
    let err_addr_limit =
        AdaptiveTxCompiler::compile_v1_message(signer.pubkey(), &[wide_ix], blockhash, config)
            .unwrap_err();
    match err_addr_limit {
        TxError::SanitizationError(msg) => assert!(msg.contains("SIMD-0385 address limit")),
        other => panic!("Expected SanitizationError for >64 accounts, got: {other:?}"),
    }
}

#[test]
fn test_large_payload_v1_vs_legacy_ceiling() {
    let signer = test_signer();
    let blockhash = [42u8; 32];

    // Create an instruction with 1,500 bytes of data (exceeds IPv6 1232-byte MTU)
    let large_data = vec![0xAB; 1500];
    let large_ix = CompiledInstruction {
        program_id: Pubkey::new_from_array(SYSTEM_PROGRAM_ID),
        accounts: vec![
            AccountMeta::new(signer.pubkey(), true),
            AccountMeta::new(Pubkey::new_unique(), false),
        ],
        data: large_data,
    };

    // 1. Attempting Legacy compilation must fail closed due to MTU limit
    let legacy_res = AdaptiveTxCompiler::build_signed_legacy_transaction(
        &signer,
        std::slice::from_ref(&large_ix),
        blockhash,
        FormatSelectionReason::ExplicitRequested(TransactionVersion::Legacy),
    );
    assert!(legacy_res.is_err());
    match legacy_res.unwrap_err() {
        TxError::TransactionTooLarge { size, limit } => {
            assert!(size > SOLANA_PACKET_LEGACY_MTU);
            assert_eq!(limit, SOLANA_PACKET_LEGACY_MTU);
        }
        other => panic!("Expected TransactionTooLarge, got: {other:?}"),
    }

    // 2. Attempting adaptive compile with legacy-only capabilities must fail closed
    let adapt_legacy = AdaptiveTxCompiler::compile_adaptive(
        &signer,
        std::slice::from_ref(&large_ix),
        blockhash,
        &[],
        None,
        CompilerCapabilities::legacy_only(),
        None,
    );
    assert!(adapt_legacy.is_err());

    // 3. Adaptive compile with v1_capable() must succeed and select V1
    let adapt_v1 = AdaptiveTxCompiler::compile_adaptive(
        &signer,
        &[large_ix],
        blockhash,
        &[],
        Some(valid_v1_config()),
        CompilerCapabilities::v1_capable(),
        None,
    )
    .expect("adaptive compile with v1 support");

    assert_eq!(adapt_v1.version, TransactionVersion::V1);
    assert!(adapt_v1.wire_size() > SOLANA_PACKET_LEGACY_MTU);
    assert!(adapt_v1.wire_size() <= SOLANA_PACKET_V1_MTU);
    match adapt_v1.selection_reason {
        FormatSelectionReason::LargePayloadV1 { wire_size, .. } => {
            assert!(wire_size > SOLANA_PACKET_LEGACY_MTU);
        }
        other => panic!("Expected LargePayloadV1 reason, got: {other:?}"),
    }

    // Verify cryptographic signature
    assert!(adapt_v1.verify_signatures(&[signer.pubkey()]).unwrap());
}

#[test]
fn test_adaptive_alt_selection_for_many_accounts() {
    let signer = test_signer();
    let blockhash = [55u8; 32];

    // Generate 40 unique accounts
    let accounts_pool: Vec<Pubkey> = (0..40).map(|_| Pubkey::new_unique()).collect();

    // Create an Address Lookup Table with 30 of these accounts
    let alt_key = Pubkey::new_unique();
    let alt = AddressLookupTable {
        key: alt_key,
        addresses: accounts_pool[0..30].to_vec(),
    };

    let mut ix_accounts = vec![AccountMeta::new(signer.pubkey(), true)];
    for acc in &accounts_pool {
        ix_accounts.push(AccountMeta::new(*acc, false));
    }

    let ix = CompiledInstruction {
        program_id: Pubkey::new_from_array(SYSTEM_PROGRAM_ID),
        accounts: ix_accounts,
        data: vec![1, 2, 3, 4],
    };

    let capabilities = CompilerCapabilities::v0_standard();

    let tx = AdaptiveTxCompiler::compile_adaptive(
        &signer,
        &[ix],
        blockhash,
        &[alt],
        None,
        capabilities,
        None,
    )
    .expect("adaptive compile with ALT");

    assert_eq!(tx.version, TransactionVersion::V0);
    assert!(tx.wire_size() <= SOLANA_PACKET_LEGACY_MTU);
    match tx.selection_reason {
        FormatSelectionReason::AltRequiredForMtu { table_count, .. } => {
            assert_eq!(table_count, 1);
        }
        FormatSelectionReason::V0Default { .. } => {}
        other => panic!("Expected AltRequiredForMtu or V0Default, got: {other:?}"),
    }

    // Check V0 wire layout: byte 0 is signature count (1), byte 65 is 0x80
    assert_eq!(tx.wire_bytes[0], 1);
    assert_eq!(tx.wire_bytes[65], V0_VERSION_DISCRIMINATOR);
    assert_eq!(tx.max_supported_transaction_version(), Some(0));

    // Verify signature
    assert!(tx.verify_signatures(&[signer.pubkey()]).unwrap());
}

#[test]
fn test_adaptive_selection_small_payload_legacy_and_v0() {
    let signer = test_signer();
    let blockhash = [12u8; 32];
    let ix = sample_system_instruction(signer.pubkey(), Pubkey::new_unique(), 10_000);

    // 1. Under legacy_only: compiles Legacy format
    let legacy_tx = AdaptiveTxCompiler::compile_adaptive(
        &signer,
        std::slice::from_ref(&ix),
        blockhash,
        &[],
        None,
        CompilerCapabilities::legacy_only(),
        None,
    )
    .expect("compile legacy");

    assert_eq!(legacy_tx.version, TransactionVersion::Legacy);
    assert_eq!(legacy_tx.max_supported_transaction_version(), None);
    assert!(legacy_tx.verify_signatures(&[signer.pubkey()]).unwrap());

    // 2. Under v0_standard: compiles V0 format as canonical default
    let v0_tx = AdaptiveTxCompiler::compile_adaptive(
        &signer,
        &[ix],
        blockhash,
        &[],
        None,
        CompilerCapabilities::v0_standard(),
        None,
    )
    .expect("compile v0");

    assert_eq!(v0_tx.version, TransactionVersion::V0);
    assert_eq!(v0_tx.max_supported_transaction_version(), Some(0));
    assert!(v0_tx.verify_signatures(&[signer.pubkey()]).unwrap());
}

#[test]
fn test_fail_closed_on_mtu_breach() {
    let signer = test_signer();
    let blockhash = [99u8; 32];

    // Create an instruction payload that causes the total transaction to exceed 4,096 bytes
    let huge_data = vec![0xFF; 4100];
    let huge_ix = CompiledInstruction {
        program_id: Pubkey::new_from_array(SYSTEM_PROGRAM_ID),
        accounts: vec![AccountMeta::new(signer.pubkey(), true)],
        data: huge_data,
    };

    let result = AdaptiveTxCompiler::compile_adaptive(
        &signer,
        &[huge_ix],
        blockhash,
        &[],
        Some(valid_v1_config()),
        CompilerCapabilities::v1_capable(),
        None,
    );

    assert!(result.is_err());
    match result.unwrap_err() {
        TxError::TransactionTooLarge { size, limit } => {
            assert!(size > SOLANA_PACKET_V1_MTU);
            assert_eq!(limit, SOLANA_PACKET_V1_MTU);
        }
        other => panic!("Expected TransactionTooLarge breach, got: {other:?}"),
    }
}

#[test]
fn test_differential_signature_verification_across_all_formats() {
    let signer = test_signer();
    let blockhash = [33u8; 32];
    let ix = sample_system_instruction(signer.pubkey(), Pubkey::new_unique(), 42_000);

    // Legacy
    let legacy = AdaptiveTxCompiler::build_signed_legacy_transaction(
        &signer,
        std::slice::from_ref(&ix),
        blockhash,
        FormatSelectionReason::ExplicitRequested(TransactionVersion::Legacy),
    )
    .expect("build legacy");
    assert!(legacy.verify_signatures(&[signer.pubkey()]).unwrap());

    // V0
    let v0 = AdaptiveTxCompiler::build_signed_v0_transaction(
        &signer,
        std::slice::from_ref(&ix),
        blockhash,
        &[],
        FormatSelectionReason::ExplicitRequested(TransactionVersion::V0),
    )
    .expect("build v0");
    assert!(v0.verify_signatures(&[signer.pubkey()]).unwrap());

    // V1
    let v1 = AdaptiveTxCompiler::build_signed_v1_transaction(
        &signer,
        &[ix],
        blockhash,
        V1ConfigParams {
            priority_fee_lamports: Some(0),
            compute_unit_limit: Some(1_400_000),
            loaded_accounts_data_size_limit: Some(67_108_864),
            heap_size: None,
        },
        FormatSelectionReason::ExplicitRequested(TransactionVersion::V1),
    )
    .expect("build v1");
    assert!(v1.verify_signatures(&[signer.pubkey()]).unwrap());

    // Cross-signature extraction validation
    assert_eq!(
        extract_adaptive_transaction_signature(&legacy.wire_bytes),
        legacy.signature()
    );
    assert_eq!(
        extract_adaptive_transaction_signature(&v0.wire_bytes),
        v0.signature()
    );
    assert_eq!(
        extract_adaptive_transaction_signature(&v1.wire_bytes),
        v1.signature()
    );
}


#[test]
fn test_v1_only_compiler_and_unsigned_shadow_envelope() {
    let signer = test_signer();
    let blockhash = [0x5Au8; 32];
    let ix = sample_system_instruction(signer.pubkey(), Pubkey::new_unique(), 42_000);
    let config = V1ConfigParams {
        priority_fee_lamports: Some(12_000),
        compute_unit_limit: Some(250_000),
        loaded_accounts_data_size_limit: Some(2 * 1024 * 1024),
        heap_size: None,
    };

    let signed = AdaptiveTxCompiler::compile_v1_only(
        &signer,
        std::slice::from_ref(&ix),
        blockhash,
        config,
    )
    .expect("V1-only compiler must produce V1");
    assert_eq!(signed.version, TransactionVersion::V1);
    assert_eq!(signed.wire_bytes[0], V1_VERSION_DISCRIMINATOR);
    assert_eq!(signed.max_supported_transaction_version(), Some(1));
    assert!(signed.wire_size() <= SOLANA_PACKET_V1_MTU);

    let unsigned = AdaptiveTxCompiler::build_unsigned_v1_simulation_transaction(
        signer.pubkey(),
        &[ix],
        blockhash,
        config,
    )
    .expect("Shadow builder must produce unsigned V1 simulation bytes");
    assert_eq!(unsigned[0], V1_VERSION_DISCRIMINATOR);
    assert_eq!(CompiledAdaptiveTransaction::parse_version_from_wire(&unsigned).unwrap(), TransactionVersion::V1);
    assert!(unsigned[unsigned.len() - 64..].iter().all(|byte| *byte == 0));
    assert!(unsigned.len() <= SOLANA_PACKET_V1_MTU);
}

#[test]
fn test_v1_shadow_builder_rejects_secondary_signer() {
    let payer = Pubkey::new_unique();
    let secondary = Pubkey::new_unique();
    let ix = CompiledInstruction {
        program_id: Pubkey::new_unique(),
        accounts: vec![AccountMeta {
            pubkey: secondary,
            is_signer: true,
            is_writable: true,
        }],
        data: vec![1],
    };
    let result = AdaptiveTxCompiler::build_unsigned_v1_simulation_transaction(
        payer,
        &[ix],
        [7u8; 32],
        V1ConfigParams {
            compute_unit_limit: Some(200_000),
            loaded_accounts_data_size_limit: Some(1_048_576),
            ..Default::default()
        },
    );
    assert!(result.is_err(), "secondary signer must not be impersonated in Shadow");
}

#[test]
fn test_v1_capable_profile_never_downgrades_small_payload() {
    let signer = test_signer();
    let ix = sample_system_instruction(signer.pubkey(), Pubkey::new_unique(), 1_000);
    let tx = AdaptiveTxCompiler::compile_adaptive(
        &signer,
        &[ix],
        [0x31u8; 32],
        &[],
        Some(valid_v1_config()),
        CompilerCapabilities::v1_capable(),
        None,
    )
    .expect("V1-capable profile must compile V1 even for a small payload");
    assert_eq!(tx.version, TransactionVersion::V1);
    assert_eq!(tx.wire_bytes[0], V1_VERSION_DISCRIMINATOR);
    assert_eq!(tx.max_supported_transaction_version(), Some(1));
}
