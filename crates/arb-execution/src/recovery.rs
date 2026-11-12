//! Phase 16.5 — Failure Recovery & Execution State Machine.
//!
//! Explicit execution state management for atomic arbitrage transactions.
//!
//! State machine:
//!   Discovered → Quoted → Validated → Built → Simulated → Signed → Submitted
//!   → Pending → Landed → Confirmed → Finalized → Reconciled
//!   (terminal failures: Failed | Expired | Rejected)
//!
//! Recovery rules:
//! 1. Never blindly retry an uncertain transaction.
//! 2. Determine whether the original signature landed before retrying.
//! 3. Maintain transaction signatures and bundle identifiers.
//! 4. Use bounded retries with exponential backoff and jitter.
//! 5. Rebuild transactions when blockhashes expire.
//! 6. Revalidate pool state before rebuilding.
//! 7. Recalculate profitability before every new attempt.
//! 8. Apply idempotency protections.
//! 9. Prevent duplicate trade execution.
//! 10. Record every state transition.

use arb_types::Slot;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::result::Result as StdResult;
use std::sync::Arc;
use tracing::{debug, info};

/// Result type for state machine operations.
pub type StateMachineResult<T> = StdResult<T, StateMachineError>;

/// Execution states for an arbitrage opportunity.
///
/// The normal happy path flows through all states left-to-right. Failure
/// modes branch off to `Failed`, `Expired`, or `Rejected`, which are
/// terminal unless explicitly reset by an operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ExecutionState {
    /// Route detected and initial quote obtained.
    Discovered,
    /// Quote validated against freshness and profitability constraints.
    Quoted,
    /// Route optimizer confirmed optimal trade size.
    Validated,
    /// Transaction instructions built (borrow → swaps → repay → tip).
    Built,
    /// Pre-flight simulation passed profitability gate.
    Simulated,
    /// Transaction signed with payer keypair.
    Signed,
    /// Transaction submitted to RPC / Jito block engine.
    Submitted,
    /// Awaiting landing confirmation (bundle status polling).
    Pending,
    /// Transaction/bundle landed on-chain (in a block).
    Landed,
    /// Transaction confirmed (sufficient confirmations).
    Confirmed,
    /// Transaction finalized (irreversible).
    Finalized,
    /// Outcome recorded, profit/loss accounted.
    Reconciled,
    /// Terminal: execution failed (reverted, simulation error, etc.).
    Failed,
    /// Terminal: blockhash expired before submission.
    Expired,
    /// Terminal: route rejected (profit floor not met, risk gate, etc.).
    Rejected,
}

impl ExecutionState {
    /// True when the state is terminal (no further transitions possible).
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Reconciled | Self::Failed | Self::Expired | Self::Rejected
        )
    }

    /// True when the state represents a successful final outcome.
    pub fn is_success(self) -> bool {
        matches!(self, Self::Finalized | Self::Reconciled)
    }

    /// True when the state represents a terminal failure.
    pub fn is_failure(self) -> bool {
        matches!(self, Self::Failed | Self::Expired | Self::Rejected)
    }

    /// Human-readable description for logs and telemetry.
    pub fn label(self) -> &'static str {
        match self {
            Self::Discovered => "discovered",
            Self::Quoted => "quoted",
            Self::Validated => "validated",
            Self::Built => "built",
            Self::Simulated => "simulated",
            Self::Signed => "signed",
            Self::Submitted => "submitted",
            Self::Pending => "pending",
            Self::Landed => "landed",
            Self::Confirmed => "confirmed",
            Self::Finalized => "finalized",
            Self::Reconciled => "reconciled",
            Self::Failed => "failed",
            Self::Expired => "expired",
            Self::Rejected => "rejected",
        }
    }
}

/// Allowed state transitions.
/// Returns `Ok(())` when `from` may transition to `to`, `Err` otherwise.
fn valid_transition(from: ExecutionState, to: ExecutionState) -> StateMachineResult<()> {
    use ExecutionState::*;

    // Terminal states cannot transition further.
    if from.is_terminal() {
        return Err(StateMachineError::InvalidTransition {
            from,
            to,
            reason: format!("state {from:?} is terminal and cannot transition to {to:?}"),
        });
    }

    // Terminal states cannot transition further.
    if from.is_terminal() {
        return Err(StateMachineError::InvalidTransition {
            from,
            to,
            reason: format!("state {from:?} is terminal and cannot transition to {to:?}"),
        });
    }

    let allowed = match from {
        Discovered => matches!(to, Quoted | Rejected | Expired),
        Quoted => matches!(to, Validated | Rejected | Expired),
        Validated => matches!(to, Built | Rejected | Expired),
        Built => matches!(to, Simulated | Rejected | Expired),
        Simulated => matches!(to, Signed | Rejected | Failed | Expired),
        Signed => matches!(to, Submitted | Rejected | Expired),
        Submitted => matches!(to, Pending | Failed | Rejected | Expired),
        Pending => matches!(to, Landed | Failed | Rejected | Expired),
        Landed => matches!(to, Confirmed | Failed | Rejected),
        Confirmed => matches!(to, Finalized | Failed | Rejected),
        Finalized => matches!(to, Reconciled),
        Reconciled => matches!(to, Discovered), // allow restart for new cycle
        // Terminal states already handled above.
        Failed | Expired | Rejected => false,
    };

    if allowed {
        Ok(())
    } else {
        Err(StateMachineError::InvalidTransition {
            from,
            to,
            reason: format!("invalid transition from {from:?} to {to:?}"),
        })
    }
}

/// Structured error types for the execution state machine.
#[derive(Debug, thiserror::Error)]
pub enum StateMachineError {
    #[error("invalid state transition from {from:?} to {to:?}: {reason}")]
    InvalidTransition {
        from: ExecutionState,
        to: ExecutionState,
        reason: String,
    },

    #[error("opportunity {opportunity_id:?} already in state {current:?}")]
    DuplicateState {
        opportunity_id: u64,
        current: ExecutionState,
    },

    #[error(
        "idempotency key collision: opportunity {opportunity_id:?} already executed with key {key}"
    )]
    IdempotencyCollision { opportunity_id: u64, key: String },

    #[error("duplicate execution detected for opportunity {opportunity_id:?}")]
    DuplicateExecution { opportunity_id: u64 },

    #[error("blockhash expired at slot {slot:?}, max {max:?}")]
    BlockhashExpired { slot: Slot, max: Slot },

    #[error("retry budget exhausted: {attempts} attempts, max {max:?}")]
    RetryBudgetExhausted { attempts: u32, max: u32 },

    #[error("signature {signature:?} not found for opportunity {opportunity_id:?}")]
    SignatureNotFound {
        opportunity_id: u64,
        signature: Option<String>,
    },

    #[error("bundle {bundle_id:?} not found for opportunity {opportunity_id:?}")]
    BundleNotFound {
        opportunity_id: u64,
        bundle_id: Option<String>,
    },

    #[error("circuit breaker is open")]
    CircuitBreakerOpen,

    #[error("pool state stale: slot {slot:?}, max age {max:?}")]
    StalePoolState { slot: Slot, max: Slot },

    #[error("profitability check failed: {reason}")]
    ProfitabilityFailed { reason: String },

    #[error("internal error: {0}")]
    Internal(String),
}

/// Result of a state transition attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateTransition {
    pub opportunity_id: u64,
    pub from: ExecutionState,
    pub to: ExecutionState,
    pub timestamp_ms: u64,
    pub reason: Option<String>,
}

/// Retry configuration with exponential backoff and jitter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetryConfig {
    /// Maximum number of retry attempts.
    pub max_attempts: u32,
    /// Base delay in milliseconds.
    pub base_delay_ms: u64,
    /// Maximum delay in milliseconds.
    pub max_delay_ms: u64,
    /// Jitter factor (0.0 = no jitter, 1.0 = full jitter).
    pub jitter_factor: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_delay_ms: 100,
            max_delay_ms: 30_000,
            jitter_factor: 0.5,
        }
    }
}

/// Compute the backoff delay for attempt `n` (0-indexed) with exponential
/// backoff and bounded jitter.
pub fn backoff_delay_ms(attempt: u32, config: &RetryConfig) -> u64 {
    let exponential = config.base_delay_ms * 2u64.saturating_pow(attempt);
    let capped = exponential.min(config.max_delay_ms);
    if config.jitter_factor == 0.0 {
        return capped;
    }
    let jitter_range = (capped as f64) * config.jitter_factor;
    let jitter = (jitter_range as u64).saturating_div(2);
    capped.saturating_add(jitter)
}

/// Idempotency key for deduplication.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IdempotencyKey(pub String);

impl IdempotencyKey {
    pub fn new(opportunity_id: u64, attempt: u32) -> Self {
        Self(format!("{opportunity_id}:{attempt}"))
    }
}

/// Execution context tracked for each opportunity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionContext {
    pub opportunity_id: u64,
    pub state: ExecutionState,
    /// Signatures observed for this opportunity (used to determine if a
    /// previous attempt landed before retrying).
    pub signatures: HashSet<String>,
    /// Bundle identifiers for Jito submissions.
    pub bundle_ids: HashSet<String>,
    /// Number of attempts made.
    pub attempt_count: u32,
    /// Last state transition timestamp.
    pub last_transition_ms: u64,
    /// Idempotency keys already used.
    pub idempotency_keys: HashSet<IdempotencyKey>,
    /// Whether the opportunity has been marked as executed (duplicate prevention).
    pub executed: bool,
    /// Blockhash slot at signing time (for expiry detection).
    pub blockhash_slot: Option<Slot>,
    /// Maximum slot before blockhash expires.
    pub blockhash_max_slot: Option<Slot>,
}

impl ExecutionContext {
    pub fn new(opportunity_id: u64) -> Self {
        Self {
            opportunity_id,
            state: ExecutionState::Discovered,
            signatures: HashSet::new(),
            bundle_ids: HashSet::new(),
            attempt_count: 0,
            last_transition_ms: now_ms(),
            idempotency_keys: HashSet::new(),
            executed: false,
            blockhash_slot: None,
            blockhash_max_slot: None,
        }
    }
}

/// Thread-safe execution state machine.
///
/// Tracks the lifecycle of each arbitrage opportunity through its execution
/// states, enforcing valid transitions, idempotency, retry bounds, and
/// duplicate execution prevention.
#[derive(Debug, Clone)]
pub struct ExecutionStateMachine {
    contexts: Arc<Mutex<HashMap<u64, ExecutionContext>>>,
    retry_config: RetryConfig,
    /// Circuit breaker reference (external). When open, new submissions are rejected.
    circuit_breaker_open: Arc<Mutex<bool>>,
}

impl Default for ExecutionStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl ExecutionStateMachine {
    /// Create a new state machine with default retry configuration.
    pub fn new() -> Self {
        Self {
            contexts: Arc::new(Mutex::new(HashMap::new())),
            retry_config: RetryConfig::default(),
            circuit_breaker_open: Arc::new(Mutex::new(false)),
        }
    }

    /// Create with custom retry configuration.
    pub fn with_retry_config(config: RetryConfig) -> Self {
        Self {
            contexts: Arc::new(Mutex::new(HashMap::new())),
            retry_config: config,
            circuit_breaker_open: Arc::new(Mutex::new(false)),
        }
    }

    /// Set the circuit breaker state (open = reject all new submissions).
    pub fn set_circuit_breaker_open(&self, open: bool) {
        *self.circuit_breaker_open.lock() = open;
    }

    /// Is the circuit breaker open?
    pub fn is_circuit_breaker_open(&self) -> bool {
        *self.circuit_breaker_open.lock()
    }

    /// Register a new opportunity at the `Discovered` state.
    /// Returns an error if the opportunity already exists.
    pub fn register(&self, opportunity_id: u64) -> StateMachineResult<()> {
        let mut contexts = self.contexts.lock();
        if contexts.contains_key(&opportunity_id) {
            return Err(StateMachineError::DuplicateState {
                opportunity_id,
                current: contexts[&opportunity_id].state,
            });
        }
        contexts.insert(opportunity_id, ExecutionContext::new(opportunity_id));
        debug!(
            opportunity_id,
            "registered opportunity in state: discovered"
        );
        Ok(())
    }

    /// Transition an opportunity from `from` to `to`, recording the transition.
    ///
    /// Validates the transition is permitted, records the timestamp, and
    /// updates the opportunity's state.
    pub fn transition(
        &self,
        opportunity_id: u64,
        from: ExecutionState,
        to: ExecutionState,
        reason: Option<&str>,
    ) -> StateMachineResult<StateTransition> {
        let mut contexts = self.contexts.lock();
        let ctx = contexts.get_mut(&opportunity_id).ok_or_else(|| {
            StateMachineError::Internal(format!("opportunity {opportunity_id} not registered"))
        })?;

        if ctx.state != from {
            return Err(StateMachineError::InvalidTransition {
                from,
                to,
                reason: format!(
                    "opportunity is in state {:?}, expected {:?}",
                    ctx.state, from
                ),
            });
        }

        valid_transition(from, to)?;

        let now = now_ms();
        let transition = StateTransition {
            opportunity_id,
            from,
            to,
            timestamp_ms: now,
            reason: reason.map(|s| s.to_string()),
        };

        ctx.state = to;
        ctx.last_transition_ms = now;

        // Mark executed when reaching a terminal success state.
        if to.is_success() {
            ctx.executed = true;
        }

        info!(
            opportunity_id,
            from = ?from,
            to = ?to,
            reason = reason.unwrap_or(""),
            "state transition"
        );

        Ok(transition)
    }

    /// Transition without specifying the expected `from` state (uses current).
    pub fn transition_to(
        &self,
        opportunity_id: u64,
        to: ExecutionState,
        reason: Option<&str>,
    ) -> StateMachineResult<StateTransition> {
        let contexts = self.contexts.lock();
        let from = contexts
            .get(&opportunity_id)
            .ok_or_else(|| {
                StateMachineError::Internal(format!("opportunity {opportunity_id} not registered"))
            })?
            .state;
        drop(contexts);
        self.transition(opportunity_id, from, to, reason)
    }

    /// Get the current state of an opportunity.
    pub fn state(&self, opportunity_id: u64) -> Option<ExecutionState> {
        self.contexts
            .lock()
            .get(&opportunity_id)
            .map(|ctx| ctx.state)
    }

    /// Record a signature for an opportunity (used to detect if a previous
    /// submission landed before retrying).
    pub fn record_signature(&self, opportunity_id: u64, signature: &str) -> StateMachineResult<()> {
        let mut contexts = self.contexts.lock();
        let ctx = contexts.get_mut(&opportunity_id).ok_or_else(|| {
            StateMachineError::Internal(format!("opportunity {opportunity_id} not registered"))
        })?;
        ctx.signatures.insert(signature.to_string());
        Ok(())
    }

    /// Record a bundle identifier for an opportunity.
    pub fn record_bundle_id(&self, opportunity_id: u64, bundle_id: &str) -> StateMachineResult<()> {
        let mut contexts = self.contexts.lock();
        let ctx = contexts.get_mut(&opportunity_id).ok_or_else(|| {
            StateMachineError::Internal(format!("opportunity {opportunity_id} not registered"))
        })?;
        ctx.bundle_ids.insert(bundle_id.to_string());
        Ok(())
    }

    /// Check whether any previously recorded signature for this opportunity
    /// has landed on-chain. This is the critical "determine whether the
    /// original signature landed before retrying" check.
    ///
    /// `is_landed` is a caller-provided predicate that checks a signature
    /// against the RPC.
    pub fn has_landed_signature<F>(
        &self,
        opportunity_id: u64,
        is_landed: F,
    ) -> StateMachineResult<bool>
    where
        F: Fn(&str) -> StateMachineResult<bool>,
    {
        let contexts = self.contexts.lock();
        let ctx = contexts.get(&opportunity_id).ok_or_else(|| {
            StateMachineError::Internal(format!("opportunity {opportunity_id} not registered"))
        })?;
        let signatures: Vec<String> = ctx.signatures.iter().cloned().collect();
        drop(contexts);

        for sig in signatures {
            if is_landed(&sig)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Check whether any previously recorded bundle ID has landed.
    pub fn has_landed_bundle<F>(
        &self,
        opportunity_id: u64,
        is_landed: F,
    ) -> StateMachineResult<bool>
    where
        F: Fn(&str) -> StateMachineResult<bool>,
    {
        let contexts = self.contexts.lock();
        let ctx = contexts.get(&opportunity_id).ok_or_else(|| {
            StateMachineError::Internal(format!("opportunity {opportunity_id} not registered"))
        })?;
        let bundle_ids: Vec<String> = ctx.bundle_ids.iter().cloned().collect();
        drop(contexts);

        for bid in bundle_ids {
            if is_landed(&bid)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Check idempotency: returns `Ok(())` if the key is new, `Err` if already used.
    pub fn check_idempotency(
        &self,
        opportunity_id: u64,
        key: &IdempotencyKey,
    ) -> StateMachineResult<()> {
        let mut contexts = self.contexts.lock();
        let ctx = contexts.get_mut(&opportunity_id).ok_or_else(|| {
            StateMachineError::Internal(format!("opportunity {opportunity_id} not registered"))
        })?;
        if ctx.idempotency_keys.contains(key) {
            return Err(StateMachineError::IdempotencyCollision {
                opportunity_id,
                key: key.0.clone(),
            });
        }
        ctx.idempotency_keys.insert(key.clone());
        Ok(())
    }

    /// Mark an opportunity as executed (prevents duplicate execution).
    pub fn mark_executed(&self, opportunity_id: u64) -> StateMachineResult<()> {
        let mut contexts = self.contexts.lock();
        let ctx = contexts.get_mut(&opportunity_id).ok_or_else(|| {
            StateMachineError::Internal(format!("opportunity {opportunity_id} not registered"))
        })?;
        if ctx.executed {
            return Err(StateMachineError::DuplicateExecution { opportunity_id });
        }
        ctx.executed = true;
        Ok(())
    }

    /// Check whether this opportunity has already been executed.
    pub fn is_executed(&self, opportunity_id: u64) -> StateMachineResult<bool> {
        let contexts = self.contexts.lock();
        Ok(contexts
            .get(&opportunity_id)
            .map(|ctx| ctx.executed)
            .unwrap_or(false))
    }

    /// Increment the attempt counter. Returns `Err` if max attempts exceeded.
    pub fn increment_attempt(&self, opportunity_id: u64) -> StateMachineResult<u32> {
        let mut contexts = self.contexts.lock();
        let ctx = contexts.get_mut(&opportunity_id).ok_or_else(|| {
            StateMachineError::Internal(format!("opportunity {opportunity_id} not registered"))
        })?;
        ctx.attempt_count = ctx.attempt_count.saturating_add(1);
        if ctx.attempt_count > self.retry_config.max_attempts {
            return Err(StateMachineError::RetryBudgetExhausted {
                attempts: ctx.attempt_count,
                max: self.retry_config.max_attempts,
            });
        }
        Ok(ctx.attempt_count)
    }

    /// Get the current attempt count.
    pub fn attempt_count(&self, opportunity_id: u64) -> StateMachineResult<u32> {
        let contexts = self.contexts.lock();
        Ok(contexts
            .get(&opportunity_id)
            .map(|ctx| ctx.attempt_count)
            .unwrap_or(0))
    }

    /// Record the blockhash slot and max slot for expiry tracking.
    pub fn record_blockhash(
        &self,
        opportunity_id: u64,
        slot: Slot,
        max_slot: Slot,
    ) -> StateMachineResult<()> {
        let mut contexts = self.contexts.lock();
        let ctx = contexts.get_mut(&opportunity_id).ok_or_else(|| {
            StateMachineError::Internal(format!("opportunity {opportunity_id} not registered"))
        })?;
        ctx.blockhash_slot = Some(slot);
        ctx.blockhash_max_slot = Some(max_slot);
        Ok(())
    }

    /// Check whether the blockhash has expired given the current slot.
    pub fn is_blockhash_expired(
        &self,
        opportunity_id: u64,
        current_slot: Slot,
    ) -> StateMachineResult<bool> {
        let contexts = self.contexts.lock();
        let ctx = contexts.get(&opportunity_id).ok_or_else(|| {
            StateMachineError::Internal(format!("opportunity {opportunity_id} not registered"))
        })?;
        match ctx.blockhash_max_slot {
            Some(max) if current_slot.0 > max.0 => Ok(true),
            _ => Ok(false),
        }
    }

    /// Get the backoff delay in milliseconds for the next retry attempt.
    pub fn next_backoff_ms(&self, opportunity_id: u64) -> StateMachineResult<u64> {
        let attempt = self.attempt_count(opportunity_id)?;
        Ok(backoff_delay_ms(attempt, &self.retry_config))
    }

    /// Get all recorded state transitions for an opportunity (not implemented
    /// as a persistent log here; callers should record transitions externally).
    pub fn context(&self, opportunity_id: u64) -> Option<ExecutionContext> {
        self.contexts.lock().get(&opportunity_id).cloned()
    }

    /// Number of tracked opportunities.
    pub fn len(&self) -> usize {
        self.contexts.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.contexts.lock().is_empty()
    }
}

/// Current timestamp in milliseconds since UNIX epoch.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_machine() -> ExecutionStateMachine {
        ExecutionStateMachine::new()
    }

    #[test]
    fn test_happy_path_transitions() {
        let machine = test_machine();
        machine.register(1).unwrap();

        machine
            .transition_to(1, ExecutionState::Quoted, None)
            .unwrap();
        machine
            .transition_to(1, ExecutionState::Validated, None)
            .unwrap();
        machine
            .transition_to(1, ExecutionState::Built, None)
            .unwrap();
        machine
            .transition_to(1, ExecutionState::Simulated, None)
            .unwrap();
        machine
            .transition_to(1, ExecutionState::Signed, None)
            .unwrap();
        machine
            .transition_to(1, ExecutionState::Submitted, None)
            .unwrap();
        machine
            .transition_to(1, ExecutionState::Pending, None)
            .unwrap();
        machine
            .transition_to(1, ExecutionState::Landed, None)
            .unwrap();
        machine
            .transition_to(1, ExecutionState::Confirmed, None)
            .unwrap();
        machine
            .transition_to(1, ExecutionState::Finalized, None)
            .unwrap();
        machine
            .transition_to(1, ExecutionState::Reconciled, None)
            .unwrap();

        assert_eq!(machine.state(1), Some(ExecutionState::Reconciled));
        assert!(machine.state(1).unwrap().is_success());
        assert!(machine.state(1).unwrap().is_terminal());
    }

    #[test]
    fn test_invalid_transition_rejected() {
        let machine = test_machine();
        machine.register(1).unwrap();

        // Cannot go from Discovered directly to Built.
        let err = machine.transition_to(1, ExecutionState::Built, None);
        assert!(err.is_err());
        assert!(matches!(
            err,
            Err(StateMachineError::InvalidTransition { .. })
        ));

        // State unchanged.
        assert_eq!(machine.state(1), Some(ExecutionState::Discovered));
    }

    #[test]
    fn test_terminal_state_no_transition() {
        let machine = test_machine();
        machine.register(1).unwrap();
        machine
            .transition_to(1, ExecutionState::Rejected, None)
            .unwrap();

        let err = machine.transition_to(1, ExecutionState::Discovered, None);
        assert!(err.is_err());
    }

    #[test]
    fn test_duplicate_registration() {
        let machine = test_machine();
        machine.register(1).unwrap();
        let err = machine.register(1);
        assert!(err.is_err());
    }

    #[test]
    fn test_idempotency_key() {
        let machine = test_machine();
        machine.register(1).unwrap();

        let key1 = IdempotencyKey::new(1, 0);
        machine.check_idempotency(1, &key1).unwrap();

        let err = machine.check_idempotency(1, &key1);
        assert!(err.is_err());
        assert!(matches!(
            err,
            Err(StateMachineError::IdempotencyCollision { .. })
        ));
    }

    #[test]
    fn test_duplicate_execution_prevention() {
        let machine = test_machine();
        machine.register(1).unwrap();
        machine.mark_executed(1).unwrap();

        let err = machine.mark_executed(1);
        assert!(err.is_err());
        assert!(matches!(
            err,
            Err(StateMachineError::DuplicateExecution { .. })
        ));
    }

    #[test]
    fn test_retry_budget_exhausted() {
        let machine = ExecutionStateMachine::with_retry_config(RetryConfig {
            max_attempts: 2,
            ..Default::default()
        });
        machine.register(1).unwrap();

        machine.increment_attempt(1).unwrap(); // attempt 1
        machine.increment_attempt(1).unwrap(); // attempt 2
        let err = machine.increment_attempt(1); // attempt 3 > max 2
        assert!(err.is_err());
        assert!(matches!(
            err,
            Err(StateMachineError::RetryBudgetExhausted { .. })
        ));
    }

    #[test]
    fn test_backoff_delay() {
        let config = RetryConfig {
            base_delay_ms: 100,
            max_delay_ms: 10_000,
            jitter_factor: 0.0,
            ..Default::default()
        };
        assert_eq!(backoff_delay_ms(0, &config), 100);
        assert_eq!(backoff_delay_ms(1, &config), 200);
        assert_eq!(backoff_delay_ms(2, &config), 400);
        assert_eq!(backoff_delay_ms(10, &config), 10_000); // capped
    }

    #[test]
    fn test_circuit_breaker() {
        let machine = test_machine();
        assert!(!machine.is_circuit_breaker_open());
        machine.set_circuit_breaker_open(true);
        assert!(machine.is_circuit_breaker_open());
        machine.set_circuit_breaker_open(false);
        assert!(!machine.is_circuit_breaker_open());
    }

    #[test]
    fn test_blockhash_expiry() {
        let machine = test_machine();
        machine.register(1).unwrap();
        machine.record_blockhash(1, Slot(100), Slot(150)).unwrap();

        assert!(!machine.is_blockhash_expired(1, Slot(149)).unwrap());
        assert!(machine.is_blockhash_expired(1, Slot(151)).unwrap());
    }

    #[test]
    fn test_state_labels() {
        assert_eq!(ExecutionState::Discovered.label(), "discovered");
        assert_eq!(ExecutionState::Reconciled.label(), "reconciled");
        assert_eq!(ExecutionState::Failed.label(), "failed");
        assert!(!ExecutionState::Discovered.is_terminal());
        assert!(ExecutionState::Failed.is_terminal());
        assert!(ExecutionState::Reconciled.is_success());
        assert!(!ExecutionState::Pending.is_success());
    }
}
