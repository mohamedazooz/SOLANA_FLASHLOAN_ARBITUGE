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

use arb_tx::{TxError, build_swap_hop_instruction, get_associated_token_address};
use arb_types::{DexProtocol, MintId, PoolExecutionAccounts, PoolId, SwapHop, TokenProgramKind};
use solana_pubkey::Pubkey;

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
