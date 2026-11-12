//! Inline Zero-Allocation V0 Wire Size Estimator (Module 2).
//!
//! Before serializing bincode, compute the exact wire byte size in O(N) time.
//!
//! # V0 Wire Format Breakdown
//! - Signatures: compact_u16(num_sigs) + num_sigs * 64
//! - Version Prefix: 1 byte (0x80 for V0)
//! - MessageHeader: 3 bytes (num_required_signatures, num_readonly_signed, num_readonly_unsigned)
//! - Static Account Keys: compact_u16(static_keys.len()) + static_keys.len() * 32
//! - Recent Blockhash: 32 bytes
//! - Compiled Instructions: compact_u16(ixs.len()) + sum of each instruction:
//!   1 (program_id_index) + compact_u16(accounts.len()) + accounts.len() + compact_u16(data.len()) + data.len()
//! - Address Table Lookups: compact_u16(lookups.len()) + sum of each lookup:
//!   32 (table_key) + compact_u16(writable.len()) + writable.len() + compact_u16(readonly.len()) + readonly.len()

use arb_tx::CompiledInstruction;
use solana_pubkey::Pubkey;
use std::collections::HashSet;

/// Size of a compact-u16 encoding of `n` in bytes (1..=3 for realistic sizes).
///
/// - 0..=127        -> 1 byte
/// - 128..=16383    -> 2 bytes
/// - >= 16384       -> 3 bytes
#[inline]
pub fn compact_u16_len(n: usize) -> usize {
    if n < 0x80 {
        1
    } else if n < 0x4000 {
        2
    } else {
        3
    }
}

/// Encode a compact-u16 into the provided buffer.
pub fn encode_compact_u16(n: usize, buf: &mut Vec<u8>) {
    let mut val = n as u16;
    loop {
        let mut byte = (val & 0x7f) as u8;
        val >>= 7;
        if val != 0 {
            byte |= 0x80;
        }
        buf.push(byte);
        if val == 0 {
            break;
        }
    }
}

/// Calculate the exact V0 wire size for a transaction with the given components.
pub fn estimate_v0_wire_size(
    payer: Pubkey,
    instructions: &[CompiledInstruction],
    _recent_blockhash: &[u8; 32],
    alt_accounts: &[Pubkey],
) -> Result<usize, arb_tx::TxError> {
    // Build the static account keys (inline accounts not in ALT)
    // For estimation, we need to determine which accounts are inline vs in ALT
    let static_keys = collect_static_keys(payer, instructions, alt_accounts);
    let static_keys_len = static_keys.len();

    // Build ALT lookup tables
    let alt_tables = build_alt_lookups(instructions, alt_accounts);
    let num_lookups = alt_tables.len();

    // Count signatures (typically 1 for single-signer)
    let num_sigs = 1;

    // Header counts
    let (_num_required_signatures, _num_readonly_signed, _num_readonly_unsigned) =
        count_header_fields(payer, &static_keys, &alt_tables);

    // Start calculating
    let mut total = 0;

    // 1. Signatures vector: compact_u16(num_sigs) + num_sigs * 64
    total += compact_u16_len(num_sigs) + num_sigs * 64;

    // 2. Version prefix: 1 byte (0x80)
    total += 1;

    // 3. Message header: 3 bytes
    total += 3;

    // 4. Static account keys: compact_u16(len) + len * 32
    total += compact_u16_len(static_keys_len) + static_keys_len * 32;

    // 5. Recent blockhash: 32 bytes
    total += 32;

    // 6. Compiled instructions
    total += compact_u16_len(instructions.len());
    for ix in instructions {
        total += estimate_v0_instruction_size(ix, &static_keys, &alt_tables);
    }

    // 7. Address table lookups
    total += compact_u16_len(num_lookups);
    for table in &alt_tables {
        total += 32; // table key
        total += compact_u16_len(table.writable_indexes.len()) + table.writable_indexes.len();
        total += compact_u16_len(table.readonly_indexes.len()) + table.readonly_indexes.len();
    }

    Ok(total)
}

/// Estimate the wire size of a single compiled instruction in V0 format.
fn estimate_v0_instruction_size(
    ix: &CompiledInstruction,
    static_keys: &[Pubkey],
    alt_tables: &[AltLookup],
) -> usize {
    // 1 byte: program_id_index
    // compact_u16(accounts.len()) + accounts.len() (each account index is 1 byte)
    // compact_u16(data.len()) + data.len()
    let mut size = 1; // program_id_index

    // Program ID index
    let _prog_idx = find_account_index(ix.program_id, static_keys, alt_tables);
    // Program index is always 1 byte in V0 (u8)
    // (already counted above)

    // Accounts
    size += compact_u16_len(ix.accounts.len());
    size += ix.accounts.len(); // each account index is 1 byte (u8)

    // Data
    size += compact_u16_len(ix.data.len());
    size += ix.data.len();

    size
}

/// Find the account index for a pubkey in V0 message (static keys + ALT).
fn find_account_index(pubkey: Pubkey, static_keys: &[Pubkey], alt_tables: &[AltLookup]) -> u8 {
    // Check static keys first
    for (i, &key) in static_keys.iter().enumerate() {
        if key == pubkey {
            return i as u8;
        }
    }

    // Check ALT tables
    let static_len = static_keys.len();
    for table in alt_tables {
        for (i, &key) in table.writable_indexes.iter().enumerate() {
            if key == pubkey {
                return (static_len + i) as u8;
            }
        }
        for (i, &key) in table.readonly_indexes.iter().enumerate() {
            if key == pubkey {
                return (static_len + table.writable_indexes.len() + i) as u8;
            }
        }
    }

    // Fallback (should not happen in valid transactions)
    0
}

/// Collect all static (inline) account keys for the V0 message.
fn collect_static_keys(
    payer: Pubkey,
    instructions: &[CompiledInstruction],
    alt_accounts: &[Pubkey],
) -> Vec<Pubkey> {
    let mut keys = Vec::new();
    let mut seen = HashSet::new();
    let alt_set: HashSet<Pubkey> = alt_accounts.iter().copied().collect();

    // Payer is always first (signer, writable)
    keys.push(payer);
    seen.insert(payer);

    // Add all instruction accounts that are NOT in the ALT
    for ix in instructions {
        // Program ID
        if !alt_set.contains(&ix.program_id) && seen.insert(ix.program_id) {
            keys.push(ix.program_id);
        }

        for acc in &ix.accounts {
            if !alt_set.contains(&acc.pubkey) && seen.insert(acc.pubkey) {
                keys.push(acc.pubkey);
            }
        }
    }

    keys
}

/// Count header fields for V0 message.
fn count_header_fields(
    _payer: Pubkey,
    static_keys: &[Pubkey],
    alt_tables: &[AltLookup],
) -> (u8, u8, u8) {
    let num_required_signatures = 1; // payer
    let num_readonly_signed = 0;
    let mut num_readonly_unsigned = 0;

    // Static keys after payer
    for (i, &_key) in static_keys.iter().enumerate() {
        if i == 0 {
            continue; // payer already counted
        }
        // We don't have privilege info here, so we estimate conservatively
        // In practice, this is computed during actual compilation
        num_readonly_unsigned += 1;
    }

    // ALT accounts - we don't know privileges at estimation time
    // Conservative: assume all ALT accounts are readonly unsigned
    for table in alt_tables {
        num_readonly_unsigned += table.readonly_indexes.len() as u8;
        num_readonly_unsigned += table.writable_indexes.len() as u8;
    }

    (
        num_required_signatures,
        num_readonly_signed,
        num_readonly_unsigned,
    )
}

/// ALT lookup table representation for size estimation.
#[derive(Debug, Clone)]
struct AltLookup {
    writable_indexes: Vec<Pubkey>,
    readonly_indexes: Vec<Pubkey>,
}

/// Build ALT lookup tables from instructions and ALT-eligible accounts.
fn build_alt_lookups(
    instructions: &[CompiledInstruction],
    alt_accounts: &[Pubkey],
) -> Vec<AltLookup> {
    let alt_set: HashSet<Pubkey> = alt_accounts.iter().copied().collect();
    let mut writable = HashSet::new();
    let mut readonly = HashSet::new();

    for ix in instructions {
        for acc in &ix.accounts {
            if alt_set.contains(&acc.pubkey) {
                if acc.is_writable {
                    writable.insert(acc.pubkey);
                } else {
                    readonly.insert(acc.pubkey);
                }
            }
        }
    }

    // Remove any writable from readonly (privilege escalation)
    for w in &writable {
        readonly.remove(w);
    }

    // Single ALT table for now (the master ALT)
    if writable.is_empty() && readonly.is_empty() {
        vec![]
    } else {
        vec![AltLookup {
            writable_indexes: writable.into_iter().collect(),
            readonly_indexes: readonly.into_iter().collect(),
        }]
    }
}

/// Exact V0 wire cost per instruction (used for byte budget accounting).
pub fn v0_instruction_wire_cost(ix: &CompiledInstruction) -> usize {
    1 + compact_u16_len(ix.accounts.len())
        + ix.accounts.len()
        + compact_u16_len(ix.data.len())
        + ix.data.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_tx::{AccountMeta, CompiledInstruction};
    use solana_pubkey::Pubkey;

    #[test]
    fn test_compact_u16_len() {
        assert_eq!(compact_u16_len(0), 1);
        assert_eq!(compact_u16_len(1), 1);
        assert_eq!(compact_u16_len(127), 1);
        assert_eq!(compact_u16_len(128), 2);
        assert_eq!(compact_u16_len(16383), 2);
        assert_eq!(compact_u16_len(16384), 3);
        assert_eq!(compact_u16_len(65535), 3);
    }

    #[test]
    fn test_encode_compact_u16() {
        let mut buf = Vec::new();
        encode_compact_u16(1, &mut buf);
        assert_eq!(buf, vec![1]);

        buf.clear();
        encode_compact_u16(127, &mut buf);
        assert_eq!(buf, vec![127]);

        buf.clear();
        encode_compact_u16(128, &mut buf);
        assert_eq!(buf, vec![0x80, 1]);

        buf.clear();
        encode_compact_u16(16383, &mut buf);
        assert_eq!(buf, vec![0xFF, 0x7F]);

        buf.clear();
        encode_compact_u16(16384, &mut buf);
        assert_eq!(buf, vec![0x80, 0x80, 1]);
    }

    #[test]
    fn test_v0_instruction_wire_cost() {
        let ix = CompiledInstruction {
            program_id: Pubkey::new_unique(),
            accounts: vec![
                AccountMeta::new(Pubkey::new_unique(), true),
                AccountMeta::new_readonly(Pubkey::new_unique(), false),
            ],
            data: vec![1, 2, 3, 4],
        };
        // 1 (prog_idx) + 1 (compact_u16(2)) + 2 (accounts) + 1 (compact_u16(4)) + 4 (data) = 9
        assert_eq!(v0_instruction_wire_cost(&ix), 9);
    }
}
