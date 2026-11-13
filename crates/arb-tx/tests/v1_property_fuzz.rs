//! Property-based and fuzz tests for the Solana Transaction V1 path (SIMD-0385)
//! and for the Kamino flash-loan fee math that funds V1 arbitrage payloads.
//!
//! Properties covered:
//! - V1 wire-format invariants for arbitrary valid instruction sets:
//!   0x81 discriminator, single-signature header, blockhash placement,
//!   config-mask encoding, inline account count, MTU compliance.
//! - Signed V1 envelope invariants: message at head, trailing 64-byte
//!   signature, cryptographic verification, version detection, and
//!   tamper detection (any single flipped message byte breaks verification).
//! - Resource-limit enforcement: 64-account boundary, 64-instruction boundary,
//!   10 KiB instruction-data limit, 4,096-byte wire boundary (4,096 accepted /
//!   4,097 rejected), 1,233-byte V1 payload accepted.
//! - Config validation is fail-closed: invalid CU / loaded-data / heap values
//!   are rejected by both the signed and the unsigned simulation builders.
//! - V1-only compiler policy: never downgrades, mandatory resource config,
//!   compute-budget instructions stripped from the V1 message.
//! - Kamino U68F60 fee math: differential against an independently formulated
//!   round-half-away-from-zero reference, monotonicity in the borrow amount,
//!   1-unit floor, fee <= principal, disabled sentinel, and the exact
//!   `BorrowTooSmall` failure boundary.

use arb_tx::{
    AccountMeta, COMPUTE_BUDGET_PROGRAM_ID, CompiledInstruction, KeypairSigner, TxError,
    adaptive::{
        AdaptiveTxCompiler, CompiledAdaptiveTransaction, CompilerCapabilities,
        FormatSelectionReason, MAX_V1_ADDRESSES, MAX_V1_INSTRUCTIONS, SOLANA_PACKET_V1_MTU,
        TransactionVersion, V1_VERSION_DISCRIMINATOR, V1ConfigParams, count_unique_accounts,
        extract_adaptive_transaction_signature, rpc_max_supported_version_from_wire,
    },
    kamino::{
        KAMINO_FRACTION_FRAC_BITS, KaminoFeeError, calculate_kamino_flash_fee,
        calculate_kamino_flash_fee_checked,
    },
};
use proptest::prelude::*;
use solana_pubkey::Pubkey;
use std::str::FromStr;

const V1_MAX_COMPUTE_UNIT_LIMIT: u32 = 1_400_000;
const V1_MAX_LOADED_DATA_LIMIT: u32 = 67_108_864;

// ---------------------------------------------------------------------------
// Strategies
// ---------------------------------------------------------------------------

fn arb_pubkey() -> impl Strategy<Value = Pubkey> {
    proptest::array::uniform32(any::<u8>()).prop_map(Pubkey::new_from_array)
}

fn arb_blockhash() -> impl Strategy<Value = [u8; 32]> {
    proptest::array::uniform32(any::<u8>())
}

fn arb_seed() -> impl Strategy<Value = [u8; 32]> {
    proptest::array::uniform32(any::<u8>())
}

/// A valid V1 resource configuration: CU limit and loaded-data limit are
/// always present and inside the runtime-accepted ranges; the heap request,
/// when present, is a 1 KiB multiple in 32..=256 KiB.
fn arb_valid_config() -> impl Strategy<Value = V1ConfigParams> {
    (
        proptest::option::of(any::<u64>()),
        1u32..=V1_MAX_COMPUTE_UNIT_LIMIT,
        1u32..=V1_MAX_LOADED_DATA_LIMIT,
        proptest::option::of((0u32..=224).prop_map(|k| 32_768 + k * 1_024)),
    )
        .prop_map(
            |(priority_fee_lamports, compute_unit_limit, loaded, heap_size)| V1ConfigParams {
                priority_fee_lamports,
                compute_unit_limit: Some(compute_unit_limit),
                loaded_accounts_data_size_limit: Some(loaded),
                heap_size,
            },
        )
}

/// Instruction sets that always compile: shared account/program pools keep the
/// unique-account count far below 64, and bounded data keeps the message far
/// below the 4,096-byte MTU (worst case ~1.8 KB).
fn arb_small_instructions() -> impl Strategy<Value = Vec<CompiledInstruction>> {
    (
        proptest::collection::vec(arb_pubkey(), 1..=16),
        proptest::collection::vec(arb_pubkey(), 1..=3),
        proptest::collection::vec(
            (
                0usize..3,
                proptest::collection::vec((0usize..16, any::<bool>()), 0..=8),
                proptest::collection::vec(any::<u8>(), 0..=64),
            ),
            0..=16,
        ),
    )
        .prop_map(|(pool, programs, templates)| {
            templates
                .into_iter()
                .map(|(p, accs, data)| CompiledInstruction {
                    program_id: programs[p % programs.len()],
                    accounts: accs
                        .into_iter()
                        .map(|(a, writable)| {
                            if writable {
                                AccountMeta::new(pool[a % pool.len()], false)
                            } else {
                                AccountMeta::new_readonly(pool[a % pool.len()], false)
                            }
                        })
                        .collect(),
                    data,
                })
                .collect()
        })
}

/// Instruction sets whose message is guaranteed to exceed the 4,096-byte V1
/// MTU: 12 instructions x >= 600 data bytes ~ >= 7.6 KB of payload alone.
fn arb_oversized_instructions() -> impl Strategy<Value = Vec<CompiledInstruction>> {
    (
        proptest::collection::vec(arb_pubkey(), 8..=8),
        arb_pubkey(),
        proptest::collection::vec(
            (
                proptest::collection::vec((0usize..8, any::<bool>()), 0..=4),
                600usize..=10_240,
            ),
            12..=12,
        ),
    )
        .prop_map(|(pool, program, templates)| {
            templates
                .into_iter()
                .map(|(accs, data_len)| CompiledInstruction {
                    program_id: program,
                    accounts: accs
                        .into_iter()
                        .map(|(a, writable)| {
                            if writable {
                                AccountMeta::new(pool[a], false)
                            } else {
                                AccountMeta::new_readonly(pool[a], false)
                            }
                        })
                        .collect(),
                    data: vec![0u8; data_len],
                })
                .collect()
        })
}

/// An invalid compute-unit limit for the fail-closed config validators:
/// absent, zero, one past the maximum, or u32::MAX.
fn arb_invalid_resource_limit() -> impl Strategy<Value = Option<u32>> {
    prop_oneof![
        Just(None),
        Just(Some(0u32)),
        Just(Some(V1_MAX_COMPUTE_UNIT_LIMIT + 1)),
        Just(Some(u32::MAX)),
    ]
}

fn arb_invalid_loaded_data_limit() -> impl Strategy<Value = Option<u32>> {
    prop_oneof![
        Just(None),
        Just(Some(0u32)),
        Just(Some(V1_MAX_LOADED_DATA_LIMIT + 1)),
        Just(Some(u32::MAX)),
    ]
}

/// A heap request that must always be rejected: either outside 32..=256 KiB or
/// not a 1 KiB multiple (forcing the low bit on guarantees the multiple check
/// fails).
fn arb_invalid_heap() -> impl Strategy<Value = u32> {
    any::<u32>().prop_map(|v| if v % 1_024 == 0 { v | 1 } else { v })
}

/// A V1 config with an invalid CU or loaded-data limit (used against the
/// simulation builder, which also rejects `None`).
fn arb_invalid_sim_config() -> impl Strategy<Value = V1ConfigParams> {
    prop_oneof![
        (
            arb_invalid_resource_limit(),
            1u32..=V1_MAX_LOADED_DATA_LIMIT
        )
            .prop_map(|(compute_unit_limit, loaded)| V1ConfigParams {
                priority_fee_lamports: None,
                compute_unit_limit,
                loaded_accounts_data_size_limit: Some(loaded),
                heap_size: None,
            }),
        (
            1u32..=V1_MAX_COMPUTE_UNIT_LIMIT,
            arb_invalid_loaded_data_limit()
        )
            .prop_map(|(compute_unit_limit, loaded_accounts_data_size_limit)| {
                V1ConfigParams {
                    priority_fee_lamports: None,
                    compute_unit_limit: Some(compute_unit_limit),
                    loaded_accounts_data_size_limit,
                    heap_size: None,
                }
            }),
    ]
}

/// Distinct pubkeys guaranteed unique by construction: a random 31-byte tail
/// with a distinct first byte, so shrinking can never collapse two keys into
/// one and flip an account-limit boundary test spuriously.
fn distinct_pubkeys(count: usize, tail: [u8; 32]) -> Vec<Pubkey> {
    (0..count)
        .map(|i| {
            let mut bytes = tail;
            bytes[0] = i as u8;
            Pubkey::new_from_array(bytes)
        })
        .collect()
}

fn keyed_pubkey(first_byte: u8, tail: [u8; 32]) -> Pubkey {
    let mut bytes = tail;
    bytes[0] = first_byte;
    Pubkey::new_from_array(bytes)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    // -----------------------------------------------------------------------
    // V1 wire-format invariants
    // -----------------------------------------------------------------------

    #[test]
    fn v1_message_header_invariants(
        ixs in arb_small_instructions(),
        bh in arb_blockhash(),
        cfg in arb_valid_config(),
        seed in arb_seed(),
    ) {
        let payer = Pubkey::new_from_array(seed);
        let msg = AdaptiveTxCompiler::compile_v1_message(payer, &ixs, bh, cfg)
            .expect("small instruction sets must always compile");

        prop_assert_eq!(msg[0], V1_VERSION_DISCRIMINATOR, "byte 0 must be the 0x81 V1 discriminator");
        prop_assert_eq!(msg[1], 1, "exactly one required signature (the payer)");
        prop_assert_eq!(msg[2], 0, "payer is a writable signer, so no read-only signed accounts");

        let (mask, values) = cfg.compute_mask_and_values();
        prop_assert_eq!(u32::from_le_bytes([msg[4], msg[5], msg[6], msg[7]]), mask);
        prop_assert_eq!(&msg[8..40], &bh[..], "lifetime specifier is the recent blockhash");
        prop_assert_eq!(msg[40] as usize, ixs.len(), "NumInstructions header field");

        let num_addresses = msg[41] as usize;
        prop_assert!(num_addresses <= MAX_V1_ADDRESSES);
        prop_assert_eq!(num_addresses, count_unique_accounts(payer, &ixs));

        let values_offset = 42 + num_addresses * 32;
        prop_assert_eq!(&msg[values_offset..values_offset + values.len()], &values[..],
            "config values are encoded contiguously after the inline addresses");
        prop_assert!(msg.len() <= SOLANA_PACKET_V1_MTU, "V1 message must fit the 4,096-byte MTU");
    }

    #[test]
    fn v1_signed_envelope_invariants(
        ixs in arb_small_instructions(),
        bh in arb_blockhash(),
        cfg in arb_valid_config(),
        seed in arb_seed(),
    ) {
        let signer = KeypairSigner::from_seed(&seed);
        let tx = AdaptiveTxCompiler::build_signed_v1_transaction(
            &signer,
            &ixs,
            bh,
            cfg,
            FormatSelectionReason::ExplicitRequested(TransactionVersion::V1),
        )
        .expect("valid config + small instructions must always sign");

        prop_assert_eq!(tx.version, TransactionVersion::V1);
        prop_assert_eq!(tx.signatures.len(), 1);
        prop_assert_eq!(tx.wire_bytes[0], V1_VERSION_DISCRIMINATOR);
        prop_assert_eq!(tx.wire_size(), tx.message_bytes.len() + 64,
            "wire = message at head + one trailing 64-byte signature");
        prop_assert_eq!(&tx.wire_bytes[..tx.message_bytes.len()], &tx.message_bytes[..]);
        prop_assert_eq!(&tx.wire_bytes[tx.wire_bytes.len() - 64..], &tx.signatures[0][..],
            "signatures are serialized at the tail");

        prop_assert!(matches!(tx.verify_signatures(&[signer.pubkey()]), Ok(true)),
            "the produced signature must cryptographically verify");
        prop_assert!(matches!(
            CompiledAdaptiveTransaction::parse_version_from_wire(&tx.wire_bytes),
            Ok(TransactionVersion::V1),
        ));
        prop_assert!(matches!(rpc_max_supported_version_from_wire(&tx.wire_bytes), Ok(Some(1))),
            "RPC pre-flight must pass maxSupportedTransactionVersion = 1");
        prop_assert!(extract_adaptive_transaction_signature(&tx.wire_bytes).is_some());
        prop_assert!(tx.wire_size() <= SOLANA_PACKET_V1_MTU);
    }

    #[test]
    fn v1_signature_tamper_detection(
        ixs in arb_small_instructions(),
        bh in arb_blockhash(),
        cfg in arb_valid_config(),
        seed in arb_seed(),
        byte_idx in 0usize..1_024,
        impostor in arb_pubkey(),
    ) {
        let signer = KeypairSigner::from_seed(&seed);
        let tx = AdaptiveTxCompiler::build_signed_v1_transaction(
            &signer,
            &ixs,
            bh,
            cfg,
            FormatSelectionReason::ExplicitRequested(TransactionVersion::V1),
        )
        .expect("valid config + small instructions must always sign");

        let mut tampered = tx.clone();
        let i = byte_idx % tampered.message_bytes.len();
        tampered.message_bytes[i] ^= 0x01;
        prop_assert!(
            !matches!(tampered.verify_signatures(&[signer.pubkey()]), Ok(true)),
            "any single flipped message byte must break signature verification",
        );

        prop_assert!(
            !matches!(tx.verify_signatures(&[impostor]), Ok(true)),
            "verification against a non-signing pubkey must never succeed",
        );
    }

    #[test]
    fn v1_compile_adaptive_is_v1_only_never_downgrades(
        ixs in arb_small_instructions(),
        bh in arb_blockhash(),
        cfg in arb_valid_config(),
        seed in arb_seed(),
    ) {
        let signer = KeypairSigner::from_seed(&seed);
        let via_adaptive = AdaptiveTxCompiler::compile_adaptive(
            &signer,
            &ixs,
            bh,
            &[],
            Some(cfg),
            CompilerCapabilities::v1_capable(),
            None,
        )
        .expect("V1-capable profile with a valid config must compile");
        let via_v1_only = AdaptiveTxCompiler::compile_v1_only(&signer, &ixs, bh, cfg)
            .expect("the same inputs must compile through the V1-only entry point");

        prop_assert_eq!(via_adaptive.version, TransactionVersion::V1,
            "V1-capable profiles must never downgrade to Legacy/V0");
        prop_assert_eq!(via_adaptive.wire_bytes.as_slice(), via_v1_only.wire_bytes.as_slice(),
            "compile_adaptive and compile_v1_only must produce identical V1 wire bytes");
        prop_assert!(matches!(
            CompiledAdaptiveTransaction::parse_version_from_wire(&via_adaptive.wire_bytes),
            Ok(TransactionVersion::V1),
        ));
        prop_assert!(via_adaptive.wire_size() <= SOLANA_PACKET_V1_MTU);
        prop_assert!(matches!(via_adaptive.verify_signatures(&[signer.pubkey()]), Ok(true)));
    }

    #[test]
    fn v1_compile_adaptive_requires_config(seed in arb_seed(), ixs in arb_small_instructions(), bh in arb_blockhash()) {
        let signer = KeypairSigner::from_seed(&seed);
        let err = AdaptiveTxCompiler::compile_adaptive(
            &signer,
            &ixs,
            bh,
            &[],
            None,
            CompilerCapabilities::v1_capable(),
            None,
        )
        .expect_err("V1-capable profile without a resource config must fail closed");
        prop_assert!(matches!(err, TxError::SanitizationError(_)), "got: {err:?}");
    }

    #[test]
    fn v1_strips_compute_budget_instructions(
        ixs in arb_small_instructions(),
        bh in arb_blockhash(),
        cfg in arb_valid_config(),
        seed in arb_seed(),
    ) {
        let payer = Pubkey::new_from_array(seed);
        let baseline = AdaptiveTxCompiler::compile_v1_message(payer, &ixs, bh, cfg)
            .expect("small instruction sets must always compile");

        let mut padded = Vec::with_capacity(ixs.len() + 2);
        padded.push(CompiledInstruction {
            program_id: Pubkey::new_from_array(COMPUTE_BUDGET_PROGRAM_ID),
            accounts: vec![],
            data: vec![2, 0, 0, 0, 0x40, 0x0d, 0x03, 0x00],
        });
        padded.extend(ixs.iter().cloned());
        padded.push(CompiledInstruction {
            program_id: Pubkey::new_from_array(COMPUTE_BUDGET_PROGRAM_ID),
            accounts: vec![],
            data: vec![2, 0, 0, 0, 0x40, 0x0d, 0x03, 0x00],
        });

        let with_budget = AdaptiveTxCompiler::compile_v1_message(payer, &padded, bh, cfg)
            .expect("padding with compute-budget no-ops must not change compilability");
        prop_assert_eq!(with_budget, baseline,
            "V1 resource config lives in the message header, so legacy ComputeBudget instructions must be stripped");
    }

    // -----------------------------------------------------------------------
    // Resource limits
    // -----------------------------------------------------------------------

    #[test]
    fn v1_account_limit_boundary(tail in arb_seed(), cfg in arb_valid_config()) {
        let payer = keyed_pubkey(0xfe, tail);
        let program = keyed_pubkey(0x7f, tail);

        let build = |n_accounts: usize| {
            distinct_pubkeys(n_accounts, tail)
                .into_iter()
                .map(|pk| AccountMeta::new_readonly(pk, false))
                .collect::<Vec<_>>()
        };
        let ixs = |n_accounts: usize| {
            vec![CompiledInstruction {
                program_id: program,
                accounts: build(n_accounts),
                data: vec![0u8; 8],
            }]
        };

        let ok = AdaptiveTxCompiler::compile_v1_message(payer, &ixs(62), [7u8; 32], cfg)
            .expect("payer + program + 62 accounts = 64 unique addresses must compile");
        prop_assert_eq!(ok[41] as usize, 64, "NumAddresses must hit exactly the SIMD-0385 limit");

        let err = AdaptiveTxCompiler::compile_v1_message(payer, &ixs(63), [7u8; 32], V1ConfigParams::default())
            .expect_err("65 unique addresses must be rejected");
        prop_assert!(matches!(err, TxError::SanitizationError(_)), "got: {err:?}");
    }

    #[test]
    fn v1_instruction_limit_boundary(
        tail in arb_seed(),
        pool in proptest::collection::vec(arb_pubkey(), 4..=4),
        bh in arb_blockhash(),
        cfg in arb_valid_config(),
    ) {
        let payer = keyed_pubkey(0xfd, tail);
        let program = keyed_pubkey(0x7f, tail);

        let build = |n: usize| {
            (0..n)
                .map(|i| CompiledInstruction {
                    program_id: program,
                    accounts: vec![AccountMeta::new_readonly(pool[i % pool.len()], false)],
                    data: vec![i as u8; 4],
                })
                .collect::<Vec<_>>()
        };

        let ok = AdaptiveTxCompiler::compile_v1_message(payer, &build(MAX_V1_INSTRUCTIONS), bh, cfg)
            .expect("64 instructions must compile");
        prop_assert_eq!(ok[40] as usize, MAX_V1_INSTRUCTIONS);

        let err = AdaptiveTxCompiler::compile_v1_message(payer, &build(MAX_V1_INSTRUCTIONS + 1), bh, V1ConfigParams::default())
            .expect_err("65 instructions must be rejected (SIMD-0160)");
        prop_assert!(matches!(err, TxError::SanitizationError(_)), "got: {err:?}");
    }

    #[test]
    fn v1_oversized_message_is_rejected(
        ixs in arb_oversized_instructions(),
        cfg in arb_valid_config(),
        seed in arb_seed(),
    ) {
        let payer = Pubkey::new_from_array(seed);
        let err = AdaptiveTxCompiler::compile_v1_message(payer, &ixs, [9u8; 32], cfg)
            .expect_err("messages above 4,096 bytes must be rejected");
        match err {
            TxError::TransactionTooLarge { size, limit } => {
                prop_assert_eq!(limit, SOLANA_PACKET_V1_MTU);
                prop_assert!(size > SOLANA_PACKET_V1_MTU, "reported size {size} must exceed the MTU");
            }
            other => panic!("expected TransactionTooLarge, got: {other:?}"),
        }
    }

    #[test]
    fn v1_instruction_data_limit_is_enforced(
        data_len in 10_241usize..=20_000,
        cfg in arb_valid_config(),
        seed in arb_seed(),
    ) {
        let payer = Pubkey::new_from_array(seed);
        let program = Pubkey::new_from_array([0x11u8; 32]);
        let ixs = vec![CompiledInstruction {
            program_id: program,
            accounts: vec![],
            data: vec![0u8; data_len],
        }];
        let err = AdaptiveTxCompiler::compile_v1_message(payer, &ixs, [9u8; 32], cfg)
            .expect_err("instruction data above the 10 KiB Solana limit must be rejected");
        prop_assert!(matches!(err, TxError::SanitizationError(_)), "got: {err:?}");
    }

    #[test]
    fn v1_simulation_builder_rejects_invalid_configs(
        cfg in arb_invalid_sim_config(),
        seed in arb_seed(),
        ixs in arb_small_instructions(),
    ) {
        let payer = Pubkey::new_from_array(seed);
        let err = AdaptiveTxCompiler::build_unsigned_v1_simulation_transaction(payer, &ixs, [5u8; 32], cfg)
            .expect_err("simulation envelope must fail closed on invalid resource limits");
        prop_assert!(matches!(err, TxError::SanitizationError(_)), "got: {err:?}");
    }

    #[test]
    fn v1_signed_builder_rejects_invalid_configs(
        cu in arb_invalid_resource_limit(),
        data in arb_invalid_loaded_data_limit(),
        bad_heap in arb_invalid_heap(),
        seed in arb_seed(),
        ixs in arb_small_instructions(),
    ) {
        let signer = KeypairSigner::from_seed(&seed);
        let cfg = V1ConfigParams {
            priority_fee_lamports: None,
            compute_unit_limit: cu,
            loaded_accounts_data_size_limit: data,
            heap_size: Some(bad_heap),
        };
        let err = AdaptiveTxCompiler::build_signed_v1_transaction(
            &signer,
            &ixs,
            [5u8; 32],
            cfg,
            FormatSelectionReason::ExplicitRequested(TransactionVersion::V1),
        )
        .expect_err("signed V1 must fail closed on any invalid resource config");
        prop_assert!(matches!(err, TxError::SanitizationError(_)), "got: {err:?}");
    }

    #[test]
    fn v1_simulation_envelope_zero_fills_signature_slots(
        ixs in arb_small_instructions(),
        bh in arb_blockhash(),
        cfg in arb_valid_config(),
        seed in arb_seed(),
    ) {
        let payer = Pubkey::new_from_array(seed);
        let wire = AdaptiveTxCompiler::build_unsigned_v1_simulation_transaction(payer, &ixs, bh, cfg)
            .expect("valid config + small instructions must build the simulation envelope");
        let message = AdaptiveTxCompiler::compile_v1_message(payer, &ixs, bh, cfg)
            .expect("the same inputs must compile standalone");
        prop_assert_eq!(wire.len(), message.len() + 64);
        prop_assert!(wire[message.len()..].iter().all(|&b| b == 0),
            "signature slots must be zero-filled for sigVerify: false simulation");
        prop_assert_eq!(&wire[..message.len()], &message[..]);
    }

    // -----------------------------------------------------------------------
    // Wire-size boundaries
    // -----------------------------------------------------------------------

    #[test]
    fn v1_wire_size_boundaries(seed in arb_seed()) {
        let signer = KeypairSigner::from_seed(&seed);
        let program = Pubkey::new_from_array([0x22u8; 32]);
        let account = Pubkey::new_from_array([0x33u8; 32]);
        // Fixed valid config: CU + loaded-data values = 8 bytes in the message.
        let cfg = V1ConfigParams {
            priority_fee_lamports: None,
            compute_unit_limit: Some(200_000),
            loaded_accounts_data_size_limit: Some(V1_MAX_LOADED_DATA_LIMIT),
            heap_size: None,
        };

        let signed = |data_len: usize| {
            let ixs = vec![CompiledInstruction {
                program_id: program,
                accounts: vec![AccountMeta::new_readonly(account, false)],
                data: vec![0u8; data_len],
            }];
            AdaptiveTxCompiler::build_signed_v1_transaction(
                &signer,
                &ixs,
                [3u8; 32],
                cfg,
                FormatSelectionReason::ExplicitRequested(TransactionVersion::V1),
            )
        };
        // message = 42 header + 3*32 addresses + 8 config values + 4 ix header + 1 account index + data_len
        // wire    = message + 64-byte trailing signature = 215 + data_len

        let tx_1233 = signed(1_018).expect("a 1,233-byte V1 transaction must be accepted");
        prop_assert_eq!(tx_1233.wire_size(), 1_233,
            "a payload one byte above the legacy 1,232-byte MTU stays valid under V1");

        let tx_4096 = signed(3_881).expect("a 4,096-byte V1 transaction must be accepted");
        prop_assert_eq!(tx_4096.wire_size(), SOLANA_PACKET_V1_MTU,
            "the V1 MTU boundary itself must be accepted");

        let err = signed(3_882).expect_err("a 4,097-byte V1 transaction must be rejected");
        match err {
            TxError::TransactionTooLarge { size, limit } => {
                prop_assert_eq!(size, 4_097);
                prop_assert_eq!(limit, SOLANA_PACKET_V1_MTU);
            }
            other => panic!("expected TransactionTooLarge, got: {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Kamino flash-loan fee math (U68F60)
    // -----------------------------------------------------------------------

    #[test]
    fn kamino_fee_differential(amount in any::<u64>(), fee_sf in any::<u64>()) {
        let checked = calculate_kamino_flash_fee_checked(amount, fee_sf);
        let infallible = calculate_kamino_flash_fee(amount, fee_sf);

        if fee_sf == u64::MAX {
            prop_assert_eq!(checked, Err(KaminoFeeError::FlashLoanDisabled));
            prop_assert_eq!(infallible, u64::MAX, "disabled reserve sentinel");
            return Ok(());
        }
        if fee_sf == 0 || amount == 0 {
            prop_assert_eq!(checked, Ok(0));
            prop_assert_eq!(infallible, 0);
            return Ok(());
        }

        // Independent reference: fee = round(amount * sf / 2^60) with a floor
        // of one atomic unit, ties away from zero, expressed with div/mod
        // instead of the shift/mask formulation used by the implementation.
        let scale = 1u128 << KAMINO_FRACTION_FRAC_BITS;
        let product = amount as u128 * fee_sf as u128;
        let bits = product.max(scale);
        if bits >= (amount as u128) << KAMINO_FRACTION_FRAC_BITS {
            prop_assert_eq!(checked, Err(KaminoFeeError::BorrowTooSmall));
            return Ok(());
        }
        let (q, r) = (bits / scale, bits % scale);
        let reference = (q + u128::from(r * 2 >= scale)).min(u64::MAX as u128) as u64;

        prop_assert_eq!(checked, Ok(reference), "checked fee must match the reference rounding");
        prop_assert_eq!(infallible, reference,
            "infallible fee must agree with the checked fee on non-disabled reserves");
    }

    #[test]
    fn kamino_fee_monotonic_in_amount(fee_sf in any::<u64>(), a in any::<u64>(), b in any::<u64>()) {
        let (small, large) = if a <= b { (a, b) } else { (b, a) };
        let fee_small = calculate_kamino_flash_fee(small, fee_sf);
        let fee_large = calculate_kamino_flash_fee(large, fee_sf);
        prop_assert!(
            fee_small <= fee_large,
            "fee({small}) = {fee_small} must not exceed fee({large}) = {fee_large}",
        );
    }

    #[test]
    fn kamino_fee_bounded_by_principal_and_floor(
        amount in 2u64..=u64::MAX,
        fee_sf in 1u64..(1u128 << KAMINO_FRACTION_FRAC_BITS) as u64,
    ) {
        let fee = calculate_kamino_flash_fee_checked(amount, fee_sf)
            .expect("sf < 2^60 and amount >= 2 must never hit BorrowTooSmall");
        prop_assert!(fee >= 1, "the 1-atomic-unit klend minimum fee must hold");
        prop_assert!(fee <= amount, "an exclusive origination fee can never exceed the principal");
    }

    #[test]
    fn kamino_borrow_too_small_boundary(amount in 1u64..=u64::MAX, fee_sf in 1u64..u64::MAX) {
        let checked = calculate_kamino_flash_fee_checked(amount, fee_sf);
        let should_fail = fee_sf >= (1u128 << KAMINO_FRACTION_FRAC_BITS) as u64 || amount == 1;
        prop_assert_eq!(
            checked.is_err(),
            should_fail,
            "BorrowTooSmall must trigger exactly when fee_bits >= amount_bits",
        );
        if should_fail {
            prop_assert!(matches!(checked, Err(KaminoFeeError::BorrowTooSmall)), "got: {checked:?}");
        }
    }

}

/// Walk the compiled V1 message payload section and return the data payload of
/// the instruction at `ix_index` (post-strip executed ordering).
fn instruction_data_from_v1_message(msg: &[u8], ix_index: usize) -> Vec<u8> {
    let num_ixs = msg[40] as usize;
    let num_addresses = msg[41] as usize;
    let mut offset = 42 + num_addresses * 32;
    let (mask, values_len) = {
        let mask = u32::from_le_bytes([msg[4], msg[5], msg[6], msg[7]]);
        let values_len = mask.count_ones() as usize * 4;
        (mask, values_len)
    };
    let _ = mask;
    offset += values_len;
    offset += num_ixs * 4;
    for i in 0..num_ixs {
        let acc_count = msg[42 + num_addresses * 32 + values_len + i * 4 + 1] as usize;
        let data_len = u16::from_le_bytes([
            msg[42 + num_addresses * 32 + values_len + i * 4 + 2],
            msg[42 + num_addresses * 32 + values_len + i * 4 + 3],
        ]) as usize;
        let payload_len = acc_count + data_len;
        if i == ix_index {
            return msg[offset + acc_count..offset + payload_len].to_vec();
        }
        offset += payload_len;
    }
    panic!("instruction {ix_index} not found in V1 message");
}

#[test]
fn v1_rewrites_klend_repay_borrow_index_after_compute_budget_strip() {
    let payer = Pubkey::new_from_array([1u8; 32]);
    let klend = Pubkey::from_str(arb_tx::KAMINO_KLEND_PROGRAM_ID).unwrap();
    let borrow = CompiledInstruction {
        program_id: klend,
        accounts: vec![],
        data: {
            let mut d = arb_tx::FLASH_BORROW_DISCRIMINATOR.to_vec();
            d.extend_from_slice(&1_000_000_000u64.to_le_bytes());
            d
        },
    };
    let repay = |borrow_ix_index: u8| CompiledInstruction {
        program_id: klend,
        accounts: vec![],
        data: {
            let mut d = arb_tx::FLASH_REPAY_DISCRIMINATOR.to_vec();
            d.extend_from_slice(&1_000_000_000u64.to_le_bytes());
            d.push(borrow_ix_index);
            d
        },
    };
    let cu = || CompiledInstruction {
        program_id: Pubkey::new_from_array(COMPUTE_BUDGET_PROGRAM_ID),
        accounts: vec![],
        data: vec![2, 0, 0, 0, 0x40, 0x0d, 0x03, 0x00],
    };

    // Build-time layout: [CU, CU, borrow(2), repay(idx=2)]. After the V1
    // ComputeBudget strip the executed layout is [borrow(0), repay(idx=0)],
    // so the repay's embedded borrow_instruction_index must be rewritten 2 -> 0.
    let msg = AdaptiveTxCompiler::compile_v1_message(
        payer,
        &[cu(), cu(), borrow, repay(2)],
        [0u8; 32],
        V1ConfigParams::default(),
    )
    .expect("must compile");
    assert_eq!(msg[40] as usize, 2, "ComputeBudget no-ops must be stripped");
    let repay_data = instruction_data_from_v1_message(&msg, 1);
    assert_eq!(&repay_data[0..8], &arb_tx::FLASH_REPAY_DISCRIMINATOR);
    assert_eq!(
        repay_data[16], 0,
        "borrow_instruction_index must reference the executed position"
    );

    // Without ComputeBudget no-ops the index is already executed-correct.
    let msg = AdaptiveTxCompiler::compile_v1_message(
        payer,
        &[
            CompiledInstruction {
                program_id: klend,
                accounts: vec![],
                data: {
                    let mut d = arb_tx::FLASH_BORROW_DISCRIMINATOR.to_vec();
                    d.extend_from_slice(&1_000_000_000u64.to_le_bytes());
                    d
                },
            },
            repay(0),
        ],
        [0u8; 32],
        V1ConfigParams::default(),
    )
    .expect("must compile");
    let repay_data = instruction_data_from_v1_message(&msg, 1);
    assert_eq!(repay_data[16], 0);

    // Fail closed: an index pointing at a stripped ComputeBudget instruction
    // can never pair with a borrow after the strip.
    let err = AdaptiveTxCompiler::compile_v1_message(
        payer,
        &[cu(), repay(0)],
        [0u8; 32],
        V1ConfigParams::default(),
    )
    .expect_err("borrow_instruction_index pointing at a stripped instruction must be rejected");
    assert!(matches!(err, TxError::SanitizationError(_)), "got: {err:?}");
}

#[test]
fn v1_executed_instruction_index_maps_through_compute_budget_strip() {
    use arb_tx::adaptive::v1_executed_instruction_index;
    let cu = Pubkey::new_from_array(COMPUTE_BUDGET_PROGRAM_ID);
    let other = Pubkey::new_from_array([9u8; 32]);
    let ix = |program: Pubkey| CompiledInstruction {
        program_id: program,
        accounts: vec![],
        data: vec![],
    };
    let ixs = vec![ix(cu), ix(cu), ix(other), ix(cu), ix(other)];
    assert_eq!(v1_executed_instruction_index(&ixs, 2), Some(0));
    assert_eq!(v1_executed_instruction_index(&ixs, 4), Some(1));
    assert_eq!(v1_executed_instruction_index(&ixs, 0), None);
    assert_eq!(v1_executed_instruction_index(&ixs, 3), None);
    assert_eq!(v1_executed_instruction_index(&ixs, 5), None);
}

#[test]
fn kamino_main_market_wsol_fee_samples() {
    // Main-market WSOL reserve: sf = 11_529_215_046_068 (1e-5 = 0.1 bps).
    const WSOL_FLASH_LOAN_FEE_SF: u64 = 11_529_215_046_068;
    assert_eq!(
        calculate_kamino_flash_fee_checked(1_000_000_000, WSOL_FLASH_LOAN_FEE_SF),
        Ok(10_000),
        "1 SOL borrow must cost exactly 10,000 lamports",
    );
    assert_eq!(
        calculate_kamino_flash_fee_checked(2_000_000_000, WSOL_FLASH_LOAN_FEE_SF),
        Ok(20_000),
    );
    assert_eq!(
        calculate_kamino_flash_fee_checked(123_456_789, WSOL_FLASH_LOAN_FEE_SF),
        Ok(1_235),
        "1234.56788... rounds away from zero to 1,235",
    );
}
