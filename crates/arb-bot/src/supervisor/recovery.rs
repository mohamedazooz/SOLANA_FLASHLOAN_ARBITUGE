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
            let status = match self
                .rpc_pool
                .poll_signature_until_terminal(
                    &intent.signature,
                    POLL_ATTEMPTS,
                    POLL_INTERVAL,
                )
                .await
            {
                Ok(s) if s.is_landed() => "landed",
                Ok(arb_ingest::SignatureStatus::Failed) => "failed_onchain",
                _ => {
                    if current_slot >= intent.dispatch_slot + BLOCKHASH_VALIDITY_SLOTS {
                        "expired_unconfirmed"
                    } else {
                        "unresolved_after_poll"
                    }
                }
            };
            warn!(
                attempt_id = intent.attempt_id,
                provider = %intent.provider,
                status,
                "dispatch intent reconciled (no resubmission — new opportunities dispatch fresh transactions)"
            );
            if let Err(err) = self.store.resolve_dispatch_intent(intent.attempt_id, status) {
                warn!(?err, attempt_id = intent.attempt_id, "failed to persist reconciliation");
            }
        }
    }
}
