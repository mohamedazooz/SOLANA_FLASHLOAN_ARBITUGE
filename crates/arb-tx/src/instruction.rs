//! Core instruction primitives: AccountMeta, CompiledInstruction,
//! compute-budget instructions, Jito tip builders and wire helpers.

use super::*;

/// Metadata for accounts passed in Solana instructions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountMeta {
    pub pubkey: Pubkey,
    pub is_signer: bool,
    pub is_writable: bool,
}

impl AccountMeta {
    pub fn new(pubkey: Pubkey, is_signer: bool) -> Self {
        Self {
            pubkey,
            is_signer,
            is_writable: true,
        }
    }

    pub fn new_readonly(pubkey: Pubkey, is_signer: bool) -> Self {
        Self {
            pubkey,
            is_signer,
            is_writable: false,
        }
    }
}

/// Generic compiled instruction ready for serialization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledInstruction {
    pub program_id: Pubkey,
    pub accounts: Vec<AccountMeta>,
    pub data: Vec<u8>,
}

/// Build Compute Budget Unit Limit instruction.
pub fn build_compute_unit_limit(units: u32) -> CompiledInstruction {
    let mut data = Vec::with_capacity(5);
    data.push(2); // SetComputeUnitLimit instruction tag
    data.extend_from_slice(&units.to_le_bytes());
    CompiledInstruction {
        program_id: Pubkey::new_from_array(COMPUTE_BUDGET_PROGRAM_ID),
        accounts: Vec::new(),
        data,
    }
}

/// Build Compute Budget Unit Price (Priority Fee in micro-lamports) instruction.
pub fn build_compute_unit_price(micro_lamports: u64) -> CompiledInstruction {
    let mut data = Vec::with_capacity(9);
    data.push(3); // SetComputeUnitPrice instruction tag
    data.extend_from_slice(&micro_lamports.to_le_bytes());
    CompiledInstruction {
        program_id: Pubkey::new_from_array(COMPUTE_BUDGET_PROGRAM_ID),
        accounts: Vec::new(),
        data,
    }
}

/// Get next Jito tip account using round-robin distribution (delegates to arb-constants).
pub fn get_next_jito_tip_account() -> Pubkey {
    arb_constants::jito_tip_account_round_robin()
}

/// Build native SOL transfer instruction for Jito Tip account.
pub fn build_jito_tip_instruction(
    payer: Pubkey,
    tip_account: Pubkey,
    lamports: Lamports,
) -> CompiledInstruction {
    let mut data = Vec::with_capacity(12);
    data.extend_from_slice(&2u32.to_le_bytes()); // SystemProgram::Transfer instruction index = 2
    data.extend_from_slice(&lamports.as_u64().to_le_bytes());

    CompiledInstruction {
        program_id: Pubkey::new_from_array(SYSTEM_PROGRAM_ID),
        accounts: vec![
            AccountMeta::new(payer, true),
            AccountMeta::new(tip_account, false),
        ],
        data,
    }
}

/// Attach the Jito `jitodontfront` marker to an existing instruction.
///
/// A bundle containing a transaction that references the marker account is
/// rejected by the block engine unless that transaction sits at index 0, which
/// is what stops a searcher's own bundle from being sandwiched.
///
/// The marker is appended as a **read-only account of a real instruction**, not
/// as a standalone instruction: the marker account does not exist on chain, so
/// a transaction that named it as a program would fail to execute and take the
/// whole bundle down with it. Appending it to, say, the tip transfer costs one
/// extra account and no extra instruction.
pub fn attach_jitodontfront(instruction: &mut CompiledInstruction) {
    let marker = arb_constants::jitodontfront_marker();
    if instruction
        .accounts
        .iter()
        .any(|meta| meta.pubkey == marker)
    {
        return;
    }
    instruction
        .accounts
        .push(AccountMeta::new_readonly(marker, false));
}

/// Build a tip transfer that also carries the `jitodontfront` marker.
///
/// Convenience wrapper for the common case: the tip instruction is the last one
/// in the transaction and the only place the marker is needed.
pub fn build_marked_jito_tip_instruction(
    payer: Pubkey,
    tip_account: Pubkey,
    lamports: Lamports,
) -> CompiledInstruction {
    let mut instruction = build_jito_tip_instruction(payer, tip_account, lamports);
    attach_jitodontfront(&mut instruction);
    instruction
}

/// Ensure MEV sandwich protection on an instruction set.
/// If no instruction carries the `jitodontfront` marker account, attaches it
/// to the first instruction as a read-only account.
pub fn ensure_bundle_mev_protection(instructions: &mut [CompiledInstruction]) {
    let marker = arb_constants::jitodontfront_marker();
    let already_marked = instructions
        .iter()
        .any(|ix| ix.accounts.iter().any(|a| a.pubkey == marker));
    if !already_marked && !instructions.is_empty() {
        attach_jitodontfront(&mut instructions[0]);
    }
}

/// Extract the primary transaction signature from serialized transaction wire bytes.
///
/// Supports Legacy, V0, and V1 (SIMD-0385 trailing signatures) transaction formats.
pub fn extract_transaction_signature(tx_bytes: &[u8]) -> Option<String> {
    extract_adaptive_transaction_signature(tx_bytes)
}

/// Validate that a bundle of transactions conforms to Jito constraints and MEV protection:
/// 1. Bundle contains 1..=5 transactions.
/// 2. Total size of each transaction conforms to its wire-format limit:
///    legacy/v0 stay at the 1,232-byte MTU; v1 (SIMD-0385) may reach 4,096
///    bytes. Jito block engines already carry the larger v1 size — that is
///    the documented pre-SIMD-0296 workaround (see SIMD-0296 motivation) — so
///    the size gate must be version-aware rather than a blanket 1,232.
pub fn validate_jito_bundle(txs: &[Vec<u8>]) -> Result<(), TxError> {
    if txs.is_empty() || txs.len() > 5 {
        return Err(TxError::InvalidHopCount(txs.len()));
    }
    for tx in txs.iter() {
        if tx.is_empty() {
            return Err(TxError::ZeroBorrowAmount);
        }
        // Version-aware size gate. A malformed/unknown envelope is treated as
        // legacy and held to the tighter 1,232-byte bound (fail-closed).
        let limit = match CompiledAdaptiveTransaction::parse_version_from_wire(tx) {
            Ok(TransactionVersion::V1) => SOLANA_PACKET_V1_MTU,
            _ => SOLANA_PACKET_MTU_LIMIT,
        };
        if tx.len() > limit {
            return Err(TxError::TransactionTooLarge {
                size: tx.len(),
                limit,
            });
        }
    }
    Ok(())
}
