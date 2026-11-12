//! Adaptive Transaction Compiler supporting Solana Legacy, Version 0 (v0 + ALT),
//! and Version 1 (v1 SIMD-0160, SIMD-0296, SIMD-0385) transactions.
//!
//! # Architecture & Standards
//! - **Legacy**: Compact-u16 signature array at head, legacy message with 3-byte header,
//!   inline accounts, recent blockhash, and compact instructions. Ceiling: 1,232 bytes MTU.
//! - **Version 0 (v0)**: Compact-u16 signature array at head, version byte 0x80, 3-byte header,
//!   static accounts, blockhash, compact instructions, and Address Lookup Tables (ALTs). Ceiling: 1,232 bytes MTU.
//! - **Version 1 (v1)**: SIMD-0385 wire format. Version discriminator 0x81 at byte 0, 3-byte header,
//!   32-bit `TransactionConfigMask`, 32-byte blockhash, `u8` num_instructions, `u8` num_addresses,
//!   inline 32-byte accounts (no ALTs needed, up to 64 accounts), config values (embedded compute unit limit,
//!   loaded accounts data size limit, priority fee), fixed 4-byte instruction headers, contiguous instruction payloads,
//!   and trailing Ed25519 signatures at the packet tail. Ceiling: 4,096 bytes MTU (SIMD-0296).

use crate::error::TxError;
use crate::{AddressLookupTable, CompiledInstruction, KeypairSigner, TxBuilder};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use solana_pubkey::Pubkey;
use std::collections::{BTreeMap, HashMap};

/// Maximum MTU payload for legacy and v0 transactions (constrained by IPv6 MTU).
pub const SOLANA_PACKET_LEGACY_MTU: usize = 1232;

/// Maximum MTU payload for v1 transactions (SIMD-0296: 4 KiB memory page).
pub const SOLANA_PACKET_V1_MTU: usize = 4096;

/// Maximum static instruction count for v1 transactions (SIMD-0160).
pub const MAX_V1_INSTRUCTIONS: usize = 64;

/// Maximum static address count for v1 transactions (SIMD-0385).
pub const MAX_V1_ADDRESSES: usize = 64;

/// Version discriminator byte for Solana Transaction v0.
pub const V0_VERSION_DISCRIMINATOR: u8 = 0x80;

/// Version discriminator byte for Solana Transaction v1 (SIMD-0385).
pub const V1_VERSION_DISCRIMINATOR: u8 = 0x81;

/// Configuration mask bit 0 and 1: Total priority fee in lamports (8 bytes, little-endian u64).
pub const CONFIG_MASK_PRIORITY_FEE: u32 = 0x03;

/// Configuration mask bit 2: Compute unit limit (4 bytes, little-endian u32).
pub const CONFIG_MASK_CU_LIMIT: u32 = 0x04;

/// Configuration mask bit 3: Loaded accounts data size limit (4 bytes, little-endian u32).
pub const CONFIG_MASK_LOADED_DATA_LIMIT: u32 = 0x08;

/// Configuration mask bit 4: Requested heap size (4 bytes, little-endian u32).
pub const CONFIG_MASK_HEAP_SIZE: u32 = 0x10;

/// Solana transaction format versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TransactionVersion {
    /// Unversioned legacy Solana transaction (max 1,232 bytes, no ALTs).
    Legacy,
    /// Solana Version 0 versioned transaction with Address Lookup Tables (max 1,232 bytes).
    V0,
    /// Solana Version 1 transaction with inline accounts, native resource config, and trailing signatures (max 4,096 bytes).
    V1,
}

impl TransactionVersion {
    /// Numerical `maxSupportedTransactionVersion` value passed to Solana RPC.
    /// The RPC wire contract is: legacy is unversioned, v0 is `0`, and v1 is `1`.
    /// `None` denotes legacy unversioned transactions.
    pub fn as_rpc_version(&self) -> Option<u8> {
        match self {
            Self::Legacy => None,
            Self::V0 => Some(0),
            Self::V1 => Some(1),
        }
    }

    /// Maximum wire format size in bytes.
    pub fn max_mtu(&self) -> usize {
        match self {
            Self::Legacy | Self::V0 => SOLANA_PACKET_LEGACY_MTU,
            Self::V1 => SOLANA_PACKET_V1_MTU,
        }
    }
}

/// Compilation capabilities and constraints supported by the target RPC, validator, or private relay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompilerCapabilities {
    /// Whether the downstream executor / RPC supports unversioned legacy transactions.
    pub supports_legacy: bool,
    /// Whether the downstream executor / RPC supports v0 transactions.
    pub supports_v0: bool,
    /// Whether the downstream executor / RPC supports v1 4096-byte transactions (SIMD-0296 / SIMD-0385).
    pub supports_v1: bool,
    /// Whether Address Lookup Tables are active and resolvable.
    pub supports_alt: bool,
    /// Maximum allowed transaction size in bytes for the downstream channel.
    pub max_tx_bytes: usize,
}

impl Default for CompilerCapabilities {
    fn default() -> Self {
        Self::v1_capable()
    }
}

impl CompilerCapabilities {
    /// Standard Solana Mainnet / Jito capabilities: supports Legacy and V0 with ALTs, max 1,232 bytes.
    pub fn v0_standard() -> Self {
        Self {
            supports_legacy: true,
            supports_v0: true,
            supports_v1: false,
            supports_alt: true,
            max_tx_bytes: SOLANA_PACKET_LEGACY_MTU,
        }
    }

    /// V1-only release profile: V1 enabled, Legacy/V0/ALT deliberately disabled.
    pub fn v1_capable() -> Self {
        Self {
            supports_legacy: false,
            supports_v0: false,
            supports_v1: true,
            supports_alt: false,
            max_tx_bytes: SOLANA_PACKET_V1_MTU,
        }
    }

    /// Legacy-only environment.
    pub fn legacy_only() -> Self {
        Self {
            supports_legacy: true,
            supports_v0: false,
            supports_v1: false,
            supports_alt: false,
            max_tx_bytes: SOLANA_PACKET_LEGACY_MTU,
        }
    }

    /// Highest version accepted by RPC requests: v1 = 1, v0 = 0.
    pub fn max_supported_transaction_version(&self) -> Option<u8> {
        if self.supports_v1 {
            Some(1)
        } else if self.supports_v0 {
            Some(0)
        } else {
            None
        }
    }
}

/// Native resource configuration parameters for Solana Transaction v1 (SIMD-0385).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct V1ConfigParams {
    /// Total priority fee in lamports (replaces ComputeBudget set_compute_unit_price).
    pub priority_fee_lamports: Option<u64>,
    /// Explicit compute unit limit (replaces ComputeBudget set_compute_unit_limit).
    pub compute_unit_limit: Option<u32>,
    /// Maximum loaded accounts data size limit in bytes.
    pub loaded_accounts_data_size_limit: Option<u32>,
    /// Heap size request in bytes.
    pub heap_size: Option<u32>,
}

impl V1ConfigParams {
    /// Compute the 32-bit config bitmask and contiguous 4-byte config value chunks.
    pub fn compute_mask_and_values(&self) -> (u32, Vec<u8>) {
        let mut mask: u32 = 0;
        let mut values = Vec::new();

        if let Some(fee) = self.priority_fee_lamports {
            mask |= CONFIG_MASK_PRIORITY_FEE;
            values.extend_from_slice(&fee.to_le_bytes()); // 8 bytes (two 4-byte entries)
        }
        if let Some(cu) = self.compute_unit_limit {
            mask |= CONFIG_MASK_CU_LIMIT;
            values.extend_from_slice(&cu.to_le_bytes()); // 4 bytes
        }
        if let Some(loaded) = self.loaded_accounts_data_size_limit {
            mask |= CONFIG_MASK_LOADED_DATA_LIMIT;
            values.extend_from_slice(&loaded.to_le_bytes()); // 4 bytes
        }
        if let Some(heap) = self.heap_size {
            mask |= CONFIG_MASK_HEAP_SIZE;
            values.extend_from_slice(&heap.to_le_bytes()); // 4 bytes
        }

        (mask, values)
    }

    /// Returns true if any native resource configuration is populated.
    pub fn is_active(&self) -> bool {
        self.priority_fee_lamports.is_some()
            || self.compute_unit_limit.is_some()
            || self.loaded_accounts_data_size_limit.is_some()
            || self.heap_size.is_some()
    }
}

/// Concrete rationale explaining why a specific transaction format was selected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FormatSelectionReason {
    /// Fits within 1,232 bytes as a standard legacy transaction without requiring ALTs.
    SmallPayloadLegacy {
        accounts_count: usize,
        wire_size: usize,
    },
    /// Selected V0 with Address Lookup Tables to compress > 35 accounts into <= 1,232 bytes MTU.
    AltRequiredForMtu {
        accounts_count: usize,
        table_count: usize,
        wire_size: usize,
    },
    /// Default V0 format chosen for standard multi-hop execution.
    V0Default {
        accounts_count: usize,
        wire_size: usize,
    },
    /// Payload exceeds 1,232 bytes or requires native resource headers; compiled as V1 (up to 4,096 bytes).
    LargePayloadV1 {
        accounts_count: usize,
        wire_size: usize,
    },
    /// Provider restricted format from requested version to a supported fallback.
    ProviderRestricted {
        requested: TransactionVersion,
        selected: TransactionVersion,
        reason: String,
    },
    /// Explicitly requested by caller configuration.
    ExplicitRequested(TransactionVersion),
}

/// Fully compiled, signed, and validated Solana transaction produced adaptively.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledAdaptiveTransaction {
    /// Format version of the compiled transaction.
    pub version: TransactionVersion,
    /// Wire bytes ready for RPC simulateTransaction, Jito, or gossip broadcast.
    pub wire_bytes: Vec<u8>,
    /// Cryptographic 64-byte Ed25519 signatures contained in the transaction.
    pub signatures: Vec<[u8; 64]>,
    /// Raw serialized Message byte buffer signed by the participants.
    pub message_bytes: Vec<u8>,
    /// Justification for why this wire format was chosen.
    pub selection_reason: FormatSelectionReason,
}

impl CompiledAdaptiveTransaction {
    /// Encode serialized transaction wire bytes into standard Base64 string for RPC calls.
    pub fn to_base64(&self) -> String {
        BASE64_STANDARD.encode(&self.wire_bytes)
    }

    /// Base58 encoded string of the primary payer signature.
    pub fn signature(&self) -> Option<String> {
        self.signatures
            .first()
            .map(|sig| bs58::encode(sig).into_string())
    }

    /// RPC option `maxSupportedTransactionVersion` required to deserialize this transaction.
    pub fn max_supported_transaction_version(&self) -> Option<u8> {
        self.version.as_rpc_version()
    }

    /// Wire size in bytes.
    pub fn wire_size(&self) -> usize {
        self.wire_bytes.len()
    }

    /// Cryptographically verify the Ed25519 signatures of the transaction against its message buffer.
    pub fn verify_signatures(&self, signers: &[Pubkey]) -> Result<bool, TxError> {
        if signers.len() != self.signatures.len() {
            return Err(TxError::SigningError(format!(
                "Signer count mismatch: expected {} keys, found {} signatures",
                signers.len(),
                self.signatures.len()
            )));
        }

        for (pubkey, sig_bytes) in signers.iter().zip(self.signatures.iter()) {
            let pubkey_bytes = pubkey.to_bytes();
            let verifying_key = VerifyingKey::from_bytes(&pubkey_bytes)
                .map_err(|e| TxError::SigningError(format!("Invalid public key {pubkey}: {e}")))?;
            let signature = Signature::from_bytes(sig_bytes);
            verifying_key
                .verify(&self.message_bytes, &signature)
                .map_err(|e| {
                    TxError::SigningError(format!(
                        "Cryptographic verification failed for signer {pubkey}: {e}"
                    ))
                })?;
        }

        Ok(true)
    }

    /// Determine transaction format version by inspecting raw wire bytes.
    pub fn parse_version_from_wire(wire_bytes: &[u8]) -> Result<TransactionVersion, TxError> {
        if wire_bytes.is_empty() {
            return Err(TxError::Serialization(
                "Empty transaction wire bytes".into(),
            ));
        }

        // V1 starts immediately with version discriminator 0x81 at byte 0
        if wire_bytes[0] == V1_VERSION_DISCRIMINATOR {
            return Ok(TransactionVersion::V1);
        }

        // Legacy and V0 begin with compact-u16 signature count (typically 1..64)
        // For a single-signature transaction: byte 0 = 0x01, bytes 1..65 = signature, byte 65 = message start
        if wire_bytes.len() < 66 {
            return Err(TxError::Serialization(
                "Transaction wire bytes too short".into(),
            ));
        }

        let (sig_count, sig_offset) = decode_compact_u16(wire_bytes)?;
        let msg_start = sig_offset + sig_count as usize * 64;
        if wire_bytes.len() <= msg_start {
            return Err(TxError::Serialization("Truncated message bytes".into()));
        }

        if wire_bytes[msg_start] == V0_VERSION_DISCRIMINATOR {
            Ok(TransactionVersion::V0)
        } else if wire_bytes[msg_start] < 0x80 {
            Ok(TransactionVersion::Legacy)
        } else {
            Err(TxError::UnsupportedTransactionVersion(
                wire_bytes[msg_start],
            ))
        }
    }
}

/// Extract primary transaction signature from any Solana wire format (Legacy, V0, or V1).
pub fn extract_adaptive_transaction_signature(tx_bytes: &[u8]) -> Option<String> {
    if tx_bytes.is_empty() {
        return None;
    }

    // V1 layout: discriminator 0x81 at byte 0, signatures at the tail
    if tx_bytes[0] == V1_VERSION_DISCRIMINATOR {
        if tx_bytes.len() < 1 + 3 + 4 + 32 + 1 + 1 + 64 {
            return None;
        }
        let num_sigs = tx_bytes[1] as usize;
        if num_sigs == 0 || tx_bytes.len() < num_sigs * 64 {
            return None;
        }
        let sig_start = tx_bytes.len() - num_sigs * 64;
        return Some(bs58::encode(&tx_bytes[sig_start..sig_start + 64]).into_string());
    }

    // Legacy / V0 layout: compact-u16 signatures count at head
    let (sig_count, sig_offset) = decode_compact_u16(tx_bytes).ok()?;
    if sig_count >= 1 && tx_bytes.len() >= sig_offset + 64 {
        Some(bs58::encode(&tx_bytes[sig_offset..sig_offset + 64]).into_string())
    } else {
        None
    }
}

/// Map raw transaction wire bytes to the `maxSupportedTransactionVersion`
/// RPC parameter the server needs in order to deserialize them
/// (`0 = v0, 1 = v1`). `Ok(None)` means the transaction is legacy.
///
/// Pre-flight `simulateTransaction` calls must pass this value: Solana RPC
/// refuses to deserialize versioned transactions above the configured maximum
/// (the default allows legacy only), so without it every v0/v1 live pre-flight
/// fails with `UnsupportedTransactionVersion`.
pub fn rpc_max_supported_version_from_wire(wire_bytes: &[u8]) -> Result<Option<u8>, TxError> {
    CompiledAdaptiveTransaction::parse_version_from_wire(wire_bytes)
        .map(|version| version.as_rpc_version())
}

/// Adaptive transaction compiler and format selector.
pub struct AdaptiveTxCompiler;

impl AdaptiveTxCompiler {
    /// Compile a list of instructions into a Solana Transaction v1 Message byte buffer (SIMD-0385).
    ///
    /// # Wire Layout:
    /// - `[0]`: `0x81` (VersionByte)
    /// - `[1..4]`: `[num_required_signatures, num_readonly_signed_accounts, num_readonly_unsigned_accounts]`
    /// - `[4..8]`: `TransactionConfigMask: u32` (little-endian)
    /// - `[8..40]`: `LifetimeSpecifier: [u8; 32]` (recent blockhash)
    /// - `[40]`: `NumInstructions: u8` (<= 64)
    /// - `[41]`: `NumAddresses: u8` (<= 64)
    /// - `[42..]`: Inline `Addresses: [[u8; 32]; NumAddresses]`
    /// - `[...]`: `ConfigValues: [[u8; 4]]` (values for set bits in TransactionConfigMask)
    /// - `[...]`: `InstructionHeaders: [(u8, u8, u16); NumInstructions]` (fixed 4-byte header per ix)
    /// - `[...]`: Contiguous `InstructionPayloads: [InstructionPayload]`
    pub fn compile_v1_message(
        payer: Pubkey,
        instructions: &[CompiledInstruction],
        recent_blockhash: [u8; 32],
        config: V1ConfigParams,
    ) -> Result<Vec<u8>, TxError> {
        // V1 resource limits live in the signed message config. ComputeBudget
        // instructions are successful no-ops in V1, still consuming an
        // instruction slot and 150 CU, so strip them before compiling.
        let compute_budget_program = Pubkey::new_from_array(crate::COMPUTE_BUDGET_PROGRAM_ID);
        let v1_instructions: Vec<CompiledInstruction> = instructions
            .iter()
            .filter(|ix| ix.program_id != compute_budget_program)
            .cloned()
            .collect();
        let instructions = v1_instructions.as_slice();
        // SIMD-0160: Sanitization - max 64 instructions per transaction
        if instructions.len() > MAX_V1_INSTRUCTIONS {
            return Err(TxError::SanitizationError(format!(
                "SIMD-0160 instruction limit exceeded: {} > {}",
                instructions.len(),
                MAX_V1_INSTRUCTIONS
            )));
        }

        // Account ordering & deduplication:
        // Payer is always writable signer at index 0.
        let mut account_map: BTreeMap<Pubkey, (bool, bool)> = BTreeMap::new();
        account_map.insert(payer, (true, true));

        for ix in instructions {
            for acc in &ix.accounts {
                let entry = account_map.entry(acc.pubkey).or_insert((false, false));
                entry.0 |= acc.is_signer;
                entry.1 |= acc.is_writable;
            }
            // Program accounts are non-signers, readonly
            account_map.entry(ix.program_id).or_insert((false, false));
        }

        let mut writable_signers = Vec::new();
        let mut readonly_signers = Vec::new();
        let mut writable_non_signers = Vec::new();
        let mut readonly_non_signers = Vec::new();

        writable_signers.push(payer);

        for (pubkey, (is_signer, is_writable)) in account_map {
            if pubkey == payer {
                continue;
            }
            if is_signer {
                if is_writable {
                    writable_signers.push(pubkey);
                } else {
                    readonly_signers.push(pubkey);
                }
            } else if is_writable {
                writable_non_signers.push(pubkey);
            } else {
                readonly_non_signers.push(pubkey);
            }
        }

        let num_required_signatures = (writable_signers.len() + readonly_signers.len()) as u8;
        let num_readonly_signed_accounts = readonly_signers.len() as u8;
        let num_readonly_unsigned_accounts = readonly_non_signers.len() as u8;

        let mut all_accounts = Vec::new();
        all_accounts.extend(writable_signers);
        all_accounts.extend(readonly_signers);
        all_accounts.extend(writable_non_signers);
        all_accounts.extend(readonly_non_signers);

        // SIMD-0385: Sanitization - max 64 accounts
        if all_accounts.len() > MAX_V1_ADDRESSES {
            return Err(TxError::SanitizationError(format!(
                "SIMD-0385 address limit exceeded: {} > {}",
                all_accounts.len(),
                MAX_V1_ADDRESSES
            )));
        }
        if all_accounts.len()
            < (num_required_signatures as usize + num_readonly_unsigned_accounts as usize)
        {
            return Err(TxError::SanitizationError(
                "Invalid account counts in header".into(),
            ));
        }

        let key_to_index: HashMap<Pubkey, u8> = all_accounts
            .iter()
            .enumerate()
            .map(|(idx, &pk)| (pk, idx as u8))
            .collect();

        let (config_mask, config_values) = config.compute_mask_and_values();

        let mut message = Vec::with_capacity(512);

        // 1. Version Discriminator (0x81)
        message.push(V1_VERSION_DISCRIMINATOR);

        // 2. Legacy Header (3 bytes)
        message.push(num_required_signatures);
        message.push(num_readonly_signed_accounts);
        message.push(num_readonly_unsigned_accounts);

        // 3. TransactionConfigMask (4 bytes, little-endian u32)
        message.extend_from_slice(&config_mask.to_le_bytes());

        // 4. LifetimeSpecifier (32 bytes recent blockhash)
        message.extend_from_slice(&recent_blockhash);

        // 5. NumInstructions (u8)
        message.push(instructions.len() as u8);

        // 6. NumAddresses (u8)
        message.push(all_accounts.len() as u8);

        // 7. Inline Addresses array (NumAddresses * 32 bytes)
        for pk in &all_accounts {
            message.extend_from_slice(pk.as_ref());
        }

        // 8. ConfigValues array (dynamically sized based on popcount of config mask)
        message.extend_from_slice(&config_values);

        // 9. InstructionHeaders array (NumInstructions * 4 bytes)
        // Each entry: program_account_index (u8), num_instruction_accounts (u8), num_instruction_data_bytes (u16 LE)
        for ix in instructions {
            let prog_idx = *key_to_index.get(&ix.program_id).ok_or_else(|| {
                TxError::InstructionError(format!(
                    "Program ID {} missing from key map",
                    ix.program_id
                ))
            })?;
            let acc_count = ix.accounts.len();
            if acc_count > 255 {
                return Err(TxError::SanitizationError(format!(
                    "Instruction accounts exceed 255: {acc_count}"
                )));
            }
            let data_len = ix.data.len();
            if data_len > 10_240 {
                return Err(TxError::SanitizationError(format!(
                    "Instruction data exceeds Solana 10 KiB limit: {data_len}"
                )));
            }
            if data_len > u16::MAX as usize {
                return Err(TxError::SanitizationError(format!(
                    "Instruction data exceeds u16 limit: {data_len}"
                )));
            }

            message.push(prog_idx);
            message.push(acc_count as u8);
            message.extend_from_slice(&(data_len as u16).to_le_bytes());
        }

        // 10. InstructionPayloads contiguous array
        for ix in instructions {
            for acc in &ix.accounts {
                let acc_idx = *key_to_index.get(&acc.pubkey).ok_or_else(|| {
                    TxError::InstructionError(format!(
                        "Account {} missing from key map",
                        acc.pubkey
                    ))
                })?;
                message.push(acc_idx);
            }
            message.extend_from_slice(&ix.data);
        }

        if message.len() > SOLANA_PACKET_V1_MTU {
            return Err(TxError::TransactionTooLarge {
                size: message.len(),
                limit: SOLANA_PACKET_V1_MTU,
            });
        }

        Ok(message)
    }

    /// Compile and sign V1 only. This path never considers legacy, V0, or ALTs.
    /// All account addresses (including Meteora DLMM BinArray accounts) are inline.
    pub fn compile_v1_only(
        signer: &KeypairSigner,
        instructions: &[CompiledInstruction],
        recent_blockhash: [u8; 32],
        config: V1ConfigParams,
    ) -> Result<CompiledAdaptiveTransaction, TxError> {
        let unique_accounts = count_unique_accounts(signer.pubkey(), instructions);
        if unique_accounts > MAX_V1_ADDRESSES {
            return Err(TxError::SanitizationError(format!(
                "V1-only inline account limit exceeded: {unique_accounts} > {MAX_V1_ADDRESSES}; reduce route/bin-array footprint"
            )));
        }
        let tx = Self::build_signed_v1_transaction(
            signer,
            instructions,
            recent_blockhash,
            config,
            FormatSelectionReason::ExplicitRequested(TransactionVersion::V1),
        )?;
        if tx.wire_size() > SOLANA_PACKET_V1_MTU {
            return Err(TxError::TransactionTooLarge {
                size: tx.wire_size(),
                limit: SOLANA_PACKET_V1_MTU,
            });
        }
        Ok(tx)
    }

    /// Build an unsigned V1 wire envelope for non-broadcast RPC simulation.
    /// Signature slots are zero-filled and RPC simulation MUST use `sigVerify: false`.
    /// This function never loads or uses private key material.
    pub fn build_unsigned_v1_simulation_transaction(
        payer: Pubkey,
        instructions: &[CompiledInstruction],
        recent_blockhash: [u8; 32],
        config: V1ConfigParams,
    ) -> Result<Vec<u8>, TxError> {
        if config
            .compute_unit_limit
            .is_none_or(|v| v == 0 || v > 1_400_000)
        {
            return Err(TxError::SanitizationError(
                "V1 simulation requires compute-unit limit in 1..=1,400,000".into(),
            ));
        }
        if config
            .loaded_accounts_data_size_limit
            .is_none_or(|v| v == 0 || v > 67_108_864)
        {
            return Err(TxError::SanitizationError(
                "V1 simulation requires loaded-account data limit in 1..=64 MiB".into(),
            ));
        }
        for ix in instructions {
            for account in &ix.accounts {
                if account.is_signer && account.pubkey != payer {
                    return Err(TxError::SanitizationError(format!(
                        "Shadow V1 simulation cannot assert an unavailable secondary signer: {}",
                        account.pubkey
                    )));
                }
            }
        }
        let message = Self::compile_v1_message(payer, instructions, recent_blockhash, config)?;
        let signature_count = *message.get(1).ok_or_else(|| {
            TxError::Serialization("V1 message is missing its signature header".into())
        })? as usize;
        if signature_count != 1 {
            return Err(TxError::SanitizationError(format!(
                "Shadow V1 simulation expects exactly one payer signature slot, got {signature_count}"
            )));
        }
        let wire_size = message
            .len()
            .saturating_add(signature_count.saturating_mul(64));
        if wire_size > SOLANA_PACKET_V1_MTU {
            return Err(TxError::TransactionTooLarge {
                size: wire_size,
                limit: SOLANA_PACKET_V1_MTU,
            });
        }
        let mut wire = Vec::with_capacity(wire_size);
        wire.extend_from_slice(&message);
        wire.resize(wire_size, 0);
        Ok(wire)
    }

    /// Construct and sign a Solana Transaction v1 (SIMD-0385) with trailing signatures.
    pub fn build_signed_v1_transaction(
        signer: &KeypairSigner,
        instructions: &[CompiledInstruction],
        recent_blockhash: [u8; 32],
        config: V1ConfigParams,
        reason: FormatSelectionReason,
    ) -> Result<CompiledAdaptiveTransaction, TxError> {
        // A signed V1 transaction with either field omitted is guaranteed to
        // fail at runtime (zero CU / zero loaded-account bytes). Keep the low-level
        // message encoder permissive for header differential tests, but never sign
        // an invalid V1 transaction.
        if config
            .compute_unit_limit
            .is_none_or(|v| v == 0 || v > 1_400_000)
        {
            return Err(TxError::SanitizationError(
                "V1 requires an explicit compute-unit limit in 1..=1,400,000".into(),
            ));
        }
        if config
            .loaded_accounts_data_size_limit
            .is_none_or(|v| v == 0 || v > 67_108_864)
        {
            return Err(TxError::SanitizationError(
                "V1 requires an explicit loaded-accounts data limit in 1..=64 MiB".into(),
            ));
        }
        if config
            .heap_size
            .is_some_and(|v| !(32_768..=262_144).contains(&v) || v % 1_024 != 0)
        {
            return Err(TxError::SanitizationError(
                "V1 heap request must be a 1 KiB multiple in 32..=256 KiB".into(),
            ));
        }
        let message_bytes =
            Self::compile_v1_message(signer.pubkey(), instructions, recent_blockhash, config)?;
        if message_bytes.get(1).copied() != Some(1) {
            return Err(TxError::SanitizationError(
                "V1 transaction requires multiple signers but this compiler has exactly one signer"
                    .into(),
            ));
        }
        let signature = signer.sign(&message_bytes);

        // SIMD-0385 Wire format: Message is at the head, trailing signatures are appended at the tail.
        let mut wire_bytes = Vec::with_capacity(message_bytes.len() + 64);
        wire_bytes.extend_from_slice(&message_bytes);
        wire_bytes.extend_from_slice(&signature);

        if wire_bytes.len() > SOLANA_PACKET_V1_MTU {
            return Err(TxError::TransactionTooLarge {
                size: wire_bytes.len(),
                limit: SOLANA_PACKET_V1_MTU,
            });
        }

        Ok(CompiledAdaptiveTransaction {
            version: TransactionVersion::V1,
            wire_bytes,
            signatures: vec![signature],
            message_bytes,
            selection_reason: reason,
        })
    }

    /// Construct and sign a Solana Legacy transaction.
    pub fn build_signed_legacy_transaction(
        signer: &KeypairSigner,
        instructions: &[CompiledInstruction],
        recent_blockhash: [u8; 32],
        reason: FormatSelectionReason,
    ) -> Result<CompiledAdaptiveTransaction, TxError> {
        let message_bytes =
            TxBuilder::compile_message(signer.pubkey(), instructions, recent_blockhash)?;
        let signature = signer.sign(&message_bytes);

        let mut wire_bytes = Vec::with_capacity(1 + 64 + message_bytes.len());
        TxBuilder::encode_compact_u16(1, &mut wire_bytes);
        wire_bytes.extend_from_slice(&signature);
        wire_bytes.extend_from_slice(&message_bytes);

        if wire_bytes.len() > SOLANA_PACKET_LEGACY_MTU {
            return Err(TxError::TransactionTooLarge {
                size: wire_bytes.len(),
                limit: SOLANA_PACKET_LEGACY_MTU,
            });
        }

        Ok(CompiledAdaptiveTransaction {
            version: TransactionVersion::Legacy,
            wire_bytes,
            signatures: vec![signature],
            message_bytes,
            selection_reason: reason,
        })
    }

    /// Construct and sign a Solana Version 0 (v0) transaction with optional Address Lookup Tables.
    pub fn build_signed_v0_transaction(
        signer: &KeypairSigner,
        instructions: &[CompiledInstruction],
        recent_blockhash: [u8; 32],
        lookup_tables: &[AddressLookupTable],
        reason: FormatSelectionReason,
    ) -> Result<CompiledAdaptiveTransaction, TxError> {
        let message_bytes = TxBuilder::compile_v0_message(
            signer.pubkey(),
            instructions,
            recent_blockhash,
            lookup_tables,
        )
        .map_err(|e| TxError::AltCompilationError(e.to_string()))?;

        let signature = signer.sign(&message_bytes);

        let mut wire_bytes = Vec::with_capacity(1 + 64 + message_bytes.len());
        TxBuilder::encode_compact_u16(1, &mut wire_bytes);
        wire_bytes.extend_from_slice(&signature);
        wire_bytes.extend_from_slice(&message_bytes);

        if wire_bytes.len() > SOLANA_PACKET_LEGACY_MTU {
            return Err(TxError::TransactionTooLarge {
                size: wire_bytes.len(),
                limit: SOLANA_PACKET_LEGACY_MTU,
            });
        }

        Ok(CompiledAdaptiveTransaction {
            version: TransactionVersion::V0,
            wire_bytes,
            signatures: vec![signature],
            message_bytes,
            selection_reason: reason,
        })
    }

    /// Transaction compiler. When the capability profile supports V1, this
    /// method is V1-only: it never downgrades to Legacy/V0 and never uses ALTs.
    /// A V1 resource config is mandatory. Legacy/V0 compatibility compilation
    /// is reachable only when the caller explicitly supplies a non-V1 profile.
    pub fn compile_adaptive(
        signer: &KeypairSigner,
        instructions: &[CompiledInstruction],
        recent_blockhash: [u8; 32],
        lookup_tables: &[AddressLookupTable],
        v1_config: Option<V1ConfigParams>,
        capabilities: CompilerCapabilities,
        preferred_version: Option<TransactionVersion>,
    ) -> Result<CompiledAdaptiveTransaction, TxError> {
        let unique_accounts_count = count_unique_accounts(signer.pubkey(), instructions);

        // Handle explicit preference
        if let Some(pref) = preferred_version {
            match pref {
                TransactionVersion::V1 => {
                    if !capabilities.supports_v1 {
                        return Err(TxError::UnsupportedTransactionVersion(1));
                    }
                    let config = v1_config.ok_or_else(|| {
                        TxError::SanitizationError(
                            "V1-only compilation requires explicit resource configuration".into(),
                        )
                    })?;
                    return Self::compile_v1_only(signer, instructions, recent_blockhash, config);
                }
                TransactionVersion::V0 => {
                    if capabilities.supports_v1 {
                        return Err(TxError::UnsupportedTransactionVersion(0));
                    }
                    if capabilities.supports_v0 {
                        let reason = if !lookup_tables.is_empty() {
                            FormatSelectionReason::AltRequiredForMtu {
                                accounts_count: unique_accounts_count,
                                table_count: lookup_tables.len(),
                                wire_size: 0,
                            }
                        } else {
                            FormatSelectionReason::ExplicitRequested(TransactionVersion::V0)
                        };
                        return Self::build_signed_v0_transaction(
                            signer,
                            instructions,
                            recent_blockhash,
                            lookup_tables,
                            reason,
                        );
                    }
                }
                TransactionVersion::Legacy => {
                    if capabilities.supports_v1 {
                        return Err(TxError::InvalidInput(
                            "V1-only profile rejects unversioned Legacy transactions".into(),
                        ));
                    }
                    if capabilities.supports_legacy {
                        return Self::build_signed_legacy_transaction(
                            signer,
                            instructions,
                            recent_blockhash,
                            FormatSelectionReason::ExplicitRequested(TransactionVersion::Legacy),
                        );
                    }
                }
            }
        }

        // V1-only policy for V1-capable profiles: never downgrade a small
        // payload to Legacy/V0 and never compress addresses through ALTs.
        if capabilities.supports_v1 {
            let config = v1_config.ok_or_else(|| {
                TxError::SanitizationError(
                    "V1-capable profile requires explicit V1 resource configuration".into(),
                )
            })?;
            let mut tx = Self::compile_v1_only(signer, instructions, recent_blockhash, config)?;
            let legacy_wire_size =
                TxBuilder::compile_message(signer.pubkey(), instructions, recent_blockhash)
                    .map(|msg| 1 + 64 + msg.len())
                    .unwrap_or(usize::MAX);
            if legacy_wire_size > SOLANA_PACKET_LEGACY_MTU {
                tx.selection_reason = FormatSelectionReason::LargePayloadV1 {
                    accounts_count: unique_accounts_count,
                    wire_size: tx.wire_size(),
                };
            }
            return Ok(tx);
        }

        // Fast-path evaluation: Legacy/V0 compatibility is reachable only when
        // the caller explicitly declares that V1 is unavailable.
        let test_legacy_msg =
            TxBuilder::compile_message(signer.pubkey(), instructions, recent_blockhash);

        match test_legacy_msg {
            Ok(legacy_msg) => {
                let legacy_wire_size = 1 + 64 + legacy_msg.len();

                // If fits in legacy MTU without ALTs
                if legacy_wire_size <= SOLANA_PACKET_LEGACY_MTU
                    && legacy_wire_size <= capabilities.max_tx_bytes
                {
                    if capabilities.supports_v0 {
                        // V0 is the canonical format across Solana Mainnet validator pipelines
                        let v0_res = Self::build_signed_v0_transaction(
                            signer,
                            instructions,
                            recent_blockhash,
                            lookup_tables,
                            FormatSelectionReason::V0Default {
                                accounts_count: unique_accounts_count,
                                wire_size: legacy_wire_size,
                            },
                        );
                        if let Ok(tx) = v0_res {
                            return Ok(tx);
                        }
                    }

                    if capabilities.supports_legacy {
                        return Self::build_signed_legacy_transaction(
                            signer,
                            instructions,
                            recent_blockhash,
                            FormatSelectionReason::SmallPayloadLegacy {
                                accounts_count: unique_accounts_count,
                                wire_size: legacy_wire_size,
                            },
                        );
                    }
                }
            }
            Err(_) => {
                // If legacy compilation fails due to account indexing, continue to V0/V1
            }
        }

        // If lookup tables are available, try V0 compression
        if !lookup_tables.is_empty() && capabilities.supports_alt && capabilities.supports_v0 {
            let v0_result = Self::build_signed_v0_transaction(
                signer,
                instructions,
                recent_blockhash,
                lookup_tables,
                FormatSelectionReason::AltRequiredForMtu {
                    accounts_count: unique_accounts_count,
                    table_count: lookup_tables.len(),
                    wire_size: 0,
                },
            );

            if let Ok(mut tx) = v0_result {
                if tx.wire_size() <= SOLANA_PACKET_LEGACY_MTU
                    && tx.wire_size() <= capabilities.max_tx_bytes
                {
                    tx.selection_reason = FormatSelectionReason::AltRequiredForMtu {
                        accounts_count: unique_accounts_count,
                        table_count: lookup_tables.len(),
                        wire_size: tx.wire_size(),
                    };
                    return Ok(tx);
                }
            }
        }

        // Payload exceeds 1,232 bytes: attempt V1 if supported
        if capabilities.supports_v1 {
            let config = v1_config.unwrap_or_default();
            let v1_tx = Self::build_signed_v1_transaction(
                signer,
                instructions,
                recent_blockhash,
                config,
                FormatSelectionReason::LargePayloadV1 {
                    accounts_count: unique_accounts_count,
                    wire_size: 0,
                },
            )?;

            let final_wire_size = v1_tx.wire_size();
            if final_wire_size <= capabilities.max_tx_bytes {
                return Ok(CompiledAdaptiveTransaction {
                    selection_reason: FormatSelectionReason::LargePayloadV1 {
                        accounts_count: unique_accounts_count,
                        wire_size: final_wire_size,
                    },
                    ..v1_tx
                });
            }

            return Err(TxError::TransactionTooLarge {
                size: final_wire_size,
                limit: capabilities.max_tx_bytes,
            });
        }

        // If we reach here, payload exceeds 1,232 bytes and V1 is not supported
        Err(TxError::TransactionTooLarge {
            size: SOLANA_PACKET_LEGACY_MTU + 1,
            limit: SOLANA_PACKET_LEGACY_MTU,
        })
    }
}

/// Helper to count unique accounts across instructions including payer.
/// Public so the V1 fallback can pre-flight the SIMD-0385 account limit.
pub fn count_unique_accounts(payer: Pubkey, instructions: &[CompiledInstruction]) -> usize {
    let mut accounts = std::collections::HashSet::new();
    accounts.insert(payer);
    for ix in instructions {
        accounts.insert(ix.program_id);
        for acc in &ix.accounts {
            accounts.insert(acc.pubkey);
        }
    }
    accounts.len()
}

/// Decode Solana compact-u16 encoded integer from the beginning of a buffer.
/// Returns (value, bytes_consumed).
fn decode_compact_u16(bytes: &[u8]) -> Result<(u16, usize), TxError> {
    if bytes.is_empty() {
        return Err(TxError::Serialization("Empty compact-u16 buffer".into()));
    }

    let mut val: u16 = 0;
    let mut shift: usize = 0;

    for (i, &b) in bytes.iter().enumerate() {
        if i >= 3 {
            return Err(TxError::Serialization("Malformed compact-u16".into()));
        }
        val |= ((b & 0x7f) as u16) << shift;
        if b & 0x80 == 0 {
            return Ok((val, i + 1));
        }
        shift += 7;
    }

    Err(TxError::Serialization(
        "Unexpected end of compact-u16 buffer".into(),
    ))
}
