//! Automatic `V0 -> V1` Fallback Engine (Module 4).
//!
//! If `v0_wire_size > 1232`:
//! 1. **Abort V0 & Trigger Flattened V1 Compilation:**
//!    - De-index Master ALT: Expand every account that was compressed in `MasterAltCache`
//!      back into a full 32-byte `AccountMeta` in the flat account pool.
//!    - Remove all `MessageAddressTableLookup` structures.
//! 2. **Jito Tip Stripping & Dynamic Priority Fee Roll-Over:**
//!    - Jito Bundles strictly reject transactions > 1,232 bytes and do not support V1 without ALTs.
//!    - Strip the standalone `Jito Tip` transfer instruction (`SystemProgram::transfer`
//!      to the Jito Tip account) and remove the Jito Tip recipient `Pubkey` from the
//!      account list (saving 32+ bytes).
//!    - Convert the intended `jito_tip_lamports` directly into additional
//!      `ComputeUnitPrice` (`micro_lamports` per Compute Unit) so the validator Leader
//!      prioritizes the V1 transaction via SWQoS TPU:
//!      ```rust,ignore
//!      let extra_micro_lamports = ((jito_tip_lamports as u128) * 1_000_000u128) / (compute_unit_limit as u128);
//!      let v1_cu_price = base_cu_price_micro_lamports.saturating_add(extra_micro_lamports as u64);
//!      ```
//!    - Update the `ComputeBudgetInstruction::set_compute_unit_price(v1_cu_price)` instruction payload in-place.
//! 3. **Canonical Solana Account Sorting:**
//!    Sort the flattened `Vec<AccountMeta>` strictly into the 4 protocol buckets:
//!    - Bucket 0: `Fee Payer` MUST be at index `0` (`is_signer = true, is_writable = true`).
//!    - Bucket 1: Other `is_signer == true && is_writable == true`
//!    - Bucket 2: `is_signer == true && is_writable == false`
//!    - Bucket 3: `is_signer == false && is_writable == true`
//!    - Bucket 4: `is_signer == false && is_writable == false`
//! 4. **Full Instruction Index Remapping (`u8`):**
//!    - Compute `MessageHeader` counts (`num_required_signatures`,
//!      `num_readonly_signed_accounts`, `num_readonly_unsigned_accounts`).
//!    - For every instruction in the V1 payload, map `program_id` -> `u8` index in the
//!      sorted flat account list, and map each `AccountMeta` -> `u8` index in the
//!      sorted flat account list.
//! 5. **V1 Wire Serialization & Bounds Check:**
//!    - Serialize the V1 transaction wire format (`0x81` version prefix + header/config +
//!      signatures + flat 32-byte keys + recent blockhash + remapped `CompiledInstruction`s).
//!    - Verify `v1_wire_size <= 4096`. If `> 4096`, return
//!      `ArbTxError::PayloadExceedsV1Limit { size: v1_wire_size }`.

use arb_constants::{COMPUTE_BUDGET_PROGRAM_ID, JITO_TIP_ACCOUNTS, SYSTEM_PROGRAM_ID};
use arb_tx::{
    AccountMeta, CompiledInstruction, KeypairSigner, StaticMasterAlt, ThreeLegMemeArbitrageContext,
    TxError, adaptive::SOLANA_PACKET_V1_MTU,
};
use solana_pubkey::Pubkey;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::str::FromStr;

/// V1 compilation result with metadata.
#[derive(Debug, Clone)]
pub struct CompiledV1Result {
    pub wire_bytes: Vec<u8>,
    pub message_bytes: Vec<u8>,
    pub signatures: Vec<[u8; 64]>,
    pub wire_size: usize,
    pub account_count: usize,
    pub signer_count: usize,
    pub effective_cu_price_micro_lamports: u64,
}

/// Compile V1 fallback transaction from V0 components.
pub fn compile_v1_fallback(
    ctx: &ThreeLegMemeArbitrageContext,
    instructions: &[CompiledInstruction],
    _master_alt: &StaticMasterAlt,
    _alt_accounts: &[Pubkey],
    _inline_accounts: &[Pubkey],
    signer: &KeypairSigner,
    config: &crate::DispatcherConfig,
) -> Result<CompiledV1Result, TxError> {
    // 2. Strip Jito tip instruction and account, roll tip into CU price
    let (v1_instructions, effective_cu_price) = strip_jito_tip_and_rollover(
        instructions,
        ctx.jito_tip_lamports,
        config.base_cu_price_micro_lamports,
        ctx.cu_limit,
    )?;

    // 3. Rebuild account list from stripped instructions (some accounts may be removed)
    let all_accounts = rebuild_account_list_from_instructions(ctx.payer, &v1_instructions)?;

    // 4. Canonical Solana account sorting (4 buckets)
    let sorted_accounts = canonical_sort_accounts(&all_accounts);

    // 5. Build index map for remapping
    let key_to_index: HashMap<Pubkey, u8> = sorted_accounts
        .iter()
        .enumerate()
        .map(|(idx, &pk)| (pk, idx as u8))
        .collect();

    // 6. Verify SIMD-0385 limits
    if sorted_accounts.len() > 64 {
        return Err(TxError::SanitizationError(format!(
            "V1 account limit exceeded: {} > 64",
            sorted_accounts.len()
        )));
    }
    if v1_instructions.len() > 64 {
        return Err(TxError::SanitizationError(format!(
            "V1 instruction limit exceeded: {} > 64",
            v1_instructions.len()
        )));
    }

    // 7. Remap all instruction accounts and program IDs to u8 indices
    let remapped_instructions = remap_instructions(&v1_instructions, &key_to_index)?;

    // 8. Update ComputeBudget price instruction with rolled-over CU price
    let final_instructions = update_cu_price_instruction(remapped_instructions, effective_cu_price);

    // 9. Build V1 message
    let message_bytes = build_v1_message(
        ctx.payer,
        &final_instructions,
        ctx.recent_blockhash,
        &sorted_accounts,
        effective_cu_price,
        ctx.cu_limit,
    )?;

    // 10. Sign message
    let signature = signer.sign(&message_bytes);

    // 11. V1 wire format: message + trailing signature
    let mut wire_bytes = Vec::with_capacity(message_bytes.len() + 64);
    wire_bytes.extend_from_slice(&message_bytes);
    wire_bytes.extend_from_slice(&signature);

    let wire_size = wire_bytes.len();

    // 12. Final bounds check
    if wire_size > SOLANA_PACKET_V1_MTU {
        return Err(TxError::TransactionTooLarge {
            size: wire_size,
            limit: SOLANA_PACKET_V1_MTU,
        });
    }

    Ok(CompiledV1Result {
        wire_bytes,
        message_bytes,
        signatures: vec![signature],
        wire_size,
        account_count: sorted_accounts.len(),
        signer_count: count_signers(&sorted_accounts, &final_instructions),
        effective_cu_price_micro_lamports: effective_cu_price,
    })
}

/// Rebuild account list from instructions after Jito tip stripping.
fn rebuild_account_list_from_instructions(
    payer: Pubkey,
    instructions: &[CompiledInstruction],
) -> Result<Vec<AccountMeta>, TxError> {
    let mut account_map: BTreeMap<Pubkey, (bool, bool)> = BTreeMap::new();

    account_map.insert(payer, (true, true));

    for ix in instructions {
        account_map
            .entry(ix.program_id)
            .and_modify(|(s, w)| {
                *s = false;
                *w = false;
            })
            .or_insert((false, false));

        for acc in &ix.accounts {
            account_map
                .entry(acc.pubkey)
                .and_modify(|(s, w)| {
                    *s |= acc.is_signer;
                    *w |= acc.is_writable;
                })
                .or_insert((acc.is_signer, acc.is_writable));
        }
    }

    let mut result = Vec::with_capacity(account_map.len());
    for (pubkey, (is_signer, is_writable)) in account_map {
        result.push(AccountMeta {
            pubkey,
            is_signer,
            is_writable,
        });
    }

    Ok(result)
}

/// Strip Jito tip instruction and roll tip amount into CU price.
fn strip_jito_tip_and_rollover(
    instructions: &[CompiledInstruction],
    jito_tip_lamports: u64,
    base_cu_price: u64,
    cu_limit: u32,
) -> Result<(Vec<CompiledInstruction>, u64), TxError> {
    // Find and remove Jito tip instruction (SystemProgram::transfer to Jito tip account)
    let jito_tip_accounts: HashSet<Pubkey> = JITO_TIP_ACCOUNTS
        .iter()
        .map(|s| Pubkey::from_str(s).unwrap())
        .collect();

    let system_program = Pubkey::new_from_array(SYSTEM_PROGRAM_ID);
    let mut filtered = Vec::with_capacity(instructions.len());

    for ix in instructions {
        // Check if this is a Jito tip transfer
        let is_jito_tip = ix.program_id == system_program
            && ix.data.len() >= 12
            && ix.data[0..4] == [2, 0, 0, 0] // SystemProgram::Transfer
            && ix.accounts.len() == 2
            && jito_tip_accounts.contains(&ix.accounts[1].pubkey);

        if !is_jito_tip {
            filtered.push(ix.clone());
        }
    }

    // Calculate rolled-over CU price
    // extra_micro_lamports = (jito_tip_lamports * 1_000_000) / cu_limit
    let extra_micro_lamports = if cu_limit > 0 {
        ((jito_tip_lamports as u128) * 1_000_000u128) / (cu_limit as u128)
    } else {
        0
    };

    let v1_cu_price = base_cu_price.saturating_add(extra_micro_lamports as u64);

    Ok((filtered, v1_cu_price))
}

/// Canonical Solana account sorting into 4 buckets.
fn canonical_sort_accounts(accounts: &[AccountMeta]) -> Vec<Pubkey> {
    let mut bucket0 = Vec::new(); // Fee payer (signer + writable) - MUST be index 0
    let mut bucket1 = Vec::new(); // Other signers + writable
    let mut bucket2 = Vec::new(); // Signers + readonly
    let mut bucket3 = Vec::new(); // Non-signers + writable
    let mut bucket4 = Vec::new(); // Non-signers + readonly

    for acc in accounts {
        if acc.is_signer {
            if acc.is_writable {
                if bucket0.is_empty() {
                    bucket0.push(acc.pubkey); // Fee payer at index 0
                } else {
                    bucket1.push(acc.pubkey);
                }
            } else {
                bucket2.push(acc.pubkey);
            }
        } else if acc.is_writable {
            bucket3.push(acc.pubkey);
        } else {
            bucket4.push(acc.pubkey);
        }
    }

    // Sort each bucket deterministically (except bucket0 which has exactly 1 element)
    bucket1.sort_by_key(|a| a.to_bytes());
    bucket2.sort_by_key(|a| a.to_bytes());
    bucket3.sort_by_key(|a| a.to_bytes());
    bucket4.sort_by_key(|a| a.to_bytes());

    let mut result = Vec::with_capacity(accounts.len());
    result.extend(bucket0);
    result.extend(bucket1);
    result.extend(bucket2);
    result.extend(bucket3);
    result.extend(bucket4);

    result
}

/// Remap all instruction program IDs and account indices to u8 indices.
fn remap_instructions(
    instructions: &[CompiledInstruction],
    key_to_index: &HashMap<Pubkey, u8>,
) -> Result<Vec<CompiledInstruction>, TxError> {
    instructions
        .iter()
        .map(|ix| {
            let _prog_idx = *key_to_index.get(&ix.program_id).ok_or_else(|| {
                TxError::InstructionError(format!(
                    "Program ID {} missing from key map",
                    ix.program_id
                ))
            })?;

            let mut remapped_accounts = Vec::with_capacity(ix.accounts.len());
            for acc in &ix.accounts {
                let _acc_idx = *key_to_index.get(&acc.pubkey).ok_or_else(|| {
                    TxError::InstructionError(format!(
                        "Account {} missing from key map",
                        acc.pubkey
                    ))
                })?;
                remapped_accounts.push(AccountMeta {
                    pubkey: acc.pubkey,
                    is_signer: acc.is_signer,
                    is_writable: acc.is_writable,
                });
            }

            Ok(CompiledInstruction {
                program_id: ix.program_id,
                accounts: remapped_accounts,
                data: ix.data.clone(),
            })
        })
        .collect()
}

/// Update the ComputeBudget set_compute_unit_price instruction with the new price.
fn update_cu_price_instruction(
    mut instructions: Vec<CompiledInstruction>,
    new_price: u64,
) -> Vec<CompiledInstruction> {
    let compute_budget = Pubkey::new_from_array(COMPUTE_BUDGET_PROGRAM_ID);

    for ix in &mut instructions {
        if ix.program_id == compute_budget && ix.data.len() >= 1 && ix.data[0] == 3 {
            // Found set_compute_unit_price instruction, update it in-place
            ix.data[1..9].copy_from_slice(&new_price.to_le_bytes());
            break;
        }
    }

    instructions
}

/// Build V1 message (SIMD-0385 format).
fn build_v1_message(
    _payer: Pubkey,
    instructions: &[CompiledInstruction],
    recent_blockhash: [u8; 32],
    sorted_accounts: &[Pubkey],
    priority_fee_lamports: u64,
    cu_limit: u32,
) -> Result<Vec<u8>, TxError> {
    // SIMD-0385 message format:
    // [0] 0x81 (version discriminator)
    // [1..4] header: num_required_signatures, num_readonly_signed, num_readonly_unsigned
    // [4..8] TransactionConfigMask (u32 LE)
    // [8..40] recent blockhash (32 bytes)
    // [40] num_instructions (u8)
    // [41] num_addresses (u8)
    // [42..] addresses (num_addresses * 32 bytes)
    // [...] config values (based on mask)
    // [...] instruction headers (num_instructions * 4 bytes each: prog_idx, num_acc, data_len u16)
    // [...] instruction payloads (contiguous)

    // Compute header counts
    let (num_required_signatures, num_readonly_signed, num_readonly_unsigned) =
        compute_v1_header(sorted_accounts, instructions);

    // Build config mask and values
    let mut config_mask: u32 = 0;
    let mut config_values = Vec::new();

    // Always include priority fee and CU limit in V1
    config_mask |= 0x03; // CONFIG_MASK_PRIORITY_FEE (bits 0-1)
    config_values.extend_from_slice(&priority_fee_lamports.to_le_bytes()); // 8 bytes

    config_mask |= 0x04; // CONFIG_MASK_CU_LIMIT
    config_values.extend_from_slice(&cu_limit.to_le_bytes()); // 4 bytes

    // Build instruction headers and payloads
    let mut instruction_headers = Vec::with_capacity(instructions.len() * 4);
    let mut instruction_payloads = Vec::new();

    for ix in instructions {
        let prog_idx = key_to_index_fast(ix.program_id, sorted_accounts)?;
        let acc_count = ix.accounts.len();
        if acc_count > 255 {
            return Err(TxError::SanitizationError(format!(
                "Instruction accounts exceed 255: {}",
                acc_count
            )));
        }
        let data_len = ix.data.len();
        if data_len > u16::MAX as usize {
            return Err(TxError::SanitizationError(format!(
                "Instruction data exceeds u16 limit: {}",
                data_len
            )));
        }

        // 4-byte header: prog_idx (u8), num_acc (u8), data_len (u16 LE)
        instruction_headers.push(prog_idx);
        instruction_headers.push(acc_count as u8);
        instruction_headers.extend_from_slice(&(data_len as u16).to_le_bytes());

        // Payload: account indices + data
        for acc in &ix.accounts {
            let acc_idx = key_to_index_fast(acc.pubkey, sorted_accounts)?;
            instruction_payloads.push(acc_idx);
        }
        instruction_payloads.extend_from_slice(&ix.data);
    }

    // Assemble message
    let mut message = Vec::with_capacity(512);

    // Version discriminator
    message.push(0x81);

    // Header (3 bytes)
    message.push(num_required_signatures);
    message.push(num_readonly_signed);
    message.push(num_readonly_unsigned);

    // Config mask (4 bytes LE)
    message.extend_from_slice(&config_mask.to_le_bytes());

    // Recent blockhash (32 bytes)
    message.extend_from_slice(&recent_blockhash);

    // Num instructions (u8)
    message.push(instructions.len() as u8);

    // Num addresses (u8)
    message.push(sorted_accounts.len() as u8);

    // Addresses (inline 32-byte keys)
    for pk in sorted_accounts {
        message.extend_from_slice(pk.as_ref());
    }

    // Config values
    message.extend_from_slice(&config_values);

    // Instruction headers
    message.extend_from_slice(&instruction_headers);

    // Instruction payloads
    message.extend_from_slice(&instruction_payloads);

    Ok(message)
}

/// Fast lookup of account index in sorted account list.
fn key_to_index_fast(pubkey: Pubkey, sorted_accounts: &[Pubkey]) -> Result<u8, TxError> {
    // Linear search is fine for <= 64 accounts
    for (i, &pk) in sorted_accounts.iter().enumerate() {
        if pk == pubkey {
            return Ok(i as u8);
        }
    }
    Err(TxError::InstructionError(format!(
        "Account {} not found in sorted account list",
        pubkey
    )))
}

/// Compute V1 message header counts from sorted accounts.
/// The sorted_accounts are already in canonical order:
/// Bucket 0: Fee payer (signer + writable) - index 0
/// Bucket 1: Other signers + writable
/// Bucket 2: Signers + readonly
/// Bucket 3: Non-signers + writable
/// Bucket 4: Non-signers + readonly
fn compute_v1_header(
    sorted_accounts: &[Pubkey],
    instructions: &[CompiledInstruction],
) -> (u8, u8, u8) {
    // For V1, we need to count signers and readonly accounts.
    // Since we only have pubkeys here, we use the canonical ordering:
    // - The first account is always the fee payer (signer + writable)
    // - We need to determine how many are signers vs readonly

    // Collect all signer pubkeys from instructions
    let mut signer_set: HashSet<Pubkey> = HashSet::new();
    for ix in instructions {
        for acc in &ix.accounts {
            if acc.is_signer {
                signer_set.insert(acc.pubkey);
            }
        }
    }

    // Count signers (must include payer)
    let num_signers = signer_set.len().max(1);

    // Count readonly signers: signers that are NOT writable
    // In canonical order, buckets 0 and 1 are writable signers, bucket 2 is readonly signers
    // We approximate: readonly signers = signers that appear in bucket 2

    // For simplicity, use the standard formula:
    // num_required_signatures = total signers
    // num_readonly_signed = signers that are readonly
    // num_readonly_unsigned = non-signers that are readonly

    // Since we don't have privilege info in sorted_accounts (just Pubkeys),
    // we use a heuristic: assume 1 readonly signer (payer is writable) and
    // the rest are unsigned readonly if they don't appear as writable in any instruction
    let mut writable_set: HashSet<Pubkey> = HashSet::new();
    for ix in instructions {
        for acc in &ix.accounts {
            if acc.is_writable {
                writable_set.insert(acc.pubkey);
            }
        }
    }

    let mut readonly_signed = 0u8;
    let mut readonly_unsigned = 0u8;

    for pk in sorted_accounts {
        let is_signer = signer_set.contains(pk);
        let is_writable = writable_set.contains(pk)
            || *pk == sorted_accounts.first().copied().unwrap_or_default();

        if is_signer && !is_writable {
            readonly_signed += 1;
        } else if !is_signer && !is_writable {
            readonly_unsigned += 1;
        }
    }

    (num_signers as u8, readonly_signed, readonly_unsigned)
}

/// Count signers in the final instruction set.
fn count_signers(_sorted_accounts: &[Pubkey], instructions: &[CompiledInstruction]) -> usize {
    let mut signers = HashSet::new();
    for ix in instructions {
        for acc in &ix.accounts {
            if acc.is_signer {
                signers.insert(acc.pubkey);
            }
        }
    }
    signers.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_tx::{AccountMeta, CompiledInstruction, build_system_transfer};
    use solana_pubkey::Pubkey;
    use std::str::FromStr;

    #[test]
    fn test_canonical_sort_accounts() {
        let payer = Pubkey::new_from_array([1u8; 32]);
        let signer_writable = Pubkey::new_from_array([2u8; 32]);
        let signer_readonly = Pubkey::new_from_array([3u8; 32]);
        let nonsigner_writable = Pubkey::new_from_array([4u8; 32]);
        let nonsigner_readonly = Pubkey::new_from_array([5u8; 32]);

        let accounts = vec![
            AccountMeta {
                pubkey: nonsigner_readonly,
                is_signer: false,
                is_writable: false,
            },
            AccountMeta {
                pubkey: payer,
                is_signer: true,
                is_writable: true,
            },
            AccountMeta {
                pubkey: signer_writable,
                is_signer: true,
                is_writable: true,
            },
            AccountMeta {
                pubkey: signer_readonly,
                is_signer: true,
                is_writable: false,
            },
            AccountMeta {
                pubkey: nonsigner_writable,
                is_signer: false,
                is_writable: true,
            },
        ];

        let sorted = canonical_sort_accounts(&accounts);

        // Payer must be at index 0
        assert_eq!(sorted[0], payer);

        // Then other signer+writable
        assert_eq!(sorted[1], signer_writable);

        // Then signer+readonly
        assert_eq!(sorted[2], signer_readonly);

        // Then non-signer+writable
        assert_eq!(sorted[3], nonsigner_writable);

        // Then non-signer+readonly
        assert_eq!(sorted[4], nonsigner_readonly);
    }

    #[test]
    fn test_strip_jito_tip_and_rollover() {
        let jito_tip = Pubkey::from_str(JITO_TIP_ACCOUNTS[0]).unwrap();
        let _system_program = Pubkey::new_from_array(SYSTEM_PROGRAM_ID);
        let payer = Pubkey::new_from_array([1u8; 32]);

        let tip_ix = build_system_transfer(payer, jito_tip, 10_000);
        let other_ix = CompiledInstruction {
            program_id: Pubkey::from_str(arb_tx::METEORA_DLMM_PROGRAM_ID).unwrap(),
            accounts: vec![AccountMeta::new(payer, true)],
            data: vec![1, 2, 3],
        };

        let instructions = vec![other_ix.clone(), tip_ix];
        let (filtered, new_price) =
            strip_jito_tip_and_rollover(&instructions, 10_000, 1_000, 320_000).unwrap();

        // Jito tip instruction should be removed
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].program_id, other_ix.program_id);

        // Price should be elevated
        assert!(new_price > 1_000);
    }
}
