//! Integration test for GUARD-01: End-to-end guarded quote & dispatch pipeline validation.
//! Proves that a route cannot be dispatched if dependencies or pool states mutate or become stale.

use arb_engine::HopRequest;
use arb_engine::guarded::{GuardedQuoteError, quote_route_guarded, revalidate_before_dispatch};
use arb_ingest::{
    AccountDependency, AccountRole, AccountSnapshot, DependencyManifestRegistry, IngestClient,
    PoolDependencyManifest, ProviderId,
};
use arb_types::{DexProtocol, MintId, PoolId, PoolKind, PoolState, Slot};
use solana_pubkey::Pubkey;
use std::sync::Arc;

fn test_pool(
    id: u8,
    base: u8,
    quote: u8,
    reserve_in: u128,
    reserve_out: u128,
    slot: u64,
) -> PoolState {
    let pool_pk = Pubkey::new_from_array([id; 32]);
    PoolState {
        pool_id: PoolId(pool_pk),
        protocol: DexProtocol::RaydiumV4,
        slot: Slot(slot),
        write_version: 1,
        base_mint: MintId(Pubkey::new_from_array([base; 32])),
        quote_mint: MintId(Pubkey::new_from_array([quote; 32])),
        decimals_a: 9,
        decimals_b: 6,
        reserve_in,
        reserve_out,
        fee_numerator: 25,
        fee_denominator: 10_000,
        raydium_cpmm_fee: None,
        is_active: true,
        has_risky_token: false,
        last_update_slot: slot,
        last_update_ms: 1000,
        kind: PoolKind::Cpmm,
        clmm: None,
        dlmm: None,
        pump_fun: None,
        damm_v2: None,
        transfer_fee_bps: 0,
        token_programs: Default::default(),
        execution_accounts: None,
    }
}

#[test]
fn test_guard_01_rejects_route_when_state_mutates_before_dispatch() {
    let client = IngestClient::new();
    client.mark_stream_connected();
    client.record_slot(100);
    let mut manifests = DependencyManifestRegistry::default();

    let p1_pk = Pubkey::new_from_array([1; 32]);
    let p2_pk = Pubkey::new_from_array([2; 32]);
    let p1_id = PoolId(p1_pk);
    let p2_id = PoolId(p2_pk);

    // Register manifests
    let mut m1 = PoolDependencyManifest::new(p1_id, DexProtocol::RaydiumV4);
    m1.add(AccountDependency {
        pubkey: p1_pk,
        role: AccountRole::PoolState,
        expected_owner: Pubkey::new_from_array([10; 32]),
        required_for_quote: true,
    })
    .unwrap();
    manifests.register(m1).unwrap();

    let mut m2 = PoolDependencyManifest::new(p2_id, DexProtocol::RaydiumV4);
    m2.add(AccountDependency {
        pubkey: p2_pk,
        role: AccountRole::PoolState,
        expected_owner: Pubkey::new_from_array([10; 32]),
        required_for_quote: true,
    })
    .unwrap();
    manifests.register(m2).unwrap();

    // Populate initial snapshots at slot 100
    let snap1 = AccountSnapshot {
        pubkey: p1_pk,
        owner: Pubkey::new_from_array([10; 32]),
        slot: 100,
        write_version: 1,
        is_startup: false,
        provider_id: ProviderId(1),
        bank_id: Some(1),
        generation: client.stream_generation(),
        data: Arc::new(vec![0u8; 100]),
    };
    let snap2 = AccountSnapshot {
        pubkey: p2_pk,
        owner: Pubkey::new_from_array([10; 32]),
        slot: 100,
        write_version: 1,
        is_startup: false,
        provider_id: ProviderId(1),
        bank_id: Some(1),
        generation: client.stream_generation(),
        data: Arc::new(vec![0u8; 100]),
    };
    client.account_store().apply(snap1);
    client.account_store().apply(snap2);

    let mut pools = vec![
        test_pool(1, 10, 20, 10_000_000_000, 20_000_000_000, 100),
        test_pool(2, 20, 10, 20_000_000_000, 10_000_000_000, 100),
    ];
    let hops = vec![
        HopRequest {
            pool_id: p1_id,
            mint_in: MintId(Pubkey::new_from_array([10; 32])),
        },
        HopRequest {
            pool_id: p2_id,
            mint_in: MintId(Pubkey::new_from_array([20; 32])),
        },
    ];

    // Compute guarded quote
    let guarded = quote_route_guarded(
        &client,
        &manifests,
        &[p1_id, p2_id],
        &mut pools,
        &hops,
        1_000_000,
        10,
        2,
        |_snapshots, _working_pools| Ok(()),
    )
    .expect("guarded quote must succeed on initial state");

    // Immediate revalidation before dispatch succeeds
    assert!(revalidate_before_dispatch(&client, &manifests, &guarded, 10, 2).is_ok());

    // SIMULATE STATE MUTATION: A newer account snapshot arrives for pool 1 within the same slot
    let snap1_mutated = AccountSnapshot {
        pubkey: p1_pk,
        owner: Pubkey::new_from_array([10; 32]),
        slot: 100,
        write_version: 2,
        is_startup: false,
        provider_id: ProviderId(1),
        bank_id: Some(1),
        generation: client.stream_generation(),
        data: Arc::new(vec![1u8; 100]),
    };
    client.account_store().apply(snap1_mutated);

    // Revalidation immediately detects the mutation and REJECTS dispatch!
    let reval_result = revalidate_before_dispatch(&client, &manifests, &guarded, 10, 2);
    assert_eq!(
        reval_result,
        Err(GuardedQuoteError::StateChangedBeforeDispatch),
        "GUARD-01 must reject dispatch when pool dependency mutates between quote and dispatch"
    );
}

#[test]
fn test_guard_01_rejects_route_when_stream_generation_changes() {
    let client = IngestClient::new();
    client.mark_stream_connected();
    client.record_slot(100);
    let mut manifests = DependencyManifestRegistry::default();

    let p1_pk = Pubkey::new_from_array([3; 32]);
    let p1_id = PoolId(p1_pk);

    let mut m1 = PoolDependencyManifest::new(p1_id, DexProtocol::RaydiumV4);
    m1.add(AccountDependency {
        pubkey: p1_pk,
        role: AccountRole::PoolState,
        expected_owner: Pubkey::new_from_array([10; 32]),
        required_for_quote: true,
    })
    .unwrap();
    manifests.register(m1).unwrap();

    let snap = AccountSnapshot {
        pubkey: p1_pk,
        owner: Pubkey::new_from_array([10; 32]),
        slot: 100,
        write_version: 1,
        is_startup: false,
        provider_id: ProviderId(1),
        bank_id: Some(1),
        generation: client.stream_generation(),
        data: Arc::new(vec![0u8; 100]),
    };
    client.account_store().apply(snap);

    let mut pools = vec![test_pool(3, 10, 20, 10_000_000_000, 20_000_000_000, 100)];
    let hops = vec![HopRequest {
        pool_id: p1_id,
        mint_in: MintId(Pubkey::new_from_array([10; 32])),
    }];

    let guarded = quote_route_guarded(
        &client,
        &manifests,
        &[p1_id],
        &mut pools,
        &hops,
        1_000_000,
        10,
        2,
        |_snapshots, _working_pools| Ok(()),
    )
    .expect("guarded quote must succeed");

    assert!(revalidate_before_dispatch(&client, &manifests, &guarded, 10, 2).is_ok());

    // SIMULATE DISCONNECT & RECONNECT (bumps stream generation)
    client.mark_stream_disconnected();
    client.mark_stream_connected();
    client.record_slot(100);

    // Revalidation fails because disconnected stream purges accounts (fail closed)
    let reval = revalidate_before_dispatch(&client, &manifests, &guarded, 10, 2);
    assert!(
        reval.is_err(),
        "GUARD-01 must reject dispatch when stream reconnected across a gap"
    );
}
