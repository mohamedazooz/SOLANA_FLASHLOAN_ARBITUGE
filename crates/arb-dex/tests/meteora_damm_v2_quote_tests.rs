use arb_dex::engines::MeteoraDammV2QuoteEngine;
use arb_dex::quote_engine::{QuoteEngine, QuoteRequest};
use arb_types::{
    DammV2State, DexProtocol, MintId, PoolExecutionAccounts, PoolId, PoolKind, PoolState, Slot,
    TokenProgramKind,
};
use solana_pubkey::Pubkey;

const FEE_NUMERATOR: u128 = 100_000;
const FEE_DENOMINATOR: u128 = 1_000_000_000;
const AMOUNT_IN: u64 = 100_000;

fn pool(collect_fee_mode: u8) -> PoolState {
    let pool_id = PoolId(Pubkey::new_unique());
    let mint_a = MintId(Pubkey::new_unique());
    let mint_b = MintId(Pubkey::new_unique());
    let authority = Pubkey::new_unique();
    let vault_a = Pubkey::new_unique();
    let vault_b = Pubkey::new_unique();
    let event_authority = Pubkey::new_unique();
    let is_compounding = collect_fee_mode == 2;

    let damm = DammV2State {
        pool_status: 0,
        fee_version: 0,
        layout_version: 1,
        collect_fee_mode,
        activation_type: 1,
        activation_point: 1_800_000_000,
        current_point: 1_800_000_100,
        current_timestamp: 1_800_000_100,
        clock_slot: 123,
        clock_epoch: 1_097,
        sqrt_price_x64: 1u128 << 64,
        liquidity: 1_000_000_000_000,
        sqrt_min_price_x64: 1u128 << 63,
        sqrt_max_price_x64: 1u128 << 65,
        token_a_amount: Some(10_000_000),
        token_b_amount: Some(20_000_000),
        cliff_fee_numerator: FEE_NUMERATOR as u64,
        fee_scheduler_mode: 2,
        number_of_period: 0,
        period_frequency: 0,
        reduction_factor: 0,
        sqrt_price_step_bps: 0,
        scheduler_expiration_duration: 0,
        init_sqrt_price_x64: 1u128 << 64,
        protocol_fee_percent: 10,
        referral_fee_percent: 25,
        compounding_fee_bps: 2_000,
        dynamic_fee_initialized: false,
        max_volatility_accumulator: 0,
        variable_fee_control: 0,
        bin_step: 0,
        filter_period: 0,
        decay_period: 0,
        dynamic_reduction_factor: 0,
        last_update_timestamp: 0,
        bin_step_u128: 0,
        sqrt_price_reference: 0,
        volatility_accumulator: 0,
        volatility_reference: 0,
        account_graph_verified: true,
    };

    PoolState {
        pool_id,
        protocol: DexProtocol::MeteoraDammV2,
        slot: Slot(123),
        write_version: 1,
        base_mint: mint_a,
        quote_mint: mint_b,
        decimals_a: 9,
        decimals_b: 6,
        reserve_in: 10_000_000,
        reserve_out: 20_000_000,
        fee_numerator: FEE_NUMERATOR as u64,
        fee_denominator: FEE_DENOMINATOR as u64,
        raydium_cpmm_fee: None,
        is_active: true,
        has_risky_token: false,
        kind: if is_compounding {
            PoolKind::Cpmm
        } else {
            PoolKind::Clmm
        },
        clmm: None,
        dlmm: None,
        pump_fun: None,
        damm_v2: Some(damm),
        transfer_fee_bps: 0,
        token_programs: [TokenProgramKind::SplToken; 2],
        token2022_fee_context: None,
        execution_accounts: Some(PoolExecutionAccounts::MeteoraDammV2 {
            token_a_mint: mint_a.0,
            token_b_mint: mint_b.0,
            authority,
            vault_a,
            vault_b,
            event_authority,
        }),
        last_update_slot: 123,
        last_update_ms: 1,
    }
}

fn request(input: MintId, output: MintId) -> QuoteRequest {
    QuoteRequest {
        input_mint: input,
        output_mint: output,
        amount: AMOUNT_IN,
        slippage_bps: 100,
        input_transfer_fee_config: None,
        output_transfer_fee_config: None,
    }
}

fn fee_ceil(amount: u64) -> u64 {
    u64::try_from((u128::from(amount) * FEE_NUMERATOR).div_ceil(FEE_DENOMINATOR)).unwrap()
}

fn assert_fee_split_reconciles(quote: &arb_dex::quote_engine::QuoteResult) {
    let split = &quote.fees_breakdown;
    assert_eq!(
        split.lp_fee
            + split.compounding_fee
            + split.protocol_fee
            + split.fund_fee
            + split.creator_fee
            + split.referral_fee,
        split.total_fee,
        "fee allocations must partition one pool fee exactly"
    );
}

#[test]
fn both_token_charges_output_fee_in_both_directions_and_tags_fee_mint() {
    let engine = MeteoraDammV2QuoteEngine::new();
    let state = pool(0);
    let mint_a = state.base_mint;
    let mint_b = state.quote_mint;

    for (input, output, a_to_b) in [(mint_a, mint_b, true), (mint_b, mint_a, false)] {
        let quote = engine
            .quote_exact_in(&state, &request(input, output))
            .unwrap();
        let damm = state.damm_v2.as_ref().unwrap();
        let curve_output = arb_math::quote_damm_v2_exact_in_with_fee_mode(
            AMOUNT_IN,
            a_to_b,
            damm.sqrt_price_x64,
            damm.liquidity,
            damm.sqrt_min_price_x64,
            damm.sqrt_max_price_x64,
            0,
            false,
        )
        .unwrap();
        let fee = fee_ceil(curve_output);
        assert_eq!(quote.amount_out_raw, curve_output - fee);
        assert_eq!(quote.total_fee_mint, output);
        assert_eq!(quote.fees_breakdown.total_fee, fee);
        assert_eq!(
            quote.minimum_amount_out_raw,
            (quote.amount_out_raw * 9_900) / 10_000
        );
        assert_fee_split_reconciles(&quote);
    }
}

#[test]
fn only_b_charges_output_for_a_to_b_and_input_for_b_to_a() {
    let engine = MeteoraDammV2QuoteEngine::new();
    let state = pool(1);
    let mint_a = state.base_mint;
    let mint_b = state.quote_mint;

    let a_to_b = engine
        .quote_exact_in(&state, &request(mint_a, mint_b))
        .unwrap();
    let damm = state.damm_v2.as_ref().unwrap();
    let a_to_b_gross = arb_math::quote_damm_v2_exact_in_with_fee_mode(
        AMOUNT_IN,
        true,
        damm.sqrt_price_x64,
        damm.liquidity,
        damm.sqrt_min_price_x64,
        damm.sqrt_max_price_x64,
        0,
        false,
    )
    .unwrap();
    let a_to_b_fee = fee_ceil(a_to_b_gross);
    assert_eq!(a_to_b.amount_out_raw, a_to_b_gross - a_to_b_fee);
    assert_eq!(a_to_b.total_fee_mint, mint_b);
    assert_eq!(a_to_b.fees_breakdown.total_fee, a_to_b_fee);

    let b_to_a = engine
        .quote_exact_in(&state, &request(mint_b, mint_a))
        .unwrap();
    let input_fee = fee_ceil(AMOUNT_IN);
    let curve_input = AMOUNT_IN - input_fee;
    let b_to_a_gross = arb_math::quote_damm_v2_exact_in_with_fee_mode(
        curve_input,
        false,
        damm.sqrt_price_x64,
        damm.liquidity,
        damm.sqrt_min_price_x64,
        damm.sqrt_max_price_x64,
        0,
        false,
    )
    .unwrap();
    assert_eq!(b_to_a.amount_out_raw, b_to_a_gross);
    assert_eq!(b_to_a.total_fee_mint, mint_b);
    assert_eq!(b_to_a.fees_breakdown.total_fee, input_fee);
    assert_fee_split_reconciles(&a_to_b);
    assert_fee_split_reconciles(&b_to_a);
}

#[test]
fn compounding_uses_v1_tracked_reserves_and_directional_fee_side() {
    let engine = MeteoraDammV2QuoteEngine::new();
    let state = pool(2);
    let mint_a = state.base_mint;
    let mint_b = state.quote_mint;

    let a_to_b = engine
        .quote_exact_in(&state, &request(mint_a, mint_b))
        .unwrap();
    let gross_a_to_b = arb_math::quote_damm_v2_compounding_exact_in(
        AMOUNT_IN, true, 10_000_000, 20_000_000, 0, false,
    )
    .unwrap();
    let output_fee = fee_ceil(gross_a_to_b);
    assert_eq!(a_to_b.amount_out_raw, gross_a_to_b - output_fee);
    assert_eq!(a_to_b.total_fee_mint, mint_b);

    let b_to_a = engine
        .quote_exact_in(&state, &request(mint_b, mint_a))
        .unwrap();
    let input_fee = fee_ceil(AMOUNT_IN);
    let gross_b_to_a = arb_math::quote_damm_v2_compounding_exact_in(
        AMOUNT_IN - input_fee,
        false,
        10_000_000,
        20_000_000,
        0,
        false,
    )
    .unwrap();
    assert_eq!(b_to_a.amount_out_raw, gross_b_to_a);
    assert_eq!(b_to_a.total_fee_mint, mint_b);
    assert_eq!(b_to_a.fees_breakdown.total_fee, input_fee);
    assert_fee_split_reconciles(&a_to_b);
    assert_fee_split_reconciles(&b_to_a);
}

#[test]
fn rejects_clock_slot_mismatch_or_unverified_account_graph() {
    let engine = MeteoraDammV2QuoteEngine::new();
    let mut state = pool(0);
    state.slot = Slot(124);
    assert!(
        engine
            .quote_exact_in(&state, &request(state.base_mint, state.quote_mint))
            .is_err()
    );

    let mut state = pool(0);
    state.damm_v2.as_mut().unwrap().account_graph_verified = false;
    assert!(
        engine
            .quote_exact_in(&state, &request(state.base_mint, state.quote_mint))
            .is_err()
    );
}

#[test]
fn rejects_token_2022_even_when_request_contains_no_fee_schedule() {
    let engine = MeteoraDammV2QuoteEngine::new();
    let mut state = pool(0);
    state.token_programs[0] = TokenProgramKind::Token2022;
    assert!(
        engine
            .quote_exact_in(&state, &request(state.base_mint, state.quote_mint))
            .is_err()
    );
}

#[test]
fn rejects_compounding_v0_and_missing_verified_swap_accounts() {
    let engine = MeteoraDammV2QuoteEngine::new();
    let mut state = pool(2);
    state.damm_v2.as_mut().unwrap().layout_version = 0;
    state.damm_v2.as_mut().unwrap().token_a_amount = None;
    state.damm_v2.as_mut().unwrap().token_b_amount = None;
    assert!(
        engine
            .quote_exact_in(&state, &request(state.base_mint, state.quote_mint))
            .is_err()
    );

    let mut state = pool(0);
    state.execution_accounts = None;
    assert!(
        engine
            .quote_exact_in(&state, &request(state.base_mint, state.quote_mint))
            .is_err()
    );
}
#[test]
fn dispatcher_routes_damm_v2_to_specialized_engine() {
    let dispatcher = arb_dex::engines::UniversalQuoteDispatcher::new();
    let state = pool(0);
    let quote = dispatcher
        .quote_exact_in(&state, &request(state.base_mint, state.quote_mint))
        .unwrap();
    assert_eq!(quote.protocol, DexProtocol::MeteoraDammV2);
    assert!(quote.quote_validity);
    assert!(quote.amount_out_raw > 0);
}
#[test]
fn compounding_fee_is_separately_reported_for_compounding_pools() {
    let engine = MeteoraDammV2QuoteEngine::new();
    let state = pool(2);
    let quote = engine
        .quote_exact_in(&state, &request(state.base_mint, state.quote_mint))
        .unwrap();
    assert!(quote.fees_breakdown.compounding_fee > 0);
    assert_eq!(
        quote.fees_breakdown.lp_fee
            + quote.fees_breakdown.compounding_fee
            + quote.fees_breakdown.protocol_fee
            + quote.fees_breakdown.referral_fee,
        quote.fees_breakdown.total_fee
    );
}
#[test]
fn price_impact_is_positive_and_fee_excluded_for_both_curve_models() {
    let engine = MeteoraDammV2QuoteEngine::new();
    for mode in [0, 2] {
        let state = pool(mode);
        let quote = engine
            .quote_exact_in(&state, &request(state.base_mint, state.quote_mint))
            .unwrap();
        assert!(quote.price_impact.0 > 0);
        assert!(quote.price_impact.0 < 10_000);
    }
}
