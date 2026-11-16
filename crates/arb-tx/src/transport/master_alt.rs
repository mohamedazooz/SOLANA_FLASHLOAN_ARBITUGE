//! Static Master Address Lookup Table (ALT) for the 3-leg Meteora DLMM meme arbitrage compiler.
//!
//! This module owns a single, operator-provisioned Mainnet ALT that holds every address
//! that repeats across transactions: program IDs, the DLMM event authority, memo program,
//! token programs, common mints (WSOL/USDC/USDT), wallet-specific bot ATAs, all eight
//! Jito tip accounts, and operator-configured static bridge-pool addresses.
//!
//! Newly discovered Meme A / Meme B pool accounts, meme mints, meme ATAs, and their Bin
//! Arrays are NEVER placed in this ALT — they are inlined per-transaction in the V0 message.
//!
//! # Safety invariants
//!
//! - ALT-loaded accounts can never be transaction signers.
//! - The ALT is provisioned once by the operator; it is never extended automatically
//!   because a new meme pool appeared.
//! - A controlled administrative refresh may replace the table, but never silently.
//! - The table must be activated on-chain before any compiled transaction references it.

use crate::{AccountMeta, CompiledInstruction};
use arb_constants::{
    ADDRESS_LOOKUP_TABLE_PROGRAM_ID, ASSOCIATED_TOKEN_PROGRAM_ID, COMPUTE_BUDGET_PROGRAM_ID,
    JITO_TIP_ACCOUNTS, SYSTEM_PROGRAM_ID, TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID, USDC_MINT,
    WSOL_MINT,
};
use solana_pubkey::Pubkey;
use std::collections::{BTreeSet, HashSet};
use std::str::FromStr;

/// Maximum number of addresses an ALT can hold (Solana protocol limit).
pub const MAX_ALT_ADDRESSES: usize = 256;

/// Meteora DLMM program ID on Mainnet.
const METEORA_DLMM_PROGRAM_ID_STR: &str = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";

/// Memo Program ID (required by DLMM swap2 instructions).
const MEMO_PROGRAM_ID_STR: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";

/// Expected mainnet DLMM event authority PDA.
/// Derived from `[b"__event_authority"]` under the Meteora DLMM program.
/// Expected: D1ZN9Wj1fRSUQfCjhvnu1hqDMT7hzjzBBpi12nVniYD6
pub const EXPECTED_DLMM_EVENT_AUTHORITY: &str = "D1ZN9Wj1fRSUQfCjhvnu1hqDMT7hzjzBBpi12nVniYD6";

/// USDT mint (SPL Token) on Mainnet.
pub const USDT_MINT_STR: &str = "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB";

/// Result of provisioning: how many addresses were added vs skipped (already present).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisionReport {
    pub addresses_added: usize,
    pub addresses_skipped: usize,
    pub total_addresses: usize,
    pub alt_address: Pubkey,
}

/// Parsed on-chain Address Lookup Table account (verified against the official
/// `solana-address-lookup-table-interface` `LOOKUP_TABLE_META_SIZE = 56` layout).
///
/// Data layout: `ProgramState` bincode enum (u32, 1 = LookupTable) + `LookupTableMeta`
/// (deactivation_slot u64, last_extended_slot u64, last_extended_slot_start_index u8,
/// authority Option<Pubkey> tag+32, _padding u16) = 56 bytes, then N x 32-byte addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAltAccount {
    pub deactivation_slot: u64,
    pub last_extended_slot: u64,
    pub last_extended_slot_start_index: u8,
    /// `Some(authority)` when the ALT carries an authority (modifiable), `None` if frozen.
    pub authority: Option<Pubkey>,
    /// Addresses stored in the ALT (in index order).
    pub addresses: Vec<Pubkey>,
}

/// Byte offset at which the serialized meta ends and the addresses begin.
pub const LOOKUP_TABLE_META_SIZE: usize = 56;

/// Parse a raw ALT account's data. Rejects malformed data, unparsable metas,
/// wrong sizes, duplicates, and >256 addresses.
pub fn parse_alt_account_data(data: &[u8]) -> Result<ParsedAltAccount, MasterAltError> {
    if data.len() < LOOKUP_TABLE_META_SIZE {
        return Err(MasterAltError::InvalidAddress(format!(
            "ALT account data too short: {} < {}",
            data.len(),
            LOOKUP_TABLE_META_SIZE
        )));
    }
    let raw_type = u32::from_le_bytes(data[0..4].try_into().expect("slice length checked"));
    match raw_type {
        0 => {
            return Err(MasterAltError::NotProvisioned(
                "ALT is uninitialized (ProgramState::Uninitialized)".into(),
            ));
        }
        1 => {} // ProgramState::LookupTable
        other => {
            return Err(MasterAltError::InvalidAddress(format!(
                "unknown ALT ProgramState discriminant {other}"
            )));
        }
    }
    let deactivation_slot =
        u64::from_le_bytes(data[4..12].try_into().expect("slice length checked"));
    let last_extended_slot =
        u64::from_le_bytes(data[12..20].try_into().expect("slice length checked"));
    let last_extended_slot_start_index = data[20];
    let authority_tag = data[21];
    let authority = match authority_tag {
        0 => None,
        1 => Some(Pubkey::new_from_array(
            data[22..54].try_into().expect("slice length checked"),
        )),
        other => {
            return Err(MasterAltError::InvalidAddress(format!(
                "ALT authority Option tag must be 0 or 1, got {other}"
            )));
        }
    };
    // 54..56 = _padding (must be zero? tolerated — not enforced)

    let addrs = &data[LOOKUP_TABLE_META_SIZE..];
    if !addrs.len().is_multiple_of(32) {
        return Err(MasterAltError::InvalidAddress(format!(
            "ALT addresses section size {} is not a multiple of 32",
            addrs.len()
        )));
    }
    let mut addresses = Vec::with_capacity(addrs.len() / 32);
    let mut seen = std::collections::HashSet::new();
    for chunk in addrs.as_chunks::<32>().0 {
        let addr = Pubkey::new_from_array(*chunk);
        if !seen.insert(addr) {
            return Err(MasterAltError::DuplicateAddress(addr.to_string()));
        }
        addresses.push(addr);
    }
    if addresses.len() > MAX_ALT_ADDRESSES {
        return Err(MasterAltError::CapacityExceeded {
            current: addresses.len(),
            max: MAX_ALT_ADDRESSES,
        });
    }

    Ok(ParsedAltAccount {
        deactivation_slot,
        last_extended_slot,
        last_extended_slot_start_index,
        authority,
        addresses,
    })
}

/// Error type for StaticMasterAlt operations.
#[derive(Debug, thiserror::Error)]
pub enum MasterAltError {
    #[error("ALT not yet provisioned: {0}")]
    NotProvisioned(String),
    #[error("ALT capacity exceeded: {current} addresses, max {max}")]
    CapacityExceeded { current: usize, max: usize },
    #[error("Duplicate address detected: {0}")]
    DuplicateAddress(String),
    #[error("Invalid address: {0}")]
    InvalidAddress(String),
    #[error("Required address missing from ALT: {0}")]
    MissingRequiredAddress(String),
    #[error("ALT authority mismatch: expected {expected}, got {got}")]
    AuthorityMismatch { expected: String, got: String },
    #[error(
        "ALT not activated: last_extended_slot {slot} is before required activation slot {required}"
    )]
    NotActivated { slot: u64, required: u64 },
    #[error("ALT account cannot be a signer: {0}")]
    AltAccountAsSigner(String),
    #[error("Serialization error: {0}")]
    Serialization(String),
    #[error("Refresh refused: existing ALT must not be silently replaced")]
    RefreshRefused,
}

/// Categorizes a static address for reporting and membership tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AddressCategory {
    MeteoraProgram,
    MeteoraEventAuthority,
    MemoProgram,
    SplTokenProgram,
    Token2022Program,
    AssociatedTokenProgram,
    SystemProgram,
    ComputeBudgetProgram,
    WsolMint,
    UsdcMint,
    UsdtMint,
    BotWsolAta,
    BotUsdcAta,
    BotUsdtAta,
    JitoTipAccount,
    StaticBridgePool,
}

/// A single verified static address with its category and expected program owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticAddressEntry {
    pub category: AddressCategory,
    pub address: Pubkey,
    /// The program that owns this account, for validation. None when the address
    /// is itself a program ID (self-owned).
    pub owner_program: Option<Pubkey>,
    /// Human-readable label for diagnostics.
    pub label: &'static str,
}

/// In-memory snapshot of the verified Static Master ALT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticMasterAlt {
    pub alt_address: Option<Pubkey>,
    pub authority: Option<Pubkey>,
    /// Cached addresses in deterministic insertion order.
    pub addresses: Vec<Pubkey>,
    /// Category lookup for each address (parallel to `addresses`).
    pub categories: Vec<AddressCategory>,
    /// Set for O(1) membership checks.
    pub address_set: HashSet<Pubkey>,
    /// On-chain activation slot (last_extended_slot). None until provisioned/verified.
    pub last_extended_slot: Option<u64>,
    /// Minimum slot at which the ALT is considered activated / warmed up.
    pub activation_slot: Option<u64>,
}

impl StaticMasterAlt {
    /// Create an empty, unprovisioned StaticMasterAlt.
    pub fn new() -> Self {
        Self {
            alt_address: None,
            authority: None,
            addresses: Vec::new(),
            categories: Vec::new(),
            address_set: HashSet::new(),
            last_extended_slot: None,
            activation_slot: None,
        }
    }

    /// Returns true if the ALT has been provisioned and contains at least the
    /// required static addresses.
    pub fn is_provisioned(&self) -> bool {
        self.alt_address.is_some() && !self.addresses.is_empty()
    }

    /// Returns true if the ALT has been activated on-chain (last_extended_slot >= activation_slot).
    pub fn is_activated(&self) -> bool {
        match (self.last_extended_slot, self.activation_slot) {
            (Some(actual), Some(required)) => actual >= required,
            _ => false,
        }
    }

    /// Number of addresses currently cached in the ALT.
    pub fn len(&self) -> usize {
        self.addresses.len()
    }

    pub fn is_empty(&self) -> bool {
        self.addresses.is_empty()
    }

    /// Deterministic membership check: returns the category if the address is present.
    pub fn category_of(&self, address: &Pubkey) -> Option<AddressCategory> {
        if !self.address_set.contains(address) {
            return None;
        }
        self.addresses
            .iter()
            .zip(self.categories.iter())
            .find_map(|(addr, cat)| if addr == address { Some(*cat) } else { None })
    }

    /// Returns true if `address` is a member of the ALT.
    pub fn contains(&self, address: &Pubkey) -> bool {
        self.address_set.contains(address)
    }

    /// Returns the index of `address` in the deterministic address vector, or None.
    pub fn index_of(&self, address: &Pubkey) -> Option<usize> {
        self.addresses.iter().position(|a| a == address)
    }

    /// Verify that all required static address categories are present.
    pub fn verify_required_present(&self) -> Result<(), MasterAltError> {
        let required: &[AddressCategory] = &[
            AddressCategory::MeteoraProgram,
            AddressCategory::MeteoraEventAuthority,
            AddressCategory::MemoProgram,
            AddressCategory::SplTokenProgram,
            AddressCategory::Token2022Program,
            AddressCategory::AssociatedTokenProgram,
            AddressCategory::SystemProgram,
            AddressCategory::ComputeBudgetProgram,
            AddressCategory::WsolMint,
            AddressCategory::UsdcMint,
            AddressCategory::UsdtMint,
        ];
        for cat in required {
            if !self.categories.contains(cat) {
                return Err(MasterAltError::MissingRequiredAddress(format!(
                    "required category {:?} not present in ALT",
                    cat
                )));
            }
        }
        Ok(())
    }

    /// Verify ALT capacity (256 addresses) has not been exceeded.
    pub fn verify_capacity(&self) -> Result<(), MasterAltError> {
        if self.addresses.len() > MAX_ALT_ADDRESSES {
            return Err(MasterAltError::CapacityExceeded {
                current: self.addresses.len(),
                max: MAX_ALT_ADDRESSES,
            });
        }
        Ok(())
    }

    /// Verify no ALT-loaded address is marked as a signer. ALT accounts cannot be signers.
    pub fn verify_no_signers(&self, signer_addresses: &[Pubkey]) -> Result<(), MasterAltError> {
        for signer in signer_addresses {
            if self.contains(signer) {
                return Err(MasterAltError::AltAccountAsSigner(format!(
                    "ALT-loaded address {signer} cannot be a transaction signer"
                )));
            }
        }
        Ok(())
    }

    /// Verify ALT activation state against the required activation slot.
    pub fn verify_activation(&self) -> Result<(), MasterAltError> {
        let Some(required) = self.activation_slot else {
            return Err(MasterAltError::NotActivated {
                slot: 0,
                required: 0,
            });
        };
        let Some(actual) = self.last_extended_slot else {
            return Err(MasterAltError::NotActivated { slot: 0, required });
        };
        if actual < required {
            return Err(MasterAltError::NotActivated {
                slot: actual,
                required,
            });
        }
        Ok(())
    }

    /// Add a static address if not already present. Returns true if added, false if skipped (duplicate).
    pub fn add_address(&mut self, entry: StaticAddressEntry) -> Result<bool, MasterAltError> {
        if self.address_set.contains(&entry.address) {
            return Ok(false); // skipped: duplicate
        }
        if self.addresses.len() >= MAX_ALT_ADDRESSES {
            return Err(MasterAltError::CapacityExceeded {
                current: self.addresses.len(),
                max: MAX_ALT_ADDRESSES,
            });
        }
        self.address_set.insert(entry.address);
        self.addresses.push(entry.address);
        self.categories.push(entry.category);
        Ok(true)
    }

    /// Build the full set of verified static addresses for the default operator configuration.
    ///
    /// This is the deterministic address inventory. Wallet-specific ATAs are derived from
    /// the configured payer and the actual token program per mint.
    pub fn default_static_entries(
        payer: &Pubkey,
        bot_wsol_ata: Option<Pubkey>,
        bot_usdc_ata: Option<Pubkey>,
        bot_usdt_ata: Option<Pubkey>,
        static_bridge_sol_usdc: Option<Pubkey>,
        static_bridge_sol_usdt: Option<Pubkey>,
    ) -> Result<Vec<StaticAddressEntry>, MasterAltError> {
        let _ = payer;
        let mut entries: Vec<StaticAddressEntry> = Vec::new();

        let add = |entries: &mut Vec<StaticAddressEntry>,
                   addr_str: &str,
                   category: AddressCategory,
                   owner: Option<Pubkey>,
                   label: &'static str|
         -> Result<(), MasterAltError> {
            let addr = Pubkey::from_str(addr_str)
                .map_err(|e| MasterAltError::InvalidAddress(format!("{addr_str}: {e}")))?;
            entries.push(StaticAddressEntry {
                category,
                address: addr,
                owner_program: owner,
                label,
            });
            Ok(())
        };

        // --- Meteora ---
        let meteora_program = Pubkey::from_str(METEORA_DLMM_PROGRAM_ID_STR)
            .map_err(|e| MasterAltError::InvalidAddress(format!("meteora program: {e}")))?;
        add(
            &mut entries,
            METEORA_DLMM_PROGRAM_ID_STR,
            AddressCategory::MeteoraProgram,
            None,
            "Meteora DLMM program",
        )?;

        // Derive the event authority PDA using the official seed and compare to expected.
        let event_authority = derive_meteora_event_authority();
        let expected_event_authority =
            Pubkey::from_str(EXPECTED_DLMM_EVENT_AUTHORITY).map_err(|e| {
                MasterAltError::InvalidAddress(format!("expected event authority: {e}"))
            })?;
        if event_authority != expected_event_authority {
            return Err(MasterAltError::InvalidAddress(format!(
                "derived event authority {event_authority} does not match expected mainnet address {expected_event_authority}"
            )));
        }
        entries.push(StaticAddressEntry {
            category: AddressCategory::MeteoraEventAuthority,
            address: event_authority,
            owner_program: Some(meteora_program),
            label: "Meteora DLMM event authority PDA",
        });

        add(
            &mut entries,
            MEMO_PROGRAM_ID_STR,
            AddressCategory::MemoProgram,
            None,
            "Memo program",
        )?;

        // --- Token infrastructure ---
        let spl_token = Pubkey::from_str(TOKEN_PROGRAM_ID)
            .map_err(|e| MasterAltError::InvalidAddress(format!("spl token program: {e}")))?;
        let token_2022 = Pubkey::from_str(TOKEN_2022_PROGRAM_ID)
            .map_err(|e| MasterAltError::InvalidAddress(format!("token-2022 program: {e}")))?;
        let ata_program = Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM_ID)
            .map_err(|e| MasterAltError::InvalidAddress(format!("ata program: {e}")))?;
        let _ = (token_2022, ata_program);
        let compute_budget = Pubkey::new_from_array(COMPUTE_BUDGET_PROGRAM_ID);
        let system_program = Pubkey::new_from_array(SYSTEM_PROGRAM_ID);

        add(
            &mut entries,
            TOKEN_PROGRAM_ID,
            AddressCategory::SplTokenProgram,
            None,
            "SPL Token program",
        )?;
        add(
            &mut entries,
            TOKEN_2022_PROGRAM_ID,
            AddressCategory::Token2022Program,
            None,
            "Token-2022 program",
        )?;
        add(
            &mut entries,
            ASSOCIATED_TOKEN_PROGRAM_ID,
            AddressCategory::AssociatedTokenProgram,
            None,
            "Associated Token program",
        )?;
        entries.push(StaticAddressEntry {
            category: AddressCategory::SystemProgram,
            address: system_program,
            owner_program: None,
            label: "System program",
        });
        entries.push(StaticAddressEntry {
            category: AddressCategory::ComputeBudgetProgram,
            address: compute_budget,
            owner_program: None,
            label: "Compute Budget program",
        });

        // --- Common mints ---
        let wsol_mint = Pubkey::from_str(WSOL_MINT)
            .map_err(|e| MasterAltError::InvalidAddress(format!("wsol mint: {e}")))?;
        let usdc_mint = Pubkey::from_str(USDC_MINT)
            .map_err(|e| MasterAltError::InvalidAddress(format!("usdc mint: {e}")))?;
        let usdt_mint = Pubkey::from_str(USDT_MINT_STR)
            .map_err(|e| MasterAltError::InvalidAddress(format!("usdt mint: {e}")))?;
        entries.push(StaticAddressEntry {
            category: AddressCategory::WsolMint,
            address: wsol_mint,
            owner_program: Some(spl_token),
            label: "WSOL mint",
        });
        entries.push(StaticAddressEntry {
            category: AddressCategory::UsdcMint,
            address: usdc_mint,
            owner_program: Some(spl_token),
            label: "USDC mint",
        });
        entries.push(StaticAddressEntry {
            category: AddressCategory::UsdtMint,
            address: usdt_mint,
            owner_program: Some(spl_token),
            label: "USDT mint",
        });

        // --- Wallet-specific ATAs (derived from the actual token program per mint) ---
        if let Some(ata) = bot_wsol_ata {
            entries.push(StaticAddressEntry {
                category: AddressCategory::BotWsolAta,
                address: ata,
                owner_program: Some(spl_token),
                label: "Bot WSOL ATA",
            });
        }
        if let Some(ata) = bot_usdc_ata {
            entries.push(StaticAddressEntry {
                category: AddressCategory::BotUsdcAta,
                address: ata,
                owner_program: Some(spl_token),
                label: "Bot USDC ATA",
            });
        }
        if let Some(ata) = bot_usdt_ata {
            entries.push(StaticAddressEntry {
                category: AddressCategory::BotUsdtAta,
                address: ata,
                owner_program: Some(spl_token),
                label: "Bot USDT ATA",
            });
        }

        // --- Jito tip accounts (all eight verified official accounts) ---
        for (i, tip_str) in JITO_TIP_ACCOUNTS.iter().enumerate() {
            let tip_addr = Pubkey::from_str(tip_str).map_err(|e| {
                MasterAltError::InvalidAddress(format!("jito tip account {i} {tip_str}: {e}"))
            })?;
            entries.push(StaticAddressEntry {
                category: AddressCategory::JitoTipAccount,
                address: tip_addr,
                owner_program: Some(system_program),
                label: "Jito tip account",
            });
        }

        // --- Static bridge accounts (operator-configured, verified) ---
        if let Some(bridge) = static_bridge_sol_usdc {
            entries.push(StaticAddressEntry {
                category: AddressCategory::StaticBridgePool,
                address: bridge,
                owner_program: Some(meteora_program),
                label: "Static bridge pool SOL/USDC",
            });
        }
        if let Some(bridge) = static_bridge_sol_usdt {
            entries.push(StaticAddressEntry {
                category: AddressCategory::StaticBridgePool,
                address: bridge,
                owner_program: Some(meteora_program),
                label: "Static bridge pool SOL/USDT",
            });
        }

        Ok(entries)
    }

    /// Populate the ALT from a static entry list, deduplicating and validating.
    /// Returns a ProvisionReport describing added vs skipped addresses.
    pub fn populate_from_entries(
        &mut self,
        alt_address: Pubkey,
        authority: Pubkey,
        entries: Vec<StaticAddressEntry>,
        activation_slot: u64,
    ) -> Result<ProvisionReport, MasterAltError> {
        self.alt_address = Some(alt_address);
        self.authority = Some(authority);
        self.activation_slot = Some(activation_slot);
        // last_extended_slot is set when on-chain activation is verified.

        let mut added = 0usize;
        let mut skipped = 0usize;
        let mut seen: BTreeSet<Pubkey> = BTreeSet::new();

        for entry in entries {
            if !seen.insert(entry.address) {
                // Duplicate within the entry list itself.
                skipped += 1;
                continue;
            }
            if self.address_set.contains(&entry.address) {
                skipped += 1;
                continue;
            }
            if self.addresses.len() >= MAX_ALT_ADDRESSES {
                return Err(MasterAltError::CapacityExceeded {
                    current: self.addresses.len(),
                    max: MAX_ALT_ADDRESSES,
                });
            }
            self.address_set.insert(entry.address);
            self.addresses.push(entry.address);
            self.categories.push(entry.category);
            added += 1;
        }

        self.verify_required_present()?;
        self.verify_capacity()?;

        Ok(ProvisionReport {
            addresses_added: added,
            addresses_skipped: skipped,
            total_addresses: self.addresses.len(),
            alt_address,
        })
    }

    /// Mark the ALT as activated on-chain at `last_extended_slot`.
    /// Call only after confirming the table exists and is usable on chain.
    pub fn mark_activated(&mut self, last_extended_slot: u64) {
        self.last_extended_slot = Some(last_extended_slot);
    }

    /// Adopt a parsed on-chain ALT snapshot for read-only verification.
    ///
    /// This does NOT extend or modify any on-chain state. Categories are not
    /// reconstructed (chain data carries none), so `categories` stays empty and
    /// `verify_required_present()` must not be called on the result.
    pub fn from_onchain_readonly(
        alt_address: Pubkey,
        parsed: ParsedAltAccount,
        expected_authority: Option<Pubkey>,
        expected_min_last_extended_slot: Option<u64>,
    ) -> Result<Self, MasterAltError> {
        // A deactivated table (deactivation_slot != Slot::MAX) is unusable.
        if parsed.deactivation_slot != u64::MAX {
            return Err(MasterAltError::NotActivated {
                slot: parsed.deactivation_slot,
                required: u64::MAX,
            });
        }
        if let Some(expected) = expected_authority {
            match parsed.authority {
                Some(actual) if actual == expected => {}
                Some(actual) => {
                    return Err(MasterAltError::AuthorityMismatch {
                        expected: expected.to_string(),
                        got: actual.to_string(),
                    });
                }
                None => {
                    return Err(MasterAltError::AuthorityMismatch {
                        expected: expected.to_string(),
                        got: "<frozen ALT: no authority>".to_string(),
                    });
                }
            }
        }
        if let Some(min_slot) = expected_min_last_extended_slot {
            if parsed.last_extended_slot < min_slot {
                return Err(MasterAltError::NotActivated {
                    slot: parsed.last_extended_slot,
                    required: min_slot,
                });
            }
        }

        let mut alt = Self::new();
        alt.alt_address = Some(alt_address);
        alt.authority = parsed.authority;
        alt.addresses = parsed.addresses.clone();
        alt.address_set = parsed.addresses.iter().copied().collect();
        alt.last_extended_slot = Some(parsed.last_extended_slot);
        // All entries are usable once the current slot is strictly greater
        // than the last extended slot.
        alt.activation_slot = Some(parsed.last_extended_slot.saturating_add(1));
        alt.verify_capacity()?;
        Ok(alt)
    }

    /// Verify that the ALT is warm: the current slot must be strictly greater
    /// than `last_extended_slot` for all entries to be usable.
    pub fn verify_warmup(&self, current_slot: u64) -> Result<(), MasterAltError> {
        let Some(last) = self.last_extended_slot else {
            return Err(MasterAltError::NotActivated {
                slot: 0,
                required: current_slot,
            });
        };
        if current_slot <= last {
            return Err(MasterAltError::NotActivated {
                slot: last,
                required: current_slot,
            });
        }
        Ok(())
    }

    /// Number of usable addresses (all of them when warm).
    pub fn usable_addresses_len(&self, current_slot: u64) -> Result<usize, MasterAltError> {
        self.verify_warmup(current_slot)?;
        Ok(self.addresses.len())
    }
}

impl Default for StaticMasterAlt {
    fn default() -> Self {
        Self::new()
    }
}

/// Derive the standard Anchor `__event_authority` PDA for the DLMM program.
/// Official seeds from MeteoraAg/dlmm-sdk: `[b"__event_authority"]`.
fn derive_meteora_event_authority() -> Pubkey {
    let program_id = Pubkey::from_str(METEORA_DLMM_PROGRAM_ID_STR).expect("valid meteora program");
    let seeds: &[&[u8]] = &[b"__event_authority"];
    let (pda, _) = Pubkey::find_program_address(seeds, &program_id);
    pda
}

/// Derive an Associated Token Address for a wallet and mint using the given token program.
pub fn derive_ata(wallet: &Pubkey, mint: &Pubkey, token_program: &Pubkey) -> Pubkey {
    let ata_program = Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM_ID).expect("valid ata program");
    let seeds = &[wallet.as_ref(), token_program.as_ref(), mint.as_ref()];
    let (ata, _) = Pubkey::find_program_address(seeds, &ata_program);
    ata
}

/// Build the on-chain instructions to provision (create + extend) the Static Master ALT.
///
/// This is a one-time operator setup helper, NOT a transaction hot-path operation.
///
/// Behavior:
/// - Creates the ALT when absent.
/// - Extends it using valid bounded batches (max MAX_ALT_EXTEND_ADDRESSES per extend).
/// - Deduplicates addresses.
/// - Verifies authority.
/// - Reports addresses added and skipped.
/// - Requires on-chain activation before use (returns the ALT address for the operator
///   to confirm activation; does not activate automatically).
/// - Never extends the ALT automatically because a newly discovered meme pool appeared.
///
/// Returns `(alt_address, instructions, report)`.
pub fn build_or_extend_master_alt_ixs(
    payer: &Pubkey,
    authority: &Pubkey,
    recent_slot: u64,
    entries: Vec<StaticAddressEntry>,
) -> Result<(Pubkey, Vec<CompiledInstruction>, ProvisionReport), MasterAltError> {
    let program_id = Pubkey::from_str(ADDRESS_LOOKUP_TABLE_PROGRAM_ID)
        .map_err(|e| MasterAltError::InvalidAddress(format!("alt program: {e}")))?;

    // Derive the deterministic ALT PDA: [authority, recent_slot]
    let slot_bytes = recent_slot.to_le_bytes();
    let seeds: &[&[u8]] = &[authority.as_ref(), &slot_bytes];
    let (alt_address, bump) = Pubkey::find_program_address(seeds, &program_id);

    // Deduplicate entries by address, preserving first-seen order.
    let mut unique_entries: Vec<StaticAddressEntry> = Vec::new();
    let mut seen: HashSet<Pubkey> = HashSet::new();
    let mut duplicates_skipped = 0usize;
    for entry in entries {
        if !seen.insert(entry.address) {
            duplicates_skipped += 1;
            continue;
        }
        unique_entries.push(entry);
    }

    if unique_entries.len() > MAX_ALT_ADDRESSES {
        return Err(MasterAltError::CapacityExceeded {
            current: unique_entries.len(),
            max: MAX_ALT_ADDRESSES,
        });
    }

    // Build CreateLookupTable instruction.
    let mut data = Vec::with_capacity(13);
    data.extend_from_slice(&0u32.to_le_bytes()); // instruction_index = 0 (CreateLookupTable)
    data.extend_from_slice(&recent_slot.to_le_bytes());
    data.push(bump);

    let system_program = Pubkey::new_from_array(SYSTEM_PROGRAM_ID);
    let mut instructions: Vec<CompiledInstruction> = vec![CompiledInstruction {
        program_id,
        accounts: vec![
            AccountMeta::new(alt_address, false),
            AccountMeta::new_readonly(*authority, true), // authority signer
            AccountMeta::new(*payer, true),              // payer signer
            AccountMeta::new_readonly(system_program, false),
        ],
        data,
    }];

    // Chunked ExtendLookupTable instructions (bounded batches).
    let max_per_extend = crate::alt::MAX_ALT_EXTEND_ADDRESSES;
    let addresses: Vec<Pubkey> = unique_entries.iter().map(|e| e.address).collect();
    for chunk in addresses.chunks(max_per_extend) {
        let mut ext_data = Vec::with_capacity(12 + chunk.len() * 32);
        ext_data.extend_from_slice(&2u32.to_le_bytes()); // instruction_index = 2 (ExtendLookupTable)
        ext_data.extend_from_slice(&(chunk.len() as u64).to_le_bytes());
        for addr in chunk {
            ext_data.extend_from_slice(addr.as_ref());
        }
        let mut ext_accounts = vec![
            AccountMeta::new(alt_address, false),
            AccountMeta::new_readonly(*authority, true),
        ];
        ext_accounts.push(AccountMeta::new(*payer, true));
        ext_accounts.push(AccountMeta::new_readonly(system_program, false));
        instructions.push(CompiledInstruction {
            program_id,
            accounts: ext_accounts,
            data: ext_data,
        });
    }

    let report = ProvisionReport {
        addresses_added: addresses.len(),
        addresses_skipped: duplicates_skipped,
        total_addresses: addresses.len(),
        alt_address,
    };

    Ok((alt_address, instructions, report))
}

/// Administrative refresh: replace the cached address set with a new verified list.
///
/// Requires explicit operator consent (`force_replace=true`). Never silently replaces
/// an existing ALT. Returns the new report.
pub fn administrative_refresh(
    alt: &mut StaticMasterAlt,
    force_replace: bool,
    new_entries: Vec<StaticAddressEntry>,
    activation_slot: u64,
) -> Result<ProvisionReport, MasterAltError> {
    if alt.is_provisioned() && !force_replace {
        return Err(MasterAltError::RefreshRefused);
    }
    let alt_address = alt
        .alt_address
        .ok_or_else(|| MasterAltError::NotProvisioned("no ALT address to refresh".into()))?;
    let authority = alt
        .authority
        .ok_or_else(|| MasterAltError::NotProvisioned("no ALT authority".into()))?;

    // Reset and repopulate.
    alt.addresses.clear();
    alt.categories.clear();
    alt.address_set.clear();
    alt.last_extended_slot = None;

    alt.populate_from_entries(alt_address, authority, new_entries, activation_slot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arb_constants::TOKEN_PROGRAM_ID as SPL;

    fn test_payer() -> Pubkey {
        Pubkey::new_from_array([1u8; 32])
    }

    fn test_authority() -> Pubkey {
        Pubkey::new_from_array([2u8; 32])
    }

    fn make_entry(
        addr_str: &str,
        category: AddressCategory,
        owner: Option<&str>,
        label: &'static str,
    ) -> StaticAddressEntry {
        let addr = Pubkey::from_str(addr_str).unwrap();
        let owner_program = owner.map(|s| Pubkey::from_str(s).unwrap());
        StaticAddressEntry {
            category,
            address: addr,
            owner_program,
            label,
        }
    }

    #[test]
    fn test_parse_real_mainnet_meteora_alt_snapshot() {
        // Read-only snapshot of the real Mainnet Meteora DLMM ALT
        // `JA5F83HUK9L78Y12TRLCsJZbu3Tv8pCK1GfK8mVNp1sz` (public on-chain data,
        // fetched via RPC getAccountInfo). Owner: AddressLookupTable program.
        let alt_hex = "01000000ffffffffffffffff239f771600000000060148f459ea35f02146c458cac9763f4b11057255cbee3c4e45adeec3db8b2a1b37000006ddf6e1d765a193d9cbe146ceeb79ac1cb485ed5f5b37913a8cf5857eff00a906ddf6e1ee758fde18425dbce46ccddab61afc4d83b90d27febdf928d8a18bfc0000000000000000000000000000000000000000000000000000000000000000b270d67fa98c51cf0213051358962baf35742bed59c9d9445e9c0d0c85c7cd91054a535a992921064d24e87160da387c7c35b5ddbc92bb81e41fa8404105448d069b8857feab8184fb687f634618c035dac439dc1aeb3b5598a0f0000000000103c56ebfdfd0aadeb78100e9fd6616c9fd243d0a66d34699d4934ec4284737c9";
        let data: Vec<u8> = (0..alt_hex.len() / 2)
            .map(|i| u8::from_str_radix(&alt_hex[2 * i..2 * i + 2], 16).unwrap())
            .collect();
        let parsed = parse_alt_account_data(&data).unwrap();
        assert_eq!(parsed.deactivation_slot, u64::MAX, "not deactivated");
        assert_eq!(parsed.last_extended_slot, 376_938_275);
        assert_eq!(parsed.last_extended_slot_start_index, 6);
        assert_eq!(
            parsed.authority.as_ref().map(|a| a.to_string()),
            Some("5unTfT2kssBuNvHPY6LbJfJpLqEcdMxGYLWHwShaeTLi".to_string())
        );
        assert_eq!(parsed.addresses.len(), 7);

        let addr = |i: usize| parsed.addresses[i];
        assert_eq!(
            addr(0).to_string(),
            "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
        );
        assert_eq!(
            addr(1).to_string(),
            "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"
        );
        assert_eq!(addr(2).to_string(), "11111111111111111111111111111111");
        assert_eq!(addr(3).to_string(), EXPECTED_DLMM_EVENT_AUTHORITY);
        assert_eq!(
            addr(4).to_string(),
            "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr"
        );
        assert_eq!(
            addr(5).to_string(),
            "So11111111111111111111111111111111111111112"
        );
        assert_eq!(
            addr(6).to_string(),
            "FiqLtg6z5ED2TR3hWrRaB6uRLcPqXFZpSVQutSznGfS"
        );

        // Warmup: usable from last_extended_slot + 1
        let alt = StaticMasterAlt::from_onchain_readonly(
            Pubkey::from_str("JA5F83HUK9L78Y12TRLCsJZbu3Tv8pCK1GfK8mVNp1sz").unwrap(),
            parsed,
            None,
            Some(200_000_000),
        )
        .unwrap();
        assert!(alt.contains(&Pubkey::from_str(EXPECTED_DLMM_EVENT_AUTHORITY).unwrap()));
        assert!(
            alt.contains(&Pubkey::from_str("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA").unwrap())
        );
        assert_eq!(alt.usable_addresses_len(376_938_276).unwrap(), 7);
        assert!(alt.verify_warmup(376_938_276).is_ok());
        assert!(alt.verify_warmup(376_938_275).is_err());
        assert!(alt.verify_warmup(376_938_274).is_err());

        // Capacity and membership sanity
        assert_eq!(alt.len(), 7);
        assert_eq!(alt.index_of(&alt.addresses[0]), Some(0));
        assert_eq!(alt.index_of(&Pubkey::from_str(WSOL_MINT).unwrap()), Some(5));
    }

    #[test]
    fn test_parse_alt_rejects_malformed_accounts() {
        // Too short
        assert!(parse_alt_account_data(&[1, 0, 0]).is_err());
        // Uninitialized (ProgramState variant 0)
        let uninit = [0u8; 56];
        assert!(parse_alt_account_data(&uninit).is_err());
        // Unknown variant
        let mut bad = [0u8; 56];
        bad[0] = 9;
        assert!(parse_alt_account_data(&bad).is_err());
        // Non-32-byte-aligned addresses section
        let mut misaligned = [0u8; 56 + 16];
        misaligned[0] = 1;
        misaligned[20] = 1;
        misaligned[21] = 1;
        assert!(parse_alt_account_data(&misaligned).is_err());
        // Bad authority tag
        let mut bad_tag = [0u8; 56];
        bad_tag[0] = 1;
        bad_tag[20] = 1;
        bad_tag[21] = 7;
        assert!(parse_alt_account_data(&bad_tag).is_err());
        // Duplicate addresses rejected
        let mut dup = [0u8; 56 + 64];
        dup[0] = 1;
        dup[20] = 1;
        dup[21] = 1;
        for b in dup[56..56 + 32].iter_mut() {
            *b = 9;
        }
        for b in dup[88..88 + 32].iter_mut() {
            *b = 9;
        }
        assert!(parse_alt_account_data(&dup).is_err());
    }

    #[test]
    fn test_event_authority_matches_expected_mainnet() {
        let derived = derive_meteora_event_authority();
        let expected = Pubkey::from_str(EXPECTED_DLMM_EVENT_AUTHORITY).unwrap();
        assert_eq!(
            derived, expected,
            "derived event authority must equal the verified mainnet address {EXPECTED_DLMM_EVENT_AUTHORITY}"
        );
    }

    #[test]
    fn test_default_static_entries_include_all_required_categories() {
        let payer = test_payer();
        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let usdc = Pubkey::from_str(USDC_MINT).unwrap();
        let usdt = Pubkey::from_str(USDT_MINT_STR).unwrap();
        let bot_wsol_ata = derive_ata(&payer, &wsol, &Pubkey::from_str(SPL).unwrap());
        let bot_usdc_ata = derive_ata(&payer, &usdc, &Pubkey::from_str(SPL).unwrap());
        let bot_usdt_ata = derive_ata(&payer, &usdt, &Pubkey::from_str(SPL).unwrap());

        let entries = StaticMasterAlt::default_static_entries(
            &payer,
            Some(bot_wsol_ata),
            Some(bot_usdc_ata),
            Some(bot_usdt_ata),
            None,
            None,
        )
        .unwrap();

        let cats: HashSet<AddressCategory> = entries.iter().map(|e| e.category).collect();
        assert!(cats.contains(&AddressCategory::MeteoraProgram));
        assert!(cats.contains(&AddressCategory::MeteoraEventAuthority));
        assert!(cats.contains(&AddressCategory::MemoProgram));
        assert!(cats.contains(&AddressCategory::SplTokenProgram));
        assert!(cats.contains(&AddressCategory::Token2022Program));
        assert!(cats.contains(&AddressCategory::AssociatedTokenProgram));
        assert!(cats.contains(&AddressCategory::SystemProgram));
        assert!(cats.contains(&AddressCategory::ComputeBudgetProgram));
        assert!(cats.contains(&AddressCategory::WsolMint));
        assert!(cats.contains(&AddressCategory::UsdcMint));
        assert!(cats.contains(&AddressCategory::UsdtMint));
        assert!(cats.contains(&AddressCategory::BotWsolAta));
        assert!(cats.contains(&AddressCategory::BotUsdcAta));
        assert!(cats.contains(&AddressCategory::BotUsdtAta));
        assert!(cats.contains(&AddressCategory::JitoTipAccount));
        // Exactly 8 Jito tip accounts
        assert_eq!(
            entries
                .iter()
                .filter(|e| matches!(e.category, AddressCategory::JitoTipAccount))
                .count(),
            8
        );
    }

    #[test]
    fn test_default_static_entries_excludes_meme_pool_accounts() {
        let payer = test_payer();
        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let bot_wsol_ata = derive_ata(&payer, &wsol, &Pubkey::from_str(SPL).unwrap());

        let entries = StaticMasterAlt::default_static_entries(
            &payer,
            Some(bot_wsol_ata),
            None,
            None,
            None,
            None,
        )
        .unwrap();

        // No bridge pools without explicit configuration
        assert_eq!(
            entries
                .iter()
                .filter(|e| matches!(e.category, AddressCategory::StaticBridgePool))
                .count(),
            0
        );
    }

    #[test]
    fn test_populate_from_entries_deduplicates_and_reports() {
        let mut alt = StaticMasterAlt::new();
        let payer = test_payer();
        let authority = test_authority();
        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let usdc = Pubkey::from_str(USDC_MINT).unwrap();
        let spl = Pubkey::from_str(SPL).unwrap();

        // Add entries covering all required categories so verify_required_present passes.
        let entries = vec![
            StaticAddressEntry {
                category: AddressCategory::WsolMint,
                address: wsol,
                owner_program: Some(spl),
                label: "wsol",
            },
            StaticAddressEntry {
                category: AddressCategory::WsolMint,
                address: wsol,
                owner_program: Some(spl),
                label: "wsol dup", // duplicate
            },
            StaticAddressEntry {
                category: AddressCategory::UsdcMint,
                address: usdc,
                owner_program: Some(spl),
                label: "usdc",
            },
            StaticAddressEntry {
                category: AddressCategory::UsdtMint,
                address: Pubkey::from_str(USDT_MINT_STR).unwrap(),
                owner_program: Some(spl),
                label: "usdt",
            },
            StaticAddressEntry {
                category: AddressCategory::MeteoraProgram,
                address: Pubkey::from_str(METEORA_DLMM_PROGRAM_ID_STR).unwrap(),
                owner_program: None,
                label: "meteora",
            },
            StaticAddressEntry {
                category: AddressCategory::MeteoraEventAuthority,
                address: Pubkey::from_str("D1ZN9Wj1fRSUQfCjhvnu1hqDMT7hzjzBBpi12nVniYD6").unwrap(),
                owner_program: Some(Pubkey::from_str(METEORA_DLMM_PROGRAM_ID_STR).unwrap()),
                label: "event_authority",
            },
            StaticAddressEntry {
                category: AddressCategory::MemoProgram,
                address: Pubkey::from_str(MEMO_PROGRAM_ID_STR).unwrap(),
                owner_program: None,
                label: "memo",
            },
            StaticAddressEntry {
                category: AddressCategory::SplTokenProgram,
                address: spl,
                owner_program: None,
                label: "spl_token",
            },
            StaticAddressEntry {
                category: AddressCategory::Token2022Program,
                address: Pubkey::from_str(TOKEN_2022_PROGRAM_ID).unwrap(),
                owner_program: None,
                label: "token2022",
            },
            StaticAddressEntry {
                category: AddressCategory::AssociatedTokenProgram,
                address: Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM_ID).unwrap(),
                owner_program: None,
                label: "ata",
            },
            StaticAddressEntry {
                category: AddressCategory::SystemProgram,
                address: Pubkey::new_from_array([3u8; 32]),
                owner_program: None,
                label: "system",
            },
            StaticAddressEntry {
                category: AddressCategory::ComputeBudgetProgram,
                address: Pubkey::new_from_array([4u8; 32]),
                owner_program: None,
                label: "compute_budget",
            },
        ];

        let report = alt
            .populate_from_entries(payer, authority, entries, 100_000)
            .unwrap();
        // WSOL (added once, 1 dup skipped), USDC, USDT, and 8 required categories = 11 unique added, 1 skipped
        assert_eq!(report.addresses_added, 11);
        assert_eq!(report.addresses_skipped, 1);
        assert_eq!(report.total_addresses, 11);
        assert_eq!(alt.len(), 11);
        assert!(alt.contains(&wsol));
        assert!(alt.contains(&usdc));
        assert!(alt.is_provisioned());
    }

    #[test]
    fn test_capacity_limit_enforced() {
        let mut alt = StaticMasterAlt::new();
        let payer = test_payer();
        let authority = test_authority();
        let mut entries: Vec<StaticAddressEntry> = Vec::new();
        // Add enough unique addresses to exceed capacity.
        // new_from_array([i as u8]) wraps at 256, so use new_unique() for filler.
        for _ in 0..(MAX_ALT_ADDRESSES + 5) {
            entries.push(StaticAddressEntry {
                category: AddressCategory::JitoTipAccount,
                address: Pubkey::new_unique(),
                owner_program: None,
                label: "filler",
            });
        }
        let err = alt
            .populate_from_entries(payer, authority, entries, 100_000)
            .unwrap_err();
        assert!(matches!(err, MasterAltError::CapacityExceeded { .. }));
    }

    #[test]
    fn test_verify_no_signers_rejects_alt_signer() {
        let mut alt = StaticMasterAlt::new();
        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        alt.addresses.push(wsol);
        alt.address_set.insert(wsol);

        let err = alt.verify_no_signers(&[wsol]).unwrap_err();
        assert!(matches!(err, MasterAltError::AltAccountAsSigner { .. }));

        // Non-signer that is in ALT is fine
        let ok = alt.verify_no_signers(&[Pubkey::new_unique()]);
        assert!(ok.is_ok());
    }

    #[test]
    fn test_activation_verification() {
        let mut alt = StaticMasterAlt::new();
        // Not activated: no slots
        assert!(alt.verify_activation().is_err());

        alt.last_extended_slot = Some(50);
        alt.activation_slot = Some(100);
        assert!(alt.verify_activation().is_err()); // 50 < 100

        alt.last_extended_slot = Some(150);
        assert!(alt.verify_activation().is_ok()); // 150 >= 100
    }

    #[test]
    fn test_membership_and_index() {
        let mut alt = StaticMasterAlt::new();
        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let usdc = Pubkey::from_str(USDC_MINT).unwrap();
        alt.addresses.push(wsol);
        alt.addresses.push(usdc);
        alt.address_set.insert(wsol);
        alt.address_set.insert(usdc);
        alt.categories.push(AddressCategory::WsolMint);
        alt.categories.push(AddressCategory::UsdcMint);

        assert!(alt.contains(&wsol));
        assert!(!alt.contains(&Pubkey::new_unique()));
        assert_eq!(alt.index_of(&wsol), Some(0));
        assert_eq!(alt.category_of(&usdc), Some(AddressCategory::UsdcMint));
        assert_eq!(alt.category_of(&Pubkey::new_unique()), None);
    }

    #[test]
    fn test_build_or_extend_master_alt_ixs_structure() {
        let payer = test_payer();
        let authority = test_authority();
        let _wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let _usdc = Pubkey::from_str(USDC_MINT).unwrap();
        let entries = vec![
            make_entry(WSOL_MINT, AddressCategory::WsolMint, Some(SPL), "wsol"),
            make_entry(USDC_MINT, AddressCategory::UsdcMint, Some(SPL), "usdc"),
        ];

        let (alt_address, ixs, report) =
            build_or_extend_master_alt_ixs(&payer, &authority, 999_999, entries).unwrap();

        assert_eq!(report.addresses_added, 2);
        assert_eq!(report.total_addresses, 2);
        assert_eq!(alt_address, alt_address); // deterministic

        // First instruction must be CreateLookupTable
        assert_eq!(
            ixs[0].program_id,
            Pubkey::from_str(ADDRESS_LOOKUP_TABLE_PROGRAM_ID).unwrap()
        );
        assert_eq!(ixs[0].data[0..4], [0, 0, 0, 0]); // instruction_index = 0
        assert!(ixs[0].accounts.iter().any(|a| a.pubkey == alt_address));
        assert!(
            ixs[0]
                .accounts
                .iter()
                .any(|a| a.pubkey == authority && a.is_signer)
        );
        assert!(
            ixs[0]
                .accounts
                .iter()
                .any(|a| a.pubkey == payer && a.is_signer)
        );

        // 1 create + 1 extend (2 addresses fit in one extend chunk)
        assert_eq!(ixs.len(), 2);
        // Second instruction is ExtendLookupTable
        assert_eq!(ixs[1].data[0..4], [2, 0, 0, 0]); // extend
        assert_eq!(&ixs[1].data[4..12], &2u64.to_le_bytes()); // count = 2
    }

    #[test]
    fn test_build_or_extend_chunks_extend_instructions() {
        let payer = test_payer();
        let authority = test_authority();
        let mut entries: Vec<StaticAddressEntry> = Vec::new();
        for i in 0..65u8 {
            let addr = Pubkey::new_from_array([i; 32]);
            entries.push(StaticAddressEntry {
                category: AddressCategory::JitoTipAccount,
                address: addr,
                owner_program: None,
                label: "filler",
            });
        }

        let (_alt, ixs, report) =
            build_or_extend_master_alt_ixs(&payer, &authority, 12345, entries).unwrap();
        assert_eq!(report.addresses_added, 65);
        // 1 create + ceil(65 / 30) = 1 + 3 = 4 instructions
        assert_eq!(ixs.len(), 4);
        assert_eq!(ixs[0].data[0..4], [0, 0, 0, 0]); // create
        for ix in &ixs[1..] {
            assert_eq!(ix.data[0..4], [2, 0, 0, 0]); // extend
        }
    }

    #[test]
    fn test_administrative_refresh_requires_force() {
        let mut alt = StaticMasterAlt::new();
        let payer = test_payer();
        let authority = test_authority();
        let spl = Pubkey::from_str(SPL).unwrap();
        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();

        // Add entries covering all required categories.
        let entries = vec![
            StaticAddressEntry {
                category: AddressCategory::WsolMint,
                address: wsol,
                owner_program: Some(spl),
                label: "wsol",
            },
            StaticAddressEntry {
                category: AddressCategory::MeteoraProgram,
                address: Pubkey::from_str(METEORA_DLMM_PROGRAM_ID_STR).unwrap(),
                owner_program: None,
                label: "meteora",
            },
            StaticAddressEntry {
                category: AddressCategory::MeteoraEventAuthority,
                address: Pubkey::from_str("D1ZN9Wj1fRSUQfCjhvnu1hqDMT7hzjzBBpi12nVniYD6").unwrap(),
                owner_program: Some(Pubkey::from_str(METEORA_DLMM_PROGRAM_ID_STR).unwrap()),
                label: "event_authority",
            },
            StaticAddressEntry {
                category: AddressCategory::MemoProgram,
                address: Pubkey::from_str(MEMO_PROGRAM_ID_STR).unwrap(),
                owner_program: None,
                label: "memo",
            },
            StaticAddressEntry {
                category: AddressCategory::SplTokenProgram,
                address: spl,
                owner_program: None,
                label: "spl_token",
            },
            StaticAddressEntry {
                category: AddressCategory::SystemProgram,
                address: Pubkey::new_from_array([3u8; 32]),
                owner_program: None,
                label: "system",
            },
            StaticAddressEntry {
                category: AddressCategory::Token2022Program,
                address: Pubkey::from_str(TOKEN_2022_PROGRAM_ID).unwrap(),
                owner_program: None,
                label: "token2022",
            },
            StaticAddressEntry {
                category: AddressCategory::AssociatedTokenProgram,
                address: Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM_ID).unwrap(),
                owner_program: None,
                label: "ata",
            },
            StaticAddressEntry {
                category: AddressCategory::ComputeBudgetProgram,
                address: Pubkey::new_from_array([4u8; 32]),
                owner_program: None,
                label: "compute_budget",
            },
            StaticAddressEntry {
                category: AddressCategory::UsdtMint,
                address: Pubkey::from_str(USDT_MINT_STR).unwrap(),
                owner_program: Some(spl),
                label: "usdt",
            },
            StaticAddressEntry {
                category: AddressCategory::UsdcMint,
                address: Pubkey::from_str(arb_constants::USDC_MINT).unwrap(),
                owner_program: Some(spl),
                label: "usdc",
            },
        ];
        alt.populate_from_entries(payer, authority, entries, 100)
            .unwrap();

        // Refresh without force must be refused
        let err = administrative_refresh(&mut alt, false, vec![], 200).unwrap_err();
        assert!(matches!(err, MasterAltError::RefreshRefused));

        // Alt unchanged
        assert_eq!(alt.len(), 11);
    }

    #[test]
    fn test_alt_accounts_cannot_be_signers_in_v0() {
        let mut alt = StaticMasterAlt::new();
        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let usdc = Pubkey::from_str(USDC_MINT).unwrap();
        alt.addresses.push(wsol);
        alt.addresses.push(usdc);
        alt.address_set.insert(wsol);
        alt.address_set.insert(usdc);

        // wsol as signer => rejected
        assert!(alt.verify_no_signers(&[wsol]).is_err());
        // usdc as readonly => fine
        assert!(alt.verify_no_signers(&[]).is_ok());
    }
}
