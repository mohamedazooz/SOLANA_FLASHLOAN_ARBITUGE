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

    // SIMD-0385 V1 starts with a dedicated 0x81 discriminator. Its header
    // keeps the fixed account-count offset at byte 41; config values are
    // serialized AFTER the inline address array, not before it.
    if tx_bytes[0] == 0x81 {
        if tx_bytes.len() < 42 {
            return Err(TransportSelectionError::InvalidWireFormat(
                "truncated V1 fixed header".into(),
            ));
        }

        let required_signatures = usize::from(tx_bytes[1]);
        let readonly_signed = usize::from(tx_bytes[2]);
        let readonly_unsigned = usize::from(tx_bytes[3]);
        let config_mask = u32::from_le_bytes(tx_bytes[4..8].try_into().map_err(|_| {
            TransportSelectionError::InvalidWireFormat("truncated V1 config mask".into())
        })?);
        let instruction_count = usize::from(tx_bytes[40]);
        let accounts_count = usize::from(tx_bytes[41]);

        // There must be a payer signature and at least one instruction/account.
        if required_signatures == 0
            || accounts_count == 0
            || instruction_count == 0
            || required_signatures > accounts_count
            || readonly_signed > required_signatures
            || readonly_unsigned > accounts_count.saturating_sub(required_signatures)
        {
            return Err(TransportSelectionError::InvalidWireFormat(
                "invalid V1 signature, instruction, or account counts".into(),
            ));
        }

        // Only defined config bits are valid. Priority fee occupies bits 0 and
        // 1 together and contributes one u64 value; a partial pair is invalid.
        const VALID_CONFIG_MASK: u32 = 0x1f;
        const PRIORITY_FEE_MASK: u32 = 0x03;
        const CU_LIMIT_MASK: u32 = 0x04;
        const LOADED_DATA_MASK: u32 = 0x08;
        const HEAP_SIZE_MASK: u32 = 0x10;
        let priority_bits = config_mask & PRIORITY_FEE_MASK;
        if config_mask & !VALID_CONFIG_MASK != 0
            || (priority_bits != 0 && priority_bits != PRIORITY_FEE_MASK)
        {
            return Err(TransportSelectionError::InvalidWireFormat(
                "unknown or partially-set V1 transaction config mask".into(),
            ));
        }

        let config_values_len = if priority_bits == PRIORITY_FEE_MASK {
            8usize
        } else {
            0
        } + if config_mask & CU_LIMIT_MASK != 0 {
            4
        } else {
            0
        } + if config_mask & LOADED_DATA_MASK != 0 {
            4
        } else {
            0
        } + if config_mask & HEAP_SIZE_MASK != 0 {
            4
        } else {
            0
        };

        // Fixed header (42) -> inline addresses -> config values -> instruction
        // headers -> instruction payloads -> trailing signatures.
        let address_bytes = accounts_count.checked_mul(32).ok_or_else(|| {
            TransportSelectionError::InvalidWireFormat("V1 address array length overflow".into())
        })?;
        let addresses_end = 42usize.checked_add(address_bytes).ok_or_else(|| {
            TransportSelectionError::InvalidWireFormat("V1 address array offset overflow".into())
        })?;
        let config_end = addresses_end
            .checked_add(config_values_len)
            .ok_or_else(|| {
                TransportSelectionError::InvalidWireFormat(
                    "V1 config values offset overflow".into(),
                )
            })?;
        let instruction_headers_bytes = instruction_count.checked_mul(4).ok_or_else(|| {
            TransportSelectionError::InvalidWireFormat(
                "V1 instruction header length overflow".into(),
            )
        })?;
        let instruction_headers_end = config_end
            .checked_add(instruction_headers_bytes)
            .ok_or_else(|| {
                TransportSelectionError::InvalidWireFormat(
                    "V1 instruction header offset overflow".into(),
                )
            })?;
        let signature_bytes = required_signatures.checked_mul(64).ok_or_else(|| {
            TransportSelectionError::InvalidWireFormat("V1 signature length overflow".into())
        })?;
        if instruction_headers_end > tx_bytes.len()
            || signature_bytes > tx_bytes.len().saturating_sub(instruction_headers_end)
        {
            return Err(TransportSelectionError::InvalidWireFormat(
                "truncated V1 addresses, config values, instruction headers, or signatures".into(),
            ));
        }

        let mut payload_offset = instruction_headers_end;
        for index in 0..instruction_count {
            let header_offset = config_end + index * 4;
            let program_index = usize::from(tx_bytes[header_offset]);
            let instruction_accounts = usize::from(tx_bytes[header_offset + 1]);
            let data_len = usize::from(u16::from_le_bytes([
                tx_bytes[header_offset + 2],
                tx_bytes[header_offset + 3],
            ]));
            if program_index >= accounts_count {
                return Err(TransportSelectionError::InvalidWireFormat(
                    "V1 instruction program index is outside the address table".into(),
                ));
            }
            let accounts_end = payload_offset
                .checked_add(instruction_accounts)
                .ok_or_else(|| {
                    TransportSelectionError::InvalidWireFormat(
                        "V1 instruction account offset overflow".into(),
                    )
                })?;
            let data_end = accounts_end.checked_add(data_len).ok_or_else(|| {
                TransportSelectionError::InvalidWireFormat(
                    "V1 instruction data offset overflow".into(),
                )
            })?;
            if data_end > tx_bytes.len().saturating_sub(signature_bytes) {
                return Err(TransportSelectionError::InvalidWireFormat(
                    "truncated V1 instruction payload".into(),
                ));
            }
            if tx_bytes[payload_offset..accounts_end]
                .iter()
                .any(|account_index| usize::from(*account_index) >= accounts_count)
            {
                return Err(TransportSelectionError::InvalidWireFormat(
                    "V1 instruction account index is outside the address table".into(),
                ));
            }
            payload_offset = data_end;
        }

        let expected_wire_end = payload_offset.checked_add(signature_bytes).ok_or_else(|| {
            TransportSelectionError::InvalidWireFormat("V1 wire length overflow".into())
        })?;
        if expected_wire_end != tx_bytes.len() {
            return Err(TransportSelectionError::InvalidWireFormat(
                "V1 transaction has trailing bytes or an inconsistent signature section".into(),
            ));
        }

        Ok((TransactionVersion::V1, accounts_count))
    } else {
        // Legacy / V0: compact-u16 signature count precedes the message.
        let (num_sigs, sig_bytes_read) = decode_compact_u16(tx_bytes)?;
        let signature_bytes = num_sigs.checked_mul(64).ok_or_else(|| {
            TransportSelectionError::InvalidWireFormat("signature section length overflow".into())
        })?;
        let msg_start = sig_bytes_read.checked_add(signature_bytes).ok_or_else(|| {
            TransportSelectionError::InvalidWireFormat("message start overflow".into())
        })?;
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

        let accounts_start = if version == TransactionVersion::V0 {
            msg_start + 1 + 3
        } else {
            msg_start + 3
        };
        let accounts_count = if tx_bytes.len() > accounts_start {
            usize::from(tx_bytes[accounts_start] & 0x7f)
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
        // Construct a structurally valid V1 envelope with one no-account
        // instruction. Any extra test length is represented as instruction data.
        let address_bytes = usize::from(num_accounts) * 32;
        let fixed_wire_len = 42 + address_bytes + 4 + 64;
        let wire_len = total_len.max(fixed_wire_len);
        let data_len = wire_len - fixed_wire_len;
        assert!(data_len <= u16::MAX as usize);

        let mut bytes = vec![0u8; wire_len];
        bytes[0] = 0x81; // V1 discriminator
        bytes[1] = 1; // num_required_signatures
        bytes[2] = 0; // num_readonly_signed
        bytes[3] = 0; // num_readonly_unsigned
        // bytes[4..8]: zero config mask; no config values
        // bytes[8..40]: recent blockhash
        bytes[40] = 1; // num_instructions
        bytes[41] = num_accounts; // num_addresses (inline accounts count)

        let instruction_header = 42 + address_bytes;
        bytes[instruction_header] = 0; // program account index
        bytes[instruction_header + 1] = 0; // instruction account count
        bytes[instruction_header + 2..instruction_header + 4]
            .copy_from_slice(&(data_len as u16).to_le_bytes());
        // The payload and trailing 64-byte signature slot are zero-filled.
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
    fn test_transport_packet_boundary_matrix_1232_1248_4095_4096_4097() {
        let cap = TransportCapability::mainnet_v1_capable("Helius-Sender");

        // 1,232 bytes: Accepted
        let tx_1232 = dummy_v1_header(28, 1232);
        let sel_1232 = select_authoritative_transport(&tx_1232, true, true, false, &cap).unwrap();
        assert_eq!(sel_1232.packet_bytes, 1232);
        assert_eq!(sel_1232.wire_mtu_limit, 4096);

        // 1,248 bytes: Accepted (Phase 17 cross-DEX transaction size)
        let tx_1248 = dummy_v1_header(28, 1248);
        let sel_1248 = select_authoritative_transport(&tx_1248, true, true, false, &cap).unwrap();
        assert_eq!(sel_1248.packet_bytes, 1248);
        assert_eq!(sel_1248.wire_mtu_limit, 4096);

        // 4,095 bytes: Accepted
        let tx_4095 = dummy_v1_header(28, 4095);
        let sel_4095 = select_authoritative_transport(&tx_4095, true, true, false, &cap).unwrap();
        assert_eq!(sel_4095.packet_bytes, 4095);
        assert_eq!(sel_4095.wire_mtu_limit, 4096);

        // 4,096 bytes: Accepted (exact MTU limit)
        let tx_4096 = dummy_v1_header(28, 4096);
        let sel_4096 = select_authoritative_transport(&tx_4096, true, true, false, &cap).unwrap();
        assert_eq!(sel_4096.packet_bytes, 4096);
        assert_eq!(sel_4096.wire_mtu_limit, 4096);

        // 4,097 bytes: Rejected (exceeds MTU limit by 1 byte)
        let tx_4097 = dummy_v1_header(28, 4097);
        let err_4097 =
            select_authoritative_transport(&tx_4097, true, true, false, &cap).unwrap_err();
        assert_eq!(
            err_4097,
            TransportSelectionError::OversizedTransaction {
                size: 4097,
                limit: 4096,
                version: TransactionVersion::V1,
            }
        );
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

    #[test]
    fn test_v1_parser_rejects_truncated_address_table() {
        let mut tx = dummy_v1_header(32, 1_500);
        tx.truncate(42 + 31 * 32);
        assert!(matches!(
            parse_transaction_wire_header(&tx),
            Err(TransportSelectionError::InvalidWireFormat(_))
        ));
    }

    #[test]
    fn test_v1_parser_rejects_partial_priority_fee_mask() {
        let mut tx = dummy_v1_header(3, 250);
        tx[4..8].copy_from_slice(&1u32.to_le_bytes());
        assert!(matches!(
            parse_transaction_wire_header(&tx),
            Err(TransportSelectionError::InvalidWireFormat(_))
        ));
    }

    #[test]
    fn test_v1_parser_rejects_out_of_range_instruction_account_index() {
        let mut tx = dummy_v1_header(3, 208);
        let instruction_header = 42 + 3 * 32;
        tx[instruction_header + 1] = 1; // one instruction account
        tx[instruction_header + 2..instruction_header + 4].copy_from_slice(&1u16.to_le_bytes()); // one data byte
        let payload_offset = instruction_header + 4;
        tx[payload_offset] = 3; // valid indexes are 0..=2
        assert!(matches!(
            parse_transaction_wire_header(&tx),
            Err(TransportSelectionError::InvalidWireFormat(_))
        ));
    }

    #[test]
    fn test_parse_real_compiled_v1_wire_header_regression() {
        use crate::{
            AccountMeta, AdaptiveTxCompiler, CompiledInstruction, KeypairSigner, V1ConfigParams,
        };
        use solana_pubkey::Pubkey;

        let signer = KeypairSigner::new_random();
        let target_pubkey = Pubkey::new_unique();
        let program_id = Pubkey::new_unique();

        let ix = CompiledInstruction {
            program_id,
            accounts: vec![
                AccountMeta::new(signer.pubkey(), true),
                AccountMeta::new(target_pubkey, false),
            ],
            data: vec![1, 2, 3, 4],
        };

        let config = V1ConfigParams {
            priority_fee_lamports: Some(5_000),
            compute_unit_limit: Some(250_000),
            loaded_accounts_data_size_limit: Some(67_108_864),
            heap_size: None,
        };

        let compiled = AdaptiveTxCompiler::compile_v1_only(&signer, &[ix], [9u8; 32], config)
            .expect("compiling valid V1 transaction must succeed");

        let (version, count) = parse_transaction_wire_header(&compiled.wire_bytes)
            .expect("must successfully parse wire header of real V1 transaction");

        assert_eq!(version, TransactionVersion::V1);
        // signer + target + program = 3 unique accounts
        assert_eq!(count, 3);

        let cap = TransportCapability::mainnet_v1_capable("Helius-Sender");
        let selected =
            select_authoritative_transport(&compiled.wire_bytes, true, true, false, &cap)
                .expect("selection must succeed for real compiled V1 transaction");

        assert_eq!(selected.kind, TransportKind::SwqosDirect);
        assert_eq!(selected.tx_version, TransactionVersion::V1);
        assert_eq!(selected.packet_bytes, compiled.wire_bytes.len());
        assert_eq!(selected.wire_mtu_limit, 4096);
        assert!(selected.is_v1);
    }
}
