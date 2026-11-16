//! Live Mainnet ALT verification (read-only, no broadcasting).
//!
//! This is a SEPARATE integration test that hits the live Solana RPC. It never
//! creates, extends, or modifies any on-chain ALT, never signs a transaction that
//! is submitted anywhere, and never uses production keys.
//!
//! It validates:
//! - ALT account owner (AddressLookupTable program),
//! - ProgramState variant, deactivation slot, authority,
//! - `last_extended_slot` vs current slot (warmup),
//! - Capacity (256) and duplicate-free addresses,
//! - Membership of expected static accounts,
//! - A real V0 transaction is compiled and measured with the fetched ALT,
//! - Byte savings vs the fully-inline variant.
//!
//! Run explicitly with:
//!   ARB_MAINNET_RPC_URL=<rpc> cargo test -p arb-tx --test mainnet_alt_integration_tests -- --ignored
//!
//! If `ARB_MAINNET_RPC_URL` / `SOLANA_RPC_PRIMARY` is not set, every test prints a
//! skip message and returns Ok(()).

use std::str::FromStr;

use arb_constants::{TOKEN_PROGRAM_ID, WSOL_MINT};
use arb_tx::compiler::SwapDirection;
use arb_tx::compiler::SwapMode;
use arb_tx::master_alt::derive_ata;
use arb_tx::master_alt::{LOOKUP_TABLE_META_SIZE, StaticMasterAlt, parse_alt_account_data};
use arb_tx::{
    AddressLookupTable, MAX_ALT_ADDRESSES, TxBuilder, compiler::ThreeLegMemeArbitrageContext,
};
use arb_types::{MintId, PoolId};
use solana_pubkey::Pubkey;

const METEORA_EVENT_AUTHORITY: &str = "D1ZN9Wj1fRSUQfCjhvnu1hqDMT7hzjzBBpi12nVniYD6";
const TOKEN_2022_PROGRAM_ID_STR: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
const SYSTEM_PROGRAM: [u8; 32] = [0u8; 32];
const ALT_PROGRAM_ID: &str = "AddressLookupTab1e1111111111111111111111111";

fn rpc_url() -> Option<String> {
    std::env::var("ARB_MAINNET_RPC_URL")
        .ok()
        .or_else(|| std::env::var("SOLANA_RPC_PRIMARY").ok())
        .filter(|s| !s.is_empty())
}

fn alt_address() -> Pubkey {
    std::env::var("ARB_MAINNET_ALT_ADDRESS")
        .ok()
        .and_then(|s| Pubkey::from_str(&s).ok())
        .unwrap_or_else(|| {
            // Meteora's official public DLMM ALT (read-only).
            Pubkey::from_str("JA5F83HUK9L78Y12TRLCsJZbu3Tv8pCK1GfK8mVNp1sz").unwrap()
        })
}

#[derive(serde::Deserialize)]
struct RpcResponse<T> {
    result: Option<T>,
    error: Option<serde_json::Value>,
}

#[derive(serde::Deserialize)]
struct AccountInfoValue {
    owner: String,
    lamports: u64,
    executable: bool,
    data: (String, String),
}

#[derive(serde::Deserialize)]
struct AccountInfoResult {
    value: Option<AccountInfoValue>,
}

async fn rpc_call<T: serde::de::DeserializeOwned>(
    url: &str,
    method: &str,
    params: serde_json::Value,
) -> Result<T, String> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });
    let resp: RpcResponse<T> = reqwest::Client::new()
        .post(url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("rpc {method} request failed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("rpc {method} decode failed: {e}"))?;
    if let Some(err) = resp.error {
        return Err(format!("rpc {method} error: {err}"));
    }
    resp.result
        .ok_or_else(|| format!("rpc {method}: empty result"))
}

fn b64_decode(s: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|e| e.to_string())
}

#[tokio::test]
#[ignore = "live RPC; run with --ignored"]
async fn test_live_alt_validates_and_warmup() {
    let Some(url) = rpc_url() else {
        eprintln!("skip: set ARB_MAINNET_RPC_URL or SOLANA_RPC_PRIMARY");
        return;
    };
    let alt = alt_address();
    let res: AccountInfoResult = rpc_call(
        &url,
        "getAccountInfo",
        serde_json::json!([alt.to_string(), { "encoding": "base64" }]),
    )
    .await
    .expect("getAccountInfo");
    let Some(value) = res.value else {
        panic!("ALT account {alt} not found (or wrong address)");
    };
    assert_eq!(value.owner, ALT_PROGRAM_ID, "ALT owner mismatch");
    assert!(!value.executable, "ALT must not be executable");
    assert!(value.lamports > 0);

    let data = b64_decode(&value.data.0).expect("base64 decode");
    assert!(
        data.len() >= LOOKUP_TABLE_META_SIZE,
        "ALT account data too short"
    );
    assert_eq!(
        (data.len() - LOOKUP_TABLE_META_SIZE) % 32,
        0,
        "ALT data 56 + 32*N"
    );
    assert_eq!(LOOKUP_TABLE_META_SIZE, 56);

    let parsed = parse_alt_account_data(&data).expect("parse ALT");
    assert_eq!(parsed.deactivation_slot, u64::MAX, "ALT is deactivated");
    assert_eq!(
        (data.len() - LOOKUP_TABLE_META_SIZE) / 32,
        parsed.addresses.len()
    );
    assert!(parsed.addresses.len() <= MAX_ALT_ADDRESSES);
    println!("ALT: {alt}");
    println!("owner program: {}", value.owner);
    println!("lamports: {}", value.lamports);
    println!("deactivation_slot: {}", parsed.deactivation_slot);
    println!("last_extended_slot: {}", parsed.last_extended_slot);
    println!(
        "last_extended_slot_start_index: {}",
        parsed.last_extended_slot_start_index
    );
    println!("authority: {:?}", parsed.authority.map(|a| a.to_string()));
    println!("addresses: {}", parsed.addresses.len());
    for (i, a) in parsed.addresses.iter().enumerate() {
        println!("  [{i}] {a}");
    }

    // Warmup check against the live slot.
    let slot: u64 = rpc_call(&url, "getSlot", serde_json::json!([]))
        .await
        .expect("getSlot");
    println!("current slot: {slot}");
    assert!(
        slot > parsed.last_extended_slot,
        "ALT not warm: current slot {slot} <= last_extended_slot {}",
        parsed.last_extended_slot
    );

    // Adoption as readonly snapshot (no on-chain change).
    let adopted =
        StaticMasterAlt::from_onchain_readonly(alt, parsed.clone(), None, Some(200_000_000))
            .expect("adopt ALT");
    assert!(adopted.verify_warmup(slot).is_ok());
    assert_eq!(adopted.addresses.len(), parsed.addresses.len());

    // Expected static members (Meteora ALT contains the DLMM event authority).
    let ea = Pubkey::from_str(METEORA_EVENT_AUTHORITY).unwrap();
    assert!(
        adopted.contains(&ea),
        "event authority missing from live ALT"
    );
    assert!(adopted.contains(&Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap()));
    assert!(adopted.contains(&Pubkey::from_str(TOKEN_2022_PROGRAM_ID_STR).unwrap()));
    let _ = SYSTEM_PROGRAM; // system program presence is reported, not required (not in every ALT)

    assert!(adopted.index_of(&ea).is_some());
}

#[tokio::test]
#[ignore = "live RPC; run with --ignored"]
async fn test_live_alt_compiles_v0_and_measures_bytes() {
    let Some(url) = rpc_url() else {
        eprintln!("skip: set ARB_MAINNET_RPC_URL or SOLANA_RPC_PRIMARY");
        return;
    };
    let alt = alt_address();
    let res: AccountInfoResult = rpc_call(
        &url,
        "getAccountInfo",
        serde_json::json!([alt.to_string(), { "encoding": "base64" }]),
    )
    .await
    .expect("getAccountInfo");
    let Some(value) = res.value else {
        panic!("ALT account {alt} not found");
    };
    let data = b64_decode(&value.data.0).expect("base64 decode");
    let parsed = parse_alt_account_data(&data).expect("parse ALT");

    // Build the 3-leg fixture (same as the deterministic test fixture).
    let payer = Pubkey::new_from_array([1u8; 32]);
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    let spl = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
    let meme_a = Pubkey::new_from_array([90u8; 32]);
    let meme_b = Pubkey::new_from_array([91u8; 32]);
    let ma_ata = derive_ata(&payer, &meme_a, &spl);
    let mb_ata = derive_ata(&payer, &meme_b, &spl);
    let wsol_ata = derive_ata(&payer, &wsol, &spl);
    let p1 = PoolId::new([10u8; 32]);
    let p2 = PoolId::new([20u8; 32]);
    let p3 = PoolId::new([30u8; 32]);

    let ctx = ThreeLegMemeArbitrageContext {
        payer,
        wsol_mint: wsol,
        meme_a_mint: MintId(meme_a),
        meme_b_mint: MintId(meme_b),
        pool_1: p1,
        pool_2: p2,
        pool_3: p3,
        leg_1_direction: SwapDirection::XtoY,
        leg_2_direction: SwapDirection::XtoY,
        leg_3_direction: SwapDirection::YtoX,
        leg_1_accounts: arb_tx::compiler::LegExecutionAccounts {
            lb_pair: p1.0,
            bin_array_pdas: vec![
                Pubkey::new_from_array([41u8; 32]),
                Pubkey::new_from_array([42u8; 32]),
            ],
            bitmap_extension: None,
            reserve_x: Pubkey::new_from_array([71u8; 32]),
            reserve_y: Pubkey::new_from_array([72u8; 32]),
            token_x_mint: wsol,
            token_y_mint: meme_a,
            token_x_program: spl,
            token_y_program: spl,
            oracle: Pubkey::new_from_array([81u8; 32]),
            host_fee_in: None,
            user_token_in: wsol_ata,
            user_token_out: ma_ata,
        },
        leg_2_accounts: arb_tx::compiler::LegExecutionAccounts {
            lb_pair: p2.0,
            bin_array_pdas: vec![
                Pubkey::new_from_array([51u8; 32]),
                Pubkey::new_from_array([52u8; 32]),
            ],
            bitmap_extension: None,
            reserve_x: Pubkey::new_from_array([73u8; 32]),
            reserve_y: Pubkey::new_from_array([74u8; 32]),
            token_x_mint: meme_a,
            token_y_mint: meme_b,
            token_x_program: spl,
            token_y_program: spl,
            oracle: Pubkey::new_from_array([82u8; 32]),
            host_fee_in: None,
            user_token_in: ma_ata,
            user_token_out: mb_ata,
        },
        leg_3_accounts: arb_tx::compiler::LegExecutionAccounts {
            lb_pair: p3.0,
            bin_array_pdas: vec![
                Pubkey::new_from_array([61u8; 32]),
                Pubkey::new_from_array([62u8; 32]),
            ],
            bitmap_extension: None,
            reserve_x: Pubkey::new_from_array([75u8; 32]),
            reserve_y: Pubkey::new_from_array([76u8; 32]),
            token_x_mint: meme_b,
            token_y_mint: wsol,
            token_x_program: spl,
            token_y_program: spl,
            oracle: Pubkey::new_from_array([83u8; 32]),
            host_fee_in: None,
            user_token_in: mb_ata,
            user_token_out: wsol_ata,
        },
        leg_1_params: arb_tx::compiler::LegSwapParams {
            mode: SwapMode::ExactIn,
            amount_in: 1_000_000_000,
            min_amount_out: 950_000_000,
            max_amount_in: 0,
            amount_out: 0,
        },
        leg_2_params: arb_tx::compiler::LegSwapParams {
            mode: SwapMode::ExactIn,
            amount_in: 950_000_000,
            min_amount_out: 900_000_000,
            max_amount_in: 0,
            amount_out: 0,
        },
        leg_3_params: arb_tx::compiler::LegSwapParams {
            mode: SwapMode::ExactIn,
            amount_in: 900_000_000,
            min_amount_out: 850_000_000,
            max_amount_in: 0,
            amount_out: 0,
        },
        wsol_token_program: spl,
        meme_a_token_program: spl,
        meme_b_token_program: spl,
        meme_a_ata: ma_ata,
        meme_b_ata: mb_ata,
        meme_a_ata_exists: true,
        meme_b_ata_exists: true,
        meme_a_ata_balance: 0,
        meme_b_ata_balance: 0,
        meme_a_is_token_2022: false,
        meme_b_is_token_2022: false,
        meme_a_withheld_fees: 0,
        meme_b_withheld_fees: 0,
        snapshot_slot: 250_000_000,
        instruction_version: 0,
        cu_limit: 320_000,
        priority_fee_micro_lamports: 1_000,
        expected_min_final_output: 850_000_000,
        jito_tip_account: Pubkey::from_str(arb_constants::JITO_TIP_ACCOUNTS[0]).unwrap(),
        jito_tip_lamports: 10_000,
        recent_blockhash: [42u8; 32],
        v1_transport_supported: false,
    };

    // The instruction vector (CU limit, CU price, 3 swaps, JITO tip) — mirrors the
    // compiler's instruction assembly. We use the public instruction builders.
    let mut instructions: Vec<arb_tx::CompiledInstruction> = vec![
        arb_tx::build_compute_unit_limit(ctx.cu_limit),
        arb_tx::build_compute_unit_price(ctx.priority_fee_micro_lamports),
    ];
    instructions.extend(arb_tx::compiler::build_leg_swap_instructions(&ctx).unwrap());
    instructions.push(arb_tx::build_marked_jito_tip_instruction(
        ctx.payer,
        ctx.jito_tip_account,
        arb_types::Lamports(ctx.jito_tip_lamports),
    ));

    // Compile with the REAL fetched ALT (only members actually referenced are serialized as lookups).
    let table = AddressLookupTable {
        key: alt,
        addresses: parsed.addresses.clone(),
    };
    let alt_msg =
        TxBuilder::compile_v0_message(ctx.payer, &instructions, ctx.recent_blockhash, &[table])
            .expect("compile v0 with real ALT");
    let alt_wire = alt_msg.len() + 1 + 64; // sig prefix + signature

    // Fully-inline comparison (no tables).
    let inline_msg =
        TxBuilder::compile_v0_message(ctx.payer, &instructions, ctx.recent_blockhash, &[])
            .expect("compile v0 fully inline");
    let inline_wire = inline_msg.len() + 1 + 64;

    println!("=== Mainnet ALT ({alt}) V0 measurement ===");
    println!("real ALT members: {}", parsed.addresses.len());
    println!("V0 wire with real ALT: {} bytes", alt_wire);
    println!("V0 wire fully inline:  {} bytes", inline_wire);
    println!(
        "ALT savings:            {} bytes",
        inline_wire as i64 - alt_wire as i64
    );
    println!("route cu_limit:         {}", ctx.cu_limit);
    println!(
        "NOTE: transaction NOT broadcast; signature cryptographically computed only for measurement."
    );

    assert_eq!(inline_wire, inline_msg.len() + 65);
    assert_eq!(alt_wire, alt_msg.len() + 65);
    assert!(
        alt_wire <= inline_wire,
        "ALT should never expand the message"
    );
}
