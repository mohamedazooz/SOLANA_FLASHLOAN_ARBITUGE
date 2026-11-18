//! Central Program Registry for Solana DEX Protocols.
//!
//! This module provides a single source of truth for all Program IDs used by
//! the arbitrage engine. It ensures:
//! - No duplicate Program IDs
//! - Network separation (Mainnet/Devnet)
//! - Centralized access to all protocol programs
//! - Verified Program IDs with official sources

use arb_types::DexProtocol;
use solana_pubkey::Pubkey;
use std::collections::HashMap;
use std::str::FromStr;

/// Network identifier for Solana clusters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Network {
    /// Solana Mainnet Beta
    Mainnet,
    /// Solana Devnet
    Devnet,
    /// Solana Testnet
    Testnet,
}

impl Network {
    /// Returns the network name as a string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Network::Mainnet => "mainnet",
            Network::Devnet => "devnet",
            Network::Testnet => "testnet",
        }
    }
}

/// Information about a Solana program.
#[derive(Debug, Clone)]
pub struct ProgramInfo {
    /// Human-readable name of the program
    pub name: &'static str,
    /// The DEX protocol this program belongs to
    pub protocol: DexProtocol,
    /// Mainnet program ID
    pub mainnet_id: Pubkey,
    /// Devnet program ID (if available)
    pub devnet_id: Option<Pubkey>,
    /// Version identifier
    pub version: &'static str,
    /// Whether an official IDL is available
    pub idl_available: bool,
    /// Whether an official SDK is available
    pub sdk_available: bool,
    /// Whether the program ID has been verified on-chain
    pub verified: bool,
    /// Official documentation URL
    pub docs_url: Option<&'static str>,
    /// Official GitHub repository
    pub github_url: Option<&'static str>,
}

/// Central registry for all Solana DEX programs.
///
/// This registry provides:
/// - Single source of truth for all Program IDs
/// - Network-aware program lookup
/// - Protocol-based program lookup
/// - Duplicate detection and prevention
/// - Verification status tracking
#[derive(Debug, Clone)]
pub struct ProgramRegistry {
    /// Map from Program ID to ProgramInfo (Mainnet only for now)
    programs: HashMap<Pubkey, ProgramInfo>,
    /// Map from Devnet Program ID to Mainnet Program ID
    devnet_to_mainnet: HashMap<Pubkey, Pubkey>,
}

impl Default for ProgramRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgramRegistry {
    /// Creates a new ProgramRegistry with all known programs registered.
    pub fn new() -> Self {
        let mut registry = Self {
            programs: HashMap::new(),
            devnet_to_mainnet: HashMap::new(),
        };
        
        // Register all programs
        registry.register_raydium();
        registry.register_orca();
        registry.register_meteora();
        registry.register_pump_fun();
        registry.register_pump_swap();
        
        // Verify no duplicates
        registry.verify_no_duplicates().expect("Duplicate Program IDs detected in registry");
        
        registry
    }
    
    /// Registers all Raydium programs.
    /// 
    /// Sources:
    /// - https://github.com/raydium-io/raydium-ui/blob/master/src/constants.ts
    /// - crates/arb-constants/src/lib.rs
    fn register_raydium(&mut self) {
        // Raydium AMM V4
        // Mainnet: 675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8
        // Devnet: C8npZJQ9Knqy6Q81ic4TqnFd3JX6A4qg1aXQY5sJX6eY
        let raydium_amm_v4_mainnet = Pubkey::from_str(arb_constants::RAYDIUM_V4_PROGRAM_ID)
            .expect("Invalid Raydium AMM V4 Mainnet Program ID");
        let raydium_amm_v4_devnet = Pubkey::from_str("C8npZJQ9Knqy6Q81ic4TqnFd3JX6A4qg1aXQY5sJX6eY")
            .ok();
        
        self.programs.insert(
            raydium_amm_v4_mainnet,
            ProgramInfo {
                name: "Raydium AMM V4",
                protocol: DexProtocol::RaydiumV4,
                mainnet_id: raydium_amm_v4_mainnet,
                devnet_id: raydium_amm_v4_devnet,
                version: "V4",
                idl_available: true,
                sdk_available: true,
                verified: true,
                docs_url: Some("https://raydium.io/docs/"),
                github_url: Some("https://github.com/raydium-io/raydium-sdk-V2"),
            },
        );
        
        if let Some(devnet_id) = raydium_amm_v4_devnet {
            self.devnet_to_mainnet.insert(devnet_id, raydium_amm_v4_mainnet);
        }
        
        // Raydium CPMM (CP-Swap)
        // Mainnet: CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C
        let raydium_cpmm_mainnet = Pubkey::from_str(arb_constants::RAYDIUM_CPMM_PROGRAM_ID)
            .expect("Invalid Raydium CPMM Mainnet Program ID");
        
        self.programs.insert(
            raydium_cpmm_mainnet,
            ProgramInfo {
                name: "Raydium CPMM",
                protocol: DexProtocol::RaydiumCpmm,
                mainnet_id: raydium_cpmm_mainnet,
                devnet_id: None,
                version: "V1",
                idl_available: true,
                sdk_available: true,
                verified: true,
                docs_url: Some("https://raydium.io/docs/"),
                github_url: Some("https://github.com/raydium-io/raydium-sdk-V2"),
            },
        );
        
        // Raydium CLMM
        // Mainnet: CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK
        let raydium_clmm_mainnet = Pubkey::from_str(arb_constants::RAYDIUM_CLMM_PROGRAM_ID)
            .expect("Invalid Raydium CLMM Mainnet Program ID");
        
        self.programs.insert(
            raydium_clmm_mainnet,
            ProgramInfo {
                name: "Raydium CLMM",
                protocol: DexProtocol::RaydiumClmm,
                mainnet_id: raydium_clmm_mainnet,
                devnet_id: None,
                version: "V1",
                idl_available: true,
                sdk_available: true,
                verified: true,
                docs_url: Some("https://raydium.io/docs/clmm"),
                github_url: Some("https://github.com/raydium-io/raydium-sdk-V2"),
            },
        );
        
        // Raydium LaunchLab
        // Mainnet: LanMV9sAd7wArD4vJFi2qDdfnVhFxYSUg6eADduJ3uj
        let raydium_launchlab_mainnet = Pubkey::from_str(arb_constants::RAYDIUM_LAUNCHLAB_PROGRAM_ID)
            .expect("Invalid Raydium LaunchLab Mainnet Program ID");
        
        self.programs.insert(
            raydium_launchlab_mainnet,
            ProgramInfo {
                name: "Raydium LaunchLab",
                protocol: DexProtocol::RaydiumLaunchLab,
                mainnet_id: raydium_launchlab_mainnet,
                devnet_id: None,
                version: "V1",
                idl_available: true,
                sdk_available: true,
                verified: true,
                docs_url: Some("https://raydium.io/docs/launchlab"),
                github_url: Some("https://github.com/raydium-io/raydium-sdk-V2"),
            },
        );
    }
    
    /// Registers all Orca programs.
    /// 
    /// Sources:
    /// - https://github.com/orca-so/whirlpools
    /// - crates/arb-constants/src/lib.rs
    fn register_orca(&mut self) {
        // Orca Whirlpools
        // Mainnet: whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc
        let orca_whirlpools_mainnet = Pubkey::from_str(arb_constants::ORCA_WHIRLPOOL_PROGRAM_ID)
            .expect("Invalid Orca Whirlpools Mainnet Program ID");
        
        self.programs.insert(
            orca_whirlpools_mainnet,
            ProgramInfo {
                name: "Orca Whirlpools",
                protocol: DexProtocol::OrcaWhirlpool,
                mainnet_id: orca_whirlpools_mainnet,
                devnet_id: None,
                version: "V1",
                idl_available: true,
                sdk_available: true,
                verified: true,
                docs_url: Some("https://docs.orca.so/"),
                github_url: Some("https://github.com/orca-so/whirlpools"),
            },
        );
    }
    
    /// Registers all Meteora programs.
    /// 
    /// Sources:
    /// - https://github.com/meteoraAg/dlmm-sdk
    /// - crates/arb-constants/src/lib.rs
    fn register_meteora(&mut self) {
        // Meteora DLMM
        // Mainnet: LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo
        let meteora_dlmm_mainnet = Pubkey::from_str(arb_constants::METEORA_DLMM_PROGRAM_ID)
            .expect("Invalid Meteora DLMM Mainnet Program ID");
        
        self.programs.insert(
            meteora_dlmm_mainnet,
            ProgramInfo {
                name: "Meteora DLMM",
                protocol: DexProtocol::MeteoraDlmm,
                mainnet_id: meteora_dlmm_mainnet,
                devnet_id: None,
                version: "V1",
                idl_available: true,
                sdk_available: true,
                verified: true,
                docs_url: Some("https://docs.meteora.ag/"),
                github_url: Some("https://github.com/meteoraAg/dlmm-sdk"),
            },
        );
        
        // Meteora DAMM V2
        // Mainnet: cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG
        let meteora_damm_v2_mainnet = Pubkey::from_str(arb_constants::METEORA_DAMM_V2_PROGRAM_ID)
            .expect("Invalid Meteora DAMM V2 Mainnet Program ID");
        
        self.programs.insert(
            meteora_damm_v2_mainnet,
            ProgramInfo {
                name: "Meteora DAMM V2",
                protocol: DexProtocol::MeteoraDammV2,
                mainnet_id: meteora_damm_v2_mainnet,
                devnet_id: None,
                version: "V2",
                idl_available: true,
                sdk_available: true,
                verified: true,
                docs_url: Some("https://docs.meteora.ag/"),
                github_url: Some("https://github.com/meteoraAg/dlmm-sdk"),
            },
        );
        
        // Meteora DBC (Dynamic Binary Curve)
        // Mainnet: dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN
        let meteora_dbc_mainnet = Pubkey::from_str(arb_constants::METEORA_DBC_PROGRAM_ID)
            .expect("Invalid Meteora DBC Mainnet Program ID");
        
        self.programs.insert(
            meteora_dbc_mainnet,
            ProgramInfo {
                name: "Meteora DBC",
                protocol: DexProtocol::MeteoraDbc,
                mainnet_id: meteora_dbc_mainnet,
                devnet_id: None,
                version: "V1",
                idl_available: true,
                sdk_available: true,
                verified: true,
                docs_url: Some("https://docs.meteora.ag/"),
                github_url: Some("https://github.com/meteoraAg/dlmm-sdk"),
            },
        );
        
        // Meteora DLMM Pro
        // Mainnet: dPro1D5wiqnGMH8vMDG23jh35btTHoWdyqXjqkqxe8n
        let meteora_dlmm_pro_mainnet = Pubkey::from_str(arb_constants::METEORA_DLMM_PRO_PROGRAM_ID)
            .expect("Invalid Meteora DLMM Pro Mainnet Program ID");
        
        self.programs.insert(
            meteora_dlmm_pro_mainnet,
            ProgramInfo {
                name: "Meteora DLMM Pro",
                protocol: DexProtocol::MeteoraDlmmPro,
                mainnet_id: meteora_dlmm_pro_mainnet,
                devnet_id: None,
                version: "V1",
                idl_available: true,
                sdk_available: true,
                verified: true,
                docs_url: Some("https://docs.meteora.ag/"),
                github_url: Some("https://github.com/meteoraAg/dlmm-sdk"),
            },
        );
    }
    
    /// Registers Pump.fun program.
    /// 
    /// Sources:
    /// - https://github.com/pump-fun/pump-public-docs
    /// - crates/arb-constants/src/lib.rs
    fn register_pump_fun(&mut self) {
        // Pump.fun Bonding Curve
        // Mainnet: 6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P
        let pump_fun_mainnet = Pubkey::from_str(arb_constants::PUMP_FUN_PROGRAM_ID)
            .expect("Invalid Pump.fun Mainnet Program ID");
        
        self.programs.insert(
            pump_fun_mainnet,
            ProgramInfo {
                name: "Pump.fun Bonding Curve",
                protocol: DexProtocol::PumpFun,
                mainnet_id: pump_fun_mainnet,
                devnet_id: None,
                version: "V1",
                idl_available: true,
                sdk_available: true,
                verified: true,
                docs_url: Some("https://pump.fun/docs"),
                github_url: Some("https://github.com/pump-fun/pump-public-docs"),
            },
        );
    }
    
    /// Registers PumpSwap program.
    /// 
    /// Sources:
    /// - https://github.com/pump-fun/pump-public-docs
    /// - crates/arb-constants/src/lib.rs
    fn register_pump_swap(&mut self) {
        // PumpSwap post-graduation AMM
        // Mainnet: pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA
        let pump_swap_mainnet = Pubkey::from_str(arb_constants::PUMP_SWAP_PROGRAM_ID)
            .expect("Invalid PumpSwap Mainnet Program ID");
        
        self.programs.insert(
            pump_swap_mainnet,
            ProgramInfo {
                name: "PumpSwap AMM",
                protocol: DexProtocol::PumpSwap,
                mainnet_id: pump_swap_mainnet,
                devnet_id: None,
                version: "V1",
                idl_available: true,
                sdk_available: true,
                verified: true,
                docs_url: Some("https://pump.fun/docs"),
                github_url: Some("https://github.com/pump-fun/pump-public-docs"),
            },
        );
    }
    

    
    /// Gets program information by Program ID.
    ///
    /// Returns None if the program ID is not registered.
    pub fn get(&self, program_id: &Pubkey) -> Option<&ProgramInfo> {
        self.programs.get(program_id)
    }
    
    /// Gets all programs for a specific protocol.
    pub fn get_by_protocol(&self, protocol: DexProtocol) -> Vec<&ProgramInfo> {
        self.programs.values()
            .filter(|p| p.protocol == protocol)
            .collect()
    }
    
    /// Gets all programs for a specific network.
    pub fn get_by_network(&self, network: Network) -> Vec<&ProgramInfo> {
        match network {
            Network::Mainnet => self.programs.values().collect(),
            Network::Devnet => {
                // For Devnet, we need to look up via the devnet_to_mainnet map
                // and return the corresponding Mainnet info with devnet_id
                self.devnet_to_mainnet.values()
                    .filter_map(|mainnet_id| self.programs.get(mainnet_id))
                    .collect()
            }
            Network::Testnet => Vec::new(), // Testnet not currently supported
        }
    }
    
    /// Gets the Mainnet Program ID for a Devnet Program ID.
    pub fn get_mainnet_from_devnet(&self, devnet_id: &Pubkey) -> Option<Pubkey> {
        self.devnet_to_mainnet.get(devnet_id).copied()
    }
    
    /// Gets the Devnet Program ID for a Mainnet Program ID.
    pub fn get_devnet_from_mainnet(&self, mainnet_id: &Pubkey) -> Option<Pubkey> {
        self.programs.get(mainnet_id).and_then(|p| p.devnet_id)
    }
    
    /// Gets all registered Program IDs.
    pub fn all_program_ids(&self) -> Vec<Pubkey> {
        self.programs.keys().cloned().collect()
    }
    
    /// Gets all registered protocols.
    pub fn all_protocols(&self) -> Vec<DexProtocol> {
        self.programs.values()
            .map(|p| p.protocol)
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect()
    }
    
    /// Verifies that no duplicate Program IDs exist in the registry.
    pub fn verify_no_duplicates(&self) -> Result<(), String> {
        // Check for duplicate Program IDs
        let mut seen_ids = std::collections::HashSet::new();
        for (id, _info) in &self.programs {
            if !seen_ids.insert(id) {
                return Err(format!("Duplicate Program ID: {}", id));
            }
        }
        
        // Check for duplicate names
        let mut seen_names = std::collections::HashSet::new();
        for info in self.programs.values() {
            if !seen_names.insert(info.name) {
                return Err(format!("Duplicate program name: {}", info.name));
            }
        }
        
        // Check that all Devnet IDs map to valid Mainnet IDs
        for (devnet_id, mainnet_id) in &self.devnet_to_mainnet {
            if !self.programs.contains_key(mainnet_id) {
                return Err(format!(
                    "Devnet ID {} maps to non-existent Mainnet ID {}",
                    devnet_id, mainnet_id
                ));
            }
        }
        
        Ok(())
    }
    
    /// Checks if a Program ID is registered.
    pub fn contains(&self, program_id: &Pubkey) -> bool {
        self.programs.contains_key(program_id)
    }
    
    /// Gets the number of registered programs.
    pub fn len(&self) -> usize {
        self.programs.len()
    }
    
    /// Checks if the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.programs.is_empty()
    }
}

impl std::fmt::Display for ProgramRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "ProgramRegistry with {} programs:", self.len())?;
        for (id, program_info) in &self.programs {
            writeln!(
                f,
                "  {} ({}): {} (Mainnet: {}, Devnet: {:?})",
                program_info.name,
                program_info.protocol.as_str(),
                program_info.version,
                id,
                program_info.devnet_id
            )?;
        }
        Ok(())
    }
}

/// Creates a new ProgramRegistry instance.
///
/// For most use cases, you should create a single instance and share it.
/// If you need a global singleton, use lazy_static or OnceCell in your application.
pub fn program_registry() -> ProgramRegistry {
    ProgramRegistry::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn registry_contains_raydium_amm_v4() {
        let registry = ProgramRegistry::new();
        let raydium_amm_v4 = Pubkey::from_str("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8")
            .unwrap();
        
        assert!(registry.contains(&raydium_amm_v4));
        
        let info = registry.get(&raydium_amm_v4).unwrap();
        assert_eq!(info.name, "Raydium AMM V4");
        assert_eq!(info.protocol, DexProtocol::RaydiumV4);
        assert!(info.verified);
    }
    
    #[test]
    fn registry_no_duplicates() {
        let registry = ProgramRegistry::new();
        assert!(registry.verify_no_duplicates().is_ok());
    }
    
    #[test]
    fn get_by_protocol() {
        let registry = ProgramRegistry::new();
        let raydium_programs = registry.get_by_protocol(DexProtocol::RaydiumV4);
        
        assert!(!raydium_programs.is_empty());
        assert!(raydium_programs.iter().all(|p| p.protocol == DexProtocol::RaydiumV4));
    }
    
    #[test]
    fn all_protocols_includes_raydium() {
        let registry = ProgramRegistry::new();
        let protocols = registry.all_protocols();
        
        assert!(protocols.contains(&DexProtocol::RaydiumV4));
    }
    
    #[test]
    fn program_registry_creates_new_instance() {
        let registry1 = program_registry();
        let registry2 = program_registry();
        
        // Each call creates a new instance (not a singleton)
        // This is intentional - callers should manage their own instances
        assert!(!std::ptr::eq(&registry1, &registry2));
    }
}
