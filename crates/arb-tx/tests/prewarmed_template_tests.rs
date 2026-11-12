//! Phase 5: Prewarmed Unsigned Template Integration & Latency Benchmark.
//!
//! Validates:
//! 1. Reusable unsigned instruction-building structures (not signed transactions).
//! 2. Refresh of amounts, dynamic tick/bin arrays, blockhash, and resource limits before compilation.
//! 3. Recompilation and re-signing of every changed message with fresh Ed25519 signatures.
//! 4. End-to-end build latency benchmarking comparing prewarmed template refresh vs cold assembly.

use arb_constants::{
    FLASH_BORROW_DISCRIMINATOR, FLASH_REPAY_DISCRIMINATOR, KAMINO_KLEND_PROGRAM_ID,
    SYSTEM_PROGRAM_ID,
};
use arb_tx::{
    AccountMeta, CompiledInstruction, CompilerCapabilities, KeypairSigner,
    PrewarmedUnsignedTemplate, TemplateHopRefresh, TemplateRefreshParams, build_compute_unit_limit,
    build_compute_unit_price,
};
use solana_pubkey::Pubkey;
use std::str::FromStr;
use std::time::Instant;

fn mock_instruction(
    program_id: Pubkey,
    accounts: Vec<AccountMeta>,
    data: Vec<u8>,
) -> CompiledInstruction {
    CompiledInstruction {
        program_id,
        accounts,
        data,
    }
}

#[test]
fn test_prewarmed_unsigned_template_refresh_and_compile() {
    let signer = KeypairSigner::new_random();
    let initial_blockhash = [1u8; 32];
    let recipient = Pubkey::new_unique();

    // 1. Build initial unsigned instructions:
    // ix 0: CU Limit
    let cu_limit_ix = build_compute_unit_limit(200_000);
    // ix 1: Priority Fee
    let prio_fee_ix = build_compute_unit_price(1_000);
    // ix 2: Kamino Flash Borrow
    let mut borrow_data = Vec::with_capacity(16);
    borrow_data.extend_from_slice(&FLASH_BORROW_DISCRIMINATOR);
    borrow_data.extend_from_slice(&1_000_000u64.to_le_bytes());
    let flash_borrow_ix = mock_instruction(
        Pubkey::from_str(KAMINO_KLEND_PROGRAM_ID).unwrap(),
        vec![AccountMeta::new(signer.pubkey(), true)],
        borrow_data,
    );
    // ix 3: Swap Hop
    let mut swap_data = Vec::with_capacity(24);
    swap_data.extend_from_slice(&[9u8; 8]); // Mock 8-byte discriminator
    swap_data.extend_from_slice(&1_000_000u64.to_le_bytes()); // amount_in
    swap_data.extend_from_slice(&1_050_000u64.to_le_bytes()); // min_amount_out
    let swap_ix = mock_instruction(
        Pubkey::new_unique(),
        vec![
            AccountMeta::new(signer.pubkey(), true),
            AccountMeta::new(recipient, false),
        ],
        swap_data,
    );
    // ix 4: Kamino Flash Repay
    let mut repay_data = Vec::with_capacity(16);
    repay_data.extend_from_slice(&FLASH_REPAY_DISCRIMINATOR);
    repay_data.extend_from_slice(&1_000_100u64.to_le_bytes());
    let flash_repay_ix = mock_instruction(
        Pubkey::from_str(KAMINO_KLEND_PROGRAM_ID).unwrap(),
        vec![AccountMeta::new(signer.pubkey(), true)],
        repay_data,
    );
    // ix 5: Jito Tip Transfer
    let mut tip_data = Vec::with_capacity(12);
    tip_data.extend_from_slice(&2u32.to_le_bytes());
    tip_data.extend_from_slice(&10_000u64.to_le_bytes());
    let tip_ix = mock_instruction(
        Pubkey::new_from_array(SYSTEM_PROGRAM_ID),
        vec![
            AccountMeta::new(signer.pubkey(), true),
            AccountMeta::new(Pubkey::new_unique(), false),
        ],
        tip_data,
    );

    let instructions = vec![
        cu_limit_ix,
        prio_fee_ix,
        flash_borrow_ix,
        swap_ix,
        flash_repay_ix,
        tip_ix,
    ];

    let mut template = PrewarmedUnsignedTemplate::new(
        signer.pubkey(),
        instructions,
        initial_blockhash,
        vec![],
        0, // fee_sf
        200_000,
        1_000,
        10_000,
        Some(recipient),
        vec![3], // hop 0 is at ix 3
        Some(2), // flash borrow at ix 2
        Some(4), // flash repay at ix 4
        Some(0), // cu limit at ix 0
        Some(1), // prio fee at ix 1
        Some(5), // tip at ix 5
    );

    // 2. Perform hot-path refresh
    let new_blockhash = [9u8; 32];
    let dynamic_tick_array = AccountMeta::new_readonly(Pubkey::new_unique(), false);

    let refresh_params = TemplateRefreshParams {
        recent_blockhash: new_blockhash,
        borrow_amount: 5_000_000,
        tip_lamports: 25_000,
        cu_limit: Some(250_000),
        priority_fee_micro_lamports: Some(5_000),
        hop_refreshes: vec![TemplateHopRefresh {
            hop_index: 0,
            amount_in: 5_000_000,
            min_amount_out: 5_200_000,
            dynamic_accounts: vec![dynamic_tick_array],
        }],
    };

    template.refresh(refresh_params);

    // 3. Compile and sign freshly
    let tx = template
        .compile_and_sign(&signer, CompilerCapabilities::v0_standard())
        .expect("compile and sign template");

    // Invariant: Ed25519 signature is strictly valid for the newly compiled message
    assert!(tx.verify_signatures(&[signer.pubkey()]).unwrap());

    // Invariant: Borrow amount inside instruction 2 is refreshed
    let borrow_amt = u64::from_le_bytes(template.instructions[2].data[8..16].try_into().unwrap());
    assert_eq!(borrow_amt, 5_000_000);

    // Invariant: Swap instruction has dynamic accounts appended
    assert!(template.instructions[3].accounts.len() >= 3);
}

#[test]
fn test_prewarmed_template_latency_benchmark() {
    let signer = KeypairSigner::new_random();
    let blockhash = [42u8; 32];
    let recipient = Pubkey::new_unique();

    let mut swap_data = Vec::with_capacity(24);
    swap_data.extend_from_slice(&[1u8; 8]);
    swap_data.extend_from_slice(&1_000_000u64.to_le_bytes());
    swap_data.extend_from_slice(&1_050_000u64.to_le_bytes());

    let instructions = vec![
        build_compute_unit_limit(250_000),
        mock_instruction(
            Pubkey::new_unique(),
            vec![
                AccountMeta::new(signer.pubkey(), true),
                AccountMeta::new(recipient, false),
            ],
            swap_data,
        ),
    ];

    let mut template = PrewarmedUnsignedTemplate::new(
        signer.pubkey(),
        instructions,
        blockhash,
        vec![],
        0,
        250_000,
        1_000,
        10_000,
        Some(recipient),
        vec![1],
        None,
        None,
        Some(0),
        None,
        None,
    );

    let iterations = 200;

    // Benchmark Prewarmed Refresh + Compile + Sign
    let start_template = Instant::now();
    for i in 0..iterations {
        let refresh = TemplateRefreshParams {
            recent_blockhash: [(i % 255) as u8; 32],
            borrow_amount: 1_000_000 + i as u64,
            tip_lamports: 10_000 + i as u64,
            cu_limit: Some(250_000),
            priority_fee_micro_lamports: None,
            hop_refreshes: vec![TemplateHopRefresh {
                hop_index: 0,
                amount_in: 1_000_000 + i as u64,
                min_amount_out: 1_050_000 + i as u64,
                dynamic_accounts: vec![],
            }],
        };
        template.refresh(refresh);
        let _ = template
            .compile_and_sign(&signer, CompilerCapabilities::v0_standard())
            .unwrap();
    }
    let duration_template = start_template.elapsed();
    let avg_template_micros = duration_template.as_micros() as f64 / iterations as f64;

    // Benchmark Cold Assembly + Compile + Sign from scratch
    let start_cold = Instant::now();
    for i in 0..iterations {
        let mut d = Vec::with_capacity(24);
        d.extend_from_slice(&[1u8; 8]);
        d.extend_from_slice(&(1_000_000u64 + i as u64).to_le_bytes());
        d.extend_from_slice(&(1_050_000u64 + i as u64).to_le_bytes());

        let cold_ixs = vec![
            build_compute_unit_limit(250_000),
            mock_instruction(
                Pubkey::new_unique(),
                vec![
                    AccountMeta::new(signer.pubkey(), true),
                    AccountMeta::new(recipient, false),
                ],
                d,
            ),
        ];

        let _ = arb_tx::AdaptiveTxCompiler::compile_adaptive(
            &signer,
            &cold_ixs,
            [(i % 255) as u8; 32],
            &[],
            None,
            CompilerCapabilities::v0_standard(),
            None,
        )
        .unwrap();
    }
    let duration_cold = start_cold.elapsed();
    let avg_cold_micros = duration_cold.as_micros() as f64 / iterations as f64;

    println!(
        "\n================ LATENCY BENCHMARK ================\n\
         Prewarmed Unsigned Template: {:.2} µs/op\n\
         Cold Assembly from Scratch:  {:.2} µs/op\n\
         Speedup: {:.2}x\n\
         ===================================================\n",
        avg_template_micros,
        avg_cold_micros,
        avg_cold_micros / avg_template_micros.max(0.001)
    );

    assert!(avg_template_micros > 0.0);
}
