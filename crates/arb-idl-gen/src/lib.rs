//! IDL Code Generation for Solana DEX Protocols
//!
//! This crate provides utilities for generating Rust code from Solana program IDLs.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum IGenError {
    #[error("IDL file not found: {0}")]
    FileNotFound(PathBuf),
    #[error("Invalid IDL JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),
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

/// Configuration for IDL processing.
#[derive(Debug, Clone)]
pub struct GenConfig {
    pub idl_path: PathBuf,
    pub output_dir: PathBuf,
    pub program_id: Option<String>,
    pub generate_instructions: bool,
    pub generate_accounts: bool,
    pub generate_events: bool,
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

/// Parsed IDL structure (subset of Anchor IDL spec).
#[derive(Debug, Deserialize)]
pub struct Idl {
    pub address: Option<String>,
    pub metadata: IdlMetadata,
    pub instructions: Vec<IdlInstruction>,
    pub accounts: Vec<IdlAccount>,
    pub events: Vec<IdlEvent>,
    pub errors: Option<Vec<IdlError>>,
    pub types: Option<Vec<IdlTypeDefinition>>,
}

#[derive(Debug, Deserialize)]
pub struct IdlMetadata {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub spec: Option<String>,
    pub repository: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct IdlInstruction {
    pub name: String,
    pub discriminator: Option<Vec<u8>>,
    pub accounts: Vec<IdlInstructionAccount>,
    pub args: Vec<IdlInstructionArg>,
}

#[derive(Debug, Deserialize)]
pub struct IdlInstructionAccount {
    pub name: String,
    pub writable: Option<bool>,
    pub signer: Option<bool>,
    pub address: Option<String>,
    pub pda: Option<IdlPda>,
}

#[derive(Debug, Deserialize)]
pub struct IdlPda {
    pub seeds: Vec<IdlSeed>,
    pub program_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum IdlSeed {
    Literal(String),
    Kind { kind: String, value: Value },
}

#[derive(Debug, Deserialize)]
pub struct IdlInstructionArg {
    pub name: String,
    #[serde(rename = "type")]
    pub type_: Value,
}

#[derive(Debug, Deserialize)]
pub struct IdlAccount {
    pub name: String,
    pub discriminator: Option<Vec<u8>>,
}

#[derive(Debug, Deserialize)]
pub struct IdlEvent {
    pub name: String,
    pub discriminator: Option<Vec<u8>>,
}

#[derive(Debug, Deserialize)]
pub struct IdlError {
    pub code: u32,
    pub name: String,
    pub msg: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct IdlTypeDefinition {
    pub name: String,
    #[serde(rename = "type")]
    pub type_: IdlTypeDefinitionType,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum IdlTypeDefinitionType {
    Struct { fields: Vec<IdlField> },
    Enum { variants: Vec<IdlEnumVariant> },
    Type { alias: String },
}

#[derive(Debug, Deserialize)]
pub struct IdlField {
    pub name: String,
    #[serde(rename = "type")]
    pub type_: Value,
}

#[derive(Debug, Deserialize)]
pub struct IdlEnumVariant {
    pub name: String,
    pub fields: Option<Vec<IdlField>>,
}

/// IDL Processor - parses IDL and generates helper code.
pub struct IdlProcessor {
    config: GenConfig,
}

impl IdlProcessor {
    pub fn new(config: GenConfig) -> Self {
        Self { config }
    }

    pub fn load_idl(&self) -> Result<Idl> {
        let content = fs::read_to_string(&self.config.idl_path)
            .with_context(|| format!("Failed to read IDL file: {:?}", self.config.idl_path))?;
        let idl: Idl = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse IDL JSON: {:?}", self.config.idl_path))?;
        Ok(idl)
    }

    pub fn generate_helpers(&self) -> Result<()> {
        let idl = self.load_idl()?;
        fs::create_dir_all(&self.config.output_dir)?;

        if self.config.generate_accounts {
            self.generate_accounts_rs(&idl)?;
        }
        if self.config.generate_instructions {
            self.generate_instructions_rs(&idl)?;
        }
        if self.config.generate_events {
            self.generate_events_rs(&idl)?;
        }
        Ok(())
    }

    fn generate_accounts_rs(&self, idl: &Idl) -> Result<()> {
        let mut out = String::new();
        out.push_str("//! Account decoding helpers\n");
        out.push_str("//! DO NOT EDIT MANUALLY\n\n");
        out.push_str("use solana_account_info::AccountInfo;\n");
        out.push_str("use solana_pubkey::Pubkey;\n\n");
        out.push_str(
            "pub fn is_owned_by_program(account: &AccountInfo, program_id: &Pubkey) -> bool {\n",
        );
        out.push_str("    account.owner == program_id\n");
        out.push_str("}\n");
        let accounts_path = self.config.output_dir.join("accounts.rs");
        fs::write(&accounts_path, out)?;
        Ok(())
    }

    fn generate_instructions_rs(&self, idl: &Idl) -> Result<()> {
        let mut out = String::new();
        out.push_str("//! Instruction building helpers\n");
        out.push_str("//! DO NOT EDIT MANUALLY\n\n");
        out.push_str("use solana_instruction::Instruction;\n");
        out.push_str("use solana_pubkey::Pubkey;\n\n");
        out.push_str("pub fn build_instruction(\n");
        out.push_str("    program_id: &Pubkey,\n");
        out.push_str("    accounts: Vec<solana_instruction::AccountMeta>,\n");
        out.push_str("    data: Vec<u8>,\n");
        out.push_str(") -> Instruction {\n");
        out.push_str("    Instruction { program_id: *program_id, accounts, data }\n");
        out.push_str("}\n");
        let instructions_path = self.config.output_dir.join("instructions.rs");
        fs::write(&instructions_path, out)?;
        Ok(())
    }

    fn generate_events_rs(&self, idl: &Idl) -> Result<()> {
        let mut out = String::new();
        out.push_str("//! Event parsing helpers\n");
        out.push_str("//! DO NOT EDIT MANUALLY\n\n");
        let events_path = self.config.output_dir.join("events.rs");
        fs::write(&events_path, out)?;
        Ok(())
    }
}

/// Predefined configurations for major DEX protocols.
pub mod presets {
    use super::*;

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
                (
                    "AccountInfo".into(),
                    "solana_account_info::AccountInfo".into(),
                ),
            ]
            .into(),
        }
    }

    pub fn all_dex(output_base: &Path) -> Vec<GenConfig> {
        vec![raydium_clmm(
            output_base,
            "CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK",
        )]
    }
}

/// Build script helper
pub fn generate_cpi_crate(idl_path: &str, program_id: &str, crate_name: &str) -> Result<()> {
    let build_rs_content = format!(
        r#"
//! Build script for generating CPI crate from IDL.
//! This file is generated by arb-idl-gen. DO NOT EDIT MANUALLY.

use std::env;
use std::path::PathBuf;

fn main() {{
    println!("cargo:rerun-if-changed={idl_path}");
    anchor_gen::generate_cpi_crate!("{idl_path}");
    println!("cargo:rustc-env={crate_name}_PROGRAM_ID={program_id}");
}}
"#,
        idl_path = idl_path,
        program_id = program_id,
        crate_name = crate_name.to_uppercase()
    );

    fs::write("build.rs", build_rs_content)?;
    println!(
        "Generated build.rs for {} - add anchor-gen to build-dependencies in Cargo.toml",
        crate_name
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_discriminators() {
        let acc_disc = account_discriminator("test");
        let evt_disc = event_discriminator("test");
        let ix_disc = instruction_discriminator("test");
        assert_eq!(acc_disc.len(), 8);
        assert_eq!(evt_disc.len(), 8);
        assert_eq!(ix_disc.len(), 8);
        assert_ne!(acc_disc, evt_disc);
        assert_ne!(acc_disc, ix_disc);
        assert_ne!(evt_disc, ix_disc);
    }
}
