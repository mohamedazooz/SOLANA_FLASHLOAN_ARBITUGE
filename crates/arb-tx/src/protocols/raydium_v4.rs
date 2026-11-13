//! Raydium AMM v4 swap instruction builders (all swap versions).

use crate::*;

/// Complete 18-Account Matrix for Raydium v4 SwapBaseIn instruction
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaydiumV4Accounts {
    pub token_program: Pubkey,
    pub amm_pool: Pubkey,
    pub amm_authority: Pubkey,
    pub amm_open_orders: Pubkey,
    pub amm_target_orders: Pubkey,
    pub pool_coin_vault: Pubkey,
    pub pool_pc_vault: Pubkey,
    pub serum_program: Pubkey,
    pub serum_market: Pubkey,
    pub serum_bids: Pubkey,
    pub serum_asks: Pubkey,
    pub serum_event_queue: Pubkey,
    pub serum_coin_vault: Pubkey,
    pub serum_pc_vault: Pubkey,
    pub serum_vault_signer: Pubkey,
}

impl RaydiumV4Accounts {
    pub fn mainnet_sol_usdc() -> Self {
        Self {
            token_program: Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap(),
            amm_pool: Pubkey::from_str(RAYDIUM_SOL_USDC_POOL).unwrap(),
            amm_authority: Pubkey::from_str("5Q544fKrFoe6tsEbD7S8EmxGTJYAKtTVhAW5Q5pge4j1")
                .unwrap(),
            amm_open_orders: Pubkey::from_str("HmiHHzq4Fym9e1D4qzLS6LDDM3tNsCTBPDWHTLZ763jY")
                .unwrap(),
            amm_target_orders: Pubkey::from_str("CZza3Ej4Mc58MnxWA385itCC9jCo3L1D7zc3LKy1bZMR")
                .unwrap(),
            pool_coin_vault: Pubkey::from_str(RAYDIUM_SOL_VAULT).unwrap(),
            pool_pc_vault: Pubkey::from_str(RAYDIUM_USDC_VAULT).unwrap(),
            serum_program: Pubkey::from_str("srmqPvymJeFKQ4zGQed1GFppgkRHL9kaELCbyksJtPX").unwrap(),
            serum_market: Pubkey::from_str("8BnEgHoWFysVcuFFX7QztDmzuH8r5ZFvyP3sYwn1XTh6").unwrap(),
            serum_bids: Pubkey::from_str("5jWUncPNBMZJ3sTHKmMLszypVkoRK6bfEQMQUHweeQnh").unwrap(),
            serum_asks: Pubkey::from_str("EaXdHx7x3mdGA38j5RSmKYSXMzAFzzUXCLNBEDXDn1d5").unwrap(),
            serum_event_queue: Pubkey::from_str("8CvwxZ9Db6XbLD46NZwwmVDZZRDy7eydFcAGkXKh9axa")
                .unwrap(),
            serum_coin_vault: Pubkey::from_str("CKxTHwM9fPMRRvZmFnFoqKNd9pQR21c5Aq9bh5h9oghX")
                .unwrap(),
            serum_pc_vault: Pubkey::from_str("6A5NHCj1yF6urc9wZNe6Bcjj4LVszQNj5DwAWG97yzMu")
                .unwrap(),
            serum_vault_signer: Pubkey::from_str("CTz5UMLQm2SRWHzQnU62Pi4yJqbNGjgRBHqqp6oDHfF7")
                .unwrap(),
        }
    }

    /// Construct Raydium V4 accounts for an arbitrary AMM pool.
    #[allow(clippy::too_many_arguments)]
    pub fn for_pool(
        amm_pool: Pubkey,
        amm_authority: Pubkey,
        amm_open_orders: Pubkey,
        amm_target_orders: Pubkey,
        pool_coin_vault: Pubkey,
        pool_pc_vault: Pubkey,
        serum_program: Pubkey,
        serum_market: Pubkey,
        serum_bids: Pubkey,
        serum_asks: Pubkey,
        serum_event_queue: Pubkey,
        serum_coin_vault: Pubkey,
        serum_pc_vault: Pubkey,
        serum_vault_signer: Pubkey,
    ) -> Self {
        Self {
            token_program: Pubkey::from_str(TOKEN_PROGRAM_ID).unwrap(),
            amm_pool,
            amm_authority,
            amm_open_orders,
            amm_target_orders,
            pool_coin_vault,
            pool_pc_vault,
            serum_program,
            serum_market,
            serum_bids,
            serum_asks,
            serum_event_queue,
            serum_coin_vault,
            serum_pc_vault,
            serum_vault_signer,
        }
    }
}

/// Build the legacy Raydium v4 `SwapBaseIn` instruction (opcode 9) with the
/// complete 18-account OpenBook/Serum matrix.
///
/// This is the **explicit V1 compatibility path**. New code should call
/// [`build_raydium_v4_swap_instruction_versioned`] and let the pool's resolved
/// [`arb_types::RaydiumV4SwapVersion`] choose the schema. It is retained
/// because older pools without `SwapBaseInV2` support still require it.
pub fn build_raydium_v4_swap_instruction_v1_legacy(
    accounts: &RaydiumV4Accounts,
    user_authority: Pubkey,
    user_source_token: Pubkey,
    user_destination_token: Pubkey,
    amount_in: u64,
    min_amount_out: u64,
) -> Result<CompiledInstruction, TxError> {
    let program_id = Pubkey::from_str(RAYDIUM_V4_PROGRAM_ID)
        .map_err(|e| TxError::InstructionError(e.to_string()))?;
    let mut data = Vec::with_capacity(17);
    data.push(9); // SwapBaseIn opcode (V1)
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&min_amount_out.to_le_bytes());

    Ok(CompiledInstruction {
        program_id,
        accounts: vec![
            AccountMeta::new_readonly(accounts.token_program, false),
            AccountMeta::new(accounts.amm_pool, false),
            AccountMeta::new_readonly(accounts.amm_authority, false),
            AccountMeta::new(accounts.amm_open_orders, false),
            AccountMeta::new(accounts.amm_target_orders, false),
            AccountMeta::new(accounts.pool_coin_vault, false),
            AccountMeta::new(accounts.pool_pc_vault, false),
            AccountMeta::new_readonly(accounts.serum_program, false),
            AccountMeta::new(accounts.serum_market, false),
            AccountMeta::new(accounts.serum_bids, false),
            AccountMeta::new(accounts.serum_asks, false),
            AccountMeta::new(accounts.serum_event_queue, false),
            AccountMeta::new(accounts.serum_coin_vault, false),
            AccountMeta::new(accounts.serum_pc_vault, false),
            AccountMeta::new_readonly(accounts.serum_vault_signer, false),
            AccountMeta::new(user_source_token, false),
            AccountMeta::new(user_destination_token, false),
            AccountMeta::new_readonly(user_authority, true),
        ],
        data,
    })
}

/// Backwards-compatible alias for the legacy 18-account V1 builder.
pub fn build_raydium_v4_swap_instruction_18(
    accounts: &RaydiumV4Accounts,
    user_authority: Pubkey,
    user_source_token: Pubkey,
    user_destination_token: Pubkey,
    amount_in: u64,
    min_amount_out: u64,
) -> Result<CompiledInstruction, TxError> {
    build_raydium_v4_swap_instruction_v1_legacy(
        accounts,
        user_authority,
        user_source_token,
        user_destination_token,
        amount_in,
        min_amount_out,
    )
}

/// Build the canonical Raydium AMM v4 `SwapBaseInV2` instruction
/// (opcode 16, 8 accounts, no OpenBook dependencies).
///
/// This is the default path for every pool. V1 is only built when a pool is
/// explicitly marked [`arb_types::RaydiumV4SwapVersion::V1`].
pub fn build_raydium_v4_swap_instruction(
    accounts: &RaydiumV4Accounts,
    user_authority: Pubkey,
    user_source_token: Pubkey,
    user_destination_token: Pubkey,
    amount_in: u64,
    min_amount_out: u64,
) -> Result<CompiledInstruction, TxError> {
    build_raydium_v4_swap_instruction_v2(
        accounts.token_program,
        accounts.amm_pool,
        accounts.amm_authority,
        accounts.pool_coin_vault,
        accounts.pool_pc_vault,
        user_source_token,
        user_destination_token,
        user_authority,
        amount_in,
        min_amount_out,
    )
}

/// Build a Raydium AMM v4 swap for an explicit schema version.
///
/// [`arb_types::RaydiumV4SwapVersion::V2`] is the default and requires no
/// OpenBook accounts. [`arb_types::RaydiumV4SwapVersion::V1`] builds the legacy
/// 18-account instruction and fails closed if any OpenBook account is missing
/// (all-zero), so a V1 pool can never be dispatched with an incomplete matrix.
pub fn build_raydium_v4_swap_instruction_versioned(
    accounts: &RaydiumV4Accounts,
    version: arb_types::RaydiumV4SwapVersion,
    user_authority: Pubkey,
    user_source_token: Pubkey,
    user_destination_token: Pubkey,
    amount_in: u64,
    min_amount_out: u64,
) -> Result<CompiledInstruction, TxError> {
    match version {
        arb_types::RaydiumV4SwapVersion::V2 => build_raydium_v4_swap_instruction(
            accounts,
            user_authority,
            user_source_token,
            user_destination_token,
            amount_in,
            min_amount_out,
        ),
        arb_types::RaydiumV4SwapVersion::V1 => {
            let required = [
                ("amm_authority", accounts.amm_authority),
                ("amm_open_orders", accounts.amm_open_orders),
                ("amm_target_orders", accounts.amm_target_orders),
                ("serum_program", accounts.serum_program),
                ("serum_market", accounts.serum_market),
                ("serum_bids", accounts.serum_bids),
                ("serum_asks", accounts.serum_asks),
                ("serum_event_queue", accounts.serum_event_queue),
                ("serum_coin_vault", accounts.serum_coin_vault),
                ("serum_pc_vault", accounts.serum_pc_vault),
                ("serum_vault_signer", accounts.serum_vault_signer),
            ];
            if let Some((name, _)) = required.iter().find(|(_, k)| *k == Pubkey::default()) {
                return Err(TxError::MissingExecutionAccounts {
                    pool_id: accounts.amm_pool,
                    reason: format!(
                        "Raydium V4 V1 requires {name}, but the resolved account is default"
                    ),
                });
            }
            build_raydium_v4_swap_instruction_v1_legacy(
                accounts,
                user_authority,
                user_source_token,
                user_destination_token,
                amount_in,
                min_amount_out,
            )
        }
    }
}

pub fn build_raydium_v4_swap_instruction_v2(
    token_program: Pubkey,
    amm_pool: Pubkey,
    amm_authority: Pubkey,
    pool_coin_vault: Pubkey,
    pool_pc_vault: Pubkey,
    user_source_token: Pubkey,
    user_destination_token: Pubkey,
    user_authority: Pubkey,
    amount_in: u64,
    min_amount_out: u64,
) -> Result<CompiledInstruction, TxError> {
    let program_id = Pubkey::from_str(RAYDIUM_V4_PROGRAM_ID)
        .map_err(|e| TxError::InstructionError(e.to_string()))?;
    let mut data = Vec::with_capacity(17);
    data.push(16); // AmmInstruction::SwapBaseInV2 opcode
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&min_amount_out.to_le_bytes());

    Ok(CompiledInstruction {
        program_id,
        accounts: vec![
            AccountMeta::new_readonly(token_program, false),
            AccountMeta::new(amm_pool, false),
            AccountMeta::new_readonly(amm_authority, false),
            AccountMeta::new(pool_coin_vault, false),
            AccountMeta::new(pool_pc_vault, false),
            AccountMeta::new(user_source_token, false),
            AccountMeta::new(user_destination_token, false),
            AccountMeta::new_readonly(user_authority, true),
        ],
        data,
    })
}
