//! Jito leader landability.
//!
//! A tip only buys inclusion when the leader of the target slot runs
//! Jito-Solana. Against any other leader the tip is money burned with no
//! auction to win, so the decision to tip at all is gated on the schedule.
//!
//! Two sources are joined here:
//!
//! * `getLeaderSchedule`, which maps a validator **identity** to the slot
//!   offsets it leads inside one epoch;
//! * the kobe validator registry, which reports `running_jito` per validator and
//!   carries both the identity and the vote account.
//!
//! Both keys are indexed: the schedule speaks identities, while configuration
//! and operator tooling usually speak vote accounts, and a set holding only one
//! of the two silently classifies half the schedule as non-Jito.
//!
//! This module performs no I/O — it parses resolved payloads and answers a pure
//! question, so it is exhaustively testable without a network.

use arb_types::{ArbError, Result as ArbResult, Slot};
use serde_json::Value;
use solana_pubkey::Pubkey;
use std::collections::HashSet;
use std::str::FromStr;

/// One scheduled leader slot inside an epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeaderSlot {
    /// Slot offset relative to the epoch's first slot.
    pub offset: u32,
    /// Leader identity.
    pub identity: Pubkey,
}

/// Inverted leader schedule for a single epoch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaderSchedule {
    epoch: u64,
    epoch_start_slot: u64,
    slots_in_epoch: u64,
    entries: Vec<LeaderSlot>,
}

impl LeaderSchedule {
    /// Build a schedule from a `getLeaderSchedule` result object.
    ///
    /// The payload is `{ "<identity>": [offset, ...] }`. A `result` wrapper is
    /// unwrapped when present, so callers may pass either the RPC envelope or
    /// the resolved result.
    pub fn from_rpc_json(
        value: &Value,
        epoch: u64,
        epoch_start_slot: u64,
        slots_in_epoch: u64,
    ) -> ArbResult<Self> {
        if slots_in_epoch == 0 {
            return Err(ArbError::InvalidInput(
                "slots_in_epoch must be > 0".to_string(),
            ));
        }
        let object = value
            .get("result")
            .unwrap_or(value)
            .as_object()
            .ok_or_else(|| {
                ArbError::DeserializationError("leader schedule is not an object".to_string())
            })?;

        let mut entries = Vec::new();
        for (identity, offsets) in object {
            let identity = Pubkey::from_str(identity).map_err(|e| {
                ArbError::DeserializationError(format!("invalid leader identity {identity}: {e}"))
            })?;
            let offsets = offsets.as_array().ok_or_else(|| {
                ArbError::DeserializationError(format!(
                    "leader slots for {identity} are not an array"
                ))
            })?;
            for raw in offsets {
                let Some(offset) = raw.as_u64() else {
                    continue;
                };
                if offset >= slots_in_epoch {
                    // An offset outside the epoch is a malformed entry, not a
                    // leader for a future slot.
                    continue;
                }
                let Ok(offset) = u32::try_from(offset) else {
                    continue;
                };
                entries.push(LeaderSlot { offset, identity });
            }
        }

        entries.sort_unstable_by_key(|entry| entry.offset);

        Ok(Self {
            epoch,
            epoch_start_slot,
            slots_in_epoch,
            entries,
        })
    }

    /// Epoch this schedule describes.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Absolute slot of the epoch's first slot.
    pub fn epoch_start_slot(&self) -> u64 {
        self.epoch_start_slot
    }

    /// Slots in this epoch.
    pub fn slots_in_epoch(&self) -> u64 {
        self.slots_in_epoch
    }

    /// Number of scheduled leader slots.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no slot is scheduled.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// True when `slot` falls inside this epoch.
    pub fn covers(&self, slot: Slot) -> bool {
        slot.0 >= self.epoch_start_slot
            && slot.0 < self.epoch_start_slot.saturating_add(self.slots_in_epoch)
    }

    /// Leader identity for an absolute slot, or `None` when the slot is outside
    /// this schedule.
    pub fn leader_at(&self, slot: Slot) -> Option<Pubkey> {
        let offset = slot.0.checked_sub(self.epoch_start_slot)?;
        if offset >= self.slots_in_epoch {
            return None;
        }
        let offset = u32::try_from(offset).ok()?;
        let index = self
            .entries
            .binary_search_by_key(&offset, |entry| entry.offset)
            .ok()?;
        self.entries.get(index).map(|entry| entry.identity)
    }
}

/// Validators that run Jito-Solana, indexed by identity *and* vote account.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct JitoValidatorSet {
    keys: HashSet<Pubkey>,
}

impl JitoValidatorSet {
    /// Build the set from a kobe `/api/v1/validators` payload.
    ///
    /// Only entries with `running_jito == true` are kept: a registered validator
    /// that is not currently running the Jito client cannot include a bundle.
    pub fn from_kobe_json(value: &Value) -> ArbResult<Self> {
        let validators = value
            .get("validators")
            .unwrap_or(value)
            .as_array()
            .ok_or_else(|| {
                ArbError::DeserializationError("kobe payload has no validators array".to_string())
            })?;

        let mut keys = HashSet::new();
        for entry in validators {
            let running_jito = entry
                .get("running_jito")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !running_jito {
                continue;
            }
            for field in ["identity_account", "vote_account"] {
                if let Some(raw) = entry.get(field).and_then(Value::as_str)
                    && let Ok(pubkey) = Pubkey::from_str(raw)
                {
                    keys.insert(pubkey);
                }
            }
        }

        Ok(Self { keys })
    }

    /// Build the set from an explicit list of keys, for tests and fixtures.
    pub fn from_keys(keys: impl IntoIterator<Item = Pubkey>) -> Self {
        Self {
            keys: keys.into_iter().collect(),
        }
    }

    /// True when the validator is known to run Jito-Solana.
    pub fn contains(&self, key: &Pubkey) -> bool {
        self.keys.contains(key)
    }

    /// Number of indexed keys (identities and vote accounts together).
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// True when no validator is indexed.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// Whether a tip can accomplish anything in a given slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Landability {
    /// The slot's leader runs Jito-Solana.
    Jito,
    /// The slot's leader does not run Jito-Solana: a tip buys nothing.
    NonJito,
    /// The schedule or validator set does not cover this slot.
    Unknown,
}

impl Landability {
    /// True only when a tip reaches a validator that can include it.
    pub fn is_jito(self) -> bool {
        matches!(self, Self::Jito)
    }

    /// True when the answer is known.
    pub fn is_known(self) -> bool {
        !matches!(self, Self::Unknown)
    }
}

/// Classify a slot from the currently held schedule and validator set.
pub fn landability(
    schedule: Option<&LeaderSchedule>,
    validators: Option<&JitoValidatorSet>,
    slot: Slot,
) -> Landability {
    let (Some(schedule), Some(validators)) = (schedule, validators) else {
        return Landability::Unknown;
    };
    let Some(leader) = schedule.leader_at(slot) else {
        return Landability::Unknown;
    };
    if validators.contains(&leader) {
        Landability::Jito
    } else {
        Landability::NonJito
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const JITO_IDENTITY: &str = "2X5yarNUWGyyiJDtRoPATvJYeNecvWG7zCkBrY2J1to1";
    const JITO_VOTE: &str = "J1to1yufRnoWn81KYg1XkTWzmKjnYSnmE2VY8DGUJ9Qv";
    const PLAIN_IDENTITY: &str = "BCJN2vZFAHDYmufBDcbD5UAQHSyerXfc6UQkgX3mSWuh";
    const PLAIN_VOTE: &str = "647eBPzfDqmSVEg5cQofnjzeFr7PfwvHcDEcb22JWA5X";

    fn kobe_payload() -> Value {
        json!({
            "validators": [
                {
                    "identity_account": JITO_IDENTITY,
                    "vote_account": JITO_VOTE,
                    "running_jito": true,
                    "running_bam": false,
                    "mev_commission_bps": 800,
                    "active_stake": 515961248711663u64
                },
                {
                    "identity_account": PLAIN_IDENTITY,
                    "vote_account": PLAIN_VOTE,
                    "running_jito": false,
                    "running_bam": false,
                    "mev_commission_bps": 0,
                    "active_stake": 97820622382680u64
                }
            ]
        })
    }

    #[test]
    fn kobe_indexes_identity_and_vote() {
        let set = JitoValidatorSet::from_kobe_json(&kobe_payload()).expect("parse");
        assert_eq!(set.len(), 2);
        assert!(set.contains(&Pubkey::from_str(JITO_IDENTITY).expect("identity")));
        assert!(set.contains(&Pubkey::from_str(JITO_VOTE).expect("vote")));
    }

    #[test]
    fn kobe_excludes_validators_not_running_jito() {
        let set = JitoValidatorSet::from_kobe_json(&kobe_payload()).expect("parse");
        assert!(!set.contains(&Pubkey::from_str(PLAIN_IDENTITY).expect("identity")));
        assert!(!set.contains(&Pubkey::from_str(PLAIN_VOTE).expect("vote")));
    }

    #[test]
    fn kobe_rejects_payload_without_validators() {
        assert!(JitoValidatorSet::from_kobe_json(&json!({"foo": 1})).is_err());
    }

    #[test]
    fn schedule_maps_offsets_to_identities() {
        let payload = json!({
            JITO_IDENTITY: [0, 1, 2, 3],
            PLAIN_IDENTITY: [4, 5, 6, 7]
        });
        let schedule =
            LeaderSchedule::from_rpc_json(&payload, 700, 302_400_000, 432_000).expect("parse");

        assert_eq!(schedule.epoch(), 700);
        assert_eq!(schedule.len(), 8);
        assert_eq!(
            schedule.leader_at(Slot(302_400_000)),
            Some(Pubkey::from_str(JITO_IDENTITY).expect("identity"))
        );
        assert_eq!(
            schedule.leader_at(Slot(302_400_005)),
            Some(Pubkey::from_str(PLAIN_IDENTITY).expect("identity"))
        );
    }

    #[test]
    fn schedule_unwraps_the_rpc_envelope() {
        let payload = json!({ "result": { JITO_IDENTITY: [0, 1] } });
        let schedule = LeaderSchedule::from_rpc_json(&payload, 1, 0, 432_000).expect("parse");
        assert_eq!(schedule.len(), 2);
    }

    #[test]
    fn schedule_returns_none_outside_its_epoch() {
        let payload = json!({ JITO_IDENTITY: [0, 1] });
        let schedule = LeaderSchedule::from_rpc_json(&payload, 1, 1_000, 432_000).expect("parse");
        // Before the epoch.
        assert!(schedule.leader_at(Slot(999)).is_none());
        // After the epoch.
        assert!(schedule.leader_at(Slot(1_000 + 432_000)).is_none());
        // A gap inside the epoch.
        assert!(schedule.leader_at(Slot(1_050)).is_none());
    }

    #[test]
    fn schedule_drops_out_of_range_offsets() {
        let payload = json!({ JITO_IDENTITY: [0, 999_999] });
        let schedule = LeaderSchedule::from_rpc_json(&payload, 1, 0, 1_000).expect("parse");
        assert_eq!(schedule.len(), 1);
    }

    #[test]
    fn schedule_rejects_zero_epoch_length() {
        assert!(LeaderSchedule::from_rpc_json(&json!({}), 1, 0, 0).is_err());
    }

    #[test]
    fn landability_classifies_both_directions() {
        let schedule = LeaderSchedule::from_rpc_json(
            &json!({ JITO_IDENTITY: [0], PLAIN_IDENTITY: [1] }),
            1,
            500,
            432_000,
        )
        .expect("parse");
        let set = JitoValidatorSet::from_kobe_json(&kobe_payload()).expect("parse");

        assert_eq!(
            landability(Some(&schedule), Some(&set), Slot(500)),
            Landability::Jito
        );
        assert_eq!(
            landability(Some(&schedule), Some(&set), Slot(501)),
            Landability::NonJito
        );
        assert_eq!(
            landability(Some(&schedule), Some(&set), Slot(9_999)),
            Landability::Unknown
        );
        assert_eq!(
            landability(None, Some(&set), Slot(500)),
            Landability::Unknown
        );
        assert_eq!(
            landability(Some(&schedule), None, Slot(500)),
            Landability::Unknown
        );
    }

    #[test]
    fn vote_account_alone_still_identifies_a_jito_leader() {
        let schedule = LeaderSchedule::from_rpc_json(&json!({ JITO_VOTE: [0] }), 1, 0, 432_000)
            .expect("parse");
        let set = JitoValidatorSet::from_kobe_json(&kobe_payload()).expect("parse");
        assert_eq!(
            landability(Some(&schedule), Some(&set), Slot(0)),
            Landability::Jito
        );
    }
}
