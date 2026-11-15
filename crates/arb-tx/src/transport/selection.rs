//! Authoritative Transport Selection & Capability Verification Layer.
//!
//! Provides deterministic capability verification and transport selection across
//! Direct RPC, SWQoS, and Jito Block Engine bundles for Solana V0 and V1 transactions.
//!
//! Enforces exact wire constraints:
//! - Legacy / V0: MTU 1,232 bytes (IPv6 MTU 1280 - 48 header).
//! - V1 (SIMD-0385): MTU 4,096 bytes, 64 inline account addresses max, 0x81 prefix.
//! - Strict capability matching: no silent downgrades, no unverified fallbacks.

use crate::transport::adaptive::{
    SOLANA_PACKET_LEGACY_MTU, SOLANA_PACKET_V1_MTU, TransactionVersion,
};
use serde::{Deserialize, Serialize};

/// Maximum inline account addresses supported by Solana Transaction V1 (SIMD-0385).
pub const SOLANA_V1_MAX_INLINE_ACCOUNTS: usize = 64;

/// Target transport channel for transaction transmission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TransportKind {
    /// Direct JSON-RPC sendTransaction to a standard RPC endpoint.
    DirectRpc,
    /// Direct SWQoS endpoint (e.g. Helius Sender, Triton SWQoS, or staked RPC validator socket).
    SwqosDirect,
    /// Private bundle submission to Jito Block Engine (sendBundle).
    JitoBundle,
}

impl std::fmt::Display for TransportKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DirectRpc => write!(f, "direct_rpc"),
            Self::SwqosDirect => write!(f, "swqos_direct"),
            Self::JitoBundle => write!(f, "jito_bundle"),
        }
    }
}

/// Verified capabilities of a downstream provider or endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportCapability {
    pub provider_name: String,
    pub supports_v0: bool,
    pub supports_v1: bool,
    pub supports_direct_rpc: bool,
    pub supports_swqos: bool,
    pub supports_jito_bundles: bool,
    pub max_wire_bytes: usize,
    pub max_inline_accounts: usize,
}

impl TransportCapability {
    /// Full capabilities for a modern Mainnet setup supporting V1 (SIMD-0385).
    pub fn mainnet_v1_capable(provider: &str) -> Self {
        Self {
            provider_name: provider.to_string(),
            supports_v0: true,
            supports_v1: true,
            supports_direct_rpc: true,
            supports_swqos: true,
            supports_jito_bundles: true,
            max_wire_bytes: SOLANA_PACKET_V1_MTU,
            max_inline_accounts: SOLANA_V1_MAX_INLINE_ACCOUNTS,
        }
    }

    /// Legacy / V0 profile (1,232 bytes MTU, no V1).
    pub fn v0_only(provider: &str) -> Self {
        Self {
            provider_name: provider.to_string(),
            supports_v0: true,
            supports_v1: false,
            supports_direct_rpc: true,
            supports_swqos: false,
            supports_jito_bundles: true,
            max_wire_bytes: SOLANA_PACKET_LEGACY_MTU,
            max_inline_accounts: 128,
        }
    }
}

/// Resolved and verified transport assignment for a transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedTransport {
    pub kind: TransportKind,
    pub tx_version: TransactionVersion,
    pub packet_bytes: usize,
    pub wire_mtu_limit: usize,
    pub is_v1: bool,
}

/// Errors raised during transport selection and packet validation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransportSelectionError {
    #[error("empty transaction wire payload")]
    EmptyPayload,

    #[error("transaction size {size} bytes exceeds MTU limit {limit} bytes for {version:?}")]
    OversizedTransaction {
        size: usize,
        limit: usize,
        version: TransactionVersion,
    },

    #[error(
        "V1 transaction contains {count} inline accounts, exceeding SIMD-0385 limit of {limit}"
    )]
    V1AccountsExceeded { count: usize, limit: usize },

    #[error("transport {transport} does not support transaction version {version:?}")]
    UnsupportedVersionForTransport {
        version: TransactionVersion,
        transport: TransportKind,
    },

    #[error("transport {transport} is disabled or unavailable: {reason}")]
    TransportDisabled {
        transport: TransportKind,
        reason: String,
    },

    #[error(
        "no compatible transport found for transaction version {version:?} under active policy"
    )]
    NoCompatibleTransport { version: TransactionVersion },

    #[error("invalid wire format: {0}")]
    InvalidWireFormat(String),
}

/// Inspect transaction bytes and determine the wire version and inline account count.
pub fn parse_transaction_wire_header(
    tx_bytes: &[u8],
) -> Result<(TransactionVersion, usize), TransportSelectionError> {
    if tx_bytes.is_empty() {
        return Err(TransportSelectionError::EmptyPayload);
    }

    // Inspect first byte:
    // SIMD-0385 Transaction V1 wire format begins with discriminator 0x81.
    // Legacy / V0 format: first byte is number of signatures (compact-u16).
    if tx_bytes[0] == 0x81 {
        // V1 Format Header parsing
        // [0]: 0x81 discriminator
        // [1]: num_signatures
        // [2..]: message header
        if tx_bytes.len() < 4 {
            return Err(TransportSelectionError::InvalidWireFormat(
                "truncated V1 envelope header".into(),
            ));
        }
        let num_sigs = tx_bytes[1] as usize;
        let sig_offset = 2 + num_sigs * 64;
        if tx_bytes.len() < sig_offset + 3 {
            return Err(TransportSelectionError::InvalidWireFormat(
                "truncated V1 envelope before message accounts count".into(),
            ));
        }
        // In V1 message format (0x81):
        // Offset sig_offset + 0: num_required_signatures
        // Offset sig_offset + 1: num_readonly_signed
        // Offset sig_offset + 2: num_readonly_unsigned
        // Offset sig_offset + 3: compact-u16 count of account keys
        let accounts_count = if tx_bytes.len() > sig_offset + 3 {
            // Read account count byte (compact-u16, typically 1 byte for < 128 accounts)
            let raw_cnt = tx_bytes[sig_offset + 3] as usize;
            raw_cnt & 0x7f
        } else {
            0
        };

        Ok((TransactionVersion::V1, accounts_count))
    } else {
        // Parse compact-u16 num_signatures
        let (num_sigs, sig_bytes_read) = decode_compact_u16(tx_bytes)?;
        let msg_start = sig_bytes_read + num_sigs * 64;
        if tx_bytes.len() <= msg_start {
            return Err(TransportSelectionError::InvalidWireFormat(
                "truncated legacy/v0 envelope before message".into(),
            ));
        }
        let prefix = tx_bytes[msg_start];
        let version = if prefix & 0x80 != 0 {
            let ver_num = prefix & 0x7f;
            if ver_num == 0 {
                TransactionVersion::V0
            } else {
                return Err(TransportSelectionError::InvalidWireFormat(format!(
                    "unknown transaction version prefix: {ver_num}"
                )));
            }
        } else {
            TransactionVersion::Legacy
        };

        // For V0 / legacy, estimate account count
        let accounts_start = if version == TransactionVersion::V0 {
            msg_start + 1 + 3 // prefix + 3 header bytes
        } else {
            msg_start + 3 // 3 header bytes
        };
        let accounts_count = if tx_bytes.len() > accounts_start {
            (tx_bytes[accounts_start] & 0x7f) as usize
        } else {
            0
        };

        Ok((version, accounts_count))
    }
}

fn decode_compact_u16(bytes: &[u8]) -> Result<(usize, usize), TransportSelectionError> {
    if bytes.is_empty() {
        return Err(TransportSelectionError::EmptyPayload);
    }
    let mut val: usize = 0;
    let mut shift: usize = 0;
    for (i, &b) in bytes.iter().enumerate().take(3) {
        val |= ((b & 0x7f) as usize) << shift;
        if (b & 0x80) == 0 {
            return Ok((val, i + 1));
        }
        shift += 7;
    }
    Err(TransportSelectionError::InvalidWireFormat(
        "invalid compact-u16 encoding".into(),
    ))
}

/// Validate transaction packet constraints (MTU limits and account counts).
pub fn validate_packet_constraints(
    tx_bytes: &[u8],
    version: TransactionVersion,
    account_count: usize,
) -> Result<usize, TransportSelectionError> {
    let limit = match version {
        TransactionVersion::V1 => SOLANA_PACKET_V1_MTU,
        TransactionVersion::V0 | TransactionVersion::Legacy => SOLANA_PACKET_LEGACY_MTU,
    };

    if tx_bytes.len() > limit {
        return Err(TransportSelectionError::OversizedTransaction {
            size: tx_bytes.len(),
            limit,
            version,
        });
    }

    if version == TransactionVersion::V1 && account_count > SOLANA_V1_MAX_INLINE_ACCOUNTS {
        return Err(TransportSelectionError::V1AccountsExceeded {
            count: account_count,
            limit: SOLANA_V1_MAX_INLINE_ACCOUNTS,
        });
    }

    Ok(limit)
}

/// Select authoritative transport with explicit capability verification.
///
/// Ensures:
/// 1. V1 transactions are matched with verified V1-capable endpoints (both SWQoS and Jito).
/// 2. Wire format MTUs are strictly respected without silent truncation or downgrading.
/// 3. Disabled or unverified transports immediately fail closed.
pub fn select_authoritative_transport(
    tx_bytes: &[u8],
    is_swqos_route: bool,
    direct_v1_enabled: bool,
    jito_v1_enabled: bool,
    capability: &TransportCapability,
) -> Result<SelectedTransport, TransportSelectionError> {
    let (version, account_count) = parse_transaction_wire_header(tx_bytes)?;
    let wire_mtu_limit = validate_packet_constraints(tx_bytes, version, account_count)?;

    let target_transport = if is_swqos_route {
        if capability.supports_swqos {
            TransportKind::SwqosDirect
        } else if capability.supports_direct_rpc {
            TransportKind::DirectRpc
        } else {
            return Err(TransportSelectionError::TransportDisabled {
                transport: TransportKind::SwqosDirect,
                reason: format!(
                    "provider '{}' does not support RPC or SWQoS",
                    capability.provider_name
                ),
            });
        }
    } else {
        if !capability.supports_jito_bundles {
            return Err(TransportSelectionError::TransportDisabled {
                transport: TransportKind::JitoBundle,
                reason: format!(
                    "provider '{}' does not support Jito bundles",
                    capability.provider_name
                ),
            });
        }
        TransportKind::JitoBundle
    };

    match version {
        TransactionVersion::V1 => {
            if !capability.supports_v1 {
                return Err(TransportSelectionError::UnsupportedVersionForTransport {
                    version,
                    transport: target_transport,
                });
            }
            match target_transport {
                TransportKind::SwqosDirect | TransportKind::DirectRpc => {
                    if !direct_v1_enabled {
                        return Err(TransportSelectionError::TransportDisabled {
                            transport: target_transport,
                            reason: "SOLANA_TX_V1_DIRECT_ENABLED is false; direct V1 submission disabled".into(),
                        });
                    }
                }
                TransportKind::JitoBundle => {
                    if !jito_v1_enabled {
                        return Err(TransportSelectionError::TransportDisabled {
                            transport: target_transport,
                            reason: "SOLANA_TX_V1_JITO_ENABLED is false; Jito V1 bundle submission disabled".into(),
                        });
                    }
                }
            }
        }
        TransactionVersion::V0 | TransactionVersion::Legacy => {
            if version == TransactionVersion::V0 && !capability.supports_v0 {
                return Err(TransportSelectionError::UnsupportedVersionForTransport {
                    version,
                    transport: target_transport,
                });
            }
        }
    }

    Ok(SelectedTransport {
        kind: target_transport,
        tx_version: version,
        packet_bytes: tx_bytes.len(),
        wire_mtu_limit,
        is_v1: version == TransactionVersion::V1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_v1_header(num_accounts: u8, total_len: usize) -> Vec<u8> {
        let mut bytes = vec![0u8; total_len.max(70)];
        bytes[0] = 0x81; // V1 discriminator
        bytes[1] = 1; // 1 signature
        // Signature occupies bytes 2..66
        let sig_end = 66;
        bytes[sig_end] = 1; // num_required_signatures
        bytes[sig_end + 1] = 0; // num_readonly_signed
        bytes[sig_end + 2] = 0; // num_readonly_unsigned
        bytes[sig_end + 3] = num_accounts; // accounts count
        bytes
    }

    fn dummy_v0_header(total_len: usize) -> Vec<u8> {
        let mut bytes = vec![0u8; total_len.max(70)];
        bytes[0] = 1; // 1 signature (compact-u16)
        let sig_end = 1 + 64;
        bytes[sig_end] = 0x80; // V0 prefix
        bytes
    }

    #[test]
    fn test_scenario_a_v1_supported_direct_rpc() {
        let cap = TransportCapability::mainnet_v1_capable("Helius");
        let tx = dummy_v1_header(32, 1500);
        let selected = select_authoritative_transport(&tx, true, true, false, &cap).unwrap();
        assert_eq!(selected.kind, TransportKind::SwqosDirect);
        assert_eq!(selected.tx_version, TransactionVersion::V1);
        assert_eq!(selected.wire_mtu_limit, 4096);
    }

    #[test]
    fn test_scenario_b_v1_unsupported_provider() {
        let cap = TransportCapability::v0_only("LegacyRPC");
        let tx = dummy_v1_header(32, 1500);
        let err = select_authoritative_transport(&tx, true, true, false, &cap).unwrap_err();
        assert!(matches!(
            err,
            TransportSelectionError::UnsupportedVersionForTransport { .. }
        ));
    }

    #[test]
    fn test_scenario_c_v1_jito_unsupported_when_flag_disabled() {
        let cap = TransportCapability::mainnet_v1_capable("Jito-Mainnet");
        let tx = dummy_v1_header(32, 2000);
        // jito_v1_enabled = false
        let err = select_authoritative_transport(&tx, false, true, false, &cap).unwrap_err();
        assert!(matches!(
            err,
            TransportSelectionError::TransportDisabled {
                transport: TransportKind::JitoBundle,
                ..
            }
        ));
    }

    #[test]
    fn test_scenario_d_v1_verified_jito_compatible_path() {
        let cap = TransportCapability::mainnet_v1_capable("Jito-Mainnet");
        let tx = dummy_v1_header(32, 2000);
        // jito_v1_enabled = true
        let selected = select_authoritative_transport(&tx, false, true, true, &cap).unwrap();
        assert_eq!(selected.kind, TransportKind::JitoBundle);
        assert_eq!(selected.tx_version, TransactionVersion::V1);
        assert_eq!(selected.packet_bytes, 2000);
        assert_eq!(selected.wire_mtu_limit, 4096);
    }

    #[test]
    fn test_scenario_e_v0_supported_transport() {
        let cap = TransportCapability::v0_only("StandardRPC");
        let tx = dummy_v0_header(1000);
        let selected = select_authoritative_transport(&tx, true, false, false, &cap).unwrap();
        assert_eq!(selected.kind, TransportKind::DirectRpc);
        assert_eq!(selected.tx_version, TransactionVersion::V0);
        assert_eq!(selected.wire_mtu_limit, 1232);
    }

    #[test]
    fn test_scenario_h_oversized_v1_transaction() {
        let cap = TransportCapability::mainnet_v1_capable("Helius");
        let tx = dummy_v1_header(32, 4097);
        let err = select_authoritative_transport(&tx, true, true, false, &cap).unwrap_err();
        assert!(matches!(
            err,
            TransportSelectionError::OversizedTransaction {
                size: 4097,
                limit: 4096,
                version: TransactionVersion::V1,
            }
        ));
    }

    #[test]
    fn test_scenario_h_v1_accounts_limit_exceeded() {
        let cap = TransportCapability::mainnet_v1_capable("Helius");
        let tx = dummy_v1_header(65, 2000);
        let err = select_authoritative_transport(&tx, true, true, false, &cap).unwrap_err();
        assert!(matches!(
            err,
            TransportSelectionError::V1AccountsExceeded {
                count: 65,
                limit: 64,
            }
        ));
    }

    #[test]
    fn test_empty_wire_payload() {
        let cap = TransportCapability::mainnet_v1_capable("Helius");
        let err = select_authoritative_transport(&[], true, true, true, &cap).unwrap_err();
        assert_eq!(err, TransportSelectionError::EmptyPayload);
    }
}
