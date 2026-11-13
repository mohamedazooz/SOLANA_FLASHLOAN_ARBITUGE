//! Three-leg meme arbitrage pipeline integration test.
//!
//! Exercises the full connected path, not just the standalone compiler:
//!
//!   Discovery (MeteoraDlmmPoolPairDiscovery + chain resolution)
//!     -> Quote Engine (MeteoraDlmmCrossPoolQuoteEngine, discrete DLMM math)
//!     -> Trade Size Optimizer (three_leg_meme_pipeline::ThreeLegMemeSizeSearchResult)
//!     -> Account Resolver (PoolExecutionAccounts -> ThreeLegMemeArbitrageContext)
//!     -> Compiler (arb_tx::compile_3leg_meme_dlmm_arb, V0 via Static Master ALT)
//!     -> Profitability Gate (simulate_profitability_gate)
//!     -> Paper Journal (arb_sim::opportunity_recorder::OpportunityHistoryRecorder)
//!
//! Synthetic pools are used: no live mainnet pool set is required for the
//! deterministic fixture. A live mainnet simulation is intentionally NOT
//! performed (ARB_LIVE=0).

use std::str::FromStr;

use arb_engine::{
    MeteoraDlmmPoolPairDiscovery, ThreeLegMemeContextParams, build_three_leg_meme_pipeline,
    simulate_profitability_gate,
};
use arb_sim::opportunity_recorder::{CandidateOpportunityRecord, OpportunityHistoryRecorder};
use arb_tx::compile_3leg_meme_dlmm_arb;
use arb_tx::master_alt::{AddressCategory, StaticAddressEntry, StaticMasterAlt, derive_ata};
use arb_tx::{CompileResult, KeypairSigner};
use arb_types::currency::{Currency, FeeItem};
use arb_types::{
    DexProtocol, DlmmBin, DlmmFeeParams, DlmmState, MintId, PoolExecutionAccounts, PoolId,
    PoolKind, PoolState, Slot, TokenProgramKind,
};
use solana_pubkey::Pubkey;

fn wsol_mint() -> MintId {
    MintId(Pubkey::from_str(arb_constants::WSOL_MINT).unwrap())
}
fn meme_a_mint() -> MintId {
    MintId(Pubkey::new_from_array([90u8; 32]))
}
fn meme_b_mint() -> MintId {
    MintId(Pubkey::new_from_array([91u8; 32]))
}
fn spl() -> Pubkey {
    Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap()
}

fn make_dlmm_pool(
    pool_bytes_id: u8,
    active_bin_id: i32,
    bin_step: u16,
    base_factor: u16,
    base_mint: MintId,
    quote_mint: MintId,
    liquidity: u128,
) -> PoolState {
    let pool_pk = Pubkey::new_from_array([pool_bytes_id; 32]);
    let reserve_x = Pubkey::new_from_array([pool_bytes_id.wrapping_add(100); 32]);
    let reserve_y = Pubkey::new_from_array([pool_bytes_id.wrapping_add(150); 32]);
    let oracle = Pubkey::new_from_array([pool_bytes_id.wrapping_add(200); 32]);

    let bins = vec![
        DlmmBin {
            bin_id: active_bin_id - 1,
            amount_x: liquidity * 2,
            amount_y: 0,
        },
        DlmmBin {
            bin_id: active_bin_id,
            amount_x: liquidity,
            amount_y: liquidity,
        },
        DlmmBin {
            bin_id: active_bin_id + 1,
            amount_x: 0,
            amount_y: liquidity * 2,
        },
    ];

    let fee_params = DlmmFeeParams {
        base_factor,
        filter_period: 30,
        decay_period: 600,
        reduction_factor: 5000,
        variable_fee_control: 40_000,
        max_volatility_accumulator: 350_000,
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
        token_x_mint: base_mint.0,
        token_y_mint: quote_mint.0,
        reserve_x,
        reserve_y,
        oracle,
        bin_array_pdas: vec![Pubkey::new_from_array([pool_bytes_id.wrapping_add(50); 32])],
        bitmap_extension: None,
    };

    PoolState {
        pool_id: PoolId(pool_pk),
        protocol: DexProtocol::MeteoraDlmm,
        slot: Slot(467_000_000),
        write_version: 1,
        base_mint,
        quote_mint,
        decimals_a: 9,
        decimals_b: 6,
        reserve_in: liquidity * 3,
        reserve_out: liquidity * 3,
        fee_numerator: ((bin_step as u64) * (base_factor as u64) / 100).max(1),
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

/// Synthetic three-pool meme market: WSOL/MemeA (cheap A), MemeA/MemeB
/// (cheap B), MemeB/WSOL (rich WSOL). The chained route is profitable.
fn meme_pool_fixture() -> Vec<PoolState> {
    let wsol = wsol_mint();
    let ma = meme_a_mint();
    let mb = meme_b_mint();
    vec![
        make_dlmm_pool(1, -100, 10, 100, wsol, ma, 10_u128.pow(12)),
        make_dlmm_pool(2, -100, 10, 100, ma, mb, 10_u128.pow(12)),
        make_dlmm_pool(3, 400, 10, 100, mb, wsol, 10_u128.pow(12)),
    ]
}

fn params(input_size: u64) -> ThreeLegMemeContextParams {
    let payer = Pubkey::new_from_array([1u8; 32]);
    let ma = meme_a_mint();
    let mb = meme_b_mint();
    let ma_ata = derive_ata(&payer, &ma.0, &spl());
    let mb_ata = derive_ata(&payer, &mb.0, &spl());

    ThreeLegMemeContextParams {
        payer,
        meme_a_ata: ma_ata,
        meme_b_ata: mb_ata,
        meme_a_ata_exists: false,
        meme_b_ata_exists: false,
        meme_a_ata_balance: 0,
        meme_b_ata_balance: 0,
        meme_a_is_token_2022: false,
        meme_b_is_token_2022: false,
        meme_a_withheld_fees: 0,
        meme_b_withheld_fees: 0,
        wsol_token_program: spl(),
        meme_a_token_program: spl(),
        meme_b_token_program: spl(),
        snapshot_slot: 467_000_000,
        cu_limit: arb_constants::COMPUTE_BUDGET_LIMIT_3HOP,
        priority_fee_micro_lamports: 1_000,
        jito_tip_account: Pubkey::from_str(arb_constants::JITO_TIP_ACCOUNTS[0]).unwrap(),
        jito_tip_lamports: 10_000,
        recent_blockhash: [42u8; 32],
        v1_transport_supported: true,
        slippage_bps: 100,
        min_profit_lamports: 30_000,
        flash_loan_fee_sf: 0,
        network_cost_lamports: 0,
        kamino_available_liquidity: None,
        input_size,
        size_search_sizes: vec![input_size / 10, input_size / 2, input_size],
    }
}

#[test]
fn test_pipeline_discovery_quote_optimizer_resolver_chain() {
    let pools = meme_pool_fixture();

    // 1. Discovery runs and sees all three DLMM pools.
    let report = MeteoraDlmmPoolPairDiscovery::discover_pairs(&pools);
    assert_eq!(report.total_meteora_pools_discovered, 3);
    assert_eq!(report.active_and_valid_pools, 3);
    // Meme pools are all different token pairs -> no same-pair eligible pairs.
    // The 3-leg resolver does its own chain discovery (below).
    assert_eq!(report.distinct_eligible_pool_pairs, 0);

    // 2. Full pipeline: route graph -> quote -> size search -> account resolver.
    let params = params(1_000_000_000);
    let result = build_three_leg_meme_pipeline(&pools, &params).expect("pipeline must succeed");

    // Pools distinct and ordered WSOL -> MemeA -> MemeB -> WSOL.
    let [p1, p2, p3] = result.pool_ids;
    assert_ne!(p1, p2);
    assert_ne!(p2, p3);
    assert_ne!(p1, p3);

    // Exact mint transitions.
    let wsol = wsol_mint();
    let ma = meme_a_mint();
    let mb = meme_b_mint();
    assert_eq!(result.legs[0].from_mint, wsol);
    assert_eq!(result.legs[0].to_mint, ma);
    assert_eq!(result.legs[1].from_mint, ma);
    assert_eq!(result.legs[1].to_mint, mb);
    assert_eq!(result.legs[2].from_mint, mb);
    assert_eq!(result.legs[2].to_mint, wsol);

    // Quotes must be positive and directionally sane (all X -> Y here).
    assert!(result.legs[0].swap_for_y);
    assert!(result.legs[1].swap_for_y);
    assert!(result.legs[2].swap_for_y);
    for leg in &result.legs {
        assert!(leg.amount_out > 0, "leg {} produced zero output", leg.leg);
        assert_eq!(leg.bins_traversed, 0, "deep bins absorb the full trade");
    }
    assert_eq!(result.legs[0].amount_in, params.input_size);
    assert_eq!(result.legs[1].amount_in, result.legs[0].amount_out);
    assert_eq!(result.legs[2].amount_in, result.legs[1].amount_out);
    assert!(
        result.gross_profit_lamports > 0,
        "fixture route must be profitable"
    );

    // 3. Trade size optimizer: evaluated every configured candidate size.
    assert_eq!(result.size_search.tested_sizes.len(), 3);
    assert_eq!(result.size_search.best_input, params.input_size);
    assert!(result.size_search.best_net_profit_lamports > 0);

    // 4. Context sanity: pool order and mints wired, validated by the compiler.
    assert_eq!(result.context.pool_1.0, p1);
    assert_eq!(result.context.pool_2.0, p2);
    assert_eq!(result.context.pool_3.0, p3);
    assert_eq!(result.context.meme_a_mint, ma);
    assert_eq!(result.context.meme_b_mint, mb);
    assert_eq!(
        result.context.expected_min_final_output,
        result.legs[2].amount_out * (10_000 - params.slippage_bps) / 10_000
    );
    assert!(result.context.validate().is_ok());

    // Direction of legs in context matches quotes.
    assert!(matches!(
        result.context.leg_1_direction,
        arb_tx::meteora_dlmm::SwapDirection::XtoY
    ));
    assert!(matches!(
        result.context.leg_2_direction,
        arb_tx::meteora_dlmm::SwapDirection::XtoY
    ));
    assert!(matches!(
        result.context.leg_3_direction,
        arb_tx::meteora_dlmm::SwapDirection::XtoY
    ));

    // Bin arrays derived per leg (all inside default bitmap for these bins).
    for leg in [
        &result.context.leg_1_accounts,
        &result.context.leg_2_accounts,
        &result.context.leg_3_accounts,
    ] {
        assert_eq!(leg.bin_array_pdas.len(), 1);
    }

    // 5. Profitability gate passes for a profitable fixture route.
    let gross = simulate_profitability_gate(&result, params.min_profit_lamports)
        .expect("gate must accept profitable route");
    assert!(gross > 0);

    // 6. A gate that rejects unprofitable routes.
    let bad = simulate_profitability_gate(&result, (gross as u64) + 1);
    assert!(bad.is_err(), "gate must reject when floor not met");
}

#[test]
fn test_pipeline_to_compiler_v0_end_to_end() {
    let pools = meme_pool_fixture();
    let params = params(1_000_000_000);
    let pipeline = build_three_leg_meme_pipeline(&pools, &params).expect("pipeline must succeed");

    // Static Master ALT fixture: all required static entries + the static
    // bridge pool (pool 3) and its stable program-owned accounts.
    let payer = params.payer;
    let wsol = Pubkey::from_str(arb_constants::WSOL_MINT).unwrap();
    let usdc = Pubkey::from_str(arb_constants::USDC_MINT).unwrap();
    let usdt = Pubkey::from_str("Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB").unwrap();
    let bot_wsol_ata = derive_ata(&payer, &wsol, &spl());
    let bot_usdc_ata = derive_ata(&payer, &usdc, &spl());
    let bot_usdt_ata = derive_ata(&payer, &usdt, &spl());

    let mut entries = StaticMasterAlt::default_static_entries(
        &payer,
        Some(bot_wsol_ata),
        Some(bot_usdc_ata),
        Some(bot_usdt_ata),
        None,
        None,
    )
    .unwrap();
    // Pool 3 (Meme B / WSOL) is the verified static bridge pool: add it plus its
    // stable reserves and oracle as static bridge entries.
    let pool3 = &pools[2];
    if let PoolExecutionAccounts::MeteoraDlmm {
        reserve_x,
        reserve_y,
        oracle,
        ..
    } = pool3
        .execution_accounts
        .clone()
        .expect("pool3 execution accounts")
    {
        for address in [pool3.pool_id.0, reserve_x, reserve_y, oracle] {
            entries.push(StaticAddressEntry {
                category: AddressCategory::StaticBridgePool,
                address,
                owner_program: Some(
                    Pubkey::from_str(arb_constants::METEORA_DLMM_PROGRAM_ID).unwrap(),
                ),
                label: "Static bridge pool account",
            });
        }
    }
    let mut alt = StaticMasterAlt::new();
    alt.populate_from_entries(
        payer,
        Pubkey::new_from_array([2u8; 32]),
        entries,
        467_000_000,
    )
    .unwrap();
    alt.mark_activated(467_000_001);

    let signer = KeypairSigner::from_seed(&[7u8; 32]);
    let compiled = compile_3leg_meme_dlmm_arb(&pipeline.context, &alt, &signer)
        .expect("compilation must succeed");

    match &compiled {
        CompileResult::V0JitoCompatible(v0) => {
            // The prescribed compact fixture fits in a V0 transaction with the ALT.
            assert!(
                v0.wire_size <= 1232,
                "V0 wire size {} exceeds 1232",
                v0.wire_size
            );
            assert!(v0.signer_count >= 1);
            assert!(
                v0.alt_resolved_count > 0,
                "static accounts must resolve from ALT"
            );
            println!("=== three-leg pipeline V0 ===");
            println!("wire size: {} bytes", v0.wire_size);
            println!("{}", v0.byte_budget.summarize());
            // Distinct pools survive compilation through the context.
            assert_eq!(pipeline.context.pool_1.0, pipeline.pool_ids[0]);
            assert_eq!(pipeline.context.pool_2.0, pipeline.pool_ids[1]);
            assert_eq!(pipeline.context.pool_3.0, pipeline.pool_ids[2]);
        }
        CompileResult::V1DirectTransport(v1) => {
            // The fixture is expected to fit V0, but V1 remains a valid outcome
            // if ALT terms change; verify its limits.
            assert!(v1.wire_size <= 4096);
            eprintln!("V1 direct transport selected: {} bytes", v1.wire_size);
        }
        CompileResult::Rejected(reason) => {
            panic!("compiler rejected a valid route: {reason:?}");
        }
    }

    // Pools are distinct and exactly the route mints were used.
    assert_ne!(pools[0].pool_id.0, pools[1].pool_id.0);
}

#[test]
fn test_pipeline_to_paper_journal_roundtrip() {
    let pools = meme_pool_fixture();
    let params = params(1_000_000_000);
    let result = build_three_leg_meme_pipeline(&pools, &params).expect("pipeline must succeed");

    let gross = simulate_profitability_gate(&result, params.min_profit_lamports).unwrap();
    let net = gross - 9_000 - params.jito_tip_lamports as i64; // network + jito tip
    let pool_a_state = pools[0].clone();
    let pool_b_state = pools[2].clone();

    let candidate = CandidateOpportunityRecord {
        candidate_id: format!(
            "3LEG-{}-{}-{}",
            result.pool_ids[0], result.pool_ids[1], result.pool_ids[2]
        ),
        detection_timestamp_ms: 1_700_000_000_000,
        pool_a_id: pool_a_state.pool_id.0,
        pool_b_id: pool_b_state.pool_id.0,
        pool_a_slot: pool_a_state.slot.0,
        pool_b_slot: pool_b_state.slot.0,
        quote_slot: params.snapshot_slot,
        pool_a_state,
        pool_b_state,
        direction: "WSOL-MemeA-MemeB-WSOL(3LEG)".to_string(),
        trade_size_in: result.legs[0].amount_in,
        hop1_quote_out: result.legs[0].amount_out,
        hop2_final_out: result.legs[2].amount_out,
        fee_items: vec![
            FeeItem::dex_swap_fee(
                "Leg 1 (WSOL->MemeA)",
                result.legs[0].fee,
                Currency::wsol(),
                9,
                params.snapshot_slot,
                1_700_000_000_000,
            ),
            FeeItem::dex_swap_fee(
                "Leg 2 (MemeA->MemeB)",
                result.legs[1].fee,
                Currency::wsol(),
                9,
                params.snapshot_slot,
                1_700_000_000_000,
            ),
            FeeItem::dex_swap_fee(
                "Leg 3 (MemeB->WSOL)",
                result.legs[2].fee,
                Currency::wsol(),
                9,
                params.snapshot_slot,
                1_700_000_000_000,
            ),
            FeeItem::network_base_fee(9_000, params.snapshot_slot, 1_700_000_000_000),
        ],
        gross_profit_quote_raw: gross,
        total_execution_cost_quote_raw: 9_000 + params.jito_tip_lamports,
        net_profit_quote_raw: net,
        net_profit_bps: (net * 10_000) / result.legs[0].amount_in as i64,
        simulation_success: false, // never simulated or broadcast (ARB_LIVE=0)
        simulation_slot: None,
        simulation_latency_ms: 0,
        quote_freshness_cleared: false,
        disappearance_time_ms: None,
        candidate_lifetime_ms: None,
        best_observed_net_profit: Some(net),
        rejection_reason: Some("ARB_LIVE=0: paper-only record, no broadcast".to_string()),
        status: "paper".to_string(),
    };

    let log_dir =
        std::env::temp_dir().join(format!("arb_3leg_paper_journal_{}", std::process::id()));
    let recorder = OpportunityHistoryRecorder::new(&log_dir);
    recorder
        .record_candidate(&candidate)
        .expect("journal write must succeed");

    // Round-trip: the paper record is readable and preserves the accounting.
    let jsonl = std::fs::read_to_string(log_dir.join("candidates.jsonl")).expect("journal file");
    let lines: Vec<&str> = jsonl.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(!lines.is_empty(), "journal must contain the candidate");
    let parsed: serde_json::Value =
        serde_json::from_str(lines[0]).expect("journal line must parse");
    assert_eq!(
        parsed["candidate_id"],
        serde_json::json!(candidate.candidate_id)
    );
    assert_eq!(
        parsed["gross_profit_quote_raw"],
        serde_json::json!(candidate.gross_profit_quote_raw)
    );
    assert_eq!(parsed["net_profit_quote_raw"], serde_json::json!(net));
    assert_eq!(
        parsed["direction"],
        serde_json::json!("WSOL-MemeA-MemeB-WSOL(3LEG)")
    );
    assert_eq!(parsed["simulation_success"], serde_json::json!(false));
    assert_eq!(parsed["status"], serde_json::json!("paper"));
}

#[test]
fn test_pipeline_fails_closed_without_cycle() {
    // No pool touches WSOL -> no cycle.
    let wsol = wsol_mint();
    let _ = wsol;
    let usdc = Pubkey::from_str(arb_constants::USDC_MINT).unwrap();
    let pools = vec![
        make_dlmm_pool(11, 0, 10, 100, MintId(usdc), meme_a_mint(), 10_u128.pow(12)),
        make_dlmm_pool(
            12,
            0,
            10,
            100,
            meme_a_mint(),
            meme_b_mint(),
            10_u128.pow(12),
        ),
        make_dlmm_pool(13, 0, 10, 100, meme_b_mint(), MintId(usdc), 10_u128.pow(12)),
    ];
    let params = params(1_000_000_000);
    let err = build_three_leg_meme_pipeline(&pools, &params).unwrap_err();
    assert!(
        matches!(
            err,
            arb_engine::ThreeLegMemePipelineError::NoCycleFound { pools: 3 }
        ),
        "expected NoCycleFound, got {err:?}"
    );

    // All inactive -> NoEligiblePools.
    let mut inactive = meme_pool_fixture();
    for p in inactive.iter_mut() {
        p.is_active = false;
    }
    let err = build_three_leg_meme_pipeline(&inactive, &params).unwrap_err();
    assert!(
        matches!(err, arb_engine::ThreeLegMemePipelineError::NoEligiblePools),
        "expected NoEligiblePools, got {err:?}"
    );
}
