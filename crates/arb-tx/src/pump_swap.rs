//! PumpSwap v2 swap instruction builders.

use super::*;

/// PumpSwap trade mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PumpSwapTradeMode {
    /// Exact quote in: spend exact quote tokens to get min base tokens (buy_exact_quote_in_v2).
    BuyExactQuoteInV2,
    /// Exact base out: specify exact base tokens to get with max quote tokens (buy_v2).
    BuyExactBaseOutV2,
    /// Exact base in: sell exact base tokens to get min quote tokens (sell_v2).
    SellExactBaseInV2,
    /// Legacy V1 buy (for non-V2 pools like cashback pools).
    BuyV1Legacy,
    /// Legacy V1 sell (for non-V2 pools like cashback pools).
    SellV1Legacy,
}

/// Build PumpSwap V2 trade instruction with the exact 17-account matrix matching `pump_amm.json`.
#[allow(clippy::too_many_arguments)]
pub fn build_pump_swap_instruction_v2(
    pool: Pubkey,
    user_authority: Pubkey,
    base_mint: Pubkey,
    quote_mint: Pubkey,
    pool_base_vault: Pubkey,
    pool_quote_vault: Pubkey,
    global_config: Pubkey,
    protocol_fee_recipient: Pubkey,
    protocol_fee_recipient_ata: Pubkey,
    coin_creator_vault_authority: Pubkey,
    coin_creator_vault_ata: Pubkey,
    token_program_base: Pubkey,
    token_program_quote: Pubkey,
    event_authority: Pubkey,
    amount_in: u64,
    min_amount_out: u64,
    trade_mode: PumpSwapTradeMode,
) -> Result<CompiledInstruction, TxError> {
    let program_id = Pubkey::from_str(PUMP_SWAP_PROGRAM_ID)
        .map_err(|e| TxError::InstructionError(e.to_string()))?;

    let system_program = Pubkey::new_from_array(SYSTEM_PROGRAM_ID);
    let spl_ata_program = Pubkey::from_str("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL").unwrap();

    let user_base_token = get_associated_token_address_with_program_id(
        &user_authority,
        &base_mint,
        &token_program_base,
    );
    let user_quote_token = get_associated_token_address_with_program_id(
        &user_authority,
        &quote_mint,
        &token_program_quote,
    );

    let fee_recipient_ata = if protocol_fee_recipient_ata != Pubkey::default() {
        protocol_fee_recipient_ata
    } else {
        get_associated_token_address_with_program_id(
            &protocol_fee_recipient,
            &quote_mint,
            &token_program_quote,
        )
    };

    let creator_vault_ata = if coin_creator_vault_ata != Pubkey::default() {
        coin_creator_vault_ata
    } else if coin_creator_vault_authority != Pubkey::default() {
        get_associated_token_address_with_program_id(
            &coin_creator_vault_authority,
            &quote_mint,
            &token_program_quote,
        )
    } else {
        fee_recipient_ata
    };

    let discriminator = match trade_mode {
        PumpSwapTradeMode::BuyExactQuoteInV2 => PUMP_SWAP_BUY_EXACT_QUOTE_IN_V2_DISCRIMINATOR,
        PumpSwapTradeMode::BuyExactBaseOutV2 => PUMP_SWAP_BUY_V2_DISCRIMINATOR,
        PumpSwapTradeMode::SellExactBaseInV2 => PUMP_SWAP_SELL_V2_DISCRIMINATOR,
        PumpSwapTradeMode::BuyV1Legacy => PUMP_SWAP_BUY_DISCRIMINATOR,
        PumpSwapTradeMode::SellV1Legacy => PUMP_SWAP_SELL_DISCRIMINATOR,
    };

    let mut data = Vec::with_capacity(24);
    data.extend_from_slice(&discriminator);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&min_amount_out.to_le_bytes());

    let accounts_meta = vec![
        AccountMeta::new(pool, false),
        AccountMeta::new(user_authority, true),
        AccountMeta::new_readonly(global_config, false),
        AccountMeta::new_readonly(base_mint, false),
        AccountMeta::new_readonly(quote_mint, false),
        AccountMeta::new(user_base_token, false),
        AccountMeta::new(user_quote_token, false),
        AccountMeta::new(pool_base_vault, false),
        AccountMeta::new(pool_quote_vault, false),
        AccountMeta::new_readonly(protocol_fee_recipient, false),
        AccountMeta::new(fee_recipient_ata, false),
        AccountMeta::new_readonly(token_program_base, false),
        AccountMeta::new_readonly(token_program_quote, false),
        AccountMeta::new_readonly(system_program, false),
        AccountMeta::new_readonly(spl_ata_program, false),
        AccountMeta::new_readonly(event_authority, false),
        AccountMeta::new(creator_vault_ata, false),
    ];

    Ok(CompiledInstruction {
        program_id,
        accounts: accounts_meta,
        data,
    })
}

/// Build genuine PumpSwap AMM swap instruction (Buy or Sell) with real on-chain account matrix (V1 compatibility).
#[allow(clippy::too_many_arguments)]
pub fn build_pump_swap_instruction(
    pool: Pubkey,
    user_authority: Pubkey,
    base_mint: Pubkey,
    quote_mint: Pubkey,
    pool_base_vault: Pubkey,
    pool_quote_vault: Pubkey,
    global_config: Pubkey,
    fee_config: Pubkey,
    protocol_fee_recipient: Pubkey,
    protocol_fee_recipient_ata: Pubkey,
    coin_creator_vault_authority: Pubkey,
    coin_creator_vault_ata: Pubkey,
    token_program_base: Pubkey,
    token_program_quote: Pubkey,
    amount_in: u64,
    min_amount_out: u64,
    is_buy: bool,
) -> Result<CompiledInstruction, TxError> {
    let program_id = Pubkey::from_str(PUMP_SWAP_PROGRAM_ID)
        .map_err(|e| TxError::InstructionError(e.to_string()))?;

    let user_base_token = get_associated_token_address(&user_authority, &base_mint);
    let user_quote_token = get_associated_token_address(&user_authority, &quote_mint);

    let fee_recipient_ata = if protocol_fee_recipient_ata != Pubkey::default() {
        protocol_fee_recipient_ata
    } else {
        get_associated_token_address(&protocol_fee_recipient, &quote_mint)
    };

    let creator_vault_ata = if coin_creator_vault_ata != Pubkey::default() {
        coin_creator_vault_ata
    } else if coin_creator_vault_authority != Pubkey::default() {
        get_associated_token_address(&coin_creator_vault_authority, &quote_mint)
    } else {
        fee_recipient_ata
    };

    let discriminator = if is_buy {
        PUMP_SWAP_BUY_DISCRIMINATOR
    } else {
        PUMP_SWAP_SELL_DISCRIMINATOR
    };

    let mut data = Vec::with_capacity(24);
    data.extend_from_slice(&discriminator);
    data.extend_from_slice(&amount_in.to_le_bytes());
    data.extend_from_slice(&min_amount_out.to_le_bytes());

    let accounts_meta = vec![
        AccountMeta::new(pool, false),
        AccountMeta::new(user_authority, true),
        AccountMeta::new_readonly(global_config, false),
        AccountMeta::new_readonly(base_mint, false),
        AccountMeta::new_readonly(quote_mint, false),
        AccountMeta::new(user_base_token, false),
        AccountMeta::new(user_quote_token, false),
        AccountMeta::new(pool_base_vault, false),
        AccountMeta::new(pool_quote_vault, false),
        AccountMeta::new_readonly(protocol_fee_recipient, false),
        AccountMeta::new(fee_recipient_ata, false),
        AccountMeta::new_readonly(token_program_base, false),
        AccountMeta::new_readonly(token_program_quote, false),
        AccountMeta::new_readonly(coin_creator_vault_authority, false),
        AccountMeta::new(creator_vault_ata, false),
        AccountMeta::new_readonly(fee_config, false),
    ];

    Ok(CompiledInstruction {
        program_id,
        accounts: accounts_meta,
        data,
    })
}
