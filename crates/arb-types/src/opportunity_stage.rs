//! Typed opportunity qualification stages.
//!
//! Canonical taxonomy for the opportunity pipeline. A stage is a claim about
//! what has been *proven* about a candidate route; it never upgrades itself.
//! In particular, `EconomicallyPositive` (a positive integer net-profit quote)
//! is never conflated with `MainnetVerified` (a confirmed on-chain settlement
//! with realized balance deltas). Derivations in this module are conservative:
//! an ambiguous record maps to the LOWEST stage its fields prove.
//!
//! Stages, in proof order:
//!
//! `DISCOVERED` → `QUOTE_VALID` → `ECONOMICALLY_POSITIVE` → `EXECUTABLE` →
//! `SIMULATED` → `SHADOW_VALIDATED` → `MAINNET_VERIFIED`

use serde::{Deserialize, Serialize};

/// All stages in ascending proof order.
pub const OPPORTUNITY_STAGES: [OpportunityStage; 7] = [
    OpportunityStage::Discovered,
    OpportunityStage::QuoteValid,
    OpportunityStage::EconomicallyPositive,
    OpportunityStage::Executable,
    OpportunityStage::Simulated,
    OpportunityStage::ShadowValidated,
    OpportunityStage::MainnetVerified,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum OpportunityStage {
    /// A directed cycle was found over existing pools. No quote yet.
    #[serde(rename = "DISCOVERED")]
    Discovered,
    /// Every hop priced by the canonical dispatcher with complete, fresh state.
    #[serde(rename = "QUOTE_VALID")]
    QuoteValid,
    /// Integer net profit is strictly positive after ALL known costs.
    #[serde(rename = "ECONOMICALLY_POSITIVE")]
    EconomicallyPositive,
    /// The transaction builder assembled the exact route (accounts, mints,
    /// directions, min-out) from the same quote inputs.
    #[serde(rename = "EXECUTABLE")]
    Executable,
    /// `simulateTransaction` (or an equivalent preflight) succeeded.
    /// Failure at this point keeps the stage but records a rejection reason.
    #[serde(rename = "SIMULATED")]
    Simulated,
    /// Shadow-mode validation: same-slot state continuity and deterministic
    /// quote reproduction confirmed without broadcasting.
    #[serde(rename = "SHADOW_VALIDATED")]
    ShadowValidated,
    /// A real transaction landed and realized balance deltas were measured.
    /// NEVER derived automatically; only set by explicit operator-verified
    /// reconciliation evidence.
    #[serde(rename = "MAINNET_VERIFIED")]
    MainnetVerified,
}

impl OpportunityStage {
    pub fn label(&self) -> &'static str {
        match self {
            OpportunityStage::Discovered => "DISCOVERED",
            OpportunityStage::QuoteValid => "QUOTE_VALID",
            OpportunityStage::EconomicallyPositive => "ECONOMICALLY_POSITIVE",
            OpportunityStage::Executable => "EXECUTABLE",
            OpportunityStage::Simulated => "SIMULATED",
            OpportunityStage::ShadowValidated => "SHADOW_VALIDATED",
            OpportunityStage::MainnetVerified => "MAINNET_VERIFIED",
        }
    }

    /// Next stage in proof order, or `None` past `MainnetVerified`.
    pub fn advance(&self) -> Option<OpportunityStage> {
        match self {
            OpportunityStage::Discovered => Some(OpportunityStage::QuoteValid),
            OpportunityStage::QuoteValid => Some(OpportunityStage::EconomicallyPositive),
            OpportunityStage::EconomicallyPositive => Some(OpportunityStage::Executable),
            OpportunityStage::Executable => Some(OpportunityStage::Simulated),
            OpportunityStage::Simulated => Some(OpportunityStage::ShadowValidated),
            OpportunityStage::ShadowValidated => Some(OpportunityStage::MainnetVerified),
            OpportunityStage::MainnetVerified => None,
        }
    }

    /// Parse a canonical stage label (`DISCOVERED`, `QUOTE_VALID`, ...).
    pub fn from_label(label: &str) -> Option<OpportunityStage> {
        OPPORTUNITY_STAGES
            .iter()
            .copied()
            .find(|s| s.label() == label)
    }

    /// Map a legacy free-form opportunity `status` string to the lowest stage
    /// its semantics prove. Unknown strings return `None` (callers must not
    /// guess a stage for an unrecognized status).
    ///
    /// Legacy statuses are produced by the shadow pipeline
    /// (`crates/arb-sim/src/meteora_fresh_pipeline.rs`) and the paper report.
    pub fn from_legacy_status(status: &str) -> Option<OpportunityStage> {
        match status {
            // Simulation ran and succeeded on fresh state with positive net
            // profit. Conservative: this is SIMULATED, not SHADOW_VALIDATED —
            // the legacy status alone does not prove the continuity checks.
            "VERIFIED_PROFITABLE" => Some(OpportunityStage::Simulated),
            // Simulation ran (the attempt reached the simulated stage) but
            // produced a negative edge; the record's rejection reason carries
            // the detail.
            "SIMULATED_SUCCESS_NEGATIVE_EDGE" => Some(OpportunityStage::Simulated),
            // Simulation was attempted and failed; stage reached is SIMULATED.
            "REJECTED_SIM_FAIL" => Some(OpportunityStage::Simulated),
            // Quotes were produced but rejected for staleness before/at
            // simulation; the proved stage is QUOTE_VALID.
            "STALE_QUOTE" => Some(OpportunityStage::QuoteValid),
            // Rejected before any quote was produced.
            "REJECTED_NO_QUOTE" => Some(OpportunityStage::Discovered),
            _ => None,
        }
    }
}

impl std::fmt::Display for OpportunityStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

#[cfg(test)]
mod opportunity_stage_tests {
    use super::*;

    #[test]
    fn stages_are_strictly_ordered_by_proof_level() {
        for pair in OPPORTUNITY_STAGES.windows(2) {
            assert!(pair[0] < pair[1]);
        }
        assert_eq!(OPPORTUNITY_STAGES.len(), 7);
    }

    #[test]
    fn labels_round_trip() {
        for stage in OPPORTUNITY_STAGES {
            assert_eq!(OpportunityStage::from_label(stage.label()), Some(stage));
            assert_eq!(stage.to_string(), stage.label());
        }
        assert_eq!(OpportunityStage::from_label("UNKNOWN"), None);
    }

    #[test]
    fn advance_walks_the_full_ladder() {
        let mut stages = vec![OpportunityStage::Discovered];
        while let Some(next) = stages.last().copied().and_then(|s| s.advance()) {
            stages.push(next);
        }
        assert_eq!(stages.as_slice(), OPPORTUNITY_STAGES);
    }

    #[test]
    fn legacy_status_mapping_is_conservative() {
        assert_eq!(
            OpportunityStage::from_legacy_status("VERIFIED_PROFITABLE"),
            Some(OpportunityStage::Simulated)
        );
        assert_eq!(
            OpportunityStage::from_legacy_status("SIMULATED_SUCCESS_NEGATIVE_EDGE"),
            Some(OpportunityStage::Simulated)
        );
        assert_eq!(
            OpportunityStage::from_legacy_status("REJECTED_SIM_FAIL"),
            Some(OpportunityStage::Simulated)
        );
        assert_eq!(
            OpportunityStage::from_legacy_status("STALE_QUOTE"),
            Some(OpportunityStage::QuoteValid)
        );
        assert_eq!(
            OpportunityStage::from_legacy_status("REJECTED_NO_QUOTE"),
            Some(OpportunityStage::Discovered)
        );
        assert_eq!(OpportunityStage::from_legacy_status("garbage"), None);
        assert_eq!(OpportunityStage::from_legacy_status(""), None);
    }

    #[test]
    fn mainnet_verified_is_never_auto_advanced_into() {
        // Only ShadowValidated advances to MainnetVerified in the ladder;
        // no from_legacy_status string may produce it.
        for status in [
            "VERIFIED_PROFITABLE",
            "SIMULATED_SUCCESS_NEGATIVE_EDGE",
            "REJECTED_SIM_FAIL",
            "STALE_QUOTE",
            "REJECTED_NO_QUOTE",
        ] {
            assert_ne!(
                OpportunityStage::from_legacy_status(status),
                Some(OpportunityStage::MainnetVerified)
            );
        }
    }
}
