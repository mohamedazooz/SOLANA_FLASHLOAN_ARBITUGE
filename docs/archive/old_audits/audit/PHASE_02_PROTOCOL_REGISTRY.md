> [!NOTE]
> **ARCHIVED — HISTORICAL REFERENCE — NOT ACTIVE PROJECT INSTRUCTIONS**
> This document is preserved for historical auditing and forensic reference. It reflects past states and does NOT represent active project guidance or verified current status.
> For current, authoritative engineering instructions, architecture, and roadmaps, consult:
> - [docs/active/README.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/README.md)
> - [docs/active/MASTER_ROADMAP.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/MASTER_ROADMAP.md)
> - [docs/active/CURRENT_STATUS.md](file:///c:/Users/azoozz/Downloads/my arbitrge/docs/active/CURRENT_STATUS.md)

---

# Phase 2: Protocol Registry - Execution Plan

## Overview
This document outlines the execution plan for **Phase 2: Protocol Registry** as defined in the MASTER PROMPT.

## Phase Objectives
- **Central Registry**: Create a single source of truth for all Protocol IDs
- **Network Separation**: Separate Mainnet and Devnet IDs
- **Protocol Mapping**: Map Program IDs to Protocols, Adapters, and Decoders
- **IDL/SDK Validation**: Validate all Program IDs against official IDLs and SDKs
- **No Duplicate IDs**: Ensure no Program ID is mapped to multiple protocols

## Acceptance Criteria
- Every Program ID is documented and tested
- No conflicting Program ID mappings exist
- All protocols have verified Program IDs
- Central registry is the single source of truth

## Current State Assessment

Based on the Component Status Matrix from Phase 0:
- Multiple crates reference Program IDs independently
- No central registry exists
- Program IDs may be duplicated or inconsistent
- IDL/SDK validation is incomplete

## Architecture Plan

### Directory Structure
```
crates/arb-dex/src/programs/
├── mod.rs                    # Central Program Registry
├── raydium.rs                # Raydium Program IDs
├── orca.rs                   # Orca Program IDs
├── meteora.rs                # Meteora Program IDs
├── pump_fun.rs               # Pump.fun Program IDs
├── pump_swap.rs              # PumpSwap Program IDs
└── kamino.rs                 # Kamino Program IDs
```

### Program Registry Structure

```rust
// crates/arb-dex/src/programs/mod.rs

pub mod raydium;
pub mod orca;
pub mod meteora;
pub mod pump_fun;
pub mod pump_swap;
pub mod kamino;

use solana_pubkey::Pubkey;
use std::collections::HashMap;

/// Network identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Network {
    Mainnet,
    Devnet,
    Testnet,
}

/// Protocol identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DexProtocol {
    RaydiumV4,
    RaydiumCPMM,
    RaydiumCLMM,
    RaydiumAMM,
    OrcaWhirlpools,
    MeteoraDLMM,
    MeteoraDAMM,
    PumpFun,
    PumpSwap,
    Kamino,
}

/// Program information
#[derive(Debug, Clone)]
pub struct ProgramInfo {
    pub name: &'static str,
    pub protocol: DexProtocol,
    pub mainnet_id: Pubkey,
    pub devnet_id: Option<Pubkey>,
    pub version: &'static str,
    pub idl_available: bool,
    pub sdk_available: bool,
    pub verified: bool,
}

/// Central Program Registry
pub struct ProgramRegistry {
    programs: HashMap<Pubkey, ProgramInfo>,
}

impl ProgramRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            programs: HashMap::new(),
        };
        
        // Register all programs
        registry.register_raydium();
        registry.register_orca();
        registry.register_meteora();
        registry.register_pump_fun();
        registry.register_pump_swap();
        registry.register_kamino();
        
        registry
    }
    
    pub fn get(&self, program_id: &Pubkey) -> Option<&ProgramInfo> {
        self.programs.get(program_id)
    }
    
    pub fn get_by_protocol(&self, protocol: DexProtocol) -> Vec<&ProgramInfo> {
        self.programs.values()
            .filter(|p| p.protocol == protocol)
            .collect()
    }
    
    pub fn verify_no_duplicates(&self) -> Result<(), String> {
        // Check for duplicate Program IDs
        // Check for duplicate names
        // Check for conflicting mappings
        Ok(())
    }
}
```

### Individual Protocol Files

Each protocol file should contain:

```rust
// crates/arb-dex/src/programs/raydium.rs

use super::{DexProtocol, Network, ProgramInfo};
use solana_pubkey::Pubkey;

pub const RAYDIUM_AMM_V4_MAINNET: &str = "...";
pub const RAYDIUM_AMM_V4_DEVNET: &str = "...";
pub const RAYDIUM_CPMM_MAINNET: &str = "...";
pub const RAYDIUM_CLMM_MAINNET: &str = "...";

impl ProgramRegistry {
    fn register_raydium(&mut self) {
        self.programs.insert(
            Pubkey::from_str(RAYDIUM_AMM_V4_MAINNET).unwrap(),
            ProgramInfo {
                name: "Raydium AMM V4",
                protocol: DexProtocol::RaydiumV4,
                mainnet_id: Pubkey::from_str(RAYDIUM_AMM_V4_MAINNET).unwrap(),
                devnet_id: Some(Pubkey::from_str(RAYDIUM_AMM_V4_DEVNET).unwrap()),
                version: "V4",
                idl_available: true,
                sdk_available: true,
                verified: true,
            },
        );
        
        // Add more Raydium programs...
    }
}
```

## Required Program IDs

### Raydium
- [ ] Raydium AMM V4 (Mainnet)
- [ ] Raydium AMM V4 (Devnet)
- [ ] Raydium CPMM (Mainnet)
- [ ] Raydium CPMM (Devnet)
- [ ] Raydium CLMM (Mainnet)
- [ ] Raydium CLMM (Devnet)
- [ ] Raydium LaunchLab (if supported)

### Orca
- [ ] Orca Whirlpools (Mainnet)
- [ ] Orca Whirlpools (Devnet)

### Meteora
- [ ] Meteora DLMM (Mainnet)
- [ ] Meteora DLMM (Devnet)
- [ ] Meteora DAMM V2 (Mainnet)
- [ ] Meteora DAMM V2 (Devnet)

### Pump.fun
- [ ] Pump.fun Bonding Curve (Mainnet)
- [ ] Pump.fun Bonding Curve (Devnet)

### PumpSwap
- [ ] PumpSwap (Mainnet)
- [ ] PumpSwap (Devnet)

### Kamino
- [ ] Kamino Lending (Mainnet)
- [ ] Kamino Lending (Devnet)

## Validation Requirements

### For Each Program ID
1. **Official Documentation**: Verify from official protocol documentation
2. **Explorer Verification**: Check on Solana Explorer / Solscan
3. **IDL Verification**: Verify IDL is available and matches
4. **SDK Verification**: Verify SDK is available and compatible
5. **Deployment Verification**: Confirm program is deployed on the specified network

### Sources
- Primary: Official protocol GitHub repositories
- Secondary: Solana Explorer, Solscan
- Tertiary: Community documentation (with verification)

## Files to Modify

| File | Action | Priority |
|------|--------|----------|
| `crates/arb-dex/src/programs/mod.rs` | Create central registry | HIGH |
| `crates/arb-dex/src/programs/raydium.rs` | Add Raydium Program IDs | HIGH |
| `crates/arb-dex/src/programs/orca.rs` | Add Orca Program IDs | HIGH |
| `crates/arb-dex/src/programs/meteora.rs` | Add Meteora Program IDs | HIGH |
| `crates/arb-dex/src/programs/pump_fun.rs` | Add Pump.fun Program IDs | HIGH |
| `crates/arb-dex/src/programs/pump_swap.rs` | Add PumpSwap Program IDs | HIGH |
| `crates/arb-dex/src/programs/kamino.rs` | Add Kamino Program IDs | HIGH |
| `crates/arb-dex/src/lib.rs` | Export ProgramRegistry | MEDIUM |
| All crates using Program IDs | Update to use ProgramRegistry | MEDIUM |

## Tests Required

### Unit Tests
1. **Program ID Validation**: Test that all Program IDs are valid Pubkeys
2. **No Duplicates**: Test that no Program ID is registered twice
3. **Network Separation**: Test that Mainnet and Devnet IDs are distinct
4. **Protocol Mapping**: Test that each Program ID maps to the correct protocol
5. **Registry Lookup**: Test ProgramRegistry::get() and get_by_protocol()

### Integration Tests
1. **Cross-Crate Usage**: Test that other crates can use ProgramRegistry
2. **Adapter Integration**: Test that DEX adapters can access their Program IDs
3. **Ingest Integration**: Test that ingest layer can use ProgramRegistry

## Acceptance Verification

Before declaring Phase 2 complete:
- [ ] All Program IDs are documented with sources
- [ ] All Program IDs are verified on-chain
- [ ] No duplicate Program IDs exist
- [ ] Central registry is the single source of truth
- [ ] All tests pass
- [ ] No crates reference Program IDs directly (only through registry)

## Dependencies on Other Phases

- **Phase 1**: Financial Integrity (COMPLETED)
- **Phase 3**: Token-2022 (Can proceed in parallel)
- **Phase 4**: Market Data & Yellowstone (Can proceed in parallel)

## Next Phase
After Phase 2: **Phase 3: Token-2022 Extension Capability Framework**
