//! CLI tool for IDL code generation.
//!
//! Usage:
//! ```bash
//! cargo run -p arb-idl-gen -- <idl_path> <output_dir> [program_id]
//! ```
//!
//! Example:
//! ```bash
//! cargo run -p arb-idl-gen -- fixtures/idl/raydium_clmm.json crates/arb-dex/src/generated/raydium_clmm CAMMCzo5YL8w4VFF8KVHrK22GGUsp5VTaW7grrKgrWqK
//! ```

use std::path::PathBuf;

use crate::{GenConfig, IdlProcessor, presets, generate_cpi_crate};
use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "arb-idl-gen", version, about = "Generate Rust helpers from Solana program IDLs")]
struct Args {
    /// Path to IDL JSON file
    #[arg(short, long)]
    idl: Option<PathBuf>,

    /// Output directory for generated code
    #[arg(short, long)]
    out: Option<PathBuf>,

    /// Program ID (optional, for constants)
    #[arg(short, long)]
    program_id: Option<String>,

    /// Generate all major DEX IDLs (uses presets)
    #[arg(long)]
    all_dex: bool,

    /// Base output directory for all_dex mode
    #[arg(long, default_value = "crates/arb-dex/src/generated")]
    all_dex_base: PathBuf,

    /// Generate instruction builders
    #[arg(long, default_value = "true")]
    instructions: bool,

    /// Generate account decoders
    #[arg(long, default_value = "true")]
    accounts: bool,

    /// Generate events
    #[arg(long, default_value = "true")]
    events: bool,

    /// Generate build.rs snippet for anchor-gen CPI crate
    #[arg(long)]
    gen_build_rs: Option<PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::parse();

    if args.all_dex {
        println!("Generating all major DEX IDL helpers...");
        let configs = presets::all_dex(&args.all_dex_base);
        for config in configs {
            println!("Processing {} -> {:?}", config.idl_path.display(), config.output_dir);
            let processor = IdlProcessor::new(config);
            processor.generate_helpers()?;
        }
        println!("Done!");
        return Ok(());
    }

    if let Some(idl_path) = args.gen_build_rs {
        if let Some(program_id) = args.program_id {
            let crate_name = idl_path.file_stem().unwrap().to_string_lossy().replace('-', "_");
            generate_cpi_crate(idl_path.to_str().unwrap(), &program_id, &crate_name)?;
        } else {
            anyhow::bail!("--program-id required with --gen-build-rs");
        }
        return Ok(());
    }

    let idl_path = args.idl.ok_or_else(|| anyhow::anyhow!("--idl required"))?;
    let out_dir = args.out.ok_or_else(|| anyhow::anyhow!("--out required"))?;

    if !idl_path.exists() {
        anyhow::bail!("IDL file not found: {:?}", idl_path);
    }

    let config = GenConfig {
        idl_path,
        output_dir: out_dir,
        program_id: args.program_id,
        generate_instructions: args.instructions,
        generate_accounts: args.accounts,
        generate_events: args.events,
        ..Default::default()
    };

    println!("Generating IDL helpers...");
    println!("  IDL: {:?}", config.idl_path);
    println!("  Output: {:?}", config.output_dir);
    if let Some(pid) = &config.program_id {
        println!("  Program ID: {}", pid);
    }

    let processor = IdlProcessor::new(config);
    processor.generate_helpers()?;

    println!("Done! Generated helpers in {:?}", processor.config.output_dir);
    println!("\nFor full CPI crate generation, add to build.rs:");
    println!("  anchor_gen::generate_cpi_crate!(\"../../{}\");", processor.config.idl_path.display());
    println!("  (requires anchor-gen in build-dependencies)");

    Ok(())
}