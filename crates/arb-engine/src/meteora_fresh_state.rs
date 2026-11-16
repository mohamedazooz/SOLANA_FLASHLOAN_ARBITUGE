//! Fresh-state quote validation for Meteora DLMM two-pool arbitrage.
//!
//! Ensures quotes are not stale before simulation by comparing the quote
//! snapshot slot with the actual simulation context slot returned by RPC.

#[derive(Debug, Clone, Copy)]
pub struct FreshnessConfig {
    pub max_quote_age_slots: u64,
    pub max_slot_lag: u64,
    pub max_retries: u32,
}

impl Default for FreshnessConfig {
    fn default() -> Self {
        Self {
            max_quote_age_slots: 2,
            max_slot_lag: 2,
            max_retries: 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreshnessVerdict {
    Fresh,
    QuoteTooOld { quote_slot: u64, current_slot: u64 },
    SimulationLagTooHigh { quote_slot: u64, sim_slot: u64 },
    SlotRegression { quote_slot: u64, sim_slot: u64 },
}

impl FreshnessVerdict {
    pub fn is_fresh(self) -> bool {
        matches!(self, Self::Fresh)
    }
}

pub fn evaluate_freshness(
    quote_slot: u64,
    current_slot: u64,
    sim_slot: Option<u64>,
    cfg: &FreshnessConfig,
) -> FreshnessVerdict {
    if current_slot.saturating_sub(quote_slot) > cfg.max_quote_age_slots {
        return FreshnessVerdict::QuoteTooOld {
            quote_slot,
            current_slot,
        };
    }
    if let Some(s) = sim_slot {
        if s < quote_slot {
            return FreshnessVerdict::SlotRegression {
                quote_slot,
                sim_slot: s,
            };
        }
        if s.saturating_sub(quote_slot) > cfg.max_slot_lag {
            return FreshnessVerdict::SimulationLagTooHigh {
                quote_slot,
                sim_slot: s,
            };
        }
    }
    FreshnessVerdict::Fresh
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_when_within_lag() {
        let cfg = FreshnessConfig::default();
        assert!(evaluate_freshness(100, 101, Some(101), &cfg).is_fresh());
    }

    #[test]
    fn rejects_stale_quote() {
        let cfg = FreshnessConfig::default();
        assert!(!evaluate_freshness(100, 105, Some(105), &cfg).is_fresh());
    }

    #[test]
    fn rejects_high_sim_lag() {
        let cfg = FreshnessConfig::default();
        assert!(!evaluate_freshness(100, 101, Some(120), &cfg).is_fresh());
    }
}
