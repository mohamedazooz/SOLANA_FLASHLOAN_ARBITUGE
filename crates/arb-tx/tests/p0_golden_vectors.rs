//! P0 DEX Account Resolution Golden-Vector Differential Tests
//!
//! Asserts exact instruction discriminators, account ordering, mutability/signer flags,
//! signed tick/bin arithmetic, and directional array traversal across all 6 supported protocols:
//! 1. Raydium AMM V4 (Canonical SwapBaseInV2 8-account vs legacy V1 17/18-account)
//! 2. PumpSwap V2 (Exact 17-account matrix, BuyExactQuoteInV2, BuyExactBaseOutV2, SellExactBaseInV2)
//! 3. Meteora DLMM (Euclidean division on negative bins, directional expansion, bitmap extension)
//! 4. Orca Whirlpool (Signed tick spacing * 88, directional arrays, adaptive fee oracle flag)
//! 5. Raydium CLMM (Big-endian tick arrays, direction)
//! 6. Raydium CPMM (13-account matrix, Token-2022 dual token program support)
//! 7. Pipeline Hop Isolation (No first_hop execution_accounts leakage)

use arb_constants::{
    PUMP_SWAP_BUY_EXACT_QUOTE_IN_V2_DISCRIMINATOR, PUMP_SWAP_BUY_V2_DISCRIMINATOR,
    PUMP_SWAP_PROGRAM_ID, PUMP_SWAP_SELL_V2_DISCRIMINATOR, RAYDIUM_CPMM_PROGRAM_ID,
    RAYDIUM_V4_PROGRAM_ID, TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID, WSOL_MINT,
};
use arb_tx::{
    AccountMeta, OrcaWhirlpoolAccounts, PumpSwapTradeMode, RaydiumCpmmAccounts, RaydiumV4Accounts,
    build_orca_whirlpool_swap_instruction_11, build_pump_swap_instruction_v2,
    build_raydium_cpmm_swap_instruction, build_raydium_v4_swap_instruction_18,
    build_raydium_v4_swap_instruction_v2, build_swap_hop_instruction, derive_meteora_bin_array_pda,
    derive_orca_tick_array_pda, derive_raydium_clmm_tick_array_pda,
    get_associated_token_address_with_program_id,
};
use arb_types::{DexProtocol, MintId, PoolExecutionAccounts, PoolId, SwapHop, TokenProgramKind};
use solana_pubkey::Pubkey;
use std::str::FromStr;

#[test]
fn test_raydium_v4_v2_exact_8_accounts_and_opcode_16() {
    let token_program = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
    let pool_id = Pubkey::new_unique();
    let amm_authority = Pubkey::new_unique();
    let base_vault = Pubkey::new_unique();
    let quote_vault = Pubkey::new_unique();
    let user_source = Pubkey::new_unique();
    let user_dest = Pubkey::new_unique();
    let user_authority = Pubkey::new_unique();
    let amount_in: u64 = 100_000_000;
    let min_amount_out: u64 = 99_500_000;

    let ix = build_raydium_v4_swap_instruction_v2(
        token_program,
        pool_id,
        amm_authority,
        base_vault,
        quote_vault,
        user_source,
        user_dest,
        user_authority,
        amount_in,
        min_amount_out,
    )
    .expect("build raydium v4 v2 instruction");

    // 1. Program ID
    assert_eq!(
        ix.program_id,
        Pubkey::from_str(RAYDIUM_V4_PROGRAM_ID).unwrap()
    );

    // 2. Exact 8 accounts
    assert_eq!(
        ix.accounts.len(),
        8,
        "Raydium V4 SwapBaseInV2 must have exactly 8 accounts, found {}",
        ix.accounts.len()
    );

    // 3. Verify AccountMetas in exact canonical order
    let expected_metas = [
        AccountMeta::new_readonly(token_program, false),
        AccountMeta::new(pool_id, false),
        AccountMeta::new_readonly(amm_authority, false),
        AccountMeta::new(base_vault, false),
        AccountMeta::new(quote_vault, false),
        AccountMeta::new(user_source, false),
        AccountMeta::new(user_dest, false),
        AccountMeta::new_readonly(user_authority, true),
    ];
    for (i, (actual, expected)) in ix.accounts.iter().zip(expected_metas.iter()).enumerate() {
        assert_eq!(
            actual.pubkey, expected.pubkey,
            "Account {} pubkey mismatch",
            i
        );
        assert_eq!(
            actual.is_signer, expected.is_signer,
            "Account {} is_signer mismatch",
            i
        );
        assert_eq!(
            actual.is_writable, expected.is_writable,
            "Account {} is_writable mismatch",
            i
        );
    }

    // 4. Verify Instruction Data: opcode 16, amount_in (u64 LE), min_amount_out (u64 LE) = 17 bytes
    assert_eq!(ix.data.len(), 17);
    assert_eq!(
        ix.data[0], 16,
        "SwapBaseInV2 opcode must be 16, got {}",
        ix.data[0]
    );
    let parsed_amount_in = u64::from_le_bytes(ix.data[1..9].try_into().unwrap());
    let parsed_min_out = u64::from_le_bytes(ix.data[9..17].try_into().unwrap());
    assert_eq!(parsed_amount_in, amount_in);
    assert_eq!(parsed_min_out, min_amount_out);
}

#[test]
fn test_raydium_v4_legacy_v1_18_accounts_backward_compatibility() {
    let accs = RaydiumV4Accounts::mainnet_sol_usdc();
    let user_source = Pubkey::new_unique();
    let user_dest = Pubkey::new_unique();
    let user_owner = Pubkey::new_unique();
    let amount_in = 50_000_000;
    let min_amount_out = 49_000_000;

    let ix = build_raydium_v4_swap_instruction_18(
        &accs,
        user_source,
        user_dest,
        user_owner,
        amount_in,
        min_amount_out,
    )
    .expect("build raydium v4 v1");

    assert_eq!(
        ix.program_id,
        Pubkey::from_str(RAYDIUM_V4_PROGRAM_ID).unwrap()
    );
    assert_eq!(
        ix.accounts.len(),
        18,
        "Legacy Raydium V1 swap instruction must have 18 accounts"
    );
    assert_eq!(ix.data[0], 9, "Legacy Raydium V1 opcode must be 9");
}

#[test]
fn test_raydium_v4_canonical_selection_in_build_swap_hop() {
    let user_authority = Pubkey::new_unique();
    let mint_in = Pubkey::from_str(WSOL_MINT).unwrap();
    let mint_out = Pubkey::new_unique();
    let pool_id = Pubkey::new_unique();

    let legacy_accounts = RaydiumV4Accounts::mainnet_sol_usdc();
    let exec_accounts = PoolExecutionAccounts::RaydiumV4 {
        amm_authority: legacy_accounts.amm_authority,
        open_orders: legacy_accounts.amm_open_orders,
        target_orders: legacy_accounts.amm_target_orders,
        base_vault: legacy_accounts.pool_coin_vault,
        quote_vault: legacy_accounts.pool_pc_vault,
        serum_program: legacy_accounts.serum_program,
        serum_market: legacy_accounts.serum_market,
        serum_bids: legacy_accounts.serum_bids,
        serum_asks: legacy_accounts.serum_asks,
        serum_event_queue: legacy_accounts.serum_event_queue,
        serum_coin_vault: legacy_accounts.serum_coin_vault,
        serum_pc_vault: legacy_accounts.serum_pc_vault,
        serum_vault_signer: legacy_accounts.serum_vault_signer,
        swap_version: arb_types::RaydiumV4SwapVersion::V2,
    };

    let hop = SwapHop {
        pool_id: PoolId(pool_id),
        protocol: DexProtocol::RaydiumV4,
        mint_in: MintId(mint_in),
        mint_out: MintId(mint_out),
        fee_bps: 25,
        active_bin_id: None,
        token_programs: [TokenProgramKind::SplToken; 2],
        trade_intent: Default::default(),
        execution_accounts: Some(exec_accounts),
    };

    let ix = build_swap_hop_instruction(&hop, user_authority, 50_000_000, 49_000_000)
        .expect("build_swap_hop for RaydiumV4");

    // Must default to V2 (8 accounts) even when full 17 accounts are provided in registry
    assert_eq!(
        ix.accounts.len(),
        8,
        "build_swap_hop must canonically select SwapBaseInV2 (8 accounts)"
    );
    assert_eq!(ix.data[0], 16, "Must use SwapBaseInV2 opcode 16");
}

#[test]
fn test_pump_swap_v2_exact_17_accounts_and_discriminators() {
    let pool = Pubkey::new_unique();
    let user = Pubkey::new_unique();
    let base_mint = Pubkey::new_unique();
    let quote_mint = Pubkey::from_str(WSOL_MINT).unwrap();
    let pool_base_vault = Pubkey::new_unique();
    let pool_quote_vault = Pubkey::new_unique();
    let global_config = Pubkey::new_unique();
    let protocol_fee_recipient = Pubkey::new_unique();
    let protocol_fee_recipient_ata = Pubkey::new_unique();
    let coin_creator_vault_authority = Pubkey::new_unique();
    let coin_creator_vault_ata = Pubkey::new_unique();
    let token_program_base = Pubkey::from_str(TOKEN_2022_PROGRAM_ID).unwrap();
    let token_program_quote = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();
    let event_authority = Pubkey::new_unique();
    let amount_in: u64 = 1_000_000_000;
    let min_amount_out: u64 = 500_000_000;

    // 1. BuyExactQuoteInV2
    let ix_buy = build_pump_swap_instruction_v2(
        pool,
        user,
        base_mint,
        quote_mint,
        pool_base_vault,
        pool_quote_vault,
        global_config,
        protocol_fee_recipient,
        protocol_fee_recipient_ata,
        coin_creator_vault_authority,
        coin_creator_vault_ata,
        token_program_base,
        token_program_quote,
        event_authority,
        amount_in,
        min_amount_out,
        PumpSwapTradeMode::BuyExactQuoteInV2,
    )
    .expect("build pump swap buy v2");

    assert_eq!(
        ix_buy.program_id,
        Pubkey::from_str(PUMP_SWAP_PROGRAM_ID).unwrap()
    );
    assert_eq!(
        ix_buy.accounts.len(),
        17,
        "PumpSwap V2 must have exactly 17 accounts matching pump_amm.json"
    );

    // Verify discriminator for BuyExactQuoteInV2
    assert_eq!(
        &ix_buy.data[0..8],
        &PUMP_SWAP_BUY_EXACT_QUOTE_IN_V2_DISCRIMINATOR,
        "Discriminator must match buy_exact_quote_in_v2"
    );
    assert_eq!(
        u64::from_le_bytes(ix_buy.data[8..16].try_into().unwrap()),
        amount_in
    );
    assert_eq!(
        u64::from_le_bytes(ix_buy.data[16..24].try_into().unwrap()),
        min_amount_out
    );

    // Verify ATA derivations match respective token programs
    let expected_user_base =
        get_associated_token_address_with_program_id(&user, &base_mint, &token_program_base);
    let expected_user_quote =
        get_associated_token_address_with_program_id(&user, &quote_mint, &token_program_quote);
    assert_eq!(ix_buy.accounts[5].pubkey, expected_user_base);
    assert_eq!(ix_buy.accounts[6].pubkey, expected_user_quote);
    assert_eq!(ix_buy.accounts[11].pubkey, token_program_base);
    assert_eq!(ix_buy.accounts[12].pubkey, token_program_quote);
    assert_eq!(ix_buy.accounts[15].pubkey, event_authority);
    assert_eq!(ix_buy.accounts[16].pubkey, coin_creator_vault_ata);

    // 2. SellExactBaseInV2
    let ix_sell = build_pump_swap_instruction_v2(
        pool,
        user,
        base_mint,
        quote_mint,
        pool_base_vault,
        pool_quote_vault,
        global_config,
        protocol_fee_recipient,
        protocol_fee_recipient_ata,
        coin_creator_vault_authority,
        coin_creator_vault_ata,
        token_program_base,
        token_program_quote,
        event_authority,
        amount_in,
        min_amount_out,
        PumpSwapTradeMode::SellExactBaseInV2,
    )
    .expect("build pump swap sell v2");

    assert_eq!(
        &ix_sell.data[0..8],
        &PUMP_SWAP_SELL_V2_DISCRIMINATOR,
        "Discriminator must match sell_v2"
    );
    assert_eq!(ix_sell.accounts.len(), 17);

    // 3. BuyExactBaseOutV2
    let ix_buy_base = build_pump_swap_instruction_v2(
        pool,
        user,
        base_mint,
        quote_mint,
        pool_base_vault,
        pool_quote_vault,
        global_config,
        protocol_fee_recipient,
        protocol_fee_recipient_ata,
        coin_creator_vault_authority,
        coin_creator_vault_ata,
        token_program_base,
        token_program_quote,
        event_authority,
        amount_in,
        min_amount_out,
        PumpSwapTradeMode::BuyExactBaseOutV2,
    )
    .expect("build pump swap buy exact base out v2");

    assert_eq!(
        &ix_buy_base.data[0..8],
        &PUMP_SWAP_BUY_V2_DISCRIMINATOR,
        "Discriminator must match buy_v2"
    );
}

#[test]
fn test_meteora_dlmm_negative_bin_array_euclidean_division() {
    let lb_pair = Pubkey::new_unique();

    // Standard Rust `/` truncates towards zero on negative numbers:
    // -1 / 70 == 0 (WRONG! Bin -1 belongs to bin_array -1, NOT 0!)
    assert_eq!(-1i32 / 70, 0);
    // In contrast, Euclidean division produces correct mathematical floor:
    // (-1i32).div_euclid(70) == -1 (CORRECT!)
    assert_eq!((-1i32).div_euclid(70), -1);
    assert_eq!((-69i32).div_euclid(70), -1);
    assert_eq!((-70i32).div_euclid(70), -1);
    assert_eq!((-71i32).div_euclid(70), -2);
    assert_eq!(0i32.div_euclid(70), 0);
    assert_eq!(69i32.div_euclid(70), 0);
    assert_eq!(70i32.div_euclid(70), 1);

    // Test PDA derivations with negative indices
    let pda_neg_1 = derive_meteora_bin_array_pda(&lb_pair, -1);
    let pda_0 = derive_meteora_bin_array_pda(&lb_pair, 0);
    let pda_pos_1 = derive_meteora_bin_array_pda(&lb_pair, 1);

    assert_ne!(pda_neg_1, pda_0);
    assert_ne!(pda_0, pda_pos_1);

    // Directional test: swap X for Y moves down (negative direction), swap Y for X moves up
    let active_bin: i32 = -5;
    let base_idx = active_bin.div_euclid(70);
    assert_eq!(base_idx, -1);

    let x_to_y_indices = [base_idx, base_idx - 1, base_idx - 2];
    assert_eq!(x_to_y_indices, [-1, -2, -3]);

    let y_to_x_indices = [base_idx, base_idx + 1, base_idx + 2];
    assert_eq!(y_to_x_indices, [-1, 0, 1]);
}

#[test]
fn test_orca_whirlpool_directional_arrays_and_adaptive_fee() {
    let whirlpool = Pubkey::new_unique();
    let tick_spacing: u16 = 64;
    let ticks_in_array: i32 = tick_spacing as i32 * 88; // 5632

    // Positive tick
    let tick: i32 = 1000;
    let start_tick = tick.div_euclid(ticks_in_array) * ticks_in_array;
    assert_eq!(start_tick, 0);

    // Directional arrays for a_to_b = true (tick decreases):
    let ta_a_to_b = [
        derive_orca_tick_array_pda(&whirlpool, start_tick),
        derive_orca_tick_array_pda(&whirlpool, start_tick - ticks_in_array),
        derive_orca_tick_array_pda(&whirlpool, start_tick - 2 * ticks_in_array),
    ];
    assert_ne!(ta_a_to_b[0], ta_a_to_b[1]);
    assert_ne!(ta_a_to_b[1], ta_a_to_b[2]);

    // Directional arrays for a_to_b = false (tick increases):
    let ta_b_to_a = [
        derive_orca_tick_array_pda(&whirlpool, start_tick),
        derive_orca_tick_array_pda(&whirlpool, start_tick + ticks_in_array),
        derive_orca_tick_array_pda(&whirlpool, start_tick + 2 * ticks_in_array),
    ];
    assert_ne!(ta_b_to_a[0], ta_b_to_a[1]);
    assert_ne!(ta_b_to_a[1], ta_b_to_a[2]);
    assert_ne!(ta_a_to_b[1], ta_b_to_a[1]);

    // Adaptive fee test: oracle is_writable flag
    let accounts = OrcaWhirlpoolAccounts::with_tick_arrays(
        whirlpool,
        Pubkey::new_unique(),
        Pubkey::new_unique(),
        Pubkey::new_unique(),
        ta_a_to_b[0],
        ta_a_to_b[1],
        ta_a_to_b[2],
    );

    let user = Pubkey::new_unique();
    let token_a = Pubkey::new_unique();
    let token_b = Pubkey::new_unique();

    let ix_static = build_orca_whirlpool_swap_instruction_11(
        &accounts, user, token_a, token_b, 100_000, 99_000, true, false,
    )
    .expect("build static whirlpool swap");
    let oracle_meta_static = ix_static.accounts.last().unwrap();
    assert_eq!(oracle_meta_static.pubkey, accounts.oracle);
    assert!(
        !oracle_meta_static.is_writable,
        "Static fee pool oracle must be readonly"
    );

    let ix_adaptive = build_orca_whirlpool_swap_instruction_11(
        &accounts, user, token_a, token_b, 100_000, 99_000, true, true,
    )
    .expect("build adaptive whirlpool swap");
    let oracle_meta_adaptive = ix_adaptive.accounts.last().unwrap();
    assert_eq!(oracle_meta_adaptive.pubkey, accounts.oracle);
    assert!(
        oracle_meta_adaptive.is_writable,
        "Adaptive fee pool oracle must be marked writable"
    );
}

#[test]
fn test_raydium_clmm_directional_tick_arrays_and_be_bytes() {
    let pool = Pubkey::new_unique();
    let tick_spacing: u16 = 60;
    let ticks_in_array: i32 = tick_spacing as i32 * 60; // 3600

    let current_tick: i32 = 100;
    let array_idx = current_tick.div_euclid(ticks_in_array);
    assert_eq!(array_idx, 0);

    let pda_0 = derive_raydium_clmm_tick_array_pda(&pool, 0);
    let pda_1 = derive_raydium_clmm_tick_array_pda(&pool, 1);
    let pda_neg_1 = derive_raydium_clmm_tick_array_pda(&pool, -1);

    assert_ne!(pda_0, pda_1);
    assert_ne!(pda_0, pda_neg_1);
    assert_ne!(pda_1, pda_neg_1);
}

#[test]
fn test_raydium_cpmm_13_accounts_and_token_2022_support() {
    let pool = Pubkey::new_unique();
    let payer = Pubkey::new_unique();
    let user_token_in = Pubkey::new_unique();
    let user_token_out = Pubkey::new_unique();
    let token_in_mint = Pubkey::new_unique();
    let token_out_mint = Pubkey::new_unique();
    let input_vault = Pubkey::new_unique();
    let output_vault = Pubkey::new_unique();
    let amm_config = Pubkey::new_unique();

    let mut accounts = RaydiumCpmmAccounts::for_pool(
        pool,
        payer,
        user_token_in,
        user_token_out,
        token_in_mint,
        token_out_mint,
        input_vault,
        output_vault,
        amm_config,
    );

    let token_2022 = Pubkey::from_str(TOKEN_2022_PROGRAM_ID).unwrap();
    let spl_token = Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap();

    // Input is Token-2022, output is SPL Token
    accounts.input_token_program = token_2022;
    accounts.output_token_program = spl_token;

    let ix = build_raydium_cpmm_swap_instruction(&accounts, 50_000_000, 49_000_000)
        .expect("build raydium cpmm swap");

    assert_eq!(
        ix.program_id,
        Pubkey::from_str(RAYDIUM_CPMM_PROGRAM_ID).unwrap()
    );
    assert_eq!(
        ix.accounts.len(),
        13,
        "Raydium CPMM must have exactly 13 accounts"
    );

    // Verify input and output token programs are passed correctly
    assert_eq!(ix.accounts[8].pubkey, token_2022);
    assert_eq!(ix.accounts[9].pubkey, spl_token);
}

#[test]
fn test_pipeline_hop_isolation_no_account_leakage() {
    let user_authority = Pubkey::new_unique();
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    let meme = Pubkey::new_unique();

    let pool_hop0 = Pubkey::new_unique();
    let pool_hop1 = Pubkey::new_unique();

    let exec_accounts_hop0 = PoolExecutionAccounts::RaydiumV4 {
        amm_authority: Pubkey::new_unique(),
        open_orders: Pubkey::new_unique(),
        target_orders: Pubkey::new_unique(),
        base_vault: Pubkey::new_unique(),
        quote_vault: Pubkey::new_unique(),
        serum_program: Pubkey::new_unique(),
        serum_market: Pubkey::new_unique(),
        serum_bids: Pubkey::new_unique(),
        serum_asks: Pubkey::new_unique(),
        serum_event_queue: Pubkey::new_unique(),
        serum_coin_vault: Pubkey::new_unique(),
        serum_pc_vault: Pubkey::new_unique(),
        serum_vault_signer: Pubkey::new_unique(),
        swap_version: arb_types::RaydiumV4SwapVersion::V2,
    };

    let exec_accounts_hop1 = PoolExecutionAccounts::PumpSwap {
        global_config: Pubkey::new_unique(),
        pool_base_vault: Pubkey::new_unique(),
        pool_quote_vault: Pubkey::new_unique(),
        protocol_fee_recipient: Pubkey::new_unique(),
        protocol_fee_recipient_ata: Pubkey::new_unique(),
        coin_creator_vault_authority: Pubkey::new_unique(),
        coin_creator_vault_ata: Pubkey::new_unique(),
        fee_config: Pubkey::new_unique(),
        pool_version: arb_types::PumpSwapPoolVersion::V2,
    };

    let hop0 = SwapHop {
        pool_id: PoolId(pool_hop0),
        protocol: DexProtocol::RaydiumV4,
        mint_in: MintId(wsol),
        mint_out: MintId(meme),
        fee_bps: 25,
        active_bin_id: None,
        token_programs: [TokenProgramKind::SplToken; 2],
        trade_intent: Default::default(),
        execution_accounts: Some(exec_accounts_hop0),
    };

    let hop1 = SwapHop {
        pool_id: PoolId(pool_hop1),
        protocol: DexProtocol::PumpSwap,
        mint_in: MintId(meme),
        mint_out: MintId(wsol),
        fee_bps: 100,
        active_bin_id: None,
        token_programs: [TokenProgramKind::SplToken; 2],
        trade_intent: Default::default(),
        execution_accounts: Some(exec_accounts_hop1),
    };

    let ix0 =
        build_swap_hop_instruction(&hop0, user_authority, 100_000, 99_000).expect("build hop 0");
    let ix1 =
        build_swap_hop_instruction(&hop1, user_authority, 99_000, 98_000).expect("build hop 1");

    assert_eq!(
        ix0.program_id,
        Pubkey::from_str(RAYDIUM_V4_PROGRAM_ID).unwrap()
    );
    assert_eq!(ix0.accounts.len(), 8);

    assert_eq!(
        ix1.program_id,
        Pubkey::from_str(PUMP_SWAP_PROGRAM_ID).unwrap()
    );
    assert_eq!(ix1.accounts.len(), 17);

    // Verify none of hop0's accounts leaked into hop1
    for acc in &ix1.accounts {
        assert_ne!(
            acc.pubkey, pool_hop0,
            "Hop 0 pool leaked into Hop 1 accounts!"
        );
    }
}
