//! Bounded fork tracking and snapshot-reconciliation primitives for one Yellowstone source.
//! SlotStatus numeric values follow rpcpool/yellowstone-grpc geyser.proto.
use std::collections::{BTreeMap, BTreeSet, HashSet};

pub const SLOT_PROCESSED: i32 = 0;
pub const SLOT_CONFIRMED: i32 = 1;
pub const SLOT_FINALIZED: i32 = 2;
pub const SLOT_FIRST_SHRED_RECEIVED: i32 = 3;
pub const SLOT_COMPLETED: i32 = 4;
pub const SLOT_CREATED_BANK: i32 = 5;
pub const SLOT_DEAD: i32 = 6;
const MAX_TRACKED_SLOTS: usize = 8_192;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotUpdateOutcome {
    Observed,
    Dead { invalidated_slots: Vec<u64> },
    UnknownStatus(i32),
}

#[derive(Debug, Default)]
pub struct SlotIntegrity {
    parents: BTreeMap<u64, u64>,
    dead: BTreeSet<u64>,
    finalized_slot: u64,
    highest_seen: u64,
    invalidated_banks: HashSet<(u64, u64)>,
    banks_by_slot: BTreeMap<u64, BTreeSet<u64>>,
}

impl SlotIntegrity {
    pub fn observe(&mut self, slot: u64, parent: Option<u64>, status: i32) -> SlotUpdateOutcome {
        self.observe_with_bank(slot, parent, status, None)
    }

    /// Bank-aware slot observation. A bank is identified by (slot, bank_id); bank IDs
    /// must never be compared across slots or providers.
    pub fn observe_with_bank(&mut self, slot: u64, parent: Option<u64>, status: i32, bank_id: Option<u64>) -> SlotUpdateOutcome {
        self.highest_seen = self.highest_seen.max(slot);
        if let Some(parent) = parent { self.parents.insert(slot, parent); }
        if let Some(bank_id) = bank_id { self.banks_by_slot.entry(slot).or_default().insert(bank_id); }
        if status == SLOT_DEAD {
            if let Some(known_banks) = self.banks_by_slot.get(&slot) {
                self.invalidated_banks.extend(known_banks.iter().map(|id| (slot, *id)));
            }
            if let Some(bank_id) = bank_id { self.invalidated_banks.insert((slot, bank_id)); }
            let mut invalidated = BTreeSet::from([slot]);
            loop {
                let before = invalidated.len();
                for (child, parent) in &self.parents {
                    if invalidated.contains(parent) { invalidated.insert(*child); }
                }
                if invalidated.len() == before { break; }
            }
            self.dead.extend(invalidated.iter().copied());
            self.prune();
            return SlotUpdateOutcome::Dead { invalidated_slots: invalidated.into_iter().collect() };
        }
        if status == SLOT_FINALIZED { self.finalized_slot = self.finalized_slot.max(slot); }
        // Yellowstone may report a repaired/re-created bank at the same slot after
        // SLOT_DEAD. Re-open that slot only; descendants of the dead bank stay invalid.
        if status == SLOT_CREATED_BANK {
            // Do not reopen a dead slot without a candidate identity. Otherwise an
            // untagged late account update could bypass the old-bank tombstones.
            if let Some(bank_id) = bank_id {
                self.dead.remove(&slot);
                self.invalidated_banks.remove(&(slot, bank_id));
            }
        }
        if !matches!(status, SLOT_PROCESSED | SLOT_CONFIRMED | SLOT_FINALIZED | SLOT_FIRST_SHRED_RECEIVED | SLOT_COMPLETED | SLOT_CREATED_BANK) {
            return SlotUpdateOutcome::UnknownStatus(status);
        }
        self.prune();
        SlotUpdateOutcome::Observed
    }

    pub fn is_dead(&self, slot: u64) -> bool { self.dead.contains(&slot) }
    pub fn finalized_slot(&self) -> u64 { self.finalized_slot }
    pub fn highest_seen(&self) -> u64 { self.highest_seen }

    /// EntryUpdateParent invalidates the cleared candidate bank. Yellowstone's
    /// protocol guarantee is about entries; we conservatively invalidate account
    /// snapshots carrying the same (slot, bank_id) as well.
    pub fn record_bank_id(&mut self, slot: u64, bank_id: u64) {
        self.highest_seen = self.highest_seen.max(slot);
        self.banks_by_slot.entry(slot).or_default().insert(bank_id);
        if self.dead.contains(&slot) { self.invalidated_banks.insert((slot, bank_id)); }
    }

    pub fn invalidate_bank(&mut self, slot: u64, bank_id: u64) -> bool {
        self.highest_seen = self.highest_seen.max(slot);
        self.banks_by_slot.entry(slot).or_default().insert(bank_id);
        self.invalidated_banks.insert((slot, bank_id))
    }

    pub fn is_bank_dead(&self, slot: u64, bank_id: Option<u64>) -> bool {
        bank_id.is_some_and(|id| self.invalidated_banks.contains(&(slot, id)))
    }

    pub fn bank_keys_for_slots(&self, slots: &[u64]) -> Vec<(u64, u64)> {
        slots.iter().flat_map(|slot| self.banks_by_slot.get(slot).into_iter()
            .flat_map(move |ids| ids.iter().map(move |id| (*slot, *id)))).collect()
    }

    fn prune(&mut self) {
        let floor = self.highest_seen.saturating_sub(MAX_TRACKED_SLOTS as u64);
        self.parents.retain(|slot, parent| *slot >= floor && *parent >= floor);
        self.dead.retain(|slot| *slot >= floor);
        self.invalidated_banks.retain(|(slot, _)| *slot >= floor);
        self.banks_by_slot.retain(|slot, _| *slot >= floor);
    }
}

/// A bootstrap generation is ready only after every required key was returned by one
/// coherent getMultipleAccounts response (one RPC context slot).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconcileError { EmptyRequirements, WrongGeneration, ContextSlotTooOld, MissingAccount, DuplicateAccount }

#[derive(Debug, Default)]
pub struct SnapshotReconciler {
    generation: u64,
    target_slot: u64,
    required: HashSet<[u8; 32]>,
    received: HashSet<[u8; 32]>,
    context_slot: Option<u64>,
    ready: bool,
}

impl SnapshotReconciler {
    pub fn begin<I>(&mut self, generation: u64, target_slot: u64, required: I) -> Result<(), ReconcileError>
    where I: IntoIterator<Item = [u8; 32]> {
        let keys: HashSet<_> = required.into_iter().collect();
        if keys.is_empty() { return Err(ReconcileError::EmptyRequirements); }
        self.generation = generation;
        self.target_slot = target_slot;
        self.required = keys;
        self.received.clear();
        self.context_slot = None;
        self.ready = false;
        Ok(())
    }

    pub fn record_batch<I>(&mut self, generation: u64, context_slot: u64, keys: I) -> Result<bool, ReconcileError>
    where I: IntoIterator<Item = [u8; 32]> {
        if generation != self.generation { return Err(ReconcileError::WrongGeneration); }
        if context_slot < self.target_slot { return Err(ReconcileError::ContextSlotTooOld); }
        if self.context_slot.is_some_and(|slot| slot != context_slot) { return Err(ReconcileError::ContextSlotTooOld); }
        self.context_slot = Some(context_slot);
        for key in keys {
            if !self.required.contains(&key) { continue; }
            if !self.received.insert(key) { return Err(ReconcileError::DuplicateAccount); }
        }
        self.ready = self.received.len() == self.required.len();
        Ok(self.ready)
    }

    pub fn ready(&self) -> bool { self.ready }
    pub fn context_slot(&self) -> Option<u64> { self.context_slot }
    pub fn missing_count(&self) -> usize { self.required.len().saturating_sub(self.received.len()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dead_slot_invalidates_known_descendants_only() {
        let mut g = SlotIntegrity::default();
        g.observe(10, Some(9), SLOT_PROCESSED);
        g.observe(11, Some(10), SLOT_PROCESSED);
        g.observe(12, Some(11), SLOT_PROCESSED);
        g.observe(13, Some(10), SLOT_PROCESSED);
        let out = g.observe(10, Some(9), SLOT_DEAD);
        assert_eq!(out, SlotUpdateOutcome::Dead { invalidated_slots: vec![10,11,12,13] });
        assert!(g.is_dead(12));
    }
    #[test]
    fn unknown_parent_does_not_invalidate_unrelated_slots() {
        let mut g = SlotIntegrity::default();
        g.observe(20, Some(19), SLOT_PROCESSED);
        let out = g.observe(99, None, SLOT_DEAD);
        assert_eq!(out, SlotUpdateOutcome::Dead { invalidated_slots: vec![99] });
        assert!(!g.is_dead(20));
    }

    #[test]
    fn recreated_bank_reopens_slot_but_not_old_descendants() {
        let mut g = SlotIntegrity::default();
        g.observe(40, Some(39), SLOT_PROCESSED);
        g.observe(41, Some(40), SLOT_PROCESSED);
        g.observe(40, Some(39), SLOT_DEAD);
        assert!(g.is_dead(40) && g.is_dead(41));
        g.observe(40, Some(39), SLOT_CREATED_BANK);
        assert!(!g.is_dead(40));
        assert!(g.is_dead(41));
    }
    #[test]
    fn entry_update_parent_invalidates_only_the_cleared_bank() {
        let mut g = SlotIntegrity::default();
        g.observe_with_bank(50, Some(49), SLOT_CREATED_BANK, Some(700));
        g.observe_with_bank(50, Some(49), SLOT_CREATED_BANK, Some(701));
        assert!(g.invalidate_bank(50, 700));
        assert!(g.is_bank_dead(50, Some(700)));
        assert!(!g.is_bank_dead(50, Some(701)));
        assert!(!g.is_bank_dead(51, Some(700)));
    }

    #[test]
    fn created_bank_without_identity_does_not_reopen_dead_slot() {
        let mut g = SlotIntegrity::default();
        g.observe(61, Some(60), SLOT_DEAD);
        g.observe_with_bank(61, Some(60), SLOT_CREATED_BANK, None);
        assert!(g.is_dead(61));
    }

    #[test]
    fn bank_observed_after_dead_slot_is_tombstoned() {
        let mut g = SlotIntegrity::default();
        g.observe_with_bank(60, Some(59), SLOT_CREATED_BANK, Some(800));
        g.observe_with_bank(60, Some(59), SLOT_DEAD, None);
        g.record_bank_id(60, 800);
        assert!(g.is_bank_dead(60, Some(800)));
        g.observe_with_bank(60, Some(59), SLOT_CREATED_BANK, Some(801));
        assert!(!g.is_dead(60));
        assert!(g.is_bank_dead(60, Some(800)));
        assert!(!g.is_bank_dead(60, Some(801)));
    }

    #[test]
    fn reconciliation_requires_complete_single_context_batch() {
        let mut r = SnapshotReconciler::default();
        let a = [1; 32]; let b = [2; 32];
        r.begin(4, 100, [a,b]).unwrap();
        assert!(!r.record_batch(4, 100, [a]).unwrap());
        assert_eq!(r.missing_count(), 1);
        assert!(r.record_batch(4, 100, [b]).unwrap());
        assert!(r.ready());
    }
    #[test]
    fn reconciliation_rejects_old_context_and_stale_generation() {
        let mut r = SnapshotReconciler::default();
        r.begin(2, 100, [[1;32]]).unwrap();
        assert_eq!(r.record_batch(1, 101, [[1;32]]), Err(ReconcileError::WrongGeneration));
        assert_eq!(r.record_batch(2, 99, [[1;32]]), Err(ReconcileError::ContextSlotTooOld));
    }
}
