use super::*;
use crate::ProviderId;
use std::sync::Arc;

#[test]
fn rejects_disabled_buy_or_sell_in_global_config() {
    let mut data = vec![0u8; 949];
    data[..8].copy_from_slice(&PUMP_SWAP_GLOBAL_CONFIG_DISCRIMINATOR);
    data[56] = 1 << 3;
    assert_eq!(decode_pump_swap_disable_flags(&data).unwrap(), 1 << 3);
    data[56] = 0;
    data[940] = 1;
    data[941..949].copy_from_slice(&500u64.to_le_bytes());
    let decoded = decode_pump_swap_global_config(&data).unwrap();
    assert_eq!(decoded.disable_flags, 0);
    assert!(decoded.creator_fee_configurable);
    assert_eq!(decoded.max_configurable_creator_fee_bps, 500);
    let legacy = vec![0u8; 940];
    let mut legacy = legacy;
    legacy[..8].copy_from_slice(&PUMP_SWAP_GLOBAL_CONFIG_DISCRIMINATOR);
    assert!(
        !decode_pump_swap_global_config(&legacy)
            .unwrap()
            .creator_fee_configurable
    );
}

#[test]
fn decodes_fee_config_standard_and_stable_tiers_without_guessing() {
    let mut data = vec![0u8; 65];
    data[..8].copy_from_slice(&PUMP_FEES_FEE_CONFIG_DISCRIMINATOR);
    data[8] = 1;
    // On-chain BPS values.
    data[41..49].copy_from_slice(&20u64.to_le_bytes());
    data[49..57].copy_from_slice(&5u64.to_le_bytes());
    data[57..65].copy_from_slice(&5u64.to_le_bytes());
    // One standard tier and one stable tier, each with threshold 0.
    data.extend_from_slice(&1u32.to_le_bytes());
    data.extend_from_slice(&0u128.to_le_bytes());
    data.extend_from_slice(&20u64.to_le_bytes());
    data.extend_from_slice(&930u64.to_le_bytes());
    data.extend_from_slice(&30u64.to_le_bytes());
    data.extend_from_slice(&1u32.to_le_bytes());
    data.extend_from_slice(&0u128.to_le_bytes());
    data.extend_from_slice(&20u64.to_le_bytes());
    data.extend_from_slice(&930u64.to_le_bytes());
    data.extend_from_slice(&30u64.to_le_bytes());
    // Exotic flat fee schedule.
    data.extend_from_slice(&20u64.to_le_bytes());
    data.extend_from_slice(&5u64.to_le_bytes());
    data.extend_from_slice(&5u64.to_le_bytes());
    data.resize(4_097, 0);
    let config = PumpFeesFeeConfig::decode(&data).unwrap();
    assert_eq!(config.fee_tiers.len(), 1);
    assert_eq!(config.stable_fee_tiers.len(), 1);
    assert_eq!(config.exotic_flat_fees.unwrap().lp, 20);
    assert_eq!(
        config.fee_tiers[0].fees.to_ppm().unwrap(),
        (2_000, 93_000, 3_000)
    );
}

#[test]
fn token_account_decoder_checks_mint_authority_and_state() {
    let token_program = parse_program_id(TOKEN_PROGRAM_ID).unwrap();
    let token_2022 = parse_program_id(TOKEN_2022_PROGRAM_ID).unwrap();
    let mint = Pubkey::new_unique();
    let authority = Pubkey::new_unique();
    let mut data = vec![0u8; TOKEN_ACCOUNT_BASE_LEN];
    data[..32].copy_from_slice(mint.as_ref());
    data[32..64].copy_from_slice(authority.as_ref());
    data[64..72].copy_from_slice(&1234u64.to_le_bytes());
    data[108] = 1;
    let snapshot = AccountSnapshot {
        pubkey: Pubkey::new_unique(),
        owner: token_program,
        slot: 99,
        write_version: 1,
        is_startup: false,
        provider_id: ProviderId(1),
        bank_id: Some(1),
        generation: 0,
        data: Arc::new(data.clone()),
    };
    let (amount, program) = decode_token_account(
        &snapshot,
        &mint,
        &authority,
        &token_program,
        &token_2022,
        "fixture",
    )
    .unwrap();
    assert_eq!(amount, 1234);
    assert_eq!(program, token_program);

    let mut frozen = snapshot.clone();
    let mut frozen_data = data;
    frozen_data[108] = 2;
    frozen.data = Arc::new(frozen_data);
    assert!(
        decode_token_account(
            &frozen,
            &mint,
            &authority,
            &token_program,
            &token_2022,
            "fixture"
        )
        .is_err()
    );
}

#[test]
fn mint_decoder_uses_verified_token_program_supply_and_decimals() {
    let token_program = parse_program_id(TOKEN_PROGRAM_ID).unwrap();
    let mut data = vec![0u8; MINT_BASE_LEN];
    data[36..44].copy_from_slice(&1_000_000_000u64.to_le_bytes());
    data[44] = 6;
    data[45] = 1;
    let snapshot = AccountSnapshot {
        pubkey: Pubkey::new_unique(),
        owner: token_program,
        slot: 1,
        write_version: 1,
        is_startup: true,
        provider_id: ProviderId(1),
        bank_id: Some(1),
        generation: 0,
        data: Arc::new(data),
    };
    assert_eq!(
        decode_mint(&snapshot, &token_program, "fixture").unwrap(),
        (6, 1_000_000_000)
    );
}

fn put_snapshot(
    store: &AccountSnapshotStore,
    key: Pubkey,
    owner: Pubkey,
    slot: u64,
    data: Vec<u8>,
) {
    assert!(store.apply(AccountSnapshot {
        pubkey: key,
        owner,
        slot,
        write_version: 1,
        is_startup: false,
        provider_id: ProviderId(1),
        bank_id: Some(1),
        generation: 0,
        data: Arc::new(data),
    }));
}

fn token_account(mint: Pubkey, authority: Pubkey, amount: u64) -> Vec<u8> {
    let mut data = vec![0u8; TOKEN_ACCOUNT_BASE_LEN];
    data[..32].copy_from_slice(mint.as_ref());
    data[32..64].copy_from_slice(authority.as_ref());
    data[64..72].copy_from_slice(&amount.to_le_bytes());
    data[108] = 1;
    data
}

fn mint_account(decimals: u8) -> Vec<u8> {
    let mut data = vec![0u8; MINT_BASE_LEN];
    data[36..44].copy_from_slice(&1_000_000_000u64.to_le_bytes());
    data[44] = decimals;
    data[45] = 1;
    data
}

#[test]
fn joins_only_owner_verified_same_slot_pool_and_vault_state() {
    let store = AccountSnapshotStore::default();
    let pump_swap_program = parse_program_id(PUMP_SWAP_PROGRAM_ID).unwrap();
    let pump_fees_program = parse_program_id(PUMP_FEES_PROGRAM_ID).unwrap();
    let token_program = parse_program_id(TOKEN_PROGRAM_ID).unwrap();
    let pool_creator = Pubkey::new_unique();
    let base_mint = Pubkey::new_unique();
    let quote_mint = Pubkey::new_unique();
    let index_seed = 0u16.to_le_bytes();
    let (pool_key, pool_bump) = Pubkey::find_program_address(
        &[
            b"pool",
            &index_seed,
            pool_creator.as_ref(),
            base_mint.as_ref(),
            quote_mint.as_ref(),
        ],
        &pump_swap_program,
    );
    let associated_program = parse_program_id(ASSOCIATED_TOKEN_PROGRAM_ID).unwrap();
    let base_vault =
        associated_token_address(&pool_key, &base_mint, &token_program, &associated_program);
    let quote_vault =
        associated_token_address(&pool_key, &quote_mint, &token_program, &associated_program);
    let fee_config_key = Pubkey::find_program_address(
        &[b"fee_config", pump_swap_program.as_ref()],
        &pump_fees_program,
    )
    .0;

    let mut pool_data = vec![0u8; 271];
    pool_data[..8].copy_from_slice(&arb_constants::PUMP_SWAP_POOL_DISCRIMINATOR);
    pool_data[8] = pool_bump;
    pool_data[9..11].copy_from_slice(&index_seed);
    pool_data[11..43].copy_from_slice(pool_creator.as_ref());
    pool_data[43..75].copy_from_slice(base_mint.as_ref());
    pool_data[75..107].copy_from_slice(quote_mint.as_ref());
    pool_data[107..139].copy_from_slice(Pubkey::new_unique().as_ref());
    pool_data[139..171].copy_from_slice(base_vault.as_ref());
    pool_data[171..203].copy_from_slice(quote_vault.as_ref());
    pool_data[211..243].copy_from_slice(Pubkey::new_unique().as_ref());

    put_snapshot(&store, pool_key, pump_swap_program, 500, pool_data);
    put_snapshot(
        &store,
        base_vault,
        token_program,
        500,
        token_account(base_mint, pool_key, 5_000_000),
    );
    put_snapshot(
        &store,
        quote_vault,
        token_program,
        500,
        token_account(quote_mint, pool_key, 10_000_000),
    );
    put_snapshot(&store, base_mint, token_program, 499, mint_account(6));
    put_snapshot(&store, quote_mint, token_program, 499, mint_account(9));
    let mut fee_data = vec![0u8; 65];
    fee_data[..8].copy_from_slice(&PUMP_FEES_FEE_CONFIG_DISCRIMINATOR);
    fee_data[8] = Pubkey::find_program_address(
        &[b"fee_config", pump_swap_program.as_ref()],
        &pump_fees_program,
    )
    .1;
    put_snapshot(&store, fee_config_key, pump_fees_program, 500, fee_data);
    let global_config_key = parse_program_id(PUMP_SWAP_GLOBAL_CONFIG).unwrap();
    let mut global_data = vec![0u8; 949];
    global_data[..8].copy_from_slice(&PUMP_SWAP_GLOBAL_CONFIG_DISCRIMINATOR);
    put_snapshot(
        &store,
        global_config_key,
        pump_swap_program,
        500,
        global_data,
    );

    let joined = join_pump_swap_state(&store, pool_key, fee_config_key).unwrap();
    assert_eq!(joined.slot, 500);
    assert_eq!(joined.base_vault_amount, 5_000_000);
    assert_eq!(joined.quote_vault_amount, 10_000_000);
    assert_eq!((joined.base_decimals, joined.quote_decimals), (6, 9));
    assert!(joined.trading_enabled().unwrap());
}

#[test]
fn global_config_buy_and_sell_disable_bits_are_directional() {
    let mut global_data = vec![0u8; 949];
    global_data[..8].copy_from_slice(&PUMP_SWAP_GLOBAL_CONFIG_DISCRIMINATOR);
    global_data[56] = 1 << 3; // buy disabled, sell still enabled
    let joined = PumpSwapJoinedState {
        pool_key: Pubkey::new_unique(),
        slot: 100,
        header: PumpSwapPoolHeader {
            pool_bump: 1,
            index: 0,
            creator: Pubkey::new_unique(),
            base_mint: Pubkey::new_unique(),
            quote_mint: Pubkey::new_unique(),
            lp_mint: Pubkey::new_unique(),
            base_vault: Pubkey::new_unique(),
            quote_vault: Pubkey::new_unique(),
            lp_supply: 1,
            coin_creator: Pubkey::new_unique(),
            is_mayhem_mode: false,
            is_cashback_coin: false,
            virtual_quote_reserves: 0,
            creator_fee_bps: 0,
            can_edit_creator_fee: false,
            is_holder_reward: false,
            protocol_fees: 0,
            creator_fees: 0,
        },
        base_vault_amount: 1_000,
        quote_vault_amount: 2_000,
        base_decimals: 6,
        quote_decimals: 9,
        base_mint_supply: 1_000_000,
        fee_config_account: AccountSnapshot {
            pubkey: Pubkey::new_unique(),
            owner: Pubkey::new_unique(),
            slot: 100,
            write_version: 1,
            is_startup: false,
            provider_id: ProviderId(1),
            bank_id: Some(1),
            generation: 0,
            data: Arc::new(Vec::new()),
        },
        fee_config_bump: 0,
        global_config_account: AccountSnapshot {
            pubkey: Pubkey::new_unique(),
            owner: parse_program_id(PUMP_SWAP_PROGRAM_ID).unwrap(),
            slot: 100,
            write_version: 1,
            is_startup: false,
            provider_id: ProviderId(1),
            bank_id: Some(1),
            generation: 0,
            data: Arc::new(global_data),
        },
    };
    assert!(joined.trading_enabled().unwrap());
    assert!(!joined.trading_enabled_for(true).unwrap());
    assert!(joined.trading_enabled_for(false).unwrap());
}

#[test]
fn rejects_cross_slot_pool_vault_join() {
    let store = AccountSnapshotStore::default();
    let pump_swap_program = parse_program_id(PUMP_SWAP_PROGRAM_ID).unwrap();
    let token_program = parse_program_id(TOKEN_PROGRAM_ID).unwrap();
    let pool_creator = Pubkey::new_unique();
    let base_mint = Pubkey::new_unique();
    let quote_mint = Pubkey::new_unique();
    let index_seed = 0u16.to_le_bytes();
    let (pool_key, pool_bump) = Pubkey::find_program_address(
        &[
            b"pool",
            &index_seed,
            pool_creator.as_ref(),
            base_mint.as_ref(),
            quote_mint.as_ref(),
        ],
        &pump_swap_program,
    );
    let associated_program = parse_program_id(ASSOCIATED_TOKEN_PROGRAM_ID).unwrap();
    let base_vault =
        associated_token_address(&pool_key, &base_mint, &token_program, &associated_program);
    let quote_vault =
        associated_token_address(&pool_key, &quote_mint, &token_program, &associated_program);
    let mut pool_data = vec![0u8; 271];
    pool_data[..8].copy_from_slice(&arb_constants::PUMP_SWAP_POOL_DISCRIMINATOR);
    pool_data[8] = pool_bump;
    pool_data[9..11].copy_from_slice(&index_seed);
    pool_data[11..43].copy_from_slice(pool_creator.as_ref());
    pool_data[43..75].copy_from_slice(base_mint.as_ref());
    pool_data[75..107].copy_from_slice(quote_mint.as_ref());
    pool_data[139..171].copy_from_slice(base_vault.as_ref());
    pool_data[171..203].copy_from_slice(quote_vault.as_ref());
    let pump_fees_program = parse_program_id(PUMP_FEES_PROGRAM_ID).unwrap();
    let fee_config_key = Pubkey::find_program_address(
        &[b"fee_config", pump_swap_program.as_ref()],
        &pump_fees_program,
    )
    .0;
    put_snapshot(&store, pool_key, pump_swap_program, 500, pool_data);
    put_snapshot(
        &store,
        base_vault,
        token_program,
        499,
        token_account(base_mint, pool_key, 1_000),
    );
    put_snapshot(
        &store,
        quote_vault,
        token_program,
        500,
        token_account(quote_mint, pool_key, 2_000),
    );
    put_snapshot(&store, base_mint, token_program, 499, mint_account(6));
    put_snapshot(&store, quote_mint, token_program, 499, mint_account(9));
    let mut fee_data = vec![0u8; 65];
    fee_data[..8].copy_from_slice(&PUMP_FEES_FEE_CONFIG_DISCRIMINATOR);
    fee_data[8] = Pubkey::find_program_address(
        &[b"fee_config", pump_swap_program.as_ref()],
        &pump_fees_program,
    )
    .1;
    put_snapshot(&store, fee_config_key, pump_fees_program, 500, fee_data);
    let global_config_key = parse_program_id(PUMP_SWAP_GLOBAL_CONFIG).unwrap();
    let mut global_data = vec![0u8; 949];
    global_data[..8].copy_from_slice(&PUMP_SWAP_GLOBAL_CONFIG_DISCRIMINATOR);
    put_snapshot(
        &store,
        global_config_key,
        pump_swap_program,
        500,
        global_data,
    );
    let err = join_pump_swap_state(&store, pool_key, fee_config_key).unwrap_err();
    assert!(err.to_string().contains("pool/vault slot mismatch"));
}
