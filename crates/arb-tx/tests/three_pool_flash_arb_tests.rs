//! Three-pool Kamino-flash-funded Meteora DLMM arbitrage builder tests.
//!
//! Covers the directive's mandatory matrices for the
//! `borrow -> swap A -> swap B -> swap C -> repay` atomic builder
//! (`arb_tx::dlmm_3leg_flash_arb`):
//!
//! * instruction ordering and borrow/repay index wiring
//! * fail-closed route invariants (distinct pools, mint chaining, closing
//!   route, repayment coverage)
//! * V0-with-ALT path inside the 1,232-byte limit
//! * V1 path (0x81 discriminator, inline accounts, > 1,232 bytes, <= 4,096
//!   bytes), signature verification, and the 4,096-byte rejection
//! * V1 total-lamports priority-fee semantics (not micro-lamports per CU)

use arb_constants::{TOKEN_PROGRAM_ID, WSOL_MINT};
use arb_tx::adaptive::{
    AdaptiveTxCompiler, FormatSelectionReason, TransactionVersion, V1ConfigParams,
};
use arb_tx::dlmm_3leg_flash_arb::{
    FlashArb3LegError, FlashArb3LegParams, FlashArb3LegPoolLeg, assemble_flash_funded_three_pool,
    build_flash_funded_three_pool_transaction,
};
use arb_tx::{AddressLookupTable, KaminoFlashParams, KeypairSigner};
use solana_pubkey::Pubkey;
use std::str::FromStr;

const FLASH_LOAN_FEE_SF: u64 = 11_529_215_046_068; // main-market WSOL reserve, 1e-5 U68F60

fn wsol() -> Pubkey {
    Pubkey::from_str(WSOL_MINT).unwrap()
}

fn spl() -> Pubkey {
    Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap()
}

fn meme_a() -> Pubkey {
    Pubkey::new_from_array([0xA0u8; 32])
}

fn meme_b() -> Pubkey {
    Pubkey::new_from_array([0xB0u8; 32])
}

/// Leg with token_x -> token_y (swap_for_y = true).
fn leg(
    lb_pair: Pubkey,
    token_x_mint: Pubkey,
    token_y_mint: Pubkey,
    amount_in: u64,
    min_amount_out: u64,
    bin_arrays: Vec<Pubkey>,
) -> FlashArb3LegPoolLeg {
    FlashArb3LegPoolLeg {
        lb_pair,
        reserve_x: Pubkey::new_unique(),
        reserve_y: Pubkey::new_unique(),
        token_x_mint,
        token_y_mint,
        oracle: Pubkey::new_unique(),
        token_x_program: spl(),
        token_y_program: spl(),
        swap_for_y: true,
        amount_in,
        min_amount_out,
        bin_array_pdas: bin_arrays,
        bitmap_extension: None,
        host_fee_in: None,
    }
}

fn params(borrow: u64, leg_1_min: u64, leg_2_min: u64, leg_3_min: u64) -> FlashArb3LegParams {
    params_with_payer(
        Pubkey::new_from_array([7u8; 32]),
        borrow,
        leg_1_min,
        leg_2_min,
        leg_3_min,
    )
}

fn params_with_payer(
    payer: Pubkey,
    borrow: u64,
    leg_1_min: u64,
    leg_2_min: u64,
    leg_3_min: u64,
) -> FlashArb3LegParams {
    let fee = arb_tx::kamino::calculate_kamino_flash_fee(borrow, FLASH_LOAN_FEE_SF);
    FlashArb3LegParams {
        payer,
        kamino: KaminoFlashParams::default_wsol(),
        borrow_amount: borrow,
        flash_fee: fee,
        leg_1: leg(
            Pubkey::new_from_array([10u8; 32]),
            wsol(),
            meme_a(),
            borrow,
            leg_1_min,
            vec![Pubkey::new_unique(), Pubkey::new_unique()],
        ),
        leg_2: leg(
            Pubkey::new_from_array([20u8; 32]),
            meme_a(),
            meme_b(),
            leg_1_min,
            leg_2_min,
            vec![Pubkey::new_unique(), Pubkey::new_unique()],
        ),
        leg_3: leg(
            Pubkey::new_from_array([30u8; 32]),
            meme_b(),
            wsol(),
            leg_2_min,
            leg_3_min,
            vec![Pubkey::new_unique(), Pubkey::new_unique()],
        ),
        cu_limit: 400_000,
        priority_fee_micro_lamports: 50_000,
        jito_tip_account: Pubkey::from_str(arb_constants::JITO_TIP_ACCOUNTS[0]).unwrap(),
        jito_tip_lamports: 1_000,
        recent_blockhash: [0x42u8; 32],
        v1_transport_supported: false,
    }
}

fn valid_params() -> FlashArb3LegParams {
    valid_params_with_payer(Pubkey::new_from_array([7u8; 32]))
}

fn valid_params_with_payer(payer: Pubkey) -> FlashArb3LegParams {
    let borrow = 1_000_000_000u64;
    let fee = arb_tx::kamino::calculate_kamino_flash_fee(borrow, FLASH_LOAN_FEE_SF);
    let repay = borrow + fee;
    // Chain: leg1 out >= leg2 in, leg2 out >= leg3 in, leg3 out >= repay.
    params_with_payer(payer, borrow, 1_100_000_000, 1_050_000_000, repay + 500_000)
}

// ---------------------------------------------------------------------------
// Instruction ordering and wiring
// ---------------------------------------------------------------------------

#[test]
fn assembly_order_borrow_swaps_repay() {
    let p = valid_params();
    let (ixs, borrow_idx, repay_idx) = assemble_flash_funded_three_pool(&p).unwrap();

    // 2 CU ixs + 3 ATA creates + borrow + 3 swaps + repay + tip = 11
    assert_eq!(ixs.len(), 11);
    assert_eq!(borrow_idx, 5, "borrow must be instruction 5");
    assert_eq!(repay_idx, 9, "repay must be instruction 9 (after 3 swaps)");

    // Swap program ids are the DLMM program for indices 6..9.
    let dlmm = Pubkey::from_str(arb_constants::METEORA_DLMM_PROGRAM_ID).unwrap();
    for ix in &ixs[6..9] {
        assert_eq!(ix.program_id, dlmm);
    }

    // Repay amount = borrow + fee, and the repay instruction carries the real
    // borrow instruction index in its u8 argument.
    let repay = p.repay_amount();
    assert_eq!(repay, p.borrow_amount + p.flash_fee);
    let repay_ix = &ixs[repay_idx];
    let idx_byte = *repay_ix.data.last().unwrap();
    assert_eq!(idx_byte as usize, borrow_idx);
}

#[test]
fn duplicate_pools_rejected() {
    let mut p = valid_params();
    p.leg_3.lb_pair = p.leg_1.lb_pair;
    assert!(matches!(
        assemble_flash_funded_three_pool(&p),
        Err(FlashArb3LegError::PoolsNotDistinct)
    ));
}

#[test]
fn broken_mint_chain_rejected() {
    let mut p = valid_params();
    // Leg 2 suddenly claims to consume WSOL instead of meme A.
    p.leg_2.token_x_mint = wsol();
    p.leg_2.amount_in = p.leg_1.min_amount_out;
    assert!(matches!(
        assemble_flash_funded_three_pool(&p),
        Err(FlashArb3LegError::MintChainMismatch { .. })
    ));
}

#[test]
fn non_closing_route_rejected() {
    let mut p = valid_params();
    p.leg_3.token_y_mint = Pubkey::from_str(arb_constants::USDC_MINT).unwrap();
    assert!(matches!(
        assemble_flash_funded_three_pool(&p),
        Err(FlashArb3LegError::NonClosingRoute { .. })
    ));
}

#[test]
fn insufficient_final_output_rejected() {
    let borrow = 1_000_000_000u64;
    let fee = arb_tx::kamino::calculate_kamino_flash_fee(borrow, FLASH_LOAN_FEE_SF);
    let repay = borrow + fee;
    let mut p = valid_params();
    p.leg_3.min_amount_out = repay - 1; // one lamport short of repayment
    assert!(matches!(
        assemble_flash_funded_three_pool(&p),
        Err(FlashArb3LegError::LegOutputBelowRequired { leg: 3, .. })
    ));
}

#[test]
fn mid_route_output_gap_rejected() {
    let mut p = valid_params();
    p.leg_1.min_amount_out = p.leg_2.amount_in - 1;
    assert!(matches!(
        assemble_flash_funded_three_pool(&p),
        Err(FlashArb3LegError::LegOutputBelowRequired { leg: 1, .. })
    ));
}

#[test]
fn zero_borrow_rejected() {
    let p = params(0, 0, 0, 0);
    assert!(matches!(
        assemble_flash_funded_three_pool(&p),
        Err(FlashArb3LegError::ZeroBorrowAmount)
    ));
}

// ---------------------------------------------------------------------------
// V0 (with ALT) path
// ---------------------------------------------------------------------------

#[test]
fn v0_with_alt_fits_1232_bytes() {
    let signer = KeypairSigner::new_random();
    let p = valid_params_with_payer(signer.pubkey());
    let (ixs, _, _) = assemble_flash_funded_three_pool(&p).unwrap();

    // Cover every non-payer account so the V0 message stays small.
    let mut addresses: Vec<Pubkey> = Vec::new();
    for ix in &ixs {
        if ix.program_id != p.payer {
            addresses.push(ix.program_id);
        }
        for acc in &ix.accounts {
            if acc.pubkey != p.payer && !addresses.contains(&acc.pubkey) {
                addresses.push(acc.pubkey);
            }
        }
    }
    let alt = AddressLookupTable {
        key: Pubkey::new_unique(),
        addresses,
    };

    let tx = build_flash_funded_three_pool_transaction(&p, &signer, &[alt]).unwrap();
    assert!(
        tx.tx_size <= arb_tx::SOLANA_PACKET_MTU_LIMIT,
        "V0 wire {} exceeds 1,232-byte limit",
        tx.tx_size
    );
    let version =
        arb_tx::adaptive::CompiledAdaptiveTransaction::parse_version_from_wire(&tx.wire_bytes)
            .unwrap();
    assert_eq!(version, TransactionVersion::V0);
}

// ---------------------------------------------------------------------------
// V1 path
// ---------------------------------------------------------------------------

#[test]
fn v1_inline_route_compiles_and_verifies() {
    let mut p = valid_params();
    p.v1_transport_supported = true;
    let signer = KeypairSigner::new_random();
    p.payer = signer.pubkey();
    let tx = build_flash_funded_three_pool_transaction(&p, &signer, &[]).unwrap();

    // Three DLMM legs inline exceed the legacy 1,232-byte envelope: this is the
    // whole reason V1 exists.
    assert!(
        tx.tx_size > 1_232,
        "V1 wire {} unexpectedly fits legacy limit",
        tx.tx_size
    );
    assert!(
        tx.tx_size <= arb_tx::SOLANA_PACKET_V1_MTU,
        "V1 wire {} exceeds 4,096-byte limit",
        tx.tx_size
    );

    // V1 discriminator 0x81 at byte 0; signatures at the tail.
    assert_eq!(tx.wire_bytes[0], 0x81);
    let version =
        arb_tx::adaptive::CompiledAdaptiveTransaction::parse_version_from_wire(&tx.wire_bytes)
            .unwrap();
    assert_eq!(version, TransactionVersion::V1);

    // Signature must verify against the signer pubkey.
    let compiled = AdaptiveTxCompiler::build_signed_v1_transaction(
        &signer,
        &tx.instructions,
        p.recent_blockhash,
        V1ConfigParams {
            priority_fee_lamports: Some(20),
            compute_unit_limit: Some(p.cu_limit),
            loaded_accounts_data_size_limit: Some(1_048_576),
            heap_size: Some(256 * 1024),
        },
        FormatSelectionReason::LargePayloadV1 {
            accounts_count: 0,
            wire_size: 0,
        },
    )
    .unwrap();
    assert!(compiled.verify_signatures(&[signer.pubkey()]).unwrap());
}

#[test]
fn v1_rejects_unsupported_account_count() {
    let mut p = valid_params();
    p.v1_transport_supported = true;
    // Inflate each leg with enough bin arrays to exceed the SIMD-0385
    // 64-inline-address limit (240 arrays + route accounts > 64).
    for leg in [&mut p.leg_1, &mut p.leg_2, &mut p.leg_3] {
        leg.bin_array_pdas = (0..80).map(|_| Pubkey::new_unique()).collect();
    }
    let signer = KeypairSigner::new_random();
    p.payer = signer.pubkey();
    let err = build_flash_funded_three_pool_transaction(&p, &signer, &[]).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("address limit exceeded"),
        "expected SIMD-0385 address-limit rejection, got: {msg}"
    );
}

#[test]
fn v1_rejects_wire_above_4096_bytes() {
    // Compiler-level boundary: a single-signer V1 transaction whose inline
    // message plus trailing signature exceeds 4,096 bytes must be rejected.
    let signer = KeypairSigner::new_random();
    let big_ix = arb_tx::CompiledInstruction {
        program_id: Pubkey::new_unique(),
        accounts: vec![arb_tx::AccountMeta {
            pubkey: signer.pubkey(),
            is_signer: true,
            is_writable: true,
        }],
        data: vec![0u8; 3_902],
    };
    let err = AdaptiveTxCompiler::build_signed_v1_transaction(
        &signer,
        &[big_ix],
        [0x42u8; 32],
        V1ConfigParams {
            priority_fee_lamports: Some(1),
            compute_unit_limit: Some(200_000),
            loaded_accounts_data_size_limit: Some(1_048_576),
            heap_size: Some(256 * 1024),
        },
        FormatSelectionReason::LargePayloadV1 {
            accounts_count: 2,
            wire_size: 0,
        },
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("too large") || msg.contains("4096"),
        "expected 4,096-byte rejection, got: {msg}"
    );
}

#[test]
fn v1_accepts_wire_at_4096_byte_boundary() {
    // Measured V1 message overhead for this shape (1 ix, 2 addresses, 5 active
    // config flags, trailing 64-byte signature): wire = data_len + 195.
    // data_len = 3_901 lands exactly on the 4,096-byte V1 MTU.
    let signer = KeypairSigner::new_random();
    let ix = arb_tx::CompiledInstruction {
        program_id: Pubkey::new_unique(),
        accounts: vec![arb_tx::AccountMeta {
            pubkey: signer.pubkey(),
            is_signer: true,
            is_writable: true,
        }],
        data: vec![0u8; 3_901],
    };
    let tx = AdaptiveTxCompiler::build_signed_v1_transaction(
        &signer,
        &[ix],
        [0x42u8; 32],
        V1ConfigParams {
            priority_fee_lamports: Some(1),
            compute_unit_limit: Some(200_000),
            loaded_accounts_data_size_limit: Some(1_048_576),
            heap_size: Some(256 * 1024),
        },
        FormatSelectionReason::LargePayloadV1 {
            accounts_count: 2,
            wire_size: 0,
        },
    )
    .unwrap();
    assert_eq!(tx.wire_bytes.len(), 4_096, "boundary must be exact");
    assert_eq!(tx.wire_bytes[0], 0x81);
    assert!(tx.verify_signatures(&[signer.pubkey()]).unwrap());
}

// ---------------------------------------------------------------------------
// Atomicity self-check runs inside the builder
// ---------------------------------------------------------------------------

#[test]
fn builder_passes_atomic_integrity_for_valid_route() {
    let mut p = valid_params();
    p.v1_transport_supported = true;
    let signer = KeypairSigner::new_random();
    p.payer = signer.pubkey();
    let tx = build_flash_funded_three_pool_transaction(&p, &signer, &[]).unwrap();
    assert_eq!(tx.borrow_instruction_index, 5);
    assert_eq!(tx.repay_instruction_index, 9);
    assert_eq!(tx.repay_amount, tx.borrow_amount + tx.flash_fee);
    assert_eq!(tx.pool_1, p.leg_1.lb_pair);
    assert_eq!(tx.pool_2, p.leg_2.lb_pair);
    assert_eq!(tx.pool_3, p.leg_3.lb_pair);
    assert_eq!(tx.min_leg_3_out, p.leg_3.min_amount_out);
}

// ---------------------------------------------------------------------------
// Kamino fee math regression (authoritative klend Exclusive formula)
// ---------------------------------------------------------------------------

#[test]
fn flash_fee_matches_klend_exclusive_formula() {
    // 1 SOL at 1e-5 U68F60 = 10,000 lamports (round-half-away, not truncate).
    assert_eq!(
        arb_tx::kamino::calculate_kamino_flash_fee(1_000_000_000, FLASH_LOAN_FEE_SF),
        10_000
    );
    // Zero-borrow and zero-rate edges.
    assert_eq!(
        arb_tx::kamino::calculate_kamino_flash_fee(0, FLASH_LOAN_FEE_SF),
        0
    );
    assert_eq!(arb_tx::kamino::calculate_kamino_flash_fee(1_000, 0), 0);
}

// ---------------------------------------------------------------------------
// Supply-vault liquidity decoder (borrow ceiling source of truth)
// ---------------------------------------------------------------------------

#[test]
fn supply_vault_liquidity_decoder_roundtrip() {
    let mint = wsol();
    let mut acct = vec![0u8; 165];
    acct[0..32].copy_from_slice(mint.as_ref());
    acct[64..72].copy_from_slice(&5_000_000_000u64.to_le_bytes());
    assert_eq!(
        arb_tx::kamino::decode_supply_vault_liquidity(&acct, &mint).unwrap(),
        5_000_000_000
    );
    // Wrong mint fails closed.
    assert!(arb_tx::kamino::decode_supply_vault_liquidity(&acct, &meme_a()).is_err());
    // Short account fails closed.
    assert!(arb_tx::kamino::decode_supply_vault_liquidity(&acct[..71], &mint).is_err());
}

// ---------------------------------------------------------------------------
// TxBuilder V0 sanity (compile_v0_message used by the V0 path)
// ---------------------------------------------------------------------------

#[test]
fn v0_message_rejects_oversize_without_alt() {
    let signer = KeypairSigner::new_random();
    let p = valid_params_with_payer(signer.pubkey());
    // Without ALT coverage the fully-inline V0 message for this route is at
    // the edge; TxBuilder must either compile it within 1,232 bytes or fail —
    // never silently produce an oversized legacy transaction.
    // The builder must fail closed (TransactionTooLarge) instead of emitting
    // an oversized legacy transaction.
    let err = build_flash_funded_three_pool_transaction(&p, &signer, &[]).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("too large") && msg.contains("1232"),
        "expected oversize rejection, got: {msg}"
    );
}

#[test]
fn payer_signer_mismatch_rejected() {
    let mut p = valid_params();
    p.v1_transport_supported = true;
    let signer = KeypairSigner::new_random();
    // params.payer stays a different key than the signer.
    assert!(matches!(
        build_flash_funded_three_pool_transaction(&p, &signer, &[]),
        Err(FlashArb3LegError::PayerSignerMismatch { .. })
    ));
}
