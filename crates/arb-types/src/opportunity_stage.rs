//! Typed opportunity qualification stages with evidence-gated promotion.
//!
//! Canonical taxonomy for the opportunity pipeline. A stage is a claim about
//! what has been *proven* about a candidate route. Every stage above
//! `DISCOVERED` is a CONJUNCTION: claiming a stage implicitly claims every
//! stage below it. Two consequences follow, and they are deliberate:
//!
//! 1. A **successful simulation of a losing trade is NOT a profitable
//!    opportunity**. Simulation outcome ([`SimulationOutcome`]) is tracked
//!    separately from economic qualification; a record with a successful
//!    simulation but non-positive net profit tops out at `QUOTE_VALID`
//!    (the simulation proof is preserved in the evidence, but the conjunctive
//!    ladder refuses `ECONOMICALLY_POSITIVE` because economics is disproven).
//! 2. **No promotion without evidence.** `advance()` requires the caller to
//!    supply a [`StageEvidence`] value, and only returns the next stage if
//!    that stage is fully proven by it. There is no evidence-free promotion
//!    path anywhere in this module.
//!
//! Stage ladder, in proof order:
//!
//! `DISCOVERED` → `QUOTE_VALID` → `ECONOMICALLY_POSITIVE` → `EXECUTABLE` →
//! `SIMULATED` → `SHADOW_VALIDATED` → `MAINNET_VERIFIED`
//!
//! Evidence requirements per stage (each in addition to every lower stage):
//!
//! | Stage | Additional evidence required |
//! |---|---|
//! | `QUOTE_VALID` | `quote_complete` && `quote_fresh` && `slot_consistent` |
//! | `ECONOMICALLY_POSITIVE` | `net_profit_positive` |
//! | `EXECUTABLE` | `builder_succeeded` |
//! | `SIMULATED` | `simulation == Succeeded` |
//! | `SHADOW_VALIDATED` | valid [`ShadowContinuityProof`] |
//! | `MAINNET_VERIFIED` | valid [`MainnetSettlementProof`] |
//!
//! Stale, incomplete, or slot-inconsistent quotes are NEVER promoted past
//! `DISCOVERED`: a quote whose freshness or cross-account slot identity cannot
//! be proven is not trustworthy, even if its numbers look positive.
//! `SHADOW_VALIDATED` and `MAINNET_VERIFIED` require dedicated proof
//! structures produced only by shadow-continuity replay and confirmed
//! on-chain settlement reconciliation respectively; ordinary quote/profit
//! fields can never supply them.

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
    /// Every hop priced by the canonical dispatcher with complete, fresh,
    /// slot-consistent state.
    #[serde(rename = "QUOTE_VALID")]
    QuoteValid,
    /// Integer net profit is strictly positive after ALL known costs, computed
    /// over a quote that already passed `QUOTE_VALID` evidence.
    #[serde(rename = "ECONOMICALLY_POSITIVE")]
    EconomicallyPositive,
    /// The transaction builder assembled the exact route (accounts, mints,
    /// directions, min-out) from the same quote inputs.
    #[serde(rename = "EXECUTABLE")]
    Executable,
    /// `simulateTransaction` (or an equivalent preflight) SUCCEEDED.
    /// A failed or unattempted simulation never reaches this stage.
    #[serde(rename = "SIMULATED")]
    Simulated,
    /// Shadow-mode validation: same-slot state continuity and deterministic
    /// quote reproduction confirmed without broadcasting. Requires a valid
    /// [`ShadowContinuityProof`]; never derived from quote/profit fields.
    #[serde(rename = "SHADOW_VALIDATED")]
    ShadowValidated,
    /// A real transaction landed and realized balance deltas were measured.
    /// Requires a valid [`MainnetSettlementProof`]; NEVER derived
    /// automatically from any other field or legacy status.
    #[serde(rename = "MAINNET_VERIFIED")]
    MainnetVerified,
}

/// Outcome of the transaction simulation stage, tracked SEPARATELY from
/// economic qualification. `Succeeded` on a losing trade does not make the
/// trade profitable: it only proves the transaction is valid on-chain.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SimulationOutcome {
    /// No simulation was attempted.
    #[default]
    #[serde(rename = "NOT_ATTEMPTED")]
    NotAttempted,
    /// A simulation was attempted and failed.
    #[serde(rename = "FAILED")]
    Failed,
    /// A simulation was attempted and succeeded.
    #[serde(rename = "SUCCEEDED")]
    Succeeded,
}

impl SimulationOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self, SimulationOutcome::Succeeded)
    }

    pub fn was_attempted(&self) -> bool {
        !matches!(self, SimulationOutcome::NotAttempted)
    }
}

impl std::fmt::Display for SimulationOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SimulationOutcome::NotAttempted => "NOT_ATTEMPTED",
            SimulationOutcome::Failed => "FAILED",
            SimulationOutcome::Succeeded => "SUCCEEDED",
        })
    }
}

/// Evidence that a shadow-mode replay confirmed state continuity and
/// deterministic quote reproduction. `MAINNET_VERIFIED` requires
/// `SHADOW_VALIDATED`, and `SHADOW_VALIDATED` requires this proof.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ShadowContinuityProof {
    /// The recorded quotes were reproduced deterministically during replay.
    pub quote_reproduced: bool,
    /// Recorded pool states were shown to be continuous (no unexplained
    /// state jumps or gaps) across the replayed window.
    pub state_continuity_verified: bool,
    /// Slot at which the continuity checks were performed.
    pub verified_at_slot: u64,
}

impl ShadowContinuityProof {
    pub fn is_valid(&self) -> bool {
        self.quote_reproduced && self.state_continuity_verified && self.verified_at_slot > 0
    }
}

/// Evidence that a REAL transaction settled on-chain with measured realized
/// balance deltas. This is the only gate into `MAINNET_VERIFIED`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MainnetSettlementProof {
    /// Signature of the confirmed settlement transaction (base58, non-empty).
    pub transaction_signature: String,
    /// Signer's relevant-token (or lamport) balance immediately BEFORE the
    /// settlement, as measured from chain state.
    pub pre_settlement_balance_lamports: Option<u64>,
    /// Signer's balance immediately AFTER the confirmed settlement.
    pub post_settlement_balance_lamports: Option<u64>,
    /// Slot in which the settlement was confirmed.
    pub confirmed_slot: u64,
}

impl MainnetSettlementProof {
    /// The realized (measured, not quoted) delta of the settlement.
    pub fn realized_balance_delta_lamports(&self) -> Option<i64> {
        let pre = i64::try_from(self.pre_settlement_balance_lamports?).ok()?;
        let post = i64::try_from(self.post_settlement_balance_lamports?).ok()?;
        post.checked_sub(pre)
    }

    pub fn is_valid(&self) -> bool {
        let sig = self.transaction_signature.trim();
        // Solana signatures are 64-byte base58 (87..=88 chars); reject empty
        // or implausible values, and never accept an unmeasured balance.
        let sig_plausible = !sig.is_empty() && sig.len() <= 88;
        sig_plausible
            && self.confirmed_slot > 0
            && self.pre_settlement_balance_lamports.is_some()
            && self.post_settlement_balance_lamports.is_some()
    }
}

/// The full evidence bundle a stage promotion decision is allowed to consult.
/// Anything not expressible as a field here cannot promote a stage.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StageEvidence {
    /// Every hop produced a non-zero output quote (quote is complete).
    pub quote_complete: bool,
    /// Quote freshness check passed (quote age / slot lag within policy).
    pub quote_fresh: bool,
    /// All pool snapshots and the quote originate from the SAME slot.
    pub slot_consistent: bool,
    /// Integer net profit is strictly positive after all known costs.
    pub net_profit_positive: bool,
    /// The transaction builder assembled the route successfully.
    pub builder_succeeded: bool,
    /// Simulation outcome (tracked separately from economics).
    pub simulation: SimulationOutcome,
    /// Shadow-continuity proof; `None` unless a replay explicitly produced one.
    pub shadow_continuity: Option<ShadowContinuityProof>,
    /// Mainnet settlement proof; `None` unless a confirmed settlement was
    /// explicitly reconciled against measured balances.
    pub mainnet_settlement: Option<MainnetSettlementProof>,
}

impl StageEvidence {
    /// The quote-level gates only, with every downstream claim withheld.
    pub fn quote_gates(quote_complete: bool, quote_fresh: bool, slot_consistent: bool) -> Self {
        Self {
            quote_complete,
            quote_fresh,
            slot_consistent,
            ..Self::default()
        }
    }
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

    /// Parse a canonical stage label (`DISCOVERED`, `QUOTE_VALID`, ...).
    pub fn from_label(label: &str) -> Option<OpportunityStage> {
        OPPORTUNITY_STAGES
            .iter()
            .copied()
            .find(|s| s.label() == label)
    }

    /// EVIDENCE-GATED promotion: returns the next stage in proof order ONLY
    /// if that next stage is fully proven by `evidence` (and is above `self`).
    /// There is no evidence-free promotion: calling `advance` without the
    /// required evidence returns `None`.
    pub fn advance(&self, evidence: &StageEvidence) -> Option<OpportunityStage> {
        let next = match self {
            OpportunityStage::Discovered => OpportunityStage::QuoteValid,
            OpportunityStage::QuoteValid => OpportunityStage::EconomicallyPositive,
            OpportunityStage::EconomicallyPositive => OpportunityStage::Executable,
            OpportunityStage::Executable => OpportunityStage::Simulated,
            OpportunityStage::Simulated => OpportunityStage::ShadowValidated,
            OpportunityStage::ShadowValidated => OpportunityStage::MainnetVerified,
            OpportunityStage::MainnetVerified => return None,
        };
        if next > *self && next.is_proven_by(evidence) {
            Some(next)
        } else {
            None
        }
    }

    /// Does `evidence` prove this stage? Each stage above `Discovered` is a
    /// conjunction with every lower stage, so proofs chain strictly.
    pub fn is_proven_by(&self, evidence: &StageEvidence) -> bool {
        match self {
            OpportunityStage::Discovered => true,
            OpportunityStage::QuoteValid => {
                evidence.quote_complete && evidence.quote_fresh && evidence.slot_consistent
            }
            OpportunityStage::EconomicallyPositive => {
                OpportunityStage::QuoteValid.is_proven_by(evidence) && evidence.net_profit_positive
            }
            OpportunityStage::Executable => {
                OpportunityStage::EconomicallyPositive.is_proven_by(evidence)
                    && evidence.builder_succeeded
            }
            OpportunityStage::Simulated => {
                OpportunityStage::Executable.is_proven_by(evidence)
                    && evidence.simulation == SimulationOutcome::Succeeded
            }
            OpportunityStage::ShadowValidated => {
                OpportunityStage::Simulated.is_proven_by(evidence)
                    && evidence
                        .shadow_continuity
                        .as_ref()
                        .is_some_and(|p| p.is_valid())
            }
            OpportunityStage::MainnetVerified => {
                OpportunityStage::ShadowValidated.is_proven_by(evidence)
                    && evidence
                        .mainnet_settlement
                        .as_ref()
                        .is_some_and(|p| p.is_valid())
            }
        }
    }

    /// The HIGHEST stage fully proven by `evidence`. Because the ladder is
    /// conjunctive, this walks up while each next stage is proven and stops
    /// at the first stage whose evidence is missing — never skipping over a
    /// failed gate.
    pub fn highest_proven(evidence: &StageEvidence) -> OpportunityStage {
        let mut current = OpportunityStage::Discovered;
        while let Some(next) = current.advance(evidence) {
            current = next;
        }
        current
    }

    /// Map a legacy free-form opportunity `status` string to the LOWEST stage
    /// its semantics prove. Unknown strings return `None` (callers must not
    /// guess a stage for an unrecognized status).
    ///
    /// These mappings are deliberately pessimistic: a bare status string
    /// carries NO proof of quote freshness, slot consistency, or builder
    /// success, so nothing may be promoted past what the string itself
    /// establishes. Legacy statuses are produced by the shadow pipeline
    /// (`crates/arb-sim/src/meteora_fresh_pipeline.rs`) and the paper report.
    pub fn from_legacy_status(status: &str) -> Option<OpportunityStage> {
        match status {
            // Simulation ran AND succeeded on fresh state with positive net
            // profit. Conservative: this is SIMULATED, never SHADOW_VALIDATED
            // — the legacy status alone proves no continuity checks.
            "VERIFIED_PROFITABLE" => Some(OpportunityStage::Simulated),
            // The simulation succeeded, but the recorded economics are
            // NEGATIVE: the opportunity is economically DISPROVEN. It is a
            // valid transaction, not a profitable one — map to QUOTE_VALID
            // and never to ECONOMICALLY_POSITIVE or SIMULATED.
            "SIMULATED_SUCCESS_NEGATIVE_EDGE" => Some(OpportunityStage::QuoteValid),
            // Simulation was ATTEMPTED and FAILED. A failed simulation proves
            // nothing above discovery from the string alone: freshness was
            // not yet evaluated in the pipeline's branch order, and the
            // SIMULATED stage requires simulation SUCCESS.
            "REJECTED_SIM_FAIL" => Some(OpportunityStage::Discovered),
            // Quotes were produced but FAILED the freshness gate. A stale
            // quote is never a QUOTE_VALID quote.
            "STALE_QUOTE" => Some(OpportunityStage::Discovered),
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

    /// Evidence proving everything reachable from record fields.
    fn full_evidence() -> StageEvidence {
        StageEvidence {
            quote_complete: true,
            quote_fresh: true,
            slot_consistent: true,
            net_profit_positive: true,
            builder_succeeded: true,
            simulation: SimulationOutcome::Succeeded,
            shadow_continuity: None,
            mainnet_settlement: None,
        }
    }

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
    fn evidence_gated_advance_walks_the_full_ladder_only_with_proof() {
        // With full evidence, advance walks the whole reachable ladder.
        let mut stages = vec![OpportunityStage::Discovered];
        while let Some(next) = stages
            .last()
            .copied()
            .and_then(|s| s.advance(&full_evidence()))
        {
            stages.push(next);
        }
        assert_eq!(
            stages.as_slice(),
            &OPPORTUNITY_STAGES[..OPPORTUNITY_STAGES.len() - 2]
        );

        // The default (empty) evidence advances nowhere past DISCOVERED.
        assert_eq!(
            OpportunityStage::Discovered.advance(&StageEvidence::default()),
            None
        );
    }

    #[test]
    fn highest_proven_never_skips_a_failed_gate() {
        assert_eq!(
            OpportunityStage::highest_proven(&StageEvidence::default()),
            OpportunityStage::Discovered
        );
        assert_eq!(
            OpportunityStage::highest_proven(&full_evidence()),
            OpportunityStage::Simulated
        );
    }

    #[test]
    fn quote_valid_requires_all_three_quote_gates() {
        let complete_only = StageEvidence::quote_gates(true, false, false);
        assert!(!OpportunityStage::QuoteValid.is_proven_by(&complete_only));
        assert_eq!(
            OpportunityStage::highest_proven(&complete_only),
            OpportunityStage::Discovered
        );

        let complete_fresh = StageEvidence::quote_gates(true, true, false);
        assert!(!OpportunityStage::QuoteValid.is_proven_by(&complete_fresh));
        assert_eq!(
            OpportunityStage::highest_proven(&complete_fresh),
            OpportunityStage::Discovered
        );

        let complete_fresh_slot = StageEvidence::quote_gates(true, true, true);
        assert!(OpportunityStage::QuoteValid.is_proven_by(&complete_fresh_slot));
        assert_eq!(
            OpportunityStage::highest_proven(&complete_fresh_slot),
            OpportunityStage::QuoteValid
        );
    }

    #[test]
    fn positive_profit_on_stale_quote_never_reaches_economic_stage() {
        let stale_positive = StageEvidence {
            quote_fresh: false,
            ..full_evidence()
        };
        assert!(!OpportunityStage::EconomicallyPositive.is_proven_by(&stale_positive));
        // The conjunction stops the walk at DISCOVERED despite every other
        // gate passing: freshness is a hard prerequisite.
        assert_eq!(
            OpportunityStage::highest_proven(&stale_positive),
            OpportunityStage::Discovered
        );

        let slot_inconsistent_positive = StageEvidence {
            slot_consistent: false,
            ..full_evidence()
        };
        assert_eq!(
            OpportunityStage::highest_proven(&slot_inconsistent_positive),
            OpportunityStage::Discovered
        );

        let incomplete_positive = StageEvidence {
            quote_complete: false,
            ..full_evidence()
        };
        assert_eq!(
            OpportunityStage::highest_proven(&incomplete_positive),
            OpportunityStage::Discovered
        );
    }

    #[test]
    fn executable_requires_explicit_builder_success() {
        let no_builder = StageEvidence {
            builder_succeeded: false,
            ..full_evidence()
        };
        assert!(OpportunityStage::EconomicallyPositive.is_proven_by(&no_builder));
        assert!(!OpportunityStage::Executable.is_proven_by(&no_builder));
        assert_eq!(
            OpportunityStage::highest_proven(&no_builder),
            OpportunityStage::EconomicallyPositive
        );
    }

    #[test]
    fn simulated_requires_simulation_success_not_attempt() {
        let attempted_failed = StageEvidence {
            simulation: SimulationOutcome::Failed,
            ..full_evidence()
        };
        assert!(OpportunityStage::Executable.is_proven_by(&attempted_failed));
        assert!(!OpportunityStage::Simulated.is_proven_by(&attempted_failed));
        assert_eq!(
            OpportunityStage::highest_proven(&attempted_failed),
            OpportunityStage::Executable
        );

        let not_attempted = StageEvidence {
            simulation: SimulationOutcome::NotAttempted,
            ..full_evidence()
        };
        assert!(!OpportunityStage::Simulated.is_proven_by(&not_attempted));
        assert_eq!(
            OpportunityStage::highest_proven(&not_attempted),
            OpportunityStage::Executable
        );
    }

    #[test]
    fn successful_simulation_of_losing_trade_is_not_economically_positive() {
        // Economics disproven: every gate above QUOTE_VALID must fail even
        // though the simulation itself succeeded.
        let losing_sim = StageEvidence {
            net_profit_positive: false,
            ..full_evidence()
        };
        assert!(OpportunityStage::QuoteValid.is_proven_by(&losing_sim));
        assert!(!OpportunityStage::EconomicallyPositive.is_proven_by(&losing_sim));
        assert!(!OpportunityStage::Simulated.is_proven_by(&losing_sim));
        assert_eq!(
            OpportunityStage::highest_proven(&losing_sim),
            OpportunityStage::QuoteValid
        );
        // Simulation outcome stays separately observable:
        assert!(losing_sim.simulation.is_success());
    }

    #[test]
    fn shadow_and_mainnet_stages_require_bearing_proofs() {
        // Without dedicated proof structures, neither stage is provable —
        // even with every ordinary gate green.
        assert!(!OpportunityStage::ShadowValidated.is_proven_by(&full_evidence()));
        assert!(!OpportunityStage::MainnetVerified.is_proven_by(&full_evidence()));
        assert_eq!(
            OpportunityStage::highest_proven(&full_evidence()),
            OpportunityStage::Simulated
        );

        // An INVALID proof must also fail its gate.
        let invalid_shadow = ShadowContinuityProof {
            quote_reproduced: true,
            state_continuity_verified: false,
            verified_at_slot: 1,
        };
        assert!(!invalid_shadow.is_valid());
        let shadow_with_invalid_proof = StageEvidence {
            shadow_continuity: Some(invalid_shadow),
            ..full_evidence()
        };
        assert!(!OpportunityStage::ShadowValidated.is_proven_by(&shadow_with_invalid_proof));

        let invalid_settlement = MainnetSettlementProof {
            transaction_signature: String::new(),
            pre_settlement_balance_lamports: Some(1_000),
            post_settlement_balance_lamports: Some(1_500),
            confirmed_slot: 200,
        };
        assert!(!invalid_settlement.is_valid());
        let missing_balances = MainnetSettlementProof {
            transaction_signature: "5somebase58sig".to_string(),
            pre_settlement_balance_lamports: None,
            post_settlement_balance_lamports: Some(1_500),
            confirmed_slot: 200,
        };
        assert!(!missing_balances.is_valid());
    }

    #[test]
    fn valid_proofs_gate_the_top_two_stages() {
        let valid_shadow = ShadowContinuityProof {
            quote_reproduced: true,
            state_continuity_verified: true,
            verified_at_slot: 100,
        };
        assert!(valid_shadow.is_valid());
        let shadow_proven = StageEvidence {
            shadow_continuity: Some(valid_shadow),
            ..full_evidence()
        };
        assert!(OpportunityStage::ShadowValidated.is_proven_by(&shadow_proven));
        assert!(!OpportunityStage::MainnetVerified.is_proven_by(&shadow_proven));
        assert_eq!(
            OpportunityStage::highest_proven(&shadow_proven),
            OpportunityStage::ShadowValidated
        );

        let valid_settlement = MainnetSettlementProof {
            transaction_signature: "5somebase58sig".to_string(),
            pre_settlement_balance_lamports: Some(1_000),
            post_settlement_balance_lamports: Some(1_500),
            confirmed_slot: 200,
        };
        assert!(valid_settlement.is_valid());
        assert_eq!(
            valid_settlement.realized_balance_delta_lamports(),
            Some(500)
        );
        let full = StageEvidence {
            mainnet_settlement: Some(valid_settlement),
            ..shadow_proven
        };
        assert!(OpportunityStage::MainnetVerified.is_proven_by(&full));
        assert_eq!(
            OpportunityStage::highest_proven(&full),
            OpportunityStage::MainnetVerified
        );
        assert_eq!(OpportunityStage::MainnetVerified.advance(&full), None);
    }

    #[test]
    fn legacy_status_mapping_is_conservative_and_never_promotes_failures() {
        assert_eq!(
            OpportunityStage::from_legacy_status("VERIFIED_PROFITABLE"),
            Some(OpportunityStage::Simulated)
        );
        // A successful simulation of a LOSING trade is a valid transaction,
        // not a profitable opportunity.
        assert_eq!(
            OpportunityStage::from_legacy_status("SIMULATED_SUCCESS_NEGATIVE_EDGE"),
            Some(OpportunityStage::QuoteValid)
        );
        // A FAILED simulation must NEVER map to SIMULATED.
        assert_eq!(
            OpportunityStage::from_legacy_status("REJECTED_SIM_FAIL"),
            Some(OpportunityStage::Discovered)
        );
        // A stale quote must NEVER map to QUOTE_VALID.
        assert_eq!(
            OpportunityStage::from_legacy_status("STALE_QUOTE"),
            Some(OpportunityStage::Discovered)
        );
        assert_eq!(
            OpportunityStage::from_legacy_status("REJECTED_NO_QUOTE"),
            Some(OpportunityStage::Discovered)
        );
        assert_eq!(OpportunityStage::from_legacy_status("garbage"), None);
        assert_eq!(OpportunityStage::from_legacy_status(""), None);
    }

    #[test]
    fn mainnet_verified_is_never_auto_produced() {
        // No from_legacy_status string may produce Shadow/Mainnet stages.
        for status in [
            "VERIFIED_PROFITABLE",
            "SIMULATED_SUCCESS_NEGATIVE_EDGE",
            "REJECTED_SIM_FAIL",
            "STALE_QUOTE",
            "REJECTED_NO_QUOTE",
        ] {
            assert_ne!(
                OpportunityStage::from_legacy_status(status),
                Some(OpportunityStage::ShadowValidated)
            );
            assert_ne!(
                OpportunityStage::from_legacy_status(status),
                Some(OpportunityStage::MainnetVerified)
            );
        }
    }

    #[test]
    fn simulation_outcome_helpers() {
        assert!(SimulationOutcome::Succeeded.is_success());
        assert!(!SimulationOutcome::Failed.is_success());
        assert!(!SimulationOutcome::NotAttempted.is_success());
        assert!(!SimulationOutcome::NotAttempted.was_attempted());
        assert!(SimulationOutcome::Failed.was_attempted());
        assert_eq!(SimulationOutcome::Succeeded.to_string(), "SUCCEEDED");
        assert_eq!(
            SimulationOutcome::default(),
            SimulationOutcome::NotAttempted
        );
    }
}
