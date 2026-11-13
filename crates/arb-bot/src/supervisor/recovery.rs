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
//! 2. If still unknown and the blockhash validity window is provably past
//!    (`current_slot >= dispatch_slot + BLOCKHASH_VALIDITY_SLOTS`), resolve as
//!    `expired_unconfirmed`.
//! 3. Otherwise resolve as `unresolved_after_poll` — honest, no invention.
use super::*;

/// A blockhash stays valid ~150 slots (MAX_LOCKOUT_HISTORY); use a margin.
const BLOCKHASH_VALIDITY_SLOTS: i64 = 160;
const MAX_PENDING_INTENTS: i64 = 256;
const POLL_ATTEMPTS: usize = 20;
const POLL_INTERVAL: Duration = Duration::from_millis(400);

/// Deterministic reconciliation decision for one pending dispatch intent.
///
/// Pure so recovery behavior is unit-testable without an RPC: a timeout is
/// ambiguous (never a failure), `expired_unconfirmed` requires slot-proof that
/// the blockhash validity window is provably past, and anything else stays
/// honestly `unresolved_after_poll`. Never authorizes a resubmit.
pub(super) fn classify_reconciliation(
    poll: Option<arb_ingest::SignatureStatus>,
    current_slot: i64,
    dispatch_slot: i64,
) -> &'static str {
    match poll {
        Some(s) if s.is_landed() => "landed",
        Some(arb_ingest::SignatureStatus::Failed) => "failed_onchain",
        _ => {
            if current_slot >= dispatch_slot + BLOCKHASH_VALIDITY_SLOTS {
                "expired_unconfirmed"
            } else {
                "unresolved_after_poll"
            }
        }
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
        let current_slot = self.ingest.current_slot() as i64;
        for intent in pending {
            let poll = self
                .rpc_pool
                .poll_signature_until_terminal(&intent.signature, POLL_ATTEMPTS, POLL_INTERVAL)
                .await
                .ok();
            let status = classify_reconciliation(poll, current_slot, intent.dispatch_slot);
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
            resolved_status: None,
        }
    }

    #[test]
    fn landed_and_failed_statuses_resolve_to_onchain_outcome() {
        assert_eq!(
            classify_reconciliation(Some(SignatureStatus::Finalized), 2_000, 1_000),
            "landed"
        );
        assert_eq!(
            classify_reconciliation(Some(SignatureStatus::Confirmed), 1_001, 1_000),
            "landed"
        );
        assert_eq!(
            classify_reconciliation(Some(SignatureStatus::Failed), 1_001, 1_000),
            "failed_onchain"
        );
    }

    #[test]
    fn processed_is_not_landed_and_timeout_is_ambiguous() {
        assert_eq!(
            classify_reconciliation(Some(SignatureStatus::Processed), 1_100, 1_000),
            "unresolved_after_poll",
            "processed-but-not-confirmed must stay unresolved, never counted as landed"
        );
        assert_eq!(
            classify_reconciliation(None, 1_100, 1_000),
            "unresolved_after_poll",
            "poll error/timeout is ambiguous — no failure may be invented"
        );
    }

    #[test]
    fn expiry_requires_slot_proof_of_blockhash_window() {
        assert_eq!(
            classify_reconciliation(None, 1_000 + 160, 1_000),
            "expired_unconfirmed"
        );
        assert_eq!(
            classify_reconciliation(None, 1_000 + 159, 1_000),
            "unresolved_after_poll",
            "one slot before the proven window must remain unresolved"
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
}
