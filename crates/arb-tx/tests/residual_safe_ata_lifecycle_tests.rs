//! Regression suite for F-01 (P1): intermediate ATA residual balance and
//! CloseAccount failure.
//!
//! Root cause proven at HEAD: `TxBuilder::derive_swap_legs` clamps hop i+1's
//! input to hop i's guaranteed minimum output
//! (`amount_in = min(nominal_in, prev_min_out)`), while the pool can pay out
//! more than `min_out`. The surplus stays in the intermediate ATA, and the
//! builder used to emit an unconditional `CloseAccount` for every intermediate
//! ATA that was not pre-existing. Closing a non-empty SPL token account is a
//! program error (`InvalidAccountData` / account carries balance), which reverts
//! the WHOLE atomic transaction — after the Kamino repay — exactly when the
//! route beat its quote. Every favorable fill was turned into a failed tx.
//!
//! Fix verified here (Option B, safe fallback):
//! 1. The generic builder used by the Supervisor
//!    (`TxBuilder::assemble_flash_arbitrage` and the cross-dex entry
//!    `assemble_cross_dex_opportunity` that delegates to it) NEVER emits
//!    `CloseAccount` for an intermediate ATA by default.
//! 2. Closes are emitted ONLY for mints the caller explicitly proves end the
//!    route at a zero balance (`proven_zero_intermediate_mints`), and always
//!    AFTER the Kamino repay index.
//! 3. `derive_swap_legs` remains subsidy-proof: hop i+1 consumes no more than
//!    hop i's guaranteed min-out, so pre-existing wallet inventory is
//!    mathematically unconsumable by the route.
//!
//! These tests exercise the actual production assembly path, not a reference
//! compiler.

use arb_constants::slippage_amount_bps;
use arb_tx::{
    KaminoFlashParams, Lamports, TokenAmount, TxBuilder, assemble_cross_dex_opportunity,
    get_associated_token_address, is_cross_dex_opportunity,
};
use arb_types::{
    Bps, DexProtocol, ExecutionOpportunity, MintId, PoolExecutionAccounts, PoolId,
    PumpSwapTradeIntent, RouteId, Slot, SwapHop, TokenProgramKind,
};
use solana_pubkey::Pubkey;
use std::str::FromStr;

const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const KAMINO_KLEND: &str = "KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD";
const WSOL: &str = "So11111111111111111111111111111111111111112";
const USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";

fn token_program() -> Pubkey {
    Pubkey::from_str(TOKEN_PROGRAM).unwrap()
}

fn is_close_account(ix: &arb_tx::CompiledInstruction) -> bool {
    ix.program_id == token_program() && ix.data == vec![9]
}

fn mock_raydium_v4_exec_accounts() -> PoolExecutionAccounts {
    PoolExecutionAccounts::RaydiumV4 {
        amm_authority: Pubkey::new_unique(),
        open_orders: Pubkey::new_unique(),
        target_orders: Pubkey::new_unique(),
        base_vault: Pubkey::new_unique(),
        quote_vault: Pubkey::new_unique(),
        serum_program: Pubkey::new_unique(),
        serum_market: Pubkey::new_unique(),
        serum_bids: Pubkey::new_unique(),
        serum_asks: Pubkey::new_unique(),
        serum_event_queue: Pubkey::new_unique(),
        serum_coin_vault: Pubkey::new_unique(),
        serum_pc_vault: Pubkey::new_unique(),
        serum_vault_signer: Pubkey::new_unique(),
        swap_version: arb_types::RaydiumV4SwapVersion::V2,
    }
}

fn mock_raydium_clmm_exec_accounts() -> PoolExecutionAccounts {
    PoolExecutionAccounts::RaydiumClmm {
        amm_config: Pubkey::new_unique(),
        vault_0: Pubkey::new_unique(),
        vault_1: Pubkey::new_unique(),
        observation_state: Pubkey::new_unique(),
        tick_array_pdas: vec![Pubkey::new_unique(), Pubkey::new_unique()],
        bitmap_extension: None,
        pool_mint_0: Pubkey::from_str(WSOL).unwrap(),
    }
}

fn mock_orca_whirlpool_exec_accounts() -> PoolExecutionAccounts {
    PoolExecutionAccounts::OrcaWhirlpool {
        vault_a: Pubkey::new_unique(),
        vault_b: Pubkey::new_unique(),
        oracle: Pubkey::new_unique(),
        tick_array_pdas: vec![Pubkey::new_unique(), Pubkey::new_unique()],
        pool_mint_a: Pubkey::from_str(USDC).unwrap(),
    }
}

/// A 2-hop WSOL -> intermediate-meme -> WSOL route, the shape that produces one
/// ephemeral intermediate ATA and therefore one residual hazard.
fn make_intermediate_route(meme: Pubkey) -> ExecutionOpportunity {
    let wsol = Pubkey::from_str(WSOL).unwrap();
    ExecutionOpportunity {
        route_id: RouteId(1),
        slot: Slot(100),
        size_in: TokenAmount(50_000_000_000),
        expected_out: TokenAmount(50_500_000_000),
        hop_inputs: vec![50_000_000_000, 50_250_000_000, 50_500_000_000],
        gross_profit: Lamports(500_000_000),
        net_profit: Lamports(450_000_000),
        base_decimals: 9,
        price_impact_bps: Bps(10),
        hop_slippage_bps: 10,
        expected_min_final_output: 0,
        hops: vec![
            SwapHop {
                pool_id: PoolId::new([10; 32]),
                protocol: DexProtocol::RaydiumV4,
                mint_in: MintId(wsol),
                mint_out: MintId(meme),
                fee_bps: 25,
                active_bin_id: None,
                token_programs: [TokenProgramKind::SplToken; 2],
                trade_intent: PumpSwapTradeIntent::default(),
                execution_accounts: Some(mock_raydium_v4_exec_accounts()),
            },
            SwapHop {
                pool_id: PoolId::new([11; 32]),
                protocol: DexProtocol::OrcaWhirlpool,
                mint_in: MintId(meme),
                mint_out: MintId(wsol),
                fee_bps: 5,
                active_bin_id: None,
                token_programs: [TokenProgramKind::SplToken; 2],
                trade_intent: PumpSwapTradeIntent::default(),
                execution_accounts: Some(mock_orca_whirlpool_exec_accounts()),
            },
        ],
    }
}

fn assemble_supervisor_path(
    payer: Pubkey,
    opp: &ExecutionOpportunity,
    proven_zero: &[Pubkey],
) -> Vec<arb_tx::CompiledInstruction> {
    TxBuilder::assemble_flash_arbitrage(
        payer,
        Pubkey::new_unique(),
        Lamports(10_000),
        400_000,
        1000,
        opp,
        &KaminoFlashParams::default(),
        0,
        TokenAmount(30_000),
        proven_zero,
    )
    .expect("assembly succeeds on the Supervisor's generic builder path")
}

fn klend_program() -> Pubkey {
    Pubkey::from_str(KAMINO_KLEND).unwrap()
}

// ---------------------------------------------------------------------------
// Test 1: default Supervisor path must NEVER close the intermediate ATA.
// ---------------------------------------------------------------------------
#[test]
fn f01_intermediate_ata_is_never_closed_by_default_supervisor_path() {
    let payer = Pubkey::new_unique();
    let meme = Pubkey::new_from_array([88; 32]);
    let opp = make_intermediate_route(meme);

    let ixs = assemble_supervisor_path(payer, &opp, &[]);

    // 2 CU + WSOL ATA create + intermediate ATA create + borrow + 2 swaps
    // + profit guard + repay + tip. NO CloseAccount.
    assert_eq!(
        ixs.len(),
        10,
        "residual-safe lifecycle: no close instruction"
    );
    assert!(
        !ixs.iter().any(is_close_account),
        "CloseAccount on the intermediate ATA must never be emitted by the \
         generic builder: the hop's actual output can exceed the exact amount \
         the next hop consumes, and closing a non-empty token account reverts \
         the whole atomic transaction after the repay"
    );

    // The intermediate ATA is still created idempotently (route needs it).
    let ata_program = Pubkey::from_str("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL").unwrap();
    assert_eq!(
        ixs[3].program_id, ata_program,
        "intermediate ephemeral ATA create must still be present"
    );
}

// ---------------------------------------------------------------------------
// Test 2: the cross-dex Supervisor entry (assemble_cross_dex_opportunity)
// delegates to the same residual-safe builder.
// ---------------------------------------------------------------------------
#[test]
fn f01_cross_dex_supervisor_entry_never_closes_intermediate_ata() {
    let payer = Pubkey::new_unique();
    let meme = Pubkey::new_from_array([77; 32]);
    let mut opp = make_intermediate_route(meme);
    // `is_cross_dex_opportunity` maps only CLMM/CPMM/DLMM/Whirlpool platforms;
    // Raydium V4 is intentionally unsupported there, so the fixture uses the
    // production-reachable cross-DEX pair (RaydiumClmm + OrcaWhirlpool).
    opp.hops[0].protocol = DexProtocol::RaydiumClmm;
    opp.hops[0].execution_accounts = Some(mock_raydium_clmm_exec_accounts());

    assert!(
        is_cross_dex_opportunity(&opp),
        "fixture must be a cross-DEX route"
    );

    let ixs = assemble_cross_dex_opportunity(
        payer,
        Pubkey::new_unique(),
        Lamports(10_000),
        400_000,
        1000,
        &opp,
        &KaminoFlashParams::default(),
        0,
        TokenAmount(30_000),
        &[],
    )
    .expect("cross-dex assembly succeeds");

    assert!(
        !ixs.iter().any(is_close_account),
        "cross-dex entry must inherit the residual-safe ATA lifecycle"
    );
}

// ---------------------------------------------------------------------------
// Test 3: proven-zero allow-list closes appear ONLY for listed mints and ONLY
// after the Kamino repay index.
// ---------------------------------------------------------------------------
#[test]
fn f01_proven_zero_allowlist_closes_only_listed_mints_after_repay() {
    let payer = Pubkey::new_unique();
    let meme = Pubkey::new_from_array([88; 32]);
    let unlisted_other = Pubkey::new_from_array([99; 32]);
    let opp = make_intermediate_route(meme);

    let proven = vec![meme, unlisted_other];
    let ixs = assemble_supervisor_path(payer, &opp, &proven);

    let klend_ixs: Vec<usize> = ixs
        .iter()
        .enumerate()
        .filter(|(_, ix)| ix.program_id == klend_program())
        .map(|(i, _)| i)
        .collect();
    assert!(
        klend_ixs.len() >= 2,
        "borrow and repay must both be present"
    );
    let repay_idx = *klend_ixs.last().unwrap();

    let mints = [meme, unlisted_other];
    let expected_atas: Vec<Pubkey> = mints
        .iter()
        .map(|m| get_associated_token_address(&payer, m))
        .collect();

    // Count CloseAccount INSTRUCTIONS (each has 3 accounts: token account,
    // destination, authority).
    let closes: Vec<(usize, &arb_tx::CompiledInstruction)> = ixs
        .iter()
        .enumerate()
        .filter(|(_, ix)| is_close_account(ix))
        .collect();

    // Only the LISTED intermediate mint that is actually part of the route is
    // closed. `unlisted_other` is listed but is not a route intermediate, and a
    // hypothetical unlisted intermediate (none here) must never be closed.
    assert_eq!(
        closes.len(),
        1,
        "exactly one CloseAccount, for the listed route-intermediate mint only"
    );
    let (close_idx, close_ix) = closes[0];
    assert_eq!(
        close_ix.accounts[0].pubkey, expected_atas[0],
        "closed account must be the ATA of the proven-zero listed mint"
    );
    assert!(
        close_idx > repay_idx,
        "CloseAccount must come AFTER the Kamino repay so cleanup can never \
         strand the repay obligation"
    );
}

// ---------------------------------------------------------------------------
// Test 4: a listed mint that is NOT part of the route produces no closes; a
// base-asset-only route never closes anything (even with a populated list).
// ---------------------------------------------------------------------------
#[test]
fn f01_base_only_route_emits_no_closes_even_with_proven_list() {
    let payer = Pubkey::new_unique();
    let wsol = Pubkey::from_str(WSOL).unwrap();
    let usdc = Pubkey::from_str(USDC).unwrap();
    let outsider = Pubkey::new_from_array([42; 32]);

    let opp = ExecutionOpportunity {
        route_id: RouteId(2),
        slot: Slot(100),
        size_in: TokenAmount(50_000_000_000),
        expected_out: TokenAmount(50_500_000_000),
        hop_inputs: vec![50_000_000_000, 50_250_000_000, 50_500_000_000],
        gross_profit: Lamports(500_000_000),
        net_profit: Lamports(450_000_000),
        base_decimals: 9,
        price_impact_bps: Bps(10),
        hop_slippage_bps: 10,
        expected_min_final_output: 0,
        hops: vec![
            SwapHop {
                pool_id: PoolId::new([10; 32]),
                protocol: DexProtocol::RaydiumV4,
                mint_in: MintId(wsol),
                mint_out: MintId(usdc),
                fee_bps: 25,
                active_bin_id: None,
                token_programs: [TokenProgramKind::SplToken; 2],
                trade_intent: PumpSwapTradeIntent::default(),
                execution_accounts: Some(mock_raydium_v4_exec_accounts()),
            },
            SwapHop {
                pool_id: PoolId::new([11; 32]),
                protocol: DexProtocol::OrcaWhirlpool,
                mint_in: MintId(usdc),
                mint_out: MintId(wsol),
                fee_bps: 5,
                active_bin_id: None,
                token_programs: [TokenProgramKind::SplToken; 2],
                trade_intent: PumpSwapTradeIntent::default(),
                execution_accounts: Some(mock_orca_whirlpool_exec_accounts()),
            },
        ],
    };

    let ixs = assemble_supervisor_path(payer, &opp, &[outsider]);
    assert!(
        !ixs.iter().any(is_close_account),
        "WSOL/USDC ATAs are owner-managed base assets, not ephemeral route \
         intermediates; nothing may be closed"
    );
}

// ---------------------------------------------------------------------------
// Test 5: derive_swap_legs subsidy-proof clamp — hop i+1 consumes no more than
// hop i's guaranteed min-out, and the surplus (residual) is exactly the part
// that stays in the intermediate ATA.
// ---------------------------------------------------------------------------
#[test]
fn f01_swap_legs_clamp_is_subsidy_proof_and_residual_math_is_explicit() {
    // Route hops exceed their nominal inputs so the premium case (actual out =
    // expected out > min out) is what the math describes.
    let meme = Pubkey::new_from_array([88; 32]);
    let opp = make_intermediate_route(meme);

    let slippage_bps: u64 = 100; // 1%
    let legs =
        TxBuilder::derive_swap_legs(&opp, 0, 10_000, 30_000, slippage_bps).expect("legs derive");

    assert_eq!(legs.len(), 2);

    // Hop 1: input = borrow, min_out = floor(expected * (1 - slippage)).
    let expected_hop1_out = 50_250_000_000u64;
    let slip = slippage_amount_bps(expected_hop1_out, slippage_bps);
    let expected_min_out_1 = expected_hop1_out - slip;
    assert_eq!(legs[0], (50_000_000_000, expected_min_out_1));

    // Hop 2: input clamped to hop 1's guaranteed minimum — never the nominal.
    assert_eq!(
        legs[1].0, expected_min_out_1,
        "hop 2 input must equal hop 1's guaranteed min-out, so no pre-existing \
         wallet inventory can ever be consumed by the route"
    );
    assert!(
        legs[1].0 <= expected_min_out_1,
        "invariant: amount_in[i+1] <= min_out[i]"
    );

    // Residual math: when hop 1 pays out its full quoted amount
    // (`actual == expected > min_out`), the unconsumed surplus that remains in
    // the intermediate ATA is exactly the slippage margin. Under the OLD
    // behavior this residual made CloseAccount revert the whole transaction.
    let actual_hop1_out = expected_hop1_out;
    let residual = actual_hop1_out - legs[1].0;
    assert_eq!(
        residual, slip,
        "residual = actual_out - min_out = slippage margin (> 0 for every \
         favorable fill) — this is precisely the balance the old close \
         instruction choked on"
    );
    assert!(residual > 0);
}

// ---------------------------------------------------------------------------
// Test 6: invalid slippage must fail closed (never degrade into a plausible
// min-out) — the F-05 helper contract consumed by derive_swap_legs.
// ---------------------------------------------------------------------------
#[test]
fn f01_derive_swap_legs_rejects_invalid_slippage_fail_closed() {
    let meme = Pubkey::new_from_array([88; 32]);
    let opp = make_intermediate_route(meme);

    // > 10_000 bps is nonsensical (>= 100 %): reject, do not clamp.
    let res = TxBuilder::derive_swap_legs(&opp, 0, 10_000, 30_000, 10_001);
    assert!(res.is_err(), "slippage above 10_000 bps must fail closed");

    // Max valid slippage (10_000 = 100 %) still produces a coherent floor of 0.
    let res = TxBuilder::derive_swap_legs(&opp, 0, 10_000, 30_000, 10_000);
    let legs = res.expect("10_000 bps is a valid (if extreme) ceiling");
    assert_eq!(
        legs[0].1, 0,
        "100 % slip collapses the intermediate min-out to 0"
    );
    assert_eq!(legs[1].0, 0, "hop 2 input clamps to the zero floor");

    // u64::MAX-scale outputs must not overflow the slippage arithmetic.
    let wsol = Pubkey::from_str(WSOL).unwrap();
    let huge = u64::MAX;
    let opp_huge = ExecutionOpportunity {
        route_id: RouteId(3),
        slot: Slot(1),
        size_in: TokenAmount(huge),
        expected_out: TokenAmount(huge),
        hop_inputs: vec![huge / 2, huge],
        gross_profit: Lamports(0),
        net_profit: Lamports(0),
        base_decimals: 9,
        price_impact_bps: Bps(0),
        hop_slippage_bps: 10,
        expected_min_final_output: 0,
        hops: vec![SwapHop {
            pool_id: PoolId::new([10; 32]),
            protocol: DexProtocol::RaydiumV4,
            mint_in: MintId(wsol),
            mint_out: MintId(meme),
            fee_bps: 25,
            active_bin_id: None,
            token_programs: [TokenProgramKind::SplToken; 2],
            trade_intent: PumpSwapTradeIntent::default(),
            execution_accounts: Some(mock_raydium_v4_exec_accounts()),
        }],
    };
    let legs = TxBuilder::derive_swap_legs(&opp_huge, 0, 0, 0, 10_000)
        .expect("u64::MAX-scale slippage arithmetic must be checked, not overflowing");
    // 100 % slip zeroes the slippage margin; the last-hop floor (== borrow)
    // still governs min_out through the explicit max.
    assert_eq!(legs[0].1, huge / 2);
}
