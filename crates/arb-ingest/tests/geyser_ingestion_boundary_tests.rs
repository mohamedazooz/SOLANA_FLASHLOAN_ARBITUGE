//! Feature 21: Yellowstone Geyser gRPC Streaming & Slot Ordering — Tier 2 Boundary Tests.
//!
//! Validates:
//! 1. Duplicate slot notifications for the same slot are deduped (gaps_detected remains 0).
//! 2. Out-of-order / late slot arrivals do not artificially inflate gaps_detected.
//! 3. Same-slot write_version ordering: stale write_version is rejected, fresh accepted.
//! 4. write_version at u64::MAX boundary prevents integer overflow panic, rejects duplicates.
//! 5. Stream disconnect/reconnect gap tracking and snapshot freshness reconciliation.

use arb_ingest::slot_integrity::{
    SLOT_FINALIZED, SLOT_PROCESSED, SlotIntegrity, SlotUpdateOutcome,
};
use arb_ingest::{AccountSnapshot, AccountSnapshotStore, ProviderId};
use solana_pubkey::Pubkey;
use std::sync::Arc;

fn make_snapshot(key: Pubkey, slot: u64, write_version: u64, byte: u8) -> AccountSnapshot {
    AccountSnapshot {
        pubkey: key,
        owner: Pubkey::new_from_array([2u8; 32]),
        slot,
        write_version,
        is_startup: false,
        provider_id: ProviderId(1),
        bank_id: Some(1),
        generation: 0,
        data: Arc::new(vec![byte]),
    }
}

/// Boundary Case 1: Duplicate slot notifications are deduplicated without inflating gaps.
#[test]
fn test_slot_integrity_duplicate_slot_notifications_deduped() {
    let mut integrity = SlotIntegrity::default();

    // Initial baseline
    integrity.observe(99, None, SLOT_FINALIZED);

    // Feed slot 100 with parent 99 multiple times (processed, confirmed, finalized)
    for _ in 0..5 {
        let outcome = integrity.observe(100, Some(99), SLOT_PROCESSED);
        assert_eq!(outcome, SlotUpdateOutcome::Observed);
    }

    assert_eq!(integrity.highest_seen(), 100);
    assert_eq!(
        integrity.gaps_detected(),
        0,
        "Duplicate slot notifications must not increment gaps_detected"
    );
}

/// Boundary Case 2: Out-of-order slot notifications do not artificially inflate gaps.
#[test]
fn test_slot_integrity_out_of_order_slot_arrival() {
    let mut integrity = SlotIntegrity::default();

    // Baseline: slot 100
    integrity.observe(100, None, SLOT_FINALIZED);

    // Jump to 105 (with parent 104 -> gap 101, 102, 103 = 3 missing slots)
    integrity.observe(105, Some(104), SLOT_PROCESSED);
    let gaps_after_jump = integrity.gaps_detected();
    assert_eq!(gaps_after_jump, 3);

    // Late arriving slots (102, then 104) are within the already seen window [100..105]
    integrity.observe(102, Some(101), SLOT_PROCESSED);
    integrity.observe(104, Some(103), SLOT_PROCESSED);

    // Gaps detected must remain unchanged at 4 (never inflated by late arrivals)
    assert_eq!(
        integrity.gaps_detected(),
        gaps_after_jump,
        "Late/reordered slot notifications must never inflate gaps_detected"
    );

    // Slot 106 arrives with parent 105 (contiguous)
    integrity.observe(106, Some(105), SLOT_PROCESSED);
    assert_eq!(integrity.highest_seen(), 106);
    assert_eq!(integrity.gaps_detected(), gaps_after_jump);
}

/// Boundary Case 3: Same-slot write_version ordering (reject stale, accept fresh).
#[test]
fn test_account_snapshot_same_slot_write_version_ordering() {
    let store = AccountSnapshotStore::default();
    let key = Pubkey::new_unique();

    // Initial update at slot 500, write_version 10
    assert!(store.apply(make_snapshot(key, 500, 10, 1)));
    assert_eq!(store.get(&key).unwrap().data.as_slice(), &[1]);

    // Stale update: slot 500, write_version 9 -> must be rejected
    assert!(
        !store.apply(make_snapshot(key, 500, 9, 2)),
        "Older write_version at same slot must be rejected"
    );
    assert_eq!(store.get(&key).unwrap().data.as_slice(), &[1]);

    // Fresh update: slot 500, write_version 11 -> must be accepted
    assert!(
        store.apply(make_snapshot(key, 500, 11, 3)),
        "Newer write_version at same slot must be accepted"
    );
    assert_eq!(store.get(&key).unwrap().data.as_slice(), &[3]);
}

/// Boundary Case 4: write_version at u64::MAX boundary prevents integer overflow panic.
#[test]
fn test_account_snapshot_write_version_u64_max_boundary() {
    let store = AccountSnapshotStore::default();
    let key = Pubkey::new_unique();

    // Apply update with write_version = u64::MAX
    assert!(store.apply(make_snapshot(key, 100, u64::MAX, 0xaa)));

    // Duplicate (slot 100, write_version u64::MAX) must be rejected cleanly
    assert!(
        !store.apply(make_snapshot(key, 100, u64::MAX, 0xbb)),
        "Duplicate (slot, write_version::MAX) must be rejected"
    );

    // Stale slot with u64::MAX must also be rejected
    assert!(
        !store.apply(make_snapshot(key, 99, u64::MAX, 0xcc)),
        "Stale slot with write_version::MAX must be rejected"
    );

    // Fresher slot advances state cleanly
    assert!(
        store.apply(make_snapshot(key, 101, 1, 0xdd)),
        "Newer slot must supersede previous slot regardless of write_version"
    );
    assert_eq!(store.get(&key).unwrap().data.as_slice(), &[0xdd]);
}

/// Boundary Case 5: Stream disconnect/reconnect slot gap tracking and snapshot freshness.
#[test]
fn test_stream_disconnect_reconnect_slot_reconciliation() {
    let mut integrity = SlotIntegrity::default();
    let store = AccountSnapshotStore::default();
    let key = Pubkey::new_unique();

    // 1. Pre-disconnect state at slot 1,000
    integrity.observe(1_000, Some(999), SLOT_PROCESSED);
    assert!(store.apply(make_snapshot(key, 1_000, 50, 0x11)));

    // 2. Stream drops and reconnects 50 slots later at slot 1,050 (parent 1,049)
    integrity.observe(1_050, Some(1_049), SLOT_PROCESSED);

    // Gap: 1049 - 1000 - 1 = 48 slots
    assert_eq!(integrity.gaps_detected(), 48);
    assert_eq!(integrity.highest_seen(), 1_050);

    // 3. Post-reconnect snapshot applied
    assert!(store.apply(make_snapshot(key, 1_050, 1, 0x22)));

    // 4. Stale pre-disconnect snapshot replayed -> rejected
    assert!(
        !store.apply(make_snapshot(key, 1_000, 999, 0x33)),
        "Pre-disconnect snapshot must be rejected after fresh post-reconnect state"
    );
    assert_eq!(store.get(&key).unwrap().data.as_slice(), &[0x22]);
}
