//! Simulation forensic capture and deterministic replay bundle (ISSUE-P0-04).
//!
//! Bind, in one artifact, everything required to reproduce a failed
//! pre-flight simulation deterministically: the exact signed wire transaction,
//! per-hop quote/minimum-output provenance, the quote and simulation context
//! slots, bounded program logs, and the transaction wire hash. Without this
//! binding a recorded error such as `Custom(6003)` is ambiguous — the same
//! numeric code is arb-executor's `InvalidMemoProgramId` AND Meteora DLMM's
//! `ExceededAmountSlippageTolerance` — so the capturing program logs are what
//! identify the emitting program.
//!
//! Replay loads the wire bytes back and re-runs them through the local
//! LiteSVM simulation (or any deterministic harness), which re-derives all
//! economic floors from canonical sources at replay time; the bundle records
//! raw provenance, never a duplicated profit calculation.

#[cfg(feature = "litesvm")]
use crate::LocalSimResult;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Capture schema version. Bump whenever the bundle semantics change.
pub const FORENSIC_CAPTURE_VERSION: u32 = 1;

/// Program logs are untrusted, potentially unbounded output: bound them so a
/// verbose program (or adversarial state) cannot produce a multi-gigabyte
/// forensic file.
pub const MAX_CAPTURED_LOG_LINES: usize = 200;
pub const MAX_CAPTURED_LOG_LINE_CHARS: usize = 400;

/// Identifier of the program believed to have emitted the failing error when
/// the log tail contains its `${program_id}: failed` marker line.
fn emitting_program_from_logs(logs: &[String]) -> Option<String> {
    logs.iter()
        .rev()
        .find(|l| l.contains("failed") && l.contains("Program log"))
        .or_else(|| logs.iter().rev().find(|l| l.contains("failed")))
        .cloned()
}

/// One hop's quote provenance at capture time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HopProvenance {
    pub hop_index: usize,
    /// Pool public key, base58.
    pub pool_id: String,
    /// DEX protocol identifier (display name as carried by the opportunity).
    pub protocol: String,
    /// Input mint of the hop, base58.
    pub mint_in: String,
    /// Output mint of the hop, base58.
    pub mint_out: String,
    /// Nominal input the quote used for this hop.
    pub nominal_amount_in: u64,
    /// Expected output the quote returned for this hop.
    pub expected_out: u64,
    /// Slot of the pool state the quote consumed.
    pub quote_slot: u64,
}

/// Deterministic-replay bundle for one failed simulation attempt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ForensicBundle {
    pub capture_version: u32,
    pub captured_at_unix_ms: u64,
    /// Slot of the quote chain the opportunity was priced from.
    pub quote_slot: u64,
    /// Context slot reported by the simulation provider, when available.
    pub context_slot: Option<u64>,
    /// Base64 of the exact signed wire transaction that failed.
    pub tx_wire_base64: String,
    /// sha256 of the raw wire bytes (hex), binding the captured provenance to
    /// the exact transaction even if the base64 were corrupted.
    pub tx_wire_sha256: String,
    /// Per-hop quote provenance at capture time.
    pub per_hop_provenance: Vec<HopProvenance>,
    /// Slippage tolerance (bps) the builders used to derive per-hop min-outs;
    /// floors are re-derived from this at replay through the canonical builder.
    pub hop_slippage_bps: u64,
    /// Final expected output of the full route at capture time.
    pub expected_out: u64,
    /// Bounded program logs from the failed simulation.
    pub program_logs: Vec<String>,
    /// Program-best-effort attribution of the failing error (log tail scan).
    pub emitting_program_hint: Option<String>,
    /// Error text reported by the simulation provider.
    pub error: Option<String>,
    pub compute_units_consumed: u64,
}

impl ForensicBundle {
    /// Capture a failed simulation. `tx_wire` is the exact signed wire bytes.
    #[allow(clippy::too_many_arguments)]
    pub fn capture(
        tx_wire: &[u8],
        quote_slot: u64,
        context_slot: Option<u64>,
        per_hop_provenance: Vec<HopProvenance>,
        hop_slippage_bps: u64,
        expected_out: u64,
        logs: &[String],
        error: Option<String>,
        compute_units_consumed: u64,
        now_unix_ms: u64,
    ) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(tx_wire);
        let digest = hasher.finalize();
        let tx_wire_sha256: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        let bounded = Self::bound_logs(logs);
        let hint = emitting_program_from_logs(&bounded);
        Self {
            capture_version: FORENSIC_CAPTURE_VERSION,
            captured_at_unix_ms: now_unix_ms,
            quote_slot,
            context_slot,
            tx_wire_base64: {
                use base64::Engine;
                base64::engine::general_purpose::STANDARD.encode(tx_wire)
            },
            tx_wire_sha256,
            per_hop_provenance,
            hop_slippage_bps,
            expected_out,
            program_logs: bounded,
            emitting_program_hint: hint,
            error,
            compute_units_consumed,
        }
    }

    /// Bound untrusted logs: at most `MAX_CAPTURED_LOG_LINES` lines, each
    /// truncated to `MAX_CAPTURED_LOG_LINE_CHARS` chars (tail truncation keeps
    /// the line start, which is where program ids and error markers live).
    pub fn bound_logs(logs: &[String]) -> Vec<String> {
        logs.iter()
            .take(MAX_CAPTURED_LOG_LINES)
            .map(|l| {
                if l.chars().count() > MAX_CAPTURED_LOG_LINE_CHARS {
                    let s: String = l.chars().take(MAX_CAPTURED_LOG_LINE_CHARS).collect();
                    format!("{s}...[truncated]")
                } else {
                    l.clone()
                }
            })
            .collect()
    }

    /// Raw wire bytes recovered from the bundle.
    pub fn tx_wire(&self) -> Result<Vec<u8>, String> {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(self.tx_wire_base64.as_bytes())
            .map_err(|e| format!("tx wire base64 decode failed: {e}"))?;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let digest = hasher.finalize();
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        if hex != self.tx_wire_sha256 {
            return Err(format!(
                "tx wire sha256 mismatch: bundle pins {:#?}, recovered {hex}",
                self.tx_wire_sha256
            ));
        }
        Ok(bytes)
    }

    /// Write the bundle to `<dir>/forensic-<unix_ms>-<sha_prefix>.json` and
    /// return the written path.
    pub fn write_bundle(&self, dir: &Path) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(dir)?;
        let sha_head = &self.tx_wire_sha256[..16.min(self.tx_wire_sha256.len())];
        let path = dir.join(format!(
            "forensic-{}-{sha_head}.json",
            self.captured_at_unix_ms
        ));
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(&path, json)?;
        Ok(path)
    }

    /// Load a bundle written by [`Self::write_bundle`].
    pub fn load_bundle(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("read failed: {e}"))?;
        serde_json::from_slice(&bytes).map_err(|e| format!("parse failed: {e}"))
    }

    /// Replay the captured wire bytes through the local LiteSVM simulation.
    ///
    /// This is a *deterministic re-execution* hook: the VM is instantiated by
    /// the caller (programs and accounts must be deployed there per the
    /// fail-closed `simulate_locally` contract before the result may count as
    /// evidence).
    #[cfg(feature = "litesvm")]
    pub fn replay_locally(&self) -> Result<LocalSimResult, String> {
        let wire = self.tx_wire()?;
        Ok(crate::litesvm_sim::simulate_locally(&wire))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_hops() -> Vec<HopProvenance> {
        vec![
            HopProvenance {
                hop_index: 0,
                pool_id: "11111111111111111111111111111111".into(),
                protocol: "MeteoraDlmm".into(),
                mint_in: "So11111111111111111111111111111111111111112".into(),
                mint_out: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".into(),
                nominal_amount_in: 1_000_000,
                expected_out: 1_005_000,
                quote_slot: 472_000_000,
            },
            HopProvenance {
                hop_index: 1,
                pool_id: "22222222222222222222222222222222".into(),
                protocol: "RaydiumCpmm".into(),
                mint_in: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".into(),
                mint_out: "So11111111111111111111111111111111111111112".into(),
                nominal_amount_in: 1_005_000,
                expected_out: 1_006_500,
                quote_slot: 472_000_000,
            },
        ]
    }

    #[test]
    fn capture_binds_wire_hash_and_emitter_hint() {
        let wire = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        let logs = vec![
            "Program 11111 invoke [1]".to_string(),
            "Program log: Instruction: Swap".to_string(),
            "Program 11111: failed: custom program error: 0x1773".to_string(),
        ];
        let bundle = ForensicBundle::capture(
            &wire,
            472_000_000,
            Some(472_000_010),
            sample_hops(),
            10,
            1_006_500,
            &logs,
            Some("Custom(6003)".into()),
            12_345,
            1_700_000_000_000,
        );
        assert_eq!(bundle.capture_version, FORENSIC_CAPTURE_VERSION);
        assert_eq!(bundle.tx_wire_sha256.len(), 64);
        assert_eq!(
            bundle.emitting_program_hint.as_deref(),
            Some("Program 11111: failed: custom program error: 0x1773"),
            "the emitting-program hint must come from log tail"
        );
        let recovered = bundle.tx_wire().unwrap();
        assert_eq!(recovered, wire);
    }

    #[test]
    fn tampered_wire_fails_closed() {
        let bundle =
            ForensicBundle::capture(&[9u8; 32], 1, None, Vec::new(), 0, 0, &[], None, 0, 0);
        let mut tampered = bundle.clone();
        tampered.tx_wire_sha256 = "0".repeat(64);
        assert!(
            tampered.tx_wire().is_err(),
            "wire hash mismatch must refuse replay"
        );
    }

    #[test]
    fn logs_are_bounded() {
        let mut logs: Vec<String> = (0..2_000)
            .map(|i| format!("Program log: {} {}", "x".repeat(1_000), i))
            .collect();
        logs.insert(0, "head".to_string());
        let bounded = ForensicBundle::bound_logs(&logs);
        assert_eq!(bounded.len(), MAX_CAPTURED_LOG_LINES);
        assert_eq!(bounded[0], "head");
        assert!(
            bounded[1].chars().count() <= MAX_CAPTURED_LOG_LINE_CHARS + "...[truncated]".len(),
            "long lines must be truncated"
        );
    }

    #[test]
    fn roundtrip_through_disk() {
        let tmp = std::env::temp_dir().join(format!(
            "forensic-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let wire = vec![7u8; 64];
        let bundle = ForensicBundle::capture(
            &wire,
            42,
            Some(43),
            sample_hops(),
            10,
            99,
            &["Program 1: failed".to_string()],
            Some("boom".into()),
            5,
            1_700_000_000_123,
        );
        let path = bundle.write_bundle(&tmp).unwrap();
        let loaded = ForensicBundle::load_bundle(&path).unwrap();
        assert_eq!(loaded, bundle);
        assert!(
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .contains("forensic-1700000000123-")
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[cfg(feature = "litesvm")]
    #[test]
    fn replay_is_deterministic_for_identical_wire() {
        let wire = vec![3u8; 48];
        let bundle = ForensicBundle::capture(
            &wire,
            1,
            Some(1),
            Vec::new(),
            0,
            0,
            &[],
            Some("Custom(6000)".into()),
            1,
            0,
        );
        let first = bundle.replay_locally().unwrap();
        assert!(!first.success, "garbage wire must not execute");
        assert!(
            first
                .error
                .as_deref()
                .unwrap_or("")
                .contains("deserialization"),
            "replay must report the true failure class, got {first:?}"
        );
    }
}
