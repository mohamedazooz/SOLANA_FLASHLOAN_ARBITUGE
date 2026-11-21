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

use arb_idl_gen::{GenConfig, IdlGenerator, presets};
use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "arb-idl-gen", version, about = "Generate Rust code from Solana program IDLs")]
struct Args {
    /// Path to IDL JSON file
    #[arg(short, long)]
    idl: PathBuf,

    /// Output directory for generated code
    #[arg(short, long)]
    out: PathBuf,

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
}

fn main() -> Result<()> {
    let args = Args::parse();

    if args.all_dex {
        println!("Generating all major DEX IDLs...");
        let configs = presets::all_dex(&args.all_dex_base);
        for config in configs {
            println!("Generating {} -> {:?}", config.idl_path.display(), config.output_dir);
            let generator = IdlGenerator::new(config);
            generator.generate()?;
        }
        println!("Done!");
        return Ok(());
    }

    if !args.idl.exists() {
        anyhow::bail!("IDL file not found: {:?}", args.idl);
    }

    let config = GenConfig {
        idl_path: args.idl,
        output_dir: args.out,
        program_id: args.program_id,
        generate_instructions: args.instructions,
        generate_accounts: args.accounts,
        generate_events: args.events,
        ..Default::default()
    };

    println!("Generating IDL bindings...");
    println!("  IDL: {:?}", config.idl_path);
    println!("  Output: {:?}", config.output_dir);
    if let Some(pid) = &config.program_id {
        println!("  Program ID: {}", pid);
    }

    let generator = IdlGenerator::new(config);
    generator.generate()?;

    println!("Done! Generated code in {:?}", generator.config.output_dir);
    Ok(())
}