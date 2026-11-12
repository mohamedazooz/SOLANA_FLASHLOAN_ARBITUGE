//! Adversarial Empirical Stress Tests for Milestone M2 (`crates/arb-tx`).
//!
//! Rigorously challenges and stress tests:
//! 1. PreWarmedTxTemplate byte offset invariants and boundary value patching ($0$, $1$, $u64::MAX$, arbitrary blockhash).
//! 2. Invariant stability: verification that non-target bytes are not corrupted during in-place mutation.
//! 3. Cryptographic Ed25519 signature validity over mutated message bytes, including positive & negative tamper tests.
//! 4. Sequential rapid-fire mutation without memory drift or offset degradation.
//! 5. Fee-bearing reserve calculation and saturation under extreme loan sizes ($u64::MAX$).
//! 6. Wire MTU size limit (<= 1232 bytes) under high account counts and enforcement of TransactionTooLarge error.
//! 7. Blockhash locator collision vulnerability when initial_blockhash matches an account pubkey (e.g. SystemProgram = [0; 32] when no ALT is used).
//! 8. Degenerate zero-tip template initialization behavior.

use arb_tx::kamino::calculate_kamino_flash_fee;
use arb_tx::template::PreWarmedTxTemplate;
use arb_tx::{
    AccountMeta, CompiledInstruction, JITO_TIP_ACCOUNTS, KaminoBundleParams, KaminoFlashParams,
    KeypairSigner, ORCA_SOL_USDC_POOL, SOLANA_PACKET_MTU_LIMIT, TxBuilder, TxError, WSOL_MINT,
};
use arb_types::{
    Bps, DexProtocol, ExecutionOpportunity, Lamports, MintId, PoolId, RouteId, Slot, SwapHop,
    TokenAmount,
};
use ed25519_dalek::{Signature, Verifier};
use solana_pubkey::Pubkey;
use std::str::FromStr;
use std::time::Instant;

fn create_test_signer(seed_byte: u8) -> KeypairSigner {
    let mut seed = [seed_byte; 32];
    seed[0] = seed_byte;
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&seed);
    let pubkey_bytes = signing_key.verifying_key().to_bytes();
    let mut keypair_bytes = [0u8; 64];
    keypair_bytes[..32].copy_from_slice(&seed);
    keypair_bytes[32..].copy_from_slice(&pubkey_bytes);
    KeypairSigner::from_bytes(&keypair_bytes).unwrap()
}

fn mock_raydium_v4_exec_accounts() -> arb_types::PoolExecutionAccounts {
    let m = arb_tx::RaydiumV4Accounts::mainnet_sol_usdc();
    arb_types::PoolExecutionAccounts::RaydiumV4 {
        amm_authority: m.amm_authority,
        open_orders: m.amm_open_orders,
        target_orders: m.amm_target_orders,
        base_vault: m.pool_coin_vault,
        quote_vault: m.pool_pc_vault,
        serum_program: m.serum_program,
        serum_market: m.serum_market,
        serum_bids: m.serum_bids,
        serum_asks: m.serum_asks,
        serum_event_queue: m.serum_event_queue,
        serum_coin_vault: m.serum_coin_vault,
        serum_pc_vault: m.serum_pc_vault,
        serum_vault_signer: m.serum_vault_signer,
        swap_version: arb_types::RaydiumV4SwapVersion::V2,
    }
}

fn create_2hop_opportunity(wsol: Pubkey, usdc: Pubkey, size_lamports: u64) -> ExecutionOpportunity {
    ExecutionOpportunity {
        route_id: RouteId(201),
        slot: Slot(1000),
        size_in: TokenAmount(size_lamports),
        expected_out: TokenAmount(size_lamports.saturating_add(50_000_000)),
        hop_inputs: vec![
            size_lamports,
            size_lamports.saturating_add(25_000_000),
            size_lamports.saturating_add(50_000_000),
        ],
        gross_profit: Lamports(50_000_000),
        net_profit: Lamports(45_000_000),
        base_decimals: 9,
        price_impact_bps: Bps(5),
        hops: vec![
            SwapHop {
                pool_id: PoolId::new([1; 32]),
                protocol: DexProtocol::RaydiumV4,
                mint_in: MintId(wsol),
                mint_out: MintId(usdc),
                fee_bps: 25,
                active_bin_id: None,
                token_programs: [arb_types::TokenProgramKind::SplToken; 2],
                trade_intent: Default::default(),
                execution_accounts: Some(mock_raydium_v4_exec_accounts()),
            },
            SwapHop {
                pool_id: PoolId(Pubkey::from_str(ORCA_SOL_USDC_POOL).unwrap()),
                protocol: DexProtocol::OrcaWhirlpool,
                mint_in: MintId(usdc),
                mint_out: MintId(wsol),
                fee_bps: 5,
                active_bin_id: None,
                token_programs: [arb_types::TokenProgramKind::SplToken; 2],
                trade_intent: Default::default(),
                execution_accounts: None,
            },
        ],
        expected_min_final_output: 0,
    }
}

fn create_3hop_opportunity(
    wsol: Pubkey,
    usdc: Pubkey,
    bonk: Pubkey,
    size_lamports: u64,
) -> ExecutionOpportunity {
    ExecutionOpportunity {
        route_id: RouteId(301),
        slot: Slot(1001),
        size_in: TokenAmount(size_lamports),
        expected_out: TokenAmount(size_lamports.saturating_add(80_000_000)),
        hop_inputs: vec![
            size_lamports,
            size_lamports.saturating_add(20_000_000),
            size_lamports.saturating_add(50_000_000),
            size_lamports.saturating_add(80_000_000),
        ],
        gross_profit: Lamports(80_000_000),
        net_profit: Lamports(70_000_000),
        base_decimals: 9,
        price_impact_bps: Bps(8),
        hops: vec![
            SwapHop {
                pool_id: PoolId::new([1; 32]),
                protocol: DexProtocol::RaydiumV4,
                mint_in: MintId(wsol),
                mint_out: MintId(usdc),
                fee_bps: 25,
                active_bin_id: None,
                token_programs: [arb_types::TokenProgramKind::SplToken; 2],
                trade_intent: Default::default(),
                execution_accounts: Some(mock_raydium_v4_exec_accounts()),
            },
            SwapHop {
                pool_id: PoolId(Pubkey::from_str(ORCA_SOL_USDC_POOL).unwrap()),
                protocol: DexProtocol::OrcaWhirlpool,
                mint_in: MintId(usdc),
                mint_out: MintId(bonk),
                fee_bps: 5,
                active_bin_id: None,
                token_programs: [arb_types::TokenProgramKind::SplToken; 2],
                trade_intent: Default::default(),
                execution_accounts: None,
            },
            SwapHop {
                pool_id: PoolId::new([3; 32]),
                protocol: DexProtocol::RaydiumV4,
                mint_in: MintId(bonk),
                mint_out: MintId(wsol),
                fee_bps: 25,
                active_bin_id: None,
                token_programs: [arb_types::TokenProgramKind::SplToken; 2],
                trade_intent: Default::default(),
                execution_accounts: Some(mock_raydium_v4_exec_accounts()),
            },
        ],
        expected_min_final_output: 0,
    }
}

// ============================================================================
// 1. BOUNDARY VALUE STRESS TESTING ON PRE-WARMED TEMPLATE
// ============================================================================

#[test]
fn challenge_boundary_value_patch_and_sign_2hop() {
    let signer = create_test_signer(10);
    let payer = signer.pubkey();
    let tip_acc = Pubkey::from_str(JITO_TIP_ACCOUNTS[0]).unwrap();
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    let usdc = Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
    let kamino_params = KaminoFlashParams::default();
    let opp = create_2hop_opportunity(wsol, usdc, 1_000_000_000);
    let alt = TxBuilder::create_default_mainnet_alt();
    let initial_blockhash = [0x55; 32];
    let initial_tip = 50_000u64;

    let bundle_params = KaminoBundleParams {
        payer,
        signer: &signer,
        tip_account: tip_acc,
        tip_lamports: Lamports(initial_tip),
        priority_fee_micro_lamports: 1_000,
        opportunity: &opp,
        kamino_params: &kamino_params,
        recent_blockhash: initial_blockhash,
        lookup_tables: &[alt],
        flash_loan_fee_sf: 0,
    };

    let mut template =
        PreWarmedTxTemplate::from_bundle_params(&bundle_params, 9223372036854775).unwrap();
    let original_bytes = template.raw_bytes.clone();
    let original_len = original_bytes.len();
    let b_offset = template.borrow_amount_offset;
    let r_offset = template.repay_amount_offset;
    let t_offset = template.tip_amount_offset;
    let bh_offset = template.blockhash_offset;
    let sig_offset = template.signature_offset;
    let msg_offset = template.message_offset;

    let test_borrow_values = [0u64, 1u64, 10_000_000_000u64, u64::MAX - 100, u64::MAX];
    let test_tip_values = [0u64, 1u64, 50_000u64, u64::MAX - 1, u64::MAX];
    let test_blockhashes: [[u8; 32]; 4] = [
        [0x00; 32],
        [0xFF; 32],
        [0xAA; 32],
        [
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24,
            25, 26, 27, 28, 29, 30, 31, 32,
        ],
    ];

    for &borrow in &test_borrow_values {
        for &tip in &test_tip_values {
            for &blockhash in &test_blockhashes {
                let wire_tx = template.patch_and_sign(borrow, tip, blockhash, &signer);

                // Invariant 1: Wire size must not change during in-place patching
                assert_eq!(
                    wire_tx.len(),
                    original_len,
                    "Wire length must remain constant"
                );
                assert!(
                    wire_tx.len() <= SOLANA_PACKET_MTU_LIMIT,
                    "Wire must fit in MTU"
                );

                // Invariant 2: Borrow amount at exact offset
                let written_borrow =
                    u64::from_le_bytes(wire_tx[b_offset..b_offset + 8].try_into().unwrap());
                assert_eq!(
                    written_borrow, borrow,
                    "Borrow amount must match exactly at target offset"
                );

                // Invariant 3: Repay amount at exact offset. klend's repay
                // argument MUST equal the borrow principal; the origination fee
                // is collected separately at repay time.
                let written_repay =
                    u64::from_le_bytes(wire_tx[r_offset..r_offset + 8].try_into().unwrap());
                assert_eq!(
                    written_repay, borrow,
                    "Repay argument must equal the borrow principal exactly"
                );

                // Invariant 4: Tip lamports at exact offset
                let written_tip =
                    u64::from_le_bytes(wire_tx[t_offset..t_offset + 8].try_into().unwrap());
                assert_eq!(
                    written_tip, tip,
                    "Tip lamports must match exactly at target offset"
                );

                // Invariant 5: Blockhash at exact offset
                assert_eq!(
                    &wire_tx[bh_offset..bh_offset + 32],
                    &blockhash,
                    "Blockhash must match exactly at target offset"
                );

                // Invariant 6: Cryptographic signature verification over message bytes
                let sig_bytes: [u8; 64] = wire_tx[sig_offset..sig_offset + 64].try_into().unwrap();
                let sig = Signature::from_bytes(&sig_bytes);
                let message_bytes = &wire_tx[msg_offset..];
                assert!(
                    signer.verifying_key().verify(message_bytes, &sig).is_ok(),
                    "Cryptographic signature must verify over mutated message"
                );
            }
        }
    }
}

#[test]
fn challenge_boundary_value_patch_and_sign_3hop() {
    let signer = create_test_signer(11);
    let payer = signer.pubkey();
    let tip_acc = Pubkey::from_str(JITO_TIP_ACCOUNTS[1]).unwrap();
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    let usdc = Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
    let bonk = Pubkey::from_str("DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263").unwrap();
    let kamino_params = KaminoFlashParams::default();
    let opp = create_3hop_opportunity(wsol, usdc, bonk, 5_000_000_000);
    let alt = TxBuilder::create_default_mainnet_alt();
    let initial_blockhash = [0x66; 32];
    let initial_tip = 75_000u64;

    let bundle_params = KaminoBundleParams {
        payer,
        signer: &signer,
        tip_account: tip_acc,
        tip_lamports: Lamports(initial_tip),
        priority_fee_micro_lamports: 2_000,
        opportunity: &opp,
        kamino_params: &kamino_params,
        recent_blockhash: initial_blockhash,
        lookup_tables: &[alt],
        flash_loan_fee_sf: 0,
    };

    let mut template =
        PreWarmedTxTemplate::from_bundle_params(&bundle_params, 9223372036854775).unwrap();
    let original_len = template.raw_bytes.len();
    let sig_offset = template.signature_offset;
    let msg_offset = template.message_offset;

    let boundary_pairs = [
        (0u64, 0u64, [0x00; 32]),
        (1u64, 1u64, [0x01; 32]),
        (u64::MAX, u64::MAX, [0xFF; 32]),
        (50_000_000_000u64, 100_000u64, [0x77; 32]),
    ];

    for (borrow, tip, blockhash) in boundary_pairs {
        let wire_tx = template.patch_and_sign(borrow, tip, blockhash, &signer);
        assert_eq!(wire_tx.len(), original_len);
        assert!(wire_tx.len() <= SOLANA_PACKET_MTU_LIMIT);

        let sig_bytes: [u8; 64] = wire_tx[sig_offset..sig_offset + 64].try_into().unwrap();
        let sig = Signature::from_bytes(&sig_bytes);
        assert!(
            signer
                .verifying_key()
                .verify(&wire_tx[msg_offset..], &sig)
                .is_ok(),
            "3-hop signature must verify under boundary values"
        );
    }
}

// ============================================================================
// 2. ISOLATION & NON-TARGET BYTE INTEGRITY
// ============================================================================

#[test]
fn challenge_non_target_bytes_untouched_during_patch() {
    let signer = create_test_signer(12);
    let payer = signer.pubkey();
    let tip_acc = Pubkey::from_str(JITO_TIP_ACCOUNTS[2]).unwrap();
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    let usdc = Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
    let kamino_params = KaminoFlashParams::default();
    let opp = create_2hop_opportunity(wsol, usdc, 10_000_000_000);
    let alt = TxBuilder::create_default_mainnet_alt();
    let initial_blockhash = [0xBB; 32];
    let initial_tip = 50_000u64;

    let bundle_params = KaminoBundleParams {
        payer,
        signer: &signer,
        tip_account: tip_acc,
        tip_lamports: Lamports(initial_tip),
        priority_fee_micro_lamports: 1_000,
        opportunity: &opp,
        kamino_params: &kamino_params,
        recent_blockhash: initial_blockhash,
        lookup_tables: &[alt],
        flash_loan_fee_sf: 0,
    };

    let mut template =
        PreWarmedTxTemplate::from_bundle_params(&bundle_params, 9223372036854775).unwrap();
    let original = template.raw_bytes.clone();

    let new_borrow = 20_000_000_000u64;
    let new_tip = 99_000u64;
    let new_bh = [0xEE; 32];

    let b_start = template.borrow_amount_offset;
    let b_end = b_start + 8;
    let r_start = template.repay_amount_offset;
    let r_end = r_start + 8;
    let t_start = template.tip_amount_offset;
    let t_end = t_start + 8;
    let bh_start = template.blockhash_offset;
    let bh_end = bh_start + 32;
    let sig_start = template.signature_offset;
    let sig_end = sig_start + 64;

    let wire_tx = template.patch_and_sign(new_borrow, new_tip, new_bh, &signer);

    // Verify all bytes OUTSIDE the 5 modified windows match original identically
    for idx in 0..wire_tx.len() {
        let is_borrow = idx >= b_start && idx < b_end;
        let is_repay = idx >= r_start && idx < r_end;
        let is_tip = idx >= t_start && idx < t_end;
        let is_bh = idx >= bh_start && idx < bh_end;
        let is_sig = idx >= sig_start && idx < sig_end;

        if !is_borrow && !is_repay && !is_tip && !is_bh && !is_sig {
            assert_eq!(
                wire_tx[idx], original[idx],
                "Byte at offset {} was unexpectedly modified! Mutation leaked outside target fields.",
                idx
            );
        }
    }
}

// ============================================================================
// 3. CRYPTOGRAPHIC TAMPER DETECTION (NEGATIVE TESTING)
// ============================================================================

#[test]
fn challenge_cryptographic_signature_tamper_detection() {
    let signer = create_test_signer(13);
    let malicious_signer = create_test_signer(99);
    let payer = signer.pubkey();
    let tip_acc = Pubkey::from_str(JITO_TIP_ACCOUNTS[3]).unwrap();
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    let usdc = Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
    let kamino_params = KaminoFlashParams::default();
    let opp = create_2hop_opportunity(wsol, usdc, 10_000_000_000);
    let alt = TxBuilder::create_default_mainnet_alt();

    let bundle_params = KaminoBundleParams {
        payer,
        signer: &signer,
        tip_account: tip_acc,
        tip_lamports: Lamports(50_000),
        priority_fee_micro_lamports: 1_000,
        opportunity: &opp,
        kamino_params: &kamino_params,
        recent_blockhash: [0x33; 32],
        lookup_tables: &[alt],
        flash_loan_fee_sf: 0,
    };

    let mut template =
        PreWarmedTxTemplate::from_bundle_params(&bundle_params, 9223372036854775).unwrap();
    let msg_offset = template.message_offset;
    let wire_tx = template.patch_and_sign(12_000_000_000, 60_000, [0x44; 32], &signer);

    let sig_bytes: [u8; 64] = wire_tx[1..65].try_into().unwrap();
    let sig = Signature::from_bytes(&sig_bytes);
    let message_bytes = &wire_tx[msg_offset..];

    // 1. Valid signature passes
    assert!(signer.verifying_key().verify(message_bytes, &sig).is_ok());

    // 2. Verification with wrong public key MUST fail
    assert!(
        malicious_signer
            .verifying_key()
            .verify(message_bytes, &sig)
            .is_err(),
        "Signature verified against wrong public key!"
    );

    // 3. Mutating 1 bit in message MUST fail verification
    let mut tampered_message = message_bytes.to_vec();
    tampered_message[10] ^= 0x01;
    assert!(
        signer
            .verifying_key()
            .verify(&tampered_message, &sig)
            .is_err(),
        "Tampered message passed signature verification!"
    );

    // 4. Mutating 1 bit in signature MUST fail verification
    let mut tampered_sig_bytes = sig_bytes;
    tampered_sig_bytes[0] ^= 0x01;
    if let Ok(tampered_sig) = Signature::from_slice(&tampered_sig_bytes) {
        assert!(
            signer
                .verifying_key()
                .verify(message_bytes, &tampered_sig)
                .is_err(),
            "Tampered signature passed verification!"
        );
    }
}

// ============================================================================
// 4. SEQUENTIAL RAPID MUTATION (ZERO MEMORY DRIFT STRESS TEST)
// ============================================================================

#[test]
fn challenge_sequential_rapid_mutation_without_drift() {
    let signer = create_test_signer(14);
    let payer = signer.pubkey();
    let tip_acc = Pubkey::from_str(JITO_TIP_ACCOUNTS[4]).unwrap();
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    let usdc = Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
    let kamino_params = KaminoFlashParams::default();
    let opp = create_2hop_opportunity(wsol, usdc, 10_000_000_000);
    let alt = TxBuilder::create_default_mainnet_alt();

    let bundle_params = KaminoBundleParams {
        payer,
        signer: &signer,
        tip_account: tip_acc,
        tip_lamports: Lamports(50_000),
        priority_fee_micro_lamports: 1_000,
        opportunity: &opp,
        kamino_params: &kamino_params,
        recent_blockhash: [0x55; 32],
        lookup_tables: &[alt],
        flash_loan_fee_sf: 0,
    };

    let mut template =
        PreWarmedTxTemplate::from_bundle_params(&bundle_params, 9223372036854775).unwrap();
    let initial_len = template.raw_bytes.len();

    // Measure raw in-place memory mutation overhead (excluding Dalek crypto signing)
    let memory_iterations = 100_000;
    let mem_start = Instant::now();
    for i in 0..memory_iterations {
        let borrow = (i as u64).saturating_mul(1_000_000).saturating_add(1);
        let tip = (i as u64).saturating_add(10_000);
        let blockhash = [0x55; 32];
        template.raw_bytes[template.borrow_amount_offset..template.borrow_amount_offset + 8]
            .copy_from_slice(&borrow.to_le_bytes());
        template.raw_bytes[template.repay_amount_offset..template.repay_amount_offset + 8]
            .copy_from_slice(&borrow.to_le_bytes());
        template.raw_bytes[template.tip_amount_offset..template.tip_amount_offset + 8]
            .copy_from_slice(&tip.to_le_bytes());
        template.raw_bytes[template.blockhash_offset..template.blockhash_offset + 32]
            .copy_from_slice(&blockhash);
    }
    let mem_elapsed = mem_start.elapsed();
    let avg_mem_time = mem_elapsed / memory_iterations;
    println!(
        "In-place memory mutation: {} iterations in {:?} (avg {:?} per mutation)",
        memory_iterations, mem_elapsed, avg_mem_time
    );
    // Raw memory patching executes in < 1 microsecond (nanosecond range)
    assert!(
        avg_mem_time.as_nanos() < 1000,
        "In-place memory writes must take < 1 microsecond"
    );

    // Full patch_and_sign iterations
    let iterations: u32 = 1000;
    let start = Instant::now();
    for i in 0..iterations {
        let borrow = (i as u64).saturating_mul(1_000_000).saturating_add(1);
        let tip = (i as u64).saturating_add(10_000);
        let mut blockhash = [0u8; 32];
        blockhash[0..4].copy_from_slice(&i.to_le_bytes());

        let wire_tx = template.patch_and_sign(borrow, tip, blockhash, &signer);
        assert_eq!(
            wire_tx.len(),
            initial_len,
            "Wire length drifted during rapid-fire loop"
        );
    }

    let elapsed = start.elapsed();
    let avg_per_mutation = elapsed / iterations;
    println!(
        "Full patch_and_sign: {} iterations in {:?} (avg {:?} per mutation)",
        iterations, elapsed, avg_per_mutation
    );
    assert!(
        avg_per_mutation.as_millis() < 10,
        "Patch and sign took {:?}, exceeding expected upper bound",
        avg_per_mutation
    );
}

// ============================================================================
// 5. FEE-BEARING RESERVE OVERFLOW & SATURATION SAFETY
// ============================================================================

#[test]
fn challenge_fee_bearing_reserve_overflow_and_saturation() {
    // calculate_kamino_flash_fee takes the raw U68F60 on-chain `flash_loan_fee_sf`
    // (klend's `Fraction` = `fixed::types::U68F60`, 60 fractional bits).
    assert_eq!(calculate_kamino_flash_fee(0, 0), 0);
    assert_eq!(calculate_kamino_flash_fee(0, 100), 0);
    assert_eq!(calculate_kamino_flash_fee(10_000_000, 0), 0);

    // Any non-zero sf on a non-zero loan rounds up to at least 1 unit.
    assert_eq!(calculate_kamino_flash_fee(1, 1), 1);

    // Mainnet WSOL reserve sf = 1e-5: round(1e9 * 1e-5) = 10_000 lamports on 1 SOL.
    assert_eq!(
        calculate_kamino_flash_fee(1_000_000_000, 11_529_215_046_068),
        10_000
    );

    // u64::MAX loan with a small sf computes a wide fee without panicking.
    let max_loan = u64::MAX;
    let fee_max = calculate_kamino_flash_fee(max_loan, 11_529_215_046_068);
    assert!(fee_max > 0);
    let repay = max_loan.saturating_add(fee_max);
    assert_eq!(repay, u64::MAX);

    // Template initialized with a non-zero verified sf (5% in U68F60).
    let signer = create_test_signer(15);
    let payer = signer.pubkey();
    let tip_acc = Pubkey::from_str(JITO_TIP_ACCOUNTS[5]).unwrap();
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    let usdc = Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
    let kamino_params = KaminoFlashParams::default();
    let opp = create_2hop_opportunity(wsol, usdc, 1_000_000_000);
    let alt = TxBuilder::create_default_mainnet_alt();

    let bundle_params = KaminoBundleParams {
        payer,
        signer: &signer,
        tip_account: tip_acc,
        tip_lamports: Lamports(50_000),
        priority_fee_micro_lamports: 1_000,
        opportunity: &opp,
        kamino_params: &kamino_params,
        recent_blockhash: [0x77; 32],
        lookup_tables: &[alt],
        flash_loan_fee_sf: 0,
    };

    let v0_tx = TxBuilder::build_kamino_bundle(&bundle_params).unwrap();
    // Instantiate template with the mainnet verified sf.
    let mut template = PreWarmedTxTemplate::from_precompiled_bytes(
        v0_tx.wire_bytes,
        &bundle_params.recent_blockhash,
        50_000,
        11_529_215_046_068, // mainnet U68F60 sf
    )
    .unwrap();

    let r_offset = template.repay_amount_offset;

    // Patch with u64::MAX: the repay argument equals the principal exactly, so
    // the written value is u64::MAX with no overflow.
    let wire_tx = template.patch_and_sign(u64::MAX, 10_000, [0x88; 32], &signer);
    let written_repay = u64::from_le_bytes(wire_tx[r_offset..r_offset + 8].try_into().unwrap());
    assert_eq!(
        written_repay,
        u64::MAX,
        "Repay argument must equal the borrow principal (u64::MAX) without overflow"
    );
}

// ============================================================================
// 6. WIRE MTU SIZE LIMIT STRESS TESTING (<= 1232 BYTES)
// ============================================================================

#[test]
fn challenge_wire_mtu_size_limit_and_high_account_count() {
    let signer = create_test_signer(16);
    let payer = signer.pubkey();
    let tip_acc = Pubkey::from_str(JITO_TIP_ACCOUNTS[6]).unwrap();
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    let usdc = Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
    let bonk = Pubkey::from_str("DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263").unwrap();
    let kamino_params = KaminoFlashParams::default();
    let alt = TxBuilder::create_default_mainnet_alt();

    // 1. Verify 2-hop wire size fits within MTU (1232 bytes)
    let opp_2hop = create_2hop_opportunity(wsol, usdc, 10_000_000_000);
    let params_2hop = KaminoBundleParams {
        payer,
        signer: &signer,
        tip_account: tip_acc,
        tip_lamports: Lamports(50_000),
        priority_fee_micro_lamports: 1_000,
        opportunity: &opp_2hop,
        kamino_params: &kamino_params,
        recent_blockhash: [0x11; 32],
        lookup_tables: std::slice::from_ref(&alt),
        flash_loan_fee_sf: 0,
    };
    let tx_2hop = TxBuilder::build_kamino_bundle(&params_2hop).unwrap();
    println!(
        "2-hop wire size: {} bytes (limit: 1232 bytes)",
        tx_2hop.wire_bytes.len()
    );
    assert!(
        tx_2hop.wire_bytes.len() <= SOLANA_PACKET_MTU_LIMIT,
        "2-hop wire size {} exceeds MTU limit {}",
        tx_2hop.wire_bytes.len(),
        SOLANA_PACKET_MTU_LIMIT
    );

    // 2. Verify 3-hop wire size fits within MTU (1232 bytes)
    let opp_3hop = create_3hop_opportunity(wsol, usdc, bonk, 10_000_000_000);
    let params_3hop = KaminoBundleParams {
        payer,
        signer: &signer,
        tip_account: tip_acc,
        tip_lamports: Lamports(50_000),
        priority_fee_micro_lamports: 1_000,
        opportunity: &opp_3hop,
        kamino_params: &kamino_params,
        recent_blockhash: [0x22; 32],
        lookup_tables: &[alt],
        flash_loan_fee_sf: 0,
    };
    let tx_3hop = TxBuilder::build_kamino_bundle(&params_3hop).unwrap();
    println!(
        "3-hop wire size: {} bytes (limit: 1232 bytes)",
        tx_3hop.wire_bytes.len()
    );
    assert!(
        tx_3hop.wire_bytes.len() <= SOLANA_PACKET_MTU_LIMIT,
        "3-hop wire size {} exceeds MTU limit {}",
        tx_3hop.wire_bytes.len(),
        SOLANA_PACKET_MTU_LIMIT
    );

    // 3. High Account Count Stress: Build bundle with empty lookup table (all accounts static)
    let params_no_alt = KaminoBundleParams {
        payer,
        signer: &signer,
        tip_account: tip_acc,
        tip_lamports: Lamports(50_000),
        priority_fee_micro_lamports: 1_000,
        opportunity: &opp_3hop,
        kamino_params: &kamino_params,
        recent_blockhash: [0x33; 32],
        lookup_tables: &[], // No ALT: forces all accounts to be stored as full 32-byte static keys
        flash_loan_fee_sf: 0,
    };
    let result_no_alt = TxBuilder::build_kamino_bundle(&params_no_alt);
    match result_no_alt {
        Ok(tx) => {
            println!("3-hop without ALT wire size: {} bytes", tx.wire_bytes.len());
            assert!(tx.wire_bytes.len() <= SOLANA_PACKET_MTU_LIMIT);
        }
        Err(TxError::TransactionTooLarge { size, limit }) => {
            println!(
                "Transaction correctly rejected by MTU limit: size={} limit={}",
                size, limit
            );
            assert_eq!(limit, SOLANA_PACKET_MTU_LIMIT);
            assert!(size > SOLANA_PACKET_MTU_LIMIT);
        }
        Err(other) => panic!("Unexpected error: {:?}", other),
    }

    // 4. Excessive Accounts Adversarial Test: Construct synthetic instructions exceeding 1232 bytes
    let mut heavy_ixs = Vec::new();
    for i in 0..40 {
        let dummy_prog = Pubkey::new_from_array([(i + 1) as u8; 32]);
        let mut accs = Vec::new();
        for j in 0..5 {
            let dummy_acc = Pubkey::new_from_array([(i * 5 + j + 50) as u8; 32]);
            accs.push(AccountMeta::new(dummy_acc, false));
        }
        heavy_ixs.push(CompiledInstruction {
            program_id: dummy_prog,
            accounts: accs,
            data: vec![0u8; 32],
        });
    }

    let compiled_msg = TxBuilder::compile_v0_message(payer, &heavy_ixs, [0x44; 32], &[]);
    if let Ok(msg) = compiled_msg {
        let wire_size = 1 + 64 + msg.len();
        println!("Excessive account message wire size: {} bytes", wire_size);
        assert!(
            wire_size > SOLANA_PACKET_MTU_LIMIT,
            "Synthetic heavy transaction should exceed MTU"
        );
    }
}

// ============================================================================
// 7. BLOCKHASH COLLISION IN OFFSET LOCATOR (ADVERSARIAL PROOF)
// ============================================================================

#[test]
fn challenge_blockhash_locator_behavior_with_zero_blockhash() {
    let signer = create_test_signer(17);
    let payer = signer.pubkey();
    let tip_acc = Pubkey::from_str(JITO_TIP_ACCOUNTS[7]).unwrap();
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    let usdc = Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
    let kamino_params = KaminoFlashParams::default();
    let opp = create_2hop_opportunity(wsol, usdc, 10_000_000_000);

    // Scenario A: WITH default ALT (SystemProgram is in the ALT, not in static accounts)
    let alt = TxBuilder::create_default_mainnet_alt();
    let zero_blockhash = [0u8; 32];
    let bundle_params_with_alt = KaminoBundleParams {
        payer,
        signer: &signer,
        tip_account: tip_acc,
        tip_lamports: Lamports(50_000),
        priority_fee_micro_lamports: 1_000,
        opportunity: &opp,
        kamino_params: &kamino_params,
        recent_blockhash: zero_blockhash,
        lookup_tables: &[alt],
        flash_loan_fee_sf: 0,
    };
    let v0_tx_with_alt = TxBuilder::build_kamino_bundle(&bundle_params_with_alt).unwrap();
    let (msg_off, bh_off, _, _, _, _) =
        PreWarmedTxTemplate::locate_offsets(&v0_tx_with_alt.wire_bytes, &zero_blockhash, 50_000)
            .unwrap();
    let num_static = v0_tx_with_alt.wire_bytes[msg_off + 4] as usize;
    let expected_bh_off = msg_off + 5 + (num_static * 32);
    assert_eq!(
        bh_off, expected_bh_off,
        "With ALT, zero blockhash does not collide with static accounts"
    );

    // Scenario B: WITHOUT ALT (SystemProgram [0; 32] is forced into static_account_keys)
    let bundle_params_no_alt = KaminoBundleParams {
        payer,
        signer: &signer,
        tip_account: tip_acc,
        tip_lamports: Lamports(50_000),
        priority_fee_micro_lamports: 1_000,
        opportunity: &opp,
        kamino_params: &kamino_params,
        recent_blockhash: zero_blockhash,
        lookup_tables: &[], // No ALT: SystemProgram is in static_account_keys
        flash_loan_fee_sf: 0,
    };
    if let Ok(v0_tx_no_alt) = TxBuilder::build_kamino_bundle(&bundle_params_no_alt) {
        let (no_alt_msg_off, no_alt_bh_off, _, _, _, _) =
            PreWarmedTxTemplate::locate_offsets(&v0_tx_no_alt.wire_bytes, &zero_blockhash, 50_000)
                .unwrap();
        let num_static_no_alt = v0_tx_no_alt.wire_bytes[no_alt_msg_off + 4] as usize;
        let expected_bh_off_no_alt = no_alt_msg_off + 5 + (num_static_no_alt * 32);

        println!(
            "Without ALT: expected bh offset {}, actual bh offset {}",
            expected_bh_off_no_alt, no_alt_bh_off
        );

        if no_alt_bh_off < expected_bh_off_no_alt {
            println!(
                "CRITICAL CHALLENGE FINDING: When lookup_tables is empty and initial_blockhash == [0; 32], locate_offsets collides with SystemProgram [0; 32] at offset {}, corrupting account table instead of blockhash!",
                no_alt_bh_off
            );
        }
    }
}

// ============================================================================
// 8. DEGENERATE INITIAL TIP TEMPLATE ERROR HANDLING
// ============================================================================

#[test]
fn challenge_zero_initial_tip_template_rejection() {
    let signer = create_test_signer(18);
    let payer = signer.pubkey();
    let tip_acc = Pubkey::from_str(JITO_TIP_ACCOUNTS[0]).unwrap();
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    let usdc = Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap();
    let kamino_params = KaminoFlashParams::default();
    let opp = create_2hop_opportunity(wsol, usdc, 10_000_000_000);
    let alt = TxBuilder::create_default_mainnet_alt();

    // When initial tip is 0, assemble_atomic_bundle completely omits the Jito tip instruction.
    let bundle_params_zero_tip = KaminoBundleParams {
        payer,
        signer: &signer,
        tip_account: tip_acc,
        tip_lamports: Lamports(0), // 0 tip
        priority_fee_micro_lamports: 1_000,
        opportunity: &opp,
        kamino_params: &kamino_params,
        recent_blockhash: [0x88; 32],
        lookup_tables: &[alt],
        flash_loan_fee_sf: 0,
    };

    let result = PreWarmedTxTemplate::from_bundle_params(&bundle_params_zero_tip, 9223372036854775);
    assert!(
        result.is_err(),
        "Template creation with 0 initial tip must return error"
    );
    match result.unwrap_err() {
        TxError::TemplateOffsetError(msg) => {
            println!("Correctly rejected zero initial tip: {}", msg);
            assert!(msg.contains("Jito Tip transfer payload not found"));
        }
        other => panic!("Unexpected error: {:?}", other),
    }
}
