//! Master ALT Partitioning & V0 Compiler (Module 3).
//!
//! 1. Maintain a `MasterAltCache` containing our static base ALT.
//! 2. Partition all merged accounts into:
//!    - **Static Accounts:** Signers, Fee Payer, Top-Level invoked `program_id`s, and unindexed dynamic meme-coin pool/vault/bin accounts.
//!    - **ALT Writable Lookups:** Non-signer, non-top-level-program accounts present in `MasterAltCache` where `is_writable == true`.
//!    - **ALT Readonly Lookups:** Non-signer, non-top-level-program accounts present in `MasterAltCache` where `is_writable == false`.
//! 3. Sort Static Accounts canonically and build `v0::Message` with exact `CompiledInstruction` indices.

use arb_constants::{
    ASSOCIATED_TOKEN_PROGRAM_ID, COMPUTE_BUDGET_PROGRAM_ID, TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID,
};
use arb_tx::{
    AddressLookupTable, CompiledInstruction, KeypairSigner, StaticMasterAlt,
    ThreeLegMemeArbitrageContext, TxBuilder, TxError,
};
use solana_pubkey::Pubkey;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;

/// Top-level program IDs that must NEVER be placed in an ALT (must be static keys).
fn top_level_programs() -> HashSet<Pubkey> {
    let mut set = HashSet::new();
    set.insert(Pubkey::new_from_array(
        arb_constants::COMPUTE_BUDGET_PROGRAM_ID,
    ));
    set.insert(Pubkey::new_from_array(arb_constants::SYSTEM_PROGRAM_ID));
    set.insert(Pubkey::from_str(arb_constants::TOKEN_PROGRAM_ID).unwrap());
    set.insert(Pubkey::from_str(arb_constants::TOKEN_2022_PROGRAM_ID).unwrap());
    set.insert(Pubkey::from_str(arb_constants::ASSOCIATED_TOKEN_PROGRAM_ID).unwrap());
    set.insert(Pubkey::from_str(arb_constants::MEMO_PROGRAM_ID).unwrap());
    set.insert(Pubkey::from_str(arb_constants::METEORA_DLMM_PROGRAM_ID).unwrap());
    set
}

/// Partition accounts into three categories for V0 compilation.
#[derive(Debug, Clone)]
pub struct PartitionedAccounts {
    /// Static accounts (inline in message): signers, fee payer, top-level programs, dynamic accounts not in ALT.
    pub static_keys: Vec<Pubkey>,
    /// ALT writable accounts (non-signer, writable, in Master ALT).
    pub alt_writable: Vec<Pubkey>,
    /// ALT readonly accounts (non-signer, readonly, in Master ALT).
    pub alt_readonly: Vec<Pubkey>,
    /// Map from pubkey to its final V0 index (static + alt_writable + alt_readonly).
    pub index_map: HashMap<Pubkey, u8>,
}

/// Partition all accounts from instructions into static vs ALT categories.
pub fn partition_for_v0(
    payer: Pubkey,
    instructions: &[CompiledInstruction],
    master_alt: &StaticMasterAlt,
) -> Result<PartitionedAccounts, TxError> {
    // Collect all unique accounts with their merged privileges
    let mut account_privileges: HashMap<Pubkey, (bool, bool)> = HashMap::new();
    let mut program_ids: HashSet<Pubkey> = HashSet::new();

    // Top-level program IDs (must be static)
    let top_level_programs = top_level_programs();

    // First pass: collect all accounts and their max privileges
    for ix in instructions {
        program_ids.insert(ix.program_id);
        account_privileges
            .entry(ix.program_id)
            .and_modify(|(s, w)| {
                *s = false; // programs are never signers
                *w = false; // programs are readonly
            })
            .or_insert((false, false));

        for acc in &ix.accounts {
            account_privileges
                .entry(acc.pubkey)
                .and_modify(|(s, w)| {
                    *s |= acc.is_signer;
                    *w |= acc.is_writable;
                })
                .or_insert((acc.is_signer, acc.is_writable));
        }
    }

    // Payer is always signer + writable
    account_privileges
        .entry(payer)
        .and_modify(|(s, w)| {
            *s = true;
            *w = true;
        })
        .or_insert((true, true));

    // Classify each account
    let mut static_keys = Vec::new();
    let mut alt_writable = Vec::new();
    let mut alt_readonly = Vec::new();

    // Payer is ALWAYS first static key (index 0)
    static_keys.push(payer);

    // Collect all other accounts
    let mut other_accounts: Vec<Pubkey> = account_privileges
        .keys()
        .copied()
        .filter(|&k| k != payer)
        .collect();

    // Sort deterministically for canonical ordering
    other_accounts.sort_by_key(|a| a.to_bytes());

    for addr in other_accounts {
        let (is_signer, is_writable) = account_privileges[&addr];
        let is_top_level_program = top_level_programs.contains(&addr);
        let in_master_alt = master_alt.contains(&addr);

        if is_signer || is_top_level_program || !in_master_alt {
            // Must be static: signer, top-level program, or not in Master ALT
            static_keys.push(addr);
        } else if is_writable {
            // Non-signer, writable, in Master ALT -> ALT writable
            alt_writable.push(addr);
        } else {
            // Non-signer, readonly, in Master ALT -> ALT readonly
            alt_readonly.push(addr);
        }
    }

    // Build index map
    let mut index_map = HashMap::new();
    for (i, &key) in static_keys.iter().enumerate() {
        index_map.insert(key, i as u8);
    }
    let static_len = static_keys.len();
    for (i, &key) in alt_writable.iter().enumerate() {
        index_map.insert(key, (static_len + i) as u8);
    }
    let alt_writable_len = alt_writable.len();
    for (i, &key) in alt_readonly.iter().enumerate() {
        index_map.insert(key, (static_len + alt_writable_len + i) as u8);
    }

    Ok(PartitionedAccounts {
        static_keys,
        alt_writable,
        alt_readonly,
        index_map,
    })
}

/// Compile a V0 transaction with the Static Master ALT.
pub fn compile_v0(
    ctx: &ThreeLegMemeArbitrageContext,
    instructions: &[CompiledInstruction],
    master_alt: &StaticMasterAlt,
    _alt_accounts: &[Pubkey],
    _inline_accounts: &[Pubkey],
    signer: &KeypairSigner,
) -> Result<arb_tx::CompiledV0Transaction, TxError> {
    // Verify no ALT account is used as a signer
    let signer_addresses = [ctx.payer];
    master_alt
        .verify_no_signers(&signer_addresses)
        .map_err(|e| TxError::InvalidInput(format!("ALT signer violation: {e}")))?;

    // Verify ALT activation
    master_alt.verify_activation().map_err(|_| {
        TxError::InvalidInput("Static Master ALT is not activated on-chain".to_string())
    })?;

    // Verify required addresses exist in ALT
    master_alt
        .verify_required_present()
        .map_err(|e| TxError::InvalidInput(format!("ALT missing required address: {e}")))?;

    // Partition accounts for V0
    let partitioned = partition_for_v0(ctx.payer, instructions, master_alt)?;

    // Build ALT lookup table
    let alt_key = master_alt
        .alt_address
        .ok_or_else(|| TxError::InvalidInput("No ALT address configured".to_string()))?;

    let alt_table = AddressLookupTable {
        key: alt_key,
        addresses: {
            let mut addrs = Vec::new();
            addrs.extend(partitioned.alt_writable.iter().copied());
            addrs.extend(partitioned.alt_readonly.iter().copied());
            addrs
        },
    };

    // Build V0 message using TxBuilder
    let message_bytes = TxBuilder::compile_v0_message(
        ctx.payer,
        instructions,
        ctx.recent_blockhash,
        std::slice::from_ref(&alt_table),
    )
    .map_err(|e| TxError::AltCompilationError(e.to_string()))?;

    // Sign the message
    let signature = signer.sign(&message_bytes);

    // Encode wire format: compact_u16(1) + signature + message
    let mut wire_bytes = Vec::with_capacity(1 + 64 + message_bytes.len());
    TxBuilder::encode_compact_u16(1, &mut wire_bytes);
    wire_bytes.extend_from_slice(&signature);
    wire_bytes.extend_from_slice(&message_bytes);

    let wire_size = wire_bytes.len();

    // Measure components for byte budget
    let byte_budget = measure_v0_components(&message_bytes, instructions, wire_size)?;

    Ok(arb_tx::CompiledV0Transaction {
        wire_bytes,
        message_bytes,
        signatures: vec![signature],
        wire_size,
        lookup_table_address: alt_key,
        static_account_count: partitioned.static_keys.len(),
        inline_account_count: partitioned.static_keys.len(),
        alt_resolved_count: partitioned.alt_writable.len() + partitioned.alt_readonly.len(),
        signer_count: 1,
        byte_budget,
    })
}

/// Measure V0 message components for byte budget reporting.
fn measure_v0_components(
    message: &[u8],
    instructions: &[CompiledInstruction],
    wire_size: usize,
) -> Result<arb_tx::ByteBudgetReport, TxError> {
    let mut budget = arb_tx::ByteBudgetReport::new();
    budget.message_version_prefix = 1; // 0x80
    budget.header_bytes = 3;
    budget.blockhash_bytes = 32;

    // Parse message to extract component sizes
    let mut prefix_bytes = 0usize;
    let (n_static, static_hdr) = decode_compact_u16(&message[4..])?;
    budget.static_account_keys_bytes = n_static * 32;
    let signers = message[1] as usize;
    budget.inline_dynamic_account_keys_bytes = n_static.saturating_sub(signers) * 32;
    prefix_bytes += static_hdr;

    let mut i = 4 + static_hdr + n_static * 32 + 32;
    let (_n_ix, ix_hdr) = decode_compact_u16(&message[i..])?;
    prefix_bytes += ix_hdr;
    i += ix_hdr;

    let compute = Pubkey::new_from_array(COMPUTE_BUDGET_PROGRAM_ID);
    let ata_program = Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM_ID).unwrap();
    let token = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
    let token_2022 = Pubkey::from_str(TOKEN_2022_PROGRAM_ID).unwrap();

    let mut instruction_bytes = 0usize;
    let mut create_bytes = 0usize;
    let mut close_bytes = 0usize;
    let mut compute_bytes = 0usize;

    for ix in instructions.iter() {
        i += 1; // 1-byte program index
        let (n_acc, acc_hdr) = decode_compact_u16(&message[i..])?;
        i += acc_hdr + n_acc;
        let (dlen, dl_hdr) = decode_compact_u16(&message[i..])?;
        i += dl_hdr + dlen;
        let cost = 1 + acc_hdr + n_acc + dl_hdr + dlen;
        prefix_bytes += acc_hdr + dl_hdr;
        instruction_bytes += cost;
        if ix.program_id == compute {
            compute_bytes += cost;
        } else if ix.program_id == ata_program {
            create_bytes += cost;
        } else if ix.data == [9] && (ix.program_id == token || ix.program_id == token_2022) {
            close_bytes += cost;
        }
    }
    budget.compiled_instructions_bytes = instruction_bytes;
    budget.compute_configuration_bytes = compute_bytes;
    budget.create_ata_idempotent_bytes = create_bytes;
    budget.close_account_bytes = close_bytes;
    budget.jito_tip_instruction_bytes = instructions
        .last()
        .map(v0_instruction_wire_cost)
        .unwrap_or(0);

    // Address table lookup section
    let (n_lut, lut_hdr) = decode_compact_u16(&message[i..])?;
    i += lut_hdr;
    prefix_bytes += lut_hdr;
    let mut lut_bytes = lut_hdr;
    let mut index_vector_bytes = 0usize;
    let mut loaded = 0usize;
    for _ in 0..n_lut {
        i += 32; // table key
        lut_bytes += 32;
        let (w, w_hdr) = decode_compact_u16(&message[i..])?;
        i += w_hdr + w;
        lut_bytes += w_hdr + w;
        prefix_bytes += w_hdr;
        index_vector_bytes += w;
        loaded += w;

        let (r, r_hdr) = decode_compact_u16(&message[i..])?;
        i += r_hdr + r;
        lut_bytes += r_hdr + r;
        prefix_bytes += r_hdr;
        index_vector_bytes += r;
        loaded += r;
    }
    budget.alt_lookup_references_bytes = lut_bytes;
    budget.lookup_index_vectors_bytes = index_vector_bytes;
    budget.compact_u16_prefixes_bytes = prefix_bytes;

    // Savings: inlining the loaded keys would have cost 32 bytes each
    budget.alt_savings_bytes = loaded as i64 * 32 - lut_bytes as i64;
    budget.signature_vector_bytes = 1 + 64;
    budget.total_wire_size = wire_size;

    debug_assert_eq!(i, message.len(), "v0 message parse must consume every byte");

    Ok(budget)
}

/// Exact V0 wire cost per instruction.
fn v0_instruction_wire_cost(ix: &CompiledInstruction) -> usize {
    1 + crate::sizing::compact_u16_len(ix.accounts.len())
        + ix.accounts.len()
        + crate::sizing::compact_u16_len(ix.data.len())
        + ix.data.len()
}

/// Decode a compact-u16 and return (value, bytes_consumed).
fn decode_compact_u16(bytes: &[u8]) -> Result<(usize, usize), TxError> {
    let mut val: usize = 0;
    let mut shift: usize = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if i >= 3 {
            return Err(TxError::Serialization("malformed compact-u16".into()));
        }
        val |= ((b & 0x7f) as usize) << shift;
        if b & 0x80 == 0 {
            return Ok((val, i + 1));
        }
        shift += 7;
    }
    Err(TxError::Serialization(
        "unexpected end of compact-u16".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_tx::{AccountMeta, CompiledInstruction, StaticMasterAlt};
    use solana_pubkey::Pubkey;
    

    #[test]
    fn test_partition_for_v0() {
        let payer = Pubkey::new_from_array([1u8; 32]);
        let program_id = Pubkey::new_from_array(COMPUTE_BUDGET_PROGRAM_ID);
        let alt_account = Pubkey::new_from_array([42u8; 32]);

        // Create a mock master ALT with TOKEN_PROGRAM_ID in it
        let mut alt = StaticMasterAlt::new();
        alt.addresses.push(alt_account);
        alt.address_set.insert(alt_account);
        alt.categories
            .push(arb_tx::master_alt::AddressCategory::SplTokenProgram);

        let instructions = vec![CompiledInstruction {
            program_id,
            accounts: vec![
                AccountMeta::new(payer, true),
                AccountMeta::new_readonly(alt_account, false),
            ],
            data: vec![1, 2, 3],
        }];

        let partitioned = partition_for_v0(payer, &instructions, &alt).unwrap();

        // Payer should be static
        assert!(partitioned.static_keys.contains(&payer));
        // Program ID (top-level) should be static
        assert!(partitioned.static_keys.contains(&program_id));
        // Token program in ALT, non-signer, readonly -> ALT readonly
        assert!(partitioned.alt_readonly.contains(&alt_account));
        // Index map should have all three
        assert_eq!(partitioned.index_map[&payer], 0);
    }
}
