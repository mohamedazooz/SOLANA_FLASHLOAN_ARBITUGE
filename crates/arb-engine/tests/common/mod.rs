//! Shared synthetic fixtures for the 3-leg Meteora DLMM pipeline tests.
#![allow(dead_code)]

use arb_types::{
    DexProtocol, DlmmBin, DlmmFeeParams, DlmmState, MintId, PoolExecutionAccounts, PoolId,
    PoolKind, PoolState, Slot, TokenProgramKind,
};
use solana_pubkey::Pubkey;
use std::str::FromStr;

pub fn wsol_mint() -> MintId {
    MintId(Pubkey::from_str(arb_constants::WSOL_MINT).unwrap())
}
pub fn meme_a_mint() -> MintId {
    MintId(Pubkey::new_from_array([90u8; 32]))
}
pub fn meme_b_mint() -> MintId {
    MintId(Pubkey::new_from_array([91u8; 32]))
}
pub fn spl() -> Pubkey {
    Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap()
}

pub fn make_dlmm_pool(
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
        reserve_x,
        reserve_y,
        oracle,
        bin_array_pdas: vec![Pubkey::new_from_array([pool_bytes_id.wrapping_add(50); 32])],
        bitmap_extension: None,
        pool_token_x: base_mint.0,
        pool_token_y: quote_mint.0,
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
pub fn meme_pool_fixture() -> Vec<PoolState> {
    let wsol = wsol_mint();
    let ma = meme_a_mint();
    let mb = meme_b_mint();
    vec![
        make_dlmm_pool(1, -100, 10, 100, wsol, ma, 10_u128.pow(12)),
        make_dlmm_pool(2, -100, 10, 100, ma, mb, 10_u128.pow(12)),
        make_dlmm_pool(3, 400, 10, 100, mb, wsol, 10_u128.pow(12)),
    ]
}

pub fn params(input_size: u64) -> ThreeLegMemeContextParams {
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

use arb_engine::ThreeLegMemeContextParams;
use arb_tx::master_alt::derive_ata;
