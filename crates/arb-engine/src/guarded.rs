//! End-to-end state gate around pure route pricing and dispatch.
//! This is an opt-in safe entry point; callers must migrate every live path to it.
use crate::{HopRequest, RouteQuote, quote_route};
use arb_ingest::{
    AccountSnapshot, DependencyManifestRegistry, QuoteReadinessError, YellowstoneClient,
};
use arb_types::{PoolId, PoolState, Result};
use solana_pubkey::Pubkey;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountEvidence {
    pub pubkey: Pubkey,
    pub owner: Pubkey,
    pub slot: u64,
    pub write_version: u64,
    pub provider_id: u16,
    pub bank_id: Option<u64>,
    pub generation: u64,
}

impl From<&AccountSnapshot> for AccountEvidence {
    fn from(s: &AccountSnapshot) -> Self {
        Self {
            pubkey: s.pubkey,
            owner: s.owner,
            slot: s.slot,
            write_version: s.write_version,
            provider_id: s.provider_id.0,
            bank_id: s.bank_id,
            generation: s.generation,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardedQuoteError {
    Readiness(QuoteReadinessError),
    Quote(String),
    StateChangedBeforeDispatch,
}

/// A quote paired with the exact dependency versions used to authorize it.
#[derive(Debug, Clone)]
pub struct GuardedRouteQuote {
    pub quote: RouteQuote,
    pub quote_slot: u64,
    pub generation: u64,
    pub evidence: Vec<AccountEvidence>,
    pool_ids: Vec<PoolId>,
}

/// Validate all route dependencies and decode PoolState from those exact account bytes
/// before running the pure quote calculation.
pub fn quote_route_guarded(
    client: &YellowstoneClient,
    manifests: &DependencyManifestRegistry,
    pool_ids: &[PoolId],
    pools: &mut [PoolState],
    hops: &[HopRequest],
    amount_in: u64,
    max_age_slots: u64,
    max_slot_skew: u64,
    decode_from_snapshots: impl FnOnce(&[AccountSnapshot], &mut [PoolState]) -> Result<()>,
) -> std::result::Result<GuardedRouteQuote, GuardedQuoteError> {
    let quote_slot = client.current_slot();
    let snapshots = client
        .quote_dependencies_for_route(
            manifests,
            pool_ids,
            quote_slot,
            max_age_slots,
            max_slot_skew,
        )
        .map_err(GuardedQuoteError::Readiness)?;
    let generation = client.stream_generation();
    // Work on a private copy: a rejected quote must not leave partially advanced
    // reserves in the caller's reusable pool cache.
    let mut working_pools = pools.to_vec();
    // Decode the actual quote state from the exact validated bytes. A caller cannot
    // silently pass an unrelated/stale PoolState snapshot into this guarded path.
    decode_from_snapshots(&snapshots, &mut working_pools)
        .map_err(|e| GuardedQuoteError::Quote(e.to_string()))?;
    for pool in working_pools.iter() {
        let manifest = manifests
            .get(&pool.pool_id)
            .ok_or(GuardedQuoteError::StateChangedBeforeDispatch)?;
        if manifest.protocol != pool.protocol {
            return Err(GuardedQuoteError::StateChangedBeforeDispatch);
        }
        let pool_account = snapshots
            .iter()
            .find(|snapshot| snapshot.pubkey == pool.pool_id.0)
            .ok_or(GuardedQuoteError::StateChangedBeforeDispatch)?;
        if pool.slot.0 != pool_account.slot {
            return Err(GuardedQuoteError::StateChangedBeforeDispatch);
        }
    }
    let mut evidence = snapshots
        .iter()
        .map(AccountEvidence::from)
        .collect::<Vec<_>>();
    evidence.sort_by_key(|item| item.pubkey.to_bytes());
    let quote = quote_route(&mut working_pools, hops, amount_in)
        .map_err(|e| GuardedQuoteError::Quote(e.to_string()))?;
    // Recheck after quote construction to close the state-change window during pricing.
    let fresh = client
        .quote_dependencies_for_route(
            manifests,
            pool_ids,
            quote_slot,
            max_age_slots,
            max_slot_skew,
        )
        .map_err(GuardedQuoteError::Readiness)?;
    let mut fresh_evidence = fresh.iter().map(AccountEvidence::from).collect::<Vec<_>>();
    fresh_evidence.sort_by_key(|item| item.pubkey.to_bytes());
    if generation != client.stream_generation() || evidence != fresh_evidence {
        return Err(GuardedQuoteError::StateChangedBeforeDispatch);
    }
    pools.clone_from_slice(&working_pools);
    Ok(GuardedRouteQuote {
        quote,
        quote_slot,
        generation,
        evidence,
        pool_ids: pool_ids.to_vec(),
    })
}

/// Must be called immediately before transaction submission. Any account version,
/// bank, provider, generation or stream-watermark change invalidates the quote.
pub fn revalidate_before_dispatch(
    client: &YellowstoneClient,
    manifests: &DependencyManifestRegistry,
    guarded: &GuardedRouteQuote,
    max_age_slots: u64,
    max_slot_skew: u64,
) -> std::result::Result<(), GuardedQuoteError> {
    let fresh = client
        .quote_dependencies_for_route(
            manifests,
            &guarded.pool_ids,
            guarded.quote_slot,
            max_age_slots,
            max_slot_skew,
        )
        .map_err(GuardedQuoteError::Readiness)?;
    let mut evidence = fresh.iter().map(AccountEvidence::from).collect::<Vec<_>>();
    evidence.sort_by_key(|item| item.pubkey.to_bytes());
    if client.stream_generation() != guarded.generation || evidence != guarded.evidence {
        return Err(GuardedQuoteError::StateChangedBeforeDispatch);
    }
    Ok(())
}
