//! Address Lookup Table (ALT) manager.
//!
//! Encodes genuine Solana Address Lookup Table program instructions
//! (`CreateLookupTable` and `ExtendLookupTable`) using deterministic PDA derivation
//! and official binary layouts. Maintains an in-memory cache of ALT addresses.

use crate::{AccountMeta, CompiledInstruction};
use arb_constants::{ADDRESS_LOOKUP_TABLE_PROGRAM_ID, SYSTEM_PROGRAM_ID};
use solana_pubkey::Pubkey;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::RwLock;

/// Maximum addresses that can be appended in a single `ExtendLookupTable` instruction
/// without exceeding Solana MTU limits (30 addresses * 32 bytes = 960 bytes).
pub const MAX_ALT_EXTEND_ADDRESSES: usize = 30;

/// Derive the Address Lookup Table program ID.
pub fn address_lookup_table_program_id() -> Pubkey {
    Pubkey::from_str(ADDRESS_LOOKUP_TABLE_PROGRAM_ID).expect("valid ALT program ID")
}

/// Derive the Address Lookup Table address PDA and bump seed for a given authority and recent slot.
/// Seeds: `[authority.as_ref(), &recent_slot.to_le_bytes()]`
pub fn derive_lookup_table_address(authority: &Pubkey, recent_slot: u64) -> (Pubkey, u8) {
    let program_id = address_lookup_table_program_id();
    let slot_bytes = recent_slot.to_le_bytes();
    let seeds: &[&[u8]] = &[authority.as_ref(), &slot_bytes];
    Pubkey::find_program_address(seeds, &program_id)
}

/// Build genuine `CreateLookupTable` instruction.
///
/// Data layout:
/// - `[0..4]`: `instruction_index = 0u32` (CreateLookupTable)
/// - `[4..12]`: `recent_slot: u64` (little-endian)
/// - `[12..13]`: `bump_seed: u8`
///
/// Accounts:
/// 0. `lookup_table_address` (writable)
/// 1. `authority` (readonly, signer)
/// 2. `payer` (writable, signer)
/// 3. `system_program` (readonly)
pub fn create_lookup_table(
    authority: Pubkey,
    payer: Pubkey,
    recent_slot: u64,
) -> (Pubkey, CompiledInstruction) {
    let (alt_address, bump) = derive_lookup_table_address(&authority, recent_slot);
    let program_id = address_lookup_table_program_id();
    let system_program = Pubkey::new_from_array(SYSTEM_PROGRAM_ID);

    let mut data = Vec::with_capacity(13);
    data.extend_from_slice(&0u32.to_le_bytes()); // instruction_index = 0
    data.extend_from_slice(&recent_slot.to_le_bytes());
    data.push(bump);

    let instruction = CompiledInstruction {
        program_id,
        accounts: vec![
            AccountMeta::new(alt_address, false),
            AccountMeta::new_readonly(authority, true),
            AccountMeta::new(payer, true),
            AccountMeta::new_readonly(system_program, false),
        ],
        data,
    };

    (alt_address, instruction)
}

/// Build genuine `ExtendLookupTable` instruction.
///
/// Data layout:
/// - `[0..4]`: `instruction_index = 2u32` (ExtendLookupTable)
/// - `[4..12]`: count of addresses as `u64` (little-endian)
/// - `[12..]`: vector of 32-byte `Pubkey` addresses
///
/// Accounts:
/// 0. `lookup_table_address` (writable)
/// 1. `authority` (readonly, signer)
/// 2. Optional `payer` (writable, signer)
/// 3. Optional `system_program` (readonly, present if payer is present)
pub fn extend_lookup_table(
    lookup_table_address: Pubkey,
    authority: Pubkey,
    payer: Option<Pubkey>,
    addresses: &[Pubkey],
) -> CompiledInstruction {
    let program_id = address_lookup_table_program_id();

    let mut data = Vec::with_capacity(12 + addresses.len() * 32);
    data.extend_from_slice(&2u32.to_le_bytes()); // instruction_index = 2
    data.extend_from_slice(&(addresses.len() as u64).to_le_bytes());
    for addr in addresses {
        data.extend_from_slice(addr.as_ref());
    }

    let mut accounts = vec![
        AccountMeta::new(lookup_table_address, false),
        AccountMeta::new_readonly(authority, true),
    ];

    if let Some(payer_pubkey) = payer {
        let system_program = Pubkey::new_from_array(SYSTEM_PROGRAM_ID);
        accounts.push(AccountMeta::new(payer_pubkey, true));
        accounts.push(AccountMeta::new_readonly(system_program, false));
    }

    CompiledInstruction {
        program_id,
        accounts,
        data,
    }
}

/// In-memory cache of ALT pubkeys keyed by an arbitrary caller key.
#[derive(Debug, Default)]
pub struct AltManager {
    cache: RwLock<HashMap<String, Pubkey>>,
}

impl AltManager {
    pub fn new() -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Create a new ALT for `key`, caching its pubkey.
    /// Returns the derived table pubkey and instructions to create and optionally extend on chain.
    pub async fn create_alt(
        &self,
        key: &str,
        payer: Pubkey,
        addresses: &[Pubkey],
        recent_slot: u64,
    ) -> (Pubkey, Vec<CompiledInstruction>) {
        let (alt, create_ix) = create_lookup_table(payer, payer, recent_slot);
        let mut ixs = vec![create_ix];

        if !addresses.is_empty() {
            for chunk in addresses.chunks(MAX_ALT_EXTEND_ADDRESSES) {
                ixs.push(extend_lookup_table(alt, payer, Some(payer), chunk));
            }
        }

        if let Ok(mut guard) = self.cache.write() {
            guard.insert(key.to_string(), alt);
        }

        (alt, ixs)
    }

    /// Extend an existing ALT with `addresses`. Returns `ExtendLookupTable` instructions chunked by max size.
    pub async fn extend_alt(
        &self,
        alt: Pubkey,
        payer: Pubkey,
        addresses: &[Pubkey],
    ) -> Vec<CompiledInstruction> {
        let mut ixs = Vec::new();
        for chunk in addresses.chunks(MAX_ALT_EXTEND_ADDRESSES) {
            ixs.push(extend_lookup_table(alt, payer, Some(payer), chunk));
        }
        ixs
    }

    /// Look up a cached ALT pubkey by key.
    pub fn get_alt(&self, key: &str) -> Option<Pubkey> {
        self.cache.read().ok()?.get(key).copied()
    }

    /// Invalidate a cached entry.
    pub fn invalidate(&self, key: &str) {
        if let Ok(mut guard) = self.cache.write() {
            guard.remove(key);
        }
    }

    /// Number of cached entries (helper for tests).
    pub fn len(&self) -> usize {
        self.cache.read().map(|g| g.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_lookup_table_address() {
        let authority = Pubkey::new_unique();
        let slot = 250_000_000u64;
        let (pda, bump) = derive_lookup_table_address(&authority, slot);
        assert_ne!(pda, Pubkey::default());
        let _ = bump;
    }

    #[test]
    fn test_create_lookup_table_encoding() {
        let authority = Pubkey::new_unique();
        let payer = Pubkey::new_unique();
        let slot = 123_456_789u64;
        let (alt, ix) = create_lookup_table(authority, payer, slot);

        assert_eq!(ix.program_id, address_lookup_table_program_id());
        assert_eq!(ix.accounts.len(), 4);
        assert_eq!(ix.accounts[0].pubkey, alt);
        assert!(ix.accounts[0].is_writable);
        assert!(!ix.accounts[0].is_signer);

        assert_eq!(ix.accounts[1].pubkey, authority);
        assert!(!ix.accounts[1].is_writable);
        assert!(ix.accounts[1].is_signer);

        assert_eq!(ix.accounts[2].pubkey, payer);
        assert!(ix.accounts[2].is_writable);
        assert!(ix.accounts[2].is_signer);

        assert_eq!(
            ix.accounts[3].pubkey,
            Pubkey::new_from_array(SYSTEM_PROGRAM_ID)
        );
        assert!(!ix.accounts[3].is_writable);
        assert!(!ix.accounts[3].is_signer);

        assert_eq!(ix.data.len(), 13);
        assert_eq!(&ix.data[0..4], &[0, 0, 0, 0]); // instruction_index = 0
        assert_eq!(&ix.data[4..12], &slot.to_le_bytes());
    }

    #[test]
    fn test_extend_lookup_table_encoding() {
        let alt = Pubkey::new_unique();
        let authority = Pubkey::new_unique();
        let payer = Pubkey::new_unique();
        let addrs = vec![
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Pubkey::new_unique(),
        ];

        let ix = extend_lookup_table(alt, authority, Some(payer), &addrs);

        assert_eq!(ix.program_id, address_lookup_table_program_id());
        assert_eq!(ix.accounts.len(), 4);
        assert_eq!(ix.accounts[0].pubkey, alt);
        assert!(ix.accounts[0].is_writable);
        assert_eq!(ix.accounts[1].pubkey, authority);
        assert!(ix.accounts[1].is_signer);
        assert_eq!(ix.accounts[2].pubkey, payer);
        assert!(ix.accounts[2].is_signer);

        assert_eq!(ix.data.len(), 4 + 8 + 3 * 32);
        assert_eq!(&ix.data[0..4], &[2, 0, 0, 0]); // instruction_index = 2
        assert_eq!(&ix.data[4..12], &3u64.to_le_bytes()); // count = 3
        assert_eq!(&ix.data[12..44], addrs[0].as_ref());
        assert_eq!(&ix.data[44..76], addrs[1].as_ref());
        assert_eq!(&ix.data[76..108], addrs[2].as_ref());
    }

    #[test]
    fn test_cache_miss_then_hit_and_chunking() {
        let mgr = AltManager::new();
        assert!(mgr.get_alt("key1").is_none());

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let payer = Pubkey::new_unique();
        let addresses: Vec<Pubkey> = (0..65).map(|_| Pubkey::new_unique()).collect();

        let (alt, ixs) =
            rt.block_on(async { mgr.create_alt("key1", payer, &addresses, 999).await });

        assert_eq!(mgr.get_alt("key1"), Some(alt));
        // 1 create + ceil(65 / 30) = 1 + 3 = 4 instructions
        assert_eq!(ixs.len(), 4);
        assert_eq!(&ixs[0].data[0..4], &[0, 0, 0, 0]);
        assert_eq!(&ixs[1].data[0..4], &[2, 0, 0, 0]);
        assert_eq!(&ixs[2].data[0..4], &[2, 0, 0, 0]);
        assert_eq!(&ixs[3].data[0..4], &[2, 0, 0, 0]);

        mgr.invalidate("key1");
        assert!(mgr.get_alt("key1").is_none());
    }
}
