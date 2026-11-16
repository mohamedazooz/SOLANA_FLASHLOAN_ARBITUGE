//! Startup reconciliation of durable dispatch intents (R12).
//!
//! Best practice (solana.com retry guide + nyk.dev failover pattern): an RPC
//! timeout is ambiguous, never a failure. The safe unit of retry is the signed
//! transaction, and a resubmit decision must be preceded by an on-chain
//! signature check. This module never resubmits — it only reconciles the
//! persisted `dispatch_intents` left `pending` by a restart/crash:
//!
//! 1. `getSignatureStatuses` (with history) — a landed tx is resolved with its
//!    actual on-chain outcome.
//! 2. If still unknown, expiration is proven with BLOCK-HEIGHT EVIDENCE: the
//!    `lastValidBlockHeight` persisted with the dispatch intent (captured from
//!    `getLatestBlockhash` at dispatch time) compared against the live
//!    `getBlockHeight`. Slot arithmetic is never used as a substitute.
//! 3. Otherwise resolve as `unresolved_after_poll` — honest, no invention.
use super::*;

const MAX_PENDING_INTENTS: i64 = 256;
const POLL_ATTEMPTS: usize = 20;
const POLL_INTERVAL: Duration = Duration::from_millis(400);

/// Deterministic reconciliation decision for one pending dispatch intent.
///
/// Pure so recovery behavior is unit-testable without an RPC: a timeout is
/// ambiguous (never a failure), `expired_unconfirmed` requires block-height
/// evidence (live `getBlockHeight` strictly greater than the persisted
/// `lastValidBlockHeight`), and anything else — including missing evidence —
/// stays honestly `unresolved_after_poll`. Never authorizes a resubmit.
pub(super) fn classify_reconciliation(
    poll: Option<arb_ingest::SignatureStatus>,
    block_height_evidence: Option<(i64, i64)>,
) -> &'static str {
    match poll {
        Some(s) if s.is_landed() => "landed",
        Some(arb_ingest::SignatureStatus::Failed) => "failed_onchain",
        _ => match block_height_evidence {
            Some((current_height, last_valid_height)) if current_height > last_valid_height => {
                "expired_unconfirmed"
            }
            _ => "unresolved_after_poll",
        },
    }
}

impl Supervisor {
    /// Reconcile every pending dispatch intent persisted by a previous run.
    /// Called once at startup, before the run loop starts dispatching.
    pub(super) async fn reconcile_pending_dispatches(&self) {
        let pending = match self.store.pending_dispatch_intents(MAX_PENDING_INTENTS) {
            Ok(p) => p,
            Err(err) => {
                warn!(?err, "dispatch-intent reconciliation: store unavailable");
                return;
            }
        };
        if pending.is_empty() {
            return;
        }
        warn!(
            count = pending.len(),
            "reconciling dispatch intents left pending by a previous run"
        );
        let current_block_height = self
            .rpc_pool
            .get_block_height()
            .await
            .ok()
            .map(|h| h as i64);
        for intent in pending {
            let poll = self
                .rpc_pool
                .poll_signature_until_terminal(&intent.signature, POLL_ATTEMPTS, POLL_INTERVAL)
                .await
                .ok();
            let block_height_evidence = match (current_block_height, intent.last_valid_block_height)
            {
                (Some(current), Some(last_valid)) => Some((current, last_valid)),
                _ => None,
            };
            let status = classify_reconciliation(poll, block_height_evidence);
            warn!(
                attempt_id = intent.attempt_id,
                provider = %intent.provider,
                status,
                "dispatch intent reconciled (no resubmission — new opportunities dispatch fresh transactions)"
            );
            if let Err(err) = self
                .store
                .resolve_dispatch_intent(intent.attempt_id, status)
            {
                warn!(
                    ?err,
                    attempt_id = intent.attempt_id,
                    "failed to persist reconciliation"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_ingest::SignatureStatus;

    fn intent(attempt_id: i64) -> arb_store::DispatchIntentRecord {
        arb_store::DispatchIntentRecord {
            attempt_id,
            signature: format!("sig{attempt_id}"),
            provider: "swqos".into(),
            dispatch_slot: 1_000,
            last_valid_block_height: None,
            resolved_status: None,
        }
    }

    #[test]
    fn landed_and_failed_statuses_resolve_to_onchain_outcome() {
        assert_eq!(
            classify_reconciliation(Some(SignatureStatus::Finalized), Some((2_000, 1_100))),
            "landed"
        );
        assert_eq!(
            classify_reconciliation(Some(SignatureStatus::Confirmed), Some((1_001, 1_100))),
            "landed"
        );
        assert_eq!(
            classify_reconciliation(Some(SignatureStatus::Failed), Some((1_001, 1_100))),
            "failed_onchain"
        );
    }

    #[test]
    fn processed_is_not_landed_and_timeout_is_ambiguous() {
        assert_eq!(
            classify_reconciliation(Some(SignatureStatus::Processed), Some((1_100, 1_100))),
            "unresolved_after_poll",
            "processed-but-not-confirmed must stay unresolved, never counted as landed"
        );
        assert_eq!(
            classify_reconciliation(None, Some((1_100, 1_100))),
            "unresolved_after_poll",
            "poll error/timeout is ambiguous — no failure may be invented"
        );
    }

    #[test]
    fn expiry_requires_block_height_evidence_beyond_last_valid_height() {
        assert_eq!(
            classify_reconciliation(None, Some((2_000, 1_100))),
            "expired_unconfirmed",
            "live block height strictly beyond the persisted lastValidBlockHeight proves expiration"
        );
        assert_eq!(
            classify_reconciliation(None, Some((1_100, 1_100))),
            "unresolved_after_poll",
            "height equal to lastValidBlockHeight does NOT prove expiration"
        );
        assert_eq!(
            classify_reconciliation(None, Some((1_099, 1_100))),
            "unresolved_after_poll",
            "one block before expiry must remain unresolved"
        );
    }

    #[test]
    fn missing_or_one_sided_evidence_stays_unresolved() {
        assert_eq!(
            classify_reconciliation(None, None),
            "unresolved_after_poll",
            "no evidence at all must stay unresolved"
        );
        assert_eq!(
            classify_reconciliation(None, Some((2_000, 100))),
            "expired_unconfirmed"
        );
    }

    #[test]
    fn pending_intents_survive_restart_and_resolve_without_resubmission() {
        let store = arb_store::MemoryStore::new();
        store.record_dispatch_intent(&intent(1)).unwrap();
        store.record_dispatch_intent(&intent(2)).unwrap();
        assert_eq!(store.pending_dispatch_intents(256).unwrap().len(), 2);

        store
            .resolve_dispatch_intent(1, "expired_unconfirmed")
            .unwrap();
        let pending = store.pending_dispatch_intents(256).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].attempt_id, 2);

        store.resolve_dispatch_intent(1, "landed").unwrap();
        assert_eq!(store.pending_dispatch_intents(256).unwrap().len(), 1);
    }

    #[test]
    fn sqlite_store_persists_and_roundtrips_last_valid_block_height() {
        let store = arb_store::SqliteStore::open_in_memory().unwrap();
        let parent = |id: i64| arb_store::AttemptRecord {
            id,
            slot: 1_000,
            route_id: "route".into(),
            expected_net: arb_types::Lamports(0),
            tip_lamports: None,
            decision: "dispatch".into(),
        };
        store.record_attempt(&parent(1)).unwrap();
        store.record_attempt(&parent(2)).unwrap();
        let mut with_height = intent(1);
        with_height.last_valid_block_height = Some(4_242);
        store.record_dispatch_intent(&with_height).unwrap();
        let mut without_height = intent(2);
        without_height.last_valid_block_height = None;
        store.record_dispatch_intent(&without_height).unwrap();

        let pending = store.pending_dispatch_intents(256).unwrap();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].last_valid_block_height, Some(4_242));
        assert_eq!(pending[1].last_valid_block_height, None);
    }
}
