//! Fail-closed gate for promoting account snapshots into an executable quote context.
use crate::{
    AccountSnapshot, AccountSnapshotStore, CoherenceError, DependencyStamp, ProviderId,
    SlotIntegrity, validate_dependency_group,
};
use solana_pubkey::Pubkey;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy)]
pub struct QuoteReadinessContext {
    pub stream_connected: bool,
    pub provider_id: ProviderId,
    pub generation: u64,
    pub target_slot: u64,
    pub max_age_slots: u64,
    pub max_slot_skew: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuoteReadinessError {
    StreamDisconnected,
    NoSlotWatermark,
    EmptyDependencies,
    DuplicateDependency,
    MissingExpectedOwner,
    MissingAccount,
    OwnerMismatch,
    StartupSnapshot,
    WrongProvider,
    WrongGeneration,
    MissingBankIdentity,
    DeadSlot,
    FutureSlot,
    StaleSlot,
    Incoherent(CoherenceError),
    InvalidDependencyManifest,
}

/// Return an owned, coherent set only when every dependency is live and from the
/// same provider/generation. This function does not mutate state or issue network I/O.
pub fn build_quote_dependency_set(
    store: &AccountSnapshotStore,
    slots: &SlotIntegrity,
    required: &[Pubkey],
    expected_owners: &HashMap<Pubkey, Pubkey>,
    context: QuoteReadinessContext,
) -> Result<Vec<AccountSnapshot>, QuoteReadinessError> {
    if !context.stream_connected {
        return Err(QuoteReadinessError::StreamDisconnected);
    }
    if required.is_empty() {
        return Err(QuoteReadinessError::EmptyDependencies);
    }

    let mut seen = std::collections::HashSet::with_capacity(required.len());
    let mut snapshots = Vec::with_capacity(required.len());
    let mut stamps = Vec::with_capacity(required.len());
    for key in required {
        if !seen.insert(*key) {
            return Err(QuoteReadinessError::DuplicateDependency);
        }
        if !expected_owners.contains_key(key) {
            return Err(QuoteReadinessError::MissingExpectedOwner);
        }
    }
    let fetched = store.get_many(required);
    for (key, maybe_snapshot) in required.iter().zip(fetched) {
        let expected = expected_owners
            .get(key)
            .ok_or(QuoteReadinessError::MissingExpectedOwner)?;
        let snapshot = maybe_snapshot.ok_or(QuoteReadinessError::MissingAccount)?;
        if snapshot.owner != *expected {
            return Err(QuoteReadinessError::OwnerMismatch);
        }
        if snapshot.is_startup {
            return Err(QuoteReadinessError::StartupSnapshot);
        }
        if snapshot.provider_id != context.provider_id {
            return Err(QuoteReadinessError::WrongProvider);
        }
        if snapshot.generation != context.generation {
            return Err(QuoteReadinessError::WrongGeneration);
        }
        if snapshot.bank_id.is_none() {
            return Err(QuoteReadinessError::MissingBankIdentity);
        }
        if slots.is_dead(snapshot.slot) || slots.is_bank_dead(snapshot.slot, snapshot.bank_id) {
            return Err(QuoteReadinessError::DeadSlot);
        }
        if snapshot.slot > context.target_slot {
            return Err(QuoteReadinessError::FutureSlot);
        }
        if context.target_slot.saturating_sub(snapshot.slot) > context.max_age_slots {
            return Err(QuoteReadinessError::StaleSlot);
        }
        stamps.push(DependencyStamp {
            provider: snapshot.provider_id,
            slot: snapshot.slot,
            bank_id: snapshot.bank_id,
            write_version: snapshot.write_version,
            is_startup: snapshot.is_startup,
        });
        snapshots.push(snapshot);
    }
    validate_dependency_group(&stamps, context.target_slot, context.max_slot_skew)
        .map_err(QuoteReadinessError::Incoherent)?;
    Ok(snapshots)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionReadinessError {
    StreamDisconnected,
    GenerationChanged,
    NoObservedSlot,
    ObservedSlotBehindQuote,
    QuoteTooOld,
}

/// Last-moment execution guard. It does not replace full account dependency
/// validation; it prevents a quote from crossing a stream reconnect boundary
/// or being dispatched after its source watermark has gone stale.
pub fn validate_execution_readiness(
    stream_connected: bool,
    expected_generation: u64,
    current_generation: u64,
    observed_slot: u64,
    quote_slot: u64,
    max_age_slots: u64,
) -> Result<(), ExecutionReadinessError> {
    if !stream_connected {
        return Err(ExecutionReadinessError::StreamDisconnected);
    }
    if expected_generation != current_generation {
        return Err(ExecutionReadinessError::GenerationChanged);
    }
    if observed_slot == 0 {
        return Err(ExecutionReadinessError::NoObservedSlot);
    }
    if observed_slot < quote_slot {
        return Err(ExecutionReadinessError::ObservedSlotBehindQuote);
    }
    if observed_slot.saturating_sub(quote_slot) > max_age_slots {
        return Err(ExecutionReadinessError::QuoteTooOld);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slot_integrity::{SLOT_DEAD, SLOT_PROCESSED};
    use crate::{AccountSnapshot, SlotIntegrity};
    use std::sync::Arc;

    fn setup(
        startup: bool,
    ) -> (
        AccountSnapshotStore,
        SlotIntegrity,
        Pubkey,
        Pubkey,
        HashMap<Pubkey, Pubkey>,
    ) {
        let store = AccountSnapshotStore::default();
        let mut slots = SlotIntegrity::default();
        slots.observe(100, Some(99), SLOT_PROCESSED);
        let key = Pubkey::new_from_array([1; 32]);
        let owner = Pubkey::new_from_array([2; 32]);
        let snapshot = AccountSnapshot {
            pubkey: key,
            owner,
            slot: 100,
            write_version: 7,
            is_startup: startup,
            provider_id: ProviderId(1),
            bank_id: Some(55),
            generation: 4,
            data: Arc::new(vec![1, 2, 3]),
        };
        assert!(store.apply(snapshot));
        (store, slots, key, owner, HashMap::from([(key, owner)]))
    }

    fn context() -> QuoteReadinessContext {
        QuoteReadinessContext {
            stream_connected: true,
            provider_id: ProviderId(1),
            generation: 4,
            target_slot: 100,
            max_age_slots: 2,
            max_slot_skew: 0,
        }
    }

    #[test]
    fn accepts_complete_live_dependency_set() {
        let (store, slots, key, _, owners) = setup(false);
        let got = build_quote_dependency_set(&store, &slots, &[key], &owners, context()).unwrap();
        assert_eq!(got.len(), 1);
    }

    #[test]
    fn rejects_startup_disconnected_stale_and_dead_data() {
        let (store, mut slots, key, _, owners) = setup(true);
        assert!(matches!(
            build_quote_dependency_set(&store, &slots, &[key], &owners, context()),
            Err(QuoteReadinessError::StartupSnapshot)
        ));
        let mut ctx = context();
        ctx.stream_connected = false;
        assert!(matches!(
            build_quote_dependency_set(&store, &slots, &[key], &owners, ctx),
            Err(QuoteReadinessError::StreamDisconnected)
        ));
        let (live_store, _, live_key, _, live_owners) = setup(false);
        slots.observe(100, Some(99), SLOT_DEAD);
        assert!(matches!(
            build_quote_dependency_set(&live_store, &slots, &[live_key], &live_owners, context()),
            Err(QuoteReadinessError::DeadSlot)
        ));
    }

    #[test]
    fn rejects_live_processed_snapshot_without_bank_identity() {
        let store = AccountSnapshotStore::default();
        let mut slots = SlotIntegrity::default();
        slots.observe(100, Some(99), SLOT_PROCESSED);
        let key = Pubkey::new_from_array([1; 32]);
        let owner = Pubkey::new_from_array([2; 32]);
        assert!(store.apply(AccountSnapshot {
            pubkey: key,
            owner,
            slot: 100,
            write_version: 7,
            is_startup: false,
            provider_id: ProviderId(1),
            bank_id: None,
            generation: 4,
            data: Arc::new(vec![1])
        }));
        let owners = HashMap::from([(key, owner)]);
        assert!(matches!(
            build_quote_dependency_set(&store, &slots, &[key], &owners, context()),
            Err(QuoteReadinessError::MissingBankIdentity)
        ));
    }

    #[test]
    fn rejects_bank_invalidated_by_entry_update_parent() {
        let (store, mut slots, key, _, owners) = setup(false);
        assert!(slots.invalidate_bank(100, 55));
        assert!(matches!(
            build_quote_dependency_set(&store, &slots, &[key], &owners, context()),
            Err(QuoteReadinessError::DeadSlot)
        ));
    }

    #[test]
    fn rejects_stale_and_mixed_bank_dependencies() {
        let (store, slots, key_a, owner_a, _) = setup(false);
        let key_b = Pubkey::new_from_array([3; 32]);
        let owner_b = Pubkey::new_from_array([4; 32]);
        assert!(store.apply(AccountSnapshot {
            pubkey: key_b,
            owner: owner_b,
            slot: 100,
            write_version: 8,
            is_startup: false,
            provider_id: ProviderId(1),
            bank_id: Some(56),
            generation: 4,
            data: Arc::new(vec![9])
        }));
        let owners = HashMap::from([(key_a, owner_a), (key_b, owner_b)]);
        assert!(matches!(
            build_quote_dependency_set(&store, &slots, &[key_a, key_b], &owners, context()),
            Err(QuoteReadinessError::Incoherent(CoherenceError::MixedBanks))
        ));
        let mut stale = context();
        stale.target_slot = 104;
        stale.max_age_slots = 2;
        assert!(matches!(
            build_quote_dependency_set(
                &store,
                &slots,
                &[key_a],
                &HashMap::from([(key_a, owner_a)]),
                stale
            ),
            Err(QuoteReadinessError::StaleSlot)
        ));
    }

    #[test]
    fn execution_guard_rejects_disconnect_generation_change_and_stale_quote() {
        assert_eq!(
            validate_execution_readiness(false, 4, 4, 100, 100, 2),
            Err(ExecutionReadinessError::StreamDisconnected)
        );
        assert_eq!(
            validate_execution_readiness(true, 4, 5, 100, 100, 2),
            Err(ExecutionReadinessError::GenerationChanged)
        );
        assert_eq!(
            validate_execution_readiness(true, 4, 4, 103, 100, 2),
            Err(ExecutionReadinessError::QuoteTooOld)
        );
        assert_eq!(
            validate_execution_readiness(true, 4, 4, 99, 100, 2),
            Err(ExecutionReadinessError::ObservedSlotBehindQuote)
        );
    }

    #[test]
    fn execution_guard_accepts_fresh_quote_in_same_generation() {
        assert_eq!(
            validate_execution_readiness(true, 4, 4, 101, 100, 2),
            Ok(())
        );
    }

    #[test]
    fn rejects_mixed_generation_and_provider_provenance() {
        let (store, slots, key, _, owners) = setup(false);
        let mut replacement = store.get(&key).unwrap();
        replacement.generation = 3;
        replacement.write_version = 8;
        assert!(store.apply(replacement));
        assert!(matches!(
            build_quote_dependency_set(&store, &slots, &[key], &owners, context()),
            Err(QuoteReadinessError::WrongGeneration)
        ));
    }
}
