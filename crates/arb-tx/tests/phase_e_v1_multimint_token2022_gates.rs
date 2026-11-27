//! Phase E Verification Suite for `arb-tx`:
//! - G-04: Transaction v1 (`0x81`) wire envelope, <= 64 inline accounts, <= 4,096 bytes.
//! - G-04b: Multi-Mint (`USDC` + `WSOL`) Kamino FlashBorrow/Repay assembly + ATA lifecycle.
//! - G-05: Strict V1 failure modes (exceeding 64 accounts or missing V1 config fails closed; never V0 downgrade).
//! - G-34: Zero silent truncation on V1 packet ceilings + Token-2022 `swap_v2` & `ExtraAccountMetaList` PDA resolver.

use arb_constants::{
    KAMINO_MAIN_MARKET, KAMINO_USDC_FEE_VAULT, KAMINO_USDC_RESERVE, KAMINO_USDC_SUPPLY_VAULT,
    RAYDIUM_CPMM_PROGRAM_ID, TOKEN_2022_PROGRAM_ID, TOKEN_PROGRAM_ID, USDC_MINT, WSOL_MINT,
};
use arb_tx::{
    AccountMeta, AdaptiveTxCompiler, CompiledInstruction, KaminoFlashParams, KeypairSigner,
    TransactionVersion, TxBuilder, V1ConfigParams, build_route_ata_plan,
    derive_transfer_hook_extra_account_meta_list_pda, parse_transaction_wire_header,
    parse_transfer_hook_extra_account_metas,
};
use arb_types::{
    Bps, DexProtocol, ExecutionOpportunity, Lamports, MintId, PoolExecutionAccounts, PoolId,
    PumpSwapTradeIntent, RouteId, Slot, SwapHop, TokenAmount, TokenProgramKind,
};
use solana_pubkey::Pubkey;
use std::str::FromStr;

fn test_signer() -> KeypairSigner {
    KeypairSigner::from_seed(&[7u8; 32])
}

#[test]
fn gate_g04_and_g04b_usdc_multi_mint_kamino_flash_loan_compiles_v1_cleanly() {
    let signer = test_signer();
    let payer = signer.pubkey();
    let usdc = Pubkey::from_str(USDC_MINT).unwrap();
    let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
    let tip_acc = Pubkey::new_unique();

    let params = KaminoFlashParams::for_mint(usdc, payer).expect("USDC anchor supported");
    assert_eq!(
        params.reserve,
        Pubkey::from_str(KAMINO_USDC_RESERVE).unwrap()
    );
    assert_eq!(
        params.reserve_source_liquidity,
        Pubkey::from_str(KAMINO_USDC_SUPPLY_VAULT).unwrap()
    );
    assert_eq!(
        params.reserve_liquidity_fee_receiver,
        Pubkey::from_str(KAMINO_USDC_FEE_VAULT).unwrap()
    );
    assert_eq!(
        params.lending_market,
        Pubkey::from_str(KAMINO_MAIN_MARKET).unwrap()
    );

    let cpmm_exec = || PoolExecutionAccounts::RaydiumCpmm {
        amm_config: Pubkey::new_unique(),
        authority: Pubkey::new_unique(),
        vault_0: Pubkey::new_unique(),
        vault_1: Pubkey::new_unique(),
        observation_state: Pubkey::new_unique(),
    };

    let opp = ExecutionOpportunity {
        hop_slippage_bps: 10,
        cu_estimate_units: 200_000,
        cu_shape_key: "usdc-v1-test".into(),
        route_id: RouteId(42),
        slot: Slot(500),
        size_in: TokenAmount(100_000_000), // 100 USDC (6 decimals)
        expected_out: TokenAmount(105_000_000), // 105 USDC
        hop_inputs: vec![100_000_000, 750_000_000, 105_000_000],
        gross_profit: Lamports(30_000_000),
        net_profit: Lamports(29_000_000),
        base_decimals: 6,
        price_impact_bps: Bps(5),
        hops: vec![
            SwapHop {
                pool_id: PoolId::new([11; 32]),
                protocol: DexProtocol::RaydiumCpmm,
                mint_in: MintId(usdc),
                mint_out: MintId(wsol),
                fee_bps: 25,
                active_bin_id: None,
                token_programs: [TokenProgramKind::SplToken, TokenProgramKind::SplToken],
                trade_intent: PumpSwapTradeIntent::default(),
                execution_accounts: Some(cpmm_exec()),
            },
            SwapHop {
                pool_id: PoolId::new([12; 32]),
                protocol: DexProtocol::RaydiumCpmm,
                mint_in: MintId(wsol),
                mint_out: MintId(usdc),
                fee_bps: 25,
                active_bin_id: None,
                token_programs: [TokenProgramKind::SplToken, TokenProgramKind::SplToken],
                trade_intent: PumpSwapTradeIntent::default(),
                execution_accounts: Some(cpmm_exec()),
            },
        ],
        expected_min_final_output: 104_500_000,
    };

    let ata_plan = build_route_ata_plan(payer, &opp.hops).unwrap();
    assert_eq!(ata_plan.settlement_mint, usdc);
    assert_ne!(ata_plan.settlement_ata, ata_plan.wsol_ata);
    // WSOL must be created idempotently as intermediate ATA when USDC is the flash anchor.
    assert!(
        ata_plan
            .intermediate_atas
            .iter()
            .any(|(mint, _, _)| *mint == wsol),
        "WSOL intermediate ATA must be provisioned when settling in USDC"
    );

    // Pre-converted costs in 6-decimal USDC raw units (5,000 + 20,000 lamports @ $150 SOL ~= 3,750 USDC raw, tip ~ 15,000 raw).
    let ixs = TxBuilder::assemble_flash_arbitrage_with_converted_costs(
        payer,
        tip_acc,
        Lamports(100_000),
        220_000,
        1_000,
        &opp,
        &params,
        11_529_215_046_068,
        5_000,
        20_000,
        TokenAmount(50_000),
        &[],
    )
    .expect("USDC Multi-Mint assembly must succeed");

    let v1 = AdaptiveTxCompiler::compile_v1_only(
        &signer,
        &ixs,
        [9u8; 32],
        V1ConfigParams {
            priority_fee_lamports: Some(22_000),
            compute_unit_limit: Some(220_000),
            loaded_accounts_data_size_limit: Some(67_108_864),
            heap_size: None,
        },
    )
    .expect("V1 compilation for USDC Multi-Mint route must pass");

    assert!(v1.wire_bytes.len() <= 4096);
    let (version, account_count) =
        parse_transaction_wire_header(&v1.wire_bytes).expect("valid V1 wire bytes");
    assert_eq!(version, TransactionVersion::V1);
    assert!(account_count <= 64);
    assert_eq!(v1.wire_bytes[0], 0x81);
    assert_eq!(v1.wire_bytes[1], 1);
}

#[test]
fn gate_g05_and_g34_strict_v1_never_downgrades_or_truncates_beyond_64_accounts() {
    let signer = test_signer();
    let program = Pubkey::from_str(RAYDIUM_CPMM_PROGRAM_ID).unwrap();
    let accounts: Vec<AccountMeta> = (0..66)
        .map(|_| AccountMeta::new_readonly(Pubkey::new_unique(), false))
        .collect();
    let ix = CompiledInstruction {
        program_id: program,
        accounts,
        data: vec![1, 2, 3, 4],
    };

    let err = AdaptiveTxCompiler::compile_v1_only(
        &signer,
        &[ix],
        [3u8; 32],
        V1ConfigParams {
            priority_fee_lamports: Some(5_000),
            compute_unit_limit: Some(200_000),
            loaded_accounts_data_size_limit: Some(67_108_864),
            heap_size: None,
        },
    )
    .expect_err("66 accounts MUST fail closed rather than downgrade to V0 or truncate");
    let msg = err.to_string();
    assert!(
        msg.contains("64") || msg.contains("account"),
        "unexpected error: {msg}"
    );
}

#[test]
fn gate_phase_d_token2022_clmm_swap_v2_and_transfer_hook_extra_metas() {
    let payer = Pubkey::new_unique();
    let pool = Pubkey::new_unique();
    let amm_config = Pubkey::new_unique();
    let vault_0 = Pubkey::new_unique();
    let vault_1 = Pubkey::new_unique();
    let obs = Pubkey::new_unique();
    let mint_0 = Pubkey::new_unique();
    let mint_1 = Pubkey::new_unique();
    let ta0 = Pubkey::new_unique();
    let ta1 = Pubkey::new_unique();
    let ta2 = Pubkey::new_unique();
    let ext = Pubkey::new_unique();

    let accounts = arb_tx::RaydiumClmmAccounts::with_tick_arrays(
        pool,
        payer,
        Pubkey::new_unique(),
        Pubkey::new_unique(),
        vault_0,
        vault_1,
        amm_config,
        obs,
        vec![ta0, ta1, ta2],
        Some(ext),
    );
    let hook_program = Pubkey::new_unique();
    let list_pda = derive_transfer_hook_extra_account_meta_list_pda(&mint_0, &hook_program);
    let extra_acc = Pubkey::new_unique();
    let mut raw_list = vec![0u8; 16 + 35];
    raw_list[..8].copy_from_slice(&[105, 37, 101, 197, 75, 251, 102, 26]);
    raw_list[8..12].copy_from_slice(&35u32.to_le_bytes());
    raw_list[12..16].copy_from_slice(&1u32.to_le_bytes());
    raw_list[16] = 0; // static Pubkey meta
    raw_list[17..49].copy_from_slice(extra_acc.as_ref());
    raw_list[49] = 1; // is_signer
    raw_list[50] = 1; // is_writable

    let hook_metas = parse_transfer_hook_extra_account_metas(&raw_list, &mint_0, &hook_program)
        .expect("static ExtraAccountMetaList must parse");
    assert_eq!(hook_metas.len(), 3);
    assert_eq!(hook_metas[0].pubkey, extra_acc);
    assert!(hook_metas[0].is_signer);
    assert!(hook_metas[0].is_writable);
    assert_eq!(hook_metas[1].pubkey, hook_program);
    assert_eq!(hook_metas[2].pubkey, list_pda);

    let mut malformed_flags = raw_list.clone();
    malformed_flags[50] = 2;
    assert!(
        parse_transfer_hook_extra_account_metas(&malformed_flags, &mint_0, &hook_program).is_err()
    );

    let ix = arb_tx::build_raydium_clmm_swap_v2_instruction(
        &accounts,
        mint_0,
        mint_1,
        Some(ext),
        &hook_metas,
        1_000_000,
        900_000,
        arb_constants::CLMM_MIN_SQRT_PRICE_X64 + 1,
        true,
    )
    .expect("CLMM swap_v2 must build");
    assert_eq!(
        ix.accounts[8].pubkey,
        Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap()
    );
    assert_eq!(
        ix.accounts[9].pubkey,
        Pubkey::from_str(TOKEN_2022_PROGRAM_ID).unwrap()
    );
    assert_eq!(ix.accounts[13].pubkey, ext);
    assert_eq!(ix.accounts.last().unwrap().pubkey, list_pda);

    let hop = SwapHop {
        pool_id: PoolId(pool),
        protocol: DexProtocol::RaydiumClmm,
        mint_in: MintId(mint_0),
        mint_out: MintId(mint_1),
        fee_bps: 25,
        active_bin_id: None,
        token_programs: [TokenProgramKind::Token2022, TokenProgramKind::SplToken],
        trade_intent: PumpSwapTradeIntent::default(),
        execution_accounts: Some(PoolExecutionAccounts::RaydiumClmm {
            amm_config,
            vault_0,
            vault_1,
            observation_state: obs,
            tick_array_pdas: vec![ta0, ta1, ta2],
            bitmap_extension: Some(ext),
            pool_mint_0: mint_0,
        }),
    };
    let hop_ix = arb_tx::build_swap_hop_instruction(&hop, payer, 1_000_000, 900_000)
        .expect("Token-2022 Raydium CLMM hop must route through swap_v2");
    assert_eq!(
        &hop_ix.data[..8],
        &arb_tx::RAYDIUM_CLMM_SWAP_V2_DISCRIMINATOR
    );
}
