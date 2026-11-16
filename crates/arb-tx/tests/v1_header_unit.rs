//! Phase 16.1 — V1 Message Header Unit Tests.
//!
//! Tests the internal correctness of our V1 (SIMD-0385) message header
//! computation: num_required_signatures, num_readonly_signed_accounts,
//! num_readonly_unsigned_accounts, account ordering, signer classification,
//! writable/read-only classification, static account key compilation,
//! program ID handling, compiled instruction account indices, and
//! recent blockhash placement.

use arb_tx::{
    AccountMeta, CompiledInstruction,
    adaptive::{AdaptiveTxCompiler, V1ConfigParams},
};
use solana_pubkey::Pubkey;

fn test_payer() -> Pubkey {
    Pubkey::new_from_array([1u8; 32])
}

fn test_blockhash() -> [u8; 32] {
    [42u8; 32]
}

fn empty_config() -> V1ConfigParams {
    V1ConfigParams {
        priority_fee_lamports: None,
        compute_unit_limit: None,
        loaded_accounts_data_size_limit: None,
        heap_size: None,
    }
}

/// Extract the config mask from a compiled V1 message.
fn config_mask(msg: &[u8]) -> u32 {
    u32::from_le_bytes([msg[4], msg[5], msg[6], msg[7]])
}

/// Compile a V1 message and return bytes.
fn compile(
    payer: &Pubkey,
    ixs: &[CompiledInstruction],
    bh: [u8; 32],
    cfg: V1ConfigParams,
) -> Vec<u8> {
    AdaptiveTxCompiler::compile_v1_message(*payer, ixs, bh, cfg).expect("compile must succeed")
}

// ---------------------------------------------------------------------------
// num_required_signatures
// ---------------------------------------------------------------------------

#[test]
fn test_num_required_signatures_single_payer() {
    let payer = test_payer();
    let program = Pubkey::new_from_array([10u8; 32]);
    let acc = Pubkey::new_from_array([20u8; 32]);

    let ixs = vec![CompiledInstruction {
        program_id: program,
        accounts: vec![AccountMeta::new_readonly(acc, false)],
        data: vec![1],
    }];

    let msg = compile(&payer, &ixs, test_blockhash(), empty_config());
    assert_eq!(msg[1], 1, "single payer: num_required_signatures must be 1");
}

#[test]
fn test_num_required_signatures_multiple_signers() {
    let payer = test_payer();
    let signer2 = Pubkey::new_from_array([5u8; 32]);
    let signer3 = Pubkey::new_from_array([6u8; 32]);
    let program = Pubkey::new_from_array([10u8; 32]);

    let ixs = vec![CompiledInstruction {
        program_id: program,
        accounts: vec![
            AccountMeta::new(signer2, true),
            AccountMeta::new_readonly(signer3, true),
        ],
        data: vec![1],
    }];

    let msg = compile(&payer, &ixs, test_blockhash(), empty_config());
    assert_eq!(
        msg[1], 3,
        "payer + 2 signers: num_required_signatures must be 3"
    );
}

#[test]
fn test_num_required_signatures_readonly_signer_counted() {
    let payer = test_payer();
    let signer_ro = Pubkey::new_from_array([5u8; 32]);
    let program = Pubkey::new_from_array([10u8; 32]);

    let ixs = vec![CompiledInstruction {
        program_id: program,
        accounts: vec![AccountMeta::new_readonly(signer_ro, true)],
        data: vec![1],
    }];

    let msg = compile(&payer, &ixs, test_blockhash(), empty_config());
    assert_eq!(
        msg[1], 2,
        "payer + readonly signer: num_required_signatures must be 2"
    );
}

// ---------------------------------------------------------------------------
// num_readonly_signed_accounts
// ---------------------------------------------------------------------------

#[test]
fn test_num_readonly_signed_zero() {
    let payer = test_payer();
    let program = Pubkey::new_from_array([10u8; 32]);
    let acc = Pubkey::new_from_array([20u8; 32]);

    let ixs = vec![CompiledInstruction {
        program_id: program,
        accounts: vec![AccountMeta::new_readonly(acc, false)],
        data: vec![1],
    }];

    let msg = compile(&payer, &ixs, test_blockhash(), empty_config());
    assert_eq!(msg[2], 0, "no readonly signers: must be 0");
}

#[test]
fn test_num_readonly_signed_nonzero() {
    let payer = test_payer();
    let signer_ro = Pubkey::new_from_array([5u8; 32]);
    let program = Pubkey::new_from_array([10u8; 32]);

    let ixs = vec![CompiledInstruction {
        program_id: program,
        accounts: vec![AccountMeta::new_readonly(signer_ro, true)],
        data: vec![1],
    }];

    let msg = compile(&payer, &ixs, test_blockhash(), empty_config());
    assert_eq!(
        msg[2], 1,
        "one readonly signer: num_readonly_signed must be 1"
    );
}

// ---------------------------------------------------------------------------
// num_readonly_unsigned_accounts
// ---------------------------------------------------------------------------

#[test]
fn test_num_readonly_unsigned_program_only() {
    let payer = test_payer();
    let program = Pubkey::new_from_array([10u8; 32]);
    let acc = Pubkey::new_from_array([20u8; 32]);

    let ixs = vec![CompiledInstruction {
        program_id: program,
        accounts: vec![AccountMeta::new(acc, false)],
        data: vec![1],
    }];

    let msg = compile(&payer, &ixs, test_blockhash(), empty_config());
    // acc is writable non-signer (not readonly), program is readonly non-signer.
    assert_eq!(
        msg[3], 1,
        "one writable non-signer + program: num_readonly_unsigned must be 1"
    );
}

#[test]
fn test_num_readonly_unsigned_multiple() {
    let payer = test_payer();
    let program = Pubkey::new_from_array([10u8; 32]);
    let ro1 = Pubkey::new_from_array([41u8; 32]);
    let ro2 = Pubkey::new_from_array([42u8; 32]);

    let ixs = vec![CompiledInstruction {
        program_id: program,
        accounts: vec![
            AccountMeta::new_readonly(ro1, false),
            AccountMeta::new_readonly(ro2, false),
        ],
        data: vec![1],
    }];

    let msg = compile(&payer, &ixs, test_blockhash(), empty_config());
    assert_eq!(
        msg[3], 3,
        "program + 2 readonly: num_readonly_unsigned must be 3"
    );
}

// ---------------------------------------------------------------------------
// Account ordering
// ---------------------------------------------------------------------------

#[test]
fn test_account_ordering_payer_first() {
    let payer = test_payer();
    let program = Pubkey::new_from_array([10u8; 32]);
    let acc = Pubkey::new_from_array([20u8; 32]);

    let ixs = vec![CompiledInstruction {
        program_id: program,
        accounts: vec![AccountMeta::new_readonly(acc, false)],
        data: vec![1],
    }];

    let msg = compile(&payer, &ixs, test_blockhash(), empty_config());
    let num_addresses = msg[41] as usize;
    let addr_start = 42;
    let first_account = &msg[addr_start..addr_start + 32];
    assert_eq!(
        first_account,
        payer.as_ref(),
        "payer must be the first account key (index 0)"
    );
    assert!(num_addresses >= 2, "must have at least payer + program");
}

// ---------------------------------------------------------------------------
// Signer classification
// ---------------------------------------------------------------------------

#[test]
fn test_signer_classification_writable_signer_first() {
    let payer = test_payer();
    let signer_w = Pubkey::new_from_array([5u8; 32]);
    let program = Pubkey::new_from_array([10u8; 32]);

    let ixs = vec![CompiledInstruction {
        program_id: program,
        accounts: vec![AccountMeta::new(signer_w, true)],
        data: vec![1],
    }];

    let msg = compile(&payer, &ixs, test_blockhash(), empty_config());
    assert_eq!(
        msg[1], 2,
        "payer + writable signer: num_required_signatures must be 2"
    );
    assert_eq!(msg[2], 0, "no readonly signers");
}

// ---------------------------------------------------------------------------
// Writable/read-only classification
// ---------------------------------------------------------------------------

#[test]
fn test_writable_readonly_classification() {
    let payer = test_payer();
    let program = Pubkey::new_from_array([10u8; 32]);
    let acc_w = Pubkey::new_from_array([20u8; 32]);
    let acc_ro = Pubkey::new_from_array([30u8; 32]);

    let ixs = vec![CompiledInstruction {
        program_id: program,
        accounts: vec![
            AccountMeta::new(acc_w, false),
            AccountMeta::new_readonly(acc_ro, false),
        ],
        data: vec![1],
    }];

    let msg = compile(&payer, &ixs, test_blockhash(), empty_config());
    assert_eq!(msg[1], 1, "payer only: num_required_signatures");
    assert_eq!(msg[2], 0, "no readonly signed");
    assert_eq!(msg[3], 2, "program + acc_ro: num_readonly_unsigned");
}

// ---------------------------------------------------------------------------
// No duplicate account keys
// ---------------------------------------------------------------------------

#[test]
fn test_no_duplicate_account_keys() {
    let payer = test_payer();
    let program = Pubkey::new_from_array([10u8; 32]);
    let dup_acc = Pubkey::new_from_array([20u8; 32]);
    let other = Pubkey::new_from_array([30u8; 32]);

    let ixs = vec![
        CompiledInstruction {
            program_id: program,
            accounts: vec![
                AccountMeta::new(dup_acc, false),
                AccountMeta::new(dup_acc, false),
                AccountMeta::new_readonly(other, false),
            ],
            data: vec![1],
        },
        CompiledInstruction {
            program_id: program,
            accounts: vec![AccountMeta::new_readonly(dup_acc, false)],
            data: vec![2],
        },
    ];

    let msg = compile(&payer, &ixs, test_blockhash(), empty_config());
    let num_addresses = msg[41] as usize;
    let addr_start = 42;
    let mut seen = std::collections::HashSet::new();
    for i in 0..num_addresses {
        let ak = &msg[addr_start + i * 32..addr_start + (i + 1) * 32];
        assert!(
            seen.insert(ak.to_vec()),
            "duplicate account key at index {i}"
        );
    }
}

// ---------------------------------------------------------------------------
// Program ID handling
// ---------------------------------------------------------------------------

#[test]
fn test_program_id_in_account_keys() {
    let payer = test_payer();
    let program1 = Pubkey::new_from_array([10u8; 32]);
    let program2 = Pubkey::new_from_array([11u8; 32]);
    let data_acc = Pubkey::new_from_array([20u8; 32]);

    let ixs = vec![
        CompiledInstruction {
            program_id: program1,
            accounts: vec![
                AccountMeta::new_readonly(program2, false),
                AccountMeta::new(data_acc, false),
            ],
            data: vec![1],
        },
        CompiledInstruction {
            program_id: program2,
            accounts: vec![AccountMeta::new_readonly(program1, false)],
            data: vec![2],
        },
    ];

    let msg = compile(&payer, &ixs, test_blockhash(), empty_config());
    let num_addresses = msg[41] as usize;
    let addr_start = 42;
    let mut found_program1 = false;
    let mut found_program2 = false;
    for i in 0..num_addresses {
        let ak = &msg[addr_start + i * 32..addr_start + (i + 1) * 32];
        if ak == program1.as_ref() {
            found_program1 = true;
        }
        if ak == program2.as_ref() {
            found_program2 = true;
        }
    }
    assert!(found_program1, "program1 must be in account keys");
    assert!(found_program2, "program2 must be in account keys");
}

// ---------------------------------------------------------------------------
// Compiled instruction account indices
// ---------------------------------------------------------------------------

#[test]
fn test_compiled_instruction_account_indices_valid() {
    let payer = test_payer();
    let program = Pubkey::new_from_array([10u8; 32]);
    let acc_w = Pubkey::new_from_array([20u8; 32]);
    let acc_ro = Pubkey::new_from_array([30u8; 32]);

    let ixs = vec![CompiledInstruction {
        program_id: program,
        accounts: vec![
            AccountMeta::new(acc_w, false),
            AccountMeta::new_readonly(acc_ro, false),
        ],
        data: vec![1],
    }];

    let msg = compile(&payer, &ixs, test_blockhash(), empty_config());
    let num_addresses = msg[41] as usize;
    let addr_start = 42;
    let addr_end = addr_start + num_addresses * 32;
    let ix_header_start = addr_end;

    let num_acc = msg[ix_header_start + 1] as usize;
    assert_eq!(num_acc, 2);

    let payload_start = ix_header_start + 4;
    for i in 0..num_acc {
        let acc_idx = msg[payload_start + i] as usize;
        assert!(
            acc_idx < num_addresses,
            "instruction account index {acc_idx} must be < num_addresses {num_addresses}"
        );
    }
}

// ---------------------------------------------------------------------------
// Recent blockhash placement
// ---------------------------------------------------------------------------

#[test]
fn test_blockhash_placement_at_bytes_8_to_40() {
    let payer = test_payer();
    let blockhash = [7u8; 32];
    let config = empty_config();

    let program = Pubkey::new_from_array([10u8; 32]);
    let acc = Pubkey::new_from_array([20u8; 32]);

    let ixs = vec![CompiledInstruction {
        program_id: program,
        accounts: vec![AccountMeta::new_readonly(acc, false)],
        data: vec![1],
    }];

    let msg = compile(&payer, &ixs, blockhash, config);
    assert_eq!(
        &msg[8..40],
        &blockhash,
        "blockhash must be at bytes [8..40]"
    );
}

// ---------------------------------------------------------------------------
// Config mask correctness
// ---------------------------------------------------------------------------

#[test]
fn test_config_mask_empty_config_is_zero() {
    let payer = test_payer();
    let program = Pubkey::new_from_array([10u8; 32]);
    let acc = Pubkey::new_from_array([20u8; 32]);

    let ixs = vec![CompiledInstruction {
        program_id: program,
        accounts: vec![AccountMeta::new_readonly(acc, false)],
        data: vec![1],
    }];

    let msg = compile(&payer, &ixs, test_blockhash(), empty_config());
    assert_eq!(config_mask(&msg), 0, "empty config mask must be 0");
}

#[test]
fn test_config_mask_with_priority_fee_and_cu_limit() {
    let payer = test_payer();
    let program = Pubkey::new_from_array([10u8; 32]);
    let acc = Pubkey::new_from_array([20u8; 32]);

    let ixs = vec![CompiledInstruction {
        program_id: program,
        accounts: vec![AccountMeta::new_readonly(acc, false)],
        data: vec![1],
    }];

    let config = V1ConfigParams {
        priority_fee_lamports: Some(5000),
        compute_unit_limit: Some(300_000),
        loaded_accounts_data_size_limit: None,
        heap_size: None,
    };

    let msg = compile(&payer, &ixs, test_blockhash(), config);
    assert_eq!(
        config_mask(&msg),
        0x07,
        "config mask with priority fee + CU limit must be 0x07"
    );
}

// ---------------------------------------------------------------------------
// V1 version prefix
// ---------------------------------------------------------------------------

#[test]
fn test_v1_version_prefix_is_0x81() {
    let payer = test_payer();
    let program = Pubkey::new_from_array([10u8; 32]);
    let acc = Pubkey::new_from_array([20u8; 32]);

    let ixs = vec![CompiledInstruction {
        program_id: program,
        accounts: vec![AccountMeta::new_readonly(acc, false)],
        data: vec![1],
    }];

    let msg = compile(&payer, &ixs, test_blockhash(), empty_config());
    assert_eq!(msg[0], 0x81, "V1 version discriminator must be 0x81");
}

// ---------------------------------------------------------------------------
// Maximum account count (64)
// ---------------------------------------------------------------------------

#[test]
fn test_max_64_addresses() {
    let payer = test_payer();
    let program = Pubkey::new_from_array([10u8; 32]);

    let mut accounts = Vec::new();
    for i in 0..62u8 {
        let mut bytes = [0u8; 32];
        bytes[0] = i;
        accounts.push(AccountMeta::new(Pubkey::new_from_array(bytes), false));
    }
    accounts.push(AccountMeta::new_readonly(
        Pubkey::new_from_array([99u8; 32]),
        false,
    ));

    let ixs = vec![CompiledInstruction {
        program_id: program,
        accounts,
        data: vec![0],
    }];

    let result =
        AdaptiveTxCompiler::compile_v1_message(payer, &ixs, test_blockhash(), empty_config());
    if let Ok(msg) = result {
        let num_addresses = msg[41] as usize;
        assert!(num_addresses <= 64, "num_addresses must not exceed 64");
    }
}

// ---------------------------------------------------------------------------
// Test: empty instruction list accepted
// ---------------------------------------------------------------------------

#[test]
fn test_empty_instruction_list_accepted() {
    let payer = test_payer();
    let ixs: Vec<CompiledInstruction> = vec![];

    let result =
        AdaptiveTxCompiler::compile_v1_message(payer, &ixs, test_blockhash(), empty_config());
    assert!(result.is_ok(), "empty instruction list must be accepted");
}
