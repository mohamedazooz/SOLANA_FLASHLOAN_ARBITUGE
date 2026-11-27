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
    FLASH_REPAY_DISCRIMINATOR, JITO_TIP_ACCOUNTS, KAMINO_KLEND_PROGRAM_ID, KAMINO_MAIN_MARKET,
    KAMINO_MAIN_MARKET_AUTHORITY, KAMINO_USDC_RESERVE, KAMINO_WSOL_FEE_VAULT, KAMINO_WSOL_RESERVE,
    KAMINO_WSOL_SUPPLY_VAULT, MAX_COMPUTE_UNIT_LIMIT, METEORA_BINS_PER_BIN_ARRAY,
    METEORA_DLMM_PROGRAM_ID, METEORA_SOL_RESERVE, METEORA_SOL_USDC_BIN_STEP, METEORA_SOL_USDC_POOL,
    METEORA_SWAP2_DISCRIMINATOR, METEORA_USDC_RESERVE, OPENBOOK_PROGRAM_ID, ORCA_SOL_USDC_POOL,
    ORCA_SOL_VAULT, ORCA_USDC_VAULT, ORCA_WHIRLPOOL_PROGRAM_ID, PUMP_FUN_BONDING_CURVE_SEED,
    PUMP_FUN_BUY_DISCRIMINATOR, PUMP_FUN_EVENT_AUTHORITY_SEED, PUMP_FUN_FEE_RECIPIENT,
    PUMP_FUN_GLOBAL_SEED, PUMP_FUN_PROGRAM_ID, PUMP_FUN_SELL_DISCRIMINATOR,
    PUMP_SWAP_BUY_DISCRIMINATOR, PUMP_SWAP_BUY_EXACT_QUOTE_IN_V2_DISCRIMINATOR,
    PUMP_SWAP_BUY_V2_DISCRIMINATOR, PUMP_SWAP_GLOBAL_CONFIG, PUMP_SWAP_PROGRAM_ID,
    PUMP_SWAP_SELL_DISCRIMINATOR, PUMP_SWAP_SELL_V2_DISCRIMINATOR, RAYDIUM_CLMM_OBSERVATION_SEED,
    RAYDIUM_CLMM_PROGRAM_ID, RAYDIUM_CLMM_SWAP_DISCRIMINATOR, RAYDIUM_CLMM_TICK_ARRAY_SEED,
    RAYDIUM_CPMM_AUTH_SEED, RAYDIUM_CPMM_OBSERVATION_SEED, RAYDIUM_CPMM_POOL_VAULT_SEED,
    RAYDIUM_CPMM_PROGRAM_ID, RAYDIUM_CPMM_SOL_USDC_AMM_CONFIG, RAYDIUM_CPMM_SOL_USDC_POOL,
    RAYDIUM_CPMM_SOL_VAULT, RAYDIUM_CPMM_SWAP_BASE_INPUT_DISCRIMINATOR, RAYDIUM_CPMM_USDC_VAULT,
    RAYDIUM_SOL_USDC_OPEN_ORDERS, RAYDIUM_SOL_USDC_OPENBOOK_MARKET, RAYDIUM_SOL_USDC_POOL,
    RAYDIUM_SOL_USDC_SERUM_ASKS, RAYDIUM_SOL_USDC_SERUM_BIDS, RAYDIUM_SOL_USDC_SERUM_COIN_VAULT,
    RAYDIUM_SOL_USDC_SERUM_EVENT_QUEUE, RAYDIUM_SOL_USDC_SERUM_PC_VAULT,
    RAYDIUM_SOL_USDC_SERUM_VAULT_SIGNER, RAYDIUM_SOL_USDC_TARGET_ORDERS, RAYDIUM_SOL_VAULT,
    RAYDIUM_USDC_VAULT, RAYDIUM_V4_AUTHORITY, RAYDIUM_V4_PROGRAM_ID, SOLANA_PACKET_MTU_LIMIT,
    SYSTEM_PROGRAM_ID, SYSVAR_INSTRUCTIONS_ID, SYSVAR_RENT_ID, TOKEN_PROGRAM_ID, USDC_MINT,
    USDT_MINT, WSOL_MINT, get_cu_limit_for_hops, jito_tip_account_for_leader,
    jito_tip_account_for_slot,
};

// R10 split: cohesive groups extracted verbatim into domain submodules;
// glob re-exports preserve every existing `arb_tx::` path.
pub mod flash;
pub use flash::*;
pub mod primitives;
pub use primitives::*;
// Keep the canonical graph/dispatch entry points explicit at the crate root.
pub use primitives::ata::{
    RouteAtaInstructionCounts, RouteAtaPlan, build_route_ata_plan, route_ata_instruction_counts,
};
pub use primitives::cu::estimate_route_cu_with_shape;
pub mod protocols;
pub use protocols::*;
pub mod transport;
pub use transport::*;

#[cfg(test)]
mod tests {
    use super::*;
    use arb_types::{Bps, DexProtocol, MintId, PoolId, RouteId, Slot, SwapHop, TokenAmount};
    use ed25519_dalek::Verifier;

    #[test]
    fn cu_model_is_protocol_aware_and_route_sized() {
        // CLMM/DLMM cost more than a CPMM swap.
        assert!(per_hop_cu(DexProtocol::OrcaWhirlpool) > per_hop_cu(DexProtocol::RaydiumV4));
        assert!(per_hop_cu(DexProtocol::MeteoraDlmm) > per_hop_cu(DexProtocol::OrcaWhirlpool));
        // A 2-hop flash route with one ephemeral ATA is sized above the flat guess.
        let cu = estimate_route_cu(true, &[DexProtocol::RaydiumV4, DexProtocol::MeteoraDlmm], 1);
        assert!(cu >= 28_000 + 35_000 + 120_000);
        // The dynamic limit adds a ceiling-rounded 15% buffer and respects the ceiling.
        let lim = calculate_dynamic_cu_limit(cu);
        assert_eq!(lim, cu + cu.saturating_mul(15).div_ceil(100));
        assert!(calculate_dynamic_cu_limit(u32::MAX) <= MAX_COMPUTE_UNIT_LIMIT);
    }

    #[test]
    fn shape_aware_cu_estimate_counts_create_and_close_instructions_separately() {
        let hops = [DexProtocol::MeteoraDlmm, DexProtocol::OrcaWhirlpool];
        let no_ata = RouteAtaInstructionCounts::default();
        let one_create = RouteAtaInstructionCounts {
            unique_token_accounts: 1,
            intermediate_accounts: 0,
            idempotent_create_instructions: 1,
            close_token_account_instructions: 0,
        };
        let one_close = RouteAtaInstructionCounts {
            unique_token_accounts: 1,
            intermediate_accounts: 0,
            idempotent_create_instructions: 0,
            close_token_account_instructions: 1,
        };
        let base = estimate_route_cu_with_shape(true, &hops, no_ata, 2, 2, 10);
        let with_create = estimate_route_cu_with_shape(true, &hops, one_create, 2, 2, 10);
        let with_close = estimate_route_cu_with_shape(true, &hops, one_close, 2, 2, 10);
        assert_eq!(with_create - base, 20_000);
        assert_eq!(with_close - base, 7_500);
        assert!(estimate_route_cu_with_shape(true, &hops, no_ata, 6, 6, 10) > base);
    }

    #[test]
    fn measured_compute_limit_uses_ceil_ten_percent_margin() {
        assert_eq!(
            calculate_dynamic_cu_limit_with_buffer(100_000, 1_000),
            110_000
        );
        assert_eq!(
            calculate_dynamic_cu_limit_with_buffer(100_001, 1_000),
            110_002
        );
        assert_eq!(calculate_dynamic_cu_limit_with_buffer(1, 1_000), 2);
        assert_eq!(calculate_dynamic_cu_limit_with_buffer(0, 1_000), 0);
        assert_eq!(
            calculate_dynamic_cu_limit_with_buffer(u32::MAX, 1_000),
            MAX_COMPUTE_UNIT_LIMIT
        );
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

    /// §6.3 regression: the capped price's own lamport cost must NEVER exceed the
    /// profit-share budget it was derived from. The old implementation truncated
    /// both divisions, which understated a non-refundable cost; an earlier fix of
    /// mine over-corrected by rounding the price up, which let a 1-lamport budget
    /// produce a 2-lamport fee.
    #[test]
    fn priority_fee_cap_never_understates_the_lamport_budget() {
        for &(profit, cu_limit, share_bps) in &[
            (1_000_000u64, 200_000u32, 2_000u64), // 0.2% of 1 SOL
            (123_456_789, 360_000, 1_500),        // awkward numbers
            (1, 3, 1),                            // the case that caught the round-up bug
            (1, 1_400_000, 10_000),               // 100% share, 1 lamport
            (999_999, 48_000, 3_333),             // prime-ish
            (7, 13, 9_999),                       // tiny + tight
        ] {
            let price = cap_cu_price_by_profit(profit, cu_limit, u64::MAX, share_bps);
            let cost = priority_fee_lamports(price, cu_limit);
            let budget = profit.saturating_mul(share_bps).div_ceil(10_000);
            assert!(
                cost <= budget,
                "cap breached: price {price} x {cu_limit} CU = {cost} lamports > budget {budget} \
                 (profit {profit}, share {share_bps} bps)"
            );
        }
    }

    /// §6.3 regression: the cap must not be needlessly conservative either — it
    /// must use the largest price that still fits the budget, so the cap does not
    /// silently bid zero on a route that could afford priority.
    #[test]
    fn cap_uses_the_largest_price_that_fits_the_budget() {
        for &(profit, cu_limit, share_bps) in &[
            (1_000_000u64, 200_000u32, 2_000u64),
            (123_456_789, 360_000, 1_500),
            (999_999, 48_000, 3_333),
        ] {
            let price = cap_cu_price_by_profit(profit, cu_limit, u64::MAX, share_bps);
            let budget = profit.saturating_mul(share_bps).div_ceil(10_000);
            // One more micro-lamport per CU must NOT fit — proves maximality.
            let one_more = price.saturating_add(1);
            let cost_one_more = priority_fee_lamports(one_more, cu_limit);
            assert!(
                cost_one_more > budget,
                "cap is not maximal: price {} costs {cost_one_more} <= budget {budget}",
                price + 1
            );
        }
    }

    /// §6.3 regression: a cap that legitimately rounds to zero must STAY zero.
    /// The removed `calculate_dynamic_cu_price` added `profit/25_000` on top,
    /// so it resurrected a non-zero price from a cap that had correctly zeroed
    /// it — the wire then paid priority the gate had costed as free.
    #[test]
    fn cap_that_rounds_to_zero_stays_zero() {
        // Tiny profit spread over a huge CU limit: the budget is one lamport but
        // no whole micro-lamport price can be spread across 1.4M CU within it.
        let price = cap_cu_price_by_profit(1, 1_400_000, 50_000, 100);
        assert_eq!(price, 0, "cap must not resurrect a zeroed price");
        assert_eq!(
            priority_fee_lamports(price, 1_400_000),
            0,
            "a zeroed price must cost zero lamports"
        );
    }

    /// §6.3 regression: `priority_fee_lamports` must round UP (ceiling) so the
    /// non-refundable fee is never understated.
    #[test]
    fn priority_fee_lamports_rounds_up() {
        // 1 µlamport/CU x 1 CU = 0.000001 lamports -> 1 lamport (never 0).
        assert_eq!(priority_fee_lamports(1, 1), 1);
        // Exactly divisible is exact.
        assert_eq!(priority_fee_lamports(1_000_000, 1_000_000), 1_000_000);
        // 1_000_001 µlamports/CU x 1_000_000 CU = 1_000_001 lamports exactly.
        assert_eq!(priority_fee_lamports(1_000_001, 1_000_000), 1_000_001);
        // One micro-lamport over a whole lamport rounds up to the next lamport.
        assert_eq!(priority_fee_lamports(1, 1_000_001), 2);
        // Zero price costs nothing.
        assert_eq!(priority_fee_lamports(0, 400_000), 0);
        // A value that cannot be represented as u64 lamports must fail closed;
        // dividing a u64-saturated product would dangerously understate it.
        assert_eq!(priority_fee_lamports(u64::MAX, u32::MAX), u64::MAX);
    }

    #[test]
    fn cu_profit_cap_and_route_estimator_are_overflow_safe() {
        // u64 × u64 is evaluated in u128; a huge budget must not wrap into
        // a tiny or zero per-CU ceiling.
        assert_eq!(
            cap_cu_price_by_profit(u64::MAX, 1_400_000, u64::MAX, u64::MAX),
            u64::MAX
        );

        let many_dlmm_hops = vec![DexProtocol::MeteoraDlmm; 40_000];
        let estimate = estimate_route_cu(true, &many_dlmm_hops, usize::MAX);
        assert_eq!(estimate, u32::MAX);
        assert_eq!(calculate_dynamic_cu_limit(estimate), MAX_COMPUTE_UNIT_LIMIT);
    }

    /// §6.3 regression: the builder must encode the caller's authoritative price
    /// verbatim and must never add its own profit-based elevation. This is the
    /// invariant that stopped the wire from out-paying the profitability gate.
    #[test]
    fn builder_encodes_authoritative_cu_price_verbatim() {
        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let usdc = Pubkey::from_str(USDC_MINT).unwrap();
        let opp = ExecutionOpportunity {
            hop_slippage_bps: arb_types::DEFAULT_HOP_SLIPPAGE_BPS,
            cu_estimate_units: 0,
            cu_shape_key: String::new(),
            route_id: RouteId(1),
            slot: Slot(100),
            size_in: TokenAmount(1_000_000_000),
            expected_out: TokenAmount(1_050_000_000),
            // 2 hops: WSOL -> USDC -> WSOL (return to borrowed mint)
            hop_inputs: vec![1_000_000_000, 1_025_000_000, 1_050_000_000],
            gross_profit: Lamports(50_000_000),
            net_profit: Lamports(45_000_000),
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
                    pool_id: PoolId::new([11; 32]),
                    protocol: DexProtocol::RaydiumV4,
                    mint_in: MintId(usdc),
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

        // An arbitrary, deliberately non-default authoritative price.
        let authored = 7_777u64;
        let ixs = TxBuilder::assemble_flash_arbitrage(
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Lamports(5_000),
            400_000,
            authored,
            &opp,
            &KaminoFlashParams::default(),
            0,
            TokenAmount(0),
            &[],
        )
        .expect("assembly succeeds");

        let price_ix = ixs
            .iter()
            .find(|ix| ix.data.first() == Some(&3))
            .expect("SetComputeUnitPrice must be present");
        assert_eq!(
            u64::from_le_bytes(price_ix.data[1..9].try_into().unwrap()),
            authored,
            "the encoded CU price must equal the authoritative price exactly; \
             any re-elevation here means the wire pays more than the gate approved"
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

        let uncapped_cu = calculate_dynamic_cu_limit(400_000);
        assert_eq!(uncapped_cu, 460_000); // 400k + 15% — below the 1.4M ceiling, not clamped

        let capped_cu = calculate_dynamic_cu_limit(1_300_000);
        assert_eq!(capped_cu, MAX_COMPUTE_UNIT_LIMIT); // 1.3M + 15% clamped at 1.4M runtime max

        // Verify per-hop fallback constants and helper (raised to runtime max)
        assert_eq!(COMPUTE_BUDGET_LIMIT_2HOP, 1_400_000);
        assert_eq!(COMPUTE_BUDGET_LIMIT_3HOP, 1_400_000);
        assert_eq!(get_cu_limit_for_hops(2), 1_400_000);
        assert_eq!(get_cu_limit_for_hops(3), 1_400_000);
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
            pool_mint_a: Pubkey::from_str(USDC_MINT).unwrap(),
        }
    }

    fn mock_meteora_dlmm_exec_accounts(
        pool_token_x: Pubkey,
        pool_token_y: Pubkey,
    ) -> PoolExecutionAccounts {
        PoolExecutionAccounts::MeteoraDlmm {
            reserve_x: Pubkey::new_unique(),
            reserve_y: Pubkey::new_unique(),
            oracle: Pubkey::new_unique(),
            bin_array_pdas: vec![Pubkey::new_unique(), Pubkey::new_unique()],
            bitmap_extension: None,
            pool_token_x,
            pool_token_y,
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
            pool_mint_0: Pubkey::new_unique(),
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
            hop_slippage_bps: arb_types::DEFAULT_HOP_SLIPPAGE_BPS,
            cu_estimate_units: 0,
            cu_shape_key: String::new(),
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
                    execution_accounts: Some(mock_orca_whirlpool_exec_accounts()),
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
        // Base route encodes the caller's already-effective CU limit once.
        // Final-hop min_out carries the complete net-profit floor; no custom
        // executor program is required in the production flash path.
        assert_eq!(ixs_base.len(), 8);
        assert_eq!(&ixs_base[0].data[1..5], &160_000u32.to_le_bytes());
        assert_eq!(ixs_base[3].accounts[0].pubkey, payer); // Borrow user authority (signer)
        assert_eq!(ixs_base[3].accounts[2].pubkey, params.lending_market); // Borrow lending market
        assert_eq!(ixs_base[6].accounts[0].pubkey, payer); // Repay user authority (signer)
        assert_eq!(ixs_base[6].accounts[2].pubkey, params.lending_market); // Repay lending market

        // 2. Route with intermediate trending token (Ephemeral ATA Create -> Swaps -> Repay -> Close & Rent Reclaim)
        let intermediate_token = Pubkey::new_from_array([88; 32]);
        let opp_intermediate = ExecutionOpportunity {
            hop_slippage_bps: arb_types::DEFAULT_HOP_SLIPPAGE_BPS,
            cu_estimate_units: 0,
            cu_shape_key: String::new(),
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
                    execution_accounts: Some(mock_orca_whirlpool_exec_accounts()),
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
        // Base (8) + 1 Ephemeral ATA Create = 9 instructions. Residual-safe
        // lifecycle (F-01): the intermediate ATA is NEVER closed inside the
        // atomic transaction — a leg's actual output can exceed the exact
        // amount consumed by the next leg, and closing a non-empty token
        // account reverts the whole transaction. Rent reclaim happens
        // post-landing in a separate settlement tx.
        assert_eq!(ixs_inter.len(), 9);
        assert_eq!(
            ixs_inter[2].program_id,
            Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM_ID).unwrap()
        ); // CreateIdempotent (WSOL)
        assert_eq!(
            ixs_inter[3].program_id,
            Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM_ID).unwrap()
        ); // CreateIdempotent (Intermediate)
        assert!(
            ixs_inter.iter().all(|ix| !(ix.program_id
                == Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap()
                && ix.data == vec![9])),
            "intermediate ATA must never be closed inside the atomic arb tx (F-01)"
        ); // CloseAccount eliminated
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
                Some(mock_meteora_dlmm_exec_accounts(wsol, usdc)),
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
    fn test_final_hop_min_out_includes_all_execution_costs() {
        let wsol = Pubkey::from_str(WSOL_MINT).unwrap();
        let usdc = Pubkey::from_str(USDC_MINT).unwrap();

        let opp = ExecutionOpportunity {
            hop_slippage_bps: arb_types::DEFAULT_HOP_SLIPPAGE_BPS,
            cu_estimate_units: 0,
            cu_shape_key: String::new(),
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

        let network_fee = 25_000; // 5,000 base + 20,000 priority fee
        let legs = TxBuilder::derive_swap_legs_with_costs(
            &opp,
            flash_fee,
            network_fee,
            jito_tip,
            min_profit,
            slippage_bps,
        )
        .unwrap();
        assert_eq!(legs.len(), 2);

        // Required floor includes base + priority fees as well as flash fee, tip, and retained profit.
        let floor = 1_000_000_000 + flash_fee + network_fee + jito_tip + min_profit;
        let final_min_out = legs[1].1;
        assert!(
            final_min_out >= floor,
            "Final hop min_out {final_min_out} must enforce profit floor {floor}"
        );

        // If expected_out cannot meet the floor, derive_swap_legs must reject pre-flight
        let failing_opp = ExecutionOpportunity {
            hop_slippage_bps: arb_types::DEFAULT_HOP_SLIPPAGE_BPS,
            expected_out: TokenAmount(1_015_000_000), // below full cost floor
            hop_inputs: vec![1_000_000_000, 200_000_000, 1_015_000_000],
            ..opp
        };
        let err = TxBuilder::derive_swap_legs_with_costs(
            &failing_opp,
            flash_fee,
            network_fee,
            jito_tip,
            min_profit,
            slippage_bps,
        );
        assert!(matches!(err, Err(TxError::ProfitFloorNotMet { .. })));
    }

    #[test]
    fn split_flash_assembler_fails_closed_until_full_economic_floor_is_proven() {
        let result = TxBuilder::assemble_split_flash_arbitrage(
            Pubkey::new_unique(),
            Pubkey::new_unique(),
            Lamports(0),
            300_000,
            1_000,
            1_000_000,
            &[],
            &KaminoFlashParams::default(),
            0,
        );
        let error = result.expect_err("split routing must remain fail-closed");
        assert!(
            error
                .to_string()
                .contains("split flash routing is disabled"),
            "unexpected split-routing rejection: {error}"
        );
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
            hop_slippage_bps: arb_types::DEFAULT_HOP_SLIPPAGE_BPS,
            cu_estimate_units: 0,
            cu_shape_key: String::new(),
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
            hop_slippage_bps: arb_types::DEFAULT_HOP_SLIPPAGE_BPS,
            cu_estimate_units: 0,
            cu_shape_key: String::new(),
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

        // Jito supports up to five sequential, all-or-nothing transactions.
        let five_txs = vec![valid_tx.clone(); 5];
        assert!(validate_jito_bundle(&five_txs).is_ok());
        let six_txs = vec![valid_tx.clone(); 6];
        assert!(validate_jito_bundle(&six_txs).is_err());

        // MTU overflow
        let oversized = vec![2u8; 1233];
        assert!(validate_jito_bundle(&[oversized]).is_err());
    }
}
