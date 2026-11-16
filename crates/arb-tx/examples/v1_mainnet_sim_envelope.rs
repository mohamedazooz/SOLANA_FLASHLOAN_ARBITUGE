//! Emits unsigned Solana Transaction V1 (SIMD-0385) simulation envelopes for
//! read-only mainnet `simulateTransaction` probes.
//!
//! Output: a single JSON object on stdout with two base64 envelopes:
//! - `baseline`: zero-instruction V1 envelope (pure transport probe).
//! - `kamino_flash_roundtrip`: the atomic borrow/repay spine of the three-pool
//!   arbitrage transaction — idempotent WSOL ATA creation, a small SOL top-up
//!   plus `SyncNative` to fund the flash-loan fee (the borrowed principal alone
//!   can never cover klend's exclusive origination fee), then Kamino
//!   `flash_borrow_reserve_liquidity` (1 SOL from the main-market WSOL
//!   reserve) and `flash_repay_reserve_liquidity` (principal + live-derived
//!   fee).
//!
//! The envelopes are unsigned (zero-filled signature slots) and are meant to
//! be submitted with `sigVerify: false` + `replaceRecentBlockhash: true` +
//! `maxSupportedTransactionVersion: 1`. This tool never signs or broadcasts.
//! Set `V1_SIM_PAYER=<pubkey>` to use an existing funded system account as the
//! simulation fee payer (the fee payer is never charged in a simulation); the
//! default is a deterministic throwaway keypair address, which the runtime
//! rejects with `InvalidAccountForFee`.

use arb_tx::{
    KeypairSigner, WSOL_MINT,
    adaptive::{AdaptiveTxCompiler, V1ConfigParams},
    kamino::{
        KaminoFlashAccounts, build_kamino_flash_borrow, build_kamino_flash_repay,
        calculate_kamino_flash_fee,
    },
};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use solana_pubkey::Pubkey;
use std::str::FromStr;

const V1_MAX_LOADED_DATA_LIMIT: u32 = 67_108_864;
const BORROW_AMOUNT_LAMPORTS: u64 = 1_000_000_000;
const MAIN_MARKET_WSOL_FEE_SF: u64 = 11_529_215_046_068;
const FEE_TOP_UP_LAMPORTS: u64 = 1_000_000;

fn sim_config() -> V1ConfigParams {
    V1ConfigParams {
        priority_fee_lamports: Some(100_000),
        compute_unit_limit: Some(1_200_000),
        loaded_accounts_data_size_limit: Some(V1_MAX_LOADED_DATA_LIMIT),
        heap_size: None,
    }
}

fn main() {
    let payer = match std::env::var("V1_SIM_PAYER") {
        Ok(address) => Pubkey::from_str(&address).expect("valid V1_SIM_PAYER pubkey"),
        Err(_) => KeypairSigner::from_seed(&[7u8; 32]).pubkey(),
    };
    let wsol_mint = Pubkey::from_str(WSOL_MINT).expect("valid WSOL mint");
    let token_program = Pubkey::from_str(arb_tx::TOKEN_PROGRAM_ID).expect("valid token program");
    let user_wsol_ata =
        arb_tx::get_associated_token_address_with_program_id(&payer, &wsol_mint, &token_program);

    let fee = calculate_kamino_flash_fee(BORROW_AMOUNT_LAMPORTS, MAIN_MARKET_WSOL_FEE_SF);
    let repay_amount = BORROW_AMOUNT_LAMPORTS.saturating_add(fee);

    let accounts = KaminoFlashAccounts::new_wsol_main_market(payer, user_wsol_ata);
    // klend requires the repay instruction's `amount` to equal the borrow
    // principal exactly (the fee is debited on top by the handler) and its
    // `borrow_instruction_index` to reference the borrow's executed position.
    let instructions = vec![
        arb_tx::build_create_ata_idempotent(payer, payer, wsol_mint, user_wsol_ata),
        arb_tx::build_system_transfer(payer, user_wsol_ata, FEE_TOP_UP_LAMPORTS),
        arb_tx::build_sync_native(user_wsol_ata),
        build_kamino_flash_borrow(&accounts, BORROW_AMOUNT_LAMPORTS),
        build_kamino_flash_repay(&accounts, BORROW_AMOUNT_LAMPORTS, 3),
    ];
    debug_assert_eq!(
        instructions[3].data[0..8],
        arb_tx::FLASH_BORROW_DISCRIMINATOR
    );

    let blockhash = [0u8; 32];
    let baseline = AdaptiveTxCompiler::build_unsigned_v1_simulation_transaction(
        payer,
        &[],
        blockhash,
        sim_config(),
    )
    .expect("baseline envelope must build");
    let roundtrip = AdaptiveTxCompiler::build_unsigned_v1_simulation_transaction(
        payer,
        &instructions,
        blockhash,
        sim_config(),
    )
    .expect("Kamino flash roundtrip envelope must build");

    println!(
        "{}",
        serde_json::json!({
            "payer": payer.to_string(),
            "user_wsol_ata": user_wsol_ata.to_string(),
            "borrow_amount_lamports": BORROW_AMOUNT_LAMPORTS,
            "flash_fee_lamports": fee,
            "repay_amount_lamports": repay_amount,
            "instruction_count": instructions.len(),
            "baseline_wire_bytes": baseline.len(),
            "roundtrip_wire_bytes": roundtrip.len(),
            "baseline_base64": BASE64_STANDARD.encode(&baseline),
            "roundtrip_base64": BASE64_STANDARD.encode(&roundtrip),
        })
    );
}
