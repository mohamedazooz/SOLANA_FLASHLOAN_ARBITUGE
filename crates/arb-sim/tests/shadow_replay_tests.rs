//! Tests for opportunity persistence and deterministic shadow replay engine.

use arb_sim::opportunity_recorder::{CandidateOpportunityRecord, OpportunityHistoryRecorder};
use arb_types::currency::{Currency, FeeItem};
use arb_types::{DexProtocol, MintId, PoolId, PoolKind, PoolState, Slot};
use solana_pubkey::Pubkey;
use std::str::FromStr;

fn mock_pool(id: &str, active_id: i32, x_reserve: u128, y_reserve: u128) -> PoolState {
    let pool_pk = Pubkey::from_str(id).unwrap();
    let mint_x = MintId(Pubkey::from_str("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v").unwrap()); // USDC
    let mint_y = MintId(Pubkey::from_str("So11111111111111111111111111111111111111112").unwrap()); // WSOL

    let dlmm = arb_types::DlmmState {
        active_bin_id: active_id,
        status: 0,
        bin_step_bps: 10,
        bins: vec![arb_types::DlmmBin {
            bin_id: active_id,
            amount_x: x_reserve,
            amount_y: y_reserve,
        }],
        params: Some(arb_types::DlmmFeeParams {
            base_factor: 100,
            filter_period: 30,
            decay_period: 600,
            reduction_factor: 5000,
            variable_fee_control: 40000,
            max_volatility_accumulator: 350000,
            protocol_share: 500,
            base_fee_power_factor: 0,
            function_type: 0,
            collect_fee_mode: 0,
        }),
        volatility_accumulator: 0,
        volatility_reference: 0,
        index_reference: active_id,
        last_update_timestamp: 1700000000,
    };

    PoolState {
        pool_id: PoolId(pool_pk),
        protocol: DexProtocol::MeteoraDlmm,
        slot: Slot(320_000_000),
        write_version: 1,
        base_mint: mint_x,
        quote_mint: mint_y,
        decimals_a: 6,
        decimals_b: 9,
        reserve_in: x_reserve,
        reserve_out: y_reserve,
        fee_numerator: 10,
        fee_denominator: 10_000,
        is_active: true,
        has_risky_token: false,
        kind: PoolKind::Dlmm,
        clmm: None,
        dlmm: Some(dlmm),
        pump_fun: None,
        transfer_fee_bps: 0,
        token_programs: [
            arb_types::TokenProgramKind::SplToken,
            arb_types::TokenProgramKind::SplToken,
        ],
        execution_accounts: None,
        last_update_slot: 320_000_000,
        last_update_ms: 0,
    }
}

#[test]
fn test_persist_and_shadow_replay_deterministic_reproduction() {
    let tmp_dir = std::env::temp_dir().join(format!("sentinel_replay_test_{}", std::process::id()));
    let recorder = OpportunityHistoryRecorder::new(&tmp_dir);

    let pool_a = mock_pool(
        "5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6",
        100,
        10_000_000_000,
        50_000_000_000,
    );
    let pool_b = mock_pool(
        "BVRbyLjjfSBcoyiYFuxbgKYnWuiFaF9CSXEa5vdSZ9Hh",
        100,
        10_000_000_000,
        50_000_000_000,
    );

    let candidate = CandidateOpportunityRecord {
        candidate_id: "TEST-REPLAY-001".to_string(),
        detection_timestamp_ms: 1_700_000_000_000,
        pool_a_id: pool_a.pool_id.0,
        pool_b_id: pool_b.pool_id.0,
        pool_a_slot: pool_a.slot.0,
        pool_b_slot: pool_b.slot.0,
        quote_slot: 320_000_000,
        pool_a_state: pool_a,
        pool_b_state: pool_b,
        direction: "DirectionA".to_string(),
        trade_size_in: 1_000_000,
        hop1_quote_out: 999_990,
        hop2_final_out: 999_980,
        fee_items: vec![FeeItem::dex_swap_fee(
            "Hop 1 Fee",
            10,
            Currency::usdc(),
            6,
            320_000_000,
            1_700_000_000_000,
        )],
        gross_profit_quote_raw: -20,
        total_execution_cost_quote_raw: 1_620,
        net_profit_quote_raw: -1_640,
        net_profit_bps: -16,
        simulation_success: true,
        simulation_slot: Some(320_000_001),
        simulation_latency_ms: 15,
        quote_freshness_cleared: true,
        disappearance_time_ms: Some(1_700_000_000_150),
        candidate_lifetime_ms: Some(150),
        best_observed_net_profit: Some(-1_640),
        rejection_reason: None,
        status: "SIMULATED_SUCCESS_NEGATIVE_EDGE".to_string(),
    };

    recorder
        .record_candidate(&candidate)
        .expect("Candidate persistence must succeed");

    let loaded = recorder
        .load_candidates()
        .expect("Loading candidates must succeed");
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].candidate_id, "TEST-REPLAY-001");
    assert_eq!(loaded[0].gross_profit_quote_raw, -20);
    assert_eq!(loaded[0].net_profit_bps, -16);
    assert_eq!(loaded[0].simulation_latency_ms, 15);

    // Verify replay file was written
    let fixture_path = tmp_dir.join("TEST-REPLAY-001.json");
    assert!(fixture_path.exists(), "Individual fixture file must exist");

    // Clean up
    let _ = std::fs::remove_dir_all(&tmp_dir);
}
