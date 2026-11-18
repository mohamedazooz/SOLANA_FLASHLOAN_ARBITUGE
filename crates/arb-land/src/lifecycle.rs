//! Transaction Lifecycle State Machine & Confirmation Engine.
//!
//! Enforces deterministic lifecycle progression:
//! PREPARED -> SIMULATED -> SIGNED -> SUBMITTED -> PROCESSED -> CONFIRMED -> FINALIZED
//! or FAILED / EXPIRED / UNKNOWN.
//!
//! Critical Invariants:
//! 1. Never classify a transaction as expired solely because a polling timeout elapsed.
//! 2. Expiration requires positive on-chain proof: current block height > last_valid_block_height.
//! 3. Idempotent reconciliation: state transitions are strictly directional and monotonic.
//! 4. Processed is distinct from Confirmed and Finalized.

use serde::{Deserialize, Serialize};

/// Canonical 10-state lifecycle state machine for Solana transactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TransactionLifecycle {
    /// Transaction instructions assembled and validated.
    Prepared,
    /// Pre-flight RPC or local simulation passed.
    Simulated,
    /// Transaction signed by authoritative fee payer key.
    Signed,
    /// Transaction payload dispatched to RPC, SWQoS, or Jito block engine.
    Submitted,
    /// Included in a slot by a validator (processed commitment).
    Processed,
    /// Reached supermajority consensus (confirmed commitment, ~66%+ stake).
    Confirmed,
    /// 31+ confirmed blocks built on top; permanent on-chain finality.
    Finalized,
    /// Explicit on-chain failure or simulation revert.
    Failed,
    /// Proven expired: block height exceeded last_valid_block_height and signature not found.
    Expired,
    /// Ambiguous / in-flight: polling timeout elapsed while blockhash is still valid. Never blindly resend!
    Unknown,
}

impl std::fmt::Display for TransactionLifecycle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Prepared => write!(f, "PREPARED"),
            Self::Simulated => write!(f, "SIMULATED"),
            Self::Signed => write!(f, "SIGNED"),
            Self::Submitted => write!(f, "SUBMITTED"),
            Self::Processed => write!(f, "PROCESSED"),
            Self::Confirmed => write!(f, "CONFIRMED"),
            Self::Finalized => write!(f, "FINALIZED"),
            Self::Failed => write!(f, "FAILED"),
            Self::Expired => write!(f, "EXPIRED"),
            Self::Unknown => write!(f, "UNKNOWN"),
        }
    }
}

impl TransactionLifecycle {
    /// True when the transaction has reached a landed commitment on-chain.
    pub fn is_landed(&self) -> bool {
        matches!(self, Self::Processed | Self::Confirmed | Self::Finalized)
    }

    /// True when the transaction has reached durable consensus or finality.
    pub fn is_confirmed(&self) -> bool {
        matches!(self, Self::Confirmed | Self::Finalized)
    }

    /// True when state cannot transition further.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Finalized | Self::Failed | Self::Expired)
    }

    /// Human-readable landing status assertion, strictly bounded by what the
    /// verification track actually proved (PIPE-12). Never claims
    /// Confirmed/Finalized unless the lifecycle state carries that level;
    /// unproven landings are labelled as such.
    pub fn landing_status_str(self) -> &'static str {
        match self {
            Self::Finalized => "Finalized (نهائي على السلسلة)",
            Self::Confirmed => "Confirmed (مؤكد على السلسلة)",
            Self::Processed => "Processed (في البلوك، بانتظار التأكيد)",
            _ => "Landed (في البلوك، مستوى التأكيد غير مثبت)",
        }
    }

    /// Check if transition from `self` to `next` is valid under monotonic ordering.
    pub fn can_transition_to(&self, next: TransactionLifecycle) -> bool {
        if *self == next {
            return true;
        }
        match self {
            Self::Prepared => matches!(next, Self::Simulated | Self::Failed),
            Self::Simulated => matches!(next, Self::Signed | Self::Failed),
            Self::Signed => matches!(next, Self::Submitted | Self::Failed),
            Self::Submitted => matches!(
                next,
                Self::Processed
                    | Self::Confirmed
                    | Self::Finalized
                    | Self::Failed
                    | Self::Expired
                    | Self::Unknown
            ),
            Self::Processed => matches!(
                next,
                Self::Confirmed | Self::Finalized | Self::Failed | Self::Expired
            ),
            Self::Confirmed => matches!(next, Self::Finalized),
            Self::Unknown => matches!(
                next,
                Self::Processed | Self::Confirmed | Self::Finalized | Self::Failed | Self::Expired
            ),
            Self::Finalized | Self::Failed | Self::Expired => false,
        }
    }
}

/// Detailed on-chain execution audit record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionAuditRecord {
    pub signature: String,
    pub slot: u64,
    pub blockhash: String,
    pub last_valid_block_height: u64,
    pub provider: String,
    pub transport: String,
    pub lifecycle_state: TransactionLifecycle,
    pub failure_reason: Option<String>,
}

/// Evaluate landing status using last_valid_block_height vs current cluster block height.
///
/// RULE: A transaction is NEVER marked Expired simply because polling timed out.
/// It is only Expired if current_block_height > last_valid_block_height.
/// If current_block_height <= last_valid_block_height, it remains UNKNOWN (unresolved).
pub fn evaluate_lifecycle_with_block_height(
    confirmed_status: Option<&str>,
    has_err: bool,
    err_msg: Option<&str>,
    current_block_height: u64,
    last_valid_block_height: u64,
) -> (TransactionLifecycle, Option<String>) {
    if has_err {
        let reason = err_msg.unwrap_or("Transaction failed on-chain");
        return (TransactionLifecycle::Failed, Some(reason.to_string()));
    }

    if let Some(status) = confirmed_status {
        match status.to_ascii_lowercase().as_str() {
            "finalized" => (TransactionLifecycle::Finalized, None),
            "confirmed" => (TransactionLifecycle::Confirmed, None),
            "processed" => (TransactionLifecycle::Processed, None),
            _ => (TransactionLifecycle::Unknown, None),
        }
    } else {
        // No status returned from RPC
        if current_block_height > last_valid_block_height {
            (
                TransactionLifecycle::Expired,
                Some(format!(
                    "Blockhash expired: current block height {current_block_height} > lastValidBlockHeight {last_valid_block_height}"
                )),
            )
        } else {
            (
                TransactionLifecycle::Unknown,
                Some(format!(
                    "In-flight status unconfirmed, but blockhash still valid (height {current_block_height} <= {last_valid_block_height})"
                )),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lifecycle_monotonic_transitions() {
        assert!(TransactionLifecycle::Prepared.can_transition_to(TransactionLifecycle::Simulated));
        assert!(TransactionLifecycle::Simulated.can_transition_to(TransactionLifecycle::Signed));
        assert!(TransactionLifecycle::Signed.can_transition_to(TransactionLifecycle::Submitted));
        assert!(TransactionLifecycle::Submitted.can_transition_to(TransactionLifecycle::Processed));
        assert!(TransactionLifecycle::Processed.can_transition_to(TransactionLifecycle::Confirmed));
        assert!(TransactionLifecycle::Confirmed.can_transition_to(TransactionLifecycle::Finalized));

        // Terminal states cannot transition
        assert!(
            !TransactionLifecycle::Finalized.can_transition_to(TransactionLifecycle::Processed)
        );
        assert!(!TransactionLifecycle::Failed.can_transition_to(TransactionLifecycle::Submitted));
        assert!(!TransactionLifecycle::Expired.can_transition_to(TransactionLifecycle::Confirmed));
    }

    #[test]
    fn test_never_expire_on_timeout_when_height_valid() {
        let current_height = 250_000_000;
        let last_valid = 250_000_150;

        // Polling returned no status (None), but height is still valid
        let (state, reason) =
            evaluate_lifecycle_with_block_height(None, false, None, current_height, last_valid);

        assert_eq!(state, TransactionLifecycle::Unknown);
        assert!(reason.unwrap().contains("blockhash still valid"));
    }

    #[test]
    fn test_expire_only_when_height_exceeded() {
        let current_height = 250_000_200;
        let last_valid = 250_000_150;

        // Polling returned no status (None), and height is strictly exceeded
        let (state, reason) =
            evaluate_lifecycle_with_block_height(None, false, None, current_height, last_valid);

        assert_eq!(state, TransactionLifecycle::Expired);
        assert!(reason.unwrap().contains("Blockhash expired"));
    }

    #[test]
    fn test_failure_takes_precedence_with_reason() {
        let (state, reason) = evaluate_lifecycle_with_block_height(
            Some("processed"),
            true,
            Some("InstructionError(2, Custom(6033))"),
            250_000_000,
            250_000_150,
        );

        assert_eq!(state, TransactionLifecycle::Failed);
        assert_eq!(reason.unwrap(), "InstructionError(2, Custom(6033))");
    }

    #[test]
    fn test_distinguish_processed_confirmed_finalized() {
        let (s1, _) = evaluate_lifecycle_with_block_height(Some("processed"), false, None, 1, 100);
        assert_eq!(s1, TransactionLifecycle::Processed);
        assert!(s1.is_landed());
        assert!(!s1.is_confirmed());

        let (s2, _) = evaluate_lifecycle_with_block_height(Some("confirmed"), false, None, 1, 100);
        assert_eq!(s2, TransactionLifecycle::Confirmed);
        assert!(s2.is_landed());
        assert!(s2.is_confirmed());

        let (s3, _) = evaluate_lifecycle_with_block_height(Some("finalized"), false, None, 1, 100);
        assert_eq!(s3, TransactionLifecycle::Finalized);
        assert!(s3.is_landed());
        assert!(s3.is_confirmed());
        assert!(s3.is_terminal());
    }

    // PIPE-12 regression: the landing status string must never claim a higher
    // commitment level than the lifecycle actually proved.
    #[test]
    fn landing_status_str_matches_proved_commitment_level() {
        assert!(
            TransactionLifecycle::Finalized
                .landing_status_str()
                .contains("Finalized")
        );
        assert!(
            TransactionLifecycle::Confirmed
                .landing_status_str()
                .contains("Confirmed")
        );
        assert!(
            TransactionLifecycle::Processed
                .landing_status_str()
                .contains("Processed")
        );
        // Unproven / ambiguous states must NOT claim Confirmed or Finalized.
        for state in [
            TransactionLifecycle::Prepared,
            TransactionLifecycle::Simulated,
            TransactionLifecycle::Signed,
            TransactionLifecycle::Submitted,
            TransactionLifecycle::Unknown,
        ] {
            let s = state.landing_status_str();
            assert!(!s.contains("Finalized"), "{state} claims Finalized: {s}");
            assert!(!s.contains("Confirmed"), "{state} claims Confirmed: {s}");
        }
    }
}
