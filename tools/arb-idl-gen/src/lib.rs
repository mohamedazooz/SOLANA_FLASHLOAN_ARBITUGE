//! IDL Code Generation for Solana DEX Protocols
//!
//! This crate uses `anchor-gen` and `shank` to generate Rust types, account decoders,
//! and instruction builders from official DEX program IDLs.
//!
//! Generated code targets the workspace's modern Solana crates (solana-pubkey 2.x,
//! solana-instruction 3.x, etc.) and is compatible with the arb-dex/arb-tx stack.
//!
//! Usage:
//! ```bash
//! cargo run -p arb-idl-gen -- <idl_path> <output_dir>
//! ```
//!
//! For CI integration, see `scripts/gen_idl.rs`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anchor_gen::{Generator, Idl};
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum IGenError {
    #[error("IDL file not found: {0}")]
    FileNotFound(PathBuf),
    #[error("Invalid IDL JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("Anchor-gen error: {0}")]
    AnchorGen(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Compute Anchor account discriminator: first 8 bytes of SHA256("account:<name>")
pub fn account_discriminator(name: &str) -> [u8; 8] {
    let mut hasher = Sha256::new();
    hasher.update(b"account:");
    hasher.update(name.as_bytes());
    let result = hasher.finalize();
    result[..8].try_into().expect("SHA256 output is 32 bytes")
}

/// Compute Anchor event discriminator: first 8 bytes of SHA256("event:<name>")
pub fn event_discriminator(name: &str) -> [u8; 8] {
    let mut hasher = Sha256::new();
    hasher.update(b"event:");
    hasher.update(name.as_bytes());
    let result = hasher.finalize();
    result[..8].try_into().expect("SHA256 output is 32 bytes")
}

/// Compute Anchor instruction discriminator: first 8 bytes of SHA256("global:<name>")
pub fn instruction_discriminator(name: &str) -> [u8; 8] {
    let mut hasher = Sha256::new();
    hasher.update(b"global:");
    hasher.update(name.as_bytes());
    let result = hasher.finalize();
    result[..8].try_into().expect("SHA256 output is 32 bytes")
}

/// Configuration for IDL code generation.
#[derive(Debug, Clone)]
pub struct GenConfig {
    /// Input IDL file path
    pub idl_path: PathBuf,
    /// Output directory for generated code
    pub output_dir: PathBuf,
    /// Program ID (for address constants)
    pub program_id: Option<String>,
    /// Whether to generate instruction builders
    pub generate_instructions: bool,
    /// Whether to generate account decoders
    pub generate_accounts: bool,
    /// Whether to generate events
    pub generate_events: bool,
    /// Custom type mappings (IDL type -> Rust type)
    pub type_overrides: HashMap<String, String>,
}

impl Default for GenConfig {
    fn default() -> Self {
        Self {
            idl_path: PathBuf::new(),
            output_dir: PathBuf::new(),
            program_id: None,
            generate_instructions: true,
            generate_accounts: true,
            generate_events: true,
            type_overrides: HashMap::new(),
        }
    }
}

/// IDL Code Generator
pub struct IdlGenerator {
    config: GenConfig,
}

impl IdlGenerator {
    /// Create a new generator with the given configuration.
    pub fn new(config: GenConfig) -> Self {
        Self { config }
    }

    /// Load IDL from file.
    pub fn load_idl(&self) -> Result<Idl> {
        let content = fs::read_to_string(&self.config.idl_path)
            .with_context(|| format!("Failed to read IDL file: {:?}", self.config.idl_path))?;
        let idl: Idl = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse IDL JSON: {:?}", self.config.idl_path))?;
        Ok(idl)
    }

    /// Generate Rust code from IDL.
    pub fn generate(&self) -> Result<()> {
        let idl = self.load_idl()?;

        // Ensure output directory exists
        fs::create_dir_all(&self.config.output_dir)?;

        // Configure anchor-gen generator
        let mut generator = Generator::default();
        generator
            .program_id(self.config.program_id.clone())
            .generate_instructions(self.config.generate_instructions)
            .generate_accounts(self.config.generate_accounts)
            .generate_events(self.config.generate_events);

        // Apply type overrides if any
        for (idl_type, rust_type) in &self.config.type_overrides {
            generator = generator.type_override(idl_type.clone(), rust_type.clone());
        }

        // Generate code
        let generated = generator
            .generate(&idl)
            .map_err(|e| IGenError::AnchorGen(e.to_string()))?;

        // Write generated files
        for (path, content) in generated {
            let full_path = self.config.output_dir.join(path);
            if let Some(parent) = full_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&full_path, content)
                .with_context(|| format!("Failed to write generated file: {:?}", full_path))?;
        }

        // Generate additional helper files
        self.generate_helpers(&idl)?;

        Ok(())
    }

    /// Generate helper files (mod.rs, constants, etc.)
    fn generate_helpers(&self, idl: &Idl) -> Result<()> {
        // Generate mod.rs
        let mod_rs = self.generate_mod_rs(idl)?;
        let mod_path = self.config.output_dir.join("mod.rs");
        fs::write(&mod_path, mod_rs)?;

        // Generate constants.rs with program ID and discriminators
        let constants_rs = self.generate_constants_rs(idl)?;
        let constants_path = self.config.output_dir.join("constants.rs");
        fs::write(&constants_path, constants_rs)?;

        // Generate accounts.rs if accounts are generated
        if self.config.generate_accounts {
            let accounts_rs = self.generate_accounts_rs(idl)?;
            let accounts_path = self.config.output_dir.join("accounts.rs");
            fs::write(&accounts_path, accounts_rs)?;
        }

        // Generate instructions.rs if instructions are generated
        if self.config.generate_instructions {
            let instructions_rs = self.generate_instructions_rs(idl)?;
            let instructions_path = self.config.output_dir.join("instructions.rs");
            fs::write(&instructions_path, instructions_rs)?;
        }

        Ok(())
    }

    /// Generate mod.rs that re-exports all generated modules.
    fn generate_mod_rs(&self, idl: &Idl) -> Result<String> {
        let program_name = idl.metadata.name.replace('-', "_");
        let mut out = String::new();

        out.push_str("//! Generated IDL bindings\n");
        out.push_str(&format!("//! Program: {}\n", idl.metadata.name));
        out.push_str(&format!("//! Version: {}\n", idl.metadata.version));
        out.push_str("//! DO NOT EDIT MANUALLY - regenerate with `arb-idl-gen`\n\n");

        out.push_str("#![allow(clippy::all)]\n");
        out.push_str("#![allow(dead_code)]\n");
        out.push_str("#![allow(non_camel_case_types)]\n");
        out.push_str("#![allow(non_snake_case)]\n\n");

        out.push_str("pub mod accounts;\n");
        out.push_str("pub mod instructions;\n");
        out.push_str("pub mod constants;\n");
        out.push_str("pub mod events;\n\n");

        out.push_str(&format!(
            "pub use accounts::{program_name}_accounts::*;\n"
        ));
        out.push_str(&format!(
            "pub use instructions::{program_name}_instructions::*;\n"
        ));
        out.push_str("pub use constants::*;\n");
        out.push_str("pub use events::*;\n");

        Ok(out)
    }

    /// Generate constants.rs with program ID and account discriminators.
    fn generate_constants_rs(&self, idl: &Idl) -> Result<String> {
        let program_name = idl.metadata.name.replace('-', "_");
        let mut out = String::new();

        out.push_str("//! Generated constants\n");
        out.push_str("//! DO NOT EDIT MANUALLY\n\n");

        out.push_str("use solana_pubkey::Pubkey;\n");
        out.push_str("use std::str::FromStr;\n\n");

        // Program ID constant
        if let Some(program_id) = &self.config.program_id {
            out.push_str(&format!(
                "pub const PROGRAM_ID: Pubkey = Pubkey::from_str(\"{program_id}\").unwrap();\n\n"
            ));
        }

        // Account discriminators
        out.push_str("// Account discriminators (first 8 bytes of SHA256(\"account:{name}\"))\n");
        for account in &idl.accounts {
            let discriminator = account_discriminator(&account.name);
            let const_name = account.name.to_uppercase();
            out.push_str(&format!(
                "pub const {const_name}_DISCRIMINATOR: [u8; 8] = {discriminator:?};\n"
            ));
        }
        out.push_str("\n");

        // Event discriminators
        if !idl.events.is_empty() {
            out.push_str("// Event discriminators\n");
            for event in &idl.events {
                let discriminator = event_discriminator(&event.name);
                let const_name = event.name.to_uppercase();
                out.push_str(&format!(
                    "pub const {const_name}_EVENT_DISCRIMINATOR: [u8; 8] = {discriminator:?};\n"
                ));
            }
            out.push_str("\n");
        }

        // Instruction discriminators
        if !idl.instructions.is_empty() {
            out.push_str("// Instruction discriminators (first 8 bytes of SHA256(\"global:{name}\"))\n");
            for ix in &idl.instructions {
                let discriminator = instruction_discriminator(&ix.name);
                let const_name = ix.name.to_uppercase();
                out.push_str(&format!(
                    "pub const {const_name}_IX_DISCRIMINATOR: [u8; 8] = {discriminator:?};\n"
                ));
            }
        }

        Ok(out)
    }

    /// Generate accounts.rs with helper functions for account decoding.
    fn generate_accounts_rs(&self, _idl: &Idl) -> Result<String> {
        let mut out = String::new();

        out.push_str("//! Account decoding helpers\n");
        out.push_str("//! DO NOT EDIT MANUALLY\n\n");

        out.push_str("use solana_account_info::AccountInfo;\n");
        out.push_str("use solana_pubkey::Pubkey;\n");
        out.push_str("use crate::accounts::*;\n\n");

        out.push_str("/// Check if an account is owned by this program.\n");
        out.push_str("pub fn is_owned_by_program(account: &AccountInfo, program_id: &Pubkey) -> bool {\n");
        out.push_str("    account.owner == program_id\n");
        out.push_str("}\n\n");

        out.push_str("/// Check if an account has the expected discriminator.\n");
        out.push_str("pub fn has_discriminator(account: &AccountInfo, expected: &[u8; 8]) -> bool {\n");
        out.push_str("    if account.data_len() < 8 {\n");
        out.push_str("        return false;\n");
        out.push_str("    }\n");
        out.push_str("    let data = account.try_borrow_data().ok();\n");
        out.push_str("    data.as_ref().map(|d| &d[0..8] == expected).unwrap_or(false)\n");
        out.push_str("}\n\n");

        out.push_str("/// Try to deserialize an account by discriminator.\n");
        out.push_str("pub fn try_deserialize_account<'a, T: crate::accounts::AccountDeserialize>(\n");
        out.push_str("    account: &'a AccountInfo,\n");
        out.push_str(") -> Result<&'a T, crate::accounts::AccountDeserializeError> {\n");
        out.push_str("    T::try_deserialize(&mut &account.data.borrow()[8..])\n");
        out.push_str("}\n");

        Ok(out)
    }

    /// Generate instructions.rs with helper functions for instruction building.
    fn generate_instructions_rs(&self, _idl: &Idl) -> Result<String> {
        let mut out = String::new();

        out.push_str("//! Instruction building helpers\n");
        out.push_str("//! DO NOT EDIT MANUALLY\n\n");

        out.push_str("use solana_instruction::Instruction;\n");
        out.push_str("use solana_pubkey::Pubkey;\n");
        out.push_str("use crate::instructions::*;\n\n");

        out.push_str("/// Build a program instruction with accounts and data.\n");
        out.push_str("pub fn build_instruction(\n");
        out.push_str("    program_id: &Pubkey,\n");
        out.push_str("    accounts: Vec<solana_instruction::AccountMeta>,\n");
        out.push_str("    data: Vec<u8>,\n");
        out.push_str(") -> Instruction {\n");
        out.push_str("    Instruction {\n");
        out.push_str("        program_id: *program_id,\n");
        out.push_str("        accounts,\n");
        out.push_str("        data,\n");
        out.push_str("    }\n");
        out.push_str("}\n");

        Ok(out)
    }
}

/// Batch generator for multiple IDLs.
pub struct BatchGenerator {
    configs: Vec<GenConfig>,
}

impl BatchGenerator {
    pub fn new(configs: Vec<GenConfig>) -> Self {
        Self { configs }
    }

    pub fn generate_all(&self) -> Result<Vec<PathBuf>> {
        let mut generated_dirs = Vec::new();
        for config in &self.configs {
            let generator = IdlGenerator::new(config.clone());
            generator.generate()?;
            generated_dirs.push(config.output_dir.clone());
        }
        Ok(generated_dirs)
    }
}

/// Predefined configurations for major DEX protocols.
pub mod presets {
    use super::*;

    /// Raydium CLMM IDL generation config
    pub fn raydium_clmm(output_base: &Path, program_id: &str) -> GenConfig {
        GenConfig {
            idl_path: PathBuf::from("fixtures/idl/raydium_clmm.json"),
            output_dir: output_base.join("raydium_clmm"),
            program_id: Some(program_id.to_string()),
            generate_instructions: true,
            generate_accounts: true,
            generate_events: true,
            type_overrides: [
                ("Pubkey".into(), "solana_pubkey::Pubkey".into()),
                ("AccountInfo".into(), "solana_account_info::AccountInfo".into()),
            ].into(),
        }
    }

    /// Orca Whirlpool IDL generation config
    pub fn orca_whirlpool(output_base: &Path, program_id: &str) -> GenConfig {
        GenConfig {
            idl_path: PathBuf::from("fixtures/idl/orca_whirlpool.json"),
            output_dir: output_base.join("orca_whirlpool"),
            program_id: Some(program_id.to_string()),
            generate_instructions: true,
            generate_accounts: true,
            generate_events: true,
            type_overrides: [
                ("Pubkey".into(), "solana_pubkey::Pubkey".into()),
                ("AccountInfo".into(), "solana_account_info::AccountInfo".into()),
            ].into(),
        }
    }

    /// Meteora DLMM IDL generation config
    pub fn meteora_dlmm(output_base: &Path, program_id: &str) -> GenConfig {
        GenConfig {
            idl_path: PathBuf::from("fixtures/idl/meteora_dlmm.json"),
            output_dir: output_base.join("meteora_dlmm"),
            program_id: Some(program_id.to_string()),
            generate_instructions: true,
            generate_accounts: true,
            generate_events: true,
            type_overrides: [
                ("Pubkey".into(), "solana_pubkey::Pubkey".into()),
                ("AccountInfo".into(), "solana_account_info::AccountInfo".into()),
            ].into(),
        }
    }

    /// Meteora DAMM V2 IDL generation config
    pub fn meteora_damm_v2(output_base: &Path, program_id: &str) -> GenConfig {
        GenConfig {
            idl_path: PathBuf::from("fixtures/idl/meteora_damm_v2.json"),
            output_dir: output_base.join("meteora_damm_v2"),
            program_id: Some(program_id.to_string()),
            generate_instructions: true,
            generate_accounts: true,
            generate_events: true,
            type_overrides: [
                ("Pubkey".into(), "solana_pubkey::Pubkey".into()),
                ("AccountInfo".into(), "solana_account_info::AccountInfo".into()),
            ].into(),
        }
    }

    /// PumpSwap IDL generation config
    pub fn pump_swap(output_base: &Path, program_id: &str) -> GenConfig {
        GenConfig {
            idl_path: PathBuf::from("fixtures/idl/pump_swap.json"),
            output_dir: output_base.join("pump_swap"),
            program_id: Some(program_id.to_string()),
            generate_instructions: true,
            generate_accounts: true,
            generate_events: true,
            type_overrides: [
                ("Pubkey".into(), "solana_pubkey::Pubkey".into()),
                ("AccountInfo".into(), "solana_account_info::AccountInfo".into()),
            ].into(),
        }
    }

    /// All major DEX configs
    pub fn all_dex(output_base: &Path) -> Vec<GenConfig> {
        vec![
            raydium_clmm(output_base, "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK"),
            orca_whirlpool(output_base, "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc"),
            meteora_dlmm(output_base, "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo"),
            meteora_damm_v2(output_base, "cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG"),
            pump_swap(output_base, "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA"),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_gen_config_default() {
        let config = GenConfig::default();
        assert!(config.idl_path.as_os_str().is_empty());
        assert!(config.output_dir.as_os_str().is_empty());
    }

    #[test]
    fn test_presets_all_dex() {
        let dir = tempdir().unwrap();
        let configs = presets::all_dex(dir.path());
        assert_eq!(configs.len(), 5);
    }
}