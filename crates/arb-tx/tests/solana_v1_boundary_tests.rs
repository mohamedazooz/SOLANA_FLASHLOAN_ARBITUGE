//! Feature 20: V0 (with ALTs) vs V1 (4096B Wire Ceiling) Compilation — Tier 2 Boundary Tests.
//!
//! Validates:
//! 1. Exactly 4,096 bytes serialized wire representation is accepted with discriminator 0x81.
//! 2. Exactly 4,097 bytes wire size is strictly rejected with TransactionTooLarge error.
//! 3. Exactly 64 inline accounts is accepted at the SIMD-0385 ceiling (msg[41] == 64).
//! 4. Exactly 65 inline accounts is rejected with SanitizationError.
//! 5. Sanitization rejects invalid instruction structures / account limits.

use arb_tx::{
    AccountMeta, CompiledInstruction, KeypairSigner,
    adaptive::{
        AdaptiveTxCompiler, FormatSelectionReason, MAX_V1_ADDRESSES, MAX_V1_INSTRUCTIONS,
        SOLANA_PACKET_V1_MTU, TransactionVersion, V1ConfigParams,
    },
};
use solana_pubkey::Pubkey;

fn test_config() -> V1ConfigParams {
    V1ConfigParams {
        priority_fee_lamports: Some(1),
        compute_unit_limit: Some(200_000),
        loaded_accounts_data_size_limit: Some(1_048_576),
        heap_size: Some(256 * 1024),
    }
}

/// Boundary Case 1: Exactly 4,096 bytes serialized wire representation accepted.
#[test]
fn test_v1_exact_4096_bytes_wire_size_accepted() {
    let signer = KeypairSigner::new_random();
    let config = test_config();
    let reason = FormatSelectionReason::LargePayloadV1 {
        accounts_count: 2,
        wire_size: 0,
    };

    // 1. Probe base transaction overhead with 0 instruction data bytes
    let probe = AdaptiveTxCompiler::build_signed_v1_transaction(
        &signer,
        &[CompiledInstruction {
            program_id: Pubkey::new_unique(),
            accounts: vec![AccountMeta {
                pubkey: signer.pubkey(),
                is_signer: true,
                is_writable: true,
            }],
            data: vec![],
        }],
        [0x42u8; 32],
        config,
        reason.clone(),
    )
    .expect("probe transaction build must succeed");

    let overhead = probe.wire_bytes.len();
    assert!(overhead < SOLANA_PACKET_V1_MTU);

    // 2. Calibrate instruction data length to hit exactly 4,096 bytes
    let data_len = SOLANA_PACKET_V1_MTU - overhead;
    let built = AdaptiveTxCompiler::build_signed_v1_transaction(
        &signer,
        &[CompiledInstruction {
            program_id: Pubkey::new_unique(),
            accounts: vec![AccountMeta {
                pubkey: signer.pubkey(),
                is_signer: true,
                is_writable: true,
            }],
            data: vec![0xabu8; data_len],
        }],
        [0x42u8; 32],
        config,
        reason,
    )
    .expect("4,096-byte transaction compilation must succeed");

    assert_eq!(
        built.wire_bytes.len(),
        4096,
        "Transaction wire size must be exactly 4,096 bytes"
    );
    assert_eq!(
        built.wire_bytes[0], 0x81,
        "Version discriminator must be 0x81"
    );

    let version =
        arb_tx::adaptive::CompiledAdaptiveTransaction::parse_version_from_wire(&built.wire_bytes)
            .expect("wire parse must succeed");
    assert_eq!(version, TransactionVersion::V1);
    assert!(built.verify_signatures(&[signer.pubkey()]).unwrap());
}

/// Boundary Case 2: Exactly 4,097 bytes wire size is strictly rejected.
#[test]
fn test_v1_exact_4097_bytes_wire_size_rejected() {
    let signer = KeypairSigner::new_random();
    let config = test_config();
    let reason = FormatSelectionReason::LargePayloadV1 {
        accounts_count: 2,
        wire_size: 0,
    };

    let probe = AdaptiveTxCompiler::build_signed_v1_transaction(
        &signer,
        &[CompiledInstruction {
            program_id: Pubkey::new_unique(),
            accounts: vec![AccountMeta {
                pubkey: signer.pubkey(),
                is_signer: true,
                is_writable: true,
            }],
            data: vec![],
        }],
        [0x42u8; 32],
        config,
        reason.clone(),
    )
    .unwrap();

    let overhead = probe.wire_bytes.len();
    let data_len = (SOLANA_PACKET_V1_MTU + 1) - overhead;

    let result = AdaptiveTxCompiler::build_signed_v1_transaction(
        &signer,
        &[CompiledInstruction {
            program_id: Pubkey::new_unique(),
            accounts: vec![AccountMeta {
                pubkey: signer.pubkey(),
                is_signer: true,
                is_writable: true,
            }],
            data: vec![0u8; data_len],
        }],
        [0x42u8; 32],
        config,
        reason,
    );

    assert!(
        result.is_err(),
        "4,097 bytes transaction must be rejected with size limit error"
    );
    let err_str = result.err().unwrap().to_string();
    assert!(
        err_str.contains("exceeds limit")
            || err_str.contains("too large")
            || err_str.contains("4096"),
        "Error message must specify MTU limit violation: {}",
        err_str
    );
}

/// Boundary Case 3: Exactly 64 inline accounts is accepted at SIMD-0385 ceiling.
#[test]
fn test_v1_sixty_four_inline_accounts_exact_boundary() {
    let payer = Pubkey::new_from_array([1u8; 32]);
    let program = Pubkey::new_from_array([10u8; 32]);

    // Payer (1) + Program (1) + 62 accounts = 64 unique accounts
    let mut accounts = Vec::new();
    for i in 0..62u8 {
        let mut bytes = [0u8; 32];
        bytes[0] = i + 20;
        accounts.push(AccountMeta::new(Pubkey::new_from_array(bytes), false));
    }

    let ixs = vec![CompiledInstruction {
        program_id: program,
        accounts,
        data: vec![0],
    }];

    let result = AdaptiveTxCompiler::compile_v1_message(
        payer,
        &ixs,
        [0x42u8; 32],
        V1ConfigParams {
            priority_fee_lamports: None,
            compute_unit_limit: None,
            loaded_accounts_data_size_limit: None,
            heap_size: None,
        },
    );

    assert!(
        result.is_ok(),
        "64 inline accounts must compile successfully"
    );
    let msg = result.unwrap();
    let num_addresses = msg[41] as usize;
    assert_eq!(
        num_addresses, 64,
        "NumAddresses at byte 41 must be exactly 64"
    );
    assert_eq!(num_addresses, MAX_V1_ADDRESSES);
}

/// Boundary Case 4: Exactly 65 inline accounts is rejected with SanitizationError.
#[test]
fn test_v1_sixty_five_inline_accounts_rejected() {
    let payer = Pubkey::new_from_array([1u8; 32]);
    let program = Pubkey::new_from_array([10u8; 32]);

    // Payer (1) + Program (1) + 63 accounts = 65 unique accounts
    let mut accounts = Vec::new();
    for i in 0..63u8 {
        let mut bytes = [0u8; 32];
        bytes[0] = i + 20;
        accounts.push(AccountMeta::new(Pubkey::new_from_array(bytes), false));
    }

    let ixs = vec![CompiledInstruction {
        program_id: program,
        accounts,
        data: vec![0],
    }];

    let result = AdaptiveTxCompiler::compile_v1_message(
        payer,
        &ixs,
        [0x42u8; 32],
        V1ConfigParams {
            priority_fee_lamports: None,
            compute_unit_limit: None,
            loaded_accounts_data_size_limit: None,
            heap_size: None,
        },
    );

    assert!(result.is_err(), "65 inline accounts must be rejected");
    let err = result.err().unwrap().to_string();
    assert!(
        err.contains("SIMD-0385 address limit exceeded: 65 > 64"),
        "Expected SIMD-0385 address limit error, got: {}",
        err
    );
}

/// Boundary Case 5: Instruction count exceeding SIMD-0160 limit (65 > 64) rejected.
#[test]
fn test_v1_zero_accounts_instruction_rejected() {
    let payer = Pubkey::new_from_array([1u8; 32]);
    let program = Pubkey::new_from_array([10u8; 32]);

    // Create 65 instructions (exceeding MAX_V1_INSTRUCTIONS = 64)
    let ixs: Vec<CompiledInstruction> = (0..=MAX_V1_INSTRUCTIONS)
        .map(|i| CompiledInstruction {
            program_id: program,
            accounts: vec![],
            data: vec![i as u8],
        })
        .collect();

    assert_eq!(ixs.len(), 65);

    let result = AdaptiveTxCompiler::compile_v1_message(
        payer,
        &ixs,
        [0x42u8; 32],
        V1ConfigParams {
            priority_fee_lamports: None,
            compute_unit_limit: None,
            loaded_accounts_data_size_limit: None,
            heap_size: None,
        },
    );

    assert!(
        result.is_err(),
        "65 instructions must be rejected by sanitization"
    );
    let err = result.err().unwrap().to_string();
    assert!(
        err.contains("SIMD-0160 instruction limit exceeded: 65 > 64"),
        "Expected SIMD-0160 instruction limit error, got: {}",
        err
    );
}
