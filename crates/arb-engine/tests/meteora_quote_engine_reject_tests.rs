//! QUOTE-001 / QUOTE-002 regression tests.
//!
//! Locks the Meteora DLMM cross-pool quote engine's fail-closed boundaries:
//!
//! - QUOTE-001 (ARB-003 class): a pool carrying a Token-2022 transfer fee, an
//!   active/unsupported pool status, or an unknown `collect_fee_mode` must be
//!   REJECTED outright. The engine must never price such a pool with an
//!   incomplete local model (the historical defect subtracted the NET return of
//!   `apply_transfer_fee_ceil` a second time and priced 1% of the input).
//!
//! - QUOTE-002 (ARB-009 class): `dlmm.status` and `fp.collect_fee_mode` come
//!   from account data; any value the local math does not model exactly is
//!   rejected (never defaulted to 0).

use arb_engine::MeteoraDlmmCrossPoolQuoteEngine;
use arb_types::{
    DexProtocol, DlmmBin, DlmmFeeParams, DlmmState, MintId, PoolExecutionAccounts, PoolId,
    PoolKind, PoolState, Slot, TokenProgramKind,
};
use solana_pubkey::Pubkey;

fn make_test_dlmm_pool(pool_byte: u8, mint_x_byte: u8, mint_y_byte: u8) -> PoolState {
    let pool_pk = Pubkey::new_from_array([pool_byte; 32]);
    let active_bin_id = 1_000_i32;
    let reserve_x = Pubkey::new_from_array([10 + pool_byte; 32]);
    let reserve_y = Pubkey::new_from_array([20 + pool_byte; 32]);
    let mint_x = MintId(Pubkey::new_from_array([mint_x_byte; 32]));
    let mint_y = MintId(Pubkey::new_from_array([mint_y_byte; 32]));

    let bins = vec![
        DlmmBin {
            bin_id: active_bin_id - 1,
            amount_x: 200_000_000_000,
            amount_y: 0,
            ..Default::default()
        },
        DlmmBin {
            bin_id: active_bin_id,
            amount_x: 100_000_000_000,
            amount_y: 100_000_000_000,
            ..Default::default()
        },
        DlmmBin {
            bin_id: active_bin_id + 1,
            amount_x: 0,
            amount_y: 200_000_000_000,
            ..Default::default()
        },
    ];

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
        bin_step_bps: 10,
        bins,
        params: Some(fee_params),
        volatility_accumulator: 0,
        volatility_reference: 0,
        index_reference: active_bin_id,
        last_update_timestamp: 1_700_000_000,
        support_limit_order: false,
    };

    let bin_array_index =
        arb_dex::engines::MeteoraDlmmQuoteEngine::bin_id_to_bin_array_index(active_bin_id);
    let bin_array_pda =
        arb_dex::engines::MeteoraDlmmQuoteEngine::derive_bin_array_pda(&pool_pk, bin_array_index).0;
    let exec = PoolExecutionAccounts::MeteoraDlmm {
        reserve_x,
        reserve_y,
        oracle: pool_pk,
        bin_array_pdas: vec![bin_array_pda],
        bitmap_extension: None,
        pool_token_x: mint_x.0,
        pool_token_y: mint_y.0,
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
        reserve_in: 300_000_000_000,
        reserve_out: 300_000_000_000,
        fee_numerator: 10,
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
fn valid_pool_quotes_then_transfer_fee_pool_is_rejected() {
    let pool_1 = make_test_dlmm_pool(1, 1, 2);
    let pool_2 = make_test_dlmm_pool(2, 2, 1);
    let amount_in = 1_000_000_000_u64;
    let start = pool_1.base_mint;
    let mid = pool_1.quote_mint;

    // Positive control: fee-free supported pools quote through both hops.
    let clean =
        MeteoraDlmmCrossPoolQuoteEngine::quote_two_hops(&pool_1, &pool_2, amount_in, start, mid);
    assert!(
        clean.is_some(),
        "valid fee-free supported pools must quote (QUOTE-001 positive control)"
    );

    // QUOTE-001: any non-zero transfer_fee_bps on either boundary pool is a
    // hard reject, never a re-derived quote.
    let mut pool_tf_in = pool_1.clone();
    pool_tf_in.transfer_fee_bps = 100;
    assert!(
        MeteoraDlmmCrossPoolQuoteEngine::quote_two_hops(
            &pool_tf_in,
            &pool_2,
            amount_in,
            start,
            mid
        )
        .is_none(),
        "input-side transfer fee must be rejected outright"
    );

    let mut pool_tf_out = pool_2.clone();
    pool_tf_out.transfer_fee_bps = 50;
    assert!(
        MeteoraDlmmCrossPoolQuoteEngine::quote_two_hops(
            &pool_1,
            &pool_tf_out,
            amount_in,
            start,
            mid
        )
        .is_none(),
        "output-side transfer fee must be rejected outright"
    );

    assert!(
        MeteoraDlmmCrossPoolQuoteEngine::quote_single_hop(&pool_tf_in, amount_in, start, mid)
            .is_none(),
        "single-hop quote must reject transfer-fee pools"
    );
    assert!(
        MeteoraDlmmCrossPoolQuoteEngine::build_dlmm_pool_and_bins(&pool_tf_in).is_none(),
        "pool conversion must reject transfer-fee pools"
    );
}

#[test]
fn nonzero_pool_status_is_rejected() {
    let pool_1 = make_test_dlmm_pool(1, 1, 2);
    let pool_2 = make_test_dlmm_pool(2, 2, 1);
    let amount_in = 1_000_000_000_u64;
    let start = pool_1.base_mint;
    let mid = pool_1.quote_mint;

    for status in [1_u8, 2, 7] {
        let mut pool = pool_1.clone();
        if let Some(dlmm) = pool.dlmm.as_mut() {
            dlmm.status = status;
        }
        // QUOTE-002: `dlmm.status != 0` (paused/permissioned/unknown live
        // states) must fail closed for discovery, quoting and conversion.
        assert!(
            MeteoraDlmmCrossPoolQuoteEngine::quote_single_hop(&pool, amount_in, start, mid)
                .is_none(),
            "status {status} must be rejected in single-hop quote"
        );
        assert!(
            MeteoraDlmmCrossPoolQuoteEngine::quote_two_hops(&pool, &pool_2, amount_in, start, mid)
                .is_none(),
            "status {status} must be rejected in two-hop quote"
        );
        assert!(
            MeteoraDlmmCrossPoolQuoteEngine::build_dlmm_pool_and_bins(&pool).is_none(),
            "status {status} must be rejected in pool conversion"
        );
    }
}

#[test]
fn unknown_collect_fee_mode_is_rejected() {
    let pool_1 = make_test_dlmm_pool(1, 1, 2);
    let pool_2 = make_test_dlmm_pool(2, 2, 1);
    let amount_in = 1_000_000_000_u64;
    let start = pool_1.base_mint;
    let mid = pool_1.quote_mint;

    for mode in [2_u8, 5, 255] {
        let mut pool = pool_1.clone();
        if let Some(dlmm) = pool.dlmm.as_mut() {
            if let Some(fp) = dlmm.params.as_mut() {
                fp.collect_fee_mode = mode;
            }
        }
        // QUOTE-002: an unmodeled `collect_fee_mode` would price the fee
        // distribution wrong; unknown values are rejected, never assumed to be
        // mode 0 or 1.
        assert!(
            MeteoraDlmmCrossPoolQuoteEngine::quote_single_hop(&pool, amount_in, start, mid)
                .is_none(),
            "collect_fee_mode {mode} must be rejected in single-hop quote"
        );
        assert!(
            MeteoraDlmmCrossPoolQuoteEngine::quote_two_hops(&pool, &pool_2, amount_in, start, mid)
                .is_none(),
            "collect_fee_mode {mode} must be rejected in two-hop quote"
        );
        assert!(
            MeteoraDlmmCrossPoolQuoteEngine::build_dlmm_pool_and_bins(&pool).is_none(),
            "collect_fee_mode {mode} must be rejected in pool conversion"
        );
    }
}

#[test]
fn missing_bin_array_snapshot_is_rejected() {
    let mut pool = make_test_dlmm_pool(9, 1, 2);
    if let Some(PoolExecutionAccounts::MeteoraDlmm { bin_array_pdas, .. }) =
        pool.execution_accounts.as_mut()
    {
        bin_array_pdas.clear();
    }
    assert!(
        MeteoraDlmmCrossPoolQuoteEngine::quote_single_hop(
            &pool,
            1_000_000,
            pool.base_mint,
            pool.quote_mint
        )
        .is_none(),
        "a local bin snapshot without its existence-proven BinArray account must not quote"
    );
}

#[test]
fn modeled_collect_fee_modes_are_accepted() {
    let pool_1 = make_test_dlmm_pool(1, 1, 2);
    let amount_in = 1_000_000_000_u64;
    let start = pool_1.base_mint;
    let mid = pool_1.quote_mint;

    for mode in [0_u8, 1] {
        let mut pool = pool_1.clone();
        if let Some(dlmm) = pool.dlmm.as_mut() {
            if let Some(fp) = dlmm.params.as_mut() {
                fp.collect_fee_mode = mode;
            }
        }
        assert!(
            MeteoraDlmmCrossPoolQuoteEngine::quote_single_hop(&pool, amount_in, start, mid)
                .is_some(),
            "collect_fee_mode {mode} is a modeled value and must quote"
        );
    }
}
