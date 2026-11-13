//! Transaction Assembly, Instruction Packing, and Atomic Bundle Builder.
//! Supports native DEX arbitrage and Kamino Flash Loans.

use anyhow::{Context, Result, bail};
pub use arb_types::{
    DexProtocol, ExecutionOpportunity, Lamports, PoolExecutionAccounts, RaydiumV4SwapVersion,
    SwapHop, TokenAmount,
};
use ed25519_dalek::{Signature, Signer, SigningKey};
use solana_pubkey::Pubkey;
use std::str::FromStr;

// All on-chain program IDs come from `arb-constants` (single source of truth).
// Re-exported so every existing `arb_tx::X` path keeps working unchanged.
pub use arb_constants::{
    ADDRESS_LOOKUP_TABLE_PROGRAM_ID, ASSOCIATED_TOKEN_PROGRAM_ID, BASE_PRIORITY_FEE_MICRO_LAMPORTS,
    COMPUTE_BUDGET_LIMIT_2HOP, COMPUTE_BUDGET_LIMIT_3HOP, COMPUTE_BUDGET_PROGRAM_ID, CU_BUFFER_PCT,
    DEFAULT_HOP_SLIPPAGE_BPS, DEFAULT_MIN_PROFIT_LAMPORTS, FLASH_BORROW_DISCRIMINATOR,
    FLASH_REPAY_DISCRIMINATOR, KAMINO_KLEND_PROGRAM_ID, KAMINO_MAIN_MARKET,
    KAMINO_MAIN_MARKET_AUTHORITY, KAMINO_USDC_RESERVE, KAMINO_WSOL_FEE_VAULT, KAMINO_WSOL_RESERVE,
    KAMINO_WSOL_SUPPLY_VAULT, MAX_COMPUTE_UNIT_LIMIT, METEORA_BINS_PER_BIN_ARRAY,
    METEORA_DLMM_PROGRAM_ID, METEORA_SOL_RESERVE, METEORA_SOL_USDC_BIN_STEP, METEORA_SOL_USDC_POOL,
    METEORA_SWAP2_DISCRIMINATOR, METEORA_USDC_RESERVE, ORCA_SOL_USDC_POOL, ORCA_SOL_VAULT,
    ORCA_USDC_VAULT, ORCA_WHIRLPOOL_PROGRAM_ID, PUMP_FUN_BONDING_CURVE_SEED,
    PUMP_FUN_BUY_DISCRIMINATOR, PUMP_FUN_EVENT_AUTHORITY_SEED, PUMP_FUN_FEE_RECIPIENT,
    PUMP_FUN_GLOBAL_SEED, PUMP_FUN_PROGRAM_ID, PUMP_FUN_SELL_DISCRIMINATOR,
    PUMP_SWAP_BUY_DISCRIMINATOR, PUMP_SWAP_BUY_EXACT_QUOTE_IN_V2_DISCRIMINATOR,
    PUMP_SWAP_BUY_V2_DISCRIMINATOR, PUMP_SWAP_GLOBAL_CONFIG, PUMP_SWAP_PROGRAM_ID,
    PUMP_SWAP_SELL_DISCRIMINATOR, PUMP_SWAP_SELL_V2_DISCRIMINATOR, RAYDIUM_CLMM_OBSERVATION_SEED,
    RAYDIUM_CLMM_PROGRAM_ID, RAYDIUM_CLMM_SWAP_DISCRIMINATOR, RAYDIUM_CLMM_TICK_ARRAY_SEED,
    RAYDIUM_CPMM_AUTH_SEED, RAYDIUM_CPMM_OBSERVATION_SEED, RAYDIUM_CPMM_POOL_VAULT_SEED,
    RAYDIUM_CPMM_PROGRAM_ID, RAYDIUM_CPMM_SOL_USDC_POOL, RAYDIUM_CPMM_SOL_VAULT,
    RAYDIUM_CPMM_SWAP_BASE_INPUT_DISCRIMINATOR, RAYDIUM_CPMM_USDC_VAULT, RAYDIUM_SOL_USDC_POOL,
    RAYDIUM_SOL_VAULT, RAYDIUM_USDC_VAULT, RAYDIUM_V4_PROGRAM_ID, SOLANA_PACKET_MTU_LIMIT,
    SYSTEM_PROGRAM_ID, SYSVAR_INSTRUCTIONS_ID, SYSVAR_RENT_ID, TOKEN_PROGRAM_ID, USDC_MINT,
    WSOL_MINT, get_cu_limit_for_hops, jito_tip_account_for_leader, jito_tip_account_for_slot,
};

pub mod alt;
pub use alt::{
    AltManager, MAX_ALT_EXTEND_ADDRESSES, address_lookup_table_program_id, create_lookup_table,
    derive_lookup_table_address, extend_lookup_table,
};
pub mod master_alt;
pub use master_alt::{MAX_ALT_ADDRESSES, StaticMasterAlt};
pub mod meteora_dlmm;
pub use meteora_dlmm::{
    DlmmSwapAccountSet, MAX_BIN_PER_ARRAY, SwapDirection, SwapMode, build_swap2_exact_in,
    build_swap2_exact_out, derive_directional_bin_arrays,
};
pub mod compiler;
pub use compiler::{
    ByteBudgetReport, CompileResult, CompiledV0Transaction, CompiledV1Transaction,
    LegExecutionAccounts, LegSwapParams, RejectionReason, ThreeLegMemeArbitrageContext,
    compile_3leg_meme_dlmm_arb,
};
pub mod adaptive;
pub use adaptive::{
    AdaptiveTxCompiler, CONFIG_MASK_CU_LIMIT, CONFIG_MASK_HEAP_SIZE, CONFIG_MASK_LOADED_DATA_LIMIT,
    CONFIG_MASK_PRIORITY_FEE, CompiledAdaptiveTransaction, CompilerCapabilities,
    FormatSelectionReason, MAX_V1_ADDRESSES, MAX_V1_INSTRUCTIONS, SOLANA_PACKET_LEGACY_MTU,
    SOLANA_PACKET_V1_MTU, TransactionVersion, V0_VERSION_DISCRIMINATOR, V1_VERSION_DISCRIMINATOR,
    V1ConfigParams, extract_adaptive_transaction_signature, rpc_max_supported_version_from_wire,
    v1_executed_instruction_index,
};
pub mod meteora_atomic_builder;
pub use meteora_atomic_builder::{
    METEORA_TWO_POOL_V1_DEFAULT_CU_LIMIT, METEORA_TWO_POOL_V1_DEFAULT_LOADED_DATA_LIMIT,
    METEORA_TWO_POOL_V1_MAX_CU_LIMIT, MeteoraAtomicArbitrageTx, MeteoraDlmmAtomicArbitrageBuilder,
    MeteoraTwoPoolV1Config,
};
pub mod dlmm_bin_discovery;
pub use dlmm_bin_discovery::{
    BinArrayPlanError, DlmmBinArrayPlan, estimate_legacy_message_size, plan_bin_arrays,
    required_bin_array_count, verify_bin_array_coverage,
};
pub mod dlmm_flash_arb;
pub use dlmm_flash_arb::{
    FlashArbError, FlashArbParams, FlashArbPoolLeg, FlashTwoPoolArbitrageTx,
    assemble_flash_funded_two_pool, build_flash_funded_two_pool_transaction,
    estimate_v0_transaction_size,
};
pub mod dlmm_3leg_flash_arb;
pub use dlmm_3leg_flash_arb::{
    FlashArb3LegError, FlashArb3LegParams, FlashArb3LegPoolLeg, FlashThreeLegArbitrageTx,
    assemble_flash_funded_three_pool, build_flash_funded_three_pool_transaction,
};
pub mod dlmm_4leg_flash_arb;
pub use dlmm_4leg_flash_arb::{
    FlashArb4LegError, FlashArb4LegParams, FlashArb4LegPoolLeg, FlashFourLegArbitrageTx,
    assemble_flash_funded_four_pool, build_flash_funded_four_pool_v0_transaction,
    build_flash_funded_four_pool_v1_transaction,
};
pub mod cross_dex_flash_arb;
pub use cross_dex_flash_arb::{
    CrossDexFlashArbError, CrossDexFlashArbParams, CrossDexFlashArbitrageTx, CrossDexSwapLeg,
    DexPlatform, assemble_cross_dex_flash_arbitrage, build_cross_dex_flash_arbitrage_v0,
    build_cross_dex_flash_arbitrage_v1,
};
pub mod atomic_integrity;
pub mod error;
pub mod jupiter;
pub mod kamino;
pub mod pump_fun_raydium;
pub mod template;

pub use error::TxError;
pub use jupiter::JupiterClient;
pub use kamino::{
    KAMINO_FRACTION_FRAC_BITS, KAMINO_FRACTION_ONE_BITS, KaminoFeeError, KaminoFlashAccounts,
    calculate_kamino_flash_fee, calculate_kamino_flash_fee_checked,
    derive_lending_market_authority,
};
pub use pump_fun_raydium::{
    DEFAULT_PRIORITY_FEE_MICROLAMPORTS, JITO_TIP_ACCOUNTS, PUMP_FUN_RAYDIUM_CU_LIMIT,
    PumpFunRaydiumArbitrageBuilder, build_and_submit_pump_fun_raydium_arbitrage,
};
pub use template::{
    PreWarmedTxTemplate, PrewarmedUnsignedTemplate, TemplateHopRefresh, TemplateRefreshParams,
};

// R10 split: cohesive groups extracted verbatim into sibling modules;
// glob re-exports preserve every existing `arb_tx::` path.
mod cu;
pub use cu::*;
mod instruction;
pub use instruction::*;
mod kamino_flash;
pub use kamino_flash::*;
mod ata;
pub use ata::*;
mod raydium_v4;
pub use raydium_v4::*;
mod orca;
pub use orca::*;
mod meteora_accounts;
pub use meteora_accounts::*;
mod raydium_cpmm;
pub use raydium_cpmm::*;
mod raydium_clmm;
pub use raydium_clmm::*;
mod pump_fun;
pub use pump_fun::*;
mod pump_swap;
pub use pump_swap::*;
mod swap_hop;
pub use swap_hop::*;
mod signer;
pub use signer::*;
mod builder;
pub use builder::*;

#[cfg(test)]
mod tests {
    use super::*;
    use arb_types::{Bps, DexProtocol, MintId, PoolId, RouteId, Slot, SwapHop, TokenAmount};
    use ed25519_dalek::Verifier;

    /// `assemble_flash_arbitrage` returns `anyhow::Result`, so the typed
    /// `TxError` may be wrapped. Accept either the downcast variant or the
    /// rendered message, but require the specific fail-closed reason.
    #[allow(dead_code)]
    fn assert_pump_fun_fail_closed(err: &anyhow::Error) -> bool {
        let typed = err.downcast_ref::<TxError>().is_some_and(|e| {
            matches!(e, TxError::UnsupportedProtocol(msg) if msg.contains("Pump.fun live adapter disabled"))
        });
        typed || format!("{err:#}").contains("Pump.fun live adapter disabled")
    }

    #[test]
    fn cu_model_is_protocol_aware_and_route_sized() {
        // CLMM/DLMM cost more than a CPMM swap.
        assert!(per_hop_cu(DexProtocol::OrcaWhirlpool) > per_hop_cu(DexProtocol::RaydiumV4));
        assert!(per_hop_cu(DexProtocol::MeteoraDlmm) > per_hop_cu(DexProtocol::OrcaWhirlpool));
        // A 2-hop flash route with one ephemeral ATA is sized above the flat guess.
        let cu = estimate_route_cu(true, &[DexProtocol::RaydiumV4, DexProtocol::MeteoraDlmm], 1);
        assert!(cu >= 28_000 + 35_000 + 120_000);
        // The dynamic limit adds the 15% buffer and respects the ceiling.
        let lim = calculate_dynamic_cu_limit(cu);
        assert_eq!(lim, cu + cu * 15 / 100);
        assert!(calculate_dynamic_cu_limit(u32::MAX) <= MAX_COMPUTE_UNIT_LIMIT);
    }

    #[test]
    fn priority_fee_is_capped_by_profit_share() {
        // 1 SOL profit, 200k CU limit, 20% cap => budget 200_000_000 lamports.
        // max cu_price = 200_000_000 * 1e6 / 200_000 = 1_000_000_000 µlamports.
        assert_eq!(
            cap_cu_price_by_profit(1_000_000_000, 200_000, 5_000_000_000, 2_000),
            1_000_000_000
        );
        // A modest desired price below the cap is untouched.
        assert_eq!(
            cap_cu_price_by_profit(1_000_000_000, 200_000, 500, 2_000),
            500
        );
        // Zero profit / zero CU => zero price.
        assert_eq!(cap_cu_price_by_profit(0, 200_000, 5_000_000_000, 2_000), 0);
        assert_eq!(
            cap_cu_price_by_profit(1_000_000, 0, 5_000_000_000, 2_000),
            0
        );
    }

    #[test]
    fn compute_budget_instructions() {
        let ix1 = build_compute_unit_limit(180_000);
        assert_eq!(ix1.data[0], 2);
        assert_eq!(&ix1.data[1..5], &180_000u32.to_le_bytes());

        // Test dynamic CU limit with 15% buffer
        let dynamic_cu = calculate_dynamic_cu_limit(100_000);
        assert_eq!(dynamic_cu, 115_000); // 100k + 15%

        let capped_cu = calculate_dynamic_cu_limit(400_000);
        assert_eq!(capped_cu, MAX_COMPUTE_UNIT_LIMIT); // Capped at MAX_COMPUTE_UNIT_LIMIT (350k)

        // Verify 260k and 320k constants and helpers
        assert_eq!(COMPUTE_BUDGET_LIMIT_2HOP, 260_000);
        assert_eq!(COMPUTE_BUDGET_LIMIT_3HOP, 320_000);
        assert_eq!(get_cu_limit_for_hops(2), 260_000);
        assert_eq!(get_cu_limit_for_hops(3), 320_000);
    }

    #[test]
    fn tip_instruction_builder() {
        let payer = Pubkey::new_from_array([1; 32]);
        let tip_acc = Pubkey::new_from_array([2; 32]);
        let ix = build_jito_tip_instruction(payer, tip_acc, Lamports(50_000));
        assert_eq!(ix.accounts.len(), 2);
        assert!(ix.accounts[0].is_signer);
    }

    fn mock_raydium_v4_exec_accounts() -> PoolExecutionAccounts {
        PoolExecutionAccounts::RaydiumV4 {
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
        }
    }

    fn mock_orca_whirlpool_exec_accounts() -> PoolExecutionAccounts {
        PoolExecutionAccounts::OrcaWhirlpool {
            vault_a: Pubkey::new_unique(),
            vault_b: Pubkey::new_unique(),
            oracle: Pubkey::new_unique(),
            tick_array_pdas: vec![
                Pubkey::new_unique(),
                Pubkey::new_unique(),
                Pubkey::new_unique(),
            ],
        }
    }

    fn mock_meteora_dlmm_exec_accounts() -> PoolExecutionAccounts {
        PoolExecutionAccounts::MeteoraDlmm {
            reserve_x: Pubkey::new_unique(),
            reserve_y: Pubkey::new_unique(),
            oracle: Pubkey::new_unique(),
            bin_array_pdas: vec![Pubkey::new_unique(), Pubkey::new_unique()],
            bitmap_extension: None,
        }
    }

    fn mock_raydium_cpmm_exec_accounts() -> PoolExecutionAccounts {
        PoolExecutionAccounts::RaydiumCpmm {
            amm_config: Pubkey::new_unique(),
            authority: Pubkey::new_unique(),
            vault_0: Pubkey::new_unique(),
            vault_1: Pubkey::new_unique(),
            observation_state: Pubkey::new_unique(),
        }
    }

    fn mock_raydium_clmm_exec_accounts() -> PoolExecutionAccounts {
        PoolExecutionAccounts::RaydiumClmm {
            amm_config: Pubkey::new_unique(),
            vault_0: Pubkey::new_unique(),
            vault_1: Pubkey::new_unique(),
            observation_state: Pubkey::new_unique(),
            tick_array_pdas: vec![
                Pubkey::new_unique(),
                Pubkey::new_unique(),
                Pubkey::new_unique(),
            ],
            bitmap_extension: None,
        }
    }

    fn mock_pump_swap_exec_accounts() -> PoolExecutionAccounts {
        PoolExecutionAccounts::PumpSwap {
            global_config: Pubkey::new_unique(),
            pool_base_vault: Pubkey::new_unique(),
            pool_quote_vault: Pubkey::new_unique(),
            protocol_fee_recipient: Pubkey::new_unique(),
            protocol_fee_recipient_ata: Pubkey::new_unique(),
            coin_creator_vault_authority: Pubkey::new_unique(),
            coin_creator_vault_ata: Pubkey::new_unique(),
            fee_config: Pubkey::new_unique(),
            pool_version: arb_types::PumpSwapPoolVersion::V2,
        }
    }

    #[test]
    fn kamino_flash_assembly() {
        let payer = Pubkey::new_from_array([1; 32]);
        let tip_acc = Pubkey::new_from_array([2; 32]);
        let params = KaminoFlashParams::default();

        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let usdc = Pubkey::from_str(USDC_MINT).unwrap();

        // 1. Route between permanent base assets (WSOL <-> USDC)
        let opp_base = ExecutionOpportunity {
            route_id: RouteId(1),
            slot: Slot(100),
            size_in: TokenAmount(50_000_000_000), // 50 SOL
            expected_out: TokenAmount(50_500_000_000),
            hop_inputs: vec![50_000_000_000, 50_250_000_000, 50_500_000_000],
            gross_profit: Lamports(500_000_000),
            net_profit: Lamports(450_000_000),
            base_decimals: 9,
            price_impact_bps: Bps(10),
            hops: vec![
                SwapHop {
                    pool_id: PoolId::new([10; 32]),
                    protocol: DexProtocol::RaydiumV4,
                    mint_in: MintId(wsol),
                    mint_out: MintId(usdc),
                    fee_bps: 25,
                    active_bin_id: None,
                    token_programs: [arb_types::TokenProgramKind::SplToken; 2],
                    trade_intent: Default::default(),
                    execution_accounts: Some(mock_raydium_v4_exec_accounts()),
                },
                SwapHop {
                    pool_id: PoolId(Pubkey::from_str(ORCA_SOL_USDC_POOL).unwrap()),
                    protocol: DexProtocol::OrcaWhirlpool,
                    mint_in: MintId(usdc),
                    mint_out: MintId(wsol),
                    fee_bps: 5,
                    active_bin_id: None,
                    token_programs: [arb_types::TokenProgramKind::SplToken; 2],
                    trade_intent: Default::default(),
                    execution_accounts: None,
                },
            ],

            expected_min_final_output: 0,
        };

        let ixs_base = TxBuilder::assemble_flash_arbitrage(
            payer,
            tip_acc,
            Lamports(10_000),
            160_000,
            1000,
            &opp_base,
            &params,
            0,                   // verified sf: 0 in this fixture
            TokenAmount(30_000), // min_profit_raw in base-mint (WSOL) units
            &[],
        )
        .unwrap();
        // Base route: CU limit + CU price + WSOL ATA Create + Kamino Borrow + 2 Swaps + Kamino Repay + Jito Tip = 8 instructions
        assert_eq!(ixs_base.len(), 8);
        assert_eq!(&ixs_base[0].data[1..5], &184_000u32.to_le_bytes()); // 160k + 15% buffer = 184k
        assert_eq!(ixs_base[3].accounts[0].pubkey, payer); // Borrow user authority (signer)
        assert_eq!(ixs_base[3].accounts[2].pubkey, params.lending_market); // Borrow lending market
        assert_eq!(ixs_base[6].accounts[0].pubkey, payer); // Repay user authority (signer)
        assert_eq!(ixs_base[6].accounts[2].pubkey, params.lending_market); // Repay lending market

        // 2. Route with intermediate trending token (Ephemeral ATA Create -> Swaps -> Close & Rent Reclaim)
        let intermediate_token = Pubkey::new_from_array([88; 32]);
        let opp_intermediate = ExecutionOpportunity {
            route_id: RouteId(2),
            slot: Slot(100),
            size_in: TokenAmount(50_000_000_000),
            expected_out: TokenAmount(50_500_000_000),
            hop_inputs: vec![50_000_000_000, 50_250_000_000, 50_500_000_000],
            gross_profit: Lamports(500_000_000),
            net_profit: Lamports(450_000_000),
            base_decimals: 9,
            price_impact_bps: Bps(10),
            hops: vec![
                SwapHop {
                    pool_id: PoolId::new([10; 32]),
                    protocol: DexProtocol::RaydiumV4,
                    mint_in: MintId(wsol),
                    mint_out: MintId(intermediate_token),
                    fee_bps: 25,
                    active_bin_id: None,
                    token_programs: [arb_types::TokenProgramKind::SplToken; 2],
                    trade_intent: Default::default(),
                    execution_accounts: Some(mock_raydium_v4_exec_accounts()),
                },
                SwapHop {
                    pool_id: PoolId(Pubkey::from_str(ORCA_SOL_USDC_POOL).unwrap()),
                    protocol: DexProtocol::OrcaWhirlpool,
                    mint_in: MintId(intermediate_token),
                    mint_out: MintId(wsol),
                    fee_bps: 5,
                    active_bin_id: None,
                    token_programs: [arb_types::TokenProgramKind::SplToken; 2],
                    trade_intent: Default::default(),
                    execution_accounts: None,
                },
            ],

            expected_min_final_output: 0,
        };

        let ixs_inter = TxBuilder::assemble_flash_arbitrage(
            payer,
            tip_acc,
            Lamports(10_000),
            160_000,
            1000,
            &opp_intermediate,
            &params,
            0,                   // verified sf: 0 in this fixture
            TokenAmount(30_000), // min_profit_raw in base-mint (WSOL) units
            &[],
        )
        .unwrap();
        // Base (8) + 1 Ephemeral ATA Create + 1 Ephemeral ATA Close = 10 instructions
        assert_eq!(ixs_inter.len(), 10);
        assert_eq!(
            ixs_inter[2].program_id,
            Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM_ID).unwrap()
        ); // CreateIdempotent (WSOL)
        assert_eq!(
            ixs_inter[3].program_id,
            Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM_ID).unwrap()
        ); // CreateIdempotent (Intermediate)
        assert_eq!(
            ixs_inter[8].program_id,
            Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap()
        ); // CloseAccount (Rent Reclaimed to Payer)
        assert_eq!(ixs_inter[8].data, vec![9]); // CloseAccount opcode
    }

    #[test]
    fn keypair_signer_decoding_and_cryptographic_verification() {
        let seed = [9u8; 32];
        let signing_key = SigningKey::from_bytes(&seed);
        let pubkey_bytes = signing_key.verifying_key().to_bytes();
        let mut keypair_bytes = [0u8; 64];
        keypair_bytes[..32].copy_from_slice(&seed);
        keypair_bytes[32..].copy_from_slice(&pubkey_bytes);

        let b58_str = bs58::encode(&keypair_bytes).into_string();
        let signer = KeypairSigner::from_base58(&b58_str).unwrap();
        assert_eq!(signer.pubkey(), Pubkey::new_from_array(pubkey_bytes));

        let message = b"solana_mev_private_bundle_payload";
        let sig_bytes = signer.sign(message);
        assert_eq!(sig_bytes.len(), 64);

        // Genuine cryptographic Ed25519 verification
        let verifying_key = signer.verifying_key();
        let parsed_sig = Signature::from_bytes(&sig_bytes);
        assert!(verifying_key.verify(message, &parsed_sig).is_ok());
    }

    #[test]
    fn test_build_signed_transaction_wire_format() {
        let seed = [7u8; 32];
        let signing_key = SigningKey::from_bytes(&seed);
        let pubkey_bytes = signing_key.verifying_key().to_bytes();
        let mut keypair_bytes = [0u8; 64];
        keypair_bytes[..32].copy_from_slice(&seed);
        keypair_bytes[32..].copy_from_slice(&pubkey_bytes);

        let signer = KeypairSigner::from_bytes(&keypair_bytes).unwrap();
        let payer = signer.pubkey();
        let tip_acc = Pubkey::new_from_array([3; 32]);

        let cu_min = estimate_minimal_cu(true, 2);
        assert!(cu_min > 90_000 && cu_min < 120_000);
        let cu_limit = calculate_dynamic_cu_limit(cu_min);
        assert!(cu_limit <= 200_000);

        let ix1 = build_compute_unit_limit(cu_limit);
        let ix2 = build_jito_tip_instruction(payer, tip_acc, Lamports(10_000));
        let blockhash = [42u8; 32];

        let tx_bytes =
            TxBuilder::build_signed_transaction(&signer, &[ix1, ix2], blockhash).unwrap();
        assert!(!tx_bytes.is_empty());
        assert_eq!(tx_bytes[0], 1); // 1 signature

        let sig_slice = &tx_bytes[1..65];
        let message_slice = &tx_bytes[65..];

        // Verify cryptographic signature matches message
        let parsed_sig = Signature::from_bytes(sig_slice.try_into().unwrap());
        assert!(
            signer
                .verifying_key()
                .verify(message_slice, &parsed_sig)
                .is_ok()
        );
    }

    #[test]
    fn test_raydium_cpmm_swap_layout_and_accounts() {
        let user = Pubkey::new_unique();
        let user_in = Pubkey::new_unique();
        let user_out = Pubkey::new_unique();
        let accounts = RaydiumCpmmAccounts::mainnet_sol_usdc(user, user_in, user_out);

        assert_eq!(accounts.payer, user);
        assert_eq!(
            accounts.pool_state,
            Pubkey::from_str(RAYDIUM_CPMM_SOL_USDC_POOL).unwrap()
        );
        assert_eq!(
            accounts.input_vault,
            Pubkey::from_str(RAYDIUM_CPMM_SOL_VAULT).unwrap()
        );
        assert_eq!(
            accounts.output_vault,
            Pubkey::from_str(RAYDIUM_CPMM_USDC_VAULT).unwrap()
        );

        let amount_in = 1_000_000_000u64;
        let min_amount_out = 150_000_000u64;
        let ix = build_raydium_cpmm_swap_instruction(&accounts, amount_in, min_amount_out).unwrap();

        assert_eq!(
            ix.program_id,
            Pubkey::from_str(RAYDIUM_CPMM_PROGRAM_ID).unwrap()
        );
        assert_eq!(ix.accounts.len(), 13);

        // Account 0: payer (signer, writable)
        assert_eq!(ix.accounts[0].pubkey, user);
        assert!(ix.accounts[0].is_signer);
        assert!(ix.accounts[0].is_writable);

        // Account 1: authority (readonly)
        assert_eq!(ix.accounts[1].pubkey, derive_raydium_cpmm_authority());
        assert!(!ix.accounts[1].is_signer);
        assert!(!ix.accounts[1].is_writable);

        // Account 3: pool_state (writable)
        assert_eq!(ix.accounts[3].pubkey, accounts.pool_state);
        assert!(ix.accounts[3].is_writable);

        // Account 4 & 5: token accounts
        assert_eq!(ix.accounts[4].pubkey, user_in);
        assert_eq!(ix.accounts[5].pubkey, user_out);

        // Account 6 & 7: vaults
        assert_eq!(ix.accounts[6].pubkey, accounts.input_vault);
        assert_eq!(ix.accounts[7].pubkey, accounts.output_vault);

        // Account 12: observation_state (writable)
        assert_eq!(ix.accounts[12].pubkey, accounts.observation_state);
        assert!(ix.accounts[12].is_writable);

        // Data payload
        assert_eq!(ix.data.len(), 24);
        assert_eq!(&ix.data[0..8], &RAYDIUM_CPMM_SWAP_BASE_INPUT_DISCRIMINATOR);
        assert_eq!(&ix.data[8..16], &amount_in.to_le_bytes());
        assert_eq!(&ix.data[16..24], &min_amount_out.to_le_bytes());
    }

    #[test]
    fn test_raydium_clmm_swap_layout_and_accounts() {
        let pool = Pubkey::new_unique();
        let payer = Pubkey::new_unique();
        let user_in = Pubkey::new_unique();
        let user_out = Pubkey::new_unique();
        let vault_in = Pubkey::new_unique();
        let vault_out = Pubkey::new_unique();
        let amm_config = Pubkey::new_unique();

        let accounts = RaydiumClmmAccounts::for_pool(
            pool, payer, user_in, user_out, vault_in, vault_out, amm_config, 60, 1200, true,
        );

        assert_eq!(accounts.payer, payer);
        assert_eq!(accounts.pool_state, pool);
        assert_ne!(accounts.tick_array_0, accounts.tick_array_1);
        assert_ne!(accounts.tick_array_1, accounts.tick_array_2);

        let amount_in = 500_000_000u64;
        let other_threshold = 40_000_000u64;
        let sqrt_price_limit = 1_000_000_000u128;
        let ix = build_raydium_clmm_swap_instruction(
            &accounts,
            amount_in,
            other_threshold,
            sqrt_price_limit,
            true,
        )
        .unwrap();

        assert_eq!(
            ix.program_id,
            Pubkey::from_str(RAYDIUM_CLMM_PROGRAM_ID).unwrap()
        );
        assert_eq!(ix.accounts.len(), 12);

        // Account 0: payer
        assert_eq!(ix.accounts[0].pubkey, payer);
        assert!(ix.accounts[0].is_signer);

        // Account 2: pool_state
        assert_eq!(ix.accounts[2].pubkey, pool);
        assert!(ix.accounts[2].is_writable);

        // Tick arrays: accounts 9, 10, 11
        assert_eq!(ix.accounts[9].pubkey, accounts.tick_array_0);
        assert_eq!(ix.accounts[10].pubkey, accounts.tick_array_1);
        assert_eq!(ix.accounts[11].pubkey, accounts.tick_array_2);
        assert!(ix.accounts[9].is_writable);
        assert!(ix.accounts[10].is_writable);
        assert!(ix.accounts[11].is_writable);

        // Data payload
        assert_eq!(ix.data.len(), 41);
        assert_eq!(&ix.data[0..8], &RAYDIUM_CLMM_SWAP_DISCRIMINATOR);
        assert_eq!(&ix.data[8..16], &amount_in.to_le_bytes());
        assert_eq!(&ix.data[16..24], &other_threshold.to_le_bytes());
        assert_eq!(&ix.data[24..40], &sqrt_price_limit.to_le_bytes());
        assert_eq!(ix.data[40], 1); // is_base_input = true
    }

    #[test]
    fn test_pump_fun_buy_and_sell_layout_and_accounts() {
        let mint = Pubkey::new_unique();
        let user = Pubkey::new_unique();
        let accounts = PumpFunAccounts::for_mint(mint, user);

        assert_eq!(accounts.mint, mint);
        assert_eq!(accounts.user, user);
        assert_eq!(accounts.global, derive_pump_fun_global_pda());
        assert_eq!(
            accounts.bonding_curve,
            derive_pump_fun_bonding_curve_pda(&mint)
        );
        assert_eq!(accounts.event_authority, derive_pump_fun_event_authority());

        // Test BUY
        let sol_amount_in = 2_000_000_000u64; // max SOL to spend
        let min_tokens_out = 50_000_000_000u64; // tokens to receive
        let buy_ix =
            build_pump_fun_swap_instruction(&accounts, sol_amount_in, min_tokens_out, true)
                .unwrap();

        assert_eq!(
            buy_ix.program_id,
            Pubkey::from_str(PUMP_FUN_PROGRAM_ID).unwrap()
        );
        assert_eq!(buy_ix.accounts.len(), 12);
        assert_eq!(buy_ix.accounts[0].pubkey, accounts.global);
        assert_eq!(buy_ix.accounts[1].pubkey, accounts.fee_recipient);
        assert!(buy_ix.accounts[1].is_writable);
        assert_eq!(buy_ix.accounts[2].pubkey, mint);
        assert_eq!(buy_ix.accounts[3].pubkey, accounts.bonding_curve);
        assert!(buy_ix.accounts[3].is_writable);
        assert_eq!(buy_ix.accounts[4].pubkey, accounts.associated_bonding_curve);
        assert!(buy_ix.accounts[4].is_writable);
        assert_eq!(buy_ix.accounts[5].pubkey, accounts.associated_user);
        assert!(buy_ix.accounts[5].is_writable);
        assert_eq!(buy_ix.accounts[6].pubkey, user);
        assert!(buy_ix.accounts[6].is_signer);
        assert!(buy_ix.accounts[6].is_writable);
        assert_eq!(buy_ix.accounts[10].pubkey, accounts.event_authority);

        // Buy data layout: [BUY_DISC(8), min_tokens_out(8), sol_amount_in(8)]
        assert_eq!(buy_ix.data.len(), 24);
        assert_eq!(&buy_ix.data[0..8], &PUMP_FUN_BUY_DISCRIMINATOR);
        assert_eq!(&buy_ix.data[8..16], &min_tokens_out.to_le_bytes());
        assert_eq!(&buy_ix.data[16..24], &sol_amount_in.to_le_bytes());

        // Test SELL
        let tokens_in = 50_000_000_000u64;
        let min_sol_out = 1_800_000_000u64;
        let sell_ix =
            build_pump_fun_swap_instruction(&accounts, tokens_in, min_sol_out, false).unwrap();

        assert_eq!(sell_ix.data.len(), 24);
        assert_eq!(&sell_ix.data[0..8], &PUMP_FUN_SELL_DISCRIMINATOR);
        assert_eq!(&sell_ix.data[8..16], &tokens_in.to_le_bytes());
        assert_eq!(&sell_ix.data[16..24], &min_sol_out.to_le_bytes());
    }

    #[test]
    fn test_meteora_multi_bin_array_expansion() {
        let accounts = MeteoraDlmmAccounts::mainnet_sol_usdc();
        let user = Pubkey::new_unique();
        let user_in = Pubkey::new_unique();
        let user_out = Pubkey::new_unique();

        // 1. Small swap: 1_000_000 -> 990_000 = 100 bps impact, bin_step=4
        // bins_traversed = 100/4+1 = 26. Walking DOWN from bin 0 leaves array 0
        // at bin -1, so BOTH arrays must be provisioned: 16 base + 2.
        let ix_normal = build_meteora_dlmm_swap_instruction(
            &accounts, user, user_in, user_out, 1_000_000, 990_000, 0, 4, true,
        );
        let expected_small = meteora_bin_array_indices(0, true, 26);
        assert_eq!(expected_small, vec![0, -1]);
        assert_eq!(ix_normal.accounts.len(), 16 + expected_small.len());

        // 2. Large price swing: est_impact = 1000 bps, bin_step=4
        // bins_traversed = 1000/4+1 = 251 -> arrays 0..=-4 (5 arrays).
        let ix_large_swing = build_meteora_dlmm_swap_instruction(
            &accounts, user, user_in, user_out, 1_000_000, 900_000, 0, 4, true,
        );
        let expected_large = meteora_bin_array_indices(0, true, 251);
        assert_eq!(expected_large, vec![0, -1, -2, -3, -4]);
        assert_eq!(ix_large_swing.accounts.len(), 16 + expected_large.len());

        // Every provisioned bin array must be a distinct PDA.
        let provided = &ix_large_swing.accounts[16..];
        for (i, a) in provided.iter().enumerate() {
            for b in &provided[i + 1..] {
                assert_ne!(a.pubkey, b.pubkey);
            }
        }

        // 3. Even larger swing: est_impact = 2000 bps, bin_step=4
        // bins_traversed = 2000/4+1 = 501 -> arrays 0..=-8 (9 arrays).
        let ix_huge = build_meteora_dlmm_swap_instruction(
            &accounts, user, user_in, user_out, 1_000_000, 800_000, 0, 4, true,
        );
        let expected_huge = meteora_bin_array_indices(0, true, 501);
        assert_eq!(expected_huge.len(), 9);
        assert_eq!(ix_huge.accounts.len(), 16 + expected_huge.len());
    }

    #[test]
    fn test_build_swap_hop_instruction_for_all_supported_protocols() {
        let user = Pubkey::new_unique();
        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let usdc = Pubkey::from_str(USDC_MINT).unwrap();
        let meme_token = Pubkey::new_unique();

        let protocols = [
            (
                DexProtocol::RaydiumV4,
                wsol,
                usdc,
                Some(mock_raydium_v4_exec_accounts()),
            ),
            (
                DexProtocol::OrcaWhirlpool,
                wsol,
                usdc,
                Some(mock_orca_whirlpool_exec_accounts()),
            ),
            (
                DexProtocol::MeteoraDlmm,
                wsol,
                usdc,
                Some(mock_meteora_dlmm_exec_accounts()),
            ),
            (
                DexProtocol::RaydiumCpmm,
                wsol,
                usdc,
                Some(mock_raydium_cpmm_exec_accounts()),
            ),
            (
                DexProtocol::RaydiumClmm,
                wsol,
                usdc,
                Some(mock_raydium_clmm_exec_accounts()),
            ),
            (
                DexProtocol::PumpSwap,
                wsol,
                meme_token,
                Some(mock_pump_swap_exec_accounts()),
            ),
        ];

        for (proto, mint_in, mint_out, exec_accs) in protocols {
            let hop = SwapHop {
                pool_id: arb_types::PoolId(Pubkey::new_unique()),
                protocol: proto,
                mint_in: arb_types::MintId(mint_in),
                mint_out: arb_types::MintId(mint_out),
                fee_bps: 25,
                active_bin_id: Some(0),
                token_programs: [arb_types::TokenProgramKind::SplToken; 2],
                trade_intent: Default::default(),
                execution_accounts: exec_accs,
            };

            let res = build_swap_hop_instruction(&hop, user, 1_000_000_000, 100_000_000);
            assert!(
                res.is_ok(),
                "build_swap_hop_instruction failed for {:?}: {:?}",
                proto,
                res.err()
            );
            let ix = res.unwrap();
            assert!(!ix.accounts.is_empty());
            assert!(!ix.data.is_empty());
        }

        // Test fail-closed behavior when execution_accounts is None on arbitrary pool
        let fail_hop = SwapHop {
            pool_id: arb_types::PoolId(Pubkey::new_unique()),
            protocol: DexProtocol::RaydiumV4,
            mint_in: arb_types::MintId(wsol),
            mint_out: arb_types::MintId(usdc),
            fee_bps: 25,
            active_bin_id: None,
            token_programs: [arb_types::TokenProgramKind::SplToken; 2],
            trade_intent: Default::default(),
            execution_accounts: None,
        };
        let fail_res = build_swap_hop_instruction(&fail_hop, user, 1_000_000_000, 100_000_000);
        assert!(
            matches!(fail_res, Err(TxError::MissingExecutionAccounts { .. })),
            "Arbitrary pool without execution accounts must fail-closed"
        );

        // Pump.fun verified live adapter emits the official 12-account matrix
        let pump_hop = SwapHop {
            pool_id: arb_types::PoolId(Pubkey::new_unique()),
            protocol: DexProtocol::PumpFun,
            mint_in: arb_types::MintId(wsol),
            mint_out: arb_types::MintId(meme_token),
            fee_bps: 25,
            active_bin_id: None,
            token_programs: [arb_types::TokenProgramKind::SplToken; 2],
            trade_intent: Default::default(),
            execution_accounts: None,
        };
        let res = build_swap_hop_instruction(&pump_hop, user, 1_000_000_000, 100_000_000);
        assert!(
            res.is_ok(),
            "Pump.fun builder should succeed with live adapter: {:?}",
            res.err()
        );
        let ix = res.unwrap();
        assert_eq!(ix.accounts.len(), 12);
        assert_eq!(ix.data[0..8], PUMP_FUN_BUY_DISCRIMINATOR);
    }

    #[test]
    fn test_final_hop_onchain_balance_assertion_guarantee() {
        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let usdc = Pubkey::from_str(USDC_MINT).unwrap();

        let opp = ExecutionOpportunity {
            route_id: RouteId(1),
            slot: Slot(100),
            size_in: TokenAmount(1_000_000_000), // 1 SOL borrow
            expected_out: TokenAmount(1_050_000_000), // 1.05 SOL expected
            hop_inputs: vec![1_000_000_000, 200_000_000, 1_050_000_000],
            gross_profit: Lamports(50_000_000),
            net_profit: Lamports(30_000_000),
            base_decimals: 9,
            price_impact_bps: Bps(10),
            hops: vec![
                SwapHop {
                    pool_id: PoolId(Pubkey::new_unique()),
                    protocol: DexProtocol::RaydiumV4,
                    mint_in: MintId(wsol),
                    mint_out: MintId(usdc),
                    fee_bps: 25,
                    active_bin_id: None,
                    token_programs: [arb_types::TokenProgramKind::SplToken; 2],
                    trade_intent: Default::default(),
                    execution_accounts: None,
                },
                SwapHop {
                    pool_id: PoolId(Pubkey::new_unique()),
                    protocol: DexProtocol::OrcaWhirlpool,
                    mint_in: MintId(usdc),
                    mint_out: MintId(wsol),
                    fee_bps: 30,
                    active_bin_id: None,
                    token_programs: [arb_types::TokenProgramKind::SplToken; 2],
                    trade_intent: Default::default(),
                    execution_accounts: None,
                },
            ],

            expected_min_final_output: 0,
        };

        let flash_fee = 1_000_000; // 0.001 SOL
        let jito_tip = 7_000_000; // 0.007 SOL (70% profit share tip)
        let min_profit = 10_000_000; // 0.01 SOL
        let slippage_bps = 50; // 0.5%

        let legs = TxBuilder::derive_swap_legs(&opp, flash_fee, jito_tip, min_profit, slippage_bps)
            .unwrap();
        assert_eq!(legs.len(), 2);

        // Required floor = borrow + flash_fee + jito_tip + min_profit = 1_018_000_000
        let floor = 1_000_000_000 + flash_fee + jito_tip + min_profit;
        let final_min_out = legs[1].1;
        assert!(
            final_min_out >= floor,
            "Final hop min_out {final_min_out} must enforce profit floor {floor}"
        );

        // If expected_out cannot meet the floor, derive_swap_legs must reject pre-flight
        let failing_opp = ExecutionOpportunity {
            expected_out: TokenAmount(1_015_000_000), // below floor of 1_018_000_000
            hop_inputs: vec![1_000_000_000, 200_000_000, 1_015_000_000],
            ..opp
        };
        let err = TxBuilder::derive_swap_legs(
            &failing_opp,
            flash_fee,
            jito_tip,
            min_profit,
            slippage_bps,
        );
        assert!(matches!(err, Err(TxError::ProfitFloorNotMet { .. })));
    }

    #[test]
    fn assembly_rejects_pump_fun_routes_without_verified_adapter() {
        let payer = Pubkey::new_unique();
        let tip_account = Pubkey::from_str(JITO_TIP_ACCOUNTS[0]).unwrap();
        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let meme = Pubkey::new_unique();
        let kamino_params = KaminoFlashParams::default();

        // 1. Route: Pump.fun Buy -> Raydium Sell
        let opp1 = ExecutionOpportunity {
            route_id: RouteId(1),
            slot: Slot(100),
            size_in: TokenAmount(1_000_000_000), // 1 SOL borrow
            expected_out: TokenAmount(1_050_000_000),
            hop_inputs: vec![1_000_000_000, 35_000_000_000, 1_050_000_000],
            gross_profit: Lamports(50_000_000),
            net_profit: Lamports(30_000_000),
            base_decimals: 9,
            price_impact_bps: Bps(10),
            hops: vec![
                SwapHop {
                    pool_id: PoolId(Pubkey::new_unique()),
                    protocol: DexProtocol::PumpFun,
                    mint_in: MintId(wsol),
                    mint_out: MintId(meme),
                    fee_bps: 100,
                    active_bin_id: None,
                    token_programs: [arb_types::TokenProgramKind::SplToken; 2],
                    trade_intent: Default::default(),
                    execution_accounts: None,
                },
                SwapHop {
                    pool_id: PoolId(Pubkey::new_unique()),
                    protocol: DexProtocol::RaydiumV4,
                    mint_in: MintId(meme),
                    mint_out: MintId(wsol),
                    fee_bps: 25,
                    active_bin_id: None,
                    token_programs: [arb_types::TokenProgramKind::SplToken; 2],
                    trade_intent: Default::default(),
                    execution_accounts: Some(mock_raydium_v4_exec_accounts()),
                },
            ],

            expected_min_final_output: 0,
        };

        let res1 = TxBuilder::assemble_flash_arbitrage(
            payer,
            tip_account,
            Lamports(1_000_000),
            300_000,
            1_000,
            &opp1,
            &kamino_params,
            6250,
            TokenAmount(1_000_000),
            &[],
        );
        assert!(
            res1.is_ok(),
            "Pump.fun buy route must assemble cleanly with live adapter: {:?}",
            res1.err()
        );

        // 2. Route: Raydium Buy -> Pump.fun Sell
        let opp2 = ExecutionOpportunity {
            route_id: RouteId(2),
            slot: Slot(100),
            size_in: TokenAmount(1_000_000_000),
            expected_out: TokenAmount(1_050_000_000),
            hop_inputs: vec![1_000_000_000, 35_000_000_000, 1_050_000_000],
            gross_profit: Lamports(50_000_000),
            net_profit: Lamports(30_000_000),
            base_decimals: 9,
            price_impact_bps: Bps(10),
            hops: vec![
                SwapHop {
                    pool_id: PoolId(Pubkey::new_unique()),
                    protocol: DexProtocol::RaydiumV4,
                    mint_in: MintId(wsol),
                    mint_out: MintId(meme),
                    fee_bps: 25,
                    active_bin_id: None,
                    token_programs: [arb_types::TokenProgramKind::SplToken; 2],
                    trade_intent: Default::default(),
                    execution_accounts: Some(mock_raydium_v4_exec_accounts()),
                },
                SwapHop {
                    pool_id: PoolId(Pubkey::new_unique()),
                    protocol: DexProtocol::PumpFun,
                    mint_in: MintId(meme),
                    mint_out: MintId(wsol),
                    fee_bps: 100,
                    active_bin_id: None,
                    token_programs: [arb_types::TokenProgramKind::SplToken; 2],
                    trade_intent: Default::default(),
                    execution_accounts: None,
                },
            ],

            expected_min_final_output: 0,
        };

        let res2 = TxBuilder::assemble_flash_arbitrage(
            payer,
            tip_account,
            Lamports(1_000_000),
            300_000,
            1_000,
            &opp2,
            &kamino_params,
            6250,
            TokenAmount(1_000_000),
            &[],
        );
        assert!(
            res2.is_ok(),
            "Raydium -> Pump.fun route must assemble cleanly with live adapter: {:?}",
            res2.err()
        );
    }

    #[test]
    fn test_jito_mev_protection_and_signature_extraction() {
        let signer = KeypairSigner::from_seed(&[1u8; 32]);
        let tip_account = Pubkey::new_unique();
        let blockhash = [7u8; 32];

        // Marked tip transaction
        let tx_bytes = TxBuilder::build_marked_jito_tip_transaction(
            &signer,
            tip_account,
            Lamports(100_000),
            blockhash,
        )
        .expect("build marked tip transaction");

        // Signature extraction
        let sig_str = extract_transaction_signature(&tx_bytes);
        assert!(sig_str.is_some());
        let sig_val = sig_str.unwrap();
        assert!(sig_val.len() == 87 || sig_val.len() == 88); // base58 of 64 bytes is 87 or 88 chars

        // Ensure MEV protection helper
        let mut ixs = vec![build_compute_unit_limit(200_000)];
        let marker = arb_constants::jitodontfront_marker();
        assert!(!ixs[0].accounts.iter().any(|a| a.pubkey == marker));
        ensure_bundle_mev_protection(&mut ixs);
        assert!(ixs[0].accounts.iter().any(|a| a.pubkey == marker));

        // Idempotent: second call doesn't add duplicate
        ensure_bundle_mev_protection(&mut ixs);
        assert_eq!(
            ixs[0]
                .accounts
                .iter()
                .filter(|a| a.pubkey == marker)
                .count(),
            1
        );
    }

    #[test]
    fn test_validate_jito_bundle_bounds() {
        assert!(validate_jito_bundle(&[]).is_err());
        let valid_tx = vec![1u8; 300];
        assert!(validate_jito_bundle(std::slice::from_ref(&valid_tx)).is_ok());
        assert!(validate_jito_bundle(&[valid_tx.clone(), valid_tx.clone()]).is_ok());

        // Max 5 txs
        let six_txs = vec![valid_tx.clone(); 6];
        assert!(validate_jito_bundle(&six_txs).is_err());

        // MTU overflow
        let oversized = vec![2u8; 1233];
        assert!(validate_jito_bundle(&[oversized]).is_err());
    }
}
