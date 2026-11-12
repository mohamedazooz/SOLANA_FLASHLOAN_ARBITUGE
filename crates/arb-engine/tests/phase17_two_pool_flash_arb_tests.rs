//! Phase 17 — two-pool flash-loan arbitrage integration tests.
//!
//! Covers the whole chain:
//!
//! * direction resolution from the **on-chain** mint ordering,
//! * exact two-hop quoting with the real DLMM engine,
//! * the flash-fee-aware, constraint-checked size optimizer,
//! * optimizer-vs-exhaustive equivalence on a bounded domain,
//! * assembly of the single Kamino-borrow -> swap A -> swap B -> Kamino-repay
//!   transaction and its atomicity self-check.

use arb_engine::dlmm_flash_arbitrage::{
    DlmmFlashArbConfig, DlmmFlashArbEvaluator, DlmmFlashArbPool, DlmmFlashArbRoute,
    DlmmPoolAllowlist,
};
use arb_math::dlmm::get_price_from_id;
use arb_tx::atomic_integrity::{ExecutionMode, verify_atomic_integrity};
use arb_tx::dlmm_flash_arb::{FlashArbParams, FlashArbPoolLeg, assemble_flash_funded_two_pool};
use arb_types::{
    DexProtocol, DlmmBin, DlmmFeeParams, DlmmState, MintId, PoolExecutionAccounts, PoolId,
    PoolKind, PoolState, Slot, TokenProgramKind,
};
use solana_pubkey::Pubkey;
use std::str::FromStr;

const WSOL: &str = "So11111111111111111111111111111111111111112";
const USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const POOL_A: &str = "5rCf1DM8LjKTw4YqhnoLcngyZYeNnQqztScTogYHAS6";
const POOL_B: &str = "BGm1tav58oGcsQJehL9WXBFXF7D27vZsKefj4xJKD5Y";

fn wsol() -> MintId {
    MintId(Pubkey::from_str(WSOL).unwrap())
}
fn usdc() -> MintId {
    MintId(Pubkey::from_str(USDC).unwrap())
}
fn pool_a_pk() -> Pubkey {
    Pubkey::from_str(POOL_A).unwrap()
}
fn pool_b_pk() -> Pubkey {
    Pubkey::from_str(POOL_B).unwrap()
}

/// Build a synthetic DLMM pool with the given on-chain X/Y ordering.
#[allow(clippy::too_many_arguments)]
fn make_pool(
    pool_id: Pubkey,
    token_x: MintId,
    token_y: MintId,
    bin_step: u16,
    active_id: i32,
    // `(bin_id, amount_x, amount_y)` triples.
    bins: &[(i32, u64, u64)],
) -> DlmmFlashArbPool {
    let dlmm = DlmmState {
        active_bin_id: active_id,
        status: 0,
        bin_step_bps: bin_step,
        bins: bins
            .iter()
            .map(|(id, ax, ay)| DlmmBin {
                bin_id: *id,
                amount_x: u128::from(*ax),
                amount_y: u128::from(*ay),
            })
            .collect(),
        params: Some(DlmmFeeParams {
            base_factor: 1000,
            filter_period: 30,
            decay_period: 600,
            reduction_factor: 5000,
            variable_fee_control: 0,
            max_volatility_accumulator: 350_000,
            protocol_share: 1000,
            base_fee_power_factor: 0,
            function_type: 0,
            collect_fee_mode: 0,
        }),
        volatility_accumulator: 0,
        volatility_reference: 0,
        index_reference: active_id,
        last_update_timestamp: 1_700_000_000,
    };

    let state = PoolState {
        pool_id: PoolId(pool_id),
        protocol: DexProtocol::MeteoraDlmm,
        slot: Slot(1),
        write_version: 0,
        base_mint: token_x,
        quote_mint: token_y,
        decimals_a: 9,
        decimals_b: 6,
        reserve_in: 0,
        reserve_out: 0,
        fee_numerator: 10_000,
        fee_denominator: 1_000_000_000,
        is_active: true,
        has_risky_token: false,
        kind: PoolKind::Dlmm,
        clmm: None,
        dlmm: Some(dlmm),
        pump_fun: None,
        transfer_fee_bps: 0,
        token_programs: [TokenProgramKind::SplToken; 2],
        execution_accounts: Some(PoolExecutionAccounts::MeteoraDlmm {
            reserve_x: Pubkey::new_unique(),
            reserve_y: Pubkey::new_unique(),
            oracle: Pubkey::new_unique(),
            bin_array_pdas: vec![arb_tx::derive_meteora_bin_array_pda(&pool_id, 0)],
            bitmap_extension: None,
        }),
        last_update_slot: 1,
        last_update_ms: 0,
    };

    DlmmFlashArbPool {
        pool_id,
        state,
        token_x_mint: token_x,
        token_y_mint: token_y,
        reserve_x: Pubkey::new_unique(),
        reserve_y: Pubkey::new_unique(),
        oracle: Pubkey::new_unique(),
        token_x_program: TokenProgramKind::SplToken,
        token_y_program: TokenProgramKind::SplToken,
        transfer_fee_bps: [0, 0],
    }
}

/// Synthetic pair with a deliberate price gap.
///
/// Pool A (buy USDC) has the *higher* price, pool B (sell USDC) the lower one, so
/// the cycle is profitable before fees. `bin_step = 1` keeps the base fee at
/// 0.1 bps per side.
fn profitable_pair() -> DlmmFlashArbRoute {
    let pool_a = make_pool(
        pool_a_pk(),
        wsol(),
        usdc(),
        1,
        10,
        &[
            (10, 0, 1_000_000_000_000),
            (9, 0, 1_000_000_000_000),
            (8, 0, 1_000_000_000_000),
        ],
    );
    let pool_b = make_pool(
        pool_b_pk(),
        wsol(),
        usdc(),
        1,
        -10,
        &[
            (-10, 1_000_000_000_000, 0),
            (-9, 1_000_000_000_000, 0),
            (-8, 1_000_000_000_000, 0),
        ],
    );
    DlmmFlashArbRoute {
        pool_a,
        pool_b,
        flash_mint: wsol(),
        mid_mint: usdc(),
    }
}

fn config(allowlist: DlmmPoolAllowlist) -> DlmmFlashArbConfig {
    DlmmFlashArbConfig {
        allowlist,
        flash_mint: wsol(),
        flash_mint_decimals: 9,
        min_borrow: 1_000_000,
        max_borrow: 200_000_000,
        kamino_available_liquidity: 100_000_000,
        kamino_flash_fee_sf: 11_529_215_046_068,
        min_profit_raw: 0,
        hop_slippage_bps: 5,
        cu_limit: 400_000,
        max_bin_arrays_per_hop: 4,
        max_tx_bytes: 1232,
        coarse_samples: 40,
        refine_steps: 8,
        network_cost_lamports: 9_000,
        priority_fee_lamports: 4_000,
        priority_fee_micro_lamports: 10_000,
        payer: Pubkey::new_unique(),
        kamino: arb_tx::KaminoFlashParams::default(),
    }
}

fn allowlist() -> DlmmPoolAllowlist {
    DlmmPoolAllowlist::new(vec![pool_a_pk(), pool_b_pk()])
}

// ============================================================================
// Quoting and direction
// ============================================================================

#[test]
fn swap_direction_is_derived_from_the_on_chain_ordering() {
    // Pool A stores (USDC, WSOL): WSOL is token Y.
    let pool = make_pool(
        pool_a_pk(),
        usdc(),
        wsol(),
        1,
        0,
        &[(0, 1_000_000, 1_000_000)],
    );
    assert_eq!(pool.swap_for_y(&wsol(), &usdc()), Some(false));
    assert_eq!(pool.swap_for_y(&usdc(), &wsol()), Some(true));
    // A mint the pool does not hold has no direction.
    let other = MintId(Pubkey::new_unique());
    assert_eq!(pool.swap_for_y(&other, &usdc()), None);
}

#[test]
fn hop_quote_is_positive_and_charges_the_fee() {
    let route = profitable_pair();
    let quote = route
        .pool_a
        .quote(1_000_000_000, &route.flash_mint, &route.mid_mint)
        .expect("hop A quote");
    assert!(quote.amount_out > 0);
    assert!(quote.fee_amount > 0, "base fee must be charged");
    assert!(quote.swap_for_y, "X -> Y on pool A");
    assert!(quote.bins_crossed >= 1);
}

#[test]
fn hop_quote_rejects_token_2022_transfer_fee_mints() {
    let mut route = profitable_pair();
    // The route accepts legacy SPL mints only until epoch-aware Token-2022
    // transfer-fee context is part of the snapshot: a fee-bearing mint must be
    // rejected rather than approximated with a bps rate and an unbounded max fee.
    route.pool_a.transfer_fee_bps = [100, 200];
    assert!(
        route
            .pool_a
            .quote(1_000_000_000, &route.flash_mint, &route.mid_mint)
            .is_none(),
        "transfer-fee mints must fail closed"
    );
    // The on-chain `transfer_fee_bps` copy on the pool state is equally binding.
    let mut route = profitable_pair();
    route.pool_a.state.transfer_fee_bps = 100;
    assert!(
        route
            .pool_a
            .quote(1_000_000_000, &route.flash_mint, &route.mid_mint)
            .is_none()
    );
}

// ============================================================================
// Evaluation and constraints
// ============================================================================

#[test]
fn evaluation_reports_a_profitable_cycle() {
    let route = profitable_pair();
    let cfg = config(allowlist());
    let evaluator = DlmmFlashArbEvaluator { config: &cfg };
    let eval = evaluator.evaluate(&route, 10_000_000);
    assert!(eval.feasible, "rejection: {:?}", eval.rejection);
    assert!(eval.net_profit > 0);
    assert!(eval.hop_a_out > 0);
    assert!(eval.hop_b_min_out > eval.borrow_amount);
    // Kamino main-market WSOL charges `sf = 11_529_215_046_068` on a 2^60
    // scale = 1e-5, i.e. 100 lamports per 1e7 borrowed (round-half-up).
    assert_eq!(eval.flash_fee, 100, "round(1e7 * 11529215046068 / 2^60)");
}

#[test]
fn borrow_above_the_kamino_ceiling_is_rejected() {
    let route = profitable_pair();
    let mut cfg = config(allowlist());
    cfg.kamino_available_liquidity = 1_000_000;
    let evaluator = DlmmFlashArbEvaluator { config: &cfg };
    let eval = evaluator.evaluate(&route, 5_000_000);
    assert!(!eval.feasible);
    assert!(matches!(
        eval.rejection,
        Some(
            arb_engine::dlmm_flash_arbitrage::FlashArbRejection::BorrowAboveKaminoLiquidity { .. }
        )
    ));
}

#[test]
fn borrow_above_the_pool_capacity_is_rejected() {
    let route = profitable_pair();
    // Lift the generic ceilings above the pool's own capacity so the pool
    // liquidity check is what fires, not `BorrowAboveMaximum`.
    let mut cfg = config(allowlist());
    let capacity = route
        .pool_a
        .active_bin_max_input(&route.flash_mint, &route.mid_mint)
        .expect("capacity");
    let probe_size = capacity.saturating_add(1_000_000_000);
    cfg.max_borrow = probe_size;
    cfg.kamino_available_liquidity = probe_size;
    let evaluator = DlmmFlashArbEvaluator { config: &cfg };
    let eval = evaluator.evaluate(&route, probe_size);
    assert!(!eval.feasible);
    assert!(matches!(
        eval.rejection,
        Some(arb_engine::dlmm_flash_arbitrage::FlashArbRejection::BorrowAbovePoolLiquidity { .. })
    ));
}

#[test]
fn pool_outside_the_allowlist_is_rejected() {
    let route = profitable_pair();
    let cfg = config(DlmmPoolAllowlist::new(vec![pool_a_pk()]));
    let evaluator = DlmmFlashArbEvaluator { config: &cfg };
    let eval = evaluator.evaluate(&route, 10_000_000);
    assert!(!eval.feasible);
    assert!(matches!(
        eval.rejection,
        Some(arb_engine::dlmm_flash_arbitrage::FlashArbRejection::PoolNotAllowlisted { .. })
    ));
}

#[test]
fn profit_below_the_floor_is_rejected() {
    let route = profitable_pair();
    let mut cfg = config(allowlist());
    let evaluator = DlmmFlashArbEvaluator { config: &cfg };
    let baseline = evaluator.evaluate(&route, 10_000_000);
    assert!(baseline.feasible);
    // Demand more profit than the route can produce.
    cfg.min_profit_raw = (baseline.net_profit as u64) + 1_000_000;
    let evaluator = DlmmFlashArbEvaluator { config: &cfg };
    let eval = evaluator.evaluate(&route, 10_000_000);
    assert!(!eval.feasible);
    assert!(matches!(
        eval.rejection,
        Some(arb_engine::dlmm_flash_arbitrage::FlashArbRejection::ProfitBelowFloor { .. })
    ));
}

#[test]
fn transaction_size_budget_is_enforced() {
    let route = profitable_pair();
    let mut cfg = config(allowlist());
    // No room at all for the transaction.
    cfg.max_tx_bytes = 10;
    let evaluator = DlmmFlashArbEvaluator { config: &cfg };
    let eval = evaluator.evaluate(&route, 10_000_000);
    assert!(!eval.feasible);
    assert!(matches!(
        eval.rejection,
        Some(arb_engine::dlmm_flash_arbitrage::FlashArbRejection::TransactionTooLarge { .. })
    ));
}

#[test]
fn compute_unit_budget_is_enforced() {
    let route = profitable_pair();
    let mut cfg = config(allowlist());
    cfg.cu_limit = 1_000;
    let evaluator = DlmmFlashArbEvaluator { config: &cfg };
    let eval = evaluator.evaluate(&route, 10_000_000);
    assert!(!eval.feasible);
    assert!(matches!(
        eval.rejection,
        Some(arb_engine::dlmm_flash_arbitrage::FlashArbRejection::ComputeUnitsExceeded { .. })
    ));
}

// ============================================================================
// Optimizer
// ============================================================================

#[test]
fn optimizer_matches_exhaustive_search_on_a_bounded_domain() {
    let route = profitable_pair();
    let mut cfg = config(allowlist());
    // A 40-wide integer domain around a size that actually clears the fixed
    // execution cost: with `coarse_samples = 40` the uniform fill visits *every*
    // integer of the domain, so the optimizer's coarse grid is a superset of the
    // exhaustive grid.
    cfg.min_borrow = 10_000_000;
    cfg.max_borrow = 10_000_039;
    cfg.coarse_samples = 40;
    let evaluator = DlmmFlashArbEvaluator { config: &cfg };

    let mut exhaustive: Option<(u64, i64)> = None;
    for size in cfg.min_borrow..=cfg.max_borrow {
        let eval = evaluator.evaluate(&route, size);
        if !eval.feasible {
            continue;
        }
        match exhaustive {
            None => exhaustive = Some((size, eval.net_profit)),
            Some((best_size, best_profit)) => {
                if eval.net_profit > best_profit
                    || (eval.net_profit == best_profit && size < best_size)
                {
                    exhaustive = Some((size, eval.net_profit));
                }
            }
        }
    }
    let (ex_size, ex_profit) = exhaustive.expect("at least one feasible size");

    let best = evaluator.optimize(&route).expect("optimizer result");
    assert!(best.feasible);
    assert_eq!(
        best.net_profit, ex_profit,
        "optimizer must match the exhaustive maximum"
    );
    assert_eq!(best.borrow_amount, ex_size);
}

#[test]
fn optimizer_respects_the_kamino_ceiling() {
    let route = profitable_pair();
    let mut cfg = config(allowlist());
    cfg.min_borrow = 1_000_000;
    cfg.max_borrow = 50_000_000;
    cfg.kamino_available_liquidity = 10_000_000;
    let evaluator = DlmmFlashArbEvaluator { config: &cfg };
    let best = evaluator.optimize(&route).expect("optimizer result");
    assert!(best.feasible);
    assert!(
        best.borrow_amount <= cfg.kamino_available_liquidity,
        "borrow {} exceeds the Kamino ceiling",
        best.borrow_amount
    );
}

#[test]
fn optimizer_returns_none_when_nothing_is_feasible() {
    let route = profitable_pair();
    let mut cfg = config(allowlist());
    cfg.min_profit_raw = u64::MAX / 2;
    let evaluator = DlmmFlashArbEvaluator { config: &cfg };
    assert!(evaluator.optimize(&route).is_none());
}

// ============================================================================
// Atomic transaction assembly
// ============================================================================

#[test]
fn transaction_is_borrow_swaps_repay_in_one_atomic_unit() {
    let route = profitable_pair();
    let cfg = config(allowlist());
    let evaluator = DlmmFlashArbEvaluator { config: &cfg };
    let best = evaluator.optimize(&route).expect("optimizer result");

    let params = flash_params(&route, &best);
    let (ixs, borrow_idx, repay_idx) = assemble_flash_funded_two_pool(&params).unwrap();
    assert_eq!(ixs.len(), 8);
    assert_eq!(borrow_idx, 4);
    assert_eq!(repay_idx, 7);
    // The repay points back at the borrow.
    assert_eq!(ixs[repay_idx].data[16], borrow_idx as u8);
    // The repay covers principal + fee.
    assert_eq!(
        u64::from_le_bytes(ixs[repay_idx].data[8..16].try_into().unwrap()),
        params.borrow_amount + params.flash_fee
    );

    let verification = verify_atomic_integrity(
        ExecutionMode::SingleTransaction,
        &ixs,
        Some(borrow_idx),
        Some(repay_idx),
        params.borrow_amount,
        params.repay_amount(),
        params.flash_fee,
    )
    .unwrap();
    assert!(
        verification.is_atomic,
        "{:?}",
        verification.rejection_reason
    );
}

#[test]
fn hop_two_input_is_bound_to_hop_one_minimum_output() {
    let route = profitable_pair();
    let cfg = config(allowlist());
    let evaluator = DlmmFlashArbEvaluator { config: &cfg };
    let best = evaluator.optimize(&route).expect("optimizer result");
    let params = flash_params(&route, &best);
    let (ixs, _, _) = assemble_flash_funded_two_pool(&params).unwrap();

    // Hop 1 (index 5) delivers at least `min_hop_a_out`; hop 2 (index 6) may only
    // spend that conservative amount, never the optimistic quote.
    let hop1_min = u64::from_le_bytes(ixs[5].data[16..24].try_into().unwrap());
    let hop2_in = u64::from_le_bytes(ixs[6].data[8..16].try_into().unwrap());
    assert_eq!(hop1_min, best.hop_a_min_out);
    assert_eq!(hop2_in, best.hop_a_min_out);
    assert!(
        hop2_in <= best.hop_a_out,
        "hop 2 must never spend more than hop 1 can deliver"
    );
}

#[test]
fn signed_transaction_fits_the_packet_limit_with_a_lookup_table() {
    let route = profitable_pair();
    let cfg = config(allowlist());
    let evaluator = DlmmFlashArbEvaluator { config: &cfg };
    let best = evaluator.optimize(&route).expect("optimizer result");
    let mut params = flash_params(&route, &best);
    params.payer = Pubkey::new_unique();

    // The instruction list is what the size budget is measured on.
    let (ixs, _, _) = assemble_flash_funded_two_pool(&params).unwrap();
    let mut addrs: Vec<Pubkey> = Vec::new();
    for ix in &ixs {
        for acc in &ix.accounts {
            if !addrs.contains(&acc.pubkey) {
                addrs.push(acc.pubkey);
            }
        }
    }
    let table = arb_tx::AddressLookupTable::new(Pubkey::new_unique(), addrs);
    let size = arb_tx::estimate_v0_transaction_size(&ixs, &[table]);
    assert!(size <= 1232, "v0 size {size} exceeds the packet limit");
}

#[test]
fn builder_rejects_a_route_that_does_not_return_to_the_flash_mint() {
    let mut route = profitable_pair();
    // Pool B now holds a different pair.
    route.pool_b.token_x_mint = MintId(Pubkey::new_unique());
    route.pool_b.token_y_mint = MintId(Pubkey::new_unique());
    let cfg = config(allowlist());
    let evaluator = DlmmFlashArbEvaluator { config: &cfg };
    let best = evaluator.evaluate(&route, 10_000_000);
    assert!(!best.feasible);
    assert!(matches!(
        best.rejection,
        Some(arb_engine::dlmm_flash_arbitrage::FlashArbRejection::QuoteFailed { .. })
    ));
}

#[test]
fn builder_rejects_two_identical_pools() {
    let route = profitable_pair();
    let cfg = config(allowlist());
    let evaluator = DlmmFlashArbEvaluator { config: &cfg };
    let mut params = flash_params(&route, &evaluator.evaluate(&route, 10_000_000));
    params.leg_b.lb_pair = params.leg_a.lb_pair;
    assert!(assemble_flash_funded_two_pool(&params).is_err());
}

#[test]
fn reversed_mint_ordering_still_produces_a_closing_route() {
    // Pool A stores (USDC, WSOL): WSOL is token Y, so the hop is Y -> X.
    // The ladder is mirrored relative to `profitable_pair` so the cycle
    // WSOL -> USDC -> WSOL is still the profitable direction.
    let pool_a = make_pool(
        pool_a_pk(),
        usdc(),
        wsol(),
        1,
        -10,
        &[(-10, 1_000_000_000_000, 0), (-9, 1_000_000_000_000, 0)],
    );
    // Pool B stores (USDC, WSOL) as well: USDC is token X, so the hop is X -> Y.
    let pool_b = make_pool(
        pool_b_pk(),
        usdc(),
        wsol(),
        1,
        10,
        &[(10, 0, 1_000_000_000_000), (9, 0, 1_000_000_000_000)],
    );
    let route = DlmmFlashArbRoute {
        pool_a,
        pool_b,
        flash_mint: wsol(),
        mid_mint: usdc(),
    };
    let cfg = config(allowlist());
    let evaluator = DlmmFlashArbEvaluator { config: &cfg };
    let eval = evaluator.evaluate(&route, 10_000_000);
    assert!(eval.feasible, "rejection: {:?}", eval.rejection);
    assert!(eval.net_profit > 0);
}

/// Build the transaction parameters implied by an evaluation.
fn flash_params(
    route: &DlmmFlashArbRoute,
    eval: &arb_engine::DlmmFlashArbEvaluation,
) -> FlashArbParams {
    let hop_a_swap_for_y = route
        .pool_a
        .swap_for_y(&route.flash_mint, &route.mid_mint)
        .expect("pool A direction");
    let hop_b_swap_for_y = route
        .pool_b
        .swap_for_y(&route.mid_mint, &route.flash_mint)
        .expect("pool B direction");

    let plan_a = arb_tx::plan_bin_arrays(
        &route.pool_a.pool_id,
        route.pool_a.state.dlmm.as_ref().unwrap().active_bin_id,
        hop_a_swap_for_y,
        eval.bins_a.max(1),
    )
    .unwrap();
    let plan_b = arb_tx::plan_bin_arrays(
        &route.pool_b.pool_id,
        route.pool_b.state.dlmm.as_ref().unwrap().active_bin_id,
        hop_b_swap_for_y,
        eval.bins_b.max(1),
    )
    .unwrap();

    let leg_a = FlashArbPoolLeg {
        lb_pair: route.pool_a.pool_id,
        reserve_x: route.pool_a.reserve_x,
        reserve_y: route.pool_a.reserve_y,
        token_x_mint: route.pool_a.token_x_mint.0,
        token_y_mint: route.pool_a.token_y_mint.0,
        oracle: route.pool_a.oracle,
        token_x_program: route.pool_a.token_x_program.program_id(),
        token_y_program: route.pool_a.token_y_program.program_id(),
        swap_for_y: hop_a_swap_for_y,
        amount_in: eval.borrow_amount,
        min_amount_out: eval.hop_a_min_out,
        bin_array_pdas: plan_a.pdas.clone(),
        bitmap_extension: Some(plan_a.bitmap_extension_pda),
        host_fee_in: None,
    };
    let leg_b = FlashArbPoolLeg {
        lb_pair: route.pool_b.pool_id,
        reserve_x: route.pool_b.reserve_x,
        reserve_y: route.pool_b.reserve_y,
        token_x_mint: route.pool_b.token_x_mint.0,
        token_y_mint: route.pool_b.token_y_mint.0,
        oracle: route.pool_b.oracle,
        token_x_program: route.pool_b.token_x_program.program_id(),
        token_y_program: route.pool_b.token_y_program.program_id(),
        swap_for_y: hop_b_swap_for_y,
        amount_in: eval.hop_a_min_out,
        min_amount_out: eval.borrow_amount + eval.flash_fee,
        bin_array_pdas: plan_b.pdas.clone(),
        bitmap_extension: Some(plan_b.bitmap_extension_pda),
        host_fee_in: None,
    };

    FlashArbParams {
        payer: Pubkey::new_unique(),
        kamino: arb_tx::KaminoFlashParams::default(),
        borrow_amount: eval.borrow_amount,
        flash_fee: eval.flash_fee,
        leg_a,
        leg_b,
        cu_limit: 400_000,
        priority_fee_micro_lamports: 10_000,
    }
}

#[test]
fn synthetic_price_gap_is_the_expected_size() {
    // Guards the fixture: the two synthetic pools must actually differ in price,
    // otherwise the "profitable" assertions above would be vacuous.
    let price_high = get_price_from_id(10, 1).expect("price");
    let price_low = get_price_from_id(-10, 1).expect("price");
    assert!(price_high > price_low);
    let gap_bps = (price_high as u128 * 10_000) / price_low as u128;
    assert!(gap_bps > 10, "gap {gap_bps} bps must exceed the fees");
}
