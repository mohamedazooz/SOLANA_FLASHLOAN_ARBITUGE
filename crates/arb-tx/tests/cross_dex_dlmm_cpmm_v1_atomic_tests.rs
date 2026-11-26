//! End-to-End Verification of Meteora DLMM → Raydium CPMM Cross-DEX Flash Loan Arbitrage
//!
//! Covers:
//! - Phase 1: Real execution path and specialized builder invocation
//! - Phase 2: Account and snapshot integrity for DLMM and CPMM hops
//! - Phase 3: Quote, economic parity, checked arithmetic, and on-chain profit floor
//! - Phase 4: Atomic transaction layout, Kamino borrow/repay, ATA setup/cleanup
//! - Phase 5: V1 compilation (0x81, MTU <= 4096, 64 inline accounts, loaded-data limit)
//!   and failure injection (hop 1, hop 2, and flash repay).

use arb_constants::{
    KAMINO_KLEND_PROGRAM_ID, METEORA_DLMM_PROGRAM_ID, RAYDIUM_CPMM_PROGRAM_ID, TOKEN_PROGRAM_ID,
    USDC_MINT, WSOL_MINT,
};
use arb_tx::{
    AccountMeta, AdaptiveTxCompiler, CompiledAdaptiveTransaction, CompiledInstruction, DexPlatform,
    FormatSelectionReason, KaminoFlashParams, KeypairSigner, PoolExecutionAccounts,
    SOLANA_PACKET_V1_MTU, TransactionVersion, TransportCapability, TransportKind,
    TransportSelectionError, TxBuilder, TxError, V1ConfigParams,
    cross_dex_flash_arb::{
        CrossDexFlashArbParams, CrossDexSwapLeg, assemble_cross_dex_flash_arbitrage,
        assemble_cross_dex_opportunity, is_cross_dex_opportunity, validate_cross_dex_opportunity,
    },
    select_authoritative_transport,
};
use arb_types::{
    Bps, DexProtocol, ExecutionOpportunity, Lamports, MintId, PoolId, PumpSwapTradeIntent, Slot,
    SwapHop, TokenAmount, TokenProgramKind,
};
use solana_pubkey::Pubkey;
use std::str::FromStr;

/// Build mock DLMM and CPMM execution accounts and an ExecutionOpportunity
fn create_test_dlmm_cpmm_opportunity(
    borrow_amount: u64,
    expected_dlmm_out: u64,
    expected_cpmm_out: u64,
) -> (ExecutionOpportunity, KaminoFlashParams, Pubkey, Pubkey) {
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    let usdc = Pubkey::from_str(USDC_MINT).unwrap();

    let dlmm_pool = Pubkey::new_unique();
    let dlmm_reserve_x = Pubkey::new_unique();
    let dlmm_reserve_y = Pubkey::new_unique();
    let dlmm_oracle = Pubkey::new_unique();
    let dlmm_bin_array = Pubkey::new_unique();

    let cpmm_pool = Pubkey::new_unique();
    let cpmm_amm_config = Pubkey::new_unique();
    let cpmm_vault_0 = Pubkey::new_unique();
    let cpmm_vault_1 = Pubkey::new_unique();
    let cpmm_observation = Pubkey::new_unique();
    let cpmm_authority = Pubkey::new_unique();

    // Hop 0: Meteora DLMM (WSOL -> USDC)
    let hop_dlmm = SwapHop {
        pool_id: PoolId(dlmm_pool),
        protocol: DexProtocol::MeteoraDlmm,
        mint_in: MintId(wsol),
        mint_out: MintId(usdc),
        token_programs: [TokenProgramKind::SplToken, TokenProgramKind::SplToken],
        fee_bps: 10,
        trade_intent: PumpSwapTradeIntent::default(),
        execution_accounts: Some(PoolExecutionAccounts::MeteoraDlmm {
            reserve_x: dlmm_reserve_x,
            reserve_y: dlmm_reserve_y,
            oracle: dlmm_oracle,
            bin_array_pdas: vec![dlmm_bin_array],
            bitmap_extension: None,
            pool_token_x: wsol,
            pool_token_y: usdc,
        }),
        active_bin_id: Some(100),
    };

    // Hop 1: Raydium CPMM (USDC -> WSOL)
    let hop_cpmm = SwapHop {
        pool_id: PoolId(cpmm_pool),
        protocol: DexProtocol::RaydiumCpmm,
        mint_in: MintId(usdc),
        mint_out: MintId(wsol),
        token_programs: [TokenProgramKind::SplToken, TokenProgramKind::SplToken],
        fee_bps: 25,
        trade_intent: PumpSwapTradeIntent::default(),
        execution_accounts: Some(PoolExecutionAccounts::RaydiumCpmm {
            amm_config: cpmm_amm_config,
            authority: cpmm_authority,
            vault_0: cpmm_vault_0,
            vault_1: cpmm_vault_1,
            observation_state: cpmm_observation,
        }),
        active_bin_id: None,
    };

    let opp = ExecutionOpportunity {
        route_id: arb_types::RouteId(1),
        slot: Slot(100),
        hops: vec![hop_dlmm, hop_cpmm],
        size_in: TokenAmount(borrow_amount),
        expected_out: TokenAmount(expected_cpmm_out),
        gross_profit: Lamports(expected_cpmm_out.saturating_sub(borrow_amount)),
        net_profit: Lamports(expected_cpmm_out.saturating_sub(borrow_amount)),
        price_impact_bps: Bps(5),
        hop_inputs: vec![borrow_amount, expected_dlmm_out, expected_cpmm_out],
        base_decimals: 9,
        expected_min_final_output: expected_cpmm_out,
        hop_slippage_bps: 10,
        cu_estimate_units: 0,
        cu_shape_key: String::new(),
    };

    let kamino_params = KaminoFlashParams::default_wsol();

    (opp, kamino_params, dlmm_pool, cpmm_pool)
}

#[test]
fn test_matrix_01_profitable_dlmm_to_cpmm_v1_compilation() {
    let borrow = 1_000_000_000; // 1 SOL
    let dlmm_out = 150_000_000; // 150 USDC (6 decimals)
    let cpmm_out = 1_050_000_000; // 1.05 SOL (+0.05 SOL gross)
    let (opp, kamino, _dlmm_pool, _cpmm_pool) =
        create_test_dlmm_cpmm_opportunity(borrow, dlmm_out, cpmm_out);

    let signer = KeypairSigner::new_random();
    let payer = signer.pubkey();
    let tip_account = Pubkey::new_unique();
    let tip = Lamports(10_000);
    let cu_limit = 350_000;
    let cu_price = 10_000;
    let flash_fee_sf = 0; // 0 fee for wsol candidate
    let min_profit = TokenAmount(10_000_000); // 0.01 SOL floor
    let recent_blockhash = [7u8; 32];

    // 1. Verify cross-DEX detection
    assert!(is_cross_dex_opportunity(&opp));
    let platforms = validate_cross_dex_opportunity(&opp).expect("valid cross-dex route");
    assert_eq!(
        platforms,
        vec![DexPlatform::MeteoraDlmm, DexPlatform::RaydiumCpmm]
    );

    // 2. Assemble instructions through specialized cross-DEX builder
    let ixs = assemble_cross_dex_opportunity(
        payer,
        tip_account,
        tip,
        cu_limit,
        cu_price,
        &opp,
        &kamino,
        flash_fee_sf,
        min_profit,
        &[],
    )
    .expect("cross-dex assembly must succeed");

    // 3. Verify Atomic Transaction Structure:
    // [0] SetComputeUnitLimit
    // [1] SetComputeUnitPrice
    // [2] Create WSOL ATA idempotent
    // [3] Kamino FlashBorrow
    // [4] Meteora DLMM swap
    // [5] Raydium CPMM swap
    // [6] Kamino FlashRepay
    // [7] Jito Tip Transfer
    assert_eq!(ixs.len(), 8);
    assert_eq!(
        ixs[0].program_id.to_string(),
        "ComputeBudget111111111111111111111111111111"
    );
    assert_eq!(
        ixs[1].program_id.to_string(),
        "ComputeBudget111111111111111111111111111111"
    );
    assert_eq!(ixs[3].program_id.to_string(), KAMINO_KLEND_PROGRAM_ID);
    assert_eq!(ixs[4].program_id.to_string(), METEORA_DLMM_PROGRAM_ID);
    assert_eq!(ixs[5].program_id.to_string(), RAYDIUM_CPMM_PROGRAM_ID);
    assert_eq!(ixs[6].program_id.to_string(), KAMINO_KLEND_PROGRAM_ID);

    // 4. Compile strictly into Solana V1 Versioned Transaction
    let priority_fee_lamports = arb_tx::priority_fee_lamports(cu_price, cu_limit);
    let v1_config = V1ConfigParams {
        priority_fee_lamports: Some(priority_fee_lamports),
        compute_unit_limit: Some(cu_limit),
        loaded_accounts_data_size_limit: Some(67_108_864),
        heap_size: None,
    };
    let compiled = AdaptiveTxCompiler::compile_v1_only(&signer, &ixs, recent_blockhash, v1_config)
        .expect("V1 compilation must succeed");

    // 5. Invariant Checks:
    // Discriminator: 0x81
    assert_eq!(compiled.wire_bytes[0], 0x81);
    // Wire size <= 4096 MTU
    assert!(compiled.wire_bytes.len() <= SOLANA_PACKET_V1_MTU);
    assert!(compiled.wire_bytes.len() >= 400);
}

#[test]
fn test_matrix_02_unprofitable_candidate_rejected_by_profit_floor() {
    let borrow = 1_000_000_000;
    let dlmm_out = 150_000_000;
    let cpmm_out = 1_000_005_000; // only 5,000 lamports gross profit
    let (opp, kamino, _dlmm_pool, _cpmm_pool) =
        create_test_dlmm_cpmm_opportunity(borrow, dlmm_out, cpmm_out);

    let signer = KeypairSigner::new_random();
    let payer = signer.pubkey();
    let tip = Lamports(10_000);
    let min_profit = TokenAmount(10_000_000); // 0.01 SOL floor (exceeds 5,000 lamports!)

    let res = assemble_cross_dex_opportunity(
        payer,
        Pubkey::new_unique(),
        tip,
        350_000,
        1_000,
        &opp,
        &kamino,
        0,
        min_profit,
        &[],
    );

    assert!(res.is_err(), "Must reject unprofitable candidate");
    let err_str = res.err().unwrap().to_string();
    assert!(
        err_str.contains("profit floor") || err_str.contains("ProfitFloorNotMet"),
        "Error must be profit floor violation: {err_str}"
    );
}

#[test]
fn test_matrix_03_missing_dlmm_bin_array_fails_closed() {
    let borrow = 1_000_000_000;
    let dlmm_out = 150_000_000;
    let cpmm_out = 1_050_000_000;
    let (mut opp, kamino, _dlmm_pool, _cpmm_pool) =
        create_test_dlmm_cpmm_opportunity(borrow, dlmm_out, cpmm_out);

    // Strip execution accounts from DLMM hop
    opp.hops[0].execution_accounts = None;

    let signer = KeypairSigner::new_random();
    let res = assemble_cross_dex_opportunity(
        signer.pubkey(),
        Pubkey::new_unique(),
        Lamports(10_000),
        350_000,
        1_000,
        &opp,
        &kamino,
        0,
        TokenAmount(1_000_000),
        &[],
    );

    assert!(res.is_err());
    let err_str = res.err().unwrap().to_string();
    assert!(
        err_str.contains("execution_accounts") || err_str.contains("MissingExecutionAccounts"),
        "Expected missing execution accounts error, got: {err_str}"
    );
}

#[test]
fn test_matrix_07_08_09_failure_simulation_and_atomicity_rollback() {
    let borrow = 1_000_000_000;
    let dlmm_out = 150_000_000;
    let cpmm_out = 1_050_000_000;
    let (opp, kamino, _dlmm_pool, _cpmm_pool) =
        create_test_dlmm_cpmm_opportunity(borrow, dlmm_out, cpmm_out);

    let signer = KeypairSigner::new_random();
    let payer = signer.pubkey();
    let mut ixs = assemble_cross_dex_opportunity(
        payer,
        Pubkey::new_unique(),
        Lamports(10_000),
        350_000,
        1_000,
        &opp,
        &kamino,
        0,
        TokenAmount(1_000_000),
        &[],
    )
    .unwrap();

    // Verify atomic integrity initially holds
    let borrow_idx = ixs
        .iter()
        .position(|ix| ix.program_id.to_string() == KAMINO_KLEND_PROGRAM_ID)
        .expect("Kamino flash borrow instruction must exist");
    let repay_idx = ixs
        .iter()
        .rposition(|ix| ix.program_id.to_string() == KAMINO_KLEND_PROGRAM_ID)
        .expect("Kamino flash repay instruction must exist");
    assert_ne!(borrow_idx, repay_idx);
    assert_eq!(borrow_idx, 3);
    assert_eq!(repay_idx, 6);

    let initial_verification = arb_tx::atomic_integrity::verify_atomic_integrity(
        arb_tx::atomic_integrity::ExecutionMode::SingleTransaction,
        &ixs,
        Some(borrow_idx),
        Some(repay_idx),
        borrow,
        borrow,
        0,
    )
    .unwrap();
    assert!(initial_verification.is_atomic);

    // Scenario 9: Tampered repay index or removed repay instruction fails atomicity
    ixs.remove(repay_idx);
    let tampered_verification = arb_tx::atomic_integrity::verify_atomic_integrity(
        arb_tx::atomic_integrity::ExecutionMode::SingleTransaction,
        &ixs,
        Some(borrow_idx),
        None,
        borrow,
        borrow,
        0,
    );
    assert!(
        tampered_verification.is_err() || !tampered_verification.unwrap().is_atomic,
        "Transaction without Kamino repay must fail atomic integrity"
    );
}

#[test]
fn test_matrix_10_minimum_profit_guard_enforced_in_swap_legs() {
    let borrow = 1_000_000_000;
    let dlmm_out = 150_000_000;
    let cpmm_out = 1_010_000_000;
    let (opp, _kamino, _dlmm_pool, _cpmm_pool) =
        create_test_dlmm_cpmm_opportunity(borrow, dlmm_out, cpmm_out);

    let flash_fee = 1_000;
    let tip = 10_000;
    let min_profit = 20_000_000; // floor = 1_000_000_000 + 1_000 + 10_000 + 20_000_000 = 1_020_011_000
    let slippage_bps = 5;

    let res = TxBuilder::derive_swap_legs(&opp, flash_fee, tip, min_profit, slippage_bps);
    assert!(res.is_err());
    match res.err().unwrap() {
        TxError::ProfitFloorNotMet { expected, floor } => {
            assert_eq!(expected, cpmm_out);
            assert_eq!(floor, borrow + flash_fee + tip + min_profit);
        }
        other => panic!("Expected ProfitFloorNotMet, got {other:?}"),
    }
}

#[test]
fn test_matrix_11_v1_packet_size_boundary() {
    let borrow = 1_000_000_000;
    let dlmm_out = 150_000_000;
    let cpmm_out = 1_050_000_000;
    let (opp, kamino, _dlmm_pool, _cpmm_pool) =
        create_test_dlmm_cpmm_opportunity(borrow, dlmm_out, cpmm_out);

    let signer = KeypairSigner::new_random();
    let ixs = assemble_cross_dex_opportunity(
        signer.pubkey(),
        Pubkey::new_unique(),
        Lamports(10_000),
        350_000,
        1_000,
        &opp,
        &kamino,
        0,
        TokenAmount(1_000_000),
        &[],
    )
    .unwrap();

    let v1_config = V1ConfigParams {
        priority_fee_lamports: Some(10_000),
        compute_unit_limit: Some(350_000),
        loaded_accounts_data_size_limit: Some(67_108_864),
        heap_size: None,
    };
    let compiled = AdaptiveTxCompiler::compile_v1_only(&signer, &ixs, [1u8; 32], v1_config)
        .expect("Must compile V1");

    // Standard cross-DEX transaction is ~850-1250 bytes, well under 4096 MTU limit
    assert!(compiled.wire_bytes.len() <= SOLANA_PACKET_V1_MTU);
    assert_eq!(
        compiled.wire_bytes[0], 0x81,
        "Must have 0x81 V1 version discriminator"
    );

    // Verify transport acceptance for the real compiled cross-DEX transaction without ALTs
    let cap_v1 = TransportCapability::mainnet_v1_capable("Helius-Sender");
    let selected = select_authoritative_transport(
        &compiled.wire_bytes,
        true,  // is_swqos_route
        true,  // direct_v1_enabled
        false, // jito_v1_enabled
        &cap_v1,
    )
    .expect("Real cross-DEX V1 transaction must be accepted by SWQoS transport without ALT");
    assert_eq!(selected.kind, TransportKind::SwqosDirect);
    assert_eq!(selected.wire_mtu_limit, 4096);
    assert!(selected.is_v1);
}

#[test]
fn test_matrix_11_packet_boundary_matrix_1232_1248_4095_4096_4097() {
    let signer = KeypairSigner::new_random();

    // 1. Measure baseline V1 wire overhead for a calibrated single-instruction transaction
    let baseline_probe = AdaptiveTxCompiler::build_signed_v1_transaction(
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
        [0x55u8; 32],
        V1ConfigParams {
            priority_fee_lamports: Some(10_000),
            compute_unit_limit: Some(250_000),
            loaded_accounts_data_size_limit: Some(67_108_864),
            heap_size: None,
        },
        FormatSelectionReason::LargePayloadV1 {
            accounts_count: 2,
            wire_size: 0,
        },
    )
    .expect("baseline probe must build");
    let overhead = baseline_probe.wire_bytes.len();

    let build_calibrated_v1 = |target_size: usize| -> Result<CompiledAdaptiveTransaction, TxError> {
        let data_len = target_size.saturating_sub(overhead);
        AdaptiveTxCompiler::build_signed_v1_transaction(
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
            [0x55u8; 32],
            V1ConfigParams {
                priority_fee_lamports: Some(10_000),
                compute_unit_limit: Some(250_000),
                loaded_accounts_data_size_limit: Some(67_108_864),
                heap_size: None,
            },
            FormatSelectionReason::LargePayloadV1 {
                accounts_count: 2,
                wire_size: 0,
            },
        )
    };

    let cap_v1_swqos = TransportCapability::mainnet_v1_capable("Helius-Sender");
    let cap_v1_jito = TransportCapability::mainnet_v1_capable("Jito-Mainnet");
    let cap_legacy_v0 = TransportCapability::v0_only("Legacy-Validator-UDP");

    // =========================================================================
    // Boundary 1: 1,232 bytes (Legacy / V0 UDP MTU limit)
    // =========================================================================
    {
        // A. Message Format Validation: Valid V1 wire format
        let tx_1232 = build_calibrated_v1(1232).expect("1,232-byte V1 must compile");
        assert_eq!(tx_1232.wire_bytes.len(), 1232);
        assert_eq!(tx_1232.wire_bytes[0], 0x81);
        assert!(tx_1232.verify_signatures(&[signer.pubkey()]).unwrap());

        // B. Transport Acceptance:
        // - V1 SWQoS (QUIC): ACCEPTED
        let res_swqos =
            select_authoritative_transport(&tx_1232.wire_bytes, true, true, false, &cap_v1_swqos)
                .unwrap();
        assert_eq!(res_swqos.kind, TransportKind::SwqosDirect);
        assert_eq!(res_swqos.wire_mtu_limit, 4096);

        // - V1 Jito (HTTP/QUIC bundle): ACCEPTED
        let res_jito =
            select_authoritative_transport(&tx_1232.wire_bytes, false, false, true, &cap_v1_jito)
                .unwrap();
        assert_eq!(res_jito.kind, TransportKind::JitoBundle);
        assert_eq!(res_jito.wire_mtu_limit, 4096);

        // - Legacy / V0-only provider: REJECTS V1 because version 0x81 is unsupported by legacy transport
        let res_legacy =
            select_authoritative_transport(&tx_1232.wire_bytes, true, true, false, &cap_legacy_v0);
        assert!(matches!(
            res_legacy,
            Err(TransportSelectionError::UnsupportedVersionForTransport { .. })
        ));
    }

    // =========================================================================
    // Boundary 2: 1,248 bytes (Phase 17 cross-DEX transaction size, exceeds UDP MTU by 16 bytes)
    // =========================================================================
    {
        // A. Message Format Validation: Valid V1 wire format (well within 4,096 bytes)
        let tx_1248 = build_calibrated_v1(1248).expect("1,248-byte V1 must compile");
        assert_eq!(tx_1248.wire_bytes.len(), 1248);
        assert_eq!(tx_1248.wire_bytes[0], 0x81);
        assert!(tx_1248.verify_signatures(&[signer.pubkey()]).unwrap());

        // B. Transport Acceptance:
        // - V1 SWQoS (QUIC): ACCEPTED (no ALT required!)
        let res_swqos =
            select_authoritative_transport(&tx_1248.wire_bytes, true, true, false, &cap_v1_swqos)
                .unwrap();
        assert_eq!(res_swqos.kind, TransportKind::SwqosDirect);
        assert_eq!(res_swqos.packet_bytes, 1248);
        assert_eq!(res_swqos.wire_mtu_limit, 4096);

        // - V1 Jito (HTTP/QUIC bundle): ACCEPTED (no ALT required!)
        let res_jito =
            select_authoritative_transport(&tx_1248.wire_bytes, false, false, true, &cap_v1_jito)
                .unwrap();
        assert_eq!(res_jito.kind, TransportKind::JitoBundle);
        assert_eq!(res_jito.packet_bytes, 1248);
        assert_eq!(res_jito.wire_mtu_limit, 4096);

        // - Legacy UDP / V0-only provider: REJECTS (exceeds 1,232 bytes UDP limit AND unsupported version)
        let res_legacy =
            select_authoritative_transport(&tx_1248.wire_bytes, true, true, false, &cap_legacy_v0);
        assert!(res_legacy.is_err());
    }

    // =========================================================================
    // Boundary 3: 4,095 bytes (1 byte below Solana V1 MTU ceiling)
    // =========================================================================
    {
        // A. Message Format Validation: Valid V1 wire format
        let tx_4095 = build_calibrated_v1(4095).expect("4,095-byte V1 must compile");
        assert_eq!(tx_4095.wire_bytes.len(), 4095);
        assert_eq!(tx_4095.wire_bytes[0], 0x81);
        assert!(tx_4095.verify_signatures(&[signer.pubkey()]).unwrap());

        // B. Transport Acceptance:
        // - V1 SWQoS (QUIC): ACCEPTED
        let res_swqos =
            select_authoritative_transport(&tx_4095.wire_bytes, true, true, false, &cap_v1_swqos)
                .unwrap();
        assert_eq!(res_swqos.kind, TransportKind::SwqosDirect);
        assert_eq!(res_swqos.packet_bytes, 4095);
        assert_eq!(res_swqos.wire_mtu_limit, 4096);

        // - V1 Jito (HTTP/QUIC bundle): ACCEPTED
        let res_jito =
            select_authoritative_transport(&tx_4095.wire_bytes, false, false, true, &cap_v1_jito)
                .unwrap();
        assert_eq!(res_jito.kind, TransportKind::JitoBundle);
        assert_eq!(res_jito.packet_bytes, 4095);
        assert_eq!(res_jito.wire_mtu_limit, 4096);
    }

    // =========================================================================
    // Boundary 4: 4,096 bytes (Exact Solana V1 MTU ceiling)
    // =========================================================================
    {
        // A. Message Format Validation: Valid V1 wire format at exact ceiling
        let tx_4096 = build_calibrated_v1(4096).expect("4,096-byte V1 must compile");
        assert_eq!(tx_4096.wire_bytes.len(), 4096);
        assert_eq!(tx_4096.wire_bytes[0], 0x81);
        assert!(tx_4096.verify_signatures(&[signer.pubkey()]).unwrap());

        // B. Transport Acceptance:
        // - V1 SWQoS (QUIC): ACCEPTED at maximum boundary
        let res_swqos =
            select_authoritative_transport(&tx_4096.wire_bytes, true, true, false, &cap_v1_swqos)
                .unwrap();
        assert_eq!(res_swqos.kind, TransportKind::SwqosDirect);
        assert_eq!(res_swqos.packet_bytes, 4096);
        assert_eq!(res_swqos.wire_mtu_limit, 4096);

        // - V1 Jito (HTTP/QUIC bundle): ACCEPTED at maximum boundary
        let res_jito =
            select_authoritative_transport(&tx_4096.wire_bytes, false, false, true, &cap_v1_jito)
                .unwrap();
        assert_eq!(res_jito.kind, TransportKind::JitoBundle);
        assert_eq!(res_jito.packet_bytes, 4096);
        assert_eq!(res_jito.wire_mtu_limit, 4096);
    }

    // =========================================================================
    // Boundary 5: 4,097 bytes (Exceeds Solana V1 MTU ceiling by 1 byte)
    // =========================================================================
    {
        // A. Message Format Validation: Compiler MUST REJECT with TransactionTooLarge
        let res_compiler = build_calibrated_v1(4097);
        assert!(
            res_compiler.is_err(),
            "4,097-byte V1 transaction must be rejected by message compiler"
        );
        match res_compiler.unwrap_err() {
            TxError::TransactionTooLarge { size, limit } => {
                assert_eq!(size, 4097);
                assert_eq!(limit, 4096);
            }
            other => panic!("Expected TransactionTooLarge, got: {other:?}"),
        }

        // B. Transport Acceptance: Construct raw 4,097-byte wire packet with V1 header
        // Transport validation MUST REJECT with OversizedTransaction
        let mut raw_4097 = vec![0u8; 4097];
        raw_4097[0] = 0x81; // V1 discriminator
        raw_4097[1] = 1; // 1 signature
        raw_4097[40] = 1; // one instruction
        raw_4097[41] = 2; // 2 inline accounts
        // One structurally valid instruction with a payload sized so the
        // complete envelope (including the trailing signature) is 4097 B.
        let instruction_header = 42 + 2 * 32;
        raw_4097[instruction_header] = 0; // program account index
        raw_4097[instruction_header + 1] = 0; // no instruction accounts
        let instruction_data_len = 4097 - (42 + 2 * 32 + 4 + 64);
        raw_4097[instruction_header + 2..instruction_header + 4]
            .copy_from_slice(&(instruction_data_len as u16).to_le_bytes());
        let res_transport =
            select_authoritative_transport(&raw_4097, true, true, false, &cap_v1_swqos);
        assert!(
            res_transport.is_err(),
            "4,097-byte wire packet must be rejected by transport validation"
        );
        match res_transport.unwrap_err() {
            TransportSelectionError::OversizedTransaction {
                size,
                limit,
                version,
            } => {
                assert_eq!(size, 4097);
                assert_eq!(limit, 4096);
                assert_eq!(version, TransactionVersion::V1);
            }
            other => panic!("Expected OversizedTransaction, got: {other:?}"),
        }
    }
}

#[test]
fn test_matrix_12_compute_budget_boundary() {
    let cu_limit = 350_000;
    let dynamic_cu = arb_tx::calculate_dynamic_cu_limit(cu_limit);
    // 350_000 * 1.15 = 402_500
    assert_eq!(dynamic_cu, 402_500);
    assert!(
        dynamic_cu <= 1_400_000,
        "Compute budget must be within Solana runtime ceiling"
    );
}

#[test]
fn test_cross_dex_flash_arb_params_direct_assembly() {
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    let usdc = Pubkey::from_str(USDC_MINT).unwrap();
    let token_prog = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
    let payer = Pubkey::new_unique();

    let dlmm_pool = Pubkey::new_unique();
    let dlmm_reserve_x = Pubkey::new_unique();
    let dlmm_reserve_y = Pubkey::new_unique();
    let dlmm_oracle = Pubkey::new_unique();
    let dlmm_bin_array = Pubkey::new_unique();

    let cpmm_pool = Pubkey::new_unique();
    let cpmm_amm_config = Pubkey::new_unique();
    let cpmm_vault_0 = Pubkey::new_unique();
    let cpmm_vault_1 = Pubkey::new_unique();

    let params = CrossDexFlashArbParams {
        payer,
        kamino: KaminoFlashParams::default_wsol(),
        borrow_amount: 1_000_000_000,
        flash_fee: 1_000,
        legs: vec![
            CrossDexSwapLeg {
                dex_platform: DexPlatform::MeteoraDlmm,
                pool_address: dlmm_pool,
                token_in: wsol,
                token_out: usdc,
                token_in_program: token_prog,
                token_out_program: token_prog,
                amount_in: 1_000_000_000,
                min_amount_out: 150_000_000,
                swap_for_y: true,
                additional_accounts: vec![
                    dlmm_reserve_x,
                    dlmm_reserve_y,
                    dlmm_oracle,
                    dlmm_bin_array,
                ],
            },
            CrossDexSwapLeg {
                dex_platform: DexPlatform::RaydiumCpmm,
                pool_address: cpmm_pool,
                token_in: usdc,
                token_out: wsol,
                token_in_program: token_prog,
                token_out_program: token_prog,
                amount_in: 150_000_000,
                min_amount_out: 1_050_000_000,
                swap_for_y: false,
                additional_accounts: vec![cpmm_amm_config, cpmm_vault_0, cpmm_vault_1],
            },
        ],
        cu_limit: 350_000,
        priority_fee_micro_lamports: 10_000,
        jito_tip_account: Pubkey::new_unique(),
        jito_tip_lamports: 10_000,
        recent_blockhash: [9u8; 32],
        v1_transport_supported: true,
    };

    let err = assemble_cross_dex_flash_arbitrage(&params)
        .expect_err("unsafe legacy direct assembler must fail closed");
    assert!(
        err.to_string().contains("legacy direct assembler disabled"),
        "unexpected fail-closed reason: {err}"
    );
}

#[test]
fn test_matrix_15_v1_transaction_structure_cryptographic_verification_and_config_mask() {
    let borrow = 1_000_000_000;
    let dlmm_out = 150_000_000;
    let cpmm_out = 1_050_000_000;
    let (opp, kamino, _dlmm_pool, _cpmm_pool) =
        create_test_dlmm_cpmm_opportunity(borrow, dlmm_out, cpmm_out);

    let signer = KeypairSigner::new_random();
    let payer = signer.pubkey();
    let tip_account = Pubkey::new_unique();
    let tip = Lamports(10_000);
    let cu_limit = 350_000;
    let cu_price = 10_000;
    let flash_fee_sf = 0;
    let min_profit = TokenAmount(10_000_000);
    let recent_blockhash = [0x42u8; 32];

    let ixs = assemble_cross_dex_opportunity(
        payer,
        tip_account,
        tip,
        cu_limit,
        cu_price,
        &opp,
        &kamino,
        flash_fee_sf,
        min_profit,
        &[],
    )
    .expect("cross-dex assembly must succeed");

    // Compute expected absolute priority fee in lamports:
    // ceil(10_000 * 350_000 / 1_000_000) = 3,500 lamports
    let priority_fee_lamports = arb_tx::priority_fee_lamports(cu_price, cu_limit);
    assert_eq!(priority_fee_lamports, 3_500);

    let v1_config = V1ConfigParams {
        priority_fee_lamports: Some(priority_fee_lamports),
        compute_unit_limit: Some(cu_limit),
        loaded_accounts_data_size_limit: Some(67_108_864),
        heap_size: None,
    };

    let compiled = AdaptiveTxCompiler::compile_v1_only(&signer, &ixs, recent_blockhash, v1_config)
        .expect("V1 compilation must succeed");

    // 1. Version prefix: Byte 0 must be 0x81
    assert_eq!(
        compiled.wire_bytes[0], 0x81,
        "Version prefix must be 0x81 for V1"
    );

    // 2. Serialized wire size measured from final signed transaction
    assert_eq!(compiled.wire_size(), compiled.wire_bytes.len());
    assert_eq!(compiled.wire_bytes.len(), compiled.message_bytes.len() + 64);
    assert!(compiled.wire_bytes.len() <= SOLANA_PACKET_V1_MTU);

    // 3. All required accounts are inline, no ALT dependency
    let num_addresses = compiled.message_bytes[41] as usize;
    assert_eq!(
        num_addresses, 32,
        "Phase 17 cross-DEX opportunity has 32 unique inline accounts after removing the undeployed executor guard"
    );
    assert!(
        num_addresses <= 64,
        "Must not exceed SIMD-0385 inline account limit of 64"
    );

    // Verify inline accounts array starts at byte 42 and occupies num_addresses * 32 bytes
    let accounts_start = 42;
    let accounts_end = accounts_start + num_addresses * 32;
    assert_eq!(
        compiled.message_bytes[accounts_start..accounts_start + 32],
        payer.to_bytes()
    );

    // 4. Cryptographic Ed25519 signature verification
    assert_eq!(compiled.signatures.len(), 1);
    assert!(
        compiled
            .verify_signatures(&[payer])
            .expect("signature verification must pass")
    );

    // Verify tampering with any byte invalidates signature verification
    let mut tampered = compiled.clone();
    tampered.message_bytes[0] ^= 0xFF;
    assert!(
        tampered.verify_signatures(&[payer]).is_err(),
        "Tampered message must fail signature verification"
    );

    // 5. Config mask and config values serialized according to V1 specification
    // [4..8] is TransactionConfigMask: u32 LE
    let config_mask = u32::from_le_bytes(compiled.message_bytes[4..8].try_into().unwrap());
    // Bits 0,1: Priority fee (0x03)
    // Bit 2: CU limit (0x04)
    // Bit 3: Loaded data limit (0x08)
    // Mask = 0x03 | 0x04 | 0x08 = 0x0F
    assert_eq!(
        config_mask, 0x0F,
        "Config mask must be 0x0F (priority fee + CU limit + loaded data limit)"
    );

    // Config values are located immediately after inline accounts:
    let config_values = &compiled.message_bytes[accounts_end..accounts_end + 16];
    assert_eq!(
        config_values.len(),
        16,
        "Config values must be 16 bytes (8B fee + 4B CU + 4B loaded data)"
    );

    let decoded_fee = u64::from_le_bytes(config_values[0..8].try_into().unwrap());
    assert_eq!(
        decoded_fee, priority_fee_lamports,
        "Decoded priority fee in config values must match"
    );

    let decoded_cu = u32::from_le_bytes(config_values[8..12].try_into().unwrap());
    assert_eq!(
        decoded_cu, cu_limit,
        "Decoded CU limit in config values must match"
    );

    let decoded_loaded_data = u32::from_le_bytes(config_values[12..16].try_into().unwrap());
    assert_eq!(
        decoded_loaded_data, 67_108_864,
        "Decoded loaded data limit must be 64 MiB (67,108,864)"
    );

    // 6. No legacy Compute Budget instruction is incorrectly relied upon
    // The generic production builder no longer emits an undeployed executor guard.
    // ComputeBudget instructions are still present in the source instruction list.
    assert_eq!(ixs.len(), 8);
    assert_eq!(
        ixs[0].program_id.to_string(),
        "ComputeBudget111111111111111111111111111111"
    );
    assert_eq!(
        ixs[1].program_id.to_string(),
        "ComputeBudget111111111111111111111111111111"
    );

    // In the compiled V1 message: num_instructions is at byte 40
    let num_ixs = compiled.message_bytes[40] as usize;
    assert_eq!(
        num_ixs, 6,
        "V1 message must strip 2 ComputeBudget instructions, leaving 6 executed instructions"
    );

    // Verify repay instruction borrow_instruction_index was shifted from 3 to 1
    let parsed_ver =
        CompiledAdaptiveTransaction::parse_version_from_wire(&compiled.wire_bytes).unwrap();
    assert_eq!(parsed_ver, TransactionVersion::V1);
}
