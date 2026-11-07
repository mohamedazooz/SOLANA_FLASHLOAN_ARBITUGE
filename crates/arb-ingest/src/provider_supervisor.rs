//! Deterministic provider health/failover policy primitives.
//! Network stream ownership remains in the caller; provider-local slot/version values
//! are never compared across providers.
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProviderId(pub u16);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderStatus { Healthy, Degraded, OpenCircuit, Disabled }

#[derive(Debug, Clone)]
pub struct ProviderHealth {
    pub id: ProviderId,
    pub priority: u16,
    pub status: ProviderStatus,
    pub consecutive_failures: u32,
    pub last_success: Option<Instant>,
    pub last_failure: Option<Instant>,
    pub observed_slot: Option<u64>,
    pub cooldown_until: Option<Instant>,
}

impl ProviderHealth {
    pub fn new(id: ProviderId, priority: u16) -> Self {
        Self { id, priority, status: ProviderStatus::Healthy, consecutive_failures: 0,
            last_success: None, last_failure: None, observed_slot: None, cooldown_until: None }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SupervisorPolicy {
    pub failure_threshold: u32,
    pub cooldown: Duration,
    pub max_slot_lag: u64,
    pub stale_after: Duration,
}

impl Default for SupervisorPolicy {
    fn default() -> Self {
        Self { failure_threshold: 3, cooldown: Duration::from_secs(2), max_slot_lag: 8,
            stale_after: Duration::from_secs(3) }
    }
}

#[derive(Debug, Clone)]
pub struct ProviderSupervisor { policy: SupervisorPolicy, providers: Vec<ProviderHealth> }

impl ProviderSupervisor {
    pub fn new(mut providers: Vec<ProviderHealth>, policy: SupervisorPolicy) -> Self {
        providers.sort_by_key(|p| p.priority);
        Self { policy, providers }
    }

    pub fn providers(&self) -> &[ProviderHealth] { &self.providers }

    pub fn record_success(&mut self, id: ProviderId, slot: u64, now: Instant) -> bool {
        let Some(p) = self.providers.iter_mut().find(|p| p.id == id) else { return false; };
        p.consecutive_failures = 0;
        p.last_success = Some(now);
        p.observed_slot = Some(slot);
        p.cooldown_until = None;
        p.status = ProviderStatus::Healthy;
        true
    }

    pub fn record_failure(&mut self, id: ProviderId, now: Instant) -> bool {
        let Some(p) = self.providers.iter_mut().find(|p| p.id == id) else { return false; };
        p.consecutive_failures = p.consecutive_failures.saturating_add(1);
        p.last_failure = Some(now);
        p.status = if p.consecutive_failures >= self.policy.failure_threshold {
            p.cooldown_until = Some(now + self.policy.cooldown);
            ProviderStatus::OpenCircuit
        } else { ProviderStatus::Degraded };
        true
    }

    /// Select the highest-priority provider that is healthy, fresh and close to the
    /// observed cluster slot. `cluster_slot` must come from an independent slot source.
    pub fn select(&mut self, cluster_slot: u64, now: Instant) -> Option<ProviderId> {
        for p in &mut self.providers {
            if p.status == ProviderStatus::Disabled { continue; }
            if p.status == ProviderStatus::OpenCircuit {
                if p.cooldown_until.is_some_and(|until| now < until) { continue; }
                // Half-open probe: only a successful stream observation restores health.
                p.status = ProviderStatus::Degraded;
            }
            let fresh = p.last_success.is_some_and(|t| now.saturating_duration_since(t) <= self.policy.stale_after);
            let in_lag = p.observed_slot.is_some_and(|s| cluster_slot.saturating_sub(s) <= self.policy.max_slot_lag && s <= cluster_slot);
            if p.status == ProviderStatus::Healthy && fresh && in_lag { return Some(p.id); }
        }
        None
    }

    pub fn disable(&mut self, id: ProviderId) -> bool {
        let Some(p) = self.providers.iter_mut().find(|p| p.id == id) else { return false; };
        p.status = ProviderStatus::Disabled;
        true
    }
}

/// Provenance attached to a dependency snapshot before it can enter a quote context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DependencyStamp {
    pub provider: ProviderId,
    pub slot: u64,
    pub bank_id: Option<u64>,
    pub write_version: u64,
    pub is_startup: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoherenceError { Empty, MixedProviders, MixedBanks, FutureSlot, SlotSkew, StartupSnapshot }

/// Validate a dependency group. Use max_slot_skew=0 for tightly-coupled pool/vault
/// state; wider skew is only appropriate for slowly-changing configuration accounts.
pub fn validate_dependency_group(stamps: &[DependencyStamp], target_slot: u64, max_slot_skew: u64)
    -> Result<(), CoherenceError>
{
    let Some(first) = stamps.first() else { return Err(CoherenceError::Empty); };
    if stamps.iter().any(|s| s.is_startup) { return Err(CoherenceError::StartupSnapshot); }
    if stamps.iter().any(|s| s.provider != first.provider) { return Err(CoherenceError::MixedProviders); }
    let mut min_slot = u64::MAX;
    let mut max_slot = 0u64;
    let mut bank_by_slot = std::collections::HashMap::with_capacity(stamps.len());
    for s in stamps {
        if s.slot > target_slot { return Err(CoherenceError::FutureSlot); }
        if bank_by_slot.insert(s.slot, s.bank_id).is_some_and(|bank| bank != s.bank_id) {
            return Err(CoherenceError::MixedBanks);
        }
        min_slot = min_slot.min(s.slot);
        max_slot = max_slot.max(s.slot);
    }
    if max_slot.saturating_sub(min_slot) > max_slot_skew { return Err(CoherenceError::SlotSkew); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn now() -> Instant { Instant::now() }
    #[test]
    fn failover_skips_open_circuit_and_recovers_after_success() {
        let t = now();
        let policy = SupervisorPolicy { failure_threshold: 2, cooldown: Duration::from_secs(5), max_slot_lag: 8, stale_after: Duration::from_secs(3) };
        let mut s = ProviderSupervisor::new(vec![ProviderHealth::new(ProviderId(1), 0), ProviderHealth::new(ProviderId(2), 1)], policy);
        s.record_success(ProviderId(1), 100, t); s.record_success(ProviderId(2), 100, t);
        s.record_failure(ProviderId(1), t); s.record_failure(ProviderId(1), t);
        assert_eq!(s.select(100, t), Some(ProviderId(2)));
        s.record_success(ProviderId(1), 101, t + Duration::from_secs(6));
        assert_eq!(s.select(101, t + Duration::from_secs(6)), Some(ProviderId(1)));
    }
    #[test]
    fn stale_or_lagging_provider_is_not_selected() {
        let t = now();
        let mut s = ProviderSupervisor::new(vec![ProviderHealth::new(ProviderId(1), 0)], SupervisorPolicy::default());
        s.record_success(ProviderId(1), 80, t);
        assert_eq!(s.select(100, t), None);
        assert_eq!(s.select(80, t + Duration::from_secs(4)), None);
    }
    #[test]
    fn dependency_group_checks_provenance_and_slot_skew() {
        let a = DependencyStamp { provider: ProviderId(1), slot: 100, bank_id: Some(7), write_version: 2, is_startup: false };
        let b = DependencyStamp { slot: 100, ..a };
        assert_eq!(validate_dependency_group(&[a,b], 100, 0), Ok(()));
        let c = DependencyStamp { slot: 99, ..a };
        assert_eq!(validate_dependency_group(&[a,c], 100, 0), Err(CoherenceError::SlotSkew));
        let d = DependencyStamp { provider: ProviderId(2), ..a };
        assert_eq!(validate_dependency_group(&[a,d], 100, 1), Err(CoherenceError::MixedProviders));
        let next_slot = DependencyStamp { slot: 99, bank_id: Some(6), ..a };
        assert_eq!(validate_dependency_group(&[a,next_slot], 100, 1), Ok(()));
    }
}
