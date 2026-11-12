//! Meteora DLMM Atomic Builder Integration Tests.
//!
//! Verifies:
//! 1. Atomic two-swap transaction compilation.
//! 2. Strict MTU <= 1232 bytes enforcement on the legacy path.
//! 3. Distinct pool requirement (`pool_a != pool_b`).
//! 4. Intermediate token isolation.
//! 5. Solana Transaction v1 (SIMD-0385) compilation of the same route: `0x81`
//!    discriminator, <= 4096 byte payload, native compute budget, 64-address ceiling.

use arb_tx::{
    KeypairSigner, MeteoraDlmmAtomicArbitrageBuilder, MeteoraTwoPoolV1Config,
    SOLANA_PACKET_MTU_LIMIT, SOLANA_PACKET_V1_MTU, TransactionVersion,
};
use arb_types::{
    DexProtocol, DlmmBin, DlmmFeeParams, DlmmState, MintId, PoolExecutionAccounts, PoolId,
    PoolKind, PoolState, Slot, TokenProgramKind,
};
use solana_pubkey::Pubkey;
use std::collections::BTreeSet;

fn make_test_dlmm_pool(
    pool_bytes_id: u8,
    active_bin_id: i32,
    bin_step: u16,
    mint_x_byte: u8,
    mint_y_byte: u8,
) -> PoolState {
    let pool_pk = Pubkey::new_from_array([pool_bytes_id; 32]);
    let reserve_x = Pubkey::new_from_array([10 + pool_bytes_id; 32]);
    let reserve_y = Pubkey::new_from_array([20 + pool_bytes_id; 32]);
    let mint_x = MintId(Pubkey::new_from_array([mint_x_byte; 32]));
    let mint_y = MintId(Pubkey::new_from_array([mint_y_byte; 32]));

    let bins = vec![DlmmBin {
        bin_id: active_bin_id,
        amount_x: 50_000_000_000,
        amount_y: 50_000_000_000,
    }];

    let fee_params = DlmmFeeParams {
        base_factor: 100,
        filter_period: 30,
        decay_period: 600,
        reduction_factor: 5000,
        variable_fee_control: 40000,
        max_volatility_accumulator: 350000,
        protocol_share: 1000,
        base_fee_power_factor: 0,
        function_type: 0,
        collect_fee_mode: 0,
    };

    let dlmm_state = DlmmState {
        active_bin_id,
        status: 0,
        bin_step_bps: bin_step,
        bins,
        params: Some(fee_params),
        volatility_accumulator: 0,
        volatility_reference: 0,
        index_reference: active_bin_id,
        last_update_timestamp: 1_700_000_000,
    };

    let exec = PoolExecutionAccounts::MeteoraDlmm {
        reserve_x,
        reserve_y,
        oracle: pool_pk,
        bin_array_pdas: vec![Pubkey::new_from_array([30 + pool_bytes_id; 32])],
        bitmap_extension: None,
    };

    PoolState {
        pool_id: PoolId(pool_pk),
        protocol: DexProtocol::MeteoraDlmm,
        slot: Slot(467_000_000),
        write_version: 1,
        base_mint: mint_x,
        quote_mint: mint_y,
        decimals_a: 9,
        decimals_b: 6,
        reserve_in: 50_000_000_000,
        reserve_out: 50_000_000_000,
        fee_numerator: bin_step as u64,
        fee_denominator: 10_000,
        is_active: true,
        has_risky_token: false,
        kind: PoolKind::Dlmm,
        clmm: None,
        dlmm: Some(dlmm_state),
        pump_fun: None,
        transfer_fee_bps: 0,
        token_programs: [TokenProgramKind::SplToken, TokenProgramKind::SplToken],
        execution_accounts: Some(exec),
        last_update_slot: 467_000_000,
        last_update_ms: 1_700_000_000_000,
    }
}

#[test]
fn test_atomic_builder_success_within_mtu() {
    let pool1 = make_test_dlmm_pool(1, 0, 10, 1, 2);
    let pool2 = make_test_dlmm_pool(2, 50, 10, 1, 2);

    let signer = KeypairSigner::from_seed(&[88u8; 32]);
    let blockhash = [99u8; 32];

    let tx_res = MeteoraDlmmAtomicArbitrageBuilder::build_atomic_two_swap(
        &signer, &pool1, &pool2, true, // swap_a_first
        20_000_000, 21_000_000, 20_500_000, 25, // 25 bps slippage
        300_000, 100_000, blockhash,
    );

    assert!(tx_res.is_ok(), "Tx build failed: {:?}", tx_res.err());
    let tx = tx_res.unwrap();

    assert_eq!(tx.instructions.len(), 6);
    assert!(tx.tx_size <= SOLANA_PACKET_MTU_LIMIT);
    println!(
        "Compiled Meteora atomic 2-swap tx size: {} bytes",
        tx.tx_size
    );
    assert_eq!(tx.amount_in, 20_000_000);
    assert!(tx.min_hop1_out <= 21_000_000);
    assert_eq!(tx.min_final_out, 20_500_000);
}

#[test]
fn test_atomic_builder_rejects_identical_pools() {
    let pool1 = make_test_dlmm_pool(1, 0, 10, 1, 2);

    let signer = KeypairSigner::from_seed(&[88u8; 32]);
    let blockhash = [99u8; 32];

    let tx_res = MeteoraDlmmAtomicArbitrageBuilder::build_atomic_two_swap(
        &signer, &pool1, &pool1, // same pool!
        true, 20_000_000, 21_000_000, 20_500_000, 25, 300_000, 100_000, blockhash,
    );

    assert!(tx_res.is_err(), "Must reject identical pools");
}

#[test]
fn test_intermediate_balance_isolation_min_hop1() {
    let pool1 = make_test_dlmm_pool(1, 0, 10, 1, 2);
    let pool2 = make_test_dlmm_pool(2, 50, 10, 1, 2);

    let signer = KeypairSigner::from_seed(&[88u8; 32]);
    let blockhash = [99u8; 32];

    let expected_hop1 = 21_000_000u64;
    let tx = MeteoraDlmmAtomicArbitrageBuilder::build_atomic_two_swap(
        &signer,
        &pool1,
        &pool2,
        true,
        20_000_000,
        expected_hop1,
        20_500_000,
        25,
        300_000,
        100_000,
        blockhash,
    )
    .unwrap();

    // min_hop1_out must be <= expected_hop1_out (conservative slippage)
    assert!(tx.min_hop1_out <= expected_hop1);
    // hop2 uses min_hop1_out as exact input amount (isolated from wallet inventory)
    assert!(tx.min_hop1_out >= 1);
}

#[test]
fn test_atomic_builder_rejects_zero_amounts() {
    let pool1 = make_test_dlmm_pool(1, 0, 10, 1, 2);
    let pool2 = make_test_dlmm_pool(2, 50, 10, 1, 2);

    let signer = KeypairSigner::from_seed(&[88u8; 32]);
    let blockhash = [99u8; 32];

    let tx_res = MeteoraDlmmAtomicArbitrageBuilder::build_atomic_two_swap(
        &signer, &pool1, &pool2, true, 0, 21_000_000, 20_500_000, 25, 300_000, 100_000, blockhash,
    );
    assert!(tx_res.is_err(), "Must reject zero amount_in");
}

/// P2 evidence: the executable instruction must carry exactly the bin arrays the
/// quote says the swap traverses. A shallow swap gets one array; a deep swap gets
/// several. There is no 3-array execution ceiling.
///
/// The expected array counts are derived from `meteora_bin_array_indices`, the same
/// helper the builder uses, so a boundary fix in the helper can never leave this test
/// asserting a stale arithmetic shortcut.
#[test]
fn test_atomic_builder_bin_arrays_scale_with_quote_traversal() {
    let pool1 = make_test_dlmm_pool(1, 0, 10, 1, 2);
    let pool2 = make_test_dlmm_pool(2, 50, 10, 1, 2);
    let signer = KeypairSigner::from_seed(&[88u8; 32]);
    let blockhash = [99u8; 32];

    let build = |hop1_bins: u32, hop2_bins: u32| {
        MeteoraDlmmAtomicArbitrageBuilder::build_atomic_two_swap_with_traversal(
            &signer, &pool1, &pool2, true, 20_000_000, 21_000_000, 20_500_000, 25, 300_000,
            100_000, blockhash, hop1_bins, hop2_bins,
        )
        .expect("build must succeed")
    };

    // Route shape for this fixture: pool1 (base 1, quote 2) is hop 1 and swaps Y->X from
    // active bin 0; pool2 (base 1, quote 2) is hop 2 and swaps X->Y from active bin 50.
    let expected_arrays = |hop1_bins: u32, hop2_bins: u32| -> usize {
        arb_tx::meteora_bin_array_indices(0, false, hop1_bins).len()
            + arb_tx::meteora_bin_array_indices(50, true, hop2_bins).len()
    };

    // 16 fixed accounts per DLMM swap; everything above that is a bin array.
    const FIXED: usize = 16;
    let count_arrays = |tx: &arb_tx::MeteoraAtomicArbitrageTx| -> usize {
        tx.instructions
            .iter()
            .map(|ix| ix.accounts.len().saturating_sub(FIXED))
            .sum()
    };

    let shallow = build(1, 1);
    assert_eq!(
        count_arrays(&shallow),
        expected_arrays(1, 1),
        "a 1-bin traversal must carry exactly the active array per hop"
    );

    // Deep traversal: the array count must follow the helper, never a fixed cap.
    let deep = build(210, 140);
    let expected_deep = expected_arrays(210, 140);
    assert_eq!(
        count_arrays(&deep),
        expected_deep,
        "deep swap must carry every array the traversal touches"
    );
    // Prove the instruction is not capped at three arrays anywhere.
    assert!(
        count_arrays(&deep) > 3,
        "deep traversal must exceed the old 3-array ceiling: {}",
        count_arrays(&deep)
    );
    assert!(
        deep.tx_size <= SOLANA_PACKET_MTU_LIMIT,
        "{} arrays still fit the 1,232-byte MTU: {} bytes",
        count_arrays(&deep),
        deep.tx_size
    );
    println!(
        "deep legacy route ({} arrays, {} instructions): {} bytes of {}",
        count_arrays(&deep),
        deep.instructions.len(),
        deep.tx_size,
        SOLANA_PACKET_MTU_LIMIT
    );
}

// ============================================================================
// Solana Transaction v1 (SIMD-0385) two-pool route
// ============================================================================

/// Distinct addresses the transaction references, including the payer and program ids.
fn distinct_addresses(
    payer: Pubkey,
    instructions: &[arb_tx::CompiledInstruction],
) -> BTreeSet<Pubkey> {
    let mut set = BTreeSet::new();
    set.insert(payer);
    for ix in instructions {
        set.insert(ix.program_id);
        for acc in &ix.accounts {
            set.insert(acc.pubkey);
        }
    }
    set
}

fn build_v1(
    pool_a: &PoolState,
    pool_b: &PoolState,
    hop1_bins: u32,
    hop2_bins: u32,
) -> Result<arb_tx::MeteoraAtomicArbitrageTx, arb_tx::TxError> {
    let signer = KeypairSigner::from_seed(&[88u8; 32]);
    MeteoraDlmmAtomicArbitrageBuilder::build_atomic_two_swap_v1(
        &signer,
        pool_a,
        pool_b,
        true,
        20_000_000,
        21_000_000,
        20_500_000,
        25,
        MeteoraTwoPoolV1Config::default(),
        [99u8; 32],
        hop1_bins,
        hop2_bins,
    )
}

#[test]
fn test_v1_route_uses_the_version_discriminator_and_fits_4096() {
    let pool1 = make_test_dlmm_pool(1, 0, 10, 1, 2);
    let pool2 = make_test_dlmm_pool(2, 50, 10, 1, 2);

    let tx = build_v1(&pool1, &pool2, 1, 1).expect("v1 build must succeed");

    assert_eq!(tx.version, TransactionVersion::V1);
    assert_eq!(tx.wire_bytes[0], 0x81, "v1 wire starts with 0x81");
    assert!(tx.tx_size <= SOLANA_PACKET_V1_MTU);
    println!("v1 two-pool route: {} bytes", tx.tx_size);
    assert_eq!(tx.amount_in, 20_000_000);
    assert_eq!(tx.min_final_out, 20_500_000);
    // The cycle closes back into the quote mint, so only two ATAs are needed.
    assert_eq!(tx.instructions.len(), 4);
    assert_eq!(tx.hop1_pool, pool1.pool_id.0);
    assert_eq!(tx.hop2_pool, pool2.pool_id.0);
}

#[test]
fn test_v1_route_carries_no_compute_budget_instructions() {
    let pool1 = make_test_dlmm_pool(1, 0, 10, 1, 2);
    let pool2 = make_test_dlmm_pool(2, 50, 10, 1, 2);
    let tx = build_v1(&pool1, &pool2, 1, 1).unwrap();

    let cb = Pubkey::new_from_array(arb_tx::COMPUTE_BUDGET_PROGRAM_ID);
    assert!(
        tx.instructions.iter().all(|ix| ix.program_id != cb),
        "v1 must not spend instruction slots on ComputeBudgetProgram"
    );

    // The legacy path still needs them, and reports the legacy version.
    let signer = KeypairSigner::from_seed(&[88u8; 32]);
    let legacy = MeteoraDlmmAtomicArbitrageBuilder::build_atomic_two_swap(
        &signer, &pool1, &pool2, true, 20_000_000, 21_000_000, 20_500_000, 25, 300_000, 100_000,
        [99u8; 32],
    )
    .unwrap();
    assert_eq!(legacy.version, TransactionVersion::Legacy);
    assert_eq!(
        legacy.instructions.len(),
        tx.instructions.len() + 2,
        "legacy = v1 route + compute-unit limit + compute-unit price"
    );
    println!(
        "same two-pool route: legacy={} bytes ({} ix), v1={} bytes ({} ix)",
        legacy.tx_size,
        legacy.instructions.len(),
        tx.tx_size,
        tx.instructions.len()
    );
}

#[test]
fn test_v1_route_binds_the_final_minimum_out_floor() {
    let pool1 = make_test_dlmm_pool(1, 0, 10, 1, 2);
    let pool2 = make_test_dlmm_pool(2, 50, 10, 1, 2);
    let tx = build_v1(&pool1, &pool2, 1, 1).unwrap();

    // Both swap instructions carry (amount_in, min_amount_out) at data[8..16] / [16..24].
    let hop2 = tx
        .instructions
        .iter()
        .rev()
        .find(|ix| ix.data.len() == 28)
        .expect("hop 2 swap instruction");
    let mut floor = [0u8; 8];
    floor.copy_from_slice(&hop2.data[16..24]);
    assert_eq!(u64::from_le_bytes(floor), tx.min_final_out);
    // Hop 2 spends exactly the guaranteed hop-1 output, never the wallet inventory.
    let mut hop2_in = [0u8; 8];
    hop2_in.copy_from_slice(&hop2.data[8..16]);
    assert_eq!(u64::from_le_bytes(hop2_in), tx.min_hop1_out);
    assert!(tx.min_hop1_out <= 21_000_000);
}

#[test]
fn test_v1_route_rejects_an_invalid_native_config() {
    let pool1 = make_test_dlmm_pool(1, 0, 10, 1, 2);
    let pool2 = make_test_dlmm_pool(2, 50, 10, 1, 2);
    let signer = KeypairSigner::from_seed(&[88u8; 32]);

    for bad in [
        MeteoraTwoPoolV1Config {
            compute_unit_limit: 0,
            ..MeteoraTwoPoolV1Config::default()
        },
        MeteoraTwoPoolV1Config {
            compute_unit_limit: 1_400_001,
            ..MeteoraTwoPoolV1Config::default()
        },
        MeteoraTwoPoolV1Config {
            loaded_accounts_data_size_limit: 0,
            ..MeteoraTwoPoolV1Config::default()
        },
    ] {
        let res = MeteoraDlmmAtomicArbitrageBuilder::build_atomic_two_swap_v1(
            &signer, &pool1, &pool2, true, 20_000_000, 21_000_000, 20_500_000, 25, bad, [99u8; 32],
            1, 1,
        );
        assert!(res.is_err(), "invalid v1 config must fail closed: {bad:?}");
    }

    // The shipped default must convert cleanly.
    MeteoraTwoPoolV1Config::default()
        .to_v1_config_params()
        .expect("default v1 config must be valid");
}

#[test]
fn test_v1_route_rejects_identical_pools() {
    let pool1 = make_test_dlmm_pool(1, 0, 10, 1, 2);
    let res = build_v1(&pool1, &pool1, 1, 1);
    assert!(matches!(res, Err(arb_tx::TxError::InvalidInput(_))));
}

#[test]
fn test_v1_route_stays_inside_the_64_address_budget() {
    let pool1 = make_test_dlmm_pool(1, 0, 10, 1, 2);
    let pool2 = make_test_dlmm_pool(2, 50, 10, 1, 2);
    let signer = KeypairSigner::from_seed(&[88u8; 32]);

    // A deep two-hop traversal: four arrays per hop on this route.
    let deep = build_v1(&pool1, &pool2, 210, 210).unwrap();
    let addrs = distinct_addresses(signer.pubkey(), &deep.instructions);
    assert!(
        addrs.len() <= 64,
        "deep v1 route must fit the 64-address budget, got {}",
        addrs.len()
    );
    assert!(deep.tx_size <= SOLANA_PACKET_V1_MTU);
    println!(
        "deep v1 route ({} arrays): {} bytes, {} distinct addresses",
        deep.instructions
            .iter()
            .map(|ix| ix.accounts.len().saturating_sub(16))
            .sum::<usize>(),
        deep.tx_size,
        addrs.len()
    );

    // Beyond the budget the builder must name the route, not emit a broken tx.
    match build_v1(&pool1, &pool2, 2_100, 2_100) {
        Err(arb_tx::TxError::InvalidInput(msg)) => {
            assert!(
                msg.contains("distinct addresses"),
                "unexpected error: {msg}"
            );
        }
        other => panic!("expected a fail-closed address-budget error, got {other:?}"),
    }
}
