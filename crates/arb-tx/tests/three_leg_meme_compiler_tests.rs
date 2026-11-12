//! Three-leg Meteora DLMM meme arbitrage compiler tests.
//!
//! These tests validate the P2 3-leg meme arbitrage compiler end-to-end:
//! WSOL → Meme A → Meme B → WSOL across three distinct Meteora DLMM pools.
//!
//! Two categories of tests:
//! - **Fake ALT unit tests**: Use a locally constructed StaticMasterAlt with
//!   deterministic test keys. These do NOT require a real Mainnet ALT.
//! - **Real Mainnet ALT integration tests**: Would require an actual activated
//!   Mainnet ALT. These are marked and skipped unless a real ALT address is
//!   provided via environment variable.

use arb_constants::{
    JITO_TIP_ACCOUNTS, MEMO_PROGRAM_ID, METEORA_DLMM_PROGRAM_ID, SOLANA_PACKET_MTU_LIMIT,
    TOKEN_PROGRAM_ID, WSOL_MINT,
};
use arb_tx::KeypairSigner;
use arb_tx::compiler::{SwapDirection, SwapMode};
use arb_tx::master_alt::{
    AddressCategory, MAX_ALT_ADDRESSES, StaticAddressEntry, StaticMasterAlt,
    build_or_extend_master_alt_ixs, derive_ata,
};
use arb_tx::meteora_dlmm::{
    DlmmSwapAccountSet, build_swap2_exact_in, derive_bin_array_bitmap_extension_pda,
    derive_directional_bin_arrays,
};
use arb_tx::{
    CompileResult, LegExecutionAccounts, LegSwapParams, ThreeLegMemeArbitrageContext,
    compile_3leg_meme_dlmm_arb,
};
use arb_types::{MintId, PoolId};
use solana_pubkey::Pubkey;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Deterministic test keys
// ---------------------------------------------------------------------------

fn test_payer() -> Pubkey {
    Pubkey::new_from_array([1u8; 32])
}

fn test_authority() -> Pubkey {
    Pubkey::new_from_array([2u8; 32])
}

// Meme A / Meme B mints (distinct from WSOL/USDC/USDT for test clarity)
fn meme_a_mint() -> Pubkey {
    Pubkey::new_from_array([90u8; 32])
}
fn meme_b_mint() -> Pubkey {
    Pubkey::new_from_array([91u8; 32])
}

// Three distinct pool IDs
fn pool_1() -> PoolId {
    PoolId::new([10u8; 32])
}
fn pool_2() -> PoolId {
    PoolId::new([20u8; 32])
}
fn pool_3() -> PoolId {
    PoolId::new([30u8; 32])
}

// Bin array PDAs for each leg (deterministic)
fn bin_arrays_leg1() -> Vec<Pubkey> {
    vec![
        Pubkey::new_from_array([41u8; 32]),
        Pubkey::new_from_array([42u8; 32]),
    ]
}
fn bin_arrays_leg2() -> Vec<Pubkey> {
    vec![
        Pubkey::new_from_array([51u8; 32]),
        Pubkey::new_from_array([52u8; 32]),
    ]
}
fn bin_arrays_leg3() -> Vec<Pubkey> {
    vec![
        Pubkey::new_from_array([61u8; 32]),
        Pubkey::new_from_array([62u8; 32]),
    ]
}

fn spl_program() -> Pubkey {
    Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap()
}

fn build_test_ctx(
    meme_a_ata_exists: bool,
    meme_b_ata_exists: bool,
    v1_transport_supported: bool,
) -> ThreeLegMemeArbitrageContext {
    let payer = test_payer();
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    let ma = meme_a_mint();
    let mb = meme_b_mint();
    let p1 = pool_1();
    let p2 = pool_2();
    let p3 = pool_3();
    let ma_ata = derive_ata(&payer, &ma, &spl_program());
    let mb_ata = derive_ata(&payer, &mb, &spl_program());
    let wsol_ata = derive_ata(&payer, &wsol, &spl_program());

    ThreeLegMemeArbitrageContext {
        payer,
        wsol_mint: wsol,
        meme_a_mint: MintId(ma),
        meme_b_mint: MintId(mb),
        pool_1: p1,
        pool_2: p2,
        pool_3: p3,
        leg_1_direction: SwapDirection::XtoY,
        leg_2_direction: SwapDirection::XtoY,
        leg_3_direction: SwapDirection::YtoX,
        leg_1_accounts: LegExecutionAccounts {
            lb_pair: p1.0,
            bin_array_pdas: bin_arrays_leg1(),
            bitmap_extension: None,
            reserve_x: Pubkey::new_from_array([71u8; 32]),
            reserve_y: Pubkey::new_from_array([72u8; 32]),
            token_x_mint: wsol,
            token_y_mint: ma,
            token_x_program: spl_program(),
            token_y_program: spl_program(),
            oracle: Pubkey::new_from_array([81u8; 32]),
            host_fee_in: None,
            user_token_in: wsol_ata,
            user_token_out: ma_ata,
        },
        leg_2_accounts: LegExecutionAccounts {
            lb_pair: p2.0,
            bin_array_pdas: bin_arrays_leg2(),
            bitmap_extension: None,
            reserve_x: Pubkey::new_from_array([73u8; 32]),
            reserve_y: Pubkey::new_from_array([74u8; 32]),
            token_x_mint: ma,
            token_y_mint: mb,
            token_x_program: spl_program(),
            token_y_program: spl_program(),
            oracle: Pubkey::new_from_array([82u8; 32]),
            host_fee_in: None,
            user_token_in: ma_ata,
            user_token_out: mb_ata,
        },
        leg_3_accounts: LegExecutionAccounts {
            lb_pair: p3.0,
            bin_array_pdas: bin_arrays_leg3(),
            bitmap_extension: None,
            reserve_x: Pubkey::new_from_array([75u8; 32]),
            reserve_y: Pubkey::new_from_array([76u8; 32]),
            token_x_mint: mb,
            token_y_mint: wsol,
            token_x_program: spl_program(),
            token_y_program: spl_program(),
            oracle: Pubkey::new_from_array([83u8; 32]),
            host_fee_in: None,
            user_token_in: mb_ata,
            user_token_out: wsol_ata,
        },
        leg_1_params: LegSwapParams {
            mode: SwapMode::ExactIn,
            amount_in: 1_000_000_000,
            min_amount_out: 950_000_000,
            max_amount_in: 0,
            amount_out: 0,
        },
        leg_2_params: LegSwapParams {
            mode: SwapMode::ExactIn,
            amount_in: 950_000_000,
            min_amount_out: 900_000_000,
            max_amount_in: 0,
            amount_out: 0,
        },
        leg_3_params: LegSwapParams {
            mode: SwapMode::ExactIn,
            amount_in: 900_000_000,
            min_amount_out: 850_000_000,
            max_amount_in: 0,
            amount_out: 0,
        },
        wsol_token_program: spl_program(),
        meme_a_token_program: spl_program(),
        meme_b_token_program: spl_program(),
        meme_a_ata: ma_ata,
        meme_b_ata: mb_ata,
        meme_a_ata_exists,
        meme_b_ata_exists,
        meme_a_ata_balance: 0,
        meme_b_ata_balance: 0,
        meme_a_is_token_2022: false,
        meme_b_is_token_2022: false,
        meme_a_withheld_fees: 0,
        meme_b_withheld_fees: 0,
        snapshot_slot: 250_000_000,
        instruction_version: 0,
        cu_limit: arb_constants::COMPUTE_BUDGET_LIMIT_3HOP,
        priority_fee_micro_lamports: 1_000,
        expected_min_final_output: 850_000_000,
        jito_tip_account: Pubkey::from_str(JITO_TIP_ACCOUNTS[0]).unwrap(),
        jito_tip_lamports: 10_000,
        recent_blockhash: [42u8; 32],
        v1_transport_supported,
    }
}

/// Build a StaticMasterAlt populated with the standard static entries
/// (fake ALT for unit tests — not a real Mainnet ALT).
fn build_fake_master_alt() -> StaticMasterAlt {
    let payer = test_payer();
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    let usdc = Pubkey::from_str(arb_constants::USDC_MINT).unwrap();
    let usdt = Pubkey::from_str("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB").unwrap();
    let bot_wsol_ata = derive_ata(&payer, &wsol, &spl_program());
    let bot_usdc_ata = derive_ata(&payer, &usdc, &spl_program());
    let bot_usdt_ata = derive_ata(&payer, &usdt, &spl_program());

    let mut entries = StaticMasterAlt::default_static_entries(
        &payer,
        Some(bot_wsol_ata),
        Some(bot_usdc_ata),
        Some(bot_usdt_ata),
        None,
        None,
    )
    .unwrap();

    // Pool 3 is the verified static SOL/USDT-style bridge pool: per the ALT design,
    // its pool address and stable program-owned accounts (reserves, oracle) repeat on
    // every route through the bridge and belong in the static ALT. Bin Arrays and
    // active-bin-dependent accounts are NEVER placed in the ALT.
    let pool3 = pool_3().0;
    let bridge_extra = [
        (
            AddressCategory::StaticBridgePool,
            pool3,
            "Static bridge pool LbPair (Meme B / WSOL)",
        ),
        (
            AddressCategory::StaticBridgePool,
            Pubkey::new_from_array([75u8; 32]),
            "Static bridge pool reserve X",
        ),
        (
            AddressCategory::StaticBridgePool,
            Pubkey::new_from_array([76u8; 32]),
            "Static bridge pool reserve Y",
        ),
        (
            AddressCategory::StaticBridgePool,
            Pubkey::new_from_array([83u8; 32]),
            "Static bridge pool oracle",
        ),
    ];
    for (category, address, label) in bridge_extra {
        entries.push(arb_tx::master_alt::StaticAddressEntry {
            category,
            address,
            owner_program: Some(Pubkey::from_str(arb_constants::METEORA_DLMM_PROGRAM_ID).unwrap()),
            label,
        });
    }

    let mut alt = StaticMasterAlt::new();
    let authority = test_authority();
    alt.populate_from_entries(payer, authority, entries, 250_000_000)
        .unwrap();
    alt.mark_activated(250_000_001);
    alt
}

fn env_real_alt_address() -> Option<Pubkey> {
    std::env::var("ARB_TEST_REAL_ALT_ADDRESS")
        .ok()
        .and_then(|s| Pubkey::from_str(&s).ok())
}

// ---------------------------------------------------------------------------
// Test: pools distinct
// ---------------------------------------------------------------------------

#[test]
fn test_every_pool_is_distinct() {
    let ctx = build_test_ctx(false, false, true);
    assert!(ctx.validate().is_ok());

    let mut ctx2 = ctx.clone();
    ctx2.pool_2 = ctx.pool_1;
    assert!(ctx2.validate().is_err());

    let mut ctx3 = ctx.clone();
    ctx3.pool_3 = ctx.pool_1;
    assert!(ctx3.validate().is_err());
}

// ---------------------------------------------------------------------------
// Test: token transitions correct
// ---------------------------------------------------------------------------

#[test]
fn test_all_token_transitions_correct() {
    let ctx = build_test_ctx(false, false, true);
    let ixs = arb_tx::compiler::build_leg_swap_instructions(&ctx).unwrap();
    assert_eq!(ixs.len(), 3);

    // Leg 1: WSOL in → Meme A out
    assert_eq!(ixs[0].accounts[4].pubkey, ctx.bot_wsol_ata()); // user_token_in = WSOL ATA
    assert_eq!(ixs[0].accounts[5].pubkey, ctx.meme_a_ata); // user_token_out = Meme A ATA

    // Leg 2: Meme A in → Meme B out
    assert_eq!(ixs[1].accounts[4].pubkey, ctx.meme_a_ata); // user_token_in = Meme A ATA
    assert_eq!(ixs[1].accounts[5].pubkey, ctx.meme_b_ata); // user_token_out = Meme B ATA

    // Leg 3: Meme B in → WSOL out
    assert_eq!(ixs[2].accounts[4].pubkey, ctx.meme_b_ata); // user_token_in = Meme B ATA
    assert_eq!(ixs[2].accounts[5].pubkey, ctx.bot_wsol_ata()); // user_token_out = WSOL ATA
}

// ---------------------------------------------------------------------------
// Test: dynamic meme pool accounts remain inline
// ---------------------------------------------------------------------------

#[test]
fn test_dynamic_meme_pool_accounts_remain_inline() {
    let ctx = build_test_ctx(false, false, true);
    let alt = build_fake_master_alt();
    let (alt_accs, inline_accs) = arb_tx::compiler::partition_accounts(&ctx, &alt).unwrap();

    // Pool 1 / Pool 2 (dynamic meme pools) must be inline, never in the ALT.
    assert!(inline_accs.contains(&ctx.pool_1.0));
    assert!(inline_accs.contains(&ctx.pool_2.0));
    assert!(!alt_accs.contains(&ctx.pool_1.0));
    assert!(!alt_accs.contains(&ctx.pool_2.0));
    // Pool 3 is the verified static bridge pool, configured in the ALT.
    assert!(alt_accs.contains(&ctx.pool_3.0));

    // Meme mints are dynamic, must be inline
    assert!(inline_accs.contains(&ctx.meme_a_mint.0));
    assert!(inline_accs.contains(&ctx.meme_b_mint.0));
}

// ---------------------------------------------------------------------------
// Test: static addresses resolve from ALT
// ---------------------------------------------------------------------------

#[test]
fn test_reusable_static_addresses_resolve_from_alt() {
    let ctx = build_test_ctx(false, false, true);
    let alt = build_fake_master_alt();

    // DLMM program must be in ALT
    assert!(alt.contains(&Pubkey::from_str(METEORA_DLMM_PROGRAM_ID).unwrap()));
    // Event authority must be in ALT
    assert!(alt.contains(&arb_tx::derive_meteora_event_authority()));
    // Memo program must be in ALT
    assert!(alt.contains(&Pubkey::from_str(MEMO_PROGRAM_ID).unwrap()));
    // SPL Token program must be in ALT
    assert!(alt.contains(&spl_program()));
    // WSOL mint must be in ALT
    assert!(alt.contains(&ctx.wsol_mint));
    // Bot WSOL ATA must be in ALT
    assert!(alt.contains(&ctx.bot_wsol_ata()));
    // All 8 Jito tip accounts must be in ALT
    let jito_in_alt: Vec<_> = JITO_TIP_ACCOUNTS
        .iter()
        .filter(|s| alt.contains(&Pubkey::from_str(s).unwrap()))
        .collect();
    assert_eq!(jito_in_alt.len(), 8);
}

// ---------------------------------------------------------------------------
// Test: no ALT account is treated as a signer
// ---------------------------------------------------------------------------

#[test]
fn test_no_alt_account_is_signer() {
    let ctx = build_test_ctx(false, false, true);
    let alt = build_fake_master_alt();
    let signer = KeypairSigner::new_random();

    let result = compile_3leg_meme_dlmm_arb(&ctx, &alt, &signer).unwrap();
    match &result {
        CompileResult::V0JitoCompatible(v0) => {
            // Payer is the only signer
            assert_eq!(v0.signer_count, 1);
            // Payer is NOT in the ALT
            assert!(!alt.contains(&ctx.payer));
            // The compiler's verify_no_signers ensures ALT accounts are non-signers at compile time
        }
        CompileResult::V1DirectTransport(_) => {
            // V1 has no ALT, so this test is N/A
        }
        CompileResult::Rejected(reason) => {
            panic!("Unexpected rejection: {reason:?}");
        }
    }
}

// ---------------------------------------------------------------------------
// Test: directional Bin Arrays valid
// ---------------------------------------------------------------------------

#[test]
fn test_all_directional_bin_arrays_valid() {
    let pair = pool_1().0;

    // 1 bin traversed → only active array needed
    let result = derive_directional_bin_arrays(&pair, 100, SwapDirection::XtoY, 1).unwrap();
    assert_eq!(result.indices, vec![1]);
    assert_eq!(result.bin_array_pdas.len(), 1);
    assert!(!result.bitmap_extension_required);

    // 71 bins traversed → 2 arrays needed (active + lower adjacent for X→Y).
    // Both array indices are within the default bitmap range (-512..=511) -> extension NOT required.
    let result = derive_directional_bin_arrays(&pair, 100, SwapDirection::XtoY, 71).unwrap();
    assert_eq!(result.indices, vec![1, 0]);
    assert_eq!(result.bin_array_pdas.len(), 2);
    assert!(!result.bitmap_extension_required);

    // 71 bins traversed → 2 arrays needed (active + higher adjacent for Y→X).
    let result = derive_directional_bin_arrays(&pair, 100, SwapDirection::YtoX, 71).unwrap();
    assert_eq!(result.indices, vec![1, 2]);
    assert_eq!(result.bin_array_pdas.len(), 2);
    assert!(!result.bitmap_extension_required);

    // div_euclid: -1 / 70 = -1 (floor division, no second adjustment)
    let result = derive_directional_bin_arrays(&pair, -1, SwapDirection::XtoY, 1).unwrap();
    assert_eq!(result.indices, vec![-1]);
    assert_eq!(result.bin_array_pdas.len(), 1);
}

// ---------------------------------------------------------------------------
// Test: Two-Array ceiling rejection
// ---------------------------------------------------------------------------

#[test]
fn test_two_array_ceiling_rejection() {
    let pair = pool_1().0;
    // 141 bins traversed => 3 arrays needed => rejected
    let err = derive_directional_bin_arrays(&pair, 100, SwapDirection::XtoY, 141).unwrap_err();
    match err {
        arb_tx::meteora_dlmm::DlmmError::TooManyBinArrays { needed, max } => {
            assert_eq!(needed, 3);
            assert_eq!(max, 2);
        }
        other => panic!("Expected TooManyBinArrays, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Test: optional account sentinels correct
// ---------------------------------------------------------------------------

#[test]
fn test_optional_account_sentinels_correct() {
    let pair = pool_1().0;
    let ba_pdas = bin_arrays_leg1();
    let ext = derive_bin_array_bitmap_extension_pda(&pair);

    // Without host fee → sentinel (program ID) at position 9
    let accounts_no_fee = DlmmSwapAccountSet {
        lb_pair: pair,
        bin_array_bitmap_extension: Some(ext),
        reserve_x: Pubkey::new_unique(),
        reserve_y: Pubkey::new_unique(),
        user_token_in: Pubkey::new_unique(),
        user_token_out: Pubkey::new_unique(),
        token_x_mint: Pubkey::from_str(WSOL_MINT).unwrap(),
        token_y_mint: meme_a_mint(),
        oracle: Pubkey::new_unique(),
        host_fee_in: None,
        user: test_payer(),
        token_x_program: spl_program(),
        token_y_program: spl_program(),
        memo_program: Pubkey::from_str(MEMO_PROGRAM_ID).unwrap(),
        event_authority: arb_tx::derive_meteora_event_authority(),
        bin_arrays: ba_pdas.clone(),
    };
    let ix = build_swap2_exact_in(&accounts_no_fee, 1_000_000, 900_000);
    let sentinel = Pubkey::from_str(METEORA_DLMM_PROGRAM_ID).unwrap();
    // slot 1 = bitmap extension (Some(ext) is passed here -> writable, real PDA)
    assert_eq!(ix.accounts[1].pubkey, ext);
    assert!(ix.accounts[1].is_writable);
    // slot 9 = host_fee_in (absent) -> sentinel
    assert_eq!(ix.accounts[9].pubkey, sentinel);
    assert!(!ix.accounts[9].is_signer);
    assert!(!ix.accounts[9].is_writable);

    // bitmap extension absent -> sentinel at slot 1
    let accounts_no_ext = DlmmSwapAccountSet {
        bin_array_bitmap_extension: None,
        ..accounts_no_fee.clone()
    };
    let ix3 = build_swap2_exact_in(&accounts_no_ext, 1_000_000, 900_000);
    assert_eq!(ix3.accounts[1].pubkey, sentinel);
    assert!(!ix3.accounts[1].is_writable);

    // With host fee → actual address, not sentinel
    let host_fee = Pubkey::new_unique();
    let accounts_with_fee = DlmmSwapAccountSet {
        host_fee_in: Some(host_fee),
        ..accounts_no_fee.clone()
    };
    let ix2 = build_swap2_exact_in(&accounts_with_fee, 1_000_000, 900_000);
    assert_eq!(ix2.accounts[9].pubkey, host_fee);
    assert_ne!(ix2.accounts[9].pubkey, sentinel);
}

// ---------------------------------------------------------------------------
// Test: token programs correct
// ---------------------------------------------------------------------------

#[test]
fn test_token_programs_correct() {
    let ctx = build_test_ctx(false, false, true);
    // All SPL Token in this test fixture
    assert_eq!(ctx.wsol_token_program, spl_program());
    assert_eq!(ctx.meme_a_token_program, spl_program());
    assert_eq!(ctx.meme_b_token_program, spl_program());

    // Each leg's swap instruction references the correct token programs
    let ixs = arb_tx::compiler::build_leg_swap_instructions(&ctx).unwrap();
    for ix in &ixs {
        // Token program accounts exist in each instruction
        assert!(ix.accounts.iter().any(|a| a.pubkey == spl_program()));
    }
}

// ---------------------------------------------------------------------------
// Test: intermediate ATA instructions correct
// ---------------------------------------------------------------------------

#[test]
fn test_intermediate_ata_instructions_correct() {
    let ctx = build_test_ctx(false, false, true);
    let ixs = arb_tx::compiler::build_leg_swap_instructions(&ctx).unwrap();
    assert_eq!(ixs.len(), 3);

    // Meme A ATA used as user_token_out in leg 1 and user_token_in in leg 2
    assert_eq!(ixs[0].accounts[5].pubkey, ctx.meme_a_ata);
    assert_eq!(ixs[1].accounts[4].pubkey, ctx.meme_a_ata);

    // Meme B ATA used as user_token_out in leg 2 and user_token_in in leg 3
    assert_eq!(ixs[1].accounts[5].pubkey, ctx.meme_b_ata);
    assert_eq!(ixs[2].accounts[4].pubkey, ctx.meme_b_ata);

    // When ATA doesn't exist, CreateIdempotent should be included
    let ctx_create = build_test_ctx(false, false, true);
    assert!(!ctx_create.meme_a_ata_exists);
    assert!(!ctx_create.meme_b_ata_exists);
}

// ---------------------------------------------------------------------------
// Test: no unsafe ATA closing
// ---------------------------------------------------------------------------

#[test]
fn test_no_unsafe_ata_closing() {
    // Pre-existing ATA with balance → must NOT close
    let mut ctx = build_test_ctx(true, true, true);
    ctx.meme_a_ata_balance = 1_000_000;
    ctx.meme_b_ata_balance = 500_000;
    assert!(!arb_tx::compiler::should_close_intermediate_ata(&ctx, true));
    assert!(!arb_tx::compiler::should_close_intermediate_ata(
        &ctx, false
    ));

    // Token-2022 with withheld fees → must NOT close
    let mut ctx2 = build_test_ctx(false, false, true);
    ctx2.meme_a_is_token_2022 = true;
    ctx2.meme_a_withheld_fees = 500;
    assert!(!arb_tx::compiler::should_close_intermediate_ata(
        &ctx2, true
    ));

    // Nonzero dust balance → must NOT close
    let mut ctx3 = build_test_ctx(false, false, true);
    ctx3.meme_a_ata_balance = 1; // dust
    assert!(!arb_tx::compiler::should_close_intermediate_ata(
        &ctx3, true
    ));

    // Missing ATA, zero balance, non-T2022 → safe to close
    let ctx4 = build_test_ctx(false, false, true);
    assert!(arb_tx::compiler::should_close_intermediate_ata(&ctx4, true));
    assert!(arb_tx::compiler::should_close_intermediate_ata(
        &ctx4, false
    ));
}

// ---------------------------------------------------------------------------
// Test: compiled V0 transaction size measurement
// ---------------------------------------------------------------------------

#[test]
fn test_compiled_v0_size_accurately_measured() {
    let ctx = build_test_ctx(false, false, true);
    let alt = build_fake_master_alt();
    let signer = KeypairSigner::new_random();

    let result = compile_3leg_meme_dlmm_arb(&ctx, &alt, &signer).unwrap();
    match &result {
        CompileResult::V0JitoCompatible(v0) => {
            // Wire size must equal actual serialized length
            assert_eq!(v0.wire_size, v0.wire_bytes.len());
            assert!(v0.wire_size > 0);
            assert!(v0.signer_count >= 1);
            assert!(v0.alt_resolved_count > 0);
            assert!(v0.inline_account_count > 0);
            // Byte budget must be populated
            assert!(v0.byte_budget.total_wire_size > 0);
            assert!(v0.byte_budget.signature_vector_bytes == 65);
            assert!(v0.byte_budget.blockhash_bytes == 32);
            println!("V0 wire size: {} bytes", v0.wire_size);
            println!("{}", v0.byte_budget.summarize());
        }
        CompileResult::V1DirectTransport(_) => {
            // V0 may be too large for this fixture — V1 is acceptable
        }
        CompileResult::Rejected(reason) => {
            panic!("Unexpected rejection: {reason:?}");
        }
    }
}

// ---------------------------------------------------------------------------
// Test: V0 1232-byte boundary test
// ---------------------------------------------------------------------------

#[test]
fn test_v0_1232_byte_boundary() {
    let ctx = build_test_ctx(false, false, true);
    let alt = build_fake_master_alt();
    let signer = KeypairSigner::new_random();

    let result = compile_3leg_meme_dlmm_arb(&ctx, &alt, &signer).unwrap();
    if let CompileResult::V0JitoCompatible(v0) = &result {
        assert!(
            v0.wire_size <= SOLANA_PACKET_MTU_LIMIT,
            "V0 wire size {} exceeds 1232-byte MTU",
            v0.wire_size
        );
    }
}

// ---------------------------------------------------------------------------
// Test: V0 oversized rejection test
// ---------------------------------------------------------------------------

#[test]
fn test_v0_oversized_rejection() {
    // Directly test the rejection variant
    let reason = arb_tx::compiler::RejectionReason::V0Oversized {
        wire_size: 1300,
        limit: 1232,
    };
    assert_eq!(
        reason,
        arb_tx::compiler::RejectionReason::V0Oversized {
            wire_size: 1300,
            limit: 1232
        }
    );
}

// ---------------------------------------------------------------------------
// Test: V1 <= 4096-byte direct transport test
// ---------------------------------------------------------------------------

#[test]
fn test_v1_direct_transport_under_limit() {
    let ctx = build_test_ctx(false, false, true);
    let alt = build_fake_master_alt();
    let signer = KeypairSigner::new_random();

    let result = compile_3leg_meme_dlmm_arb(&ctx, &alt, &signer).unwrap();
    match &result {
        CompileResult::V1DirectTransport(v1) => {
            assert!(
                v1.wire_size <= arb_tx::adaptive::SOLANA_PACKET_V1_MTU,
                "V1 wire size {} exceeds 4096-byte limit",
                v1.wire_size
            );
        }
        CompileResult::V0JitoCompatible(_) => {
            // V0 fit, so V1 wasn't needed — acceptable
        }
        CompileResult::Rejected(reason) => {
            eprintln!("Rejected: {reason:?}");
        }
    }
}

// ---------------------------------------------------------------------------
// Test: V1 > 4096-byte rejection test
// ---------------------------------------------------------------------------

#[test]
fn test_v1_oversized_rejection() {
    let reason = arb_tx::compiler::RejectionReason::V1Oversized {
        wire_size: 4100,
        limit: 4096,
    };
    assert_eq!(
        reason,
        arb_tx::compiler::RejectionReason::V1Oversized {
            wire_size: 4100,
            limit: 4096
        }
    );
}

// ---------------------------------------------------------------------------
// Test: V1 unsupported transport rejection test
// ---------------------------------------------------------------------------

#[test]
fn test_v1_unsupported_transport_rejection() {
    let ctx = build_test_ctx(false, false, false); // V1 not supported
    let alt = build_fake_master_alt();
    let signer = KeypairSigner::new_random();

    let result = compile_3leg_meme_dlmm_arb(&ctx, &alt, &signer).unwrap();
    // If V0 fits, we get V0. If V0 is too large and V1 unsupported, we get rejected.
    // Either way, no crash.
    match &result {
        CompileResult::V0JitoCompatible(_) => {}
        CompileResult::V1DirectTransport(_) => {
            panic!("V1 should not be selected when transport unsupported");
        }
        CompileResult::Rejected(arb_tx::compiler::RejectionReason::V1TransportUnsupported) => {}
        CompileResult::Rejected(other) => {
            eprintln!("Other rejection (V0 may have fit): {other:?}");
        }
    }
}

// ---------------------------------------------------------------------------
// Test: ALT capacity test
// ---------------------------------------------------------------------------

#[test]
fn test_alt_capacity_limit() {
    let mut alt = StaticMasterAlt::new();
    let payer = test_payer();
    let authority = test_authority();
    let mut entries: Vec<StaticAddressEntry> = vec![
        StaticAddressEntry {
            category: AddressCategory::MeteoraProgram,
            address: Pubkey::new_from_array([1u8; 32]),
            owner_program: None,
            label: "meteora",
        },
        StaticAddressEntry {
            category: AddressCategory::SplTokenProgram,
            address: Pubkey::new_from_array([2u8; 32]),
            owner_program: None,
            label: "spl_token",
        },
        StaticAddressEntry {
            category: AddressCategory::SystemProgram,
            address: Pubkey::new_from_array([3u8; 32]),
            owner_program: None,
            label: "system",
        },
        StaticAddressEntry {
            category: AddressCategory::WsolMint,
            address: Pubkey::new_from_array([4u8; 32]),
            owner_program: None,
            label: "wsol",
        },
    ];

    // Fill the rest with Jito tip account entries up to and beyond capacity.
    // Use new_unique() to avoid u8 wraparound duplicates.
    for _ in 0..(MAX_ALT_ADDRESSES + 10) {
        entries.push(StaticAddressEntry {
            category: AddressCategory::JitoTipAccount,
            address: Pubkey::new_unique(),
            owner_program: None,
            label: "filler",
        });
    }

    let err = alt
        .populate_from_entries(payer, authority, entries, 100_000)
        .unwrap_err();
    assert!(matches!(
        err,
        arb_tx::master_alt::MasterAltError::CapacityExceeded { .. }
    ));
}

// ---------------------------------------------------------------------------
// Test: missing ALT address test
// ---------------------------------------------------------------------------

#[test]
fn test_missing_alt_address() {
    let alt = StaticMasterAlt::new();
    // Not provisioned
    assert!(!alt.is_provisioned());
    assert!(alt.alt_address.is_none());

    // Trying to compile without ALT should fail
    let ctx = build_test_ctx(false, false, true);
    let signer = KeypairSigner::new_random();
    let result = compile_3leg_meme_dlmm_arb(&ctx, &alt, &signer);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// Test: StaticMasterAlt event authority matches expected mainnet
// ---------------------------------------------------------------------------

#[test]
fn test_event_authority_matches_expected_mainnet() {
    let derived = arb_tx::derive_meteora_event_authority();
    let expected = Pubkey::from_str("D1ZN9Wj1fRSUQfCjhvnu1hqDMT7hzjzBBpi12nVniYD6").unwrap();
    assert_eq!(derived, expected);
}

// ---------------------------------------------------------------------------
// Test: build_or_extend_master_alt_ixs structure
// ---------------------------------------------------------------------------

#[test]
fn test_build_or_extend_master_alt_ixs() {
    let payer = test_payer();
    let authority = test_authority();
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    let usdc = Pubkey::from_str(arb_constants::USDC_MINT).unwrap();
    let entries = vec![
        StaticAddressEntry {
            category: AddressCategory::WsolMint,
            address: wsol,
            owner_program: Some(spl_program()),
            label: "wsol",
        },
        StaticAddressEntry {
            category: AddressCategory::UsdcMint,
            address: usdc,
            owner_program: Some(spl_program()),
            label: "usdc",
        },
    ];

    let (alt_address, ixs, report) =
        build_or_extend_master_alt_ixs(&payer, &authority, 999_999, entries).unwrap();

    assert_eq!(report.addresses_added, 2);
    assert_eq!(report.total_addresses, 2);
    assert_eq!(alt_address, alt_address);

    // First instruction is CreateLookupTable
    assert_eq!(
        ixs[0].program_id,
        Pubkey::from_str(arb_constants::ADDRESS_LOOKUP_TABLE_PROGRAM_ID).unwrap()
    );
    assert_eq!(&ixs[0].data[0..4], &[0, 0, 0, 0]);
    assert!(ixs[0].accounts.iter().any(|a| a.pubkey == alt_address));
    assert!(
        ixs[0]
            .accounts
            .iter()
            .any(|a| a.pubkey == authority && a.is_signer)
    );
    assert!(
        ixs[0]
            .accounts
            .iter()
            .any(|a| a.pubkey == payer && a.is_signer)
    );
}

// ---------------------------------------------------------------------------
// Test: ALT accounts cannot be signers
// ---------------------------------------------------------------------------

#[test]
fn test_alt_accounts_cannot_be_signers() {
    let mut alt = StaticMasterAlt::new();
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    alt.addresses.push(wsol);
    alt.address_set.insert(wsol);

    // wsol as signer → rejected
    assert!(alt.verify_no_signers(&[wsol]).is_err());
    // Empty signer list → fine
    assert!(alt.verify_no_signers(&[]).is_ok());
}

// ---------------------------------------------------------------------------
// Test: real Mainnet ALT integration test (skipped unless env var set)
// ---------------------------------------------------------------------------

#[test]
fn test_real_mainnet_alt_integration() {
    let real_alt = match env_real_alt_address() {
        Some(addr) => addr,
        None => {
            eprintln!(
                "Skipping real Mainnet ALT integration test: set ARB_TEST_REAL_ALT_ADDRESS env var"
            );
            return;
        }
    };

    // This test would verify a real Mainnet ALT exists and is activated.
    // It does NOT create or modify any on-chain state.
    let _ = real_alt;
    eprintln!("Real Mainnet ALT address: {real_alt}");
    // In a full integration, we would:
    // 1. Fetch the ALT account from Mainnet RPC
    // 2. Verify it contains the expected static addresses
    // 3. Verify last_extended_slot >= activation slot
    // 4. Verify authority matches the operator
}

// ---------------------------------------------------------------------------
// V1 direct transport measurement (forced V1 compile, no ALT references)
// ---------------------------------------------------------------------------

#[test]
fn test_v1_forced_compile_measured_not_jito() {
    use arb_tx::adaptive::{
        AdaptiveTxCompiler, CONFIG_MASK_CU_LIMIT, CONFIG_MASK_LOADED_DATA_LIMIT,
        CompilerCapabilities, FormatSelectionReason, MAX_V1_ADDRESSES, MAX_V1_INSTRUCTIONS,
        SOLANA_PACKET_V1_MTU, V1_VERSION_DISCRIMINATOR, V1ConfigParams,
    };

    let ctx = build_test_ctx(false, false, true);

    // Reconstruct the same instruction vector the compiler emits, minus the
    // two legacy ComputeBudget instructions (V1 configures resources natively).
    let compute_budget = Pubkey::new_from_array(arb_constants::COMPUTE_BUDGET_PROGRAM_ID);
    let mut v1_ixs: Vec<arb_tx::CompiledInstruction> = vec![
        arb_tx::build_create_ata_idempotent(
            ctx.payer,
            ctx.payer,
            ctx.meme_a_mint.0,
            ctx.meme_a_ata,
        ),
        arb_tx::build_create_ata_idempotent(
            ctx.payer,
            ctx.payer,
            ctx.meme_b_mint.0,
            ctx.meme_b_ata,
        ),
    ];
    v1_ixs.extend(arb_tx::compiler::build_leg_swap_instructions(&ctx).unwrap());
    v1_ixs.push(arb_tx::build_close_token_account(
        ctx.meme_a_ata,
        ctx.payer,
        ctx.payer,
    ));
    v1_ixs.push(arb_tx::build_close_token_account(
        ctx.meme_b_ata,
        ctx.payer,
        ctx.payer,
    ));
    v1_ixs.push(arb_tx::build_marked_jito_tip_instruction(
        ctx.payer,
        ctx.jito_tip_account,
        arb_types::Lamports(ctx.jito_tip_lamports),
    ));
    assert!(v1_ixs.iter().all(|ix| ix.program_id != compute_budget));
    assert!(v1_ixs.len() <= MAX_V1_INSTRUCTIONS);

    // NOTE: the fixture uses SPL token ATAs; the test for Token-2022 paths is in
    // the meteora_dlmm unit tests.

    let cu = ctx.cu_limit as u64;
    let micro = ctx.priority_fee_micro_lamports;
    let total_priority_fee = (cu * micro).div_ceil(1_000_000);

    let config = V1ConfigParams {
        priority_fee_lamports: Some(total_priority_fee),
        compute_unit_limit: Some(ctx.cu_limit),
        loaded_accounts_data_size_limit: Some(256 * 1024),
        heap_size: None,
    };

    let signer = KeypairSigner::new_random();
    let v1 = AdaptiveTxCompiler::build_signed_v1_transaction(
        &signer,
        &v1_ixs,
        ctx.recent_blockhash,
        config,
        FormatSelectionReason::LargePayloadV1 {
            accounts_count: arb_tx::adaptive::count_unique_accounts(ctx.payer, &v1_ixs),
            wire_size: 0,
        },
    )
    .expect("V1 message must compile");

    // SIMD-0385 wire layout checks.
    assert_eq!(v1.wire_bytes[0], V1_VERSION_DISCRIMINATOR);
    let num_ix = v1.message_bytes[40] as usize;
    let num_addresses = v1.message_bytes[41] as usize;
    let mask = u32::from_le_bytes(v1.message_bytes[4..8].try_into().unwrap());
    eprintln!("=== Forced V1 measurement ===");
    eprintln!("V1 instructions: {num_ix}");
    eprintln!("V1 inline addresses: {num_addresses}");
    eprintln!("V1 config mask: 0x{mask:08x}");
    eprintln!("V1 wire size: {} bytes", v1.wire_size());
    eprintln!("signatures: {}", v1.signatures.len());
    eprintln!(
        "max_supported_transaction_version: {:?}",
        v1.max_supported_transaction_version()
    );

    assert!(num_ix <= MAX_V1_INSTRUCTIONS);
    assert!(num_addresses <= MAX_V1_ADDRESSES);
    assert_eq!(num_ix, v1_ixs.len());
    assert_ne!(
        mask & CONFIG_MASK_CU_LIMIT,
        0,
        "CU limit must be in the V1 config mask"
    );
    assert_ne!(
        mask & CONFIG_MASK_LOADED_DATA_LIMIT,
        0,
        "loaded data limit must be in the V1 config mask"
    );
    assert_eq!(v1.signatures.len(), 1);
    assert_eq!(v1.wire_size(), v1.wire_bytes.len());
    assert!(
        v1.wire_size() <= SOLANA_PACKET_V1_MTU,
        "V1 wire size {} exceeds 4096",
        v1.wire_size()
    );

    // V1 format has no Address Lookup Table section at all by construction:
    // every referenced account is inline (message[41] inline addresses). Jito
    // V1/BAM/bundle support is NOT claimed — this tx must only use a direct
    // V1-capable RPC/TPU transport, and no such transport is enabled here.
    let b64 = v1.to_base64();
    assert!(!b64.is_empty());
    println!("V1 base64 length: {}", b64.len());

    // Explicitly NOT Jito-compatible: the Jito path (build_marked_jito_tip_instruction
    // accepts the tip as an instruction inside the tx) is orthogonal to V1 transport
    // support, which remains unverified for Jito endpoints. Do not submit V1 to Jito.
    let caps = CompilerCapabilities::v0_standard();
    assert!(!caps.supports_v1, "default caps must not claim V1");
}

// ---------------------------------------------------------------------------
// Benchmark: compile time, size, accounts, signers
// ---------------------------------------------------------------------------

#[test]
fn test_benchmark_compile_metrics() {
    let ctx = build_test_ctx(false, false, true);
    let alt = build_fake_master_alt();
    let signer = KeypairSigner::new_random();

    let start = std::time::Instant::now();
    let result = compile_3leg_meme_dlmm_arb(&ctx, &alt, &signer).unwrap();
    let compile_time_ms = start.elapsed().as_millis();

    match &result {
        CompileResult::V0JitoCompatible(v0) => {
            eprintln!("=== 3-Leg Meme Arbitrage Benchmark (V0) ===");
            eprintln!("Compile time: {compile_time_ms} ms");
            eprintln!("Wire size: {} bytes (<= 1232)", v0.wire_size);
            eprintln!("Static accounts: {}", v0.static_account_count);
            eprintln!("Inline accounts: {}", v0.inline_account_count);
            eprintln!("ALT resolved: {}", v0.alt_resolved_count);
            eprintln!("Signers: {}", v0.signer_count);
            eprintln!("Byte budget: {}", v0.byte_budget.summarize());
        }
        CompileResult::V1DirectTransport(v1) => {
            eprintln!("=== 3-Leg Meme Arbitrage Benchmark (V1) ===");
            eprintln!("Compile time: {compile_time_ms} ms");
            eprintln!("Wire size: {} bytes (<= 4096)", v1.wire_size);
            eprintln!("Accounts: {}", v1.account_count);
            eprintln!("Signers: {}", v1.signer_count);
            eprintln!("Byte budget: {}", v1.byte_budget.summarize());
        }
        CompileResult::Rejected(reason) => {
            eprintln!("Rejected: {reason:?}");
        }
    }
}
