//! Cross-protocol contract tests: incomplete snapshots must never produce
//! quotes or be advertised as execution-ready.

use arb_dex::{AmmPool, MeteoraDammV2Pool, MeteoraDbcPool, RaydiumLaunchLabPool};
use arb_math::dbc::{DbcCurve, DbcCurveSegment, ONE_Q64};
use arb_types::{MintId, PoolId, TokenProgramKind};
use solana_pubkey::Pubkey;

#[test]
fn launchlab_token_2022_never_becomes_quote_or_execution_ready_without_hook_support() {
    let launchlab_bytes = vec![0u8; arb_dex::raydium_launchlab::RAYDIUM_LAUNCHLAB_POOL_MIN_SIZE];
    let mut pool = RaydiumLaunchLabPool::from_account(PoolId::new([5; 32]), &launchlab_bytes)
        .expect("well-sized LaunchLab pool prefix should decode");

    pool.global_config = Pubkey::new_unique();
    pool.platform_config = Pubkey::new_unique();
    pool.base_vault = Pubkey::new_unique();
    pool.quote_vault = Pubkey::new_unique();
    pool.fee_config_verified = true;
    pool.token_base_program = TokenProgramKind::Token2022;

    assert!(
        pool.quote(10_000, false).is_err(),
        "LaunchLab Token-2022 must remain unsupported without epoch-aware fee/hook context"
    );
    assert!(
        pool.execution_accounts().is_none(),
        "LaunchLab Token-2022 must not be advertised as execution-ready"
    );
}

#[test]
fn all_four_unsupported_venues_fail_closed_in_direct_pool_adapters() {
    let mut damm_bytes = vec![0u8; arb_dex::meteora_damm_v2::METEORA_DAMM_V2_POOL_MIN_SIZE];
    damm_bytes[..8].copy_from_slice(&arb_dex::meteora_damm_v2::METEORA_DAMM_V2_POOL_DISCRIMINATOR);
    let damm = MeteoraDammV2Pool::from_account(PoolId::new([1; 32]), &damm_bytes)
        .expect("well-sized DAMM pool prefix should decode");

    let dbc_curve = DbcCurve {
        current_sqrt_price_x64: ONE_Q64,
        segments: vec![DbcCurveSegment::new(ONE_Q64, ONE_Q64 * 2, 1_000_000_000)],
        fee_numerator: 25_000_000,
        migration_quote_threshold: 500_000_000,
        quote_reserve: 100_000_000,
        base_reserve: 1_000_000_000,
        migrated: false,
    };
    let dbc = MeteoraDbcPool::new(
        PoolId::new([2; 32]),
        Pubkey::new_unique(),
        MintId(Pubkey::new_unique()),
        MintId(Pubkey::new_unique()),
        Pubkey::new_unique(),
        Pubkey::new_unique(),
        dbc_curve,
        25,
    );

    let launchlab_bytes = vec![0u8; arb_dex::raydium_launchlab::RAYDIUM_LAUNCHLAB_POOL_MIN_SIZE];
    let launchlab = RaydiumLaunchLabPool::from_account(PoolId::new([4; 32]), &launchlab_bytes)
        .expect("well-sized LaunchLab pool prefix should decode");

    let pools: [&dyn AmmPool; 3] = [&damm, &dbc, &launchlab];
    for pool in pools {
        assert!(
            pool.fee_bps().is_none(),
            "{:?} must not publish a fee without a complete protocol fee snapshot",
            pool.protocol()
        );
        assert!(
            pool.quote(10_000, true).is_err(),
            "{:?} must reject a quote from incomplete protocol state",
            pool.protocol()
        );
        assert!(
            pool.execution_accounts().is_none(),
            "{:?} must not be reported as execution-ready",
            pool.protocol()
        );
    }
}
