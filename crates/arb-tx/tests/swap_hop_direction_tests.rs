//! P1-1 regression tests: swap-hop direction derivation and fail-closed guards.
//!
//! Defects covered (audit branch PR #19):
//! 1. Raydium CLMM direction was lexicographic (`mint_in < mint_out`) — proven
//!    invalid: live pools do not enforce `mint_0 < mint_1`. Direction must come
//!    from the pool's on-chain `token_mint_0` (@73..105).
//! 2. Orca direction was a WSOL comparison — silently inverted on non-WSOL
//!    pools. Direction must come from on-chain `token_mint_a` (@101..133).
//! 3. Empty `tick_array_pdas` silently produced zero-pubkey tick arrays that
//!    revert on-chain after paying priority fees.
//! 4. Default/unknown pool IDs silently fell back to hardcoded SOL-USDC pools
//!    (Orca / Raydium V4 / Meteora DLMM / Raydium CPMM), executing against a
//!    pool the quote never priced.
//! 5. Meteora DLMM passed `mint_in` as `token_x_mint`; the program validates
//!    x/y mints against the LbPair, so every Y->X hop failed on-chain.

use arb_tx::{
    TxError, build_swap_hop_instruction, get_associated_token_address,
    get_associated_token_address_with_program_id,
};
use arb_types::{DexProtocol, MintId, PoolExecutionAccounts, PoolId, SwapHop, TokenProgramKind};
use solana_pubkey::Pubkey;
use std::str::FromStr;

fn hop(
    pool_id: Pubkey,
    protocol: DexProtocol,
    mint_in: Pubkey,
    mint_out: Pubkey,
    exec: Option<PoolExecutionAccounts>,
) -> SwapHop {
    SwapHop {
        pool_id: PoolId(pool_id),
        protocol,
        mint_in: MintId(mint_in),
        mint_out: MintId(mint_out),
        fee_bps: 25,
        active_bin_id: None,
        token_programs: [TokenProgramKind::SplToken; 2],
        trade_intent: Default::default(),
        execution_accounts: exec,
    }
}

fn raydium_clmm_exec(pool_mint_0: Pubkey, tick_arrays: Vec<Pubkey>) -> PoolExecutionAccounts {
    PoolExecutionAccounts::RaydiumClmm {
        amm_config: Pubkey::new_unique(),
        vault_0: Pubkey::new_unique(),
        vault_1: Pubkey::new_unique(),
        observation_state: Pubkey::new_unique(),
        tick_array_pdas: tick_arrays,
        bitmap_extension: None,
        pool_mint_0,
    }
}

fn orca_exec(pool_mint_a: Pubkey, tick_arrays: Vec<Pubkey>) -> PoolExecutionAccounts {
    PoolExecutionAccounts::OrcaWhirlpool {
        vault_a: Pubkey::new_unique(),
        vault_b: Pubkey::new_unique(),
        oracle: Pubkey::new_unique(),
        tick_array_pdas: tick_arrays,
        pool_mint_a,
    }
}

fn dlmm_exec(token_x: Pubkey, token_y: Pubkey) -> PoolExecutionAccounts {
    PoolExecutionAccounts::MeteoraDlmm {
        reserve_x: Pubkey::new_unique(),
        reserve_y: Pubkey::new_unique(),
        oracle: Pubkey::new_unique(),
        bin_array_pdas: vec![Pubkey::new_unique(), Pubkey::new_unique()],
        bitmap_extension: None,
        pool_token_x: token_x,
        pool_token_y: token_y,
    }
}

/// Regression: pool_mint_0 > mint_1 lexicographically. The old rule
/// (`mint_in < mint_out`) would flip the direction; the new rule
/// (`mint_in == pool_mint_0`) must select vault_0 as input vault
/// (instruction account index 5).
#[test]
fn raydium_clmm_direction_uses_pool_mint_0_not_lexicographic() {
    // Deterministic ordering: force mint_0 to be lexicographically GREATER
    // than mint_1 via the leading byte. (Pubkey::new_unique() is
    // counter-based and near-monotonic in this SDK, so rejection-sampling a
    // smaller key would loop forever.)
    let mut m0 = Pubkey::new_unique().to_bytes();
    m0[0] = 0xFF;
    let mut m1 = Pubkey::new_unique().to_bytes();
    m1[0] = 0x00;
    let pool_mint_0 = Pubkey::new_from_array(m0);
    let pool_mint_1 = Pubkey::new_from_array(m1);
    let vault_0 = Pubkey::new_unique();
    let vault_1 = Pubkey::new_unique();
    let exec = PoolExecutionAccounts::RaydiumClmm {
        amm_config: Pubkey::new_unique(),
        vault_0,
        vault_1,
        observation_state: Pubkey::new_unique(),
        tick_array_pdas: vec![
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
        ],
        bitmap_extension: None,
        pool_mint_0,
    };

    // mint_in == pool_mint_0 (zero_for_one = true) even though
    // mint_in > mint_out lexicographically — old code would have chosen
    // zero_for_one = false and used vault_1 as input.
    let swap = hop(
        Pubkey::new_unique(),
        DexProtocol::RaydiumClmm,
        pool_mint_0,
        pool_mint_1,
        Some(exec),
    );
    let ix = build_swap_hop_instruction(&swap, Pubkey::new_unique(), 1_000_000, 900_000)
        .expect("CLMM hop must build");
    assert_eq!(
        ix.accounts[5].pubkey, vault_0,
        "input vault must be vault_0 when mint_in == pool_mint_0"
    );
    assert_eq!(ix.accounts[6].pubkey, vault_1);
}

/// Regression: empty tick arrays must fail closed, never produce zero-pubkey
/// tick array accounts.
#[test]
fn raydium_clmm_empty_tick_arrays_fail_closed() {
    let m0 = Pubkey::new_unique();
    let m1 = Pubkey::new_unique();
    let swap = hop(
        Pubkey::new_unique(),
        DexProtocol::RaydiumClmm,
        m0,
        m1,
        Some(raydium_clmm_exec(m0, vec![])),
    );
    let err = build_swap_hop_instruction(&swap, Pubkey::new_unique(), 1_000_000, 900_000)
        .expect_err("empty tick arrays must be rejected");
    assert!(
        matches!(err, TxError::InvalidExecutionAccounts { .. }),
        "{err}"
    );
}

/// Regression: default pool_mint_0 in execution accounts must fail closed
/// instead of deriving a bogus direction.
#[test]
fn raydium_clmm_default_pool_mint_0_fails_closed() {
    let m1 = Pubkey::new_unique();
    let swap = hop(
        Pubkey::new_unique(),
        DexProtocol::RaydiumClmm,
        Pubkey::new_unique(),
        m1,
        Some(raydium_clmm_exec(
            Pubkey::default(),
            vec![Pubkey::new_unique()],
        )),
    );
    let err = build_swap_hop_instruction(&swap, Pubkey::new_unique(), 1_000_000, 900_000)
        .expect_err("default pool_mint_0 must be rejected");
    assert!(
        matches!(err, TxError::InvalidExecutionAccounts { .. }),
        "{err}"
    );
}

/// Regression: Orca direction must come from on-chain token_mint_a, not a WSOL
/// comparison. Non-WSOL pair where mint_in == pool_mint_a must produce
/// a_to_b = true (data[33] == 1 and token_owner_a = ATA(user, mint_in)).
#[test]
fn orca_direction_uses_pool_mint_a_not_wsol() {
    let user = Pubkey::new_unique();
    let mint_a = Pubkey::new_unique();
    let mint_b = Pubkey::new_unique();
    let swap = hop(
        Pubkey::new_unique(),
        DexProtocol::OrcaWhirlpool,
        mint_a,
        mint_b,
        Some(orca_exec(
            mint_a,
            vec![
                Pubkey::new_unique(),
                Pubkey::new_unique(),
                Pubkey::new_unique(),
            ],
        )),
    );
    let ix =
        build_swap_hop_instruction(&swap, user, 1_000_000, 900_000).expect("Orca hop must build");
    assert_eq!(
        ix.data[41], 1,
        "a_to_b must be true when mint_in == pool_mint_a"
    );
    assert_eq!(
        ix.accounts[3].pubkey,
        get_associated_token_address(&user, &mint_a),
        "token_owner_a must be the ATA of the input mint when a_to_b"
    );

    // Reverse direction: mint_in == mint_b (== not pool_mint_a) -> a_to_b = false.
    let rev = hop(
        Pubkey::new_unique(),
        DexProtocol::OrcaWhirlpool,
        mint_b,
        mint_a,
        Some(orca_exec(
            mint_a,
            vec![
                Pubkey::new_unique(),
                Pubkey::new_unique(),
                Pubkey::new_unique(),
            ],
        )),
    );
    let ix_rev = build_swap_hop_instruction(&rev, user, 1_000_000, 900_000)
        .expect("Orca reverse hop must build");
    assert_eq!(
        ix_rev.data[41], 0,
        "a_to_b must be false when mint_in != pool_mint_a"
    );
}

/// Token-2022 ATA derivation must follow the hop's per-mint token program.
#[test]
fn orca_user_atas_use_each_mints_declared_token_program() {
    let user = Pubkey::new_unique();
    let mint_a = Pubkey::new_unique();
    let mint_b = Pubkey::new_unique();
    let mut swap = hop(
        Pubkey::new_unique(),
        DexProtocol::OrcaWhirlpool,
        mint_a,
        mint_b,
        Some(orca_exec(
            mint_a,
            vec![
                Pubkey::new_unique(),
                Pubkey::new_unique(),
                Pubkey::new_unique(),
            ],
        )),
    );
    swap.token_programs = [TokenProgramKind::Token2022; 2];

    let ix = build_swap_hop_instruction(&swap, user, 1_000_000, 900_000)
        .expect("Token-2022 Orca hop must build via swap_v2");
    let token_2022 = Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID).unwrap();
    assert_eq!(&ix.data[..8], &arb_tx::ORCA_WHIRLPOOL_SWAP_V2_DISCRIMINATOR);
    assert_eq!(ix.accounts[0].pubkey, token_2022);
    assert_eq!(ix.accounts[1].pubkey, token_2022);
    assert_eq!(
        ix.accounts[7].pubkey,
        get_associated_token_address_with_program_id(&user, &mint_a, &token_2022)
    );
    assert_eq!(
        ix.accounts[9].pubkey,
        get_associated_token_address_with_program_id(&user, &mint_b, &token_2022)
    );
}

#[test]
fn orca_mixed_token_program_pair_routes_via_swap_v2() {
    let user = Pubkey::new_unique();
    let mint_a = Pubkey::new_unique();
    let mint_b = Pubkey::new_unique();
    let mut swap = hop(
        Pubkey::new_unique(),
        DexProtocol::OrcaWhirlpool,
        mint_a,
        mint_b,
        Some(orca_exec(
            mint_a,
            vec![
                Pubkey::new_unique(),
                Pubkey::new_unique(),
                Pubkey::new_unique(),
            ],
        )),
    );
    swap.token_programs = [TokenProgramKind::Token2022, TokenProgramKind::SplToken];
    let ix = build_swap_hop_instruction(&swap, user, 1_000_000, 900_000)
        .expect("Orca swap_v2 must support mixed SPL/Token-2022 programs");
    let token_2022 = Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID).unwrap();
    let spl_token = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap();
    assert_eq!(&ix.data[..8], &arb_tx::ORCA_WHIRLPOOL_SWAP_V2_DISCRIMINATOR);
    assert_eq!(ix.accounts[0].pubkey, token_2022);
    assert_eq!(ix.accounts[1].pubkey, spl_token);
    assert_eq!(
        ix.accounts[7].pubkey,
        get_associated_token_address_with_program_id(&user, &mint_a, &token_2022)
    );
    assert_eq!(
        ix.accounts[9].pubkey,
        get_associated_token_address_with_program_id(&user, &mint_b, &spl_token)
    );
}

/// Regression: empty Orca tick arrays must fail closed.
#[test]
fn orca_empty_tick_arrays_fail_closed() {
    let ma = Pubkey::new_unique();
    let mb = Pubkey::new_unique();
    let swap = hop(
        Pubkey::new_unique(),
        DexProtocol::OrcaWhirlpool,
        ma,
        mb,
        Some(orca_exec(ma, vec![])),
    );
    let err = build_swap_hop_instruction(&swap, Pubkey::new_unique(), 1_000_000, 900_000)
        .expect_err("empty tick arrays must be rejected");
    assert!(
        matches!(err, TxError::InvalidExecutionAccounts { .. }),
        "{err}"
    );
}

/// Regression: Orca hop with default pool_id must fail closed — the old code
/// silently swapped against the hardcoded ORCA_SOL_USDC_POOL.
#[test]
fn orca_default_pool_id_fails_closed_no_hardcoded_fallback() {
    let ma = Pubkey::new_unique();
    let mb = Pubkey::new_unique();
    let swap = hop(
        Pubkey::default(),
        DexProtocol::OrcaWhirlpool,
        ma,
        mb,
        Some(orca_exec(ma, vec![Pubkey::new_unique()])),
    );
    let err = build_swap_hop_instruction(&swap, Pubkey::new_unique(), 1_000_000, 900_000)
        .expect_err("default pool_id must be rejected");
    assert!(
        matches!(err, TxError::InvalidExecutionAccounts { .. }),
        "{err}"
    );
}

/// Regression: Raydium V4 hop without execution accounts must fail closed for
/// any pool id (including default) — no hardcoded RAYDIUM_SOL_USDC_POOL swap.
#[test]
fn raydium_v4_missing_exec_accounts_fail_closed_no_hardcoded_fallback() {
    for pool_id in [Pubkey::default(), Pubkey::new_unique()] {
        let swap = hop(
            pool_id,
            DexProtocol::RaydiumV4,
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            None,
        );
        let err = build_swap_hop_instruction(&swap, Pubkey::new_unique(), 1_000_000, 900_000)
            .expect_err("missing V4 execution accounts must be rejected");
        assert!(
            matches!(err, TxError::MissingExecutionAccounts { .. }),
            "{err}"
        );
    }
}

fn damm_v2_exec(token_a_mint: Pubkey, token_b_mint: Pubkey) -> PoolExecutionAccounts {
    PoolExecutionAccounts::MeteoraDammV2 {
        token_a_mint,
        token_b_mint,
        authority: Pubkey::new_unique(),
        vault_a: Pubkey::new_unique(),
        vault_b: Pubkey::new_unique(),
        event_authority: Pubkey::new_unique(),
    }
}
/// Regression: DLMM Y->X hop must pass the pool's on-chain token_x/token_y in
/// the fixed x/y slots (instruction account indices 6/7), not mint_in/mint_out.
#[test]
fn dlmm_y_to_x_hop_uses_pool_mints_not_hop_mints() {
    let token_x = Pubkey::new_unique();
    let token_y = Pubkey::new_unique();
    let exec = dlmm_exec(token_x, token_y);

    // Y -> X hop: mint_in is token_y. Old code passed mint_in as token_x_mint,
    // which the DLMM program rejects (validated against the LbPair).
    let swap = hop(
        Pubkey::new_unique(),
        DexProtocol::MeteoraDlmm,
        token_y,
        token_x,
        Some(exec),
    );
    let ix = build_swap_hop_instruction(&swap, Pubkey::new_unique(), 1_000_000, 900_000)
        .expect("DLMM Y->X hop must build");
    assert_eq!(
        ix.accounts[6].pubkey, token_x,
        "token_x_mint slot must carry the pool's token_x"
    );
    assert_eq!(
        ix.accounts[7].pubkey, token_y,
        "token_y_mint slot must carry the pool's token_y"
    );
}

/// Regression: DLMM hop whose mints do not belong to the pool must fail closed.
#[test]
fn dlmm_mint_mismatch_fails_closed() {
    let token_x = Pubkey::new_unique();
    let token_y = Pubkey::new_unique();
    let swap = hop(
        Pubkey::new_unique(),
        DexProtocol::MeteoraDlmm,
        Pubkey::new_unique(),
        Pubkey::new_unique(),
        Some(dlmm_exec(token_x, token_y)),
    );
    let err = build_swap_hop_instruction(&swap, Pubkey::new_unique(), 1_000_000, 900_000)
        .expect_err("hops with foreign mints must be rejected");
    assert!(
        matches!(err, TxError::InvalidExecutionAccounts { .. }),
        "{err}"
    );
}

/// Regression: DLMM default pool mints in execution accounts must fail closed.
#[test]
fn dlmm_default_pool_mints_fail_closed() {
    let swap = hop(
        Pubkey::new_unique(),
        DexProtocol::MeteoraDlmm,
        Pubkey::new_unique(),
        Pubkey::new_unique(),
        Some(dlmm_exec(Pubkey::default(), Pubkey::default())),
    );
    let err = build_swap_hop_instruction(&swap, Pubkey::new_unique(), 1_000_000, 900_000)
        .expect_err("default pool mints must be rejected");
    assert!(
        matches!(err, TxError::InvalidExecutionAccounts { .. }),
        "{err}"
    );
}

/// PR24 §5 regression: a Y->X DLMM hop must place the POOL's programs in the
/// fixed swap2 slots (11 = token_x_program, 12 = token_y_program), never the
/// hop's (in, out) programs. Fixture: X = SPL Token, Y = Token-2022; the hop
/// sells Y (in, Token-2022) for X (out, SPL). The old mapping put the 2022
/// program in the x slot — fatal on-chain for any Token-2022 reverse hop.
#[test]
fn dlmm_reverse_hop_maps_programs_to_pool_xy_slots() {
    use std::str::FromStr;

    let token_x = Pubkey::new_unique(); // SPL side
    let token_y = Pubkey::new_unique(); // Token-2022 side
    let swap = SwapHop {
        pool_id: PoolId(Pubkey::new_unique()),
        protocol: DexProtocol::MeteoraDlmm,
        mint_in: MintId(token_y),
        mint_out: MintId(token_x),
        fee_bps: 25,
        active_bin_id: None,
        // SwapHop contract: (mint_in, mint_out) = (2022, SPL).
        token_programs: [TokenProgramKind::Token2022, TokenProgramKind::SplToken],
        trade_intent: Default::default(),
        execution_accounts: Some(dlmm_exec(token_x, token_y)),
    };
    let ix = build_swap_hop_instruction(&swap, Pubkey::new_unique(), 1_000_000, 900_000)
        .expect("reverse DLMM hop must build");
    let spl = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap();
    let t22 = Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID).unwrap();
    assert_eq!(
        ix.accounts[11].pubkey, spl,
        "slot 11 must be token_x_program (SPL)"
    );
    assert_eq!(
        ix.accounts[12].pubkey, t22,
        "slot 12 must be token_y_program (2022)"
    );
}

/// PR24 §5 regression: the forward X->Y hop keeps the natural orientation.
#[test]
fn dlmm_forward_hop_maps_programs_to_pool_xy_slots() {
    use std::str::FromStr;

    let token_x = Pubkey::new_unique(); // Token-2022 side
    let token_y = Pubkey::new_unique(); // SPL side
    let swap = SwapHop {
        pool_id: PoolId(Pubkey::new_unique()),
        protocol: DexProtocol::MeteoraDlmm,
        mint_in: MintId(token_x),
        mint_out: MintId(token_y),
        fee_bps: 25,
        active_bin_id: None,
        token_programs: [TokenProgramKind::Token2022, TokenProgramKind::SplToken],
        trade_intent: Default::default(),
        execution_accounts: Some(dlmm_exec(token_x, token_y)),
    };
    let ix = build_swap_hop_instruction(&swap, Pubkey::new_unique(), 1_000_000, 900_000)
        .expect("forward DLMM hop must build");
    let spl = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap();
    let t22 = Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID).unwrap();
    assert_eq!(
        ix.accounts[11].pubkey, t22,
        "slot 11 must be token_x_program (2022)"
    );
    assert_eq!(
        ix.accounts[12].pubkey, spl,
        "slot 12 must be token_y_program (SPL)"
    );
}
/// DAMM v2 A/B order is the order stored in Pool, not lexicographic mint order.
#[test]
fn damm_v2_a_to_b_uses_pool_mint_order_even_when_a_is_lexicographically_greater() {
    let mut a_bytes = Pubkey::new_unique().to_bytes();
    a_bytes[0] = 0xff;
    let mut b_bytes = Pubkey::new_unique().to_bytes();
    b_bytes[0] = 0x00;
    let token_a = Pubkey::new_from_array(a_bytes);
    let token_b = Pubkey::new_from_array(b_bytes);
    let vault_a = Pubkey::new_unique();
    let vault_b = Pubkey::new_unique();
    let exec = PoolExecutionAccounts::MeteoraDammV2 {
        token_a_mint: token_a,
        token_b_mint: token_b,
        authority: Pubkey::new_unique(),
        vault_a,
        vault_b,
        event_authority: Pubkey::new_unique(),
    };
    let user = Pubkey::new_unique();
    let swap = hop(
        Pubkey::new_unique(),
        DexProtocol::MeteoraDammV2,
        token_a,
        token_b,
        Some(exec),
    );
    let ix = build_swap_hop_instruction(&swap, user, 1_000_000, 900_000)
        .expect("DAMM v2 A-to-B hop must build");
    let spl = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap();
    assert_eq!(ix.accounts[4].pubkey, vault_a);
    assert_eq!(ix.accounts[5].pubkey, vault_b);
    assert_eq!(ix.accounts[6].pubkey, token_a);
    assert_eq!(ix.accounts[7].pubkey, token_b);
    assert_eq!(ix.accounts[9].pubkey, spl);
    assert_eq!(ix.accounts[10].pubkey, spl);
    assert_eq!(
        ix.accounts[2].pubkey,
        get_associated_token_address_with_program_id(&user, &token_a, &spl)
    );
    assert_eq!(
        ix.accounts[3].pubkey,
        get_associated_token_address_with_program_id(&user, &token_b, &spl)
    );
}
/// Reverse DAMM v2 direction still keeps pool A/B mints and vaults in fixed slots.
#[test]
fn damm_v2_b_to_a_keeps_fixed_pool_mint_and_vault_slots() {
    let token_a = Pubkey::new_unique();
    let token_b = Pubkey::new_unique();
    let exec = damm_v2_exec(token_a, token_b);
    let user = Pubkey::new_unique();
    let swap = hop(
        Pubkey::new_unique(),
        DexProtocol::MeteoraDammV2,
        token_b,
        token_a,
        Some(exec),
    );
    let ix = build_swap_hop_instruction(&swap, user, 1_000_000, 900_000)
        .expect("DAMM v2 B-to-A hop must build");
    let spl = Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap();
    assert_eq!(ix.accounts[6].pubkey, token_a);
    assert_eq!(ix.accounts[7].pubkey, token_b);
    assert_eq!(
        ix.accounts[2].pubkey,
        get_associated_token_address_with_program_id(&user, &token_b, &spl)
    );
    assert_eq!(
        ix.accounts[3].pubkey,
        get_associated_token_address_with_program_id(&user, &token_a, &spl)
    );
}

/// A hop with mints not matching the pool must never infer A/B from sorting.
#[test]
fn damm_v2_foreign_mints_fail_closed() {
    let token_a = Pubkey::new_unique();
    let token_b = Pubkey::new_unique();
    let swap = hop(
        Pubkey::new_unique(),
        DexProtocol::MeteoraDammV2,
        Pubkey::new_unique(),
        Pubkey::new_unique(),
        Some(damm_v2_exec(token_a, token_b)),
    );
    let err = build_swap_hop_instruction(&swap, Pubkey::new_unique(), 1_000_000, 900_000)
        .expect_err("foreign DAMM v2 mints must be rejected");
    assert!(
        matches!(err, TxError::InvalidExecutionAccounts { .. }),
        "{err}"
    );
}

/// Regression: Raydium CLMM sqrt_price_limit_x64 must stay strictly within
/// MIN_SQRT_PRICE_X64..MAX_SQRT_PRICE_X64 directional bounds even on
/// decimal-skewed swaps rather than computing a broken output/input ratio.
#[test]
fn raydium_clmm_sqrt_price_limit_uses_directional_bounds() {
    let m0 = Pubkey::new_unique();
    let m1 = Pubkey::new_unique();
    let exec = raydium_clmm_exec(m0, vec![Pubkey::new_unique()]);

    let hop_zfo = hop(
        Pubkey::new_unique(),
        DexProtocol::RaydiumClmm,
        m0,
        m1,
        Some(exec.clone()),
    );
    let ix_zfo = build_swap_hop_instruction(&hop_zfo, Pubkey::new_unique(), 1_000_000_000, 50_000)
        .expect("zero_for_one CLMM hop must build");
    let limit_zfo = u128::from_le_bytes(ix_zfo.data[24..40].try_into().unwrap());
    assert_eq!(limit_zfo, arb_constants::CLMM_MIN_SQRT_PRICE_X64 + 1);

    let hop_ofz = hop(
        Pubkey::new_unique(),
        DexProtocol::RaydiumClmm,
        m1,
        m0,
        Some(exec),
    );
    let ix_ofz = build_swap_hop_instruction(&hop_ofz, Pubkey::new_unique(), 50_000, 1_000_000_000)
        .expect("one_for_zero CLMM hop must build");
    let limit_ofz = u128::from_le_bytes(ix_ofz.data[24..40].try_into().unwrap());
    assert_eq!(
        limit_ofz,
        arb_constants::RAYDIUM_CLMM_MAX_SQRT_PRICE_X64 - 1
    );
}

/// Regression: PumpFun builder rejects Token-2022 hops fail closed until create_v2/V3
/// token-program-aware execution support is proven.
#[test]
fn pump_fun_token2022_fails_closed() {
    let mut swap = hop(
        Pubkey::new_unique(),
        DexProtocol::PumpFun,
        Pubkey::from_str(arb_constants::WSOL_MINT).unwrap(),
        Pubkey::new_unique(),
        None,
    );
    swap.token_programs = [
        arb_types::TokenProgramKind::SplToken,
        arb_types::TokenProgramKind::Token2022,
    ];
    let err = build_swap_hop_instruction(&swap, Pubkey::new_unique(), 1_000_000, 500_000)
        .expect_err("Token-2022 PumpFun swap must fail closed");
    assert!(err.to_string().contains("SPL Token"));
}
